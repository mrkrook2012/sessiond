use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use color_eyre::eyre::Result;
use interprocess::local_socket::prelude::*;
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

use crate::frontend::slop_terrm_widget::TerminalWidget;
use crate::ipc::consts::socket_name;
use crate::ipc::ipc_connection::IpcConnection;
use crate::ipc::protocol::*;

// Fallback size used only if we somehow can't read the real terminal
// dimensions at startup (e.g. `terminal.size()` fails).
const FALLBACK_ROWS: u16 = 100;
const FALLBACK_COLS: u16 = 180;

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

/// Everything the render loop needs, shared with the background receiver
/// thread behind a mutex.
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
    terminal_widget: Option<(u64, TerminalWidget)>,
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

    fn current_terminal_id(&self) -> Option<u64> {
        self.terminal_widget.as_ref().map(|(id, _)| *id)
    }

    fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.session_list_index)
    }

    /// Re-point `terminal_widget` at whatever window is now current (or
    /// clear it if there isn't one), creating a fresh widget if the
    /// terminal changed or resizing the existing one in place otherwise.
    /// Call this after anything that can change which window/terminal is
    /// displayed or how big it should be: a fresh `UpdatedSession`, the
    /// user switching windows, or a resize.
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

    fn update_screen(&mut self, terminal_id: u64, data: &[u8]) {
        if let Some((id, widget)) = &mut self.terminal_widget {
            if *id == terminal_id {
                widget.update_screen(data);
            }
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

pub fn run_frontend() -> Result<()> {
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("frontend.log")?;
    log_line(&mut log, "Starting Frontend");

    enable_raw_mode().unwrap();
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    // EnterAlternateScreen doesn't guarantee the buffer is actually blank
    // (depends on the terminal emulator, and a prior crashed run can leave
    // stale content behind). Force a full clear + redraw so the session
    // list never shows leftover terminal output underneath it.
    terminal.clear()?;

    // Figure out the real terminal-pane size *before* we say Start, so the
    // server sizes the very first PTY correctly instead of us resizing it
    // a frame later.
    let (start_rows, start_cols) = terminal
        .size()
        .map(|size| terminal_pane_size(Rect::new(0, 0, size.width, size.height)))
        .unwrap_or((FALLBACK_ROWS, FALLBACK_COLS));
    let (start_rows, start_cols) = (start_rows.max(1), start_cols.max(1));

    let stream = LocalSocketStream::connect(socket_name()).unwrap();
    let (mut sender, mut receiver) = IpcConnection::new(stream).split().unwrap();

    sender.send(&ClientMessage::Ping).unwrap();
    let response: ServerMessage = receiver.recv().unwrap();
    log_line(&mut log, format!("Got response {:?}", response));

    sender
        .send(&ClientMessage::Start {
            rows: start_rows,
            cols: start_cols,
        })
        .unwrap();

    // Kick things off on the session-list screen.
    sender.send(&ClientMessage::ListSessions).unwrap();

    let state = Arc::new(Mutex::new(AppState::new(start_rows, start_cols)));

    // Background thread: everything coming from the server updates shared
    // state; the draw loop just reads it.
    let state_bg = state.clone();
    std::thread::spawn(move || {
        let state = state_bg;
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open("frontend_recv_thread.out")
            .unwrap();
        loop {
            let message: ServerMessage = match receiver.recv() {
                Ok(m) => m,
                Err(e) => {
                    log_line(&mut log, format!("recv error: {e:?}"));
                    break;
                }
            };
            let mut st = state.lock().unwrap();
            match message {
                ServerMessage::Ok => {
                    st.status = "Done".to_owned();
                }
                ServerMessage::Pong => {}
                ServerMessage::Started => {}

                // Covers session creation, session open, and every
                // structural change within a session (window create/kill)
                // - the server just hands back the authoritative
                // SessionInfo each time.
                ServerMessage::UpdatedSession { session_info } => {
                    let len = session_info.windows_info.len();
                    st.current_session = Some(session_info);
                    st.current_window_index = if len == 0 {
                        0
                    } else {
                        st.current_window_index.min(len - 1)
                    };
                    st.screen = Screen::Terminal;
                    st.status = "Session updated".to_owned();
                    // The current window (and therefore its terminal) may
                    // have changed, or we may have just switched into a
                    // session/window for the first time - reconcile
                    // either way.
                    st.sync_terminal();
                }

                // Sent for the plain session-list screen, but per the
                // protocol comment it's *also* sent instead of
                // UpdatedSession when a session disappears entirely, or
                // when its last window was just killed (leaving it with
                // zero windows). Refresh the list, and refresh whatever
                // session we're currently viewing from it: if it's gone,
                // or still there but now windowless, there's nothing left
                // to show on the terminal screen, so fall back to the
                // list. Otherwise just pick up the (possibly changed)
                // window set.
                ServerMessage::ListedSessions { sessions } => {
                    let refreshed_current = st.current_session.as_ref().and_then(|current| {
                        sessions
                            .iter()
                            .find(|listed| listed.session_id == current.session_id)
                            .cloned()
                    });

                    st.sessions = sessions;
                    if st.session_list_index >= st.sessions.len() {
                        st.session_list_index = st.sessions.len().saturating_sub(1);
                    }

                    match refreshed_current {
                        Some(session) if !session.windows_info.is_empty() => {
                            st.current_window_index =
                                st.current_window_index.min(session.windows_info.len() - 1);
                            st.current_session = Some(session);
                            st.sync_terminal();
                        }
                        _ if st.current_session.is_some() => {
                            st.current_session = None;
                            st.current_window_index = 0;
                            st.screen = Screen::SessionList;
                            st.status = "Session closed".to_owned();
                            st.sync_terminal();
                        }
                        _ => {}
                    }
                }

                ServerMessage::TerminalData { terminals } => {
                    for update in &terminals {
                        st.update_screen(update.terminal_id, update.data.as_slice());
                    }
                }
            }
        }
    });

    // -------- draw / input loop --------
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
                    let (pane_rows, pane_cols) = terminal_pane_size(Rect::new(0, 0, cols, rows));
                    let (pane_rows, pane_cols) = (pane_rows.max(1), pane_cols.max(1));

                    let mut st = state.lock().unwrap();
                    if st.last_sent_size != (pane_rows, pane_cols) {
                        st.last_sent_size = (pane_rows, pane_cols);
                        st.sync_terminal();
                        drop(st);
                        sender
                            .send(&ClientMessage::UpdateScreenSize {
                                rows: pane_rows,
                                cols: pane_cols,
                            })
                            .unwrap();
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
                                let name = text.clone();
                                st.input_mode = InputMode::Normal;
                                if !name.is_empty() {
                                    let working_dir =
                                        std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::CreateSession {
                                            working_dir,
                                            command: None,
                                            session_name: Some(name),
                                        })
                                        .unwrap();
                                    continue;
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
                                if let Some(terminal_id) = st.current_terminal_id() {
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::WriteTerminal {
                                            terminal_id,
                                            data: vec![0x02],
                                        })
                                        .unwrap();
                                }
                            }
                            // c: new window
                            KeyCode::Char('c') => {
                                if let Some(session) = st.current_session.as_ref() {
                                    let session_id = session.session_id;
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::CreateWindow { session_id })
                                        .unwrap();
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
                                    st.current_window_index = (st.current_window_index + 1) % len;
                                    let window_id =
                                        st.current_session.as_ref().unwrap().windows_info
                                            [st.current_window_index]
                                            .window_id;
                                    // We already know this window's
                                    // terminal_id locally (it came in on
                                    // the session's last UpdatedSession),
                                    // so swap the widget over immediately
                                    // rather than waiting on a round trip.
                                    st.sync_terminal();
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::ChangeWindow { window_id })
                                        .unwrap();
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
                                    st.current_window_index =
                                        st.current_window_index.checked_sub(1).unwrap_or(len - 1);
                                    let window_id =
                                        st.current_session.as_ref().unwrap().windows_info
                                            [st.current_window_index]
                                            .window_id;
                                    st.sync_terminal();
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::ChangeWindow { window_id })
                                        .unwrap();
                                }
                            }
                            // x: kill the current window. If it's not
                            // the last window in the session, the server
                            // needs to be told what should become
                            // current afterward - pick a neighbor locally
                            // (mirroring n/p) and send ChangeWindow for
                            // it right after the kill. If it *is* the
                            // last window, killing it closes the session
                            // entirely (we'll get a ListedSessions back),
                            // so there's nothing to change focus to.
                            KeyCode::Char('x') => {
                                if let Some(session) = st.current_session.as_ref() {
                                    let len = session.windows_info.len();
                                    let window_id =
                                        session.windows_info[st.current_window_index].window_id;

                                    let next = if len > 1 {
                                        let next_index = if st.current_window_index + 1 < len {
                                            st.current_window_index + 1
                                        } else {
                                            st.current_window_index - 1
                                        };
                                        Some((
                                            next_index,
                                            session.windows_info[next_index].window_id,
                                        ))
                                    } else {
                                        None
                                    };

                                    if let Some((next_index, _)) = next {
                                        st.current_window_index = next_index;
                                        st.sync_terminal();
                                    }

                                    drop(st);
                                    sender
                                        .send(&ClientMessage::KillWindow { window_id })
                                        .unwrap();
                                    if let Some((_, next_window_id)) = next {
                                        sender
                                            .send(&ClientMessage::ChangeWindow {
                                                window_id: next_window_id,
                                            })
                                            .unwrap();
                                    }
                                }
                            }
                            // d: detach (leave the session, back to the
                            // session list)
                            KeyCode::Char('d') => {
                                drop(st);
                                sender.send(&ClientMessage::ExitSession).unwrap();
                                sender.send(&ClientMessage::ListSessions).unwrap();
                                let mut st = state.lock().unwrap();
                                st.screen = Screen::SessionList;
                                st.current_session = None;
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
                                if let Some(session) = st.selected_session() {
                                    let session_id = session.session_id;
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::OpenSession { session_id })
                                        .unwrap();
                                    continue;
                                }
                            }
                            KeyCode::Char('x') | KeyCode::Char('d') => {
                                if let Some(session) = st.selected_session() {
                                    let session_id = session.session_id;
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::KillSession { session_id })
                                        .unwrap();
                                    sender.send(&ClientMessage::ListSessions).unwrap();
                                    continue;
                                }
                            }
                            // e: export the selected session. The server
                            // is expected to reply with a plain Ok once
                            // the export is written; see the ServerMessage::Ok
                            // arm above for the status update.
                            KeyCode::Char('e') => {
                                if let Some(session) = st.selected_session() {
                                    let session_id = session.session_id;
                                    st.status = "Exporting session...".to_owned();
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::ExportSession { session_id })
                                        .unwrap();
                                    continue;
                                }
                            }
                            KeyCode::Char('r') => {
                                drop(st);
                                sender.send(&ClientMessage::ListSessions).unwrap();
                                continue;
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
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: vec![0x03],
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Char('v') if ctrl => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: vec![0x16],
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Char('r') if ctrl => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: vec![0x12],
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                // 1-9: jump directly to the window at that
                                // position (frontend-only window_nr - just
                                // the 1-based index into windows_info, not
                                // known to the server).
                                KeyCode::Char(c) if c.is_ascii_digit() && !ctrl => {
                                    if let Some(digit) = c.to_digit(10) {
                                        if digit > 0 {
                                            let idx = digit as usize - 1;
                                            if let Some(session) = st.current_session.as_ref() {
                                                if let Some(window) = session.windows_info.get(idx)
                                                {
                                                    let window_id = window.window_id;
                                                    st.current_window_index = idx;
                                                    st.sync_terminal();
                                                    drop(st);
                                                    sender
                                                        .send(&ClientMessage::ChangeWindow {
                                                            window_id,
                                                        })
                                                        .unwrap();
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Enter => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\r".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Backspace => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: vec![0x7f],
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Left => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\x1b[D".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Right => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\x1b[C".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Up => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\x1b[A".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Down => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\x1b[B".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                // Tab / Shift-Tab: needed for bash's
                                // completion (single Tab, or cycling
                                // through a completion menu with
                                // Shift-Tab). Crossterm reports these as
                                // their own KeyCodes rather than
                                // Char('\t'), so they need explicit
                                // handling or they'd otherwise be
                                // silently dropped by the catch-all below.
                                KeyCode::Tab => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: vec![0x09],
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::BackTab => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: b"\x1b[Z".to_vec(),
                                            })
                                            .unwrap();
                                        continue;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if let Some(terminal_id) = st.current_terminal_id() {
                                        let mut buf = [0u8; 4];
                                        let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
                                        drop(st);
                                        sender
                                            .send(&ClientMessage::WriteTerminal {
                                                terminal_id,
                                                data: bytes,
                                            })
                                            .unwrap();
                                        continue;
                                    }
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

    disable_raw_mode()?;
    execute!(std::io::stdout(), Show, LeaveAlternateScreen)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

fn draw_session_list(frame: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let chunks = screen_layout(area);

    let title = Paragraph::new("Sessions").block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    if let InputMode::NewSessionName(text) = &st.input_mode {
        let input = Paragraph::new(format!("New session name: {text}_"))
            .block(Block::default().borders(Borders::ALL).title("New Session"));
        frame.render_widget(input, chunks[1]);
    } else {
        let items: Vec<ListItem> = st
            .sessions
            .iter()
            .map(|s| {
                ListItem::new(format!(
                    "{} (id {}, {} window(s))",
                    s.session_name,
                    s.session_id,
                    s.windows_info.len()
                ))
            })
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

    let keybinds = Line::from(vec![
        Span::styled(" n ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("new  "),
        Span::styled(" o/Enter ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("open  "),
        Span::styled(" x/d ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("kill  "),
        Span::styled(" e ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("export  "),
        Span::styled(" r ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("refresh  "),
        Span::styled(" q ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("quit"),
    ]);
    let bar = Paragraph::new(keybinds).block(Block::default().borders(Borders::ALL));
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

    let session_name = st
        .current_session
        .as_ref()
        .map(|s| s.session_name.clone())
        .unwrap_or_default();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Session: {session_name}")),
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
