//! Windows virtual-key code -> [`Key`] for keys that must be encoded from the key event.
//!
//! Policy proven in chrome-winui (keyboard routing): non-text keys and Ctrl/Alt chords are encoded from
//! `PreviewKeyDown` and marked handled; plain text keys are left alone so XAML raises `CharacterReceived`
//! (and, with an IME, composition). `VK_PROCESSKEY` / `VK_PACKET` are left to `CharacterReceived`.
//!
//! [`classify`] is the full decision (the key assembler uses it); [`key_for_vk`] keeps the narrower
//! question "must this key be encoded from the key event?" for callers that only need that.

use crate::{
    Key, KeypadKey, ModifierKey, Mods,
    scancode::{PhysicalKey, physical_key},
};

pub const VK_PROCESSKEY: i32 = 0xE5;
pub const VK_PACKET: i32 = 0xFF;

/// What a virtual key is, as far as the terminal is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// Encoded from the key event itself: Enter, Tab, Escape, cursor and editing keys, F1..F24, Menu,
    /// PrintScreen, Pause and the navigation half of the keypad. No `CharacterReceived` is expected.
    Named(Key),
    /// Shift, Ctrl, Alt and Windows keys, with their side.
    Modifier(ModifierKey),
    /// Caps Lock, Num Lock and Scroll Lock.
    Lock(Key),
    /// Types text through `CharacterReceived` when no chord is held. The key is the identity used
    /// for chords (Ctrl+C): letters, digits, punctuation, Space, keypad digits and operators.
    Text(Key),
    /// `VK_PROCESSKEY`: an IME is composing; the text arrives through the IME.
    Ime,
    /// `VK_PACKET`: injected Unicode text; the text arrives through `CharacterReceived`.
    Packet,
    /// Media, browser and launch keys, IME mode keys, the fake Shift of the keypad and anything
    /// unknown: not ours, so never swallowed.
    Ignored,
}

/// The unshifted US character of an OEM punctuation key.
fn oem_char(vk: i32) -> Option<char> {
    Some(match vk {
        0xBA => ';',
        0xBB => '=',
        0xBC => ',',
        0xBD => '-',
        0xBE => '.',
        0xBF => '/',
        0xC0 => '`',
        0xDB => '[',
        0xDC => '\\',
        0xDD => ']',
        0xDE => '\'',
        // The ISO key and the extra key on some layouts have no US counterpart.
        0xDF => '`',
        0xE2 => '<',
        _ => return None,
    })
}

/// A punctuation key is identified by its position, so Ctrl+ü on a German layout (which sits where
/// `[` is on a US board) is Ctrl+[. Letters and digits keep the layout's own identity, and a position
/// that is a letter on a US board (Dvorak's comma) falls back to the virtual key.
fn oem_key(vk: i32, scan_code: u16, extended: bool) -> Option<Key> {
    let positional =
        physical_key(scan_code, extended).and_then(PhysicalKey::us_char).filter(|c| !c.is_alphanumeric() && *c != ' ');
    positional.or_else(|| oem_char(vk)).map(Key::Char)
}

/// The navigation half of the keypad (NumLock off) shares its virtual keys with the dedicated
/// cluster and differs by the missing `E0` prefix. The scan code guards against synthesized
/// input (an on-screen keyboard sends arrows without any scan code).
fn keypad_navigation(vk: i32, scan_code: u16, extended: bool) -> Option<KeypadKey> {
    if extended || !(0x47..=0x53).contains(&scan_code) {
        return None;
    }
    Some(match vk {
        0x21 => KeypadKey::PageUp,
        0x22 => KeypadKey::PageDown,
        0x23 => KeypadKey::End,
        0x24 => KeypadKey::Home,
        0x25 => KeypadKey::Left,
        0x26 => KeypadKey::Up,
        0x27 => KeypadKey::Right,
        0x28 => KeypadKey::Down,
        0x2D => KeypadKey::Insert,
        0x2E => KeypadKey::Delete,
        _ => return None,
    })
}

/// Classifies a virtual key. `scan_code` is `KeyStatus.ScanCode` (0 when unknown) and `extended` is
/// `KeyStatus.IsExtendedKey`.
pub fn classify(vk: i32, scan_code: u16, extended: bool) -> KeyClass {
    use KeyClass::*;

    let named = |key: Key| Named(keypad_navigation(vk, scan_code, extended).map_or(key, Key::Keypad));
    match vk {
        0x08 => Named(Key::Backspace),
        0x09 => Named(Key::Tab),
        0x0C => Named(Key::Keypad(KeypadKey::Begin)),
        0x0D if extended => Named(Key::Keypad(KeypadKey::Enter)),
        0x0D => Named(Key::Enter),
        0x1B => Named(Key::Escape),

        // Windows brackets keypad navigation with a fake Shift (`E0 2A`) while Shift is held; real
        // Shift keys never carry the prefix.
        0x10 | 0xA0 | 0xA1 if extended => Ignored,
        0xA0 => Modifier(ModifierKey::LeftShift),
        0xA1 => Modifier(ModifierKey::RightShift),
        0x10 if scan_code == 0x36 => Modifier(ModifierKey::RightShift),
        0x10 => Modifier(ModifierKey::LeftShift),
        0xA2 => Modifier(ModifierKey::LeftControl),
        0xA3 => Modifier(ModifierKey::RightControl),
        0x11 if extended => Modifier(ModifierKey::RightControl),
        0x11 => Modifier(ModifierKey::LeftControl),
        0xA4 => Modifier(ModifierKey::LeftAlt),
        0xA5 => Modifier(ModifierKey::RightAlt),
        0x12 if extended => Modifier(ModifierKey::RightAlt),
        0x12 => Modifier(ModifierKey::LeftAlt),
        0x5B => Modifier(ModifierKey::LeftSuper),
        0x5C => Modifier(ModifierKey::RightSuper),

        // VK_CANCEL is what Ctrl+Pause reports.
        0x03 | 0x13 => Named(Key::Pause),
        0x14 => Lock(Key::CapsLock),
        0x90 => Lock(Key::NumLock),
        0x91 => Lock(Key::ScrollLock),
        0x2C => Named(Key::PrintScreen),
        0x5D => Named(Key::Menu),

        0x21 => named(Key::PageUp),
        0x22 => named(Key::PageDown),
        0x23 => named(Key::End),
        0x24 => named(Key::Home),
        0x25 => named(Key::Left),
        0x26 => named(Key::Up),
        0x27 => named(Key::Right),
        0x28 => named(Key::Down),
        0x2D => named(Key::Insert),
        0x2E => named(Key::Delete),
        0x70..=0x87 => Named(Key::F((vk - 0x6F) as u8)),

        0x20 => Text(Key::Char(' ')),
        0x30..=0x39 => Text(Key::Char(char::from(vk as u8))),
        0x41..=0x5A => Text(Key::Char(char::from(vk as u8).to_ascii_lowercase())),
        0x60..=0x69 => Text(Key::Keypad(KeypadKey::Digit((vk - 0x60) as u8))),
        0x6A => Text(Key::Keypad(KeypadKey::Multiply)),
        0x6B => Text(Key::Keypad(KeypadKey::Add)),
        0x6C => Text(Key::Keypad(KeypadKey::Separator)),
        0x6D => Text(Key::Keypad(KeypadKey::Subtract)),
        0x6E => Text(Key::Keypad(KeypadKey::Decimal)),
        0x6F => Text(Key::Keypad(KeypadKey::Divide)),
        0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 => oem_key(vk, scan_code, extended).map_or(Ignored, Text),

        VK_PROCESSKEY => Ime,
        VK_PACKET => Packet,
        _ => Ignored,
    }
}

/// `Some(key)` when the key must be encoded from the key event, `None` when it is a text key (its
/// text arrives through `CharacterReceived`), belongs to the IME or is not ours. Text keys count
/// as encoded from the event while Ctrl or Alt is held. Same as [`key_for_vk_scan`] without a scan
/// code.
pub fn key_for_vk(vk: i32, extended: bool, mods: Mods) -> Option<Key> {
    key_for_vk_scan(vk, 0, extended, mods)
}

/// [`key_for_vk`] with the scan code, which tells keypad navigation from the cursor cluster and
/// names OEM punctuation keys by position.
pub fn key_for_vk_scan(vk: i32, scan_code: u16, extended: bool, mods: Mods) -> Option<Key> {
    match classify(vk, scan_code, extended) {
        KeyClass::Named(key) | KeyClass::Lock(key) => Some(key),
        KeyClass::Modifier(modifier) => Some(Key::Modifier(modifier)),
        KeyClass::Text(key) => mods.intersects(Mods::CTRL | Mods::ALT).then_some(key),
        KeyClass::Ime | KeyClass::Packet | KeyClass::Ignored => None,
    }
}

/// The make code of a key by its US position, for events that arrive without one (WinUI reports 0
/// for Tab). `None` for keys with no fixed position.
pub fn default_scan_code(vk: i32, extended: bool) -> Option<u16> {
    let key = match vk {
        0x08 => PhysicalKey::Backspace,
        0x09 => PhysicalKey::Tab,
        0x0D if extended => PhysicalKey::NumpadEnter,
        0x0D => PhysicalKey::Enter,
        0x1B => PhysicalKey::Escape,
        0x21 => PhysicalKey::PageUp,
        0x22 => PhysicalKey::PageDown,
        0x23 => PhysicalKey::End,
        0x24 => PhysicalKey::Home,
        0x25 => PhysicalKey::ArrowLeft,
        0x26 => PhysicalKey::ArrowUp,
        0x27 => PhysicalKey::ArrowRight,
        0x28 => PhysicalKey::ArrowDown,
        0x2D => PhysicalKey::Insert,
        0x2E => PhysicalKey::Delete,
        0x20 | 0x30..=0x39 | 0x41..=0x5A => {
            let c = char::from(vk as u8).to_ascii_lowercase();
            PhysicalKey::all().find(|key| key.us_char() == Some(c))?
        }
        0x70..=0x87 => PhysicalKey::from_name(&format!("F{}", vk - 0x6F))?,
        _ => return None,
    };
    Some(key.scan_code().0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keys_are_left_to_character_received() {
        assert_eq!(key_for_vk(0x41, false, Mods::NONE), None);
        assert_eq!(key_for_vk(0x41, false, Mods::SHIFT), None);
        assert_eq!(key_for_vk(VK_PROCESSKEY, false, Mods::NONE), None);
        assert_eq!(key_for_vk(VK_PACKET, false, Mods::NONE), None);
    }

    #[test]
    fn chords_and_navigation_are_encoded_from_the_event() {
        assert_eq!(key_for_vk(0x43, false, Mods::CTRL), Some(Key::Char('c')));
        assert_eq!(key_for_vk(0x0D, false, Mods::NONE), Some(Key::Enter));
        assert_eq!(key_for_vk(0x0D, true, Mods::NONE), Some(Key::Keypad(KeypadKey::Enter)));
        assert_eq!(key_for_vk(0x26, true, Mods::NONE), Some(Key::Up));
        assert_eq!(key_for_vk(0x70, false, Mods::NONE), Some(Key::F(1)));
        assert_eq!(key_for_vk(0x87, false, Mods::NONE), Some(Key::F(24)));
        assert_eq!(key_for_vk(0xBC, false, Mods::CTRL), Some(Key::Char(',')));
    }

    #[test]
    fn function_keys_f13_to_f24_are_named() {
        for n in 13..=24u8 {
            assert_eq!(key_for_vk(0x6F + i32::from(n), false, Mods::NONE), Some(Key::F(n)));
        }
        assert_eq!(key_for_vk(0x88, false, Mods::NONE), None);
    }

    #[test]
    fn keypad_digits_and_operators_are_text_unless_a_chord_is_held() {
        for n in 0..=9u8 {
            let vk = 0x60 + i32::from(n);
            assert_eq!(key_for_vk(vk, false, Mods::NUM_LOCK), None, "{n}");
            assert_eq!(key_for_vk(vk, false, Mods::CTRL | Mods::NUM_LOCK), Some(Key::Keypad(KeypadKey::Digit(n))));
        }
        assert_eq!(classify(0x6A, 0x37, false), KeyClass::Text(Key::Keypad(KeypadKey::Multiply)));
        assert_eq!(classify(0x6B, 0x4E, false), KeyClass::Text(Key::Keypad(KeypadKey::Add)));
        assert_eq!(classify(0x6C, 0, false), KeyClass::Text(Key::Keypad(KeypadKey::Separator)));
        assert_eq!(classify(0x6D, 0x4A, false), KeyClass::Text(Key::Keypad(KeypadKey::Subtract)));
        assert_eq!(classify(0x6E, 0x53, false), KeyClass::Text(Key::Keypad(KeypadKey::Decimal)));
        assert_eq!(classify(0x6F, 0x35, true), KeyClass::Text(Key::Keypad(KeypadKey::Divide)));
    }

    #[test]
    fn keypad_navigation_is_told_from_the_cursor_cluster_by_the_prefix() {
        // NumLock off: keypad 4 reports VK_LEFT, scan 0x4B, no prefix.
        assert_eq!(classify(0x25, 0x4B, false), KeyClass::Named(Key::Keypad(KeypadKey::Left)));
        assert_eq!(classify(0x25, 0x4B, true), KeyClass::Named(Key::Left));
        assert_eq!(classify(0x24, 0x47, false), KeyClass::Named(Key::Keypad(KeypadKey::Home)));
        assert_eq!(classify(0x2E, 0x53, false), KeyClass::Named(Key::Keypad(KeypadKey::Delete)));
        assert_eq!(classify(0x2D, 0x52, false), KeyClass::Named(Key::Keypad(KeypadKey::Insert)));
        assert_eq!(classify(0x0C, 0x4C, false), KeyClass::Named(Key::Keypad(KeypadKey::Begin)));
    }

    #[test]
    fn arrows_without_a_scan_code_are_the_cursor_cluster() {
        // An on-screen keyboard sends VK_LEFT without prefix and without scan code.
        assert_eq!(classify(0x25, 0, false), KeyClass::Named(Key::Left));
        assert_eq!(key_for_vk(0x25, false, Mods::NONE), Some(Key::Left));
    }

    #[test]
    fn modifier_keys_carry_their_side() {
        assert_eq!(classify(0xA0, 0x2A, false), KeyClass::Modifier(ModifierKey::LeftShift));
        assert_eq!(classify(0xA1, 0x36, false), KeyClass::Modifier(ModifierKey::RightShift));
        assert_eq!(classify(0x10, 0x2A, false), KeyClass::Modifier(ModifierKey::LeftShift));
        assert_eq!(classify(0x10, 0x36, false), KeyClass::Modifier(ModifierKey::RightShift));
        assert_eq!(classify(0x11, 0x1D, false), KeyClass::Modifier(ModifierKey::LeftControl));
        assert_eq!(classify(0x11, 0x1D, true), KeyClass::Modifier(ModifierKey::RightControl));
        assert_eq!(classify(0x12, 0x38, false), KeyClass::Modifier(ModifierKey::LeftAlt));
        assert_eq!(classify(0x12, 0x38, true), KeyClass::Modifier(ModifierKey::RightAlt));
        assert_eq!(classify(0xA4, 0, false), KeyClass::Modifier(ModifierKey::LeftAlt));
        assert_eq!(classify(0x5B, 0x5B, true), KeyClass::Modifier(ModifierKey::LeftSuper));
        assert_eq!(classify(0x5C, 0x5C, true), KeyClass::Modifier(ModifierKey::RightSuper));
        assert_eq!(key_for_vk(0x10, false, Mods::NONE), Some(Key::Modifier(ModifierKey::LeftShift)));
    }

    #[test]
    fn the_fake_shift_around_keypad_navigation_is_ignored() {
        assert_eq!(classify(0x10, 0x2A, true), KeyClass::Ignored);
        assert_eq!(classify(0xA0, 0x2A, true), KeyClass::Ignored);
        assert_eq!(classify(0xA1, 0x36, true), KeyClass::Ignored);
    }

    #[test]
    fn lock_and_system_keys() {
        assert_eq!(classify(0x14, 0x3A, false), KeyClass::Lock(Key::CapsLock));
        assert_eq!(classify(0x90, 0x45, true), KeyClass::Lock(Key::NumLock));
        assert_eq!(classify(0x91, 0x46, false), KeyClass::Lock(Key::ScrollLock));
        assert_eq!(classify(0x13, 0x45, false), KeyClass::Named(Key::Pause));
        assert_eq!(classify(0x03, 0x46, true), KeyClass::Named(Key::Pause));
        assert_eq!(classify(0x2C, 0x37, true), KeyClass::Named(Key::PrintScreen));
        assert_eq!(classify(0x5D, 0x5D, true), KeyClass::Named(Key::Menu));
    }

    #[test]
    fn media_browser_and_ime_mode_keys_are_not_ours() {
        // Browser back/forward/refresh/stop/search/favorites/home, volume, media, launch.
        for vk in 0xA6..=0xB7 {
            assert_eq!(classify(vk, 0, true), KeyClass::Ignored, "{vk:#x}");
            assert_eq!(key_for_vk(vk, true, Mods::CTRL), None, "{vk:#x}");
        }
        // IME mode keys (Kana, Junja, Final, Hanja/Kanji, Convert, NonConvert, Accept, ModeChange).
        for vk in [0x15, 0x17, 0x18, 0x19, 0x1C, 0x1D, 0x1E, 0x1F, 0x5F] {
            assert_eq!(classify(vk, 0, false), KeyClass::Ignored, "{vk:#x}");
        }
        assert_eq!(classify(0xFA, 0, false), KeyClass::Ignored);
        assert_eq!(classify(-1, 0, false), KeyClass::Ignored);
        assert_eq!(classify(0x1000, 0, false), KeyClass::Ignored);
    }

    #[test]
    fn ime_and_injected_text_are_told_apart() {
        assert_eq!(classify(VK_PROCESSKEY, 0, false), KeyClass::Ime);
        assert_eq!(classify(VK_PACKET, 0, false), KeyClass::Packet);
    }

    #[test]
    fn oem_punctuation_is_named_by_scan_code() {
        // German: ü is VK_OEM_1 at the position of [, + is VK_OEM_PLUS at the position of ].
        assert_eq!(classify(0xBA, 0x1A, false), KeyClass::Text(Key::Char('[')));
        assert_eq!(classify(0xBB, 0x1B, false), KeyClass::Text(Key::Char(']')));
        // The same virtual keys without a scan code fall back to the US names.
        assert_eq!(classify(0xBA, 0, false), KeyClass::Text(Key::Char(';')));
        assert_eq!(classify(0xBB, 0, false), KeyClass::Text(Key::Char('=')));
        // US: the scan code agrees with the virtual key.
        assert_eq!(classify(0xBC, 0x33, false), KeyClass::Text(Key::Char(',')));
        assert_eq!(classify(0xDE, 0x28, false), KeyClass::Text(Key::Char('\'')));
        assert_eq!(classify(0xC0, 0x29, false), KeyClass::Text(Key::Char('`')));
    }

    #[test]
    fn oem_key_at_a_letter_position_keeps_the_virtual_key() {
        // Dvorak puts the comma where Q... W is on a US board: Ctrl+, must not become Ctrl+W.
        assert_eq!(classify(0xBC, 0x11, false), KeyClass::Text(Key::Char(',')));
        // Colemak: semicolon sits at P.
        assert_eq!(classify(0xBA, 0x19, false), KeyClass::Text(Key::Char(';')));
    }

    #[test]
    fn the_iso_key_is_a_text_key() {
        assert_eq!(classify(0xE2, 0x56, false), KeyClass::Text(Key::Char('<')));
        assert_eq!(classify(0xE2, 0, false), KeyClass::Text(Key::Char('<')));
    }

    #[test]
    fn letters_and_digits_keep_the_layout_identity() {
        // AZERTY: the key at the position of Q reports VK_A.
        assert_eq!(classify(0x41, 0x10, false), KeyClass::Text(Key::Char('a')));
        // QWERTZ: the key at the position of Y reports VK_Z.
        assert_eq!(classify(0x5A, 0x15, false), KeyClass::Text(Key::Char('z')));
        assert_eq!(classify(0x31, 0x02, false), KeyClass::Text(Key::Char('1')));
        assert_eq!(classify(0x20, 0x39, false), KeyClass::Text(Key::Char(' ')));
    }

    #[test]
    fn a_missing_scan_code_is_filled_from_the_us_position() {
        assert_eq!(default_scan_code(0x09, false), Some(0x0F));
        assert_eq!(default_scan_code(0x41, false), Some(0x1E));
        assert_eq!(default_scan_code(0x20, false), Some(0x39));
        assert_eq!(default_scan_code(0x0D, true), Some(0x1C));
        assert_eq!(default_scan_code(0x26, true), Some(0x48));
        assert_eq!(default_scan_code(0x70, false), Some(0x3B));
        assert_eq!(default_scan_code(0x87, false), Some(0x76));
        assert_eq!(default_scan_code(0x10, false), None);
    }
}
