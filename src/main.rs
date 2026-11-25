use clap::Parser;
use color_eyre::Result;
use std::fs;
use tracing::level_filters::LevelFilter;

use sessiond::ids::SessionId;

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
        Some(cli::Commands::Attach { name }) => {
            // Resolve the name before the TUI owns the terminal, so errors print normally.
            let session = cli::resolve_attach_target(&name).await?;
            run_tui(cli.verbose.tracing_level_filter(), Some(session)).await?;
        }
        // Every other subcommand is handled by the cli module.
        Some(command) => {
            logging::init(cli.verbose.tracing_level_filter());
            cli::dispatch(command).await?;
        }
        None => run_tui(cli.verbose.tracing_level_filter(), None).await?,
    }

    Ok(())
}

/// Runs the TUI, attached to `session` if given. The TUI owns the terminal, so it logs to
/// a file instead of stderr; the daemon it starts gets the same log level.
async fn run_tui(level: LevelFilter, session: Option<SessionId>) -> Result<()> {
    let log_path = logging::init_file(level, logging::FRONTEND_LOG)?;

    cli::ensure_daemon_running().await?;
    if let Err(e) = frontend::app::run_frontend(session).await {
        tracing::error!("Frontend failed: {e:#}");
        return Err(e.wrap_err(format!("See {}", log_path.display())));
    }
    Ok(())
}
