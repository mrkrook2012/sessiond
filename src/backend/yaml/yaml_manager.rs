use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::Result;

use crate::backend::yaml::models::YamlSession;

/// Overrides the directory session YAML files are stored in (used by tests).
pub const DATA_DIR_ENV_VAR: &str = "DAEMON_TEST_DATA_DIR";

pub struct YamlManager {
    storage_dir: PathBuf,
}

impl YamlManager {
    pub fn new() -> Result<Self> {
        let storage_dir = match std::env::var_os(DATA_DIR_ENV_VAR) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var("HOME")?).join(".local/share/daemon_test"),
        };

        fs::create_dir_all(&storage_dir)?;

        Ok(Self { storage_dir })
    }

    pub fn get_storage_dir(&self) -> &PathBuf {
        &self.storage_dir
    }

    pub fn get_session_path(&self, name: &str) -> PathBuf {
        self.storage_dir.join(format!("{name}.yaml"))
    }

    /// All `*.yaml` files in the storage directory, sorted for a stable order.
    pub fn list_session_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.storage_dir)? {
            let path = entry?.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "yaml") {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
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

    /// Every stored YAML file belonging to `unit_name`. The file name keeps the session's
    /// original case (`Foo.yaml`) while units are lowercase, and there can be more than one
    /// (`Foo.yaml` and `foo.yaml`), so match on the name inside the file, falling back to
    /// the file name for files that do not parse.
    pub fn find_session_files(&self, unit_name: &str) -> Result<Vec<PathBuf>> {
        Ok(self
            .list_session_files()?
            .into_iter()
            .filter(|path| {
                let name = match self.load_session(path.clone()) {
                    Ok(session) => session.name,
                    Err(_) => path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                name.eq_ignore_ascii_case(unit_name)
            })
            .collect())
    }
}
