use std::collections::HashMap;

use crate::backend::window::Window;

pub struct WindowManager {
    last_id: u64,
    windows: HashMap<u64, Window>, //window_id
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            last_id: 0,
            windows: HashMap::new(),
        }
    }

    pub fn create_window(&mut self, terminal_id: u64) -> u64 {
        self.last_id += 1;
        let window = Window::new(terminal_id);
        self.windows.insert(self.last_id.clone(), window);
        self.last_id
    }
}
