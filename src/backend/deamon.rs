use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use std::fs::File;
use std::sync::Mutex;

use color_eyre::eyre::Result;
use daemonize::Daemonize;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use interprocess::local_socket::{ListenerOptions, traits::Listener};

use crate::backend::deamon_manager::DaemonManager;
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::SessionManager;
use crate::backend::terminal_manager::TerminalManager;
use crate::backend::window_manager::WindowManager;
use crate::ipc::consts::{SOCKET_PATH, socket_name};
use crate::ipc::ipc_connection::IpcConnection;
use crate::ipc::protocol::ClientMessage;

pub async fn run_daemon() -> Result<()> {
    // Check if socket exists
    // if so check if not stale

    if Path::new(SOCKET_PATH).exists() {
        // There is a file
        let listener = ListenerOptions::new()
            .name(SOCKET_PATH.to_fs_name::<GenericFilePath>().unwrap())
            .create_sync();
        match listener {
            Ok(_listener) => {
                println!("Daemon not yet running, starting daemon");
            }
            Err(e) => {
                eprintln!("Deamon already running {}", e);
                return Ok(());
            }
        }
    }

    let stdout = File::create("./daemon.out").unwrap();
    let stderr = File::create("./daemon.err").unwrap();
    let daemonize = Daemonize::new().stdout(stdout).stderr(stderr);
    println!("Trying to daemonize");
    match daemonize.start() {
        Ok(_) => println!("Succes, daemonized"),
        Err(e) => eprintln!("Error, {}", e),
    }
    let user_manager = MiniUserManager::new();
    let window_manager = WindowManager::new();
    let terminal_manager = TerminalManager::new();
    let session_manager = SessionManager::new();

    let daemon_manager = Arc::new(Mutex::new(DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
    )));
    let update_daemon_manager = Arc::clone(&daemon_manager);

    let listener = ListenerOptions::new()
        .name(socket_name())
        .create_sync()
        .unwrap();

    let conn = listener.accept().unwrap();
    let (sender, mut reciever) = IpcConnection::new(conn).split().unwrap();
    let safe_sender = Arc::new(Mutex::new(sender));

    let update_daemon_manager = Arc::clone(&daemon_manager);
    let update_sender = Arc::clone(&safe_sender);

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(16));

            let data = update_daemon_manager.lock().unwrap().get_wanted_data();

            println!("Sending {:?}", data);

            update_sender.lock().unwrap().send(&data).unwrap();
        }
    });

    loop {
        let client_message: ClientMessage = reciever.recv().unwrap();

        let response = daemon_manager
            .lock()
            .unwrap()
            .process_message(client_message);

        safe_sender.lock().unwrap().send(&response).unwrap();
    }
}
