use clap::Parser;
use color_eyre::Result;
use std::fs::{self, File};
use std::process::Command;
use std::thread;
use std::time::Duration;

pub mod backend;
pub mod frontend;
pub mod ipc;
pub mod cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        Some(cli::Commands::StartDaemon) => {
            // Clean up any stale socket file
            let _ = fs::remove_file("/tmp/penar.sock");
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
        Some(cli::Commands::Run { command }) => {
            cli::handle_run_command(command).await?;
        }
        Some(cli::Commands::List) => {
            cli::handle_list_sessions().await?;
        }
        Some(cli::Commands::Attach { name }) => {
            cli::handle_attach_session(name).await?;
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
        None => {
            // Original behavior: start daemon if not running, then run frontend
            while fs::metadata("/tmp/penar.sock").is_err() {
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
