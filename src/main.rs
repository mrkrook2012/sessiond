use clap::{Parser, Subcommand};
use color_eyre::Result;
use interprocess::local_socket::traits::Stream;
use rand::random_range;
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub mod backend;
pub mod frontend;
pub mod ipc;

#[derive(Parser)]
#[command(name = "daemon_test")]
#[command(about = "A daemon and terminal manager", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run as a daemon
    #[arg(long)]
    daemon: bool,
}

#[derive(Subcommand)]
enum Commands {
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.daemon {
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

    match cli.command {
        Some(Commands::Run { command }) => {
            handle_run_command(command).await?;
        }
        Some(Commands::List) => {
            handle_list_sessions().await?;
        }
        Some(Commands::Attach { name }) => {
            handle_attach_session(name).await?;
        }
        Some(Commands::Kill { name }) => {
            handle_kill_session(name).await?;
        }
        None => {
            // Original behavior: start daemon if not running, then run frontend
            while fs::metadata("/tmp/penar.sock").is_err() {
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

fn is_yaml_config_file(arg: &str) -> bool {
    let path = Path::new(arg);
    let has_yaml_ext = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    );
    has_yaml_ext && path.is_file()
}

async fn handle_run_command(command: String) -> Result<()> {
    // Check if daemon is running
    if fs::metadata("/tmp/penar.sock").is_err() {
        println!("Daemon not running. Starting daemon...");
        let stdout = File::create("./daemon.out").unwrap();
        let stderr = File::create("./daemon.err").unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .arg("--daemon")
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
        while fs::metadata("/tmp/penar.sock").is_err() && attempts < 10 {
            thread::sleep(Duration::from_millis(500));
            attempts += 1;
        }
        if fs::metadata("/tmp/penar.sock").is_err() {
            return Err(color_eyre::eyre::eyre!("Daemon failed to start in time"));
        }
    }

    let stream = interprocess::local_socket::Stream::connect(crate::ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = crate::ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // NEW: if the argument points to an existing .yaml/.yml file, run it as a config
    if is_yaml_config_file(&command) {
        println!("Starting session from config file: {}", command);

        sender.send(&crate::ipc::protocol::ClientMessage::StartSessionFromFile {
            file_path: command.clone(),
        })?;

        return match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
            crate::ipc::protocol::ServerMessage::Ok => {
                println!("Session started successfully from {}", command);
                Ok(())
            }
            _ => Err(color_eyre::eyre::eyre!(
                "Failed to start session from config file"
            )),
        };
    }

    // Existing behavior: treat the argument as a literal shell command
    let session_name = generate_random_session_name();

    println!("Starting session: {} (running: {})", session_name, command);

    sender.send(&crate::ipc::protocol::ClientMessage::CreateSession {
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

        match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
            crate::ipc::protocol::ServerMessage::ListedSessions { sessions } => {
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
async fn handle_list_sessions() -> Result<()> {
    if fs::metadata("/tmp/penar.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(crate::ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = crate::ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    sender.send(&crate::ipc::protocol::ClientMessage::ListSessions)?;

    match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
        crate::ipc::protocol::ServerMessage::ListedSessions { sessions } => {
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

async fn handle_attach_session(name: String) -> Result<()> {
    if fs::metadata("/tmp/penar.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(crate::ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = crate::ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    // We need to find the session_id from the name first.
    // First, list sessions.
    sender.send(&crate::ipc::protocol::ClientMessage::ListSessions)?;

    let session_id = match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
        crate::ipc::protocol::ServerMessage::ListedSessions { sessions } => sessions
            .into_iter()
            .find(|s| s.session_name == name)
            .map(|s| s.session_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session not found: {}", name))?,
        _ => return Err(color_eyre::eyre::eyre!("Failed to list sessions.")),
    };

    sender.send(&crate::ipc::protocol::ClientMessage::OpenSession { session_id })?;

    match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
        crate::ipc::protocol::ServerMessage::UpdatedSession { session_info } => {
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

async fn handle_kill_session(name: String) -> Result<()> {
    if fs::metadata("/tmp/penar.sock").is_err() {
        return Err(color_eyre::eyre::eyre!("Daemon not running."));
    }

    let stream = interprocess::local_socket::Stream::connect(crate::ipc::consts::socket_name())?;
    let (mut sender, mut receiver) = crate::ipc::ipc_connection::IpcConnection::new(stream)
        .split()
        .unwrap();

    sender.send(&crate::ipc::protocol::ClientMessage::ListSessions)?;

    let session_id = match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
        crate::ipc::protocol::ServerMessage::ListedSessions { sessions } => sessions
            .into_iter()
            .find(|s| s.session_name == name)
            .map(|s| s.session_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session not found: {}", name))?,
        _ => return Err(color_eyre::eyre::eyre!("Failed to list sessions.")),
    };

    sender.send(&crate::ipc::protocol::ClientMessage::KillSession { session_id })?;

    match receiver.recv::<crate::ipc::protocol::ServerMessage>()? {
        crate::ipc::protocol::ServerMessage::ListedSessions { .. } => {
            println!("Killed session: {}", name);
        }
        crate::ipc::protocol::ServerMessage::UpdatedSession { .. } => {
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

fn generate_random_session_name() -> String {
    let names = vec![
        "Henk",
        "Dikke Lul",
        "Bink",
        "Aardappel",
        "shiroi bakemono",
        "people in paris",
    ];
    let nr = random_range(0..names.len());
    let random_name = names.get(nr).unwrap();
    format!("session-{}", random_name)
}
