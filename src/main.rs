use clap::Parser;
use color_eyre::Result;
use std::fs::{self, File};
use std::process::Command;
use std::thread;
use std::time::Duration;

pub mod backend;
pub mod cli;
pub mod frontend;
pub mod ipc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        Some(cli::Commands::StartDaemon) => {
            // Clean up any stale socket file
            let _ = fs::remove_file("/tmp/daemon_test.sock");
            let result = crate::backend::deamon::run_daemon().await;
            match result {
                Ok(()) => {
                    println!("Stopping daemon");
                }
                Err(e) => {
                    eprintln!("Got an error while running the daemon, {}", e);
                }
            }
            return Ok(());
        }
        Some(cli::Commands::Run { command, name }) => {
            cli::handle_run_command(command, name).await?;
        }
        Some(cli::Commands::List { names }) => cli::handle_list_sessions(names).await?,
        Some(cli::Commands::Status) => cli::handle_status(),
        Some(cli::Commands::Attach { name }) => {
            cli::handle_attach_session(name).await?;
        }
        Some(cli::Commands::StartSysd { file_path }) => {
            cli::handle_start_sysd(file_path).await?;
        }
        Some(cli::Commands::StopSysd { name }) => {
            cli::handle_stop_sysd(name).await?;
        }
        Some(cli::Commands::Kill { name }) => {
            cli::handle_kill_session(name).await?;
        }
        Some(cli::Commands::KillAll) => {
            cli::handle_kill_all_sessions().await?;
        }
        Some(cli::Commands::KillDaemon) => {
            cli::handle_kill_daemon().await?;
        }
        Some(cli::Commands::Export { name, dir }) => cli::handle_export_session(name, dir).await?,
        Some(cli::Commands::ExportSysd { name }) => cli::handle_export_sysd(name).await?,
        None => {
            // Original behavior: start daemon if not running, then run frontend
            if fs::metadata("/tmp/daemon_test.sock").is_err() {
                println!("Starting daemon");
                let stdout = File::create("./daemon.out").unwrap();
                let stderr = File::create("./daemon.err").unwrap();
                let result = Command::new(std::env::current_exe().unwrap())
                    .arg("start-daemon")
                    .stdout(stdout)
                    .stderr(stderr)
                    .spawn();

                match result {
                    Err(e) => {
                        eprintln!("Error while starting Daemon {}", e);
                    }
                    Ok(_) => {
                        println!("Started daemon successfully");
                    }
                }

                thread::sleep(Duration::from_millis(1000));
            }
            crate::frontend::frontend::run_frontend()?;
        }
    }

    Ok(())
}
