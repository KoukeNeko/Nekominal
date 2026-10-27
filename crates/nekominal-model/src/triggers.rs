//! Triggers (`settings.json` `triggers`): regular expressions matched against the lines of the output, each with what
//! it does when a line matches, as iTerm2's triggers. The terminal core matches (`nekominal_term::triggers`); this is
//! the setting, its checks, and the pure parts of the actions.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::settings::{boolean, text, Expected, Problems};
use crate::themes::Color;

/// The most triggers read: every line of output runs through all of them.
pub const MAX_TRIGGERS: usize = 64;
/// The longest pattern, in characters.
pub const MAX_PATTERN_CHARS: usize = 1024;
/// The colour of a highlight that names none: a dark amber that light text stays readable on.
pub const DEFAULT_HIGHLIGHT: &str = "#7a5c00";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub name: String,
    pub pattern: String,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub action: Action,
    /// Only while the pane runs this program (`claude.exe`, or `claude`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Matches the line the cursor is on as it is written, for a prompt that ends without a line break.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub instant: bool,
}

fn is_true(value: &bool) -> bool {
    *value
}

/// What a trigger does with a line that matches.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    /// The matched cells get this background (`#RRGGBB`).
    Highlight { color: String },
    /// A notification with this text, `$0` the match and `$1`..`$9` its groups.
    Notify { text: String },
    /// The tab is marked as wanting attention.
    Badge,
    /// This text goes to the program, as typed (`"y\r"`). Nothing is sent unless `allowSend` is true.
    Send {
        text: String,
        #[serde(rename = "allowSend", skip_serializing_if = "std::ops::Not::not")]
        allow_send: bool,
    },
    Bell,
}

impl Action {
    const NAMES: [&str; 5] = ["highlight", "notify", "badge", "send", "bell"];

    /// The colour of a highlight.
    pub fn highlight(&self) -> Option<Color> {
        match self {
            Action::Highlight { color } => Color::parse_hex(color),
            _ => None,
        }
    }
}

/// The `triggers` of a document. An entry with a missing or bad pattern or action is reported and left out, and so are
/// the entries past [`MAX_TRIGGERS`].
pub(crate) fn read(source: &Map<String, Value>, problems: &mut Problems) -> Vec<Trigger> {
    let Some(value) = source.get("triggers") else { return Vec::new() };
    let Value::Array(entries) = value else {
        problems.fail("", "triggers", Expected::Array);
        return Vec::new();
    };
    if entries.len() > MAX_TRIGGERS {
        problems.fail("", "triggers", Expected::MostEntries(MAX_TRIGGERS));
    }
    let mut triggers = Vec::new();
    for (index, entry) in entries.iter().take(MAX_TRIGGERS).enumerate() {
        let path = format!("triggers[{index}]");
        let Value::Object(map) = entry else {
            problems.fail("", &path, Expected::Object);
            continue;
        };
        if let Some(trigger) = read_one(map, &path, problems) {
            triggers.push(trigger);
        }
    }
    triggers
}

fn read_one(map: &Map<String, Value>, path: &str, problems: &mut Problems) -> Option<Trigger> {
    let pattern = text(map, "pattern", "", path, problems);
    if pattern.is_empty() {
        problems.fail(path, "pattern", Expected::Required);
        return None;
    }
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        problems.fail(path, "pattern", Expected::MostChars(MAX_PATTERN_CHARS));
        return None;
    }
    if let Err(error) = regex::Regex::new(&pattern) {
        // The parser's message draws the pattern with a caret over several lines; its last line says what is wrong.
        let error = error.to_string();
        problems.fail(path, "pattern", Expected::Regex(error.lines().last().unwrap_or_default().trim().to_owned()));
        return None;
    }
    let action = match map.get("action").and_then(Value::as_str) {
        None if !map.contains_key("action") => {
            problems.fail(path, "action", Expected::Required);
            return None;
        }
        Some("highlight") => {
            let color = text(map, "color", DEFAULT_HIGHLIGHT, path, problems);
            if Color::parse_hex(&color).is_none() {
                problems.fail(path, "color", Expected::Color);
                return None;
            }
            Action::Highlight { color }
        }
        Some("notify") => Action::Notify { text: text(map, "text", "$0", path, problems) },
        Some("badge") => Action::Badge,
        Some("send") => {
            let text = text(map, "text", "", path, problems);
            if text.is_empty() {
                problems.fail(path, "text", Expected::Required);
                return None;
            }
            Action::Send { text, allow_send: boolean(map, "allowSend", false, path, problems) }
        }
        Some("bell") => Action::Bell,
        _ => {
            problems.fail(path, "action", Expected::OneOf(Action::NAMES.map(str::to_owned).to_vec()));
            return None;
        }
    };
    let program = text(map, "program", "", path, problems);
    Some(Trigger {
        name: text(map, "name", &pattern, path, problems),
        enabled: boolean(map, "enabled", true, path, problems),
        program: (!program.is_empty()).then_some(program),
        instant: boolean(map, "instant", false, path, problems),
        pattern,
        action,
    })
}

/// `template` with `$0` replaced by the match and `$1`..`$9` by its groups (empty when the group did not take part);
/// `$$` is a dollar sign.
pub fn substitute(template: &str, groups: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match (ch, chars.peek()) {
            ('$', Some('$')) => {
                chars.next();
                out.push('$');
            }
            ('$', Some(digit @ '0'..='9')) => {
                let index = *digit as usize - '0' as usize;
                chars.next();
                out.push_str(groups.get(index).map_or("", String::as_str));
            }
            _ => out.push(ch),
        }
    }
    out
}

/// One trigger of a pane sends at most once in this time.
pub const SEND_GAP: Duration = Duration::from_secs(2);
/// A pane sends at most this many times in [`SEND_WINDOW`], whatever triggers ask.
pub const SEND_BURST: usize = 5;
pub const SEND_WINDOW: Duration = Duration::from_secs(60);

/// What the send triggers of one pane sent lately: a rule whose text makes the program print its own pattern again
/// would otherwise answer itself forever.
#[derive(Debug, Default)]
pub struct SendGuard {
    recent: VecDeque<Instant>,
    last_by_rule: HashMap<usize, Instant>,
}

impl SendGuard {
    /// Whether trigger `rule` may send at `now`; one that may is counted as sent.
    pub fn allow(&mut self, now: Instant, rule: usize) -> bool {
        while self.recent.front().is_some_and(|at| now.duration_since(*at) >= SEND_WINDOW) {
            self.recent.pop_front();
        }
        let too_soon = self.last_by_rule.get(&rule).is_some_and(|at| now.duration_since(*at) < SEND_GAP);
        if too_soon || self.recent.len() >= SEND_BURST {
            return false;
        }
        self.recent.push_back(now);
        self.last_by_rule.insert(rule, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{parse, serialize};
    use crate::strings::Lang;

    #[test]
    fn a_trigger_is_read_with_its_defaults_and_written_back_the_same() {
        let source = r##"{ "triggers": [
            { "name": "errors", "pattern": "error: (.+)", "action": "highlight", "color": "#ff0000" },
            { "pattern": "Do you want", "action": "notify", "program": "claude.exe", "instant": true },
            { "pattern": "\\[y/N\\]", "action": "send", "text": "y\r", "allowSend": true, "enabled": false },
            { "pattern": "done", "action": "badge" },
            { "pattern": "ding", "action": "bell" }
        ] }"##;
        let result = parse(Some(source));
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        let triggers = &result.settings.triggers;
        assert_eq!(triggers.len(), 5);
        assert_eq!(triggers[0].action, Action::Highlight { color: "#ff0000".into() });
        assert_eq!(triggers[0].action.highlight(), Some(Color::rgb(0xff0000)));
        let notify = &triggers[1];
        assert_eq!((notify.name.as_str(), notify.enabled, notify.instant), ("Do you want", true, true), "the name is the pattern");
        assert_eq!(notify.action, Action::Notify { text: "$0".into() });
        assert_eq!(notify.program.as_deref(), Some("claude.exe"));
        assert_eq!(triggers[2].action, Action::Send { text: "y\r".into(), allow_send: true });
        assert!(!triggers[2].enabled);
        assert_eq!((&triggers[3].action, &triggers[4].action), (&Action::Badge, &Action::Bell));
        let again = parse(Some(&serialize(&result.settings)));
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.settings.triggers, *triggers);
        let written = serialize(&result.settings);
        assert!(written.contains("\"action\": \"send\"") && written.contains("\"allowSend\": true"), "{written}");
    }

    #[test]
    fn a_send_is_off_unless_the_rule_allows_it() {
        let result = parse(Some(r#"{ "triggers": [ { "pattern": "x", "action": "send", "text": "y" } ] }"#));
        assert_eq!(result.settings.triggers[0].action, Action::Send { text: "y".into(), allow_send: false });
    }

    #[test]
    fn a_bad_trigger_is_reported_and_left_out() {
        let long = "a".repeat(MAX_PATTERN_CHARS + 1);
        let source = format!(
            r##"{{ "triggers": [
                {{ "pattern": "(unclosed", "action": "bell" }},
                {{ "action": "bell" }},
                {{ "pattern": "x", "action": "explode" }},
                {{ "pattern": "x" }},
                {{ "pattern": "x", "action": "highlight", "color": "red" }},
                {{ "pattern": "x", "action": "send" }},
                {{ "pattern": "{long}", "action": "bell" }},
                "not an object",
                {{ "pattern": "fine", "action": "bell" }}
            ] }}"##
        );
        let result = parse(Some(&source));
        assert_eq!(result.settings.triggers.len(), 1);
        assert_eq!(result.settings.triggers[0].pattern, "fine");
        let problems: Vec<(&str, &Expected)> = result.problems.iter().map(|p| (p.key.as_str(), &p.expected)).collect();
        assert_eq!(problems.len(), 8, "{problems:?}");
        assert!(matches!(problems[0], ("triggers[0].pattern", Expected::Regex(_))));
        assert_eq!(problems[1], ("triggers[1].pattern", &Expected::Required));
        assert!(matches!(problems[2], ("triggers[2].action", Expected::OneOf(_))));
        assert_eq!(problems[3], ("triggers[3].action", &Expected::Required));
        assert_eq!(problems[4], ("triggers[4].color", &Expected::Color));
        assert_eq!(problems[5], ("triggers[5].text", &Expected::Required));
        assert_eq!(problems[6], ("triggers[6].pattern", &Expected::MostChars(MAX_PATTERN_CHARS)));
        assert_eq!(problems[7], ("triggers[7]", &Expected::Object));
        assert_eq!(result.problems[0].text(Lang::EnUs), "triggers[0].pattern: Not a valid regular expression: error: unclosed group");
        assert_eq!(result.problems[6].text(Lang::EnUs), format!("triggers[6].pattern: At most {MAX_PATTERN_CHARS} characters"));
    }

    #[test]
    fn past_the_cap_the_triggers_are_left_out_and_that_is_said_once() {
        let one = r#"{ "pattern": "x", "action": "bell" }"#;
        let source = format!(r#"{{ "triggers": [{}] }}"#, vec![one; MAX_TRIGGERS + 3].join(","));
        let result = parse(Some(&source));
        assert_eq!(result.settings.triggers.len(), MAX_TRIGGERS);
        assert_eq!(result.problems.len(), 1);
        assert_eq!(result.problems[0].text(Lang::EnUs), format!("triggers: At most {MAX_TRIGGERS}; the rest are left out"));
        assert_eq!(parse(Some(r#"{ "triggers": {} }"#)).problems[0].expected, Expected::Array);
    }

    #[test]
    fn the_text_takes_the_match_and_its_groups() {
        let groups = ["error: disk full".to_owned(), "disk full".to_owned()];
        assert_eq!(substitute("$0", &groups), "error: disk full");
        assert_eq!(substitute("Failed ($1)", &groups), "Failed (disk full)");
        assert_eq!(substitute("$2 is not there, $$1 is a dollar", &groups), " is not there, $1 is a dollar");
        assert_eq!(substitute("ends with $", &groups), "ends with $");
        assert_eq!(substitute("設定 $1", &groups), "設定 disk full");
    }

    #[test]
    fn a_send_waits_between_two_of_one_rule_and_a_pane_sends_few_a_minute() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = SendGuard::default();
        assert!(guard.allow(at(0), 0));
        assert!(!guard.allow(at(500), 0), "the same rule again at once");
        assert!(guard.allow(at(600), 1), "another rule is not held by the first");
        assert!(guard.allow(at(2000), 0));
        assert!(guard.allow(at(4000), 0));
        assert!(guard.allow(at(6000), 0));
        assert!(!guard.allow(at(8000), 0), "five in a minute already");
        assert!(!guard.allow(at(9000), 2));
        assert!(guard.allow(at(60_001), 2), "the first went out of the window");
    }
}
