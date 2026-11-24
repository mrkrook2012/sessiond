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
    deamon_manager::DaemonManager,
    session_manager::{RunId, SessionId},
    systemd_client::SystemdClient,
    terminal_manager::TerminalId, window_manager::WindowId,
};
use crate::ipc::{
    consts::socket_path,
    protocol::{SessionInfo, SysdAction, SysdWaitStatus, TerminalUpdate},
    service::DaemonService,
};

/// Upper bound for one `wait_sysd_session` call; clients loop.
const MAX_WAIT_MS: u64 = 60_000;

#[derive(Clone)]
pub struct DaemonRpcServer {
    manager: Arc<Mutex<DaemonManager>>,
    /// `None` when the session bus / user systemd is not reachable (e.g. in CI).
    systemd: Option<SystemdClient>,
}

impl DaemonRpcServer {
    pub fn new(manager: Arc<Mutex<DaemonManager>>, systemd: Option<SystemdClient>) -> Self {
        Self { manager, systemd }
    }

    fn systemd(&self) -> Result<&SystemdClient, String> {
        self.systemd
            .as_ref()
            .ok_or_else(|| "systemd is not available".to_string())
    }

    /// `daemon-reload` after unit files changed. Not fatal: the file is already written.
    async fn reload_systemd(&self) {
        match &self.systemd {
            Some(systemd) => {
                if let Err(e) = systemd.reload().await {
                    error!("{e:#}");
                }
            }
            None => warn!("systemd not available, skipping daemon-reload"),
        }
    }
}

// Note on locking: systemd calls are made *without* holding the manager lock.
// Starting/stopping a unit runs its ExecStart/ExecStop, which call back into this daemon.

impl DaemonService for DaemonRpcServer {
    async fn get_wanted_data(self, _: context::Context, terminal_id: TerminalId) -> TerminalUpdate {
        let mut manager = self.manager.lock().await;
        manager.get_wanted_data(terminal_id)
    }

    async fn start(self, _: context::Context, rows: u16, cols: u16) {
        let mut manager = self.manager.lock().await;
        manager.handle_start(rows, cols);
    }

    async fn update_screen_size(self, _: context::Context, rows: u16, cols: u16) {
        let mut manager = self.manager.lock().await;
        manager.handle_update_screen_size(rows, cols);
    }

    async fn create_session(
        self,
        _: context::Context,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        let mut manager = self.manager.lock().await;
        manager.handle_create_session(working_dir, command, name)
    }

    async fn create_sysd_session(
        self,
        _: context::Context,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Vec<SessionInfo> {
        let sessions = self
            .manager
            .lock()
            .await
            .handle_create_sysd_session(working_dir, command, name);
        self.reload_systemd().await;
        sessions
    }

    async fn exit_session(self, _: context::Context) -> Vec<SessionInfo> {
        let mut manager = self.manager.lock().await;
        manager.handle_exit_session()
    }

    async fn kill_session(self, _: context::Context, id: SessionId) -> Vec<SessionInfo> {
        let mut manager = self.manager.lock().await;
        manager.handle_kill_session(id)
    }

    async fn write_terminal(self, _: context::Context, terminal_id: TerminalId, data: Vec<u8>) {
        let mut manager = self.manager.lock().await;
        manager.handle_write(terminal_id, data);
    }

    async fn list_sessions(self, _: context::Context) -> Vec<SessionInfo> {
        let manager = self.manager.lock().await;
        manager.handle_list_sessions()
    }

    async fn create_window(self, _: context::Context, id: SessionId) -> SessionInfo {
        let mut manager = self.manager.lock().await;
        manager.handle_create_window(id)
    }

    async fn open_session(self, _: context::Context, id: SessionId) -> Result<SessionInfo, String> {
        let mut manager = self.manager.lock().await;
        manager.handle_open_session(id).map_err(|e| e.to_string())
    }

    async fn change_window(self, _: context::Context, window_id: WindowId) {
        let mut manager = self.manager.lock().await;
        manager.handle_change_window(window_id);
    }

    async fn kill_window(
        self,
        _: context::Context,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<SessionInfo, String> {
        let mut manager = self.manager.lock().await;
        manager
            .handle_kill_window(id, window_id)
            .map_err(|e| e.to_string())
    }

    async fn export_session_to_sysd(
        self,
        _: context::Context,
        id: SessionId,
    ) -> Result<(), String> {
        self.manager
            .lock()
            .await
            .handle_export_session(id)
            .map_err(|e| e.to_string())?;
        self.reload_systemd().await;
        Ok(())
    }

    async fn start_session_from_file(
        self,
        _: context::Context,
        file_path: std::path::PathBuf,
    ) -> Result<(), String> {
        let mut manager = self.manager.lock().await;
        manager
            .handle_start_session_from_file(file_path)
            .map_err(|e| e.to_string())
    }

    async fn kill_daemon(self, _: context::Context) {
        // Tell waiting `__internal-systemd-start` processes first and give their replies
        // a moment to go out, so they exit cleanly instead of seeing a dropped connection
        // (which they treat as a crash, making systemd restart them and the daemon).
        self.manager.lock().await.begin_shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        self.manager.lock().await.handle_kill_deamon()
    }

    async fn export_session_to_file(
        self,
        _: context::Context,
        id: SessionId,
        working_dir: std::path::PathBuf,
    ) -> Result<std::path::PathBuf, String> {
        let manager = self.manager.lock().await;
        manager
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
            .manager
            .lock()
            .await
            .sysd_unit(id)
            .map_err(|e| e.to_string())?;
        let systemd = self.systemd()?;

        let result = match sysd_action {
            SysdAction::Start => systemd.start(&unit).await,
            SysdAction::Stop => systemd.stop(&unit).await,
            SysdAction::Restart => systemd.restart(&unit).await,
            SysdAction::Enable => systemd.enable(&unit).await,
            SysdAction::Disable => systemd.disable(&unit).await,
        };
        result.map_err(|e| {
            warn!(unit = %unit, ?sysd_action, "systemd action failed: {e:#}");
            format!("{e:#}")
        })?;

        let mut manager = self.manager.lock().await;

        // The event watcher picks this up as well, but set it now so the reply is accurate.
        match sysd_action {
            SysdAction::Enable => manager.set_sysd_enabled(&unit, true),
            SysdAction::Disable => manager.set_sysd_enabled(&unit, false),
            _ => {}
        }

        Ok(manager.get_all_session_info())
    }

    async fn start_sysd_session_from_file(
        self,
        _: context::Context,
        file_path: std::path::PathBuf,
    ) -> Result<RunId, String> {
        let mut manager = self.manager.lock().await;
        manager
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
            let (status, mut changes) = self.manager.lock().await.sysd_wait_status(&name, run);
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
        let mut manager = self.manager.lock().await;
        manager
            .handle_stop_sysd_session(id)
            .map_err(|e| e.to_string())
    }

    async fn delete_sysd_session(
        self,
        _: context::Context,
        name: String,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        let (unit, has_unit_file) = self
            .manager
            .lock()
            .await
            .sysd_delete_target(&name)
            .map_err(|e| e.to_string())?;

        // Best effort: take the unit out of systemd before its file disappears.
        // Disable needs the unit file to still exist.
        if let Some(systemd) = self.systemd.as_ref().filter(|_| has_unit_file) {
            if let Err(e) = systemd.disable(&unit).await {
                warn!("{e:#}");
            }
            if let Err(e) = systemd.stop(&unit).await {
                warn!("{e:#}");
            }
        }

        let removed = self
            .manager
            .lock()
            .await
            .handle_delete_sysd_session(&unit)
            .map_err(|e| e.to_string())?;

        if has_unit_file {
            self.reload_systemd().await;
        }
        Ok(removed)
    }

}

pub async fn run_server(
    manager: Arc<Mutex<DaemonManager>>,
    systemd: Option<SystemdClient>,
) -> Result<()> {
    let server = DaemonRpcServer::new(manager, systemd);

    let mut listener =
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
        .map(|channel| {
            let rpc_server = server.clone();

            channel
                .execute(rpc_server.serve())
                .for_each(|request| async move {
                    tokio::spawn(request);
                })
        })
        .buffer_unordered(10)
        .for_each(|_| async {})
        .await;

    Ok(())
}
