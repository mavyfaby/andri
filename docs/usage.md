# andri — Usage Guide

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> A user-facing guide to the CLI **as it works today**. For the full intended design
> surface see [cli.md](cli.md); for protocol/mode internals see the other docs.
>
> **Implemented now:** TCP throughput, UDP throughput/loss/jitter.
> **Not yet:** file transfer (`--file` is stubbed), `--bidir`, the browser dashboard (v2).

## Quick start

andri runs `iperf3`-style: a **server** on one host, a **client** on another. Both run the
same binary. (On one machine for a quick test, use `127.0.0.1`.)

```sh
# On the receiving host:
andri --server

# On the sending host — pick a mode (required):
andri --client <server-ip> --tcp
andri --client <server-ip> --udp --bitrate 1G
```

## Choosing a role

Exactly one is required:

| Flag | Meaning |
|---|---|
| `--server` | Run as the receiver. Accepts tests; learns the mode from each client. Takes no mode flag. |
| `--client <HOST>` | Run as the sender, connecting to `<HOST>`. Requires a mode. |

## Choosing a mode (client)

Exactly one mode is **required** — there is no default.

| Flag | Status | What it measures |
|---|---|---|
| `--tcp` | ✅ works | Raw TCP throughput (the wire's max, no rate cap). |
| `--udp` | ✅ works | UDP throughput + packet loss + jitter at a target rate. **Requires `--bitrate`.** |
| `--file <PATH>` | ⛔ not implemented yet | Real file-transfer speed (planned). |

**Why `--udp` needs `--bitrate`:** UDP has no congestion control, so it sends at exactly
the rate you choose — that offered load is the input to the experiment, not a cap. TCP
needs no rate; it discovers the maximum itself. See [udp.md](udp.md).

## Options

| Flag | Default | Applies to | Description |
|---|---|---|---|
| `-d, --duration <secs>` | `10` | client | Measurement window length (warm-up is added then excluded). |
| `-P, --parallel <n>` | `1` | client (TCP) | Concurrent data streams. |
| `-b, --bitrate <rate>` | *required for UDP* | client (UDP) | Target send rate: `1G`, `500M`, `10M`, `64K`, or a bare number (bits/s). |
| `--packet <bytes>` | `1472` | client (UDP) | Datagram payload size (1472 avoids 1500-MTU fragmentation). |
| `--format <unit>` | `mbps` | both | Output unit: `bits`, `bytes`, `mbps`, `gbps`. Each role formats its own readout. |
| `--json` | off | client | Emit machine-readable JSON to stdout (suppresses the banner/progress). |
| `-p, --port <port>` | `5201` | both | Control port (server binds; client dials). |
| `-v, --verbose` | off | client | Show a sample of the (random) payload, for debugging. |
| `--bidir` | off | — | Parsed but **not implemented** yet. |

### Rate & size units

- **Bitrate** (`--bitrate`) is **decimal** (networking convention): `K`=10³, `M`=10⁶,
  `G`=10⁹ bits/s. `1G` = 1,000,000,000 bits/s.
- **Duration** is in seconds; **packet** is in bytes.

## Reading the output

A run prints a config banner, a live once-per-second readout, then a summary.

**TCP:**
```
andri 0.1.0 — client
  target     192.168.1.10:5201
  mode       TCP
  duration   10s (+ 1s warm-up, excluded)
  streams    1
  ...
[  1.0s] 942.10 Mbps  (warm-up, excluded)
[  2.0s] 941.80 Mbps
...
TCP throughput: 941.92 Mbps over 10.0s, 1 stream(s)
```

**UDP:**
```
  mode       UDP
  bitrate    100.00 Mbps
  packet     1472 bytes
...
[  2.0s] 100.00 Mbps sent | 8492 pkts
...
UDP: 99.98 Mbps over 10.0s | loss 12/84918 (0.01%) | jitter 0.042 ms
```

Notes:
- The **warm-up second** is sent but excluded from the result (TCP slow-start); it's
  labeled in the live readout, which is why you see one more line than `--duration`.
- The client's live lines are **send-side**; the final summary is the **server's
  authoritative received-side** numbers (loss/jitter come from the receiver).
- The live readout appears on whichever host runs that side (server prints received
  stats; client prints sent stats).

## JSON output (`--json`)

For scripting. One object matching the result schema ([protocol.md](protocol.md) §3.6),
with a per-second `samples[]` time series:

```json
{
  "role": "receive",
  "bytes": 6250112,
  "duration_secs": 0.998,
  "bits_per_sec": 50088544.69,
  "bytes_per_sec": 6261068.08,
  "packets_expected": 4246,
  "packets_received": 4246,
  "packets_lost": 0,
  "loss_ratio": 0.0,
  "jitter_ms": 0.0069,
  "samples": [ { "t_secs": 1.0, "bytes": 6250112, "bits_per_sec": 50088544.69, "packets_lost": 0, "jitter_ms": 0.0069 } ]
}
```

TCP results leave the UDP fields (`packets_*`, `loss_ratio`, `jitter_ms`) `null`.

## Examples

```sh
# Quick gigabit LAN check
andri --client 192.168.1.10 --tcp

# 4 parallel TCP streams for 30s, results in Gbps
andri --client 192.168.1.10 --tcp -P 4 -d 30 --format gbps

# UDP at 1 Gbit/s — watch loss and jitter
andri --client 192.168.1.10 --udp --bitrate 1G

# Simulate a video stream's UDP load
andri --client 192.168.1.10 --udp --bitrate 5M

# Find clean UDP capacity: sweep the rate up until loss climbs
andri --client 192.168.1.10 --udp --bitrate 500M
andri --client 192.168.1.10 --udp --bitrate 900M
andri --client 192.168.1.10 --udp --bitrate 950M

# Machine-readable, on a non-default port
andri --client 192.168.1.10 --tcp -p 9000 --json
```

## Notes & caveats

- **Loopback numbers are not network speeds.** Running client and server on the same host
  measures memory/kernel-copy speed (often 100+ Gbit/s for TCP), not a link.
- **UDP loss at high rates** can be inflated by the receiver's kernel buffer overflowing
  rather than real network loss — andri leaves `SO_RCVBUF` at the OS default in v1. See
  [udp.md](udp.md) §6 for how to raise it.
- This guide tracks the current build; flags marked not-implemented are parsed but inert.
