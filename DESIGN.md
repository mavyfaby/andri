# andri — Design

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

This document captures the architecture and protocol decisions for `andri`. It is the
source of truth for implementation; the [README](README.md) is the user-facing summary.
Testing approach and the project-wide test index live in
[docs/testing.md](docs/testing.md).

## Goals

- One binary, three measurements: **TCP throughput**, **UDP throughput + loss/jitter**,
  and **real file-transfer speed**.
- Cleanly separate **network-only** measurement from **end-to-end file copy** (disk
  included), so a slow result can be attributed to the wire or to I/O.
- `iperf3`-style ergonomics: `--server` on one host, `--client <ip>` on the other.

## Stack

| Concern | Choice | Notes |
|---|---|---|
| Async runtime | `tokio` | `TcpStream`, `TcpListener`, `UdpSocket`. |
| CLI parsing | `clap` (derive) | |
| Control serialization | `serde` + `serde_json` | May swap to `bincode`/`postcard` later for compactness. |
| Timing | `std::time::Instant` | Monotonic clock **only** — never `SystemTime`. |

## Topology: control channel vs. data channels

Every session has exactly one **control connection** and one or more **data connections**.

- **Control connection** — a TCP connection opened first by the client. Carries
  negotiation, the start signal, and final results as length-delimited JSON messages.
  It stays open for the lifetime of the session.
- **Data connections** — opened per test. Parallel streams = N `tokio` tasks, each owning
  its own connection. Each task reports its byte count via an `AtomicU64`, sampled once
  per second on the control side for the live readout.

```
client                                   server
  │  ── control TCP connect ──────────────▶ │
  │  ── Negotiate { mode, params } ───────▶ │
  │  ◀───────────────── Start { ok, port } ─ │
  │  ══ data stream 1 ════════════════════▶ │
  │  ══ data stream 2 ════════════════════▶ │   (N parallel tasks)
  │  ◀──────────────────────── Result {...} ─ │
```

## Control protocol messages

> **Full spec:** [docs/protocol.md](docs/protocol.md) — message framing, complete schema,
> session state machine, timeouts, and RFC grounding. The sketch below is the overview.

Serde structs/enums, JSON to start. Sketch:

```rust
enum Mode { Tcp, Udp, File }

struct Negotiate {
    mode: Mode,
    duration_secs: u64,
    parallel: u32,
    buffer_bytes: usize,
    bitrate: Option<u64>,   // UDP target bits/s
    bidir: bool,
}

struct Start {
    ok: bool,
    data_port: u16,         // where the client should connect its data streams
    error: Option<String>,
}

struct Result {
    bytes: u64,
    duration_secs: f64,
    bits_per_sec: f64,
    bytes_per_sec: f64,
    // UDP only:
    packets_sent: Option<u64>,
    packets_lost: Option<u64>,
    jitter_ms: Option<f64>,
}
```

## Per-mode notes & gotchas

### TCP throughput
> **Full spec:** [docs/tcp.md](docs/tcp.md). Overview below.

- Reuse a **single buffer** in the hot loop — no per-iteration allocation.
- Default buffer **64 KiB**.
- Include a **warm-up period** whose bytes are excluded from the measurement, so TCP
  slow-start doesn't depress the number.
- Support **parallel streams** and **configurable socket buffers** (`SO_SNDBUF`/`SO_RCVBUF`).

### UDP throughput + loss/jitter
> **Full spec:** [docs/udp.md](docs/udp.md) — datagram layout, pacing math, RFC 3550
> jitter, RFC 7680 loss. Overview below.

- **Pace the sender — don't busy-spin.** For high rates, send small bursts per
  `tokio::time::interval` tick:
  `packets_per_tick = bitrate ÷ packet_size ÷ ticks_per_sec`.
- Stamp each datagram at a **fixed offset, little-endian**: `u64` sequence number +
  `u64` nanosecond send-timestamp.
- Receiver computes **loss** from sequence gaps and **jitter** via the RFC 3550
  algorithm.

### File transfer
> **Full spec:** [docs/file.md](docs/file.md). CLI surface: [docs/cli.md](docs/cli.md).
> Overview below.

- Read a real file from disk → stream → write on the far end.
- `--null-source` flag sources from in-memory / null instead of disk, to **isolate the
  network from disk I/O**.

### General
- **Monotonic clock only** (`Instant`).
- Report **both bits/s and bytes/s**.
- **Long enough** default duration (**10s**) to average out transients.
- **Bidirectional** means send + receive **simultaneously**, not back-to-back.

## Client model

andri supports **two client surfaces** against one server:

1. **Thin client binary** (`andri --client <ip>`) — speaks the custom TCP control protocol
   ([docs/protocol.md](docs/protocol.md)) and can run **all three modes**, including raw
   UDP loss/jitter.
2. **Browser** *(v2 — deferred)* — the server will host an embedded HTTP + WebSocket
   dashboard ([docs/web.md](docs/web.md)). Zero install; good for compatibility and ease
   of use. Not part of v1 (see v1 scope / deferrals below).

The browser path exists for reach, but it is **deliberately constrained for honesty**:

- **No raw UDP.** Browsers cannot open raw UDP sockets, so the per-datagram seq/timestamp
  stamping behind RFC 3550 jitter and RFC 7680 loss is impossible. UDP mode is disabled in
  the browser and clearly marked as requiring the client binary.
- **TCP is labeled "WebSocket throughput," not raw TCP.** It is TCP underneath (WS rides on
  TCP), but without `SO_SNDBUF` tuning or raw parallel streams — so it must never be
  presented as an `iperf3`-equivalent raw number.
- **File transfer is a true end-to-end measurement** over HTTP and needs no caveat.

| Mode | Client binary | Browser |
|---|---|---|
| TCP throughput | ✅ raw | ⚠️ WebSocket (labeled) |
| UDP loss/jitter | ✅ raw | ❌ unavailable |
| File transfer | ✅ | ✅ |

## v1 scope (decided)

v1 is the **client binary + three raw modes**, nothing more:

- **Modes:** raw TCP, UDP (loss/jitter), single-file transfer, plus the network-only vs.
  end-to-end isolation flag.
- **Reporting:** summary **and** per-second time series (`Result.samples[]`), in bits/s
  and bytes/s, with a live once-per-second readout and `--json`.
- **Mechanics:** JSON control serialization; per-stream `AtomicU64` byte counters sampled
  once/sec; UDP default packet 1472 bytes; `--server`/`--client` flag-style CLI.
- **Security:** plaintext, trusted LAN.

## Deferred to v2

- **Web dashboard** (the whole HTTP/WebSocket browser surface — [docs/web.md](docs/web.md)).
- Results aggregation across client/browser; TLS/auth (control + web); WebRTC UDP.
- File: multi-file/directory transfer; resume/range-restart.
- Config file / profiles.
- UDP: DF bit + path-MTU reporting; network-vs-kernel-drop loss attribution.
- TCP: RTT-based socket-buffer auto-tuning; per-stream (not just aggregate) time series.
- Control serialization swap to CBOR/`bincode`/`postcard` (framing already supports it).

## References

`andri`'s methodology is grounded in published IETF standards rather than ad-hoc
heuristics. Each reference below notes **how strongly** it applies — we distinguish
specs we implement *exactly* from ones we are *informed by*. None of these are claims of
formal conformance unless stated.

### Transport protocols (what we measure)

- **RFC 768** — *User Datagram Protocol* (Postel, Aug 1980).
  <https://www.rfc-editor.org/info/rfc768>
  The datagram format underneath UDP mode. *Foundational.*
- **RFC 9293** — *Transmission Control Protocol (TCP)* (Eddy, Ed., Aug 2022; obsoletes
  RFC 793). <https://www.rfc-editor.org/info/rfc9293>
  The transport underneath TCP and file modes. *Foundational.*
- **RFC 5681** — *TCP Congestion Control* (Allman, Paxson, Blanton, Sep 2009).
  <https://www.rfc-editor.org/info/rfc5681>
  Defines slow-start — the reason TCP mode uses a warm-up period whose bytes are
  excluded from the result. *Directly motivates a design choice.*
- **RFC 8085** — *UDP Usage Guidelines* (Eggert, Fairhurst, Shepherd, Mar 2017).
  <https://www.rfc-editor.org/info/rfc8085>
  Guidance on rate-limiting/pacing UDP senders — the basis for andri pacing bursts per
  timer tick instead of busy-spinning. *Informs implementation.*

### Benchmarking methodology (how we measure throughput & loss)

- **RFC 1242** — *Benchmarking Terminology for Network Interconnection Devices*
  (Bradner, Ed., Jul 1991). <https://www.rfc-editor.org/info/rfc1242>
  Definitions of throughput, frame loss, latency. *Terminology we follow.*
- **RFC 2544** — *Benchmarking Methodology for Network Interconnect Devices*
  (Bradner, McQuaid, Mar 1999). <https://www.rfc-editor.org/info/rfc2544>
  Standard throughput / frame-loss / duration methodology.
  *Informed by — note RFC 2544 targets lab testing of devices (routers/switches), not
  host-to-host LAN tests, so we borrow its conventions, not its exact procedure.*

### IP performance metrics (loss & jitter definitions)

- **RFC 2330** — *Framework for IP Performance Metrics* (Paxson, Almes, Mahdavi,
  Mathis, May 1998). <https://www.rfc-editor.org/info/rfc2330>
  The IPPM framework our loss/jitter metrics sit within. *Framework context.*
- **RFC 7680** — *A One-Way Loss Metric for IP Performance Metrics (IPPM)* (Almes,
  Kalidindi, Zekauskas, Morton, Ed., Jan 2016; obsoletes RFC 2680).
  <https://www.rfc-editor.org/info/rfc7680>
  Defines one-way packet loss — what UDP mode reports from sequence-number gaps.
  *Informs metric definition.*
- **RFC 3393** — *IP Packet Delay Variation Metric for IP Performance Metrics (IPPM)*
  (Demichelis, Chimento, Nov 2002). <https://www.rfc-editor.org/info/rfc3393>
  The IP-layer delay-variation (jitter) metric. *Informs metric definition.*
- **RFC 5481** — *Packet Delay Variation Applicability Statement* (Morton, Claise,
  Mar 2009). <https://www.rfc-editor.org/info/rfc5481>
  Clarifies which delay-variation formulation to use when. *Informs metric choice.*

### Jitter algorithm (what we implement exactly)

- **RFC 3550** — *RTP: A Transport Protocol for Real-Time Applications* (Schulzrinne,
  Casner, Frederick, Jacobson, Jul 2003). <https://www.rfc-editor.org/info/rfc3550>
  §6.4.1 specifies the interarrival jitter estimator. **UDP mode implements this
  formula exactly** — the one claim of precise conformance.

### TCP throughput testing

- **RFC 6349** — *Framework for TCP Throughput Testing* (Constantine, Forget, Geib,
  Schrage, Aug 2011). <https://www.rfc-editor.org/info/rfc6349>
  Methodology for measuring TCP throughput (warm-up, reporting, parallelism).
  *Informed by — it is a framework, not a conformance target; we approximate its
  setup rather than implement it fully.*

### Per-mode mapping

| Mode | Implements exactly | Informed by |
|---|---|---|
| **TCP** | — | RFC 9293, RFC 5681 (warm-up), RFC 6349, RFC 2544/1242 |
| **UDP** | RFC 3550 §6.4.1 (jitter) | RFC 768, RFC 8085 (pacing), RFC 7680 (loss), RFC 3393/5481, RFC 2544/1242 |
| **File** | — | RFC 9293, RFC 6349 |
