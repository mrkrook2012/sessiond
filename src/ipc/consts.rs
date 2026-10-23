use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

pub const VERSION: f32 = 0.00;
pub const SOCKET_PATH: &str = "/tmp/daemon_test.sock";
pub const UPDATE_SOCKET_PATH: &str = "/tmp/penar2.sock";

pub fn socket_name() -> Name<'static> {
    SOCKET_PATH
        .to_fs_name::<GenericFilePath>()
        .expect("invalid socket path")
}

pub fn update_socket_name() -> Name<'static> {
    UPDATE_SOCKET_PATH
        .to_fs_name::<GenericFilePath>()
        .expect("invalid socket path")
}
