use crate::backend::{
    session_manager::{Session, SysdSession, UnitName},
    systemd_manager::SystemdManager,
};
use color_eyre::eyre::Result;
use std::{
    collections::HashMap, path::{Path, PathBuf}, str::FromStr,
};
use zbus_systemd::{
    systemd1::ManagerProxy,
    zbus::{Connection, fdo::PropertiesChanged},
};

pub struct SystemdServiceManager {
    sessions: HashMap<UnitName, Session>, // Only the sysd variant in kind
    connection: Connection,
}

impl SystemdServiceManager {
    pub async fn new() -> Result<Self> {
        println!("Trying to connect to systemd");
        let connection = Connection::session().await?;
        println!("Connected to systemd");

        let sessions: HashMap<UnitName, Session> = HashMap::new();

        Ok(Self {
            connection,
            sessions,
        })
    }

    pub async fn create_service_from_yaml(
        &mut self,
        yaml_path: &Path,
        name: UnitName,
    ) -> Result<()> {
        // TODO checking name
        // Create the systemd file
        // Reload systemd daemon
        // Should start watcher that updates info
        let bin_path = std::env::current_exe()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let name_string: String = name.to_string();
        let yaml_path = yaml_path.to_str().unwrap();
        let mut sysd_dir_builder = PathBuf::from(std::env::var("HOME")?);
        sysd_dir_builder.push(".config/systemd/user/");
        
        let user = std::env::var("USER")?;

        SystemdManager::write_default_template_to_path(
            yaml_path.into(),
            user,
            &sysd_dir_builder,
            name
        ).unwrap();

        let manager = ManagerProxy::new(&self.connection).await?;
        manager.reload().await?;

        // TODO start watchdog to pipe updates
        //self.run_watchdog(name);
        Ok(())
    }

    pub async fn start_session(name: UnitName) {}

    pub async fn stop_session(name: UnitName) {}

    pub async fn enable_session(name: UnitName) {}

    pub async fn disable_session(name: UnitName) {}

    pub async fn reload_systemd(&self) {
        let manager = self.get_manager().await.unwrap();
        manager.reload().await.unwrap();
    }
    async fn get_manager(&self) -> Result<ManagerProxy<'_>> {
        Ok(ManagerProxy::new(&self.connection).await?)
    }

    async fn run_watchdog(&mut self, name: UnitName) -> Result<()> {
        // TODO
        let ref1 = self.get_session_by_name_mut_unsafe(name.clone());
        let ref2 = self.get_session_by_name_mut_unsafe(name.clone());
        let manager = self.get_manager().await.unwrap();
        manager.subscribe().await.unwrap();
        let unit = manager.get_unit(name.clone().to_string()).await.unwrap();

        Ok(())
    }

    fn get_session_by_name_mut_unsafe(&mut self, name: UnitName) -> &mut Session {
        self.sessions.get_mut(&name).unwrap()
    }
}
pub enum ServiceState {
    Running,
    Starting,
    Stopping,
    Stopped { exit_code: u8 },
    Idle, // Not running due to not being started and not enabled
}
