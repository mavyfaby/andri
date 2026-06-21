# andri — Web Dashboard (HTTP + WebSocket)

**Author:** [mavyfaby](https://github.com/mavyfaby) &lt;maverickfabroa@gmail.com&gt;

> Spec for the browser-facing surface served by the `andri` server. See
> [DESIGN.md](../DESIGN.md) for the overview and [docs/protocol.md](protocol.md) for the
> independent binary↔binary control protocol. Status: **draft**.

The key words **MUST**, **SHOULD**, **MAY** are interpreted per
[RFC 2119](https://www.rfc-editor.org/info/rfc2119) /
[RFC 8174](https://www.rfc-editor.org/info/rfc8174).

## 1. Purpose & relationship to the binary protocol

The server (`andri --server`) hosts an embedded HTTP server in addition to its raw
control/data listeners. A browser on another machine connects, gets a single-page UI, and
can **launch tests and watch live results** — no client binary install required. This
exists for **compatibility and ease of use**: any device with a browser can run a quick
test.

Two distinct client surfaces talk to the same server:

| Surface | Transport | Modes available |
|---|---|---|
| **`andri` client binary** | custom TCP control protocol ([protocol.md](protocol.md)) | raw TCP, **raw UDP (loss/jitter)**, file |
| **Browser dashboard** | HTTP + WebSocket (this doc) | TCP-over-WebSocket throughput, file transfer |

## 2. Honesty constraints (non-negotiable)

The browser is a convenience surface; it **MUST NOT** present a measurement as something
it isn't. For a benchmarking tool, a misleading number is worse than a missing one.

- **No raw UDP.** Browsers cannot open raw UDP sockets, so the per-datagram
  sequence/timestamp stamping behind RFC 3550 jitter and RFC 7680 loss is not achievable.
  The dashboard **MUST** disable the UDP mode and explain that it requires the client
  binary. (WebRTC DataChannel — SCTP/DTLS/UDP — was considered as an approximation and
  rejected for v1 to avoid numbers that look like raw-UDP results but aren't.)
- **TCP is "WebSocket throughput," not raw TCP.** It is TCP underneath (WS rides on TCP),
  but without `SO_SNDBUF` tuning or raw parallel streams. The UI **MUST** label it as such
  and **MUST NOT** present it as an `iperf3`-equivalent raw figure.
- **File transfer** over HTTP is a genuine end-to-end measurement and needs no caveat.

## 3. HTTP surface

Served per HTTP semantics [RFC 9110](https://www.rfc-editor.org/info/rfc9110) over HTTP/1.1
[RFC 9112](https://www.rfc-editor.org/info/rfc9112). For untrusted networks the server
**MAY** terminate TLS 1.3 ([RFC 8446](https://www.rfc-editor.org/info/rfc8446)); the v1
default is plaintext on a trusted LAN.

| Method & path | Purpose |
|---|---|
| `GET /` | The single-page dashboard (HTML/JS/CSS, embedded in the binary). |
| `GET /api/capabilities` | JSON: server version, available modes, limits (mirrors §5 of protocol.md). |
| `POST /api/file` | File-transfer test: client uploads (or downloads) a body of N bytes; server measures end-to-end throughput. |
| `GET /ws` | WebSocket upgrade — the control + live-readout channel (§4). |

Assets are embedded in the binary (e.g. via `rust-embed`) so the single-binary,
no-dependencies promise holds — the dashboard ships inside `andri`, nothing to install.

## 4. WebSocket channel

The dashboard uses one WebSocket ([RFC 6455](https://www.rfc-editor.org/info/rfc6455)),
established by the standard HTTP Upgrade handshake — which replaces the binary protocol's
`Hello`/`Welcome` (the upgrade *is* the handshake, so no bespoke greeting is needed).

It carries two things:

1. **Control** — JSON messages mirroring a subset of the binary `Negotiate` / `Run` /
   `Stop` / `Result` messages, minus UDP fields. Same `#[serde(tag = "type")]` style.
2. **TCP-throughput data** — binary WebSocket frames carry the payload; the server counts
   bytes. Labeled "WebSocket throughput" per §2.

Live once-per-second readout is pushed server→browser as JSON text frames during a run.

## 5. Capability matrix

| Mode | Browser dashboard | Notes |
|---|---|---|
| **File transfer** | ✅ Full | `POST`/`GET` of N bytes — a real end-to-end measurement. |
| **TCP throughput** | ⚠️ Approximate | "WebSocket throughput" — TCP underneath, framed and untunable. |
| **UDP loss/jitter** | ❌ Unavailable | Requires the client binary; UI explains why. |

## 6. Open questions

- Whether the server aggregates browser runs and binary-client runs into one results view.
- Auth for the HTTP surface on shared LANs (token, or TLS client certs).
- Whether to add a documented-as-approximate WebRTC UDP mode in a later version.

## References

In addition to the RFCs in [DESIGN.md](../DESIGN.md#references) and
[protocol.md](protocol.md#references):

- **[RFC 9110](https://www.rfc-editor.org/info/rfc9110)** — HTTP Semantics (Fielding,
  Nottingham, Reschke, Eds., Jun 2022). *HTTP surface.*
- **[RFC 9112](https://www.rfc-editor.org/info/rfc9112)** — HTTP/1.1 (Fielding,
  Nottingham, Reschke, Eds., Jun 2022). *HTTP messaging.*
- **[RFC 6455](https://www.rfc-editor.org/info/rfc6455)** — The WebSocket Protocol
  (Fette, Melnikov, Dec 2011). **The dashboard control/data channel uses this.**
- **[RFC 8446](https://www.rfc-editor.org/info/rfc8446)** — TLS 1.3 (Rescorla, Aug 2018).
  *Deferred — optional HTTPS.*

As elsewhere: only the protocols we actually implement (HTTP, WebSocket) are exact claims;
TLS is deferred, and WebRTC is explicitly out of scope for v1.
