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
    // Check if socket exists, and if so whether it is stale
    if Path::new(&socket_path()).exists() {
        let listener = ListenerOptions::new()
            .name(socket_path().to_fs_name::<GenericFilePath>().unwrap())
            .create_sync();
        match listener {
            Ok(_listener) => {
                println!("Daemon not yet running, starting daemon");
            }
            Err(e) => {
                eprintln!("Daemon already running {}", e);
                return Ok(());
            }
        }
    }

    let user_manager = MiniUserManager::new();
    let window_manager = WindowManager::new();
    let terminal_manager = TerminalManager::new();
    let session_manager = SessionManager::new();
    let systemd_manager = Arc::new(TokioMutex::new(SystemdServiceManager::new().await?));

    // The DaemonManager is moved into the manager thread and owned by it alone.
    let daemon_manager = DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
        systemd_manager,
    );

    let listener = ListenerOptions::new()
        .name(socket_name())
        .create_sync()
        .unwrap();

    let (event_tx, event_rx) = mpsc::channel::<Event>();

    // The single owner of all state.
    tokio::task::spawn_blocking(move || run_manager(daemon_manager, event_rx));
    println!("Daemon noises");

    // The accept loop blocks, so keep it off the async executor.
    tokio::task::spawn_blocking(move || {
        let mut next_id: ClientId = 0;

        loop {
            let conn = match listener.accept() {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    continue;
                }
            };

            let (mut sender, mut receiver) = match IpcConnection::new(conn).split() {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("Failed to split connection: {}", e);
                    continue;
                }
            };

            let id = next_id;
            next_id += 1;

            let (outbox_tx, outbox_rx) = mpsc::sync_channel::<ServerMessage>(CLIENT_QUEUE);

            // Register the client before any of its messages can arrive.
            if event_tx
                .send(Event::Connected {
                    id,
                    outbox: outbox_tx,
                })
                .is_err()
            {
                eprintln!("Manager thread is gone, shutting down");
                break;
            }
            println!("Client {id} connected");

            // Writer: drains this client's outbox onto the socket.
            // Exits when the socket breaks or the manager drops the outbox.
            thread::spawn(move || {
                for msg in outbox_rx {
                    if sender.send(&msg).is_err() {
                        break;
                    }
                }
            });

            // Reader: forwards this client's messages to the manager.
            let reader_events = event_tx.clone();
            thread::spawn(move || {
                loop {
                    let msg: ClientMessage = match receiver.recv() {
                        Ok(msg) => msg,
                        Err(e) => {
                            eprintln!("Client {id} disconnected: {e}");
                            break;
                        }
                    };
                    if reader_events.send(Event::Message { id, msg }).is_err() {
                        return;
                    }
                }
                let _ = reader_events.send(Event::Disconnected { id });
            });
        }
    })
    .await?;

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
                clients.retain(|_, outbox| {
                    !matches!(
                        outbox.try_send(data.clone()),
                        Err(TrySendError::Disconnected(_))
                    )
                });
            }
        }
    }
}
