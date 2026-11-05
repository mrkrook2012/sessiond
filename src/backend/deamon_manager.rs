use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::backend::session_manager::{SessionId, SessionKind, UnitName};
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalId;
use crate::backend::window_manager::WindowId;
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::backend::{
    mini_user_manager::MiniUserManager, session_manager::SessionManager,
    terminal_manager::TerminalManager, window_manager::WindowManager,
};
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::{
    ClientMessage, ServerMessage, SessionInfo, SysdAction, SysdData, TerminalUpdate,
};
use color_eyre::eyre::{Ok, OptionExt, Result, bail, eyre};
use tokio::sync::Mutex;

pub struct DaemonManager {
    user_manager: MiniUserManager,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
    systemd_manager: Arc<Mutex<SystemdServiceManager>>, //TODO Remove the mutex when cleaning up async
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

    pub fn get_wanted_data(&mut self, terminal_id: TerminalId) -> TerminalUpdate {
        self.terminal_manager.get_data_from_terminal(terminal_id)
    }

    // TODO Remove the old ServerMessage, replace with fitting return type
    pub fn handle_stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        // Find the current session,
        // Remove all windows,
        // Kill all terminals
        // Then change state to stopped
        println!("Stopping sysd session: {:?}", id);
        let session = self
            .session_manager
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;

        // Get all the terminals and windows
        let window_ids = self
            .session_manager
            .get_all_windows_from_session(id)
            .ok_or_eyre("Found no windows in session")?;
        let mut terminal_ids: Vec<TerminalId> = vec![];
        for window_id in window_ids {
            terminal_ids.push(
                self.window_manager
                    .get_terminal_id_from_window_id(window_id),
            );
        }

        println!("Session has windows: {:?}", window_ids);
        println!("Session has terminals: {:?}", terminal_ids);

        // Remove all running terminals and refrences to them
        for terminal_id in terminal_ids {
            self.terminal_manager.kill_terminal(terminal_id);
        }
        for window_id in window_ids {
            self.window_manager.kill_window(*window_id);
        }

        Ok(())
    }

    pub fn handle_execute_sysd_action(
        &mut self,
        id: SessionId,
        sysd_action: SysdAction,
    ) -> Result<Vec<SessionInfo>> {
        // These should trigger the sysd command,
        // This will cause the execution of start/stop-sysd, which will actually change the state
        // Enabling/disablng will be done here, as they don't actually affect the session
        let session = self
            .session_manager
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;
        match &mut session.kind {
            SessionKind::Ephemoral { windows } => {
                bail!("Cannot call sysd operation on non-sysd session")
            }
            SessionKind::SysdBased(sysd_session) => {
                // Match on different kinds of operation
                match sysd_action {
                    // These will trigger sysd-start/stop due to them being present in the sysd file.
                    // This file contains the info needed for a start and stop
                    SysdAction::Start => sysd_session.start(),
                    SysdAction::Stop => sysd_session.stop(),

                    SysdAction::Restart => sysd_session.restart(),
                    SysdAction::Enable => sysd_session.enable(),
                    SysdAction::Disable => sysd_session.disable(),
                }
            }
        }

        Ok(self.get_all_session_info())
    }

    pub fn handle_start_sysd_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        println!("Got file");
        let input = std::fs::read_to_string(file_path)?;
        let session_yaml: YamlSession = serde_yaml::from_str(&input)?;
        let name = UnitName::new(&session_yaml.name)?;
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

        self.session_manager.create_sysd_session(name, windows);
        Ok(())
    }

    pub fn handle_export_session_to_file(
        &self,
        id: SessionId,
        mut path: PathBuf,
    ) -> Result<PathBuf> {
        let session = self.get_session_info_by_id(id);
        let name = session.name;
        let yaml = self.get_session_yaml(id)?;
        println!("path: {:?}, name {}", path, name);
        path.push(name);
        path.add_extension("yaml");
        fs::write(path.as_path(), yaml)?;
        Ok(path)
    }

    // TODO think about working directories and storing files
    pub fn handle_start_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
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
        Ok(())
    }

    pub fn handle_export_session(&mut self, id: SessionId) -> Result<()> {
        // Create .yaml for starting
        // Kill original session
        // Store .yaml in ~/.local/share/{daemon_test}/{name}.service
        // Generate sysd file pointing to .yaml
        // Profit?
        let yaml = self.get_session_yaml(id)?;
        let name = self.get_session_info_by_id(id).name;

        // Kill original session
        self.handle_kill_session(id);

        let home = std::env::var("HOME")?;
        let path_to_yaml_file = PathBuf::from(home)
            .join(".local/share/daemon_test")
            .join(format!("{name}.yaml"));

        println!("Writing to: {:?}", path_to_yaml_file);
        std::fs::write(&path_to_yaml_file, yaml)?;
        let unit = UnitName::new(&name)?; // validate synchronously, so bad names reach the client
        let manager = Arc::clone(&self.systemd_manager);
        let path = path_to_yaml_file.clone();

        // TODO, this might not actualy succeed
        tokio::spawn(async move {
            let mut manager = manager.lock().await;
            if let Err(err) = manager.create_service_from_yaml(&path, unit).await {
                eprintln!("failed to create systemd service: {err:?}");
            }
        });

        Ok(())
    }

    pub fn get_session_yaml(&self, id: SessionId) -> Result<String> {
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

    pub fn handle_change_window(&mut self, window_id: WindowId) {
        self.user_manager.clear_all();
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);
        self.user_manager.add_terminal(terminal_id);
    }

    pub fn handle_update_screen_size(&mut self, rows: u16, cols: u16) {
        self.user_manager.update_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
    }

    // TODO Remove
    // pub fn handle_kill_terminal(
    //     &mut self,
    //     id: SessionId,
    //     terminal_id: TerminalId,
    // ) -> ServerMessage {
    //     // Unsub from terminal
    //     // Kill terminal
    //     // Check window terminal count, if 1 kill window, else rm window from tree
    //     // check window count in session, if zero kill session
    //     let window_id = self
    //         .window_manager
    //         .get_window_id_from_terminal(terminal_id)
    //         .unwrap();

    //     ServerMessage::UpdatedSession {
    //         session_info: self.get_session_info_by_id(id),
    //     }
    // }

    // TODO find a more robust solution
    pub fn handle_kill_session(&mut self, id: SessionId) -> Vec<SessionInfo> {
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
        self.get_all_session_info()
    }

    pub fn handle_kill_window(
        &mut self,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<SessionInfo> {
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
        }

        Ok(self.get_session_info_by_id(id))
    }

    pub fn handle_create_window(&mut self, id: SessionId) -> SessionInfo {
        // Create new terminal
        // Add new terminal to window manager
        // Switch to new window
        let (rows, cols) = self.user_manager.get_dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols);
        let window_id = self.window_manager.create_window(terminal_id);
        self.session_manager
            .add_window_to_session(id, window_id)
            .unwrap();
        self.user_manager.clear_all();
        self.user_manager.add_terminal(terminal_id);
        self.get_session_info_by_id(id)
    }

    pub fn handle_exit_session(&mut self) -> Vec<SessionInfo> {
        // Exiting is just stopping sending.
        self.user_manager.clear_all();
        self.get_all_session_info()
    }

    pub fn handle_open_session(&mut self, id: SessionId) -> Result<SessionInfo> {
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
        Ok(session_info)
    }

    pub fn handle_start(&mut self, rows: u16, cols: u16) {
        // Add user information
        self.user_manager.add_user(rows, cols);
    }

    pub fn handle_write(&mut self, terminal_id: TerminalId, data: Vec<u8>) {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice());
    }

    pub fn handle_create_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo> {
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

        self.get_all_session_info()
    }

    pub fn handle_create_sysd_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        let name = name.unwrap_or_else(|| "penar".to_string());

        // Create terminal
        let (rows, cols) = self.user_manager.get_dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols);

        // Set working directory
        self.terminal_manager
            .execute_command_on_terminal(terminal_id, format!("cd {working_dir} && clear"));

        // Execute optional command
        if let Some(command) = command {
            self.terminal_manager
                .execute_command_on_terminal(terminal_id, command);
        }

        // Create window
        let window_id = self.window_manager.create_window(terminal_id);

        // Create systemd-backed session
        let unit_name = UnitName::new(&name).expect("Invalid systemd unit name");

        let session_id = self
            .session_manager
            .create_sysd_session(unit_name, vec![window_id]);

        // Export session YAML and generate systemd service
        let home = std::env::var("HOME").expect("HOME environment variable not set");

        let session_dir = PathBuf::from(home).join(".local/share/daemon_test");

        fs::create_dir_all(&session_dir).expect("Failed to create session directory");

        self.handle_export_session(session_id)
            .expect("Failed to export systemd session");

        self.get_all_session_info()
    }

    pub fn get_all_session_info(&self) -> Vec<SessionInfo> {
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
                    match &session.kind {
                        SessionKind::Ephemoral { windows: _ } => None,
                        SessionKind::SysdBased(sysd_session) => Some(sysd_session.get_sysd_data()),
                    },
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
            match &session.kind {
                SessionKind::Ephemoral { windows: _ } => None,
                SessionKind::SysdBased(sysd_session) => Some(sysd_session.get_sysd_data()),
            },
        )
    }

    pub fn handle_list_sessions(&self) -> Vec<SessionInfo> {
        self.get_all_session_info()
    }

    pub fn handle_kill_deamon(&self) -> ! {
        std::fs::remove_file(socket_path()).unwrap();
        std::process::exit(0);
    }
}
