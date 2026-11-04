use clap::{Parser, Subcommand};
use color_eyre::Result;
use color_eyre::eyre::eyre;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::backend::session_manager::SessionId;
use crate::ipc::client::DaemonClient;
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::SessionInfo;

/// Directory for daemon.out / daemon.err.
const LOG_DIR_ENV_VAR: &str = "DAEMON_TEST_LOG_DIR";

#[derive(Parser)]
#[command(name = "daemon_test")]
#[command(about = "A daemon and terminal manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Starts a new session with the given command or based on file
    Run {
        command: String,
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Lists all active sessions
    List {
        #[arg(long)]
        names: bool,
    },
    /// Attaches to an existing session by name
    Attach { name: String },
    /// Kills an existing session by name
    Kill { name: String },
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
        name: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
    /// Exports a session as a systemd service
    ExportSysd { name: String },
    /// Starts a session from a config file on behalf of systemd
    StartSysd { file_path: PathBuf },
    /// Stops a systemd-managed session by name, on behalf of systemd
    StopSysd { name: String },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn is_yaml_config_file(arg: &str) -> bool {
    let path = Path::new(arg);

    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    ) && path.is_file()
}

pub fn generate_random_name() -> String {
    let names = ["henk", "bink", "aardappel", "piet", "jannes"];
    names[rand::random_range(0..names.len())].to_string()
}

/// True if a daemon is accepting connections on the socket.
pub fn daemon_running() -> bool {
    std::os::unix::net::UnixStream::connect(socket_path()).is_ok()
}

/// Connect to the running daemon using tarpc.
async fn connect() -> Result<DaemonClient> {
    if !daemon_running() {
        return Err(eyre!("Daemon not running."));
    }

    DaemonClient::connect().await
}

/// Retrieve all active sessions.
async fn list_sessions(rpc: &DaemonClient) -> Result<Vec<SessionInfo>> {
    rpc.list_sessions().await
}

/// Find a session by name.
async fn find_session_id(rpc: &DaemonClient, name: &str) -> Result<SessionId> {
    list_sessions(rpc)
        .await?
        .into_iter()
        .find(|session| session.name == name)
        .map(|session| session.id)
        .ok_or_else(|| eyre!("Session not found: {}", name))
}

/// Start the daemon in the background if necessary and wait until it accepts
/// connections. Progress messages go to stderr.
async fn ensure_daemon_running() -> Result<()> {
    if daemon_running() {
        return Ok(());
    }

    eprintln!("Daemon not running. Starting daemon...");

    let log_dir = std::env::var_os(LOG_DIR_ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let stdout = File::create(log_dir.join("daemon.out"))?;
    let stderr = File::create(log_dir.join("daemon.err"))?;

    let mut child = Command::new(std::env::current_exe()?)
        .arg("start-daemon")
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| eyre!("Error while starting daemon: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if daemon_running() {
            break;
        }

        if let Some(status) = child.try_wait()? {
            return Err(eyre!(
                "Daemon exited during startup ({status}). See {}",
                log_dir.join("daemon.err").display()
            ));
        }

        if Instant::now() >= deadline {
            return Err(eyre!("Daemon failed to start in time"));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    eprintln!("Started daemon successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

pub async fn handle_run_command(command: String, name: Option<String>) -> Result<()> {
    ensure_daemon_running().await?;
    let rpc = connect().await?;

    // Start from YAML.
    if is_yaml_config_file(&command) {
        if name.is_some() {
            eprintln!("Warning: --name is ignored when starting from a config file");
        }

        eprintln!("Starting session from config file: {command}");

        let absolute_path = fs::canonicalize(&command)
            .map_err(|e| eyre!("Failed to get absolute path for config file: {e}"))?;

        rpc.start_session_from_file(absolute_path).await?;

        println!("Session started successfully from {command}");
    } else {
        // Check for duplicate names.
        if let Some(ref name) = name {
            let sessions = list_sessions(&rpc).await?;

            if sessions.iter().any(|session| &session.name == name) {
                return Err(eyre!("A session named '{}' already exists", name));
            }
        }

        let name = name.unwrap_or_else(generate_random_name);

        eprintln!("Starting session: {} (running: {})", name, command);

        let sessions = rpc
            .create_session(".".to_string(), Some(command), Some(name.clone()))
            .await?;

        let session = sessions
            .iter()
            .find(|session| session.name == name)
            .ok_or_else(|| eyre!("Failed to find newly created session in list"))?;

        println!("Session created successfully: {}", session.name);
    }

    Ok(())
}

/// Starts a systemd-managed session from a config file.
pub async fn handle_start_sysd(file_path: PathBuf) -> Result<()> {
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

    ensure_daemon_running().await?;
    let rpc = connect().await?;

    eprintln!(
        "Starting systemd session from config file: {}",
        absolute_path.display()
    );

    rpc.start_sysd_session_from_file(absolute_path).await?;

    println!(
        "Systemd session started successfully from {}",
        file_path.display()
    );

    Ok(())
}

/// Stops a systemd-managed session. Does not start the daemon.
pub async fn handle_stop_sysd(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    println!("Stopping systemd session: {name}");

    rpc.stop_sysd_session(id).await?;

    println!("Systemd session stopped successfully: {name}");

    Ok(())
}

pub async fn handle_list_sessions(names_only: bool) -> Result<()> {
    let rpc = connect().await?;
    let sessions = list_sessions(&rpc).await?;

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
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    let session_info = rpc.open_session(id).await?;

    println!(
        "Attached to session: {} (window with terminal running)",
        session_info.name
    );

    Ok(())
}

pub async fn handle_kill_session(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    rpc.kill_session(id).await?;

    println!("Killed session: {name}");

    Ok(())
}

pub async fn handle_export_session(name: String, dir: Option<PathBuf>) -> Result<()> {
    let rpc = connect().await?;

    // The daemon has its own working directory, so send an absolute path.
    let export_dir = match dir {
        Some(dir) => {
            let absolute = fs::canonicalize(&dir)
                .map_err(|e| eyre!("Invalid export directory {}: {e}", dir.display()))?;

            if !absolute.is_dir() {
                return Err(eyre!(
                    "Export path is not a directory: {}",
                    absolute.display()
                ));
            }

            absolute
        }
        None => std::env::current_dir()?,
    };

    let id = find_session_id(&rpc, &name).await?;

    let path = rpc.export_session_to_file(id, export_dir).await?;

    println!("Exported session '{}' to {}", name, path.display());

    Ok(())
}

pub async fn handle_export_sysd(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    rpc.export_session_to_sysd(id).await?;

    println!("Exported session '{}' to systemd", name);
    Ok(())
}

pub async fn handle_kill_all_sessions() -> Result<()> {
    let rpc = connect().await?;

    let ids: Vec<SessionId> = list_sessions(&rpc)
        .await?
        .into_iter()
        .map(|session| session.id)
        .collect();

    // Wait for every response so that all sessions are actually killed.
    for id in ids {
        rpc.kill_session(id).await?;
    }
    println!("Killed all active sessions");
    Ok(())
}

pub async fn handle_kill_daemon() -> Result<()> {
    if !daemon_running() {
        println!("Daemon not running.");
        return Ok(());
    }

    let rpc = connect().await?;

    // The daemon exits immediately when processing KillDaemon.
    // A transport error is therefore expected.
    let _ = rpc.kill_daemon().await;

    drop(rpc);

    let deadline = Instant::now() + Duration::from_secs(5);

    while daemon_running() {
        if Instant::now() >= deadline {
            return Err(eyre!("Daemon did not stop in time"));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
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
