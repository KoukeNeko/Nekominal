//! Settings model: defaults, tolerant parsing of `settings.json`, and minimal serialization.
//!
//! Parsing never fails: an invalid value falls back to its default and is reported in
//! [`Parsed::problems`], so a typo in the file can never prevent the terminal from starting.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    commands::Command,
    strings::{Lang, Text},
    themes,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

impl CursorStyle {
    /// The choices in the order every list of them shows.
    pub const ALL: [CursorStyle; 3] = [CursorStyle::Block, CursorStyle::Bar, CursorStyle::Underline];

    pub fn text(self) -> Text {
        match self {
            CursorStyle::Block => Text::CursorBlock,
            CursorStyle::Bar => Text::CursorBar,
            CursorStyle::Underline => Text::CursorUnderline,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RightClick {
    /// Copy the selection, or paste when nothing is selected.
    Smart,
    Menu,
}

impl RightClick {
    pub const ALL: [RightClick; 2] = [RightClick::Smart, RightClick::Menu];

    pub fn text(self) -> Text {
        match self {
            RightClick::Smart => Text::RightClickSmart,
            RightClick::Menu => Text::RightClickMenu,
        }
    }
}

/// What a program in a pane may do with the clipboard through OSC 52.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Osc52Policy {
    Off,
    /// A program may copy text to the clipboard, which is how a full-screen program that has its own selection
    /// (Claude Code, tmux, vim) copies. The default.
    Write,
    /// A program may also read the clipboard. Whatever is on it reaches that program, so it is opt-in.
    ReadWrite,
}

impl Osc52Policy {
    pub const ALL: [Osc52Policy; 3] = [Osc52Policy::Off, Osc52Policy::Write, Osc52Policy::ReadWrite];

    pub fn text(self) -> Text {
        match self {
            Osc52Policy::Off => Text::ToggleOff,
            Osc52Policy::Write => Text::Osc52Write,
            Osc52Policy::ReadWrite => Text::Osc52ReadWrite,
        }
    }
}

/// Language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Language {
    /// Follows the system: Traditional Chinese for any Chinese locale, English otherwise.
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "en-US")]
    EnUs,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::Auto, Language::ZhTw, Language::EnUs];

    /// `system_tag` is the locale name of the user (`zh-Hant-TW`).
    pub fn resolve(self, system_tag: &str) -> Lang {
        match self {
            Language::Auto => Lang::from_system_tag(system_tag),
            Language::ZhTw => Lang::ZhTw,
            Language::EnUs => Lang::EnUs,
        }
    }

    pub fn text(self) -> Text {
        match self {
            Language::Auto => Text::CommonAuto,
            Language::ZhTw => Text::LanguageZhTw,
            Language::EnUs => Text::LanguageEnUs,
        }
    }
}

/// How much room the tab strip takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TabBarDensity {
    Comfortable,
    Compact,
}

impl TabBarDensity {
    pub fn text(self) -> Text {
        match self {
            TabBarDensity::Comfortable => Text::DensityComfortable,
            TabBarDensity::Compact => Text::DensityCompact,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CustomProfile {
    pub id: String,
    pub name: String,
    /// Empty for an entry that only adjusts a profile found on the machine (its id is that profile's).
    pub program: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(flatten)]
    pub options: ProfileOptions,
}

/// What a profile changes for its own panes, against the settings of all of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProfileOptions {
    /// Environment variables its program gets, besides those of every pane.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// The color scheme of its panes, instead of the window's.
    #[serde(rename = "colorScheme", skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    /// The size of the text of its panes, in points.
    #[serde(rename = "fontSize", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// It runs as administrator: from a window that is not, choosing it opens an elevated window (UAC asks first).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub elevate: bool,
}

/// Text the palette and the suggestions type into the focused pane, by its name: a command with `"\r`" at its end runs it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Snippet {
    pub name: String,
    pub input: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    /// Family names in fallback order.
    pub family: Vec<String>,
    pub size: f64,
    pub weight: u16,
    pub line_height: f64,
}

impl Font {
    /// The size while the zoom commands are in effect: one point per step, within [`FONT_SIZE_RANGE`].
    pub fn size_after_zoom(&self, steps: i32) -> f64 {
        (self.size + f64::from(steps)).clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cursor {
    pub style: CursorStyle,
    pub blink: bool,
    /// How long the cursor stays on, and then off, while it blinks, in milliseconds.
    #[serde(rename = "blinkRate")]
    pub blink_rate: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteSettings {
    pub warn_multiline: bool,
    pub warn_large_bytes: u64,
}

/// When the bell of a program is also heard. Nothing else in the interface makes a sound, whatever this says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BellSound {
    /// Never: the default.
    None,
    /// Only when the pane that rang is not the one being looked at (another tab, or the window is not active).
    Background,
    Always,
}

impl BellSound {
    pub const ALL: [BellSound; 3] = [BellSound::None, BellSound::Background, BellSound::Always];

    pub fn text(self) -> Text {
        match self {
            // "None" is the word the backdrop row uses for nothing.
            BellSound::None => Text::BackdropNone,
            BellSound::Background => Text::BellSoundBackground,
            BellSound::Always => Text::BellSoundAlways,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bell {
    /// A tab that rings out of sight gets an attention mark.
    pub visual: bool,
    /// The taskbar button flashes when the window is not active.
    pub taskbar: bool,
    pub sound: BellSound,
}

/// A thing a tab title can say (`tabs.title.parts`). The names are the words of `settings.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlePart {
    /// What a program or an agent called the session (OSC 0, 2 or 7501), unless that only repeats the profile, the shell
    /// or the agent.
    Title,
    /// The last folder of the working directory.
    Folder,
    /// The name of the profile.
    Profile,
    /// The program of the pane without its extension: `pwsh`.
    Shell,
    /// The name an agent gave itself (the `app` of OSC 7501).
    Agent,
}

impl TitlePart {
    pub const ALL: [TitlePart; 5] =
        [TitlePart::Title, TitlePart::Folder, TitlePart::Profile, TitlePart::Shell, TitlePart::Agent];

    pub fn name(self) -> &'static str {
        match self {
            TitlePart::Title => "title",
            TitlePart::Folder => "folder",
            TitlePart::Profile => "profile",
            TitlePart::Shell => "shell",
            TitlePart::Agent => "agent",
        }
    }

    pub fn from_name(name: &str) -> Option<TitlePart> {
        TitlePart::ALL.into_iter().find(|part| part.name() == name)
    }
}

/// One slot of a title: the parts it can be filled by, `title|folder|profile` in the file. The slot says the first of
/// them that has something to say, so a tab with no program title falls back to its folder, and one without a folder
/// to its profile. There is nothing more to it: no conditions, no text of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleSlot(pub Vec<TitlePart>);

impl TitleSlot {
    /// The slot written as `text`; the first name that is not a part is the error.
    pub fn parse(text: &str) -> Result<TitleSlot, String> {
        let parts = text
            .split('|')
            .map(|name| TitlePart::from_name(name.trim()).ok_or_else(|| name.trim().to_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TitleSlot(parts))
    }
}

impl std::fmt::Display for TitleSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.0.iter().map(|part| part.name()).collect();
        f.write_str(&names.join("|"))
    }
}

impl Serialize for TitleSlot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// How the title of a tab is made when the user has not named it: its slots in order, with the empty ones left out and
/// the others joined by [`TITLE_SEPARATOR`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TabTitle {
    pub parts: Vec<TitleSlot>,
}

/// What stands between the slots of a title.
pub const TITLE_SEPARATOR: &str = " · ";

impl TabTitle {
    /// The slots as the settings drawer shows and takes them: `folder, title|profile`.
    pub fn to_text(&self) -> String {
        self.parts.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    }

    /// The reverse of [`TabTitle::to_text`]. `None` when a name is not a part or there is no slot at all.
    pub fn from_text(text: &str) -> Option<TabTitle> {
        let parts = text
            .split(',')
            .map(str::trim)
            .filter(|slot| !slot.is_empty())
            .map(|slot| TitleSlot::parse(slot).ok())
            .collect::<Option<Vec<_>>>()?;
        (!parts.is_empty()).then_some(TabTitle { parts })
    }
}

impl Default for TabTitle {
    /// What the program says, else the folder, else the profile.
    fn default() -> Self {
        TabTitle { parts: vec![TitleSlot(vec![TitlePart::Title, TitlePart::Folder, TitlePart::Profile])] }
    }
}

/// Where the tabs are: a row in the title bar, or a column at the left of the window as a list or as icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TabLayout {
    Top,
    List,
    Icons,
}

impl TabLayout {
    pub const ALL: [TabLayout; 3] = [TabLayout::Top, TabLayout::List, TabLayout::Icons];

    pub fn text(self) -> Text {
        match self {
            TabLayout::Top => Text::LayoutTop,
            TabLayout::List => Text::LayoutList,
            TabLayout::Icons => Text::LayoutIcons,
        }
    }

    /// The tabs stand in a sidebar.
    pub fn is_side(self) -> bool {
        self != TabLayout::Top
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tabs {
    pub title: TabTitle,
    pub layout: TabLayout,
    /// The sidebar is shown; it is hidden and shown again with the button of the title bar or Ctrl+B.
    pub sidebar_open: bool,
    /// The list of the sidebar puts the tabs that wait for the user first instead of keeping their order.
    pub group_by_state: bool,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs { title: TabTitle::default(), layout: TabLayout::Top, sidebar_open: true, group_by_state: false }
    }
}

/// Window backdrop shown behind (and through) the translucent terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backdrop {
    /// Real blur of whatever is behind the window.
    Acrylic,
    Mica,
    MicaAlt,
    /// Opaque.
    None,
}

impl Backdrop {
    /// The choices in the order every list of them shows (the quick flyout, the settings drawer).
    pub const ALL: [Backdrop; 4] = [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::None];

    /// Position in [`Backdrop::ALL`], the selected index of a list of the choices.
    pub fn index(self) -> i32 {
        Backdrop::ALL.iter().position(|choice| *choice == self).map_or(-1, |at| at as i32)
    }

    pub fn text(self) -> Text {
        match self {
            Backdrop::Acrylic => Text::BackdropAcrylic,
            Backdrop::Mica => Text::BackdropMica,
            Backdrop::MicaAlt => Text::BackdropMicaAlt,
            Backdrop::None => Text::BackdropNone,
        }
    }
}

/// Valid range of `appearance.opacity`: lower than this and the text becomes hard to read.
pub const OPACITY_RANGE: (f64, f64) = (0.2, 1.0);
/// Change made by one "increase / decrease background opacity" command.
pub const OPACITY_STEP: f64 = 0.05;
/// Valid range of `font.size`, also the bounds of the temporary font zoom.
pub const FONT_SIZE_RANGE: (f64, f64) = (6.0, 72.0);
/// The blink of the cursor, each phase, in milliseconds: from quick to slow.
pub const BLINK_RATE_RANGE: (f64, f64) = (200.0, 1500.0);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub backdrop: Backdrop,
    /// Opacity of the terminal background over the backdrop (1.0 = opaque).
    pub opacity: f64,
    pub tab_bar_density: TabBarDensity,
}

impl Appearance {
    /// The opacity after `steps` presses of the increase (positive) or decrease (negative) command,
    /// kept inside [`OPACITY_RANGE`] and free of floating point drift (0.72 + 0.05 is 0.77).
    pub fn opacity_after(&self, steps: i32) -> f64 {
        let moved = self.opacity + f64::from(steps) * OPACITY_STEP;
        ((moved * 100.0).round() / 100.0).clamp(OPACITY_RANGE.0, OPACITY_RANGE.1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyboard {
    pub kitty_protocol: bool,
}

/// What the status bar does for a pane that runs no agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoAgent {
    /// Keeps the folder and the branch.
    Place,
    Hide,
}

impl NoAgent {
    pub const ALL: [NoAgent; 2] = [NoAgent::Place, NoAgent::Hide];

    pub fn text(self) -> Text {
        match self {
            NoAgent::Place => Text::StatusBarPlace,
            NoAgent::Hide => Text::StatusBarHide,
        }
    }
}

/// The bar at the bottom of the window: what the agent of the focused pane is doing, and where the pane is.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusBar {
    pub show: bool,
    pub no_agent: NoAgent,
    pub state: bool,
    pub model: bool,
    pub context: bool,
    pub cost: bool,
    pub time: bool,
    pub branch: bool,
    pub folder: bool,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self { show: true, no_agent: NoAgent::Place, state: true, model: true, context: true, cost: true, time: false, branch: true, folder: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: Language,
    pub theme: String,
    /// Colour schemes of the user's own, in the JSON of Windows Terminal's; listed after the built-in ones.
    pub schemes: Vec<themes::Scheme>,
    pub font: Font,
    pub cursor: Cursor,
    pub appearance: Appearance,
    pub tabs: Tabs,
    pub scrollback: u32,
    /// Profile id; empty selects the first discovered profile.
    pub default_profile: String,
    pub profiles: Vec<CustomProfile>,
    pub snippets: Vec<Snippet>,
    pub copy_on_select: bool,
    /// While the view is scrolled up into a command's output, the line its command was typed on stays at the top.
    pub sticky_header: bool,
    pub right_click: RightClick,
    pub paste: PasteSettings,
    pub osc52: Osc52Policy,
    pub bell: Bell,
    pub keyboard: Keyboard,
    /// Sets environment variables that make coding-agent CLIs use their full terminal features.
    pub agent_environment: bool,
    pub status_bar: StatusBar,
    /// Makes PowerShell report its working directory and prompt marks.
    pub shell_integration: bool,
    /// Ask before closing a window, tab or pane that holds running sessions.
    pub confirm_close: bool,
    /// Number of running sessions a close must end before it asks; 1 asks whenever one is running.
    pub confirm_close_threshold: u32,
    /// Reopen the tabs and panes of the previous run on start.
    pub restore_session: bool,
    /// Explorer's right-click menu of a folder has "Open in Nekominal".
    pub explorer_menu: bool,
    /// A new pane takes the environment Nekominal was started with, rather than the one Windows gives a program started
    /// anew (what Windows Terminal does by default, and its `compatibility.reloadEnvironmentVariables` turns off).
    pub inherit_environment: bool,
    /// The welcome of the first start, where the layout is picked, was seen.
    pub welcomed: bool,
    /// Command id to shortcuts; an empty list unbinds a default.
    pub keybindings: BTreeMap<String, Vec<String>>,
}

impl Settings {
    /// Whether closing something that ends `running_sessions` sessions needs a confirmation.
    pub fn should_confirm_close(&self, running_sessions: u32) -> bool {
        self.confirm_close && running_sessions >= self.confirm_close_threshold.max(1)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Auto,
            theme: themes::DEFAULT_THEME.to_owned(),
            schemes: Vec::new(),
            font: Font {
                family: ["Cascadia Mono", "Consolas", "Microsoft JhengHei UI", "Segoe UI Emoji"]
                    .map(String::from)
                    .to_vec(),
                size: 14.0,
                weight: 400,
                line_height: 1.1,
            },
            // The pace Windows gives a text caret by default.
            cursor: Cursor { style: CursorStyle::Block, blink: true, blink_rate: 530 },
            appearance: Appearance { backdrop: Backdrop::Acrylic, opacity: 0.72, tab_bar_density: TabBarDensity::Comfortable },
            tabs: Tabs::default(),
            scrollback: 10_000,
            default_profile: String::new(),
            profiles: Vec::new(),
            snippets: Vec::new(),
            copy_on_select: false,
            sticky_header: true,
            right_click: RightClick::Smart,
            paste: PasteSettings { warn_multiline: true, warn_large_bytes: 5120 },
            osc52: Osc52Policy::Write,
            bell: Bell { visual: true, taskbar: true, sound: BellSound::None },
            keyboard: Keyboard { kitty_protocol: true },
            agent_environment: true,
            status_bar: StatusBar::default(),
            shell_integration: true,
            confirm_close: true,
            confirm_close_threshold: 1,
            restore_session: false,
            explorer_menu: true,
            inherit_environment: false,
            welcomed: false,
            keybindings: BTreeMap::new(),
        }
    }
}

/// Removes line and block comments outside of strings so the file can be annotated by hand.
pub fn strip_json_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match (chars[i], chars.get(i + 1)) {
            ('"', _) => {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                let end = (i + 1).min(chars.len());
                out.extend(&chars[start..end]);
                i = end;
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            (ch, _) => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

#[derive(Debug)]
pub struct Parsed {
    pub settings: Settings,
    /// What was wrong with the file; the affected values fall back to defaults.
    pub problems: Vec<Problem>,
}

/// What a value of `settings.json` should have been.
#[derive(Clone, Debug, PartialEq)]
pub enum Expected {
    Number { min: f64, max: f64 },
    Bool,
    Text,
    Object,
    Array,
    TextOrList,
    FontFamily,
    /// One of these names.
    OneOf(Vec<String>),
    Required,
    /// A colour such as #1a1b26.
    Color,
    /// A colour scheme that exists; this is the name that was not found.
    KnownTheme(String),
    KnownCommand,
    /// A shortcut no other binding of the user has; this is the command that has it.
    Clash(Command),
    /// A shortcut that can be read; this is the text that could not be.
    Shortcut(String),
    /// The whole file is not JSON; this is what the parser said.
    Parsable(String),
    JsonObject,
}

impl Expected {
    /// The sentence in `lang`: the string table has one for each kind.
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Expected::Number { min, max } => Text::ProblemNumber.format(lang, &[min, max]),
            Expected::Bool => Text::ProblemBool.get(lang).to_owned(),
            Expected::Text => Text::ProblemText.get(lang).to_owned(),
            Expected::Object => Text::ProblemObject.get(lang).to_owned(),
            Expected::Array => Text::ProblemArray.get(lang).to_owned(),
            Expected::TextOrList => Text::ProblemTextOrList.get(lang).to_owned(),
            Expected::FontFamily => Text::ProblemFontFamily.get(lang).to_owned(),
            Expected::OneOf(names) => {
                let separator = match lang {
                    Lang::ZhTw => "、",
                    Lang::EnUs => ", ",
                };
                Text::ProblemOneOf.format(lang, &[&names.join(separator)])
            }
            Expected::Required => Text::ProblemRequired.get(lang).to_owned(),
            Expected::Color => Text::ProblemColor.get(lang).to_owned(),
            Expected::KnownTheme(name) => Text::ProblemUnknownTheme.format(lang, &[name]),
            Expected::KnownCommand => Text::ProblemUnknownCommand.get(lang).to_owned(),
            Expected::Clash(owner) => Text::KeyConflict.format(lang, &[&owner.label(lang)]),
            Expected::Shortcut(text) => Text::ProblemBadShortcut.format(lang, &[text]),
            Expected::Parsable(error) => Text::ProblemUnparsable.format(lang, &[error]),
            Expected::JsonObject => Text::ProblemJsonObject.get(lang).to_owned(),
        }
    }
}

/// A value of `settings.json` that was not valid and was replaced by its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// The dotted key of the value (`font.size`, `profiles[0].program`); empty when the whole file is the problem.
    pub key: String,
    pub expected: Expected,
}

impl Problem {
    pub fn new(key: impl Into<String>, expected: Expected) -> Problem {
        Problem { key: key.into(), expected }
    }

    /// "font.size: Needs a number from 6 to 72", or the sentence alone when the whole file is the problem.
    pub fn text(&self, lang: Lang) -> String {
        let sentence = self.expected.text(lang);
        if self.key.is_empty() { sentence } else { Text::LabelValue.format(lang, &[&self.key, &sentence]) }
    }
}

#[derive(Default)]
struct Problems(Vec<Problem>);

impl Problems {
    fn fail(&mut self, path: &str, key: &str, expected: Expected) {
        let prefix = if path.is_empty() { String::new() } else { format!("{path}.") };
        self.0.push(Problem::new(format!("{prefix}{key}"), expected));
    }
}

fn number(source: &Map<String, Value>, key: &str, fallback: f64, (min, max): (f64, f64), path: &str, problems: &mut Problems) -> f64 {
    match source.get(key) {
        None => fallback,
        Some(value) => match value.as_f64() {
            Some(n) if n.is_finite() && n >= min && n <= max => n,
            _ => {
                problems.fail(path, key, Expected::Number { min, max });
                fallback
            }
        },
    }
}

fn boolean(source: &Map<String, Value>, key: &str, fallback: bool, path: &str, problems: &mut Problems) -> bool {
    match source.get(key) {
        None => fallback,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            problems.fail(path, key, Expected::Bool);
            fallback
        }
    }
}

fn text(source: &Map<String, Value>, key: &str, fallback: &str, path: &str, problems: &mut Problems) -> String {
    match source.get(key) {
        None => fallback.to_owned(),
        Some(Value::String(value)) => value.clone(),
        Some(_) => {
            problems.fail(path, key, Expected::Text);
            fallback.to_owned()
        }
    }
}

fn one_of<T: Copy>(source: &Map<String, Value>, key: &str, allowed: &[(&str, T)], fallback: T, path: &str, problems: &mut Problems) -> T {
    match source.get(key) {
        None => fallback,
        Some(value) => match value.as_str().and_then(|s| allowed.iter().find(|(name, _)| *name == s)) {
            Some((_, found)) => *found,
            None => {
                let names = allowed.iter().map(|(name, _)| (*name).to_owned()).collect();
                problems.fail(path, key, Expected::OneOf(names));
                fallback
            }
        },
    }
}

fn section(source: &Map<String, Value>, key: &str, path: &str, problems: &mut Problems) -> Map<String, Value> {
    match source.get(key) {
        None => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => {
            problems.fail(path, key, Expected::Object);
            Map::new()
        }
    }
}

/// The family names of a comma separated list such as `"Cascadia Mono", Consolas`, without quotes or blanks.
pub fn split_font_families(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The slots of `tabs.title.parts`: an array of strings such as `["folder", "title|profile"]`. A name that is not a
/// part, an entry that is not text, a value that is not an array and an empty array are each reported once, under the
/// key of the setting, and leave the default in effect.
fn title_slots(source: &Map<String, Value>, fallback: &[TitleSlot], problems: &mut Problems) -> Vec<TitleSlot> {
    let names = || TitlePart::ALL.iter().map(|part| part.name().to_owned()).collect();
    let read = |item: &Value| match item {
        Value::String(text) => TitleSlot::parse(text).map_err(|_| Expected::OneOf(names())),
        _ => Err(Expected::Text),
    };
    let found = match source.get("parts") {
        None => return fallback.to_vec(),
        Some(Value::Array(items)) if items.is_empty() => Err(Expected::OneOf(names())),
        Some(Value::Array(items)) => items.iter().map(read).collect::<Result<Vec<_>, _>>(),
        Some(_) => Err(Expected::Array),
    };
    found.unwrap_or_else(|expected| {
        problems.fail("tabs.title", "parts", expected);
        fallback.to_vec()
    })
}

/// Accepts an array of family names or a comma separated string such as `"Cascadia Mono", Consolas`.
fn font_families(source: &Map<String, Value>, fallback: &[String], problems: &mut Problems) -> Vec<String> {
    let parsed: Option<Vec<String>> = match source.get("family") {
        None => return fallback.to_vec(),
        Some(Value::Array(items)) => items.iter().map(|v| v.as_str().map(|s| s.trim().to_owned())).collect(),
        Some(Value::String(list)) => Some(split_font_families(list)),
        Some(_) => None,
    };
    match parsed.map(|names| names.into_iter().filter(|n| !n.is_empty()).collect::<Vec<_>>()) {
        Some(names) if !names.is_empty() => names,
        _ => {
            problems.fail("font", "family", Expected::FontFamily);
            fallback.to_vec()
        }
    }
}

fn read_profiles(source: &Map<String, Value>, problems: &mut Problems) -> Vec<CustomProfile> {
    let Some(value) = source.get("profiles") else { return Vec::new() };
    let Value::Array(entries) = value else {
        problems.fail("", "profiles", Expected::Array);
        return Vec::new();
    };
    let mut profiles = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("profiles[{index}]");
        let Value::Object(map) = entry else {
            problems.fail("", &path, Expected::Object);
            continue;
        };
        let program = text(map, "program", "", &path, problems);
        let given_id = text(map, "id", "", &path, problems);
        // Without a program an entry can only adjust a profile found on the machine, which its id names.
        if program.is_empty() && given_id.is_empty() {
            problems.fail(&path, "program", Expected::Required);
            continue;
        }
        // An entry that only adjusts a found shell keeps its name empty unless it gives one: a name equal to the id is a
        // rename like any other.
        let name = text(map, "name", if program.is_empty() { "" } else { &program }, &path, problems);
        // What cannot be read is reported and the rest of the entry kept, so that the user sees it before a save drops it.
        let args = match map.get("args") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    let arg = v.as_str().map(str::to_owned);
                    if arg.is_none() {
                        problems.fail(&path, &format!("args[{i}]"), Expected::Text);
                    }
                    arg
                })
                .collect(),
            Some(_) => {
                problems.fail(&path, "args", Expected::Array);
                Vec::new()
            }
        };
        let cwd = text(map, "cwd", "", &path, problems);
        // An empty id could never be picked (the empty default profile means "the first one").
        // Profiles of one name get ids of their own (`custom-Node`, `custom-Node-2`), or merge would keep only the first.
        let id = Some(given_id).filter(|id| !id.is_empty()).unwrap_or_else(|| {
            (1..)
                .map(|n| if n == 1 { format!("custom-{name}") } else { format!("custom-{name}-{n}") })
                .find(|id| !profiles.iter().any(|p: &CustomProfile| p.id == *id))
                .unwrap_or_default()
        });
        let env = match map.get("env") {
            Some(Value::Object(vars)) => vars
                .iter()
                .filter_map(|(k, v)| {
                    if !v.is_string() {
                        problems.fail(&format!("{path}.env"), k, Expected::Text);
                    }
                    Some((k.clone(), v.as_str()?.to_owned()))
                })
                .collect(),
            Some(_) => {
                problems.fail(&path, "env", Expected::Object);
                BTreeMap::new()
            }
            None => BTreeMap::new(),
        };
        let scheme = text(map, "colorScheme", "", &path, problems);
        // NaN stands for a size that was reported: the profile then follows the window's.
        let font_size = map.contains_key("fontSize").then(|| number(map, "fontSize", f64::NAN, FONT_SIZE_RANGE, &path, problems)).filter(|size| !size.is_nan());
        let elevate = boolean(map, "elevate", false, &path, problems);
        profiles.push(CustomProfile {
            id,
            name,
            program,
            args,
            cwd: (!cwd.is_empty()).then_some(cwd),
            options: ProfileOptions { env, color_scheme: (!scheme.is_empty()).then_some(scheme), font_size, elevate },
        });
    }
    profiles
}

fn read_snippets(source: &Map<String, Value>, problems: &mut Problems) -> Vec<Snippet> {
    let Some(value) = source.get("snippets") else { return Vec::new() };
    let Value::Array(entries) = value else {
        problems.fail("", "snippets", Expected::Array);
        return Vec::new();
    };
    let mut snippets = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("snippets[{index}]");
        let Value::Object(map) = entry else {
            problems.fail("", &path, Expected::Object);
            continue;
        };
        let input = text(map, "input", "", &path, problems);
        if input.is_empty() {
            problems.fail(&path, "input", Expected::Required);
            continue;
        }
        // A snippet without a name is called by what it types, up to its first line.
        let fallback: String = input.lines().next().unwrap_or_default().chars().take(40).collect();
        let name = text(map, "name", &fallback, &path, problems);
        snippets.push(Snippet { name, input });
    }
    snippets
}

/// The `schemes` of a document. One with a missing or bad colour is reported and left out; one with the name of an
/// earlier one replaces it, as a scheme copied in twice is one the user changed.
fn read_schemes(source: &Map<String, Value>, problems: &mut Problems) -> Vec<themes::Scheme> {
    let Some(value) = source.get("schemes") else { return Vec::new() };
    let Value::Array(entries) = value else {
        problems.fail("", "schemes", Expected::Array);
        return Vec::new();
    };
    let mut schemes: Vec<themes::Scheme> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("schemes[{index}]");
        let Value::Object(map) = entry else {
            problems.fail("", &path, Expected::Object);
            continue;
        };
        match themes::Scheme::from_json(map) {
            Ok(scheme) => match schemes.iter_mut().find(|s| s.name == scheme.name) {
                Some(earlier) => *earlier = scheme,
                None => schemes.push(scheme),
            },
            Err((key, expected)) => problems.fail(&path, &key, expected),
        }
    }
    schemes
}

/// Removes the commas before a `}` or a `]`, outside of strings: Windows Terminal writes its file with them.
fn strip_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            if ch == '\\' {
                out.extend(chars.get(i..i + 2).unwrap_or(&chars[i..]));
                i += 2;
                continue;
            }
            in_string = ch != '"';
        } else if ch == '"' {
            in_string = true;
        } else if ch == ',' && matches!(chars[i + 1..].iter().find(|c| !c.is_whitespace()), Some('}' | ']')) {
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Adds to `settings` the `schemes` of a Windows Terminal `settings.json` (comments and trailing commas allowed) that
/// it has no scheme of by name, built-in or its own. A scheme that cannot be read is left out. The names added, or
/// what the parser said of a file that is not JSON.
pub fn import_schemes(settings: &mut Settings, windows_terminal: &str) -> Result<Vec<String>, String> {
    let text = strip_trailing_commas(&strip_json_comments(windows_terminal.trim_start_matches('\u{feff}')));
    let document: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    // Only the log reads the error, so it is not translated.
    let Value::Object(root) = document else { return Err("not a JSON object".to_owned()) };
    let mut added = Vec::new();
    for scheme in read_schemes(&root, &mut Problems::default()) {
        if !themes::is_known(&settings.schemes, &scheme.name) {
            added.push(scheme.name.to_string());
            settings.schemes.push(scheme);
        }
    }
    Ok(added)
}

/// Adds the scheme of an iTerm2 preset (`.itermcolors`) named `name` to `settings`, in place of a scheme of their own with
/// that name: importing a preset again takes its newer colours. What is wrong with the file, when it is no preset.
pub fn import_itermcolors(settings: &mut Settings, name: &str, plist: &str) -> Result<(), String> {
    let scheme = themes::Scheme::from_itermcolors(name, plist)?;
    match settings.schemes.iter_mut().find(|own| own.name == scheme.name) {
        Some(own) => *own = scheme,
        None => settings.schemes.push(scheme),
    }
    Ok(())
}

fn read_keybindings(source: &Map<String, Value>, problems: &mut Problems) -> BTreeMap<String, Vec<String>> {
    let Some(value) = source.get("keybindings") else { return BTreeMap::new() };
    let Value::Object(map) = value else {
        problems.fail("", "keybindings", Expected::Object);
        return BTreeMap::new();
    };
    let mut bindings = BTreeMap::new();
    for (id, shortcut) in map {
        match shortcut {
            // An empty string unbinds the command.
            Value::String(s) if s.is_empty() => {
                bindings.insert(id.clone(), Vec::new());
            }
            Value::String(s) => {
                bindings.insert(id.clone(), vec![s.clone()]);
            }
            Value::Array(items) if items.iter().all(Value::is_string) => {
                bindings.insert(id.clone(), items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
            }
            _ => problems.fail("keybindings", id, Expected::TextOrList),
        }
    }
    bindings
}

/// Parses the contents of `settings.json`. `None` or blank text yields the defaults.
pub fn parse(source: Option<&str>) -> Parsed {
    let defaults = Settings::default();
    // Some editors still write a byte order mark, which JSON parsers reject.
    let Some(source) = source.map(|s| s.trim_start_matches('\u{feff}')).filter(|s| !s.trim().is_empty()) else {
        return Parsed { settings: defaults, problems: Vec::new() };
    };
    let document: Value = match serde_json::from_str(&strip_json_comments(source)) {
        Ok(value) => value,
        Err(error) => {
            return Parsed { settings: defaults, problems: vec![Problem::new("", Expected::Parsable(error.to_string()))] };
        }
    };
    let Value::Object(root) = document else {
        return Parsed { settings: defaults, problems: vec![Problem::new("", Expected::JsonObject)] };
    };

    let mut problems = Problems::default();
    let d = &defaults;

    let font = section(&root, "font", "", &mut problems);
    let cursor = section(&root, "cursor", "", &mut problems);
    let appearance = section(&root, "appearance", "", &mut problems);
    let paste = section(&root, "paste", "", &mut problems);
    let bell = section(&root, "bell", "", &mut problems);
    let keyboard = section(&root, "keyboard", "", &mut problems);
    let status_bar = section(&root, "statusBar", "", &mut problems);
    let tabs = section(&root, "tabs", "", &mut problems);
    let tab_title = section(&tabs, "title", "tabs", &mut problems);

    let theme_name = text(&root, "theme", &d.theme, "", &mut problems);
    let schemes = read_schemes(&root, &mut problems);
    let theme = if themes::is_known(&schemes, &theme_name) {
        theme_name
    } else {
        problems.fail("", "theme", Expected::KnownTheme(theme_name));
        d.theme.clone()
    };

    let settings = Settings {
        language: one_of(
            &root,
            "language",
            &[("auto", Language::Auto), ("zh-TW", Language::ZhTw), ("en-US", Language::EnUs)],
            d.language,
            "",
            &mut problems,
        ),
        theme,
        schemes,
        font: Font {
            family: font_families(&font, &d.font.family, &mut problems),
            size: number(&font, "size", d.font.size, FONT_SIZE_RANGE, "font", &mut problems),
            weight: number(&font, "weight", d.font.weight as f64, (100.0, 900.0), "font", &mut problems) as u16,
            line_height: number(&font, "lineHeight", d.font.line_height, (1.0, 2.0), "font", &mut problems),
        },
        cursor: Cursor {
            style: one_of(
                &cursor,
                "style",
                &[("block", CursorStyle::Block), ("bar", CursorStyle::Bar), ("underline", CursorStyle::Underline)],
                d.cursor.style,
                "cursor",
                &mut problems,
            ),
            blink: boolean(&cursor, "blink", d.cursor.blink, "cursor", &mut problems),
            blink_rate: number(&cursor, "blinkRate", f64::from(d.cursor.blink_rate), BLINK_RATE_RANGE, "cursor", &mut problems).round() as u32,
        },
        appearance: Appearance {
            backdrop: one_of(
                &appearance,
                "backdrop",
                &[("acrylic", Backdrop::Acrylic), ("mica", Backdrop::Mica), ("mica-alt", Backdrop::MicaAlt), ("none", Backdrop::None)],
                d.appearance.backdrop,
                "appearance",
                &mut problems,
            ),
            opacity: number(&appearance, "opacity", d.appearance.opacity, OPACITY_RANGE, "appearance", &mut problems),
            tab_bar_density: one_of(
                &appearance,
                "tabBarDensity",
                &[("comfortable", TabBarDensity::Comfortable), ("compact", TabBarDensity::Compact)],
                d.appearance.tab_bar_density,
                "appearance",
                &mut problems,
            ),
        },
        tabs: Tabs {
            title: TabTitle { parts: title_slots(&tab_title, &d.tabs.title.parts, &mut problems) },
            layout: one_of(&tabs, "layout", &[("top", TabLayout::Top), ("list", TabLayout::List), ("icons", TabLayout::Icons)], d.tabs.layout, "tabs", &mut problems),
            sidebar_open: boolean(&tabs, "sidebarOpen", d.tabs.sidebar_open, "tabs", &mut problems),
            group_by_state: boolean(&tabs, "groupByState", d.tabs.group_by_state, "tabs", &mut problems),
        },
        scrollback: number(&root, "scrollback", d.scrollback as f64, (0.0, 1_000_000.0), "", &mut problems) as u32,
        default_profile: text(&root, "defaultProfile", &d.default_profile, "", &mut problems),
        profiles: read_profiles(&root, &mut problems),
        snippets: read_snippets(&root, &mut problems),
        copy_on_select: boolean(&root, "copyOnSelect", d.copy_on_select, "", &mut problems),
        sticky_header: boolean(&root, "stickyHeader", d.sticky_header, "", &mut problems),
        right_click: one_of(
            &root,
            "rightClick",
            &[("smart", RightClick::Smart), ("menu", RightClick::Menu)],
            d.right_click,
            "",
            &mut problems,
        ),
        paste: PasteSettings {
            warn_multiline: boolean(&paste, "warnMultiline", d.paste.warn_multiline, "paste", &mut problems),
            warn_large_bytes: number(&paste, "warnLargeBytes", d.paste.warn_large_bytes as f64, (0.0, 100_000_000.0), "paste", &mut problems) as u64,
        },
        osc52: one_of(
            &root,
            "osc52",
            &[("off", Osc52Policy::Off), ("write", Osc52Policy::Write), ("read-write", Osc52Policy::ReadWrite)],
            d.osc52,
            "",
            &mut problems,
        ),
        bell: Bell {
            visual: boolean(&bell, "visual", d.bell.visual, "bell", &mut problems),
            taskbar: boolean(&bell, "taskbar", d.bell.taskbar, "bell", &mut problems),
            sound: one_of(
                &bell,
                "sound",
                &[("none", BellSound::None), ("background", BellSound::Background), ("always", BellSound::Always)],
                d.bell.sound,
                "bell",
                &mut problems,
            ),
        },
        keyboard: Keyboard { kitty_protocol: boolean(&keyboard, "kittyProtocol", d.keyboard.kitty_protocol, "keyboard", &mut problems) },
        agent_environment: boolean(&root, "agentEnvironment", d.agent_environment, "", &mut problems),
        status_bar: {
            let b = &d.status_bar;
            let flag = |key: &str, default: bool, problems: &mut Problems| boolean(&status_bar, key, default, "statusBar", problems);
            StatusBar {
                show: flag("show", b.show, &mut problems),
                no_agent: one_of(&status_bar, "noAgent", &[("place", NoAgent::Place), ("hide", NoAgent::Hide)], b.no_agent, "statusBar", &mut problems),
                state: flag("state", b.state, &mut problems),
                model: flag("model", b.model, &mut problems),
                context: flag("context", b.context, &mut problems),
                cost: flag("cost", b.cost, &mut problems),
                time: flag("time", b.time, &mut problems),
                branch: flag("branch", b.branch, &mut problems),
                folder: flag("folder", b.folder, &mut problems),
            }
        },
        shell_integration: boolean(&root, "shellIntegration", d.shell_integration, "", &mut problems),
        confirm_close: boolean(&root, "confirmClose", d.confirm_close, "", &mut problems),
        confirm_close_threshold: number(&root, "confirmCloseThreshold", d.confirm_close_threshold as f64, (1.0, 1000.0), "", &mut problems) as u32,
        restore_session: boolean(&root, "restoreSession", d.restore_session, "", &mut problems),
        explorer_menu: boolean(&root, "explorerMenu", d.explorer_menu, "", &mut problems),
        inherit_environment: boolean(&root, "inheritEnvironment", d.inherit_environment, "", &mut problems),
        welcomed: boolean(&root, "welcomed", d.welcomed, "", &mut problems),
        keybindings: read_keybindings(&root, &mut problems),
    };
    Parsed { settings, problems: problems.0 }
}

fn diff(value: &Value, base: &Value) -> Option<Value> {
    match (value, base) {
        (Value::Object(value), Value::Object(base)) => {
            let changed: Map<String, Value> = value
                .iter()
                .filter_map(|(key, entry)| {
                    let delta = match base.get(key) {
                        Some(base_entry) => diff(entry, base_entry),
                        None => Some(entry.clone()),
                    };
                    delta.map(|d| (key.clone(), d))
                })
                .collect();
            (!changed.is_empty()).then_some(Value::Object(changed))
        }
        _ => (value != base).then(|| value.clone()),
    }
}

/// Writes whole numbers without a fractional part (`16`, not `16.0`).
fn tidy_numbers(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9e15 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(tidy_numbers).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, tidy_numbers(v))).collect()),
        other => other,
    }
}

/// Serializes only what differs from the defaults, so future default changes still apply.
///
/// The file is written from the settings, not edited: the comments of a hand-written `settings.json` and the keys this
/// version does not know cannot survive a rewrite. `store::save_keeping_original` keeps the first original for that.
pub fn serialize(settings: &Settings) -> String {
    let value = serde_json::to_value(settings).expect("settings serialize");
    let base = serde_json::to_value(Settings::default()).expect("defaults serialize");
    let changed = diff(&value, &base).map(tidy_numbers).unwrap_or_else(|| Value::Object(Map::new()));
    format!("{}\n", serde_json::to_string_pretty(&changed).expect("settings serialize"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Parsed {
        parse(Some(source))
    }

    #[test]
    fn strips_comments_but_keeps_strings() {
        let stripped = strip_json_comments("{ // note\n \"url\": \"http://x//y\", /* block */ \"n\": 1 }");
        let value: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["url"], "http://x//y");
        assert_eq!(value["n"], 1);
    }

    #[test]
    fn handles_escaped_quotes_and_unterminated_comments() {
        let value: Value = serde_json::from_str(&strip_json_comments(r#"{"a": "say \"// hi\""}"#)).unwrap();
        assert_eq!(value["a"], "say \"// hi\"");
        assert_eq!(strip_json_comments("{} /* never closed").trim(), "{}");
    }

    #[test]
    fn missing_or_blank_files_give_the_defaults() {
        assert_eq!(parse(None).settings, Settings::default());
        assert_eq!(parse(Some("  \n")).settings, Settings::default());
        assert!(parse(None).problems.is_empty());
    }

    #[test]
    fn valid_values_override_the_defaults() {
        let result = parsed(r#"{ "theme": "Dracula", "font": { "size": 18 }, "cursor": { "style": "bar" }, "scrollback": 500 }"#);
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        let s = result.settings;
        assert_eq!(s.theme, "Dracula");
        assert_eq!(s.font.size, 18.0);
        assert_eq!(s.font.family, Settings::default().font.family);
        assert_eq!(s.cursor, Cursor { style: CursorStyle::Bar, blink: true, blink_rate: 530 });
        assert_eq!(s.scrollback, 500);
    }

    #[test]
    fn invalid_values_fall_back_and_are_reported_individually() {
        let result = parsed(r#"{ "font": { "size": 400 }, "cursor": { "style": "star" }, "theme": "Nope", "copyOnSelect": "yes" }"#);
        let s = &result.settings;
        assert_eq!(s.font.size, Settings::default().font.size);
        assert_eq!(s.cursor.style, CursorStyle::Block);
        assert_eq!(s.theme, Settings::default().theme);
        assert!(!s.copy_on_select);
        assert_eq!(result.problems.len(), 4);
        let keys: Vec<&str> = result.problems.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["theme", "font.size", "cursor.style", "copyOnSelect"]);
    }

    #[test]
    fn a_problem_is_data_that_each_language_says_in_its_own_words() {
        let result = parsed(r#"{ "font": { "size": 400 }, "cursor": { "style": "star" }, "theme": "Nope", "copyOnSelect": "yes", "appearance": 3, "profiles": [ { "name": "x" } ] }"#);
        let says = |lang| result.problems.iter().map(|p| p.text(lang)).collect::<Vec<_>>();
        assert_eq!(
            says(Lang::ZhTw),
            [
                "appearance：需要物件",
                "theme：找不到「Nope」",
                "font.size：需要 6 到 72 的數字",
                "cursor.style：需要 block、bar、underline 其中之一",
                "profiles[0].program：必填",
                "copyOnSelect：需要 true 或 false",
            ]
        );
        assert_eq!(
            says(Lang::EnUs),
            [
                "appearance: Needs an object",
                "theme: No color scheme \"Nope\"",
                "font.size: Needs a number from 6 to 72",
                "cursor.style: Needs one of block, bar, underline",
                "profiles[0].program: Required",
                "copyOnSelect: Needs true or false",
            ]
        );
        assert!(says(Lang::EnUs).iter().all(|text| text.chars().all(|c| !('\u{2E80}'..='\u{9FFF}').contains(&c))), "no Chinese in English");
        // A file that is not JSON, or not an object, is one problem about the whole file: no key.
        let broken = parsed(r#"{ "theme": "#);
        assert_eq!(broken.problems.len(), 1);
        assert_eq!(broken.problems[0].key, "");
        assert!(broken.problems[0].text(Lang::EnUs).starts_with("Cannot parse: "));
        assert_eq!(parsed("[1]").problems[0].text(Lang::ZhTw), "需要是 JSON 物件");
    }

    #[test]
    fn broken_json_never_panics() {
        let result = parsed(r#"{ "theme": "#);
        assert_eq!(result.settings, Settings::default());
        assert_eq!(result.problems.len(), 1);
    }

    #[test]
    fn a_non_object_document_is_rejected() {
        assert_eq!(parsed("[1, 2]").problems.len(), 1);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let result = parsed(r#"{ "somethingNew": true }"#);
        assert_eq!(result.settings, Settings::default());
        assert!(result.problems.is_empty());
    }

    #[test]
    fn font_family_accepts_arrays_and_css_style_lists() {
        let list = parsed(r#"{ "font": { "family": "\"Cascadia Code\", Consolas, 'Microsoft JhengHei UI'" } }"#);
        assert_eq!(list.settings.font.family, vec!["Cascadia Code", "Consolas", "Microsoft JhengHei UI"]);
        let array = parsed(r#"{ "font": { "family": ["A", " B "] } }"#);
        assert_eq!(array.settings.font.family, vec!["A", "B"]);
        let bad = parsed(r#"{ "font": { "family": 3 } }"#);
        assert_eq!(bad.problems.len(), 1);
        assert_eq!(bad.settings.font.family, Settings::default().font.family);
    }

    #[test]
    fn custom_profiles_are_read_and_entries_without_a_program_are_skipped() {
        let result = parsed(r#"{ "profiles": [ { "id": "node", "name": "Node", "program": "node.exe", "args": ["-i"], "cwd": "C:\\work" }, { "name": "broken" } ] }"#);
        assert_eq!(
            result.settings.profiles,
            vec![CustomProfile {
                id: "node".into(),
                name: "Node".into(),
                program: "node.exe".into(),
                args: vec!["-i".into()],
                cwd: Some("C:\\work".into()),
                options: Default::default(),
            }]
        );
        assert_eq!(result.problems.len(), 1);
    }

    #[test]
    fn keybindings_accept_strings_arrays_and_empty_strings() {
        let result = parsed(r#"{ "keybindings": { "tab.new": "Ctrl+T", "paste": ["Ctrl+V", "Shift+Insert"], "pane.close": "", "x": 3 } }"#);
        let keys = &result.settings.keybindings;
        assert_eq!(keys["tab.new"], vec!["Ctrl+T"]);
        assert_eq!(keys["paste"], vec!["Ctrl+V", "Shift+Insert"]);
        assert!(keys["pane.close"].is_empty());
        assert!(!keys.contains_key("x"));
        assert_eq!(result.problems.len(), 1);
    }

    #[test]
    fn appearance_defaults_to_translucent_acrylic_and_accepts_every_backdrop() {
        let defaults = Settings::default().appearance;
        assert_eq!((defaults.backdrop, defaults.opacity), (Backdrop::Acrylic, 0.72));
        for (name, expected) in [("acrylic", Backdrop::Acrylic), ("mica", Backdrop::Mica), ("mica-alt", Backdrop::MicaAlt), ("none", Backdrop::None)] {
            let result = parsed(&format!(r#"{{ "appearance": {{ "backdrop": "{name}", "opacity": 0.5 }} }}"#));
            assert!(result.problems.is_empty(), "{name}: {:?}", result.problems);
            assert_eq!(result.settings.appearance, Appearance { backdrop: expected, opacity: 0.5, ..Settings::default().appearance });
        }
    }

    #[test]
    fn invalid_appearance_values_fall_back_and_are_reported() {
        let result = parsed(r#"{ "appearance": { "backdrop": "glass", "opacity": 5 } }"#);
        assert_eq!(result.settings.appearance, Settings::default().appearance);
        assert_eq!(result.problems.len(), 2);
    }

    #[test]
    fn appearance_changes_round_trip() {
        let changed = Settings { appearance: Appearance { backdrop: Backdrop::MicaAlt, opacity: 0.9, ..Settings::default().appearance }, ..Settings::default() };
        assert_eq!(parse(Some(&serialize(&changed))).settings, changed);
    }

    #[test]
    fn the_defaults_serialize_to_an_empty_object() {
        assert_eq!(serialize(&Settings::default()), "{}\n");
    }

    #[test]
    fn only_changed_values_are_written_and_they_round_trip() {
        let mut changed = Settings { theme: "Nord".into(), font: Font { size: 16.0, ..Settings::default().font }, ..Settings::default() };
        changed.keybindings.insert("paste".into(), Vec::new());
        let written = serialize(&changed);
        let value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value, serde_json::json!({ "theme": "Nord", "font": { "size": 16 }, "keybindings": { "paste": [] } }));
        assert_eq!(parse(Some(&written)).settings, changed);
    }

    #[test]
    fn fractional_sizes_and_profiles_round_trip() {
        let mut changed = Settings::default();
        changed.font.size = 13.5;
        changed.font.family = vec!["Fira Code".into()];
        changed.profiles.push(CustomProfile { id: "a".into(), name: "A".into(), program: "a.exe".into(), args: vec![], cwd: None, options: Default::default() });
        let written = serialize(&changed);
        assert_eq!(parse(Some(&written)).settings, changed);
    }

    /// Parses `{ <entry> }`, checks that nothing was reported, and returns the settings.
    fn parsed_ok(entry: &str) -> Settings {
        let result = parsed(&format!("{{ {entry} }}"));
        assert!(result.problems.is_empty(), "{entry}: {:?}", result.problems);
        result.settings
    }

    /// Parses `{ <entry> }` holding a bad value and checks that it was reported once and replaced by the default.
    fn rejected(entry: &str) {
        let result = parsed(&format!("{{ {entry} }}"));
        assert_eq!(result.problems.len(), 1, "{entry}: {:?}", result.problems);
        assert_eq!(result.settings, Settings::default(), "{entry}");
    }

    fn round_trips(changed: Settings) -> String {
        let written = serialize(&changed);
        assert_eq!(parse(Some(&written)).settings, changed, "{written}");
        written
    }

    #[test]
    fn language_defaults_to_auto_and_accepts_every_choice() {
        assert_eq!(Settings::default().language, Language::Auto);
        for (name, expected) in [("auto", Language::Auto), ("zh-TW", Language::ZhTw), ("en-US", Language::EnUs)] {
            assert_eq!(parsed_ok(&format!(r#""language": "{name}""#)).language, expected);
        }
        rejected(r#""language": "fr-FR""#);
        rejected(r#""language": 1"#);
        let written = round_trips(Settings { language: Language::ZhTw, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "language": "zh-TW" }));
    }

    #[test]
    fn language_choices_are_written_as_the_language_tags_the_strings_use() {
        for (choice, lang) in [(Language::ZhTw, Lang::ZhTw), (Language::EnUs, Lang::EnUs)] {
            assert_eq!(serde_json::to_value(choice).unwrap(), Value::String(lang.tag().to_owned()));
            assert_eq!(choice.resolve("fr-FR"), lang);
        }
    }

    #[test]
    fn the_language_setting_overrides_the_system_unless_it_is_auto() {
        assert_eq!(Language::Auto.resolve("zh-Hant-TW"), Lang::ZhTw);
        assert_eq!(Language::Auto.resolve("ja-JP"), Lang::EnUs);
        assert_eq!(Language::ZhTw.resolve("en-US"), Lang::ZhTw);
        assert_eq!(Language::EnUs.resolve("zh-TW"), Lang::EnUs);
    }

    #[test]
    fn a_profile_carries_its_own_environment_scheme_font_size_and_elevation() {
        let result = parsed(r#"{ "profiles": [
            { "id": "pwsh-admin", "name": "Admin", "program": "pwsh.exe", "env": { "LANG": "zh_TW", "BAD": 1 }, "colorScheme": "Nord", "fontSize": 14, "elevate": true },
            { "id": "cmd", "cwd": "D:\\" },
            { "name": "nothing" }
        ] }"#);
        let admin = &result.settings.profiles[0];
        assert_eq!(admin.options.env.get("LANG").map(String::as_str), Some("zh_TW"));
        assert!(!admin.options.env.contains_key("BAD"), "a value that is not text is left out, reported");
        assert_eq!((admin.options.color_scheme.as_deref(), admin.options.font_size, admin.options.elevate), (Some("Nord"), Some(14.0), true));
        assert_eq!((result.settings.profiles[1].program.as_str(), result.settings.profiles[1].cwd.as_deref()), ("", Some("D:\\")), "an id alone adjusts a profile found on the machine");
        assert_eq!(result.settings.profiles.len(), 2, "no program and no id: nothing to run, reported");
        assert_eq!(result.problems.len(), 2, "{:?}", result.problems);
        let written = round_trips(Settings { profiles: vec![admin.clone()], ..Settings::default() });
        let value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["profiles"][0]["colorScheme"], "Nord");
        assert_eq!(value["profiles"][0]["elevate"], true);
    }

    #[test]
    fn snippets_are_read_with_their_names_and_written_back() {
        let result = parsed(r#"{ "snippets": [ { "name": "status", "input": "git status\r" }, { "input": "cargo test -p nekominal-shell" }, { "name": "empty" } ] }"#);
        assert_eq!(result.problems.len(), 1, "the one without input is reported: {:?}", result.problems);
        let names: Vec<&str> = result.settings.snippets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["status", "cargo test -p nekominal-shell"], "a snippet without a name is called by what it types");
        assert_eq!(result.settings.snippets[0].input, "git status\r");
        let written = round_trips(Settings { snippets: result.settings.snippets.clone(), ..Settings::default() });
        assert!(written.contains("git status\\r"), "{written}");
    }

    /// A scheme as Windows Terminal writes one, with the name and the background given.
    fn wt_scheme(name: &str, background: &str) -> String {
        format!(
            r##"{{ "name": "{name}", "background": "{background}", "foreground": "#c0caf5", "cursorColor": "#ffffff", "selectionBackground": "#33467c",
                "black": "#15161e", "red": "#f7768e", "green": "#9ece6a", "yellow": "#e0af68", "blue": "#7aa2f7", "purple": "#bb9af7",
                "cyan": "#7dcfff", "white": "#a9b1d6", "brightBlack": "#414868", "brightRed": "#f7768e", "brightGreen": "#9ece6a",
                "brightYellow": "#e0af68", "brightBlue": "#7aa2f7", "brightPurple": "#bb9af7", "brightCyan": "#7dcfff", "brightWhite": "#c0caf5" }}"##
        )
    }

    #[test]
    fn schemes_in_the_json_of_windows_terminal_are_read_and_chosen_by_name() {
        let s = parsed_ok(&format!(r#""theme": "Mine", "schemes": [{}]"#, wt_scheme("Mine", "#123")));
        assert_eq!(s.theme, "Mine", "a scheme of the file is a theme the file can choose");
        let mine = themes::find(&s.schemes, "Mine");
        assert_eq!(mine.background, themes::Color::rgb(0x112233), "#RGB is read as #RRGGBB");
        assert_eq!((mine.cursor, mine.ansi[5], mine.ansi[15]), (themes::Color::rgb(0xffffff), themes::Color::rgb(0xbb9af7), themes::Color::rgb(0xc0caf5)));
        // `magenta` stands for `purple`, and the cursor and the selection may be left out.
        let loose = wt_scheme("Loose", "#000000").replace(r#""purple""#, r#""magenta""#).replace(r##""cursorColor": "#ffffff", "selectionBackground": "#33467c","##, "");
        let s = parsed_ok(&format!(r#""schemes": [{loose}]"#));
        assert_eq!((s.schemes[0].ansi[5], s.schemes[0].cursor), (themes::Color::rgb(0xbb9af7), themes::Color::rgb(0xc0caf5)));
    }

    #[test]
    fn a_scheme_with_a_missing_or_bad_colour_is_reported_and_left_out() {
        let bad = wt_scheme("Bad", "blue");
        let missing = wt_scheme("Missing", "#000000").replace(r##""brightWhite": "#c0caf5""##, r#""extra": 1"#);
        let nameless = wt_scheme("", "#000000");
        let result = parsed(&format!(r#"{{ "theme": "Bad", "schemes": [{bad}, {missing}, {nameless}, 3, {}] }}"#, wt_scheme("Good", "#000000")));
        let names: Vec<&str> = result.settings.schemes.iter().map(|s| &*s.name).collect();
        assert_eq!(names, ["Good"]);
        assert_eq!(result.settings.theme, Settings::default().theme, "a theme naming a scheme left out falls back");
        let problems: Vec<(&str, &Expected)> = result.problems.iter().map(|p| (p.key.as_str(), &p.expected)).collect();
        assert_eq!(
            problems,
            [
                ("schemes[0].background", &Expected::Color),
                ("schemes[1].brightWhite", &Expected::Required),
                ("schemes[2].name", &Expected::Required),
                ("schemes[3]", &Expected::Object),
                ("theme", &Expected::KnownTheme("Bad".into())),
            ]
        );
        assert_eq!(result.problems[0].text(Lang::EnUs), "schemes[0].background: Needs a color such as #RRGGBB");
    }

    #[test]
    fn a_scheme_with_a_built_in_name_replaces_the_built_in_one() {
        let s = parsed_ok(&format!(r#""theme": "Nord", "schemes": [{}]"#, wt_scheme("Nord", "#010203")));
        assert_eq!(themes::find(&s.schemes, &s.theme).background, themes::Color::rgb(0x010203));
        assert_eq!(themes::theme_names(&s.schemes).filter(|name| *name == "Nord").count(), 1);
    }

    #[test]
    fn schemes_are_written_back_with_the_keys_of_windows_terminal_and_read_back() {
        let s = parsed_ok(&format!(r#""schemes": [{}, {}]"#, wt_scheme("One", "#101010"), wt_scheme("Two", "#202020")));
        let written = round_trips(s);
        let value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["schemes"][1]["name"], "Two");
        assert_eq!(value["schemes"][0]["background"], "#101010");
        assert_eq!(value["schemes"][0]["brightPurple"], "#bb9af7");
        assert_eq!(value["schemes"][0]["selectionBackground"], "#33467c");
        assert!(!serialize(&Settings::default()).contains("schemes"), "no schemes of one's own, nothing written");
    }

    #[test]
    fn importing_from_windows_terminal_adds_the_schemes_not_there_by_name_and_reads_its_comments_and_trailing_commas() {
        let mut settings = parsed_ok(&format!(r#""schemes": [{}]"#, wt_scheme("Mine", "#101010")));
        let windows_terminal = format!(
            "\u{feff}{{\n  // Windows Terminal's own file\n  \"profiles\": {{ \"list\": [], }},\n  \"schemes\": [ {}, {}, {}, {}, /* a bad one */ {}, ],\n}}",
            wt_scheme("Mine", "#ffffff"),
            wt_scheme("Campbell", "#ffffff"),
            wt_scheme("New \\\"q\\\", ]", "#303030"),
            wt_scheme("Other", "#404040"),
            wt_scheme("Broken", "nope"),
        );
        let added = import_schemes(&mut settings, &windows_terminal).expect("the file is read");
        assert_eq!(added, ["New \"q\", ]", "Other"], "what is there by name, built-in or one's own, is kept");
        assert_eq!(themes::find(&settings.schemes, "Mine").background, themes::Color::rgb(0x101010));
        assert_eq!(import_schemes(&mut settings, &windows_terminal), Ok(Vec::new()), "a second import adds nothing");
        assert_eq!(settings.schemes.len(), 3);
        assert!(import_schemes(&mut settings, "{ not json").is_err());
        assert_eq!(import_schemes(&mut settings, "{}"), Ok(Vec::new()), "a file without schemes adds none");
    }

    #[test]
    fn the_explorer_menu_is_on_unless_turned_off_and_written_back() {
        assert!(Settings::default().explorer_menu);
        let written = round_trips(Settings { explorer_menu: false, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "explorerMenu": false }));
    }

    #[test]
    fn the_command_line_stays_pinned_while_scrolled_unless_turned_off() {
        assert!(Settings::default().sticky_header);
        assert!(!parsed_ok(r#""stickyHeader": false"#).sticky_header);
        let written = round_trips(Settings { sticky_header: false, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "stickyHeader": false }));
    }

    #[test]
    fn a_pane_takes_a_fresh_environment_unless_asked_to_inherit() {
        assert!(!Settings::default().inherit_environment);
        let written = round_trips(Settings { inherit_environment: true, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "inheritEnvironment": true }));
    }

    #[test]
    fn restoring_the_session_is_opt_in() {
        assert!(!Settings::default().restore_session);
        assert!(parsed_ok(r#""restoreSession": true"#).restore_session);
        rejected(r#""restoreSession": "yes""#);
        let written = round_trips(Settings { restore_session: true, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "restoreSession": true }));
    }

    #[test]
    fn the_blink_rate_of_the_cursor_is_kept_in_its_range_and_written_back() {
        assert_eq!(Settings::default().cursor.blink_rate, 530);
        assert_eq!(parsed_ok(r#""cursor": { "blinkRate": 800 }"#).cursor.blink_rate, 800);
        let quick = parsed(r#"{ "cursor": { "blinkRate": 5 } }"#);
        assert_eq!(quick.settings.cursor.blink_rate, 530, "out of range falls back to the default, as every number of the file does");
        assert_eq!(quick.problems.len(), 1, "and is reported");
        let written = round_trips(Settings { cursor: Cursor { blink_rate: 900, ..Settings::default().cursor }, ..Settings::default() });
        assert!(written.contains("\"blinkRate\": 900"), "{written}");
    }

    #[test]
    fn the_close_threshold_defaults_to_any_running_session_and_is_validated() {
        assert_eq!(Settings::default().confirm_close_threshold, 1);
        assert_eq!(parsed_ok(r#""confirmCloseThreshold": 3"#).confirm_close_threshold, 3);
        for bad in ["0", "-2", "5000", r#""3""#, "null"] {
            rejected(&format!(r#""confirmCloseThreshold": {bad}"#));
        }
        let written = round_trips(Settings { confirm_close_threshold: 4, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "confirmCloseThreshold": 4 }));
    }

    #[test]
    fn closing_asks_only_at_or_above_the_threshold_and_only_when_enabled() {
        let ask = |confirm_close, confirm_close_threshold, running| Settings { confirm_close, confirm_close_threshold, ..Settings::default() }.should_confirm_close(running);
        assert!(!ask(true, 1, 0), "nothing is running");
        assert!(ask(true, 1, 1));
        assert!(!ask(true, 3, 2));
        assert!(ask(true, 3, 3));
        assert!(ask(true, 3, 10));
        assert!(!ask(false, 1, 10), "the switch wins over the threshold");
        assert!(ask(true, 0, 1), "a threshold of zero still needs a running session");
        assert!(!ask(true, 0, 0));
    }

    #[test]
    fn the_tab_bar_density_is_an_appearance_choice() {
        assert_eq!(Settings::default().appearance.tab_bar_density, TabBarDensity::Comfortable);
        assert_eq!(parsed_ok(r#""appearance": { "tabBarDensity": "compact" }"#).appearance.tab_bar_density, TabBarDensity::Compact);
        assert_eq!(parsed_ok(r#""appearance": { "tabBarDensity": "comfortable" }"#).appearance.tab_bar_density, TabBarDensity::Comfortable);
        rejected(r#""appearance": { "tabBarDensity": "tiny" }"#);
        let mut changed = Settings::default();
        changed.appearance.tab_bar_density = TabBarDensity::Compact;
        let written = round_trips(changed);
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "appearance": { "tabBarDensity": "compact" } }));
    }

    fn slots(texts: &[&str]) -> Vec<TitleSlot> {
        texts.iter().map(|text| TitleSlot::parse(text).expect("a slot of known parts")).collect()
    }

    #[test]
    fn the_title_of_a_tab_is_the_first_of_title_folder_and_profile_unless_the_file_says_otherwise() {
        assert_eq!(Settings::default().tabs.title.parts, slots(&["title|folder|profile"]));
        assert_eq!(Settings::default().tabs.title.to_text(), "title|folder|profile");
        let own = parsed_ok(r#""tabs": { "title": { "parts": ["folder", "title|profile", " agent "] } }"#);
        assert_eq!(own.tabs.title.parts, slots(&["folder", "title|profile", "agent"]));
        // Only what differs from the default is written, and it reads back.
        assert!(!serialize(&Settings::default()).contains("tabs"));
        let written = round_trips(own);
        assert_eq!(
            serde_json::from_str::<Value>(&written).unwrap(),
            serde_json::json!({ "tabs": { "title": { "parts": ["folder", "title|profile", "agent"] } } })
        );
    }

    #[test]
    fn a_part_the_title_does_not_know_goes_through_the_problems_of_the_file_and_the_default_stays() {
        for entry in [
            r#""tabs": { "title": { "parts": ["folder", "branch"] } }"#,
            r#""tabs": { "title": { "parts": ["title|nope"] } }"#,
            r#""tabs": { "title": { "parts": [] } }"#,
            r#""tabs": { "title": { "parts": [""] } }"#,
        ] {
            let result = parsed(&format!("{{ {entry} }}"));
            assert_eq!(result.problems.len(), 1, "{entry}: {:?}", result.problems);
            assert_eq!(result.problems[0].key, "tabs.title.parts", "{entry}");
            assert!(matches!(result.problems[0].expected, Expected::OneOf(_)), "{entry}");
            assert_eq!(result.settings, Settings::default(), "{entry}");
        }
        let result = parsed(r#"{ "tabs": { "title": { "parts": ["folder", "oops"] } } }"#);
        assert_eq!(result.problems[0].text(Lang::EnUs), "tabs.title.parts: Needs one of title, folder, profile, shell, agent");
        assert_eq!(result.problems[0].text(Lang::ZhTw), "tabs.title.parts：需要 title、folder、profile、shell、agent 其中之一");
        // What is not a list of text is told as such.
        for (entry, expected) in [
            (r#""tabs": { "title": { "parts": "folder" } }"#, Expected::Array),
            (r#""tabs": { "title": { "parts": ["folder", 3] } }"#, Expected::Text),
            (r#""tabs": { "title": 5 }"#, Expected::Object),
            (r#""tabs": [] "#, Expected::Object),
        ] {
            let result = parsed(&format!("{{ {entry} }}"));
            assert_eq!(result.problems.len(), 1, "{entry}: {:?}", result.problems);
            assert_eq!(result.problems[0].expected, expected, "{entry}");
            assert_eq!(result.settings, Settings::default(), "{entry}");
        }
    }

    #[test]
    fn the_title_format_is_typed_as_slots_separated_by_commas_and_parts_by_bars() {
        let title = TabTitle::from_text(" folder , title|profile,, agent ").expect("a valid format");
        assert_eq!(title.parts, slots(&["folder", "title|profile", "agent"]));
        assert_eq!(title.to_text(), "folder, title|profile, agent");
        assert_eq!(TabTitle::from_text(&title.to_text()), Some(title));
        for text in ["", " , ", "folder, branch", "title|", "|folder", "Title"] {
            assert_eq!(TabTitle::from_text(text), None, "{text:?}");
        }
        assert_eq!(TitlePart::ALL.map(TitlePart::name), ["title", "folder", "profile", "shell", "agent"]);
        for part in TitlePart::ALL {
            assert_eq!(TitlePart::from_name(part.name()), Some(part));
        }
    }

    #[test]
    fn osc52_lets_programs_copy_but_never_read_unless_the_file_says_so() {
        assert_eq!(Settings::default().osc52, Osc52Policy::Write);
        for (name, expected) in [("off", Osc52Policy::Off), ("write", Osc52Policy::Write), ("read-write", Osc52Policy::ReadWrite)] {
            assert_eq!(parsed_ok(&format!(r#""osc52": "{name}""#)).osc52, expected, "{name}");
        }
        for bad in [r#""read""#, r#""readwrite""#, r#""yes""#, "true", "1", "null"] {
            rejected(&format!(r#""osc52": {bad}"#));
        }
        assert_eq!(serialize(&Settings::default()), "{}\n", "the default is not written");
        for (policy, name) in [(Osc52Policy::Off, "off"), (Osc52Policy::ReadWrite, "read-write")] {
            let written = round_trips(Settings { osc52: policy, ..Settings::default() });
            assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "osc52": name }));
        }
    }

    #[test]
    fn the_bell_is_silent_unless_the_file_asks_for_a_sound() {
        let defaults = Settings::default().bell;
        assert_eq!((defaults.visual, defaults.taskbar, defaults.sound), (true, true, BellSound::None));
        for (name, expected) in [("none", BellSound::None), ("background", BellSound::Background), ("always", BellSound::Always)] {
            let result = parsed(&format!(r#"{{ "bell": {{ "sound": "{name}" }} }}"#));
            assert!(result.problems.is_empty(), "{name}: {:?}", result.problems);
            assert_eq!(result.settings.bell.sound, expected);
        }
        // Anything else keeps the silent default and is reported.
        let result = parsed(r#"{ "bell": { "sound": "loud" } }"#);
        assert_eq!(result.settings.bell.sound, BellSound::None);
        assert_eq!(result.problems.len(), 1);
        let written = round_trips(Settings { bell: Bell { sound: BellSound::Background, ..defaults }, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "bell": { "sound": "background" } }));
    }

    #[test]
    fn a_backdrop_list_maps_each_choice_to_its_selected_index() {
        for (at, backdrop) in Backdrop::ALL.into_iter().enumerate() {
            assert_eq!(backdrop.index(), at as i32);
        }
    }

    #[test]
    fn opacity_steps_stay_in_range_and_free_of_drift() {
        let at = |opacity| Appearance { opacity, ..Settings::default().appearance };
        assert_eq!(at(0.72).opacity_after(1), 0.77);
        assert_eq!(at(0.72).opacity_after(-1), 0.67);
        assert_eq!(at(0.72).opacity_after(0), 0.72);
        assert_eq!(at(0.97).opacity_after(1), 1.0);
        assert_eq!(at(1.0).opacity_after(1), 1.0);
        assert_eq!(at(0.22).opacity_after(-1), 0.2);
        assert_eq!(at(0.2).opacity_after(-5), 0.2);
        // Ten presses one at a time land where one press of ten does.
        let mut stepped = at(0.3);
        for _ in 0..10 {
            stepped.opacity = stepped.opacity_after(1);
        }
        assert_eq!(stepped.opacity, at(0.3).opacity_after(10));
        assert_eq!(stepped.opacity, 0.8);
    }

    #[test]
    fn font_zoom_moves_one_point_per_step_inside_the_valid_sizes() {
        let font = Settings::default().font;
        assert_eq!(font.size_after_zoom(0), 14.0);
        assert_eq!(font.size_after_zoom(3), 17.0);
        assert_eq!(font.size_after_zoom(-4), 10.0);
        assert_eq!(font.size_after_zoom(500), FONT_SIZE_RANGE.1);
        assert_eq!(font.size_after_zoom(-500), FONT_SIZE_RANGE.0);
    }

    #[test]
    fn a_byte_order_mark_does_not_break_the_file() {
        let result = parse(Some("\u{feff}{ \"theme\": \"Nord\" }"));
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        assert_eq!(result.settings.theme, "Nord");
        assert_eq!(parse(Some("\u{feff}")).settings, Settings::default());
    }

    #[test]
    fn a_custom_profile_with_an_empty_id_gets_the_default_id() {
        let result = parsed(r#"{ "profiles": [ { "id": "", "name": "Node", "program": "node.exe" } ] }"#);
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        assert_eq!(result.settings.profiles[0].id, "custom-Node");
    }

    #[test]
    fn profiles_of_one_name_without_ids_get_ids_of_their_own() {
        let result = parsed(r#"{ "profiles": [ { "name": "Node", "program": "a.exe" }, { "name": "Node", "program": "b.exe" }, { "name": "Node", "program": "c.exe" } ] }"#);
        let ids: Vec<&str> = result.settings.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["custom-Node", "custom-Node-2", "custom-Node-3"]);
    }

    #[test]
    fn a_profile_value_that_cannot_be_read_is_reported_and_the_rest_of_the_entry_kept() {
        let result = parsed(r#"{ "profiles": [ { "name": "Node", "program": "node.exe", "fontSize": 80, "env": { "N": 1, "A": "b" }, "args": [1, "x"] } ] }"#);
        let keys: Vec<&str> = result.problems.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["profiles[0].args[0]", "profiles[0].env.N", "profiles[0].fontSize"]);
        let profile = &result.settings.profiles[0];
        assert_eq!((&profile.args[..], profile.options.font_size), (&["x".to_owned()][..], None));
        assert_eq!(profile.options.env.keys().collect::<Vec<_>>(), ["A"]);
        assert_eq!(parsed(r#"{ "profiles": [ { "program": "x", "args": "-i" } ] }"#).problems[0].key, "profiles[0].args");
    }

    #[test]
    fn every_choice_of_the_new_settings_has_its_own_label() {
        let texts: Vec<Text> = [Language::Auto, Language::ZhTw, Language::EnUs].map(Language::text).into();
        assert_eq!(texts, [Text::CommonAuto, Text::LanguageZhTw, Text::LanguageEnUs]);
        let densities = [TabBarDensity::Comfortable, TabBarDensity::Compact].map(TabBarDensity::text);
        assert_ne!(densities[0], densities[1]);
        let backdrops = [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::None].map(Backdrop::text);
        assert_eq!(backdrops, [Text::BackdropAcrylic, Text::BackdropMica, Text::BackdropMicaAlt, Text::BackdropNone]);
        let cursors = [CursorStyle::Block, CursorStyle::Bar, CursorStyle::Underline].map(CursorStyle::text);
        assert_eq!(cursors, [Text::CursorBlock, Text::CursorBar, Text::CursorUnderline]);
        assert_ne!(RightClick::Smart.text(), RightClick::Menu.text());
    }
}
