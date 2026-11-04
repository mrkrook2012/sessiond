use bincode_next::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::backend::{
    session_manager::{SessionId, SessionState},
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

#[derive(Decode, Encode, Debug, Serialize, serde::Deserialize)]
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

#[derive(Decode, Encode, Debug, Clone, Serialize, serde::Deserialize)]
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
    pub data: Vec<u8>,
}
