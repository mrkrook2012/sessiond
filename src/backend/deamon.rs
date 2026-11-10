use std::sync::Arc;

use color_eyre::eyre::Result;
use tokio::sync::Mutex;

use crate::backend::deamon_manager::DaemonManager;
use crate::backend::mini_user_manager::MiniUserManager;
use crate::backend::session_manager::SessionManager;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::terminal_manager::TerminalManager;
use crate::backend::window_manager::WindowManager;
use crate::backend::yaml::yaml_manager::YamlManager;
use crate::ipc::server::run_server;

pub async fn run_daemon() -> Result<()> {
    let user_manager = MiniUserManager::new();
    let window_manager = WindowManager::new();
    let terminal_manager = TerminalManager::new();
    let session_manager = SessionManager::new();
    let yaml_manager = YamlManager::new()?;

    let systemd_manager = SystemdServiceManager::new();
    

    let daemon_manager = DaemonManager::new(
        user_manager,
        terminal_manager,
        window_manager,
        session_manager,
        systemd_manager,
        yaml_manager
    );

    println!("Starting daemon");

    run_server(daemon_manager).await?;

    Ok(())
}
