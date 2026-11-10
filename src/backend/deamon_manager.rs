use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::session_manager::{SessionId, SessionKind, UnitName};
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalId;
use crate::backend::window_manager::WindowId;
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::backend::yaml::yaml_manager::YamlManager;
use crate::backend::{
    mini_user_manager::MiniUserManager, session_manager::SessionManager,
    terminal_manager::TerminalManager, window_manager::WindowManager,
};
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::{SessionInfo, SysdAction, TerminalUpdate};
use color_eyre::eyre::{OptionExt, Result, bail, eyre};

pub struct DaemonManager {
    user_manager: MiniUserManager,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
    systemd_manager: SystemdServiceManager,
    yaml_manager: YamlManager,
}

impl DaemonManager {
    pub fn new(
        user_manager: MiniUserManager,
        terminal_manager: TerminalManager,
        window_manager: WindowManager,
        session_manager: SessionManager,
        systemd_manager: SystemdServiceManager,
        yaml_manager: YamlManager,
    ) -> Self {
        Self {
            user_manager,
            terminal_manager,
            window_manager,
            session_manager,
            systemd_manager,
            yaml_manager,
        }
    }

    pub fn get_wanted_data(&mut self, terminal_id: TerminalId) -> TerminalUpdate {
        self.terminal_manager.get_data_from_terminal(terminal_id)
    }

    pub fn handle_stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        println!("Stopping sysd session: {:?}", id);

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

        for terminal_id in terminal_ids {
            self.terminal_manager.kill_terminal(terminal_id);
        }

        for window_id in window_ids {
            self.window_manager.kill_window(*window_id);
        }

        self.session_manager.stop_sysd_session(id);

        Ok(())
    }

    // TODO Move this to frontend only, idc
    pub fn handle_execute_sysd_action(
        &mut self,
        id: SessionId,
        sysd_action: SysdAction,
    ) -> Result<Vec<SessionInfo>> {
        let session = self
            .session_manager
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemoral { windows: _ } => {
                bail!("Cannot call sysd operation on non-sysd session")
            }
            SessionKind::SysdBased(sysd_session) => match sysd_action {
                SysdAction::Start => sysd_session.start(),
                SysdAction::Stop => sysd_session.stop(),
                SysdAction::Restart => sysd_session.restart(),
                SysdAction::Enable => sysd_session.enable(),
                SysdAction::Disable => sysd_session.disable(),
            },
        }

        Ok(self.get_all_session_info())
    }

    pub fn handle_start_sysd_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        let session_yaml = self.yaml_manager.load_session(file_path)?;
        let name = UnitName::new(&session_yaml.name)?;

        println!("Parsed yaml {:?}", session_yaml);

        let mut windows: Vec<WindowId> = vec![];

        for window in session_yaml.windows {
            println!("Adding window with command {}", window.command);

            let terminal_id = self.terminal_manager.create_terminal(100, 100);

            let working_dir = window.working_directory;

            self.terminal_manager
                .execute_command_on_terminal(terminal_id, format!("cd {working_dir}"));

            self.terminal_manager
                .execute_command_on_terminal(terminal_id, window.command);

            let window_id = self.window_manager.create_window(terminal_id);
            windows.push(window_id);
        }

        // If it's already there, just start, else create
        if (self
            .session_manager
            .does_session_name_exist(&name.to_string()))
        {
            let id = self
                .session_manager
                .get_id_by_name(&name.to_string())
                .unwrap();
            self.session_manager.start_sysd_session(id, windows);
        } else {
            self.session_manager
                .create_running_sysd_session(name, windows);
        }

        Ok(())
    }

    pub fn handle_export_session_to_file(
        &self,
        id: SessionId,
        mut path: PathBuf,
    ) -> Result<PathBuf> {
        let session_yaml = self.build_yaml_session(id)?;

        println!("path: {:?}, name {}", path, session_yaml.name);

        path.push(&session_yaml.name);
        path.set_extension("yaml");

        self.yaml_manager.save_session_to(&session_yaml, &path)?;

        Ok(path)
    }

    // TODO think about working directories and storing files
    pub fn handle_start_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        let session_yaml = self.yaml_manager.load_session(file_path)?;

        println!("Parsed yaml {:?}", session_yaml);

        let mut windows: Vec<WindowId> = vec![];

        for window in session_yaml.windows {
            println!("Adding window with command {}", window.command);

            let terminal_id = self.terminal_manager.create_terminal(100, 100);

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
        let session_yaml = self.build_yaml_session(id)?;

        // Validate the name before doing anything destructive.
        let unit = UnitName::new(&session_yaml.name)?;

        // Persist the YAML before killing the original session.
        // If saving fails, the original session remains intact.
        let path = self.yaml_manager.save_session(&session_yaml)?;

        self.handle_kill_session(id);

        self.systemd_manager.create_service_from_yaml(&path, unit)?;

        Ok(())
    }

    /// Builds the YAML representation of a running session.
    ///
    /// This method does not serialize or write anything.
    /// Persistence is handled by YamlManager.
    fn build_yaml_session(&self, id: SessionId) -> Result<YamlSession> {
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

        Ok(YamlSession {
            name: session.name.clone(),
            windows: yaml_windows,
        })
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

    // TODO find a more robust solution
    pub fn handle_kill_session(&mut self, id: SessionId) -> Vec<SessionInfo> {
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

        if remaining.is_empty() {
            self.session_manager.kill_session(id);
        }

        Ok(self.get_session_info_by_id(id))
    }

    pub fn handle_create_window(&mut self, id: SessionId) -> SessionInfo {
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
        self.user_manager.clear_all();
        self.get_all_session_info()
    }

    pub fn handle_open_session(&mut self, id: SessionId) -> Result<SessionInfo> {
        self.user_manager.set_current_id(id);

        let ids = self
            .session_manager
            .get_all_windows_from_session(id)
            .unwrap();

        let window_id = ids
            .first()
            .ok_or_else(|| eyre!("No windows found for session"))?;

        self.user_manager.add_terminal(
            self.window_manager
                .get_terminal_id_from_window_id(window_id),
        );

        let session_info = self
            .get_all_session_info()
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| eyre!("Session info not found"))?;

        Ok(session_info)
    }

    pub fn handle_start(&mut self, rows: u16, cols: u16) {
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

        self.session_manager
            .create_ephem_session(vec![window_id], name);

        self.get_all_session_info()
    }

    pub fn handle_create_sysd_session(
        &mut self,
        working_dir: String,
        command: Option<String>, //TODO MUST BE SOME
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        // TODO THIS FUNCTION NEEDS TO DIE
        // Start terminals
        // Add terminals to windows
        // Create running Sysd Session.
        // We can only create a stopped version
        // A start command must be executed later by client

        println!("Got reqeust to make new sysd service");
        let name = name.unwrap_or_else(|| "penar".to_string());

        let command = command.unwrap();

        // Create systemd-backed session
        let unit_name = UnitName::new(&name).expect("Invalid systemd unit name");

        let session_yaml = YamlSession {
            name,
            windows: vec![YamlWindow {
                command,
                working_directory: working_dir,
            }],
        };

        let path = self.yaml_manager.save_session(&session_yaml).unwrap();

        // TODO, this might not actualy succeed
        self.systemd_manager
            .create_service_from_yaml(&path, unit_name.clone())
            .unwrap();

        self.session_manager.create_stopped_sysd_session(unit_name);
        self.get_all_session_info()
    }

    pub fn handle_delete_sysd_session(&mut self, id: SessionId) -> Result<Vec<SessionInfo>> {
        // TODO needs to be it's own method
        let mut terminal_ids: Vec<TerminalId> = Vec::new();
        let name = &self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre(eyre!("Session not found"))?
            .name;

        let windows = self
            .session_manager
            .get_all_windows_from_session(id)
            .cloned()
            .unwrap_or_default();

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

        self.systemd_manager.remove_sysd_service_file(name)?;
        self.yaml_manager.delete_session(name)?;
        self.session_manager.kill_session(id);

        Ok(self.get_all_session_info())
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
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|window_id| {
                            self.window_manager
                                .get_window_by_id(window_id)
                                .get_window_info()
                        })
                        .collect(),
                    match &session.kind {
                        SessionKind::Ephemoral { .. } => None,
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
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|window_id| {
                    self.window_manager
                        .get_window_by_id(window_id)
                        .get_window_info()
                })
                .collect(),
            match &session.kind {
                SessionKind::Ephemoral { .. } => None,
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
