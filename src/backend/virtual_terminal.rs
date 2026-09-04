use portable_pty::{CommandBuilder, PtyPair, PtySize, native_pty_system};
use std::sync::Arc;
use std::sync::Mutex;
use vt100::Parser;

pub struct VirtualTerminal {
    id: u64,
    parser: Arc<Mutex<Parser>>,
    pair: PtyPair, // master and slave of Pty
    writer: Box<dyn std::io::Write + Send>,
    width_factor: f32,
    height_factor: f32,
    global_rows: u16,
    global_cols: u16,
}

impl VirtualTerminal {
    pub fn new(id: u64, global_rows: u16, global_cols: u16) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: global_rows,
                cols: global_cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();
        pair.slave
            .spawn_command(CommandBuilder::new("/bin/bash"))
            .unwrap();
        let writer = pair.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(Parser::new(global_rows, global_cols, 0)));

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
            id: id,
            parser: parser,
            pair: pair,
            writer: writer,
            width_factor: 1.0,
            height_factor: 1.0,
            global_rows,
            global_cols,
        }
    }

    pub fn get_id(&self) -> u64 {
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

    pub fn get_ratio(&self) -> (f32, f32) {
        (self.height_factor, self.width_factor)
    }
    pub fn set_ratio(&mut self, height_factor: f32, width_factor: f32) {
        self.height_factor = height_factor;
        self.width_factor = width_factor;
        self.recalculate_size();
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
                rows: rows,
                cols: cols,
                pixel_height: 0,
                pixel_width: 0,
            })
            .unwrap();
    }
}
