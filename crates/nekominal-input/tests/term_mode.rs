//! `InputModes::from_term_mode` against the real alacritty_terminal: bytes the application writes
//! go in, the modes come out, and the encoders react.

mod common;
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::Dimensions,
    term::{Config, Term, TermMode},
    vte::ansi::Processor,
};
use common::*;
use nekominal_input::*;

struct Quiet;

impl EventListener for Quiet {
    fn send_event(&self, _event: Event) {}
}

struct Size;

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        24
    }

    fn screen_lines(&self) -> usize {
        24
    }

    fn columns(&self) -> usize {
        80
    }
}

struct Emulator {
    term: Term<Quiet>,
    processor: Processor,
}

impl Emulator {
    fn new() -> Self {
        let config = Config { kitty_keyboard: true, ..Config::default() };
        Emulator { term: Term::new(config, &Size, Quiet), processor: Processor::new() }
    }

    /// What the application writes to the terminal.
    fn app_writes(&mut self, bytes: &[u8]) -> &mut Self {
        self.processor.advance(&mut self.term, bytes);
        self
    }

    fn modes(&self) -> InputModes {
        InputModes::from_term_mode(*self.term.mode())
    }
}

#[test]
fn a_fresh_terminal_has_no_input_modes_except_alternate_scroll() {
    let modes = Emulator::new().modes();
    assert_eq!(modes.kitty, KittyFlags::NONE);
    assert!(!modes.app_cursor && !modes.app_keypad && !modes.bracketed_paste && !modes.focus_reporting);
    assert_eq!(modes.mouse_mode, MouseMode::None);
    assert_eq!(modes.mouse_encoding, MouseEncoding::Default);
    assert!(!modes.alt_screen);
    // alacritty_terminal starts with 1007 set; it only acts on the alternate screen.
    assert!(modes.alternate_scroll);
}

#[test]
fn preferences_keep_their_defaults() {
    let modes = InputModes::from_term_mode(TermMode::all());
    assert!(modes.alt_sends_escape);
    assert!(modes.backspace_sends_delete);
    assert!(!modes.lock_modifiers_in_disambiguate);
}

#[test]
fn kitty_push_pop_and_set_flow_into_the_flags() {
    let mut emulator = Emulator::new();
    assert_eq!(emulator.app_writes(b"\x1b[>1u").modes().kitty, KittyFlags::DISAMBIGUATE);
    assert_eq!(
        emulator.app_writes(b"\x1b[>31u").modes().kitty,
        KittyFlags::DISAMBIGUATE
            | KittyFlags::REPORT_EVENTS
            | KittyFlags::REPORT_ALTERNATES
            | KittyFlags::REPORT_ALL_KEYS
            | KittyFlags::REPORT_TEXT
    );
    assert_eq!(emulator.app_writes(b"\x1b[<u").modes().kitty, KittyFlags::DISAMBIGUATE);
    assert_eq!(emulator.app_writes(b"\x1b[<u").modes().kitty, KittyFlags::NONE);
    // Set with mode 1 replaces, 2 adds, 3 removes.
    assert_eq!(emulator.app_writes(b"\x1b[=8;1u").modes().kitty, KittyFlags::REPORT_ALL_KEYS);
    assert_eq!(
        emulator.app_writes(b"\x1b[=2;2u").modes().kitty,
        KittyFlags::REPORT_ALL_KEYS | KittyFlags::REPORT_EVENTS
    );
    assert_eq!(emulator.app_writes(b"\x1b[=8;3u").modes().kitty, KittyFlags::REPORT_EVENTS);
}

#[test]
fn kitty_flags_are_per_screen() {
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[>1u");
    assert_eq!(emulator.modes().kitty, KittyFlags::DISAMBIGUATE);
    emulator.app_writes(b"\x1b[?1049h");
    assert!(emulator.modes().alt_screen);
    assert_eq!(emulator.modes().kitty, KittyFlags::NONE);
    emulator.app_writes(b"\x1b[>8u");
    assert_eq!(emulator.modes().kitty, KittyFlags::REPORT_ALL_KEYS);
    emulator.app_writes(b"\x1b[?1049l");
    assert!(!emulator.modes().alt_screen);
    assert_eq!(emulator.modes().kitty, KittyFlags::DISAMBIGUATE);
}

#[test]
fn shift_enter_end_to_end() {
    let mut emulator = Emulator::new();
    let shift_enter = key(Key::Enter).with_mods(SHIFT);
    assert_eq!(enc(shift_enter.clone(), &emulator.modes()), r"\r");
    emulator.app_writes(b"\x1b[>1u");
    assert_eq!(enc(shift_enter.clone(), &emulator.modes()), r"\e[13;2u");
    emulator.app_writes(b"\x1b[<u");
    assert_eq!(enc(shift_enter, &emulator.modes()), r"\r");
}

#[test]
fn cursor_and_keypad_modes() {
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[?1h\x1b=");
    let modes = emulator.modes();
    assert!(modes.app_cursor && modes.app_keypad);
    assert_eq!(enc(key(Key::Up), &modes), r"\eOA");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(1))), &modes), r"\eOq");
    emulator.app_writes(b"\x1b[?1l\x1b>");
    let modes = emulator.modes();
    assert!(!modes.app_cursor && !modes.app_keypad);
}

#[test]
fn paste_and_focus_modes() {
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[?2004h\x1b[?1004h");
    let modes = emulator.modes();
    assert!(modes.bracketed_paste && modes.focus_reporting);
    assert_eq!(show(&prepare_paste("x", modes.bracketed_paste)), r"\e[200~x\e[201~");
    assert!(encode_focus(true, &modes).is_some());
    emulator.app_writes(b"\x1b[?2004l\x1b[?1004l");
    let modes = emulator.modes();
    assert!(!modes.bracketed_paste && !modes.focus_reporting);
}

#[test]
fn mouse_modes_are_exclusive_and_map_to_the_most_capable() {
    let mut emulator = Emulator::new();
    assert_eq!(emulator.app_writes(b"\x1b[?1000h").modes().mouse_mode, MouseMode::Normal);
    assert_eq!(emulator.app_writes(b"\x1b[?1002h").modes().mouse_mode, MouseMode::Button);
    assert_eq!(emulator.app_writes(b"\x1b[?1003h").modes().mouse_mode, MouseMode::Any);
    assert_eq!(emulator.app_writes(b"\x1b[?1003l").modes().mouse_mode, MouseMode::None);
    // A TermMode built by hand with several tracking bits: the most capable wins.
    let hand = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
    assert_eq!(InputModes::from_term_mode(hand).mouse_mode, MouseMode::Button);
    let hand = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
    assert_eq!(InputModes::from_term_mode(hand).mouse_mode, MouseMode::Any);
}

#[test]
fn mouse_encodings() {
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[?1000h");
    assert_eq!(emulator.modes().mouse_encoding, MouseEncoding::Default);
    assert_eq!(emulator.app_writes(b"\x1b[?1006h").modes().mouse_encoding, MouseEncoding::Sgr);
    // 1005 and 1006 replace each other.
    assert_eq!(emulator.app_writes(b"\x1b[?1005h").modes().mouse_encoding, MouseEncoding::Utf8);
    assert_eq!(emulator.app_writes(b"\x1b[?1006h").modes().mouse_encoding, MouseEncoding::Sgr);
    assert_eq!(emulator.app_writes(b"\x1b[?1006l").modes().mouse_encoding, MouseEncoding::Default);
    // A hand-built TermMode with both: SGR is the one that is unambiguous.
    let hand = TermMode::SGR_MOUSE | TermMode::UTF8_MOUSE;
    assert_eq!(InputModes::from_term_mode(hand).mouse_encoding, MouseEncoding::Sgr);
}

#[test]
fn end_to_end_click_in_sgr_mode() {
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[?1000h\x1b[?1006h");
    let modes = emulator.modes();
    let click = MouseInput::new(MouseKind::Press(MouseButton::Left), 9, 4);
    assert_eq!(
        show(&encode_mouse(&click, &modes, &mut MouseState::default()).unwrap()),
        r"\e[<0;10;5M"
    );
}

#[test]
fn alacritty_terminal_does_not_track_x10_urxvt_or_sgr_pixels() {
    // These DECSETs are ignored by the emulator, so the modes stay at their defaults. A host that
    // needs them must sniff the stream itself and set `mouse_mode` / `mouse_encoding`.
    let mut emulator = Emulator::new();
    emulator.app_writes(b"\x1b[?9h");
    assert_eq!(emulator.modes().mouse_mode, MouseMode::None);
    emulator.app_writes(b"\x1b[?1000h\x1b[?1015h");
    assert_eq!(emulator.modes().mouse_encoding, MouseEncoding::Default);
    emulator.app_writes(b"\x1b[?1016h");
    assert_eq!(emulator.modes().mouse_encoding, MouseEncoding::Default);
}

#[test]
fn alternate_scroll_end_to_end() {
    let mut emulator = Emulator::new();
    let wheel = MouseInput::new(MouseKind::Wheel(WheelDirection::Up), 0, 0);
    let on_main = emulator.modes();
    assert!(encode_mouse(&wheel, &on_main, &mut MouseState::default()).is_none());
    emulator.app_writes(b"\x1b[?1049h");
    let on_alt = emulator.modes();
    assert_eq!(show(&encode_mouse(&wheel, &on_alt, &mut MouseState::default()).unwrap()), r"\e[A");
    emulator.app_writes(b"\x1b[?1007l");
    assert!(encode_mouse(&wheel, &emulator.modes(), &mut MouseState::default()).is_none());
}

#[test]
fn kitty_requests_are_ignored_unless_the_emulator_is_configured_for_them() {
    // `Config::kitty_keyboard` is off by default in alacritty_terminal.
    let mut term = Term::new(Config::default(), &Size, Quiet);
    let mut processor: Processor = Processor::new();
    processor.advance(&mut term, b"\x1b[>1u");
    assert_eq!(InputModes::from_term_mode(*term.mode()).kitty, KittyFlags::NONE);
}
