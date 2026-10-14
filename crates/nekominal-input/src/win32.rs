//! win32-input-mode: `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`, a lossless carrier of a Win32
//! `KEY_EVENT_RECORD` that ConPTY (and Windows Terminal) understand on the input pipe.
//!
//! This is an alternative to the VT / kitty encoders for the Windows backend: the toolkit fills
//! the record from its own key event, ConPTY synthesizes the key event for the child and, for
//! children in VT-input mode, translates it back to whatever the child asked for.

use crate::{
    key::{Key, KeyInput, Mods},
    keyboard::encode_key,
    modes::InputModes,
};

/// `dwControlKeyState` bits.
pub mod control_state {
    pub const RIGHT_ALT_PRESSED: u32 = 0x0001;
    pub const LEFT_ALT_PRESSED: u32 = 0x0002;
    pub const RIGHT_CTRL_PRESSED: u32 = 0x0004;
    pub const LEFT_CTRL_PRESSED: u32 = 0x0008;
    pub const SHIFT_PRESSED: u32 = 0x0010;
    pub const NUMLOCK_ON: u32 = 0x0020;
    pub const SCROLLLOCK_ON: u32 = 0x0040;
    pub const CAPSLOCK_ON: u32 = 0x0080;
    pub const ENHANCED_KEY: u32 = 0x0100;
}

/// The fields of a Win32 `KEY_EVENT_RECORD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Win32Key {
    /// `wVirtualKeyCode`.
    pub virtual_key: u16,
    /// `wVirtualScanCode`.
    pub scan_code: u16,
    /// `uChar.UnicodeChar`: one UTF-16 code unit, 0 for none.
    pub unicode: u16,
    pub key_down: bool,
    /// `dwControlKeyState`.
    pub control_state: u32,
    pub repeat_count: u16,
}

impl Win32Key {
    /// `dwControlKeyState` for a modifier set, with the left-hand variants of Alt and Ctrl; set
    /// `ENHANCED_KEY` and the right-hand bits yourself when you know them.
    pub fn control_state_from_mods(mods: Mods) -> u32 {
        use control_state::*;
        let mut state = 0;
        if mods.contains(Mods::SHIFT) {
            state |= SHIFT_PRESSED;
        }
        if mods.contains(Mods::ALT) {
            state |= LEFT_ALT_PRESSED;
        }
        if mods.contains(Mods::CTRL) {
            state |= LEFT_CTRL_PRESSED;
        }
        if mods.contains(Mods::CAPS_LOCK) {
            state |= CAPSLOCK_ON;
        }
        if mods.contains(Mods::NUM_LOCK) {
            state |= NUMLOCK_ON;
        }
        state
    }
}

/// `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`
pub fn encode_win32_input(key: &Win32Key) -> Vec<u8> {
    format!(
        "\x1b[{};{};{};{};{};{}_",
        key.virtual_key,
        key.scan_code,
        key.unicode,
        u8::from(key.key_down),
        key.control_state,
        key.repeat_count.max(1)
    )
    .into_bytes()
}

/// Text as synthesized key events (virtual key 0, one UTF-16 unit each, a surrogate pair for
/// characters beyond the BMP), the way Windows' `SendInput(KEYEVENTF_UNICODE)` and Windows
/// Terminal's IME path deliver it. Each unit is a key down followed by a key up.
pub fn encode_win32_text(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in text.encode_utf16() {
        for key_down in [true, false] {
            out.extend(encode_win32_input(&Win32Key {
                virtual_key: 0,
                scan_code: 0,
                unicode: unit,
                key_down,
                control_state: 0,
                repeat_count: 1,
            }));
        }
    }
    out
}

/// Which encoding carries a key event to a ConPTY child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConptyRoute {
    /// The program asked for kitty keyboard flags: send what [`encode_key`] produces. ConPTY
    /// passes it through to children that read VT input and tracks no kitty state of its own.
    Kitty,
    /// win32-input-mode: lossless for Win32 console children (Shift+Enter, Ctrl+Space, F13..F24,
    /// Menu, key releases) and rendered as legacy VT bytes for VT-input children.
    Win32,
}

/// The kitty flags the program pushed decide the route; without them the win32-input-mode
/// record is the better carrier on ConPTY (see the blueprint for the measurements).
pub fn conpty_route(modes: &InputModes) -> ConptyRoute {
    if modes.kitty.changes_encoding() {
        ConptyRoute::Kitty
    } else {
        ConptyRoute::Win32
    }
}

/// Encodes one key event for a ConPTY child. `record` is the `KEY_EVENT_RECORD` the toolkit
/// derived from the same event (virtual key, scan code, UTF-16 unit, control state, down/up);
/// `input` is its toolkit-independent description. Dead key presses are never sent.
pub fn encode_key_conpty(input: &KeyInput, record: &Win32Key, modes: &InputModes) -> Option<Vec<u8>> {
    if input.key == Key::Dead {
        return None;
    }
    match conpty_route(modes) {
        ConptyRoute::Kitty => encode_key(input, modes),
        ConptyRoute::Win32 => Some(encode_win32_input(record)),
    }
}