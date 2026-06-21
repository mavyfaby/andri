# andri — TCP Mode (throughput)

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> Detailed design for the raw TCP throughput mode. See [DESIGN.md](../DESIGN.md) for the
> overview and master [References](../DESIGN.md#references); session setup is in
> [docs/protocol.md](protocol.md). Status: **draft**.

The key words **MUST**, **SHOULD**, **MAY** are interpreted per
[RFC 2119](https://www.rfc-editor.org/info/rfc2119) /
[RFC 8174](https://www.rfc-editor.org/info/rfc8174).

TCP mode measures raw stream throughput over one or more `TcpStream` connections
([RFC 9293](https://www.rfc-editor.org/info/rfc9293)). It is the highest-fidelity
throughput number andri produces — a real raw socket, tunable buffers, parallel streams.
(The browser dashboard's WebSocket-based TCP is a separate, explicitly-labeled
approximation; see [docs/web.md](web.md) §2.)

## 1. Roles & connections

- After `Start` ([protocol.md](protocol.md) §3.4), the client opens `parallel` data
  connections to the server's ephemeral `data_port`.
- Each connection is one `tokio` task. The sender writes; the receiver reads and counts.
- Direction follows the session: client sends by default; `bidir` runs both directions
  simultaneously, each direction with its own set of streams and counters.

## 2. The hot loop

- **Reuse a single buffer** per stream — no per-iteration allocation. Default
  `buffer_bytes` = 64 KiB (65536).
- The payload content is irrelevant to throughput; the sender **MAY** fill from a
  cheap pattern (or the `server_seed`) and the receiver discards after counting.
- Byte counts are reported via an `AtomicU64` per stream, sampled once per second by the
  control side for the live readout (see DESIGN.md topology).

## 3. Warm-up & TCP slow-start

TCP begins each connection in **slow-start**
([RFC 5681](https://www.rfc-editor.org/info/rfc5681)): the congestion window ramps from
small, so early throughput understates the steady-state rate.

- andri sends for `warmup_secs` (default 1) **before** the measurement window and
  **excludes** those bytes from the result. This is the direct reason the warm-up exists.
- The measurement window is the subsequent `duration_secs` (default 10) — long enough to
  average out transients, per the methodology spirit of
  [RFC 6349](https://www.rfc-editor.org/info/rfc6349) and
  [RFC 2544](https://www.rfc-editor.org/info/rfc2544).
- Warm-up boundaries are time-based per stream, keyed off the monotonic clock.

## 4. Socket tuning

- `SO_SNDBUF` / `SO_RCVBUF` are configurable; larger buffers help fill the
  bandwidth-delay product on higher-latency links. andri exposes a `--buffer`-adjacent
  socket-buffer knob (distinct from the application read/write `buffer_bytes`).
- `TCP_NODELAY` **SHOULD** be set on throughput streams to avoid Nagle batching skewing
  small-write timing; for bulk throughput with 64 KiB writes its effect is minor but the
  setting is made explicit.
- Parallel streams exist precisely because a single TCP flow may not saturate a link
  (congestion control, single-core bottlenecks); aggregate throughput is the sum across
  streams over the common window.

## 5. Timing & reporting

- Monotonic clock only (`std::time::Instant`), never `SystemTime`.
- Throughput = in-window bytes ÷ in-window duration, summed across streams, reported in
  both bits/s and bytes/s ([protocol.md](protocol.md) §3.6).
- Per-stream and aggregate figures are both available; the summary reports aggregate.

## 6. Edge cases & decisions

- **Uneven streams**: streams may finish their warm-up at slightly different instants;
  each excludes its own warm-up independently, then all share the aggregate measurement
  window opened by `Run`.
- **Connection setup cost** (3-way handshake) happens before `Run`, so it never counts
  against throughput.
- **A stream dies mid-test**: the test reports partial aggregate and flags the degraded
  stream count rather than aborting, unless all streams drop.

## 7. Open questions

- Expose per-second per-stream time series, or only aggregate summary?
- Whether to auto-tune socket buffers from a measured RTT, or leave fully manual.

## References

See the master list in [DESIGN.md](../DESIGN.md#references). Load-bearing here:

- **[RFC 9293](https://www.rfc-editor.org/info/rfc9293)** — Transmission Control Protocol.
  *The transport.*
- **[RFC 5681](https://www.rfc-editor.org/info/rfc5681)** — TCP Congestion Control.
  *Slow-start — the reason for warm-up.*
- **[RFC 6349](https://www.rfc-editor.org/info/rfc6349)** — Framework for TCP Throughput
  Testing. *Informs methodology (warm-up, reporting, parallelism).*
- **[RFC 2544](https://www.rfc-editor.org/info/rfc2544)** / **[RFC 1242](https://www.rfc-editor.org/info/rfc1242)**
  — Benchmarking methodology & terminology. *Informs duration/throughput conventions.*
