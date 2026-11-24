//! Turns key presses into the bytes an xterm would send for them, which is what the
//! programs in the daemon's terminals (`TERM=xterm-256color`) expect.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const ESC: u8 = 0x1b;

/// The bytes for `key`, or `None` for keys a terminal does not send anything for.
///
/// `application_cursor` is the program's DECCKM mode (set by full-screen programs like
/// btop, vim or less): unmodified arrows and Home/End are then sent as `ESC O x` instead
/// of `ESC [ x`.
pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // xterm's modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4). 1 means "none".
    let modifier = 1
        + u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(alt)
        + 4 * u8::from(ctrl);

    let bytes = match key.code {
        KeyCode::Char(c) => return Some(encode_char(c, ctrl, alt)),

        KeyCode::Enter => with_alt(alt, vec![b'\r']),
        KeyCode::Tab => with_alt(alt, vec![b'\t']),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![ESC],
        // Ctrl+Backspace deletes the word before the cursor: ^W is "delete word" in
        // readline, zsh, fish, vim and most other line editors.
        KeyCode::Backspace if ctrl => vec![0x17],
        KeyCode::Backspace => with_alt(alt, vec![0x7f]),

        KeyCode::Up => cursor_key(b'A', modifier, application_cursor),
        KeyCode::Down => cursor_key(b'B', modifier, application_cursor),
        KeyCode::Right => cursor_key(b'C', modifier, application_cursor),
        KeyCode::Left => cursor_key(b'D', modifier, application_cursor),
        KeyCode::Home => cursor_key(b'H', modifier, application_cursor),
        KeyCode::End => cursor_key(b'F', modifier, application_cursor),

        KeyCode::Insert => tilde_key(2, modifier),
        KeyCode::Delete => tilde_key(3, modifier),
        KeyCode::PageUp => tilde_key(5, modifier),
        KeyCode::PageDown => tilde_key(6, modifier),

        KeyCode::F(n @ 1..=4) => {
            let letter = b"PQRS"[usize::from(n - 1)];
            if modifier == 1 {
                vec![ESC, b'O', letter]
            } else {
                format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
            }
        }
        KeyCode::F(n @ 5..=12) => {
            // xterm skips 16 and 22.
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)];
            tilde_key(code, modifier)
        }

        _ => return None,
    };
    Some(bytes)
}

/// A printable key, with Ctrl turning it into a control character and Alt prefixing ESC.
fn encode_char(c: char, ctrl: bool, alt: bool) -> Vec<u8> {
    let control = if ctrl { control_byte(c) } else { None };
    let bytes = match control {
        Some(byte) => vec![byte],
        None => {
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
    };
    with_alt(alt, bytes)
}

/// The control character Ctrl+`c` produces, as on a VT/xterm keyboard.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, ESC);
    }
    bytes
}

/// Arrows and Home/End: `ESC [ x` (`ESC O x` in application mode), or `ESC [ 1 ; m x` with
/// modifiers, which is how Ctrl+Left / Ctrl+Right reach the shell as word movement.
fn cursor_key(letter: u8, modifier: u8, application_cursor: bool) -> Vec<u8> {
    match (modifier, application_cursor) {
        (1, true) => vec![ESC, b'O', letter],
        (1, false) => vec![ESC, b'[', letter],
        _ => format!("\x1b[1;{modifier}{}", letter as char).into_bytes(),
    }
}

/// Insert/Delete/PageUp/PageDown/F5-F12: `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
fn tilde_key(code: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}
