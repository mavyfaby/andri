//! Shared control session: handshake, negotiate, Run/Stop, result exchange.
//!
//! This is the mode-agnostic control flow for both roles (`docs/protocol.md`).
//! Once data connections are set up it hands off to the per-mode data path in
//! `crate::modes`. v1 implements TCP; UDP and file negotiate to a not-yet-
//! implemented rejection.

use crate::cli::{self, Cli};
use crate::modes::tcp;
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

    // Negotiate (§3.3). Only TCP is implemented in v1.
    let Msg::Negotiate(neg) = proto::read_msg(&mut ctrl).await? else {
        return reject(&mut ctrl, ProtoError::UnexpectedMessage).await;
    };
    if neg.mode != Mode::Tcp {
        return reject(&mut ctrl, ProtoError::Internal).await;
    }

    // Bind an ephemeral data listener and tell the client where (§3.4).
    let data_listener = TcpListener::bind(("0.0.0.0", 0)).await?;
    let data_port = data_listener.local_addr()?.port();
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

    // `stop` resolves when the client's Stop message arrives.
    let stop = async {
        let _ = proto::read_msg(&mut ctrl).await; // expected: Msg::Stop
    };
    let result = tcp::serve(data_listener, &neg, fmt, stop).await?;

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
    // v1 implements TCP; UDP/file report their own not-yet-implemented error.
    match mode {
        cli::Mode::Tcp => {}
        cli::Mode::Udp => return Err(crate::modes::udp::not_implemented()),
        cli::Mode::File => return Err(crate::modes::file::not_implemented()),
    }
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

    // Negotiate (§3.3).
    let neg = Negotiate {
        mode: Mode::Tcp,
        duration_secs: opts.duration,
        warmup_secs: WARMUP_SECS,
        parallel: opts.parallel,
        buffer_bytes: WRITE_BUF,
        bidir: opts.bidir,
        bitrate_bps: None,
        packet_bytes: None,
        file_len: None,
        null_source: None,
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

    // Run the data path: open streams, send for the window, live progress.
    proto::write_msg(&mut ctrl, &Msg::Run).await?;
    tcp::drive(host, &start, &neg, args.format, opts.json).await?;

    // Tell the server the window is over and read its authoritative result.
    proto::write_msg(&mut ctrl, &Msg::Stop).await?;
    let Msg::Result(result) = proto::read_msg(&mut ctrl).await? else {
        return Err(io::Error::other("expected Result"));
    };

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
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
    eprintln!("  streams    {}", opts.parallel);
    eprintln!("  buffer     {} KiB", WRITE_BUF / 1024);
    eprintln!("  bidir      {}", opts.bidir);
    eprintln!("  format     {:?}", args.format);
    eprintln!();
}
