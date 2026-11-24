//! Helpers shared by the unit tests.

use std::time::{Duration, Instant};

/// Polls `probe` until it returns `Some`, or panics with `what` after `timeout`.
/// Terminals run asynchronously, so anything read back from one needs this.
pub fn wait_for<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `wait_for` with the timeout used for shell output.
pub fn eventually<T>(what: &str, probe: impl FnMut() -> Option<T>) -> T {
    wait_for(what, Duration::from_secs(10), probe)
}

/// The text on a terminal screen, from the formatted state the daemon sends to clients.
pub fn screen_text(formatted: &[u8], rows: u16, cols: u16) -> String {
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(formatted);
    parser.screen().contents()
}
