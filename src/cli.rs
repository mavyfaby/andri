//! Command-line interface. Mirrors the surface in `docs/cli.md`.
//!
//! Flag style (`--server` / `--client <host>`) is the locked v1 choice, to match
//! iperf3 muscle memory.

use clap::{Args, Parser};

/// Default control port (TCP). In the IANA User/Registered range (RFC 6335).
pub const DEFAULT_PORT: u16 = 5201;

#[derive(Parser, Debug)]
#[command(name = "andri", version, about = "Fast, all-in-one LAN speed tester")]
pub struct Cli {
    #[command(flatten)]
    pub role: Role,

    #[command(flatten)]
    pub client_opts: ClientOpts,

    /// Control port (server binds; client dials).
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    pub port: u16,

    /// Verbose logging.
    #[arg(short, long)]
    pub verbose: bool,
}

/// Exactly one of `--server` / `--client <host>` must be present.
#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
pub struct Role {
    /// Run as the server (receiver), accepting tests from clients.
    #[arg(long)]
    pub server: bool,

    /// Run as the client, connecting to the server at `<host>`.
    #[arg(long, value_name = "HOST")]
    pub client: Option<String>,
}

/// Which measurement mode the client runs. `--tcp` is the default if none given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Tcp,
    Udp,
    File,
}

/// Client-only run options. Ignored on the server, which takes its parameters
/// from the client's `Negotiate` message over the control channel.
#[derive(Args, Debug)]
pub struct ClientOpts {
    #[command(flatten)]
    pub mode: ModeFlags,

    /// Measurement window length, seconds.
    #[arg(short, long, default_value_t = 10)]
    pub duration: u64,

    /// Number of concurrent data streams.
    #[arg(short = 'P', long, default_value_t = 1)]
    pub parallel: u32,

    /// Send and receive simultaneously.
    #[arg(long)]
    pub bidir: bool,

    /// Emit machine-readable JSON results to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Mode selection: at most one of `--tcp` / `--udp` / `--file <path>`.
/// None means TCP (the default).
#[derive(Args, Debug)]
#[group(required = false, multiple = false)]
pub struct ModeFlags {
    /// Raw TCP throughput (default).
    #[arg(long)]
    pub tcp: bool,

    /// UDP throughput with loss and jitter.
    #[arg(long)]
    pub udp: bool,

    /// Real file transfer from PATH.
    #[arg(long, value_name = "PATH")]
    pub file: Option<String>,
}

impl ModeFlags {
    /// Resolve the selected mode. The clap group guarantees mutual exclusion;
    /// absence of all flags defaults to TCP.
    pub fn mode(&self) -> Mode {
        if self.udp {
            Mode::Udp
        } else if self.file.is_some() {
            Mode::File
        } else {
            Mode::Tcp
        }
    }
}
