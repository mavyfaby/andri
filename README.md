# andri

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Standards-based](https://img.shields.io/badge/standards-RFC%203550%20%7C%206349%20%7C%202544-informational.svg)](DESIGN.md#references)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org)

**Fast, all-in-one LAN speed tester.** Measure TCP throughput, UDP loss/jitter, and real file-transfer speeds — one standards-based, self-contained Rust binary.

![andri demo](docs/demo.gif)

> Demo on Apple M5 (10-core), 16 GB, macOS 26.5.1 (arm64). Numbers are **loopback**
> (`127.0.0.1`) — this machine's memory/kernel-copy speed, **not** a network link. On a
> real LAN, andri reports your actual throughput (e.g. ~0.94 Gbit/s on gigabit Ethernet).

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

## Why andri

Most LAN benchmarks measure one thing. `iperf3` gives you raw socket throughput; file-copy tools give you end-to-end speed; few give you UDP loss and jitter without ceremony. `andri` puts all three in a single binary and lets you isolate **network-only** performance from **end-to-end file copy** with a flag — so you can tell whether a slow transfer is the wire or the disk.

- **TCP throughput** — raw stream bandwidth, with parallel streams and configurable socket buffers.
- **UDP throughput** — paced sending with per-packet loss and jitter (RFC 3550 §6.4.1).
- **File transfer** — real file read from disk → streamed → written on the far end, with a flag to source from memory and isolate the network from disk I/O.

andri's measurement methodology follows established IETF standards rather than ad-hoc heuristics — see [Standards & Methodology](#standards--methodology).

### How it compares

`andri` doesn't aim to dethrone `iperf3` on raw throughput — `iperf3` is battle-tested and
highly tuned. andri's point is **breadth in one tool** and the **network-vs-disk
isolation** no single tool gives you:

| | andri | iperf3 | LAN file copy |
|---|:---:|:---:|:---:|
| TCP throughput | ✅ | ✅ | — |
| UDP loss + jitter | ✅ | ✅ | — |
| **Real file-transfer speed** | ✅ | — | ✅ |
| **Network-only vs. end-to-end isolation** (`--null-source`) | ✅ | — | — |
| Single self-contained binary | ✅ | ✅ | n/a |
| Standards-grounded (RFC 3550 jitter, etc.) | ✅ | ✅ | — |

The differentiator is the bottom-left quadrant: andri measures the *real* file copy **and**
the network-only ceiling, so the gap between them tells you whether the disk or the wire is
your bottleneck — in one run, one tool.

## Install

From crates.io (recommended — no Gatekeeper prompts, builds locally):

```sh
cargo install andri
```

Or grab a prebuilt binary for your platform from the
[latest release](https://github.com/mavyfaby/andri/releases/latest)
(Linux x64/ARM64, macOS x64/ARM64, Windows x64).

Or build from source:

```sh
git clone https://github.com/mavyfaby/andri
cd andri
cargo build --release
# binary at target/release/andri
```

### macOS: "Apple could not verify…"

Prebuilt binaries aren't notarized, so macOS Gatekeeper blocks them on first run.
Either install via `cargo install andri` (locally built, no prompt), or clear the
quarantine flag on the downloaded binary:

```sh
xattr -d com.apple.quarantine ./andri
```

## Usage

`andri` runs `iperf3`-style: start a server on one host, point a client at it.

```sh
# On the receiving host:
andri --server

# On another machine with andri installed:
andri --client 192.168.1.10
```

> A zero-install **browser dashboard** (served by the same binary) is planned for v2 —
> see [docs/web.md](docs/web.md). v1 has no browser client; you run the `andri` binary on
> both ends (`--server` and `--client`).

### Modes

```sh
andri --client 192.168.1.10 --tcp                 # raw TCP throughput
andri --client 192.168.1.10 --udp --bitrate 1G    # UDP throughput + loss/jitter
andri --client 192.168.1.10 --file ./big.iso      # real file transfer
andri --client 192.168.1.10 --file ./big.iso --null-source  # network-only (skip disk read)
```

### Common flags (planned)

| Flag | Default | Description |
|---|---|---|
| `--duration <secs>` | `10` | Test length. |
| `--parallel <n>` | `1` | Number of concurrent data streams. |
| `--bitrate <rate>` | — | Target send rate for UDP (e.g. `1G`, `500M`). |
| `--buffer <bytes>` | `64KiB` | Per-stream buffer size. |
| `--bidir` | off | Send and receive simultaneously. |
| `--null-source` | off | File mode: stream from memory instead of disk. |

Throughput is reported in both bits/s and bytes/s, with a live once-per-second readout during the run.

**See [docs/usage.md](docs/usage.md)** for the full usage guide — every flag, how to read the output, JSON format, and what's implemented today (all three modes: TCP, UDP, file).

## Standards & Methodology

andri is built on published IETF standards so results are defensible and comparable, not invented. Full citations and the precise per-mode mapping live in [DESIGN.md](DESIGN.md#references); the short version:

- **UDP jitter** is computed **exactly per [RFC 3550](https://www.rfc-editor.org/info/rfc3550) §6.4.1** (the RTP interarrival jitter estimator).
- **UDP packet loss** follows the one-way loss metric of [RFC 7680](https://www.rfc-editor.org/info/rfc7680) (IPPM), within the [RFC 2330](https://www.rfc-editor.org/info/rfc2330) framework; delay-variation definitions track [RFC 3393](https://www.rfc-editor.org/info/rfc3393) / [RFC 5481](https://www.rfc-editor.org/info/rfc5481), and sender pacing follows [RFC 8085](https://www.rfc-editor.org/info/rfc8085).
- **TCP throughput** is informed by [RFC 6349](https://www.rfc-editor.org/info/rfc6349); the warm-up period exists because of TCP slow-start ([RFC 5681](https://www.rfc-editor.org/info/rfc5681)).
- **Throughput & loss terminology/methodology** follow [RFC 1242](https://www.rfc-editor.org/info/rfc1242) and [RFC 2544](https://www.rfc-editor.org/info/rfc2544).
- Transports: TCP ([RFC 9293](https://www.rfc-editor.org/info/rfc9293)), UDP ([RFC 768](https://www.rfc-editor.org/info/rfc768)).

> We distinguish what we implement *exactly* (RFC 3550 jitter) from what we are *informed by* (RFC 2544 and 6349 are methodologies/frameworks, not conformance targets — RFC 2544 in particular targets lab device testing, not host-to-host LAN tests). See DESIGN.md for the honest per-claim strength.

## Project structure

```
andri/
├── Cargo.toml            # crate metadata, deps, release profile
├── README.md             # this file
├── DESIGN.md             # architecture & decisions (source of truth)
├── LICENSE               # Apache-2.0
├── src/
│   ├── main.rs           # entry point; parse args, dispatch server/client
│   ├── cli.rs            # clap CLI (--server/--client, mode & format flags)
│   ├── proto.rs          # control-protocol messages + length-delimited framing
│   ├── session.rs        # shared control handshake (both roles)
│   ├── meter.rs          # byte counters, per-second sampler, payload fill
│   └── modes/            # per-mode data paths
│       ├── tcp.rs        #   TCP throughput (implemented)
│       ├── udp.rs        #   UDP loss/jitter (implemented)
│       └── file.rs       #   file transfer + --null-source (implemented)
└── docs/                 # detailed design specs (RFC-grounded)
    ├── protocol.md       #   control protocol & wire format
    ├── cli.md            #   CLI reference
    ├── tcp.md            #   TCP mode design
    ├── udp.md            #   UDP mode design
    ├── file.md           #   file-transfer mode design
    ├── web.md            #   browser dashboard (deferred to v2)
    └── testing.md        #   test strategy & project-wide test index
```

Shared, mode-agnostic code (`proto`, `session`, `meter`, `cli`) lives at the top of
`src/`; only the per-mode data movement lives under `modes/`.

## Testing

```sh
cargo test            # run all unit tests
cargo test cli        # filter to one module's tests (e.g. cli, proto, meter)
cargo test -- --nocapture   # show println!/dbg! output
cargo clippy --all-targets  # lint
cargo fmt --check           # formatting check
```

Tests are pure and need no network — the control framing is exercised against an
in-memory stream rather than a socket. See [docs/testing.md](docs/testing.md) for the
test strategy and a per-module index of every test and what it guards.

## Contributing

Bug reports, fixes, and well-scoped features are welcome. Run `just check` before opening a
PR, keep design docs in sync with behavior changes, and be honest about measurements. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the full guide and [DESIGN.md](DESIGN.md) for the
architecture and roadmap.

## Author

[mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

## Acknowledgements

Design docs drafted with assistance from [Claude](https://www.anthropic.com/claude); all
architecture and design decisions are the author's.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
