use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct YamlSession {
    pub session_name: String,
    pub windows: Vec<YamlWindow>,
}

impl YamlSession {}

#[derive(Debug, Serialize, Deserialize)]
pub struct YamlWindow {
    pub command: String,
}
