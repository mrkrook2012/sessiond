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
            ClientMessage::UpdateScreenSize { rows, cols } => {
                self.handle_update_screen_size(rows, cols)
            }
            ClientMessage::CreateSession {
                working_dir,
                command,
                session_name,
            } => self.handle_create_session(working_dir, command, session_name),
            ClientMessage::ExitSession => self.handle_exit_session(),
            ClientMessage::KillSession { session_id } => self.handle_kill_session(session_id),
            ClientMessage::CreateTerminal {
                start_terminal_id: _,
                split_type: _,
                working_dir: _,
                command: _,
            } => todo!(),
            ClientMessage::KillTerminal { terminal_id } => self.handle_kill_terminal(terminal_id),
            ClientMessage::WriteTerminal { terminal_id, data } => {
                self.handle_write(terminal_id, data)
            }
            ClientMessage::ListSessions {} => self.handle_list_sessions(),
            ClientMessage::CreateWindow { session_id } => self.handle_create_window(session_id),
            ClientMessage::OpenSession { session_id } => self.handle_open_session(session_id),
            ClientMessage::ChangeWindow { window_id } => self.handle_change_window(window_id),
            ClientMessage::KillWindow { window_id } => self.handle_kill_window(window_id),
            ClientMessage::RenameWindow { window_id, name } => {
                self.handle_rename_window(window_id, name)
            }
        }
    }

    fn handle_rename_window(&mut self, window_id: u64, name: String) -> ServerMessage {
        self.window_manager.set_window_name(window_id, name);
        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(self.user_manager.get_current_session_id()),
        }
    }

    fn handle_change_window(&mut self, window_id: u64) -> ServerMessage {
        self.user_manager.clear_all();
        let terminal_ids = self.window_manager.get_terminals_in_window(window_id);
        for terminal_id in terminal_ids {
            self.user_manager.add_terminal(terminal_id);
        }
        Ok
    }

    fn handle_update_screen_size(&mut self, rows: u16, cols: u16) -> ServerMessage {
        self.user_manager.update_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
        Ok
    }

    fn handle_kill_session(&mut self, session_id: u64) -> ServerMessage {
        // This command will only be given from the list selection
        let mut terminal_ids: Vec<u64> = Vec::new();
        let windows = self
            .session_manager
            .get_all_windows_from_session(session_id);
        for window_id in windows {
            terminal_ids.append(
                self.window_manager
                    .get_terminals_in_window(window_id)
                    .as_mut(),
            );
        }

        for terminal_id in terminal_ids {
            self.terminal_manager.kill_terminal(terminal_id);
        }
        let sessions = self.get_all_session_info();
        ServerMessage::ListedSessions { sessions }
    }

    fn handle_kill_terminal(&mut self, terminal_id: u64) -> ServerMessage {
        // Kill the terminal,
        // Kill any windows or even sessions if they're now empty
        self.terminal_manager.kill_terminal(terminal_id);
        self.user_manager.remove_terminal(terminal_id);
        let window_id = self.window_manager.get_window_id_from_terminal(terminal_id);
        self.window_manager
            .remove_terminal_from_window(window_id, terminal_id);
        let terminal_ids = self.window_manager.get_terminals_in_window(window_id);
        let mut killed_session = false;
        if terminal_ids.len() == 0 {
            // Just removed the last terminal in the window, remove window from sess
            let session_id = self.session_manager.get_session_id_from_window(window_id);
            self.session_manager.remove_window_from_session(window_id);
            let remaining_windows = self
                .session_manager
                .get_all_windows_from_session(session_id);
            if remaining_windows.len() == 0 {
                // This was last terminal in last window, kill the session
                self.session_manager.kill_session(session_id);
                killed_session = true;
            }
        }
        if killed_session {
            return ServerMessage::ListedSessions {
                sessions: self.get_all_session_info(),
            };
        }
        ServerMessage::Ok
    }

    fn handle_kill_window(&mut self, window_id: u64) -> ServerMessage {
        let session_id = self.session_manager.get_session_id_from_window(window_id);

        let terminals = self.window_manager.get_terminals_in_window(window_id);
        for terminal_id in terminals {
            self.terminal_manager.kill_terminal(terminal_id);
            self.user_manager.remove_terminal(terminal_id);
        }
        let remaining_windows = self
            .session_manager
            .get_all_windows_from_session(session_id);
        let mut killed_session = false;
        if remaining_windows.len() == 0 {
            // This was last terminal in last window, kill the session
            self.session_manager.kill_session(session_id);
            killed_session = true;
        }
        if killed_session {
            return ServerMessage::ListedSessions {
                sessions: self.get_all_session_info(),
            };
        }
        ServerMessage::UpdatedSession {
            session_info: self
                .get_session_info_by_id(self.session_manager.get_session_id_from_window(window_id)),
        }
    }

    fn handle_create_window(&mut self, session_id: u64) -> ServerMessage {
        // Create new terminal
        // Add new terminal to window manager
        // Switch to new window
        let (rows, cols) = self.user_manager.get_dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols);
        let window_id = self.window_manager.create_window(terminal_id);
        self.session_manager
            .add_window_to_session(session_id, window_id);
        self.user_manager.clear_all();
        self.user_manager.add_terminal(terminal_id);
        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(session_id),
        }
    }

    fn handle_exit_session(&mut self) -> ServerMessage {
        // Exiting is just stopping sending.
        self.user_manager.clear_all();
        Ok
    }

    fn handle_open_session(&mut self, session_id: u64) -> ServerMessage {
        // Get sessions
        // Get first window in session
        // get terminals in first window
        // Add terminals to user setting
        // Update thread will send data based on user setting.
        self.user_manager.set_current_session_id(session_id);
        let ids = self
            .session_manager
            .get_window_ids_by_session_id(session_id);
        let window_id = ids.first().unwrap();
        let terminals = self.window_manager.get_terminals_in_window(*window_id);

        for terminal_id in &terminals {
            self.user_manager.add_terminal(terminal_id.clone());
        }
        let session_info = self
            .get_all_session_info()
            .into_iter()
            .find(|info| info.session_id == session_id)
            .unwrap();
        ServerMessage::UpdatedSession { session_info }
    }

    fn handle_start(&mut self, rows: u16, cols: u16) -> ServerMessage {
        // Add user information
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
            .execute_command_on_terminal(terminal_id, format!("cd {working_dir} && clear"));
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
        self.user_manager.set_current_session_id(session_id);

        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(session_id),
        }
    }

    fn get_all_session_info(&self) -> Vec<SessionInfo> {
        self.session_manager
            .list_sessions()
            .iter()
            .map(|session| {
                SessionInfo::new(
                    session.session_id,
                    session.session_name.clone(),
                    session
                        .windows
                        .iter()
                        .map(|window_id| {
                            self.window_manager
                                .get_window_by_id(*window_id)
                                .get_window_info()
                        })
                        .collect(),
                )
            })
            .collect()
    }
    fn get_session_info_by_id(&self, session_id: u64) -> SessionInfo {
        let session = self.session_manager.get_session_by_id(session_id);
        SessionInfo::new(
            session_id,
            session.session_name.clone(),
            session
                .windows
                .iter()
                .map(|window_id| {
                    self.window_manager
                        .get_window_by_id(*window_id)
                        .get_window_info()
                })
                .collect(),
        )
    }
    fn handle_list_sessions(&self) -> ServerMessage {
        ServerMessage::ListedSessions {
            sessions: self.get_all_session_info(),
        }
    }
}
