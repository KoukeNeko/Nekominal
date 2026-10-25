//! `KeyAssembler`: the WinUI 3 keyboard event stream in, the bytes for the PTY out.
//!
//! WinUI hands a terminal four kinds of keyboard events that do not line up one to one with key
//! presses: `KeyDown`, `CharacterReceived` (one UTF-16 unit, only when the key was not handled and no
//! IME is composing), `KeyUp`, and losing focus. This module turns that stream into [`KeyInput`]s
//! and Win32 key records and encodes them, so the window code stays a thin adapter and every rule
//! below is covered by a sequence test that feeds recorded event shapes and compares bytes.
//!
//! The rules:
//!
//! * **Text keys are not handled.** XAML raises `CharacterReceived` only for a key that was left
//!   alone, so a `KeyDown` of a letter, digit, punctuation key or the space bar is held (`pending`)
//!   until the character of the same dispatch round arrives, and the two are sent together: the
//!   Win32 record needs the character (`PSReadLine` reads it from there) and the kitty protocol
//!   reports it. A key that never produces a character (a dead key) sends nothing.
//! * **Everything else is encoded from the key event and handled**: Enter, Tab, Escape, Backspace,
//!   cursor and editing keys, F1..F24, Menu, modifier and lock keys, and any text key with Ctrl, Alt
//!   or Windows held.
//! * **AltGr is Ctrl+Alt with a character.** Windows reports it as a left Ctrl press followed by a
//!   right Alt press a few milliseconds apart. That pair is the evidence ([`ALTGR_WINDOW_MS`]); Ctrl+Alt
//!   with a printable character and no such evidence is AltGr too, and Ctrl+Alt without a character
//!   is a chord.
//! * **`VK_PROCESSKEY` and `VK_PACKET` are text, never keys.** Characters nobody asked for (injected
//!   Unicode, the result of Alt+numpad entry) go out as UTF-8 text.
//! * **A release is sent only for a press that was sent**, on the same route, so a key whose press a
//!   dialog consumed, an IME swallowed or a route change split does not leak a lone release.
//! * **Focus loss releases everything** that is still down, so no program sees a stuck modifier.
//!
//! The assembler reads no clock and no OS state: the host supplies a modifier snapshot and a
//! timestamp with each event.

use crate::{
    key::{Key, KeyEvent, KeyInput, KeypadKey, ModifierKey, Mods, is_altgr, is_control},
    keyboard::{encode_text, lower, upper},
    modes::InputModes,
    scancode::{PhysicalKey, physical_key},
    utf16::Utf16Assembler,
    vk::{KeyClass, classify, default_scan_code},
    win32::{ConptyRoute, Win32Key, conpty_route, control_state, encode_key_conpty},
};

/// How soon after the left Ctrl press a right Alt press still counts as the pair Windows
/// synthesizes for AltGr.
pub const ALTGR_WINDOW_MS: u32 = 50;

/// The most keys tracked as held at once; the oldest is forgotten beyond that.
const MAX_HELD_KEYS: usize = 32;

/// A key transition as WinUI 3 reports it (`KeyRoutedEventArgs` plus the keyboard state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyStroke {
    /// `args.Key().0`: the Win32 virtual-key code.
    pub vk: i32,
    /// `KeyStatus.ScanCode`; 0 when WinUI does not know it (Tab).
    pub scan_code: u16,
    /// `KeyStatus.IsExtendedKey`.
    pub extended: bool,
    /// `KeyStatus.WasKeyDown`, on a `KeyDown`: the key is auto-repeating.
    pub repeat: bool,
    /// `KeyStatus.RepeatCount`.
    pub repeat_count: u16,
    /// The modifier and lock state after this event, as
    /// `InputKeyboardSource.GetKeyStateForCurrentThread` reports it from inside the handler: Shift,
    /// Ctrl, Alt and the Windows key held, Caps Lock and Num Lock toggled.
    pub mods: Mods,
    /// Milliseconds on any monotonic clock (`GetTickCount` wraps harmlessly). Only differences of
    /// nearby events are used.
    pub time_ms: u32,
}

impl KeyStroke {
    pub fn new(vk: i32, scan_code: u16, extended: bool, mods: Mods) -> Self {
        KeyStroke { vk, scan_code, extended, repeat: false, repeat_count: 1, mods, time_ms: 0 }
    }

    pub fn with_repeat(mut self) -> Self {
        self.repeat = true;
        self
    }

    pub fn with_time(mut self, time_ms: u32) -> Self {
        self.time_ms = time_ms;
        self
    }

    /// The key by position, for shortcuts that must not depend on the layout.
    pub fn physical_key(&self) -> Option<PhysicalKey> {
        physical_key(self.scan_code, self.extended)
    }
}

/// One keyboard event of the WinUI stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardEvent {
    KeyDown(KeyStroke),
    KeyUp(KeyStroke),
    /// `CharacterReceived`: one UTF-16 code unit.
    CharacterReceived(u16),
    /// The end of the dispatch round that delivered a `KeyDown` with [`KeyOutput::pending`]: no
    /// character is coming for the held key any more.
    Flush,
    /// The pane or the window lost keyboard focus.
    FocusLost,
}

/// What to do about one event.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct KeyOutput {
    /// Bytes for the PTY, in order. Empty when nothing is sent.
    pub bytes: Vec<u8>,
    /// `KeyDown` and `KeyUp`: call `SetHandled(true)`. False for text keys (XAML must raise
    /// `CharacterReceived`), IME and injected-text keys, and keys that are not the terminal's (media
    /// keys); always true for `CharacterReceived`.
    pub handled: bool,
    /// `KeyDown` only: a text key is waiting for its character. Post a [`KeyboardEvent::Flush`] to
    /// the dispatcher queue so a key that produces no character (a dead key, an unmatched Ctrl+Alt
    /// chord) is resolved without waiting for the next keyboard event.
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingKind {
    /// No Ctrl, Alt or Windows key held.
    Plain,
    /// Ctrl+Alt after the AltGr pair: a character is expected, without one nothing is sent.
    AltGr,
    /// Ctrl+Alt without the pair: AltGr if a printable character arrives, a chord otherwise.
    CtrlAlt,
}

#[derive(Debug)]
struct Pending {
    stroke: KeyStroke,
    identity: Key,
    kind: PendingKind,
}

/// Names a physical key the way a Win32 console record does: the virtual key (the generic code for
/// a modifier, whatever the side), the scan code and the `E0` prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyId {
    vk: u16,
    scan_code: u16,
    extended: bool,
}

/// A key that is down: what to send when it goes up.
#[derive(Debug)]
struct HeldKey {
    id: KeyId,
    key: Key,
    /// The route the press went out on; `None` when nothing was sent.
    sent: Option<ConptyRoute>,
    altgr: bool,
    /// The record's `UnicodeChar`, repeated on the release.
    unicode: u16,
    base_layout_key: Option<char>,
}

/// Stateful translation of [`KeyboardEvent`]s into PTY bytes. One per pane: it remembers held keys,
/// the modifier sides, a high surrogate and a text key waiting for its character.
#[derive(Debug, Default)]
pub struct KeyAssembler {
    utf16: Utf16Assembler,
    pending: Option<Pending>,
    held: Vec<HeldKey>,
    /// Which modifier keys are down, one bit per [`modifier_bit`].
    modifiers: u16,
    left_ctrl_pressed_at: u32,
    altgr: bool,
}

fn modifier_bit(key: ModifierKey) -> u16 {
    match key {
        ModifierKey::LeftShift => 1,
        ModifierKey::RightShift => 1 << 1,
        ModifierKey::LeftControl => 1 << 2,
        ModifierKey::RightControl => 1 << 3,
        ModifierKey::LeftAlt => 1 << 4,
        ModifierKey::RightAlt => 1 << 5,
        ModifierKey::LeftSuper => 1 << 6,
        ModifierKey::RightSuper => 1 << 7,
        _ => 0,
    }
}

/// The key of the same kind on the other side.
fn other_side(key: ModifierKey) -> ModifierKey {
    use ModifierKey::*;
    match key {
        LeftShift => RightShift,
        RightShift => LeftShift,
        LeftControl => RightControl,
        RightControl => LeftControl,
        LeftAlt => RightAlt,
        RightAlt => LeftAlt,
        LeftSuper => RightSuper,
        RightSuper => LeftSuper,
        other => other,
    }
}

/// The virtual key a Win32 console record names a modifier by: the generic code, with the side in the
/// scan code and the `E0` prefix.
fn modifier_record_vk(key: ModifierKey) -> u16 {
    use ModifierKey::*;
    match key {
        LeftShift | RightShift => 0x10,
        LeftControl | RightControl => 0x11,
        LeftAlt | RightAlt => 0x12,
        LeftSuper => 0x5B,
        RightSuper => 0x5C,
        _ => 0,
    }
}

fn modifier_physical(key: ModifierKey) -> Option<PhysicalKey> {
    use ModifierKey::*;
    Some(match key {
        LeftShift => PhysicalKey::ShiftLeft,
        RightShift => PhysicalKey::ShiftRight,
        LeftControl => PhysicalKey::ControlLeft,
        RightControl => PhysicalKey::ControlRight,
        LeftAlt => PhysicalKey::AltLeft,
        RightAlt => PhysicalKey::AltRight,
        LeftSuper => PhysicalKey::MetaLeft,
        RightSuper => PhysicalKey::MetaRight,
        _ => return None,
    })
}

/// The `UnicodeChar` of a Win32 key record for a key press the OS produced no text for, as the
/// console reports it: control characters for Ctrl chords, the character itself for Alt chords,
/// nothing for Ctrl+Alt. Reproduces the records the OpenConsole differential test feeds in.
fn console_unicode(key: Key, mods: Mods, physical: Option<PhysicalKey>) -> u16 {
    let (ctrl, alt, shift) = (mods.contains(Mods::CTRL), mods.contains(Mods::ALT), mods.contains(Mods::SHIFT));
    let unit = |c: char| u16::try_from(u32::from(c)).unwrap_or(0);
    match key {
        Key::Enter | Key::Keypad(KeypadKey::Enter) => {
            if ctrl {
                0x0A
            } else {
                0x0D
            }
        }
        Key::Tab => 0x09,
        Key::Escape => 0x1B,
        Key::Backspace => {
            if ctrl {
                0x7F
            } else {
                0x08
            }
        }
        Key::Char(_) if ctrl && alt => 0,
        Key::Char(c) if ctrl => match c {
            'a'..='z' => u16::from(c as u8 - b'a' + 1),
            '[' => 0x1B,
            '\\' => 0x1C,
            ']' => 0x1D,
            '6' if shift => 0x1E,
            '-' if shift => 0x1F,
            ' ' => 0x20,
            _ => 0,
        },
        Key::Char(c) if shift => unit(if c.is_alphabetic() {
            upper(c)
        } else {
            physical.and_then(PhysicalKey::us_shifted_char).unwrap_or(c)
        }),
        Key::Char(c) => unit(c),
        Key::Keypad(pad) if !ctrl && !alt => pad.text_char().map_or(0, unit),
        _ => 0,
    }
}

/// The key a typed character belongs to: the character without Shift and Caps Lock.
///
/// Letters, and anything the layout types that is not ASCII (`é`, `ü`, `с`, `€`), are keyed by their
/// lower case so the kitty "all keys" report names what was typed. A shifted symbol goes back to the
/// key's unshifted US character when the US board agrees that is what Shift does to it, and stays
/// the typed character where the layout differs (the German `/` on Shift+7).
fn typed_key(identity: Key, text: char, mods: Mods, physical: Option<PhysicalKey>, altgr: bool) -> Key {
    if !matches!(identity, Key::Char(_)) {
        return identity;
    }
    if text.is_alphabetic() || altgr || !mods.contains(Mods::SHIFT) {
        return Key::Char(lower(text));
    }
    match physical {
        Some(key) if key.us_shifted_char() == Some(text) => key.us_char().map_or(Key::Char(text), Key::Char),
        _ => Key::Char(text),
    }
}

/// The scan code of an event, filled from the key's US position when WinUI reports none (Tab).
fn scan_code_of(stroke: &KeyStroke, class: KeyClass) -> u16 {
    if stroke.scan_code != 0 {
        return stroke.scan_code;
    }
    match class {
        KeyClass::Modifier(modifier) => modifier_physical(modifier).map_or(0, |key| key.scan_code().0),
        _ => default_scan_code(stroke.vk, stroke.extended).unwrap_or(0),
    }
}

fn key_id(stroke: &KeyStroke, class: KeyClass) -> KeyId {
    let vk = match class {
        KeyClass::Modifier(modifier) => modifier_record_vk(modifier),
        _ => u16::try_from(stroke.vk).unwrap_or(0),
    };
    KeyId { vk, scan_code: stroke.scan_code, extended: stroke.extended }
}

/// A console record has no Windows-key bit, so a chord with it would arrive as the bare key.
fn is_super_chord_on_win32(mods: Mods, key: Key, route: ConptyRoute) -> bool {
    route == ConptyRoute::Win32 && mods.contains(Mods::SUPER) && !matches!(key, Key::Modifier(_))
}

fn is_lock(key: Key) -> bool {
    matches!(key, Key::CapsLock | Key::NumLock | Key::ScrollLock)
}

impl KeyAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one event and returns what to send and how to answer XAML.
    pub fn handle(&mut self, event: KeyboardEvent, modes: &InputModes) -> KeyOutput {
        match event {
            KeyboardEvent::KeyDown(stroke) => self.key_down(&stroke, modes),
            KeyboardEvent::KeyUp(stroke) => self.key_up(&stroke, modes),
            KeyboardEvent::CharacterReceived(unit) => self.character_received(unit, modes),
            KeyboardEvent::Flush => self.resolve_pending(modes),
            KeyboardEvent::FocusLost => self.focus_lost(modes),
        }
    }

    // ---- key down ---------------------------------------------------------------------------

    fn key_down(&mut self, raw: &KeyStroke, modes: &InputModes) -> KeyOutput {
        // A key still waiting for a character has had its round: its character is not coming.
        let mut out = self.resolve_pending(modes);
        let class = classify(raw.vk, raw.scan_code, raw.extended);
        let stroke = KeyStroke { scan_code: scan_code_of(raw, class), ..*raw };

        match class {
            KeyClass::Ignored | KeyClass::Ime | KeyClass::Packet => {}
            KeyClass::Modifier(modifier) => {
                out.bytes.extend(self.modifier_down(modifier, &stroke, modes));
                out.handled = true;
            }
            KeyClass::Lock(key) | KeyClass::Named(key) => {
                out.bytes.extend(self.key_event_down(key, &stroke, class, modes));
                out.handled = true;
            }
            KeyClass::Text(identity) => self.text_down(identity, &stroke, modes, &mut out),
        }
        out
    }

    fn text_down(&mut self, identity: Key, stroke: &KeyStroke, modes: &InputModes, out: &mut KeyOutput) {
        let held = stroke.mods.only(Mods::HELD);
        // Alt+numpad digits compose a character; the digits themselves are not keys.
        if held == Mods::ALT && (0x60..=0x69).contains(&stroke.vk) {
            return;
        }
        let kind = if held.contains(Mods::CTRL | Mods::ALT) && !held.contains(Mods::SUPER) {
            Some(if self.altgr { PendingKind::AltGr } else { PendingKind::CtrlAlt })
        } else if !held.intersects(Mods::CTRL | Mods::ALT | Mods::SUPER) {
            Some(PendingKind::Plain)
        } else {
            None
        };
        match kind {
            Some(kind) => {
                self.pending = Some(Pending { stroke: *stroke, identity, kind });
                out.pending = true;
            }
            None => {
                out.bytes.extend(self.key_event_down(identity, stroke, KeyClass::Text(identity), modes));
                out.handled = true;
            }
        }
    }

    fn is_held(&self, id: KeyId) -> bool {
        self.held.iter().any(|key| key.id == id)
    }

    fn remember(&mut self, held: HeldKey) {
        self.held.retain(|key| key.id != held.id);
        if self.held.len() == MAX_HELD_KEYS {
            self.held.remove(0);
        }
        self.held.push(held);
    }

    /// The modifiers of the keys seen going down.
    fn tracked_mods(&self) -> Mods {
        let down = |keys: [ModifierKey; 2]| keys.iter().any(|key| self.modifiers & modifier_bit(*key) != 0);
        let mut mods = Mods::NONE;
        for (keys, bit) in [
            ([ModifierKey::LeftShift, ModifierKey::RightShift], Mods::SHIFT),
            ([ModifierKey::LeftControl, ModifierKey::RightControl], Mods::CTRL),
            ([ModifierKey::LeftAlt, ModifierKey::RightAlt], Mods::ALT),
            ([ModifierKey::LeftSuper, ModifierKey::RightSuper], Mods::SUPER),
        ] {
            if down(keys) {
                mods |= bit;
            }
        }
        mods
    }

    /// The `dwControlKeyState` of an event: the side of Ctrl and Alt comes from the keys seen going
    /// down (left when none was seen), Shift and the locks from the snapshot, AltGr as its pair.
    fn control_state(&self, mods: Mods, extended: bool, altgr_text: bool) -> u32 {
        let mut state = Win32Key::control_state_from_mods(mods.only(Mods::SHIFT | Mods::LOCKS));
        let down = |key: ModifierKey| self.modifiers & modifier_bit(key) != 0;
        if altgr_text {
            state |= control_state::LEFT_CTRL_PRESSED | control_state::RIGHT_ALT_PRESSED;
        } else {
            for (bit, left, right, left_flag, right_flag) in [
                (
                    Mods::CTRL,
                    ModifierKey::LeftControl,
                    ModifierKey::RightControl,
                    control_state::LEFT_CTRL_PRESSED,
                    control_state::RIGHT_CTRL_PRESSED,
                ),
                (
                    Mods::ALT,
                    ModifierKey::LeftAlt,
                    ModifierKey::RightAlt,
                    control_state::LEFT_ALT_PRESSED,
                    control_state::RIGHT_ALT_PRESSED,
                ),
            ] {
                if mods.contains(bit) {
                    let right_down = down(right);
                    if down(left) || !right_down {
                        state |= left_flag;
                    }
                    if right_down {
                        state |= right_flag;
                    }
                }
            }
        }
        if extended {
            state |= control_state::ENHANCED_KEY;
        }
        state
    }

    /// A press is a repeat when the key is already down on our side.
    fn event_of(&self, id: KeyId, stroke: &KeyStroke) -> KeyEvent {
        if stroke.repeat && self.is_held(id) { KeyEvent::Repeat } else { KeyEvent::Press }
    }

    fn modifier_down(&mut self, modifier: ModifierKey, stroke: &KeyStroke, modes: &InputModes) -> Vec<u8> {
        let id = key_id(stroke, KeyClass::Modifier(modifier));
        // Windows repeats a held modifier; programs get nothing new from it.
        if stroke.repeat && self.is_held(id) {
            return Vec::new();
        }
        self.modifiers |= modifier_bit(modifier);
        match modifier {
            ModifierKey::LeftControl => self.left_ctrl_pressed_at = stroke.time_ms,
            ModifierKey::RightAlt => {
                let ctrl_first = self.modifiers & modifier_bit(ModifierKey::LeftControl) != 0;
                if ctrl_first && stroke.time_ms.wrapping_sub(self.left_ctrl_pressed_at) <= ALTGR_WINDOW_MS {
                    self.altgr = true;
                }
            }
            _ => {}
        }

        let mods = stroke.mods | modifier.bit();
        let input = KeyInput::new(Key::Modifier(modifier)).with_mods(mods);
        let record = Win32Key {
            virtual_key: id.vk,
            scan_code: id.scan_code,
            unicode: 0,
            key_down: true,
            control_state: self.control_state(mods, id.extended, false),
            repeat_count: stroke.repeat_count,
        };
        self.send_press(id, &record, &input, modes)
    }

    /// A key encoded from the event: named keys, lock keys and text keys under a chord.
    fn key_event_down(&mut self, key: Key, stroke: &KeyStroke, class: KeyClass, modes: &InputModes) -> Vec<u8> {
        let id = key_id(stroke, class);
        let event = self.event_of(id, stroke);
        // A held lock key repeats like any other and would toggle the lock over and over.
        if is_lock(key) && event == KeyEvent::Repeat {
            return Vec::new();
        }
        let physical = stroke.physical_key();
        let mut input = KeyInput::new(key).with_mods(stroke.mods).with_event(event);
        if matches!(key, Key::Char(_)) {
            input.base_layout_key = physical.and_then(PhysicalKey::us_char);
        }
        let record = Win32Key {
            virtual_key: id.vk,
            scan_code: id.scan_code,
            unicode: console_unicode(key, stroke.mods, physical),
            key_down: true,
            control_state: self.control_state(stroke.mods, id.extended, false),
            repeat_count: stroke.repeat_count,
        };
        self.send_press(id, &record, &input, modes)
    }

    /// Sends a press and remembers the key so that its release can follow.
    fn send_press(&mut self, id: KeyId, record: &Win32Key, input: &KeyInput, modes: &InputModes) -> Vec<u8> {
        let route = conpty_route(modes);
        let bytes = if is_super_chord_on_win32(input.mods, input.key, route) {
            Vec::new()
        } else {
            encode_key_conpty(input, record, modes).unwrap_or_default()
        };
        self.remember(HeldKey {
            id,
            key: input.key,
            sent: (!bytes.is_empty()).then_some(route),
            altgr: input.altgr,
            unicode: record.unicode,
            base_layout_key: input.base_layout_key,
        });
        bytes
    }

    // ---- character --------------------------------------------------------------------------

    fn character_received(&mut self, unit: u16, modes: &InputModes) -> KeyOutput {
        let mut out = KeyOutput { handled: true, ..KeyOutput::default() };
        let Some(ch) = self.utf16.push(unit) else {
            return out;
        };
        let text = ch.to_string();
        match &self.pending {
            Some(pending) if Self::is_text_of(pending, &text, ch) => {
                if let Some(pending) = self.pending.take() {
                    out.bytes = self.send_text_key(pending, ch, modes);
                }
            }
            // A control character (what Windows makes of Ctrl+letter) is not the key's text.
            Some(_) => {}
            // Text nobody asked for: injected Unicode, Alt+numpad entry, the second character of a
            // dead key that did not combine.
            None => out.bytes = encode_text(&text),
        }
        out
    }

    fn is_text_of(pending: &Pending, text: &str, ch: char) -> bool {
        match pending.kind {
            PendingKind::Plain => !is_control(ch),
            // A space is not an AltGr character (Ctrl+Alt+Space is a chord).
            PendingKind::AltGr | PendingKind::CtrlAlt => is_altgr(Mods::CTRL | Mods::ALT, Some(text)),
        }
    }

    /// The key and its character, sent together.
    fn send_text_key(&mut self, pending: Pending, ch: char, modes: &InputModes) -> Vec<u8> {
        let Pending { stroke, identity, kind } = pending;
        let id = key_id(&stroke, KeyClass::Text(identity));
        let altgr = kind != PendingKind::Plain;
        let physical = stroke.physical_key();
        let text = ch.to_string();

        let mut input = KeyInput::new(typed_key(identity, ch, stroke.mods, physical, altgr))
            .with_mods(stroke.mods)
            .with_text(text.as_str())
            .with_event(self.event_of(id, &stroke));
        input.altgr = altgr;
        if matches!(identity, Key::Char(_)) {
            input.base_layout_key = physical.and_then(PhysicalKey::us_char);
        }

        // A record carries one UTF-16 unit: a character beyond the BMP goes out as text on the
        // Win32 route, with no record to release afterwards.
        let unicode = u16::try_from(u32::from(ch));
        if unicode.is_err() && conpty_route(modes) == ConptyRoute::Win32 {
            self.remember(HeldKey {
                id,
                key: input.key,
                sent: None,
                altgr,
                unicode: 0,
                base_layout_key: input.base_layout_key,
            });
            return encode_text(&text);
        }
        let record = Win32Key {
            virtual_key: id.vk,
            scan_code: id.scan_code,
            unicode: unicode.unwrap_or(0),
            key_down: true,
            control_state: self.control_state(stroke.mods, id.extended, altgr),
            repeat_count: stroke.repeat_count,
        };
        self.send_press(id, &record, &input, modes)
    }

    /// Resolves a text key that is still waiting for its character: a plain key and an AltGr key
    /// send nothing (a dead key, a key with no character on this layout), a Ctrl+Alt key without
    /// the AltGr evidence is the chord.
    fn resolve_pending(&mut self, modes: &InputModes) -> KeyOutput {
        let mut out = KeyOutput::default();
        let Some(Pending { stroke, identity, kind }) = self.pending.take() else {
            return out;
        };
        let class = KeyClass::Text(identity);
        match kind {
            PendingKind::CtrlAlt => out.bytes = self.key_event_down(identity, &stroke, class, modes),
            PendingKind::Plain | PendingKind::AltGr => self.remember(HeldKey {
                id: key_id(&stroke, class),
                key: identity,
                sent: None,
                altgr: false,
                unicode: 0,
                base_layout_key: None,
            }),
        }
        out
    }

    // ---- key up and focus ---------------------------------------------------------------------

    fn key_up(&mut self, raw: &KeyStroke, modes: &InputModes) -> KeyOutput {
        let mut out = self.resolve_pending(modes);
        let class = classify(raw.vk, raw.scan_code, raw.extended);
        if matches!(class, KeyClass::Ignored | KeyClass::Ime | KeyClass::Packet) {
            return out;
        }
        let stroke = KeyStroke { scan_code: scan_code_of(raw, class), ..*raw };
        out.handled = true;

        let mut mods = stroke.mods;
        if let KeyClass::Modifier(modifier) = class {
            self.modifiers &= !modifier_bit(modifier);
            if matches!(modifier, ModifierKey::RightAlt | ModifierKey::LeftControl) {
                self.altgr = false;
            }
            // The state after the release: the modifier stays held while its other side is down.
            if self.modifiers & modifier_bit(other_side(modifier)) == 0 {
                mods = mods.without(modifier.bit());
            }
        }
        let id = key_id(&stroke, class);
        if let Some(index) = self.held.iter().position(|key| key.id == id) {
            let key = self.held.remove(index);
            out.bytes.extend(self.release_bytes(&key, mods, modes));
        }
        out
    }

    fn release_bytes(&self, key: &HeldKey, mods: Mods, modes: &InputModes) -> Vec<u8> {
        if key.sent != Some(conpty_route(modes)) {
            return Vec::new();
        }
        let mut input = KeyInput::new(key.key).with_mods(mods).with_event(KeyEvent::Release);
        input.altgr = key.altgr;
        input.base_layout_key = key.base_layout_key;
        let record = Win32Key {
            virtual_key: key.id.vk,
            scan_code: key.id.scan_code,
            unicode: key.unicode,
            key_down: false,
            control_state: self.control_state(mods, key.id.extended, key.altgr),
            repeat_count: 1,
        };
        encode_key_conpty(&input, &record, modes).unwrap_or_default()
    }

    /// Releases every key that is still down, newest first so that modifiers go last, and forgets all
    /// keyboard state.
    fn focus_lost(&mut self, modes: &InputModes) -> KeyOutput {
        let mut out = self.resolve_pending(modes);
        while let Some(key) = self.held.pop() {
            if let Key::Modifier(modifier) = key.key {
                self.modifiers &= !modifier_bit(modifier);
            }
            let mods = self.tracked_mods();
            out.bytes.extend(self.release_bytes(&key, mods, modes));
        }
        self.modifiers = 0;
        self.altgr = false;
        self.utf16 = Utf16Assembler::default();
        out
    }
}
