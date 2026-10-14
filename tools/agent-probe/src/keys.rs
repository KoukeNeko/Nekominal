//! Key chords as the WinUI keyboard stream delivers them, so the harness exercises the same
//! `KeyAssembler` the application uses.

use nekominal_input::{Key, KeyStroke, KeyboardEvent, KeypadKey, Mods, PhysicalKey};

/// A key and the modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySpec {
    pub key: Key,
    pub mods: Mods,
}

impl KeySpec {
    pub fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods }
    }

    /// `"enter"`, `"shift+enter"`, `"ctrl+w"`, `"alt+v"`, `"ctrl+backspace"`, `"up"`, `"pgup"`, `"f5"`, `"a"`, `"?"`.
    /// A single upper-case letter means Shift plus the letter.
    pub fn parse(text: &str) -> Result<KeySpec, String> {
        let mut mods = Mods::NONE;
        let mut parts: Vec<&str> = text.split('+').collect();
        // "ctrl++" is Ctrl and the plus key.
        if text.ends_with("++") {
            parts.truncate(parts.len().saturating_sub(2));
            parts.push("+");
        }
        let key_name = parts.pop().filter(|name| !name.is_empty()).ok_or_else(|| format!("no key in {text:?}"))?;
        for part in parts {
            mods |= match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Mods::CTRL,
                "alt" | "option" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "win" | "super" => Mods::SUPER,
                other => return Err(format!("unknown modifier {other:?} in {text:?}")),
            };
        }
        let key = match key_name.to_ascii_lowercase().as_str() {
            "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" | "bs" => Key::Backspace,
            "esc" | "escape" => Key::Escape,
            "space" => Key::Char(' '),
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pgup" | "pageup" => Key::PageUp,
            "pgdn" | "pagedown" => Key::PageDown,
            "insert" => Key::Insert,
            "delete" | "del" => Key::Delete,
            name if name.len() >= 2 && name.starts_with('f') && name[1..].chars().all(|c| c.is_ascii_digit()) => {
                let number: u8 = name[1..].parse().map_err(|_| format!("bad function key {name:?}"))?;
                Key::F(number)
            }
            _ => {
                let mut chars = key_name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => {
                        if c.is_uppercase() {
                            mods |= Mods::SHIFT;
                        }
                        Key::Char(c.to_lowercase().next().unwrap_or(c))
                    }
                    _ => return Err(format!("unknown key {key_name:?} in {text:?}")),
                }
            }
        };
        Ok(KeySpec { key, mods })
    }

    /// Enter, keypad Enter and the two control characters that mean the same (Ctrl+M is CR, Ctrl+J is LF):
    /// the keys that can submit a prompt.
    pub fn is_enter_like(&self) -> bool {
        match self.key {
            Key::Enter | Key::Keypad(KeypadKey::Enter) => true,
            Key::Char(c) => self.mods.contains(Mods::CTRL) && matches!(c, 'm' | 'j'),
            _ => false,
        }
    }
}

const OEM: &[(char, i32)] = &[
    (';', 0xBA),
    ('=', 0xBB),
    (',', 0xBC),
    ('-', 0xBD),
    ('.', 0xBE),
    ('/', 0xBF),
    ('`', 0xC0),
    ('[', 0xDB),
    ('\\', 0xDC),
    (']', 0xDD),
    ('\'', 0xDE),
];

/// Virtual key code and the `E0` prefix of a key, as WinUI reports them.
fn vk_of(key: Key) -> Option<(i32, bool)> {
    Some(match key {
        Key::Char(c @ 'a'..='z') => (i32::from(c.to_ascii_uppercase() as u8), false),
        Key::Char(c @ '0'..='9') => (i32::from(c as u8), false),
        Key::Char(' ') => (0x20, false),
        Key::Char(c) => (OEM.iter().find(|(oem, _)| *oem == c)?.1, false),
        Key::Enter => (0x0D, false),
        Key::Tab => (0x09, false),
        Key::Backspace => (0x08, false),
        Key::Escape => (0x1B, false),
        Key::Up => (0x26, true),
        Key::Down => (0x28, true),
        Key::Left => (0x25, true),
        Key::Right => (0x27, true),
        Key::Home => (0x24, true),
        Key::End => (0x23, true),
        Key::PageUp => (0x21, true),
        Key::PageDown => (0x22, true),
        Key::Insert => (0x2D, true),
        Key::Delete => (0x2E, true),
        Key::F(n @ 1..=24) => (0x6F + i32::from(n), false),
        _ => return None,
    })
}

fn scan_of(key: Key) -> u16 {
    match key {
        Key::Char(c) => PhysicalKey::all().find(|k| k.us_char() == Some(c)).map_or(0, |k| k.scan_code().0),
        // The assembler fills the scan code of named keys from their US position.
        _ => 0,
    }
}

/// The character a plain or Shift-modified text key types on a US layout.
fn typed_char(c: char, shift: bool) -> char {
    if !shift {
        return c;
    }
    if c.is_ascii_alphabetic() {
        return c.to_ascii_uppercase();
    }
    PhysicalKey::all().find(|k| k.us_char() == Some(c)).and_then(PhysicalKey::us_shifted_char).unwrap_or(c)
}

/// `(vk, scan code, extended)` of a modifier, as WinUI reports the left-hand keys.
const MODIFIER_KEYS: [(Mods, i32, u16); 4] =
    [(Mods::CTRL, 0x11, 0x1D), (Mods::ALT, 0x12, 0x38), (Mods::SHIFT, 0x10, 0x2A), (Mods::SUPER, 0x5B, 0x5B)];

/// The keyboard events of pressing and releasing `spec`: modifiers down, the key (and its character for a
/// plain text key), the key up, modifiers up.
pub fn events_for(spec: &KeySpec) -> Result<Vec<KeyboardEvent>, String> {
    let (vk, extended) = vk_of(spec.key).ok_or_else(|| format!("no virtual key for {:?}", spec.key))?;
    let scan = scan_of(spec.key);
    let mut events = Vec::new();
    let mut held = Mods::NONE;
    let mut pressed = Vec::new();
    for (bit, modifier_vk, modifier_scan) in MODIFIER_KEYS {
        if spec.mods.contains(bit) {
            held |= bit;
            pressed.push((bit, modifier_vk, modifier_scan));
            events.push(KeyboardEvent::KeyDown(KeyStroke::new(modifier_vk, modifier_scan, false, held)));
        }
    }
    let stroke = KeyStroke::new(vk, scan, extended, spec.mods);
    events.push(KeyboardEvent::KeyDown(stroke));
    if let Key::Char(c) = spec.key
        && !spec.mods.intersects(Mods::CTRL | Mods::ALT | Mods::SUPER)
    {
        let mut units = [0u16; 2];
        for unit in typed_char(c, spec.mods.contains(Mods::SHIFT)).encode_utf16(&mut units) {
            events.push(KeyboardEvent::CharacterReceived(*unit));
        }
        events.push(KeyboardEvent::Flush);
    }
    events.push(KeyboardEvent::KeyUp(stroke));
    for (bit, modifier_vk, modifier_scan) in pressed.into_iter().rev() {
        held = held.without(bit);
        events.push(KeyboardEvent::KeyUp(KeyStroke::new(modifier_vk, modifier_scan, false, held)));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse() {
        assert_eq!(KeySpec::parse("enter").unwrap(), KeySpec::new(Key::Enter, Mods::NONE));
        assert_eq!(KeySpec::parse("shift+enter").unwrap(), KeySpec::new(Key::Enter, Mods::SHIFT));
        assert_eq!(KeySpec::parse("Ctrl+Backspace").unwrap(), KeySpec::new(Key::Backspace, Mods::CTRL));
        assert_eq!(
            KeySpec::parse("ctrl+alt+shift+f5").unwrap(),
            KeySpec::new(Key::F(5), Mods::CTRL | Mods::ALT | Mods::SHIFT)
        );
        assert_eq!(KeySpec::parse("A").unwrap(), KeySpec::new(Key::Char('a'), Mods::SHIFT));
        assert_eq!(KeySpec::parse("ctrl++").unwrap(), KeySpec::new(Key::Char('+'), Mods::CTRL));
        assert_eq!(KeySpec::parse("pgup").unwrap(), KeySpec::new(Key::PageUp, Mods::NONE));
        assert!(KeySpec::parse("hyper+a").is_err());
        assert!(KeySpec::parse("").is_err());
        assert!(KeySpec::parse("ctrl+").is_err());
    }

    #[test]
    fn only_the_keys_that_can_submit_are_enter_like() {
        for text in ["enter", "shift+enter", "alt+enter", "ctrl+enter", "ctrl+j", "ctrl+m"] {
            assert!(KeySpec::parse(text).unwrap().is_enter_like(), "{text}");
        }
        for text in ["j", "ctrl+w", "tab", "esc", "ctrl+c", "alt+v", "backspace"] {
            assert!(!KeySpec::parse(text).unwrap().is_enter_like(), "{text}");
        }
    }

    #[test]
    fn a_plain_letter_is_key_down_character_flush_key_up() {
        let events = events_for(&KeySpec::parse("a").unwrap()).unwrap();
        assert!(matches!(events[0], KeyboardEvent::KeyDown(s) if s.vk == 0x41 && s.scan_code == 0x1E));
        assert_eq!(events[1], KeyboardEvent::CharacterReceived(u16::from(b'a')));
        assert_eq!(events[2], KeyboardEvent::Flush);
        assert!(matches!(events[3], KeyboardEvent::KeyUp(_)));
    }

    #[test]
    fn a_chord_brackets_the_key_with_its_modifiers_and_sends_no_character() {
        let events = events_for(&KeySpec::parse("ctrl+shift+w").unwrap()).unwrap();
        assert_eq!(events.len(), 6, "{events:?}");
        assert!(matches!(events[0], KeyboardEvent::KeyDown(s) if s.vk == 0x11));
        assert!(matches!(events[1], KeyboardEvent::KeyDown(s) if s.vk == 0x10));
        assert!(matches!(events[2], KeyboardEvent::KeyDown(s) if s.vk == 0x57 && s.mods == (Mods::CTRL | Mods::SHIFT)));
        assert!(matches!(events[4], KeyboardEvent::KeyUp(s) if s.vk == 0x10));
        assert!(matches!(events[5], KeyboardEvent::KeyUp(s) if s.vk == 0x11 && s.mods == Mods::NONE));
    }

    #[test]
    fn shift_types_the_shifted_symbol() {
        let events = events_for(&KeySpec::new(Key::Char('1'), Mods::SHIFT)).unwrap();
        assert!(events.contains(&KeyboardEvent::CharacterReceived(u16::from(b'!'))));
    }
}
