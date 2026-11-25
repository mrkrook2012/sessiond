use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::Result;
use serde::{Deserialize, Serialize};

/// Overrides the directory session YAML files are stored in (used by tests).
pub const DATA_DIR_ENV_VAR: &str = "SESSIOND_DATA_DIR";

/// The directory session config files (`<name>.yaml`) are stored in.
pub struct SessionFiles {
    storage_dir: PathBuf,
}

impl SessionFiles {
    pub fn new() -> Result<Self> {
        let storage_dir = match std::env::var_os(DATA_DIR_ENV_VAR) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var("HOME")?).join(".local/share/sessiond"),
        };

        Self::with_dir(storage_dir)
    }

    /// Stores session files in `storage_dir`, creating it if needed.
    pub fn with_dir(storage_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&storage_dir)?;

        Ok(Self { storage_dir })
    }

    pub fn dir(&self) -> &PathBuf {
        &self.storage_dir
    }

    pub fn path_for(&self, name: &str) -> PathBuf {
        self.storage_dir.join(format!("{name}.yaml"))
    }

    /// All `*.yaml` files in the storage directory, sorted for a stable order.
    pub fn list(&self) -> Result<Vec<PathBuf>> {
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

    pub fn load(&self, path: &Path) -> Result<SessionConfig> {
        use color_eyre::eyre::WrapErr;
        let input = fs::read_to_string(path)
            .wrap_err_with(|| format!("Failed to read session file {}", path.display()))?;
        serde_yaml::from_str(&input)
            .wrap_err_with(|| format!("Invalid session file {}", path.display()))
    }

    pub fn save(&self, session: &SessionConfig) -> Result<PathBuf> {
        let path = self.path_for(&session.name);
        self.save_to(session, &path)?;
        Ok(path)
    }

    /// Writes via a temporary file and a rename, so the file is never seen half written:
    /// a systemd unit may load it at any moment.
    pub fn save_to(&self, session: &SessionConfig, path: &Path) -> Result<()> {
        let yaml = self.serialize(session)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, yaml)?;
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    pub fn serialize(&self, session: &SessionConfig) -> Result<String> {
        Ok(serde_yaml::to_string(session)?)
    }

    pub fn deserialize(&self, yaml: &str) -> Result<SessionConfig> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    /// Every stored YAML file belonging to `unit_name`. The file name keeps the session's
    /// original case (`Foo.yaml`) while units are lowercase, and there can be more than one
    /// (`Foo.yaml` and `foo.yaml`), so match on the name inside the file, falling back to
    /// the file name for files that do not parse.
    pub fn find_for_unit(&self, unit_name: &str) -> Result<Vec<PathBuf>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|path| {
                let name = match self.load(path) {
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

/// A session as stored in a YAML file: what to recreate when it is started.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub name: String,
    pub windows: Vec<WindowConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WindowConfig {
    pub command: String,
    pub working_directory: String,
    /// Restart the (systemd) session when this window's command exits on its own.
    /// Ignored for ephemeral sessions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tracked: bool,
}
