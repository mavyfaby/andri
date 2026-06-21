mod cli;
mod proto;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Cli::parse();

    let result = match (args.role.server, &args.role.client) {
        (true, _) => run_server(&args).await,
        (false, Some(host)) => run_client(&args, host).await,
        // clap's required, mutually-exclusive group makes this unreachable, but
        // matching it explicitly avoids relying on that invariant with a panic.
        (false, None) => unreachable!("clap group guarantees a role is selected"),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("andri: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Server role (`--server`). Stub — control/data listeners land next.
async fn run_server(args: &Cli) -> std::io::Result<()> {
    println!("andri server (stub) — would listen on port {}", args.port);
    Ok(())
}

/// Client role (`--client <host>`). Stub — handshake + modes land next.
async fn run_client(args: &Cli, host: &str) -> std::io::Result<()> {
    let mode = args.client_opts.mode.mode();
    println!(
        "andri client (stub) — would connect to {host}:{} for {mode:?} test",
        args.port
    );
    Ok(())
}
