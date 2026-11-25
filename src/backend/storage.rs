//! Everything that persists sessions: their YAML config files and systemd unit files.

pub mod session_files;

use tracing::{debug, info, warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
 
use crate::backend::sysd::UnitName;
use crate::backend::sysd::unit_file;
use crate::backend::storage::session_files::{SessionConfig, SessionFiles};
use color_eyre::eyre::Result;
 
/// Everything that touches disk or systemd: YAML session files and unit files.
/// It knows nothing about live terminals or windows.
pub struct SessionStore {
    files: SessionFiles,
    /// The YAML file each sysd session was last started from: the one its unit's
    /// `ExecStart` points at, and so the one an update must overwrite.
    sysd_configs: HashMap<UnitName, PathBuf>,
}

impl SessionStore {
    pub fn new(files: SessionFiles) -> Self {
        Self {
            files,
            sysd_configs: HashMap::new(),
        }
    }

    pub fn load(&mut self, file_path: &Path) -> Result<SessionConfig> {
        self.files.load(file_path)
    }

    /// Remembers that `unit` was started from `path`.
    pub fn record_sysd_config(&mut self, unit: UnitName, path: PathBuf) {
        self.sysd_configs.insert(unit, path);
    }

    /// The file `unit` was started from, if it was started by this daemon.
    pub fn sysd_config(&self, unit: &UnitName) -> Option<&Path> {
        self.sysd_configs.get(unit).map(PathBuf::as_path)
    }

    /// Overwrites the YAML file at `path`.
    pub fn save_to(&self, session: &SessionConfig, path: &Path) -> Result<()> {
        self.files.save_to(session, path)
    }
 
    /// Saves to the default session location and returns the path.
    pub fn save(&mut self, session: &SessionConfig) -> Result<PathBuf> {
        self.files.save(session)
    }
 
    /// Saves as `<dir>/<name>.yaml` and returns the full path.
    pub fn export_to_dir(&self, session: &SessionConfig, mut dir: PathBuf) -> Result<PathBuf> {
        dir.push(&session.name);
        dir.set_extension("yaml");
 
        self.files.save_to(session, &dir)?;
        Ok(dir)
    }
 
    /// Writes the unit file. The caller must reload systemd afterwards.
    pub fn create_service(&mut self, yaml_path: &Path, unit: &UnitName) -> Result<()> {
        unit_file::create(yaml_path, unit)
    }
 
    /// Finds sysd sessions that exist on disk (YAML + unit file) so they can be
    /// registered as stopped sessions when the daemon starts.
    ///
    /// YAML files without a unit file are plain exports/leftovers and are skipped,
    /// as are files that fail to parse. Duplicate unit names keep the first match.
    pub fn discover_sysd_sessions(&self) -> Result<Vec<StoredSysdSession>> {
        let mut found: Vec<StoredSysdSession> = Vec::new();

        for path in self.files.list()? {
            let yaml = match self.files.load(&path) {
                Ok(yaml) => yaml,
                Err(e) => {
                    warn!("Skipping {}: failed to parse ({e})", path.display());
                    continue;
                }
            };

            let unit = match UnitName::new(&yaml.name) {
                Ok(unit) => unit,
                Err(e) => {
                    warn!("Skipping {}: invalid unit name ({e})", path.display());
                    continue;
                }
            };

            if !unit_file::exists(&unit) {
                // Normal for exported sessions that were never turned into a service.
                debug!("Skipping {}: no unit file for '{}'", path.display(), unit);
                continue;
            }

            if found.iter().any(|s| s.unit == unit) {
                warn!("Skipping {}: duplicate of unit '{}'", path.display(), unit);
                continue;
            }

            found.push(StoredSysdSession { unit });
        }

        Ok(found)
    }

    pub fn unit_file_exists(&self, unit: &UnitName) -> bool {
        unit_file::exists(unit)
    }

    /// Removes the unit file and every YAML file of this session. Files that are already
    /// gone are skipped, so a half-deleted session can still be cleaned up.
    /// Returns the files that were removed.
    pub fn delete(&mut self, unit: &UnitName) -> Result<Vec<PathBuf>> {
        let mut files = self.files.find_for_unit(unit.as_str())?;
        if unit_file::exists(unit) {
            files.push(unit_file::path(unit)?);
        }

        for file in &files {
            match std::fs::remove_file(file) {
                Ok(()) => info!("Deleted {}", file.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(color_eyre::eyre::eyre!(
                        "Failed to delete {}: {e}",
                        file.display()
                    ));
                }
            }
        }

        self.sysd_configs.remove(unit);
        Ok(files)
    }
}

/// A sysd session found on disk at daemon startup.
pub struct StoredSysdSession {
    pub unit: UnitName,
}
