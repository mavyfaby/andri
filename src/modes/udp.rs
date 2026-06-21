//! UDP throughput + loss/jitter data path (`docs/udp.md`).
//!
//! Not yet implemented. When built, this mode owns: paced sending (RFC 8085),
//! per-datagram seq + send-timestamp stamping, RFC 3550 §6.4.1 jitter, and
//! RFC 7680 one-way loss. The control handshake is shared via `session.rs`.

use std::io;

/// Placeholder until the UDP data path lands. The server rejects UDP `Negotiate`
/// with `ProtoError::Internal` in the meantime (see `session.rs`).
pub fn not_implemented() -> io::Error {
    io::Error::other("UDP mode is not implemented yet")
}
