#![allow(dead_code)]

use nekominal_input::*;

pub mod typist;

/// Printable rendering of a byte stream: ESC as `\e`, CR as `\r`, LF as `\n`, TAB as `\t`, other
/// controls as `\xNN`, everything else as UTF-8 text.
pub fn show(bytes: &[u8]) -> String {
    let mut out = String::new();
    for ch in String::from_utf8_lossy(bytes).chars() {
        match ch {
            '\x1b' => out.push_str("\\e"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub const NOTHING: &str = "<none>";

/// Encodes a key event and renders the result.
pub fn enc(input: KeyInput, modes: &InputModes) -> String {
    match encode_key(&input, modes) {
        Some(bytes) => show(&bytes),
        None => NOTHING.to_string(),
    }
}

pub fn legacy() -> InputModes {
    InputModes::default()
}

/// Modes with the given kitty flag bits (1 disambiguate, 2 events, 4 alternates, 8 all keys, 16 text).
pub fn kitty(bits: u8) -> InputModes {
    InputModes { kitty: KittyFlags::from_bits_truncate(bits), ..InputModes::default() }
}

pub fn app_cursor(mut modes: InputModes) -> InputModes {
    modes.app_cursor = true;
    modes
}

pub fn app_keypad(mut modes: InputModes) -> InputModes {
    modes.app_keypad = true;
    modes
}

/// A key with no text and no modifiers.
pub fn key(key: Key) -> KeyInput {
    KeyInput::new(key)
}

/// Plain typing of a printable key: the OS delivers its character as text.
pub fn typed(c: char) -> KeyInput {
    KeyInput::new(Key::Char(c)).with_text(c.to_string())
}

/// A chord of a printable key: modifiers held, the OS delivers no text.
pub fn chord(c: char, mods: Mods) -> KeyInput {
    KeyInput::new(Key::Char(c)).with_mods(mods)
}

pub const SHIFT: Mods = Mods::SHIFT;
pub const ALT: Mods = Mods::ALT;
pub const CTRL: Mods = Mods::CTRL;
pub const SUPER: Mods = Mods::SUPER;
pub const CAPS: Mods = Mods::CAPS_LOCK;
pub const NUM: Mods = Mods::NUM_LOCK;

pub fn ctrl_alt() -> Mods {
    CTRL | ALT
}
