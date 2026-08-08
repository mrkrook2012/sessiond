use std::fs::OpenOptions;
use std::io::BufReader;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::Result;
use interprocess::local_socket::prelude::*;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::EnterAlternateScreen;
use ratatui::crossterm::terminal::LeaveAlternateScreen;
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::enable_raw_mode;
use std::sync::Mutex;

use crate::frontend::slop_terrm_widget::TerminalWidget;
use crate::ipc::consts::socket_name;
use crate::ipc::ipc_connection::IpcConnection;
use crate::ipc::protocol::*;

pub fn run_frontend() -> Result<()> {
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("frontend.log")?;
    writeln!(log, "Starting Frontend")?;
    enable_raw_mode().unwrap();
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let stream = LocalSocketStream::connect(socket_name()).unwrap();
    // Try to bind to socket, if doesn't exist, start daemon.

    let (mut sender, mut reciever) = IpcConnection::new(stream).split().unwrap();
    sender.send(&ClientMessage::Ping).unwrap();
    let response: ServerMessage = reciever.recv().unwrap();
    writeln!(log, "Got response {:?}", response)?;
    sender
        .send(&&ClientMessage::Start {
            rows: 100,
            cols: 180,
        })
        .unwrap();

    sender
        .send(&ClientMessage::CreateSession {
            working_dir: "/home/meindert".to_owned(),
            command: None,
            session_name: None,
        })
        .unwrap();
    let mut terminal_id_glob = 0;
    loop {
        let message: ServerMessage = reciever.recv().unwrap();
        match message {
            ServerMessage::Ok => {}
            ServerMessage::Pong => todo!(),
            ServerMessage::Started => {}
            ServerMessage::CreatedWindow {
                window_id,
                terminal_id,
            } => todo!(),
            ServerMessage::CreatedSession {
                session_info,
                window_id,
                terminal_id,
            } => {
                terminal_id_glob = terminal_id;
                break;
            }
            ServerMessage::ListedSessions { sessions } => todo!(),
            ServerMessage::CreatedTerminal { terminal_id } => todo!(),
            ServerMessage::TerminalData { terminals } => {} // Ignore in cause of out of order
            ServerMessage::OpenedSession {
                window_ids,
                terminal_id,
            } => todo!(),
        }
    }

    writeln!(log, "We got terminal id {}", terminal_id_glob).unwrap();
    let terminal_widget = Arc::new(Mutex::new(TerminalWidget::new(100, 180)));
    let terminal_clone = terminal_widget.clone();
    tokio::task::spawn_blocking(move || {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open("./frontend_recv_tread.out")
            .unwrap();
        let terminal_widget = terminal_clone;
        loop {
            let message: ServerMessage = reciever.recv().unwrap();
            match message {
                ServerMessage::Ok => {}
                ServerMessage::Pong => {}
                ServerMessage::Started => todo!(),
                ServerMessage::CreatedWindow {
                    window_id,
                    terminal_id,
                } => todo!(),
                ServerMessage::CreatedSession {
                    session_info,
                    window_id,
                    terminal_id,
                } => todo!(),
                ServerMessage::ListedSessions { sessions } => todo!(),
                ServerMessage::CreatedTerminal { terminal_id } => todo!(),
                ServerMessage::TerminalData { terminals } => {
                    writeln!(log, "Got something").unwrap();
                    if terminals.len() == 1 {
                        let data = terminals.first().unwrap();
                        terminal_widget
                            .lock()
                            .unwrap()
                            .update_screen(data.data.as_slice());
                    }
                }
                ServerMessage::OpenedSession {
                    window_ids,
                    terminal_id,
                } => todo!(),
            }
        }
    });
    loop {
        terminal
            .draw(|frame| {
                let area = frame.area();
                terminal_widget
                    .lock()
                    .unwrap()
                    .render(area, frame.buffer_mut());
            })
            .inspect_err(|e| eprintln!("terminal draw failed: {e}"))
            .ok();

        if event::poll(Duration::from_millis(16))? {
            writeln!(log, "Got keypress").unwrap();
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,

                    KeyCode::Enter => {
                        let bytes = b"\r";
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    KeyCode::Backspace => {
                        let bytes: &[u8; 1] = &[0x7f];
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    KeyCode::Char(c) => {
                        let mut buf = [0u8; 4];
                        let bytes = c.encode_utf8(&mut buf).as_bytes();
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                        writeln!(log, "Sent key {c}").unwrap();
                    }

                    KeyCode::Left => {
                        let bytes = b"\x1b[D";
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    KeyCode::Right => {
                        let bytes = b"\x1b[C";
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    KeyCode::Up => {
                        let bytes = b"\x1b[A";
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    KeyCode::Down => {
                        let bytes = b"\x1b[B";
                        let vec = bytes.to_vec();
                        sender
                            .send(&ClientMessage::WriteTerminal {
                                terminal_id: terminal_id_glob,
                                data: vec,
                            })
                            .unwrap();
                    }

                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?;
    execute!(std::io::stdout(), Show, LeaveAlternateScreen)?;
    Ok(())
}
