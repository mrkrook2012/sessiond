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
    deamon_manager::DaemonManager, session_manager::SessionId, terminal_manager::TerminalId,
    window_manager::WindowId,
};
use crate::ipc::{
    consts::socket_path,
    protocol::{SessionInfo, SysdAction, TerminalUpdate},
    service::DaemonService,
};

#[derive(Clone)]
pub struct DaemonRpcServer {
    manager: Arc<Mutex<DaemonManager>>,
}

impl DaemonRpcServer {
    pub fn new(manager: DaemonManager) -> Self {
        Self {
            manager: Arc::new(Mutex::new(manager)),
        }
    }
}

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
        let mut manager = self.manager.lock().await;
        manager.handle_export_session(id).map_err(|e| e.to_string())
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
        let manager = self.manager.lock().await;
        manager.handle_kill_deamon()
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
        let mut manager = self.manager.lock().await;
        manager
            .handle_execute_sysd_action(id, sysd_action)
            .map_err(|e| e.to_string())
    }

    async fn start_sysd_session_from_file(
        self,
        _: context::Context,
        file_path: std::path::PathBuf,
    ) -> Result<(), String> {
        let mut manager = self.manager.lock().await;
        manager
            .handle_start_sysd_session_from_file(file_path)
            .map_err(|e| e.to_string())
    }

    async fn stop_sysd_session(self, _: context::Context, id: SessionId) -> Result<(), String> {
        let mut manager = self.manager.lock().await;
        manager
            .handle_stop_sysd_session(id)
            .map_err(|e| e.to_string())
    }
}

pub async fn run_server(manager: DaemonManager) -> Result<()> {
    let server = DaemonRpcServer::new(manager);

    let mut listener =
        tarpc::serde_transport::unix::listen(socket_path(), Bincode::default).await?;

    println!("Daemon listening on {}", socket_path().display());

    listener
        .filter_map(|result| async move {
            match result {
                Ok(transport) => Some(transport),
                Err(error) => {
                    eprintln!("Accept error: {error}");
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
