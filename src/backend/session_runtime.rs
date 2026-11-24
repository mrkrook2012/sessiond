use crate::backend::session_manager::{SessionId, SessionKind, SessionManager, UnitName};
use crate::backend::terminal_manager::{TerminalId, TerminalManager};
use crate::backend::window_manager::{WindowId, WindowManager};
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::ipc::protocol::{SessionInfo, TerminalUpdate};
use color_eyre::eyre::{OptionExt, Result, bail, eyre};

/// Everything that concerns *live* state: users, terminals, windows and sessions.
///
/// It knows nothing about YAML files or systemd. Persistence lives in `SessionStore`.
pub struct SessionRuntime {
    user_manager: MiniUserManager,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
}

impl SessionRuntime {
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

    // ---------------------------------------------------------------- terminal io

    pub fn get_terminal_data(&mut self, terminal_id: TerminalId) -> TerminalUpdate {
        self.terminal_manager.get_data_from_terminal(terminal_id)
    }

    pub fn write(&mut self, terminal_id: TerminalId, data: Vec<u8>) {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice());
    }

    // ---------------------------------------------------------------- client view

    pub fn add_user(&mut self, rows: u16, cols: u16) {
        self.user_manager.add_user(rows, cols);
    }

    pub fn update_screen_size(&mut self, rows: u16, cols: u16) {
        self.user_manager.update_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
    }

    pub fn change_window(&mut self, window_id: WindowId) {
        self.user_manager.clear_all();

        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);

        self.user_manager.add_terminal(terminal_id);
    }

    pub fn exit_session(&mut self) {
        self.user_manager.clear_all();
    }

    pub fn open_session(&mut self, id: SessionId) -> Result<SessionInfo> {
        self.user_manager.set_current_id(id);

        let window_id = self
            .windows_of(id)
            .into_iter()
            .next()
            .ok_or_else(|| eyre!("No windows found for session"))?;

        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);
        self.user_manager.add_terminal(terminal_id);

        self.session_info(id)
            .ok_or_else(|| eyre!("Session info not found"))
    }

    // ---------------------------------------------------------------- creation

    /// Spawns one terminal + window per entry. Used when loading sessions from YAML.
    pub fn spawn_windows(&mut self, windows: Vec<YamlWindow>) -> Vec<WindowId> {
        windows
            .into_iter()
            .map(|window| {
                let terminal_id = self.terminal_manager.create_terminal(100, 100);

                self.terminal_manager.execute_command_on_terminal(
                    terminal_id,
                    format!("cd {}", window.working_directory),
                );
                self.terminal_manager
                    .execute_command_on_terminal(terminal_id, window.command);

                self.window_manager.create_window(terminal_id)
            })
            .collect()
    }

    /// Interactive creation of a single-window ephemeral session, sized to the current user.
    pub fn create_ephemeral_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) {
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
    }

    pub fn add_ephemeral_session(&mut self, name: String, windows: Vec<WindowId>) {
        self.session_manager.create_ephem_session(windows, name);
    }

    /// If a sysd session with this name already exists, start it; otherwise create it running.
    pub fn start_or_create_sysd_session(&mut self, name: UnitName, windows: Vec<WindowId>) {
        let name_str = name.to_string();

        if self.session_manager.does_session_name_exist(&name_str) {
            let id = self.session_manager.get_id_by_name(&name_str).unwrap();
            self.session_manager.start_sysd_session(id, windows);
        } else {
            self.session_manager
                .create_running_sysd_session(name, windows);
        }
    }

    pub fn add_stopped_sysd_session(&mut self, unit: UnitName, enabled: bool) {
        self.session_manager.create_stopped_sysd_session(unit, enabled);
    }

    pub fn session_name_exists(&self, name: &str) -> bool {
        self.session_manager.does_session_name_exist(name)
    }

    pub fn create_window(&mut self, id: SessionId) -> Result<SessionInfo> {
        let (rows, cols) = self.user_manager.get_dimensions();

        let terminal_id = self.terminal_manager.create_terminal(rows, cols);
        let window_id = self.window_manager.create_window(terminal_id);

        self.session_manager.add_window_to_session(id, window_id)?;

        self.user_manager.clear_all();
        self.user_manager.add_terminal(terminal_id);

        self.session_info(id)
            .ok_or_eyre("Session not found after adding window")
    }

    // ---------------------------------------------------------------- teardown

    /// Kills every terminal and window belonging to the session.
    /// The session entry itself is left to the caller (stop vs. kill).
    fn teardown_windows(&mut self, id: SessionId) {
        for window_id in self.windows_of(id) {
            let terminal_id = self
                .window_manager
                .get_terminal_id_from_window_id(&window_id);

            self.terminal_manager.kill_terminal(terminal_id);
            self.user_manager.remove_terminal(terminal_id);
            self.window_manager.kill_window(window_id);
        }
    }

    pub fn stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        if self.session_manager.get_all_windows_from_session(id).is_none() {
            bail!("Found no windows in session");
        }

        self.teardown_windows(id);
        self.session_manager.stop_sysd_session(id);
        Ok(())
    }

    pub fn kill_session(&mut self, id: SessionId) {
        self.teardown_windows(id);
        self.session_manager.kill_session(id);
    }

    pub fn kill_window(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let terminal_id = self
            .window_manager
            .get_terminal_id_from_window_id(&window_id);

        self.terminal_manager.kill_terminal(terminal_id);
        self.user_manager.remove_terminal(terminal_id);
        self.window_manager.kill_window(window_id);

        self.session_manager
            .remove_window_from_session(id, window_id)?;

        if self.windows_of(id).is_empty() {
            self.session_manager.kill_session(id);
        }

        Ok(())
    }

    // ---------------------------------------------------------------- sysd state

    /// The unit behind a sysd session. Errors for ephemeral sessions.
    pub fn sysd_unit(&self, id: SessionId) -> Result<UnitName> {
        let session = self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre("Session not found")?;

        match &session.kind {
            SessionKind::Ephemoral { .. } => {
                bail!("Cannot call sysd operation on non-sysd session")
            }
            SessionKind::SysdBased(sysd_session) => Ok(sysd_session.unit().clone()),
        }
    }

    pub fn sysd_units(&self) -> Vec<UnitName> {
        self.session_manager.sysd_units()
    }

    pub fn set_sysd_enabled(&mut self, unit: &UnitName, enabled: bool) {
        if let Some((_, sysd)) = self.session_manager.get_sysd_session_by_unit_mut(unit) {
            sysd.set_enabled(enabled);
        }
    }

    /// systemd reports the unit as no longer active: stop the session if it is still running.
    pub fn handle_unit_deactivated(&mut self, unit: &UnitName) {
        let Some((id, sysd)) = self.session_manager.get_sysd_session_by_unit_mut(unit) else {
            return;
        };
        if sysd.is_running() {
            println!("Unit {} went inactive, stopping session", unit.to_string());
            let _ = self.stop_sysd_session(id);
        }
    }

    // ---------------------------------------------------------------- queries

    pub fn session_name(&self, id: SessionId) -> Result<String> {
        Ok(self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre("Session not found")?
            .name
            .clone())
    }

    /// Builds the YAML representation of a running session. Does not write anything.
    pub fn to_yaml(&self, id: SessionId) -> Result<YamlSession> {
        let session = self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre("Session not found")?;

        if self.session_manager.get_all_windows_from_session(id).is_none() {
            bail!("Trying to get windowless sessions to yaml, error");
        }

        let windows = self
            .windows_of(id)
            .iter()
            .map(|window_id| {
                let terminal_id = self
                    .window_manager
                    .get_terminal_id_from_window_id(window_id);

                YamlWindow {
                    command: self.terminal_manager.get_command_from_terminal(terminal_id),
                    working_directory: self
                        .terminal_manager
                        .get_working_dir_from_terminal(terminal_id),
                }
            })
            .collect();

        Ok(YamlSession {
            name: session.name.clone(),
            windows,
        })
    }

    pub fn session_info(&self, id: SessionId) -> Option<SessionInfo> {
        let session = self.session_manager.get_session_by_id(id)?;

        let windows: Vec<_> = session
            .get_windows()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|window_id| {
                self.window_manager
                    .get_window_by_id(window_id)
                    .get_window_info()
            })
            .collect();

        let sysd_data = match &session.kind {
            SessionKind::Ephemoral { .. } => None,
            SessionKind::SysdBased(sysd_session) => Some(sysd_session.get_sysd_data()),
        };

        Some(SessionInfo::new(
            id,
            session.name.clone(),
            windows,
            sysd_data,
        ))
    }

    pub fn all_session_info(&self) -> Vec<SessionInfo> {
        let ids: Vec<SessionId> = self
            .session_manager
            .list_sessions()
            .iter()
            .map(|session| session.id)
            .collect();

        ids.into_iter()
            .filter_map(|id| self.session_info(id))
            .collect()
    }

    fn windows_of(&self, id: SessionId) -> Vec<WindowId> {
        self.session_manager
            .get_all_windows_from_session(id)
            .cloned()
            .unwrap_or_default()
    }
}