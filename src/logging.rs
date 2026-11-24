//! Logging setup. Everything logs through `tracing`; this decides what gets printed.
//!
//! By default only warnings and errors are shown. `-v` adds info, `-vv` debug, `-vvv`
//! trace; `-q` shows errors only. `RUST_LOG` (e.g. `RUST_LOG=daemon_test=debug,tarpc=info`)
//! overrides the flags completely.
//!
//! Logs go to stderr, so command output on stdout stays clean. For a daemon started in
//! the background that is `daemon.err`; for systemd units, the journal.

use std::io::IsTerminal;
use std::sync::OnceLock;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

/// The level a daemon spawned by this process should use. Normally the level this
/// process logs at; the TUI logs nothing itself but still sets this.
pub static LEVEL_FOR_CHILDREN: OnceLock<LevelFilter> = OnceLock::new();

pub fn init(level: LevelFilter) {
    let _ = LEVEL_FOR_CHILDREN.set(level);
    if level == LevelFilter::OFF {
        return;
    }

    // Our own crate at the requested level. Dependencies (tarpc, zbus) are noisy, so
    // they only show warnings unless asked for with RUST_LOG.
    let default_filter = format!("warn,{}={level}", env!("CARGO_CRATE_NAME"));
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init();
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
