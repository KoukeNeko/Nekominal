//! The rows of the settings drawer, in one table.
//!
//! A row says which section it is in, what it is called, which control edits it and how it reads and writes
//! [`Settings`]. The drawer builds its controls from the table, its search box and the command palette (which lists
//! "Settings: Background opacity") filter it, and the tests hold it against `settings.rs`: no key is left out or shown
//! twice, and a row changes its own key and nothing else. The shortcuts page is not in the table: its rows are the
//! commands of `commands.rs`, and what the user binds is stored in `Settings::keybindings`.

use crate::{
    commands::Command,
    fuzzy,
    keymap::Keymap,
    settings::{
        BLINK_RATE_RANGE, Backdrop, BellSound, CursorStyle, FONT_SIZE_RANGE, LONG_COMMAND_RANGE, Language, NoAgent, TabLayout, OPACITY_RANGE, Osc52Policy, RightClick, Settings,
        TabTitle, split_font_families,
    },
    strings::{Lang, Text},
    themes,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Terminal,
    Profiles,
    Agents,
    Keys,
    Advanced,
    /// The version of the app and whether a newer one is out: no settings, only what the page shows.
    About,
}

impl Section {
    /// In the order of the section strip.
    pub const ALL: [Section; 7] =
        [Section::Appearance, Section::Terminal, Section::Profiles, Section::Agents, Section::Keys, Section::Advanced, Section::About];

    pub fn text(self) -> Text {
        match self {
            Section::Appearance => Text::SettingsAppearance,
            Section::Terminal => Text::SettingsTerminal,
            Section::Profiles => Text::SettingsProfiles,
            Section::Agents => Text::SettingsAgents,
            Section::Keys => Text::SettingsKeys,
            Section::Advanced => Text::SettingsAdvanced,
            Section::About => Text::SettingsAbout,
        }
    }
}

/// What a row holds, in the form its control hands over.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    /// The position of the chosen option in [`Control::Choice`].
    Index(usize),
    Number(f64),
    Text(String),
}

/// The bounds and steps of a control that holds a number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    /// One press of an arrow key or of the plus and minus buttons.
    pub step: f64,
    /// One press of Page Up and Page Down.
    pub large: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    /// Off or on: two choices, [`Text::ToggleOff`] then [`Text::ToggleOn`].
    Toggle,
    /// A few named options; the value is the position of the chosen one.
    Choice(&'static [Text]),
    /// A percentage that is dragged.
    Slider(Range),
    /// A number that is typed or stepped.
    Number(Range),
    /// The colour schemes as tiles; the value is the name of the scheme.
    Themes,
    /// The profiles a tab can open, found at run time; the value is the id of the chosen one.
    Profiles,
    /// The layouts of the tabs as pictures of the window; the value is the position of the chosen one, as for a choice.
    Layouts(&'static [Text]),
    /// A line of text.
    Text,
}

/// One row of the drawer. Two rows are the same row when they have the same `id`.
#[derive(Clone, Copy, Debug)]
pub struct Descriptor {
    /// The key of the setting in `settings.json` (`appearance.opacity`): also what the row is found by.
    pub id: &'static str,
    pub section: Section,
    pub label: Text,
    pub control: Control,
    pub get: fn(&Settings) -> Value,
    /// Applies a value of the kind `get` returns; any other kind, or a value the setting cannot take, changes nothing.
    pub set: fn(&mut Settings, Value),
}

impl PartialEq for Descriptor {
    fn eq(&self, other: &Descriptor) -> bool {
        self.id == other.id
    }
}

impl Descriptor {
    pub fn label(&self, lang: Lang) -> &'static str {
        self.label.get(lang)
    }

    /// Whether the setting does anything with the others as they are: the rows of the sidebar do nothing with the tabs on
    /// top, and the groups only in a list.
    pub fn applies(&self, settings: &Settings) -> bool {
        match self.id {
            "tabs.sidebarOpen" => settings.tabs.layout.is_side(),
            "tabs.groupByState" => settings.tabs.layout == crate::settings::TabLayout::List,
            "cursor.blinkRate" => settings.cursor.blink,
            _ => true,
        }
    }

    /// The row differs from the default.
    pub fn is_modified(&self, settings: &Settings) -> bool {
        (self.get)(settings) != (self.get)(&Settings::default())
    }

    /// Puts the setting back to its default.
    pub fn reset(&self, settings: &mut Settings) {
        (self.set)(settings, (self.get)(&Settings::default()));
    }

    /// The names the row can be found by besides its label: its options, the names of the colour schemes (the user's own
    /// among them) and the key.
    fn other_names(&self, lang: Lang, schemes: &[themes::Scheme]) -> Vec<String> {
        let mut names = Vec::new();
        match self.control {
            Control::Toggle => {
                names.extend([Text::ToggleOff, Text::ToggleOn].map(|text| text.get(lang).to_owned()));
            }
            Control::Choice(options) | Control::Layouts(options) => names.extend(options.iter().map(|text| text.get(lang).to_owned())),
            Control::Themes => names.extend(themes::theme_names(schemes).map(str::to_owned)),
            Control::Slider(_) | Control::Number(_) | Control::Profiles | Control::Text => {}
        }
        names.push(self.id.to_owned());
        names
    }
}

/// Bytes in the unit of the large paste warning.
const KIB: f64 = 1024.0;

const PERCENT: Range = Range { min: OPACITY_RANGE.0 * 100.0, max: OPACITY_RANGE.1 * 100.0, step: 1.0, large: 5.0 };

fn choice<T: Copy + PartialEq>(all: &[T], current: T) -> Value {
    Value::Index(all.iter().position(|option| *option == current).unwrap_or(0))
}

fn chosen<T: Copy>(all: &[T], value: Value) -> Option<T> {
    match value {
        Value::Index(index) => all.get(index).copied(),
        _ => None,
    }
}

fn number(value: Value, range: Range) -> Option<f64> {
    match value {
        Value::Number(n) if n.is_finite() => Some(n.clamp(range.min, range.max)),
        _ => None,
    }
}

macro_rules! toggle {
    ($id:literal, $section:ident, $label:ident, $($field:tt)+) => {
        Descriptor {
            id: $id,
            section: Section::$section,
            label: Text::$label,
            control: Control::Toggle,
            get: |s| Value::Bool(s.$($field)+),
            set: |s, v| {
                if let Value::Bool(on) = v {
                    s.$($field)+ = on;
                }
            },
        }
    };
}

/// A whole number held in a field of the settings.
macro_rules! count {
    ($id:literal, $section:ident, $label:ident, $range:expr, $($field:tt)+) => {
        Descriptor {
            id: $id,
            section: Section::$section,
            label: Text::$label,
            control: Control::Number($range),
            get: |s| Value::Number(f64::from(s.$($field)+)),
            set: |s, v| {
                if let Some(n) = number(v, $range) {
                    s.$($field)+ = n.round() as _;
                }
            },
        }
    };
}

const BACKDROPS: [Text; 4] = [Text::BackdropAcrylic, Text::BackdropMica, Text::BackdropMicaAlt, Text::BackdropNone];
const CURSOR_STYLES: [Text; 3] = [Text::CursorBlock, Text::CursorBar, Text::CursorUnderline];
const RIGHT_CLICKS: [Text; 2] = [Text::RightClickSmart, Text::RightClickMenu];
const BELL_SOUNDS: [Text; 3] = [Text::BackdropNone, Text::BellSoundBackground, Text::BellSoundAlways];
const OSC52_POLICIES: [Text; 3] = [Text::ToggleOff, Text::Osc52Write, Text::Osc52ReadWrite];
const LANGUAGES: [Text; 3] = [Text::CommonAuto, Text::LanguageZhTw, Text::LanguageEnUs];
const NO_AGENT: [Text; 2] = [Text::StatusBarPlace, Text::StatusBarHide];
const LAYOUTS: [Text; 3] = [Text::LayoutTop, Text::LayoutList, Text::LayoutIcons];

const FONT_SIZE: Range = Range { min: FONT_SIZE_RANGE.0, max: FONT_SIZE_RANGE.1, step: 1.0, large: 4.0 };
const LINE_HEIGHT: Range = Range { min: 1.0, max: 2.0, step: 0.05, large: 0.25 };
const SCROLLBACK: Range = Range { min: 0.0, max: 1_000_000.0, step: 1000.0, large: 10_000.0 };
/// The large paste warning in KiB, 0 (off) to the 100 MB the file accepts.
const LARGE_PASTE: Range = Range { min: 0.0, max: 97_656.0, step: 1.0, large: 100.0 };
const CLOSE_THRESHOLD: Range = Range { min: 1.0, max: 1000.0, step: 1.0, large: 5.0 };
const BLINK_RATE: Range = Range { min: BLINK_RATE_RANGE.0, max: BLINK_RATE_RANGE.1, step: 50.0, large: 250.0 };
const LONG_COMMAND: Range = Range { min: LONG_COMMAND_RANGE.0, max: LONG_COMMAND_RANGE.1, step: 5.0, large: 60.0 };

/// Every row of the drawer, section by section, in the order the section shows them.
pub const TABLE: &[Descriptor] = &[
    // Appearance
    Descriptor {
        id: "tabs.layout",
        section: Section::Appearance,
        label: Text::SettingsTabLayout,
        control: Control::Layouts(&LAYOUTS),
        get: |s| choice(&TabLayout::ALL, s.tabs.layout),
        set: |s, v| {
            if let Some(layout) = chosen(&TabLayout::ALL, v) {
                s.tabs.layout = layout;
            }
        },
    },
    toggle!("tabs.sidebarOpen", Appearance, SettingsSidebarOpen, tabs.sidebar_open),
    toggle!("tabs.groupByState", Appearance, SettingsGroupByState, tabs.group_by_state),
    Descriptor {
        id: "theme",
        section: Section::Appearance,
        label: Text::SettingsTheme,
        control: Control::Themes,
        get: |s| Value::Text(s.theme.clone()),
        set: |s, v| {
            if let Value::Text(name) = v
                && themes::is_known(&s.schemes, &name)
            {
                s.theme = name;
            }
        },
    },
    Descriptor {
        id: "appearance.backdrop",
        section: Section::Appearance,
        label: Text::SettingsBackdrop,
        control: Control::Choice(&BACKDROPS),
        get: |s| choice(&Backdrop::ALL, s.appearance.backdrop),
        set: |s, v| {
            if let Some(backdrop) = chosen(&Backdrop::ALL, v) {
                s.appearance.backdrop = backdrop;
            }
        },
    },
    Descriptor {
        id: "appearance.opacity",
        section: Section::Appearance,
        label: Text::SettingsOpacity,
        control: Control::Slider(PERCENT),
        get: |s| Value::Number((s.appearance.opacity * 100.0).round()),
        set: |s, v| {
            if let Some(percent) = number(v, PERCENT) {
                s.appearance.opacity = percent.round() / 100.0;
            }
        },
    },
    Descriptor {
        id: "font.family",
        section: Section::Appearance,
        label: Text::SettingsFontFamily,
        control: Control::Text,
        get: |s| Value::Text(s.font.family.join(", ")),
        set: |s, v| {
            if let Value::Text(list) = v {
                let families = split_font_families(&list);
                if !families.is_empty() {
                    s.font.family = families;
                }
            }
        },
    },
    Descriptor {
        id: "font.size",
        section: Section::Appearance,
        label: Text::SettingsFontSize,
        control: Control::Number(FONT_SIZE),
        get: |s| Value::Number(s.font.size),
        set: |s, v| {
            if let Some(size) = number(v, FONT_SIZE) {
                s.font.size = size;
            }
        },
    },
    Descriptor {
        id: "font.lineHeight",
        section: Section::Appearance,
        label: Text::SettingsLineHeight,
        control: Control::Number(LINE_HEIGHT),
        get: |s| Value::Number(s.font.line_height),
        set: |s, v| {
            if let Some(height) = number(v, LINE_HEIGHT) {
                s.font.line_height = (height * 100.0).round() / 100.0;
            }
        },
    },
    Descriptor {
        id: "cursor.style",
        section: Section::Appearance,
        label: Text::SettingsCursorStyle,
        control: Control::Choice(&CURSOR_STYLES),
        get: |s| choice(&CursorStyle::ALL, s.cursor.style),
        set: |s, v| {
            if let Some(style) = chosen(&CursorStyle::ALL, v) {
                s.cursor.style = style;
            }
        },
    },
    toggle!("cursor.blink", Appearance, SettingsCursorBlink, cursor.blink),
    count!("cursor.blinkRate", Appearance, SettingsCursorBlinkRate, BLINK_RATE, cursor.blink_rate),
    Descriptor {
        id: "tabs.title.parts",
        section: Section::Appearance,
        label: Text::SettingsTabTitle,
        control: Control::Text,
        get: |s| Value::Text(s.tabs.title.to_text()),
        set: |s, v| {
            if let Value::Text(text) = v
                && let Some(title) = TabTitle::from_text(&text)
            {
                s.tabs.title = title;
            }
        },
    },
    // Terminal
    count!("scrollback", Terminal, SettingsScrollback, SCROLLBACK, scrollback),
    toggle!("copyOnSelect", Terminal, SettingsCopyOnSelect, copy_on_select),
    toggle!("stickyHeader", Terminal, SettingsStickyHeader, sticky_header),
    Descriptor {
        id: "rightClick",
        section: Section::Terminal,
        label: Text::SettingsRightClick,
        control: Control::Choice(&RIGHT_CLICKS),
        get: |s| choice(&RightClick::ALL, s.right_click),
        set: |s, v| {
            if let Some(action) = chosen(&RightClick::ALL, v) {
                s.right_click = action;
            }
        },
    },
    toggle!("paste.warnMultiline", Terminal, SettingsWarnMultiline, paste.warn_multiline),
    Descriptor {
        id: "paste.warnLargeBytes",
        section: Section::Terminal,
        label: Text::SettingsWarnLarge,
        control: Control::Number(LARGE_PASTE),
        get: |s| Value::Number((s.paste.warn_large_bytes as f64 / KIB).round()),
        set: |s, v| {
            if let Some(kib) = number(v, LARGE_PASTE) {
                s.paste.warn_large_bytes = (kib.round() * KIB) as u64;
            }
        },
    },
    toggle!("bell.visual", Terminal, SettingsBellVisual, bell.visual),
    toggle!("bell.taskbar", Terminal, SettingsBellTaskbar, bell.taskbar),
    Descriptor {
        id: "bell.sound",
        section: Section::Terminal,
        label: Text::SettingsBellSound,
        control: Control::Choice(&BELL_SOUNDS),
        get: |s| choice(&BellSound::ALL, s.bell.sound),
        set: |s, v| {
            if let Some(sound) = chosen(&BellSound::ALL, v) {
                s.bell.sound = sound;
            }
        },
    },
    toggle!("notifications.toast", Terminal, SettingsToast, notifications.toast),
    count!("notifications.longCommand", Terminal, SettingsLongCommand, LONG_COMMAND, notifications.long_command),
    toggle!("confirmClose", Terminal, SettingsConfirmClose, confirm_close),
    count!("confirmCloseThreshold", Terminal, SettingsConfirmCloseThreshold, CLOSE_THRESHOLD, confirm_close_threshold),
    toggle!("restoreSession", Terminal, SettingsRestoreSession, restore_session),
    toggle!("explorerMenu", Advanced, SettingsExplorerMenu, explorer_menu),
    toggle!("inheritEnvironment", Advanced, SettingsInheritEnvironment, inherit_environment),
    Descriptor {
        id: "editor",
        section: Section::Advanced,
        label: Text::SettingsEditor,
        control: Control::Text,
        get: |s| Value::Text(s.editor.clone()),
        set: |s, v| {
            if let Value::Text(text) = v
                && crate::editor::is_valid(&text)
            {
                s.editor = text.trim().to_owned();
            }
        },
    },
    // Profiles
    Descriptor {
        id: "defaultProfile",
        section: Section::Profiles,
        label: Text::SettingsDefaultProfile,
        control: Control::Profiles,
        get: |s| Value::Text(s.default_profile.clone()),
        set: |s, v| {
            if let Value::Text(id) = v {
                s.default_profile = id;
            }
        },
    },
    // Agents
    toggle!("agentEnvironment", Agents, SettingsAgentEnvironment, agent_environment),
    toggle!("statusBar.show", Agents, SettingsStatusBar, status_bar.show),
    Descriptor {
        id: "statusBar.noAgent",
        section: Section::Agents,
        label: Text::SettingsStatusBarNoAgent,
        control: Control::Choice(&NO_AGENT),
        get: |s| choice(&NoAgent::ALL, s.status_bar.no_agent),
        set: |s, v| {
            if let Some(mode) = chosen(&NoAgent::ALL, v) {
                s.status_bar.no_agent = mode;
            }
        },
    },
    toggle!("statusBar.state", Agents, SettingsStatusState, status_bar.state),
    toggle!("statusBar.model", Agents, SettingsStatusModel, status_bar.model),
    toggle!("statusBar.context", Agents, SettingsStatusContext, status_bar.context),
    toggle!("statusBar.cost", Agents, SettingsStatusCost, status_bar.cost),
    toggle!("statusBar.time", Agents, SettingsStatusTime, status_bar.time),
    toggle!("statusBar.branch", Agents, SettingsStatusBranch, status_bar.branch),
    toggle!("statusBar.folder", Agents, SettingsStatusFolder, status_bar.folder),
    toggle!("keyboard.kittyProtocol", Agents, SettingsKitty, keyboard.kitty_protocol),
    toggle!("shellIntegration", Agents, SettingsShellIntegration, shell_integration),
    Descriptor {
        id: "osc52",
        section: Section::Agents,
        label: Text::SettingsOsc52,
        control: Control::Choice(&OSC52_POLICIES),
        get: |s| choice(&Osc52Policy::ALL, s.osc52),
        set: |s, v| {
            if let Some(policy) = chosen(&Osc52Policy::ALL, v) {
                s.osc52 = policy;
            }
        },
    },
    // Advanced
    Descriptor {
        id: "language",
        section: Section::Advanced,
        label: Text::SettingsLanguage,
        control: Control::Choice(&LANGUAGES),
        get: |s| choice(&Language::ALL, s.language),
        set: |s, v| {
            if let Some(language) = chosen(&Language::ALL, v) {
                s.language = language;
            }
        },
    },
];

pub fn find(id: &str) -> Option<&'static Descriptor> {
    TABLE.iter().find(|row| row.id == id)
}

/// The rows of a section, in the order it shows them.
pub fn in_section(section: Section) -> impl Iterator<Item = &'static Descriptor> {
    TABLE.iter().filter(move |row| row.section == section)
}

/// What the search of the drawer finds: a setting, or a command whose shortcut can be changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Setting(&'static Descriptor),
    Key(Command),
}

impl Row {
    pub fn section(&self) -> Section {
        match self {
            Row::Setting(row) => row.section,
            Row::Key(_) => Section::Keys,
        }
    }
}

/// The headings that split a page into sections, each over the row with this id and the rows after it on its page.
const HEADINGS: &[(&str, Text)] = &[("notifications.toast", Text::SettingsNotificationsHeading)];

/// The heading that starts a section of its page at the row with this id, if one does.
pub fn heading_before(id: &str) -> Option<Text> {
    HEADINGS.iter().find(|(first, _)| *first == id).map(|(_, text)| *text)
}

/// The page a heading is on: the page of the row it stands over.
pub fn heading_section(text: Text) -> Section {
    HEADINGS.iter().find(|(_, heading)| *heading == text).and_then(|(first, _)| find(first)).map_or(Section::Terminal, |row| row.section)
}

/// The rows of the shortcuts page: every command, in the order of the palette, the ones it leaves out (the copy that
/// leaves the key to the program, the numbered tabs, the profiles menu) included: each has a shortcut to change.
pub fn key_rows() -> impl Iterator<Item = Command> {
    Command::ALL.iter().copied()
}

/// How much better a match of the label is than a match of anything else the row is called.
const LABEL_BONUS: f64 = 100.0;
const OPTION_BONUS: f64 = 50.0;

fn best(query: &str, texts: &[String], bonus: f64) -> Option<f64> {
    texts.iter().filter_map(|text| fuzzy::fuzzy_score(query, text)).max_by(f64::total_cmp).map(|score| score + bonus)
}

/// The rows that match `query` in `lang`, best match first. The label counts most; the options of a row, the names of
/// the colour schemes, the key in `settings.json` and the shortcuts bound to a command also find it. An empty query
/// finds nothing: the drawer then shows the section that is chosen.
pub fn search(query: &str, lang: Lang, keymap: &Keymap) -> Vec<Row> {
    search_with(query, lang, keymap, &[])
}

/// [`search`], the user's own colour schemes (`schemes`) also finding the row of the colour scheme by their names.
pub fn search_with(query: &str, lang: Lang, keymap: &Keymap, schemes: &[themes::Scheme]) -> Vec<Row> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let settings = TABLE.iter().map(|row| {
        let label = best(query, &[row.label(lang).to_owned()], LABEL_BONUS);
        let other = best(query, &row.other_names(lang, schemes), 0.0);
        (Row::Setting(row), label.into_iter().chain(other).max_by(f64::total_cmp))
    });
    let keys = key_rows().map(|command| {
        let label = best(query, &[command.label(lang)], LABEL_BONUS);
        let shortcuts: Vec<String> = keymap.shortcuts(command).iter().map(ToString::to_string).collect();
        let other = best(query, &shortcuts, OPTION_BONUS);
        (Row::Key(command), label.into_iter().chain(other).max_by(f64::total_cmp))
    });
    let mut found: Vec<(f64, usize, Row)> = settings
        .chain(keys)
        .enumerate()
        .filter_map(|(order, (row, score))| score.map(|score| (score, order, row)))
        .collect();
    // Equal scores keep the order of the drawer.
    found.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value as Json;

    use super::*;
    use crate::{commands::default_bindings, keymap::build_keymap};

    /// The keys of `settings.json` the drawer has no row for, and why.
    const NOT_SHOWN: &[(&str, &str)] = &[
        ("profiles", "the profiles page lists and edits them, a page of its own for each"),
        ("snippets", "the Terminal page lists them under its rows, a line each with its fields"),
        ("keybindings", "the shortcuts page edits them"),
        ("schemes", "the colour scheme tiles list them, with buttons that import them and a delete button on the user's own"),
        ("font.weight", "the renderer has no base weight yet, so a row would change nothing"),
        ("appearance.tabBarDensity", "the tab strip has one density yet, so a row would change nothing"),
        ("paste.images", "the row waits for its approved design"),
        ("welcomed", "the welcome of the first start sets it; there is nothing to choose"),
        ("notifications.push.url", "the push row waits for its approved design"),
        ("notifications.push.kind", "the push row waits for its approved design"),
        ("notifications.push.token", "the push row waits for its approved design"),
        ("notifications.push.when", "the push row waits for its approved design"),
    ];

    /// The dotted path of every value of `json` that is not an object with members.
    fn leaves(prefix: &str, json: &Json, out: &mut BTreeSet<String>) {
        match json {
            Json::Object(map) if !map.is_empty() => {
                for (key, value) in map {
                    let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                    leaves(&path, value, out);
                }
            }
            _ => {
                out.insert(prefix.to_owned());
            }
        }
    }

    fn keymap() -> Keymap {
        build_keymap(default_bindings(), &Default::default()).keymap
    }

    #[test]
    fn every_key_of_the_settings_has_exactly_one_row_or_a_reason_not_to() {
        let mut keys = BTreeSet::new();
        leaves("", &serde_json::to_value(Settings::default()).unwrap(), &mut keys);
        let mut shown = BTreeSet::new();
        for row in TABLE {
            assert!(shown.insert(row.id), "{} has two rows", row.id);
            assert!(keys.contains(row.id), "{} is not a key of settings.json", row.id);
        }
        for key in &keys {
            let reason = NOT_SHOWN.iter().find(|(skipped, _)| skipped == key);
            assert_eq!(shown.contains(key.as_str()), reason.is_none(), "{key} is either shown or excused, not both");
        }
        for (key, _) in NOT_SHOWN {
            assert!(keys.contains(*key), "{key} is excused but is no key of settings.json");
        }
    }

    #[test]
    fn a_row_has_a_label_in_both_languages_and_no_label_twice_in_a_section() {
        for lang in Lang::ALL {
            for section in Section::ALL {
                let mut seen = BTreeSet::new();
                for row in in_section(section) {
                    assert!(!row.label(lang).is_empty(), "{}", row.id);
                    assert!(seen.insert(row.label(lang)), "{lang:?}: two rows of {section:?} read {}", row.label(lang));
                }
            }
        }
        for section in Section::ALL {
            assert_ne!(section.text().get(Lang::ZhTw), section.text().get(Lang::EnUs));
        }
    }

    #[test]
    fn the_sections_hold_what_the_study_gives_them() {
        let ids = |section| in_section(section).map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids(Section::Profiles), ["defaultProfile"]);
        assert_eq!(ids(Section::Keys), Vec::<&str>::new(), "the shortcuts page lists commands, not settings");
        assert_eq!(ids(Section::Advanced), ["explorerMenu", "inheritEnvironment", "editor", "language"]);
        assert_eq!(ids(Section::About), Vec::<&str>::new(), "the About page shows the version, not settings");
        assert!(ids(Section::Terminal).contains(&"confirmClose"), "the close confirmation moved from General");
        assert!(ids(Section::Profiles).iter().chain(&ids(Section::Terminal)).all(|id| *id != "theme"));
        assert_eq!(ids(Section::Appearance)[..6], ["tabs.layout", "tabs.sidebarOpen", "tabs.groupByState", "theme", "appearance.backdrop", "appearance.opacity"]);
        assert_eq!(ids(Section::Agents).last(), Some(&"osc52"));
    }

    #[test]
    fn the_options_of_a_choice_are_the_labels_of_the_values_it_stands_for() {
        let options = |id: &str| match find(id).unwrap().control {
            Control::Choice(options) => options.to_vec(),
            other => panic!("{id} is {other:?}"),
        };
        assert_eq!(options("appearance.backdrop"), Backdrop::ALL.map(Backdrop::text));
        assert_eq!(options("cursor.style"), CursorStyle::ALL.map(CursorStyle::text));
        assert_eq!(options("rightClick"), RightClick::ALL.map(RightClick::text));
        assert_eq!(options("bell.sound"), BellSound::ALL.map(BellSound::text));
        assert_eq!(options("osc52"), Osc52Policy::ALL.map(Osc52Policy::text));
        assert_eq!(options("statusBar.noAgent"), NoAgent::ALL.map(NoAgent::text));
        assert_eq!(options("language"), Language::ALL.map(Language::text));
    }

    /// A value other than the default that the control of the row can hand over.
    fn other_than_default(row: &Descriptor) -> Value {
        let default = (row.get)(&Settings::default());
        match (row.control, default) {
            (Control::Toggle, Value::Bool(on)) => Value::Bool(!on),
            (Control::Choice(options) | Control::Layouts(options), Value::Index(index)) => Value::Index((index + 1) % options.len()),
            (Control::Slider(range) | Control::Number(range), Value::Number(n)) => {
                let moved = if n + range.step <= range.max { n + range.step } else { n - range.step };
                Value::Number((moved * 100.0).round() / 100.0)
            }
            (Control::Themes, _) => Value::Text("Nord".to_owned()),
            (Control::Profiles, _) => Value::Text("wsl".to_owned()),
            (Control::Text, _) if row.id == "tabs.title.parts" => Value::Text("folder, title|profile".to_owned()),
            (Control::Text, _) if row.id == "editor" => Value::Text(r#"zed "{file}:{line}""#.to_owned()),
            (Control::Text, _) => Value::Text("Fira Code, Consolas".to_owned()),
            (control, value) => panic!("{}: {control:?} cannot hold {value:?}", row.id),
        }
    }

    #[test]
    fn a_row_reads_back_what_it_wrote_and_resetting_it_restores_the_default() {
        for row in TABLE {
            let wanted = other_than_default(row);
            let mut settings = Settings::default();
            assert!(!row.is_modified(&settings), "{}", row.id);
            (row.set)(&mut settings, wanted.clone());
            assert_eq!((row.get)(&settings), wanted, "{}", row.id);
            assert!(row.is_modified(&settings), "{}", row.id);
            row.reset(&mut settings);
            assert_eq!(settings, Settings::default(), "{}: reset leaves something behind", row.id);
        }
    }

    #[test]
    fn a_row_changes_its_own_key_in_the_file_and_no_other() {
        for row in TABLE {
            let mut settings = Settings::default();
            (row.set)(&mut settings, other_than_default(row));
            let written: Json = serde_json::from_str(&crate::settings::serialize(&settings)).unwrap();
            let mut keys = BTreeSet::new();
            leaves("", &written, &mut keys);
            assert_eq!(keys, BTreeSet::from([row.id.to_owned()]), "{}", row.id);
            assert_eq!(crate::settings::parse(Some(&crate::settings::serialize(&settings))).settings, settings);
        }
    }

    #[test]
    fn a_value_the_setting_cannot_take_changes_nothing() {
        let mut settings = Settings::default();
        let before = settings.clone();
        for (id, bad) in [
            ("theme", Value::Text("No Such Scheme".into())),
            ("appearance.backdrop", Value::Index(9)),
            ("appearance.backdrop", Value::Bool(true)),
            ("keyboard.kittyProtocol", Value::Number(1.0)),
            ("font.size", Value::Number(f64::NAN)),
            ("font.family", Value::Text(" , ".into())),
            ("tabs.title.parts", Value::Text("folder, branch".into())),
            ("tabs.title.parts", Value::Text(" , ".into())),
            ("osc52", Value::Text("write".into())),
            ("editor", Value::Text("code -g".into())),
            ("editor", Value::Text("{file}".into())),
        ] {
            (find(id).unwrap().set)(&mut settings, bad);
            assert_eq!(settings, before, "{id}");
        }
    }

    #[test]
    fn numbers_are_kept_inside_their_range_and_whole_where_the_setting_is() {
        let set = |id: &str, n: f64| {
            let mut settings = Settings::default();
            (find(id).unwrap().set)(&mut settings, Value::Number(n));
            settings
        };
        assert_eq!(set("font.size", 500.0).font.size, FONT_SIZE_RANGE.1);
        assert_eq!(set("font.size", 1.0).font.size, FONT_SIZE_RANGE.0);
        assert_eq!(set("appearance.opacity", 5.0).appearance.opacity, 0.2);
        assert_eq!(set("appearance.opacity", 60.0).appearance.opacity, 0.6);
        assert_eq!(set("appearance.opacity", 60.4).appearance.opacity, 0.6);
        assert_eq!(set("font.lineHeight", 1.234).font.line_height, 1.23);
        assert_eq!(set("scrollback", 2500.4).scrollback, 2500);
        assert_eq!(set("confirmCloseThreshold", 0.0).confirm_close_threshold, 1);
        // The large paste warning is typed in KiB and kept in bytes; 0 turns it off.
        assert_eq!(set("paste.warnLargeBytes", 8.0).paste.warn_large_bytes, 8192);
        assert_eq!(set("paste.warnLargeBytes", 0.0).paste.warn_large_bytes, 0);
        assert_eq!((find("paste.warnLargeBytes").unwrap().get)(&Settings::default()), Value::Number(5.0));
    }

    #[test]
    fn the_font_row_takes_a_comma_separated_list_and_shows_the_one_in_effect() {
        let row = find("font.family").unwrap();
        assert_eq!(
            (row.get)(&Settings::default()),
            Value::Text("Cascadia Mono, Consolas, Microsoft JhengHei UI, Segoe UI Emoji".into())
        );
        let mut settings = Settings::default();
        (row.set)(&mut settings, Value::Text("\"Fira Code\", 'Consolas' ,, ".into()));
        assert_eq!(settings.font.family, ["Fira Code", "Consolas"]);
    }

    #[test]
    fn searching_finds_a_setting_by_its_label_in_either_language() {
        let keymap = keymap();
        let first = |query: &str, lang| match search(query, lang, &keymap).first() {
            Some(Row::Setting(row)) => row.id,
            other => panic!("{query:?} in {lang:?} found {other:?}"),
        };
        assert_eq!(first("不透明", Lang::ZhTw), "appearance.opacity");
        assert_eq!(first("opacity", Lang::EnUs), "appearance.opacity");
        assert_eq!(first("背景材質", Lang::ZhTw), "appearance.backdrop");
        assert_eq!(first("backdrop", Lang::EnUs), "appearance.backdrop");
        assert_eq!(first("色彩配置", Lang::ZhTw), "theme");
        assert_eq!(first("scrollback", Lang::EnUs), "scrollback");
        assert_eq!(first("語言", Lang::ZhTw), "language");
        assert!(search("   ", Lang::ZhTw, &keymap).is_empty());
        assert!(search("zzzzqqqq", Lang::EnUs, &keymap).is_empty());
    }

    #[test]
    fn a_colour_scheme_of_ones_own_finds_the_row_of_the_colour_scheme() {
        let keymap = keymap();
        let mut mine = themes::scheme("Nord").clone();
        mine.name = "Zebrafish Night".into();
        let found = |schemes: &[themes::Scheme]| search_with("zebrafish", Lang::EnUs, &keymap, schemes).into_iter().any(|row| row == Row::Setting(find("theme").unwrap()));
        assert!(found(&[mine]));
        assert!(!found(&[]));
    }

    #[test]
    fn searching_also_finds_a_row_by_an_option_a_colour_scheme_or_the_key_of_the_file() {
        let keymap = keymap();
        let ids = |query: &str, lang| -> Vec<&'static str> {
            search(query, lang, &keymap)
                .into_iter()
                .filter_map(|row| if let Row::Setting(row) = row { Some(row.id) } else { None })
                .collect()
        };
        assert_eq!(ids("壓克力", Lang::ZhTw)[0], "appearance.backdrop");
        assert_eq!(ids("mica alt", Lang::EnUs)[0], "appearance.backdrop");
        assert_eq!(ids("nord", Lang::EnUs)[0], "theme");
        assert_eq!(ids("kittyProtocol", Lang::ZhTw)[0], "keyboard.kittyProtocol");
        assert!(ids("Read and write", Lang::EnUs).contains(&"osc52"));
    }

    #[test]
    fn commands_are_found_by_name_and_by_shortcut_next_to_the_settings() {
        let keymap = keymap();
        assert_eq!(search("Scrollback", Lang::EnUs, &keymap), [Row::Setting(find("scrollback").unwrap())]);
        let found = search("Scroll", Lang::EnUs, &keymap);
        assert!(found.contains(&Row::Setting(find("scrollback").unwrap())));
        assert!(found.contains(&Row::Key(Command::ScrollTop)));
        assert!(search("新增分頁", Lang::ZhTw, &keymap).contains(&Row::Key(Command::TabNew)));
        assert_eq!(search("Ctrl+Shift+T", Lang::EnUs, &keymap).first(), Some(&Row::Key(Command::TabNew)));
        assert!(search("Ctrl+Shift+T", Lang::EnUs, &keymap).iter().all(|row| row.section() == Section::Keys));
    }

    #[test]
    fn the_shortcuts_page_lists_every_command_in_the_order_of_the_palette() {
        let listed: Vec<Command> = key_rows().collect();
        assert_eq!(listed.first(), Some(&Command::TabNew));
        assert!(listed.contains(&Command::EditSmartCopy) && listed.contains(&Command::TabGoto(3)) && listed.contains(&Command::TabProfiles));
        assert!(listed.contains(&Command::TabClose), "a command without a shortcut can be given one");
        assert_eq!(listed.len(), Command::ALL.len());
    }
}
