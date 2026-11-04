use crate::ipc::protocol::{ClientMessage, ServerMessage};

#[tarpc::service]
pub trait DaemonService {
    /// Execute a daemon operation.
    async fn process(message: ClientMessage) -> Result<ServerMessage, String>;

    /// Retrieve the currently selected terminal data.
    async fn get_wanted_data() -> ServerMessage;
}