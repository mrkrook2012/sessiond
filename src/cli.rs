use clap::{Parser, Subcommand};
use color_eyre::Result;
use color_eyre::eyre::eyre;
use tracing::info;

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::backend::session::SessionId;
use crate::ipc::client::DaemonClient;
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::{SessionInfo, SessionLookup, SysdAction, SysdWaitStatus};

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

    /// -v info, -vv debug, -vvv trace; -q errors only. RUST_LOG overrides this.
    #[command(flatten)]
    pub verbose: clap_verbosity_flag::Verbosity<clap_verbosity_flag::WarnLevel>,
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

        /// The command to run. A single argument is a shell command line, so
        /// `run "make && ./serve"` works; several arguments are a command and its
        /// arguments, passed on exactly as given (`run sh -c 'sleep 300'`).
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Lists all active sessions
    List {
        /// Print only the names, one per line
        #[arg(long)]
        names: bool,

        /// Only list systemd-based sessions
        #[arg(long)]
        services: bool,
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

        #[arg(short, long, value_hint = clap::ValueHint::DirPath)]
        dir: Option<PathBuf>,
    },
    /// Starts an ephemeral session from a config file
    Import {
        #[arg(value_hint = clap::ValueHint::FilePath)]
        file: PathBuf,
    },
    /// Manages the daemon process
    Daemon {
        #[command(subcommand)]
        action: DaemonCommands,
    },
    /// Prints a shell completion script.
    ///
    /// fish: daemon_test completions fish > ~/.config/fish/completions/daemon_test.fish
    Completions { shell: CompletionShell },

    // Internal commands, invoked by the generated systemd units.
    #[command(name = "__internal-systemd-start", hide = true)]
    InternalSystemdStart { config: PathBuf },
    #[command(name = "__internal-systemd-stop", hide = true)]
    InternalSystemdStop { name: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum CompletionShell {
    Fish,
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
    /// Saves the running service's current windows to its session file, so the next
    /// (re)start brings them back
    Update { name: String },
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
        Commands::List { names, services } => handle_list_sessions(names, services).await,
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
        Commands::Completions { shell } => {
            print_completions(shell);
            Ok(())
        }
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
pub fn shell_quote(arg: &str) -> String {
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

/// The command line `run` types into the terminal.
///
/// One argument already is a command line (`run "echo hello && echo world"`), so it is used
/// as is: quoting it would make the shell look for a program with that whole name. Several
/// arguments are a command and its arguments, so each is quoted to reach it unchanged.
pub fn command_line(args: &[String]) -> String {
    match args {
        [line] => line.clone(),
        args => shell_join(args),
    }
}

/// Turn argv into one command line for the terminal.
pub fn shell_join(args: &[String]) -> String {
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

/// Find a session by name. Returns its id and its own name (which may differ in case from
/// `name`). If there is no such session, the error suggests similar names.
async fn find_session(rpc: &DaemonClient, name: &str) -> Result<(SessionId, String)> {
    match rpc.session_id_by_name(name).await? {
        SessionLookup::Found { id, name } => Ok((id, name)),
        SessionLookup::NotFound { similar } => Err(eyre!(session_not_found_message(name, &similar))),
    }
}

/// `Session not found: 'x'`, followed by a hint: the similar names, or how to list sessions.
pub fn session_not_found_message(name: &str, similar: &[String]) -> String {
    let hint = match similar {
        [] => format!("Run `{} list` to see all sessions.", env!("CARGO_PKG_NAME")),
        [only] => format!("Did you mean '{only}'?"),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(|name| format!("'{name}'")).collect();
            format!("Did you mean {} or '{last}'?", rest.join(", "))
        }
    };
    format!("Session not found: '{name}'\n{hint}")
}

/// Start the daemon in the background if necessary and wait until it accepts
/// connections. Progress messages go to stderr.
pub async fn ensure_daemon_running() -> Result<()> {
    if daemon_running() {
        return Ok(());
    }

    info!("Daemon not running, starting it");

    let log_dir = std::env::var_os(LOG_DIR_ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let stdout = File::create(log_dir.join("daemon.out"))?;
    let stderr = File::create(log_dir.join("daemon.err"))?;

    let mut child = Command::new(std::env::current_exe()?)
        .args(["daemon", "start"])
        .args(crate::logging::verbosity_args())
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

    info!("Daemon started");
    Ok(())
}

// ---------------------------------------------------------------------------
// General commands
// ---------------------------------------------------------------------------

async fn handle_run(systemd: bool, name: Option<String>, command: Vec<String>) -> Result<()> {
    ensure_daemon_running().await?;
    let rpc = connect().await?;

    let name = name.unwrap_or_else(generate_random_name);
    let command = command_line(&command);

    // The daemon has its own working directory, so send ours explicitly.
    let working_dir = std::env::current_dir()?.to_string_lossy().into_owned();

    let kind = if systemd {
        "systemd service"
    } else {
        "session"
    };
    info!("Starting {kind} '{name}' running: {command}");

    if systemd {
        rpc.create_sysd_session(working_dir, Some(command), Some(name.clone()))
            .await?;

        // The daemon starts the unit through systemd; its ExecStart then spawns the windows.
        let (id, _) = find_session(&rpc, &name).await?;
        rpc.execute_sysd_action(id, SysdAction::Start).await?;
    } else {
        rpc.create_session(working_dir, Some(command), Some(name))
            .await?;
    }

    Ok(())
}

async fn handle_list_sessions(names_only: bool, services_only: bool) -> Result<()> {
    let rpc = connect().await?;
    let mut sessions = list_sessions(&rpc).await?;

    if services_only {
        sessions.retain(|session| session.sysd_data.is_some());
    }

    if names_only {
        for session in sessions {
            println!("{}", session.name);
        }
    } else if sessions.is_empty() {
        println!("No active sessions.");
    } else {
        println!("Active sessions:");

        for session in sessions {
            match session.sysd_data {
                Some(sysd) => println!(
                    "- {} [service, {}, {}]",
                    session.name,
                    if sysd.running { "running" } else { "stopped" },
                    if sysd.enabled { "enabled" } else { "disabled" },
                ),
                None => println!("- {}", session.name),
            }
        }
    }

    Ok(())
}

async fn handle_attach_session(name: String) -> Result<()> {
    let rpc = connect().await?;
    let (id, _) = find_session(&rpc, &name).await?;

    let session_info = rpc.open_session(id).await?;

    println!(
        "Attached to session: {} (window with terminal running)",
        session_info.name
    );

    Ok(())
}

async fn handle_kill_session(name: String) -> Result<()> {
    let rpc = connect().await?;
    let (id, name) = find_session(&rpc, &name).await?;

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
            let rpc = connect().await?;

            // By name, so leftovers the daemon never registered can be deleted too.
            let removed = rpc.delete_sysd_session(name.clone()).await?;

            println!("Deleted service: {name}");
            for file in removed {
                println!("  removed {}", file.display());
            }
            Ok(())
        }
        ServiceCommands::Update { name } => {
            let rpc = connect().await?;
            let (id, name) = find_session(&rpc, &name).await?;

            let path = rpc.update_sysd_session(id).await?;

            println!("Updated service: {name}");
            println!("  wrote {}", path.display());
            Ok(())
        }
        action => {
            let (name, sysd_action, past_tense) = match action {
                ServiceCommands::Start { name } => (name, SysdAction::Start, "Started"),
                ServiceCommands::Stop { name } => (name, SysdAction::Stop, "Stopped"),
                ServiceCommands::Restart { name } => (name, SysdAction::Restart, "Restarted"),
                ServiceCommands::Enable { name } => (name, SysdAction::Enable, "Enabled"),
                ServiceCommands::Disable { name } => (name, SysdAction::Disable, "Disabled"),
                ServiceCommands::Delete { .. } | ServiceCommands::Update { .. } => unreachable!(),
            };

            let rpc = connect().await?;
            let (id, name) = find_session(&rpc, &name).await?;

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

    let (id, name) = find_session(&rpc, &name).await?;

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

    info!("Starting session from config file {}", file.display());

    rpc.start_session_from_file(absolute_path).await?;

    println!("Session started successfully from {}", file.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Shell completions
// ---------------------------------------------------------------------------

/// Name the completions are registered for: the installed binary.
const BIN_NAME: &str = "daemon_test";

/// The generated script covers subcommands and flags; `FISH_DYNAMIC` adds what clap
/// cannot know: live session names, and the command wrapped by `run`.
fn print_completions(shell: CompletionShell) {
    use clap::CommandFactory;
    use std::io::Write;

    let mut stdout = std::io::stdout();
    match shell {
        CompletionShell::Fish => {
            let mut generated = Vec::new();
            clap_complete::generate(
                clap_complete::Shell::Fish,
                &mut Cli::command(),
                BIN_NAME,
                &mut generated,
            );

            // clap_complete also offers hidden subcommands; the internal ones are for systemd only.
            for line in String::from_utf8_lossy(&generated).lines() {
                if !line.contains("-a \"__internal-") {
                    let _ = writeln!(stdout, "{line}");
                }
            }
            let _ = write!(stdout, "{FISH_DYNAMIC}");
        }
    }
}

const FISH_DYNAMIC: &str = include_str!("completions/dynamic.fish");

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

    info!("Starting systemd session from {}", absolute_path.display());

    let yaml_name = session_name_from_config(&absolute_path)?;
    let run = rpc.start_sysd_session_from_file(absolute_path).await?;

    info!("Systemd session '{yaml_name}' started, waiting for it to end");

    // Stay alive for as long as the session runs: this process is the unit's main process.
    // Exiting with an error makes systemd (Restart=on-failure) restart the unit.
    loop {
        let status = match rpc
            .wait_sysd_session(yaml_name.clone(), run, Duration::from_secs(30))
            .await
        {
            Ok(status) => status,
            // `daemon stop` removes the socket before exiting; a crash leaves it behind.
            Err(_) if !socket_path().exists() => {
                info!("Daemon was stopped, stopping '{yaml_name}'");
                return Ok(());
            }
            // The daemon died: an error, so systemd restarts us (and with us, the daemon).
            Err(e) => return Err(e),
        };

        match status {
            SysdWaitStatus::Running => continue,
            SysdWaitStatus::TrackedExited { code } => {
                return Err(eyre!(
                    "Tracked command in '{yaml_name}' exited with code {code}, restarting"
                ));
            }
            SysdWaitStatus::Ended => {
                info!("Systemd session '{yaml_name}' ended");
                return Ok(());
            }
        }
    }
}

/// The session name as the daemon stores it (the unit name, lowercased).
pub fn session_name_from_config(path: &std::path::Path) -> Result<String> {
    let yaml: crate::backend::storage::session_files::SessionConfig =
        serde_yaml::from_str(&fs::read_to_string(path)?)?;
    Ok(crate::backend::systemd::UnitName::new(&yaml.name)?.into_string())
}

/// Stops a systemd-managed session. Does not start the daemon.
/// A missing daemon or session is not an error: there is nothing left to stop,
/// and failing here would leave the unit in the `failed` state.
async fn handle_internal_systemd_stop(name: String) -> Result<()> {
    if !daemon_running() {
        info!("Daemon not running, nothing to stop for '{name}'");
        return Ok(());
    }

    let rpc = connect().await?;
    let Ok((id, name)) = find_session(&rpc, &name).await else {
        info!("No session named '{name}', nothing to stop");
        return Ok(());
    };

    info!("Stopping systemd session '{name}'");

    rpc.stop_sysd_session(id).await?;

    info!("Systemd session '{name}' stopped");

    Ok(())
}
