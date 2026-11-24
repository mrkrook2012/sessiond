use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use color_eyre::eyre::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::backend::session_manager::SessionId;
use crate::backend::terminal_manager::TerminalId;
use crate::backend::window_manager::WindowId;
use crate::frontend::session_list::{
    action_done, action_in_progress, is_service_key, new_session, reselect, service_action_for_key,
    service_keys, sort_sessions,
};
use crate::frontend::slop_terrm_widget::TerminalWidget;
use crate::ipc::client::DaemonClient;
use crate::ipc::protocol::{SessionInfo, SysdAction, SysdData, WindowInfo};

// Fallback size used only if we somehow can't read the real terminal
// dimensions at startup (e.g. `terminal.size()` fails).
const FALLBACK_ROWS: u16 = 100;
const FALLBACK_COLS: u16 = 180;

/// How long the poller sleeps when there is no visible terminal.
const POLL_IDLE: Duration = Duration::from_millis(50);
/// How long the poller sleeps after a request that returned no new data
/// (avoids a hot loop if `get_wanted_data` returns immediately).
const POLL_EMPTY_BACKOFF: Duration = Duration::from_millis(10);
/// How long the poller sleeps after an RPC error.
const POLL_ERROR_BACKOFF: Duration = Duration::from_millis(250);
/// How often the session list is refreshed, so service states changed elsewhere (systemctl,
/// a tracked command restarting its service, another client) show up on their own.
const LIST_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Which screen is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    SessionList,
    Terminal,
}

/// Whether we're taking normal keybind input, or capturing free text for a
/// new session's name.
#[derive(Debug, Clone)]
enum InputMode {
    Normal,
    NewSessionName(String),
    /// Ctrl-b (the tmux-style prefix) was just pressed; the next keypress
    /// is interpreted as a command instead of being sent to the terminal.
    PrefixPending,
}

/// Work for the RPC worker task. The UI thread never awaits; it just
/// queues one of these. The worker processes them strictly in order, which
/// matters for things like terminal input and window switching.
#[derive(Debug)]
enum Command {
    ListSessions,
    CreateSession {
        working_dir: String,
        name: String,
    },
    OpenSession {
        id: SessionId,
    },
    KillSession {
        id: SessionId,
    },
    ExitSession,
    CreateWindow {
        id: SessionId,
    },
    KillWindow {
        id: SessionId,
        window_id: WindowId,
        /// Window that should be focused once the kill has completed.
        focus_after: Option<WindowId>,
    },
    ChangeWindow {
        window_id: WindowId,
    },
    WriteTerminal {
        terminal_id: TerminalId,
        data: Vec<u8>,
    },
    UpdateScreenSize {
        rows: u16,
        cols: u16,
    },
    ExportSessionToSysd {
        id: SessionId,
    },
    ExportSessionToFile {
        id: SessionId,
        working_dir: PathBuf,
    },
    /// Start / stop / restart / enable / disable a systemd session.
    SysdAction {
        id: SessionId,
        /// For the status line.
        name: String,
        action: SysdAction,
    },
}

/// Everything the render loop needs, shared with the background tasks
/// behind a mutex.
struct AppState {
    screen: Screen,
    sessions: Vec<SessionInfo>,
    session_list_index: usize,
    current_session: Option<SessionInfo>,
    current_window_index: usize,
    // The single terminal widget for whatever window is currently
    // displayed, tagged with the terminal_id it belongs to so we know
    // when to replace it (switched windows) vs. just resize it (same
    // window, new size). Scrollback is lost when switching away from a
    // window, same trade-off as before.
    terminal_widget: Option<(TerminalId, TerminalWidget)>,
    input_mode: InputMode,
    status: String,
    should_quit: bool,
    // Last (rows, cols) we told the server about, so we don't spam
    // UpdateScreenSize on every coalesced resize event. Also doubles as
    // the size new/resized terminal widgets get created/resized to.
    last_sent_size: (u16, u16),
}

impl AppState {
    fn new(initial_rows: u16, initial_cols: u16) -> Self {
        Self {
            screen: Screen::SessionList,
            sessions: Vec::new(),
            session_list_index: 0,
            current_session: None,
            current_window_index: 0,
            terminal_widget: None,
            input_mode: InputMode::Normal,
            status: String::new(),
            should_quit: false,
            last_sent_size: (initial_rows, initial_cols),
        }
    }

    fn current_window(&self) -> Option<&WindowInfo> {
        self.current_session
            .as_ref()?
            .windows_info
            .get(self.current_window_index)
    }

    fn current_terminal_id(&self) -> Option<TerminalId> {
        self.terminal_widget.as_ref().map(|(id, _)| *id)
    }

    fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.session_list_index)
    }

    /// Re-point `terminal_widget` at whatever window is now current (or
    /// clear it if there isn't one), creating a fresh widget if the
    /// terminal changed or resizing the existing one in place otherwise.
    /// Call this after anything that can change which window/terminal is
    /// displayed or how big it should be: a fresh session info, the user
    /// switching windows, or a resize.
    ///
    /// The terminal poller reads `current_terminal_id()` to decide which
    /// terminal to request updates for, so this is also what steers it.
    fn sync_terminal(&mut self) {
        let terminal_id = self.current_window().map(|w| w.terminal_id);
        let (rows, cols) = self.last_sent_size;
        match terminal_id {
            Some(id) => match &mut self.terminal_widget {
                Some((current_id, widget)) if *current_id == id => {
                    widget.resize(rows, cols);
                }
                _ => {
                    self.terminal_widget = Some((id, TerminalWidget::new(rows, cols)));
                }
            },
            None => self.terminal_widget = None,
        }
    }

    fn update_screen(&mut self, terminal_id: TerminalId, data: &[u8]) {
        if let Some((id, widget)) = &mut self.terminal_widget {
            if *id == terminal_id {
                widget.update_screen(data);
            }
        }
    }

    /// Apply an authoritative `SessionInfo` for a session we're viewing
    /// (reply to open_session / create_window / kill_window).
    fn apply_session_info(&mut self, session_info: SessionInfo) {
        let len = session_info.windows_info.len();
        self.current_session = Some(session_info);
        self.current_window_index = if len == 0 {
            0
        } else {
            self.current_window_index.min(len - 1)
        };
        self.screen = Screen::Terminal;
        self.status = "Session updated".to_owned();
        // The current window (and therefore its terminal) may have
        // changed, or we may have just switched into a session/window for
        // the first time - reconcile either way.
        self.sync_terminal();
    }

    /// Apply a fresh session list (reply to list_sessions / create_session
    /// / kill_session / exit_session). Refreshes whatever session we're
    /// currently viewing from it: if it's gone, or still there but now
    /// windowless, there's nothing left to show on the terminal screen, so
    /// fall back to the list. Otherwise just pick up the (possibly
    /// changed) window set.
    ///
    /// The list is kept sorted, and the cursor stays on the session it was on, so a
    /// refresh never moves it onto a different session.
    fn apply_session_list(&mut self, mut sessions: Vec<SessionInfo>) {
        let refreshed_current = self.current_session.as_ref().and_then(|current| {
            sessions
                .iter()
                .find(|listed| listed.id == current.id)
                .cloned()
        });

        let selected = self.selected_session().map(|session| session.id);
        sort_sessions(&mut sessions);
        self.session_list_index = reselect(&sessions, selected, self.session_list_index);
        self.sessions = sessions;

        match refreshed_current {
            Some(session) if !session.windows_info.is_empty() => {
                self.current_window_index = self
                    .current_window_index
                    .min(session.windows_info.len() - 1);
                self.current_session = Some(session);
                self.sync_terminal();
            }
            _ if self.current_session.is_some() => {
                self.current_session = None;
                self.current_window_index = 0;
                self.screen = Screen::SessionList;
                self.status = "Session closed".to_owned();
                self.sync_terminal();
            }
            _ => {}
        }
    }

    /// Make the window with this id the current one (if it's in the
    /// current session).
    fn focus_window(&mut self, window_id: WindowId) {
        let Some(session) = self.current_session.as_ref() else {
            return;
        };
        if let Some(idx) = session
            .windows_info
            .iter()
            .position(|w| w.window_id == window_id)
        {
            self.current_window_index = idx;
            self.sync_terminal();
        }
    }
}

fn log_line(log: &mut std::fs::File, msg: impl AsRef<str>) {
    let _ = writeln!(log, "{}", msg.as_ref());
}

/// The vertical split shared by both screens: a 3-row top bar, a flexible
/// middle area, and a 3-row bottom keybind bar.
fn screen_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area)
}

/// Given the *full* terminal area, work out how many rows/cols are
/// actually available for the terminal pane (the middle chunk), which is
/// what the server-side PTY should be sized against. This is deliberately
/// kept in lockstep with `draw_terminal_screen`'s layout below - if that
/// layout changes, update this too.
fn terminal_pane_size(area: Rect) -> (u16, u16) {
    let middle = screen_layout(area)[1];
    (middle.height, middle.width)
}

// ---------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------

async fn connect_and_start(rows: u16, cols: u16) -> Result<DaemonClient> {
    let client = DaemonClient::connect().await?;
    client.start(rows, cols).await?;
    Ok(client)
}

/// Continuously requests updates for whichever terminal is currently
/// visible and feeds them into the widget.
async fn run_terminal_poller(client: Arc<DaemonClient>, state: Arc<Mutex<AppState>>) {
    loop {
        // Never hold the lock across an await.
        let (terminal_id, quit) = {
            let st = state.lock().unwrap();
            (st.current_terminal_id(), st.should_quit)
        };
        if quit {
            break;
        }
        let Some(terminal_id) = terminal_id else {
            tokio::time::sleep(POLL_IDLE).await;
            continue;
        };

        match client.get_wanted_data(terminal_id).await {
            Ok(update) => {
                let empty = update.data.is_empty();
                {
                    // `update_screen` ignores the data if the visible
                    // terminal changed while the request was in flight.
                    let mut st = state.lock().unwrap();
                    st.update_screen(update.terminal_id, update.data.as_slice());
                }
                if empty {
                    tokio::time::sleep(POLL_EMPTY_BACKOFF).await;
                }
            }
            Err(e) => {
                state.lock().unwrap().status = format!("Error: {e}");
                tokio::time::sleep(POLL_ERROR_BACKOFF).await;
            }
        }
    }
}

/// Executes queued commands one at a time, in order, and applies their
/// results to the shared state.
async fn run_worker(
    client: Arc<DaemonClient>,
    state: Arc<Mutex<AppState>>,
    mut rx: UnboundedReceiver<Command>,
) {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("frontend_worker.log")
        .ok();

    while let Some(cmd) = rx.recv().await {
        if let Err(e) = handle_command(&client, &state, cmd).await {
            if let Some(log) = log.as_mut() {
                log_line(log, format!("command error: {e:?}"));
            }
            // Show errors in the status line rather than eprint!, which
            // would scribble over the TUI.
            state.lock().unwrap().status = format!("Error: {e}");
        }
    }
}

async fn handle_command(
    client: &DaemonClient,
    state: &Mutex<AppState>,
    cmd: Command,
) -> Result<()> {
    match cmd {
        Command::ListSessions => {
            let sessions = client.list_sessions().await?;
            state.lock().unwrap().apply_session_list(sessions);
        }

        Command::CreateSession { working_dir, name } => {
            let before: Vec<SessionId> =
                state.lock().unwrap().sessions.iter().map(|s| s.id).collect();
            let sessions = client.create_session(working_dir, None, Some(name)).await?;
            // The RPC only returns the list: the new session is the one that was not there before.
            let newest = new_session(&before, &sessions);
            state.lock().unwrap().apply_session_list(sessions);
            if let Some(id) = newest {
                let info = client.open_session(id).await?;
                state.lock().unwrap().apply_session_info(info);
            }
        }

        Command::OpenSession { id } => {
            let info = client.open_session(id).await?;
            state.lock().unwrap().apply_session_info(info);
        }

        Command::KillSession { id } => {
            let sessions = client.kill_session(id).await?;
            state.lock().unwrap().apply_session_list(sessions);
        }

        Command::ExitSession => {
            let sessions = client.exit_session().await?;
            state.lock().unwrap().apply_session_list(sessions);
        }

        Command::CreateWindow { id } => {
            let info = client.create_window(id).await?;
            state.lock().unwrap().apply_session_info(info);
        }

        Command::KillWindow {
            id,
            window_id,
            focus_after,
        } => {
            let info = client.kill_window(id, window_id).await?;
            if let Some(next) = focus_after {
                client.change_window(next).await?;
            }
            if info.windows_info.is_empty() {
                // Last window gone: the session is closed, refresh the
                // list (which also drops us back to the session list).
                let sessions = client.list_sessions().await?;
                state.lock().unwrap().apply_session_list(sessions);
            } else {
                let mut st = state.lock().unwrap();
                st.apply_session_info(info);
                if let Some(next) = focus_after {
                    st.focus_window(next);
                }
            }
        }

        Command::ChangeWindow { window_id } => {
            client.change_window(window_id).await?;
        }

        Command::WriteTerminal { terminal_id, data } => {
            client.write_terminal(terminal_id, data).await?;
        }

        Command::UpdateScreenSize { rows, cols } => {
            client.update_screen_size(rows, cols).await?;
        }

        Command::ExportSessionToSysd { id } => {
            client.export_session_to_sysd(id).await?;
            state.lock().unwrap().status = "Done".to_owned();
        }

        Command::ExportSessionToFile { id, working_dir } => {
            let path = client.export_session_to_file(id, working_dir).await?;
            state.lock().unwrap().status = format!("Exported to {}", path.display());
        }

        Command::SysdAction { id, name, action } => {
            // The reply only says systemd accepted the job; the state the list shows catches
            // up as the unit actually starts or stops (see `run_list_refresher`).
            let sessions = client.execute_sysd_action(id, action).await?;
            let mut st = state.lock().unwrap();
            st.apply_session_list(sessions);
            st.status = format!("{} service '{name}'", action_done(action));
        }
    }
    Ok(())
}

/// Refreshes the session list every `LIST_REFRESH_INTERVAL` until the UI quits.
///
/// Goes through the worker queue like every other request, so a refresh can never
/// overtake (or be overtaken by) an action the user just took.
async fn run_list_refresher(state: Arc<Mutex<AppState>>, tx: UnboundedSender<Command>) {
    let mut interval = tokio::time::interval(LIST_REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if state.lock().unwrap().should_quit || tx.send(Command::ListSessions).is_err() {
            break;
        }
    }
}

/// Queue raw bytes for the currently visible terminal (no-op if none).
fn write_terminal(st: &AppState, tx: &UnboundedSender<Command>, data: Vec<u8>) {
    if let Some(terminal_id) = st.current_terminal_id() {
        let _ = tx.send(Command::WriteTerminal { terminal_id, data });
    }
}

// ---------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------

/// Must be called from within a tokio runtime (e.g. `#[tokio::main]`). The
/// blocking crossterm UI loop runs on `spawn_blocking`, so it never stalls
/// the runtime's workers.
pub async fn run_frontend() -> Result<()> {
    // The directory the frontend was launched from. Used as the target
    // directory for "export session to file". Captured up front, before
    // we touch the terminal, so a failure here can't leave it in raw mode.
    let start_dir = std::env::current_dir()?;

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("frontend.log")?;
    log_line(&mut log, "Starting Frontend");

    // Figure out the real terminal-pane size *before* we say Start, so the
    // server sizes the very first PTY correctly instead of us resizing it
    // a frame later.
    let (start_rows, start_cols) = terminal
        .size()
        .map(|size| terminal_pane_size(Rect::new(0, 0, size.width, size.height)))
        .unwrap_or((FALLBACK_ROWS, FALLBACK_COLS));
    let (start_rows, start_cols) = (start_rows.max(1), start_cols.max(1));

    // Connect before entering raw mode / the alternate screen so a
    // connection failure prints a normal error instead of leaving the
    // terminal in a broken state.
    let client = Arc::new(connect_and_start(start_rows, start_cols).await?);
    log_line(&mut log, "Connected to daemon");

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    // EnterAlternateScreen doesn't guarantee the buffer is actually blank
    // (depends on the terminal emulator, and a prior crashed run can leave
    // stale content behind). Force a full clear + redraw so the session
    // list never shows leftover terminal output underneath it.
    terminal.clear()?;

    let state = Arc::new(Mutex::new(AppState::new(start_rows, start_cols)));
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();

    tokio::spawn(run_worker(client.clone(), state.clone(), cmd_rx));
    tokio::spawn(run_terminal_poller(client.clone(), state.clone()));
    // The first tick fires immediately, which also loads the list on start.
    tokio::spawn(run_list_refresher(state.clone(), cmd_tx.clone()));

    // -------- draw / input loop (blocking, on its own thread) --------
    let ui_state = state.clone();
    let ui_result = tokio::task::spawn_blocking(move || -> Result<()> {
        let state = ui_state;
        let send = |c: Command| {
            let _ = cmd_tx.send(c);
        };
        loop {
            {
                let st = state.lock().unwrap();
                if st.should_quit {
                    break;
                }
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        match st.screen {
                            Screen::SessionList => draw_session_list(frame, area, &st),
                            Screen::Terminal => draw_terminal_screen(frame, area, &st),
                        }
                    })
                    .inspect_err(|e| eprintln!("terminal draw failed: {e}"))
                    .ok();
            }

            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    // The terminal emulator/window was resized. `Event::Resize`
                    // gives us the new *full* terminal size in (cols, rows);
                    // work out how much of that the terminal pane actually
                    // gets, and tell the server so it can resize the PTY.
                    Event::Resize(cols, rows) => {
                        let (pane_rows, pane_cols) =
                            terminal_pane_size(Rect::new(0, 0, cols, rows));
                        let (pane_rows, pane_cols) = (pane_rows.max(1), pane_cols.max(1));

                        let mut st = state.lock().unwrap();
                        if st.last_sent_size != (pane_rows, pane_cols) {
                            st.last_sent_size = (pane_rows, pane_cols);
                            st.sync_terminal();
                            send(Command::UpdateScreenSize {
                                rows: pane_rows,
                                cols: pane_cols,
                            });
                        }
                    }

                    Event::Key(key) => {
                        let mut st = state.lock().unwrap();

                        // Text-entry mode for naming a new session takes
                        // priority.
                        if let InputMode::NewSessionName(ref mut text) = st.input_mode {
                            match key.code {
                                KeyCode::Esc => {
                                    st.input_mode = InputMode::Normal;
                                }
                                KeyCode::Enter => {
                                    let name = std::mem::take(text);
                                    st.input_mode = InputMode::Normal;
                                    if !name.is_empty() {
                                        let working_dir = std::env::var("HOME")
                                            .unwrap_or_else(|_| "/tmp".to_owned());
                                        send(Command::CreateSession { working_dir, name });
                                    }
                                }
                                KeyCode::Backspace => {
                                    text.pop();
                                }
                                KeyCode::Char(c) => {
                                    text.push(c);
                                }
                                _ => {}
                            }
                            continue;
                        }

                        // We just saw Ctrl-b; this key is the tmux-style
                        // command, not terminal input. One-shot: reset to
                        // Normal up front.
                        if matches!(st.input_mode, InputMode::PrefixPending) {
                            st.input_mode = InputMode::Normal;
                            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

                            match key.code {
                                // Ctrl-b Ctrl-b: send a literal Ctrl-b through
                                // to the terminal instead of treating it as a
                                // command.
                                KeyCode::Char('b') if ctrl => {
                                    write_terminal(&st, &cmd_tx, vec![0x02]);
                                }
                                // c: new window
                                KeyCode::Char('c') => {
                                    if let Some(session) = st.current_session.as_ref() {
                                        send(Command::CreateWindow { id: session.id });
                                    }
                                }
                                // n: next window
                                KeyCode::Char('n') => {
                                    let len = st
                                        .current_session
                                        .as_ref()
                                        .map(|s| s.windows_info.len())
                                        .unwrap_or(0);
                                    if len > 0 {
                                        st.current_window_index =
                                            (st.current_window_index + 1) % len;
                                        // We already know this window's
                                        // terminal_id locally, so swap the
                                        // widget over immediately rather than
                                        // waiting on a round trip; the poller
                                        // then starts requesting its data.
                                        st.sync_terminal();
                                        if let Some(w) = st.current_window() {
                                            send(Command::ChangeWindow {
                                                window_id: w.window_id,
                                            });
                                        }
                                    }
                                }
                                // p: previous window
                                KeyCode::Char('p') => {
                                    let len = st
                                        .current_session
                                        .as_ref()
                                        .map(|s| s.windows_info.len())
                                        .unwrap_or(0);
                                    if len > 0 {
                                        st.current_window_index = st
                                            .current_window_index
                                            .checked_sub(1)
                                            .unwrap_or(len - 1);
                                        st.sync_terminal();
                                        if let Some(w) = st.current_window() {
                                            send(Command::ChangeWindow {
                                                window_id: w.window_id,
                                            });
                                        }
                                    }
                                }
                                // x: kill the current window. If it's not the
                                // last window in the session, pick a neighbor
                                // to focus afterward; the worker sends
                                // ChangeWindow for it after the kill and then
                                // re-resolves its index in the new window
                                // list. If it *is* the last window, killing it
                                // closes the session and the worker falls
                                // back to the session list.
                                KeyCode::Char('x') => {
                                    if let Some(session) = st.current_session.as_ref() {
                                        let idx = st.current_window_index;
                                        let len = session.windows_info.len();
                                        if let Some(window) = session.windows_info.get(idx) {
                                            let focus_after = if len > 1 {
                                                let next_index =
                                                    if idx + 1 < len { idx + 1 } else { idx - 1 };
                                                Some(session.windows_info[next_index].window_id)
                                            } else {
                                                None
                                            };
                                            send(Command::KillWindow {
                                                id: session.id,
                                                window_id: window.window_id,
                                                focus_after,
                                            });
                                        }
                                    }
                                }
                                // d: detach (leave the session, back to the
                                // session list)
                                KeyCode::Char('d') => {
                                    send(Command::ExitSession);
                                    st.screen = Screen::SessionList;
                                    st.current_session = None;
                                    st.current_window_index = 0;
                                    st.sync_terminal();
                                }
                                // Anything else: not implemented - just drop
                                // back to normal mode, same as tmux ignoring
                                // an unbound key after the prefix.
                                _ => {}
                            }
                            continue;
                        }

                        match st.screen {
                            Screen::SessionList => match key.code {
                                KeyCode::Char('q') => {
                                    st.should_quit = true;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if !st.sessions.is_empty() {
                                        st.session_list_index = st
                                            .session_list_index
                                            .checked_sub(1)
                                            .unwrap_or(st.sessions.len() - 1);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if !st.sessions.is_empty() {
                                        st.session_list_index =
                                            (st.session_list_index + 1) % st.sessions.len();
                                    }
                                }
                                KeyCode::Char('n') => {
                                    st.input_mode = InputMode::NewSessionName(String::new());
                                }
                                KeyCode::Enter | KeyCode::Char('o') => {
                                    match st.selected_session() {
                                        // A stopped service has no windows to show.
                                        Some(SessionInfo {
                                            name,
                                            sysd_data: Some(SysdData { running: false, .. }),
                                            ..
                                        }) => {
                                            st.status = format!(
                                                "Service '{name}' is stopped, press s to start it"
                                            );
                                        }
                                        Some(session) => {
                                            send(Command::OpenSession { id: session.id });
                                        }
                                        None => {}
                                    }
                                }
                                KeyCode::Char('x') | KeyCode::Char('d') => {
                                    if let Some(session) = st.selected_session() {
                                        // kill_session already returns the
                                        // refreshed list, no extra
                                        // ListSessions needed.
                                        send(Command::KillSession { id: session.id });
                                    }
                                }
                                // e: turn the selected (non-service) session into
                                // a systemd service.
                                KeyCode::Char('e') => match st.selected_session() {
                                    Some(session) if session.sysd_data.is_some() => {
                                        st.status = format!(
                                            "'{}' already is a systemd service",
                                            session.name
                                        );
                                    }
                                    Some(session) => {
                                        let id = session.id;
                                        st.status = "Exporting session to systemd...".to_owned();
                                        send(Command::ExportSessionToSysd { id });
                                    }
                                    None => {}
                                },
                                // s / R / a: systemd operations. Only systemd
                                // sessions offer them, and only the ones that fit
                                // the state on screen (see `service_keys`).
                                KeyCode::Char(c) if is_service_key(c) => {
                                    let Some(session) = st.selected_session() else {
                                        continue;
                                    };
                                    match service_action_for_key(session, c) {
                                        Some(action) => {
                                            let (id, name) = (session.id, session.name.clone());
                                            st.status = format!(
                                                "{} service '{name}'...",
                                                action_in_progress(action)
                                            );
                                            send(Command::SysdAction { id, name, action });
                                        }
                                        None if session.sysd_data.is_none() => {
                                            st.status = format!(
                                                "'{}' is not a systemd service (press e to turn it into one)",
                                                session.name
                                            );
                                        }
                                        // A service key that does not apply in this
                                        // state, like restart on a stopped service.
                                        None => {}
                                    }
                                }
                                // f: export the selected session to a file in
                                // the directory the frontend was started in.
                                KeyCode::Char('f') => {
                                    if let Some(session) = st.selected_session() {
                                        let id = session.id;
                                        st.status = "Exporting session to file...".to_owned();
                                        send(Command::ExportSessionToFile {
                                            id,
                                            working_dir: start_dir.clone(),
                                        });
                                    }
                                }
                                KeyCode::Char('r') => {
                                    send(Command::ListSessions);
                                }
                                _ => {}
                            },

                            Screen::Terminal => {
                                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                                match key.code {
                                    // tmux-style prefix: the next keypress is a
                                    // command (see the PrefixPending handling
                                    // above), not terminal input.
                                    KeyCode::Char('b') if ctrl => {
                                        st.input_mode = InputMode::PrefixPending;
                                    }

                                    // Ctrl-c: interrupt (SIGINT via the
                                    // terminal driver), Ctrl-v: literal-next,
                                    // Ctrl-r: reverse history search. These
                                    // need to be forwarded as raw control
                                    // bytes rather than falling through to
                                    // the generic KeyCode::Char(c) handler
                                    // below, since that path doesn't know
                                    // about the Ctrl modifier.
                                    KeyCode::Char('c') if ctrl => {
                                        write_terminal(&st, &cmd_tx, vec![0x03]);
                                    }
                                    KeyCode::Char('v') if ctrl => {
                                        write_terminal(&st, &cmd_tx, vec![0x16]);
                                    }
                                    KeyCode::Char('r') if ctrl => {
                                        write_terminal(&st, &cmd_tx, vec![0x12]);
                                    }
                                    // 1-9: jump directly to the window at that
                                    // position (frontend-only window_nr - just
                                    // the 1-based index into windows_info, not
                                    // known to the server).
                                    KeyCode::Char(c) if c.is_ascii_digit() && !ctrl => {
                                        if let Some(digit) = c.to_digit(10) {
                                            if digit > 0 {
                                                let idx = digit as usize - 1;
                                                let window_id = st
                                                    .current_session
                                                    .as_ref()
                                                    .and_then(|s| s.windows_info.get(idx))
                                                    .map(|w| w.window_id);
                                                if let Some(window_id) = window_id {
                                                    st.current_window_index = idx;
                                                    st.sync_terminal();
                                                    send(Command::ChangeWindow { window_id });
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Enter => write_terminal(&st, &cmd_tx, b"\r".to_vec()),
                                    KeyCode::Backspace => write_terminal(&st, &cmd_tx, vec![0x7f]),
                                    KeyCode::Left => {
                                        write_terminal(&st, &cmd_tx, b"\x1b[D".to_vec())
                                    }
                                    KeyCode::Right => {
                                        write_terminal(&st, &cmd_tx, b"\x1b[C".to_vec())
                                    }
                                    KeyCode::Up => write_terminal(&st, &cmd_tx, b"\x1b[A".to_vec()),
                                    KeyCode::Down => {
                                        write_terminal(&st, &cmd_tx, b"\x1b[B".to_vec())
                                    }
                                    // Tab / Shift-Tab: needed for bash's
                                    // completion (single Tab, or cycling
                                    // through a completion menu with
                                    // Shift-Tab). Crossterm reports these as
                                    // their own KeyCodes rather than
                                    // Char('\t'), so they need explicit
                                    // handling or they'd otherwise be
                                    // silently dropped by the catch-all below.
                                    KeyCode::Tab => write_terminal(&st, &cmd_tx, vec![0x09]),
                                    KeyCode::BackTab => {
                                        write_terminal(&st, &cmd_tx, b"\x1b[Z".to_vec())
                                    }
                                    KeyCode::Char(c) => {
                                        let mut buf = [0u8; 4];
                                        let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
                                        write_terminal(&st, &cmd_tx, bytes);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        Ok(())
    })
    .await;

    // Always restore the terminal, even if the UI loop failed or panicked.
    disable_raw_mode()?;
    execute!(std::io::stdout(), Show, LeaveAlternateScreen)?;
    ui_result??;
    Ok(())
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

/// The session list's layout: like `screen_layout`, but with a two-line keybind bar
/// (actions for the selected session, then the general ones).
fn session_list_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .split(area)
}

/// A key in a keybind bar, followed by what it does.
fn keybind(key: impl Into<String>, label: impl Into<String>) -> [Span<'static>; 2] {
    [
        Span::styled(
            format!(" {} ", key.into()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("{}  ", label.into())),
    ]
}

/// "● running" / "○ stopped" and "autostart on" / "autostart off", coloured.
fn service_state_spans(sysd: SysdData) -> [Span<'static>; 2] {
    let running = if sysd.running {
        Span::styled("● running", Style::default().fg(Color::Green))
    } else {
        Span::styled("○ stopped", Style::default().fg(Color::DarkGray))
    };
    let autostart = if sysd.enabled {
        Span::styled("autostart on", Style::default().fg(Color::Cyan))
    } else {
        Span::styled("autostart off", Style::default().fg(Color::DarkGray))
    };
    [running, autostart]
}

/// One row of the session list: name, kind and state, window count.
fn session_row(session: &SessionInfo, name_width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(format!("{:<name_width$}  ", session.name))];

    match session.sysd_data {
        Some(sysd) => {
            let [running, autostart] = service_state_spans(sysd);
            spans.push(Span::styled("service  ", Style::default().fg(Color::Magenta)));
            spans.push(running);
            spans.push(Span::raw("  "));
            spans.push(autostart);
        }
        None => spans.push(Span::styled("session", Style::default().fg(Color::Blue))),
    }

    // A stopped service has no windows; saying "0 windows" there would just be noise.
    let windows = session.windows_info.len();
    if windows > 0 {
        let plural = if windows == 1 { "" } else { "s" };
        spans.push(Span::styled(
            format!("  {windows} window{plural}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

/// The keys that apply to the selected session. Service operations only appear for
/// systemd sessions, and only those that fit the state shown (see `service_keys`).
fn selected_session_keybinds(session: Option<&SessionInfo>) -> Line<'static> {
    let Some(session) = session else {
        return Line::from(Span::styled(
            " No sessions yet",
            Style::default().fg(Color::DarkGray),
        ));
    };

    let mut spans = Vec::new();
    match session.sysd_data {
        Some(sysd) => {
            if sysd.running {
                spans.extend(keybind("o/Enter", "open"));
            }
            for service_key in service_keys(sysd) {
                spans.extend(keybind(service_key.key, service_key.label));
            }
        }
        None => {
            spans.extend(keybind("o/Enter", "open"));
            spans.extend(keybind("e", "make service"));
        }
    }
    spans.extend(keybind("f", "export file"));
    spans.extend(keybind("x/d", "kill"));
    Line::from(spans)
}

fn draw_session_list(frame: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let chunks = session_list_layout(area);

    // The status is shown in the title bar so export results (and errors)
    // are actually visible.
    let title_text = if st.status.is_empty() {
        "Sessions".to_owned()
    } else {
        format!("Sessions  -  {}", st.status)
    };
    let title = Paragraph::new(title_text).block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    if let InputMode::NewSessionName(text) = &st.input_mode {
        let input = Paragraph::new(format!("New session name: {text}_"))
            .block(Block::default().borders(Borders::ALL).title("New Session"));
        frame.render_widget(input, chunks[1]);
    } else {
        let name_width = st
            .sessions
            .iter()
            .map(|s| s.name.chars().count())
            .max()
            .unwrap_or(0);
        let items: Vec<ListItem> = st
            .sessions
            .iter()
            .map(|s| ListItem::new(session_row(s, name_width)))
            .collect();

        let mut list_state = ListState::default();
        if !st.sessions.is_empty() {
            list_state.select(Some(st.session_list_index));
        }

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Sessions"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, chunks[1], &mut list_state);
    }

    let general = Line::from(
        [
            keybind("n", "new"),
            keybind("j/k", "move"),
            keybind("r", "refresh"),
            keybind("q", "quit"),
        ]
        .concat(),
    );
    let bar = Paragraph::new(vec![selected_session_keybinds(st.selected_session()), general])
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(bar, chunks[2]);
}

fn draw_terminal_screen(frame: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let chunks = screen_layout(area);

    // Top bar: windows as a horizontal tab list. Windows no longer carry
    // a name, so just label them by id.
    let titles: Vec<Line> = st
        .current_session
        .as_ref()
        .map(|s| {
            s.windows_info
                .iter()
                .enumerate()
                .map(|(i, _w)| Line::from(format!("{}", i + 1)))
                .collect()
        })
        .unwrap_or_default();

    let mut title = vec![Span::raw(format!(
        "Session: {} ",
        st.current_session.as_ref().map_or("", |s| s.name.as_str())
    ))];
    if let Some(sysd) = st.current_session.as_ref().and_then(|s| s.sysd_data) {
        let [running, autostart] = service_state_spans(sysd);
        title.extend([Span::raw("[service "), running, Span::raw(", "), autostart, Span::raw("] ")]);
    }

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(title)),
        )
        .select(st.current_window_index)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Middle: the current window's single terminal.
    if let Some((_, widget)) = &st.terminal_widget {
        widget.render(chunks[1], frame.buffer_mut());
    }

    // Bottom bar: a small "Ctrl-b" prefix indicator on the left (tmux's
    // status-bar convention), red normally and green while we're waiting
    // for the command key, plus the keybind legend.
    let prefix_pending = matches!(st.input_mode, InputMode::PrefixPending);
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(11), Constraint::Min(1)])
        .split(chunks[2]);

    let prefix_color = if prefix_pending {
        Color::Green
    } else {
        Color::Red
    };
    let prefix_box = Paragraph::new(Line::from(Span::styled(
        "Ctrl-b",
        Style::default()
            .fg(Color::Black)
            .bg(prefix_color)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(ratatui::layout::Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(prefix_box, bottom_chunks[0]);

    let keybinds = Line::from(vec![
        Span::styled(" c ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("new window  "),
        Span::styled(" n/p ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("next/prev window  "),
        Span::styled(" 1-9 ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("jump to window  "),
        Span::styled(" x ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("kill window  "),
        Span::styled(" d ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("detach"),
    ]);

    let bar = Paragraph::new(keybinds).block(Block::default().borders(Borders::ALL));
    frame.render_widget(bar, bottom_chunks[1]);
}
