//! End-to-end tests: run the real `daemon_test` binary against an isolated daemon.
//!
//! Every test gets its own socket, data directory and log directory (see `harness`).
//! Tests that need a user systemd instance or fish skip themselves when those are missing.

mod harness;

mod completions;
mod daemon;
mod export_import;
mod logging;
mod sessions;
mod systemd;
