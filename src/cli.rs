use clap::{Parser, Subcommand};
use color_eyre::Result;
use color_eyre::eyre::eyre;

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::backend::session_manager::SessionId;
use crate::ipc::client::DaemonClient;
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::{SessionInfo, SysdAction};

/// Directory for daemon.out / daemon.err.
const LOG_DIR_ENV_VAR: &str = "DAEMON_TEST_LOG_DIR";

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "daemon_test")]
#[command(about = "A daemon and terminal manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Starts a new session running the given command
    Run {
        /// Create a systemd service running the command instead of an ephemeral session
        #[arg(long)]
        systemd: bool,

        /// Name of the session (random if omitted)
        #[arg(short, long)]
        name: Option<String>,

        /// The command and its arguments
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Lists all active sessions
    List {
        #[arg(long)]
        names: bool,
    },
    // TODO Fix this
    /// Attaches to an existing session by name
    Attach { name: String },
    /// Kills a session by name, or all sessions with --all
    Kill {
        name: Option<String>,

        #[arg(long, conflicts_with = "name", required_unless_present = "name")]
        all: bool,
    },
    /// Manages systemd-based sessions
    Service {
        #[command(subcommand)]
        action: ServiceCommands,
    },
    /// Exports an ephemeral session to a config file
    Export {
        name: String,

        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
    /// Starts an ephemeral session from a config file
    Import { file: PathBuf },
    /// Manages the daemon process
    Daemon {
        #[command(subcommand)]
        action: DaemonCommands,
    },

    // Internal commands, invoked by the generated systemd units.
    #[command(name = "__internal-systemd-start", hide = true)]
    InternalSystemdStart { config: PathBuf },
    #[command(name = "__internal-systemd-stop", hide = true)]
    InternalSystemdStop { name: String },
}

#[derive(Subcommand)]
pub enum ServiceCommands {
    /// Starts the service
    Start { name: String },
    /// Stops the service
    Stop { name: String },
    /// Restarts the service
    Restart { name: String },
    /// Enables the service at boot
    Enable { name: String },
    /// Disables the service at boot
    Disable { name: String },
    /// Deletes the systemd-managed session
    Delete { name: String },
}

#[derive(Subcommand)]
pub enum DaemonCommands {
    /// Runs the daemon (in the foreground)
    Start,
    /// Stops the daemon and waits until it has exited
    Stop,
    /// Exits with code 0 if the daemon is running, 1 otherwise
    Status,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Runs a CLI command. `daemon start` is not handled here, because it runs the
/// daemon itself and belongs to whatever owns the daemon entry point (main).
pub async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Run {
            systemd,
            name,
            command,
        } => handle_run(systemd, name, command).await,
        Commands::List { names } => handle_list_sessions(names).await,
        Commands::Attach { name } => handle_attach_session(name).await,
        Commands::Kill { name, all } => match (name, all) {
            (_, true) => handle_kill_all_sessions().await,
            (Some(name), false) => handle_kill_session(name).await,
            (None, false) => Err(eyre!("Specify a session name or --all")),
        },
        Commands::Service { action } => handle_service(action).await,
        Commands::Export { name, dir } => handle_export_session(name, dir).await,
        Commands::Import { file } => handle_import(file).await,
        Commands::Daemon { action } => match action {
            DaemonCommands::Start => Err(eyre!("`daemon start` must be handled by the caller")),
            DaemonCommands::Stop => handle_daemon_stop().await,
            DaemonCommands::Status => handle_daemon_status(),
        },
        Commands::InternalSystemdStart { config } => handle_internal_systemd_start(config).await,
        Commands::InternalSystemdStop { name } => handle_internal_systemd_stop(name).await,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn generate_random_name() -> String {
    let names = ["henk", "bink", "aardappel", "piet", "jannes"];
    names[rand::random_range(0..names.len())].to_string()
}

/// Quote a single argument so a shell reads it back as one word.
fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));

    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// Turn argv into one command line for the terminal.
fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
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
        .args(["daemon", "start"])
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
// General commands
// ---------------------------------------------------------------------------

async fn handle_run(systemd: bool, name: Option<String>, command: Vec<String>) -> Result<()> {
    ensure_daemon_running().await?;
    let rpc = connect().await?;

    // Check for duplicate names.
    if let Some(ref name) = name {
        let sessions = list_sessions(&rpc).await?;

        if sessions.iter().any(|session| &session.name == name) {
            return Err(eyre!("A session named '{}' already exists", name));
        }
    }

    let name = name.unwrap_or_else(generate_random_name);
    let command = shell_join(&command);

    // The daemon has its own working directory, so send ours explicitly.
    let working_dir = std::env::current_dir()?.to_string_lossy().into_owned();

    let kind = if systemd {
        "systemd service"
    } else {
        "session"
    };
    eprintln!("Starting {kind}: {name} (running: {command})");

    let sessions = if systemd {
        let sessions = rpc
            .create_sysd_session(working_dir, Some(command), Some(name.clone()))
            .await?;
        Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg(format!("{}.service", name))
            .output()
            .unwrap();
        sessions
    } else {
        rpc.create_session(working_dir, Some(command), Some(name.clone()))
            .await?
    };

    // let session = sessions
    //     .iter()
    //     .find(|session| session.name == name)
    //     .ok_or_else(|| eyre!("Failed to find newly created {kind} in list"))?;

    // println!("Created {kind}: {}", session.name);

    Ok(())
}

async fn handle_list_sessions(names_only: bool) -> Result<()> {
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

async fn handle_attach_session(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    let session_info = rpc.open_session(id).await?;

    println!(
        "Attached to session: {} (window with terminal running)",
        session_info.name
    );

    Ok(())
}

async fn handle_kill_session(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    rpc.kill_session(id).await?;

    println!("Killed session: {name}");

    Ok(())
}

async fn handle_kill_all_sessions() -> Result<()> {
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

// ---------------------------------------------------------------------------
// Service commands (systemd-based sessions)
// ---------------------------------------------------------------------------

async fn handle_service(action: ServiceCommands) -> Result<()> {
    match action {
        ServiceCommands::Delete { name } => {
            let mut rpc = connect().await?;
            let id = find_session_id(&rpc, &name).await?;

            rpc.delete_sysd_session(id).await?;

            println!("Deleted service: {name}");
            Ok(())
        }
        action => {
            let (name, sysd_action, past_tense) = match action {
                ServiceCommands::Start { name } => (name, SysdAction::Start, "Started"),
                ServiceCommands::Stop { name } => (name, SysdAction::Stop, "Stopped"),
                ServiceCommands::Restart { name } => (name, SysdAction::Restart, "Restarted"),
                ServiceCommands::Enable { name } => (name, SysdAction::Enable, "Enabled"),
                ServiceCommands::Disable { name } => (name, SysdAction::Disable, "Disabled"),
                ServiceCommands::Delete { .. } => unreachable!(),
            };

            let rpc = connect().await?;
            let id = find_session_id(&rpc, &name).await?;

            // The daemon rejects this for non-systemd sessions.
            rpc.execute_sysd_action(id, sysd_action).await?;

            println!("{past_tense} service: {name}");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Export / import (ephemeral sessions)
// ---------------------------------------------------------------------------

async fn handle_export_session(name: String, dir: Option<PathBuf>) -> Result<()> {
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

async fn handle_import(file: PathBuf) -> Result<()> {
    ensure_daemon_running().await?;
    let rpc = connect().await?;

    let absolute_path = fs::canonicalize(&file)
        .map_err(|e| eyre!("Failed to get absolute path for {}: {e}", file.display()))?;

    if !absolute_path.is_file() {
        return Err(eyre!(
            "Config path is not a file: {}",
            absolute_path.display()
        ));
    }

    eprintln!("Starting session from config file: {}", file.display());

    rpc.start_session_from_file(absolute_path).await?;

    println!("Session started successfully from {}", file.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Daemon commands
// ---------------------------------------------------------------------------

async fn handle_daemon_stop() -> Result<()> {
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
fn handle_daemon_status() -> ! {
    if daemon_running() {
        println!("Daemon running");
        std::process::exit(0);
    } else {
        println!("Daemon not running");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Internal commands (invoked by systemd)
// ---------------------------------------------------------------------------

/// Starts a systemd-managed session from a config file.
async fn handle_internal_systemd_start(file_path: PathBuf) -> Result<()> {
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
async fn handle_internal_systemd_stop(name: String) -> Result<()> {
    let rpc = connect().await?;
    let id = find_session_id(&rpc, &name).await?;

    println!("Stopping systemd session: {name}");

    rpc.stop_sysd_session(id).await?;

    println!("Systemd session stopped successfully: {name}");

    Ok(())
}
