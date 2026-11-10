use crate::backend::{
    session_manager::{Session, SysdSession, UnitName},
    systemd_manager::SystemdManager,
};
use color_eyre::eyre::Result;
use std::{
    collections::HashMap, path::{Path, PathBuf}, process::Command, str::FromStr,
};
use zbus_systemd::{
    systemd1::ManagerProxy,
    zbus::{Connection, fdo::PropertiesChanged},
};

pub struct SystemdServiceManager {
}

impl SystemdServiceManager {
    pub  fn new() -> Self {



        Self {
        }
    }
    pub fn remove_sysd_service_file(&mut self, name: &str) -> Result {
        SystemdManager::remove_sysd_service_file(name)
    }

    pub fn create_service_from_yaml(
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
        Command::new("systemctl")
        .arg("--user").arg("daemon-reload").output().unwrap();

        Ok(())
    }

}

