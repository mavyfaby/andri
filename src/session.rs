//! Shared control session: handshake, negotiate, Run/Stop, result exchange.
//!
//! This is the mode-agnostic control flow for both roles (`docs/protocol.md`).
//! Once data connections are set up it hands off to the per-mode data path in
//! `crate::modes`. All three modes (TCP, UDP, file) are implemented.

use crate::cli::{self, Cli};
use crate::modes::{file, tcp, udp};
use crate::proto::{self, Mode, Msg, Negotiate, ProtoError, Start};
use std::io;
use tokio::net::{TcpListener, TcpStream};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Default application buffer (until --buffer is wired).
const WRITE_BUF: usize = 64 * 1024;
/// Warm-up window excluded from the measurement (docs/tcp.md §3).
const WARMUP_SECS: u64 = 1;

// ---- Server -------------------------------------------------------------

/// Bind the control listener and serve sessions until interrupted.
pub async fn serve(port: u16, fmt: cli::Format) -> io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    println!("andri server listening on 0.0.0.0:{port}");
    loop {
        let (ctrl, peer) = listener.accept().await?;
        // One session at a time in v1 — keeps the live readout unambiguous.
        if let Err(e) = serve_session(ctrl, fmt).await {
            eprintln!("session with {peer} ended: {e}");
        }
    }
}

/// Drive one client session through the control state machine.
async fn serve_session(mut ctrl: TcpStream, fmt: cli::Format) -> io::Result<()> {
    // Greeting (§3.1/§3.2).
    let Msg::Hello(hello) = proto::read_msg(&mut ctrl).await? else {
        return reject(&mut ctrl, ProtoError::UnexpectedMessage).await;
    };
    let accepted = hello.protocol_version == proto::PROTOCOL_VERSION;
    proto::write_msg(
        &mut ctrl,
        &Msg::Welcome(proto::Welcome {
            protocol_version: proto::PROTOCOL_VERSION,
            server_version: VERSION.into(),
            nonce: hello.nonce,
            accepted,
        }),
    )
    .await?;
    if !accepted {
        return reject(&mut ctrl, ProtoError::VersionMismatch).await;
    }

    // Negotiate (§3.3). All three modes are implemented.
    let Msg::Negotiate(neg) = proto::read_msg(&mut ctrl).await? else {
        return reject(&mut ctrl, ProtoError::UnexpectedMessage).await;
    };

    // Bind the mode's data listener (TCP, used by TCP + file) / socket (UDP) and
    // tell the client its ephemeral port (§3.4).
    let tcp_data;
    let udp_data;
    let data_port = match neg.mode {
        Mode::Tcp | Mode::File => {
            let l = TcpListener::bind(("0.0.0.0", 0)).await?;
            let port = l.local_addr()?.port();
            tcp_data = Some(l);
            udp_data = None;
            port
        }
        Mode::Udp => {
            let (s, port) = udp::bind_data().await?;
            tcp_data = None;
            udp_data = Some(s);
            port
        }
    };
    proto::write_msg(
        &mut ctrl,
        // Fixed payload seed for now; drives the client's incompressible fill.
        // A per-session random seed lands with verification.
        &Msg::Start(Start {
            data_port,
            server_seed: 0x5EED_2026,
        }),
    )
    .await?;

    // Wait for Run (§3.5), then run the mode's data path until Stop.
    let Msg::Run = proto::read_msg(&mut ctrl).await? else {
        return reject(&mut ctrl, ProtoError::UnexpectedMessage).await;
    };

    // TCP/UDP run until the client's `Stop` arrives; file is bounded by file_len
    // and ignores Stop. Build the `stop` future only inside the arms that use it,
    // so it doesn't hold a borrow of `ctrl` across the final write.
    let result = match neg.mode {
        Mode::Tcp => {
            let stop = async {
                let _ = proto::read_msg(&mut ctrl).await; // expected: Msg::Stop
            };
            tcp::serve(tcp_data.unwrap(), &neg, fmt, stop).await?
        }
        Mode::Udp => {
            let stop = async {
                let _ = proto::read_msg(&mut ctrl).await;
            };
            udp::serve(udp_data.unwrap(), &neg, fmt, stop).await?
        }
        Mode::File => {
            let r = file::serve(tcp_data.unwrap(), &neg, fmt).await?;
            // File is bounded by file_len, but the client still sends `Stop` to
            // keep the control framing uniform. Drain it: closing the control
            // socket with that message unread would trigger an RST, not a clean
            // FIN, and the client would see "connection reset" reading Result.
            let _ = proto::read_msg(&mut ctrl).await; // expected: Msg::Stop
            r
        }
    };

    proto::write_msg(&mut ctrl, &Msg::Result(result)).await?;
    Ok(())
}

/// Send a terminal error and return (the connection is then dropped).
async fn reject(ctrl: &mut TcpStream, err: ProtoError) -> io::Result<()> {
    proto::write_msg(ctrl, &Msg::Error(err)).await?;
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("rejected: {err:?}"),
    ))
}

// ---- Client -------------------------------------------------------------

/// Connect, negotiate, run the data path, print results.
pub async fn connect(args: &Cli, host: &str) -> io::Result<()> {
    let opts = &args.client_opts;
    // A mode must be chosen explicitly — no default.
    let mode = opts.mode.selected().ok_or_else(|| {
        io::Error::other("a mode is required: pass one of --tcp, --udp, or --file <path>")
    })?;
    // UDP requires an explicit target rate — there is no sensible default for
    // the experiment's offered load (docs/udp.md). Fail fast, before connecting.
    let udp_bitrate = if mode == cli::Mode::Udp {
        Some(opts.bitrate.ok_or_else(|| {
            io::Error::other("--udp requires --bitrate (e.g. --bitrate 1G, 500M, 10M)")
        })?)
    } else {
        None
    };
    // File mode: --file <PATH> is required (even with --null-source, which only
    // skips reading the contents, not the size). The file length bounds the
    // transfer. Resolve it before connecting so a missing file fails fast.
    let file_path = opts.mode.file.clone();
    let file_len = if mode == cli::Mode::File {
        let path = file_path
            .as_deref()
            .ok_or_else(|| io::Error::other("--file requires a path"))?;
        Some(file::file_len(path).await?)
    } else {
        None
    };
    if !opts.json {
        print_config(args, host, mode);
    }

    // Control connection + greeting (§3.1/§3.2).
    let mut ctrl = TcpStream::connect((host, args.port)).await?;
    let nonce = 0xA17D_2026; // session-binding only; CSPRNG deferred (proto §3.1)
    proto::write_msg(
        &mut ctrl,
        &Msg::Hello(proto::Hello {
            protocol_version: proto::PROTOCOL_VERSION,
            client_version: VERSION.into(),
            nonce,
        }),
    )
    .await?;
    let Msg::Welcome(welcome) = proto::read_msg(&mut ctrl).await? else {
        return Err(io::Error::other("expected Welcome"));
    };
    if !welcome.accepted || welcome.nonce != nonce {
        return Err(io::Error::other("server rejected handshake"));
    }

    // Negotiate (§3.3). Each mode fills only its relevant fields.
    let is_udp = mode == cli::Mode::Udp;
    let is_file = mode == cli::Mode::File;
    let neg = Negotiate {
        mode: match mode {
            cli::Mode::Tcp => Mode::Tcp,
            cli::Mode::Udp => Mode::Udp,
            cli::Mode::File => Mode::File,
        },
        // File mode has no fixed window/warm-up; it runs until file_len bytes move.
        duration_secs: opts.duration,
        warmup_secs: if is_file { 0 } else { WARMUP_SECS },
        parallel: opts.parallel,
        buffer_bytes: WRITE_BUF,
        bidir: opts.bidir,
        bitrate_bps: udp_bitrate,
        packet_bytes: is_udp.then_some(opts.packet),
        file_len,
        null_source: is_file.then_some(opts.null_source),
    };
    proto::write_msg(&mut ctrl, &Msg::Negotiate(neg.clone())).await?;
    let Msg::Start(start) = proto::read_msg(&mut ctrl).await? else {
        return Err(io::Error::other("expected Start"));
    };

    // Under --verbose, confirm the payload is incompressible (debug only).
    if args.verbose {
        let sample = tcp::payload_sample(&start, neg.buffer_bytes);
        eprintln!("  payload    {}\n", crate::meter::payload_preview(&sample));
    }

    // Run the data path for the negotiated mode.
    proto::write_msg(&mut ctrl, &Msg::Run).await?;
    match mode {
        cli::Mode::Tcp => tcp::drive(host, &start, &neg, args.format, opts.json).await?,
        cli::Mode::Udp => udp::drive(host, &start, &neg, args.format, opts.json).await?,
        cli::Mode::File => {
            file::drive(
                host,
                &start,
                &neg,
                file_path.as_deref(),
                opts.null_source,
                args.format,
                opts.json,
            )
            .await?
        }
    }

    // Tell the server the window is over and read its authoritative result.
    proto::write_msg(&mut ctrl, &Msg::Stop).await?;
    let Msg::Result(result) = proto::read_msg(&mut ctrl).await? else {
        return Err(io::Error::other("expected Result"));
    };

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if is_udp {
        println!(
            "\nUDP: {} over {:.1}s | loss {}/{} ({:.2}%) | jitter {:.3} ms",
            args.format.render(result.bits_per_sec),
            result.duration_secs,
            result.packets_lost.unwrap_or(0),
            result.packets_expected.unwrap_or(0),
            result.loss_ratio.unwrap_or(0.0) * 100.0,
            result.jitter_ms.unwrap_or(0.0),
        );
    } else if is_file {
        let src = if opts.null_source {
            "memory (--null-source)"
        } else {
            "disk"
        };
        println!(
            "\nFile transfer: {} over {:.3}s | {} bytes from {src}",
            args.format.render(result.bits_per_sec),
            result.duration_secs,
            result.bytes,
        );
    } else {
        println!(
            "\nTCP throughput: {} over {:.1}s, {} stream(s)",
            args.format.render(result.bits_per_sec),
            result.duration_secs,
            opts.parallel,
        );
    }
    Ok(())
}

/// Print an ffmpeg-style banner of the resolved run options (to stderr).
fn print_config(args: &Cli, host: &str, mode: cli::Mode) {
    let opts = &args.client_opts;
    eprintln!("andri {VERSION} — client");
    eprintln!("  target     {host}:{}", args.port);
    eprintln!("  mode       {mode}");
    eprintln!(
        "  duration   {}s (+ {WARMUP_SECS}s warm-up, excluded)",
        opts.duration
    );
    if mode == cli::Mode::Udp {
        // bitrate is required for UDP, so it is Some here.
        let bps = opts.bitrate.unwrap_or(0);
        eprintln!("  bitrate    {}", args.format.render(bps as f64));
        eprintln!("  packet     {} bytes", opts.packet);
    } else {
        eprintln!("  streams    {}", opts.parallel);
        eprintln!("  buffer     {} KiB", WRITE_BUF / 1024);
    }
    eprintln!("  bidir      {}", opts.bidir);
    eprintln!("  format     {:?}", args.format);
    eprintln!();
}
