use clap::Parser;
use color_eyre::Result;
use std::fs;

use sessiond::ipc::consts::socket_path;
use sessiond::{backend, cli, frontend, logging};

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

            match backend::daemon::run().await {
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
            // The TUI owns the terminal, so it logs to a file instead of stderr.
            // The daemon it starts gets the same log level.
            let log_path = logging::init_file(cli.verbose.tracing_level_filter(), logging::FRONTEND_LOG)?;

            cli::ensure_daemon_running().await?;
            if let Err(e) = frontend::app::run_frontend().await {
                tracing::error!("Frontend failed: {e:#}");
                return Err(e.wrap_err(format!("See {}", log_path.display())));
            }
        }
    }

    Ok(())
}
