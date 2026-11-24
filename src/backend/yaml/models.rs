use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct YamlSession {
    pub name: String,
    pub windows: Vec<YamlWindow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YamlWindow {
    pub command: String,
    pub working_directory: String,
    /// Restart the (systemd) session when this window's command exits on its own.
    /// Ignored for ephemeral sessions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tracked: bool,
}
