use bincode_next::{Decode, Encode};
use std::path::PathBuf;

#[derive(Decode, Encode, Debug)]
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
        session_id: u64,
    },
    KillWindow {
        window_id: u64,
    },
    ChangeWindow {
        window_id: u64,
    },

    // Is the start command, will start a session with 1 terminal running shell
    CreateSession {
        working_dir: String,
        command: Option<String>,
        session_name: Option<String>,
    },
    OpenSession {
        session_id: u64,
    },
    ListSessions,
    ExitSession,
    KillSession {
        session_id: u64,
    },

    ExportSession {
        session_id: u64,
    },

    StartSessionFromFile{
        file_path: PathBuf,
    },

    // Reserved for manual terminal creation
    // Should be unused, create new window is only way.
    CreateTerminal {
        start_terminal_id: u64,
        working_dir: String,
        command: Option<String>,
    },

    // Should not be used, kill the window instead.
    KillTerminal {
        terminal_id: u64,
    },

    WriteTerminal {
        terminal_id: u64,
        data: Vec<u8>,
    },
    
    /// Kill the daemon process
    KillDaemon,
}

#[derive(Decode, Encode, Debug)]
pub enum ServerMessage {
    Ok,
    Pong,
    Started,
    Error{error: String},
    UpdatedSession { session_info: SessionInfo },

    // This will be returned from a list sessions, and later change names, stop restart ect
    // but also if you kill the last terminal, window or just kill a session
    ListedSessions { sessions: Vec<SessionInfo> },
    TerminalData { terminals: Vec<TerminalUpdate> },
}

// #[derive(Decode, Encode, Debug, Clone)]
// pub struct ExportSessionInfo {
//     pub session_name: String,
//     pub windows_info: Vec<WindowInfo>,
// }

// #[derive(Decode, Encode, Debug, Clone)]
// pub struct ExportWindowInfo {
//     pub working_dir: String,
//     pub command: String,
// }

#[derive(Decode, Encode, Debug, Clone)]
pub struct SessionInfo {
    pub session_id: u64,
    pub session_name: String,
    pub windows_info: Vec<WindowInfo>,
}

#[derive(Decode, Encode, Debug, Clone)]
pub struct WindowInfo {
    pub window_id: u64,
    pub terminal_id: u64,
}

impl WindowInfo {
    pub fn new(window_id: u64, terminal_id: u64) -> Self {
        Self {
            window_id,
            terminal_id,
        }
    }
}

impl SessionInfo {
    pub fn new(session_id: u64, session_name: String, windows_info: Vec<WindowInfo>) -> Self {
        Self {
            session_id,
            session_name,
            windows_info,
        }
    }
}

#[derive(Decode, Encode, Debug)]
pub struct TerminalUpdate {
    pub terminal_id: u64,
    pub data: Vec<u8>,
}
