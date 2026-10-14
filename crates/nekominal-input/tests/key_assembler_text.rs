//! KeyAssembler sequence tests, part 3: layouts (US, US-International, German, French, Chinese
//! Traditional English mode, Russian), AltGr, dead keys, injected and astral text, IME keys.

mod common;
use PhysicalKey::*;
use common::{typist::*, *};
use nekominal_input::win32::control_state::*;
use nekominal_input::*;

fn both_routes(layout: fn() -> Layout, text: &str) {
    // Kitty "disambiguate" sends typing as text; the Win32 route carries it in the records.
    let mut kitty = Rig::new(layout()).with_kitty(1);
    kitty.type_text(text);
    assert_eq!(kitty.take(), text, "{} on the kitty route", kitty.layout.name);
    let mut win32 = Rig::new(layout());
    win32.type_text(text);
    assert_eq!(win32_typed(&win32.out), text, "{} on the Win32 route", win32.layout.name);
}

// ---- typing on layouts ---------------------------------------------------------------------------------

#[test]
fn us_types_every_printable_ascii_character() {
    let ascii: String = (0x20u8..=0x7E).map(char::from).collect();
    both_routes(Layout::us, &ascii);
}

#[test]
fn chinese_traditional_in_english_mode_types_like_us() {
    both_routes(Layout::zhuyin_english, "Hello, World! 123 @#$%^&*()_+{}|:\"<>?~");
}

#[test]
fn german_types_its_symbols_through_shift_and_altgr() {
    both_routes(Layout::german, "Größe: 100 € @home {a[b]c} ~ | \\ # < > ? ! \"§ $ % & / ( ) = + * - _ , ; . : '");
}

#[test]
fn german_swaps_y_and_z() {
    both_routes(Layout::german, "zyZY");
}

#[test]
fn french_types_digits_on_shift_and_symbols_on_altgr() {
    both_routes(
        Layout::french,
        "Bonjour à tous, ça va ? é è ù ~ # { [ | ` \\ ^ @ ] } € $ £ ¤ * µ ; : ! § ) ° = + & \" ' ( - _ 1234567890",
    );
}

#[test]
fn french_has_azerty_letters() {
    both_routes(Layout::french, "azerty qsdfgh wxcvbn AZERTY m,;:!");
}

#[test]
fn us_international_types_its_altgr_letters() {
    both_routes(Layout::us_international, "áéíóú ÁÉ ñ ß € plain");
}

#[test]
fn russian_types_cyrillic() {
    both_routes(Layout::russian, "фыцй сч мч ФЫ");
}

#[test]
fn every_layout_key_is_a_text_key_for_the_classifier() {
    for layout in [
        Layout::us(),
        Layout::us_international(),
        Layout::german(),
        Layout::french(),
        Layout::russian(),
        Layout::zhuyin_english(),
    ] {
        let mut scans = std::collections::HashSet::new();
        for key in &layout.keys {
            let (scan, ext) = key.key.scan_code();
            assert!(scans.insert(scan), "{}: scan {scan:#x} twice", layout.name);
            assert!(
                matches!(vk::classify(key.vk, scan, ext), vk::KeyClass::Text(_)),
                "{}: {} vk {:#x}",
                layout.name,
                key.key.name(),
                key.vk
            );
        }
    }
}

#[test]
fn ctrl_chords_follow_the_layout_letter_on_every_layout() {
    // The key labelled Z on a German board is where Y is on a US board; Ctrl+Z is still 0x1A.
    let mut rig = Rig::new(Layout::german());
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(KeyY);
    assert_eq!(rig.take(), tap_records(0x5A, 0x15, 0x1A, LEFT_CTRL_PRESSED));
    // AZERTY: the key labelled A is where Q is.
    let mut rig = Rig::new(Layout::french());
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(KeyQ);
    assert_eq!(rig.take(), tap_records(0x41, 0x10, 0x01, LEFT_CTRL_PRESSED));
    // Russian: Ctrl+С is the key C of the board, so it interrupts.
    let mut rig = Rig::new(Layout::russian());
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(KeyC);
    assert_eq!(rig.take(), tap_records(0x43, 0x2E, 0x03, LEFT_CTRL_PRESSED));
}

#[test]
fn a_ctrl_chord_on_a_punctuation_key_uses_the_position() {
    // German ü sits where [ is on a US board.
    let mut rig = Rig::new(Layout::german());
    rig.hold(Mod::Ctrl);
    rig.out.clear();
    rig.tap(BracketLeft);
    assert_eq!(rig.take(), tap_records(0xBA, 0x1A, 0x1B, LEFT_CTRL_PRESSED));
}

// ---- AltGr -----------------------------------------------------------------------------------------------------

#[test]
fn german_altgr_e_is_the_euro_sign_with_the_pair_windows_synthesizes() {
    let mut rig = Rig::new(Layout::german());
    rig.chord(&[Mod::RightAlt], KeyE);
    assert_eq!(
        rig.take(),
        // The fake left Ctrl, then the right Alt (a prefixed key), then the character with the pair.
        record(0x11, 0x1D, 0, true, LEFT_CTRL_PRESSED)
            + &record(0x12, 0x38, 0, true, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED | ENHANCED_KEY)
            + &tap_records(0x45, 0x12, 0x20AC, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED)
            + &record(0x12, 0x38, 0, false, LEFT_CTRL_PRESSED | ENHANCED_KEY)
            + &record(0x11, 0x1D, 0, false, 0)
    );
}

#[test]
fn an_altgr_key_is_not_handled_so_that_the_character_arrives() {
    let mut rig = Rig::new(Layout::german());
    rig.hold(Mod::RightAlt);
    rig.key_down(KeyE);
    let down = rig.log.iter().rev().find(|s| matches!(s.event, KeyboardEvent::KeyDown(k) if k.vk == 0x45)).unwrap();
    assert!(!down.handled && down.pending);
}

#[test]
fn altgr_symbols_on_german_and_french() {
    let mut rig = Rig::new(Layout::german()).with_kitty(1);
    rig.type_text("@{[]}\\~|€µ²³");
    assert_eq!(rig.take(), "@{[]}\\~|€µ²³");
    let mut rig = Rig::new(Layout::french()).with_kitty(1);
    rig.type_text("~#{[|`\\^@]}€¤");
    assert_eq!(rig.take(), "~#{[|`\\^@]}€¤");
}

#[test]
fn altgr_with_shift() {
    let mut rig = Rig::new(Layout::us_international()).with_kitty(1);
    rig.chord(&[Mod::Shift, Mod::RightAlt], KeyE);
    assert_eq!(rig.take(), "É");
}

#[test]
fn the_character_makes_altgr_even_when_the_fake_ctrl_came_too_early_to_count() {
    let mut rig = Rig::new(Layout::german());
    rig.hold_altgr_slowly(ALTGR_WINDOW_MS + 100);
    rig.out.clear();
    rig.tap(KeyE);
    // The key record carries the euro sign and the pair, as with the evidence.
    assert_eq!(rig.take(), tap_records(0x45, 0x12, 0x20AC, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED));
}

#[test]
fn the_pair_counts_up_to_the_window_and_not_beyond() {
    // A key with no character shows which side of the window the pair fell on: AltGr sends
    // nothing, a Ctrl+Alt chord is sent.
    let sent_for = |gap: u32| {
        let mut rig = Rig::new(Layout::german());
        rig.hold_altgr_slowly(gap);
        rig.out.clear();
        rig.tap(KeyX);
        rig.take()
    };
    assert_eq!(sent_for(ALTGR_WINDOW_MS), "");
    assert_ne!(sent_for(ALTGR_WINDOW_MS + 1), "");
}

#[test]
fn an_altgr_key_without_a_character_sends_nothing() {
    let mut rig = Rig::new(Layout::german());
    rig.chord(&[Mod::RightAlt], KeyX);
    assert_eq!(
        rig.take(),
        record(0x11, 0x1D, 0, true, LEFT_CTRL_PRESSED)
            + &record(0x12, 0x38, 0, true, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED | ENHANCED_KEY)
            + &record(0x12, 0x38, 0, false, LEFT_CTRL_PRESSED | ENHANCED_KEY)
            + &record(0x11, 0x1D, 0, false, 0)
    );
}

#[test]
fn ctrl_and_left_alt_on_a_layout_with_altgr_is_the_chord() {
    let mut rig = Rig::new(Layout::german());
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.tap(KeyE);
    assert_eq!(rig.take(), tap_records(0x45, 0x12, 0, LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED));
}

#[test]
fn ctrl_and_right_alt_pressed_together_on_a_layout_without_altgr_look_like_altgr() {
    // The known cost of the timing evidence: on a US layout Ctrl+right Alt+Q pressed within the
    // window is taken for AltGr, which types nothing there. Pressed apart, it is the chord.
    let chord = |gap: u32| {
        let mut rig = Rig::us();
        rig.hold(Mod::Ctrl);
        rig.advance(gap);
        rig.hold(Mod::RightAlt);
        rig.out.clear();
        rig.tap(KeyQ);
        rig.take()
    };
    assert_eq!(chord(0), "");
    assert_eq!(chord(ALTGR_WINDOW_MS + 1), tap_records(0x51, 0x10, 0, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED));
}

#[test]
fn releasing_altgr_ends_the_pair() {
    let mut rig = Rig::new(Layout::german());
    rig.chord(&[Mod::RightAlt], KeyE);
    rig.out.clear();
    // Plain E afterwards is plain e.
    rig.tap(KeyE);
    assert_eq!(rig.take(), tap_records(0x45, 0x12, 'e' as u16, 0));
}

// ---- dead keys ---------------------------------------------------------------------------------------------------

#[test]
fn a_dead_key_sends_nothing_by_itself() {
    let mut rig = Rig::new(Layout::french());
    rig.tap(BracketLeft);
    assert_eq!(rig.take(), "");
    // The press was left unhandled so that Windows can compose the next character.
    assert!(!rig.log[0].handled);
}

#[test]
fn french_circumflex_then_e_is_one_character_with_the_key_of_the_vowel() {
    let mut rig = Rig::new(Layout::french());
    rig.tap(BracketLeft);
    rig.tap(KeyE);
    assert_eq!(rig.take(), tap_records(0x45, 0x12, 'ê' as u16, 0));
}

#[test]
fn a_dead_key_resolves_without_the_flush_when_the_next_key_arrives() {
    let mut rig = Rig::new(Layout::french());
    rig.flush = false;
    rig.tap(BracketLeft);
    rig.tap(KeyE);
    assert_eq!(rig.take(), tap_records(0x45, 0x12, 'ê' as u16, 0));
}

#[test]
fn us_international_acute_then_vowel_and_then_space() {
    let mut rig = Rig::new(Layout::us_international()).with_kitty(1);
    rig.tap(Quote);
    rig.tap(KeyE);
    rig.tap(Quote);
    rig.tap(Space);
    assert_eq!(rig.take(), "é'");
}

#[test]
fn a_dead_key_that_does_not_combine_types_the_accent_and_the_key() {
    // Two characters arrive after the one key down: the first goes out with the key, the second is
    // text.
    let mut rig = Rig::new(Layout::us_international()).with_kitty(1);
    rig.tap(Quote);
    rig.tap(KeyX);
    assert_eq!(rig.take(), "'x");
    let mut rig = Rig::new(Layout::us_international());
    rig.tap(Quote);
    rig.tap(KeyX);
    assert_eq!(rig.take(), record(0x58, 0x2D, '\'' as u16, true, 0) + "x" + &record(0x58, 0x2D, '\'' as u16, false, 0));
}

#[test]
fn the_same_dead_key_twice_is_the_accent_itself() {
    let mut rig = Rig::new(Layout::french()).with_kitty(1);
    rig.tap(BracketLeft);
    rig.tap(BracketLeft);
    assert_eq!(rig.take(), "^");
}

#[test]
fn german_acute_and_grave() {
    let mut rig = Rig::new(Layout::german()).with_kitty(1);
    rig.tap(Equal);
    rig.tap(KeyA);
    rig.chord(&[Mod::Shift], Equal);
    rig.tap(KeyO);
    assert_eq!(rig.take(), "áò");
}

#[test]
fn french_diaeresis_on_shift_and_a_capital() {
    let mut rig = Rig::new(Layout::french()).with_kitty(1);
    rig.chord(&[Mod::Shift], BracketLeft);
    rig.chord(&[Mod::Shift], KeyE);
    assert_eq!(rig.take(), "Ë");
}

#[test]
fn a_word_with_dead_keys() {
    let mut rig = Rig::new(Layout::french()).with_kitty(1);
    rig.type_text("être àâ");
    assert_eq!(rig.take(), "être àâ");
}

#[test]
fn a_dead_key_press_has_no_release() {
    let mut rig = Rig::new(Layout::french()).with_kitty(1 | 2 | 8);
    rig.tap(BracketLeft);
    assert_eq!(rig.take(), "");
}

// ---- injected, astral and IME text -----------------------------------------------------------------------------------

#[test]
fn injected_unicode_is_utf8_on_every_route() {
    for bits in [0, 1, 1 | 2, 8, 31] {
        let mut rig = Rig::us().with_kitty(bits);
        rig.inject_text("你好，ㄋㄧˇ");
        assert_eq!(rig.take(), "你好，ㄋㄧˇ", "kitty flags {bits}");
    }
}

#[test]
fn an_astral_character_arrives_as_two_character_events() {
    let mut rig = Rig::us();
    rig.inject_text("😀");
    assert_eq!(rig.take(), "😀");
    // Packet keys are not handled: their characters must arrive.
    assert!(rig.log.iter().filter(|s| matches!(s.event, KeyboardEvent::KeyDown(_))).all(|s| !s.handled));
}

#[test]
fn text_with_emoji_keeps_its_order() {
    let mut rig = Rig::us().with_kitty(1);
    rig.tap(KeyA);
    rig.inject_text("😀👨‍👩‍👧");
    rig.tap(KeyB);
    assert_eq!(rig.take(), "a😀👨‍👩‍👧b");
}

#[test]
fn unpaired_surrogates_are_dropped() {
    let mut rig = Rig::us();
    rig.character(0xDE00);
    rig.character('a' as u16);
    assert_eq!(rig.take(), "a");
    rig.character(0xD83D);
    rig.character('b' as u16);
    assert_eq!(rig.take(), "b");
}

#[test]
fn a_pair_split_by_focus_loss_is_not_completed() {
    let mut rig = Rig::us();
    rig.character(0xD83D);
    rig.focus_lost();
    rig.character(0xDE00);
    assert_eq!(rig.take(), "");
}

#[test]
fn an_astral_character_on_a_key_goes_out_as_text_on_the_win32_route() {
    // A record carries one UTF-16 unit, so the character cannot ride on the key.
    let mut asm = KeyAssembler::new();
    let modes = InputModes::default();
    let key = KeyStroke::new(0x41, 0x1E, false, Mods::NONE);
    let mut out = Vec::new();
    out.extend(asm.handle(KeyboardEvent::KeyDown(key), &modes).bytes);
    out.extend(asm.handle(KeyboardEvent::CharacterReceived(0xD83D), &modes).bytes);
    out.extend(asm.handle(KeyboardEvent::CharacterReceived(0xDE00), &modes).bytes);
    out.extend(asm.handle(KeyboardEvent::KeyUp(key), &modes).bytes);
    assert_eq!(show(&out), "😀");
}

#[test]
fn the_ime_composing_key_is_left_alone() {
    let mut rig = Rig::us();
    let processkey = KeyStroke::new(vk::VK_PROCESSKEY, 0, false, Mods::NONE);
    let down = rig.feed(KeyboardEvent::KeyDown(processkey));
    let up = rig.feed(KeyboardEvent::KeyUp(processkey));
    assert!(down.bytes.is_empty() && !down.handled && !down.pending);
    assert!(up.bytes.is_empty() && !up.handled);
}

#[test]
fn a_release_the_ime_took_the_press_of_sends_nothing() {
    // With an IME composing only the KeyUp of the letter reaches the terminal.
    let mut rig = Rig::us().with_kitty(1 | 2 | 8);
    let up = KeyStroke::new(0x53, 0x1F, false, Mods::NONE);
    rig.feed(KeyboardEvent::KeyUp(up));
    assert_eq!(rig.take(), "");
}

// ---- Alt+numpad ----------------------------------------------------------------------------------------------------

#[test]
fn alt_numpad_digits_compose_a_character_and_are_not_keys() {
    let mut rig = Rig::us();
    rig.num_lock(true);
    rig.hold(Mod::Alt);
    rig.tap(Numpad2);
    rig.tap(Numpad3);
    rig.tap(Numpad3);
    rig.release(Mod::Alt);
    rig.character('é' as u16);
    assert_eq!(
        rig.take(),
        record(0x12, 0x38, 0, true, LEFT_ALT_PRESSED | NUMLOCK_ON) + &record(0x12, 0x38, 0, false, NUMLOCK_ON) + "é"
    );
    // The digit presses were left to Windows.
    assert!(
        rig.log
            .iter()
            .filter(|s| matches!(s.event, KeyboardEvent::KeyDown(k) if k.vk >= 0x60 && k.vk <= 0x69))
            .all(|s| !s.handled)
    );
}

// ---- release rules ------------------------------------------------------------------------------------------------------

#[test]
fn a_release_without_a_press_sends_nothing() {
    let mut rig = Rig::us();
    rig.key_up(Enter);
    rig.key_up(ArrowLeft);
    rig.key_up(KeyA);
    // A Shift pressed elsewhere and released here.
    rig.feed(KeyboardEvent::KeyUp(KeyStroke::new(0x10, 0x36, false, Mods::NONE)));
    assert_eq!(rig.take(), "");
}

#[test]
fn focus_loss_releases_held_keys_newest_first() {
    let mut rig = Rig::us();
    rig.hold(Mod::Shift);
    rig.hold(Mod::Ctrl);
    rig.key_down(ArrowLeft);
    rig.out.clear();
    rig.focus_lost();
    assert_eq!(
        rig.take(),
        record(0x25, 0x4B, 0, false, SHIFT_PRESSED | LEFT_CTRL_PRESSED | ENHANCED_KEY)
            + &record(0x11, 0x1D, 0, false, SHIFT_PRESSED)
            + &record(0x10, 0x2A, 0, false, 0)
    );
}

#[test]
fn focus_loss_forgets_altgr_and_the_modifier_sides() {
    let mut rig = Rig::new(Layout::german());
    rig.hold(Mod::RightAlt);
    rig.out.clear();
    rig.focus_lost();
    assert_eq!(
        rig.take(),
        record(0x12, 0x38, 0, false, LEFT_CTRL_PRESSED | ENHANCED_KEY) + &record(0x11, 0x1D, 0, false, 0)
    );
    // The pair is forgotten: Ctrl and left Alt pressed afterwards, on a key with no character,
    // are the chord and not a leftover AltGr that would send nothing.
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.tap(KeyX);
    assert_eq!(rig.take(), tap_records(0x58, 0x2D, 0, LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED));
}

#[test]
fn focus_loss_under_disambiguate_has_nothing_to_release() {
    let mut rig = Rig::us().with_kitty(1);
    rig.hold(Mod::Shift);
    rig.key_down(Enter);
    rig.out.clear();
    rig.focus_lost();
    assert_eq!(rig.take(), "");
}

#[test]
fn focus_loss_under_events_reports_the_releases() {
    let mut rig = Rig::us().with_kitty(8 | 2);
    rig.hold(Mod::Shift);
    rig.key_down(KeyA);
    rig.out.clear();
    rig.focus_lost();
    assert_eq!(rig.take(), r"\e[97;2:3u\e[57441;1:3u");
}

#[test]
fn focus_loss_resolves_a_waiting_key() {
    let mut rig = Rig::us();
    rig.flush = false;
    rig.hold(Mod::Ctrl);
    rig.hold(Mod::Alt);
    rig.out.clear();
    rig.key_down(KeyQ);
    assert_eq!(rig.take(), "");
    rig.focus_lost();
    let cs = LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED;
    // The key goes down, then every held key is released, the key first.
    assert!(rig.take().starts_with(&record(0x51, 0x10, 0, true, cs)));
}

#[test]
fn a_held_lock_key_does_not_toggle_again_on_repeat() {
    let mut rig = Rig::us();
    rig.key_down(CapsLock);
    rig.repeat(CapsLock, 3);
    rig.key_up(CapsLock);
    assert_eq!(rig.take(), record(0x14, 0x3A, 0, true, CAPSLOCK_ON) + &record(0x14, 0x3A, 0, false, CAPSLOCK_ON));
}

#[test]
fn num_lock_changes_what_the_keypad_is() {
    let mut rig = Rig::us();
    rig.tap(NumLock);
    assert_eq!(rig.take(), tap_records(0x90, 0x45, 0, NUMLOCK_ON | ENHANCED_KEY));
    rig.tap(Numpad5);
    assert_eq!(rig.take(), tap_records(0x65, 0x4C, '5' as u16, NUMLOCK_ON));
}

// ---- robustness ---------------------------------------------------------------------------------------------------------------

#[test]
fn arbitrary_event_streams_never_panic() {
    // A deterministic xorshift: hostile virtual keys, scan codes, characters and flag combinations
    // in any order, under every route.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let interesting = [
        0, 3, 8, 9, 13, 16, 17, 18, 20, 27, 32, 65, 90, 0x5B, 0x5D, 0x60, 0x6F, 0x70, 0x87, 0xA0, 0xA5, 0xBA, 0xE2,
        0xE5, 0xFF, -1, 300, 70000,
    ];
    for flags in [0u8, 1, 2, 3, 8, 10, 24, 31] {
        let modes = InputModes { kitty: KittyFlags::from_bits_truncate(flags), ..InputModes::default() };
        let mut asm = KeyAssembler::new();
        for _ in 0..20_000 {
            let r = next();
            let vk = if r & 1 == 0 {
                interesting[(r >> 8) as usize % interesting.len()]
            } else {
                (r >> 16) as i32 % 400 - 20
            };
            let stroke = KeyStroke {
                vk,
                scan_code: if r & 2 == 0 { (r >> 24) as u16 } else { (r >> 24) as u16 & 0x7f },
                extended: r & 4 != 0,
                repeat: r & 8 != 0,
                repeat_count: (r >> 32) as u16,
                mods: Mods::from_bits_truncate((r >> 40) as u8),
                time_ms: (r >> 48) as u32,
            };
            let event = match (r >> 4) % 8 {
                0..=2 => KeyboardEvent::KeyDown(stroke),
                3 | 4 => KeyboardEvent::KeyUp(stroke),
                5 | 6 => KeyboardEvent::CharacterReceived(if r & 0x100 != 0 {
                    (r >> 20) as u16
                } else {
                    0xD800 + ((r >> 20) as u16 & 0x7ff)
                }),
                _ if r & 0x200 != 0 => KeyboardEvent::Flush,
                _ => KeyboardEvent::FocusLost,
            };
            let out = asm.handle(event, &modes);
            assert!(out.bytes.len() < 4096);
        }
    }
}
