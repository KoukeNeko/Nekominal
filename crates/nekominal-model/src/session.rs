//! The tabs and panes of a window as they are saved between runs, so a workspace can be restored: per pane the
//! profile and working directory, per tab the layout tree with its split ratios, the active and zoomed pane,
//! plus the active tab and where the window was.
//!
//! The file is JSON with a `version`. Reading never fails: whatever cannot be used is dropped and reported in
//! [`Parsed::problems`], and an unreadable file yields an empty session, so a damaged file can never keep the
//! terminal from starting. Fields added later need no version change, since unknown keys are ignored and a
//! missing one takes its default; the version moves only when an existing field changes meaning.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::layout::{Axis, MAX_RATIO, MIN_RATIO, Node, PaneId, Split};

/// The version written, and the newest one read.
pub const VERSION: u32 = 1;

/// The session file was written by a newer version of the app, which this one cannot read: it must not be written over
/// either, or going back to the newer version loses its tabs.
pub fn written_by_newer(text: &str) -> bool {
    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
        .ok()
        .and_then(|document| document.get("version").and_then(Value::as_u64))
        .is_some_and(|version| version > u64::from(VERSION))
}
/// More tabs or panes than this in a file are dropped: the file is outside the program's control, and
/// restoring would start one shell for each.
pub const MAX_TABS: usize = 100;
pub const MAX_PANES_PER_TAB: usize = 32;
/// Windows outside these sizes (in pixels) are not a placement worth restoring.
pub const MIN_WINDOW_SIZE: u32 = 100;
pub const MAX_WINDOW_SIZE: u32 = 100_000;
const MAX_COORDINATE: i64 = 100_000;

/// The part of a window's top edge a user drags it by, and how much of it must be on a screen.
const TITLE_STRIP_HEIGHT: u32 = 40;
const MIN_GRAB_WIDTH: i64 = 100;
const MIN_GRAB_HEIGHT: i64 = 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneRecord {
    /// Profile id; empty or unknown opens the default profile.
    pub profile: String,
    /// Working directory the pane was in; a directory that no longer exists is the shell's to handle.
    pub cwd: Option<String>,
    /// Name of the file that holds the pane's text (see [`is_buffer_name`]), shown again when it is restored.
    pub buffer: Option<String>,
}

/// A name the session may give the file of a pane's text: letters, digits and `-`, then `.vt`. The file lives in a
/// directory of the program's own and the session file is outside its control: anything else (a path, `..`) is not a
/// name to open.
pub fn is_buffer_name(name: &str) -> bool {
    name.strip_suffix(".vt").is_some_and(|stem| (1..=64).contains(&stem.len()) && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabRecord {
    pub layout: Node,
    /// A record for every pane of `layout`; a missing one reads as the default profile.
    pub panes: BTreeMap<PaneId, PaneRecord>,
    pub active: PaneId,
    pub zoomed: Option<PaneId>,
    /// The name and the colour (an index of the colours of the tabs) the user gave the tab.
    pub name: Option<String>,
    pub color: Option<usize>,
}

/// A rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    /// Width and height of the area both rectangles cover (zero when they only touch or are apart).
    fn overlap(&self, other: &PixelRect) -> (i64, i64) {
        let span = |start: i32, length: u32, other_start: i32, other_length: u32| {
            let begin = i64::from(start).max(i64::from(other_start));
            let end = (i64::from(start) + i64::from(length))
                .min(i64::from(other_start) + i64::from(other_length));
            (end - begin).max(0)
        };
        (
            span(self.x, self.width, other.x, other.width),
            span(self.y, self.height, other.y, other.height),
        )
    }
}

/// Where the window was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The restored (normal) rectangle: not the maximized one, and not the parking spot of a minimized window.
    pub rect: PixelRect,
    pub maximized: bool,
}

impl Placement {
    /// Whether enough of the title strip lies inside one of `work_areas` to grab it. A monitor that was
    /// unplugged or a changed resolution can leave a saved window out of reach; the caller then places
    /// the window itself.
    pub fn is_reachable_on(&self, work_areas: &[PixelRect]) -> bool {
        let strip = PixelRect {
            height: self.rect.height.min(TITLE_STRIP_HEIGHT),
            ..self.rect
        };
        work_areas.iter().any(|area| {
            let (width, height) = strip.overlap(area);
            width >= MIN_GRAB_WIDTH && height >= MIN_GRAB_HEIGHT
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub window: Option<Placement>,
    pub tabs: Vec<TabRecord>,
    pub active_tab: usize,
}

#[derive(Debug)]
pub struct Parsed {
    pub session: Session,
    /// Human-readable problems; what they name was dropped or replaced by a default.
    pub problems: Vec<String>,
}

impl Session {
    /// The file contents. Pane ids are not written: panes are identified by their place in the tree.
    pub fn to_json(&self) -> String {
        let text = serde_json::to_string_pretty(&self.to_value())
            .expect("a JSON value always serializes");
        format!("{text}\n")
    }

    /// The object of [`Session::to_json`], for a file that keeps several sessions (the saved layouts).
    pub fn to_value(&self) -> Value {
        let mut root = Map::new();
        root.insert("version".into(), json!(VERSION));
        if let Some(placement) = &self.window {
            root.insert("window".into(), window_to_value(placement));
        }
        root.insert("activeTab".into(), json!(self.active_tab));
        root.insert(
            "tabs".into(),
            Value::Array(self.tabs.iter().map(tab_to_value).collect()),
        );
        Value::Object(root)
    }

    /// Reads the file contents. Pane ids are assigned afresh, from 1, in tab order and then tree order, so
    /// they are unique across the whole session.
    pub fn parse(text: &str) -> Parsed {
        let mut problems = Vec::new();
        let session = read_session(text, &mut problems).unwrap_or_default();
        Parsed { session, problems }
    }
}

// --- writing ---------------------------------------------------------------------------------------

fn window_to_value(placement: &Placement) -> Value {
    json!({
        "x": placement.rect.x,
        "y": placement.rect.y,
        "width": placement.rect.width,
        "height": placement.rect.height,
        "maximized": placement.maximized,
    })
}

fn tab_to_value(tab: &TabRecord) -> Value {
    let order = tab.layout.pane_ids();
    let position = |id: PaneId| order.iter().position(|candidate| *candidate == id);
    let mut map = Map::new();
    map.insert("active".into(), json!(position(tab.active).unwrap_or(0)));
    if let Some(index) = tab.zoomed.and_then(position) {
        map.insert("zoomed".into(), json!(index));
    }
    if let Some(name) = &tab.name {
        map.insert("name".into(), json!(name));
    }
    if let Some(color) = tab.color {
        map.insert("color".into(), json!(color));
    }
    map.insert("layout".into(), node_to_value(&tab.layout, &tab.panes));
    Value::Object(map)
}

fn node_to_value(node: &Node, panes: &BTreeMap<PaneId, PaneRecord>) -> Value {
    match node {
        Node::Leaf(id) => {
            let record = panes.get(id).cloned().unwrap_or_default();
            let mut map = Map::new();
            map.insert("profile".into(), json!(record.profile));
            if let Some(cwd) = record.cwd {
                map.insert("cwd".into(), json!(cwd));
            }
            if let Some(buffer) = record.buffer {
                map.insert("buffer".into(), json!(buffer));
            }
            Value::Object(map)
        }
        Node::Split(split) => json!({
            "axis": match split.axis { Axis::Row => "row", Axis::Column => "column" },
            "ratio": usable_ratio(split.ratio),
            "first": node_to_value(&split.first, panes),
            "second": node_to_value(&split.second, panes),
        }),
    }
}

/// A ratio that keeps both sides visible; anything that is not a number becomes an even split.
fn usable_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() {
        ratio.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        0.5
    }
}

// --- reading ---------------------------------------------------------------------------------------

/// What reading one tab needs besides the JSON: the id counter that spans the session, the records
/// collected so far, and the problems.
struct TabReader<'a> {
    next_id: &'a mut PaneId,
    panes: BTreeMap<PaneId, PaneRecord>,
    problems: &'a mut Vec<String>,
    /// The pane limit is reported once per tab, not once per pane over it.
    limit_reported: bool,
}

fn read_session(text: &str, problems: &mut Vec<String>) -> Option<Session> {
    // Some editors write a byte order mark, which JSON parsers reject.
    let document: Value = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(value) => value,
        Err(error) => {
            problems.push(format!("工作階段檔無法解析：{error}"));
            return None;
        }
    };
    let Value::Object(root) = document else {
        problems.push("工作階段檔需要是 JSON 物件".to_owned());
        return None;
    };

    match root.get("version").and_then(Value::as_u64) {
        Some(version) if (1..=u64::from(VERSION)).contains(&version) => {}
        Some(version) if version > u64::from(VERSION) => {
            problems.push(format!("version：{version} 比這個版本能讀的 {VERSION} 新"));
            return None;
        }
        _ => {
            problems.push("version：需要 1 以上的整數".to_owned());
            return None;
        }
    }

    let window = read_window(root.get("window"), problems);
    let entries: &[Value] = match root.get("tabs") {
        None => &[],
        Some(Value::Array(entries)) => entries,
        Some(_) => {
            problems.push("tabs：需要陣列".to_owned());
            &[]
        }
    };
    if entries.len() > MAX_TABS {
        problems.push(format!("tabs：只讀取前 {MAX_TABS} 個分頁"));
    }

    let mut next_id: PaneId = 1;
    let mut tabs = Vec::new();
    for (index, entry) in entries.iter().take(MAX_TABS).enumerate() {
        if let Some(tab) = read_tab(entry, &format!("tabs[{index}]"), &mut next_id, problems) {
            tabs.push(tab);
        }
    }

    let active_tab = match root.get("activeTab") {
        None => 0,
        Some(value) => match value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < tabs.len().max(1))
        {
            Some(index) => index,
            None => {
                problems.push("activeTab：不是現有的分頁".to_owned());
                0
            }
        },
    };
    Some(Session {
        window,
        tabs,
        active_tab,
    })
}

fn read_window(value: Option<&Value>, problems: &mut Vec<String>) -> Option<Placement> {
    let value = value?;
    let Value::Object(map) = value else {
        problems.push("window：需要物件".to_owned());
        return None;
    };
    let coordinate = |key: &str| {
        map.get(key)
            .and_then(Value::as_i64)
            .filter(|n| n.abs() <= MAX_COORDINATE)
            .and_then(|n| i32::try_from(n).ok())
    };
    let size = |key: &str| {
        map.get(key)
            .and_then(Value::as_u64)
            .filter(|n| (u64::from(MIN_WINDOW_SIZE)..=u64::from(MAX_WINDOW_SIZE)).contains(n))
            .and_then(|n| u32::try_from(n).ok())
    };
    let (Some(x), Some(y), Some(width), Some(height)) = (
        coordinate("x"),
        coordinate("y"),
        size("width"),
        size("height"),
    ) else {
        problems.push(format!("window：x、y 需要 ±{MAX_COORDINATE} 內的整數，width、height 需要 {MIN_WINDOW_SIZE} 到 {MAX_WINDOW_SIZE} 的整數"));
        return None;
    };
    let maximized = match map.get("maximized") {
        None => false,
        Some(Value::Bool(maximized)) => *maximized,
        Some(_) => {
            problems.push("window.maximized：需要 true 或 false".to_owned());
            false
        }
    };
    Some(Placement {
        rect: PixelRect {
            x,
            y,
            width,
            height,
        },
        maximized,
    })
}

fn read_tab(
    value: &Value,
    path: &str,
    next_id: &mut PaneId,
    problems: &mut Vec<String>,
) -> Option<TabRecord> {
    let Value::Object(map) = value else {
        problems.push(format!("{path}：需要物件"));
        return None;
    };
    let Some(layout_value) = map.get("layout") else {
        problems.push(format!("{path}.layout：必填"));
        return None;
    };
    let mut reader = TabReader {
        next_id,
        panes: BTreeMap::new(),
        problems,
        limit_reported: false,
    };
    // A layout without a usable pane has already been explained by the node that failed.
    let layout = reader.node(layout_value, &format!("{path}.layout"))?;
    let order = layout.pane_ids();

    // `active` and `zoomed` are positions in tree order; `null` means the same as absent.
    let mut pick = |key: &str| -> Option<PaneId> {
        let value = map.get(key).filter(|value| !value.is_null())?;
        let index = value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < order.len());
        if index.is_none() {
            reader
                .problems
                .push(format!("{path}.{key}：不是這個分頁的窗格"));
        }
        index.map(|n| order[n])
    };
    let active = pick("active").unwrap_or(order[0]);
    let zoomed = pick("zoomed");
    // What is not a name or a small number is left out: a tab without them is a plain tab.
    let name = map.get("name").and_then(Value::as_str).map(str::to_owned).filter(|name| !name.trim().is_empty());
    let color = map.get("color").and_then(Value::as_u64).filter(|c| *c < 64).map(|c| c as usize);
    Some(TabRecord {
        layout,
        panes: reader.panes,
        active,
        zoomed,
        name,
        color,
    })
}

impl TabReader<'_> {
    /// One node of the layout tree. A leaf is an object without `first`, `second` and `axis`. A split that
    /// loses one child to a problem becomes the other child; one that loses both, and a leaf past the pane
    /// limit, give `None`.
    fn node(&mut self, value: &Value, path: &str) -> Option<Node> {
        let Value::Object(map) = value else {
            self.problems.push(format!("{path}：需要物件"));
            return None;
        };
        let is_split = ["axis", "first", "second"]
            .iter()
            .any(|key| map.contains_key(*key));
        if !is_split {
            return self.leaf(map, path);
        }

        let axis = match map.get("axis").and_then(Value::as_str) {
            Some("row") => Axis::Row,
            Some("column") => Axis::Column,
            _ => {
                self.problems
                    .push(format!("{path}.axis：需要 row 或 column 其中之一"));
                Axis::Row
            }
        };
        let ratio = match map.get("ratio") {
            None => 0.5,
            Some(value) => match value.as_f64().filter(|r| r.is_finite()) {
                Some(ratio) => usable_ratio(ratio),
                None => {
                    self.problems.push(format!("{path}.ratio：需要數字"));
                    0.5
                }
            },
        };
        let mut child = |key: &str| match map.get(key) {
            Some(value) => self.node(value, &format!("{path}.{key}")),
            None => {
                self.problems.push(format!("{path}.{key}：必填"));
                None
            }
        };
        let first = child("first");
        let second = child("second");
        match (first, second) {
            (Some(first), Some(second)) => Some(Node::Split(Box::new(Split {
                axis,
                ratio,
                first,
                second,
            }))),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        }
    }

    fn leaf(&mut self, map: &Map<String, Value>, path: &str) -> Option<Node> {
        if self.panes.len() >= MAX_PANES_PER_TAB {
            if !self.limit_reported {
                self.limit_reported = true;
                self.problems.push(format!(
                    "{path}：每個分頁只讀取前 {MAX_PANES_PER_TAB} 個窗格"
                ));
            }
            return None;
        }
        let profile = match map.get("profile") {
            None => String::new(),
            Some(Value::String(profile)) => profile.clone(),
            Some(_) => {
                self.problems.push(format!("{path}.profile：需要文字"));
                String::new()
            }
        };
        let cwd = match map.get("cwd") {
            None => None,
            Some(Value::String(cwd)) => Some(cwd.clone()).filter(|cwd| !cwd.is_empty()),
            Some(_) => {
                self.problems.push(format!("{path}.cwd：需要文字"));
                None
            }
        };
        let buffer = match map.get("buffer") {
            None => None,
            Some(Value::String(name)) if is_buffer_name(name) => Some(name.clone()),
            Some(_) => {
                self.problems.push(format!("{path}.buffer：需要 .vt 檔名"));
                None
            }
        };
        let id = *self.next_id;
        *self.next_id += 1;
        self.panes.insert(id, PaneRecord { profile, cwd, buffer });
        Some(Node::Leaf(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_of_a_newer_version_is_told_apart_from_one_this_version_writes() {
        assert!(written_by_newer(&format!(r#"{{ "version": {} }}"#, VERSION + 1)));
        assert!(!written_by_newer(&format!(r#"{{ "version": {VERSION} }}"#)));
        assert!(!written_by_newer("not json"), "a broken file is no newer version: it is written over");
    }

    fn pane(profile: &str, cwd: Option<&str>) -> PaneRecord {
        PaneRecord {
            profile: profile.into(),
            cwd: cwd.map(str::to_owned),
            buffer: None,
        }
    }

    fn split(axis: Axis, ratio: f64, first: Node, second: Node) -> Node {
        Node::Split(Box::new(Split {
            axis,
            ratio,
            first,
            second,
        }))
    }

    /// Tab of three panes: `a` on the left, `b` above `c` on the right. Ids are deliberately not contiguous.
    fn three_pane_tab() -> TabRecord {
        TabRecord {
            layout: split(
                Axis::Row,
                0.4,
                Node::Leaf(7),
                split(Axis::Column, 0.25, Node::Leaf(3), Node::Leaf(11)),
            ),
            panes: BTreeMap::from([
                (7, pane("pwsh", Some("C:\\a"))),
                (3, pane("cmd", None)),
                (11, pane("wsl-Ubuntu", Some("\\\\wsl$\\Ubuntu\\home"))),
            ]),
            active: 3,
            zoomed: Some(11),
            name: Some("build · 編譯".into()),
            color: Some(2),
        }
    }

    fn one_pane_tab(profile: &str) -> TabRecord {
        TabRecord {
            layout: Node::Leaf(1),
            panes: BTreeMap::from([(1, pane(profile, None))]),
            active: 1,
            zoomed: None,
            name: None,
            color: None,
        }
    }

    fn window() -> Placement {
        Placement {
            rect: PixelRect {
                x: -1800,
                y: 60,
                width: 1200,
                height: 800,
            },
            maximized: true,
        }
    }

    fn parse_ok(text: &str) -> Session {
        let parsed = Session::parse(text);
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        parsed.session
    }

    /// Problems of parsing `text`, which must still produce a session.
    fn problems_of(text: &str) -> Vec<String> {
        Session::parse(text).problems
    }

    #[test]
    fn a_session_round_trips_with_ids_renumbered_in_tree_order() {
        let session = Session {
            window: Some(window()),
            tabs: vec![one_pane_tab("pwsh"), three_pane_tab()],
            active_tab: 1,
        };
        let restored = parse_ok(&session.to_json());

        assert_eq!(restored.window, Some(window()));
        assert_eq!(restored.active_tab, 1);
        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.tabs[0], one_pane_tab("pwsh"));

        let second = &restored.tabs[1];
        assert_eq!(
            second.layout,
            split(
                Axis::Row,
                0.4,
                Node::Leaf(2),
                split(Axis::Column, 0.25, Node::Leaf(3), Node::Leaf(4))
            )
        );
        assert_eq!(
            second.panes,
            BTreeMap::from([
                (2, pane("pwsh", Some("C:\\a"))),
                (3, pane("cmd", None)),
                (4, pane("wsl-Ubuntu", Some("\\\\wsl$\\Ubuntu\\home")))
            ])
        );
        assert_eq!(
            (second.active, second.zoomed),
            (3, Some(4)),
            "active and zoomed follow their panes"
        );
    }

    #[test]
    fn writing_what_was_read_changes_nothing() {
        let session = Session {
            window: Some(window()),
            tabs: vec![three_pane_tab(), one_pane_tab("x")],
            active_tab: 0,
        };
        let first = session.to_json();
        assert_eq!(parse_ok(&first).to_json(), first);
    }

    #[test]
    fn an_empty_session_is_written_and_read() {
        let json = Session::default().to_json();
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap(),
            json!({ "version": 1, "activeTab": 0, "tabs": [] })
        );
        assert_eq!(parse_ok(&json), Session::default());
    }

    #[test]
    fn the_file_has_the_documented_shape() {
        let session = Session {
            window: Some(window()),
            tabs: vec![three_pane_tab()],
            active_tab: 0,
        };
        let value: Value = serde_json::from_str(&session.to_json()).unwrap();
        assert_eq!(
            value,
            json!({
                "version": 1,
                "window": { "x": -1800, "y": 60, "width": 1200, "height": 800, "maximized": true },
                "activeTab": 0,
                "tabs": [{
                    "active": 1,
                    "zoomed": 2,
                    "name": "build · 編譯",
                    "color": 2,
                    "layout": {
                        "axis": "row", "ratio": 0.4,
                        "first": { "profile": "pwsh", "cwd": "C:\\a" },
                        "second": {
                            "axis": "column", "ratio": 0.25,
                            "first": { "profile": "cmd" },
                            "second": { "profile": "wsl-Ubuntu", "cwd": "\\\\wsl$\\Ubuntu\\home" }
                        }
                    }
                }]
            })
        );
    }

    #[test]
    fn pane_ids_are_unique_across_tabs() {
        let session = Session {
            window: None,
            tabs: vec![one_pane_tab("a"), one_pane_tab("b"), three_pane_tab()],
            active_tab: 0,
        };
        let restored = parse_ok(&session.to_json());
        let ids: Vec<PaneId> = restored
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        for tab in &restored.tabs {
            assert_eq!(
                tab.panes.keys().copied().collect::<Vec<_>>(),
                tab.layout.pane_ids()
            );
        }
    }

    #[test]
    fn a_pane_without_a_record_is_written_with_the_default_profile() {
        let mut tab = one_pane_tab("pwsh");
        tab.panes.clear();
        let restored = parse_ok(
            &Session {
                window: None,
                tabs: vec![tab],
                active_tab: 0,
            }
            .to_json(),
        );
        assert_eq!(restored.tabs[0].panes[&1], PaneRecord::default());
    }

    #[test]
    fn an_active_or_zoomed_pane_outside_the_tree_is_written_as_the_first_pane_or_not_at_all() {
        let mut tab = three_pane_tab();
        tab.active = 99;
        tab.zoomed = Some(98);
        let restored = parse_ok(
            &Session {
                window: None,
                tabs: vec![tab],
                active_tab: 0,
            }
            .to_json(),
        );
        assert_eq!(
            (restored.tabs[0].active, restored.tabs[0].zoomed),
            (1, None)
        );
    }

    #[test]
    fn ratios_that_are_not_finite_are_written_as_an_even_split() {
        let mut tab = three_pane_tab();
        tab.layout = split(Axis::Row, f64::NAN, Node::Leaf(7), Node::Leaf(3));
        let restored = parse_ok(
            &Session {
                window: None,
                tabs: vec![tab],
                active_tab: 0,
            }
            .to_json(),
        );
        let Node::Split(read) = &restored.tabs[0].layout else {
            panic!("a split")
        };
        assert_eq!(read.ratio, 0.5);
    }

    #[test]
    fn broken_or_unusable_files_give_an_empty_session_and_one_problem() {
        for text in [
            "",
            "{",
            "[1, 2]",
            "null",
            "42",
            "{}",
            r#"{ "version": 0 }"#,
            r#"{ "version": "1" }"#,
            r#"{ "version": 1.5 }"#,
            r#"{ "version": -1 }"#,
        ] {
            let parsed = Session::parse(text);
            assert_eq!(parsed.session, Session::default(), "{text:?}");
            assert_eq!(parsed.problems.len(), 1, "{text:?}: {:?}", parsed.problems);
        }
    }

    #[test]
    fn a_newer_version_is_refused_instead_of_misread() {
        let parsed =
            Session::parse(r#"{ "version": 2, "tabs": [{ "layout": { "profile": "pwsh" } }] }"#);
        assert_eq!(parsed.session, Session::default());
        assert_eq!(parsed.problems.len(), 1);
        assert!(
            parsed.problems[0].contains("version"),
            "{:?}",
            parsed.problems
        );
    }

    #[test]
    fn unknown_keys_are_ignored_and_a_byte_order_mark_is_skipped() {
        let text = "\u{feff}{ \"version\": 1, \"future\": { \"a\": 1 }, \"tabs\": [ { \"extra\": true, \"layout\": { \"profile\": \"pwsh\", \"icon\": \"x\" } } ] }";
        let session = parse_ok(text);
        assert_eq!(session.tabs, vec![one_pane_tab("pwsh")]);
    }

    #[test]
    fn a_bad_tab_is_skipped_and_its_neighbours_survive() {
        let text = r#"{ "version": 1, "activeTab": 1, "tabs": [
            { "layout": { "profile": "a" } },
            { "layout": 5 },
            "not a tab",
            { "nothing": true },
            { "layout": { "profile": "b" } } ] }"#;
        let parsed = Session::parse(text);
        assert_eq!(parsed.problems.len(), 3, "{:?}", parsed.problems);
        let profiles: Vec<&str> = parsed
            .session
            .tabs
            .iter()
            .map(|tab| tab.panes.values().next().unwrap().profile.as_str())
            .collect();
        assert_eq!(profiles, ["a", "b"]);
        assert_eq!(
            parsed.session.active_tab, 1,
            "activeTab counts the tabs that were kept"
        );
        let ids: Vec<PaneId> = parsed
            .session
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .collect();
        assert_eq!(ids, [1, 2], "a skipped tab uses no ids");
    }

    #[test]
    fn tabs_that_are_not_a_list_are_reported() {
        let parsed = Session::parse(r#"{ "version": 1, "tabs": { "layout": {} } }"#);
        assert_eq!(parsed.session.tabs, vec![]);
        assert_eq!(parsed.problems.len(), 1);
    }

    #[test]
    fn the_active_tab_and_pane_must_exist() {
        let text = r#"{ "version": 1, "activeTab": 7, "tabs": [ { "active": 5, "zoomed": 9, "layout": { "axis": "row", "first": {}, "second": {} } } ] }"#;
        let parsed = Session::parse(text);
        assert_eq!(parsed.problems.len(), 3, "{:?}", parsed.problems);
        assert_eq!(parsed.session.active_tab, 0);
        let tab = &parsed.session.tabs[0];
        assert_eq!((tab.active, tab.zoomed), (1, None));
        // With no tabs at all, activeTab 0 is the only value that cannot be wrong.
        assert_eq!(
            parse_ok(r#"{ "version": 1, "activeTab": 0 }"#).active_tab,
            0
        );
        assert_eq!(problems_of(r#"{ "version": 1, "activeTab": 1 }"#).len(), 1);
    }

    #[test]
    fn ratios_are_kept_in_range_and_bad_ones_become_even() {
        for (ratio, expected) in [
            ("0.01", MIN_RATIO),
            ("5", MAX_RATIO),
            ("-3", MIN_RATIO),
            ("0.3", 0.3),
        ] {
            let text = format!(
                r#"{{ "version": 1, "tabs": [ {{ "layout": {{ "axis": "row", "ratio": {ratio}, "first": {{}}, "second": {{}} }} }} ] }}"#
            );
            let session = parse_ok(&text);
            let Node::Split(read) = &session.tabs[0].layout else {
                panic!("a split")
            };
            assert_eq!(read.ratio, expected, "ratio {ratio}");
        }
        for bad in [r#""half""#, "null", "[0.5]"] {
            let text = format!(
                r#"{{ "version": 1, "tabs": [ {{ "layout": {{ "axis": "column", "ratio": {bad}, "first": {{}}, "second": {{}} }} }} ] }}"#
            );
            let parsed = Session::parse(&text);
            assert_eq!(parsed.problems.len(), 1, "{bad}: {:?}", parsed.problems);
            let Node::Split(read) = &parsed.session.tabs[0].layout else {
                panic!("a split")
            };
            assert_eq!(read.ratio, 0.5);
        }
    }

    #[test]
    fn an_unknown_axis_becomes_a_row_and_is_reported() {
        let parsed = Session::parse(
            r#"{ "version": 1, "tabs": [ { "layout": { "axis": "diagonal", "first": {}, "second": {} } } ] }"#,
        );
        assert_eq!(parsed.problems.len(), 1);
        let Node::Split(read) = &parsed.session.tabs[0].layout else {
            panic!("a split")
        };
        assert_eq!(read.axis, Axis::Row);
    }

    #[test]
    fn a_split_that_lost_a_child_becomes_the_other_child() {
        let missing = Session::parse(
            r#"{ "version": 1, "tabs": [ { "layout": { "axis": "row", "first": { "profile": "keep" } } } ] }"#,
        );
        assert_eq!(missing.problems.len(), 1);
        assert_eq!(missing.session.tabs[0].layout, Node::Leaf(1));
        assert_eq!(missing.session.tabs[0].panes[&1].profile, "keep");

        let broken = Session::parse(
            r#"{ "version": 1, "tabs": [ { "layout": { "axis": "row", "first": 3, "second": { "profile": "keep" } } } ] }"#,
        );
        assert_eq!(broken.problems.len(), 1);
        assert_eq!(broken.session.tabs[0].layout, Node::Leaf(1));

        let both = Session::parse(
            r#"{ "version": 1, "tabs": [ { "layout": { "axis": "row", "first": 1, "second": 2 } } ] }"#,
        );
        assert_eq!(both.session.tabs, vec![]);
        assert_eq!(
            both.problems.len(),
            2,
            "one for each child: {:?}",
            both.problems
        );
    }

    #[test]
    fn panes_read_their_profile_and_directory_tolerantly() {
        let text = r#"{ "version": 1, "tabs": [ { "layout": { "axis": "column", "first": { "profile": 5, "cwd": "" }, "second": { "cwd": 7 } } } ] }"#;
        let parsed = Session::parse(text);
        assert_eq!(parsed.problems.len(), 2, "{:?}", parsed.problems);
        let panes = &parsed.session.tabs[0].panes;
        assert_eq!(
            panes[&1],
            PaneRecord::default(),
            "an empty directory is no directory"
        );
        assert_eq!(panes[&2], PaneRecord::default());
        assert_eq!(
            parse_ok(r#"{ "version": 1, "tabs": [ { "layout": {} } ] }"#).tabs[0].panes[&1],
            PaneRecord::default()
        );
    }

    #[test]
    fn a_file_cannot_start_an_unbounded_number_of_shells() {
        let tabs: Vec<String> = (0..MAX_TABS + 20)
            .map(|_| r#"{ "layout": {} }"#.to_owned())
            .collect();
        let parsed = Session::parse(&format!(
            r#"{{ "version": 1, "tabs": [{}] }}"#,
            tabs.join(",")
        ));
        assert_eq!(parsed.session.tabs.len(), MAX_TABS);
        assert_eq!(parsed.problems.len(), 1);

        let mut layout = r#"{}"#.to_owned();
        for _ in 0..MAX_PANES_PER_TAB + 10 {
            layout = format!(r#"{{ "axis": "row", "first": {{}}, "second": {layout} }}"#);
        }
        let parsed = Session::parse(&format!(
            r#"{{ "version": 1, "tabs": [ {{ "layout": {layout} }} ] }}"#
        ));
        let tab = &parsed.session.tabs[0];
        assert_eq!(tab.layout.pane_ids().len(), MAX_PANES_PER_TAB);
        assert_eq!(tab.panes.len(), MAX_PANES_PER_TAB);
        assert_eq!(parsed.problems.len(), 1, "{:?}", parsed.problems);
    }

    #[test]
    fn a_hostile_nesting_depth_is_a_parse_problem_not_a_crash() {
        let depth = 5_000;
        let layout = format!(
            "{}{{}}{}",
            r#"{ "axis": "row", "first": {}, "second": "#.repeat(depth),
            " }".repeat(depth)
        );
        let parsed = Session::parse(&format!(
            r#"{{ "version": 1, "tabs": [ {{ "layout": {layout} }} ] }}"#
        ));
        assert_eq!(parsed.session, Session::default());
        assert_eq!(parsed.problems.len(), 1);
    }

    #[test]
    fn the_window_is_read_exactly_or_dropped_with_a_report() {
        let text = |window: &str| format!(r#"{{ "version": 1, "window": {window} }}"#);
        let ok = parse_ok(&text(
            r#"{ "x": -10, "y": 20, "width": 800, "height": 600 }"#,
        ));
        assert_eq!(
            ok.window,
            Some(Placement {
                rect: PixelRect {
                    x: -10,
                    y: 20,
                    width: 800,
                    height: 600
                },
                maximized: false
            })
        );
        for bad in [
            r#"{ "x": 0, "y": 0, "width": 99, "height": 600 }"#,
            r#"{ "x": 0, "y": 0, "width": 800, "height": 100001 }"#,
            r#"{ "x": 200000, "y": 0, "width": 800, "height": 600 }"#,
            r#"{ "x": 0, "y": 0, "width": 800 }"#,
            r#"{ "x": 0.5, "y": 0, "width": 800, "height": 600 }"#,
            r#"{ "x": "0", "y": 0, "width": 800, "height": 600 }"#,
            r#"{ "x": 0, "y": 0, "width": -800, "height": 600 }"#,
            r#"[0, 0, 800, 600]"#,
        ] {
            let parsed = Session::parse(&text(bad));
            assert_eq!(parsed.session.window, None, "{bad}");
            assert_eq!(parsed.problems.len(), 1, "{bad}: {:?}", parsed.problems);
        }
        let flag = Session::parse(&text(
            r#"{ "x": 0, "y": 0, "width": 800, "height": 600, "maximized": "yes" }"#,
        ));
        assert_eq!(flag.problems.len(), 1);
        assert_eq!(flag.session.window.map(|w| w.maximized), Some(false));
    }

    fn area(x: i32, y: i32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    fn placed(x: i32, y: i32, width: u32, height: u32) -> Placement {
        Placement {
            rect: area(x, y, width, height),
            maximized: false,
        }
    }

    #[test]
    fn a_window_is_reachable_when_enough_of_its_title_strip_is_on_a_screen() {
        let screen = [area(0, 0, 1920, 1080)];
        assert!(placed(100, 100, 800, 600).is_reachable_on(&screen));
        assert!(placed(0, 0, 1920, 1080).is_reachable_on(&screen));
        assert!(
            !placed(3000, 100, 800, 600).is_reachable_on(&screen),
            "wholly beside the screen"
        );
        assert!(
            !placed(100, 1200, 800, 600).is_reachable_on(&screen),
            "wholly below it"
        );
        assert!(
            !placed(-30000, -30000, 160, 160).is_reachable_on(&screen),
            "the parking spot of a minimized window"
        );
        assert!(
            !placed(100, -30, 800, 600).is_reachable_on(&screen),
            "title strip mostly above the top edge"
        );
        assert!(
            placed(100, -10, 800, 600).is_reachable_on(&screen),
            "30 of 40 pixels still show"
        );
        assert!(
            !placed(-750, 100, 800, 600).is_reachable_on(&screen),
            "only 50 pixels of width on screen"
        );
        assert!(placed(-700, 100, 800, 600).is_reachable_on(&screen));
        assert!(
            !placed(100, 100, 800, 600).is_reachable_on(&[]),
            "no screen at all"
        );
    }

    #[test]
    fn a_window_on_a_second_monitor_is_reachable_while_that_monitor_exists() {
        let monitors = [area(0, 0, 1920, 1080), area(-1920, 0, 1920, 1080)];
        let on_second = placed(-1800, 50, 1200, 800);
        assert!(on_second.is_reachable_on(&monitors));
        assert!(!on_second.is_reachable_on(&monitors[..1]));
        let across = placed(-600, 50, 1200, 800);
        assert!(
            across.is_reachable_on(&monitors[..1]),
            "600 pixels of the strip are on the first monitor"
        );
        assert!(
            across.is_reachable_on(&monitors[1..]),
            "and 600 on the second"
        );
    }

    #[test]
    fn the_file_of_a_panes_text_round_trips_and_a_session_without_one_reads_as_before() {
        let mut tab = three_pane_tab();
        tab.panes.get_mut(&3).unwrap().buffer = Some("1761400000000-2-3.vt".into());
        let json = Session { window: None, tabs: vec![tab], active_tab: 0 }.to_json();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["tabs"][0]["layout"]["second"]["first"], json!({ "profile": "cmd", "buffer": "1761400000000-2-3.vt" }));
        assert_eq!(value["tabs"][0]["layout"]["first"].get("buffer"), None, "a pane without text writes no key");
        let restored = parse_ok(&json);
        assert_eq!(restored.tabs[0].panes[&2].buffer.as_deref(), Some("1761400000000-2-3.vt"), "ids are renumbered in tree order");

        // What 0.0.23 wrote, with no buffer anywhere.
        let old = r#"{ "version": 1, "activeTab": 0, "tabs": [ { "active": 0, "layout": { "profile": "pwsh", "cwd": "C:\\a" } } ] }"#;
        assert_eq!(parse_ok(old).tabs[0].panes[&1], pane("pwsh", Some("C:\\a")));
    }

    #[test]
    fn a_buffer_that_is_not_a_plain_file_name_is_dropped_with_a_report() {
        for bad in [r#""..\\..\\secrets.vt""#, r#""C:\\x.vt""#, r#""a/b.vt""#, r#""x.txt""#, r#"".vt""#, "5", r#""a b.vt""#] {
            let parsed = Session::parse(&format!(r#"{{ "version": 1, "tabs": [ {{ "layout": {{ "profile": "p", "buffer": {bad} }} }} ] }}"#));
            assert_eq!(parsed.problems.len(), 1, "{bad}: {:?}", parsed.problems);
            assert_eq!(parsed.session.tabs[0].panes[&1], pane("p", None), "{bad}");
        }
        assert!(is_buffer_name("1761400000000-12-3.vt"));
        assert!(!is_buffer_name(&format!("{}.vt", "a".repeat(65))));
    }

    #[test]
    fn a_null_zoomed_pane_means_no_zoom() {
        let session = parse_ok(
            r#"{ "version": 1, "tabs": [ { "zoomed": null, "active": null, "layout": { "axis": "row", "first": {}, "second": {} } } ] }"#,
        );
        assert_eq!((session.tabs[0].active, session.tabs[0].zoomed), (1, None));
    }
}
