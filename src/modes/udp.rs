//! UDP throughput + loss/jitter data path (`docs/udp.md`).
//!
//! The control handshake lives in `session.rs`; this module owns the UDP data
//! path: paced sending (RFC 8085), per-datagram seq + send-timestamp stamping,
//! one-way loss (RFC 7680) from sequence gaps, and interarrival jitter
//! (RFC 3550 §6.4.1). The server binds an ephemeral UDP socket and the client
//! sends datagrams to it; the server owns the authoritative loss/jitter result.

use crate::cli::Format;
use crate::proto::{Negotiate, RoleDir, Start, TestResult};
use std::io;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// Fixed datagram header: seq (u64 LE) + send_ns (u64 LE). See docs/udp.md §2.
const HEADER_LEN: usize = 16;
/// RFC 3550 §6.4.1 jitter smoothing gain (the standard 1/16).
const JITTER_GAIN: f64 = 1.0 / 16.0;
/// Pacing tick rate: send a small burst this many times per second (§3).
const TICKS_PER_SEC: u64 = 1000;

/// Packets to send per pacing tick to hit `bitrate` bits/s with `packet_bytes`
/// datagrams: `bitrate / (packet_bytes * 8) / ticks_per_sec` (§3). Fractional;
/// the sender carries the remainder so the average rate is exact over time.
fn packets_per_tick(bitrate: u64, packet_bytes: usize, ticks_per_sec: u64) -> f64 {
    let bits_per_packet = (packet_bytes as f64) * 8.0;
    bitrate as f64 / bits_per_packet / ticks_per_sec as f64
}

/// Write the 16-byte little-endian header into `buf`.
fn stamp(buf: &mut [u8], seq: u64, send_ns: u64) {
    buf[0..8].copy_from_slice(&seq.to_le_bytes());
    buf[8..16].copy_from_slice(&send_ns.to_le_bytes());
}

/// Read (seq, send_ns) from a received datagram, if it's long enough.
fn unstamp(buf: &[u8]) -> Option<(u64, u64)> {
    if buf.len() < HEADER_LEN {
        return None;
    }
    let seq = u64::from_le_bytes(buf[0..8].try_into().unwrap());
    let send_ns = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    Some((seq, send_ns))
}

// ---- Server (receiver) --------------------------------------------------

/// Bind an ephemeral UDP socket for the data path; returns it and its port so
/// `session` can advertise the port in `Start`.
pub async fn bind_data() -> io::Result<(UdpSocket, u16)> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).await?;
    let port = sock.local_addr()?.port();
    Ok((sock, port))
}

/// Receive and measure until `stop` resolves (the control side read `Stop`).
/// Computes throughput, one-way loss, and RFC 3550 jitter over the window,
/// excluding the warm-up packets by sequence.
pub async fn serve(
    sock: UdpSocket,
    neg: &Negotiate,
    fmt: Format,
    stop: impl std::future::Future<Output = ()>,
) -> io::Result<TestResult> {
    let packet_bytes = neg.packet_bytes.unwrap_or(1472).max(HEADER_LEN);
    // Warm-up packets are excluded from loss/jitter/throughput by sequence: any
    // seq below this threshold is part of the slow-start window.
    let bits_per_packet = (packet_bytes as u64) * 8;
    let pps = neg.bitrate_bps.unwrap_or(0) / bits_per_packet.max(1);
    let warmup_packets = pps * neg.warmup_secs;
    let warmup = Duration::from_secs(neg.warmup_secs);

    let mut buf = vec![0u8; packet_bytes.max(65_535)];
    let mut acc = LossJitter::new(warmup_packets);
    let t0 = Instant::now();
    let mut measure_start: Option<Instant> = None;

    // Once-per-second live readout + time series, sampled from the accumulator.
    let mut samples = Vec::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.tick().await; // first tick is immediate; skip it
    let mut last = acc.snapshot();
    let mut last_t = t0;

    // Receive loop races against `stop` and the 1s sampler.
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = ticker.tick() => {
                let now = Instant::now();
                let cur = acc.snapshot();
                let interval_bytes = cur.bytes.saturating_sub(last.bytes);
                let interval_lost = cur.lost.saturating_sub(last.lost);
                let secs = now.duration_since(last_t).as_secs_f64();
                let bps = if secs > 0.0 { interval_bytes as f64 * 8.0 / secs } else { 0.0 };
                let elapsed = now.duration_since(t0);
                let tag = if elapsed <= warmup + Duration::from_millis(100) {
                    "  (warm-up, excluded)"
                } else {
                    ""
                };
                eprintln!(
                    "[{:5.1}s] {} | loss {} | jitter {:.3} ms{tag}",
                    elapsed.as_secs_f64(),
                    fmt.render(bps),
                    interval_lost,
                    cur.jitter_ms,
                );
                samples.push(crate::proto::Sample {
                    t_secs: elapsed.as_secs_f64(),
                    bytes: interval_bytes,
                    bits_per_sec: bps,
                    packets_lost: Some(interval_lost),
                    jitter_ms: Some(cur.jitter_ms),
                });
                last = cur;
                last_t = now;
            }
            r = sock.recv_from(&mut buf) => {
                let (n, _peer) = r?;
                let Some((seq, send_ns)) = unstamp(&buf[..n]) else { continue };
                let recv_ns = t0.elapsed().as_nanos() as u64;
                if seq >= warmup_packets && measure_start.is_none() {
                    measure_start = Some(Instant::now());
                }
                acc.observe(seq, send_ns, recv_ns, n as u64);
            }
        }
    }

    let secs = measure_start
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let mut result = acc.into_result(secs);
    result.samples = samples;
    Ok(result)
}

/// Point-in-time totals for the once-per-second readout.
struct Snapshot {
    bytes: u64,
    lost: u64,
    jitter_ms: f64,
}

/// Accumulates loss and jitter state across received datagrams (§4).
struct LossJitter {
    warmup_packets: u64,
    bytes: u64,
    received: u64,
    min_seq: Option<u64>,
    max_seq: u64,
    // RFC 3550 jitter state: previous transit time and smoothed jitter (ns).
    prev_transit: Option<i64>,
    jitter_ns: f64,
}

impl LossJitter {
    fn new(warmup_packets: u64) -> Self {
        Self {
            warmup_packets,
            bytes: 0,
            received: 0,
            min_seq: None,
            max_seq: 0,
            prev_transit: None,
            jitter_ns: 0.0,
        }
    }

    fn observe(&mut self, seq: u64, send_ns: u64, recv_ns: u64, n: u64) {
        // Only in-window packets count toward the reported metrics.
        if seq < self.warmup_packets {
            return;
        }
        self.bytes += n;
        self.received += 1;
        self.min_seq = Some(self.min_seq.map_or(seq, |m| m.min(seq)));
        self.max_seq = self.max_seq.max(seq);

        // RFC 3550 §6.4.1: transit = recv - send; D = transit_i - transit_{i-1};
        // J += (|D| - J) / 16. Clock offset cancels because D is a difference of
        // differences, so the two unsynchronized monotonic clocks are fine.
        let transit = recv_ns as i64 - send_ns as i64;
        if let Some(prev) = self.prev_transit {
            let d = (transit - prev).abs() as f64;
            self.jitter_ns += (d - self.jitter_ns) * JITTER_GAIN;
        }
        self.prev_transit = Some(transit);
    }

    /// A point-in-time view for the once-per-second readout. `lost` is the
    /// running total (seq span seen minus packets received).
    fn snapshot(&self) -> Snapshot {
        let expected = match self.min_seq {
            Some(min) => self.max_seq - min + 1,
            None => 0,
        };
        Snapshot {
            bytes: self.bytes,
            lost: expected.saturating_sub(self.received),
            jitter_ms: self.jitter_ns / 1e6,
        }
    }

    fn into_result(self, secs: f64) -> TestResult {
        let expected = match self.min_seq {
            Some(min) => self.max_seq - min + 1,
            None => 0,
        };
        let lost = expected.saturating_sub(self.received);
        let loss_ratio = if expected > 0 {
            lost as f64 / expected as f64
        } else {
            0.0
        };
        let bits_per_sec = if secs > 0.0 {
            (self.bytes as f64 * 8.0) / secs
        } else {
            0.0
        };
        TestResult {
            role: RoleDir::Receive,
            bytes: self.bytes,
            duration_secs: secs,
            bits_per_sec,
            bytes_per_sec: if secs > 0.0 {
                self.bytes as f64 / secs
            } else {
                0.0
            },
            packets_expected: Some(expected),
            packets_received: Some(self.received),
            packets_lost: Some(lost),
            loss_ratio: Some(loss_ratio),
            jitter_ms: Some(self.jitter_ns / 1e6),
            samples: Vec::new(),
        }
    }
}

// ---- Client (sender) ----------------------------------------------------

/// Send paced datagrams to the server's data port for `warmup + duration`.
/// Pacing follows RFC 8085: a small burst per timer tick, with fractional
/// packets carried forward so the average rate matches `bitrate_bps` (§3).
///
/// Prints send-side per-second stats unless `quiet`. These are *sent* rates
/// (what the client paced out); the authoritative loss/jitter come from the
/// server's `Result`.
pub async fn drive(
    host: &str,
    start: &Start,
    neg: &Negotiate,
    fmt: Format,
    quiet: bool,
) -> io::Result<()> {
    let packet_bytes = neg.packet_bytes.unwrap_or(1472).max(HEADER_LEN);
    let bitrate = neg.bitrate_bps.unwrap_or(1_000_000_000);

    let sock = UdpSocket::bind(("0.0.0.0", 0)).await?;
    sock.connect((host, start.data_port)).await?;

    let packets_per_tick = packets_per_tick(bitrate, packet_bytes, TICKS_PER_SEC);

    let mut buf = vec![0u8; packet_bytes];
    // Fill padding once with the seed (incompressible); header is rewritten per packet.
    crate::meter::fill_random(&mut buf, start.server_seed);

    let total = Duration::from_secs(neg.duration_secs + neg.warmup_secs);
    let warmup = Duration::from_secs(neg.warmup_secs);
    let t0 = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_micros(1_000_000 / TICKS_PER_SEC));

    let mut seq: u64 = 0;
    let mut carry: f64 = 0.0;
    // Send-side per-second readout state.
    let mut next_report = Duration::from_secs(1);
    let mut last_packets: u64 = 0;
    while t0.elapsed() < total {
        ticker.tick().await;
        carry += packets_per_tick;
        let mut to_send = carry.floor() as u64;
        carry -= to_send as f64;
        while to_send > 0 {
            let send_ns = t0.elapsed().as_nanos() as u64;
            stamp(&mut buf, seq, send_ns);
            sock.send(&buf).await?;
            seq += 1;
            to_send -= 1;
        }

        // Emit a send-side line roughly once per second.
        let elapsed = t0.elapsed();
        if !quiet && elapsed >= next_report {
            let interval_packets = seq - last_packets;
            let bps = interval_packets as f64 * (packet_bytes as f64 * 8.0);
            let tag = if elapsed <= warmup + Duration::from_millis(100) {
                "  (warm-up, excluded)"
            } else {
                ""
            };
            eprintln!(
                "[{:5.1}s] {} sent | {} pkts{tag}",
                elapsed.as_secs_f64(),
                fmt.render(bps),
                interval_packets,
            );
            last_packets = seq;
            next_report += Duration::from_secs(1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pacing math: at 1 Gbit/s with 1472-byte datagrams and 1000 ticks/s,
    /// send ~84.9 packets/tick (so the average rate is 1 Gbit/s).
    #[test]
    fn packets_per_tick_math() {
        let ppt = packets_per_tick(1_000_000_000, 1472, 1000);
        // 1e9 / (1472*8) / 1000 = 84.92…
        assert!((ppt - 84.92).abs() < 0.01, "got {ppt}");

        // Sanity: reconstruct the rate from the per-tick count.
        let rate = ppt * (1472.0 * 8.0) * 1000.0;
        assert!((rate - 1e9).abs() < 1.0);
    }

    /// Very low rates yield a sub-1 per-tick count (carried via the remainder).
    #[test]
    fn packets_per_tick_low_rate() {
        let ppt = packets_per_tick(64_000, 1472, 1000); // 64 Kbit/s
        assert!(ppt < 1.0 && ppt > 0.0, "got {ppt}");
    }

    #[test]
    fn stamp_unstamp_roundtrip() {
        let mut buf = vec![0u8; 32];
        stamp(&mut buf, 12345, 678_900_000);
        assert_eq!(unstamp(&buf), Some((12345, 678_900_000)));
    }

    #[test]
    fn unstamp_rejects_short_datagram() {
        assert_eq!(unstamp(&[0u8; 8]), None);
    }

    /// No loss, evenly-spaced arrivals → jitter ≈ 0, loss 0.
    #[test]
    fn perfect_stream_has_no_loss_or_jitter() {
        let mut lj = LossJitter::new(0);
        for seq in 0..100u64 {
            // send and recv advance in lockstep: constant transit → D = 0.
            lj.observe(seq, seq * 1000, seq * 1000 + 500, 100);
        }
        let r = lj.into_result(1.0);
        assert_eq!(r.packets_lost, Some(0));
        assert_eq!(r.packets_received, Some(100));
        assert!(r.jitter_ms.unwrap() < 1e-6);
    }

    /// A sequence gap is counted as loss (RFC 7680).
    #[test]
    fn sequence_gap_is_loss() {
        let mut lj = LossJitter::new(0);
        for seq in [0u64, 1, 2, 5, 6] {
            lj.observe(seq, seq * 1000, seq * 1000 + 500, 100);
        }
        let r = lj.into_result(1.0);
        // expected = max(6) - min(0) + 1 = 7; received = 5; lost = 2 (seq 3,4).
        assert_eq!(r.packets_expected, Some(7));
        assert_eq!(r.packets_received, Some(5));
        assert_eq!(r.packets_lost, Some(2));
    }

    /// Warm-up packets (seq below threshold) are excluded from the metrics.
    #[test]
    fn warmup_packets_excluded() {
        let mut lj = LossJitter::new(10);
        for seq in 0..20u64 {
            lj.observe(seq, seq * 1000, seq * 1000 + 500, 100);
        }
        let r = lj.into_result(1.0);
        // Only seq 10..=19 count: 10 packets, expected 10, no loss.
        assert_eq!(r.packets_received, Some(10));
        assert_eq!(r.packets_expected, Some(10));
        assert_eq!(r.packets_lost, Some(0));
    }
}
