//! The terminal state that decides how input is encoded.

/// The five kitty keyboard protocol enhancement flags, in protocol bit order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct KittyFlags(u8);

impl KittyFlags {
    pub const NONE: KittyFlags = KittyFlags(0);
    /// 0b1: Esc, alt/ctrl chords and non-text keypad keys become `CSI u`.
    pub const DISAMBIGUATE: KittyFlags = KittyFlags(1);
    /// 0b10: key repeat and release events.
    pub const REPORT_EVENTS: KittyFlags = KittyFlags(2);
    /// 0b100: shifted and base-layout key codes in the key field.
    pub const REPORT_ALTERNATES: KittyFlags = KittyFlags(4);
    /// 0b1000: every key, including text keys and modifier keys, as an escape code.
    pub const REPORT_ALL_KEYS: KittyFlags = KittyFlags(8);
    /// 0b10000: the text of a key event as code points (needs `REPORT_ALL_KEYS`).
    pub const REPORT_TEXT: KittyFlags = KittyFlags(16);
    pub const ALL: KittyFlags = KittyFlags(31);

    /// Bits as `CSI ? flags u` reports them.
    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits_truncate(bits: u8) -> KittyFlags {
        KittyFlags(bits & 31)
    }

    pub const fn contains(self, other: KittyFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: KittyFlags) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when the flags change how any key is encoded. Alternates and text alone are
    /// refinements of the other three and have no effect by themselves.
    pub const fn changes_encoding(self) -> bool {
        self.0 & (1 | 2 | 8) != 0
    }
}

impl std::ops::BitOr for KittyFlags {
    type Output = KittyFlags;
    fn bitor(self, rhs: KittyFlags) -> KittyFlags {
        KittyFlags(self.0 | rhs.0)
    }
}

/// Which mouse events the application asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    None,
    /// DECSET 9: button presses only.
    X10,
    /// DECSET 1000: presses and releases (and wheel).
    Normal,
    /// DECSET 1002: also motion while a button is held.
    Button,
    /// DECSET 1003: all motion.
    Any,
}

/// How reported positions are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseEncoding {
    /// `CSI M Cb Cx Cy`, one byte each, positions up to 223 (95 when the PTY decodes UTF-8).
    #[default]
    Default,
    /// DECSET 1005.
    Utf8,
    /// DECSET 1006: `CSI < b ; x ; y M|m`.
    Sgr,
    /// DECSET 1015: `CSI b ; x ; y M`.
    Urxvt,
    /// DECSET 1016: SGR with pixel positions.
    SgrPixels,
}

/// Everything `encode_*` needs to know about the terminal and the user's preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputModes {
    /// DECCKM (`CSI ? 1 h`): unmodified cursor keys, Home and End use SS3.
    pub app_cursor: bool,
    /// DECKPAM (`ESC =`): keypad keys send SS3 sequences.
    pub app_keypad: bool,
    pub kitty: KittyFlags,
    /// Alt prefixes the character with ESC (the xterm `metaSendsEscape` behaviour). When false,
    /// Alt sets the eighth bit of an ASCII character (as a UTF-8 encoded U+0080..U+00FF).
    pub alt_sends_escape: bool,
    /// Backspace sends 0x7f (DEL); false sends 0x08. Ctrl+Backspace sends the other one. Programs
    /// on Windows read 0x08 as Ctrl+Backspace, so the default must stay `true`.
    pub backspace_sends_delete: bool,
    pub mouse_mode: MouseMode,
    pub mouse_encoding: MouseEncoding,
    /// DECSET 1004.
    pub focus_reporting: bool,
    /// DECSET 2004.
    pub bracketed_paste: bool,
    /// DECSET 1007: wheel events become cursor keys on the alternate screen.
    pub alternate_scroll: bool,
    /// The alternate screen is active (alternate scroll only applies there).
    pub alt_screen: bool,
    /// Include caps lock / num lock bits in kitty modifiers of every `CSI u` event (what the
    /// spec asks for), not only under "report all keys". Off by default: a parser that does not
    /// mask bits 64/128 would see `ESC [ 27 ; 129 u` for Escape whenever NumLock is on.
    pub lock_modifiers_in_disambiguate: bool,
    /// The PTY decodes its input as UTF-8 text, as ConPTY does: a byte above 0x7f that is not
    /// part of valid UTF-8 arrives as U+FFFD. That limits the default mouse encoding (one byte
    /// per value) to positions up to 95; applications that need more use SGR or UTF-8 (1005).
    /// Measured against the bundled OpenConsole 1.25; see the blueprint.
    pub pty_decodes_utf8: bool,
}

impl Default for InputModes {
    fn default() -> Self {
        InputModes {
            app_cursor: false,
            app_keypad: false,
            kitty: KittyFlags::NONE,
            alt_sends_escape: true,
            backspace_sends_delete: true,
            mouse_mode: MouseMode::None,
            mouse_encoding: MouseEncoding::Default,
            focus_reporting: false,
            bracketed_paste: false,
            alternate_scroll: false,
            alt_screen: false,
            lock_modifiers_in_disambiguate: false,
            pty_decodes_utf8: true,
        }
    }
}

impl InputModes {
    /// Mouse reports are wanted by the application; the UI must not treat the events itself
    /// (selection, scrolling) unless the user holds the bypass modifier.
    pub fn mouse_tracking(&self) -> bool {
        self.mouse_mode != MouseMode::None
    }

    /// Reads the modes `alacritty_terminal` tracks. It tracks neither DECSET 9 (X10), 1015
    /// (URXVT) nor 1016 (SGR-pixels), so those stay at their defaults here; set
    /// `mouse_mode` / `mouse_encoding` from your own DECSET sniffer when you need them.
    /// `alt_sends_escape`, `backspace_sends_delete`, `lock_modifiers_in_disambiguate` and
    /// `pty_decodes_utf8` are preferences of the host and keep their defaults.
    #[cfg(feature = "alacritty")]
    pub fn from_term_mode(mode: alacritty_terminal::term::TermMode) -> Self {
        use alacritty_terminal::term::TermMode as T;

        let mut kitty = KittyFlags::NONE;
        for (flag, bit) in [
            (T::DISAMBIGUATE_ESC_CODES, KittyFlags::DISAMBIGUATE),
            (T::REPORT_EVENT_TYPES, KittyFlags::REPORT_EVENTS),
            (T::REPORT_ALTERNATE_KEYS, KittyFlags::REPORT_ALTERNATES),
            (T::REPORT_ALL_KEYS_AS_ESC, KittyFlags::REPORT_ALL_KEYS),
            (T::REPORT_ASSOCIATED_TEXT, KittyFlags::REPORT_TEXT),
        ] {
            if mode.contains(flag) {
                kitty = kitty | bit;
            }
        }

        // alacritty_terminal keeps the three tracking modes mutually exclusive; if a caller built a
        // TermMode by hand, the most capable one wins.
        let mouse_mode = if mode.contains(T::MOUSE_MOTION) {
            MouseMode::Any
        } else if mode.contains(T::MOUSE_DRAG) {
            MouseMode::Button
        } else if mode.contains(T::MOUSE_REPORT_CLICK) {
            MouseMode::Normal
        } else {
            MouseMode::None
        };
        let mouse_encoding = if mode.contains(T::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if mode.contains(T::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::Default
        };

        InputModes {
            app_cursor: mode.contains(T::APP_CURSOR),
            app_keypad: mode.contains(T::APP_KEYPAD),
            kitty,
            mouse_mode,
            mouse_encoding,
            focus_reporting: mode.contains(T::FOCUS_IN_OUT),
            bracketed_paste: mode.contains(T::BRACKETED_PASTE),
            alternate_scroll: mode.contains(T::ALTERNATE_SCROLL),
            alt_screen: mode.contains(T::ALT_SCREEN),
            ..InputModes::default()
        }
    }
}
