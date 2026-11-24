use std::path::PathBuf;
 
use crate::backend::session_manager::UnitName;
use crate::backend::systemd_service_manager::SystemdServiceManager;
use crate::backend::yaml::models::YamlSession;
use crate::backend::yaml::yaml_manager::YamlManager;
use color_eyre::eyre::Result;
 
/// Everything that touches disk or systemd: YAML session files and unit files.
/// It knows nothing about live terminals or windows.
pub struct SessionStore {
    yaml_manager: YamlManager,
    systemd_manager: SystemdServiceManager,
}
 
impl SessionStore {
    pub fn new(yaml_manager: YamlManager, systemd_manager: SystemdServiceManager) -> Self {
        Self {
            yaml_manager,
            systemd_manager,
        }
    }
 
    pub fn load(&mut self, file_path: PathBuf) -> Result<YamlSession> {
        self.yaml_manager.load_session(file_path)
    }
 
    /// Saves to the default session location and returns the path.
    pub fn save(&mut self, session: &YamlSession) -> Result<PathBuf> {
        self.yaml_manager.save_session(session)
    }
 
    /// Saves as `<dir>/<name>.yaml` and returns the full path.
    pub fn export_to_dir(&self, session: &YamlSession, mut dir: PathBuf) -> Result<PathBuf> {
        dir.push(&session.name);
        dir.set_extension("yaml");
 
        self.yaml_manager.save_session_to(session, &dir)?;
        Ok(dir)
    }
 
    pub fn create_service(&mut self, yaml_path: &PathBuf, unit: UnitName) -> Result<()> {
        self.systemd_manager.create_service_from_yaml(yaml_path, unit)
    }
 
    /// Removes both the unit file and the YAML file.
    pub fn delete(&mut self, name: &str) -> Result<()> {
        self.systemd_manager.remove_sysd_service_file(name)?;
        self.yaml_manager.delete_session(name)?;
        Ok(())
    }
}
 