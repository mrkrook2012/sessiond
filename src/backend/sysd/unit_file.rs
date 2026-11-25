//! File-side systemd work: the `.service` unit files sysd sessions run from.
//! Everything that talks to systemd itself lives in `systemd::client`.

use color_eyre::eyre::{Result, bail, eyre};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::backend::sysd::UnitName;
use crate::ipc::consts::SOCKET_ENV_VAR;

/// Overrides the directory unit files are written to / looked up in.
/// Only meant for tests: systemd itself will not see units outside its search path.
pub const SYSD_DIR_ENV_VAR: &str = "SESSIOND_SYSD_DIR";

/// The user unit directory: `SESSIOND_SYSD_DIR` if set, otherwise `~/.config/systemd/user`.
pub fn dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(SYSD_DIR_ENV_VAR) {
        return Ok(PathBuf::from(dir));
    }
    let home: String = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

/// Where the unit file for `name` lives in the user unit directory.
pub fn path(name: &UnitName) -> Result<PathBuf> {
    Ok(dir()?.join(format!("{name}.service")))
}

/// True if a unit file for `name` exists in the user unit directory.
pub fn exists(name: &UnitName) -> bool {
    path(name).map(|path| path.is_file()).unwrap_or(false)
}

/// Writes the unit file for `name` into the user unit directory, for the current user,
/// starting the session described by `yaml_path`. The caller must reload systemd afterwards.
pub fn create(yaml_path: &Path, name: &UnitName) -> Result<()> {
    let dir = dir()?;
    fs::create_dir_all(&dir)?;
    let user = std::env::var("USER")?;
    write_to(&dir, yaml_path, &user, name)
}

/// Writes `<dir>/<name>.service`, running as `user` and starting the session described by
/// `yaml_path`.
pub fn write_to(dir: &Path, yaml_path: &Path, user: &str, name: &UnitName) -> Result<()> {
    let mut file_path = dir.to_path_buf();
    file_path.push(name.as_str());
    file_path.add_extension("service");

    let bin_path = std::env::current_exe()?
        .into_os_string()
        .into_string()
        .map_err(|path| eyre!("Path of this binary is not valid UTF-8: {path:?}"))?;
    if bin_path.contains("(deleted)") {
        bail!("This binary was deleted or replaced while running ({bin_path}); restart the daemon");
    }

    let start_command = format!("{bin_path} __internal-sysd-start {:?}", yaml_path);
    let stop_command = format!("{bin_path} __internal-sysd-stop {:?}", name.as_str());

    let mut file_contents = render(user, dir, start_command, stop_command, RestartPolicy::OnFailure);

    // A daemon on a non-default socket (tests) must get the unit's ExecStart/ExecStop
    // to talk back to *it*, not to the default daemon.
    if let Some(socket) = std::env::var_os(SOCKET_ENV_VAR) {
        let line = format!("Environment=\"{SOCKET_ENV_VAR}={}\"\n", socket.to_string_lossy());
        file_contents = file_contents.replacen("[Service]\n", &format!("[Service]\n{line}"), 1);
    }

    fs::write(&file_path, file_contents)
        .map_err(|e| eyre!("Failed to write unit file {}: {e}", file_path.display()))?;
    Ok(())
}

fn render(
    user: &str,
    directory: &Path,
    start_command: String,
    stop_command: String,
    restart: RestartPolicy,
) -> String {
    "[Unit]
    Description=A sessiond managed systemd service
    After=network.target

    [Service]
    # The start command stays running for as long as the session does, and exits
    # with an error when a tracked command exits, so systemd restarts the session.
    Type=simple
    # The daemon may have been started from this unit; stopping or restarting
    # the unit must only end our start command, not the daemon and its sessions.
    KillMode=process
    User={user}
    WorkingDirectory={work_dir}

    ExecStart={start_command} 
    ExecStop={stop_command}

    Restart={restart_policy}
    RestartSec=3

    [Install]
    WantedBy=default.target
    
    "
    .replace("{user}", user)
    .replace("{start_command}", &start_command)
    .replace("{stop_command}", &stop_command)
    .replace("{work_dir}", &directory.to_string_lossy())
    .replace("{restart_policy}", restart.as_str())
}

pub enum RestartPolicy {
    No,
    OnSuccess,
    OnFailure,
    OnAbnormal,
    OnWatchdog,
    OnAbort,
    Always,
}

impl RestartPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::OnSuccess => "on-success",
            Self::OnFailure => "on-failure",
            Self::OnAbnormal => "on-abnormal",
            Self::OnWatchdog => "on-watchdog",
            Self::OnAbort => "on-abort",
            Self::Always => "always",
        }
    }
}
