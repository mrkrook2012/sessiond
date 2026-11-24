use crate::backend::{session_manager::UnitName, systemd_manager::SystemdManager};
use color_eyre::eyre::Result;
use std::path::Path;

/// File-side systemd work: writing and removing unit files.
/// Everything that talks to systemd itself lives in `SystemdClient`.
pub struct SystemdServiceManager {}

impl SystemdServiceManager {
    pub fn new() -> Self {
        Self {}
    }


    pub fn create_service_from_yaml(
        &mut self,
        yaml_path: &Path,
        name: &UnitName,
    ) -> Result<()> {
        // TODO checking name
        let sysd_dir_builder = SystemdManager::unit_dir()?;
        std::fs::create_dir_all(&sysd_dir_builder)?;

        let user = std::env::var("USER")?;

        SystemdManager::write_default_template_to_path(
            yaml_path,
            &user,
            &sysd_dir_builder,
            name
        )?;

        Ok(())
    }

    /// True if a unit file for this session exists in the user unit directory.
    pub fn unit_file_exists(&self, name: &UnitName) -> bool {
        SystemdManager::unit_file_path(name.as_str())
            .map(|path| path.is_file())
            .unwrap_or(false)
    }
}

