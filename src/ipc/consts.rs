use interprocess::local_socket::{GenericFilePath, Name, ToFsName};
use std::path::PathBuf;

pub const VERSION: f32 = 0.00;
pub const DEFAULT_SOCKET_PATH: &str = "/tmp/daemon_test.sock";
pub const SOCKET_ENV_VAR: &str = "DAEMON_TEST_SOCKET";

/// Socket location: `DAEMON_TEST_SOCKET` if set, otherwise the default.
/// The auto-spawned daemon inherits the env var, so CLI and daemon always agree.
pub fn socket_path() -> PathBuf {
    std::env::var_os(SOCKET_ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

pub fn socket_name() -> Name<'static> {
    socket_path()
        .to_fs_name::<GenericFilePath>()
        .expect("invalid socket path")
}
