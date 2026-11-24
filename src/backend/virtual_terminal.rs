use tracing::{debug, error, trace, warn};
use portable_pty::{Child, MasterPty, PtyPair, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
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
///
/// `__osc133_launch <n>` reports `133;L;<n>` once the shell gets to it; see `PendingLaunch`.
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
__osc133_launch() { printf '\033]133;L;%s\007' "$1"; }
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

/// What the shell is doing, as far as the daemon knows.
#[derive(Default)]
struct ShellState {
    /// The command / directory the shell last reported (`133;C` / `133;P`).
    reported_command: String,
    reported_pwd: String,
    /// A launch the shell has not caught up with yet.
    launch: Option<PendingLaunch>,
}

/// A command the daemon typed into the shell that the shell has not started yet.
///
/// The shell runs typed input in its own time, so right after a launch its reports still
/// describe whatever it did before (nothing, the setup's `clear`, the `cd`). Until it
/// reports starting the launched command, the launch itself is the truth: `command` and
/// `working_dir` are what the terminal is about to run, and where.
///
/// The shell reports `133;L;<id>` right before the launched command line, so the first
/// `133;C` after it is that command, and from then on the shell's reports are current.
struct PendingLaunch {
    id: u64,
    command: String,
    working_dir: String,
    /// The shell reported `133;L;<id>`.
    reached: bool,
}

impl ShellState {
    fn command(&self) -> &str {
        match &self.launch {
            Some(launch) => &launch.command,
            None => &self.reported_command,
        }
    }

    fn pwd(&self) -> &str {
        match &self.launch {
            Some(launch) => &launch.working_dir,
            None => &self.reported_pwd,
        }
    }
}

/// vt100 callbacks: picks our OSC 133 sequences out of the output stream.
struct ShellHooks {
    terminal_id: TerminalId,
    state: Arc<Mutex<ShellState>>,
    events: UnboundedSender<TerminalEvent>,
}

impl Callbacks for ShellHooks {
    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() != Some(&b"133".as_slice()) {
            return;
        }
        let value = params.get(2).map(|v| String::from_utf8_lossy(v).into_owned());

        match (params.get(1).copied(), value) {
            (Some(b"C"), Some(command)) => {
                let mut state = self.state.lock().unwrap();
                if state.launch.as_ref().is_some_and(|launch| launch.reached) {
                    // This is the launched command starting: the reports are current now.
                    state.launch = None;
                }
                state.reported_command = command;
            }
            (Some(b"P"), Some(pwd)) => self.state.lock().unwrap().reported_pwd = pwd,
            (Some(b"L"), Some(id)) => {
                let id: Option<u64> = id.trim().parse().ok();
                let mut state = self.state.lock().unwrap();
                if let Some(launch) = state.launch.as_mut().filter(|launch| Some(launch.id) == id) {
                    launch.reached = true;
                }
            }
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
    state: Arc<Mutex<ShellState>>,
    /// Source of `PendingLaunch` ids.
    launches: u64,
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

        let state = Arc::new(Mutex::new(ShellState::default()));

        let hooks = ShellHooks {
            terminal_id: id,
            state: Arc::clone(&state),
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
            state,
            launches: 0,
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

    /// Changes to `working_dir` and runs `command` there (if any; tracked if `tracked`).
    ///
    /// From the moment this returns, `get_current_command` and `get_current_pwd` describe
    /// the launch, even though the shell may not have read the input yet. See `PendingLaunch`.
    pub fn launch(&mut self, working_dir: &str, command: Option<String>, tracked: bool) {
        self.launches += 1;
        let id = self.launches;

        // Recorded before anything is typed, so nobody can read a state from before the launch.
        self.state.lock().unwrap().launch = Some(PendingLaunch {
            id,
            command: command.clone().unwrap_or_default(),
            working_dir: working_dir.to_owned(),
            reached: false,
        });

        self.execute_command(format!("cd {working_dir} && clear && __osc133_launch {id}"));
        match command {
            Some(command) if tracked => self.run_tracked(command),
            Some(command) => self.execute_command(command),
            None => {}
        }
    }

    pub fn get_screen(&self) -> Vec<u8> {
        self.parser.lock().unwrap().screen().state_formatted()
    }

    pub fn get_current_command(&self) -> String {
        self.state.lock().unwrap().command().to_owned()
    }

    pub fn get_current_pwd(&self) -> String {
        self.state.lock().unwrap().pwd().to_owned()
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

impl VirtualTerminal {
    /// Starts stopping the shell; see `Termination`. Returns `None` if that already happened.
    pub fn begin_termination(&mut self) -> Option<Termination> {
        let child = self.child.take()?;
        Some(Termination::start(self.id, child, &*self.pair.master))
    }
}

impl Drop for VirtualTerminal {
    /// Never blocks: the rest of the termination happens on a reaper thread.
    fn drop(&mut self) {
        if let Some(termination) = self.begin_termination() {
            let deadline = Instant::now() + TERMINATE_GRACE;
            std::thread::spawn(move || finish_all(vec![termination], deadline));
        }
    }
}

/// A shell (and whatever runs in it) that has been asked to stop.
///
/// The shell is the session leader of the PTY; a running command lives in its own
/// (foreground) process group. `start` sends both SIGHUP + SIGTERM. Whatever is left
/// once the shell exits, or once the grace period ends, gets SIGKILL, and the shell is
/// reaped so no zombies are left behind.
pub struct Termination {
    id: TerminalId,
    child: Box<dyn Child + Send + Sync>,
    groups: Vec<libc::pid_t>,
}

impl Termination {
    fn start(id: TerminalId, child: Box<dyn Child + Send + Sync>, master: &dyn MasterPty) -> Self {
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

        Self { id, child, groups }
    }

    /// If the shell has exited, SIGKILLs what is left of its command and returns true.
    fn try_finish(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                trace!(terminal = ?self.id, ?status, "Shell exited");
                // The shell is gone; make sure its command is too.
                signal_groups(&self.groups, libc::SIGKILL);
                true
            }
            _ => false,
        }
    }

    /// SIGKILLs everything that is left and reaps the shell. Blocks until it is gone.
    fn kill(mut self) {
        debug!(terminal = ?self.id, "Shell did not exit in time, sending SIGKILL");
        signal_groups(&self.groups, libc::SIGKILL);
        if let Err(e) = self.child.wait() {
            error!(terminal = ?self.id, "Failed to reap shell: {e}");
        }
    }
}

/// Waits until every shell has exited or `deadline` passes, then kills what is left.
/// Blocks until all of them are reaped.
pub fn finish_all(mut terminations: Vec<Termination>, deadline: Instant) {
    loop {
        terminations.retain_mut(|termination| !termination.try_finish());
        if terminations.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    for termination in terminations {
        termination.kill();
    }
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
