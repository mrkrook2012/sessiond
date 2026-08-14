use bincode_next::{Decode, Encode};

use crate::backend::{
    session_manager::Session,
    tree::{Node, SplitDirection},
};

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

    // Reserved for manual terminal creation
    CreateTerminal {
        start_terminal_id: u64,
        split_type: SplitDirection,
        working_dir: String,
        command: Option<String>,
    },

    // Resize by dragging, resizes due to opening/closing are calculated
    ResizeTerminal {
        expand_terminal_id: u64,
        shrink_terminal_id: u64,
        percent: f32,
    },

    KillTerminal {
        terminal_id: u64,
    },

    WriteTerminal {
        terminal_id: u64,
        data: Vec<u8>,
    },
}

#[derive(Decode, Encode, Debug)]
pub enum ServerMessage {
    Ok,
    Pong,
    Started,
    CreatedWindow {
        window_id: u64,
        terminal_id: u64,
    },

    // TODO ChangedWindow message that returns the layout with terminal_ids
    CreatedSession {
        session_info: SessionInfo,
    },
    ListedSessions {
        sessions: Vec<SessionInfo>,
    },
    CreatedTerminal {
        terminal_id: u64,
        new_layout: Node,
    },
    TerminalData {
        terminals: Vec<TerminalUpdate>,
    },
    // First window id is current window
    OpenedSession {
        session_info: SessionInfo,
    },
}

#[derive(Decode, Encode, Debug)]
pub struct SessionInfo {
    pub session_id: u64,
    pub session_name: String,
    pub windows_info: Vec<WindowInfo>,
}

#[derive(Decode, Encode, Debug)]
pub struct WindowInfo {
    pub window_id: u64,
    pub window_name: String,
    pub layout: Node,
}

impl WindowInfo {
    pub fn new(window_id: u64, window_name: String, layout: Node) -> Self {
        Self { window_id, window_name, layout }
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
