//! The kitty keyboard protocol, all five enhancement flags.
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>
//!
//! Event routing follows the spec text, kitty's `key_encoding.c` and Windows Terminal's
//! `TerminalInput::_encodeKitty`; where they differ the choice is noted at the rule.

use super::{keypad_code, Norm};
use crate::{
    key::{Key, KeyEvent, KeypadKey, MediaKey, ModifierKey, Mods},
    modes::{InputModes, KittyFlags},
};

pub(super) enum Outcome {
    Bytes(Vec<u8>),
    /// The event is deliberately not reported.
    Skip,
    /// Not affected by the active flags: use the legacy encoding.
    Legacy,
}

/// One `CSI code[:shifted[:base]] [; mods[:event]] [; text] final` sequence.
struct Csi {
    code: u32,
    shifted: Option<u32>,
    base: Option<u32>,
    /// Modifier bits without the `+ 1`.
    mods: u8,
    /// 1 press, 2 repeat, 3 release.
    event: u8,
    text: Vec<u32>,
    final_byte: char,
}

impl Csi {
    fn new(code: u32, final_byte: char) -> Csi {
        Csi { code, shifted: None, base: None, mods: 0, event: 1, text: Vec::new(), final_byte }
    }

    /// Field layout of kitty's `serialize()`: the key number is omitted only when it is 1 and
    /// nothing follows it (`CSI A`); the modifier field is empty when only text follows.
    fn serialize(&self) -> Vec<u8> {
        let second = self.mods != 0 || self.event != 1;
        let third = !self.text.is_empty();
        let alternates = self.shifted.is_some() || self.base.is_some();
        let mut out = String::from("\x1b[");
        if self.code != 1 || alternates || second || third {
            out += &self.code.to_string();
        }
        if alternates {
            out.push(':');
            if let Some(shifted) = self.shifted {
                out += &shifted.to_string();
            }
            if let Some(base) = self.base {
                out.push(':');
                out += &base.to_string();
            }
        }
        if second || third {
            out.push(';');
            if second {
                out += &(u32::from(self.mods) + 1).to_string();
                if self.event != 1 {
                    out.push(':');
                    out += &self.event.to_string();
                }
            }
        }
        if third {
            out.push(';');
            let text: Vec<String> = self.text.iter().map(u32::to_string).collect();
            out += &text.join(":");
        }
        out.push(self.final_byte);
        out.into_bytes()
    }
}

struct Ctx<'a, 'n> {
    n: &'a Norm<'n>,
    flags: KittyFlags,
    locks: bool,
    event: u8,
}

impl Ctx<'_, '_> {
    fn disambiguate(&self) -> bool {
        self.flags.contains(KittyFlags::DISAMBIGUATE)
    }

    fn all_keys(&self) -> bool {
        self.flags.contains(KittyFlags::REPORT_ALL_KEYS)
    }

    fn release(&self) -> bool {
        self.n.event == KeyEvent::Release
    }

    /// The modifier bits of an escape-coded event. Lock bits only when asked for: under "report
    /// all keys" (spec), or via `lock_modifiers_in_disambiguate`.
    fn mod_bits(&self) -> u8 {
        let mut bits = self.n.held();
        if self.locks {
            bits |= self.n.mods.only(Mods::LOCKS);
        }
        bits.bits()
    }

    fn csi(&self, code: u32, final_byte: char) -> Csi {
        let mut csi = Csi::new(code, final_byte);
        csi.mods = self.mod_bits();
        csi.event = self.event;
        csi
    }

    fn bytes(&self, code: u32, final_byte: char) -> Outcome {
        Outcome::Bytes(self.csi(code, final_byte).serialize())
    }
}

/// Functional keys that keep their legacy `CSI 1 ; m X` / `CSI n ; m ~` shape.
fn letter_form(key: Key) -> Option<(u32, char)> {
    Some(match key {
        Key::Up => (1, 'A'),
        Key::Down => (1, 'B'),
        Key::Right => (1, 'C'),
        Key::Left => (1, 'D'),
        Key::Home => (1, 'H'),
        Key::End => (1, 'F'),
        Key::Insert => (2, '~'),
        Key::Delete => (3, '~'),
        Key::PageUp => (5, '~'),
        Key::PageDown => (6, '~'),
        Key::F(1) => (1, 'P'),
        Key::F(2) => (1, 'Q'),
        Key::F(3) => (13, '~'),
        Key::F(4) => (1, 'S'),
        Key::F(5) => (15, '~'),
        Key::F(6) => (17, '~'),
        Key::F(7) => (18, '~'),
        Key::F(8) => (19, '~'),
        Key::F(9) => (20, '~'),
        Key::F(10) => (21, '~'),
        Key::F(11) => (23, '~'),
        Key::F(12) => (24, '~'),
        Key::Keypad(KeypadKey::Begin) => (1, 'E'),
        _ => return None,
    })
}

fn media_code(key: MediaKey) -> u32 {
    match key {
        MediaKey::Play => 57428,
        MediaKey::Pause => 57429,
        MediaKey::PlayPause => 57430,
        MediaKey::Reverse => 57431,
        MediaKey::Stop => 57432,
        MediaKey::FastForward => 57433,
        MediaKey::Rewind => 57434,
        MediaKey::TrackNext => 57435,
        MediaKey::TrackPrevious => 57436,
        MediaKey::Record => 57437,
        MediaKey::LowerVolume => 57438,
        MediaKey::RaiseVolume => 57439,
        MediaKey::MuteVolume => 57440,
    }
}

fn modifier_code(key: ModifierKey) -> u32 {
    match key {
        ModifierKey::LeftShift => 57441,
        ModifierKey::LeftControl => 57442,
        ModifierKey::LeftAlt => 57443,
        ModifierKey::LeftSuper => 57444,
        ModifierKey::LeftHyper => 57445,
        ModifierKey::LeftMeta => 57446,
        ModifierKey::RightShift => 57447,
        ModifierKey::RightControl => 57448,
        ModifierKey::RightAlt => 57449,
        ModifierKey::RightSuper => 57450,
        ModifierKey::RightHyper => 57451,
        ModifierKey::RightMeta => 57452,
        ModifierKey::IsoLevel3Shift => 57453,
        ModifierKey::IsoLevel5Shift => 57454,
    }
}

/// Functional keys that are only ever `CSI number u`.
fn csi_u_code(key: Key) -> Option<u32> {
    Some(match key {
        Key::F(n @ 13..=35) => 57376 + u32::from(n) - 13,
        Key::Keypad(k) if k.text_char().is_none() && k != KeypadKey::Begin => keypad_code(k),
        Key::PrintScreen => 57361,
        Key::Pause => 57362,
        Key::Menu => 57363,
        Key::Media(m) => media_code(m),
        _ => return None,
    })
}

pub(super) fn encode(n: &Norm, modes: &InputModes) -> Outcome {
    let flags = modes.kitty;
    let report_events = flags.contains(KittyFlags::REPORT_EVENTS);

    // Releases exist only on request; a repeat is a press otherwise.
    if n.event == KeyEvent::Release && !report_events {
        return Outcome::Skip;
    }
    let event = if report_events {
        match n.event {
            KeyEvent::Press => 1,
            KeyEvent::Repeat => 2,
            KeyEvent::Release => 3,
        }
    } else {
        1
    };
    let cx = Ctx {
        n,
        flags,
        locks: flags.contains(KittyFlags::REPORT_ALL_KEYS) || modes.lock_modifiers_in_disambiguate,
        event,
    };

    match n.key {
        Key::Dead => Outcome::Skip,

        // Modifier and lock keys exist only as "report all keys" events. Windows auto-repeats
        // held modifier keys; those repeats are noise.
        Key::Modifier(key) => modifier_key(&cx, modifier_code(key)),
        Key::CapsLock => modifier_key(&cx, 57358),
        Key::ScrollLock => modifier_key(&cx, 57359),
        Key::NumLock => modifier_key(&cx, 57360),

        Key::Char(_) | Key::Unidentified => text_key(&cx, 0),
        Key::Keypad(k) if k.text_char().is_some() => text_key(&cx, keypad_code(k)),

        // Enter, Tab and Backspace stay legacy bytes until modified, so a shell survives a
        // program that crashed with the flags still on.
        Key::Enter => c0_key(&cx, 13),
        Key::Tab => c0_key(&cx, 9),
        Key::Backspace => c0_key(&cx, 127),

        Key::Escape => {
            if cx.release() || cx.disambiguate() || cx.all_keys() {
                cx.bytes(27, 'u')
            } else {
                Outcome::Legacy
            }
        }

        key => {
            if let Some((code, final_byte)) = letter_form(key) {
                // Any of disambiguate / events / all keys switches these to the CSI form (SS3 is
                // not used), with an event type when requested.
                cx.bytes(code, final_byte)
            } else if let Some(code) = csi_u_code(key) {
                if cx.release() || cx.disambiguate() || cx.all_keys() {
                    cx.bytes(code, 'u')
                } else {
                    Outcome::Legacy
                }
            } else {
                Outcome::Legacy
            }
        }
    }
}

fn modifier_key(cx: &Ctx, code: u32) -> Outcome {
    if !cx.all_keys() || cx.n.event == KeyEvent::Repeat {
        return Outcome::Skip;
    }
    cx.bytes(code, 'u')
}

fn c0_key(cx: &Ctx, code: u32) -> Outcome {
    if cx.all_keys() {
        return cx.bytes(code, 'u');
    }
    // No release events for these keys unless everything is reported.
    if cx.release() {
        return Outcome::Skip;
    }
    // Shift counts: Shift+Enter is the whole point.
    if cx.disambiguate() && !cx.n.held().is_empty() {
        return cx.bytes(code, 'u');
    }
    Outcome::Legacy
}

/// Keys that type text: letters, digits, symbols, keypad digits, unnamed keys with text.
/// `keypad` is the functional key number of a keypad key, 0 otherwise.
fn text_key(cx: &Ctx, keypad: u32) -> Outcome {
    let n = cx.n;
    let (mut code, is_key) = match n.key {
        Key::Char(c) => (c as u32, true),
        _ => (keypad, keypad != 0),
    };

    // The text of the event, for the associated-text field: what the OS delivered, or the
    // character of a plain key that was typed without a chord.
    let text: Option<String> = match n.text {
        Some(t) => Some(t.to_owned()),
        None if is_key && !n.chord() => n.typed_char().map(String::from),
        None => None,
    };

    // Plain and Shift-only typing is sent as text; everything else becomes an escape code.
    let escape = cx.all_keys() || (cx.disambiguate() && n.chord()) || cx.release();
    if !escape {
        return Outcome::Legacy;
    }

    if !is_key {
        // No key behind the text (a composed string): kitty's key number 0, which is only
        // meaningful with the text attached.
        if cx.release() {
            return Outcome::Skip;
        }
        let reportable = cx.all_keys() && cx.flags.contains(KittyFlags::REPORT_TEXT) && text.is_some();
        if !reportable {
            return Outcome::Legacy;
        }
        code = 0;
    }

    let mut csi = cx.csi(code, 'u');
    if cx.flags.contains(KittyFlags::REPORT_ALTERNATES) && is_key && keypad == 0 {
        csi.shifted = n.shifted_char().map(|c| c as u32);
        csi.base = n.base_char().map(|c| c as u32);
    }
    if cx.flags.contains(KittyFlags::REPORT_TEXT)
        && !cx.release()
        && let Some(text) = &text
    {
        csi.text = text.chars().map(|c| c as u32).collect();
    }
    Outcome::Bytes(csi.serialize())
}
