//! Key events to bytes: the xterm/VT encodings, plus the kitty keyboard protocol on top.
//!
//! `encode_key` is a pure function of the event and the terminal modes. The kitty protocol stack
//! and query handling live in the emulator (`alacritty_terminal`); this module only consumes the
//! resulting flags.

use crate::{
    key::{is_control, Key, KeyEvent, KeyInput, KeypadKey, Mods},
    modes::InputModes,
};

mod kitty;
mod legacy;

pub(crate) const ESC: u8 = 0x1b;

/// An event with the toolkit's quirks removed: AltGr stripped, control-character text dropped,
/// letters lower-cased, modifier keys carrying their own bit.
pub(crate) struct Norm<'a> {
    pub key: Key,
    pub mods: Mods,
    pub text: Option<&'a str>,
    pub event: KeyEvent,
    shifted: Option<char>,
    base: Option<char>,
}

/// The lower case of `c` when it is a single character, else `c`.
pub(crate) fn lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// The upper case of `c` when it is a single character, else `c`.
pub(crate) fn upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// What Shift does to a key when the toolkit did not say: upper case for letters, the US layout
/// for ASCII symbols. Wrong on layouts whose shifted symbols differ; supply `shifted_key` there.
fn shift_fallback(c: char) -> char {
    if c.is_alphabetic() {
        return upper(c);
    }
    match c {
        '`' => '~',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        other => other,
    }
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

impl<'a> Norm<'a> {
    pub fn new(input: &'a KeyInput) -> Self {
        let mut mods = input.mods;
        if input.altgr {
            mods = mods.without(Mods::CTRL | Mods::ALT);
        }
        if let Key::Modifier(m) = input.key
            && input.event != KeyEvent::Release
        {
            mods |= m.bit();
        }
        let text = input
            .text
            .as_deref()
            .filter(|t| !t.is_empty() && !t.chars().any(is_control));
        let key = match input.key {
            Key::Char(c) => Key::Char(lower(c)),
            other => other,
        };
        Norm {
            key,
            mods,
            text,
            event: input.event,
            shifted: input.shifted_key,
            base: input.base_layout_key,
        }
    }

    pub fn shift(&self) -> bool {
        self.mods.contains(Mods::SHIFT)
    }

    /// Modifiers that are held down (no lock state).
    pub fn held(&self) -> Mods {
        self.mods.only(Mods::HELD)
    }

    /// Any modifier except Shift: the chord is not plain typing.
    pub fn chord(&self) -> bool {
        self.held().without(Mods::SHIFT).bits() != 0
    }

    /// The base character of a text-producing key (the key code kitty reports).
    pub fn key_char(&self) -> Option<char> {
        match self.key {
            Key::Char(c) => Some(c),
            Key::Keypad(k) => k.text_char(),
            _ => None,
        }
    }

    /// The single character this key press types: the OS text when it is one character, else
    /// the key with Shift applied (`shifted_key`, falling back to upper case).
    pub fn typed_char(&self) -> Option<char> {
        if let Some(c) = self.text.and_then(single_char) {
            return Some(c);
        }
        let key = self.key_char()?;
        if self.shift() && matches!(self.key, Key::Char(_)) {
            Some(self.shifted.unwrap_or_else(|| shift_fallback(key)))
        } else {
            Some(key)
        }
    }

    /// The Shift-layer character, for the kitty "shifted key" field; `None` when Shift is not
    /// held or it equals the key itself.
    pub fn shifted_char(&self) -> Option<char> {
        if !self.shift() {
            return None;
        }
        let key = self.key_char()?;
        let shifted = self
            .shifted
            .or_else(|| self.text.and_then(single_char))
            .unwrap_or_else(|| shift_fallback(key));
        (shifted != key).then_some(shifted)
    }

    /// The PC-101 layout character of the same physical key, when it differs.
    pub fn base_char(&self) -> Option<char> {
        let base = self.base?;
        (Some(base) != self.key_char()).then_some(base)
    }

    /// The text this event types, as bytes to send when it is typing: the OS text, else the
    /// derived character.
    pub fn typed_text(&self) -> Option<String> {
        match self.text {
            Some(t) => Some(t.to_owned()),
            None => self.typed_char().map(String::from),
        }
    }
}

/// Encodes one key event. `None` means nothing is sent: a release in legacy mode, a modifier key
/// on its own, a dead key, a key with no encoding.
pub fn encode_key(input: &KeyInput, modes: &InputModes) -> Option<Vec<u8>> {
    let norm = Norm::new(input);
    // AltGr presses are text events. One that typed nothing (a key with no AltGr-layer
    // character) must not leak out as the bare key.
    if input.altgr && norm.text.is_none() && input.event != KeyEvent::Release {
        return None;
    }
    if modes.kitty.changes_encoding() {
        match kitty::encode(&norm, modes) {
            kitty::Outcome::Bytes(bytes) => return Some(bytes),
            kitty::Outcome::Skip => return None,
            kitty::Outcome::Legacy => {}
        }
    }
    legacy::encode(&norm, modes)
}

/// Text that is not a key press: an IME commit, a composed string, dropped text. Sent as UTF-8
/// with control characters removed, so it can never inject a sequence.
pub fn encode_text(text: &str) -> Vec<u8> {
    text.chars().filter(|c| !is_control(*c)).collect::<String>().into_bytes()
}

/// kitty's functional key number of a keypad key.
pub(crate) fn keypad_code(key: KeypadKey) -> u32 {
    match key {
        KeypadKey::Digit(n) => 57399 + n.min(9) as u32,
        KeypadKey::Decimal => 57409,
        KeypadKey::Divide => 57410,
        KeypadKey::Multiply => 57411,
        KeypadKey::Subtract => 57412,
        KeypadKey::Add => 57413,
        KeypadKey::Enter => 57414,
        KeypadKey::Equal => 57415,
        KeypadKey::Separator => 57416,
        KeypadKey::Left => 57417,
        KeypadKey::Right => 57418,
        KeypadKey::Up => 57419,
        KeypadKey::Down => 57420,
        KeypadKey::PageUp => 57421,
        KeypadKey::PageDown => 57422,
        KeypadKey::Home => 57423,
        KeypadKey::End => 57424,
        KeypadKey::Insert => 57425,
        KeypadKey::Delete => 57426,
        KeypadKey::Begin => 57427,
    }
}
