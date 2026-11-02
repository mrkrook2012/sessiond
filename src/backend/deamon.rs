use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use std::fs::File;
use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;

use color_eyre::eyre::Result;
use daemonize::Daemonize;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use interprocess::local_socket::{ListenerOptions, traits::Listener};

use crate::backend::deamon_manager::DaemonManager;
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::SessionManager;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalManager;
use crate::backend::window_manager::WindowManager;
use crate::ipc::consts::{socket_path, socket_name};
use crate::ipc::ipc_connection::IpcConnection;
use crate::ipc::protocol::{ClientMessage, ServerMessage};

pub async fn run_daemon() -> Result<()> {
    // Check if socket exists
    // if so check if not stale

    if Path::new(&socket_path()).exists() {
        // There is a file
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

    let daemon_manager = Arc::new(Mutex::new(DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
        systemd_manager,
    )));
    println!("Daemon noises");
    let listener = ListenerOptions::new()
        .name(socket_name())
        .create_sync()
        .unwrap();

    // Outer loop: accept new clients forever, daemon_manager survives across all of them
    loop {
        let conn = match listener.accept() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Failed to accept connection: {}", e);
                continue;
            }
        };

        let (sender, mut receiver) = match IpcConnection::new(conn).split() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("Failed to split connection: {}", e);
                continue;
            }
        };

        let safe_sender = Arc::new(Mutex::new(sender));

        // Flag so the update thread knows to stop once this client is gone
        let running = Arc::new(AtomicBool::new(true));

        let update_daemon_manager = Arc::clone(&daemon_manager);
        let update_sender = Arc::clone(&safe_sender);
        let update_running = Arc::clone(&running);

        let update_handle = std::thread::spawn(move || {
            while update_running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(16));

                let data = update_daemon_manager.lock().unwrap().get_wanted_data();

                if update_sender.lock().unwrap().send(&data).is_err() {
                    // client is gone, stop trying
                    break;
                }
            }
        });

        // Inner loop: serve this one client until it disconnects
        loop {
            let client_message: ClientMessage = match receiver.recv() {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("Client disconnected: {}", e);
                    break;
                }
            };
            println!("Recieved Message: {:?}", client_message);

            let response = match daemon_manager
                .lock()
                .unwrap()
                .process_message(client_message)
            {
                Ok(res) => res,
                Err(e) => ServerMessage::Error {
                    error: e.to_string(),
                },
            };
            println!("Responded: {:?}", response);
            if safe_sender.lock().unwrap().send(&response).is_err() {
                eprintln!("Failed to send response, client likely disconnected");
                break;
            }
        }

        running.store(false, Ordering::Relaxed);
        let _ = update_handle.join();

        println!("Client disconnected, waiting for next connection");
    }
}
