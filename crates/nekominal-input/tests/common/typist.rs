//! A typist and a small operating system for the key assembler tests.
//!
//! The layouts below are tables of the Windows keyboard layouts (scan code -> virtual key and the
//! characters of the four shift levels) written from the published `KBDUS`, `KBDUSX`, `KBDGR`,
//! `KBDFR` and `KBDRU` tables; they were not captured from hardware. `Rig` plays the part of Windows
//! and XAML around a `KeyAssembler`: it produces `KeyDown`, `CharacterReceived` and `KeyUp` events
//! the way WinUI 3 delivers them (a fake left Ctrl before the right Alt of an AltGr press, dead keys
//! without a character, a composed character after the next key, no `CharacterReceived` for a key
//! the assembler handled) and feeds them in, so a test describes what the user does and compares the
//! bytes that reach the PTY.

use nekominal_input::*;

pub const TICK_MS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Nothing,
    Ch(char),
    /// A dead key: `standalone` is typed when it does not combine, `accent` selects the combination.
    Dead {
        standalone: char,
        accent: char,
    },
}

#[derive(Clone, Debug)]
pub struct LayoutKey {
    pub key: PhysicalKey,
    pub vk: i32,
    pub normal: Out,
    pub shift: Out,
    pub altgr: Out,
    pub shift_altgr: Out,
}

impl LayoutKey {
    pub fn new(key: PhysicalKey, vk: i32, normal: char, shift: char) -> Self {
        LayoutKey {
            key,
            vk,
            normal: Out::Ch(normal),
            shift: Out::Ch(shift),
            altgr: Out::Nothing,
            shift_altgr: Out::Nothing,
        }
    }

    pub fn altgr(mut self, c: char) -> Self {
        self.altgr = Out::Ch(c);
        self
    }

    pub fn shift_altgr(mut self, c: char) -> Self {
        self.shift_altgr = Out::Ch(c);
        self
    }

    pub fn dead(mut self, standalone: char, accent: char) -> Self {
        self.normal = Out::Dead { standalone, accent };
        self
    }

    pub fn shift_dead(mut self, standalone: char, accent: char) -> Self {
        self.shift = Out::Dead { standalone, accent };
        self
    }

    /// The Shift level types nothing.
    pub fn without_shift_level(mut self) -> Self {
        self.shift = Out::Nothing;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub name: &'static str,
    pub keys: Vec<LayoutKey>,
    /// The right Alt key is AltGr: Windows presses a fake left Ctrl before it.
    pub has_altgr: bool,
}

fn us_keys() -> Vec<LayoutKey> {
    use PhysicalKey::*;
    let mut keys = Vec::new();
    for (key, vk) in [
        (Digit1, 0x31),
        (Digit2, 0x32),
        (Digit3, 0x33),
        (Digit4, 0x34),
        (Digit5, 0x35),
        (Digit6, 0x36),
        (Digit7, 0x37),
        (Digit8, 0x38),
        (Digit9, 0x39),
        (Digit0, 0x30),
        (Minus, 0xBD),
        (Equal, 0xBB),
        (BracketLeft, 0xDB),
        (BracketRight, 0xDD),
        (Backslash, 0xDC),
        (Semicolon, 0xBA),
        (Quote, 0xDE),
        (Backquote, 0xC0),
        (Comma, 0xBC),
        (Period, 0xBE),
        (Slash, 0xBF),
    ] {
        keys.push(LayoutKey::new(key, vk, key.us_char().unwrap(), key.us_shifted_char().unwrap()));
    }
    for key in PhysicalKey::all().filter(|key| key.us_char().is_some_and(|c| c.is_ascii_lowercase())) {
        let c = key.us_char().unwrap();
        keys.push(LayoutKey::new(key, c.to_ascii_uppercase() as i32, c, c.to_ascii_uppercase()));
    }
    keys.push(LayoutKey::new(Space, 0x20, ' ', ' '));
    keys
}

impl Layout {
    fn set(mut self, key: LayoutKey) -> Layout {
        match self.keys.iter_mut().find(|k| k.key == key.key) {
            Some(slot) => *slot = key,
            None => self.keys.push(key),
        }
        self
    }

    /// KBDUS.
    pub fn us() -> Layout {
        Layout { name: "US", keys: us_keys(), has_altgr: false }
    }

    /// The Microsoft Bopomofo IME in English mode hands the keys to the US layout unchanged.
    pub fn zhuyin_english() -> Layout {
        Layout { name: "Chinese (Traditional) - US, English mode", ..Layout::us() }
    }

    /// KBDUSX: dead `'`, `"`, `` ` ``, `~`, `^` and the AltGr layer of accented letters.
    pub fn us_international() -> Layout {
        use PhysicalKey::*;
        Layout { name: "US-International", has_altgr: true, ..Layout::us() }
            .set(LayoutKey::new(Quote, 0xDE, '\'', '"').dead('\'', '´').shift_dead('"', '¨'))
            .set(LayoutKey::new(Backquote, 0xC0, '`', '~').dead('`', '`').shift_dead('~', '~'))
            .set(LayoutKey::new(Digit6, 0x36, '6', '^').shift_dead('^', '^'))
            .set(LayoutKey::new(Digit5, 0x35, '5', '%').altgr('€'))
            .set(LayoutKey::new(KeyE, 0x45, 'e', 'E').altgr('é').shift_altgr('É'))
            .set(LayoutKey::new(KeyU, 0x55, 'u', 'U').altgr('ú').shift_altgr('Ú'))
            .set(LayoutKey::new(KeyI, 0x49, 'i', 'I').altgr('í').shift_altgr('Í'))
            .set(LayoutKey::new(KeyO, 0x4F, 'o', 'O').altgr('ó').shift_altgr('Ó'))
            .set(LayoutKey::new(KeyA, 0x41, 'a', 'A').altgr('á').shift_altgr('Á'))
            .set(LayoutKey::new(KeyN, 0x4E, 'n', 'N').altgr('ñ').shift_altgr('Ñ'))
            .set(LayoutKey::new(KeyS, 0x53, 's', 'S').altgr('ß'))
    }

    /// KBDGR.
    pub fn german() -> Layout {
        use PhysicalKey::*;
        Layout { name: "German", has_altgr: true, ..Layout::us() }
            .set(LayoutKey::new(Backquote, 0xDC, '^', '°').dead('^', '^'))
            .set(LayoutKey::new(Digit2, 0x32, '2', '"').altgr('²'))
            .set(LayoutKey::new(Digit3, 0x33, '3', '§').altgr('³'))
            .set(LayoutKey::new(Digit6, 0x36, '6', '&'))
            .set(LayoutKey::new(Digit7, 0x37, '7', '/').altgr('{'))
            .set(LayoutKey::new(Digit8, 0x38, '8', '(').altgr('['))
            .set(LayoutKey::new(Digit9, 0x39, '9', ')').altgr(']'))
            .set(LayoutKey::new(Digit0, 0x30, '0', '=').altgr('}'))
            .set(LayoutKey::new(Minus, 0xDB, 'ß', '?').altgr('\\'))
            .set(LayoutKey::new(Equal, 0xDD, '´', '`').dead('´', '´').shift_dead('`', '`'))
            .set(LayoutKey::new(KeyQ, 0x51, 'q', 'Q').altgr('@'))
            .set(LayoutKey::new(KeyE, 0x45, 'e', 'E').altgr('€'))
            .set(LayoutKey::new(KeyY, 0x5A, 'z', 'Z'))
            .set(LayoutKey::new(KeyZ, 0x59, 'y', 'Y'))
            .set(LayoutKey::new(KeyM, 0x4D, 'm', 'M').altgr('µ'))
            .set(LayoutKey::new(BracketLeft, 0xBA, 'ü', 'Ü'))
            .set(LayoutKey::new(BracketRight, 0xBB, '+', '*').altgr('~'))
            .set(LayoutKey::new(Semicolon, 0xC0, 'ö', 'Ö'))
            .set(LayoutKey::new(Quote, 0xDE, 'ä', 'Ä'))
            .set(LayoutKey::new(Backslash, 0xBF, '#', '\''))
            .set(LayoutKey::new(IntlBackslash, 0xE2, '<', '>').altgr('|'))
            .set(LayoutKey::new(Comma, 0xBC, ',', ';'))
            .set(LayoutKey::new(Period, 0xBE, '.', ':'))
            .set(LayoutKey::new(Slash, 0xBD, '-', '_'))
    }

    /// KBDFR: AZERTY, digits on Shift, dead `^` and `¨`.
    pub fn french() -> Layout {
        use PhysicalKey::*;
        Layout { name: "French", has_altgr: true, ..Layout::us() }
            .set(LayoutKey::new(Backquote, 0xDF, '²', '²').without_shift_level())
            .set(LayoutKey::new(Digit1, 0x31, '&', '1'))
            .set(LayoutKey::new(Digit2, 0x32, 'é', '2').altgr('~'))
            .set(LayoutKey::new(Digit3, 0x33, '"', '3').altgr('#'))
            .set(LayoutKey::new(Digit4, 0x34, '\'', '4').altgr('{'))
            .set(LayoutKey::new(Digit5, 0x35, '(', '5').altgr('['))
            .set(LayoutKey::new(Digit6, 0x36, '-', '6').altgr('|'))
            .set(LayoutKey::new(Digit7, 0x37, 'è', '7').altgr('`'))
            .set(LayoutKey::new(Digit8, 0x38, '_', '8').altgr('\\'))
            .set(LayoutKey::new(Digit9, 0x39, 'ç', '9').altgr('^'))
            .set(LayoutKey::new(Digit0, 0x30, 'à', '0').altgr('@'))
            .set(LayoutKey::new(Minus, 0xDB, ')', '°').altgr(']'))
            .set(LayoutKey::new(Equal, 0xBB, '=', '+').altgr('}'))
            .set(LayoutKey::new(KeyQ, 0x41, 'a', 'A'))
            .set(LayoutKey::new(KeyW, 0x5A, 'z', 'Z'))
            .set(LayoutKey::new(KeyE, 0x45, 'e', 'E').altgr('€'))
            .set(LayoutKey::new(KeyA, 0x51, 'q', 'Q'))
            .set(LayoutKey::new(KeyZ, 0x57, 'w', 'W'))
            .set(LayoutKey::new(KeyM, 0xBC, ',', '?'))
            .set(LayoutKey::new(Semicolon, 0x4D, 'm', 'M'))
            .set(LayoutKey::new(BracketLeft, 0xDD, '^', '¨').dead('^', '^').shift_dead('¨', '¨'))
            .set(LayoutKey::new(BracketRight, 0xBA, '$', '£').altgr('¤'))
            .set(LayoutKey::new(Quote, 0xC0, 'ù', '%'))
            .set(LayoutKey::new(Backslash, 0xDC, '*', 'µ'))
            .set(LayoutKey::new(IntlBackslash, 0xE2, '<', '>'))
            .set(LayoutKey::new(Comma, 0xBE, ';', '.'))
            .set(LayoutKey::new(Period, 0xBF, ':', '/'))
            .set(LayoutKey::new(Slash, 0xDF, '!', '§'))
    }

    /// KBDRU, the letter keys the tests use.
    pub fn russian() -> Layout {
        use PhysicalKey::*;
        let mut layout = Layout { name: "Russian", has_altgr: false, ..Layout::us() };
        for (key, vk, lower, upper) in [
            (KeyQ, 0x51, 'й', 'Й'),
            (KeyW, 0x57, 'ц', 'Ц'),
            (KeyA, 0x41, 'ф', 'Ф'),
            (KeyS, 0x53, 'ы', 'Ы'),
            (KeyC, 0x43, 'с', 'С'),
            (KeyX, 0x58, 'ч', 'Ч'),
            (KeyV, 0x56, 'м', 'М'),
        ] {
            layout = layout.set(LayoutKey::new(key, vk, lower, upper));
        }
        layout
    }

    fn key(&self, key: PhysicalKey) -> Option<&LayoutKey> {
        self.keys.iter().find(|k| k.key == key)
    }
}

/// Dead key combinations of the accents the layouts above use.
fn compose(accent: char, base: char) -> Option<char> {
    let table: &[(char, &str, &str)] = &[
        ('´', "aeiouy", "áéíóúý"),
        ('`', "aeiou", "àèìòù"),
        ('^', "aeiou", "âêîôû"),
        ('¨', "aeiouy", "äëïöüÿ"),
        ('~', "ano", "ãñõ"),
    ];
    let lower = base.to_lowercase().next()?;
    let (_, bases, composed) = table.iter().find(|(a, _, _)| *a == accent)?;
    let index = bases.chars().position(|b| b == lower)?;
    let result = composed.chars().nth(index)?;
    Some(if base.is_uppercase() { result.to_uppercase().next()? } else { result })
}

/// The keys a keyboard has besides the layout.
fn fixed_vk(key: PhysicalKey) -> Option<i32> {
    use PhysicalKey::*;
    Some(match key {
        Escape => 0x1B,
        Backspace => 0x08,
        Tab => 0x09,
        Enter | NumpadEnter => 0x0D,
        CapsLock => 0x14,
        NumLock => 0x90,
        ScrollLock => 0x91,
        Pause => 0x13,
        PrintScreen => 0x2C,
        ContextMenu => 0x5D,
        ArrowUp => 0x26,
        ArrowDown => 0x28,
        ArrowLeft => 0x25,
        ArrowRight => 0x27,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,
        Insert => 0x2D,
        Delete => 0x2E,
        NumpadAdd => 0x6B,
        NumpadSubtract => 0x6D,
        NumpadMultiply => 0x6A,
        NumpadDivide => 0x6F,
        MediaPlayPause => 0xB3,
        AudioVolumeUp => 0xAF,
        BrowserBack => 0xA6,
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7A,
        F12 => 0x7B,
        F13 => 0x7C,
        F14 => 0x7D,
        F15 => 0x7E,
        F16 => 0x7F,
        F17 => 0x80,
        F18 => 0x81,
        F19 => 0x82,
        F20 => 0x83,
        F21 => 0x84,
        F22 => 0x85,
        F23 => 0x86,
        F24 => 0x87,
        _ => return None,
    })
}

/// The numpad: (NumLock on virtual key, character, NumLock off virtual key).
fn numpad(key: PhysicalKey) -> Option<(i32, char, i32)> {
    use PhysicalKey::*;
    Some(match key {
        Numpad0 => (0x60, '0', 0x2D),
        Numpad1 => (0x61, '1', 0x23),
        Numpad2 => (0x62, '2', 0x28),
        Numpad3 => (0x63, '3', 0x22),
        Numpad4 => (0x64, '4', 0x25),
        Numpad5 => (0x65, '5', 0x0C),
        Numpad6 => (0x66, '6', 0x27),
        Numpad7 => (0x67, '7', 0x24),
        Numpad8 => (0x68, '8', 0x26),
        Numpad9 => (0x69, '9', 0x21),
        NumpadDecimal => (0x6E, '.', 0x2E),
        _ => return None,
    })
}

fn units(chars: Vec<char>) -> Vec<u16> {
    chars.into_iter().flat_map(|c| c.encode_utf16(&mut [0; 2]).to_vec()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mod {
    Shift,
    RightShift,
    Ctrl,
    RightCtrl,
    Alt,
    /// The right Alt key: AltGr on layouts that have it.
    RightAlt,
    Win,
}

impl Mod {
    fn key(self) -> PhysicalKey {
        match self {
            Mod::Shift => PhysicalKey::ShiftLeft,
            Mod::RightShift => PhysicalKey::ShiftRight,
            Mod::Ctrl => PhysicalKey::ControlLeft,
            Mod::RightCtrl => PhysicalKey::ControlRight,
            Mod::Alt => PhysicalKey::AltLeft,
            Mod::RightAlt => PhysicalKey::AltRight,
            Mod::Win => PhysicalKey::MetaLeft,
        }
    }

    fn vk(self) -> i32 {
        match self {
            Mod::Shift | Mod::RightShift => 0x10,
            Mod::Ctrl | Mod::RightCtrl => 0x11,
            Mod::Alt | Mod::RightAlt => 0x12,
            Mod::Win => 0x5B,
        }
    }
}

/// What happened for one event: the event, how the assembler answered, and what it sent.
#[derive(Clone, Debug)]
pub struct Step {
    pub event: KeyboardEvent,
    pub handled: bool,
    pub pending: bool,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
struct Held {
    shift: [bool; 2],
    ctrl: [bool; 2],
    alt: [bool; 2],
    win: bool,
    caps: bool,
    num: bool,
}

pub struct Rig {
    pub asm: KeyAssembler,
    pub modes: InputModes,
    pub layout: Layout,
    /// Post `Flush` after the dispatch round of a `KeyDown` that is pending (what the shell does).
    pub flush: bool,
    /// Every byte sent and not yet taken, in order.
    pub out: Vec<u8>,
    pub log: Vec<Step>,
    now: u32,
    held: Held,
    dead: Option<(char, char)>,
}

impl Rig {
    pub fn new(layout: Layout) -> Rig {
        Rig {
            asm: KeyAssembler::new(),
            modes: InputModes::default(),
            layout,
            flush: true,
            out: Vec::new(),
            log: Vec::new(),
            now: 1_000,
            held: Held::default(),
            dead: None,
        }
    }

    pub fn us() -> Rig {
        Rig::new(Layout::us())
    }

    /// A rig whose program pushed these kitty flag bits.
    pub fn with_kitty(mut self, bits: u8) -> Rig {
        self.modes.kitty = KittyFlags::from_bits_truncate(bits);
        self
    }

    pub fn num_lock(&mut self, on: bool) {
        self.held.num = on;
    }

    pub fn advance(&mut self, ms: u32) {
        self.now += ms;
    }

    /// The bytes sent since the last call, printable.
    pub fn take(&mut self) -> String {
        let text = super::show(&self.out);
        self.out.clear();
        text
    }

    /// The modifier and lock state as `GetKeyStateForCurrentThread` reports it.
    pub fn snapshot(&self) -> Mods {
        let mut mods = Mods::NONE;
        let h = &self.held;
        for (down, bit) in [
            (h.shift.iter().any(|d| *d), Mods::SHIFT),
            (h.ctrl.iter().any(|d| *d), Mods::CTRL),
            (h.alt.iter().any(|d| *d), Mods::ALT),
            (h.win, Mods::SUPER),
            (h.caps, Mods::CAPS_LOCK),
            (h.num, Mods::NUM_LOCK),
        ] {
            if down {
                mods |= bit;
            }
        }
        mods
    }

    pub fn feed(&mut self, event: KeyboardEvent) -> KeyOutput {
        let result = self.asm.handle(event, &self.modes);
        self.out.extend_from_slice(&result.bytes);
        self.log.push(Step { event, handled: result.handled, pending: result.pending, bytes: result.bytes.clone() });
        result
    }

    pub fn last(&self) -> &Step {
        self.log.last().expect("an event was fed")
    }

    // ---- the operating system -----------------------------------------------------------------

    fn stroke(&self, key: PhysicalKey, vk: i32, repeat: bool) -> KeyStroke {
        let (scan_code, extended) = key.scan_code();
        let mut stroke = KeyStroke::new(vk, scan_code, extended, self.snapshot()).with_time(self.now);
        stroke.repeat = repeat;
        stroke
    }

    fn altgr_down(&self) -> bool {
        self.layout.has_altgr && self.held.alt[1] && self.held.ctrl[0]
    }

    fn typed(&mut self, out: Out) -> Vec<char> {
        match out {
            Out::Nothing => Vec::new(),
            Out::Ch(c) => match self.dead.take() {
                None => vec![c],
                Some((standalone, accent)) => match compose(accent, c) {
                    Some(composed) => vec![composed],
                    None if c == ' ' => vec![standalone],
                    None => vec![standalone, c],
                },
            },
            Out::Dead { standalone, accent } => match self.dead.replace((standalone, accent)) {
                None => Vec::new(),
                // The same dead key twice types the accent itself.
                Some((_, previous)) if previous == accent => {
                    self.dead = None;
                    vec![standalone]
                }
                Some((previous, _)) => vec![previous],
            },
        }
    }

    /// What `TranslateMessage` makes of a key press the application did not handle.
    fn characters(&mut self, key: PhysicalKey) -> Vec<u16> {
        let (ctrl, alt) = (self.held.ctrl.iter().any(|d| *d), self.held.alt.iter().any(|d| *d));
        let altgr = self.altgr_down();
        if self.held.win || (alt && !altgr) {
            return Vec::new();
        }
        let shift = self.held.shift.iter().any(|d| *d);

        if let Some(layout_key) = self.layout.key(key).cloned() {
            if ctrl && !altgr {
                return match layout_key.normal {
                    Out::Ch(c) if c.is_ascii_lowercase() => vec![c as u16 - 'a' as u16 + 1],
                    Out::Ch(' ') => vec![0x20],
                    _ => Vec::new(),
                };
            }
            let letter = matches!(layout_key.normal, Out::Ch(c) if c.is_alphabetic());
            let upper = shift ^ (self.held.caps && letter);
            let out = match (altgr, upper) {
                (false, false) => layout_key.normal,
                (false, true) => layout_key.shift,
                (true, false) => layout_key.altgr,
                (true, true) => layout_key.shift_altgr,
            };
            return units(self.typed(out));
        }
        if let Some((_, c, _)) = numpad(key).filter(|_| self.held.num) {
            return vec![c as u16];
        }
        match key {
            PhysicalKey::NumpadAdd => vec!['+' as u16],
            PhysicalKey::NumpadSubtract => vec!['-' as u16],
            PhysicalKey::NumpadMultiply => vec!['*' as u16],
            PhysicalKey::NumpadDivide => vec!['/' as u16],
            PhysicalKey::Enter | PhysicalKey::NumpadEnter => vec![0x0D],
            PhysicalKey::Tab => vec![0x09],
            PhysicalKey::Backspace => vec![0x08],
            PhysicalKey::Escape => vec![0x1B],
            _ => Vec::new(),
        }
    }

    fn vk_of(&self, key: PhysicalKey) -> i32 {
        if let Some(layout_key) = self.layout.key(key) {
            return layout_key.vk;
        }
        if let Some((on, _, off)) = numpad(key) {
            return if self.held.num { on } else { off };
        }
        fixed_vk(key).unwrap_or_else(|| panic!("no virtual key for {}", key.name()))
    }

    // ---- the typist -----------------------------------------------------------------------------

    pub fn key_down(&mut self, key: PhysicalKey) {
        self.key_down_with(key, false);
    }

    fn key_down_with(&mut self, key: PhysicalKey, repeat: bool) {
        self.now += TICK_MS;
        let vk = self.vk_of(key);
        match key {
            PhysicalKey::CapsLock if !repeat => self.held.caps = !self.held.caps,
            PhysicalKey::NumLock if !repeat => self.held.num = !self.held.num,
            _ => {}
        }
        let stroke = self.stroke(key, vk, repeat);
        let result = self.feed(KeyboardEvent::KeyDown(stroke));
        if !result.handled {
            for unit in self.characters(key) {
                self.feed(KeyboardEvent::CharacterReceived(unit));
            }
        }
        if result.pending && self.flush {
            self.feed(KeyboardEvent::Flush);
        }
    }

    pub fn key_up(&mut self, key: PhysicalKey) {
        self.now += TICK_MS;
        let vk = self.vk_of(key);
        let stroke = self.stroke(key, vk, false);
        self.feed(KeyboardEvent::KeyUp(stroke));
    }

    pub fn tap(&mut self, key: PhysicalKey) {
        self.key_down(key);
        self.key_up(key);
    }

    /// Auto-repeat: `count` more `KeyDown`s with `WasKeyDown` set.
    pub fn repeat(&mut self, key: PhysicalKey, count: usize) {
        for _ in 0..count {
            self.key_down_with(key, true);
        }
    }

    fn set_modifier(&mut self, m: Mod, down: bool) {
        let held = &mut self.held;
        match m {
            Mod::Shift => held.shift[0] = down,
            Mod::RightShift => held.shift[1] = down,
            Mod::Ctrl => held.ctrl[0] = down,
            Mod::RightCtrl => held.ctrl[1] = down,
            Mod::Alt => held.alt[0] = down,
            Mod::RightAlt => held.alt[1] = down,
            Mod::Win => held.win = down,
        }
    }

    /// One modifier key event; the state after it is what the handler's snapshot shows.
    fn modifier_event(&mut self, m: Mod, down: bool, repeat: bool) {
        self.set_modifier(m, down);
        let stroke = self.stroke(m.key(), m.vk(), repeat);
        self.feed(if down { KeyboardEvent::KeyDown(stroke) } else { KeyboardEvent::KeyUp(stroke) });
    }

    pub fn hold(&mut self, m: Mod) {
        self.now += TICK_MS;
        if m == Mod::RightAlt && self.layout.has_altgr {
            // Windows presses a left Ctrl in front of AltGr, in the same message round.
            self.modifier_event(Mod::Ctrl, true, false);
        }
        self.modifier_event(m, true, false);
    }

    pub fn release(&mut self, m: Mod) {
        self.now += TICK_MS;
        self.modifier_event(m, false, false);
        if m == Mod::RightAlt && self.layout.has_altgr {
            self.modifier_event(Mod::Ctrl, false, false);
        }
    }

    /// AltGr on a layout that has it, with the fake left Ctrl arriving `gap_ms` before the right Alt.
    pub fn hold_altgr_slowly(&mut self, gap_ms: u32) {
        self.now += TICK_MS;
        self.modifier_event(Mod::Ctrl, true, false);
        self.now += gap_ms;
        self.modifier_event(Mod::RightAlt, true, false);
    }

    pub fn hold_repeat(&mut self, m: Mod) {
        self.now += TICK_MS;
        self.modifier_event(m, true, true);
    }

    /// Modifiers down, the key tapped, modifiers up in reverse.
    pub fn chord(&mut self, mods: &[Mod], key: PhysicalKey) {
        for m in mods {
            self.hold(*m);
        }
        self.tap(key);
        for m in mods.iter().rev() {
            self.release(*m);
        }
    }

    /// Types text on the layout: for every character the key, the modifier level and the dead key in
    /// front of it that produce it.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let (key, mods, dead) =
                self.how_to_type(c).unwrap_or_else(|| panic!("{} cannot type {c:?}", self.layout.name));
            if let Some((dead_key, dead_mods)) = dead {
                self.chord(&dead_mods, dead_key);
            }
            self.chord(&mods, key);
        }
    }

    #[allow(clippy::type_complexity)]
    fn how_to_type(&self, c: char) -> Option<(PhysicalKey, Vec<Mod>, Option<(PhysicalKey, Vec<Mod>)>)> {
        let altgr: Vec<Mod> = if self.layout.has_altgr { vec![Mod::RightAlt] } else { Vec::new() };
        let levels = |lk: &LayoutKey| {
            [
                (lk.normal, vec![]),
                (lk.shift, vec![Mod::Shift]),
                (lk.altgr, altgr.clone()),
                (lk.shift_altgr, [vec![Mod::Shift], altgr.clone()].concat()),
            ]
        };
        for lk in &self.layout.keys {
            for (out, mods) in levels(lk) {
                if out == Out::Ch(c) {
                    return Some((lk.key, mods, None));
                }
            }
        }
        // Not on a key by itself: a dead key and a base character.
        for dead in &self.layout.keys {
            for (out, dead_mods) in levels(dead) {
                let Out::Dead { accent, .. } = out else { continue };
                for base in &self.layout.keys {
                    for (base_out, base_mods) in levels(base) {
                        if matches!(base_out, Out::Ch(b) if compose(accent, b) == Some(c)) {
                            return Some((base.key, base_mods, Some((dead.key, dead_mods))));
                        }
                    }
                }
            }
        }
        None
    }

    // ---- character events on their own ----------------------------------------------------------

    pub fn character(&mut self, unit: u16) {
        self.feed(KeyboardEvent::CharacterReceived(unit));
    }

    /// `SendInput(KEYEVENTF_UNICODE)`: for every UTF-16 unit a `VK_PACKET` key down, the unit as
    /// `CharacterReceived`, a key up.
    pub fn inject_text(&mut self, text: &str) {
        for unit in text.encode_utf16() {
            self.now += TICK_MS;
            let stroke = KeyStroke::new(vk::VK_PACKET, 0, false, self.snapshot()).with_time(self.now);
            let result = self.feed(KeyboardEvent::KeyDown(stroke));
            if !result.handled {
                self.character(unit);
            }
            self.now += TICK_MS;
            self.feed(KeyboardEvent::KeyUp(stroke));
        }
    }

    pub fn focus_lost(&mut self) {
        self.feed(KeyboardEvent::FocusLost);
        // The keys are released elsewhere and never seen again; the lock state stays.
        self.held = Held { caps: self.held.caps, num: self.held.num, ..Held::default() };
    }
}

/// The Win32 key record of the wire format, in the printable form of `show`.
pub fn record(vk: u16, scan_code: u16, unicode: u16, down: bool, control_state: u32) -> String {
    format!("\\e[{vk};{scan_code};{unicode};{};{control_state};1_", u8::from(down))
}

/// Key down record followed by the key up record.
pub fn tap_records(vk: u16, scan_code: u16, unicode: u16, control_state: u32) -> String {
    record(vk, scan_code, unicode, true, control_state) + &record(vk, scan_code, unicode, false, control_state)
}

/// One `CSI Vk;Sc;Uc;Kd;Cs;Rc _` record of a Win32 route stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rec {
    pub vk: u16,
    pub scan_code: u16,
    pub unicode: u16,
    pub down: bool,
    pub control_state: u32,
}

/// A Win32 route stream taken apart: the records, and the text (UTF-8) between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Record(Rec),
    Text(String),
}

pub fn parse_stream(bytes: &[u8]) -> Vec<Item> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut items = Vec::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix("\x1b[") {
            let end = body.find('_').expect("a record ends with _");
            let f: Vec<u32> = body[..end].split(';').map(|f| f.parse().expect("numeric field")).collect();
            items.push(Item::Record(Rec {
                vk: f[0] as u16,
                scan_code: f[1] as u16,
                unicode: f[2] as u16,
                down: f[3] == 1,
                control_state: f[4],
            }));
            rest = &body[end + 1..];
        } else {
            let end = rest.find('\x1b').unwrap_or(rest.len());
            items.push(Item::Text(rest[..end].to_string()));
            rest = &rest[end..];
        }
    }
    items
}

/// The characters the key down records of a Win32 route stream carry, with text that is not a
/// record (UTF-8 sent as text) in place.
pub fn win32_typed(bytes: &[u8]) -> String {
    let mut typed = String::new();
    let mut units: Vec<u16> = Vec::new();
    for item in parse_stream(bytes) {
        match item {
            Item::Record(rec) if rec.down && rec.unicode != 0 => units.push(rec.unicode),
            Item::Record(_) => {}
            Item::Text(text) => {
                typed.push_str(&String::from_utf16_lossy(&units));
                units.clear();
                typed.push_str(&text);
            }
        }
    }
    typed.push_str(&String::from_utf16_lossy(&units));
    typed
}
