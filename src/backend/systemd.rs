//! Everything to do with systemd: the unit files sysd sessions run from and the
//! D-Bus client that starts, stops and watches them.

pub mod client;
pub mod unit_file;

use bincode_next::{Decode, Encode};
use color_eyre::eyre::Result;

use crate::backend::session_manager::validate_session_name;

/// A sysd session's systemd unit name, without `.service`: a valid session name
/// (see `validate_session_name`), lowercased.
///
/// This only guarantees the name is safe to use in file names and unit files. Whether
/// systemd accepts it, and whether a unit by that name already exists, only systemd can
/// say: see `SystemdClient::ensure_unit_name_free`, which is asked before a service is made.
#[derive(Debug, Clone, Eq, Hash, PartialEq, Encode, Decode)]
pub struct UnitName(String);

impl UnitName {
    pub fn new(name: &str) -> Result<Self> {
        validate_session_name(name)?;
        Ok(UnitName(name.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for UnitName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
