use tracing::{error, info, trace, warn};
use std::sync::Arc;

use color_eyre::eyre::Result;
use futures::StreamExt;
use tarpc::{
    context,
    server::{self, Channel},
    tokio_serde::formats::Bincode,
};
use tokio::sync::Mutex;

use crate::backend::{
    daemon::Daemon,
    sysd::UnitName,
    sysd::client::SysdClient,
};
use crate::ids::{RunId, SessionId, TerminalId, WindowId};
use crate::ipc::{
    consts::socket_path,
    protocol::{SessionInfo, SessionLookup, SysdAction, SysdWaitStatus, TerminalUpdate},
    service::DaemonService,
};

/// Upper bound for one `wait_sysd_session` call; clients loop.
const MAX_WAIT_MS: u64 = 60_000;

#[derive(Clone)]
pub struct DaemonRpcServer {
    daemon: Arc<Mutex<Daemon>>,
    /// `None` when the session bus / user systemd is not reachable (e.g. in CI).
    sysd: Option<SysdClient>,
}

impl DaemonRpcServer {
    pub fn new(daemon: Arc<Mutex<Daemon>>, sysd: Option<SysdClient>) -> Self {
        Self { daemon, sysd }
    }

    fn sysd(&self) -> Result<&SysdClient, String> {
        self.sysd
            .as_ref()
            .ok_or_else(|| "systemd is not available".to_string())
    }

    /// Errors if systemd rejects `unit` as a name or already has a unit by it, so we never
    /// write a unit file that replaces or shadows someone else's. Without systemd there is
    /// nothing to collide with, so the check is skipped.
    async fn ensure_unit_name_free(&self, unit: &UnitName) -> Result<(), String> {
        match &self.sysd {
            Some(sysd) => sysd
                .ensure_unit_name_free(unit)
                .await
                .map_err(|e| format!("{e:#}")),
            None => Ok(()),
        }
    }

    /// `daemon-reload` after unit files changed. Not fatal: the file is already written.
    async fn reload_sysd(&self) {
        match &self.sysd {
            Some(sysd) => {
                if let Err(e) = sysd.reload().await {
                    error!("{e:#}");
                }
            }
            None => warn!("systemd not available, skipping daemon-reload"),
        }
    }
}

// Note on locking: systemd calls are made *without* holding the daemon lock.
// Starting/stopping a unit runs its ExecStart/ExecStop, which call back into this daemon.

impl DaemonService for DaemonRpcServer {
    async fn get_wanted_data(
        self,
        _: context::Context,
        terminal_id: TerminalId,
        scrollback: u32,
        since: u64,
    ) -> Result<TerminalUpdate, String> {
        self.daemon
            .lock()
            .await
            .get_wanted_data(terminal_id, scrollback, since)
            .map_err(|e| e.to_string())
    }

    async fn start(self, _: context::Context, rows: u16, cols: u16) {
        let mut daemon = self.daemon.lock().await;
        daemon.handle_start(rows, cols);
    }

    async fn update_screen_size(self, _: context::Context, rows: u16, cols: u16) {
        let mut daemon = self.daemon.lock().await;
        daemon.handle_update_screen_size(rows, cols);
    }

    async fn create_session(
        self,
        _: context::Context,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>, String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_create_session(working_dir, command, name)
            .map_err(|e| e.to_string())
    }

    async fn create_sysd_session(
        self,
        _: context::Context,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>, String> {
        // The daemon's own checks first (cheap, and the clearer error), then systemd's,
        // without the lock. Creating re-checks the daemon's side under the lock.
        let unit = self
            .daemon
            .lock()
            .await
            .new_service_unit(name.as_deref())
            .map_err(|e| e.to_string())?;
        self.ensure_unit_name_free(&unit).await?;

        let sessions = self
            .daemon
            .lock()
            .await
            .handle_create_sysd_session(working_dir, command, name)
            .map_err(|e| format!("{e:#}"))?;
        self.reload_sysd().await;
        Ok(sessions)
    }

    async fn exit_session(self, _: context::Context) -> Vec<SessionInfo> {
        let mut daemon = self.daemon.lock().await;
        daemon.handle_exit_session()
    }

    async fn kill_session(self, _: context::Context, id: SessionId) -> Vec<SessionInfo> {
        let mut daemon = self.daemon.lock().await;
        daemon.handle_kill_session(id)
    }

    async fn write_terminal(
        self,
        _: context::Context,
        terminal_id: TerminalId,
        data: Vec<u8>,
    ) -> Result<(), String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_write(terminal_id, data)
            .map_err(|e| e.to_string())
    }

    async fn list_sessions(self, _: context::Context) -> Vec<SessionInfo> {
        let daemon = self.daemon.lock().await;
        daemon.handle_list_sessions()
    }

    async fn session_id_by_name(self, _: context::Context, name: String) -> SessionLookup {
        self.daemon.lock().await.handle_session_id_by_name(&name)
    }

    async fn create_window(self, _: context::Context, id: SessionId) -> Result<SessionInfo, String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_create_window(id)
            .map_err(|e| e.to_string())
    }

    async fn open_session(self, _: context::Context, id: SessionId) -> Result<SessionInfo, String> {
        let mut daemon = self.daemon.lock().await;
        daemon.handle_open_session(id).map_err(|e| e.to_string())
    }

    async fn change_window(self, _: context::Context, window_id: WindowId) -> Result<(), String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_change_window(window_id)
            .map_err(|e| e.to_string())
    }

    async fn kill_window(
        self,
        _: context::Context,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<SessionInfo, String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_kill_window(id, window_id)
            .map_err(|e| e.to_string())
    }

    async fn export_session_to_sysd(
        self,
        _: context::Context,
        id: SessionId,
    ) -> Result<(), String> {
        let unit = self
            .daemon
            .lock()
            .await
            .export_unit(id)
            .map_err(|e| e.to_string())?;
        self.ensure_unit_name_free(&unit).await?;

        self.daemon
            .lock()
            .await
            .handle_export_session(id)
            .map_err(|e| e.to_string())?;
        self.reload_sysd().await;
        Ok(())
    }

    async fn start_session_from_file(
        self,
        _: context::Context,
        file_path: std::path::PathBuf,
    ) -> Result<(), String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_start_session_from_file(file_path)
            .map_err(|e| e.to_string())
    }

    async fn kill_daemon(self, _: context::Context) {
        // Blocks this worker for at most the terminate grace period, then exits the process.
        self.daemon.lock().await.handle_kill_daemon()
    }

    async fn export_session_to_file(
        self,
        _: context::Context,
        id: SessionId,
        working_dir: std::path::PathBuf,
    ) -> Result<std::path::PathBuf, String> {
        let daemon = self.daemon.lock().await;
        daemon
            .handle_export_session_to_file(id, working_dir)
            .map_err(|e| e.to_string())
    }

    async fn execute_sysd_action(
        self,
        _: context::Context,
        id: SessionId,
        sysd_action: SysdAction,
    ) -> Result<Vec<SessionInfo>, String> {
        let unit = self
            .daemon
            .lock()
            .await
            .sysd_unit(id)
            .map_err(|e| e.to_string())?;
        let sysd = self.sysd()?;

        let result = match sysd_action {
            SysdAction::Start => sysd.start(&unit).await,
            SysdAction::Stop => sysd.stop(&unit).await,
            SysdAction::Restart => sysd.restart(&unit).await,
            SysdAction::Enable => sysd.enable(&unit).await,
            SysdAction::Disable => sysd.disable(&unit).await,
        };
        result.map_err(|e| {
            warn!(unit = %unit, ?sysd_action, "systemd action failed: {e:#}");
            format!("{e:#}")
        })?;

        let mut daemon = self.daemon.lock().await;

        // The event watcher picks this up as well, but set it now so the reply is accurate.
        match sysd_action {
            SysdAction::Enable => daemon.set_sysd_enabled(&unit, true),
            SysdAction::Disable => daemon.set_sysd_enabled(&unit, false),
            _ => {}
        }

        Ok(daemon.get_all_session_info())
    }

    async fn start_sysd_session_from_file(
        self,
        _: context::Context,
        file_path: std::path::PathBuf,
    ) -> Result<RunId, String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_start_sysd_session_from_file(file_path)
            .map_err(|e| e.to_string())
    }

    async fn wait_sysd_session(
        self,
        _: context::Context,
        name: String,
        run: RunId,
        timeout_ms: u64,
    ) -> Result<SysdWaitStatus, String> {
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));

        loop {
            let (status, mut changes) = self.daemon.lock().await.sysd_wait_status(&name, run);
            if status != SysdWaitStatus::Running {
                return Ok(status);
            }
            // The lock is released while waiting.
            match tokio::time::timeout_at(deadline, changes.changed()).await {
                Ok(Ok(())) => continue,
                _ => return Ok(SysdWaitStatus::Running),
            }
        }
    }

    async fn stop_sysd_session(self, _: context::Context, id: SessionId) -> Result<(), String> {
        let mut daemon = self.daemon.lock().await;
        daemon
            .handle_stop_sysd_session(id)
            .map_err(|e| e.to_string())
    }

    async fn update_sysd_session(
        self,
        _: context::Context,
        id: SessionId,
    ) -> Result<std::path::PathBuf, String> {
        self.daemon
            .lock()
            .await
            .handle_update_sysd_session(id)
            .map_err(|e| e.to_string())
    }

    async fn delete_sysd_session(
        self,
        _: context::Context,
        name: String,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        let (unit, has_unit_file) = self
            .daemon
            .lock()
            .await
            .sysd_delete_target(&name)
            .map_err(|e| e.to_string())?;

        // Best effort: take the unit out of systemd before its file disappears.
        // Disable needs the unit file to still exist.
        if let Some(sysd) = self.sysd.as_ref().filter(|_| has_unit_file) {
            if let Err(e) = sysd.disable(&unit).await {
                warn!("{e:#}");
            }
            if let Err(e) = sysd.stop(&unit).await {
                warn!("{e:#}");
            }
        }

        let removed = self
            .daemon
            .lock()
            .await
            .handle_delete_sysd_session(&unit)
            .map_err(|e| e.to_string())?;

        if has_unit_file {
            self.reload_sysd().await;
        }
        Ok(removed)
    }

}

pub async fn run_server(
    daemon: Arc<Mutex<Daemon>>,
    sysd: Option<SysdClient>,
) -> Result<()> {
    let server = DaemonRpcServer::new(daemon, sysd);

    let listener =
        tarpc::serde_transport::unix::listen(socket_path(), Bincode::default).await?;

    info!("Daemon listening on {}", socket_path().display());

    listener
        .filter_map(|result| async move {
            match result {
                Ok(transport) => {
                    // Every liveness probe connects too, so this is trace, not debug.
                    trace!("Client connected");
                    Some(transport)
                }
                Err(error) => {
                    warn!("Failed to accept connection: {error}");
                    None
                }
            }
        })
        .map(server::BaseChannel::with_defaults)
        .for_each(|channel| {
            // Every connection gets its own task: clients like `__internal-sysd-start`
            // stay connected for as long as their session runs, so a fixed number of
            // concurrently served connections would lock everyone else out.
            let requests = channel.execute(server.clone().serve()).for_each(|request| async move {
                tokio::spawn(request);
            });
            tokio::spawn(requests);
            async {}
        })
        .await;

    Ok(())
}
