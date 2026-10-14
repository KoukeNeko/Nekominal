//! Settings model: defaults, tolerant parsing of `settings.json`, and minimal serialization.
//!
//! Parsing never fails: an invalid value falls back to its default and is reported in
//! [`Parsed::problems`], so a typo in the file can never prevent the terminal from starting.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
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
    pub fn text(self) -> Text {
        match self {
            RightClick::Smart => Text::RightClickSmart,
            RightClick::Menu => Text::RightClickMenu,
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
    pub program: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteSettings {
    pub warn_multiline: bool,
    pub warn_large_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bell {
    pub visual: bool,
    pub taskbar: bool,
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
    /// The choices in the order every list of them shows (the quick flyout, the settings page).
    pub const ALL: [Backdrop; 4] = [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::None];

    /// Position in [`Backdrop::ALL`], the selected index of a list of the choices.
    pub fn index(self) -> i32 {
        Backdrop::ALL.iter().position(|choice| *choice == self).map_or(-1, |at| at as i32)
    }

    /// The choice at a selected index; `None` for -1, which a list reports while it moves its selection.
    pub fn from_index(index: i32) -> Option<Backdrop> {
        usize::try_from(index).ok().and_then(|at| Backdrop::ALL.get(at).copied())
    }

    /// The order the "switch backdrop" command cycles through.
    pub fn next(self) -> Backdrop {
        match self {
            Backdrop::Acrylic => Backdrop::Mica,
            Backdrop::Mica => Backdrop::MicaAlt,
            Backdrop::MicaAlt => Backdrop::None,
            Backdrop::None => Backdrop::Acrylic,
        }
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: Language,
    pub theme: String,
    pub font: Font,
    pub cursor: Cursor,
    pub appearance: Appearance,
    pub scrollback: u32,
    /// Profile id; empty selects the first discovered profile.
    pub default_profile: String,
    pub profiles: Vec<CustomProfile>,
    pub copy_on_select: bool,
    pub right_click: RightClick,
    pub paste: PasteSettings,
    pub bell: Bell,
    pub keyboard: Keyboard,
    /// Sets environment variables that make coding-agent CLIs use their full terminal features.
    pub agent_environment: bool,
    /// Makes PowerShell report its working directory and prompt marks.
    pub shell_integration: bool,
    /// Ask before closing a window, tab or pane that holds running sessions.
    pub confirm_close: bool,
    /// Number of running sessions a close must end before it asks; 1 asks whenever one is running.
    pub confirm_close_threshold: u32,
    /// Reopen the tabs and panes of the previous run on start.
    pub restore_session: bool,
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
            font: Font {
                family: ["Cascadia Mono", "Consolas", "Microsoft JhengHei UI", "Segoe UI Emoji"]
                    .map(String::from)
                    .to_vec(),
                size: 14.0,
                weight: 400,
                line_height: 1.1,
            },
            cursor: Cursor { style: CursorStyle::Block, blink: true },
            appearance: Appearance { backdrop: Backdrop::Acrylic, opacity: 0.72, tab_bar_density: TabBarDensity::Comfortable },
            scrollback: 10_000,
            default_profile: String::new(),
            profiles: Vec::new(),
            copy_on_select: false,
            right_click: RightClick::Smart,
            paste: PasteSettings { warn_multiline: true, warn_large_bytes: 5120 },
            bell: Bell { visual: true, taskbar: true },
            keyboard: Keyboard { kitty_protocol: true },
            agent_environment: true,
            shell_integration: true,
            confirm_close: true,
            confirm_close_threshold: 1,
            restore_session: false,
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
    /// Human-readable problems; the affected values fall back to defaults.
    pub problems: Vec<String>,
}

#[derive(Default)]
struct Problems(Vec<String>);

impl Problems {
    fn fail(&mut self, path: &str, key: &str, requirement: &str) {
        let prefix = if path.is_empty() { String::new() } else { format!("{path}.") };
        self.0.push(format!("{prefix}{key}：{requirement}"));
    }
}

fn number(source: &Map<String, Value>, key: &str, fallback: f64, (min, max): (f64, f64), path: &str, problems: &mut Problems) -> f64 {
    match source.get(key) {
        None => fallback,
        Some(value) => match value.as_f64() {
            Some(n) if n.is_finite() && n >= min && n <= max => n,
            _ => {
                problems.fail(path, key, &format!("需要 {min} 到 {max} 的數字"));
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
            problems.fail(path, key, "需要 true 或 false");
            fallback
        }
    }
}

fn text(source: &Map<String, Value>, key: &str, fallback: &str, path: &str, problems: &mut Problems) -> String {
    match source.get(key) {
        None => fallback.to_owned(),
        Some(Value::String(value)) => value.clone(),
        Some(_) => {
            problems.fail(path, key, "需要文字");
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
                let names: Vec<&str> = allowed.iter().map(|(name, _)| *name).collect();
                problems.fail(path, key, &format!("需要 {} 其中之一", names.join("、")));
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
            problems.fail(path, key, "需要物件");
            Map::new()
        }
    }
}

/// Accepts an array of family names or a comma separated string such as `"Cascadia Mono", Consolas`.
fn font_families(source: &Map<String, Value>, fallback: &[String], problems: &mut Problems) -> Vec<String> {
    let parsed: Option<Vec<String>> = match source.get("family") {
        None => return fallback.to_vec(),
        Some(Value::Array(items)) => items.iter().map(|v| v.as_str().map(|s| s.trim().to_owned())).collect(),
        Some(Value::String(list)) => Some(
            list.split(',')
                .map(|name| name.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_owned())
                .collect(),
        ),
        Some(_) => None,
    };
    match parsed.map(|names| names.into_iter().filter(|n| !n.is_empty()).collect::<Vec<_>>()) {
        Some(names) if !names.is_empty() => names,
        _ => {
            problems.fail("font", "family", "需要字型名稱的陣列或以逗號分隔的文字");
            fallback.to_vec()
        }
    }
}

fn read_profiles(source: &Map<String, Value>, problems: &mut Problems) -> Vec<CustomProfile> {
    let Some(value) = source.get("profiles") else { return Vec::new() };
    let Value::Array(entries) = value else {
        problems.0.push("profiles：需要陣列".to_owned());
        return Vec::new();
    };
    let mut profiles = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("profiles[{index}]");
        let Value::Object(map) = entry else {
            problems.0.push(format!("{path}：需要物件"));
            continue;
        };
        let program = text(map, "program", "", &path, problems);
        if program.is_empty() {
            problems.0.push(format!("{path}.program：必填"));
            continue;
        }
        let name = text(map, "name", &program, &path, problems);
        let args = match map.get("args") {
            Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        };
        let cwd = text(map, "cwd", "", &path, problems);
        // An empty id could never be picked (the empty default profile means "the first one").
        let id = Some(text(map, "id", "", &path, problems)).filter(|id| !id.is_empty()).unwrap_or_else(|| format!("custom-{name}"));
        profiles.push(CustomProfile {
            id,
            name,
            program,
            args,
            cwd: (!cwd.is_empty()).then_some(cwd),
        });
    }
    profiles
}

fn read_keybindings(source: &Map<String, Value>, problems: &mut Problems) -> BTreeMap<String, Vec<String>> {
    let Some(value) = source.get("keybindings") else { return BTreeMap::new() };
    let Value::Object(map) = value else {
        problems.0.push("keybindings：需要物件".to_owned());
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
            _ => problems.0.push(format!("keybindings.{id}：需要文字或文字陣列")),
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
        Err(error) => return Parsed { settings: defaults, problems: vec![format!("設定檔無法解析：{error}")] },
    };
    let Value::Object(root) = document else {
        return Parsed { settings: defaults, problems: vec!["設定檔需要是 JSON 物件".to_owned()] };
    };

    let mut problems = Problems::default();
    let d = &defaults;

    let font = section(&root, "font", "", &mut problems);
    let cursor = section(&root, "cursor", "", &mut problems);
    let appearance = section(&root, "appearance", "", &mut problems);
    let paste = section(&root, "paste", "", &mut problems);
    let bell = section(&root, "bell", "", &mut problems);
    let keyboard = section(&root, "keyboard", "", &mut problems);

    let theme_name = text(&root, "theme", &d.theme, "", &mut problems);
    let theme = if themes::is_known(&theme_name) {
        theme_name
    } else {
        problems.0.push(format!("theme：找不到「{theme_name}」"));
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
        scrollback: number(&root, "scrollback", d.scrollback as f64, (0.0, 1_000_000.0), "", &mut problems) as u32,
        default_profile: text(&root, "defaultProfile", &d.default_profile, "", &mut problems),
        profiles: read_profiles(&root, &mut problems),
        copy_on_select: boolean(&root, "copyOnSelect", d.copy_on_select, "", &mut problems),
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
        bell: Bell {
            visual: boolean(&bell, "visual", d.bell.visual, "bell", &mut problems),
            taskbar: boolean(&bell, "taskbar", d.bell.taskbar, "bell", &mut problems),
        },
        keyboard: Keyboard { kitty_protocol: boolean(&keyboard, "kittyProtocol", d.keyboard.kitty_protocol, "keyboard", &mut problems) },
        agent_environment: boolean(&root, "agentEnvironment", d.agent_environment, "", &mut problems),
        shell_integration: boolean(&root, "shellIntegration", d.shell_integration, "", &mut problems),
        confirm_close: boolean(&root, "confirmClose", d.confirm_close, "", &mut problems),
        confirm_close_threshold: number(&root, "confirmCloseThreshold", d.confirm_close_threshold as f64, (1.0, 1000.0), "", &mut problems) as u32,
        restore_session: boolean(&root, "restoreSession", d.restore_session, "", &mut problems),
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
        assert_eq!(s.cursor, Cursor { style: CursorStyle::Bar, blink: true });
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
        let all = result.problems.join("\n");
        assert!(all.contains("font.size") && all.contains("cursor.style"));
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
        changed.profiles.push(CustomProfile { id: "a".into(), name: "A".into(), program: "a.exe".into(), args: vec![], cwd: None });
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
    fn restoring_the_session_is_opt_in() {
        assert!(!Settings::default().restore_session);
        assert!(parsed_ok(r#""restoreSession": true"#).restore_session);
        rejected(r#""restoreSession": "yes""#);
        let written = round_trips(Settings { restore_session: true, ..Settings::default() });
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), serde_json::json!({ "restoreSession": true }));
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

    #[test]
    fn the_backdrop_command_visits_every_backdrop_and_comes_back() {
        let mut seen = vec![Backdrop::Acrylic];
        let mut current = Backdrop::Acrylic;
        for _ in 0..3 {
            current = current.next();
            assert!(!seen.contains(&current), "{current:?} came up twice");
            seen.push(current);
        }
        assert_eq!(current.next(), Backdrop::Acrylic);
        assert_eq!(seen, vec![Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::None]);
    }

    #[test]
    fn a_backdrop_list_maps_selected_indexes_both_ways_and_ignores_no_selection() {
        for (at, backdrop) in Backdrop::ALL.into_iter().enumerate() {
            assert_eq!(backdrop.index(), at as i32);
            assert_eq!(Backdrop::from_index(at as i32), Some(backdrop));
        }
        // A list reports -1 while its selection moves from one choice to the next: that is not a choice.
        assert_eq!(Backdrop::from_index(-1), None);
        assert_eq!(Backdrop::from_index(Backdrop::ALL.len() as i32), None);
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
