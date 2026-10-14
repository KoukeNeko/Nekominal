//! The kitty keyboard protocol: flags 1 (disambiguate), 2 (events), 4 (alternates), 8 (all keys),
//! 16 (associated text), alone and combined.

mod common;
use common::*;
use nekominal_input::*;

const D: u8 = 1;
const E: u8 = 2;
const A: u8 = 4;
const K: u8 = 8;
const T: u8 = 16;

fn release(input: KeyInput) -> KeyInput {
    input.with_event(KeyEvent::Release)
}

fn repeat(input: KeyInput) -> KeyInput {
    input.with_event(KeyEvent::Repeat)
}

// ---- flag 1: disambiguate -------------------------------------------------------------------

#[test]
fn d_escape_is_always_an_escape_code() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Escape), &m), r"\e[27u");
    assert_eq!(enc(key(Key::Escape).with_mods(ALT), &m), r"\e[27;3u");
    assert_eq!(enc(key(Key::Escape).with_mods(SHIFT), &m), r"\e[27;2u");
    assert_eq!(enc(key(Key::Escape).with_mods(CTRL), &m), r"\e[27;5u");
}

#[test]
fn d_ctrl_alt_and_shift_alt_chords_become_csi_u_with_the_unshifted_key() {
    let m = kitty(D);
    assert_eq!(enc(chord('a', CTRL), &m), r"\e[97;5u");
    // The key code is always the unshifted one: 97, never 65.
    assert_eq!(enc(chord('a', CTRL | SHIFT), &m), r"\e[97;6u");
    assert_eq!(enc(chord('A', CTRL | SHIFT), &m), r"\e[97;6u");
    assert_eq!(enc(chord('b', ALT), &m), r"\e[98;3u");
    assert_eq!(enc(chord('a', CTRL | ALT), &m), r"\e[97;7u");
    assert_eq!(enc(chord('a', SHIFT | ALT), &m), r"\e[97;4u");
    assert_eq!(enc(chord('a', SUPER), &m), r"\e[97;9u");
    assert_eq!(enc(chord('1', CTRL), &m), r"\e[49;5u");
}

#[test]
fn d_ambiguous_legacy_pairs_are_told_apart() {
    let m = kitty(D);
    // Ctrl+[ vs Escape, Ctrl+I vs Tab, Ctrl+M vs Enter, Ctrl+J vs LF, Ctrl+Space vs NUL.
    assert_eq!(enc(chord('[', CTRL), &m), r"\e[91;5u");
    assert_eq!(enc(key(Key::Escape), &m), r"\e[27u");
    assert_eq!(enc(chord('i', CTRL), &m), r"\e[105;5u");
    assert_eq!(enc(key(Key::Tab), &m), r"\t");
    assert_eq!(enc(chord('m', CTRL), &m), r"\e[109;5u");
    assert_eq!(enc(key(Key::Enter), &m), r"\r");
    assert_eq!(enc(chord(' ', CTRL), &m), r"\e[32;5u");
    assert_eq!(enc(chord('c', CTRL), &m), r"\e[99;5u");
}

#[test]
fn d_plain_and_shift_only_typing_stays_text() {
    let m = kitty(D);
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), "A");
    assert_eq!(enc(KeyInput::new(Key::Char('1')).with_mods(SHIFT).with_text("!"), &m), "!");
    assert_eq!(enc(KeyInput::new(Key::Char('你')).with_text("你"), &m), "你");
    // Lock state alone does not make a key an escape code.
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(CAPS).with_text("A"), &m), "A");
}

#[test]
fn d_shift_enter_ctrl_enter_alt_enter() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Enter), &m), r"\r");
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\e[13;2u");
    assert_eq!(enc(key(Key::Enter).with_mods(ALT), &m), r"\e[13;3u");
    assert_eq!(enc(key(Key::Enter).with_mods(CTRL), &m), r"\e[13;5u");
    assert_eq!(enc(key(Key::Enter).with_mods(CTRL | SHIFT), &m), r"\e[13;6u");
}

#[test]
fn d_tab_and_backspace() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Tab), &m), r"\t");
    // Shift+Tab is Tab with Shift here, not CSI Z (fixterms erratum).
    assert_eq!(enc(key(Key::Tab).with_mods(SHIFT), &m), r"\e[9;2u");
    assert_eq!(enc(key(Key::Tab).with_mods(CTRL), &m), r"\e[9;5u");
    assert_eq!(enc(key(Key::Backspace), &m), r"\x7f");
    assert_eq!(enc(key(Key::Backspace).with_mods(CTRL), &m), r"\e[127;5u");
    assert_eq!(enc(key(Key::Backspace).with_mods(ALT), &m), r"\e[127;3u");
}

#[test]
fn d_lock_state_alone_leaves_the_c0_keys_legacy() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Enter).with_mods(NUM | CAPS), &m), r"\r");
    assert_eq!(enc(key(Key::Backspace).with_mods(NUM), &m), r"\x7f");
}

#[test]
fn d_functional_keys_use_the_csi_forms_even_in_application_cursor_mode() {
    let m = app_cursor(kitty(D));
    assert_eq!(enc(key(Key::Up), &m), r"\e[A");
    assert_eq!(enc(key(Key::Home), &m), r"\e[H");
    assert_eq!(enc(key(Key::End), &m), r"\e[F");
    assert_eq!(enc(key(Key::Up).with_mods(CTRL), &m), r"\e[1;5A");
    assert_eq!(enc(key(Key::F(1)), &m), r"\e[P");
    assert_eq!(enc(key(Key::F(2)), &m), r"\e[Q");
    assert_eq!(enc(key(Key::F(3)), &m), r"\e[13~");
    assert_eq!(enc(key(Key::F(4)), &m), r"\e[S");
    assert_eq!(enc(key(Key::F(3)).with_mods(SHIFT), &m), r"\e[13;2~");
    assert_eq!(enc(key(Key::F(5)), &m), r"\e[15~");
    assert_eq!(enc(key(Key::F(12)).with_mods(CTRL), &m), r"\e[24;5~");
    assert_eq!(enc(key(Key::Delete), &m), r"\e[3~");
    assert_eq!(enc(key(Key::Insert).with_mods(SHIFT), &m), r"\e[2;2~");
    assert_eq!(enc(key(Key::PageUp), &m), r"\e[5~");
    assert_eq!(enc(key(Key::PageDown), &m), r"\e[6~");
}

#[test]
fn d_function_keys_beyond_f12_use_their_private_use_numbers() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::F(13)), &m), r"\e[57376u");
    assert_eq!(enc(key(Key::F(24)), &m), r"\e[57387u");
    assert_eq!(enc(key(Key::F(35)), &m), r"\e[57398u");
    assert_eq!(enc(key(Key::F(13)).with_mods(CTRL), &m), r"\e[57376;5u");
    assert_eq!(enc(key(Key::F(36)), &m), NOTHING);
}

#[test]
fn d_keypad_keys_that_type_text_stay_text() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(5))).with_text("5"), &m), "5");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Add)), &m), "+");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Decimal)).with_mods(NUM), &m), ".");
}

#[test]
fn d_keypad_keys_without_text_are_separate_keys() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Enter)), &m), r"\e[57414u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Left)), &m), r"\e[57417u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Delete)), &m), r"\e[57426u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Home)).with_mods(CTRL), &m), r"\e[57423;5u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Begin)), &m), r"\e[E");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(5))).with_mods(CTRL), &m), r"\e[57404;5u");
}

#[test]
fn d_other_functional_keys() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Menu), &m), r"\e[57363u");
    assert_eq!(enc(key(Key::PrintScreen), &m), r"\e[57361u");
    assert_eq!(enc(key(Key::Pause), &m), r"\e[57362u");
    assert_eq!(enc(key(Key::Media(MediaKey::PlayPause)), &m), r"\e[57430u");
    assert_eq!(enc(key(Key::Media(MediaKey::MuteVolume)), &m), r"\e[57440u");
}

#[test]
fn d_modifier_keys_and_dead_keys_are_not_reported() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Modifier(ModifierKey::LeftShift)), &m), NOTHING);
    assert_eq!(enc(key(Key::CapsLock), &m), NOTHING);
    assert_eq!(enc(key(Key::Dead), &m), NOTHING);
}

#[test]
fn d_releases_are_not_reported_and_repeats_are_presses() {
    let m = kitty(D);
    assert_eq!(enc(release(chord('a', CTRL)), &m), NOTHING);
    assert_eq!(enc(release(key(Key::Escape)), &m), NOTHING);
    assert_eq!(enc(repeat(chord('a', CTRL)), &m), r"\e[97;5u");
    assert_eq!(enc(repeat(key(Key::Up)), &m), r"\e[A");
}

#[test]
fn d_altgr_characters_stay_text() {
    let m = kitty(D);
    let euro = KeyInput::new(Key::Char('€')).with_mods(ctrl_alt()).with_text("€").with_altgr();
    assert_eq!(enc(euro, &m), "€");
}

#[test]
fn d_lock_bits_on_escape_codes_are_opt_in() {
    let mut m = kitty(D);
    assert_eq!(enc(key(Key::Escape).with_mods(NUM), &m), r"\e[27u");
    assert_eq!(enc(chord('a', CTRL).with_mods(CTRL | CAPS), &m), r"\e[97;5u");
    assert_eq!(enc(key(Key::Up).with_mods(NUM), &m), r"\e[A");
    m.lock_modifiers_in_disambiguate = true;
    assert_eq!(enc(key(Key::Escape).with_mods(NUM), &m), r"\e[27;129u");
    assert_eq!(enc(chord('a', CTRL | CAPS), &m), r"\e[97;69u");
    assert_eq!(enc(key(Key::Up).with_mods(NUM), &m), r"\e[1;129A");
}

// ---- flag 2: event types --------------------------------------------------------------------

#[test]
fn e_chords_report_repeat_and_release() {
    let m = kitty(D | E);
    assert_eq!(enc(chord('a', CTRL), &m), r"\e[97;5u");
    assert_eq!(enc(repeat(chord('a', CTRL)), &m), r"\e[97;5:2u");
    assert_eq!(enc(release(chord('a', CTRL)), &m), r"\e[97;5:3u");
}

#[test]
fn e_text_keys_press_as_text_but_release_as_csi_u() {
    let m = kitty(D | E);
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(repeat(typed('a')), &m), "a");
    assert_eq!(enc(release(typed('a')), &m), r"\e[97;1:3u");
    assert_eq!(enc(release(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A")), &m), r"\e[97;2:3u");
}

#[test]
fn e_enter_tab_backspace_have_no_release_without_all_keys() {
    let m = kitty(D | E);
    for k in [Key::Enter, Key::Tab, Key::Backspace] {
        assert_eq!(enc(release(key(k)), &m), NOTHING, "{k:?}");
        assert_eq!(enc(release(key(k)).with_mods(SHIFT), &m), NOTHING, "{k:?}");
    }
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\e[13;2u");
    assert_eq!(enc(repeat(key(Key::Enter).with_mods(SHIFT)), &m), r"\e[13;2:2u");
}

#[test]
fn e_functional_keys_carry_the_event_type() {
    let m = kitty(D | E);
    assert_eq!(enc(release(key(Key::Escape)), &m), r"\e[27;1:3u");
    assert_eq!(enc(repeat(key(Key::Up)), &m), r"\e[1;1:2A");
    assert_eq!(enc(release(key(Key::Up)), &m), r"\e[1;1:3A");
    assert_eq!(enc(release(key(Key::Up).with_mods(CTRL)), &m), r"\e[1;5:3A");
    assert_eq!(enc(release(key(Key::F(5))), &m), r"\e[15;1:3~");
    assert_eq!(enc(release(key(Key::F(13))), &m), r"\e[57376;1:3u");
    assert_eq!(enc(release(key(Key::Keypad(KeypadKey::Enter))), &m), r"\e[57414;1:3u");
}

#[test]
fn e_alone_changes_functional_keys_to_csi_and_leaves_the_rest_legacy() {
    let m = app_cursor(kitty(E));
    // Cursor keys switch to the CSI form (a program that asked for events parses CSI only).
    assert_eq!(enc(key(Key::Up), &m), r"\e[A");
    assert_eq!(enc(release(key(Key::Up)), &m), r"\e[1;1:3A");
    // Escape and typing stay legacy on press.
    assert_eq!(enc(key(Key::Escape), &m), r"\e");
    assert_eq!(enc(release(key(Key::Escape)), &m), r"\e[27;1:3u");
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(chord('c', CTRL), &m), r"\x03");
    assert_eq!(enc(release(chord('c', CTRL)), &m), r"\e[99;5:3u");
    // F13 is still the VT220 code on press.
    assert_eq!(enc(key(Key::F(13)), &m), r"\e[25~");
}

#[test]
fn e_modifier_keys_are_not_reported_without_all_keys() {
    let m = kitty(D | E);
    let shift = KeyInput::new(Key::Modifier(ModifierKey::LeftShift));
    assert_eq!(enc(shift.clone(), &m), NOTHING);
    assert_eq!(enc(release(shift), &m), NOTHING);
}

// ---- flag 8: all keys as escape codes -------------------------------------------------------

#[test]
fn k_every_key_is_an_escape_code() {
    let m = kitty(K);
    assert_eq!(enc(typed('a'), &m), r"\e[97u");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), r"\e[97;2u");
    assert_eq!(enc(typed(' '), &m), r"\e[32u");
    assert_eq!(enc(key(Key::Enter), &m), r"\e[13u");
    assert_eq!(enc(key(Key::Tab), &m), r"\e[9u");
    assert_eq!(enc(key(Key::Backspace), &m), r"\e[127u");
    assert_eq!(enc(key(Key::Escape), &m), r"\e[27u");
    assert_eq!(enc(typed('你'), &m), r"\e[20320u");
    // Functional keys keep their forms.
    assert_eq!(enc(key(Key::Up), &m), r"\e[A");
    assert_eq!(enc(key(Key::F(1)), &m), r"\e[P");
}

#[test]
fn k_lock_state_is_reported_on_every_key() {
    let m = kitty(K);
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(CAPS).with_text("A"), &m), r"\e[97;65u");
    assert_eq!(enc(key(Key::Enter).with_mods(NUM), &m), r"\e[13;129u");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(CAPS | NUM), &m), r"\e[97;193u");
    assert_eq!(enc(key(Key::Up).with_mods(NUM), &m), r"\e[1;129A");
    // Shift + lock combine.
    assert_eq!(enc(chord('a', SHIFT | CAPS), &m), r"\e[97;66u");
}

#[test]
fn k_modifier_keys_are_reported_with_their_own_bit() {
    let m = kitty(K);
    let press = |k: ModifierKey| KeyInput::new(Key::Modifier(k));
    assert_eq!(enc(press(ModifierKey::LeftShift), &m), r"\e[57441;2u");
    assert_eq!(enc(press(ModifierKey::LeftControl), &m), r"\e[57442;5u");
    assert_eq!(enc(press(ModifierKey::LeftAlt), &m), r"\e[57443;3u");
    assert_eq!(enc(press(ModifierKey::LeftSuper), &m), r"\e[57444;9u");
    assert_eq!(enc(press(ModifierKey::LeftHyper), &m), r"\e[57445;17u");
    assert_eq!(enc(press(ModifierKey::LeftMeta), &m), r"\e[57446;33u");
    assert_eq!(enc(press(ModifierKey::RightShift), &m), r"\e[57447;2u");
    assert_eq!(enc(press(ModifierKey::RightControl), &m), r"\e[57448;5u");
    assert_eq!(enc(press(ModifierKey::RightAlt), &m), r"\e[57449;3u");
    assert_eq!(enc(press(ModifierKey::RightSuper), &m), r"\e[57450;9u");
    // Other modifiers already held are kept.
    assert_eq!(enc(press(ModifierKey::LeftControl).with_mods(SHIFT), &m), r"\e[57442;6u");
}

#[test]
fn k_modifier_key_release_and_repeat() {
    let m = kitty(K | E);
    let ctrl = KeyInput::new(Key::Modifier(ModifierKey::LeftControl));
    // On release the toolkit's state no longer has Ctrl.
    assert_eq!(enc(release(ctrl.clone()), &m), r"\e[57442;1:3u");
    // Two Ctrl keys down, one released: the toolkit still reports Ctrl.
    assert_eq!(enc(release(ctrl.clone()).with_mods(CTRL), &m), r"\e[57442;5:3u");
    // Windows auto-repeats held modifiers; that is noise.
    assert_eq!(enc(repeat(ctrl), &m), NOTHING);
}

#[test]
fn k_lock_keys() {
    let m = kitty(K);
    assert_eq!(enc(key(Key::CapsLock).with_mods(CAPS), &m), r"\e[57358;65u");
    assert_eq!(enc(key(Key::ScrollLock), &m), r"\e[57359u");
    assert_eq!(enc(key(Key::NumLock).with_mods(NUM), &m), r"\e[57360;129u");
}

#[test]
fn k_keypad_keys_are_always_keypad_keys() {
    let m = kitty(K);
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(1))).with_text("1"), &m), r"\e[57400u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Digit(1))).with_text("1").with_mods(NUM), &m), r"\e[57400;129u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Add)), &m), r"\e[57413u");
    assert_eq!(enc(key(Key::Keypad(KeypadKey::Decimal)), &m), r"\e[57409u");
}

#[test]
fn k_release_and_repeat_events_of_text_keys() {
    let m = kitty(K | E);
    assert_eq!(enc(typed('a'), &m), r"\e[97u");
    assert_eq!(enc(repeat(typed('a')), &m), r"\e[97;1:2u");
    assert_eq!(enc(release(typed('a')), &m), r"\e[97;1:3u");
    assert_eq!(enc(release(key(Key::Enter)), &m), r"\e[13;1:3u");
    assert_eq!(enc(release(key(Key::Backspace)), &m), r"\e[127;1:3u");
}

// ---- flag 16: associated text ---------------------------------------------------------------

#[test]
fn t_text_is_embedded_as_code_points() {
    let m = kitty(K | T);
    assert_eq!(enc(typed('a'), &m), r"\e[97;;97u");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), r"\e[97;2;65u");
    assert_eq!(enc(KeyInput::new(Key::Char('h')).with_text("hi"), &m), r"\e[104;;104:105u");
    assert_eq!(enc(typed('你'), &m), r"\e[20320;;20320u");
    assert_eq!(enc(typed('😀'), &m), r"\e[128512;;128512u");
}

#[test]
fn t_text_without_a_key_uses_key_number_zero() {
    let m = kitty(K | T);
    assert_eq!(enc(KeyInput::new(Key::Unidentified).with_text("å"), &m), r"\e[0;;229u");
    assert_eq!(enc(KeyInput::new(Key::Unidentified).with_text("你好"), &m), r"\e[0;;20320:22909u");
    // Without the text flag there is nothing to report: plain text.
    assert_eq!(enc(KeyInput::new(Key::Unidentified).with_text("你"), &kitty(K)), "你");
    assert_eq!(enc(KeyInput::new(Key::Unidentified), &m), NOTHING);
}

#[test]
fn t_chords_carry_no_text_and_keys_derive_it_when_missing() {
    let m = kitty(K | T);
    assert_eq!(enc(chord('a', CTRL), &m), r"\e[97;5u");
    // A toolkit that does not pass text for a plain key press still gets the text.
    assert_eq!(enc(key(Key::Char('a')), &m), r"\e[97;;97u");
    assert_eq!(enc(chord('a', SHIFT), &m), r"\e[97;2;65u");
}

#[test]
fn t_events_other_than_press_have_no_text() {
    let m = kitty(K | E | T);
    assert_eq!(enc(typed('a'), &m), r"\e[97;;97u");
    assert_eq!(enc(repeat(typed('a')), &m), r"\e[97;1:2;97u");
    assert_eq!(enc(release(typed('a')), &m), r"\e[97;1:3u");
}

#[test]
fn t_altgr_text_with_all_keys() {
    let m = kitty(K | T);
    let euro = KeyInput::new(Key::Char('€')).with_mods(ctrl_alt()).with_text("€").with_altgr();
    assert_eq!(enc(euro, &m), r"\e[8364;;8364u");
}

#[test]
fn t_without_all_keys_text_only_rides_on_chords() {
    // Flag 16 is defined as an enhancement of flag 8; D alone plus T still sends plain typing as text.
    let m = kitty(D | T);
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(chord('a', ALT), &m), r"\e[97;3u");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(ALT).with_text("å"), &m), r"\e[97;3;229u");
}

// ---- flag 4: alternate keys -----------------------------------------------------------------

#[test]
fn a_shifted_key_is_reported_when_shift_is_held() {
    let m = kitty(D | A);
    assert_eq!(enc(chord('a', CTRL | SHIFT), &m), r"\e[97:65;6u");
    assert_eq!(enc(chord('1', CTRL | SHIFT).with_shifted('!'), &m), r"\e[49:33;6u");
    assert_eq!(enc(KeyInput::new(Key::Char('=')).with_mods(CTRL | SHIFT).with_text("+"), &m), r"\e[61:43;6u");
    // Not held: nothing to report.
    assert_eq!(enc(chord('a', CTRL), &m), r"\e[97;5u");
}

#[test]
fn a_shifted_symbol_defaults_to_the_us_layout() {
    let m = kitty(D | A);
    assert_eq!(enc(chord('1', CTRL | SHIFT), &m), r"\e[49:33;6u");
    assert_eq!(enc(chord('-', CTRL | SHIFT), &m), r"\e[45:95;6u");
    // The toolkit says better: a layout where Shift+2 is a double quote.
    assert_eq!(enc(chord('2', CTRL | SHIFT).with_shifted('"'), &m), r"\e[50:34;6u");
}

#[test]
fn a_base_layout_key_is_reported_when_it_differs() {
    let m = kitty(D | A);
    // Russian layout, Ctrl+С: the key is с (1089), the PC-101 key is c (99).
    assert_eq!(enc(chord('с', CTRL).with_base_layout('c'), &m), r"\e[1089::99;5u");
    assert_eq!(enc(chord('с', CTRL | SHIFT).with_base_layout('c').with_shifted('С'), &m), r"\e[1089:1057:99;6u");
    // Same character: not repeated.
    assert_eq!(enc(chord('c', CTRL).with_base_layout('c'), &m), r"\e[99;5u");
}

#[test]
fn a_alone_or_with_text_only_has_no_effect() {
    assert_eq!(enc(chord('c', CTRL), &kitty(A)), r"\x03");
    assert_eq!(enc(chord('c', CTRL), &kitty(T)), r"\x03");
    assert_eq!(enc(chord('c', CTRL), &kitty(A | T)), r"\x03");
    assert_eq!(enc(typed('a'), &kitty(A | T)), "a");
}

#[test]
fn a_does_not_touch_functional_keys() {
    let m = kitty(D | A);
    assert_eq!(enc(key(Key::Up).with_mods(SHIFT), &m), r"\e[1;2A");
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\e[13;2u");
    assert_eq!(enc(key(Key::F(13)).with_mods(SHIFT), &m), r"\e[57376;2u");
}

#[test]
fn a_with_all_keys_and_text_uses_every_field() {
    let m = kitty(K | A | T);
    assert_eq!(
        enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m),
        r"\e[97:65;2;65u"
    );
    assert_eq!(enc(typed('a'), &m), r"\e[97;;97u");
}

#[test]
fn full_protocol_flags() {
    let m = kitty(D | E | A | K | T);
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), r"\e[97:65;2;65u");
    assert_eq!(enc(release(KeyInput::new(Key::Char('a')).with_mods(SHIFT)), &m), r"\e[97:65;2:3u");
    assert_eq!(enc(key(Key::Enter), &m), r"\e[13u");
    assert_eq!(enc(release(key(Key::Escape)), &m), r"\e[27;1:3u");
    assert_eq!(enc(repeat(key(Key::Left)), &m), r"\e[1;1:2D");
    assert_eq!(enc(key(Key::Modifier(ModifierKey::LeftShift)), &m), r"\e[57441;2u");
}

#[test]
fn no_flags_change_nothing() {
    let m = kitty(0);
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\r");
    assert_eq!(enc(key(Key::Escape), &m), r"\e");
    assert_eq!(enc(chord('a', CTRL), &m), r"\x01");
}

// ---- parity with the previous implementation (legacy/tauri-xterm/src/kitty-keyboard.test.ts)

#[test]
fn parity_modified_enter_tab_and_backspace_become_csi_u() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\e[13;2u");
    assert_eq!(enc(key(Key::Enter).with_mods(CTRL), &m), r"\e[13;5u");
    assert_eq!(enc(key(Key::Tab).with_mods(SHIFT), &m), r"\e[9;2u");
    assert_eq!(enc(key(Key::Backspace).with_mods(ALT), &m), r"\e[127;3u");
}

#[test]
fn parity_plain_enter_tab_and_backspace_stay_legacy() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Enter), &m), r"\r");
    assert_eq!(enc(key(Key::Tab), &m), r"\t");
    assert_eq!(enc(key(Key::Backspace), &m), r"\x7f");
}

#[test]
fn parity_escape_is_always_disambiguated() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Escape), &m), r"\e[27u");
    assert_eq!(enc(key(Key::Escape).with_mods(ALT), &m), r"\e[27;3u");
}

#[test]
fn parity_ctrl_and_alt_text_keys_use_the_unshifted_key_code() {
    let m = kitty(D);
    assert_eq!(enc(chord('a', CTRL), &m), r"\e[97;5u");
    assert_eq!(enc(chord('A', CTRL | SHIFT), &m), r"\e[97;6u");
    assert_eq!(enc(chord('b', ALT), &m), r"\e[98;3u");
    assert_eq!(enc(chord(' ', CTRL), &m), r"\e[32;5u");
}

#[test]
fn parity_plain_and_shift_only_text_is_text() {
    let m = kitty(D);
    assert_eq!(enc(typed('a'), &m), "a");
    assert_eq!(enc(KeyInput::new(Key::Char('a')).with_mods(SHIFT).with_text("A"), &m), "A");
}

#[test]
fn parity_functional_keys_keep_the_xterm_modified_forms() {
    // The old code returned "legacy applies" and let xterm.js produce these bytes.
    let m = kitty(D);
    assert_eq!(enc(key(Key::Up).with_mods(CTRL), &m), r"\e[1;5A");
    assert_eq!(enc(key(Key::F(5)).with_mods(SHIFT), &m), r"\e[15;2~");
    assert_eq!(enc(key(Key::Delete).with_mods(SHIFT), &m), r"\e[3;2~");
}

#[test]
fn parity_modifier_only_presses_are_not_reported() {
    let m = kitty(D);
    assert_eq!(enc(key(Key::Modifier(ModifierKey::LeftShift)).with_mods(SHIFT), &m), NOTHING);
    assert_eq!(enc(key(Key::Modifier(ModifierKey::LeftControl)).with_mods(CTRL), &m), NOTHING);
}

#[test]
fn parity_altgr_presses_are_text() {
    let m = kitty(D);
    let euro = KeyInput::new(Key::Char('e')).with_mods(CTRL | ALT).with_text("€").with_altgr();
    assert_eq!(enc(euro, &m), "€");
}

#[test]
fn parity_nothing_is_encoded_while_the_protocol_is_off() {
    let m = kitty(0);
    assert_eq!(enc(key(Key::Enter).with_mods(SHIFT), &m), r"\r");
    assert_eq!(enc(key(Key::Escape), &m), r"\e");
}
