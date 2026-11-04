use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as TokioMutex;

use color_eyre::eyre::Result;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use interprocess::local_socket::{ListenerOptions, traits::Listener};

use crate::backend::deamon_manager::DaemonManager;
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::SessionManager;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalManager;
use crate::backend::window_manager::WindowManager;
use crate::ipc::consts::{socket_name, socket_path};
use crate::ipc::ipc_connection::IpcConnection;
use crate::ipc::protocol::{ClientMessage, ServerMessage};
use crate::ipc::server::run_server;

// TODO Improve this later
type ClientId = u64;

/// How often connected clients get a state update.
const TICK: Duration = Duration::from_millis(16);

/// Per-client outbound queue size. If a client falls this far behind,
/// tick updates are dropped (they are snapshots, so that's fine) and
/// a client that can't even take a direct response is disconnected.
const CLIENT_QUEUE: usize = 64;

/// Everything the manager thread needs to react to.
/// All state changes flow through this one channel.
enum Event {
    Connected {
        id: ClientId,
        outbox: SyncSender<ServerMessage>,
    },
    Message {
        id: ClientId,
        msg: ClientMessage,
    },
    Disconnected {
        id: ClientId,
    },
}

pub async fn run_daemon() -> Result<()> {
    let user_manager = MiniUserManager::new();
    let window_manager = WindowManager::new();
    let terminal_manager = TerminalManager::new();
    let session_manager = SessionManager::new();

    let systemd_manager =
        Arc::new(TokioMutex::new(
            SystemdServiceManager::new().await?
        ));

    let daemon_manager = DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
        systemd_manager,
    );

    println!("Starting daemon");

    run_server(daemon_manager).await?;

    Ok(())
}
/// Actor loop: owns the `DaemonManager` and the client table, so no locking is needed.
fn run_manager(mut daemon_manager: DaemonManager, events: Receiver<Event>) {
    let mut clients: HashMap<ClientId, SyncSender<ServerMessage>> = HashMap::new();
    let mut next_tick = Instant::now() + TICK;

    loop {
        let timeout = next_tick.saturating_duration_since(Instant::now());

        match events.recv_timeout(timeout) {
            Ok(Event::Connected { id, outbox }) => {
                clients.insert(id, outbox);
            }
            Ok(Event::Disconnected { id }) => {
                clients.remove(&id);
                println!("Client {id} removed, {} remaining", clients.len());
            }
            Ok(Event::Message { id, msg }) => {
                println!("Received from {id}: {:?}", msg);

                let response = match daemon_manager.process_message(msg) {
                    Ok(res) => res,
                    Err(e) => ServerMessage::Error {
                        error: e.to_string(),
                    },
                };
                println!("Responding to {id}: {:?}", response);

                // Responses go only to the client that asked.
                let dead = clients
                    .get(&id)
                    .is_some_and(|outbox| outbox.try_send(response).is_err());
                if dead {
                    clients.remove(&id);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        // Periodic broadcast to everyone.
        if Instant::now() >= next_tick {
            next_tick = Instant::now() + TICK;

            if !clients.is_empty() {
                let data = daemon_manager.get_wanted_data();
                match &data {
                    ServerMessage::TerminalData { terminals } => {
                        clients.retain(|_, outbox| {
                            !matches!(
                                outbox.try_send(data.clone()),
                                Err(TrySendError::Disconnected(_))
                            )
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}
