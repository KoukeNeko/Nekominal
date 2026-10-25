//! Saved layouts: the tabs of a window (their panes, folders, profiles, names and colours) kept under a name, to be opened
//! again in a window. Each layout is a session (`session.rs`), written and read the way the session is, so a damaged
//! entry costs only itself.
//!
//! They live in their own file, `%APPDATA%\Nekominal\layouts.json`, not in `settings.json`: the app rewrites the file each
//! time a layout is saved, renamed or deleted, and a rewrite of the settings file loses the comments a user wrote in it.

use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{Map, Value, json};

use crate::{
    session::{self, Session},
    store,
};

const FILE: &str = "layouts.json";

/// The layouts by name, in the order of their names (the order the palette lists them in).
pub type Layouts = BTreeMap<String, Session>;

pub fn path() -> Option<PathBuf> {
    store::config_dir().map(|dir| dir.join(FILE))
}

/// The file contents: `{ "version": 1, "layouts": { "<name>": <a session> } }`.
pub fn to_json(layouts: &Layouts) -> String {
    let entries: Map<String, Value> = layouts
        .iter()
        .map(|(name, layout)| (name.clone(), layout.to_value()))
        .collect();
    let document = json!({ "version": session::VERSION, "layouts": entries });
    let text = serde_json::to_string_pretty(&document).expect("a JSON value always serializes");
    format!("{text}\n")
}

/// Reads the file. Never fails: a layout that cannot be used, or has no tab left, is dropped and told in the problems.
pub fn parse(text: &str) -> (Layouts, Vec<String>) {
    let mut layouts = Layouts::new();
    let mut problems = Vec::new();
    let document = match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(document) => document,
        Err(error) => {
            problems.push(format!("layouts: {error}"));
            return (layouts, problems);
        }
    };
    let Some(entries) = document.get("layouts").and_then(Value::as_object) else {
        problems.push("layouts: needs an object".to_owned());
        return (layouts, problems);
    };
    for (name, value) in entries {
        let parsed = Session::parse(&value.to_string());
        problems.extend(parsed.problems.into_iter().map(|problem| format!("{name}: {problem}")));
        if name.trim().is_empty() || parsed.session.tabs.is_empty() {
            problems.push(format!("{name:?}: left out, it has no name or no tab"));
            continue;
        }
        layouts.insert(name.clone(), parsed.session);
    }
    (layouts, problems)
}

/// The name a layout is saved under: what was typed without the spaces around it; none when nothing is left.
pub fn name(typed: &str) -> Option<String> {
    Some(typed.trim().to_owned()).filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layout::{Axis, Node, Split},
        session::{PaneRecord, TabRecord},
    };

    fn two_panes() -> Session {
        let layout = Node::Split(Box::new(Split { axis: Axis::Row, ratio: 0.3, first: Node::Leaf(1), second: Node::Leaf(2) }));
        let panes = [
            (1, PaneRecord { profile: "pwsh".into(), cwd: Some(r"D:\src\app".into()) }),
            (2, PaneRecord { profile: "claude".into(), cwd: Some(r"D:\src\app-fix".into()) }),
        ]
        .into();
        Session {
            window: None,
            tabs: vec![TabRecord { layout, panes, active: 2, zoomed: None, name: Some("agents".into()), color: Some(3) }],
            active_tab: 0,
        }
    }

    #[test]
    fn layouts_round_trip_with_their_panes_folders_profiles_names_and_colours() {
        let layouts: Layouts = [("work 工作".to_owned(), two_panes()), ("second".to_owned(), two_panes())].into();
        let (read, problems) = parse(&to_json(&layouts));
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(read, layouts);
    }

    #[test]
    fn a_damaged_layout_costs_only_itself_and_a_damaged_file_costs_nothing_but_the_layouts() {
        let text = r#"{ "version": 1, "layouts": { "bad": { "version": 1, "tabs": "no" }, "good": { "version": 1, "tabs": [ { "layout": { "profile": "cmd" } } ] } } }"#;
        let (read, problems) = parse(text);
        assert_eq!(read.keys().collect::<Vec<_>>(), ["good"]);
        assert!(!problems.is_empty());
        for broken in ["", "[]", "{ \"layouts\": 3 }"] {
            let (read, problems) = parse(broken);
            assert!(read.is_empty() && !problems.is_empty(), "{broken:?}");
        }
    }

    #[test]
    fn a_layout_is_named_by_what_was_typed_without_the_spaces_around_it() {
        assert_eq!(name("  agents "), Some("agents".to_owned()));
        assert_eq!(name("   "), None);
    }
}
