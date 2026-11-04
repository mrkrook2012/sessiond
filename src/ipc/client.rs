use color_eyre::eyre::Result;

use tarpc::{
    client,
    context,
    tokio_serde::formats::Bincode,
};

use crate::ipc::{
    consts::socket_path,
    protocol::{ClientMessage, ServerMessage},
    service::DaemonServiceClient,
};

pub async fn connect() -> Result<DaemonServiceClient> {
    let transport = tarpc::serde_transport::unix::connect(
        socket_path(),
        Bincode::default,
    )
    .await?;

    let client = DaemonServiceClient::new(
        client::Config::default(),
        transport,
    )
    .spawn();

    Ok(client)
}

pub async fn send(
    client: &DaemonServiceClient,
    message: ClientMessage,
) -> Result<ServerMessage> {
    let response = client
        .process(context::current(), message)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("RPC error: {e}"))?
        .map_err(|e| color_eyre::eyre::eyre!("Daemon error: {e}"))?;

    Ok(response)
}

pub async fn get_terminal_data(
    client: &DaemonServiceClient,
) -> Result<ServerMessage> {
    let response = client
        .get_wanted_data(context::current())
        .await?;

    Ok(response)
}