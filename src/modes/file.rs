//! File-transfer data path (`docs/file.md`).
//!
//! Not yet implemented. When built, this mode owns: real disk read → stream →
//! write (or `--null-source` in-memory), single-file transfer, optional fsync
//! and verification. The control handshake is shared via `session.rs`.

use std::io;

/// Placeholder until the file data path lands. The server rejects file
/// `Negotiate` with `ProtoError::Internal` in the meantime (see `session.rs`).
pub fn not_implemented() -> io::Error {
    io::Error::other("file-transfer mode is not implemented yet")
}
