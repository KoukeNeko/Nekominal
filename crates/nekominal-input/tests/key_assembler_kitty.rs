//! KeyAssembler sequence tests, part 2: the kitty keyboard protocol route (the program pushed flags),
//! key release and repeat events, alternate keys from the scan code, and route changes mid-press.
//! Flags: D 1 disambiguate, E 2 events, A 4 alternates, K 8 all keys, T 16 text.

mod common;
use PhysicalKey::*;
use common::typist::*;
use nekominal_input::*;

const D: u8 = 1;
const E: u8 = 2;
const A: u8 = 4;
const K: u8 = 8;
const T: u8 = 16;

fn kitty_rig(bits: u8) -> Rig {
    Rig::us().with_kitty(bits)
}

// ---- disambiguate -------------------------------------------------------------------------------

#[test]
fn d_plain_typing_is_text() {
    let mut rig = kitty_rig(D);
    rig.type_text("Hi!");
    assert_eq!(rig.take(), "Hi!");
}

#[test]
fn d_ctrl_c_is_csi_u() {
    let mut rig = kitty_rig(D);
    rig.chord(&[Mod::Ctrl], KeyC);
    assert_eq!(rig.take(), r"\e[99;5u");
}

#[test]
fn d_alt_letter_is_csi_u() {
    let mut rig = kitty_rig(D);
    rig.chord(&[Mod::Alt], KeyA);
    assert_eq!(rig.take(), r"\e[97;3u");
}

#[test]
fn d_shift_enter_is_csi_u_and_plain_enter_stays_a_carriage_return() {
    let mut rig = kitty_rig(D);
    rig.chord(&[Mod::Shift], Enter);
    assert_eq!(rig.take(), r"\e[13;2u");
    rig.tap(Enter);
    assert_eq!(rig.take(), r"\r");
}

#[test]
fn d_escape_ctrl_space_and_alt_escape() {
    let mut rig = kitty_rig(D);
    rig.tap(Escape);
    assert_eq!(rig.take(), r"\e[27u");
    rig.chord(&[Mod::Ctrl], Space);
    assert_eq!(rig.take(), r"\e[32;5u");
    rig.chord(&[Mod::Alt], Escape);
    assert_eq!(rig.take(), r"\e[27;3u");
}

#[test]
fn d_tab_and_shift_tab() {
    let mut rig = kitty_rig(D);
    rig.tap(Tab);
    assert_eq!(rig.take(), r"\t");
    rig.chord(&[Mod::Shift], Tab);
    assert_eq!(rig.take(), r"\e[9;2u");
}

#[test]
fn d_cursor_function_and_keypad_keys() {
    let mut rig = kitty_rig(D);
    rig.tap(ArrowUp);
    rig.tap(F5);
    rig.tap(F13);
    rig.tap(ContextMenu);
    rig.tap(NumpadEnter);
    assert_eq!(rig.take(), r"\e[A\e[15~\e[57376u\e[57363u\e[57414u");
}

#[test]
fn d_keypad_navigation_with_numlock_off_is_the_keypad_key() {
    let mut rig = kitty_rig(D);
    rig.tap(Numpad4);
    assert_eq!(rig.take(), r"\e[57417u");
    // The dedicated cluster is the plain key.
    rig.tap(ArrowLeft);
    assert_eq!(rig.take(), r"\e[D");
}

#[test]
fn d_keypad_digits_with_numlock_on_stay_text() {
    let mut rig = kitty_rig(D);
    rig.num_lock(true);
    rig.tap(Numpad7);
    rig.tap(NumpadAdd);
    assert_eq!(rig.take(), "7+");
}

#[test]
fn d_modifier_keys_and_lock_keys_send_nothing() {
    let mut rig = kitty_rig(D);
    rig.hold(Mod::Shift);
    rig.release(Mod::Shift);
    rig.tap(CapsLock);
    assert_eq!(rig.take(), "");
}

#[test]
fn d_ctrl_alt_chord_without_a_character_is_csi_u() {
    let mut rig = kitty_rig(D);
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.tap(KeyQ);
    assert_eq!(rig.take(), r"\e[113;7u");
}

#[test]
fn d_without_events_a_repeat_is_just_another_press() {
    let mut rig = kitty_rig(D);
    rig.key_down(KeyA);
    rig.repeat(KeyA, 2);
    rig.key_up(KeyA);
    assert_eq!(rig.take(), "aaa");
}

// ---- events -------------------------------------------------------------------------------------

#[test]
fn e_text_keys_press_as_text_and_release_as_csi_u() {
    let mut rig = kitty_rig(D | E);
    rig.tap(KeyA);
    assert_eq!(rig.take(), r"a\e[97;1:3u");
}

#[test]
fn e_the_release_of_a_shifted_key_keeps_the_modifier_that_is_still_down() {
    let mut rig = kitty_rig(D | E);
    rig.hold(Mod::Shift);
    rig.tap(KeyA);
    rig.release(Mod::Shift);
    assert_eq!(rig.take(), r"A\e[97;2:3u");
}

#[test]
fn e_a_chord_reports_press_repeat_and_release() {
    let mut rig = kitty_rig(D | E);
    rig.hold(Mod::Ctrl);
    rig.key_down(KeyA);
    rig.repeat(KeyA, 2);
    rig.key_up(KeyA);
    assert_eq!(rig.take(), r"\e[97;5u\e[97;5:2u\e[97;5:2u\e[97;5:3u");
}

#[test]
fn e_enter_has_no_release_but_escape_and_arrows_do() {
    let mut rig = kitty_rig(D | E);
    rig.tap(Enter);
    assert_eq!(rig.take(), r"\r");
    rig.tap(Escape);
    assert_eq!(rig.take(), r"\e[27u\e[27;1:3u");
    rig.tap(ArrowUp);
    assert_eq!(rig.take(), r"\e[A\e[1;1:3A");
}

// ---- all keys ------------------------------------------------------------------------------------

#[test]
fn k_every_key_is_an_escape_code() {
    let mut rig = kitty_rig(K);
    rig.tap(KeyA);
    rig.tap(Enter);
    rig.tap(Backspace);
    rig.tap(Tab);
    rig.tap(Space);
    assert_eq!(rig.take(), r"\e[97u\e[13u\e[127u\e[9u\e[32u");
}

#[test]
fn k_modifier_keys_are_reported_with_their_side() {
    let mut rig = kitty_rig(K);
    rig.hold(Mod::Shift);
    rig.hold(Mod::RightCtrl);
    assert_eq!(rig.take(), r"\e[57441;2u\e[57448;6u");
}

#[test]
fn k_shifted_letters_carry_the_modifier_but_not_the_capital() {
    let mut rig = kitty_rig(K);
    rig.chord(&[Mod::Shift], KeyA);
    assert_eq!(rig.take(), r"\e[57441;2u\e[97;2u");
}

#[test]
fn k_lock_keys_report_the_lock_state() {
    let mut rig = kitty_rig(K);
    rig.tap(CapsLock);
    assert_eq!(rig.take(), r"\e[57358;65u");
}

#[test]
fn k_and_e_report_releases_of_everything_and_drop_modifier_repeats() {
    let mut rig = kitty_rig(K | E);
    rig.hold(Mod::Shift);
    rig.hold_repeat(Mod::Shift);
    rig.tap(KeyA);
    rig.release(Mod::Shift);
    assert_eq!(rig.take(), r"\e[57441;2u\e[97;2u\e[97;2:3u\e[57441;1:3u");
}

#[test]
fn k_and_e_an_auto_repeat_is_event_type_two() {
    let mut rig = kitty_rig(K | E);
    rig.key_down(KeyA);
    rig.repeat(KeyA, 1);
    rig.key_up(KeyA);
    assert_eq!(rig.take(), r"\e[97u\e[97;1:2u\e[97;1:3u");
}

#[test]
fn k_and_e_enter_is_released_too() {
    let mut rig = kitty_rig(K | E);
    rig.tap(Enter);
    assert_eq!(rig.take(), r"\e[13u\e[13;1:3u");
}

// ---- text and alternates ----------------------------------------------------------------------------

#[test]
fn kt_text_rides_on_the_escape_code() {
    let mut rig = kitty_rig(K | T);
    rig.tap(KeyA);
    assert_eq!(rig.take(), r"\e[97;;97u");
    rig.chord(&[Mod::Shift], KeyA);
    assert_eq!(rig.take(), r"\e[57441;2u\e[97;2;65u");
}

#[test]
fn kt_a_composed_character_is_keyed_by_itself() {
    let mut rig = Rig::new(Layout::french()).with_kitty(K | T);
    rig.type_text("é");
    assert_eq!(rig.take(), r"\e[233;;233u");
}

#[test]
fn a_the_base_layout_key_comes_from_the_scan_code() {
    // AZERTY: the key labelled A is where Q is on a US board.
    let mut rig = Rig::new(Layout::french()).with_kitty(D | A);
    rig.chord(&[Mod::Ctrl], KeyQ);
    assert_eq!(rig.take(), r"\e[97::113;5u");
    // QWERTZ: the key labelled Z is where Y is.
    let mut rig = Rig::new(Layout::german()).with_kitty(D | A);
    rig.chord(&[Mod::Ctrl], KeyY);
    assert_eq!(rig.take(), r"\e[122::121;5u");
}

#[test]
fn a_a_key_on_its_own_position_has_no_base_layout_field() {
    let mut rig = kitty_rig(D | A);
    rig.chord(&[Mod::Ctrl], KeyC);
    assert_eq!(rig.take(), r"\e[99;5u");
    // Ctrl+С on a Russian layout is the key C of the board.
    let mut rig = Rig::new(Layout::russian()).with_kitty(D | A);
    rig.chord(&[Mod::Ctrl], KeyC);
    assert_eq!(rig.take(), r"\e[99;5u");
}

#[test]
fn a_the_shifted_key_is_the_us_symbol_when_nothing_was_typed() {
    let mut rig = kitty_rig(D | A);
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Shift);
    rig.tap(Digit1);
    assert_eq!(rig.take(), r"\e[49:33;6u");
}

// ---- AltGr and dead keys under kitty -------------------------------------------------------------------

#[test]
fn d_altgr_characters_are_text() {
    let mut rig = Rig::new(Layout::german()).with_kitty(D);
    rig.type_text("€@{");
    assert_eq!(rig.take(), "€@{");
}

#[test]
fn kt_altgr_is_keyed_by_the_character_and_reports_both_modifier_keys() {
    let mut rig = Rig::new(Layout::german()).with_kitty(K | T);
    rig.type_text("€");
    // Left Ctrl and right Alt as Windows presses them for AltGr, then the character, whose key
    // has no Ctrl and Alt of its own.
    assert_eq!(rig.take(), r"\e[57442;5u\e[57449;7u\e[8364;;8364u");
}

#[test]
fn d_dead_key_and_vowel_is_one_character() {
    let mut rig = Rig::new(Layout::french()).with_kitty(D);
    rig.type_text("ê");
    assert_eq!(rig.take(), "ê");
}

#[test]
fn k_dead_key_sends_nothing_and_the_composed_character_is_keyed_by_itself() {
    let mut rig = Rig::new(Layout::french()).with_kitty(K);
    rig.type_text("ê");
    assert_eq!(rig.take(), r"\e[234u");
}

// ---- changing routes ---------------------------------------------------------------------------------------

#[test]
fn a_release_is_not_sent_on_another_route_than_the_press() {
    let mut rig = Rig::us();
    rig.key_down(KeyA);
    assert_eq!(rig.take(), record(0x41, 0x1E, 'a' as u16, true, 0));
    // The program pushed kitty flags while the key was down.
    rig.modes.kitty = KittyFlags::from_bits_truncate(D | E);
    rig.key_up(KeyA);
    assert_eq!(rig.take(), "");
}

#[test]
fn a_release_is_not_sent_for_a_press_that_went_to_a_program_that_left_kitty_mode() {
    let mut rig = kitty_rig(D | E);
    rig.key_down(KeyA);
    assert_eq!(rig.take(), "a");
    rig.modes.kitty = KittyFlags::NONE;
    rig.key_up(KeyA);
    assert_eq!(rig.take(), "");
}

#[test]
fn the_route_follows_the_flags_for_every_key() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Shift], Enter);
    assert_eq!(
        rig.take(),
        record(0x10, 0x2A, 0, true, 0x10) + &tap_records(0x0D, 0x1C, 13, 0x10) + &record(0x10, 0x2A, 0, false, 0)
    );
    rig.modes.kitty = KittyFlags::from_bits_truncate(D);
    rig.chord(&[Mod::Shift], Enter);
    assert_eq!(rig.take(), r"\e[13;2u");
}
