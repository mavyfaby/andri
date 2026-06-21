//! Per-mode data paths. Each module owns only the mode-specific data movement;
//! the shared control handshake lives in `crate::session`.

pub mod file;
pub mod tcp;
pub mod udp;
