//! TCP throughput data path (`docs/tcp.md`).
//!
//! The control handshake lives in `session.rs`; this module owns only the
//! mode-specific data movement: the server's receive loops + measurement, and
//! the client's send loops + live progress. Hot loops reuse one buffer (no
//! per-iteration allocation) and the payload is incompressible random bytes.

use crate::cli::Format;
use crate::meter::{self, Counter};
use crate::proto::{Negotiate, RoleDir, Start, TestResult};
use std::io;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Read buffer for the receive hot loop. Reused per stream, never reallocated.
const READ_BUF: usize = 256 * 1024;
/// Mixer for deriving distinct per-stream payload seeds.
const SEED_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Server side: accept `parallel` data connections, measure until `stop` fires,
/// and return the authoritative received-byte result (warm-up excluded).
///
/// `stop` resolves when the control side reads the client's `Stop` message.
pub async fn serve(
    data_listener: TcpListener,
    neg: &Negotiate,
    fmt: Format,
    stop: impl std::future::Future<Output = ()>,
) -> io::Result<TestResult> {
    let mut streams = Vec::with_capacity(neg.parallel as usize);
    for _ in 0..neg.parallel {
        let (s, _) = data_listener.accept().await?;
        streams.push(s);
    }

    let counters: Vec<Counter> = (0..streams.len()).map(|_| Counter::new()).collect();
    let t0 = Instant::now();

    let mut handles = Vec::new();
    for (stream, counter) in streams.into_iter().zip(counters.iter().cloned()) {
        handles.push(tokio::spawn(recv_loop(stream, counter)));
    }

    let sampler = tokio::spawn(meter::run_sampler(
        counters.clone(),
        t0,
        Duration::from_secs(neg.duration_secs + neg.warmup_secs),
        Duration::from_secs(neg.warmup_secs),
        fmt,
        false,
    ));

    // Snapshot bytes/time at warm-up end; those are the baseline we subtract so
    // the result reflects steady-state, not slow-start (docs/tcp.md §3).
    let warmup_dur = Duration::from_secs(neg.warmup_secs);
    let warmup_counters = counters.clone();
    let warmup_snap = tokio::spawn(async move {
        tokio::time::sleep(warmup_dur).await;
        (sum_bytes(&warmup_counters), Instant::now())
    });

    // Wait for Stop, snapshot final total before aborting receivers (tail race).
    stop.await;
    let final_bytes = sum_bytes(&counters);
    let final_at = Instant::now();
    for h in handles {
        h.abort();
    }
    let samples = sampler.await.unwrap_or_default();
    let (warmup_bytes, warmup_at) = warmup_snap.await.unwrap_or((0, t0));

    let bytes = final_bytes.saturating_sub(warmup_bytes);
    let secs = final_at.duration_since(warmup_at).as_secs_f64();

    Ok(TestResult {
        role: RoleDir::Receive,
        bytes,
        duration_secs: secs,
        bits_per_sec: meter::bits_per_sec(bytes, secs),
        bytes_per_sec: if secs > 0.0 { bytes as f64 / secs } else { 0.0 },
        packets_expected: None,
        packets_received: None,
        packets_lost: None,
        loss_ratio: None,
        jitter_ms: None,
        samples,
    })
}

/// Client side: open `parallel` data connections to the server's data port, send
/// for `warmup + duration`, and show live per-second progress unless `quiet`.
pub async fn drive(
    host: &str,
    start: &Start,
    neg: &Negotiate,
    fmt: Format,
    quiet: bool,
) -> io::Result<()> {
    let mut streams = Vec::with_capacity(neg.parallel as usize);
    for _ in 0..neg.parallel {
        let s = TcpStream::connect((host, start.data_port)).await?;
        s.set_nodelay(true)?; // docs/tcp.md §4
        streams.push(s);
    }

    let total = Duration::from_secs(neg.duration_secs + neg.warmup_secs);
    let t0 = Instant::now();
    let deadline = t0 + total;

    let counters: Vec<Counter> = (0..streams.len()).map(|_| Counter::new()).collect();
    let mut handles = Vec::new();
    for (i, (stream, counter)) in streams
        .into_iter()
        .zip(counters.iter().cloned())
        .enumerate()
    {
        // Per-stream seed so streams don't all send byte-identical buffers.
        let seed = start.server_seed ^ (i as u64).wrapping_mul(SEED_MIX);
        let buf_bytes = neg.buffer_bytes;
        handles.push(tokio::spawn(send_loop(
            stream, deadline, counter, seed, buf_bytes,
        )));
    }

    let warmup = Duration::from_secs(neg.warmup_secs);
    let sampler = (!quiet).then(|| {
        tokio::spawn(meter::run_sampler(
            counters.clone(),
            t0,
            total,
            warmup,
            fmt,
            false,
        ))
    });

    for h in handles {
        let _ = h.await;
    }
    if let Some(s) = sampler {
        let _ = s.await;
    }
    Ok(())
}

/// Build the random payload buffer the send loop reuses (for --verbose preview).
pub fn payload_sample(start: &Start, buffer_bytes: usize) -> Vec<u8> {
    let mut sample = vec![0u8; buffer_bytes];
    meter::fill_random(&mut sample, start.server_seed);
    sample
}

fn sum_bytes(counters: &[Counter]) -> u64 {
    counters.iter().map(Counter::get).sum()
}

/// Drain a data stream, counting bytes. Reuses one buffer (no per-read alloc).
async fn recv_loop(mut stream: TcpStream, counter: Counter) -> io::Result<()> {
    let mut buf = vec![0u8; READ_BUF];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(()); // peer closed
        }
        counter.add(n as u64);
    }
}

/// Write a once-filled incompressible buffer until `deadline`, counting bytes.
/// Alloc- and RNG-free hot loop.
async fn send_loop(
    mut stream: TcpStream,
    deadline: Instant,
    counter: Counter,
    seed: u64,
    buffer_bytes: usize,
) -> io::Result<()> {
    let mut buf = vec![0u8; buffer_bytes];
    meter::fill_random(&mut buf, seed);
    while Instant::now() < deadline {
        stream.write_all(&buf).await?;
        counter.add(buf.len() as u64);
    }
    stream.flush().await?;
    Ok(())
}
