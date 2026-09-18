use std::path::Path;
use color_eyre::eyre::Result;
use tokio::fs;
use std::path::PathBuf;


pub struct SystemdManager {}

impl SystemdManager {

    pub async fn write_default_template(
        path: PathBuf,
        user: String,
        working_dir: &Path,
        start_command: String,
    ) -> Result<()> {
        let file_contents = Self::get_filled_template(&user, working_dir, start_command, RestartPolicy::Always);
        fs::write(path, file_contents).await?;

        Ok(())
    }

    fn get_filled_template(
        user: &str,
        working_dir: &Path,
        start_command: String,
        restart: RestartPolicy,
    ) -> String {
        "[Unit]
        Description=A daemon_test managed systemd service
        After=network.target

        [Service]
        Type=simple
        User={user}
        WorkingDirectory={work_dir}

        ExecStart={start_command}

        Restart={restart_policy}
        RestartSec=10

        [Install]
        WantedBy=multi-user.target
        
        "
        .to_owned()
        .replace("{user}", &user)
        .replace("{start_command}", &start_command)
        .replace("{work_dir}", working_dir.to_str().unwrap())
        .replace("{restart_policy}", restart.as_str())
    }

    fn get_file_location(user: &str, name: &str) -> PathBuf {
        PathBuf::from("/home")
            .join(user)
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
