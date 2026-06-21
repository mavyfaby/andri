# andri — Testing

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> How andri is tested, how to run the tests, and what each existing test covers.
> This is the single index of every test in the project — when a module gains
> tests, add a subsection for it under [Tests by module](#tests-by-module).
> Status: **growing** — tests are added alongside each module as it lands.

## Contents

- [Running the tests](#running-the-tests)
- [Strategy](#strategy)
- [Why `Cursor` instead of a real socket](#why-cursor-instead-of-a-real-socket)
- [Tests by module](#tests-by-module)
  - [`src/proto.rs` — control protocol](#srcprotors--control-protocol)
  - [`src/cli.rs` — CLI parsing & formatting](#srcclirs--cli-parsing--formatting)
  - [`src/meter.rs` — metering & payload](#srcmeterrs--metering--payload)
- [What is intentionally *not* tested](#what-is-intentionally-not-tested)
- [Adding tests for a new module](#adding-tests-for-a-new-module)

## Running the tests

```sh
cargo test            # all tests
cargo test proto      # only the proto (control-protocol) tests
cargo test -- --nocapture   # show println! / dbg! output
```

Tests compile under the `test` profile and never ship in the release binary
(`#[cfg(test)]`). They do not require the network, a server, or any fixtures.

## Strategy

andri's tests are layered by how much machinery they touch:

1. **Pure unit tests** — serialization, framing, math (jitter, pacing, loss). Fast,
   deterministic, no I/O. This is where most coverage lives, because the
   measurement *correctness* (RFC 3550 jitter, RFC 7680 loss, pacing math) is pure
   computation and can be checked against known inputs.
2. **In-memory I/O tests** — exercise the async read/write paths against a fake
   stream (`std::io::Cursor`) instead of a real socket. The framing helpers
   `write_msg`/`read_msg` are generic over `AsyncWrite`/`AsyncRead`, so a `Cursor`
   stands in for a `TcpStream` with zero networking.
3. **Loopback integration tests** *(planned)* — start a server and client on
   `127.0.0.1` in one process and run a short real test end-to-end. These will live
   in `tests/` and assert on the exchanged `Result`, not on exact throughput
   (which is environment-dependent).

Async tests use `#[tokio::test]`, which spins up a Tokio runtime per test.

## Why `Cursor` instead of a real socket

A `Cursor<Vec<u8>>` is an in-memory buffer that implements the same async read/write
traits a `TcpStream` does. Writing pushes bytes into the `Vec`; wrapping that `Vec`
in a new `Cursor` and reading pulls them back out. This lets a framing round-trip be
tested with no ports, no `await` on real I/O, and fully deterministic timing — the
test is really asking *"if these exact bytes go out, do these exact bytes come back
as the same message?"*

## Tests by module

This section indexes every test in the project, grouped by the module it lives in.
Add a new subsection here whenever a module gains tests.

### `src/proto.rs` — control protocol

These verify the control-protocol wire format from
[docs/protocol.md](protocol.md) §2–§3.

| Test | What it does | Why it matters |
|---|---|---|
| `roundtrip_all_variants` | Builds one of **every** `Msg` variant (Hello, Welcome, Negotiate, Start, Run, Stop, Result, Error), writes it with `write_msg`, reads it back with `read_msg`, and asserts the decoded value equals the original. | Catches any field that fails to serialize/deserialize and any framing bug that corrupts a message. Comparison is done via `serde_json::Value` because `Msg` intentionally doesn't derive `PartialEq`. |
| `frame_has_be_length_prefix` | Frames a `Msg::Run`, then checks the first 4 bytes decode (big-endian) to exactly the payload length, and that the payload is the literal JSON `{"type":"Run"}`. | Locks the §2 wire contract: a 4-byte **big-endian** length prefix + JSON body. A future refactor that changed byte order or framing would fail here. |
| `oversized_declared_length_is_rejected` | Hand-crafts a frame whose declared length is `MAX_FRAME_BYTES + 1`, then calls `read_msg`. | Confirms the abuse guard (§2): an oversized declared length is rejected with `InvalidData` **before** the body is read, so a malicious peer can't make us allocate a huge buffer. |
| `enum_wire_tokens` | Serializes `Mode`, `RoleDir`, and `ProtoError` and asserts the exact JSON tokens (e.g. `"tcp"`, `"send"`, `"data_connect_failed"`). | Pins the on-wire spelling of enums. Renaming a Rust variant without thinking would silently change the protocol; this test makes that change loud. |

### `src/cli.rs` — CLI parsing & formatting

| Test | What it does | Why it matters |
|---|---|---|
| `format_render_units` | Renders `1e9` bits/s in all four `--format` units and asserts the exact strings (`1000000000 bit/s`, `125000000 byte/s`, `1000.00 Mbps`, `1.00 Gbps`). | Locks the unit conversions a user reads — a wrong divisor would misreport throughput. |
| `mode_flags_selected` | Resolves `ModeFlags` for each flag combination, asserting none → `None`, and each flag → its `Mode`. | Guards the **explicit-mode** rule: there is no default, so absence must yield `None` (the client then errors). |
| `mode_display_is_uppercase` | Asserts `Mode` displays as `TCP`/`UDP`/`FILE`. | Pins the banner casing (not Rust's `Debug` `Tcp`/`Udp`/`File`). |
| `cli_definition_is_valid` | Runs clap's `Command::debug_assert()`. | Catches arg/group misconfiguration (conflicts, duplicate flags) at test time instead of first run. |

### `src/meter.rs` — metering & payload

| Test | What it does | Why it matters |
|---|---|---|
| `bits_per_sec_basic` | Checks `bits_per_sec` (e.g. 125 MB/s → 1 Gbit/s). | The core throughput conversion every result depends on. |
| `bits_per_sec_guards_zero_duration` | Passes 0.0 and negative durations. | Prevents divide-by-zero / NaN in the result when a window is degenerate. |
| `payload_preview_format` | Checks the hex sample + distinct-byte count, and that all-zeros reads as `1/256 distinct`. | Confirms the `--verbose` payload check can actually distinguish incompressible random data from zeros. |
| `fill_random_is_deterministic` | Same seed → identical bytes. | Reproducible/verifiable payload (seeded from `server_seed`). |
| `fill_random_varies_by_seed` | Different seeds → different bytes. | Parallel streams don't send byte-identical buffers. |
| `fill_random_is_high_entropy` | A 4 KiB fill hits >200 distinct byte values. | The payload is incompressible (won't be inflated by compressing links). |
| `fill_random_handles_remainder` | Fills a non-multiple-of-8 length. | The 8-byte-chunked fill must cover the tail bytes. |

## What is intentionally *not* tested here

- **Absolute throughput numbers** — they depend on the machine/network and aren't a
  correctness property. Integration tests assert structure and sanity bounds, not
  Gbit/s figures.
- **The deferred web dashboard** — not part of v1 (see [docs/web.md](web.md)).

## Adding tests for a new module

Put fast, pure tests in a `#[cfg(test)] mod tests` block at the bottom of the module
(as in `proto.rs`). Reserve `tests/` (integration) for things that need a running
server + client. Prefer asserting on **decoded values and invariants** over on
golden byte strings, except where the byte layout *is* the contract (framing,
datagram layout) — there, pin the bytes.

When you add tests, also:

1. Add (or extend) the module's subsection under
   [Tests by module](#tests-by-module) with a row per test.
2. Add the subsection to the [Contents](#contents) list so the index stays complete.
