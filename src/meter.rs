//! Throughput metering: lock-free byte counters and once-per-second sampling.
//!
//! Each data stream owns a `Counter` (an `Arc<AtomicU64>`) and adds to it in its
//! hot loop. A sampler reads the aggregate once per second to build the live
//! readout and the `Result.samples[]` time series (`docs/protocol.md` §3.6,
//! `docs/tcp.md` §2). Monotonic clock only (`Instant`).

use crate::cli::Format;
use crate::proto::Sample;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A shared, lock-free byte counter for one data stream.
#[derive(Clone, Default)]
pub struct Counter(Arc<AtomicU64>);

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `n` bytes. `Relaxed` is sufficient: we never use the counter to
    /// establish ordering of other memory, only to total bytes.
    #[inline]
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Sums a set of per-stream counters.
fn total(counters: &[Counter]) -> u64 {
    counters.iter().map(Counter::get).sum()
}

/// Fill `buf` with pseudo-random, incompressible bytes derived from `seed`.
///
/// Done once before the send loop and reused unchanged, so the randomness cost
/// is paid a single time (not per write) and the hot loop stays alloc/RNG-free.
/// We need *incompressibility* (so compressing links can't inflate throughput,
/// matching iperf3's random default), not cryptographic strength — splitmix64 is
/// a fast, well-distributed seeded generator that's plenty for that. The seed is
/// the protocol's `server_seed`, so content is reproducible/verifiable.
pub fn fill_random(buf: &mut [u8], seed: u64) {
    // splitmix64
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut chunks = buf.chunks_exact_mut(8);
    for c in &mut chunks {
        c.copy_from_slice(&next().to_le_bytes());
    }
    let rem = chunks.into_remainder();
    if !rem.is_empty() {
        let bytes = next().to_le_bytes();
        rem.copy_from_slice(&bytes[..rem.len()]);
    }
}

/// A short debug description of a payload buffer: a hex sample of the leading
/// bytes plus a distinct-byte-value count as a quick entropy sanity check.
/// Used under `--verbose` to confirm the payload is incompressible, not zeros.
pub fn payload_preview(buf: &[u8]) -> String {
    let n = buf.len().min(16);
    let hex: String = buf[..n].iter().map(|b| format!("{b:02x} ")).collect();
    let distinct = buf.iter().collect::<std::collections::HashSet<_>>().len();
    format!("{}… ({distinct}/256 distinct byte values)", hex.trim_end())
}

/// Bits per second from a byte delta over an elapsed duration.
pub fn bits_per_sec(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        0.0
    } else {
        (bytes as f64 * 8.0) / secs
    }
}

/// Sample the aggregate counters once per second for `total` time, emitting a
/// live readout line and collecting the per-second time series.
///
/// `t0` is the send start (before warm-up); `total` is the whole sending window
/// (`warmup + duration`). Lines within the first `warmup` are tagged as warm-up,
/// since those bytes are excluded from the final result. Returns the samples for
/// `Result.samples[]`; each is for that 1s interval.
pub async fn run_sampler(
    counters: Vec<Counter>,
    t0: Instant,
    window: Duration,
    warmup: Duration,
    fmt: Format,
    quiet: bool,
) -> Vec<Sample> {
    let mut samples = Vec::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.tick().await; // first tick fires immediately; skip it

    let mut last_bytes = total(&counters);
    let mut last_t = t0;

    loop {
        ticker.tick().await;
        let now = Instant::now();
        let cur = total(&counters);

        let interval_bytes = cur.saturating_sub(last_bytes);
        let interval_secs = now.duration_since(last_t).as_secs_f64();
        let bps = bits_per_sec(interval_bytes, interval_secs);

        samples.push(Sample {
            t_secs: now.duration_since(t0).as_secs_f64(),
            bytes: interval_bytes,
            bits_per_sec: bps,
            packets_lost: None,
            jitter_ms: None,
        });

        let elapsed = now.duration_since(t0);
        if !quiet {
            // Tag lines inside the warm-up window: their bytes are excluded from
            // the final result, which is why the line count exceeds --duration.
            let tag = if elapsed <= warmup + Duration::from_millis(100) {
                "  (warm-up, excluded)"
            } else {
                ""
            };
            eprintln!("[{:5.1}s] {}{tag}", elapsed.as_secs_f64(), fmt.render(bps));
        }

        last_bytes = cur;
        last_t = now;

        if elapsed >= window {
            break;
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same seed must produce identical bytes (reproducible/verifiable payload).
    #[test]
    fn fill_random_is_deterministic() {
        let mut a = [0u8; 4096];
        let mut b = [0u8; 4096];
        fill_random(&mut a, 0x5EED_2026);
        fill_random(&mut b, 0x5EED_2026);
        assert_eq!(a, b);
    }

    /// Different seeds produce different bytes (per-stream variation).
    #[test]
    fn fill_random_varies_by_seed() {
        let mut a = [0u8; 4096];
        let mut b = [0u8; 4096];
        fill_random(&mut a, 1);
        fill_random(&mut b, 2);
        assert_ne!(a, b);
    }

    /// Output must not be a low-entropy run of identical bytes (it would be
    /// compressible and defeat the purpose). Check the byte values vary.
    #[test]
    fn fill_random_is_high_entropy() {
        let mut buf = [0u8; 4096];
        fill_random(&mut buf, 0x5EED_2026);
        let distinct = buf.iter().collect::<std::collections::HashSet<_>>().len();
        // A uniform random 4 KiB buffer hits the vast majority of 256 values.
        assert!(distinct > 200, "only {distinct} distinct byte values");
    }

    /// A non-multiple-of-8 length is filled completely (remainder handling).
    #[test]
    fn fill_random_handles_remainder() {
        let mut buf = [0u8; 13];
        fill_random(&mut buf, 7);
        assert!(buf.iter().any(|&b| b != 0));
    }
}
