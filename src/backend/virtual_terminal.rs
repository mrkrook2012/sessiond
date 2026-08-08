use portable_pty::{CommandBuilder, PtyPair, PtySize, native_pty_system};
use std::sync::Arc;
use std::sync::Mutex;
use vt100::Parser;

pub struct VirtualTerminal {
    parser: Arc<Mutex<Parser>>,
    pair: PtyPair, // master and slave of Pty
    writer: Box<dyn std::io::Write + Send>,
}

impl VirtualTerminal {
    pub fn new(id: u64, rows: u16, cols: u16) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows,
                cols: cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();
        pair.slave
            .spawn_command(CommandBuilder::new("/bin/bash"))
            .unwrap();
        let writer = pair.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(Parser::new(rows, cols, 0)));

        // TODO
        // Temporary workaround, this should be owned seperately
        let reader = pair.master.try_clone_reader().unwrap();
        let parser2 = parser.clone();
        tokio::task::spawn_blocking(move || {
            let parser = parser2;
            let mut reader = reader;
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser.lock().unwrap().process(&buffer[..n]);
                    }
                    Err(e) => {
                        eprintln!("PTY read error: {e}");
                        break;
                    }
                }
            }
        });

        Self {
            parser: parser,
            pair: pair,
            writer: writer,
        }
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

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
        self.pair
            .master
            .resize(PtySize {
                rows: rows,
                cols: cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();
    }
}
