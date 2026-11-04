use clap::{Parser, Subcommand};
use color_eyre::Result;
use color_eyre::eyre::eyre;
use interprocess::local_socket::Stream;
use interprocess::local_socket::traits::Stream as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::backend::session_manager::SessionId;
use crate::ipc::consts::{socket_name, socket_path};
use crate::ipc::ipc_connection::{IpcConnection, IpcReceiver, IpcSender};
use crate::ipc::protocol::{ClientMessage, ServerMessage, SessionInfo};

/// Directory for `daemon.out` / `daemon.err` (default: current directory).
const LOG_DIR_ENV_VAR: &str = "DAEMON_TEST_LOG_DIR";

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

        /// Optional name for the new session (random name if omitted)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Lists all active sessions
    List {
        /// Print only session names, one per line (machine-readable)
        #[arg(long)]
        names: bool,
    },
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
    /// Kills the daemon process and waits until it has stopped
    KillDaemon,
    /// Starts the daemon
    StartDaemon,
    /// Exits with code 0 if the daemon is running, 1 otherwise
    Status,
    /// Exports a session to a config file
    Export {
        /// The name of the session to export
        name: String,
        /// Directory to export the file to (defaults to the current directory)
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
    /// Exports a session as a systemd service
    ExportSysd {
        /// The name of the session to export
        name: String,
    },
    /// Starts a session from a config file on behalf of systemd
    StartSysd {
        /// Path to the session config file (yaml)
        file_path: PathBuf,
    },
    /// Stops a systemd-managed session by name, on behalf of systemd
    StopSysd {
        /// The name of the session to stop
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn is_yaml_config_file(arg: &str) -> bool {
    let path = Path::new(arg);
    let has_yaml_ext = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    );
    has_yaml_ext && path.is_file()
}

pub fn generate_random_name() -> String {
    let names = ["henk", "bink", "aardappel", "piet", "jannes"];
    let nr = rand::random_range(0..names.len());
    names[nr].to_string()
}

/// True if a daemon is accepting connections on the socket. Unlike checking
/// that the socket file exists, this is not fooled by stale files left
/// behind by a crashed daemon.
pub fn daemon_running() -> bool {
    std::os::unix::net::UnixStream::connect(socket_path()).is_ok()
}

fn log_dir() -> PathBuf {
    std::env::var_os(LOG_DIR_ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Opens a connection without checking that the daemon is running first.
fn open_connection() -> Result<(IpcSender, IpcReceiver)> {
    let stream = Stream::connect(socket_name())?;
    Ok(IpcConnection::new(stream).split()?)
}

/// Connects to the running daemon, or fails with "Daemon not running."
fn connect() -> Result<(IpcSender, IpcReceiver)> {
    if !daemon_running() {
        return Err(eyre!("Daemon not running."));
    }
    open_connection()
}

fn list_sessions(sender: &mut IpcSender, receiver: &mut IpcReceiver) -> Result<Vec<SessionInfo>> {
    sender.send(&ClientMessage::ListSessions)?;
    match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { sessions } => Ok(sessions),
        ServerMessage::Error { error } => Err(eyre!("Failed to list sessions: {}", error)),
        _ => Err(eyre!("Failed to list sessions.")),
    }
}

fn find_session_id(
    sender: &mut IpcSender,
    receiver: &mut IpcReceiver,
    name: &str,
) -> Result<SessionId> {
    list_sessions(sender, receiver)?
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| s.id)
        .ok_or_else(|| eyre!("Session not found: {}", name))
}

/// Starts the daemon in the background if needed and waits until it accepts
/// connections. Progress messages go to stderr so stdout stays clean.
fn ensure_daemon_running() -> Result<()> {
    if daemon_running() {
        return Ok(());
    }

    eprintln!("Daemon not running. Starting daemon...");

    let log_dir = log_dir();
    let stdout = File::create(log_dir.join("daemon.out"))?;
    let stderr = File::create(log_dir.join("daemon.err"))?;

    // The child inherits our environment, so it sees the same
    // DAEMON_TEST_SOCKET / DAEMON_TEST_LOG_DIR as we do.
    let mut child = Command::new(std::env::current_exe()?)
        .arg("start-daemon")
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| eyre!("Error while starting Daemon {}", e))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon_running() {
        if let Some(status) = child.try_wait()? {
            return Err(eyre!(
                "Daemon exited during startup ({}). See {}",
                status,
                log_dir.join("daemon.err").display()
            ));
        }
        if Instant::now() > deadline {
            return Err(eyre!("Daemon failed to start in time"));
        }
        thread::sleep(Duration::from_millis(50));
    }

    eprintln!("Started daemon successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

pub async fn handle_run_command(command: String, name: Option<String>) -> Result<()> {
    ensure_daemon_running()?;
    let (mut sender, mut receiver) = open_connection()?;

    // If the argument points to an existing .yaml/.yml file, run it as a config
    if is_yaml_config_file(&command) {
        if name.is_some() {
            eprintln!("Warning: --name is ignored when starting from a config file");
        }
        eprintln!("Starting session from config file: {}", command);

        let absolute_path = fs::canonicalize(&command)
            .map_err(|e| eyre!("Failed to get absolute path for config file: {}", e))?;

        sender.send(&ClientMessage::StartSessionFromFile {
            file_path: absolute_path,
        })?;

        return match receiver.recv::<ServerMessage>()? {
            ServerMessage::Ok => {
                println!("Session started successfully from {}", command);
                Ok(())
            }
            ServerMessage::TerminalData { terminals: _ } => Ok(()),
            ServerMessage::Error { error } => {
                Err(eyre!("Failed to start session from config file, {}", error))
            }
            val => Err(eyre!("Unknown error has occured, {:?}", val)),
        };
    }

    // Literal shell command. Make sure an explicit name is not taken.
    // TODO: should be enforced in the backend (this check is racy).
    if let Some(ref n) = name {
        let existing = list_sessions(&mut sender, &mut receiver)?;
        if existing.iter().any(|s| &s.name == n) {
            return Err(eyre!("A session named '{}' already exists", n));
        }
    }

    let name = name.unwrap_or_else(generate_random_name);

    eprintln!("Starting session: {} (running: {})", name, command);

    sender.send(&ClientMessage::CreateSession {
        working_dir: ".".to_string(),
        command: Some(command),
        name: Some(name.clone()),
    })?;

    // Wait for ListedSessions, discarding other messages that may arrive first.
    // Note: recv() blocks, so this timeout is only checked between messages.
    let timeout = Duration::from_secs(5);
    let start = Instant::now();

    let sessions = loop {
        if start.elapsed() > timeout {
            return Err(eyre!("Timeout waiting for session list"));
        }

        match receiver.recv::<ServerMessage>()? {
            ServerMessage::ListedSessions { sessions } => break sessions,
            ServerMessage::Error { error } => {
                return Err(eyre!("Failed to create session: {}", error));
            }
            _ => continue,
        }
    };

    let session_info = sessions
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| eyre!("Failed to find newly created session in list"))?;

    println!("Session created successfully: {}", session_info.name);
    Ok(())
}

/// `start-sysd <file_path>`: asks the daemon to start a session from a config
/// file as a systemd-managed session. Starts the daemon first if needed.
pub async fn handle_start_sysd(file_path: PathBuf) -> Result<()> {
    // Validate and make absolute: the daemon has its own working directory.
    let absolute_path = fs::canonicalize(&file_path).map_err(|e| {
        eyre!(
            "Failed to get absolute path for config file {}: {}",
            file_path.display(),
            e
        )
    })?;
    if !absolute_path.is_file() {
        return Err(eyre!(
            "Config path is not a file: {}",
            absolute_path.display()
        ));
    }

    ensure_daemon_running()?;
    let (mut sender, mut receiver) = open_connection()?;

    eprintln!(
        "Starting systemd session from config file: {}",
        absolute_path.display()
    );

    sender.send(&ClientMessage::StartSysdSessionFromFile {
        file_path: absolute_path,
    })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::Ok
        | ServerMessage::UpdatedSession { .. }
        | ServerMessage::ListedSessions { .. } => {
            println!(
                "Systemd session started successfully from {}",
                file_path.display()
            );
            Ok(())
        }
        ServerMessage::Error { error } => Err(eyre!(
            "Failed to start systemd session from config file: {}",
            error
        )),
        val => Err(eyre!(
            "Unexpected response while starting systemd session: {:?}",
            val
        )),
    }
}

/// `stop-sysd <name>`: asks the daemon to stop a systemd-managed session.
/// Does not start the daemon: if it isn't running there is nothing to stop.
pub async fn handle_stop_sysd(name: String) -> Result<()> {
    let (mut sender, mut receiver) = connect()?;
    let id = find_session_id(&mut sender, &mut receiver, &name)?;

    println!("Stopping systemd session: {}", name);

    sender.send(&ClientMessage::StopSysdSession { id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::Ok
        | ServerMessage::UpdatedSession { .. }
        | ServerMessage::ListedSessions { .. } => {
            println!("Systemd session stopped successfully: {}", name);
            Ok(())
        }
        ServerMessage::Error { error } => Err(eyre!("Failed to stop systemd session: {}", error)),
        val => Err(eyre!(
            "Unexpected response while stopping systemd session: {:?}",
            val
        )),
    }
}

pub async fn handle_list_sessions(names_only: bool) -> Result<()> {
    let (mut sender, mut receiver) = connect()?;
    let sessions = list_sessions(&mut sender, &mut receiver)?;

    if names_only {
        for session in sessions {
            println!("{}", session.name);
        }
    } else if sessions.is_empty() {
        println!("No active sessions.");
    } else {
        println!("Active sessions:");
        for session in sessions {
            println!("- {}", session.name);
        }
    }

    Ok(())
}

pub async fn handle_attach_session(name: String) -> Result<()> {
    let (mut sender, mut receiver) = connect()?;
    let id = find_session_id(&mut sender, &mut receiver, &name)?;

    sender.send(&ClientMessage::OpenSession { id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::UpdatedSession { session_info } => {
            println!(
                "Attached to session: {} (window with terminal running)",
                session_info.name
            );
            Ok(())
        }
        ServerMessage::Error { error } => Err(eyre!("Failed to attach to session: {}", error)),
        _ => Err(eyre!("Failed to attach to session.")),
    }
}

pub async fn handle_kill_session(name: String) -> Result<()> {
    let (mut sender, mut receiver) = connect()?;
    let id = find_session_id(&mut sender, &mut receiver, &name)?;

    sender.send(&ClientMessage::KillSession { id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::ListedSessions { .. } | ServerMessage::UpdatedSession { .. } => {
            println!("Killed session: {}", name);
            Ok(())
        }
        ServerMessage::Error { error } => Err(eyre!("Failed to kill session: {}", error)),
        _ => Err(eyre!("Failed to kill session. Expected list or update.")),
    }
}

pub async fn handle_export_session(name: String, dir: Option<PathBuf>) -> Result<()> {
    if !daemon_running() {
        return Err(eyre!("Daemon not running."));
    }

    // The daemon has its own working directory, so always send an absolute path.
    let export_dir = match dir {
        Some(d) => {
            let abs = fs::canonicalize(&d)
                .map_err(|e| eyre!("Invalid export directory {}: {}", d.display(), e))?;
            if !abs.is_dir() {
                return Err(eyre!("Export path is not a directory: {}", abs.display()));
            }
            abs
        }
        None => std::env::current_dir()?,
    };

    let (mut sender, mut receiver) = open_connection()?;
    let id = find_session_id(&mut sender, &mut receiver, &name)?;

    sender.send(&ClientMessage::ExportSessionToFile {
        id,
        working_dir: export_dir,
    })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::ExportedSessionToFile { path } => {
            println!("Exported session '{}' to {}", name, path.display());
            Ok(())
        }
        ServerMessage::Error { error } => Err(eyre!("Failed to export session: {}", error)),
        val => Err(eyre!("Unexpected response while exporting: {:?}", val)),
    }
}

pub async fn handle_export_sysd(name: String) -> Result<()> {
    let (mut sender, mut receiver) = connect()?;
    let id = find_session_id(&mut sender, &mut receiver, &name)?;

    sender.send(&ClientMessage::ExportSessionToSysd { id })?;

    match receiver.recv::<ServerMessage>()? {
        ServerMessage::Ok => {
            println!("Exported session '{}' to systemd", name);
            Ok(())
        }
        ServerMessage::Error { error } => {
            Err(eyre!("Failed to export session to systemd: {}", error))
        }
        val => Err(eyre!(
            "Unexpected response while exporting to systemd: {:?}",
            val
        )),
    }
}

pub async fn handle_kill_all_sessions() -> Result<()> {
    let (mut sender, mut receiver) = connect()?;

    let ids: Vec<SessionId> = list_sessions(&mut sender, &mut receiver)?
        .into_iter()
        .map(|s| s.id)
        .collect();

    // Wait for each reply so that when we return, the sessions are really gone.
    for id in ids {
        sender.send(&ClientMessage::KillSession { id })?;
        if let ServerMessage::Error { error } = receiver.recv::<ServerMessage>()? {
            return Err(eyre!("Failed to kill session: {}", error));
        }
    }

    println!("Killed all active sessions");
    Ok(())
}

pub async fn handle_kill_daemon() -> Result<()> {
    if !daemon_running() {
        println!("Daemon not running.");
        return Ok(());
    }

    {
        let (mut sender, _receiver) = open_connection()?;
        // Tell the daemon to exit gracefully
        sender.send(&ClientMessage::KillDaemon)?;
    } // connection dropped here so we don't hold the daemon up

    // Wait until it actually stops accepting connections.
    let deadline = Instant::now() + Duration::from_secs(5);
    while daemon_running() {
        if Instant::now() > deadline {
            return Err(eyre!("Daemon did not stop in time"));
        }
        thread::sleep(Duration::from_millis(50));
    }

    println!("Daemon stopped");
    Ok(())
}

/// Exit code 0 if the daemon is running, 1 otherwise.
pub fn handle_status() -> ! {
    if daemon_running() {
        println!("Daemon running");
        std::process::exit(0);
    } else {
        println!("Daemon not running");
        std::process::exit(1);
    }
}
