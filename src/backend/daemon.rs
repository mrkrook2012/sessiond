use tracing::{error, info, warn};
use std::sync::Arc;

use color_eyre::eyre::Result;
use tokio::sync::Mutex;

use crate::backend::daemon_manager::DaemonManager;
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::SessionManager;
use crate::backend::systemd_client::{SystemdClient, watch_events};
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalManager;
use crate::backend::window_manager::WindowManager;
use crate::backend::yaml::yaml_manager::YamlManager;
use crate::ipc::server::run_server;

pub async fn run_daemon() -> Result<()> {
    let user_manager = MiniUserManager::new();
    let window_manager = WindowManager::new();
    let (terminal_events, mut terminal_event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let terminal_manager = TerminalManager::new(terminal_events);
    let session_manager = SessionManager::new();
    let yaml_manager = YamlManager::new()?;

    let systemd_manager = SystemdServiceManager::new();
    

    let mut daemon_manager = DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
        systemd_manager,
        yaml_manager
    );

    info!(pid = std::process::id(), "Starting daemon");

    // Pick up sysd sessions that exist on disk but were never started by this daemon.
    // A failure here should not keep the daemon from starting.
    match daemon_manager.register_stored_sysd_sessions() {
        Ok(count) => info!("Registered {count} stored sysd session(s)"),
        Err(e) => error!("Failed to scan for stored sysd sessions: {e:#}"),
    }

    let daemon_manager = Arc::new(Mutex::new(daemon_manager));

    // Events from terminals (tracked commands exiting) are applied here, outside the PTY threads.
    let event_manager = daemon_manager.clone();
    tokio::spawn(async move {
        while let Some(event) = terminal_event_receiver.recv().await {
            event_manager.lock().await.handle_terminal_event(event);
        }
    });

    // Without a session bus (CI, containers) the daemon still works; sysd actions just fail.
    let systemd = match SystemdClient::connect().await {
        Ok(client) => {
            let watcher_client = client.clone();
            let watcher_manager = daemon_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = watch_events(watcher_client, watcher_manager).await {
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

    run_server(daemon_manager, systemd).await?;

    Ok(())
}
