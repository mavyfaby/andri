# andri — Real-network benchmarking & iperf3 validation

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> How to run andri on a **real LAN** (not loopback) and validate its TCP number against
> `iperf3` on the same link. Loopback figures measure memory/kernel-copy speed, not a
> network — these steps produce numbers that actually mean something.

## Why this matters

Every figure in the demo is loopback (`127.0.0.1`) — it reflects the test machine's
memory bandwidth, not a network link. To trust andri (and to compare it fairly to
`iperf3`), you need **two machines on the same physical network**. This doc is the
procedure; the goal is for andri's raw-TCP throughput to land within a few percent of
`iperf3` on the same link. If it does, the measurement is sound. If it doesn't, that's a
bug worth filing.

## Setup

You need:
- **Two machines** on the same LAN (wired is best — Wi-Fi adds variance). Note each one's
  IP (`ip addr` / `ifconfig`).
- **andri** on both (`cargo install andri`, or copy the release binary).
- **iperf3** on both (`brew install iperf3`, `apt install iperf3`, …), for the comparison.
- Know the **link speed** (1 GbE, 2.5 GbE, 10 GbE, Wi-Fi). It's the ceiling you're checking
  against — e.g. 1 GbE tops out near ~940 Mbit/s of real TCP payload.

Throughout: **Server = machine A**, **Client = machine B**. Replace `A.B.C.D` with
machine A's LAN IP.

## 1. andri — TCP throughput

```sh
# Machine A (server)
andri --server

# Machine B (client)
andri --client A.B.C.D --tcp -d 10 --format mbps
```

Run it a few times; take the steady-state number (warm-up is already excluded). For a
1 GbE link expect roughly 900–950 Mbit/s.

Try parallel streams too — a single TCP flow doesn't always saturate a link:

```sh
andri --client A.B.C.D --tcp -d 10 -P 4 --format mbps
```

## 2. iperf3 — the reference

```sh
# Machine A
iperf3 -s

# Machine B
iperf3 -c A.B.C.D -t 10
```

Note iperf3's `receiver` number — that's the apples-to-apples figure (andri reports the
server's authoritative received bytes too).

## 3. Compare

Fill in a table like this (this is the template for the README once you have real numbers):

| Link | andri TCP | iperf3 TCP | Δ |
|---|---|---|---|
| 1 GbE (wired) | _e.g._ 938 Mbit/s | _e.g._ 941 Mbit/s | −0.3% |
| Wi-Fi 6 | … | … | … |

Within a few percent → andri's TCP path is sound. A large gap → investigate (socket
buffers, single-stream vs. parallel, CPU bound on one end).

## 4. andri — UDP loss & jitter (no iperf3 equivalent needed)

```sh
# Machine B — sweep the rate up toward the link ceiling and watch where loss begins
andri --client A.B.C.D --udp --bitrate 500M -d 10
andri --client A.B.C.D --udp --bitrate 900M -d 10
andri --client A.B.C.D --udp --bitrate 950M -d 10
```

On a real LAN at a sustainable rate, loss should be near zero and jitter sub-millisecond.
Loss that climbs only past the link's capacity is correct behavior. **Caveat:** loss at
very high rates can be the receiver's kernel buffer overflowing rather than the network —
raise `SO_RCVBUF` via the OS first (see [docs/udp.md](udp.md#6-edge-cases--decisions)).

## 5. andri — file transfer (the differentiator)

```sh
# Machine B — real file vs. network-only, same file
andri --client A.B.C.D --file ./bigfile.bin --format mbps
andri --client A.B.C.D --file ./bigfile.bin --null-source --format mbps
```

The gap between the two is your disk: if `--null-source` is much faster than the real-file
run, the disk (read side) is your bottleneck, not the wire.

## Reporting

When you publish numbers, always state: **link type/speed, wired vs Wi-Fi, both machines'
OS/CPU, and andri + iperf3 versions.** Reproducibility is the point — a number without its
conditions isn't a measurement.
