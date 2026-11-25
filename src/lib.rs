//! Library half of the crate. `main.rs` is a thin entry point on top of it, and the
//! tests in `tests/` use it to exercise the internals directly.

/// The version of this build, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod backend;
pub mod cli;
pub mod frontend;
pub mod ids;
pub mod ipc;
pub mod logging;
pub mod names;
