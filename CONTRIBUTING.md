# Contributing to andri

Thanks for your interest in andri. Bug reports, fixes, and well-scoped features are all
welcome. This guide covers how the project is set up and what's expected of a change.

## Ground rules

- **Be honest about measurements.** andri is a benchmarking tool — its value is trustworthy
  numbers. Don't add anything that could inflate or misrepresent a result (e.g. compressible
  payloads, counting warm-up bytes, presenting an approximation as a raw measurement). When a
  number has a caveat, surface it.
- **Match the surrounding code.** Same idioms, comment density, and naming as the file you're
  editing.
- **No `unsafe` in v1.** andri deliberately uses safe `std` only. Anything needing `unsafe`
  (e.g. `setsockopt` for `SO_RCVBUF`) is deferred — see the roadmap in
  [DESIGN.md](DESIGN.md).

## Getting started

```sh
git clone https://github.com/mavyfaby/andri
cd andri
cargo build
```

andri uses [`just`](https://github.com/casey/just) for dev workflows (`brew install just`).
Run `just` to list recipes.

| Command | What it does |
| --- | --- |
| `just check` | Format check + clippy (deny warnings) + tests — the pre-commit gate |
| `just test` | Run the test suite |
| `just fmt` | Apply formatting |
| `just smoke` | Loopback end-to-end test of all three modes |
| `just server` / `just client <host>` | Run either role |
| `just release` | Optimized build |

Plain `cargo` works too — `just` just bundles the common sequences.

## Before you open a PR

Your change must pass the same gate CI enforces:

```sh
just check        # == cargo fmt --check + cargo clippy -D warnings + cargo test
```

CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs this on every push and PR;
warnings fail the build, so keep it clean.

## Tests

- Put fast, pure unit tests in a `#[cfg(test)] mod tests` block at the bottom of the module
  (see `src/proto.rs`, `src/meter.rs`).
- Prefer asserting on **decoded values and invariants**, except where a byte layout *is* the
  contract (framing, datagram layout) — there, pin the bytes.
- Don't assert on absolute throughput numbers; they're environment-dependent.
- Update [docs/testing.md](docs/testing.md) — it's the project-wide test index. Add a row for
  each new test, and list new modules in its Contents.

See [docs/testing.md](docs/testing.md) for the full strategy.

## Design docs

andri's design is documented and should stay in sync with the code:

- [DESIGN.md](DESIGN.md) — architecture, decisions, and the roadmap (source of truth).
- [docs/protocol.md](docs/protocol.md) — control protocol & wire format.
- [docs/tcp.md](docs/tcp.md), [docs/udp.md](docs/udp.md), [docs/file.md](docs/file.md) —
  per-mode design.
- [docs/cli.md](docs/cli.md) / [docs/usage.md](docs/usage.md) — CLI reference / user guide.

If your change alters behavior, a protocol field, or a flag, update the relevant doc in the
same PR. The methodology is grounded in IETF standards (RFC 3550 jitter, RFC 7680 loss, …);
keep the "implemented exactly" vs. "informed by" distinction honest.

## Scope

andri's v1 is intentionally focused: three raw modes (TCP, UDP, file) via the client binary,
on a trusted LAN. A number of things are explicitly deferred to v2+ (web dashboard, TUI,
TLS, `SO_RCVBUF` tuning, multi-file transfer, …) — see the roadmap in
[DESIGN.md](DESIGN.md#roadmap-deferred). If you want to work on one of those, open an issue
first so we can align on approach.

## Commit & PR conventions

- Use clear, conventional-style commit subjects where it fits: `feat(udp): …`, `fix: …`,
  `docs: …`, `test: …`, `ci: …`, `chore: …`.
- Keep PRs focused — one logical change per PR is easier to review.
- Describe *what* and *why*, and note anything you couldn't test (e.g. "only tested on
  loopback / macOS").

## Reporting bugs

Open an issue with: what you ran (the exact command), what you expected, what happened, and
your OS/arch. For measurement weirdness, include whether it was loopback or a real link and
the rate involved — a lot of "loss" at high UDP rates is the receiver's kernel buffer, not the
network (see [docs/udp.md](docs/udp.md#6-edge-cases--decisions)).

## License

By contributing, you agree your contributions are licensed under the project's
[Apache-2.0](LICENSE) license.
