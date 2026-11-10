use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::Result;

use crate::backend::yaml::models::YamlSession;

pub struct YamlManager {
    storage_dir: PathBuf,
}

impl YamlManager {
    pub fn new() -> Result<Self> {
        let home: String = std::env::var("HOME")?;

        let storage_dir = PathBuf::from(home).join(".local/share/daemon_test");

        fs::create_dir_all(&storage_dir)?;

        Ok(Self { storage_dir })
    }

    pub fn get_storage_dir(&self) -> &PathBuf {
        &self.storage_dir
    }

    pub fn get_session_path(&self, name: &str) -> PathBuf {
        self.storage_dir.join(format!("{name}.yaml"))
    }

    pub fn load_session(&self, path: PathBuf) -> Result<YamlSession> {
        let input = fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&input)?)
    }

    pub fn save_session(&self, session: &YamlSession) -> Result<PathBuf> {
        let path = self.get_session_path(&session.name);
        self.save_session_to(session, &path)?;
        Ok(path)
    }

    pub fn save_session_to(&self, session: &YamlSession, path: &PathBuf) -> Result<()> {
        let yaml = self.serialize_session(session)?;
        fs::write(path, yaml)?;
        Ok(())
    }

    pub fn serialize_session(&self, session: &YamlSession) -> Result<String> {
        Ok(serde_yaml::to_string(session)?)
    }

    pub fn deserialize_session(&self, yaml: &str) -> Result<YamlSession> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    pub fn delete_session(&self, name: &str) -> Result<()> {
        let path = self.get_session_path(name);
        println!("Deleting {:?}", path);
        fs::remove_file(path)?;
        Ok(())
    }
}
