use crate::backend::deamon::run_daemon;
use crate::frontend::frontend::run_frontend;
use color_eyre::Result;
use std::fs::{self, File};
use std::process::Command;
use std::thread;
use std::time::Duration;


pub mod backend;
pub mod ipc;
pub mod frontend;

#[tokio::main]
async fn main() -> Result<()> {
    match fs::remove_file("/tmp/penar.sock") {
        Ok(_) => println!("Deleted file"),
        Err(_) => {}
    }
    if std::env::args().any(|a| a == "--daemon") {
        let result = run_daemon();
        match result.await {
            Ok(()) => {
                println!("Stoppinig daemon");
            }
            Err(e) => {
                eprintln!("Got an error while running the daemon, {}", e);
            }
        }
    }
    println!("Hello, world!");

    while !fs::exists("/tmp/penar.sock").unwrap() {
        println!("Starting daemon");
        let stdout = File::create("./daemon.out").unwrap();
        let stderr = File::create("./daemon.err").unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .arg("--daemon")
            .stdout(stdout)
            .stderr(stderr)
            .spawn();

        match result {
            Err(e) => {
                eprintln!("Error while starting Daemon {}", e);
            }
            Ok(_) => {
                println!("Started daemon succesfully");
            }
        }

        thread::sleep(Duration::from_millis(1000));
    }
    run_frontend()?;

    Ok(())
}
