mod cli;
mod meter;
mod modes;
mod proto;
mod session;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Cli::parse();

    let result = match (args.role.server, &args.role.client) {
        (true, _) => session::serve(args.port, args.format).await,
        (false, Some(host)) => session::connect(&args, host).await,
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
