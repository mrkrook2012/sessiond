use std::path::PathBuf;

use color_eyre::eyre::{Result, eyre};
use tarpc::{client, context, tokio_serde::formats::Bincode};

use crate::backend::{
    session_manager::{RunId, SessionId},
    terminal_manager::TerminalId,
    window_manager::WindowId,
};
use crate::ipc::{
    consts::socket_path,
    protocol::{SessionInfo, SessionLookup, SysdAction, SysdWaitStatus, TerminalUpdate},
    service::DaemonServiceClient,
};

pub struct DaemonClient {
    client: DaemonServiceClient,
}

impl DaemonClient {
    pub async fn connect() -> Result<Self> {
        let transport =
            tarpc::serde_transport::unix::connect(socket_path(), Bincode::default).await?;

        let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

        Ok(Self { client })
    }

    // Internal helpers

    async fn rpc_call<T>(
        &self,
        future: impl std::future::Future<Output = std::result::Result<T, tarpc::client::RpcError>>,
    ) -> Result<T> {
        future.await.map_err(|e| eyre!("RPC error: {e}"))
    }

    async fn rpc_call_result<T>(
        &self,
        future: impl std::future::Future<
            Output = std::result::Result<std::result::Result<T, String>, tarpc::client::RpcError>,
        >,
    ) -> Result<T> {
        future
            .await
            .map_err(|e| eyre!("RPC error: {e}"))?
            .map_err(|e| eyre!("Daemon error: {e}"))
    }

    // Terminal operations

    /// See `DaemonService::get_wanted_data`.
    pub async fn get_wanted_data(
        &self,
        terminal_id: TerminalId,
        scrollback: u32,
        since: u64,
    ) -> Result<TerminalUpdate> {
        self.rpc_call(self.client.get_wanted_data(
            context::current(),
            terminal_id,
            scrollback,
            since,
        ))
        .await
    }

    pub async fn start(&self, rows: u16, cols: u16) -> Result<()> {
        self.rpc_call(self.client.start(context::current(), rows, cols))
            .await
    }

    pub async fn update_screen_size(&self, rows: u16, cols: u16) -> Result<()> {
        self.rpc_call(
            self.client
                .update_screen_size(context::current(), rows, cols),
        )
        .await
    }

    pub async fn write_terminal(&self, terminal_id: TerminalId, data: Vec<u8>) -> Result<()> {
        self.rpc_call(
            self.client
                .write_terminal(context::current(), terminal_id, data),
        )
        .await
    }

    pub async fn change_window(&self, window_id: WindowId) -> Result<()> {
        self.rpc_call(self.client.change_window(context::current(), window_id))
            .await
    }

    // Session operations

    pub async fn create_session(
        &self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>> {
        self.rpc_call(
            self.client
                .create_session(context::current(), working_dir, command, name),
        )
        .await
    }
    pub async fn delete_sysd_session(&self, name: String) -> Result<Vec<PathBuf>> {
        self.rpc_call_result(self.client.delete_sysd_session(context::current(), name))
            .await
    }

    pub async fn create_sysd_session(
        &self,
        working_dir: String,
        command: Option<String>,
        name: Option<String>,
    ) -> Result<Vec<SessionInfo>, tarpc::client::RpcError> {
        self.client
            .create_sysd_session(context::current(), working_dir, command, name)
            .await
    }

    pub async fn exit_session(&self) -> Result<Vec<SessionInfo>> {
        self.rpc_call(self.client.exit_session(context::current()))
            .await
    }

    pub async fn kill_session(&self, id: SessionId) -> Result<Vec<SessionInfo>> {
        self.rpc_call(self.client.kill_session(context::current(), id))
            .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.rpc_call(self.client.list_sessions(context::current()))
            .await
    }

    pub async fn session_id_by_name(&self, name: &str) -> Result<SessionLookup> {
        self.rpc_call(self.client.session_id_by_name(context::current(), name.to_owned()))
            .await
    }

    pub async fn open_session(&self, id: SessionId) -> Result<SessionInfo> {
        self.rpc_call_result(self.client.open_session(context::current(), id))
            .await
    }

    // Window operations

    pub async fn create_window(&self, id: SessionId) -> Result<SessionInfo> {
        self.rpc_call(self.client.create_window(context::current(), id))
            .await
    }

    pub async fn kill_window(&self, id: SessionId, window_id: WindowId) -> Result<SessionInfo> {
        self.rpc_call_result(self.client.kill_window(context::current(), id, window_id))
            .await
    }

    // File operations
    pub async fn export_session_to_file(
        &self,
        id: SessionId,
        working_dir: PathBuf,
    ) -> Result<PathBuf> {
        self.rpc_call_result(self.client.export_session_to_file(
            context::current(),
            id,
            working_dir,
        ))
        .await
    }

    pub async fn start_session_from_file(&self, file_path: PathBuf) -> Result<()> {
        self.rpc_call_result(
            self.client
                .start_session_from_file(context::current(), file_path),
        )
        .await
    }

    // Systemd operations

    pub async fn export_session_to_sysd(&self, id: SessionId) -> Result<()> {
        self.rpc_call_result(self.client.export_session_to_sysd(context::current(), id))
            .await
    }

    pub async fn execute_sysd_action(
        &self,
        id: SessionId,
        sysd_action: SysdAction,
    ) -> Result<Vec<SessionInfo>> {
        self.rpc_call_result(
            self.client
                .execute_sysd_action(context::current(), id, sysd_action),
        )
        .await
    }

    pub async fn start_sysd_session_from_file(&self, file_path: PathBuf) -> Result<RunId> {
        self.rpc_call_result(
            self.client
                .start_sysd_session_from_file(context::current(), file_path),
        )
        .await
    }

    /// Long-polls for up to `timeout`. Returns `Running` if nothing happened in that time.
    pub async fn wait_sysd_session(
        &self,
        name: String,
        run: RunId,
        timeout: std::time::Duration,
    ) -> Result<SysdWaitStatus> {
        // The default deadline (10s) is shorter than the wait; leave room for the reply.
        let mut ctx = context::current();
        ctx.deadline = std::time::Instant::now() + timeout + std::time::Duration::from_secs(5);

        self.rpc_call_result(self.client.wait_sysd_session(
            ctx,
            name,
            run,
            timeout.as_millis() as u64,
        ))
        .await
    }

    pub async fn update_sysd_session(&self, id: SessionId) -> Result<PathBuf> {
        self.rpc_call_result(self.client.update_sysd_session(context::current(), id))
            .await
    }

    pub async fn stop_sysd_session(&self, id: SessionId) -> Result<()> {
        self.rpc_call_result(self.client.stop_sysd_session(context::current(), id))
            .await
    }

    // Daemon operations

    pub async fn kill_daemon(&self) -> Result<()> {
        self.rpc_call(self.client.kill_daemon(context::current()))
            .await
    }
}
