use tracing::{info, warn};
use std::time::Duration;

use crate::backend::session_manager::{
    RunId, SessionId, SessionKind, SessionManager, SessionState,
};
use crate::backend::terminal_manager::{TerminalId, TerminalManager};
use crate::backend::window_manager::{WindowId, WindowManager};
use crate::backend::client_view::ClientView;
use crate::backend::systemd::UnitName;
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::ipc::protocol::{SessionInfo, SessionLookup, SysdWaitStatus, TerminalUpdate};
use color_eyre::eyre::{OptionExt, Result, bail, eyre};

/// A tracked command that exits within this long after the user typed into its window
/// counts as stopped by hand (Ctrl+C, `stop` in a server console), and does not restart the session.
const MANUAL_STOP_WINDOW: Duration = Duration::from_secs(30);

/// Everything that concerns *live* state: the client view, terminals, windows and sessions.
///
/// It knows nothing about YAML files or systemd. Persistence lives in `SessionStore`.
pub struct SessionRuntime {
    view: ClientView,
    terminal_manager: TerminalManager,
    window_manager: WindowManager,
    session_manager: SessionManager,
}

impl SessionRuntime {
    pub fn new(
        view: ClientView,
        terminal_manager: TerminalManager,
        window_manager: WindowManager,
        session_manager: SessionManager,
    ) -> Self {
        Self {
            view,
            terminal_manager,
            window_manager,
            session_manager,
        }
    }

    // ---------------------------------------------------------------- terminal io

    pub fn get_terminal_data(&self, terminal_id: TerminalId, scrollback: u32, since: u64) -> Result<TerminalUpdate> {
        self.terminal_manager
            .snapshot(terminal_id, scrollback, since)
            .ok_or_else(|| eyre!("Terminal not found (its window may have been closed)"))
    }

    pub fn write(&mut self, terminal_id: TerminalId, data: Vec<u8>) -> Result<()> {
        self.terminal_manager
            .send_bytes_to_terminal(terminal_id, data.as_slice())
    }

    // ---------------------------------------------------------------- client view

    /// A client connected with a screen of this size. Every terminal is sized to it, so
    /// what programs draw (and what the client is sent) fits the client's pane.
    pub fn add_user(&mut self, rows: u16, cols: u16) {
        self.view.set_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
    }

    pub fn update_screen_size(&mut self, rows: u16, cols: u16) {
        self.view.set_dimensions(rows, cols);
        self.terminal_manager.resize_all_terminals(rows, cols);
    }

    pub fn change_window(&mut self, window_id: WindowId) -> Result<()> {
        let terminal_id = self.terminal_of(window_id)?;

        self.view.hide_all();
        self.view.show_terminal(terminal_id);
        Ok(())
    }

    pub fn exit_session(&mut self) {
        self.view.hide_all();
    }

    pub fn open_session(&mut self, id: SessionId) -> Result<SessionInfo> {
        let window_id = self
            .session_manager
            .get_all_windows_from_session(id)
            .and_then(|windows| windows.first().copied())
            .ok_or_else(|| eyre!("No windows found for session"))?;
        let terminal_id = self.terminal_of(window_id)?;

        self.view.set_current_session(id);
        self.view.show_terminal(terminal_id);

        self.session_info(id)
            .ok_or_else(|| eyre!("Session info not found"))
    }

    // ---------------------------------------------------------------- creation

    /// Errors unless `name` is a valid session name that no session uses yet.
    /// Check before spawning anything, so a bad name costs nothing.
    pub fn check_new_session_name(&self, name: &str) -> Result<()> {
        self.session_manager.check_new_name(name)
    }

    /// Spawns one terminal + window per entry. Used when loading sessions from YAML.
    ///
    /// With `track`, windows marked `tracked` report when their command exits; only
    /// sysd sessions do anything with that, so ephemeral sessions pass `false`.
    ///
    /// All or nothing: if one terminal cannot be created, the ones already spawned are
    /// torn down again.
    pub fn spawn_windows(&mut self, windows: Vec<YamlWindow>, track: bool) -> Result<Vec<WindowId>> {
        let mut spawned = Vec::with_capacity(windows.len());
        for window in windows {
            match self.spawn_window(&window.working_directory, Some(window.command), track, window.tracked) {
                Ok(window_id) => spawned.push(window_id),
                Err(e) => {
                    self.destroy_windows(&spawned);
                    return Err(e);
                }
            }
        }
        Ok(spawned)
    }

    /// One terminal sized to the current user, its window, and `command` launched in it.
    fn spawn_window(
        &mut self,
        working_dir: &str,
        command: Option<String>,
        track: bool,
        tracked: bool,
    ) -> Result<WindowId> {
        let (rows, cols) = self.view.dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols)?;
        self.terminal_manager
            .launch_on_terminal(terminal_id, working_dir, command, track && tracked)?;
        Ok(self
            .window_manager
            .create_window_with_tracking(terminal_id, tracked))
    }

    /// Interactive creation of a single-window ephemeral session, sized to the current user.
    /// Without a name, the session gets the first free `session-N`.
    pub fn create_ephemeral_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<SessionId> {
        let name = name.unwrap_or_else(|| self.session_manager.free_default_name());
        self.check_new_session_name(&name)?;

        let window_id = self.spawn_window(&working_dir, command, false, false)?;
        self.add_ephemeral_session(name, vec![window_id])
    }

    /// Registers already spawned windows as an ephemeral session. If the name is
    /// rejected, the windows are torn down, so they never outlive a failed call.
    pub fn add_ephemeral_session(&mut self, name: String, windows: Vec<WindowId>) -> Result<SessionId> {
        self.session_manager
            .create_ephem_session(windows.clone(), name)
            .inspect_err(|_| self.destroy_windows(&windows))
    }

    /// Starts sysd session `name` with these windows: restarts it if it already exists,
    /// otherwise creates it running. Returns the id of this run.
    ///
    /// Fails, without touching any session or spawning anything, if `name` belongs to an
    /// ephemeral session.
    pub fn start_or_create_sysd_session(
        &mut self,
        name: UnitName,
        windows: Vec<YamlWindow>,
    ) -> Result<RunId> {
        let existing = self.session_manager.get_id_by_name(name.as_str());
        match existing {
            Some(id) => {
                if self.sysd_unit(id).is_err() {
                    let existing_name = self.session_name(id)?;
                    bail!(
                        "Cannot start service '{name}': ephemeral session '{existing_name}' already uses that name"
                    );
                }
            }
            None => self.check_new_session_name(name.as_str())?,
        }

        let windows = self.spawn_windows(windows, true)?;

        let result = match existing {
            Some(id) => {
                // Should already be stopped (ExecStop runs first), but never leak old windows.
                self.teardown_windows(id);
                self.session_manager.start_sysd_session(id, windows.clone())
            }
            None => self
                .session_manager
                .create_running_sysd_session(name, windows.clone()),
        };
        result.inspect_err(|_| self.destroy_windows(&windows))
    }

    pub fn add_stopped_sysd_session(&mut self, unit: UnitName, enabled: bool) -> Result<SessionId> {
        self.session_manager.create_stopped_sysd_session(unit, enabled)
    }

    pub fn session_name_exists(&self, name: &str) -> bool {
        self.session_manager.does_session_name_exist(name)
    }

    pub fn create_window(&mut self, id: SessionId) -> Result<SessionInfo> {
        // Check first, so a session that cannot take a window does not leak a terminal.
        if self.session_manager.get_all_windows_from_session(id).is_none() {
            bail!("Session not found, or it is a stopped service");
        }

        let (rows, cols) = self.view.dimensions();
        let terminal_id = self.terminal_manager.create_terminal(rows, cols)?;
        let window_id = self.window_manager.create_window(terminal_id);

        if let Err(e) = self.session_manager.add_window_to_session(id, window_id) {
            self.destroy_windows(&[window_id]);
            return Err(e);
        }

        self.view.hide_all();
        self.view.show_terminal(terminal_id);

        self.session_info(id)
            .ok_or_eyre("Session not found after adding window")
    }

    // ---------------------------------------------------------------- teardown

    /// Kills every terminal and window belonging to the session.
    /// The session entry itself is left to the caller (stop vs. kill).
    fn teardown_windows(&mut self, id: SessionId) {
        let windows = self.windows_of(id);
        self.destroy_windows(&windows);
    }

    /// Kills these windows and their terminals. Windows that are already gone are skipped,
    /// so this never fails halfway and leaves the managers disagreeing.
    fn destroy_windows(&mut self, windows: &[WindowId]) {
        for &window_id in windows {
            let Some(window) = self.window_manager.kill_window(window_id) else {
                warn!(?window_id, "Tried to destroy a window that does not exist");
                continue;
            };
            self.terminal_manager.kill_terminal(window.terminal_id);
            self.view.hide_terminal(window.terminal_id);
        }
    }

    pub fn stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        // Reject ephemeral sessions before tearing anything down.
        self.sysd_unit(id)?;

        if self.session_manager.get_all_windows_from_session(id).is_none() {
            bail!("Found no windows in session");
        }

        self.teardown_windows(id);
        self.session_manager.stop_sysd_session(id)
    }

    /// Stops every terminal of every session and waits until their processes are gone.
    pub fn shutdown(&mut self) {
        self.terminal_manager.shutdown();
    }

    pub fn kill_session(&mut self, id: SessionId) {
        self.teardown_windows(id);
        self.session_manager.kill_session(id);
    }

    pub fn kill_window(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let in_session = self
            .session_manager
            .get_all_windows_from_session(id)
            .is_some_and(|windows| windows.contains(&window_id));
        if !in_session {
            bail!("Window not found in this session");
        }

        self.session_manager
            .remove_window_from_session(id, window_id)?;
        self.destroy_windows(&[window_id]);

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

    pub fn lookup_by_name(&self, name: &str) -> SessionLookup {
        self.session_manager.lookup_by_name(name)
    }

    pub fn session_id_by_name(&self, name: &str) -> Option<SessionId> {
        self.session_manager.get_id_by_name(name)
    }

    pub fn sysd_units(&self) -> Vec<UnitName> {
        self.session_manager.sysd_units()
    }

    pub fn set_sysd_enabled(&mut self, unit: &UnitName, enabled: bool) {
        if let Some((_, sysd)) = self.session_manager.get_sysd_session_by_unit_mut(unit) {
            sysd.set_enabled(enabled);
        }
    }

    /// A tracked command exited. Records it on the session unless it looks like the user
    /// stopped it by hand, in which case the session is left alone.
    /// Returns true if the session state changed.
    pub fn handle_tracked_exit(&mut self, terminal_id: TerminalId, code: i32) -> bool {
        let Some(window_id) = self.window_manager.get_window_id_from_terminal(terminal_id) else {
            return false;
        };
        if !self
            .window_manager
            .get_window_by_id(&window_id)
            .is_some_and(|window| window.tracked)
        {
            return false;
        }
        let Some(id) = self.session_manager.get_session_id_by_window(window_id) else {
            return false;
        };

        let manual = self
            .terminal_manager
            .last_user_input(terminal_id)
            .is_some_and(|at| at.elapsed() < MANUAL_STOP_WINDOW);

        let Some(session) = self.session_manager.get_session_by_id_mut(id) else {
            return false;
        };
        let SessionKind::SysdBased(sysd) = &mut session.kind else {
            return false;
        };

        if manual {
            info!(
                session = %session.name,
                code,
                "Tracked command exited shortly after user input; treating it as a manual stop, not restarting"
            );
            return false;
        }

        warn!(session = %session.name, code, "Tracked command exited, session will be restarted");
        sysd.record_tracked_exit(code);
        true
    }

    /// Where run `run` of sysd session `name` stands, for `__internal-systemd-start`.
    pub fn sysd_wait_status(&self, name: &str, run: RunId) -> SysdWaitStatus {
        let Some(id) = self.session_manager.get_id_by_name(name) else {
            return SysdWaitStatus::Ended;
        };
        let Some(SessionKind::SysdBased(sysd)) = self
            .session_manager
            .get_session_by_id(id)
            .map(|session| &session.kind)
        else {
            return SysdWaitStatus::Ended;
        };

        match sysd.state() {
            SessionState::Running {
                run: current,
                tracked_exit,
                ..
            } if *current == run => match tracked_exit {
                Some(code) => SysdWaitStatus::TrackedExited { code: *code },
                None => SysdWaitStatus::Running,
            },
            _ => SysdWaitStatus::Ended,
        }
    }

    /// systemd reports the unit as no longer active: stop the session if it is still running.
    pub fn handle_unit_deactivated(&mut self, unit: &UnitName) {
        let Some((id, sysd)) = self.session_manager.get_sysd_session_by_unit_mut(unit) else {
            return;
        };
        if sysd.is_running() {
            info!(unit = %unit, "Unit went inactive, stopping its session");
            let _ = self.stop_sysd_session(id);
        }
    }

    // ---------------------------------------------------------------- queries

    pub fn session_name(&self, id: SessionId) -> Result<&str> {
        Ok(&self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre("Session not found")?
            .name)
    }

    /// Builds the YAML representation of a running session. Does not write anything.
    pub fn to_yaml(&self, id: SessionId) -> Result<YamlSession> {
        let session = self
            .session_manager
            .get_session_by_id(id)
            .ok_or_eyre("Session not found")?;

        let Some(window_ids) = self.session_manager.get_all_windows_from_session(id) else {
            bail!("Trying to get windowless sessions to yaml, error");
        };

        // A sysd session already says which windows are tracked: a window added later
        // (a monitor like btop) must not start restarting the service when it is quit.
        // Ephemeral sessions have no say yet, so everything that runs something is tracked.
        let infer_tracking = matches!(session.kind, SessionKind::Ephemoral { .. });

        let windows = window_ids
            .iter()
            .map(|window_id| {
                let window = self
                    .window_manager
                    .get_window_by_id(window_id)
                    .ok_or_eyre("Session refers to a window that does not exist")?;
                let missing_terminal = || eyre!("Window refers to a terminal that does not exist");

                let command = self
                    .terminal_manager
                    .get_command_from_terminal(window.terminal_id)
                    .ok_or_else(missing_terminal)?;
                let working_directory = self
                    .terminal_manager
                    .get_working_dir_from_terminal(window.terminal_id)
                    .ok_or_else(missing_terminal)?;
                let tracked = window.tracked || (infer_tracking && !command.trim().is_empty());

                Ok(YamlWindow {
                    command,
                    working_directory,
                    tracked,
                })
            })
            .collect::<Result<_>>()?;

        Ok(YamlSession {
            name: session.name.clone(),
            windows,
        })
    }

    pub fn session_info(&self, id: SessionId) -> Option<SessionInfo> {
        let session = self.session_manager.get_session_by_id(id)?;

        let windows: Vec<_> = session
            .get_windows()
            .into_iter()
            .flatten()
            .filter_map(|window_id| self.window_manager.get_window_info_by_id(window_id))
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

    fn terminal_of(&self, window_id: WindowId) -> Result<TerminalId> {
        self.window_manager
            .get_terminal_id_from_window_id(&window_id)
            .ok_or_eyre("Window not found")
    }

    fn windows_of(&self, id: SessionId) -> Vec<WindowId> {
        self.session_manager
            .get_all_windows_from_session(id)
            .cloned()
            .unwrap_or_default()
    }
}
