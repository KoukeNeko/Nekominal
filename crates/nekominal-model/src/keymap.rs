//! Keyboard shortcuts: parsing, display and matching by physical key.
//!
//! A key is named by its W3C UI Events `code` (`KeyT`, `Digit1`, `Minus`, `ArrowLeft`, `F11`), the same
//! names `nekominal_input::scancode::PhysicalKey::name` yields for a scan code. Matching the key position
//! instead of the character keeps a shortcut working on every layout and while an IME is composing; the
//! table of names below is the contract between the two crates (`tests/key_names.rs` enforces it).

use std::{collections::BTreeMap, fmt};

use crate::{
    commands::Command,
    settings::{Expected, Problem},
    strings::Lang,
};

/// What a key types, which decides whether a bare press of it can be a shortcut at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    Letter,
    Digit,
    Punctuation,
    Space,
    /// Enter, Tab, arrows, Insert, Home, ...: types no character.
    Named,
    Function,
}

/// (W3C code, name shown in menus, class).
const KEYS: &[(&str, &str, KeyClass)] = &[
    ("KeyA", "A", KeyClass::Letter),
    ("KeyB", "B", KeyClass::Letter),
    ("KeyC", "C", KeyClass::Letter),
    ("KeyD", "D", KeyClass::Letter),
    ("KeyE", "E", KeyClass::Letter),
    ("KeyF", "F", KeyClass::Letter),
    ("KeyG", "G", KeyClass::Letter),
    ("KeyH", "H", KeyClass::Letter),
    ("KeyI", "I", KeyClass::Letter),
    ("KeyJ", "J", KeyClass::Letter),
    ("KeyK", "K", KeyClass::Letter),
    ("KeyL", "L", KeyClass::Letter),
    ("KeyM", "M", KeyClass::Letter),
    ("KeyN", "N", KeyClass::Letter),
    ("KeyO", "O", KeyClass::Letter),
    ("KeyP", "P", KeyClass::Letter),
    ("KeyQ", "Q", KeyClass::Letter),
    ("KeyR", "R", KeyClass::Letter),
    ("KeyS", "S", KeyClass::Letter),
    ("KeyT", "T", KeyClass::Letter),
    ("KeyU", "U", KeyClass::Letter),
    ("KeyV", "V", KeyClass::Letter),
    ("KeyW", "W", KeyClass::Letter),
    ("KeyX", "X", KeyClass::Letter),
    ("KeyY", "Y", KeyClass::Letter),
    ("KeyZ", "Z", KeyClass::Letter),
    ("Digit0", "0", KeyClass::Digit),
    ("Digit1", "1", KeyClass::Digit),
    ("Digit2", "2", KeyClass::Digit),
    ("Digit3", "3", KeyClass::Digit),
    ("Digit4", "4", KeyClass::Digit),
    ("Digit5", "5", KeyClass::Digit),
    ("Digit6", "6", KeyClass::Digit),
    ("Digit7", "7", KeyClass::Digit),
    ("Digit8", "8", KeyClass::Digit),
    ("Digit9", "9", KeyClass::Digit),
    ("Minus", "-", KeyClass::Punctuation),
    ("Equal", "=", KeyClass::Punctuation),
    ("BracketLeft", "[", KeyClass::Punctuation),
    ("BracketRight", "]", KeyClass::Punctuation),
    ("Backslash", "\\", KeyClass::Punctuation),
    ("Semicolon", ";", KeyClass::Punctuation),
    ("Quote", "'", KeyClass::Punctuation),
    ("Backquote", "`", KeyClass::Punctuation),
    ("Comma", ",", KeyClass::Punctuation),
    ("Period", ".", KeyClass::Punctuation),
    ("Slash", "/", KeyClass::Punctuation),
    ("Space", "Space", KeyClass::Space),
    ("Enter", "Enter", KeyClass::Named),
    ("Tab", "Tab", KeyClass::Named),
    ("Escape", "Escape", KeyClass::Named),
    ("Backspace", "Backspace", KeyClass::Named),
    ("Delete", "Delete", KeyClass::Named),
    ("Insert", "Insert", KeyClass::Named),
    ("Home", "Home", KeyClass::Named),
    ("End", "End", KeyClass::Named),
    ("PageUp", "PageUp", KeyClass::Named),
    ("PageDown", "PageDown", KeyClass::Named),
    ("ArrowUp", "Up", KeyClass::Named),
    ("ArrowDown", "Down", KeyClass::Named),
    ("ArrowLeft", "Left", KeyClass::Named),
    ("ArrowRight", "Right", KeyClass::Named),
    ("F1", "F1", KeyClass::Function),
    ("F2", "F2", KeyClass::Function),
    ("F3", "F3", KeyClass::Function),
    ("F4", "F4", KeyClass::Function),
    ("F5", "F5", KeyClass::Function),
    ("F6", "F6", KeyClass::Function),
    ("F7", "F7", KeyClass::Function),
    ("F8", "F8", KeyClass::Function),
    ("F9", "F9", KeyClass::Function),
    ("F10", "F10", KeyClass::Function),
    ("F11", "F11", KeyClass::Function),
    ("F12", "F12", KeyClass::Function),
    ("F13", "F13", KeyClass::Function),
    ("F14", "F14", KeyClass::Function),
    ("F15", "F15", KeyClass::Function),
    ("F16", "F16", KeyClass::Function),
    ("F17", "F17", KeyClass::Function),
    ("F18", "F18", KeyClass::Function),
    ("F19", "F19", KeyClass::Function),
    ("F20", "F20", KeyClass::Function),
    ("F21", "F21", KeyClass::Function),
    ("F22", "F22", KeyClass::Function),
    ("F23", "F23", KeyClass::Function),
    ("F24", "F24", KeyClass::Function),
];

/// Spellings accepted in `settings.json` besides the displayed name and the W3C code.
const ALIASES: &[(&str, &str)] = &[
    ("esc", "Escape"),
    ("return", "Enter"),
    ("del", "Delete"),
    ("ins", "Insert"),
    ("pgup", "PageUp"),
    ("pgdn", "PageDown"),
    // "Ctrl++" names the plus key, which shares its position with `=`.
    ("plus", "Equal"),
];

/// A key by position on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(&'static str);

impl Key {
    /// The key with this W3C `code`; `None` for modifier keys and anything shortcuts cannot use.
    pub fn from_code(code: &str) -> Option<Key> {
        KEYS.iter()
            .find(|(known, ..)| *known == code)
            .map(|(known, ..)| Key(known))
    }

    /// Every key a shortcut can name, in table order.
    pub fn all() -> impl Iterator<Item = Key> {
        KEYS.iter().map(|(code, ..)| Key(code))
    }

    /// The W3C `code`.
    pub fn code(self) -> &'static str {
        self.0
    }

    /// The name menus and tooltips show (`T`, `1`, `-`, `Up`, `Space`).
    pub fn label(self) -> &'static str {
        self.entry().1
    }

    pub fn class(self) -> KeyClass {
        self.entry().2
    }

    /// The key types a character, so pressing it without Ctrl, Alt or Win is typing, not a shortcut.
    pub fn types_text(self) -> bool {
        matches!(
            self.class(),
            KeyClass::Letter | KeyClass::Digit | KeyClass::Punctuation | KeyClass::Space
        )
    }

    /// An active IME takes punctuation and Space chords (full-width comma, mode switches) before the
    /// application sees them, so a command that matters must have another binding.
    pub fn is_swallowed_by_ime(self) -> bool {
        matches!(self.class(), KeyClass::Punctuation | KeyClass::Space)
    }

    fn entry(self) -> &'static (&'static str, &'static str, KeyClass) {
        KEYS.iter()
            .find(|(code, ..)| *code == self.0)
            .expect("a Key always holds a table code")
    }

    fn parse(name: &str) -> Option<Key> {
        let lower = name.to_ascii_lowercase();
        if let Some((_, code)) = ALIASES.iter().find(|(alias, _)| *alias == lower) {
            return Key::from_code(code);
        }
        KEYS.iter()
            .find(|(code, label, _)| {
                code.eq_ignore_ascii_case(name) || label.eq_ignore_ascii_case(name)
            })
            .map(|(code, ..)| Key(code))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key.
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub mods: Modifiers,
    pub key: Key,
}

impl Shortcut {
    /// Parses `Ctrl+Shift+T`: modifiers in any order and case, one key by displayed name, alias or W3C code.
    pub fn parse(text: &str) -> Option<Shortcut> {
        let mut parts: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        // "Ctrl++" splits into an empty trailing part; treat the literal plus as the key.
        if text.trim().ends_with("++") {
            parts.push("plus");
        }
        let mut mods = Modifiers::default();
        let mut key = None;
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" | "option" => mods.alt = true,
                "win" | "meta" | "cmd" => mods.meta = true,
                _ if key.is_some() => return None,
                _ => key = Some(Key::parse(part)?),
            }
        }
        Some(Shortcut { mods, key: key? })
    }

    /// The modifier set must be exactly this one: Ctrl+Shift+C is not Ctrl+C.
    pub fn matches(&self, mods: Modifiers, key: Key) -> bool {
        self.mods == mods && self.key == key
    }
}

/// `Ctrl+Alt+Shift+Win+Key`, the order Windows documents shortcuts in, and the form `parse` reads back.
impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.mods.ctrl, "Ctrl"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
            (self.mods.meta, "Win"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(self.key.label())
    }
}

/// The shortcuts of every bound command. Earlier entries win when two shortcuts collide, so a binding the
/// user wrote beats a default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<(Command, Vec<Shortcut>)>,
}

#[derive(Debug)]
pub struct BuiltKeymap {
    pub keymap: Keymap,
    /// What is wrong with the user's bindings; those entries are ignored.
    pub problems: Vec<Problem>,
}

/// Combines the default bindings with the `keybindings` of the settings. A command present in `overrides`
/// takes only the override: an empty list unbinds it, and an unparsable shortcut is dropped.
pub fn build_keymap<'a>(
    defaults: impl IntoIterator<Item = (Command, &'a [&'a str])>,
    overrides: &BTreeMap<String, Vec<String>>,
) -> BuiltKeymap {
    let mut bindings: Vec<(Command, Vec<Shortcut>)> = Vec::new();
    let mut overridden: Vec<Command> = Vec::new();
    let mut problems = Vec::new();

    for (id, texts) in overrides {
        let Some(command) = Command::from_id(id) else {
            problems.push(Problem::new(format!("keybindings.{id}"), Expected::KnownCommand));
            continue;
        };
        let mut shortcuts = Vec::new();
        for text in texts {
            let Some(shortcut) = Shortcut::parse(text) else {
                problems.push(Problem::new(format!("keybindings.{id}"), Expected::Shortcut(text.clone())));
                continue;
            };
            // Two of the user's bindings with one chord: the first (by command id) keeps it, and the clash is told.
            match bindings.iter().find(|(_, taken)| taken.contains(&shortcut)) {
                Some((owner, _)) => problems.push(Problem::new(format!("keybindings.{id}"), Expected::Clash(*owner))),
                None => shortcuts.push(shortcut),
            }
        }
        overridden.push(command);
        if !shortcuts.is_empty() {
            bindings.push((command, shortcuts));
        }
    }

    let claimed = bindings.len();
    for (command, texts) in defaults {
        if overridden.contains(&command) {
            continue;
        }
        // A chord the user gave another command is that command's alone: menus and tooltips must not list it here.
        let shortcuts: Vec<Shortcut> = texts
            .iter()
            .filter_map(|text| Shortcut::parse(text))
            .filter(|shortcut| !bindings[..claimed].iter().any(|(_, taken)| taken.contains(shortcut)))
            .collect();
        if !shortcuts.is_empty() {
            bindings.push((command, shortcuts));
        }
    }
    BuiltKeymap {
        keymap: Keymap { bindings },
        problems,
    }
}

impl Keymap {
    /// The command bound to this exact chord. `code` is the W3C name of the physical key; an unknown one
    /// (a modifier key on its own, a media key) matches nothing.
    pub fn find(&self, mods: Modifiers, code: &str) -> Option<Command> {
        let key = Key::from_code(code)?;
        self.bindings
            .iter()
            .find(|(_, shortcuts)| shortcuts.iter().any(|shortcut| shortcut.matches(mods, key)))
            .map(|(command, _)| *command)
    }

    pub fn shortcuts(&self, command: Command) -> &[Shortcut] {
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == command)
            .map_or(&[], |(_, shortcuts)| shortcuts)
    }

    /// The first shortcut as menus show it; empty when the command is unbound.
    pub fn shortcut_text(&self, command: Command) -> String {
        self.shortcuts(command)
            .first()
            .map(Shortcut::to_string)
            .unwrap_or_default()
    }

    /// Label with its shortcut in parentheses, the one form every tooltip uses.
    pub fn tooltip(&self, command: Command, lang: Lang) -> String {
        let label = command.label(lang);
        match self.shortcuts(command).first() {
            Some(shortcut) => format!("{label} ({shortcut})"),
            None => label,
        }
    }

    /// The rows of the keys page of the settings: the commands the palette lists that have a shortcut, in
    /// palette order, with every shortcut they have.
    pub fn rows(&self, lang: Lang) -> Vec<KeyRow> {
        Command::ALL
            .iter()
            .filter(|command| command.in_palette())
            .filter_map(|command| {
                let shortcuts: Vec<String> = self
                    .shortcuts(*command)
                    .iter()
                    .map(Shortcut::to_string)
                    .collect();
                (!shortcuts.is_empty()).then(|| KeyRow {
                    command: *command,
                    label: command.label(lang),
                    shortcuts,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRow {
    pub command: Command,
    pub label: String,
    pub shortcuts: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(text: &str) -> Shortcut {
        Shortcut::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    fn mods(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
        Modifiers {
            ctrl,
            shift,
            alt,
            meta: false,
        }
    }

    #[test]
    fn key_names_are_unique_and_every_key_parses_back_from_each_spelling() {
        for key in Key::all() {
            assert_eq!(Key::from_code(key.code()), Some(key));
            assert_eq!(Key::parse(key.code()), Some(key), "code of {}", key.code());
            assert_eq!(
                Key::parse(key.label()),
                Some(key),
                "label of {}",
                key.code()
            );
            assert_eq!(Key::parse(&key.label().to_ascii_lowercase()), Some(key));
        }
        for (alias, code) in ALIASES {
            assert_eq!(Key::parse(alias).map(Key::code), Some(*code));
            assert!(
                KEYS.iter()
                    .all(|(known, label, _)| !known.eq_ignore_ascii_case(alias)
                        && !label.eq_ignore_ascii_case(alias)),
                "{alias} shadows a key"
            );
        }
        let mut codes: Vec<_> = KEYS.iter().map(|(code, ..)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len());
    }

    #[test]
    fn keys_use_physical_names_for_letters_digits_and_punctuation() {
        assert_eq!(Key::from_code("KeyT").map(Key::label), Some("T"));
        assert_eq!(Key::from_code("Digit1").map(Key::label), Some("1"));
        assert_eq!(Key::from_code("Minus").map(Key::label), Some("-"));
        assert_eq!(Key::from_code("Equal").map(Key::label), Some("="));
        assert_eq!(
            Key::from_code("ArrowLeft").map(Key::code),
            Some("ArrowLeft")
        );
        assert_eq!(Key::from_code("F11").map(Key::code), Some("F11"));
        for unusable in [
            "ControlLeft",
            "ShiftRight",
            "AltLeft",
            "MetaLeft",
            "CapsLock",
            "t",
            "KeyTT",
            "",
        ] {
            assert_eq!(Key::from_code(unusable), None, "{unusable}");
        }
    }

    #[test]
    fn parses_modifiers_in_any_order_and_case() {
        let expected = Shortcut {
            mods: Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: false,
            },
            key: Key::from_code("KeyT").unwrap(),
        };
        assert_eq!(shortcut("shift+CTRL+t"), expected);
        assert_eq!(shortcut(" Ctrl + Shift + T "), expected);
        assert_eq!(shortcut("Control+Shift+KeyT"), expected);
    }

    #[test]
    fn understands_key_aliases() {
        assert_eq!(shortcut("Alt+Left").key.code(), "ArrowLeft");
        assert_eq!(shortcut("Ctrl+PgUp").key.code(), "PageUp");
        assert_eq!(shortcut("Ctrl+PageUp").key.code(), "PageUp");
        assert_eq!(shortcut("f5").key.code(), "F5");
        assert_eq!(shortcut("Ctrl+Space").key.code(), "Space");
        assert_eq!(shortcut("Shift+Insert").key.code(), "Insert");
        assert_eq!(shortcut("Ctrl+Comma").key.code(), "Comma");
        assert_eq!(
            shortcut("Win+Esc").mods,
            Modifiers {
                meta: true,
                ..Modifiers::default()
            }
        );
        assert_eq!(Shortcut::parse("Ctrl+Foo"), None);
        assert_eq!(Shortcut::parse("Ctrl+F25"), None);
    }

    #[test]
    fn treats_a_trailing_plus_as_the_plus_key() {
        let plus = shortcut("Ctrl++");
        assert_eq!((plus.mods.ctrl, plus.key.code()), (true, "Equal"));
        assert_eq!(shortcut("Ctrl+Plus"), plus);
    }

    #[test]
    fn rejects_shortcuts_without_a_key_or_with_two_keys() {
        for text in [
            "Ctrl+Shift",
            "A+B",
            "",
            "   ",
            "Ctrl+Shift+",
            "+",
            "Ctrl+nonsense+",
        ] {
            assert_eq!(Shortcut::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn rejects_characters_that_are_not_a_key_position() {
        // "!" is Shift+1 and "é" depends on the layout; neither names a physical key.
        for text in ["Ctrl+!", "Ctrl+é", "Ctrl+你"] {
            assert_eq!(Shortcut::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn formatting_reads_back_to_the_same_shortcut() {
        for key in Key::all() {
            for (ctrl, shift, alt, meta) in [
                (false, false, false, false),
                (true, false, false, false),
                (true, true, true, true),
                (false, true, true, false),
            ] {
                let original = Shortcut {
                    mods: Modifiers {
                        ctrl,
                        shift,
                        alt,
                        meta,
                    },
                    key,
                };
                assert_eq!(
                    Shortcut::parse(&original.to_string()),
                    Some(original),
                    "{original}"
                );
            }
        }
    }

    #[test]
    fn formatting_orders_modifiers_and_names_keys_for_menus() {
        assert_eq!(shortcut("shift+alt+ctrl+t").to_string(), "Ctrl+Alt+Shift+T");
        assert_eq!(shortcut("ctrl+win+left").to_string(), "Ctrl+Win+Left");
        assert_eq!(shortcut("ctrl+space").to_string(), "Ctrl+Space");
        assert_eq!(shortcut("alt+shift+=").to_string(), "Alt+Shift+=");
        assert_eq!(shortcut("ctrl+,").to_string(), "Ctrl+,");
    }

    #[test]
    fn matching_requires_the_exact_modifier_set() {
        let copy = shortcut("Ctrl+Shift+C");
        let c = Key::from_code("KeyC").unwrap();
        assert!(copy.matches(mods(true, true, false), c));
        assert!(!copy.matches(mods(true, false, false), c));
        assert!(!copy.matches(mods(true, true, true), c));
        assert!(!copy.matches(
            Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: true
            },
            c
        ));
        assert!(!copy.matches(mods(true, true, false), Key::from_code("KeyV").unwrap()));
    }

    fn defaults() -> Vec<(Command, &'static [&'static str])> {
        vec![
            (Command::TabNew, &["Ctrl+Shift+T"][..]),
            (Command::PaneClose, &["Ctrl+Shift+W"][..]),
            (Command::EditPaste, &["Ctrl+V", "Shift+Insert"][..]),
        ]
    }

    fn overrides(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(id, texts)| {
                (
                    id.to_string(),
                    texts.iter().map(|t| t.to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn overrides_replace_defaults_and_an_empty_override_unbinds() {
        let built = build_keymap(
            defaults(),
            &overrides(&[("tab.new", &["Ctrl+T"]), ("edit.paste", &[])]),
        );
        assert!(built.problems.is_empty(), "{:?}", built.problems);
        let keymap = built.keymap;
        assert_eq!(keymap.shortcut_text(Command::TabNew), "Ctrl+T");
        assert_eq!(keymap.shortcuts(Command::TabNew).len(), 1);
        assert!(keymap.shortcuts(Command::EditPaste).is_empty());
        assert_eq!(keymap.shortcut_text(Command::PaneClose), "Ctrl+Shift+W");
        assert_eq!(
            keymap.find(mods(true, true, false), "KeyT"),
            None,
            "the replaced default no longer fires"
        );
    }

    #[test]
    fn finds_the_command_for_a_key_event() {
        let keymap = build_keymap(defaults(), &BTreeMap::new()).keymap;
        assert_eq!(
            keymap.find(mods(false, true, false), "Insert"),
            Some(Command::EditPaste)
        );
        assert_eq!(
            keymap.find(mods(true, true, false), "KeyT"),
            Some(Command::TabNew)
        );
        assert_eq!(keymap.find(mods(true, false, false), "KeyX"), None);
        assert_eq!(keymap.find(mods(true, false, false), "ControlLeft"), None);
        assert_eq!(keymap.find(mods(true, false, false), ""), None);
    }

    #[test]
    fn unparsable_overrides_are_reported_and_leave_the_command_unbound() {
        let built = build_keymap(
            defaults(),
            &overrides(&[
                ("tab.new", &["nonsense+"]),
                ("pane.close", &["Ctrl+Q", "???"]),
            ]),
        );
        assert_eq!(built.problems.len(), 2);
        assert_eq!(built.problems[0], Problem::new("keybindings.pane.close", Expected::Shortcut("???".into())));
        assert_eq!(built.problems[1], Problem::new("keybindings.tab.new", Expected::Shortcut("nonsense+".into())));
        assert_eq!(built.problems[0].text(Lang::ZhTw), "keybindings.pane.close：「???」不是有效的快速鍵");
        assert_eq!(built.problems[1].text(Lang::EnUs), "keybindings.tab.new: \"nonsense+\" is not a valid shortcut");
        assert!(built.keymap.shortcuts(Command::TabNew).is_empty());
        assert_eq!(built.keymap.shortcut_text(Command::PaneClose), "Ctrl+Q");
    }

    #[test]
    fn overrides_for_unknown_commands_are_reported_and_ignored() {
        let built = build_keymap(
            defaults(),
            &overrides(&[("paste", &["Ctrl+V"]), ("tab.new", &["Ctrl+T"])]),
        );
        assert_eq!(built.problems.len(), 1);
        assert_eq!(built.problems[0], Problem::new("keybindings.paste", Expected::KnownCommand));
        assert_eq!(built.keymap.shortcut_text(Command::TabNew), "Ctrl+T");
    }

    #[test]
    fn a_binding_the_user_wrote_beats_a_default_on_the_same_chord() {
        // pane.close keeps its default Ctrl+Shift+W; the user gives the same chord to tab.new.
        let keymap = build_keymap(defaults(), &overrides(&[("tab.new", &["Ctrl+Shift+W"])])).keymap;
        assert_eq!(
            keymap.find(mods(true, true, false), "KeyW"),
            Some(Command::TabNew)
        );
        assert_eq!(keymap.find(mods(true, true, false), "KeyT"), None);
        assert!(keymap.shortcuts(Command::PaneClose).is_empty(), "pane.close no longer lists the chord it lost");
    }

    #[test]
    fn two_bindings_the_user_wrote_on_one_chord_are_reported() {
        let built = build_keymap(defaults(), &overrides(&[("tab.close", &["Ctrl+K"]), ("tab.new", &["Ctrl+K", "Ctrl+J"])]));
        assert_eq!(built.problems, [Problem::new("keybindings.tab.new", Expected::Clash(Command::TabClose))]);
        assert_eq!(built.keymap.shortcut_text(Command::TabNew), "Ctrl+J");
        assert_eq!(built.problems[0].text(Lang::EnUs), "keybindings.tab.new: Conflicts with \"Close tab\"");
    }

    #[test]
    fn lookup_order_does_not_depend_on_how_the_defaults_were_listed() {
        let forward =
            build_keymap(defaults(), &overrides(&[("tab.close", &["Ctrl+Shift+W"])])).keymap;
        let mut reversed_defaults = defaults();
        reversed_defaults.reverse();
        let reversed = build_keymap(
            reversed_defaults,
            &overrides(&[("tab.close", &["Ctrl+Shift+W"])]),
        )
        .keymap;
        assert_eq!(
            forward.find(mods(true, true, false), "KeyW"),
            Some(Command::TabClose)
        );
        assert_eq!(
            reversed.find(mods(true, true, false), "KeyW"),
            Some(Command::TabClose)
        );
    }

    #[test]
    fn tooltips_append_the_first_shortcut_to_the_label() {
        let keymap = build_keymap(defaults(), &BTreeMap::new()).keymap;
        assert_eq!(
            keymap.tooltip(Command::TabNew, Lang::ZhTw),
            "新增分頁 (Ctrl+Shift+T)"
        );
        assert_eq!(
            keymap.tooltip(Command::TabNew, Lang::EnUs),
            "New tab (Ctrl+Shift+T)"
        );
        assert_eq!(keymap.tooltip(Command::TabClose, Lang::ZhTw), "關閉分頁");
    }

    #[test]
    fn the_keys_page_lists_bound_palette_commands_with_all_their_shortcuts() {
        use crate::commands::default_bindings;
        let defaults = build_keymap(default_bindings(), &BTreeMap::new()).keymap;
        let rows = defaults.rows(Lang::EnUs);
        let row = |id: &str| rows.iter().find(|row| row.command.id() == id);

        assert_eq!(
            rows[0],
            KeyRow {
                command: Command::TabNew,
                label: "New tab".into(),
                shortcuts: vec!["Ctrl+Shift+T".into()]
            }
        );
        assert_eq!(
            row("edit.copy").unwrap().shortcuts,
            ["Ctrl+Shift+C", "Ctrl+Insert"]
        );
        assert!(row("tab.close").is_none(), "an unbound command has no row");
        assert!(
            row("edit.smartCopy").is_none() && row("tab.goto3").is_none(),
            "what the palette hides stays off the page"
        );
        assert!(
            rows.iter()
                .all(|row| row.command.in_palette() && !row.shortcuts.is_empty())
        );

        let overrides = overrides(&[("tab.close", &["Ctrl+Shift+Q"]), ("edit.paste", &[])]);
        let changed = build_keymap(default_bindings(), &overrides)
            .keymap
            .rows(Lang::ZhTw);
        assert_eq!(
            changed
                .iter()
                .find(|row| row.command == Command::TabClose)
                .unwrap()
                .shortcuts,
            ["Ctrl+Shift+Q"]
        );
        assert!(changed.iter().all(|row| row.command != Command::EditPaste));
        assert_eq!(changed[0].label, "新增分頁");
    }

    #[test]
    fn only_keys_that_type_text_need_a_modifier_to_be_shortcuts() {
        assert!(Key::from_code("KeyA").unwrap().types_text());
        assert!(Key::from_code("Space").unwrap().types_text());
        assert!(Key::from_code("Comma").unwrap().types_text());
        assert!(!Key::from_code("Enter").unwrap().types_text());
        assert!(!Key::from_code("F11").unwrap().types_text());
        assert!(Key::from_code("Comma").unwrap().is_swallowed_by_ime());
        assert!(Key::from_code("Space").unwrap().is_swallowed_by_ime());
        assert!(!Key::from_code("Digit1").unwrap().is_swallowed_by_ime());
        assert!(!Key::from_code("KeyA").unwrap().is_swallowed_by_ime());
    }
}
