use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::num::NonZeroU16;

use vt100::{Color as VtColor, Parser};

use crate::ipc::protocol::TerminalUpdate;

/// Shows one of the daemon's terminals: the last screen it sent, redrawn from scratch.
pub struct TerminalWidget {
    parser: Parser,
    /// Lines scrolled back into the history in the screen shown; 0 is the live screen.
    scrollback: u32,
    alternate_screen: bool,
}

impl TerminalWidget {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(nonzero(rows), nonzero(cols), 0),
            scrollback: 0,
            alternate_screen: false,
        }
    }

    /// Shows the screen in `update`. An empty update means "unchanged": the current
    /// screen stays.
    ///
    /// The screen is parsed at the size the daemon's terminal has, not the pane's: the
    /// data positions everything for that size, and replaying it at another size wraps
    /// every line in the wrong place. If the sizes differ for a moment (the pane was just
    /// resized), the screen is clipped or padded instead.
    pub fn update_screen(&mut self, update: &TerminalUpdate) {
        if update.data.is_empty() {
            return;
        }
        self.parser = Parser::new(nonzero(update.rows), nonzero(update.cols), 0);
        self.parser.process(&update.data);
        self.scrollback = update.scrollback;
        self.alternate_screen = update.alternate_screen;
    }

    /// True if the program asked for application cursor keys (DECCKM).
    pub fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    /// True while a full-screen program (btop, vim, less) has the alternate screen.
    /// It has no scrollback of its own.
    pub fn alternate_screen(&self) -> bool {
        self.alternate_screen
    }

    pub fn scrollback(&self) -> u32 {
        self.scrollback
    }

    pub fn render(&self, area: Rect, buffer: &mut Buffer) {
        // The cursor belongs to the live screen, not to a page of history.
        render_terminal(self.parser.screen(), area, buffer, self.scrollback == 0);
    }
}

fn render_terminal(screen: &vt100::Screen, area: Rect, buffer: &mut Buffer, show_cursor: bool) {
    let (screen_rows, screen_cols) = screen.size();

    let rows = screen_rows.get().min(area.height);
    let cols = screen_cols.get().min(area.width);

    for row in 0..rows {
        for col in 0..cols {
            let Some(vt_cell) = screen.cell(row, col) else {
                continue;
            };

            let Some(tui_cell) = buffer.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };

            if vt_cell.is_wide_continuation() {
                continue;
            }

            let contents = vt_cell.contents();

            tui_cell.set_symbol(if contents.is_empty() { " " } else { contents });

            tui_cell.set_style(cell_style(vt_cell));
        }
    }

    // Render cursor as an inverted cell.
    if show_cursor && !screen.hide_cursor() {
        let (cursor_row, cursor_col) = screen.cursor_position();

        if cursor_row < rows && cursor_col < cols {
            let x = area.x + cursor_col;
            let y = area.y + cursor_row;

            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

/// vt100 sizes are non-zero; a 0 becomes 1.
fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut modifiers = Modifier::empty();

    if cell.bold() {
        modifiers.insert(Modifier::BOLD);
    }

    if cell.dim() {
        modifiers.insert(Modifier::DIM);
    }

    if cell.italic() {
        modifiers.insert(Modifier::ITALIC);
    }

    if cell.underline() {
        modifiers.insert(Modifier::UNDERLINED);
    }

    if cell.inverse() {
        modifiers.insert(Modifier::REVERSED);
    }

    Style::default()
        .fg(vt_color_to_ratatui(cell.fgcolor()))
        .bg(vt_color_to_ratatui(cell.bgcolor()))
        .add_modifier(modifiers)
}

fn vt_color_to_ratatui(color: VtColor) -> Color {
    match color {
        VtColor::Default => Color::Reset,

        VtColor::Idx(index) => Color::Indexed(index),

        VtColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
