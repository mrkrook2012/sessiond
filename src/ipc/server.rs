use std::sync::Arc;

use color_eyre::eyre::Result;
use futures::StreamExt;
use tokio::sync::Mutex;

use tarpc::{
    context,
    server::{self, Channel},
    tokio_serde::formats::Bincode,
};

use crate::backend::deamon_manager::DaemonManager;
use crate::ipc::service::{DaemonService};
use crate::ipc::consts::socket_path;

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
    async fn process(
        self,
        _: context::Context,
        message: crate::ipc::protocol::ClientMessage,
    ) -> Result<crate::ipc::protocol::ServerMessage, String> {
        let mut manager = self.manager.lock().await;

        manager
            .process_message(message)
            .map_err(|e| e.to_string())
    }

    async fn get_wanted_data(
        self,
        _: context::Context,
    ) -> crate::ipc::protocol::ServerMessage {
        let mut manager = self.manager.lock().await;
        manager.get_wanted_data()
    }
}

pub async fn run_server(manager: DaemonManager) -> Result<()> {
    let server = DaemonRpcServer::new(manager);

    let mut listener =
        tarpc::serde_transport::unix::listen(
            socket_path(),
            Bincode::default,
        )
        .await?;

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
    .buffer_unordered(100)
    .for_each(|_| async {})
    .await;

    Ok(())
}