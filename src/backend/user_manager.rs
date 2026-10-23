use std::collections::HashMap;

use mk_utils::AssignOps;

use crate::backend::terminal_manager::TerminalId;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, AssignOps)]
pub struct UserId(u64);

#[derive(Debug)]
pub enum UserManagerError {
    UserNotFound(UserId),
    UserAlreadyExists(UserId),
    TerminalAlreadyVisible(TerminalId),
    TerminalNotVisible(TerminalId),
}

pub struct UserManager {
    user_ids: Vec<UserId>,
    visible_terminals: HashMap<UserId, Vec<TerminalId>>, // user_id -> terminal_ids
    screen_sizes: HashMap<UserId, (u16, u16)>,
}

impl UserManager {
    pub fn new() -> Self {
        Self {
            user_ids: Vec::new(),
            visible_terminals: HashMap::new(),
            screen_sizes: HashMap::new(),
        }
    }

    pub fn add_user(&mut self, user_id: UserId) -> Result<(), UserManagerError> {
        if self.user_ids.contains(&user_id) {
            return Err(UserManagerError::UserAlreadyExists(user_id));
        }

        self.user_ids.push(user_id);
        Ok(())
    }

    pub fn remove_user(&mut self, user_id: UserId) -> Result<(), UserManagerError> {
        // TODO User id stuff
        let Some(index) = self.user_ids.iter().position(|&id| id == user_id) else {
            return Err(UserManagerError::UserNotFound(user_id));
        };

        self.user_ids.remove(index);
        self.visible_terminals.remove(&user_id);
        self.screen_sizes.remove(&user_id);

        Ok(())
    }

    pub fn has_user(&self, user_id: UserId) -> bool {
        self.user_ids.contains(&user_id)
    }

    pub fn users(&self) -> &[UserId] {
        &self.user_ids
    }

    pub fn add_visible_terminal(
        &mut self,
        user_id: UserId,
        terminal_id: TerminalId,
    ) -> Result<(), UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        let terminals = self.visible_terminals.entry(user_id).or_default();

        if terminals.contains(&terminal_id) {
            return Err(UserManagerError::TerminalAlreadyVisible(terminal_id));
        }

        terminals.push(terminal_id);

        Ok(())
    }

    pub fn remove_visible_terminal(
        &mut self,
        user_id: UserId,
        terminal_id: TerminalId,
    ) -> Result<(), UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        let terminals = self.visible_terminals.get_mut(&user_id).unwrap();

        let Some(index) = terminals.iter().position(|&id| id == terminal_id) else {
            return Err(UserManagerError::TerminalNotVisible(terminal_id));
        };

        terminals.remove(index);

        Ok(())
    }

    pub fn clear_terminals(&mut self) {
        self.visible_terminals.clear();
    }

    pub fn get_visible_terminals(&self, user_id: UserId) -> Result<&[TerminalId], UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        Ok(self
            .visible_terminals
            .get(&user_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]))
    }

    pub fn set_screen_size(
        &mut self,
        user_id: UserId,
        width: u16,
        height: u16,
    ) -> Result<(), UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        self.screen_sizes.insert(user_id, (width, height));

        Ok(())
    }

    pub fn get_screen_size(&self, user_id: UserId) -> Result<Option<(u16, u16)>, UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        Ok(self.screen_sizes.get(&user_id).copied())
    }

    pub fn terminal_visible_to_user(
        &self,
        user_id: UserId,
        terminal_id: TerminalId,
    ) -> Result<bool, UserManagerError> {
        if !self.has_user(user_id) {
            return Err(UserManagerError::UserNotFound(user_id));
        }

        Ok(self
            .visible_terminals
            .get(&user_id)
            .map(|terminals| terminals.contains(&terminal_id))
            .unwrap_or(false))
    }
}
