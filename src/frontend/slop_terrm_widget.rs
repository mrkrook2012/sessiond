use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use vt100::{Color as VtColor, Parser};

pub struct TerminalWidget {
    parser: Parser,
}

impl TerminalWidget {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, 0),
        }
    }

    pub fn update_screen(&mut self, state: &[u8]) {
        self.parser = Parser::new(
            self.parser.screen().size().0,
            self.parser.screen().size().1,
            0,
        );

        self.parser.process(state);
    }

    pub fn render(&self, area: Rect, buffer: &mut Buffer) {
        render_terminal(self.parser.screen(), area, buffer);
    }
}

fn render_terminal(
    screen: &vt100::Screen,
    area: Rect,
    buffer: &mut Buffer,
) {
    let (screen_rows, screen_cols) = screen.size();

    let rows = screen_rows.min(area.height);
    let cols = screen_cols.min(area.width);

    for row in 0..rows {
        for col in 0..cols {
            let Some(vt_cell) = screen.cell(row, col) else {
                continue;
            };

            let Some(tui_cell) =
                buffer.cell_mut((area.x + col, area.y + row))
            else {
                continue;
            };

            if vt_cell.is_wide_continuation() {
                continue;
            }

            let contents = vt_cell.contents();

            tui_cell.set_symbol(if contents.is_empty() {
                " "
            } else {
                contents
            });

            tui_cell.set_style(cell_style(vt_cell));
        }
    }

    // Render cursor as an inverted cell.
    if !screen.hide_cursor() {
        let (cursor_row, cursor_col) = screen.cursor_position();

        if cursor_row < rows && cursor_col < cols {
            let x = area.x + cursor_col;
            let y = area.y + cursor_row;

            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_style(
                    cell.style().add_modifier(Modifier::REVERSED)
                );
            }
        }
    }
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

        VtColor::Idx(index) => {
            Color::Indexed(index)
        }

        VtColor::Rgb(r, g, b) => {
            Color::Rgb(r, g, b)
        }
    }
}