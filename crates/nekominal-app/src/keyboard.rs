//! Reading the keyboard from XAML key events.
//!
//! Routing (chrome-winui.md section 12, critique.md C6): the window content root sees `PreviewKeyDown` first
//! and runs application shortcuts, matched by physical key so they keep working on every layout; the focused
//! pane then encodes non-text keys and Ctrl/Alt chords from the same event and marks it handled; plain text
//! reaches the pane through `CharacterReceived` (or the IME).

use nekominal_input::{Mods, scancode::physical_key, vk};
use nekominal_model::keymap::Modifiers;
use nekominal_winui_sys::*;

const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;

pub const VK_ESCAPE: i32 = 0x1B;
pub const VK_RETURN: i32 = 0x0D;
pub const VK_UP: i32 = 0x26;
pub const VK_DOWN: i32 = 0x28;

fn is_down(code: i32) -> bool {
    InputKeyboardSource::GetKeyStateForCurrentThread(VirtualKey(code)).map(|state| (state.0 & 1) != 0).unwrap_or(false)
}

/// The modifier keys held right now, as the model's shortcut matcher wants them.
pub fn held_modifiers() -> Modifiers {
    Modifiers {
        ctrl: is_down(VK_CONTROL),
        shift: is_down(VK_SHIFT),
        alt: is_down(VK_MENU),
        meta: is_down(VK_LWIN) || is_down(VK_RWIN),
    }
}

/// The same modifiers for the input encoder.
pub fn input_mods(held: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    for (pressed, flag) in
        [(held.shift, Mods::SHIFT), (held.ctrl, Mods::CTRL), (held.alt, Mods::ALT), (held.meta, Mods::SUPER)]
    {
        if pressed {
            mods |= flag;
        }
    }
    mods
}

/// The W3C `code` of the physical key of a key event (`KeyT`, `Digit1`, `ArrowLeft`); `None` for keys no
/// shortcut can name. XAML reports a scan code of 0 for Tab, so the position is derived from the virtual key.
pub fn physical_code(virtual_key: i32, status: &CorePhysicalKeyStatus) -> Option<&'static str> {
    let extended = status.is_extended_key;
    let scan = u16::try_from(status.scan_code)
        .ok()
        .filter(|scan| *scan != 0)
        .or_else(|| vk::default_scan_code(virtual_key, extended))?;
    physical_key(scan, extended).map(|key| key.name())
}
