use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::backend::session_manager::SessionId;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalId;
use crate::backend::window_manager::WindowId;
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::backend::{
    mini_user_manager::MiniUserManager, session_manager::SessionManager,
    terminal_manager::TerminalManager, window_manager::WindowManager,
};
use crate::ipc::protocol::{ClientMessage, ServerMessage, SessionInfo};
use color_eyre::eyre::{Ok, OptionExt, Result, eyre};
use tokio::sync::{Mutex, Notify};

pub struct DaemonManager {
    user_manager: MiniUserManager,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
    systemd_manager: Arc<Mutex<SystemdServiceManager>>,
}

impl DaemonManager {
    pub fn new(
        user_manager: MiniUserManager,
        terminal_manager: TerminalManager,
        window_manager: WindowManager,
        session_manager: SessionManager,
        systemd_manager: Arc<Mutex<SystemdServiceManager>>,
    ) -> Self {
        Self {
            user_manager,
            terminal_manager,
            window_manager,
            session_manager,
            systemd_manager,
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
                name,
            } => Ok(self.handle_create_session(working_dir, command, name)),
            ClientMessage::ExitSession => Ok(self.handle_exit_session()),
            ClientMessage::KillSession { id } => Ok(self.handle_kill_session(id)),
            ClientMessage::CreateTerminal {
                //TODO not needed?
                start_terminal_id: _,
                working_dir: _,
                command: _,
            } => todo!(),

            ClientMessage::KillTerminal { terminal_id, id } => {
                Ok(self.handle_kill_terminal(id, terminal_id))
            }
            ClientMessage::WriteTerminal { terminal_id, data } => {
                Ok(self.handle_write(terminal_id, data))
            }
            ClientMessage::ListSessions {} => Ok(self.handle_list_sessions()),
            ClientMessage::CreateWindow { id } => Ok(self.handle_create_window(id)),
            ClientMessage::OpenSession { id } => self.handle_open_session(id),
            ClientMessage::ChangeWindow { window_id } => Ok(self.handle_change_window(window_id)),
            ClientMessage::KillWindow { window_id, id } => self.handle_kill_window(id, window_id),
            ClientMessage::ExportSessionToSysd { id } => self.handle_export_session(id),
            ClientMessage::StartSessionFromFile { file_path } => {
                self.handle_start_session_from_file(file_path)
            }
                ClientMessage::KillDaemon => {self.handle_kill_deamon(); Ok(ServerMessage::Ok)},
            ClientMessage::UpdateSysdSessionEnabled { id, enabled } => todo!(),
            ClientMessage::SetRunningSysdSession { id, should_run } => todo!(),
            ClientMessage::ExportSessionToFile { id, working_dir } => {
                self.handle_export_session_to_file(id, working_dir)
            }
        }
    }

    fn handle_export_session_to_file(
        &self,
        id: SessionId,
        mut path: PathBuf,
    ) -> Result<ServerMessage> {
        let session = self.get_session_info_by_id(id);
        let name = session.name;
        let yaml = self.get_session_yaml(id)?;
        println!("path: {:?}, name {}", path, name);
        path.push(name);
        path.add_extension(".yaml");
        fs::write(path.as_path(), yaml)?;
        Ok(ServerMessage::ExportedSessionToFile { path })
    }
    // TODO think about working directories and storing files
    fn handle_start_session_from_file(
        &mut self,
        file_path: std::path::PathBuf,
    ) -> Result<ServerMessage> {
        println!("Got file");
        let input = std::fs::read_to_string(file_path)?;
        let session_yaml: YamlSession = serde_yaml::from_str(&input)?;
        println!("Parsed yaml {:?}", session_yaml);

        let mut windows: Vec<WindowId> = vec![];
        for window in session_yaml.windows {
            println!("Adding window with command {}", window.command);

            let terminal_id = self.terminal_manager.create_terminal(100, 100); // Should be usermanager ones

            let working_dir = window.working_directory;
            self.terminal_manager
                .execute_command_on_terminal(terminal_id, format!("cd {working_dir}"));

            self.terminal_manager
                .execute_command_on_terminal(terminal_id, window.command);

            let window_id = self.window_manager.create_window(terminal_id);
            windows.push(window_id);
        }

        self.session_manager
            .create_ephem_session(windows, session_yaml.name);
        Ok(ServerMessage::Ok)
    }

    fn handle_export_session(&mut self, id: SessionId) -> Result<ServerMessage> {
        todo!();
        // // Create .yaml file and store it
        // let yaml = self.get_session_yaml(id);
        // let name = self.get_session_info_by_id(id).name;
        // let home = std::env::var("HOME")?;
        // let path = PathBuf::from(home)
        //     .join(".local/share/daemon_test")
        //     .join(format!("{name}.yaml"));
        // println!("Writing to: {:?}", path);
        // std::fs::write(&path, yaml)?;

        // // Create systemd service that run that yaml.
        // let manager = Arc::clone(&self.systemd_manager);
        // tokio::spawn(async move {
        //     let mut manager = manager.lock().await;

        //     if let Err(err) = manager.create_service_from_yaml(&path, name).await {
        //         eprintln!("failed to create systemd service: {err:?}");
        //     }
        // });

        Ok(ServerMessage::Ok)
    }

    fn get_session_yaml(&self, id: SessionId) -> Result<String> {
        let session = self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre(eyre!("Session not found"))?;
        let windows = self
            .session_manager
            .get_all_windows_from_session(id)
            .ok_or_eyre(eyre!("Trying to get windowless sessions to yaml, error"))?;

        let mut yaml_windows = Vec::new();

        for window_id in windows {
            let terminal_id = self
                .window_manager
                .get_terminal_id_from_window_id(window_id);

            let command = self.terminal_manager.get_command_from_terminal(terminal_id);
            let working_directory = self
                .terminal_manager
                .get_working_dir_from_terminal(terminal_id);

            yaml_windows.push(YamlWindow {
                command,
                working_directory,
            });
        }

        let yaml_session = YamlSession {
            name: session.name.clone(),
            windows: yaml_windows,
        };

        Ok(serde_yaml::to_string(&yaml_session)?)
    }
    fn handle_change_window(&mut self, window_id: WindowId) -> ServerMessage {
        self.user_manager.clear_all();
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);
        self.user_manager.add_terminal(terminal_id);
        ServerMessage::Ok
    }

    fn handle_update_screen_size(&mut self, rows: u16, cols: u16) -> ServerMessage {
        self.user_manager.update_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
        ServerMessage::Ok
    }

    fn handle_kill_terminal(&mut self, id: SessionId, terminal_id: TerminalId) -> ServerMessage {
        // Unsub from terminal
        // Kill terminal
        // Check window terminal count, if 1 kill window, else rm window from tree
        // check window count in session, if zero kill session
        let window_id = self
            .window_manager
            .get_window_id_from_terminal(terminal_id)
            .unwrap();

        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(id),
        }
    }

    fn handle_kill_session(&mut self, id: SessionId) -> ServerMessage {
        // This command will only be given from the list selection
        let mut terminal_ids: Vec<TerminalId> = Vec::new();
        let windows = self
            .session_manager
            .get_all_windows_from_session(id)
            .unwrap();

        for window_id in windows {
            terminal_ids.push(
                self.window_manager
                    .get_terminal_id_from_window_id(&window_id),
            );
        }

        for terminal_id in terminal_ids {
            self.terminal_manager.kill_terminal(terminal_id);
            self.user_manager.remove_terminal(terminal_id);
        }
        self.session_manager.kill_session(id);
        let sessions = self.get_all_session_info();
        ServerMessage::ListedSessions { sessions }
    }

    fn handle_kill_window(&mut self, id: SessionId, window_id: WindowId) -> Result<ServerMessage> {
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);

        self.terminal_manager.kill_terminal(terminal_id);
        self.user_manager.remove_terminal(terminal_id);

        self.window_manager.kill_window(window_id);
        self.session_manager
            .remove_window_from_session(id, window_id)?;

        let remaining = self
            .session_manager
            .get_all_windows_from_session(id)
            .unwrap();

        if remaining.len() == 0 {
            //TODO This is ass,
            self.session_manager.kill_session(id);
            return Ok(ServerMessage::ListedSessions {
                sessions: self.get_all_session_info(),
            });
        }

        Ok(ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(id),
        })
    }

    fn handle_create_window(&mut self, id: SessionId) -> ServerMessage {
        // Create new terminal
        // Add new terminal to window manager
        // Switch to new window
        let (rows, cols) = self.user_manager.get_dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols);
        let window_id = self.window_manager.create_window(terminal_id);
        self.session_manager.add_window_to_session(id, window_id);
        self.user_manager.clear_all();
        self.user_manager.add_terminal(terminal_id);
        ServerMessage::UpdatedSession {
            session_info: self.get_session_info_by_id(id),
        }
    }

    fn handle_exit_session(&mut self) -> ServerMessage {
        // Exiting is just stopping sending.
        self.user_manager.clear_all();
        ServerMessage::Ok
    }

    fn handle_open_session(&mut self, id: SessionId) -> Result<ServerMessage> {
        // Get sessions
        // Get first window in session
        // get terminals in first window
        // Add terminals to user setting
        // Update thread will send data based on user setting.
        self.user_manager.set_current_id(id);
        let ids = self
            .session_manager
            .get_all_windows_from_session(id)
            .unwrap();
        let window_id = ids
            .first()
            .ok_or_else(|| color_eyre::eyre::eyre!("No windows found for session"))?;

        self.user_manager.add_terminal(
            self.window_manager
                .get_terminal_id_from_window_id(window_id),
        );

        let session_info = self
            .get_all_session_info()
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session info not found"))?;
        Ok(ServerMessage::UpdatedSession { session_info })
    }

    fn handle_start(&mut self, rows: u16, cols: u16) -> ServerMessage {
        // Add user information
        self.user_manager.add_user(rows, cols);
        crate::ipc::protocol::ServerMessage::Ok
    }

    fn handle_write(&mut self, terminal_id: TerminalId, data: Vec<u8>) -> ServerMessage {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice());
        ServerMessage::Ok
    }

    fn handle_create_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> ServerMessage {
        // Create terminal
        // Create window for terminal
        // Create Session with window
        let name = name.unwrap_or_else(|| "penar".to_string());
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
        let _id = self
            .session_manager
            .create_ephem_session(vec![window_id], name.clone());

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
                    session.id,
                    session.name.clone(),
                    session
                        .get_windows()
                        .unwrap()
                        .iter()
                        .map(|window_id| {
                            self.window_manager
                                .get_window_by_id(window_id)
                                .get_window_info()
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn get_session_info_by_id(&self, id: SessionId) -> SessionInfo {
        let session = self.session_manager.get_session_by_id(id).unwrap();
        SessionInfo::new(
            id,
            session.name.clone(),
            session
                .get_windows()
                .unwrap()
                .iter()
                .map(|window_id| {
                    self.window_manager
                        .get_window_by_id(window_id)
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

    fn handle_kill_deamon(&self) -> ! {
        std::process::exit(0);
    }
}
