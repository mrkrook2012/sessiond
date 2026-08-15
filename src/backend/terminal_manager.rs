use std::collections::HashMap;

use crate::backend::virtual_terminal::VirtualTerminal;
use crate::ipc::protocol::*;
// If a second person inspects the same session, we will not resize and their frontend will adapt

pub struct TerminalManager {
    last_id: u64,
    terminals: HashMap<u64, VirtualTerminal>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            last_id: 0,
            terminals: HashMap::new(),
        }
    }

    pub fn get_all_data(&mut self) -> Vec<TerminalUpdate> {
        self.terminals
            .iter()
            .map(|(id, terminal)| TerminalUpdate {
                terminal_id: id.clone(),
                data: terminal.get_screen(),
            })
            .collect()
    }

    pub fn get_data_from_terminal(&self, terminal_id: u64) -> TerminalUpdate {
        let term = self.get_terminal_from_id(terminal_id);
        TerminalUpdate {
            terminal_id,
            data: term.get_screen(),
        }
    }

    pub fn resize_all_terminals(&mut self, rows: u16, cols: u16) {
        for (terminal_id, terminal) in &mut self.terminals {
            terminal.set_global_size(rows, cols);
            terminal.recalculate_size();
        }
    }

    pub fn send_bytes_to_terminal(&mut self, terminal_id: u64, bytes: &[u8]) {
        self.get_terminal_from_id_mut(terminal_id)
            .process_input(bytes);
    }

    pub fn execute_command_on_terminal(&mut self, terminal_id: u64, command: String) {
        self.get_terminal_from_id_mut(terminal_id)
            .execute_command(command);
    }

    pub fn kill_terminal(&mut self, terminal_id: u64) {
        self.terminals.remove(&terminal_id).unwrap();
    }

    pub fn create_terminal(&mut self, rows: u16, cols: u16) -> u64 {
        self.last_id += 1;

        let terminal = VirtualTerminal::new(self.last_id, rows, cols);
        self.terminals.insert(self.last_id, terminal);
        self.last_id
    }

    pub fn get_terminal_from_id(&self, terminal_id: u64) -> &VirtualTerminal {
        self.terminals
            .get(&terminal_id)
            .expect("Terminal with id should exist")
    }
    pub fn get_terminal_from_id_mut(&mut self, terminal_id: u64) -> &mut VirtualTerminal {
        self.terminals
            .get_mut(&terminal_id)
            .expect("Terminal with id should exist")
    }
}
