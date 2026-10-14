//! The traditional (no kitty flags) encoding: xterm PC-style keys, Windows conventions.

mod common;
use common::*;
use nekominal_input::*;

// ---- text keys ------------------------------------------------------------------------------

#[test]
fn plain_typing_is_the_text() {
    let m = legacy();
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(typed('7'), &m), "7");
    assert_eq!(enc(typed(' '), &m), " ");
    assert_eq!(enc(typed('é'), &m), "é");
}

#[test]
fn shift_typing_uses_the_os_text() {
    let m = legacy();
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), "A");
    assert_eq!(enc(KeyInput::new(Key::Char('1')).with_mods(SHIFT).with_text("!"), &m), "!");
}

#[test]
fn composed_and_multi_character_text_passes_through() {
    let m = legacy();
    // A dead key followed by e arrives as one event with the composed text.
    assert_eq!(enc(KeyInput::new(Key::Char('e')).with_text("é"), &m), "é");
    assert_eq!(enc(KeyInput::new(Key::Char('x')).with_text("fi"), &m), "fi");
    assert_eq!(enc(KeyInput::new(Key::Char('你')).with_text("你"), &m), "你");
    assert_eq!(enc(KeyInput::new(Key::Unidentified).with_text("😀"), &m), "😀");
}

#[test]
fn dead_keys_and_nameless_keys_without_text_send_nothing() {
    let m = legacy();
    assert_eq!(enc(key(Key::Dead), &m), NOTHING);
    assert_eq!(enc(key(Key::Dead).with_text("´"), &m), NOTHING);
    assert_eq!(enc(key(Key::Unidentified), &m), NOTHING);
}

#[test]
fn alt_prefixes_escape() {
    let m = legacy();
    assert_eq!(enc(chord('a', ALT), &m), r"\ea");
    assert_eq!(enc(chord('a', ALT | SHIFT), &m), r"\eA");
    assert_eq!(enc(chord('1', ALT | SHIFT).with_shifted('!'), &m), r"\e!");
    assert_eq!(enc(chord(' ', ALT), &m), r"\e ");
    assert_eq!(enc(KeyInput::new(Key::Char('e')).with_mods(ALT).with_text("é"), &m), "\\eé");
}

#[test]
fn alt_as_eighth_bit_when_escape_is_off() {
    let mut m = legacy();
    m.alt_sends_escape = false;
    assert_eq!(enc(chord('a', ALT), &m), "\u{e1}");
    assert_eq!(enc(chord('a', ALT | CTRL), &m), "\u{81}");
    // Not representable in one byte: keep the ESC prefix so Alt is not lost.
    assert_eq!(enc(KeyInput::new(Key::Char('e')).with_mods(ALT).with_text("é"), &m), "\\eé");
    // Named keys keep the ESC prefix.
    assert_eq!(enc(key(Key::Enter).with_mods(ALT), &m), r"\e\r");
}

#[test]
fn ctrl_letters_map_to_c0() {
    let m = legacy();
    for (i, c) in ('a'..='z').enumerate() {
        let got = encode_key(&chord(c, CTRL), &m).unwrap();
        assert_eq!(got, vec![i as u8 + 1], "Ctrl+{c}");
        // Shift does not change it: Ctrl+Shift+letter is Ctrl+letter, as in xterm and Windows Terminal.
        let got = encode_key(&chord(c, CTRL | SHIFT), &m).unwrap();
        assert_eq!(got, vec![i as u8 + 1], "Ctrl+Shift+{c}");
    }
}

#[test]
fn ctrl_punctuation_follows_the_kitty_table() {
    let m = legacy();
    assert_eq!(enc(chord(' ', CTRL), &m), r"\x00");
    assert_eq!(enc(chord(' ', CTRL | SHIFT), &m), r"\x00");
    assert_eq!(enc(chord('[', CTRL), &m), r"\e");
    assert_eq!(enc(chord('\\', CTRL), &m), r"\x1c");
    assert_eq!(enc(chord(']', CTRL), &m), r"\x1d");
    assert_eq!(enc(chord('/', CTRL), &m), r"\x1f");
    // Ctrl+@, Ctrl+^, Ctrl+_ and Ctrl+? need Shift on a US layout.
    assert_eq!(enc(chord('2', CTRL | SHIFT).with_shifted('@'), &m), r"\x00");
    assert_eq!(enc(chord('6', CTRL | SHIFT).with_shifted('^'), &m), r"\x1e");
    assert_eq!(enc(chord('-', CTRL | SHIFT).with_shifted('_'), &m), r"\x1f");
    assert_eq!(enc(chord('/', CTRL | SHIFT).with_shifted('?'), &m), r"\x7f");
}

#[test]
fn shifted_symbols_fall_back_to_the_us_layout() {
    let m = legacy();
    assert_eq!(enc(chord('-', CTRL | SHIFT), &m), r"\x1f");
    assert_eq!(enc(chord('2', CTRL | SHIFT), &m), r"\x00");
    assert_eq!(enc(chord('/', CTRL | SHIFT), &m), r"\x7f");
    assert_eq!(enc(chord('1', ALT | SHIFT), &m), r"\e!");
    assert_eq!(enc(chord('=', ALT | SHIFT), &m), r"\e+");
    // The toolkit's own answer wins over the guess.
    assert_eq!(enc(chord('2', ALT | SHIFT).with_shifted('"'), &m), "\\e\"");
}

#[test]
fn ctrl_digits_follow_xterm() {
    let m = legacy();
    for (digit, byte) in [('2', "\\x00"), ('3', "\\e"), ('4', "\\x1c"), ('5', "\\x1d"), ('6', "\\x1e"), ('7', "\\x1f"), ('8', "\\x7f")] {
        assert_eq!(enc(chord(digit, CTRL), &m), byte, "Ctrl+{digit}");
    }
}

#[test]
fn ctrl_chords_without_a_control_character_send_the_character() {
    let m = legacy();
    for c in ['0', '1', '9', '-', ';', '.', ',', '='] {
        assert_eq!(enc(chord(c, CTRL), &m), c.to_string(), "Ctrl+{c}");
    }
    assert_eq!(enc(chord('1', CTRL | SHIFT).with_shifted('!'), &m), "!");
}

#[test]
fn ctrl_alt_chords_are_escape_plus_control() {
    let m = legacy();
    assert_eq!(enc(chord('a', ctrl_alt()), &m), r"\e\x01");
    assert_eq!(enc(chord(' ', ctrl_alt()), &m), r"\e\x00");
    assert_eq!(enc(chord('q', ctrl_alt()), &m), r"\e\x11");
    assert_eq!(enc(chord('1', ctrl_alt()), &m), r"\e1");
}

#[test]
fn ctrl_uses_the_base_layout_key_on_non_latin_layouts() {
    let m = legacy();
    // Ctrl+С on a Russian layout is Ctrl+C: it must still interrupt.
    assert_eq!(enc(chord('с', CTRL).with_base_layout('c'), &m), r"\x03");
    assert_eq!(enc(chord('с', CTRL | SHIFT).with_base_layout('c'), &m), r"\x03");
    assert_eq!(enc(chord('с', ctrl_alt()).with_base_layout('c'), &m), r"\e\x03");
    // Without the base layout the key has no control character and stays text.
    assert_eq!(enc(chord('с', CTRL), &m), "с");
    // Typing on that layout is unaffected.
    assert_eq!(enc(typed('с').with_base_layout('c'), &m), "с");
}

#[test]
fn altgr_text_stays_text() {
    let m = legacy();
    // German layout: AltGr+E is Ctrl+Alt+E to Win32, and types €.
    let euro = KeyInput::new(Key::Char('€')).with_mods(ctrl_alt()).with_text("€").with_altgr();
    assert_eq!(enc(euro, &m), "€");
    let at = KeyInput::new(Key::Char('@')).with_mods(ctrl_alt()).with_text("@").with_altgr();
    assert_eq!(enc(at, &m), "@");
    // AltGr+Shift keeps working too.
    let q = KeyInput::new(Key::Char('q')).with_mods(ctrl_alt() | SHIFT).with_text("Ω").with_altgr();
    assert_eq!(enc(q, &m), "Ω");
}

#[test]
fn ctrl_alt_without_altgr_is_a_chord() {
    let m = legacy();
    // Same modifier state, but the toolkit says it is not AltGr (US layout, no text).
    assert_eq!(enc(chord('e', ctrl_alt()), &m), r"\e\x05");
}

#[test]
fn altgr_without_text_sends_nothing() {
    let m = legacy();
    assert_eq!(enc(KeyInput::new(Key::Char('f')).with_mods(ctrl_alt()).with_altgr(), &m), NOTHING);
}

#[test]
fn control_character_text_is_ignored() {
    let m = legacy();
    // Some toolkits report Ctrl+A as text U+0001; the key decides.
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(CTRL).with_text("\u{1}"), &m), r"\x01");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_text("\u{9b}"), &m), "a");
}

#[test]
fn releases_send_nothing_and_repeats_are_presses() {
    let m = legacy();
    assert_eq!(enc(typed('a').with_event(KeyEvent::Release), &m), NOTHING);
    assert_eq!(enc(chord('c', CTRL).with_event(KeyEvent::Release), &m), NOTHING);
    assert_eq!(enc(key(Key::Enter).with_event(KeyEvent::Release), &m), NOTHING);
    assert_eq!(enc(typed('a').with_event(KeyEvent::Repeat), &m), "a");
    assert_eq!(enc(key(Key::Up).with_event(KeyEvent::Repeat), &m), r"\e[A");
}

#[test]
fn the_windows_key_has_no_legacy_encoding() {
    let m = legacy();
    assert_eq!(enc(chord('a', SUPER), &m), NOTHING);
    assert_eq!(enc(typed('a').with_mods(SUPER), &m), NOTHING);
}

#[test]
fn lock_state_does_not_change_legacy_output() {
    let m = legacy();
    assert_eq!(enc(typed('A').with_mods(CAPS | NUM), &m), "A");
    assert_eq!(enc(key(Key::Up).with_mods(NUM | CAPS), &m), r"\e[A");
    assert_eq!(enc(chord('c', CTRL | NUM), &m), r"\x03");
}

// ---- C0 keys --------------------------------------------------------------------------------

#[test]
fn enter() {
    let m = legacy();
    assert_eq!(enc(key(Key::Enter), &m), r"\r");
    // Shift+Enter has no legacy form: it is Enter.
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\r");
    // Ctrl+Enter is LF on PC layouts.
    assert_eq!(enc(key(Key::Enter).with_mods(CTRL), &m), r"\n");
    assert_eq!(enc(key(Key::Enter).with_mods(CTRL | SHIFT), &m), r"\n");
    assert_eq!(enc(key(Key::Enter).with_mods(ALT), &m), r"\e\r");
    assert_eq!(enc(key(Key::Enter).with_mods(ALT | CTRL), &m), r"\e\n");
}

#[test]
fn tab() {
    let m = legacy();
    assert_eq!(enc(key(Key::Tab), &m), r"\t");
    assert_eq!(enc(key(Key::Tab).with_mods(SHIFT), &m), r"\e[Z");
    // Ctrl+Tab is Tab: legacy cannot tell them apart. Ctrl+I is Tab too.
    assert_eq!(enc(key(Key::Tab).with_mods(CTRL), &m), r"\t");
    assert_eq!(enc(chord('i', CTRL), &m), r"\t");
    assert_eq!(enc(key(Key::Tab).with_mods(CTRL | SHIFT), &m), r"\e[Z");
    assert_eq!(enc(key(Key::Tab).with_mods(ALT), &m), r"\e\t");
    assert_eq!(enc(key(Key::Tab).with_mods(ALT | SHIFT), &m), r"\e\e[Z");
}

#[test]
fn ctrl_m_and_ctrl_j_are_enter_and_linefeed() {
    let m = legacy();
    assert_eq!(enc(chord('m', CTRL), &m), r"\r");
    assert_eq!(enc(chord('j', CTRL), &m), r"\n");
}

#[test]
fn backspace_is_del_and_ctrl_swaps() {
    let m = legacy();
    assert_eq!(enc(key(Key::Backspace), &m), r"\x7f");
    assert_eq!(enc(key(Key::Backspace).with_mods(SHIFT), &m), r"\x7f");
    assert_eq!(enc(key(Key::Backspace).with_mods(CTRL), &m), r"\x08");
    assert_eq!(enc(key(Key::Backspace).with_mods(ALT), &m), r"\e\x7f");
    assert_eq!(enc(key(Key::Backspace).with_mods(CTRL | ALT), &m), r"\e\x08");
}

#[test]
fn backspace_preference_swaps_the_pair() {
    let mut m = legacy();
    m.backspace_sends_delete = false;
    assert_eq!(enc(key(Key::Backspace), &m), r"\x08");
    assert_eq!(enc(key(Key::Backspace).with_mods(CTRL), &m), r"\x7f");
}

#[test]
fn escape() {
    let m = legacy();
    assert_eq!(enc(key(Key::Escape), &m), r"\e");
    assert_eq!(enc(key(Key::Escape).with_mods(CTRL), &m), r"\e");
    assert_eq!(enc(key(Key::Escape).with_mods(SHIFT), &m), r"\e");
    assert_eq!(enc(key(Key::Escape).with_mods(ALT), &m), r"\e\e");
    // Ctrl+[ is Escape in legacy: that is why programs need the kitty protocol to tell them apart.
    assert_eq!(enc(chord('[', CTRL), &m), r"\e");
}

// ---- cursor and editing keys ----------------------------------------------------------------

#[test]
fn cursor_keys_follow_decckm() {
    let normal = legacy();
    let app = app_cursor(legacy());
    for (k, normal_seq, app_seq) in [
        (Key::Up, r"\e[A", r"\eOA"),
        (Key::Down, r"\e[B", r"\eOB"),
        (Key::Right, r"\e[C", r"\eOC"),
        (Key::Left, r"\e[D", r"\eOD"),
        (Key::Home, r"\e[H", r"\eOH"),
        (Key::End, r"\e[F", r"\eOF"),
    ] {
        assert_eq!(enc(key(k), &normal), normal_seq);
        assert_eq!(enc(key(k), &app), app_seq);
    }
}

#[test]
fn modified_cursor_keys_use_csi_with_a_parameter() {
    for modes in [legacy(), app_cursor(legacy())] {
        assert_eq!(enc(key(Key::Up).with_mods(SHIFT), &modes), r"\e[1;2A");
        assert_eq!(enc(key(Key::Up).with_mods(ALT), &modes), r"\e[1;3A");
        assert_eq!(enc(key(Key::Up).with_mods(SHIFT | ALT), &modes), r"\e[1;4A");
        assert_eq!(enc(key(Key::Right).with_mods(CTRL), &modes), r"\e[1;5C");
        assert_eq!(enc(key(Key::Left).with_mods(CTRL | SHIFT), &modes), r"\e[1;6D");
        assert_eq!(enc(key(Key::Down).with_mods(CTRL | ALT), &modes), r"\e[1;7B");
        assert_eq!(enc(key(Key::Home).with_mods(CTRL), &modes), r"\e[1;5H");
        assert_eq!(enc(key(Key::End).with_mods(SHIFT), &modes), r"\e[1;2F");
        assert_eq!(enc(key(Key::Up).with_mods(SUPER), &modes), r"\e[1;9A");
    }
}

#[test]
fn editing_keys() {
    let m = legacy();
    assert_eq!(enc(key(Key::Insert), &m), r"\e[2~");
    assert_eq!(enc(key(Key::Delete), &m), r"\e[3~");
    assert_eq!(enc(key(Key::PageUp), &m), r"\e[5~");
    assert_eq!(enc(key(Key::PageDown), &m), r"\e[6~");
    assert_eq!(enc(key(Key::Delete).with_mods(CTRL), &m), r"\e[3;5~");
    assert_eq!(enc(key(Key::Insert).with_mods(SHIFT), &m), r"\e[2;2~");
    assert_eq!(enc(key(Key::PageUp).with_mods(ALT), &m), r"\e[5;3~");
    // DECCKM does not affect them.
    assert_eq!(enc(key(Key::Delete), &app_cursor(legacy())), r"\e[3~");
}

#[test]
fn function_keys() {
    let m = legacy();
    for (n, seq) in [(1, r"\eOP"), (2, r"\eOQ"), (3, r"\eOR"), (4, r"\eOS")] {
        assert_eq!(enc(key(Key::F(n)), &m), seq);
    }
    for (n, code) in [(5, 15), (6, 17), (7, 18), (8, 19), (9, 20), (10, 21), (11, 23), (12, 24)] {
        assert_eq!(enc(key(Key::F(n)), &m), format!("\\e[{code}~"));
    }
    // DECCKM does not turn F1 into anything else.
    assert_eq!(enc(key(Key::F(1)), &app_cursor(legacy())), r"\eOP");
}

#[test]
fn modified_function_keys() {
    let m = legacy();
    assert_eq!(enc(key(Key::F(1)).with_mods(SHIFT), &m), r"\e[1;2P");
    assert_eq!(enc(key(Key::F(4)).with_mods(CTRL), &m), r"\e[1;5S");
    assert_eq!(enc(key(Key::F(5)).with_mods(CTRL), &m), r"\e[15;5~");
    assert_eq!(enc(key(Key::F(12)).with_mods(SHIFT | ALT), &m), r"\e[24;4~");
}

#[test]
fn vt220_function_keys_f13_to_f20() {
    let m = legacy();
    for (n, code) in [(13, 25), (14, 26), (15, 28), (16, 29), (17, 31), (18, 32), (19, 33), (20, 34)] {
        assert_eq!(enc(key(Key::F(n)), &m), format!("\\e[{code}~"));
    }
}

#[test]
fn function_keys_without_a_legacy_code_send_nothing() {
    let m = legacy();
    for n in [0, 21, 24, 35, 36] {
        assert_eq!(enc(key(Key::F(n)), &m), NOTHING, "F{n}");
    }
}

#[test]
fn menu_and_pause_and_keys_without_encoding() {
    let m = legacy();
    assert_eq!(enc(key(Key::Menu), &m), r"\e[29~");
    assert_eq!(enc(key(Key::Pause), &m), r"\x1a");
    for k in [
        Key::PrintScreen,
        Key::ScrollLock,
        Key::NumLock,
        Key::CapsLock,
        Key::Media(MediaKey::PlayPause),
        Key::Modifier(ModifierKey::LeftShift),
        Key::Modifier(ModifierKey::RightControl),
    ] {
        assert_eq!(enc(key(k), &m), NOTHING, "{k:?}");
    }
}

// ---- keypad ---------------------------------------------------------------------------------

#[test]
fn numeric_keypad_types_its_characters() {
    let m = legacy();
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(5))).with_text("5"), &m), "5");
    // The toolkit may leave text empty: the key knows what it prints.
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(0))), &m), "0");
    for (k, c) in [
        (KeypadKey::Add, "+"),
        (KeypadKey::Subtract, "-"),
        (KeypadKey::Multiply, "*"),
        (KeypadKey::Divide, "/"),
        (KeypadKey::Decimal, "."),
        (KeypadKey::Equal, "="),
        (KeypadKey::Separator, ","),
    ] {
        assert_eq!(enc(key(Key::Keypad(k)), &m), c);
    }
    // NumLock state is not part of legacy encoding.
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(1))).with_mods(NUM), &m), "1");
}

#[test]
fn keypad_enter() {
    let m = legacy();
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Enter)), &m), r"\r");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Enter)).with_mods(CTRL), &m), r"\n");
}

#[test]
fn application_keypad_sends_ss3() {
    let m = app_keypad(legacy());
    for d in 0..=9u8 {
        let expected = format!("\\eO{}", (b'p' + d) as char);
        assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(d))), &m), expected);
    }
    for (k, f) in [
        (KeypadKey::Multiply, 'j'),
        (KeypadKey::Add, 'k'),
        (KeypadKey::Separator, 'l'),
        (KeypadKey::Subtract, 'm'),
        (KeypadKey::Decimal, 'n'),
        (KeypadKey::Divide, 'o'),
        (KeypadKey::Equal, 'X'),
        (KeypadKey::Enter, 'M'),
    ] {
        assert_eq!(enc(key(Key::Keypad(k)), &m), format!("\\eO{f}"));
    }
}

#[test]
fn application_keypad_ignores_modifiers_except_alt() {
    let m = app_keypad(legacy());
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(1))).with_mods(CTRL), &m), r"\eOq");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Enter)).with_mods(SHIFT), &m), r"\eOM");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Enter)).with_mods(ALT), &m), r"\e\eOM");
}

#[test]
fn keypad_navigation_with_numlock_off_acts_like_the_navigation_keys() {
    let m = legacy();
    for (k, seq) in [
        (KeypadKey::Left, r"\e[D"),
        (KeypadKey::Right, r"\e[C"),
        (KeypadKey::Up, r"\e[A"),
        (KeypadKey::Down, r"\e[B"),
        (KeypadKey::Home, r"\e[H"),
        (KeypadKey::End, r"\e[F"),
        (KeypadKey::PageUp, r"\e[5~"),
        (KeypadKey::PageDown, r"\e[6~"),
        (KeypadKey::Insert, r"\e[2~"),
        (KeypadKey::Delete, r"\e[3~"),
        (KeypadKey::Begin, r"\e[E"),
    ] {
        assert_eq!(enc(key(Key::Keypad(k)), &m), seq, "{k:?}");
    }
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Left)), &app_cursor(legacy())), r"\eOD");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Begin)), &app_cursor(legacy())), r"\eOE");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Left)).with_mods(CTRL), &legacy()), r"\e[1;5D");
    // Application keypad does not turn the navigation half into SS3 p..y.
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Left)), &app_keypad(legacy())), r"\e[D");
}

// ---- IME / plain text ------------------------------------------------------------------------

#[test]
fn committed_text_is_utf8_without_controls() {
    assert_eq!(encode_text("你好，世界 😀"), "你好，世界 😀".as_bytes());
    assert_eq!(encode_text("ㄋㄧˇㄏㄠˇ"), "ㄋㄧˇㄏㄠˇ".as_bytes());
    assert_eq!(encode_text("a\u{1b}[31mb\u{9b}c\u{7f}"), b"a[31mbc");
    assert_eq!(encode_text(""), b"");
}
