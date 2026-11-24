use tracing::{debug, error, trace, warn};
use portable_pty::{Child, MasterPty, PtyPair, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use vt100::{Callbacks, Parser};

use crate::backend::terminal_manager::TerminalId;

/// Lines of scrollback kept by the parser, per terminal.
const SCROLLBACK: usize = 2000;
/// What the shell is told it runs in: the daemon's parser emulates an xterm, whatever
/// terminal the daemon itself was started from (or none, under systemd).
const TERM: &str = "xterm-256color";
/// Inherited variables that describe the daemon's own terminal rather than this one.
/// Programs use them to pick terminal-specific features the parser does not support.
const FOREIGN_TERMINAL_VARS: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "ALACRITTY_LOG",
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "KITTY_PUBLIC_KEY",
    "WEZTERM_PANE",
    "WEZTERM_EXECUTABLE",
    "VTE_VERSION",
    "KONSOLE_VERSION",
    "WT_SESSION",
];
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
/// `__osc133_launch <n>` reports `133;L;<n>` once the shell gets to it, and every prompt
/// reports `133;A`; see `PendingLaunch`.
///
/// The `bind`s make the keys the frontend sends for Ctrl/Alt + arrows, Home/End and
/// Ctrl+Delete edit the command line, whatever the system's inputrc does.
const SHELL_SETUP: &str = r#"
bind '"\e[1;5C": forward-word'
bind '"\e[1;5D": backward-word'
bind '"\e[1;3C": forward-word'
bind '"\e[1;3D": backward-word'
bind '"\e[3;5~": kill-word'
bind '"\e[3~": delete-char'
bind '"\e[H": beginning-of-line'
bind '"\e[F": end-of-line'
bind '"\eOH": beginning-of-line'
bind '"\eOF": end-of-line'
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
    printf '\033]133;A\007'
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
    /// A launched command line that has not finished yet.
    launch: Option<PendingLaunch>,
}

/// A command line the daemon typed into the shell, until the user starts another one.
///
/// While it is pending, the launch itself is what the terminal runs (or last ran):
/// `command` is the whole line and `working_dir` where it was started. The shell's own
/// reports cannot say that:
/// - Right after a launch they still describe whatever the shell did before (nothing, the
///   setup's `clear`, the `cd`), because the shell runs typed input in its own time.
/// - Once it runs, bash's DEBUG trap only reports one simple command at a time, so
///   `make && ./serve` would be reported as `make`, then `./serve`.
///
/// The shell reports `133;L;<id>` right before the launched line, so the first `133;C`
/// after it is the line starting, and the next prompt (`133;A`) means it has finished.
/// A `133;C` after that is a new command the user typed: from then on the shell's reports
/// are current. A launch without a command hands over as soon as the user starts one.
struct PendingLaunch {
    id: u64,
    command: String,
    working_dir: String,
    /// The shell reported `133;L;<id>`.
    reached: bool,
    /// The launched line started running (a `133;C` after `reached`).
    started: bool,
    /// The launched line finished (a prompt, `133;A`, after `started`).
    finished: bool,
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
                match state.launch.as_mut() {
                    // A command the user started: after the launched line finished, or
                    // instead of one when nothing was launched. It takes over.
                    Some(launch) if launch.finished || (launch.reached && launch.command.is_empty()) => {
                        state.launch = None;
                    }
                    Some(launch) if launch.reached => launch.started = true,
                    _ => {}
                }
                state.reported_command = command;
            }
            (Some(b"A"), _) => {
                // Back at the prompt: the launched line has finished.
                if let Some(launch) = self.state.lock().unwrap().launch.as_mut().filter(|l| l.started) {
                    launch.finished = true;
                }
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
    /// Bumped (while holding the parser lock) whenever the screen changes.
    generation: Arc<AtomicU64>,
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

        let mut command = portable_pty::CommandBuilder::new("/bin/bash");
        command.env("TERM", TERM);
        command.env("COLORTERM", "truecolor");
        for var in FOREIGN_TERMINAL_VARS {
            command.env_remove(var);
        }
        let child = pair
            .slave
            .spawn_command(command)
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
            nonzero(global_rows),
            nonzero(global_cols),
            SCROLLBACK,
            hooks,
        )));

        // Starts at 1, so a client asking with `since: 0` always gets the screen.
        let generation = Arc::new(AtomicU64::new(1));
        spawn_reader(
            pair.master.try_clone_reader().expect("failed to clone pty reader"),
            Arc::clone(&parser),
            Arc::clone(&generation),
        );

        let mut terminal = Self {
            id,
            parser,
            generation,
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
            started: false,
            finished: false,
        });

        self.execute_command(format!("cd {working_dir} && clear && __osc133_launch {id}"));
        match command {
            Some(command) if tracked => self.run_tracked(command),
            Some(command) => self.execute_command(command),
            None => {}
        }
    }

    pub fn get_screen(&self) -> Vec<u8> {
        self.parser.lock().unwrap().screen().state_formatted().into_bytes()
    }

    /// The screen scrolled `scrollback` lines back into the history (0 = live), unless it
    /// has not changed since `since`: then `data` is empty. See `TerminalUpdate`.
    pub fn snapshot(&self, scrollback: usize, since: u64) -> Snapshot {
        let mut parser = self.parser.lock().unwrap();
        let generation = self.generation.load(Ordering::Acquire);
        let screen = parser.screen_mut();
        let (rows, cols) = screen.size();
        let alternate_screen = screen.alternate_screen();

        // The view is only moved for this one read, so the parser always processes
        // output with the live screen in view, and other clients are unaffected.
        screen.set_scrollback(scrollback);
        let scrollback = screen.scrollback();
        let data = if generation == since {
            Vec::new()
        } else {
            screen.state_formatted().into_bytes()
        };
        screen.set_scrollback(0);

        Snapshot {
            data,
            rows: rows.get(),
            cols: cols.get(),
            scrollback,
            alternate_screen,
            generation,
        }
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
        let (rows, cols) = (nonzero(rows), nonzero(cols));
        {
            let mut parser = self.parser.lock().unwrap();
            if parser.screen().size() == (rows, cols) {
                return;
            }
            // Like xterm: shrinking pushes rows from the top into the scrollback (keeping
            // the newest output and the prompt on screen), growing pulls them back.
            parser.set_size(rows, cols);
            self.generation.fetch_add(1, Ordering::Release);
        }

        let result = self.pair.master.resize(PtySize {
            rows: rows.get(),
            cols: cols.get(),
            pixel_height: 0,
            pixel_width: 0,
        });
        if let Err(e) = result {
            warn!(terminal = ?self.id, "Resize failed: {e}");
        }
    }
}

/// vt100 sizes are non-zero; a 0 (a client reporting a zero-sized pane) becomes 1.
fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

/// A terminal's screen as `VirtualTerminal::snapshot` sees it.
pub struct Snapshot {
    pub data: Vec<u8>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
    pub alternate_screen: bool,
    pub generation: u64,
}

/// Feeds PTY output into the parser until the PTY closes.
fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<Parser<ShellHooks>>>,
    generation: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let mut parser = parser.lock().unwrap();
                    parser.process(&buffer[..n]);
                    generation.fetch_add(1, Ordering::Release);
                }
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
