//! The traditional encoding: xterm's PC-style keys with the conventions of Windows Terminal and
//! kitty's legacy mode for the C0 keys (see the blueprint for the sources of each rule).

use super::{Norm, ESC};
use crate::{
    key::{Key, KeyEvent, KeypadKey, Mods},
    modes::InputModes,
};

/// xterm modifier parameter: `1 + (shift 1, alt 2, ctrl 4, super 8, hyper 16, meta 32)`. Lock keys
/// have no legacy encoding.
fn mod_param(held: Mods) -> Option<u32> {
    (!held.is_empty()).then(|| 1 + held.bits() as u32)
}

/// `ESC` + the bytes when Alt is held (the ESC prefix of the C0 keys and of Alt chords).
fn alt_prefixed(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if alt {
        out.push(ESC);
    }
    out.extend_from_slice(bytes);
    out
}

/// A CSI sequence ending in a letter: `CSI X`, or `CSI 1 ; m X` when modified. Unmodified keys of
/// the SS3 family (cursor keys under DECCKM, F1-F4, application keypad) use `SS3 X` when `ss3`.
fn letter_key(final_byte: u8, held: Mods, ss3: bool) -> Vec<u8> {
    match mod_param(held) {
        Some(m) => format!("\x1b[1;{m}{}", final_byte as char).into_bytes(),
        None if ss3 => vec![ESC, b'O', final_byte],
        None => vec![ESC, b'[', final_byte],
    }
}

/// `CSI n ~`, or `CSI n ; m ~` when modified.
fn tilde_key(code: u32, held: Mods) -> Vec<u8> {
    match mod_param(held) {
        Some(m) => format!("\x1b[{code};{m}~").into_bytes(),
        None => format!("\x1b[{code}~").into_bytes(),
    }
}

/// F5..=F20 as VT220/xterm numbers.
fn function_key_number(n: u8) -> Option<u32> {
    const TABLE: [u32; 16] = [15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34];
    TABLE.get(usize::from(n.checked_sub(5)?)).copied()
}

/// Ctrl mapping of ASCII keys (kitty's "commonly used superset" of the VT100 table). Characters
/// not listed stay as they are.
fn ctrl_char(c: char) -> Option<u8> {
    match c {
        ' ' | '@' | '2' => Some(0),
        'a'..='z' | 'A'..='Z' => Some(c as u8 & 0x1f),
        '[' | '3' => Some(27),
        '\\' | '4' => Some(28),
        ']' | '5' => Some(29),
        '^' | '~' | '6' => Some(30),
        '_' | '/' | '7' => Some(31),
        '?' | '8' => Some(127),
        _ => None,
    }
}

/// The control byte of a Ctrl chord, looked up by the shifted character first (Ctrl+Shift+/ is
/// Ctrl+?), then the key, then its PC-101 position (Ctrl+С on a Cyrillic layout is Ctrl+C).
fn ctrl_byte(n: &Norm) -> Option<u8> {
    let key = n.key_char()?;
    let shifted = if n.shift() { n.typed_char() } else { None };
    [shifted, Some(key), n.base_char()].into_iter().flatten().find_map(ctrl_char)
}

/// A UTF-8 encoded Latin-1 character: xterm's `eightBitInput` for a UTF-8 terminal.
fn meta_bit(byte: u8) -> Vec<u8> {
    char::from_u32(u32::from(byte) | 0x80)
        .map(|c| c.to_string().into_bytes())
        .unwrap_or_default()
}

fn text_key(n: &Norm, modes: &InputModes) -> Option<Vec<u8>> {
    let held = n.held();
    // The Windows key and the exotic modifiers have no legacy encoding.
    if held.intersects(Mods::SUPER | Mods::HYPER | Mods::META) {
        return None;
    }
    let (ctrl, alt) = (held.contains(Mods::CTRL), held.contains(Mods::ALT));
    let escape = modes.alt_sends_escape;

    if ctrl && let Some(byte) = ctrl_byte(n) {
        return Some(if alt && !escape { meta_bit(byte) } else { alt_prefixed(alt, &[byte]) });
    }

    // Typing, an Alt chord, or a Ctrl chord that has no control character: the character itself.
    let typed = n.typed_text()?;
    if alt && !escape && typed.is_ascii() && typed.len() == 1 {
        return Some(meta_bit(typed.as_bytes()[0]));
    }
    Some(alt_prefixed(alt, typed.as_bytes()))
}

fn keypad(n: &Norm, modes: &InputModes, key: KeypadKey) -> Option<Vec<u8>> {
    let held = n.held();
    let alt = held.contains(Mods::ALT);
    // The OS delivers the navigation half of the keypad (NumLock off) as the keys they print.
    let nav = match key {
        KeypadKey::Left => Some(Key::Left),
        KeypadKey::Right => Some(Key::Right),
        KeypadKey::Up => Some(Key::Up),
        KeypadKey::Down => Some(Key::Down),
        KeypadKey::Home => Some(Key::Home),
        KeypadKey::End => Some(Key::End),
        KeypadKey::PageUp => Some(Key::PageUp),
        KeypadKey::PageDown => Some(Key::PageDown),
        KeypadKey::Insert => Some(Key::Insert),
        KeypadKey::Delete => Some(Key::Delete),
        _ => None,
    };
    if let Some(equivalent) = nav {
        return named(n, modes, equivalent);
    }
    if key == KeypadKey::Begin {
        return Some(letter_key(b'E', held, modes.app_cursor));
    }
    if modes.app_keypad {
        // Application keypad: SS3 p..y, j..o, M, X. Like Windows Terminal, modifiers other than
        // the Alt prefix are not encoded.
        let final_byte = match key {
            KeypadKey::Digit(d) => b'p' + d.min(9),
            KeypadKey::Multiply => b'j',
            KeypadKey::Add => b'k',
            KeypadKey::Separator => b'l',
            KeypadKey::Subtract => b'm',
            KeypadKey::Decimal => b'n',
            KeypadKey::Divide => b'o',
            KeypadKey::Equal => b'X',
            KeypadKey::Enter => b'M',
            _ => return None,
        };
        return Some(alt_prefixed(alt, &[ESC, b'O', final_byte]));
    }
    if key == KeypadKey::Enter {
        return named(n, modes, Key::Enter);
    }
    text_key(n, modes)
}

fn named(n: &Norm, modes: &InputModes, key: Key) -> Option<Vec<u8>> {
    let held = n.held();
    let (ctrl, alt, shift) =
        (held.contains(Mods::CTRL), held.contains(Mods::ALT), held.contains(Mods::SHIFT));
    Some(match key {
        // Ctrl+Enter is LF on PC layouts (Windows Terminal, conhost). Shift+Enter has no legacy
        // encoding and is plain Enter.
        Key::Enter => alt_prefixed(alt, if ctrl { b"\n" } else { b"\r" }),
        Key::Tab if shift => alt_prefixed(alt, b"\x1b[Z"),
        Key::Tab => alt_prefixed(alt, b"\t"),
        // Ctrl swaps BS and DEL.
        Key::Backspace => {
            let delete = modes.backspace_sends_delete != ctrl;
            alt_prefixed(alt, if delete { b"\x7f" } else { b"\x08" })
        }
        Key::Escape => alt_prefixed(alt, &[ESC]),
        Key::Up => letter_key(b'A', held, modes.app_cursor),
        Key::Down => letter_key(b'B', held, modes.app_cursor),
        Key::Right => letter_key(b'C', held, modes.app_cursor),
        Key::Left => letter_key(b'D', held, modes.app_cursor),
        Key::Home => letter_key(b'H', held, modes.app_cursor),
        Key::End => letter_key(b'F', held, modes.app_cursor),
        Key::Insert => tilde_key(2, held),
        Key::Delete => tilde_key(3, held),
        Key::PageUp => tilde_key(5, held),
        Key::PageDown => tilde_key(6, held),
        Key::F(n @ 1..=4) => letter_key(b'O' + n, held, true),
        Key::F(n) => tilde_key(function_key_number(n)?, held),
        // xterm's Menu key is "Do" (F16).
        Key::Menu => tilde_key(29, held),
        // Traditionally Ctrl+Z on Windows consoles.
        Key::Pause => vec![0x1a],
        _ => return None,
    })
}

pub(super) fn encode(n: &Norm, modes: &InputModes) -> Option<Vec<u8>> {
    // The legacy encoding has no release events, and auto-repeat is just another press.
    if n.event == KeyEvent::Release {
        return None;
    }
    match n.key {
        Key::Char(_) => text_key(n, modes),
        Key::Unidentified => n.text.map(|t| t.as_bytes().to_vec()),
        Key::Keypad(key) => keypad(n, modes, key),
        Key::Dead
        | Key::Modifier(_)
        | Key::CapsLock
        | Key::NumLock
        | Key::ScrollLock
        | Key::PrintScreen
        | Key::Media(_) => None,
        other => named(n, modes, other),
    }
}
