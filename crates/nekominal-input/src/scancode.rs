//! Layout-independent key names from Windows scan codes.
//!
//! A scan code identifies the key's position on the board, not what it types: the same position is
//! `Q` on US, `A` on French and `Й` on Russian. Shortcuts that must keep working whatever the active
//! layout is match [`PhysicalKey`]s; text and Ctrl chords use the layout. The names are the W3C UI
//! Events `code` values, so a shortcut table written for the web or for winit reads the same.
//!
//! The table is the keyboard-driver make-code set (scan code set 1) that WinUI reports in
//! `KeyStatus.ScanCode` with `IsExtendedKey` standing for the `E0` prefix.

macro_rules! physical_keys {
    ($($variant:ident $name:literal $scan:literal $extended:literal $us:literal $us_shifted:literal;)*) => {
        /// A key by position on the board.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum PhysicalKey {
            $($variant,)*
        }

        /// (key, W3C code, scan code, extended, US layout character, US layout shifted character);
        /// `'\0'` where the key types nothing. The one source of truth of this module.
        const TABLE: &[(PhysicalKey, &str, u16, bool, char, char)] = &[
            $((PhysicalKey::$variant, $name, $scan, $extended, $us, $us_shifted),)*
        ];
    };
}

physical_keys! {
    Escape "Escape" 0x01 false '\0' '\0';
    Digit1 "Digit1" 0x02 false '1' '!';
    Digit2 "Digit2" 0x03 false '2' '@';
    Digit3 "Digit3" 0x04 false '3' '#';
    Digit4 "Digit4" 0x05 false '4' '$';
    Digit5 "Digit5" 0x06 false '5' '%';
    Digit6 "Digit6" 0x07 false '6' '^';
    Digit7 "Digit7" 0x08 false '7' '&';
    Digit8 "Digit8" 0x09 false '8' '*';
    Digit9 "Digit9" 0x0A false '9' '(';
    Digit0 "Digit0" 0x0B false '0' ')';
    Minus "Minus" 0x0C false '-' '_';
    Equal "Equal" 0x0D false '=' '+';
    Backspace "Backspace" 0x0E false '\0' '\0';
    Tab "Tab" 0x0F false '\0' '\0';
    KeyQ "KeyQ" 0x10 false 'q' 'Q';
    KeyW "KeyW" 0x11 false 'w' 'W';
    KeyE "KeyE" 0x12 false 'e' 'E';
    KeyR "KeyR" 0x13 false 'r' 'R';
    KeyT "KeyT" 0x14 false 't' 'T';
    KeyY "KeyY" 0x15 false 'y' 'Y';
    KeyU "KeyU" 0x16 false 'u' 'U';
    KeyI "KeyI" 0x17 false 'i' 'I';
    KeyO "KeyO" 0x18 false 'o' 'O';
    KeyP "KeyP" 0x19 false 'p' 'P';
    BracketLeft "BracketLeft" 0x1A false '[' '{';
    BracketRight "BracketRight" 0x1B false ']' '}';
    Enter "Enter" 0x1C false '\0' '\0';
    ControlLeft "ControlLeft" 0x1D false '\0' '\0';
    KeyA "KeyA" 0x1E false 'a' 'A';
    KeyS "KeyS" 0x1F false 's' 'S';
    KeyD "KeyD" 0x20 false 'd' 'D';
    KeyF "KeyF" 0x21 false 'f' 'F';
    KeyG "KeyG" 0x22 false 'g' 'G';
    KeyH "KeyH" 0x23 false 'h' 'H';
    KeyJ "KeyJ" 0x24 false 'j' 'J';
    KeyK "KeyK" 0x25 false 'k' 'K';
    KeyL "KeyL" 0x26 false 'l' 'L';
    Semicolon "Semicolon" 0x27 false ';' ':';
    Quote "Quote" 0x28 false '\'' '"';
    Backquote "Backquote" 0x29 false '`' '~';
    ShiftLeft "ShiftLeft" 0x2A false '\0' '\0';
    Backslash "Backslash" 0x2B false '\\' '|';
    KeyZ "KeyZ" 0x2C false 'z' 'Z';
    KeyX "KeyX" 0x2D false 'x' 'X';
    KeyC "KeyC" 0x2E false 'c' 'C';
    KeyV "KeyV" 0x2F false 'v' 'V';
    KeyB "KeyB" 0x30 false 'b' 'B';
    KeyN "KeyN" 0x31 false 'n' 'N';
    KeyM "KeyM" 0x32 false 'm' 'M';
    Comma "Comma" 0x33 false ',' '<';
    Period "Period" 0x34 false '.' '>';
    Slash "Slash" 0x35 false '/' '?';
    ShiftRight "ShiftRight" 0x36 false '\0' '\0';
    NumpadMultiply "NumpadMultiply" 0x37 false '\0' '\0';
    AltLeft "AltLeft" 0x38 false '\0' '\0';
    Space "Space" 0x39 false ' ' ' ';
    CapsLock "CapsLock" 0x3A false '\0' '\0';
    F1 "F1" 0x3B false '\0' '\0';
    F2 "F2" 0x3C false '\0' '\0';
    F3 "F3" 0x3D false '\0' '\0';
    F4 "F4" 0x3E false '\0' '\0';
    F5 "F5" 0x3F false '\0' '\0';
    F6 "F6" 0x40 false '\0' '\0';
    F7 "F7" 0x41 false '\0' '\0';
    F8 "F8" 0x42 false '\0' '\0';
    F9 "F9" 0x43 false '\0' '\0';
    F10 "F10" 0x44 false '\0' '\0';
    // NumLock is E0 45 and Pause is a bare 45 as far as Windows reports it.
    Pause "Pause" 0x45 false '\0' '\0';
    NumLock "NumLock" 0x45 true '\0' '\0';
    ScrollLock "ScrollLock" 0x46 false '\0' '\0';
    Numpad7 "Numpad7" 0x47 false '\0' '\0';
    Numpad8 "Numpad8" 0x48 false '\0' '\0';
    Numpad9 "Numpad9" 0x49 false '\0' '\0';
    NumpadSubtract "NumpadSubtract" 0x4A false '\0' '\0';
    Numpad4 "Numpad4" 0x4B false '\0' '\0';
    Numpad5 "Numpad5" 0x4C false '\0' '\0';
    Numpad6 "Numpad6" 0x4D false '\0' '\0';
    NumpadAdd "NumpadAdd" 0x4E false '\0' '\0';
    Numpad1 "Numpad1" 0x4F false '\0' '\0';
    Numpad2 "Numpad2" 0x50 false '\0' '\0';
    Numpad3 "Numpad3" 0x51 false '\0' '\0';
    Numpad0 "Numpad0" 0x52 false '\0' '\0';
    NumpadDecimal "NumpadDecimal" 0x53 false '\0' '\0';
    // The ISO key between the left Shift and Z.
    IntlBackslash "IntlBackslash" 0x56 false '\0' '\0';
    F11 "F11" 0x57 false '\0' '\0';
    F12 "F12" 0x58 false '\0' '\0';
    NumpadEqual "NumpadEqual" 0x59 false '\0' '\0';
    F13 "F13" 0x64 false '\0' '\0';
    F14 "F14" 0x65 false '\0' '\0';
    F15 "F15" 0x66 false '\0' '\0';
    F16 "F16" 0x67 false '\0' '\0';
    F17 "F17" 0x68 false '\0' '\0';
    F18 "F18" 0x69 false '\0' '\0';
    F19 "F19" 0x6A false '\0' '\0';
    F20 "F20" 0x6B false '\0' '\0';
    F21 "F21" 0x6C false '\0' '\0';
    F22 "F22" 0x6D false '\0' '\0';
    F23 "F23" 0x6E false '\0' '\0';
    KanaMode "KanaMode" 0x70 false '\0' '\0';
    Lang2 "Lang2" 0x71 false '\0' '\0';
    Lang1 "Lang1" 0x72 false '\0' '\0';
    IntlRo "IntlRo" 0x73 false '\0' '\0';
    F24 "F24" 0x76 false '\0' '\0';
    Convert "Convert" 0x79 false '\0' '\0';
    NonConvert "NonConvert" 0x7B false '\0' '\0';
    IntlYen "IntlYen" 0x7D false '\0' '\0';
    NumpadComma "NumpadComma" 0x7E false '\0' '\0';

    MediaTrackPrevious "MediaTrackPrevious" 0x10 true '\0' '\0';
    MediaTrackNext "MediaTrackNext" 0x19 true '\0' '\0';
    NumpadEnter "NumpadEnter" 0x1C true '\0' '\0';
    ControlRight "ControlRight" 0x1D true '\0' '\0';
    AudioVolumeMute "AudioVolumeMute" 0x20 true '\0' '\0';
    LaunchApp2 "LaunchApp2" 0x21 true '\0' '\0';
    MediaPlayPause "MediaPlayPause" 0x22 true '\0' '\0';
    MediaStop "MediaStop" 0x24 true '\0' '\0';
    AudioVolumeDown "AudioVolumeDown" 0x2E true '\0' '\0';
    AudioVolumeUp "AudioVolumeUp" 0x30 true '\0' '\0';
    BrowserHome "BrowserHome" 0x32 true '\0' '\0';
    NumpadDivide "NumpadDivide" 0x35 true '\0' '\0';
    PrintScreen "PrintScreen" 0x37 true '\0' '\0';
    AltRight "AltRight" 0x38 true '\0' '\0';
    Home "Home" 0x47 true '\0' '\0';
    ArrowUp "ArrowUp" 0x48 true '\0' '\0';
    PageUp "PageUp" 0x49 true '\0' '\0';
    ArrowLeft "ArrowLeft" 0x4B true '\0' '\0';
    ArrowRight "ArrowRight" 0x4D true '\0' '\0';
    End "End" 0x4F true '\0' '\0';
    ArrowDown "ArrowDown" 0x50 true '\0' '\0';
    PageDown "PageDown" 0x51 true '\0' '\0';
    Insert "Insert" 0x52 true '\0' '\0';
    Delete "Delete" 0x53 true '\0' '\0';
    MetaLeft "MetaLeft" 0x5B true '\0' '\0';
    MetaRight "MetaRight" 0x5C true '\0' '\0';
    ContextMenu "ContextMenu" 0x5D true '\0' '\0';
    Power "Power" 0x5E true '\0' '\0';
    Sleep "Sleep" 0x5F true '\0' '\0';
    WakeUp "WakeUp" 0x63 true '\0' '\0';
    BrowserSearch "BrowserSearch" 0x65 true '\0' '\0';
    BrowserFavorites "BrowserFavorites" 0x66 true '\0' '\0';
    BrowserRefresh "BrowserRefresh" 0x67 true '\0' '\0';
    BrowserStop "BrowserStop" 0x68 true '\0' '\0';
    BrowserForward "BrowserForward" 0x69 true '\0' '\0';
    BrowserBack "BrowserBack" 0x6A true '\0' '\0';
    LaunchApp1 "LaunchApp1" 0x6B true '\0' '\0';
    LaunchMail "LaunchMail" 0x6C true '\0' '\0';
    MediaSelect "MediaSelect" 0x6D true '\0' '\0';
}

/// Scan codes that name a key without being its make code: `Alt+PrintScreen` reports the SysRq
/// scan code 0x54, and `MapVirtualKey` spells the prefixes out in the high byte.
const SYSRQ: u16 = 0x54;
const PAUSE_E1: u16 = 0xE11D;

impl PhysicalKey {
    /// Every key the table knows, in table order.
    pub fn all() -> impl Iterator<Item = PhysicalKey> {
        TABLE.iter().map(|entry| entry.0)
    }

    fn entry(self) -> &'static (PhysicalKey, &'static str, u16, bool, char, char) {
        // Every variant has exactly one row, in declaration order.
        &TABLE[self as usize]
    }

    /// The W3C `code` of the key (`"KeyA"`, `"Digit1"`, `"BracketLeft"`, `"ArrowUp"`).
    pub fn name(self) -> &'static str {
        self.entry().1
    }

    pub fn from_name(name: &str) -> Option<PhysicalKey> {
        TABLE.iter().find(|entry| entry.1 == name).map(|entry| entry.0)
    }

    /// The make code and whether it carries the `E0` prefix.
    pub fn scan_code(self) -> (u16, bool) {
        let entry = self.entry();
        (entry.2, entry.3)
    }

    /// What the key types on a US layout without Shift (`'a'`, `'1'`, `'['`, `' '`): the PC-101
    /// character that Ctrl shortcuts and the kitty protocol's base-layout key refer to. `None` for
    /// keys that type nothing.
    pub fn us_char(self) -> Option<char> {
        Some(self.entry().4).filter(|c| *c != '\0')
    }

    /// What the key types on a US layout with Shift held.
    pub fn us_shifted_char(self) -> Option<char> {
        Some(self.entry().5).filter(|c| *c != '\0')
    }

    /// The key is part of the main block that types characters (letters, digits, punctuation and
    /// the space bar).
    pub fn types_text(self) -> bool {
        self.us_char().is_some()
    }
}

/// The key at a scan code, or `None` when the code is unknown, zero (WinUI reports 0 for Tab and
/// for synthesized input), or the fake Shift Windows injects around keypad keys (`E0 2A`).
///
/// `scan_code` is `KeyStatus.ScanCode`; `extended` is `KeyStatus.IsExtendedKey`. A code that
/// already carries the prefix in its high byte (`0xE04B`, as `MapVirtualKey` spells it) is
/// accepted as well.
pub fn physical_key(scan_code: u16, extended: bool) -> Option<PhysicalKey> {
    let (code, extended) = match scan_code {
        PAUSE_E1 => return Some(PhysicalKey::Pause),
        0x0100.. => match scan_code >> 8 {
            0xE0 => (scan_code & 0xFF, true),
            _ => return None,
        },
        _ => (scan_code, extended),
    };
    if code == SYSRQ && !extended {
        return Some(PhysicalKey::PrintScreen);
    }
    TABLE.iter().find(|entry| entry.2 == code && entry.3 == extended).map(|entry| entry.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_sits_at_its_own_table_row() {
        for (index, entry) in TABLE.iter().enumerate() {
            assert_eq!(entry.0 as usize, index, "{}", entry.1);
        }
    }

    #[test]
    fn scan_codes_and_names_are_unique() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!((a.2, a.3), (b.2, b.3), "{} and {} share a scan code", a.1, b.1);
                assert_ne!(a.1, b.1);
            }
        }
    }

    #[test]
    fn names_round_trip_through_the_scan_code() {
        for key in PhysicalKey::all() {
            assert_eq!(PhysicalKey::from_name(key.name()), Some(key));
            let (code, extended) = key.scan_code();
            assert_eq!(physical_key(code, extended), Some(key), "{}", key.name());
        }
    }

    #[test]
    fn prefixed_spellings_are_accepted() {
        assert_eq!(physical_key(0xE04B, false), Some(PhysicalKey::ArrowLeft));
        assert_eq!(physical_key(0xE11D, false), Some(PhysicalKey::Pause));
        assert_eq!(physical_key(0xE1FF, false), None);
        assert_eq!(physical_key(0x0200, false), None);
    }

    #[test]
    fn unknown_zero_and_fake_shift_codes_name_nothing() {
        assert_eq!(physical_key(0, false), None);
        assert_eq!(physical_key(0x2A, true), None);
        assert_eq!(physical_key(0x36, true), None);
        assert_eq!(physical_key(0x1E, true), None);
    }

    #[test]
    fn alt_print_screen_reports_sysrq() {
        assert_eq!(physical_key(0x54, false), Some(PhysicalKey::PrintScreen));
        assert_eq!(physical_key(0x37, true), Some(PhysicalKey::PrintScreen));
        assert_eq!(physical_key(0x37, false), Some(PhysicalKey::NumpadMultiply));
    }

    #[test]
    fn the_prefix_separates_the_navigation_cluster_from_the_keypad() {
        assert_eq!(physical_key(0x4B, true), Some(PhysicalKey::ArrowLeft));
        assert_eq!(physical_key(0x4B, false), Some(PhysicalKey::Numpad4));
        assert_eq!(physical_key(0x1C, true), Some(PhysicalKey::NumpadEnter));
        assert_eq!(physical_key(0x1C, false), Some(PhysicalKey::Enter));
        assert_eq!(physical_key(0x35, true), Some(PhysicalKey::NumpadDivide));
        assert_eq!(physical_key(0x38, true), Some(PhysicalKey::AltRight));
        assert_eq!(physical_key(0x45, true), Some(PhysicalKey::NumLock));
        assert_eq!(physical_key(0x45, false), Some(PhysicalKey::Pause));
    }

    #[test]
    fn the_us_characters_are_the_pc101_board() {
        let typed: String = (0x02..=0x0D).filter_map(|code| physical_key(code, false)?.us_char()).collect();
        assert_eq!(typed, "1234567890-=");
        let shifted: String = (0x02..=0x0D).filter_map(|code| physical_key(code, false)?.us_shifted_char()).collect();
        assert_eq!(shifted, "!@#$%^&*()_+");
        let top: String = (0x10..=0x1B).filter_map(|code| physical_key(code, false)?.us_char()).collect();
        assert_eq!(top, "qwertyuiop[]");
        let home: String = (0x1E..=0x29).filter_map(|code| physical_key(code, false)?.us_char()).collect();
        assert_eq!(home, "asdfghjkl;'`");
        let bottom: String = (0x2B..=0x35).filter_map(|code| physical_key(code, false)?.us_char()).collect();
        assert_eq!(bottom, "\\zxcvbnm,./");
        assert_eq!(PhysicalKey::Space.us_char(), Some(' '));
        assert_eq!(PhysicalKey::Enter.us_char(), None);
        assert_eq!(PhysicalKey::Numpad5.us_char(), None);
    }

    #[test]
    fn shifted_letters_are_the_upper_case_of_the_unshifted_ones() {
        for key in PhysicalKey::all().filter(|key| key.us_char().is_some_and(char::is_alphabetic)) {
            assert_eq!(key.us_shifted_char(), key.us_char().map(|c| c.to_ascii_uppercase()), "{}", key.name());
        }
    }
}
