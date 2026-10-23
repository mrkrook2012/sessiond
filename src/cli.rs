use clap::{Parser, Subcommand};
use color_eyre::Result;
use interprocess::local_socket::traits::Stream;
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::ipc;
use crate::ipc::protocol::ClientMessage;
use crate::ipc::protocol::ServerMessage;

#[derive(Parser)]
#[command(name = "daemon_test")]
#[command(about = "A daemon and terminal manager", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Starts a new session with the given command or based on file
    Run {
        /// The command to run in the new session or path to yaml
        command: String,
    },
    /// Lists all active sessions
    List,
    /// Attaches to an existing session by name
    Attach {
        /// The name of the session to attach to
        name: String,
    },
    /// Kills an existing session by name
    Kill {
        /// The name of the session to kill
        name: String,
    },
    /// Kills all active sessions
    KillAll,
    /// Kills the daemon process
    KillDaemon,
    /// Starts the daemon
    StartDaemon,
}

pub fn is_yaml_config_file(arg: &str) -> bool {
    let path = Path::new(arg);
    let has_yaml_ext = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    );
    has_yaml_ext && path.is_file()
}

pub fn generate_random_session_name() -> String {
    let names = vec!["henk", "bink", "aardappel", "piet", "jannes"];
    let nr = rand::random_range(0..names.len());
    names.get(nr).unwrap().to_string()
}

pub async fn handle_run_command(command: String) -> Result<()> {
    // Check if daemon is running
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        println!("Daemon not running. Starting daemon...");
        let stdout = File::create("./daemon.out").unwrap();
        let stderr = File::create("./daemon.err").unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .arg("start-daemon")
            .stdout(stdout)
            .stderr(stderr)
            .spawn();

        match result {
            Err(e) => {
                return Err(color_eyre::eyre::eyre!("Error while starting Daemon {}", e));
            }
            Ok(_) => {
                println!("Started daemon successfully");
            }
        }

        // Wait for daemon to be ready
        let mut attempts = 0;
        while fs::metadata("/tmp/daemon_test.sock").is_err() && attempts < 10 {
            thread::sleep(Duration::from_millis(500));
            attempts += 1;
        }
        if fs::metadata("/tmp/daemon_test.sock").is_err() {
            return Err(color_eyre::eyre::eyre!("Daemon failed to start in time"));
        }
    }

    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // if the argument points to an existing .yaml/.yml file, run it as a config
    if is_yaml_config_file(&command) {
        println!("Starting session from config file: {}", command);

        // Check file existence and convert to absolute path
        let path = Path::new(&command);
        if !path.exists() {
            return Err(color_eyre::eyre::eyre!(
                "Config file does not exist: {}",
                command
            ));
        }

        let absolute_path = std::fs::canonicalize(path).map_err(|e| {
            color_eyre::eyre::eyre!("Failed to get absolute path for config file: {}", e)
        })?;
        println!("Sending StartSessionFromFile message to daemon");
        sender.send(&ClientMessage::StartSessionFromFile {
            file_path: absolute_path,
        })?;

        return match receiver.recv::<ServerMessage>()? {
            ServerMessage::Ok => {
                println!("Session started successfully from {}", command);
                Ok(())
            }
            ServerMessage::TerminalData { terminals: _ } => Ok(()),

            ServerMessage::Error { error } => Err(color_eyre::eyre::eyre!(
                "Failed to start session from config file, {}",
                error
            )),

            val => Err(color_eyre::eyre::eyre!(
                "Unknown error has occured, {:?}",
                val
            )),
        };
    }

    // Existing behavior: treat the argument as a literal shell command
    let session_name = generate_random_session_name();

    println!("Starting session: {} (running: {})", session_name, command);

    sender.send(&ClientMessage::CreateSession {
        working_dir: ".".to_string(),
        command: Some(command.clone()),
        session_name: Some(session_name.clone()),
    })?;

    // Wait for ListedSessions message, discarding any other messages that might arrive first
    let timeout = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();

    let sessions = loop {
        if start.elapsed() > timeout {
            return Err(color_eyre::eyre::eyre!("Timeout waiting for session list"));
        }

        match receiver.recv::<ServerMessage>()? {
            ServerMessage::ListedSessions { sessions } => {
                break sessions;
            }
            // Discard other messages (like ping/pong, terminal data, etc.)
            _ => {
                // Continue waiting for ListedSessions
                continue;
            }
        }
    };

    let session_info = sessions
        .into_iter()
        .find(|s| s.session_name == session_name)
        .ok_or_else(|| color_eyre::eyre::eyre!("Failed to find newly created session in list"))?;
    println!(
        "Session created successfully: {}",
        session_info.session_name
    );
    Ok(())
}

pub async fn handle_list_sessions() -> Result<()> {
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    sender.send(&ClientMessage::ListSessions)?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { sessions } => {
            if sessions.is_empty() {
                println!("No active sessions.");
            } else {
                println!("Active sessions:");
                for session in sessions {
                    println!("- {}", session.session_name);
                }
            }
        }
        _ => {
            return Err(color_eyre::eyre::eyre!("Failed to list sessions."));
        }
    }

    Ok(())
}

pub async fn handle_attach_session(name: String) -> Result<()> {
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // We need to find the session_id from the name first.
    // First, list sessions.
    sender.send(&ClientMessage::ListSessions)?;

    let session_id = match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { sessions } => sessions
            .into_iter()
            .find(|s| s.session_name == name)
            .map(|s| s.session_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session not found: {}", name))?,
        _ => return Err(color_eyre::eyre::eyre!("Failed to list sessions.")),
    };

    sender.send(&ClientMessage::OpenSession { session_id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::UpdatedSession { session_info } => {
            println!(
                "Attached to session: {} (window with terminal running)",
                session_info.session_name
            );
        }
        _ => {
            return Err(color_eyre::eyre::eyre!("Failed to attach to session."));
        }
    }

    Ok(())
}

pub async fn handle_kill_session(name: String) -> Result<()> {
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    sender.send(&ClientMessage::ListSessions)?;

    let session_id = match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { sessions } => sessions
            .into_iter()
            .find(|s| s.session_name == name)
            .map(|s| s.session_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session not found: {}", name))?,
        _ => return Err(color_eyre::eyre::eyre!("Failed to list sessions.")),
    };

    sender.send(&ClientMessage::KillSession { session_id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { .. } => {
            println!("Killed session: {}", name);
        }
        ServerMessage::UpdatedSession { .. } => {
            println!("Killed session: {}", name);
        }
        _ => {
            return Err(color_eyre::eyre::eyre!(
                "Failed to kill session. Expected list or update."
            ));
        }
    }

    Ok(())
}

pub async fn handle_kill_all_sessions() -> Result<()> {
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // First, list all sessions
    sender.send(&ClientMessage::ListSessions)?;

    let session_ids: Vec<u64> = match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { sessions } => {
            sessions.into_iter().map(|s| s.session_id).collect()
        }
        _ => return Err(color_eyre::eyre::eyre!("Failed to list sessions.")),
    };

    // Kill all sessions
    for session_id in session_ids {
        sender.send(&ClientMessage::KillSession { session_id })?;

        // We don't need to wait for a response for each kill, just proceed with all
    }

    println!("Killed all active sessions");
    Ok(())
}

pub async fn handle_kill_daemon() -> Result<()> {
    if fs::metadata("/tmp/daemon_test.sock").is_err() {
        println!("Daemon not running.");
        return Ok(());
    }

    // We'll use the existing method to kill the daemon
    let stream = interprocess::local_socket::Stream::connect(ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // Send a message that tells the daemon to exit gracefully
    sender.send(&ClientMessage::KillDaemon)?;

    println!("Sent kill message to daemon");

    Ok(())
}
