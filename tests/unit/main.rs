//! In-process tests against the `daemon_test` library.
//!
//! Pure data structures are tested directly. Tests that need a terminal spawn a real
//! bash in a PTY; nothing here starts a daemon or touches systemd. For those, see `tests/e2e`.

mod backend;
mod cli;
mod ids;
mod support;
