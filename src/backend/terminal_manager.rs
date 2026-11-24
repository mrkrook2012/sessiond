use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use bincode_next::{Decode, Encode};

use mk_utils::AssignOps;
use serde::Serialize;

use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use crate::backend::virtual_terminal::{TerminalEvent, VirtualTerminal};
use crate::ipc::protocol::*;
// If a second person inspects the same session, we will not resize and their frontend will adapt

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, AssignOps, Encode, Decode, Serialize, serde::Deserialize)]
pub struct TerminalId(u64);

impl TerminalId {
    #[cfg(test)]
    pub fn test(id: u64) -> Self {
        TerminalId(id)
    }
}

pub struct TerminalManager {
    last_id: TerminalId,
    free_ids: BinaryHeap<Reverse<TerminalId>>,
    terminals: HashMap<TerminalId, VirtualTerminal>,
    events: UnboundedSender<TerminalEvent>,
}

impl TerminalManager {
    pub fn new(events: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            last_id: TerminalId(0),
            free_ids: BinaryHeap::new(),
            terminals: HashMap::new(),
            events,
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

    pub fn get_data_from_terminal(&self, terminal_id: TerminalId) -> TerminalUpdate {
        let term = self.get_terminal_from_id(terminal_id);
        TerminalUpdate {
            terminal_id: terminal_id,
            data: term.get_screen(),
        }
    }

    pub fn get_command_from_terminal(&self, terminal_id: TerminalId) -> String {
        self.get_terminal_from_id(terminal_id).get_current_command()
    }

    pub fn get_working_dir_from_terminal(&self, terminal_id: TerminalId) -> String {
        self.get_terminal_from_id(terminal_id).get_current_pwd()
    }

    pub fn resize_all_terminals(&mut self, rows: u16, cols: u16) {
        for (_terminal_id, terminal) in &mut self.terminals {
            terminal.set_global_size(rows, cols);
            terminal.recalculate_size();
        }
    }

    pub fn send_bytes_to_terminal(&mut self, terminal_id: TerminalId, bytes: &[u8]) {
        self.get_terminal_from_id_mut(terminal_id)
            .process_input(bytes);
    }

    pub fn execute_command_on_terminal(&mut self, terminal_id: TerminalId, command: String) {
        self.get_terminal_from_id_mut(terminal_id)
            .execute_command(command);
    }

    pub fn run_tracked_on_terminal(&mut self, terminal_id: TerminalId, command: String) {
        self.get_terminal_from_id_mut(terminal_id).run_tracked(command);
    }

    pub fn last_user_input(&self, terminal_id: TerminalId) -> Option<Instant> {
        self.terminals.get(&terminal_id)?.last_user_input()
    }

    pub fn kill_terminal(&mut self, terminal_id: TerminalId) {
        self.free_ids.push(Reverse(terminal_id));
        self.terminals.remove(&terminal_id).unwrap();
    }

    pub fn create_terminal(&mut self, rows: u16, cols: u16) -> TerminalId {
        let id = self.generate_id();
        let terminal = VirtualTerminal::new(id, rows, cols, self.events.clone());
        self.terminals.insert(id, terminal);
        id
    }

    pub fn get_terminal_from_id(&self, terminal_id: TerminalId) -> &VirtualTerminal {
        self.terminals
            .get(&terminal_id)
            .expect("Terminal with id should exist")
    }
    pub fn get_terminal_from_id_mut(&mut self, terminal_id: TerminalId) -> &mut VirtualTerminal {
        self.terminals
            .get_mut(&terminal_id)
            .expect("Terminal with id should exist")
    }

    fn generate_id(&mut self) -> TerminalId {
        if let Some(Reverse(id)) = self.free_ids.pop() {
            id
        } else {
            self.last_id += TerminalId(1);
            self.last_id
        }
    }
}
