use std::collections::HashMap;
use bincode_next::{Decode, Encode};

use serde::Serialize;

use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use crate::backend::guid::{Guid, GuidManager};
use crate::backend::virtual_terminal::{
    TERMINATE_GRACE, TerminalEvent, VirtualTerminal, finish_all,
};
use crate::ipc::protocol::*;
// If a second person inspects the same session, we will not resize and their frontend will adapt

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, Encode, Decode, Serialize, serde::Deserialize)]
pub struct TerminalId(pub Guid);

pub struct TerminalManager {
    guids: GuidManager,
    terminals: HashMap<TerminalId, VirtualTerminal>,
    events: UnboundedSender<TerminalEvent>,
}

impl TerminalManager {
    pub fn new(events: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            guids: GuidManager::new(),
            terminals: HashMap::new(),
            events,
        }
    }

    pub fn get_all_data(&mut self) -> Vec<TerminalUpdate> {
        self.terminals
            .iter()
            .map(|(id, terminal)| TerminalUpdate {
                terminal_id: *id,
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

    /// See `VirtualTerminal::launch`.
    pub fn launch_on_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_dir: &str,
        command: Option<String>,
        tracked: bool,
    ) {
        self.get_terminal_from_id_mut(terminal_id)
            .launch(working_dir, command, tracked);
    }

    pub fn last_user_input(&self, terminal_id: TerminalId) -> Option<Instant> {
        self.terminals.get(&terminal_id)?.last_user_input()
    }

    /// Stops every terminal and waits (at most `TERMINATE_GRACE`, then SIGKILL) until all
    /// their processes are gone. Unlike dropping a terminal, this blocks, so it is safe to
    /// exit the process afterwards.
    pub fn shutdown(&mut self) {
        let terminations = self
            .terminals
            .values_mut()
            .filter_map(VirtualTerminal::begin_termination)
            .collect();
        finish_all(terminations, Instant::now() + TERMINATE_GRACE);
        self.terminals.clear();
    }

    pub fn kill_terminal(&mut self, terminal_id: TerminalId) {
        self.terminals.remove(&terminal_id).unwrap();
    }

    pub fn create_terminal(&mut self, rows: u16, cols: u16) -> TerminalId {
        let id = TerminalId(self.guids.generate());
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
}
