use portable_pty::{Child, PtyPair, PtySize, PtySystem, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::sleep;
use std::time::Duration;
use vt100::{Callbacks, Parser};

use crate::backend::terminal_manager::TerminalId;

pub struct Caller {
    current_command: Arc<RwLock<String>>,
    current_pwd: Arc<RwLock<String>>,
}

impl Callbacks for Caller {
    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() != Some(&b"133".as_slice()) {
            return;
        }

        match params.get(1).copied() {
            Some(b"A") => {
                println!("prompt started");
            }

            Some(b"B") => {
                println!("command input started");
            }

            Some(b"C") => {
                if let Some(cmd) = params.get(2).map(|v| String::from_utf8_lossy(v)) {
                    *self.current_command.write().unwrap() = cmd.clone().into_owned();
                    println!("command execution started: {:?}", cmd);
                }
            }

            Some(b"D") => {
                println!("command finished");
            }

            Some(b"P") => {
                if let Some(pwd) = params.get(2).map(|v| String::from_utf8_lossy(v)) {
                    *self.current_pwd.write().unwrap() = pwd.clone().into_owned();
                    println!("command pwd: {:?}", pwd);
                }
            }

            _ => {}
        }
    }
}

pub struct VirtualTerminal {
    id: TerminalId,
    parser: Arc<Mutex<Parser<Caller>>>,
    pair: PtyPair,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    width_factor: f32,
    height_factor: f32,
    global_rows: u16,
    global_cols: u16,
    current_command: Arc<RwLock<String>>,
    current_pwd: Arc<RwLock<String>>,
}

impl VirtualTerminal {
    pub fn new(id: TerminalId, global_rows: u16, global_cols: u16) -> Self {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: global_rows,
                cols: global_cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();

        let child = pair
            .slave
            .spawn_command(portable_pty::CommandBuilder::new("/bin/bash"))
            .unwrap();

        let mut writer = pair.master.take_writer().unwrap();

        let current_command = Arc::new(RwLock::new(String::new()));
        let current_pwd = Arc::new(RwLock::new(String::new()));

        let caller = Caller {
            current_command: Arc::clone(&current_command),
            current_pwd: Arc::clone(&current_pwd),
        };

        let parser = Arc::new(Mutex::new(Parser::new_with_callbacks(
            global_rows,
            global_cols,
            20,
            caller,
        )));

        let reader = pair.master.try_clone_reader().unwrap();
        let parser2 = Arc::clone(&parser);

        tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buffer = [0u8; 8192];

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser2.lock().unwrap().process(&buffer[..n]);
                    }
                    Err(e) => {
                        eprintln!("PTY read error: {e}");
                        break;
                    }
                }
            }
        });

        while parser.lock().unwrap().screen().contents() == "" {
            sleep(Duration::from_millis(20));
        }

        println!("Injecting bash hooks");

        let setup = r#"
        __osc133_command_start() {
            case "$BASH_COMMAND" in
                __osc133_*|printf*|local*|trap*|PROMPT_COMMAND*)
                    return
                    ;;
            esac

            printf '\033]133;P;%s\007' "$PWD"
            printf '\033]133;C;%s\007' "$BASH_COMMAND"
        }

        trap '__osc133_command_start' DEBUG
        "#;

        writer.write_all(setup.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();

        writer.write_all(b"clear\n").unwrap();
        writer.flush().unwrap();

        Self {
            id,
            parser,
            pair,
            child,
            writer,
            width_factor: 1.0,
            height_factor: 1.0,
            global_rows,
            global_cols,
            current_command,
            current_pwd,
        }
    }

    pub fn get_id(&self) -> TerminalId {
        self.id
    }

    pub fn process_input(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
    }

    pub fn execute_command(&mut self, command: String) {
        self.writer.write_all(command.as_bytes()).unwrap();
        self.writer.write_all(b"\r").unwrap();
        self.writer.flush().unwrap();
    }

    pub fn get_screen(&self) -> Vec<u8> {
        self.parser.lock().unwrap().screen().state_formatted()
    }

    pub fn get_current_command(&self) -> String {
        self.current_command.read().unwrap().to_string()
    }

    pub fn get_current_pwd(&self) -> String {
        self.current_pwd.read().unwrap().to_string()
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

        self.pair
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();
    }
}

impl Drop for VirtualTerminal {
    fn drop(&mut self) {
        println!("Dropping VirtualTerminal {:?}", self.id);

        terminate_process_tree(&mut *self.child, &*self.pair.master);

        println!("VirtualTerminal {:?} terminated", self.id);
    }
}

/// Terminates the PTY session and its foreground process group.
///
/// portable-pty creates the shell as a session leader on Unix.
/// Interactive shells may put foreground jobs into separate
/// process groups, so we terminate both.
#[cfg(unix)]
fn terminate_process_tree(child: &mut dyn Child, master: &dyn portable_pty::MasterPty) {
    use std::collections::BTreeSet;
    use std::os::fd::RawFd;

    fn signal_group(pgid: libc::pid_t, signal: libc::c_int) {
        if pgid <= 0 {
            return;
        }

        // Negative PID means signal the entire process group.
        let result = unsafe { libc::kill(-pgid, signal) };

        if result != 0 {
            let error = std::io::Error::last_os_error();

            // ESRCH means the process group has already exited.
            if error.raw_os_error() != Some(libc::ESRCH) {
                eprintln!("Failed to signal process group {pgid}: {error}");
            }
        }
    }

    // The PTY child is the session/process-group leader.
    let leader = master
        .process_group_leader()
        .or_else(|| child.process_id().map(|pid| pid as libc::pid_t));

    let Some(leader) = leader else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };

    // Find the foreground process group attached to the PTY.
    let foreground = master.as_raw_fd().and_then(|fd: RawFd| {
        let pgid = unsafe { libc::tcgetpgrp(fd) };

        if pgid > 0 { Some(pgid) } else { None }
    });

    // Only signal groups belonging to this terminal's session.
    let session_id = unsafe { libc::getsid(leader) };

    let mut groups = BTreeSet::new();
    groups.insert(leader);

    if let Some(pgid) = foreground {
        if pgid > 0 && unsafe { libc::getsid(pgid) } == session_id {
            groups.insert(pgid);
        }
    }

    println!(
        "Terminating PTY session {leader}, process groups: {:?}",
        groups
    );

    eprintln!("before SIGTERM");
    for pgid in &groups {
        eprintln!("SIGTERM pgid={pgid}");
        unsafe { libc::kill(-*pgid, libc::SIGTERM) };
    }

    eprintln!("before grace period");
    std::thread::sleep(Duration::from_millis(300));

    eprintln!("before SIGKILL");
    for pgid in &groups {
        eprintln!("SIGKILL pgid={pgid}");
        unsafe { libc::kill(-*pgid, libc::SIGKILL) };
    }

    eprintln!("before child.wait");
    //let result = child.wait();
    //eprintln!("after child.wait: {result:?}");
}

/// Windows fallback: portable-pty kills the direct child.
/// It does not guarantee termination of all descendants.
#[cfg(not(unix))]
fn terminate_process_tree(child: &mut dyn Child, _master: &dyn portable_pty::MasterPty) {
    if let Err(e) = child.kill() {
        eprintln!("Failed to kill PTY child: {e}");
    }

    if let Err(e) = child.wait() {
        eprintln!("Failed to wait for PTY child: {e}");
    }
}
