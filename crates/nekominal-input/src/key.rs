//! Toolkit-independent keyboard types.
//!
//! The UI layer (WinUI 3, winit, ...) fills a [`KeyInput`] per key event. Nothing here knows about
//! a windowing system.

use std::{
    fmt,
    ops::{BitAnd, BitOr, BitOrAssign},
};

/// Modifier state, bit-compatible with the kitty keyboard protocol
/// (`CSI ... ; 1 + bits u`): shift 1, alt 2, ctrl 4, super 8, hyper 16, meta 32,
/// caps lock 64, num lock 128.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const CTRL: Mods = Mods(4);
    /// The Windows key.
    pub const SUPER: Mods = Mods(8);
    pub const HYPER: Mods = Mods(16);
    pub const META: Mods = Mods(32);
    pub const CAPS_LOCK: Mods = Mods(64);
    pub const NUM_LOCK: Mods = Mods(128);

    /// Caps lock and num lock: state, not keys held down.
    pub const LOCKS: Mods = Mods(64 | 128);
    /// Everything that is not a lock.
    pub const HELD: Mods = Mods(0x3f);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits_truncate(bits: u8) -> Mods {
        Mods(bits)
    }

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Mods) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn without(self, other: Mods) -> Mods {
        Mods(self.0 & !other.0)
    }

    pub const fn only(self, other: Mods) -> Mods {
        Mods(self.0 & other.0)
    }
}

impl BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

impl BitOrAssign for Mods {
    fn bitor_assign(&mut self, rhs: Mods) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Mods {
    type Output = Mods;
    fn bitand(self, rhs: Mods) -> Mods {
        Mods(self.0 & rhs.0)
    }
}

impl fmt::Debug for Mods {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [(Mods, &str); 8] = [
            (Mods::SHIFT, "SHIFT"),
            (Mods::ALT, "ALT"),
            (Mods::CTRL, "CTRL"),
            (Mods::SUPER, "SUPER"),
            (Mods::HYPER, "HYPER"),
            (Mods::META, "META"),
            (Mods::CAPS_LOCK, "CAPS_LOCK"),
            (Mods::NUM_LOCK, "NUM_LOCK"),
        ];
        let names: Vec<&str> = NAMES.iter().filter(|(m, _)| self.contains(*m)).map(|(_, n)| *n).collect();
        if names.is_empty() {
            write!(f, "Mods(NONE)")
        } else {
            write!(f, "Mods({})", names.join(" | "))
        }
    }
}

/// What happened to the key. Auto-repeat is `Repeat`, not another `Press`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum KeyEvent {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeypadKey {
    /// `0..=9`. Delivered by the OS only while NumLock is on.
    Digit(u8),
    Decimal,
    Divide,
    Multiply,
    Subtract,
    Add,
    Enter,
    Equal,
    Separator,
    // Navigation keys of the keypad. The OS delivers them while NumLock is off (on Windows:
    // VK_LEFT etc. without the ENHANCED_KEY flag).
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    /// Keypad 5 with NumLock off (VK_CLEAR).
    Begin,
}

impl KeypadKey {
    /// The character the key types when it is a text key.
    pub fn text_char(self) -> Option<char> {
        match self {
            KeypadKey::Digit(n) if n <= 9 => Some((b'0' + n) as char),
            KeypadKey::Decimal => Some('.'),
            KeypadKey::Divide => Some('/'),
            KeypadKey::Multiply => Some('*'),
            KeypadKey::Subtract => Some('-'),
            KeypadKey::Add => Some('+'),
            KeypadKey::Equal => Some('='),
            KeypadKey::Separator => Some(','),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKey {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModifierKey {
    /// The modifier bit this key contributes.
    pub fn bit(self) -> Mods {
        use ModifierKey::*;
        match self {
            LeftShift | RightShift => Mods::SHIFT,
            LeftControl | RightControl => Mods::CTRL,
            LeftAlt | RightAlt => Mods::ALT,
            LeftSuper | RightSuper => Mods::SUPER,
            LeftHyper | RightHyper => Mods::HYPER,
            LeftMeta | RightMeta => Mods::META,
            // AltGr is a layout layer, not a held modifier of its own.
            IsoLevel3Shift | IsoLevel5Shift => Mods::NONE,
        }
    }
}

/// The identity of the pressed key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable key, identified by the character it types on the active layout *without Shift
    /// and CapsLock*: lower case for letters (`Char('a')`, never `'A'`), the digit for the digit
    /// row, the unshifted symbol for punctuation. This is layout-aware (a Russian layout gives
    /// `Char('с')`; supply [`KeyInput::base_layout_key`] for Ctrl shortcuts). For an AltGr press
    /// it is the AltGr-layer character (`Char('€')`). `Char(' ')` is the space bar.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// `F(1)..=F(35)`.
    F(u8),
    Keypad(KeypadKey),
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    /// The context-menu key (VK_APPS).
    Menu,
    Media(MediaKey),
    Modifier(ModifierKey),
    /// A dead key press. Never encoded: the composed character arrives with a later event.
    Dead,
    /// A key the toolkit cannot name. Encoded only through its `text`.
    Unidentified,
}

/// One keyboard event as the UI toolkit reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub key: Key,
    /// The text the OS delivered as text input for this event (WM_CHAR / CharacterReceived),
    /// with Shift and CapsLock applied: dead-key and compose results (`"é"`), AltGr characters
    /// (`"€"`), any multi-character result. `None` for keys without text and for chords whose OS
    /// text is a control character (Ctrl+A) or was never produced (Alt+A arrives as
    /// WM_SYSCHAR). Text containing control characters is ignored.
    pub text: Option<String>,
    /// Modifier state including the effect of this very event for modifier keys.
    pub mods: Mods,
    pub event: KeyEvent,
    /// The Ctrl+Alt in `mods` is the AltGr layout shift (Windows reports AltGr as Ctrl+Alt), not
    /// something the user chose. The encoder drops CTRL and ALT for such events, so the text
    /// stays text. [`is_altgr`] implements the heuristic.
    pub altgr: bool,
    /// The character this key types with Shift held on the active layout (`'!'` for `1` on US).
    /// Optional: derived from `text` or by upper-casing when absent.
    pub shifted_key: Option<char>,
    /// The character of the same physical key on a PC-101 (US) layout; `None` when the toolkit
    /// cannot tell or it equals `key`. Makes Ctrl+С on a Cyrillic layout behave as Ctrl+C.
    pub base_layout_key: Option<char>,
}

impl KeyInput {
    pub fn new(key: Key) -> Self {
        KeyInput {
            key,
            text: None,
            mods: Mods::NONE,
            event: KeyEvent::Press,
            altgr: false,
            shifted_key: None,
            base_layout_key: None,
        }
    }

    pub fn with_mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_event(mut self, event: KeyEvent) -> Self {
        self.event = event;
        self
    }

    pub fn with_altgr(mut self) -> Self {
        self.altgr = true;
        self
    }

    pub fn with_shifted(mut self, shifted: char) -> Self {
        self.shifted_key = Some(shifted);
        self
    }

    pub fn with_base_layout(mut self, base: char) -> Self {
        self.base_layout_key = Some(base);
        self
    }
}

/// Windows reports AltGr as Ctrl+Alt. A genuine Ctrl+Alt chord has no printable text (Windows
/// produces none for Ctrl+Alt+Q on a US layout) while an AltGr chord does. This is the heuristic
/// of Windows Terminal's `TerminalInput` (`altGrPressed`): it is a guess, not a Win32 guarantee.
pub fn is_altgr(mods: Mods, text: Option<&str>) -> bool {
    mods.contains(Mods::CTRL | Mods::ALT)
        && text
            .and_then(|t| t.chars().next())
            .is_some_and(|c| c as u32 > 0x20 && c as u32 != 0x7f && !is_c1(c))
}

pub(crate) fn is_c1(c: char) -> bool {
    ('\u{80}'..='\u{9f}').contains(&c)
}

/// C0, DEL and C1 controls.
pub(crate) fn is_control(c: char) -> bool {
    (c as u32) < 0x20 || c as u32 == 0x7f || is_c1(c)
}
