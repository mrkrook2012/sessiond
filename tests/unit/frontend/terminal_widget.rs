use daemon_test::frontend::slop_terrm_widget::TerminalWidget;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

fn render(widget: &TerminalWidget, area: Rect) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, area.x + area.width, area.y + area.height));
    widget.render(area, &mut buffer);
    buffer
}

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect()
}

#[test]
fn renders_text_at_the_area_offset() {
    let mut widget = TerminalWidget::new(3, 10);
    widget.update_screen(b"hello\r\nworld");

    let buffer = render(&widget, Rect::new(2, 1, 10, 3));
    assert_eq!(row(&buffer, 1), "  hello     ");
    assert_eq!(row(&buffer, 2), "  world     ");
    assert_eq!(row(&buffer, 0).trim(), "");
}

#[test]
fn update_replaces_the_previous_screen() {
    let mut widget = TerminalWidget::new(2, 10);
    widget.update_screen(b"first line");
    widget.update_screen(b"second");

    let buffer = render(&widget, Rect::new(0, 0, 10, 2));
    assert_eq!(row(&buffer, 0), "second    ");
}

#[test]
fn colors_and_attributes_are_translated() {
    let mut widget = TerminalWidget::new(1, 10);
    // red fg, indexed 200 bg, truecolor fg, then bold+underline+italic.
    widget.update_screen(b"\x1b[31mR\x1b[0m\x1b[48;5;200mB\x1b[0m\x1b[38;2;1;2;3mT\x1b[0m\x1b[1;3;4mS\x1b[0m");

    let buffer = render(&widget, Rect::new(0, 0, 10, 1));
    assert_eq!(buffer[(0, 0)].fg, Color::Indexed(1));
    assert_eq!(buffer[(1, 0)].bg, Color::Indexed(200));
    assert_eq!(buffer[(2, 0)].fg, Color::Rgb(1, 2, 3));
    let styled = &buffer[(3, 0)];
    assert!(styled.modifier.contains(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED));
    // Default colors become Reset so the user's theme shows through.
    assert_eq!(buffer[(6, 0)].fg, Color::Reset);
}

#[test]
fn cursor_is_drawn_reversed_unless_hidden() {
    let mut widget = TerminalWidget::new(2, 10);
    widget.update_screen(b"ab");
    let buffer = render(&widget, Rect::new(0, 0, 10, 2));
    assert!(buffer[(2, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buffer[(1, 0)].modifier.contains(Modifier::REVERSED));

    widget.update_screen(b"ab\x1b[?25l");
    let buffer = render(&widget, Rect::new(0, 0, 10, 2));
    assert!(!buffer[(2, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn output_is_clipped_to_the_area() {
    let mut widget = TerminalWidget::new(3, 10);
    widget.update_screen(b"0123456789\r\nline2\r\nline3");

    let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 4));
    widget.render(Rect::new(0, 0, 4, 2), &mut buffer);

    assert_eq!(row(&buffer, 0), "0123        ");
    assert_eq!(row(&buffer, 1), "line        ");
    assert_eq!(row(&buffer, 2).trim(), "");
}

#[test]
fn resize_keeps_the_size_for_later_updates() {
    let mut widget = TerminalWidget::new(1, 4);
    widget.resize(1, 8);
    widget.update_screen(b"abcdefgh");

    let buffer = render(&widget, Rect::new(0, 0, 8, 1));
    assert_eq!(row(&buffer, 0), "abcdefgh");
}
