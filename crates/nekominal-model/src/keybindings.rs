//! Editing the shortcuts: what a chord that is being recorded runs into, and how a binding is written into the
//! `keybindings` of the settings.
//!
//! The settings hold only what differs from `Command::default_shortcuts` (the file is written as a difference from the
//! defaults), so every edit here ends by dropping an entry that says what the defaults already say. An empty list
//! unbinds a command, which is a difference too.

use std::collections::BTreeMap;

use crate::{
    commands::Command,
    keymap::{Key, KeyClass, Keymap, Shortcut},
};

/// What a chord is used for already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// A command of the application has it.
    Command(Command),
    /// A program in the terminal needs it: a control character (Ctrl+C interrupts, Ctrl+D ends the input), a Meta key
    /// of readline and emacs (Alt+B), AltGr, or typing itself. Taking it away from the programs has to be asked for.
    Program,
}

/// Whether a program in the terminal needs the chord, or it is typing. The Windows key chords are not asked about:
/// Windows takes them before the application does.
pub fn needed_by_programs(chord: Shortcut) -> bool {
    let Shortcut { mods, key } = chord;
    if mods.meta {
        return false;
    }
    match (mods.ctrl, mods.alt, mods.shift) {
        // Nothing held: typing, and the keys of the editing of a line. The function keys are the application's to give.
        (false, false, false) => key.class() != KeyClass::Function,
        // Ctrl alone: the control characters, which are the letters and four of the punctuation keys and Space.
        (true, false, false) => {
            key.class() == KeyClass::Letter
                || key.class() == KeyClass::Space
                || ["BracketLeft", "BracketRight", "Backslash"].contains(&key.code())
        }
        // Alt alone: the Meta keys of readline and emacs.
        (false, true, false) => matches!(key.class(), KeyClass::Letter | KeyClass::Digit),
        // Ctrl and Alt on a character: AltGr on many layouts.
        (true, true, false) => key.types_text(),
        // Shift alone on a character: the capital letter or the symbol.
        (false, false, true) => key.types_text(),
        _ => false,
    }
}

/// What giving `chord` to `command` would run into: another command that has it, else a program that needs it.
pub fn clash(keymap: &Keymap, chord: Shortcut, command: Command) -> Option<Clash> {
    match keymap.find(chord.mods, chord.key.code()) {
        Some(owner) if owner != command => Some(Clash::Command(owner)),
        Some(_) => None,
        None => needed_by_programs(chord).then_some(Clash::Program),
    }
}

/// The shortcuts of `command` as the keymap has them.
pub fn current(keymap: &Keymap, command: Command) -> Vec<Shortcut> {
    keymap.shortcuts(command).to_vec()
}

/// Makes `shortcuts` the bindings of `command`: an entry in `overrides`, unless they are the defaults, which need none.
pub fn set(overrides: &mut BTreeMap<String, Vec<String>>, command: Command, shortcuts: &[Shortcut]) {
    let written: Vec<String> = shortcuts.iter().map(ToString::to_string).collect();
    let defaults: Vec<String> = command.default_shortcuts().iter().map(|text| (*text).to_owned()).collect();
    if written == defaults {
        overrides.remove(command.id());
    } else {
        overrides.insert(command.id().to_owned(), written);
    }
}

/// Gives `chord` to `command`: in the place of `replacing` when it is one of its bindings, else as one more. A command
/// that had the chord loses it. The keymap is the one in effect before the change.
pub fn assign(
    overrides: &mut BTreeMap<String, Vec<String>>,
    keymap: &Keymap,
    command: Command,
    replacing: Option<Shortcut>,
    chord: Shortcut,
) {
    if let Some(Clash::Command(owner)) = clash(keymap, chord, command) {
        let kept: Vec<Shortcut> = current(keymap, owner).into_iter().filter(|held| *held != chord).collect();
        set(overrides, owner, &kept);
    }
    let mut shortcuts = current(keymap, command);
    match replacing.and_then(|old| shortcuts.iter().position(|held| *held == old)) {
        Some(at) => shortcuts[at] = chord,
        None => shortcuts.push(chord),
    }
    // Replacing a binding with one the command has already would list it twice.
    let mut seen = Vec::new();
    shortcuts.retain(|held| {
        let first = !seen.contains(held);
        seen.push(*held);
        first
    });
    set(overrides, command, &shortcuts);
}

/// Takes one binding away from `command`; the last one leaves it unbound.
pub fn remove(overrides: &mut BTreeMap<String, Vec<String>>, keymap: &Keymap, command: Command, chord: Shortcut) {
    let kept: Vec<Shortcut> = current(keymap, command).into_iter().filter(|held| *held != chord).collect();
    set(overrides, command, &kept);
}

/// Puts the defaults back for one command.
pub fn reset(overrides: &mut BTreeMap<String, Vec<String>>, command: Command) {
    overrides.remove(command.id());
}

/// Whether `command` has bindings other than its defaults.
pub fn is_modified(overrides: &BTreeMap<String, Vec<String>>, command: Command) -> bool {
    overrides.contains_key(command.id())
}

/// Key position of a W3C code, for a recorder that reads one from a key event.
pub fn chord_of(code: &str, mods: crate::keymap::Modifiers) -> Option<Shortcut> {
    Key::from_code(code).map(|key| Shortcut { mods, key })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        commands::default_bindings,
        keymap::build_keymap,
        settings::{Settings, parse, serialize},
        strings::Lang,
    };

    fn chord(text: &str) -> Shortcut {
        Shortcut::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    fn keymap(overrides: &BTreeMap<String, Vec<String>>) -> Keymap {
        build_keymap(default_bindings(), overrides).keymap
    }

    fn assigned(
        overrides: &mut BTreeMap<String, Vec<String>>,
        command: Command,
        replacing: Option<Shortcut>,
        chord: Shortcut,
    ) {
        let in_effect = keymap(overrides);
        assign(overrides, &in_effect, command, replacing, chord);
    }

    fn removed(overrides: &mut BTreeMap<String, Vec<String>>, command: Command, chord: Shortcut) {
        let in_effect = keymap(overrides);
        remove(overrides, &in_effect, command, chord);
    }

    fn texts(keymap: &Keymap, command: Command) -> Vec<String> {
        current(keymap, command).iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_chord_of_another_command_is_a_clash_and_the_commands_own_chord_is_not() {
        let keymap = keymap(&BTreeMap::new());
        assert_eq!(clash(&keymap, chord("Ctrl+Shift+T"), Command::PaneClose), Some(Clash::Command(Command::TabNew)));
        assert_eq!(clash(&keymap, chord("Ctrl+Shift+T"), Command::TabNew), None);
        assert_eq!(clash(&keymap, chord("Ctrl+Shift+Q"), Command::PaneClose), None, "nobody has it");
        assert_eq!(
            clash(&keymap, chord("Alt+Shift+="), Command::PaneSplitDown),
            Some(Clash::Command(Command::PaneSplitRight))
        );
    }

    #[test]
    fn ctrl_c_is_the_smart_copy_and_must_be_replaced_on_purpose() {
        let keymap = keymap(&BTreeMap::new());
        assert_eq!(clash(&keymap, chord("Ctrl+C"), Command::TabClose), Some(Clash::Command(Command::EditSmartCopy)));
        assert_eq!(Command::EditSmartCopy.label(Lang::ZhTw), "複製", "the name the conflict shows");
    }

    #[test]
    fn what_programs_need_is_a_clash_without_a_command() {
        let keymap = keymap(&BTreeMap::new());
        for text in [
            "Ctrl+D",
            "Ctrl+Z",
            "Ctrl+L",
            "Ctrl+[",
            "Ctrl+Space",
            "Alt+B",
            "Alt+5",
            "Ctrl+Alt+A",
            "Shift+A",
            "A",
            "Enter",
            "Escape",
            "Left",
        ] {
            assert_eq!(clash(&keymap, chord(text), Command::TabClose), Some(Clash::Program), "{text}");
        }
        for text in [
            "Ctrl+Shift+D",
            "Alt+Shift+B",
            "Alt+Home",
            "Ctrl+Alt+Left",
            "Ctrl+1",
            "Ctrl+.",
            "Ctrl+Delete",
            "Shift+Home",
            "F5",
            "F12",
            "Ctrl+Shift+Alt+Q",
        ] {
            assert_eq!(clash(&keymap, chord(text), Command::TabClose), None, "{text}");
        }
        // The Windows key is not ours to ask about.
        assert_eq!(clash(&keymap, chord("Win+L"), Command::TabClose), None);
    }

    #[test]
    fn the_defaults_the_application_ships_do_not_clash_with_what_they_are_defaults_of() {
        let keymap = keymap(&BTreeMap::new());
        for command in Command::ALL {
            for shortcut in command.default_shortcuts() {
                let found = clash(&keymap, chord(shortcut), *command);
                assert!(!matches!(found, Some(Clash::Command(_))), "{} {shortcut}: {found:?}", command.id());
            }
        }
    }

    #[test]
    fn a_new_binding_is_added_and_a_replaced_one_changes_place_in_the_list() {
        let mut overrides = BTreeMap::new();
        assigned(&mut overrides, Command::PaneClose, None, chord("Ctrl+Shift+Q"));
        assert_eq!(overrides["pane.close"], ["Ctrl+Shift+W", "Ctrl+Shift+Q"]);
        assigned(&mut overrides, Command::PaneClose, Some(chord("Ctrl+Shift+W")), chord("Ctrl+Shift+X"));
        assert_eq!(overrides["pane.close"], ["Ctrl+Shift+X", "Ctrl+Shift+Q"]);
        // The same chord twice is one binding.
        assigned(&mut overrides, Command::PaneClose, Some(chord("Ctrl+Shift+X")), chord("Ctrl+Shift+Q"));
        assert_eq!(overrides["pane.close"], ["Ctrl+Shift+Q"]);
    }

    #[test]
    fn replacing_a_chord_takes_it_from_the_command_that_had_it() {
        let mut overrides = BTreeMap::new();
        assigned(&mut overrides, Command::PaneClose, None, chord("Ctrl+Shift+T"));
        let after = keymap(&overrides);
        assert_eq!(after.find(chord("Ctrl+Shift+T").mods, "KeyT"), Some(Command::PaneClose));
        assert_eq!(texts(&after, Command::TabNew), Vec::<String>::new(), "the new tab command is left unbound");
        assert_eq!(overrides["tab.new"], Vec::<String>::new(), "and says so in the file");
        // A command with a second binding keeps it.
        let mut overrides = BTreeMap::new();
        assigned(&mut overrides, Command::TabClose, None, chord("Ctrl+V"));
        assert_eq!(overrides["edit.paste"], ["Ctrl+Shift+V", "Shift+Insert"]);
    }

    #[test]
    fn removing_the_last_binding_unbinds_and_giving_back_the_defaults_leaves_no_trace() {
        let mut overrides = BTreeMap::new();
        removed(&mut overrides, Command::AppPalette, chord("Ctrl+Shift+P"));
        assert_eq!(overrides["app.palette"], Vec::<String>::new());
        assert!(is_modified(&overrides, Command::AppPalette));
        // Binding the default again is not a difference.
        assigned(&mut overrides, Command::AppPalette, None, chord("Ctrl+Shift+P"));
        assert!(overrides.is_empty(), "{overrides:?}");
        // Dropping one of two defaults and adding it back is not a difference either.
        removed(&mut overrides, Command::PaneSplitRight, chord("Alt+Shift+="));
        assert_eq!(overrides["pane.splitRight"], ["Alt+Shift+D"]);
        assigned(&mut overrides, Command::PaneSplitRight, None, chord("Alt+Shift+="));
        assert!(overrides.is_empty(), "{overrides:?}");
    }

    #[test]
    fn reset_puts_the_defaults_back_for_one_command() {
        let mut overrides = BTreeMap::new();
        assigned(&mut overrides, Command::PaneClose, None, chord("Ctrl+Shift+T"));
        assert_eq!(overrides.len(), 2, "pane.close and the unbound tab.new");
        reset(&mut overrides, Command::PaneClose);
        assert!(!is_modified(&overrides, Command::PaneClose));
        assert_eq!(texts(&keymap(&overrides), Command::PaneClose), ["Ctrl+Shift+W"]);
        // tab.new is a difference of its own until it is reset too.
        assert!(is_modified(&overrides, Command::TabNew));
        reset(&mut overrides, Command::TabNew);
        assert!(overrides.is_empty());
    }

    #[test]
    fn what_is_edited_is_written_as_a_difference_and_read_back() {
        let mut settings = Settings::default();
        assigned(&mut settings.keybindings, Command::TabClose, None, chord("Ctrl+Shift+Q"));
        let written = serialize(&settings);
        let value: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value, serde_json::json!({ "keybindings": { "tab.close": ["Ctrl+Shift+Q"] } }));
        let back = parse(Some(&written));
        assert!(back.problems.is_empty());
        assert_eq!(back.settings, settings);
        assert_eq!(texts(&keymap(&back.settings.keybindings), Command::TabClose), ["Ctrl+Shift+Q"]);
        // Everything reset: nothing is written.
        reset(&mut settings.keybindings, Command::TabClose);
        assert_eq!(serialize(&settings), "{}\n");
    }

    #[test]
    fn a_chord_is_read_from_a_key_code_and_the_modifiers_held() {
        let ctrl = crate::keymap::Modifiers { ctrl: true, ..Default::default() };
        assert_eq!(chord_of("KeyT", ctrl), Some(chord("Ctrl+T")));
        assert_eq!(chord_of("ControlLeft", ctrl), None, "a modifier key alone is no chord");
    }
}
