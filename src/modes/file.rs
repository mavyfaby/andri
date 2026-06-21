//! File-transfer data path (`docs/file.md`).
//!
//! Measures **real-world end-to-end transfer speed**: the client reads a file
//! from disk and streams it over TCP; the server receives and measures. Unlike
//! raw TCP mode, this number deliberately includes the sender's disk read — that
//! gap (vs. raw TCP) is andri's differentiator. `--null-source` removes the disk
//! read to isolate the network.
//!
//! v1: single file, single stream, receiver discards (counts bytes, no write —
//! so this measures sender-disk + network). No warm-up exclusion (a real copy
//! includes its own ramp; excluding it would misrepresent "real-world" speed).

use crate::cli::Format;
use crate::meter::{self, Counter};
use crate::proto::{Negotiate, RoleDir, Start, TestResult};
use std::io;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Copy buffer for the disk↔socket hot loop. Reused, never reallocated.
const BUF: usize = 256 * 1024;

/// Read a file's length without opening it for transfer (client, pre-negotiate).
pub async fn file_len(path: &str) -> io::Result<u64> {
    Ok(tokio::fs::metadata(path).await?.len())
}

// ---- Server (receiver) --------------------------------------------------

/// Receive exactly `file_len` bytes from one stream and measure throughput.
/// Bytes are counted and discarded (v1 — no receiver-side disk write).
pub async fn serve(
    data_listener: TcpListener,
    neg: &Negotiate,
    fmt: Format,
) -> io::Result<TestResult> {
    let expected = neg.file_len.unwrap_or(0);
    let (mut stream, _) = data_listener.accept().await?;

    let counter = Counter::new();
    let t0 = Instant::now();
    // Live readout for the duration of the transfer (no fixed window — bounded
    // by file_len, so we sample until the bytes are in).
    let sampler = {
        let counters = vec![counter.clone()];
        tokio::spawn(sample_until_done(counters, t0, fmt, expected))
    };

    let mut buf = vec![0u8; BUF];
    let mut received: u64 = 0;
    while received < expected {
        let want = ((expected - received) as usize).min(buf.len());
        let n = stream.read(&mut buf[..want]).await?;
        if n == 0 {
            break; // peer closed early (short transfer)
        }
        received += n as u64;
        counter.add(n as u64);
    }
    let secs = t0.elapsed().as_secs_f64();
    let samples = sampler.await.unwrap_or_default();

    Ok(TestResult {
        role: RoleDir::Receive,
        bytes: received,
        duration_secs: secs,
        bits_per_sec: meter::bits_per_sec(received, secs),
        bytes_per_sec: if secs > 0.0 {
            received as f64 / secs
        } else {
            0.0
        },
        packets_expected: None,
        packets_received: None,
        packets_lost: None,
        loss_ratio: None,
        jitter_ms: None,
        samples,
    })
}

/// Sample the byte counter once per second until `expected` bytes arrive.
async fn sample_until_done(
    counters: Vec<Counter>,
    t0: Instant,
    fmt: Format,
    expected: u64,
) -> Vec<crate::proto::Sample> {
    let mut samples = Vec::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    ticker.tick().await; // immediate first tick
    let mut last_bytes = 0u64;
    let mut last_t = t0;
    loop {
        ticker.tick().await;
        let now = Instant::now();
        let cur: u64 = counters.iter().map(Counter::get).sum();
        let interval = cur.saturating_sub(last_bytes);
        let secs = now.duration_since(last_t).as_secs_f64();
        let bps = if secs > 0.0 {
            interval as f64 * 8.0 / secs
        } else {
            0.0
        };
        eprintln!(
            "[{:5.1}s] {}",
            now.duration_since(t0).as_secs_f64(),
            fmt.render(bps)
        );
        samples.push(crate::proto::Sample {
            t_secs: now.duration_since(t0).as_secs_f64(),
            bytes: interval,
            bits_per_sec: bps,
            packets_lost: None,
            jitter_ms: None,
        });
        last_bytes = cur;
        last_t = now;
        if cur >= expected {
            break;
        }
    }
    samples
}

// ---- Client (sender) ----------------------------------------------------

/// Stream `file_len` bytes to the server: read from `path` on disk, or generate
/// in-memory bytes when `null_source`. Bounded by the negotiated `file_len`.
///
/// Prints send-side per-second progress unless `quiet`. Fast transfers (e.g. on
/// loopback) may finish in under a second and show no interval line — the final
/// summary still reports the real rate.
pub async fn drive(
    host: &str,
    start: &Start,
    neg: &Negotiate,
    path: Option<&str>,
    null_source: bool,
    fmt: Format,
    quiet: bool,
) -> io::Result<()> {
    let total = neg.file_len.unwrap_or(0);
    let mut stream = TcpStream::connect((host, start.data_port)).await?;
    stream.set_nodelay(true)?;

    let mut buf = vec![0u8; BUF];
    let mut sent: u64 = 0;

    // Send-side per-second progress (sent bytes).
    let t0 = Instant::now();
    let mut next_report = std::time::Duration::from_secs(1);
    let mut last_sent: u64 = 0;
    let mut report = |sent: u64, force: bool| {
        let elapsed = t0.elapsed();
        if quiet || (!force && elapsed < next_report) {
            return;
        }
        let interval = sent - last_sent;
        let secs = elapsed.as_secs_f64();
        let bps = if secs > 0.0 {
            interval as f64 * 8.0 / secs
        } else {
            0.0
        };
        eprintln!(
            "[{:5.1}s] {} sent | {:.0}% ({}/{} bytes)",
            secs,
            fmt.render(bps),
            if total > 0 {
                sent as f64 / total as f64 * 100.0
            } else {
                100.0
            },
            sent,
            total,
        );
        last_sent = sent;
        next_report = elapsed + std::time::Duration::from_secs(1);
    };

    if null_source {
        // Fill the buffer once with incompressible bytes; reuse it (no disk).
        meter::fill_random(&mut buf, start.server_seed);
        while sent < total {
            let want = ((total - sent) as usize).min(buf.len());
            stream.write_all(&buf[..want]).await?;
            sent += want as u64;
            report(sent, false);
        }
    } else {
        let path = path.expect("file path required in file mode");
        let mut file = File::open(path).await?;
        while sent < total {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break; // EOF (file shorter than negotiated len)
            }
            stream.write_all(&buf[..n]).await?;
            sent += n as u64;
            report(sent, false);
        }
    }
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// file_len reports the on-disk size of a file.
    #[tokio::test]
    async fn file_len_reports_size() {
        let dir = std::env::temp_dir();
        let path = dir.join("andri-file-len-test.bin");
        let data = vec![0u8; 4096];
        tokio::fs::write(&path, &data).await.unwrap();
        let len = file_len(path.to_str().unwrap()).await.unwrap();
        assert_eq!(len, 4096);
        let _ = tokio::fs::remove_file(&path).await;
    }

    /// A missing file is an error, surfaced before any transfer starts.
    #[tokio::test]
    async fn file_len_missing_is_error() {
        assert!(file_len("/no/such/andri/file.bin").await.is_err());
    }
}
