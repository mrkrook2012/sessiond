use std::collections::HashMap;
use bincode_next::{Decode, Encode};

use serde::Serialize;

use std::time::Instant;

use color_eyre::eyre::{Result, eyre};
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

    pub fn get_all_data(&self) -> Vec<TerminalUpdate> {
        self.terminals
            .keys()
            .filter_map(|&id| self.get_data_from_terminal(id))
            .collect()
    }

    /// The live screen of a terminal, or `None` if there is no such terminal.
    pub fn get_data_from_terminal(&self, terminal_id: TerminalId) -> Option<TerminalUpdate> {
        self.snapshot(terminal_id, 0, 0)
    }

    /// The screen scrolled `scrollback` lines back, or no data if unchanged since `since`.
    /// `None` if there is no such terminal.
    pub fn snapshot(&self, terminal_id: TerminalId, scrollback: u32, since: u64) -> Option<TerminalUpdate> {
        let snapshot = self
            .get_terminal_from_id(terminal_id)?
            .snapshot(scrollback as usize, since);
        Some(TerminalUpdate {
            terminal_id,
            data: snapshot.data,
            rows: snapshot.rows,
            cols: snapshot.cols,
            scrollback: snapshot.scrollback.try_into().unwrap_or(u32::MAX),
            alternate_screen: snapshot.alternate_screen,
            generation: snapshot.generation,
        })
    }

    pub fn get_command_from_terminal(&self, terminal_id: TerminalId) -> Option<String> {
        Some(self.get_terminal_from_id(terminal_id)?.get_current_command())
    }

    pub fn get_working_dir_from_terminal(&self, terminal_id: TerminalId) -> Option<String> {
        Some(self.get_terminal_from_id(terminal_id)?.get_current_pwd())
    }

    pub fn resize_all_terminals(&mut self, rows: u16, cols: u16) {
        for terminal in self.terminals.values_mut() {
            terminal.set_global_size(rows, cols);
            terminal.recalculate_size();
        }
    }

    pub fn send_bytes_to_terminal(&mut self, terminal_id: TerminalId, bytes: &[u8]) -> Result<()> {
        self.terminal_mut(terminal_id)?.process_input(bytes);
        Ok(())
    }

    pub fn execute_command_on_terminal(&mut self, terminal_id: TerminalId, command: String) -> Result<()> {
        self.terminal_mut(terminal_id)?.execute_command(command);
        Ok(())
    }

    /// See `VirtualTerminal::launch`.
    pub fn launch_on_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_dir: &str,
        command: Option<String>,
        tracked: bool,
    ) -> Result<()> {
        self.terminal_mut(terminal_id)?
            .launch(working_dir, command, tracked);
        Ok(())
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

    /// Stops a terminal's shell (without blocking). Returns false if there was no such terminal.
    pub fn kill_terminal(&mut self, terminal_id: TerminalId) -> bool {
        self.terminals.remove(&terminal_id).is_some()
    }

    /// Opens a PTY with a shell in it. Fails if the PTY or the shell cannot be created.
    pub fn create_terminal(&mut self, rows: u16, cols: u16) -> Result<TerminalId> {
        let id = TerminalId(self.guids.generate());
        let terminal = VirtualTerminal::new(id, rows, cols, self.events.clone())?;
        self.terminals.insert(id, terminal);
        Ok(id)
    }

    pub fn get_terminal_from_id(&self, terminal_id: TerminalId) -> Option<&VirtualTerminal> {
        self.terminals.get(&terminal_id)
    }

    pub fn get_terminal_from_id_mut(&mut self, terminal_id: TerminalId) -> Option<&mut VirtualTerminal> {
        self.terminals.get_mut(&terminal_id)
    }

    fn terminal_mut(&mut self, terminal_id: TerminalId) -> Result<&mut VirtualTerminal> {
        self.get_terminal_from_id_mut(terminal_id)
            .ok_or_else(|| eyre!("Terminal {terminal_id:?} not found"))
    }
}
