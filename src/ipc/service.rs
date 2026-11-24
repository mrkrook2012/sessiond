use std::path::PathBuf;

use crate::backend::session_manager::{RunId, SessionId};
use crate::backend::terminal_manager::TerminalId;
use crate::backend::window_manager::WindowId;
use crate::ipc::protocol::{SessionInfo, SysdAction, SysdWaitStatus, TerminalUpdate};

#[tarpc::service]
pub trait DaemonService {
    /// Retrieve the currently selected terminal data.
    async fn get_wanted_data(terminal_id: TerminalId) -> TerminalUpdate;

    /// Initialize the daemon connection.
    async fn start(rows: u16, cols: u16);

    /// Update terminal dimensions.
    async fn update_screen_size(rows: u16, cols: u16);

    /// Create a new ephemeral session.
    async fn create_session(
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo>;

    /// Create a new systemd-backed session.
    async fn create_sysd_session(
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo>;

    /// Delete a systemd session by name: disables and stops the unit, and removes its
    /// `.service` and `.yaml` files. Returns the removed files.
    async fn delete_sysd_session(name: String) -> Result<Vec<PathBuf>, String>;

    /// Stop sending terminal updates for the current session.
    async fn exit_session() -> Vec<SessionInfo>;

    /// Kill a session.
    async fn kill_session(id: SessionId) -> Vec<SessionInfo>;

    /// Write bytes to a terminal.
    async fn write_terminal(terminal_id: TerminalId, data: Vec<u8>);

    /// List all sessions.
    async fn list_sessions() -> Vec<SessionInfo>;

    /// Create a window in a session.
    async fn create_window(id: SessionId) -> SessionInfo;

    /// Open a session.
    async fn open_session(id: SessionId) -> Result<SessionInfo, String>;

    /// Change the currently selected window.
    async fn change_window(window_id: WindowId);

    /// Kill a window.
    async fn kill_window(id: SessionId, window_id: WindowId) -> Result<SessionInfo, String>;

    /// Export a session as a systemd service.
    async fn export_session_to_sysd(id: SessionId) -> Result<(), String>;

    /// Start a session from a YAML file.
    async fn start_session_from_file(file_path: PathBuf) -> Result<(), String>;

    /// Stop the daemon.
    async fn kill_daemon();

    /// Export a session to a YAML file.
    async fn export_session_to_file(id: SessionId, working_dir: PathBuf)
    -> Result<PathBuf, String>;

    /// Execute a systemd action on a session.
    async fn execute_sysd_action(
        id: SessionId,
        sysd_action: SysdAction,
    ) -> Result<Vec<SessionInfo>, String>;

    /// Start a systemd session from a YAML file. Returns the id of this run.
    async fn start_sysd_session_from_file(file_path: PathBuf) -> Result<RunId, String>;

    /// Waits up to `timeout_ms` for run `run` of sysd session `name` to end, or for one of
    /// its tracked commands to exit. Returns `Running` on timeout.
    async fn wait_sysd_session(
        name: String,
        run: RunId,
        timeout_ms: u64,
    ) -> Result<SysdWaitStatus, String>;

    /// Stop a systemd session.
    async fn stop_sysd_session(id: SessionId) -> Result<(), String>;
}
