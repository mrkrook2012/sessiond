use crate::backend::{
    mini_user_manager::MiniUserManager, session_manager::SessionManager,
    terminal_manager::TerminalManager, window_manager::WindowManager,
};
use crate::ipc::protocol::ServerMessage::Ok;
use crate::ipc::protocol::*;

pub struct DaemonManager {
    user_manager: MiniUserManager,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
}

impl DaemonManager {
    pub fn new(
        user_manager: MiniUserManager,
        terminal_manager: TerminalManager,
        window_manager: WindowManager,
        session_manager: SessionManager,
    ) -> Self {
        Self {
            user_manager,
            terminal_manager,
            window_manager,
            session_manager,
        }
    }

    pub fn get_wanted_data(&mut self) -> ServerMessage {
        let terminals = self.user_manager.get_all_terminals();
        let mut terminal_data = Vec::new();
        for terminal_id in terminals {
            let data = self.terminal_manager.get_data_from_terminal(terminal_id);
            terminal_data.push(data);
        }
        ServerMessage::TerminalData {
            terminals: terminal_data,
        }
    }

    pub fn process_message(&mut self, client_message: ClientMessage) -> ServerMessage {
        match client_message {
            ClientMessage::Ping => ServerMessage::Pong,
            ClientMessage::Start { rows, cols } => self.handle_start(rows, cols),
            ClientMessage::UpdateScreenSize { rows: _, cols: _ } => todo!(),
            ClientMessage::CreateSession {
                working_dir,
                command,
                session_name,
            } => self.handle_create_session(working_dir, command, session_name),
            ClientMessage::ExitSession => todo!(),
            ClientMessage::KillSession => todo!(),
            ClientMessage::CreateTerminal {
                start_terminal_id: _,
                split_type: _,
                working_dir: _,
                command: _,
            } => todo!(),
            ClientMessage::ResizeTerminal {
                expand_terminal_id: _,
                shrink_terminal_id: _,
                percent: _,
            } => todo!(),
            ClientMessage::KillTerminal { terminal_id: _ } => todo!(),
            ClientMessage::WriteTerminal { terminal_id, data } => {
                self.handle_write(terminal_id, data)
            }
            ClientMessage::ListSessions {} => self.handle_list_sessions(),
            ClientMessage::CreateWindow { session_id } => todo!(),
            ClientMessage::OpenSession { session_id } => todo!(),
            ClientMessage::ChangeWindow { window_id } => todo!(),
            ClientMessage::KillWindow { window_id } => todo!(),
        }
    }

    fn handle_open_session(&mut self, session_id: u64) -> ServerMessage {
        // Get sessions
        // Get first window in session
        // get terminals in first window
        // Add terminals to user setting
        // Update thread will send data based on user setting.
        let ids = self
            .session_manager
            .get_window_ids_by_session_id(session_id);
        let window_id = ids.first().unwrap();
        let terminals = self.window_manager.get_terminals_in_window(*window_id);

        for terminal_id in &terminals {
            self.user_manager.add_terminal(terminal_id.clone());
        }
        let first_terminal = terminals.first().unwrap();
        ServerMessage::OpenedSession {
            window_ids: ids,
            terminal_id: *first_terminal,
        }
    }
    fn handle_start(&mut self, rows: u16, cols: u16) -> ServerMessage {
        // Add user information
        // Create terminal
        // Add terminal to wanted list <-- TEMP TODO
        // Create window for terminal
        self.user_manager.add_user(rows, cols);
        crate::ipc::protocol::ServerMessage::Ok
    }

    fn handle_write(&mut self, terminal_id: u64, data: Vec<u8>) -> ServerMessage {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice());
        Ok
    }

    fn handle_create_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        session_name: Option<String>,
    ) -> ServerMessage {
        // Create terminal
        // Create window for terminal
        // Create Session with window
        // Add terminal to user visible terminals
        let session_name = session_name.unwrap_or_else(|| "penar".to_string());
        let (rows, cols) = self.user_manager.get_dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols);
        self.terminal_manager
            .execute_command_on_terminal(terminal_id, format!("cd {working_dir}"));
        if let Some(command) = command {
            self.terminal_manager
                .execute_command_on_terminal(terminal_id, command);
        }
        let window_id = self.window_manager.create_window(terminal_id);

        // TODO Fix clone
        let session_id = self
            .session_manager
            .create_session(window_id, session_name.clone());
        self.user_manager.add_terminal(terminal_id);
        ServerMessage::CreatedSession {
            session_info: SessionInfo::new(session_id, session_name),
            window_id: window_id,
            terminal_id: terminal_id,
        }
    }

    fn handle_list_sessions(&self) -> ServerMessage {
        ServerMessage::ListedSessions {
            sessions: self.session_manager.list_sessions(),
        }
    }
}
