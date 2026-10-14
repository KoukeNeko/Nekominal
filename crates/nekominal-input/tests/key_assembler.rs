//! KeyAssembler sequence tests, part 1: plain typing, the keys encoded from the event and the Win32
//! record route (what a program gets while it has pushed no kitty flags). Events come from the typist
//! in `common/typist.rs`, which plays Windows and XAML.

mod common;
use PhysicalKey::*;
use common::{typist::*, *};
use nekominal_input::win32::control_state::*;
use nekominal_input::*;

fn stroke(vk: i32, scan_code: u16, extended: bool, mods: Mods) -> KeyStroke {
    KeyStroke::new(vk, scan_code, extended, mods)
}

// ---- text ---------------------------------------------------------------------------------------

#[test]
fn a_letter_is_a_key_record_with_its_character() {
    let mut rig = Rig::us();
    rig.tap(KeyA);
    assert_eq!(rig.take(), tap_records(0x41, 0x1E, 'a' as u16, 0));
}

#[test]
fn a_text_key_is_left_unhandled_so_that_the_character_arrives() {
    let mut rig = Rig::us();
    rig.key_down(KeyA);
    // KeyDown, CharacterReceived, Flush.
    assert_eq!(rig.log.len(), 3);
    let down = &rig.log[0];
    assert!(!down.handled && down.pending && down.bytes.is_empty());
    assert!(rig.log[1].handled, "the character is consumed");
    assert_eq!(rig.take(), record(0x41, 0x1E, 'a' as u16, true, 0));
}

#[test]
fn nothing_is_sent_before_the_character_arrives() {
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let out = asm.handle(KeyboardEvent::KeyDown(stroke(0x41, 0x1E, false, Mods::NONE)), &modes);
    assert!(out.bytes.is_empty() && out.pending && !out.handled);
    let out = asm.handle(KeyboardEvent::CharacterReceived('a' as u16), &modes);
    assert_eq!(show(&out.bytes), record(0x41, 0x1E, 'a' as u16, true, 0));
    assert!(out.handled && !out.pending);
}

#[test]
fn shift_makes_capitals_and_shows_in_the_control_state() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Shift], KeyA);
    assert_eq!(
        rig.take(),
        record(0x10, 0x2A, 0, true, SHIFT_PRESSED)
            + &tap_records(0x41, 0x1E, 'A' as u16, SHIFT_PRESSED)
            + &record(0x10, 0x2A, 0, false, 0)
    );
}

#[test]
fn caps_lock_types_capitals_and_reports_the_lock() {
    let mut rig = Rig::us();
    rig.tap(CapsLock);
    assert_eq!(rig.take(), tap_records(0x14, 0x3A, 0, CAPSLOCK_ON));
    rig.tap(KeyA);
    assert_eq!(rig.take(), tap_records(0x41, 0x1E, 'A' as u16, CAPSLOCK_ON));
}

#[test]
fn digits_and_punctuation_carry_their_virtual_keys() {
    let mut rig = Rig::us();
    rig.tap(Digit1);
    rig.tap(Semicolon);
    rig.tap(Comma);
    assert_eq!(
        rig.take(),
        tap_records(0x31, 0x02, '1' as u16, 0)
            + &tap_records(0xBA, 0x27, ';' as u16, 0)
            + &tap_records(0xBC, 0x33, ',' as u16, 0)
    );
}

#[test]
fn the_space_bar_is_text() {
    let mut rig = Rig::us();
    rig.tap(Space);
    assert_eq!(rig.take(), tap_records(0x20, 0x39, 0x20, 0));
    assert!(!rig.log[0].handled);
}

#[test]
fn auto_repeat_is_a_down_record_per_event_and_one_release() {
    let mut rig = Rig::us();
    rig.key_down(KeyA);
    rig.repeat(KeyA, 2);
    rig.key_up(KeyA);
    let down = record(0x41, 0x1E, 'a' as u16, true, 0);
    assert_eq!(rig.take(), down.repeat(3) + &record(0x41, 0x1E, 'a' as u16, false, 0));
}

#[test]
fn text_nobody_asked_for_goes_out_as_utf8() {
    // Alt+numpad entry: the composed character arrives after the digit keys, with no key before it.
    let mut rig = Rig::us();
    rig.character('é' as u16);
    assert_eq!(rig.take(), "é");
    // A control character is never text.
    rig.character(0x1B);
    rig.character(0x7F);
    assert_eq!(rig.take(), "");
}

// ---- keys encoded from the event ------------------------------------------------------------------

#[test]
fn enter_tab_backspace_and_escape_are_handled_and_carry_their_control_characters() {
    let mut rig = Rig::us();
    for key in [Enter, Tab, Backspace, Escape] {
        rig.tap(key);
    }
    assert_eq!(
        rig.take(),
        tap_records(0x0D, 0x1C, 13, 0)
            + &tap_records(0x09, 0x0F, 9, 0)
            + &tap_records(0x08, 0x0E, 8, 0)
            + &tap_records(0x1B, 0x01, 27, 0)
    );
    // No character event follows any of them: they were handled.
    assert!(rig.log.iter().all(|step| matches!(step.event, KeyboardEvent::KeyDown(_) | KeyboardEvent::KeyUp(_))));
    assert!(rig.log.iter().all(|step| step.handled));
}

#[test]
fn tab_without_a_scan_code_gets_the_position_of_the_key() {
    // XAML reports ScanCode 0 for Tab (it preprocesses focus navigation keys).
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let tab = stroke(0x09, 0, false, Mods::NONE);
    let down = asm.handle(KeyboardEvent::KeyDown(tab), &modes);
    let up = asm.handle(KeyboardEvent::KeyUp(tab), &modes);
    assert_eq!(show(&down.bytes), record(0x09, 0x0F, 9, true, 0));
    assert_eq!(show(&up.bytes), record(0x09, 0x0F, 9, false, 0));
}

#[test]
fn arrows_are_extended_keys() {
    let mut rig = Rig::us();
    rig.tap(ArrowUp);
    rig.tap(Home);
    assert_eq!(rig.take(), tap_records(0x26, 0x48, 0, ENHANCED_KEY) + &tap_records(0x24, 0x47, 0, ENHANCED_KEY));
}

#[test]
fn keypad_navigation_with_numlock_off_has_no_prefix() {
    let mut rig = Rig::us();
    rig.tap(Numpad8);
    assert_eq!(rig.take(), tap_records(0x26, 0x48, 0, 0));
}

#[test]
fn keypad_digits_with_numlock_on_are_text() {
    let mut rig = Rig::us();
    rig.num_lock(true);
    rig.tap(Numpad5);
    assert_eq!(rig.take(), tap_records(0x65, 0x4C, '5' as u16, NUMLOCK_ON));
    assert!(!rig.log[0].handled);
}

#[test]
fn keypad_operators_are_text() {
    let mut rig = Rig::us();
    rig.tap(NumpadAdd);
    rig.tap(NumpadDivide);
    assert_eq!(rig.take(), tap_records(0x6B, 0x4E, '+' as u16, 0) + &tap_records(0x6F, 0x35, '/' as u16, ENHANCED_KEY));
}

#[test]
fn keypad_enter_is_extended() {
    let mut rig = Rig::us();
    rig.tap(NumpadEnter);
    assert_eq!(rig.take(), tap_records(0x0D, 0x1C, 13, ENHANCED_KEY));
}

#[test]
fn function_keys_up_to_f24() {
    let mut rig = Rig::us();
    rig.tap(F1);
    rig.tap(F13);
    rig.tap(F24);
    assert_eq!(
        rig.take(),
        tap_records(0x70, 0x3B, 0, 0) + &tap_records(0x7C, 0x64, 0, 0) + &tap_records(0x87, 0x76, 0, 0)
    );
}

#[test]
fn the_menu_key_is_encoded() {
    let mut rig = Rig::us();
    rig.tap(ContextMenu);
    assert_eq!(rig.take(), tap_records(0x5D, 0x5D, 0, ENHANCED_KEY));
}

#[test]
fn pause_and_print_screen() {
    let mut rig = Rig::us();
    rig.tap(Pause);
    rig.tap(PrintScreen);
    assert_eq!(rig.take(), tap_records(0x13, 0x45, 0, 0) + &tap_records(0x2C, 0x37, 0, ENHANCED_KEY));
}

#[test]
fn ctrl_pause_arrives_as_vk_cancel_and_stays_the_pause_key() {
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let out = asm.handle(KeyboardEvent::KeyDown(stroke(0x03, 0x46, true, Mods::CTRL)), &modes);
    assert_eq!(show(&out.bytes), record(0x03, 0x46, 0, true, LEFT_CTRL_PRESSED | ENHANCED_KEY));
}

#[test]
fn media_and_browser_keys_are_not_swallowed() {
    let mut rig = Rig::us();
    for key in [MediaPlayPause, AudioVolumeUp, BrowserBack] {
        rig.tap(key);
    }
    assert_eq!(rig.take(), "");
    assert!(rig.log.iter().all(|step| !step.handled && !step.pending));
}

// ---- chords -----------------------------------------------------------------------------------------

#[test]
fn shift_enter_reaches_psreadline_as_a_record_with_shift() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Shift], Enter);
    assert_eq!(
        rig.take(),
        record(0x10, 0x2A, 0, true, SHIFT_PRESSED)
            + &tap_records(0x0D, 0x1C, 13, SHIFT_PRESSED)
            + &record(0x10, 0x2A, 0, false, 0)
    );
}

#[test]
fn ctrl_space_is_a_record_with_ctrl() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Ctrl], Space);
    assert_eq!(
        rig.take(),
        record(0x11, 0x1D, 0, true, LEFT_CTRL_PRESSED)
            + &tap_records(0x20, 0x39, 0x20, LEFT_CTRL_PRESSED)
            + &record(0x11, 0x1D, 0, false, 0)
    );
    // A chord is encoded from the event: handled, so no character follows.
    assert!(rig.log.iter().all(|step| matches!(step.event, KeyboardEvent::KeyDown(_) | KeyboardEvent::KeyUp(_))));
}

#[test]
fn ctrl_c_carries_the_control_character() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Ctrl], KeyC);
    let text = rig.take();
    assert!(text.contains(&tap_records(0x43, 0x2E, 3, LEFT_CTRL_PRESSED)), "{text}");
}

#[test]
fn ctrl_v_is_a_plain_ctrl_chord_and_pasting_is_the_apps_decision() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Ctrl], KeyV);
    assert!(rig.take().contains(&tap_records(0x56, 0x2F, 0x16, LEFT_CTRL_PRESSED)));
}

#[test]
fn every_ctrl_letter_has_its_control_character() {
    for (index, key) in
        [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM].into_iter().enumerate()
    {
        let mut rig = Rig::us();
        rig.hold(Mod::Ctrl);
        rig.out.clear();
        rig.tap(key);
        let unit = index as u16 + 1;
        let (scan, _) = key.scan_code();
        assert!(rig.take().starts_with(&record(b'A' as u16 + index as u16, scan, unit, true, LEFT_CTRL_PRESSED)));
    }
}

#[test]
fn ctrl_enter_is_a_line_feed() {
    let mut rig = Rig::us();
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(Enter);
    assert_eq!(rig.take(), tap_records(0x0D, 0x1C, 10, LEFT_CTRL_PRESSED));
}

#[test]
fn ctrl_punctuation_follows_the_console_table() {
    let mut rig = Rig::us();
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(BracketLeft);
    rig.tap(Backslash);
    rig.tap(BracketRight);
    assert_eq!(
        rig.take(),
        tap_records(0xDB, 0x1A, 0x1B, LEFT_CTRL_PRESSED)
            + &tap_records(0xDC, 0x2B, 0x1C, LEFT_CTRL_PRESSED)
            + &tap_records(0xDD, 0x1B, 0x1D, LEFT_CTRL_PRESSED)
    );
}

#[test]
fn ctrl_shift_six_and_minus_are_rs_and_us() {
    let mut rig = Rig::us();
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Shift);
    rig.out.clear();
    rig.tap(Digit6);
    rig.tap(Minus);
    let cs = LEFT_CTRL_PRESSED | SHIFT_PRESSED;
    assert_eq!(rig.take(), tap_records(0x36, 0x07, 0x1E, cs) + &tap_records(0xBD, 0x0C, 0x1F, cs));
}

#[test]
fn alt_escape_is_a_record_with_alt() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Alt], Escape);
    assert_eq!(
        rig.take(),
        record(0x12, 0x38, 0, true, LEFT_ALT_PRESSED)
            + &tap_records(0x1B, 0x01, 27, LEFT_ALT_PRESSED)
            + &record(0x12, 0x38, 0, false, 0)
    );
}

#[test]
fn alt_letter_carries_the_letter_and_alt_shift_the_capital() {
    let mut rig = Rig::us();
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.tap(KeyB);
    assert_eq!(rig.take(), tap_records(0x42, 0x30, 'b' as u16, LEFT_ALT_PRESSED));
    rig.hold(Mod::Shift);
    rig.out.clear();
    rig.tap(KeyB);
    assert_eq!(rig.take(), tap_records(0x42, 0x30, 'B' as u16, LEFT_ALT_PRESSED | SHIFT_PRESSED));
}

#[test]
fn ctrl_alt_chord_without_a_character_is_the_chord() {
    // Ctrl and left Alt pressed separately: not AltGr. Ctrl+Alt+Q has no character on a US layout.
    let mut rig = Rig::us();
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.tap(KeyQ);
    let cs = LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED;
    assert_eq!(rig.take(), tap_records(0x51, 0x10, 0, cs));
    // Held pending, then resolved by the flush of the dispatch round.
    assert!(rig.log.iter().any(|step| step.pending && !step.handled));
}

#[test]
fn the_flush_resolves_a_waiting_chord_before_the_key_comes_up() {
    let mut rig = Rig::us();
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.key_down(KeyQ);
    assert_eq!(rig.take(), record(0x51, 0x10, 0, true, LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED));
}

#[test]
fn a_chord_that_needed_a_flush_waits_for_the_next_event_without_one() {
    let mut rig = Rig::us();
    rig.flush = false;
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.key_down(KeyQ);
    assert_eq!(rig.take(), "", "still pending");
    rig.key_up(KeyQ);
    let cs = LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED;
    assert_eq!(rig.take(), tap_records(0x51, 0x10, 0, cs));
}

#[test]
fn windows_key_chords_are_not_sent_as_the_bare_key() {
    let mut rig = Rig::us();
    rig.chord(&[Mod::Win], KeyA);
    // A console record has no Windows-key bit: only the modifier's own record goes out.
    assert_eq!(rig.take(), record(0x5B, 0x5B, 0, true, ENHANCED_KEY) + &record(0x5B, 0x5B, 0, false, ENHANCED_KEY));
}

// ---- modifier keys -----------------------------------------------------------------------------------

#[test]
fn modifier_keys_carry_their_side_and_prefix() {
    let mut rig = Rig::us();
    rig.hold(Mod::RightCtrl);
    rig.hold(Mod::RightShift);
    rig.release(Mod::RightShift);
    rig.release(Mod::RightCtrl);
    assert_eq!(
        rig.take(),
        record(0x11, 0x1D, 0, true, RIGHT_CTRL_PRESSED | ENHANCED_KEY)
            + &record(0x10, 0x36, 0, true, RIGHT_CTRL_PRESSED | SHIFT_PRESSED)
            + &record(0x10, 0x36, 0, false, RIGHT_CTRL_PRESSED)
            + &record(0x11, 0x1D, 0, false, ENHANCED_KEY)
    );
}

#[test]
fn modifier_auto_repeat_is_dropped() {
    let mut rig = Rig::us();
    rig.hold(Mod::Shift);
    rig.hold_repeat(Mod::Shift);
    rig.hold_repeat(Mod::Shift);
    rig.release(Mod::Shift);
    assert_eq!(rig.take(), record(0x10, 0x2A, 0, true, SHIFT_PRESSED) + &record(0x10, 0x2A, 0, false, 0));
}

#[test]
fn the_right_hand_ctrl_sets_the_right_hand_bit() {
    let mut rig = Rig::us();
    rig.hold(Mod::RightCtrl);
    rig.out.clear();
    rig.tap(KeyC);
    assert_eq!(rig.take(), tap_records(0x43, 0x2E, 3, RIGHT_CTRL_PRESSED));
}

#[test]
fn both_alts_set_both_bits() {
    let mut rig = Rig::us();
    rig.hold(Mod::Alt);
    rig.hold(Mod::RightAlt);
    rig.out.clear();
    rig.tap(Escape);
    assert_eq!(rig.take(), tap_records(0x1B, 0x01, 27, LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED));
}

#[test]
fn a_modifier_stays_held_while_its_other_side_is_down() {
    let mut rig = Rig::us();
    rig.hold(Mod::Shift);
    rig.hold(Mod::RightShift);
    rig.out.clear();
    rig.release(Mod::Shift);
    // The right Shift is still down: the release record shows it.
    assert_eq!(rig.take(), record(0x10, 0x2A, 0, false, SHIFT_PRESSED));
}

#[test]
fn the_fake_shift_around_keypad_navigation_is_ignored() {
    // NumLock on, Shift held, keypad 4: Windows releases Shift (E0 2A) and presses it again around
    // the navigation key.
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let fake_up = asm.handle(KeyboardEvent::KeyUp(stroke(0x10, 0x2A, true, Mods::NONE)), &modes);
    let fake_down = asm.handle(KeyboardEvent::KeyDown(stroke(0x10, 0x2A, true, Mods::SHIFT)), &modes);
    assert!(fake_up.bytes.is_empty() && fake_down.bytes.is_empty());
    assert!(!fake_up.handled && !fake_down.handled);
}

// ---- virtual keys and scan codes ---------------------------------------------------------------------

#[test]
fn side_specific_virtual_keys_are_reported_with_the_generic_code() {
    // GetKeyState-style codes (VK_LSHIFT, VK_RMENU) name the same keys as VK_SHIFT, VK_MENU.
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let down = asm.handle(KeyboardEvent::KeyDown(stroke(0xA1, 0, false, Mods::SHIFT)), &modes);
    assert_eq!(show(&down.bytes), record(0x10, 0x36, 0, true, SHIFT_PRESSED));
    let up = asm.handle(KeyboardEvent::KeyUp(stroke(0xA1, 0x36, false, Mods::NONE)), &modes);
    assert_eq!(show(&up.bytes), record(0x10, 0x36, 0, false, 0));
}

#[test]
fn a_repeat_of_a_key_the_assembler_never_saw_pressed_is_a_press() {
    // The key went down before the focus arrived.
    let mut asm = KeyAssembler::new();
    let modes = InputModes { kitty: KittyFlags::from_bits_truncate(1 | 2), ..InputModes::default() };
    let up = stroke(0x41, 0x1E, false, Mods::CTRL);
    let out = asm.handle(KeyboardEvent::KeyDown(KeyStroke { repeat: true, ..up }), &modes);
    assert_eq!(show(&out.bytes), r"\e[97;5u");
}

#[test]
fn the_stroke_names_its_physical_key() {
    assert_eq!(stroke(0x41, 0x10, false, Mods::NONE).physical_key(), Some(KeyQ));
    assert_eq!(stroke(0x25, 0x4B, true, Mods::NONE).physical_key(), Some(ArrowLeft));
    assert_eq!(stroke(0x25, 0, false, Mods::NONE).physical_key(), None);
}
