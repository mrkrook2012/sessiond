//! Logging setup. Everything logs through `tracing`; this decides what gets printed.
//!
//! By default only warnings and errors are shown. `-v` adds info, `-vv` debug, `-vvv`
//! trace; `-q` shows errors only. `RUST_LOG` (e.g. `RUST_LOG=sessiond=debug,tarpc=info`)
//! overrides the flags completely.
//!
//! Where logs go:
//! - CLI commands: stderr, so command output on stdout stays clean. Under systemd
//!   (`__internal-sysd-*`) that is the journal.
//! - The daemon: stderr. A daemon started in the background has its stdout and stderr
//!   redirected to `daemon.log` in the log directory, so panics end up there too.
//! - The TUI: `frontend.log` in the log directory, since it owns the terminal.
//!
//! The log directory is `$SESSIOND_LOG_DIR`, else `$XDG_STATE_HOME/sessiond`, else
//! `~/.local/state/sessiond`. Each log file is rotated to `<name>.old` when a new
//! process starts writing it, so the previous run's log survives one restart.

use std::fs::{self, File};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::OnceLock;

use color_eyre::Result;
use color_eyre::eyre::eyre;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

/// Overrides the directory log files are written to.
pub const LOG_DIR_ENV_VAR: &str = "SESSIOND_LOG_DIR";

/// Log file of a daemon started in the background.
pub const DAEMON_LOG: &str = "daemon.log";

/// Log file of the TUI.
pub const FRONTEND_LOG: &str = "frontend.log";

/// The level a daemon spawned by this process should use. Normally the level this
/// process logs at.
pub static LEVEL_FOR_CHILDREN: OnceLock<LevelFilter> = OnceLock::new();

/// The directory log files live in, created if it does not exist yet.
pub fn log_dir() -> Result<PathBuf> {
    let dir = if let Some(dir) = std::env::var_os(LOG_DIR_ENV_VAR) {
        PathBuf::from(dir)
    } else if let Some(state) = std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        PathBuf::from(state).join(env!("CARGO_PKG_NAME"))
    } else {
        let home = std::env::var_os("HOME").ok_or_else(|| eyre!("HOME is not set"))?;
        PathBuf::from(home).join(".local/state").join(env!("CARGO_PKG_NAME"))
    };

    fs::create_dir_all(&dir).map_err(|e| eyre!("Cannot create log directory {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Opens `name` in the log directory for writing, moving the previous log to `<name>.old`.
/// Returns the file and its path.
pub fn open_log_file(name: &str) -> Result<(File, PathBuf)> {
    let path = log_dir()?.join(name);
    let _ = fs::rename(&path, path.with_extension("log.old"));
    let file = File::create(&path).map_err(|e| eyre!("Cannot create log file {}: {e}", path.display()))?;
    Ok((file, path))
}

/// Logs to stderr.
pub fn init(level: LevelFilter) {
    let _ = LEVEL_FOR_CHILDREN.set(level);
    if level == LevelFilter::OFF {
        return;
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter(level))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init();
}

/// Logs to `name` in the log directory, for processes that cannot use stderr (the TUI).
/// Returns the log file's path.
pub fn init_file(level: LevelFilter, name: &str) -> Result<PathBuf> {
    let _ = LEVEL_FOR_CHILDREN.set(level);
    let (file, path) = open_log_file(name)?;
    if level == LevelFilter::OFF {
        return Ok(path);
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter(level))
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .try_init();
    Ok(path)
}

/// Our own crate at the requested level. Dependencies (tarpc, zbus) are noisy, so they
/// only show warnings unless asked for with RUST_LOG.
fn filter(level: LevelFilter) -> EnvFilter {
    let default_filter = format!("warn,{}={level}", env!("CARGO_CRATE_NAME"));
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter))
}

/// Command line flags that reproduce this process' log level, for spawning the daemon.
/// `RUST_LOG` needs no flag: child processes inherit it.
pub fn verbosity_args() -> Vec<&'static str> {
    let level = LEVEL_FOR_CHILDREN.get().copied().unwrap_or(LevelFilter::WARN);

    let flag = if level == LevelFilter::OFF {
        "-qq"
    } else if level == LevelFilter::ERROR {
        "-q"
    } else if level == LevelFilter::INFO {
        "-v"
    } else if level == LevelFilter::DEBUG {
        "-vv"
    } else if level == LevelFilter::TRACE {
        "-vvv"
    } else {
        return Vec::new();
    };

    vec![flag]
}
