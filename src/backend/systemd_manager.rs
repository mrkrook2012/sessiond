use color_eyre::eyre::Result;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::backend::session_manager::UnitName;

pub struct SystemdManager {}

impl SystemdManager {
    pub fn remove_sysd_service_file(name: &str) -> Result {
        let home: String = std::env::var("HOME")?;

        let path = Self::get_file_location_from_home(&home, name);
        println!("Deleting {:?}", path);
        std::fs::remove_file(path)?;
        Ok(())
    }
    pub fn write_default_template_to_path(
        path_to_config_file: PathBuf,
        user: String,
        sysd_directory: &Path,
        name: UnitName,
    ) -> Result<()> {
        let mut service_file_path_buffer = sysd_directory.to_path_buf();
        service_file_path_buffer.push(name.to_string());
        service_file_path_buffer.add_extension("service");
        let bin_path = std::env::current_exe()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();

        if bin_path.contains("(deleted)") {
            panic!("Binary location has been deleted???")
        }
        let start_command = format!(
            "{bin_path} __internal-systemd-start {:?}",
            path_to_config_file.as_path()
        );
        let stop_command = format!("{bin_path} __internal-systemd-stop {:?}", name.to_string());

        let file_contents = Self::get_filled_template(
            &user,
            sysd_directory,
            start_command,
            stop_command,
            RestartPolicy::No,
        );
        fs::write(service_file_path_buffer, file_contents).unwrap();

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
        Type=oneshot
        RemainAfterExit=yes
        User={user}
        WorkingDirectory={work_dir}

        ExecStart={start_command} 
        ExecStop={stop_command}

        Restart={restart_policy}

        [Install]
        WantedBy=multi-user.target
        
        "
        .to_owned()
        .replace("{user}", &user)
        .replace("{start_command}", &start_command)
        .replace("{stop_command}", &stop_command)
        .replace("{work_dir}", directory.to_str().unwrap())
        .replace("{restart_policy}", restart.as_str())
    }

    fn get_file_location(user: &str, name: &str) -> PathBuf {
        PathBuf::from("/home")
            .join(user)
            .join(".config/systemd/user")
            .join(format!("{name}.service"))
    }

    fn get_file_location_from_home(home: &str, name: &str) -> PathBuf {
        PathBuf::from(home)
            .join(".config/systemd/user")
            .join(format!("{name}.service"))
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
