use bincode_next::{Decode, Encode};
use serde::Serialize;
use std::path::PathBuf;

use crate::backend::{
    session_manager::SessionId,
    terminal_manager::TerminalId,
    window_manager::WindowId,
};

// TODO remove SessionId from protocol, should be string, we'll parse it later.
#[derive(Decode, Encode, Debug, Serialize, serde::Deserialize)]
pub enum ClientMessage {
    Ping,

    // Gives the screen size, we store, then base terminal off it.
    // Returns UserId
    // TEMP Starts a window with terminal for testing speed
    Start {
        rows: u16,
        cols: u16,
    },

    UpdateScreenSize {
        rows: u16,
        cols: u16,
    },

    CreateWindow {
        id: SessionId,
    },
    KillWindow {
        id: SessionId,
        window_id: WindowId,
    },
    ChangeWindow {
        window_id: WindowId,
    },

    // Is the start command, will start a session with 1 terminal running shell
    CreateSession {
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    },
    OpenSession {
        id: SessionId,
    },
    ListSessions,
    ExitSession,
    KillSession {
        id: SessionId,
    },

    ExportSessionToSysd {
        id: SessionId,
    },

    ExportSessionToFile {
        id: SessionId,
        working_dir: PathBuf,
    },

    ExecuteSysdActionSession {
        id: SessionId,
        sysd_action: SysdAction,
    },

    StartSessionFromFile {
        file_path: PathBuf,
    },

    StartSysdSessionFromFile {
        file_path: PathBuf,
    },

    StopSysdSession {
        id: SessionId,
    },

    // Reserved for manual terminal creation
    // Should be unused, create new window is only way.
    CreateTerminal {
        start_terminal_id: TerminalId,
        working_dir: String,
        command: Option<String>,
    },

    // Should not be used, kill the window instead.
    KillTerminal {
        id: SessionId,
        terminal_id: TerminalId,
    },

    WriteTerminal {
        terminal_id: TerminalId,
        data: Vec<u8>,
    },

    /// Kill the daemon process
    KillDaemon,
    // Useful for cli
}

/// Answer to `session_id_by_name`.
#[derive(Decode, Encode, Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub enum SessionLookup {
    /// `name` is the session's own name, which may differ in case from what was asked for.
    Found { id: SessionId, name: String },
    /// No session has this name. `similar` holds names that look like it, best match first
    /// (empty if nothing comes close).
    NotFound { similar: Vec<String> },
}

/// Answer to `wait_sysd_session`: what happened to one run of a sysd session.
#[derive(Decode, Encode, Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
pub enum SysdWaitStatus {
    /// Nothing happened before the wait timed out; ask again.
    Running,
    /// A tracked command exited on its own. The unit should fail so systemd restarts it.
    TrackedExited { code: i32 },
    /// The run is over: the session was stopped, killed, deleted or restarted.
    Ended,
}

#[derive(Decode, Encode, Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum SysdAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

#[derive(Decode, Encode, Debug, Clone, Serialize, serde::Deserialize)]
pub enum ServerMessage {
    Ok,
    Pong,
    Started,
    Error { error: String },
    UpdatedSession { session_info: SessionInfo },

    // This will be returned from a list sessions, and later change names, stop restart ect
    // but also if you kill the last terminal, window or just kill a session
    ListedSessions { sessions: Vec<SessionInfo> },
    ExportedSessionToFile { path: PathBuf },
    TerminalData { terminals: Vec<TerminalUpdate> },
}

// #[derive(Decode, Encode, Debug, Clone)]
// pub struct ExportSessionInfo {
//     pub name: String,
//     pub windows_info: Vec<WindowInfo>,
// }

// #[derive(Decode, Encode, Debug, Clone)]
// pub struct ExportWindowInfo {
//     pub working_dir: String,
//     pub command: String,
// }

#[derive(Decode, Encode, Debug, Clone, Serialize, serde::Deserialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub windows_info: Vec<WindowInfo>,
    pub sysd_data: Option<SysdData>, // maybe improve later
}

#[derive(Decode, Encode, Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct SysdData {
    pub enabled: bool,
    pub running: bool,
}

#[derive(Decode, Encode, Debug, Clone, Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub window_id: WindowId,
    pub terminal_id: TerminalId,
}

impl WindowInfo {
    pub fn new(window_id: WindowId, terminal_id: TerminalId) -> Self {
        Self {
            window_id,
            terminal_id,
        }
    }
}

impl SessionInfo {
    pub fn new(
        id: SessionId,
        name: String,
        windows_info: Vec<WindowInfo>,
        sysd_data: Option<SysdData>,
    ) -> Self {
        Self {
            id,
            name,
            windows_info,
            sysd_data,
        }
    }
}

#[derive(Decode, Encode, Debug, Clone, Serialize, serde::Deserialize)]
pub struct TerminalUpdate {
    pub terminal_id: TerminalId,
    /// Escape codes that draw the whole screen (and set its input modes, like application
    /// cursor keys) from scratch, at `rows` x `cols`. Empty if the screen has not changed
    /// since the generation the client asked with.
    pub data: Vec<u8>,
    pub rows: u16,
    pub cols: u16,
    /// How many lines back from the bottom the screen is scrolled, clamped to the history
    /// there is. 0 is the live screen.
    pub scrollback: u32,
    /// True while a full-screen program (btop, vim, less) has the alternate screen, which
    /// has no history to scroll through. Not part of `data`: redrawing the contents does
    /// not switch screens.
    pub alternate_screen: bool,
    /// Bumped whenever the screen changes. Send it back as `since` to skip unchanged screens.
    pub generation: u64,
}
