use tracing::{debug, error, trace, warn};
use portable_pty::{Child, MasterPty, PtyPair, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use vt100::{Callbacks, Parser};

use crate::backend::terminal_manager::TerminalId;

/// Lines of scrollback kept by the parser.
const SCROLLBACK: usize = 20;
/// How long to wait for the shell to draw its first prompt.
const SHELL_STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the shell gets to exit after SIGHUP/SIGTERM before it is SIGKILLed.
pub const TERMINATE_GRACE: Duration = Duration::from_millis(500);

/// Injected into every shell. Reports the running command and directory through
/// OSC 133 sequences, and the exit code of a *tracked* command through `133;T`.
///
/// A tracked command is typed as `__osc133_track; <command>`. The prompt hook runs
/// first in PROMPT_COMMAND so it still sees the command's `$?`, and only reports once
/// per tracked command. Ctrl+C aborts the rest of the line, but the prompt hook still runs.
const SHELL_SETUP: &str = r#"
__osc133_command_start() {
    case "$BASH_COMMAND" in
        __osc133_*|printf*|local*|trap*|PROMPT_COMMAND*)
            return
            ;;
    esac

    printf '\033]133;P;%s\007' "$PWD"
    printf '\033]133;C;%s\007' "$BASH_COMMAND"
}
__osc133_track() { __osc133_tracking=1; }
__osc133_prompt() {
    local status=$?
    if [ -n "$__osc133_tracking" ]; then
        unset __osc133_tracking
        printf '\033]133;T;%s\007' "$status"
    fi
}
trap '__osc133_command_start' DEBUG
PROMPT_COMMAND="__osc133_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
clear
"#;

/// Things that happen inside a terminal that the rest of the daemon cares about.
#[derive(Debug, Clone, Copy)]
pub enum TerminalEvent {
    /// A command started with `VirtualTerminal::run_tracked` returned to the prompt.
    TrackedCommandExited { terminal_id: TerminalId, code: i32 },
}

/// vt100 callbacks: picks our OSC 133 sequences out of the output stream.
struct ShellHooks {
    terminal_id: TerminalId,
    current_command: Arc<RwLock<String>>,
    current_pwd: Arc<RwLock<String>>,
    events: UnboundedSender<TerminalEvent>,
}

impl Callbacks for ShellHooks {
    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() != Some(&b"133".as_slice()) {
            return;
        }
        let value = params.get(2).map(|v| String::from_utf8_lossy(v).into_owned());

        match (params.get(1).copied(), value) {
            (Some(b"C"), Some(command)) => *self.current_command.write().unwrap() = command,
            (Some(b"P"), Some(pwd)) => *self.current_pwd.write().unwrap() = pwd,
            (Some(b"T"), Some(code)) => {
                let code = code.trim().parse().unwrap_or(-1);
                debug!(terminal = ?self.terminal_id, code, "Tracked command exited");
                // The receiver only disappears when the daemon is shutting down.
                let _ = self.events.send(TerminalEvent::TrackedCommandExited {
                    terminal_id: self.terminal_id,
                    code,
                });
            }
            _ => {}
        }
    }
}

/// A bash shell running in a PTY, with its screen parsed by vt100.
pub struct VirtualTerminal {
    id: TerminalId,
    parser: Arc<Mutex<Parser<ShellHooks>>>,
    pair: PtyPair,
    /// Taken in `Drop` and handed to a reaper thread.
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Box<dyn Write + Send>,
    width_factor: f32,
    height_factor: f32,
    global_rows: u16,
    global_cols: u16,
    current_command: Arc<RwLock<String>>,
    current_pwd: Arc<RwLock<String>>,
    /// When the user last typed into this terminal. Used to tell a manual stop from a crash.
    last_user_input: Option<Instant>,
}

impl VirtualTerminal {
    pub fn new(
        id: TerminalId,
        global_rows: u16,
        global_cols: u16,
        events: UnboundedSender<TerminalEvent>,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: global_rows,
                cols: global_cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .expect("failed to open pty");

        let child = pair
            .slave
            .spawn_command(portable_pty::CommandBuilder::new("/bin/bash"))
            .expect("failed to spawn shell");

        let writer = pair.master.take_writer().expect("failed to take pty writer");
        debug!(terminal = ?id, shell_pid = ?child.process_id(), "Spawned shell");

        let current_command = Arc::new(RwLock::new(String::new()));
        let current_pwd = Arc::new(RwLock::new(String::new()));

        let hooks = ShellHooks {
            terminal_id: id,
            current_command: Arc::clone(&current_command),
            current_pwd: Arc::clone(&current_pwd),
            events,
        };
        let parser = Arc::new(Mutex::new(Parser::new_with_callbacks(
            global_rows,
            global_cols,
            SCROLLBACK,
            hooks,
        )));

        spawn_reader(
            pair.master.try_clone_reader().expect("failed to clone pty reader"),
            Arc::clone(&parser),
        );

        let mut terminal = Self {
            id,
            parser,
            pair,
            child: Some(child),
            writer,
            width_factor: 1.0,
            height_factor: 1.0,
            global_rows,
            global_cols,
            current_command,
            current_pwd,
            last_user_input: None,
        };

        terminal.wait_for_first_output();
        terminal.write(SHELL_SETUP.as_bytes());

        terminal
    }

    /// Input typed in shell before its rc files ran can get lost, so wait until it drew something.
    fn wait_for_first_output(&self) {
        let deadline = Instant::now() + SHELL_STARTUP_TIMEOUT;
        while self.parser.lock().unwrap().screen().contents().is_empty() {
            if Instant::now() >= deadline {
                warn!(terminal = ?self.id, "Shell printed nothing during startup");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let result = self
            .writer
            .write_all(bytes)
            .and_then(|_| self.writer.flush());
        if let Err(e) = result {
            warn!(terminal = ?self.id, "Write to terminal failed: {e}");
        }
    }

    pub fn get_id(&self) -> TerminalId {
        self.id
    }

    /// Keystrokes from a user.
    pub fn process_input(&mut self, bytes: &[u8]) {
        self.last_user_input = Some(Instant::now());
        self.write(bytes);
    }

    pub fn last_user_input(&self) -> Option<Instant> {
        self.last_user_input
    }

    pub fn execute_command(&mut self, command: String) {
        self.write(format!("{command}\r").as_bytes());
    }

    /// Runs `command`, and emits `TrackedCommandExited` once it returns to the prompt.
    pub fn run_tracked(&mut self, command: String) {
        debug!(terminal = ?self.id, %command, "Running tracked command");
        self.execute_command(format!("__osc133_track; {command}"));
    }

    pub fn get_screen(&self) -> Vec<u8> {
        self.parser.lock().unwrap().screen().state_formatted()
    }

    pub fn get_current_command(&self) -> String {
        self.current_command.read().unwrap().clone()
    }

    pub fn get_current_pwd(&self) -> String {
        self.current_pwd.read().unwrap().clone()
    }

    pub fn get_ratio(&self) -> (f32, f32) {
        (self.height_factor, self.width_factor)
    }

    pub fn set_ratio(&mut self, height_factor: f32, width_factor: f32) {
        self.height_factor = height_factor;
        self.width_factor = width_factor;
        self.recalculate_size();
    }

    pub fn recalculate_size(&mut self) {
        self.set_size(
            (self.global_rows as f32 * self.height_factor) as u16,
            (self.global_cols as f32 * self.width_factor) as u16,
        );
    }

    pub fn set_global_size(&mut self, rows: u16, cols: u16) {
        self.global_rows = rows;
        self.global_cols = cols;
    }

    fn set_size(&mut self, rows: u16, cols: u16) {
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);

        let result = self.pair.master.resize(PtySize {
            rows,
            cols,
            pixel_height: 0,
            pixel_width: 0,
        });
        if let Err(e) = result {
            warn!(terminal = ?self.id, "Resize failed: {e}");
        }
    }
}

/// Feeds PTY output into the parser until the PTY closes.
fn spawn_reader(mut reader: Box<dyn Read + Send>, parser: Arc<Mutex<Parser<ShellHooks>>>) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => parser.lock().unwrap().process(&buffer[..n]),
                // EIO is how Linux reports that the other side of the PTY is gone.
                Err(_) => break,
            }
        }
    });
}

impl Drop for VirtualTerminal {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            terminate(self.id, child, &*self.pair.master);
        }
    }
}

/// Stops the shell and everything running in it, without blocking the caller.
///
/// The shell is the session leader of the PTY; a running command lives in its own
/// (foreground) process group. Both get SIGHUP + SIGTERM now, and a reaper thread
/// SIGKILLs whatever is left after a grace period and then reaps the shell, so no
/// zombies are left behind and the daemon never waits on a slow process.
fn terminate(id: TerminalId, mut child: Box<dyn Child + Send + Sync>, master: &dyn MasterPty) {
    let mut groups: Vec<libc::pid_t> = Vec::new();

    // Note: `master.process_group_leader()` returns the *foreground* group, which is
    // the running command rather than the shell. The shell is the child itself.
    if let Some(shell) = child.process_id().map(|pid| pid as libc::pid_t) {
        groups.push(shell);

        if let Some(foreground) = master.process_group_leader() {
            // Only touch groups that belong to this terminal's session.
            if foreground != shell && unsafe { libc::getsid(foreground) == libc::getsid(shell) } {
                groups.push(foreground);
            }
        }
    }

    debug!(terminal = ?id, ?groups, "Terminating shell");

    // An interactive bash ignores SIGTERM but exits on SIGHUP (and forwards it to its jobs).
    signal_groups(&groups, libc::SIGHUP);
    signal_groups(&groups, libc::SIGTERM);

    std::thread::spawn(move || {
        let deadline = Instant::now() + TERMINATE_GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                trace!(terminal = ?id, ?status, "Shell exited");
                // The shell is gone; make sure its command is too.
                signal_groups(&groups, libc::SIGKILL);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        debug!(terminal = ?id, "Shell did not exit in time, sending SIGKILL");
        signal_groups(&groups, libc::SIGKILL);
        if let Err(e) = child.wait() {
            error!(terminal = ?id, "Failed to reap shell: {e}");
        }
    });
}

fn signal_groups(groups: &[libc::pid_t], signal: libc::c_int) {
    for &pgid in groups {
        if pgid <= 0 {
            continue;
        }
        // A negative pid signals the whole process group.
        if unsafe { libc::kill(-pgid, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            // ESRCH: the group is already gone.
            if error.raw_os_error() != Some(libc::ESRCH) {
                warn!("Failed to signal process group {pgid}: {error}");
            }
        }
    }
}
