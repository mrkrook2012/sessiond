use crate::backend::systemd_manager::SystemdManager;
use color_eyre::eyre::Result;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::sync::RwLock;
use zbus_systemd::{systemd1::ManagerProxy, zbus::Connection};

pub struct SystemdServiceManager {
    sessions: HashMap<String, SystemdSession>,
    connection: Connection,
}

impl SystemdServiceManager {
    pub async fn new() -> Result<Self> {
        println!("Trying to connect to systemd");
        let connection = Connection::session().await?;
        println!("Connected to systemd");

        let sessions: HashMap<String, SystemdSession> = HashMap::new();

        Ok(Self {
            connection,
            sessions,
        })
    }

    pub async fn create_service_from_yaml(&mut self, yaml_path: &Path, session_name: String) -> Result<()> {
        // TODO checking name
        // Create the systemd file
        // Reload systemd daemon
        // Should start watcher that updates info
        let bin_path = std::env::current_exe()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let yaml_path = yaml_path.to_str().unwrap();
        let start_command = format!("{bin_path} run {yaml_path}");
        let working_dir = PathBuf::from(std::env::var("HOME")?);
        let working_dir_string = working_dir.to_str().unwrap();
        let systemd_file_path = format!("{working_dir_string}/.config/systemd/user/{session_name}.service");

        let user = std::env::var("USER")?;

        SystemdManager::write_default_template_to_path(
            systemd_file_path.into(),
            user,
            &working_dir,
            start_command,
        )
        .await?;

        let manager = ManagerProxy::new(&self.connection).await?;
        manager.reload().await?;

        // TODO start watchdog to pipe updates
        Ok(())
    }

    pub async fn start_session(&self, session_name: String) {}

    pub async fn stop_session(&self, session_name: String) {}

    pub async fn enable_session(&self, session_name: String) {}

    pub async fn disable_session(&self, session_name: String) {}

    pub async fn reload_systemd(&self) {
        let manager = self.get_manager().await.unwrap();
        manager.reload().await.unwrap();
    }
    async fn get_manager(&self) -> Result<ManagerProxy<'_>> {
        Ok(ManagerProxy::new(&self.connection).await?)
    }

    async fn run_watchdog(session: SystemdSession) -> Result<()> {
        Ok(())
    }
}
pub enum ServiceState {
    Running,
    Starting,
    Stopping,
    Stopped { exit_code: u8 },
    Idle, // Not running due to not being started and not enabled
}

pub struct SystemdSession {
    session_name: String,
    state: RwLock<ServiceState>,
    enabled: RwLock<bool>,
}

impl SystemdSession {
    pub fn new(session_name: String) -> Self {
        Self {
            session_name,
            state: ServiceState::Idle.into(),
            enabled: false.into(),
        }
    }
}
