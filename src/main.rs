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
        Some(cli::Commands::Daemon {
            action: cli::DaemonCommands::Start,
        }) => {
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
        // Every other subcommand is handled by the cli module.
        Some(command) => cli::dispatch(command).await?,
        None => {
            // Original behavior: start daemon if not running, then run frontend
            if fs::metadata("/tmp/daemon_test.sock").is_err() {
                println!("Starting daemon");
                let stdout = File::create("./daemon.out").unwrap();
                let stderr = File::create("./daemon.err").unwrap();
                let result = Command::new(std::env::current_exe().unwrap())
                    .args(["daemon", "start"])
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
            crate::frontend::frontend::run_frontend().await?;
        }
    }

    Ok(())
}
