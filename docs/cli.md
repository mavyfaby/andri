# andri — CLI Reference

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> Detailed design for the command-line interface (clap, derive). See
> [DESIGN.md](../DESIGN.md) for the overview. Status: **draft** — flag names are not yet
> locked; this is the intended surface.

The key words **MUST**, **SHOULD**, **MAY** are interpreted per
[RFC 2119](https://www.rfc-editor.org/info/rfc2119) /
[RFC 8174](https://www.rfc-editor.org/info/rfc8174).

`andri` is one binary with two roles: **server** and **client**. (A browser dashboard
([docs/web.md](web.md)) is deferred to v2; v1 has no web interface.)

## 1. Top-level shape

```
andri --server [options]
andri --client <host> [mode] [options]
```

- `--server` and `--client <host>` are mutually exclusive and one **MUST** be present.
- `<host>` is an IP or hostname; default control port is `5201`
  ([protocol.md](protocol.md) §1).

## 2. Global options

| Flag | Default | Applies to | Description |
|---|---|---|---|
| `-p, --port <port>` | `5201` | both | Control port (server binds; client dials). |
| `-d, --duration <secs>` | `10` | client | Measurement window length. |
| `-P, --parallel <n>` | `1` | client | Number of concurrent data streams. |
| `--bind <addr>` | `0.0.0.0` | server | Address to bind listeners to. |
| `--bidir` | off | client | Send and receive simultaneously. |
| `--format <bits\|bytes\|both>` | `both` | client | Units in the summary. |
| `--json` | off | both | Emit machine-readable JSON results to stdout. |
| `-v, --verbose` | off | both | Verbose logging. |
| `-V, --version` | — | both | Print version and exit. |
| `-h, --help` | — | both | Print help and exit. |

## 3. Mode selection (client)

Exactly one mode is **required** — there is no default. The client errors if none
of `--tcp` / `--udp` / `--file <path>` is given.

| Flag | Mode doc | Description |
|---|---|---|
| `--tcp` | [tcp.md](tcp.md) | Raw TCP throughput. |
| `--udp` | [udp.md](udp.md) | UDP throughput + loss/jitter. |
| `--file <path>` | [file.md](file.md) | Real file transfer from `<path>`. |

### TCP options

| Flag | Default | Description |
|---|---|---|
| `--buffer <bytes>` | `65536` (64 KiB) | Application read/write buffer per stream. |
| `--warmup <secs>` | `1` | Bytes before the window, excluded from the result. |
| `--sndbuf <bytes>` | OS default | `SO_SNDBUF` socket buffer. |
| `--rcvbuf <bytes>` | OS default | `SO_RCVBUF` socket buffer. |
| `--no-nodelay` | off | Leave Nagle on (TCP_NODELAY off). |

### UDP options

| Flag | Default | Description |
|---|---|---|
| `-b, --bitrate <rate>` | `1G` | Target send rate, e.g. `1G`, `500M`, `10M`. |
| `--packet <bytes>` | `1472` | Datagram payload size (avoids 1500-MTU fragmentation). |
| `--warmup <secs>` | `1` | Packets before the window, excluded. |

### File options

| Flag | Default | Description |
|---|---|---|
| `--null-source` | off | Generate bytes in memory instead of reading disk. |
| `--no-write` | on with `--null-source` | Receiver discards instead of writing to disk. |
| `--fsync` | off | Force durable write per chunk (includes flush cost). |
| `--verify` | off | Checksum the transfer (timed and reported separately). |

## 4. Rate & size syntax

- **Bitrates** (`--bitrate`) use SI/decimal suffixes on bits: `K`=10³, `M`=10⁶, `G`=10⁹.
  So `1G` = 1,000,000,000 bits/s. This matches networking convention (link speeds are
  decimal bits).
- **Byte sizes** (`--buffer`, `--packet`, `--sndbuf`) use IEC/binary suffixes: `KiB`=1024,
  `MiB`=1024². A bare number is bytes.
- The two suffix systems are deliberately different because the domains differ — bits/s
  are decimal by convention, memory buffers are binary. The CLI **MUST** document this on
  `--help` to avoid the classic ambiguity.

## 5. Output

- Default: human-readable summary to stdout, with a live once-per-second readout during
  the run (to stderr, so `--json` stdout stays clean).
- `--json`: a single JSON object matching the `Result` schema
  ([protocol.md](protocol.md) §3.6) to stdout, for scripting. For `bidir`, an object per
  direction.
- Exit code `0` on a completed measurement, non-zero on protocol/connection error.

## 6. Examples

```sh
# Server (v1: control + data listeners only; browser dashboard is a v2 feature)
andri --server

# Default 10s raw TCP throughput
andri --client 192.168.1.10

# 4 parallel TCP streams, 30s, machine-readable
andri --client 192.168.1.10 --tcp -P 4 -d 30 --json

# UDP at 1 Gbit/s, report loss and jitter
andri --client 192.168.1.10 --udp -b 1G

# Real file transfer
andri --client 192.168.1.10 --file ./ubuntu.iso

# Network-only file path (no disk on either side)
andri --client 192.168.1.10 --file ./ubuntu.iso --null-source

# Bidirectional TCP
andri --client 192.168.1.10 --tcp --bidir
```

## 7. Decisions & deferrals

**v1 (decided):**
- **Flag style** (`--server` / `--client <ip>`), to match `iperf3` muscle memory — not
  subcommands.

**Deferred to v2:**
- A config file / profiles for repeated test setups.

## References

CLI conventions are tool-design choices, not RFC-governed; the only standards references
are the unit conventions and the protocol/mode docs linked above. Requirement keywords
follow [RFC 2119](https://www.rfc-editor.org/info/rfc2119) /
[RFC 8174](https://www.rfc-editor.org/info/rfc8174).
