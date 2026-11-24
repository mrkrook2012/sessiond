use std::path::PathBuf;
 
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::{SessionId, SessionManager, UnitName};
use crate::backend::session_runtime::SessionRuntime;
use crate::backend::session_store::SessionStore;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::{TerminalId, TerminalManager};
use crate::backend::window_manager::{WindowId, WindowManager};
use crate::backend::yaml::models::{YamlSession, YamlWindow};
use crate::backend::yaml::yaml_manager::YamlManager;
use crate::ipc::consts::socket_path;
use crate::ipc::protocol::{SessionInfo, TerminalUpdate};
use color_eyre::eyre::Result;
 
/// Entry point for IPC requests. Holds no logic of its own beyond coordinating:
/// - `runtime`: live terminals / windows / sessions
/// - `store`:   YAML files and systemd units
pub struct DaemonManager {
    runtime: SessionRuntime,
    store: SessionStore,
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
            runtime: SessionRuntime::new(
                user_manager,
                terminal_manager,
                window_manager,
                session_manager,
            ),
            store: SessionStore::new(yaml_manager, systemd_manager),
        }
    }
 
    // ------------------------------------------------ pure runtime delegation
 
    pub fn get_wanted_data(&mut self, terminal_id: TerminalId) -> TerminalUpdate {
        self.runtime.get_terminal_data(terminal_id)
    }
 
    pub fn handle_write(&mut self, terminal_id: TerminalId, data: Vec<u8>) {
        self.runtime.write(terminal_id, data);
    }
 
    pub fn handle_start(&mut self, rows: u16, cols: u16) {
        self.runtime.add_user(rows, cols);
    }
 
    pub fn handle_update_screen_size(&mut self, rows: u16, cols: u16) {
        self.runtime.update_screen_size(rows, cols);
    }
 
    pub fn handle_change_window(&mut self, window_id: WindowId) {
        self.runtime.change_window(window_id);
    }
 
    pub fn handle_exit_session(&mut self) -> Vec<SessionInfo> {
        self.runtime.exit_session();
        self.runtime.all_session_info()
    }
 
    pub fn handle_open_session(&mut self, id: SessionId) -> Result<SessionInfo> {
        self.runtime.open_session(id)
    }
 
    pub fn handle_list_sessions(&self) -> Vec<SessionInfo> {
        self.runtime.all_session_info()
    }
 
    pub fn get_all_session_info(&self) -> Vec<SessionInfo> {
        self.runtime.all_session_info()
    }
 
    pub fn handle_create_window(&mut self, id: SessionId) -> SessionInfo {
        // Matches the previous behaviour (panics on a bad id).
        self.runtime.create_window(id).unwrap()
    }
 
    pub fn handle_kill_window(
        &mut self,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<SessionInfo> {
        self.runtime.kill_window(id, window_id)?;
 
        // TODO: if that was the last window, the session is gone and this returns an error.
        // The old code panicked on an unwrap here. Consider returning Option<SessionInfo>.
        self.runtime
            .session_info(id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session no longer exists"))
    }
 
    pub fn handle_kill_session(&mut self, id: SessionId) -> Vec<SessionInfo> {
        self.runtime.kill_session(id);
        self.runtime.all_session_info()
    }
 
    pub fn handle_create_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        self.runtime
            .create_ephemeral_session(working_dir, command, name);
        self.runtime.all_session_info()
    }
 
    pub fn handle_stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        self.runtime.stop_sysd_session(id)
    }
 
    // ------------------------------------------------ sysd state (driven by systemd)

    /// The unit behind a sysd session; the server uses it to talk to systemd without holding the lock.
    pub fn sysd_unit(&self, id: SessionId) -> Result<UnitName> {
        self.runtime.sysd_unit(id)
    }

    pub fn sysd_units(&self) -> Vec<UnitName> {
        self.runtime.sysd_units()
    }

    pub fn set_sysd_enabled(&mut self, unit: &UnitName, enabled: bool) {
        self.runtime.set_sysd_enabled(unit, enabled);
    }

    pub fn handle_unit_deactivated(&mut self, unit: &UnitName) {
        self.runtime.handle_unit_deactivated(unit);
    }

    // ------------------------------------------------ runtime + store
 
    pub fn handle_start_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        let yaml = self.store.load(file_path)?;
 
        let windows = self.runtime.spawn_windows(yaml.windows);
        self.runtime.add_ephemeral_session(yaml.name, windows);
 
        Ok(())
    }
 
    pub fn handle_start_sysd_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        let yaml = self.store.load(file_path)?;
        let name = UnitName::new(&yaml.name)?;
 
        let windows = self.runtime.spawn_windows(yaml.windows);
        self.runtime.start_or_create_sysd_session(name, windows);
 
        Ok(())
    }
 
    pub fn handle_export_session_to_file(&self, id: SessionId, path: PathBuf) -> Result<PathBuf> {
        let yaml = self.runtime.to_yaml(id)?;
        self.store.export_to_dir(&yaml, path)
    }
 
    /// Turns a running ephemeral session into a systemd-backed one.
    pub fn handle_export_session(&mut self, id: SessionId) -> Result<()> {
        let yaml = self.runtime.to_yaml(id)?;
 
        // Validate the name before doing anything destructive.
        let unit = UnitName::new(&yaml.name)?;
 
        // Persist before killing, so a failed save leaves the original session intact.
        let path = self.store.save(&yaml)?;
 
        self.runtime.kill_session(id);
 
        self.store.create_service(&path, unit)?;
        Ok(())
    }
 
    pub fn handle_create_sysd_session(
        &mut self,
        working_dir: String,
        command: Option<String>, // TODO MUST BE SOME
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        // We can only create a stopped session here; the client sends Start later.
        let name = name.unwrap_or_else(|| "penar".to_string());
        let command = command.unwrap();
 
        let unit = UnitName::new(&name).expect("Invalid systemd unit name");
 
        let yaml = YamlSession {
            name,
            windows: vec![YamlWindow {
                command,
                working_directory: working_dir,
            }],
        };
 
        let path = self.store.save(&yaml).unwrap();
 
        // TODO, this might not actually succeed
        self.store.create_service(&path, unit.clone()).unwrap();
 
        self.runtime.add_stopped_sysd_session(unit, false);
        self.runtime.all_session_info()
    }
 
    /// Registers every sysd session that exists on disk but is not known to this
    /// daemon yet as a stopped session. Called once at daemon startup, so services
    /// that were created earlier but never started show up and can be managed.
    /// Returns the number of sessions registered.
    pub fn register_stored_sysd_sessions(&mut self) -> Result<usize> {
        let mut registered = 0;

        for stored in self.store.discover_sysd_sessions()? {
            if self.runtime.session_name_exists(&stored.unit.to_string()) {
                continue;
            }
            println!("Registering stopped sysd session '{}'", stored.unit.to_string());
            // The enabled state is filled in by the systemd event watcher once it connects.
            self.runtime.add_stopped_sysd_session(stored.unit, false);
            registered += 1;
        }

        Ok(registered)
    }

    pub fn handle_delete_sysd_session(&mut self, id: SessionId) -> Result<Vec<SessionInfo>> {
        let name = self.runtime.session_name(id)?;
 
        // Do the fallible disk/systemd work first so a failure leaves live state untouched.
        self.store.delete(&name)?;
        self.runtime.kill_session(id);
 
        Ok(self.runtime.all_session_info())
    }
 
    // ------------------------------------------------ process
 
    pub fn handle_kill_deamon(&self) -> ! {
        std::fs::remove_file(socket_path()).unwrap();
        std::process::exit(0);
    }
}
 