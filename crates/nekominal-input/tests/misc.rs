//! Paste, focus, win32-input-mode, AltGr detection and the small value types.

mod common;
use common::*;
use nekominal_input::*;

fn paste(text: &str, bracketed: bool) -> String {
    show(&prepare_paste(text, bracketed))
}

// ---- paste ----------------------------------------------------------------------------------

#[test]
fn plain_paste_is_the_text() {
    assert_eq!(paste("hello", false), "hello");
    assert_eq!(paste("", false), "");
}

#[test]
fn bracketed_paste_is_framed() {
    assert_eq!(paste("hello", true), r"\e[200~hello\e[201~");
    assert_eq!(paste("", true), r"\e[200~\e[201~");
}

#[test]
fn line_breaks_become_enter() {
    assert_eq!(paste("a\r\nb\nc\rd", false), r"a\rb\rc\rd");
    assert_eq!(paste("x\n", false), r"x\r");
    assert_eq!(paste("a\r\n\r\nb", false), r"a\r\rb");
    assert_eq!(paste("a\n\nb", false), r"a\r\rb");
    assert_eq!(paste("a\r\r\nb", false), r"a\r\rb");
    assert_eq!(paste("one\r\ntwo", true), r"\e[200~one\rtwo\e[201~");
}

#[test]
fn controls_are_stripped_but_tab_stays() {
    assert_eq!(paste("a\u{1b}[201~b\u{7}c\td", false), r"a[201~bc\td");
    assert_eq!(paste("a\0b\u{8}c\u{b}d\u{c}e\u{1f}f\u{7f}g", false), "abcdefg");
    assert_eq!(paste("a\u{9b}31mb\u{85}c\u{90}d", false), "a31mbcd");
}

#[test]
fn an_embedded_end_marker_cannot_end_the_paste_early() {
    let framed = prepare_paste("safe\u{1b}[201~; rm -rf ~\r", true);
    let end = b"\x1b[201~";
    let occurrences = framed.windows(end.len()).filter(|w| *w == end).count();
    assert_eq!(occurrences, 1, "{}", show(&framed));
    assert!(framed.ends_with(end));
    assert!(framed.starts_with(b"\x1b[200~"));
}

#[test]
fn paste_keeps_cjk_emoji_and_combining_text() {
    assert_eq!(paste("你好，世界 😀 é", false), "你好，世界 😀 é");
    assert_eq!(paste("👨‍👩‍👧", true), "\\e[200~👨‍👩‍👧\\e[201~");
}

#[test]
fn large_paste_keeps_every_byte() {
    let text = "line of text 你好\n".repeat(60_000);
    let out = prepare_paste(&text, true);
    let expected = text.replace('\n', "\r");
    assert_eq!(out.len(), expected.len() + 12);
    assert_eq!(&out[6..out.len() - 6], expected.as_bytes());
    assert!(std::str::from_utf8(&out).is_ok());
}

// ---- focus ----------------------------------------------------------------------------------

#[test]
fn focus_reports_only_when_requested() {
    let mut m = legacy();
    assert!(encode_focus(true, &m).is_none());
    assert!(encode_focus(false, &m).is_none());
    m.focus_reporting = true;
    assert_eq!(show(encode_focus(true, &m).unwrap()), r"\e[I");
    assert_eq!(show(encode_focus(false, &m).unwrap()), r"\e[O");
}

// ---- win32-input-mode -----------------------------------------------------------------------

#[test]
fn win32_input_sequence_layout() {
    use nekominal_input::win32::control_state::*;
    let enter_down = Win32Key {
        virtual_key: 0x0d,
        scan_code: 0x1c,
        unicode: 13,
        key_down: true,
        control_state: SHIFT_PRESSED,
        repeat_count: 1,
    };
    assert_eq!(show(&encode_win32_input(&enter_down)), r"\e[13;28;13;1;16;1_");
    let up = Win32Key { key_down: false, ..enter_down };
    assert_eq!(show(&encode_win32_input(&up)), r"\e[13;28;13;0;16;1_");
    let a = Win32Key { virtual_key: 0x41, scan_code: 30, unicode: 97, key_down: true, control_state: 0, repeat_count: 0 };
    // A repeat count of 0 is not valid; the default is 1.
    assert_eq!(show(&encode_win32_input(&a)), r"\e[65;30;97;1;0;1_");
}

#[test]
fn win32_control_state_from_modifiers() {
    use nekominal_input::win32::control_state::*;
    assert_eq!(Win32Key::control_state_from_mods(Mods::NONE), 0);
    assert_eq!(Win32Key::control_state_from_mods(SHIFT), SHIFT_PRESSED);
    assert_eq!(Win32Key::control_state_from_mods(CTRL | ALT), LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED);
    assert_eq!(
        Win32Key::control_state_from_mods(SHIFT | CTRL | NUM | CAPS),
        SHIFT_PRESSED | LEFT_CTRL_PRESSED | NUMLOCK_ON | CAPSLOCK_ON
    );
}

#[test]
fn win32_text_events_use_utf16_units() {
    assert_eq!(show(&encode_win32_text("a")), r"\e[0;0;97;1;0;1_\e[0;0;97;0;0;1_");
    assert_eq!(show(&encode_win32_text("你")), r"\e[0;0;20320;1;0;1_\e[0;0;20320;0;0;1_");
    // U+1F600 is the surrogate pair D83D DE00.
    assert_eq!(
        show(&encode_win32_text("😀")),
        r"\e[0;0;55357;1;0;1_\e[0;0;55357;0;0;1_\e[0;0;56832;1;0;1_\e[0;0;56832;0;0;1_"
    );
    assert_eq!(encode_win32_text(""), b"");
}

// ---- AltGr heuristic ------------------------------------------------------------------------

#[test]
fn altgr_is_ctrl_alt_with_printable_text() {
    assert!(is_altgr(CTRL | ALT, Some("€")));
    assert!(is_altgr(CTRL | ALT | SHIFT, Some("@")));
    assert!(is_altgr(CTRL | ALT, Some("ß")));
    // A genuine Ctrl+Alt chord has no text, or only a control character.
    assert!(!is_altgr(CTRL | ALT, None));
    assert!(!is_altgr(CTRL | ALT, Some("")));
    assert!(!is_altgr(CTRL | ALT, Some("\u{11}")));
    assert!(!is_altgr(CTRL | ALT, Some("\u{1b}")));
    assert!(!is_altgr(CTRL | ALT, Some(" ")));
    assert!(!is_altgr(CTRL | ALT, Some("\u{7f}")));
    assert!(!is_altgr(CTRL | ALT, Some("\u{9b}")));
    // Needs both modifiers.
    assert!(!is_altgr(CTRL, Some("a")));
    assert!(!is_altgr(ALT, Some("a")));
}

// ---- value types ----------------------------------------------------------------------------

#[test]
fn modifier_bits_match_the_kitty_protocol() {
    assert_eq!(Mods::SHIFT.bits(), 1);
    assert_eq!(Mods::ALT.bits(), 2);
    assert_eq!(Mods::CTRL.bits(), 4);
    assert_eq!(Mods::SUPER.bits(), 8);
    assert_eq!(Mods::HYPER.bits(), 16);
    assert_eq!(Mods::META.bits(), 32);
    assert_eq!(Mods::CAPS_LOCK.bits(), 64);
    assert_eq!(Mods::NUM_LOCK.bits(), 128);
    assert_eq!((SHIFT | CTRL).bits(), 5);
}

#[test]
fn modifier_set_operations() {
    let m = SHIFT | CTRL | NUM;
    assert!(m.contains(SHIFT | CTRL));
    assert!(!m.contains(ALT));
    assert!(m.intersects(ALT | CTRL));
    assert!(!Mods::NONE.intersects(SHIFT));
    assert_eq!(m.without(CTRL), SHIFT | NUM);
    assert_eq!(m.only(Mods::HELD), SHIFT | CTRL);
    assert!(Mods::NONE.is_empty());
    assert_eq!(format!("{:?}", SHIFT | CTRL), "Mods(SHIFT | CTRL)");
    assert_eq!(format!("{:?}", Mods::NONE), "Mods(NONE)");
}

#[test]
fn kitty_flag_bits() {
    assert_eq!(KittyFlags::DISAMBIGUATE.bits(), 1);
    assert_eq!(KittyFlags::REPORT_EVENTS.bits(), 2);
    assert_eq!(KittyFlags::REPORT_ALTERNATES.bits(), 4);
    assert_eq!(KittyFlags::REPORT_ALL_KEYS.bits(), 8);
    assert_eq!(KittyFlags::REPORT_TEXT.bits(), 16);
    assert_eq!(KittyFlags::from_bits_truncate(0xff), KittyFlags::ALL);
    assert!(KittyFlags::ALL.changes_encoding());
    assert!(!KittyFlags::REPORT_ALTERNATES.changes_encoding());
    assert!(!(KittyFlags::REPORT_ALTERNATES | KittyFlags::REPORT_TEXT).changes_encoding());
    assert!(KittyFlags::REPORT_EVENTS.changes_encoding());
}

#[test]
fn keypad_key_text() {
    assert_eq!(KeypadKey::Digit(7).text_char(), Some('7'));
    assert_eq!(KeypadKey::Digit(12).text_char(), None);
    assert_eq!(KeypadKey::Enter.text_char(), None);
    assert_eq!(KeypadKey::Left.text_char(), None);
    assert_eq!(KeypadKey::Add.text_char(), Some('+'));
}

#[test]
fn modifier_keys_know_their_bit() {
    assert_eq!(ModifierKey::LeftShift.bit(), Mods::SHIFT);
    assert_eq!(ModifierKey::RightControl.bit(), Mods::CTRL);
    assert_eq!(ModifierKey::RightAlt.bit(), Mods::ALT);
    assert_eq!(ModifierKey::LeftSuper.bit(), Mods::SUPER);
    assert_eq!(ModifierKey::IsoLevel3Shift.bit(), Mods::NONE);
}

#[test]
fn defaults_are_the_safe_ones() {
    let m = InputModes::default();
    assert!(m.alt_sends_escape);
    // Windows programs read 0x08 as Ctrl+Backspace, so plain Backspace must be DEL.
    assert!(m.backspace_sends_delete);
    assert!(!m.app_cursor && !m.app_keypad && !m.bracketed_paste && !m.focus_reporting);
    assert_eq!(m.mouse_mode, MouseMode::None);
    assert_eq!(m.mouse_encoding, MouseEncoding::Default);
    assert!(m.kitty.is_empty());
    assert!(!m.lock_modifiers_in_disambiguate);
}

// ---- ConPTY routing -------------------------------------------------------------------------

fn shift_enter_record(key_down: bool) -> Win32Key {
    Win32Key { virtual_key: 0x0d, scan_code: 28, unicode: 13, key_down, control_state: 0x10, repeat_count: 1 }
}

#[test]
fn conpty_route_follows_the_kitty_flags() {
    assert_eq!(conpty_route(&legacy()), ConptyRoute::Win32);
    assert_eq!(conpty_route(&kitty(1)), ConptyRoute::Kitty);
    assert_eq!(conpty_route(&kitty(2)), ConptyRoute::Kitty);
    assert_eq!(conpty_route(&kitty(8)), ConptyRoute::Kitty);
    // Alternates and text alone do not change any encoding.
    assert_eq!(conpty_route(&kitty(4)), ConptyRoute::Win32);
    assert_eq!(conpty_route(&kitty(16)), ConptyRoute::Win32);
    assert_eq!(conpty_route(&kitty(31)), ConptyRoute::Kitty);
}

#[test]
fn conpty_without_kitty_flags_sends_the_win32_record() {
    let input = key(Key::Enter).with_mods(SHIFT);
    let bytes = encode_key_conpty(&input, &shift_enter_record(true), &legacy()).unwrap();
    assert_eq!(show(&bytes), r"\e[13;28;13;1;16;1_");
    // Releases are records too: Win32 children see both halves of a key press.
    let up = input.with_event(KeyEvent::Release);
    let bytes = encode_key_conpty(&up, &shift_enter_record(false), &legacy()).unwrap();
    assert_eq!(show(&bytes), r"\e[13;28;13;0;16;1_");
}

#[test]
fn conpty_with_kitty_flags_sends_kitty_bytes() {
    let input = key(Key::Enter).with_mods(SHIFT);
    let bytes = encode_key_conpty(&input, &shift_enter_record(true), &kitty(1)).unwrap();
    assert_eq!(show(&bytes), r"\e[13;2u");
    // Without the events flag the release is not reported at all.
    let up = input.with_event(KeyEvent::Release);
    assert!(encode_key_conpty(&up, &shift_enter_record(false), &kitty(1)).is_none());
    let bytes = encode_key_conpty(&up, &shift_enter_record(false), &kitty(1 | 2));
    assert!(bytes.is_none(), "Enter has no release event without all-keys");
}

#[test]
fn conpty_never_sends_dead_key_presses() {
    let dead = key(Key::Dead);
    let record = Win32Key { virtual_key: 0xde, scan_code: 40, unicode: 0, key_down: true, control_state: 0, repeat_count: 1 };
    assert!(encode_key_conpty(&dead, &record, &legacy()).is_none());
    assert!(encode_key_conpty(&dead, &record, &kitty(1)).is_none());
}