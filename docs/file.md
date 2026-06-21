# andri — File-Transfer Mode

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> Detailed design for the file-transfer mode. See [DESIGN.md](../DESIGN.md) for the
> overview and master [References](../DESIGN.md#references); session setup is in
> [docs/protocol.md](protocol.md). Status: **draft**.

The key words **MUST**, **SHOULD**, **MAY** are interpreted per
[RFC 2119](https://www.rfc-editor.org/info/rfc2119) /
[RFC 8174](https://www.rfc-editor.org/info/rfc8174).

File-transfer mode measures **real-world end-to-end transfer speed**: read a file from
disk on one host, stream it over TCP ([RFC 9293](https://www.rfc-editor.org/info/rfc9293)),
write it on the other. Unlike raw TCP mode, this number deliberately *includes* disk I/O —
that's the point. A `--null-source` flag removes the disk to isolate the network.

## 1. Why this mode exists

Raw TCP mode answers "how fast is the wire?" File mode answers "how fast will copying this
actual file feel?" The gap between the two is your disk (and filesystem, cache, and write
path). andri's differentiator is being able to measure **both** and show the difference:

```
raw TCP throughput        →  network ceiling
file transfer (disk)      →  what you actually get
file transfer (--null)    →  network ceiling via the file path's code, no disk
```

## 2. Data path

- After `Start`, the client opens the data connection(s) and streams the file body.
- The sender pipeline: **disk read → buffer → socket write**. The receiver:
  **socket read → buffer → disk write** (or discard, see §4).
- `parallel` streams chunk the file by byte range; each stream owns a range and a task.
  The receiver reassembles by range (each chunk carries its offset, or streams write to
  pre-sized regions).
- Reuse a single buffer per stream in the copy loop — no per-iteration allocation
  (same rule as TCP/UDP).

## 3. What is measured

- The measurement covers the **file body transfer**, from first byte sent to last byte
  durably handled on the receiver. `file_len` is negotiated up front
  ([protocol.md](protocol.md) §3.3) so the receiver knows when it's complete.
- Throughput reported in both bits/s and bytes/s.
- Unlike TCP mode, file mode does **not** apply a warm-up exclusion by default — a real
  file copy includes its own ramp, and excluding it would misrepresent "real-world" speed.
  (A flag MAY enable warm-up for users who want the steady-state-only number.)

## 4. `--null-source` and disk isolation

To separate network from disk, two independent knobs:

| Knob | Sender side | Receiver side |
|---|---|---|
| default | read real file from disk | write real file to disk |
| `--null-source` | generate bytes in memory (no disk read) | — |
| receiver discard (default for `--null-source`) | — | count bytes, do not write to disk |

- With `--null-source`, the transfer exercises the *file-mode code path* (chunking,
  buffering, reassembly) at network speed without disk involvement — useful to confirm the
  file path itself isn't the bottleneck.
- Receiver-side: writing to disk vs. discarding is a separate decision; for network-only
  measurement the receiver **SHOULD** discard. Writing to `/dev/null` or an equivalent
  in-memory sink is the portable way to do this.
- **fsync policy**: by default the receiver does **not** `fsync` per chunk (that would
  measure disk-flush latency, not transfer). A `--fsync` flag MAY force durability for
  users who want to include flush cost. This choice is documented because it materially
  changes the number.

## 5. Verification (optional)

- The client MAY send a checksum (or use `server_seed` to derive expected content) so the
  receiver can verify integrity. This is **off by default** because hashing competes for
  CPU and can cap throughput on fast links; when on, it is timed separately and reported
  as a distinct figure, never folded into transfer speed.

## 6. Timing & reporting

- Monotonic clock only (`std::time::Instant`).
- Aggregate across parallel streams over the common transfer window.
- The report **SHOULD** clearly state whether disk was involved on each side and whether
  `fsync`/verification were enabled — otherwise the number is ambiguous.

## 7. Edge cases & decisions

- **File smaller than warm-up/duration norms**: file mode is bounded by `file_len`, not by
  `duration`; a tiny file yields a short, high-variance measurement — the report flags low
  sample size.
- **Sparse/compressible files**: irrelevant to the network (bytes are bytes), but disk
  read speed may vary; `--null-source` removes this variable.
- **Receiver slower disk than sender**: the measured speed reflects the slower side — which
  is the honest end-to-end answer.

## 8. Decisions & deferrals

**v1 (decided):**
- **Single-file transfer only.** One file per run.

**Deferred to v2:**
- Directory / multi-file transfer (traversal, ordering, per-file accounting).
- Resume / range-restart for very large transfers.

## References

See the master list in [DESIGN.md](../DESIGN.md#references). Load-bearing here:

- **[RFC 9293](https://www.rfc-editor.org/info/rfc9293)** — Transmission Control Protocol.
  *The transport for the file stream.*
- **[RFC 6349](https://www.rfc-editor.org/info/rfc6349)** — Framework for TCP Throughput
  Testing. *Informs the throughput methodology shared with TCP mode.*
