//! The shortcut key names of the model are the names the input crate gives to scan codes, so a key event
//! converted by `nekominal_input::scancode::physical_key` always finds the shortcut written for it.

use nekominal_input::scancode::PhysicalKey;
use nekominal_model::keymap::{Key, Modifiers};

#[test]
fn every_shortcut_key_is_a_physical_key_of_the_input_crate() {
    for key in Key::all() {
        let physical = PhysicalKey::from_name(key.code())
            .unwrap_or_else(|| panic!("{} is not a PhysicalKey name", key.code()));
        assert_eq!(physical.name(), key.code());
    }
}

#[test]
fn shortcut_labels_of_text_keys_are_what_the_key_types_on_a_us_board() {
    for key in Key::all().filter(|key| key.types_text()) {
        let physical = PhysicalKey::from_name(key.code()).unwrap();
        assert!(physical.types_text(), "{} should type text", key.code());
        if key.code() != "Space" {
            let typed = physical.us_char().unwrap().to_string();
            assert_eq!(key.label().to_ascii_lowercase(), typed, "{}", key.code());
        }
    }
}

#[test]
fn keys_that_type_nothing_are_not_text_keys_for_the_input_crate_either() {
    for key in Key::all().filter(|key| !key.types_text()) {
        let physical = PhysicalKey::from_name(key.code()).unwrap();
        assert!(!physical.types_text(), "{}", key.code());
    }
}

#[test]
fn a_scan_code_resolves_to_a_shortcut_through_the_keymap() {
    use nekominal_model::{
        commands::{Command, default_bindings},
        keymap::build_keymap,
    };
    let keymap = build_keymap(default_bindings(), &Default::default()).keymap;
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };

    // WinUI reports the T key as scan code 0x14, whatever the layout says it types.
    let key = nekominal_input::scancode::physical_key(0x14, false).expect("scan code 0x14");
    assert_eq!(keymap.find(ctrl_shift, key.name()), Some(Command::TabNew));
    // Left arrow is the extended 0x4B; the keypad 4 shares the code without the prefix and is no shortcut.
    let left = nekominal_input::scancode::physical_key(0x4B, true).unwrap();
    assert_eq!(
        keymap.find(
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            left.name()
        ),
        Some(Command::PaneFocus(nekominal_model::layout::Direction::Left))
    );
    let keypad4 = nekominal_input::scancode::physical_key(0x4B, false).unwrap();
    assert_eq!(
        keymap.find(
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            keypad4.name()
        ),
        None
    );
}
