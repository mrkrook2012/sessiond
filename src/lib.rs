//! Library half of the crate. `main.rs` is a thin entry point on top of it, and the
//! tests in `tests/` use it to exercise the internals directly.

pub mod backend;
pub mod cli;
pub mod frontend;
pub mod ipc;
pub mod logging;
