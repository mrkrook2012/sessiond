use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::backend::{
    mini_user_manager::MiniUserManager, session_manager::SessionManager,
    terminal_manager::TerminalManager, window_manager::WindowManager,
};
use crate::ipc::protocol::{ClientMessage, ServerMessage, SessionInfo};
use color_eyre::eyre::Result;

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

    pub fn process_message(&mut self, client_message: ClientMessage) -> Result<ServerMessage> {
        match client_message {
            ClientMessage::Ping => Ok(ServerMessage::Pong),
            ClientMessage::Start { rows, cols } => Ok(self.handle_start(rows, cols)),
            ClientMessage::UpdateScreenSize { rows, cols } => {
                Ok(self.handle_update_screen_size(rows, cols))
            }
            ClientMessage::CreateSession {
                working_dir,
                command,
                session_name,
            } => Ok(self.handle_create_session(working_dir, command, session_name)),
            ClientMessage::ExitSession => Ok(self.handle_exit_session()),
            ClientMessage::KillSession { session_id } => Ok(self.handle_kill_session(session_id)),
            ClientMessage::CreateTerminal {
                start_terminal_id: _,
                working_dir: _,
                command: _,
            } => todo!(),

            ClientMessage::KillTerminal { terminal_id } => {
                Ok(self.handle_kill_terminal(terminal_id))
            }
            ClientMessage::WriteTerminal { terminal_id, data } => {
                Ok(self.handle_write(terminal_id, data))
            }
            ClientMessage::ListSessions {} => Ok(self.handle_list_sessions()),
            ClientMessage::CreateWindow { session_id } => Ok(self.handle_create_window(session_id)),
            ClientMessage::OpenSession { session_id } => self.handle_open_session(session_id),
            ClientMessage::ChangeWindow { window_id } => Ok(self.handle_change_window(window_id)),
            ClientMessage::KillWindow { window_id } => Ok(self.handle_kill_window(window_id)),
            ClientMessage::ExportSession { session_id } => {
                Ok(self.handle_export_session(session_id))
            }
            ClientMessage::StartSessionFromFile { file_path } => {
                self.handle_start_session_from_file(file_path)
            }
            ClientMessage::KillDaemon => {
                // This should cause the daemon to exit
                // Return a special response that the frontend can interpret
                panic!("Got kill message, exiting");
            }
        }
    }

    // TODO think about working directories and storing files
    fn handle_start_session_from_file(&mut self, file_path: String) -> Result<ServerMessage> {
        let input = std::fs::read_to_string(file_path)?;
        let session_yaml: YamlSession = serde_yaml::from_str(&input)?;

        let session_id = self
            .session_manager
            .create_empty_session(session_yaml.session_name);
        for window in session_yaml.windows {
            let terminal_id = self.terminal_manager.create_terminal(100, 100); // Should be usermanager ones
            self.terminal_manager
                .execute_command_on_terminal(terminal_id, window.command);
            let window_id = self.window_manager.create_window(terminal_id);
            self.session_manager
                .add_window_to_session(session_id, window_id);
        }
        Ok(ServerMessage::Ok)
    }

    fn handle_export_session(&mut self, session_id: u64) -> ServerMessage {
        // Just testing
        let session = self.session_manager.get_session_by_id(session_id);
        let windows = self
            .session_manager
            .get_all_windows_from_session(session_id);

        let mut commands: Vec<String> = vec![];
        for window_id in windows {
            let terminal_id = self
                .window_manager
                .get_terminal_id_from_window_id(window_id);
            let command = self.terminal_manager.get_command_from_terminal(terminal_id);
            commands.push(command);
        }

        let mut windows: Vec<YamlWindow> = vec![];
        for command in commands {
            let window = YamlWindow { command: command };
            windows.push(window);
        }
        let yaml_session = YamlSession {
            session_name: session.session_name.clone(),
            windows: windows,
        };

        let yaml: String = serde_yaml::to_string(&yaml_session).unwrap();
        std::fs::write("./template.yaml", yaml).unwrap();
        ServerMessage::Ok
    }
    fn handle_change_window(&mut self, window_id: u64) -> ServerMessage {
        self.user_manager.clear_all();
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(window_id);
        self.user_manager.add_terminal(terminal_id);
        ServerMessage::Ok
    }

    fn handle_update_screen_size(&mut self, rows: u16, cols: u16) -> ServerMessage {
        self.user_manager.update_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
        ServerMessage::Ok
    }

    fn handle_kill_terminal(&mut self, terminal_id: u64) -> ServerMessage {
        // Unsub from terminal
        // Kill terminal
        // Check window terminal count, if 1 kill window, else rm window from tree
        // check window count in session, if zero kill session
        let window_id = self.window_manager.get_window_id_from_terminal(terminal_id);
        let session_id = self.session_manager.get_session_id_from_window(window_id);

        // self.user_manager.remove_terminal(terminal_id);
        // self.window_manager
        //     .remove_terminal_from_window(window_id, terminal_id);
        // let terminals_left = self.window_manager.get_terminals_in_window(window_id);
        // if terminals_left.len() == 0 {
        //     // kill window
        //     self.window_manager.kill_window(window_id);
        //     self.session_manager.remove_window_from_session(window_id);

        //     // Check if there are windows left
        //     if self.session_manager.is_session_empty(session_id) {
        //         // Session is dead, exit.

        //         self.session_manager.kill_session(session_id);
        //         return ServerMessage::ListedSessions {
        //             sessions: self.get_all_session_info(),
        //         };
        //     }
        // } else {
        //     // Just remove terminal from tree
        //     let window = self.window_manager.get_window_by_id_mut(window_id);
        //     let root = window.get_window_layout_ref_mut();
        //     root.remove_terminal(&mut self.terminal_manager, terminal_id);
        // }

        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(session_id),
        }
    }

    fn handle_kill_session(&mut self, session_id: u64) -> ServerMessage {
        // This command will only be given from the list selection
        let mut terminal_ids: Vec<u64> = Vec::new();
        let windows = self
            .session_manager
            .get_all_windows_from_session(session_id);

        for window_id in windows {
            terminal_ids.push(
                self.window_manager
                    .get_terminal_id_from_window_id(window_id),
            );
        }

        for terminal_id in terminal_ids {
            self.terminal_manager.kill_terminal(terminal_id);
            self.user_manager.remove_terminal(terminal_id);
        }
        self.session_manager.kill_session(session_id);
        let sessions = self.get_all_session_info();
        ServerMessage::ListedSessions { sessions }
    }

    fn handle_kill_window(&mut self, window_id: u64) -> ServerMessage {
        let session_id = self.session_manager.get_session_id_from_window(window_id);
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(window_id);

        self.terminal_manager.kill_terminal(terminal_id);
        self.user_manager.remove_terminal(terminal_id);

        self.window_manager.kill_window(window_id);
        self.session_manager.remove_window_from_session(window_id);

        let remaining = self
            .session_manager
            .get_all_windows_from_session(session_id);

        if remaining.len() == 0 {
            self.session_manager.kill_session(session_id);
            return ServerMessage::ListedSessions {
                sessions: self.get_all_session_info(),
            };
        }

        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(session_id),
        }

        // let session_id = self.session_manager.get_session_id_from_window(window_id);

        // let terminals = self.window_manager.get_terminals_in_window(window_id);
        // for terminal_id in terminals {
        //     self.terminal_manager.kill_terminal(terminal_id);
        //     self.user_manager.remove_terminal(terminal_id);
        // }
        // self.window_manager.kill_window(window_id);
        // let remaining_windows = self
        //     .session_manager
        //     .get_all_windows_from_session(session_id);
        // let mut killed_session = false;
        // if remaining_windows.len() == 0 {
        //     // This was last terminal in last window, kill the session
        //     self.session_manager.kill_session(session_id);
        //     killed_session = true;
        // }
        // if killed_session {
        //     return ServerMessage::ListedSessions {
        //         sessions: self.get_all_session_info(),
        //     };
        // }
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
        ServerMessage::Ok
    }

    fn handle_open_session(&mut self, session_id: u64) -> Result<ServerMessage> {
        // Get sessions
        // Get first window in session
        // get terminals in first window
        // Add terminals to user setting
        // Update thread will send data based on user setting.
        self.user_manager.set_current_session_id(session_id);
        let ids = self
            .session_manager
            .get_window_ids_by_session_id(session_id);
        let window_id = ids
            .first()
            .ok_or_else(|| color_eyre::eyre::eyre!("No windows found for session"))?;

        self.user_manager.add_terminal(
            self.window_manager
                .get_terminal_id_from_window_id(*window_id),
        );

        let session_info = self
            .get_all_session_info()
            .into_iter()
            .find(|info| info.session_id == session_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session info not found"))?;
        Ok(ServerMessage::UpdatedSession { session_info })
    }

    fn handle_start(&mut self, rows: u16, cols: u16) -> ServerMessage {
        // Add user information
        self.user_manager.add_user(rows, cols);
        crate::ipc::protocol::ServerMessage::Ok
    }

    fn handle_write(&mut self, terminal_id: u64, data: Vec<u8>) -> ServerMessage {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice());
        ServerMessage::Ok
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
        let _session_id = self
            .session_manager
            .create_session(window_id, session_name.clone());

        ServerMessage::ListedSessions {
            sessions: self.get_all_session_info(),
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
