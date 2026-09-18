use color_eyre::eyre::Result;
use tokio::sync::RwLock;
use zbus_systemd::{systemd1::ManagerProxy, zbus::Connection};
use std::{collections::HashMap, path::Path};

use crate::backend::session_manager::Session;

pub struct SystemdServiceManager {
    sesssions: HashMap<String, SystemdSession>,
    connection: Connection,
}

impl SystemdServiceManager {
    pub async fn create_service_from_yaml(&mut self, path: &Path) -> Result<()> {
        // TODO
        // Create the systemd file
        // Reload systemd daemon
        // Should start watcher that updates info
        let manager = ManagerProxy::new(&self.connection).await?;
        let session = SystemdSession::new( "Henk".into());
        Ok(())
    }

    pub async fn start_session(&self) {}

    pub async fn stop_session(&self) {}

    pub async fn enable_session(&self) {}

    pub async fn disable_session(&self) {}

    async fn get_manager(
        &self,
        connection: &Connection,
    ) -> Result<ManagerProxy<'_>> {
        Ok(ManagerProxy::new(connection).await?)
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
