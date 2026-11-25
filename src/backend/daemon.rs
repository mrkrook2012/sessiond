//! The daemon: its state (`Daemon`) and the task that runs it (`run`).

use tracing::{debug, error, info, warn};
use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::client_view::ClientView;
use tokio::sync::{Mutex, watch};

use crate::backend::session::{RunId, SessionId, Sessions};
use crate::backend::systemd::UnitName;
use crate::backend::session_runtime::SessionRuntime;
use crate::backend::storage::SessionStore;
use crate::backend::terminal::{TerminalId, Terminals};
use crate::backend::terminal::virtual_terminal::TerminalEvent;
use crate::backend::window::{WindowId, Windows};
use crate::backend::storage::session_files::{SessionFiles, SessionConfig, WindowConfig};
use crate::backend::systemd::client::{SystemdClient, watch_events};
use crate::ipc::consts::socket_path;
use crate::ipc::server::run_server;
use crate::ipc::protocol::{SessionInfo, SessionLookup, SysdWaitStatus, TerminalUpdate};
use color_eyre::eyre::{Result, bail};

/// Runs the daemon until it is killed: restores stored sysd sessions, starts the
/// terminal-event and systemd watchers and serves IPC requests.
pub async fn run() -> Result<()> {
    let view = ClientView::new();
    let windows = Windows::new();
    let (terminal_events, mut terminal_event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let terminals = Terminals::new(terminal_events);
    let sessions = Sessions::new();
    let files = SessionFiles::new()?;

    

    let mut daemon = Daemon::new(
        view,
        terminals,
        windows,
        sessions,
        files
    );

    info!(pid = std::process::id(), "Starting daemon");

    // Pick up sysd sessions that exist on disk but were never started by this daemon.
    // A failure here should not keep the daemon from starting.
    match daemon.register_stored_sysd_sessions() {
        Ok(count) => info!("Registered {count} stored sysd session(s)"),
        Err(e) => error!("Failed to scan for stored sysd sessions: {e:#}"),
    }

    let daemon = Arc::new(Mutex::new(daemon));

    // Events from terminals (tracked commands exiting) are applied here, outside the PTY threads.
    let event_daemon = daemon.clone();
    tokio::spawn(async move {
        while let Some(event) = terminal_event_receiver.recv().await {
            event_daemon.lock().await.handle_terminal_event(event);
        }
    });

    // Without a session bus (CI, containers) the daemon still works; sysd actions just fail.
    let systemd = match SystemdClient::connect().await {
        Ok(client) => {
            let watcher_client = client.clone();
            let watcher_daemon = daemon.clone();
            tokio::spawn(async move {
                if let Err(e) = watch_events(watcher_client, watcher_daemon).await {
                    error!("systemd event watcher stopped: {e:#}");
                }
            });
            Some(client)
        }
        Err(e) => {
            warn!("systemd integration disabled: {e:#}");
            None
        }
    };

    run_server(daemon, systemd).await?;

    Ok(())
}

/// Entry point for IPC requests. Holds no logic of its own beyond coordinating:
/// - `runtime`: live terminals / windows / sessions
/// - `store`:   YAML files and systemd units
pub struct Daemon {
    runtime: SessionRuntime,
    store: SessionStore,
    /// Bumped whenever something a `wait_sysd_session` caller cares about may have changed.
    changes: watch::Sender<u64>,
}

impl Daemon {
    pub fn new(
        view: ClientView,
        terminals: Terminals,
        windows: Windows,
        sessions: Sessions,
        files: SessionFiles,
    ) -> Self {
        Self {
            runtime: SessionRuntime::new(
                view,
                terminals,
                windows,
                sessions,
            ),
            store: SessionStore::new(files),
            changes: watch::Sender::new(0),
        }
    }

    fn notify_changed(&self) {
        self.changes.send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    // ------------------------------------------------ sysd run tracking

    /// Status of one run of a sysd session, plus a receiver that fires on the next change.
    /// Subscribing under the same lock as the check means no change can be missed.
    pub fn sysd_wait_status(&self, name: &str, run: RunId) -> (SysdWaitStatus, watch::Receiver<u64>) {
        let receiver = self.changes.subscribe();
        (self.runtime.sysd_wait_status(name, run), receiver)
    }

    pub fn handle_terminal_event(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::TrackedCommandExited { terminal_id, code } => {
                if self.runtime.handle_tracked_exit(terminal_id, code) {
                    self.notify_changed();
                }
            }
        }
    }

    // ------------------------------------------------ pure runtime delegation

    pub fn get_wanted_data(&self, terminal_id: TerminalId, scrollback: u32, since: u64) -> Result<TerminalUpdate> {
        self.runtime.get_terminal_data(terminal_id, scrollback, since)
    }

    pub fn handle_write(&mut self, terminal_id: TerminalId, data: Vec<u8>) -> Result<()> {
        self.runtime.write(terminal_id, data)
    }

    pub fn handle_start(&mut self, rows: u16, cols: u16) {
        self.runtime.add_user(rows, cols);
    }

    pub fn handle_update_screen_size(&mut self, rows: u16, cols: u16) {
        self.runtime.update_screen_size(rows, cols);
    }

    pub fn handle_change_window(&mut self, window_id: WindowId) -> Result<()> {
        self.runtime.change_window(window_id)
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

    pub fn handle_session_id_by_name(&self, name: &str) -> SessionLookup {
        self.runtime.lookup_by_name(name)
    }

    pub fn get_all_session_info(&self) -> Vec<SessionInfo> {
        self.runtime.all_session_info()
    }

    pub fn handle_create_window(&mut self, id: SessionId) -> Result<SessionInfo> {
        self.runtime.create_window(id)
    }

    pub fn handle_kill_window(
        &mut self,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<SessionInfo> {
        self.runtime.kill_window(id, window_id)?;
        self.notify_changed();

        // TODO: if that was the last window, the session is gone and this returns an error.
        // The old code panicked on an unwrap here. Consider returning Option<SessionInfo>.
        self.runtime
            .session_info(id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Session no longer exists"))
    }

    pub fn handle_kill_session(&mut self, id: SessionId) -> Vec<SessionInfo> {
        self.runtime.kill_session(id);
        debug!(?id, "Killed session");
        self.notify_changed();
        self.runtime.all_session_info()
    }

    pub fn handle_create_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>> {
        debug!(?name, ?command, "Creating session");
        self.runtime
            .create_ephemeral_session(working_dir, command, name)?;
        Ok(self.runtime.all_session_info())
    }

    pub fn handle_stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        self.runtime.stop_sysd_session(id)?;
        info!(?id, "Stopped sysd session");
        self.notify_changed();
        Ok(())
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
        self.notify_changed();
    }

    // ------------------------------------------------ runtime + store

    pub fn handle_start_session_from_file(&mut self, file_path: PathBuf) -> Result<()> {
        let yaml = self.store.load(&file_path)?;
        self.runtime.check_new_session_name(&yaml.name)?;

        let windows = self.runtime.spawn_windows(yaml.windows, false)?;
        self.runtime.add_ephemeral_session(yaml.name.clone(), windows)?;
        info!(session = %yaml.name, "Started session from file");

        Ok(())
    }

    /// Starts (or restarts) a sysd session from its YAML. Returns the id of this run,
    /// which `__internal-systemd-start` then waits on.
    pub fn handle_start_sysd_session_from_file(&mut self, file_path: PathBuf) -> Result<RunId> {
        let yaml = self.store.load(&file_path)?;
        let name = UnitName::new(&yaml.name)?;

        let run = self.runtime.start_or_create_sysd_session(name.clone(), yaml.windows)?;
        self.store.record_sysd_config(name.clone(), file_path);
        info!(session = %name, ?run, "Started sysd session");
        self.notify_changed();

        Ok(run)
    }

    /// Writes the live layout of a running sysd session (its windows, their commands and
    /// directories) back to the YAML file its unit starts from, so the next start of the
    /// service brings it back like this. The running session itself is left alone.
    /// Returns the file written.
    pub fn handle_update_sysd_session(&mut self, id: SessionId) -> Result<PathBuf> {
        let unit = self.runtime.sysd_unit(id)?;
        let running = self
            .runtime
            .session_info(id)
            .and_then(|session| session.sysd_data)
            .is_some_and(|sysd| sysd.running);
        if !running {
            bail!("Service '{unit}' is stopped: start it, change it, then update it");
        }
        let Some(path) = self.store.sysd_config(&unit).map(PathBuf::from) else {
            bail!("Don't know which file service '{unit}' was started from");
        };
        let yaml = self.runtime.to_yaml(id)?;

        self.store.save_to(&yaml, &path)?;
        info!(session = %unit, path = %path.display(), windows = yaml.windows.len(), "Updated sysd session file");
        Ok(path)
    }

    pub fn handle_export_session_to_file(&self, id: SessionId, path: PathBuf) -> Result<PathBuf> {
        let yaml = self.runtime.to_yaml(id)?;
        self.store.export_to_dir(&yaml, path)
    }

    /// The unit a new service called `name` would get, checked against everything the
    /// daemon knows: the name is valid and no session uses it. Whether systemd already has
    /// a unit by that name is asked separately (`SystemdClient::ensure_unit_name_free`),
    /// because that is a D-Bus call and must not be made while holding the daemon lock.
    pub fn new_service_unit(&self, name: Option<&str>) -> Result<UnitName> {
        let Some(name) = name else {
            bail!("A service needs a name");
        };
        let unit = UnitName::new(name)?;
        self.runtime.check_new_session_name(unit.as_str())?;
        Ok(unit)
    }

    /// The unit `handle_export_session` would create for session `id`.
    pub fn export_unit(&self, id: SessionId) -> Result<UnitName> {
        UnitName::new(self.runtime.session_name(id)?)
    }

    /// Turns a running ephemeral session into a systemd-backed one.
    pub fn handle_export_session(&mut self, id: SessionId) -> Result<()> {
        let mut yaml = self.runtime.to_yaml(id)?;

        // Validate the name before doing anything destructive.
        let unit = UnitName::new(&yaml.name)?;
        // The service is known by its unit name from now on; store it under that name too,
        // so its YAML file is `<unit>.yaml` and matches the unit.
        yaml.name = unit.as_str().to_owned();

        // Persist before killing, so a failed save leaves the original session intact.
        let path = self.store.save(&yaml)?;

        self.runtime.kill_session(id);

        self.store.create_service(&path, &unit)?;
        Ok(())
    }

    pub fn handle_create_sysd_session(
        &mut self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>> {
        // We can only create a stopped session here; the client sends Start later.
        let Some(command) = command else {
            bail!("A service needs a command to run");
        };
        // Validate the name and check it is free before writing anything to disk.
        let unit = self.new_service_unit(name.as_deref())?;

        let yaml = SessionConfig {
            name: unit.as_str().to_owned(),
            windows: vec![WindowConfig {
                command,
                working_directory: working_dir,
                // A service runs its command; restart it when it exits.
                tracked: true,
            }],
        };

        let path = self.store.save(&yaml)?;
        if let Err(e) = self.store.create_service(&path, &unit) {
            // Do not leave a YAML file behind that no unit points at.
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }

        self.runtime.add_stopped_sysd_session(unit, false)?;
        Ok(self.runtime.all_session_info())
    }

    /// Registers every sysd session that exists on disk but is not known to this
    /// daemon yet as a stopped session. Called once at daemon startup, so services
    /// that were created earlier but never started show up and can be managed.
    /// Returns the number of sessions registered.
    pub fn register_stored_sysd_sessions(&mut self) -> Result<usize> {
        let mut registered = 0;

        for stored in self.store.discover_sysd_sessions()? {
            if self.runtime.session_name_exists(stored.unit.as_str()) {
                continue;
            }
            debug!("Registering stopped sysd session '{}'", stored.unit);
            // The enabled state is filled in by the systemd event watcher once it connects.
            match self.runtime.add_stopped_sysd_session(stored.unit.clone(), false) {
                Ok(_) => registered += 1,
                Err(e) => warn!("Skipping stored sysd session '{}': {e}", stored.unit),
            }
        }

        Ok(registered)
    }

    /// The unit a `service delete <name>` refers to, and whether systemd needs to be told
    /// (it has a unit file). Errors if `name` is an ephemeral session.
    pub fn sysd_delete_target(&self, name: &str) -> Result<(UnitName, bool)> {
        let unit = UnitName::new(name)?;
        if let Some(id) = self.runtime.session_id_by_name(unit.as_str()) {
            // Errors for ephemeral sessions.
            self.runtime.sysd_unit(id)?;
        }
        let has_unit_file = self.store.unit_file_exists(&unit);
        Ok((unit, has_unit_file))
    }

    /// Deletes a sysd session: its `.service` and `.yaml` files, and the live session if the
    /// daemon has one. Also works for leftovers the daemon never registered (e.g. a YAML
    /// whose unit file is gone). Returns the removed files.
    pub fn handle_delete_sysd_session(&mut self, unit: &UnitName) -> Result<Vec<PathBuf>> {
        let session = self.runtime.session_id_by_name(unit.as_str());

        // Do the fallible disk work first so a failure leaves live state untouched.
        let removed = self.store.delete(unit)?;
        info!(unit = %unit, files = removed.len(), "Deleted sysd session");

        match session {
            Some(id) => {
                self.runtime.kill_session(id);
                self.notify_changed();
            }
            None if removed.is_empty() => {
                bail!("No service named '{unit}'")
            }
            None => {}
        }

        Ok(removed)
    }

    // ------------------------------------------------ process

    /// Stops the daemon and every process it started.
    ///
    /// The socket file goes first: a waiting `__internal-systemd-start` that loses its
    /// connection checks for it to tell this requested shutdown from a crash (and only
    /// fails, making systemd restart it, on a crash). Holding the daemon lock throughout
    /// also keeps tracked commands that die below from being reported as exits.
    pub fn handle_kill_daemon(&mut self) -> ! {
        info!("Shutdown requested");
        if let Err(e) = std::fs::remove_file(socket_path()) {
            warn!("Failed to remove socket {}: {e}", socket_path().display());
        }
        self.runtime.shutdown();
        info!("All sessions stopped, exiting");
        std::process::exit(0);
    }
}
