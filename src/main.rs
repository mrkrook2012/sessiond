use clap::Parser;
use color_eyre::Result;
use std::fs;

pub mod backend;
pub mod cli;
pub mod frontend;
pub mod ipc;
pub mod logging;

use crate::ipc::consts::socket_path;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        Some(cli::Commands::Daemon {
            action: cli::DaemonCommands::Start,
        }) => {
            logging::init(cli.verbose.tracing_level_filter());

            // Clean up any stale socket file
            let _ = fs::remove_file(socket_path());

            match crate::backend::deamon::run_daemon().await {
                Ok(()) => tracing::info!("Daemon stopped"),
                Err(e) => tracing::error!("Daemon failed: {e:#}"),
            }
        }
        // Every other subcommand is handled by the cli module.
        Some(command) => {
            logging::init(cli.verbose.tracing_level_filter());
            cli::dispatch(command).await?;
        }
        None => {
            // The TUI owns the terminal, so this process does not log to stderr.
            // The daemon it starts still gets the requested log level.
            let _ = logging::LEVEL_FOR_CHILDREN.set(cli.verbose.tracing_level_filter());

            cli::ensure_daemon_running().await?;
            crate::frontend::frontend::run_frontend().await?;
        }
    }

    Ok(())
}
