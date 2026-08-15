//! Frontend prototype: a session-list screen and a terminal screen.
//!
//! REQUIRED CHANGE to protocol.rs before this compiles:
//! `WindowInfo`'s fields (`window_id`, `window_name`, `layout`) are currently
//! private, so this module (living outside `protocol.rs`) can't read them.
//! Make them `pub`:
//!
//! ```ignore
//! #[derive(Decode, Encode, Debug)]
//! pub struct WindowInfo {
//!     pub window_id: u64,
//!     pub window_name: String,
//!     pub layout: Node,
//! }
//! ```
//!
//! Everything else (SessionInfo's fields) is already `pub`.
//!
//! ANOTHER REQUIRED CHANGE, this time to `slop_terrm_widget.rs`:
//! `TerminalWidget` needs a way to be resized in place when the terminal
//! pane changes size (window resize, or - later - split changes). If it
//! doesn't already have one, add something like:
//!
//! ```ignore
//! impl TerminalWidget {
//!     pub fn resize(&mut self, rows: u16, cols: u16) {
//!         // re-allocate/relayout the internal grid to (rows, cols),
//!         // preserving whatever scrollback/content makes sense.
//!     }
//! }
//! ```
//!
//! Until that exists, the `resize` call below won't compile - comment it
//! out (the `UpdateScreenSize` message will still be sent to the server,
//! which is the half that actually matters for PTY sizing; the local
//! widget will just snap to the new size whenever the next `TerminalData`
//! update arrives, if `update_screen` re-sizes on mismatch).

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
    RenameWindow {
        window_id: u64,
        text: String,
    },
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
    terminal_widget: TerminalWidget,
    input_mode: InputMode,
    status: String,
    should_quit: bool,
    // Last (rows, cols) we told the server about, so we don't spam
    // UpdateScreenSize on every coalesced resize event.
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
            terminal_widget: TerminalWidget::new(initial_rows, initial_cols),
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
        self.current_window()?
            .layout
            .terminal_ids()
            .first()
            .copied()
    }

    fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.session_list_index)
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
/// what the server-side PTY should be sized to. This is deliberately kept
/// in lockstep with `draw_terminal_screen`'s layout below - if that layout
/// changes, update this too.
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
                ServerMessage::Ok => {}
                ServerMessage::Pong => {}
                ServerMessage::Started => {}

                // Covers session creation, session open, and every
                // structural change within a session (window/terminal
                // create/kill/resize) - the server just hands back the
                // authoritative SessionInfo each time.
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
                }

                // Sent for the plain session-list screen, but per the
                // protocol comment it's *also* sent instead of
                // UpdatedSession when a session disappears entirely (e.g.
                // its last window/terminal was killed). So: refresh the
                // list, and if the session we're currently viewing isn't
                // in it anymore, fall back to the list screen.
                ServerMessage::ListedSessions { sessions } => {
                    st.sessions = sessions;
                    if st.session_list_index >= st.sessions.len() {
                        st.session_list_index = st.sessions.len().saturating_sub(1);
                    }

                    let current_still_alive = st
                        .current_session
                        .as_ref()
                        .map(|current| {
                            st.sessions
                                .iter()
                                .any(|listed| listed.session_id == current.session_id)
                        })
                        .unwrap_or(true);

                    if !current_still_alive {
                        st.current_session = None;
                        st.current_window_index = 0;
                        st.screen = Screen::SessionList;
                        st.status = "Session closed".to_owned();
                    }
                }

                ServerMessage::TerminalData { terminals } => {
                    let current_id = st.current_terminal_id();
                    for update in &terminals {
                        if Some(update.terminal_id) == current_id {
                            st.terminal_widget.update_screen(update.data.as_slice());
                        }
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
                        // Requires `TerminalWidget::resize` - see the
                        // module doc comment at the top of this file.
                        st.terminal_widget.resize(pane_rows, pane_cols);
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

                    if let InputMode::RenameWindow {
                        window_id,
                        ref mut text,
                    } = st.input_mode
                    {
                        match key.code {
                            KeyCode::Esc => {
                                st.input_mode = InputMode::Normal;
                            }
                            KeyCode::Enter => {
                                let name = text.clone();
                                st.input_mode = InputMode::Normal;
                                if !name.is_empty() {
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::RenameWindow { window_id, name })
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
                    // Normal up front, then optionally override below
                    // (e.g. rename drops into its own text-entry mode).
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
                            // ,: rename window
                            KeyCode::Char(',') => {
                                if let Some(window) = st.current_window() {
                                    st.input_mode = InputMode::RenameWindow {
                                        window_id: window.window_id,
                                        text: window.window_name.clone(),
                                    };
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
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::ChangeWindow { window_id })
                                        .unwrap();
                                }
                            }
                            // x: close window (we don't have separate
                            // pane/window concepts here, so this is the
                            // closest match to tmux's "close pane/window")
                            KeyCode::Char('x') => {
                                if let Some(window) = st.current_window() {
                                    let window_id = window.window_id;
                                    drop(st);
                                    sender
                                        .send(&ClientMessage::KillWindow { window_id })
                                        .unwrap();
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
                            }
                            // Anything else: not implemented (no window
                            // list, panes/splits, copy mode, or command
                            // prompt in this prototype yet) - just drop
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

    // Top bar: windows as a horizontal tab list.
    let titles: Vec<Line> = st
        .current_session
        .as_ref()
        .map(|s| {
            s.windows_info
                .iter()
                .map(|w| Line::from(w.window_name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let session_name = st
        .current_session
        .as_ref()
        .map(|s| s.session_name.clone())
        .unwrap_or_default();

    if let InputMode::RenameWindow { text, .. } = &st.input_mode {
        let input = Paragraph::new(format!("New window name: {text}_")).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Rename Window"),
        );
        frame.render_widget(input, chunks[0]);
    } else {
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
    }

    // Middle: the terminal itself. This is the same rect `terminal_pane_size`
    // computes the size for - keep the two in sync.
    st.terminal_widget.render(chunks[1], frame.buffer_mut());

    // Bottom bar: a small "Ctrl-b" prefix indicator on the left (tmux's
    // status-bar convention), red normally and green while we're waiting
    // for the command key, followed by the keybind legend.
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
        Span::styled(" , ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("rename window  "),
        Span::styled(" x ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("close window  "),
        Span::styled(" d ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("detach"),
    ]);
    let bar = Paragraph::new(keybinds).block(Block::default().borders(Borders::ALL));
    frame.render_widget(bar, bottom_chunks[1]);
}
