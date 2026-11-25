use color_eyre::eyre::{Result, bail, eyre};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::backend::session_manager::UnitName;
use crate::ipc::consts::SOCKET_ENV_VAR;

/// Overrides the directory unit files are written to / looked up in.
/// Only meant for tests: systemd itself will not see units outside its search path.
pub const SYSTEMD_DIR_ENV_VAR: &str = "DAEMON_TEST_SYSTEMD_DIR";

pub struct SystemdManager {}

impl SystemdManager {
    /// The user unit directory: `DAEMON_TEST_SYSTEMD_DIR` if set, otherwise `~/.config/systemd/user`.
    pub fn unit_dir() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os(SYSTEMD_DIR_ENV_VAR) {
            return Ok(PathBuf::from(dir));
        }
        let home: String = std::env::var("HOME")?;
        Ok(PathBuf::from(home).join(".config/systemd/user"))
    }

    pub fn unit_file_path(name: &str) -> Result<PathBuf> {
        Ok(Self::unit_dir()?.join(format!("{name}.service")))
    }

    pub fn write_default_template_to_path(
        path_to_config_file: &Path,
        user: &str,
        sysd_directory: &Path,
        name: &UnitName,
    ) -> Result<()> {
        let mut service_file_path_buffer = sysd_directory.to_path_buf();
        service_file_path_buffer.push(name.as_str());
        service_file_path_buffer.add_extension("service");
        let bin_path = std::env::current_exe()?
            .into_os_string()
            .into_string()
            .map_err(|path| eyre!("Path of this binary is not valid UTF-8: {path:?}"))?;

        if bin_path.contains("(deleted)") {
            bail!("This binary was deleted or replaced while running ({bin_path}); restart the daemon");
        }
        let start_command = format!(
            "{bin_path} __internal-systemd-start {:?}",
            path_to_config_file
        );
        let stop_command = format!("{bin_path} __internal-systemd-stop {:?}", name.as_str());

        let mut file_contents = Self::get_filled_template(
            user,
            sysd_directory,
            start_command,
            stop_command,
            RestartPolicy::OnFailure,
        );

        // A daemon on a non-default socket (tests) must get the unit's ExecStart/ExecStop
        // to talk back to *it*, not to the default daemon.
        if let Some(socket) = std::env::var_os(SOCKET_ENV_VAR) {
            let line = format!("Environment=\"{SOCKET_ENV_VAR}={}\"\n", socket.to_string_lossy());
            file_contents = file_contents.replacen("[Service]\n", &format!("[Service]\n{line}"), 1);
        }
        fs::write(&service_file_path_buffer, file_contents).map_err(|e| {
            eyre!("Failed to write unit file {}: {e}", service_file_path_buffer.display())
        })?;

        Ok(())
    }

    fn get_filled_template(
        user: &str,
        directory: &Path,
        start_command: String,
        stop_command: String,

        restart: RestartPolicy,
    ) -> String {
        "[Unit]
        Description=A daemon_test managed systemd service
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
