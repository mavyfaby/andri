//! Command-line interface. Mirrors the surface in `docs/cli.md`.
//!
//! Flag style (`--server` / `--client <host>`) is the locked v1 choice, to match
//! iperf3 muscle memory.

use clap::{Args, Parser, ValueEnum};

/// Default control port (TCP). In the IANA User/Registered range (RFC 6335).
pub const DEFAULT_PORT: u16 = 5201;

/// Throughput unit shown in output (`docs/cli.md` §2). Exactly one.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// Raw bits per second.
    Bits,
    /// Raw bytes per second.
    Bytes,
    /// Megabits per second (default; networking convention).
    #[default]
    Mbps,
    /// Gigabits per second.
    Gbps,
}

impl Format {
    /// Render a bits-per-second value in this unit, with a unit suffix.
    pub fn render(self, bits_per_sec: f64) -> String {
        match self {
            Format::Bits => format!("{bits_per_sec:.0} bit/s"),
            Format::Bytes => format!("{:.0} byte/s", bits_per_sec / 8.0),
            Format::Mbps => format!("{:.2} Mbps", bits_per_sec / 1e6),
            Format::Gbps => format!("{:.2} Gbps", bits_per_sec / 1e9),
        }
    }
}

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

    /// Throughput unit for output. Each role formats its own readout.
    #[arg(long, value_enum, default_value_t = Format::Mbps)]
    pub format: Format,

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

/// Which measurement mode the client runs. Selected explicitly; no default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Tcp,
    Udp,
    File,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Mode::Tcp => "TCP",
            Mode::Udp => "UDP",
            Mode::File => "FILE",
        };

        f.write_str(s)
    }
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

    /// UDP target send rate, e.g. `1G`, `500M`, `10M` (bits/s; SI suffixes).
    /// Required for `--udp` — UDP has no natural rate to discover, so the offered
    /// load is the experiment's input (see docs/udp.md). No default.
    #[arg(short, long, value_parser = parse_bitrate)]
    pub bitrate: Option<u64>,

    /// UDP datagram payload size in bytes (avoids 1500-MTU fragmentation).
    #[arg(long, default_value_t = 1472)]
    pub packet: usize,

    /// File mode: stream generated in-memory bytes instead of reading the file
    /// from disk, to isolate the network from the sender's disk I/O.
    #[arg(long)]
    pub null_source: bool,

    /// Emit machine-readable JSON results to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Parse a bitrate with optional SI suffix (K/M/G = 10^3/10^6/10^9 bits/s).
/// Bare numbers are bits/s. Decimal because link speeds are decimal bits.
fn parse_bitrate(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('K' | 'k') => (&s[..s.len() - 1], 1_000),
        Some('M' | 'm') => (&s[..s.len() - 1], 1_000_000),
        Some('G' | 'g') => (&s[..s.len() - 1], 1_000_000_000),
        _ => (s, 1),
    };
    let value: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid bitrate: {s:?}"))?;
    if value < 0.0 {
        return Err(format!("bitrate must be non-negative: {s:?}"));
    }
    Ok((value * mult as f64) as u64)
}

/// Mode selection: exactly one of `--tcp` / `--udp` / `--file <path>`.
///
/// The clap group is *not* marked `required`, because these flags flatten into
/// the top-level `Cli` and the server role takes no mode. The "exactly one"
/// requirement is enforced for the client after parsing (see `selected`).
#[derive(Args, Debug)]
#[group(required = false, multiple = false)]
pub struct ModeFlags {
    /// Raw TCP throughput.
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
    /// The explicitly-selected mode, or `None` if no mode flag was given.
    /// There is no default — the client requires one of `--tcp`/`--udp`/`--file`.
    pub fn selected(&self) -> Option<Mode> {
        if self.tcp {
            Some(Mode::Tcp)
        } else if self.udp {
            Some(Mode::Udp)
        } else if self.file.is_some() {
            Some(Mode::File)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Format::render` produces the right unit and conversion. 1 Gbit/s = 1e9.
    #[test]
    fn format_render_units() {
        let one_gbit = 1e9;
        assert_eq!(Format::Bits.render(one_gbit), "1000000000 bit/s");
        assert_eq!(Format::Bytes.render(one_gbit), "125000000 byte/s");
        assert_eq!(Format::Mbps.render(one_gbit), "1000.00 Mbps");
        assert_eq!(Format::Gbps.render(one_gbit), "1.00 Gbps");
    }

    /// Mode resolves from exactly the flag set; absence yields None (no default).
    #[test]
    fn mode_flags_selected() {
        let none = ModeFlags {
            tcp: false,
            udp: false,
            file: None,
        };
        assert_eq!(none.selected(), None);

        let tcp = ModeFlags {
            tcp: true,
            udp: false,
            file: None,
        };
        assert_eq!(tcp.selected(), Some(Mode::Tcp));

        let udp = ModeFlags {
            tcp: false,
            udp: true,
            file: None,
        };
        assert_eq!(udp.selected(), Some(Mode::Udp));

        let file = ModeFlags {
            tcp: false,
            udp: false,
            file: Some("x.iso".into()),
        };
        assert_eq!(file.selected(), Some(Mode::File));
    }

    /// Modes display uppercase in the banner (TCP/UDP/FILE, not Debug casing).
    #[test]
    fn mode_display_is_uppercase() {
        assert_eq!(Mode::Tcp.to_string(), "TCP");
        assert_eq!(Mode::Udp.to_string(), "UDP");
        assert_eq!(Mode::File.to_string(), "FILE");
    }

    /// The clap definition itself is internally consistent (catches arg conflicts
    /// / duplicate flags at test time rather than first run).
    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
