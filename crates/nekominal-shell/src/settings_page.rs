//! The settings page: a strip of tabs, one for each section, with the search line at its right, and the rows of the
//! section below it.
//!
//! The page is a display list. `build` turns the state of the page and the settings into rectangles with what is drawn
//! in them and what they do when pressed (`Part`), so that drawing and hit testing read the same geometry. The rows are
//! those of `settings_ui::TABLE`; nothing here knows what a setting means. Pure apart from the text measure the caller
//! hands in.

use nekominal_model::{
    commands::Command,
    keymap::Keymap,
    profiles::Profile,
    settings::Settings,
    settings_ui::{self, Control, Descriptor, Range, Section, Value},
    strings::{Lang, Text},
    themes,
};
use nekominal_ui::Rect;

use crate::{
    logic::{Mark, profile_mark},
    design::{GAP, INSET, MARGIN},
    popup::MARK_GAP,
    widgets::{MARK_SIZE, THUMB},
};

/// The strip on top of the page: its height, where its tabs start below its top, and the room each tab leaves around its
/// text, in DIP. The search line and the button for the file are at its right.
pub const STRIP: f32 = 44.0;
const TAB_TOP: f32 = 8.0;
const TAB_PAD: f32 = MARGIN;
const STRIP_EDGE: f32 = INSET;
const SEARCH_WIDTH: f32 = 220.0;
const SEARCH_HEIGHT: f32 = 28.0;
const PAD: f32 = 24.0;
const CONTENT_MAX: f32 = 720.0;
const CONTENT_TOP: f32 = STRIP + 20.0;
const ROW: f32 = 52.0;
const ROW_GAP: f32 = 4.0;
const CONTROL: f32 = 28.0;
const SWITCH: (f32, f32) = (44.0, 24.0);
const SLIDER_WIDTH: f32 = 200.0;
const FIELD_WIDTH: f32 = 280.0;
const TILE: (f32, f32) = (148.0, 64.0);
const TILE_GAP: f32 = 2.0 * GAP;
const RADIO_ROW: f32 = 36.0;
const RESET_ROOM: f32 = 32.0;
/// The row of an agent on the Agent page: its logo, its name and version, and a line that says how it is connected.
const CLAUDE_ROW: f32 = 68.0;
/// How much taller a row is when its control goes under its label.
const STACK: f32 = 48.0;

/// What the page remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct Page {
    /// The index of the section shown in `Section::ALL`; a query shows what it finds in every section instead.
    pub section: usize,
    pub query: String,
    /// How far the rows are scrolled up, in DIP.
    pub scroll: f32,
    /// The row with the keyboard, as an index into the entries on show.
    pub focus: Option<usize>,
    pub hover: Option<Part>,
    /// The slider held by the pointer.
    pub drag: Option<&'static str>,
    /// The text field being typed in: the id of its row and what it holds so far.
    pub editing: Option<(&'static str, String)>,
    /// The command whose shortcut the next chord pressed becomes, and whether the last chord was one a program needs.
    pub capturing: Option<Command>,
    pub reserved: bool,
    /// The caret and the selection of the search line, and of the text field being edited.
    pub cursor: crate::textfield::Cursor,
    pub edit_cursor: crate::textfield::Cursor,
}

/// What a rectangle does when it is pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Nav(usize),
    Search,
    /// Opens the settings file.
    File,
    Toggle(&'static str),
    Option(&'static str, usize),
    Minus(&'static str),
    Plus(&'static str),
    Slider(&'static str),
    Field(&'static str),
    /// The color scheme at this index of `themes::theme_names`.
    Tile(usize),
    /// The profile at this index.
    Profile(usize),
    Reset(&'static str),
    /// The row of a command on the shortcuts page: pressing it waits for the new shortcut.
    Binding(Command),
    /// Puts the shortcuts of a command back to the default.
    ResetKey(Command),
    /// Connects Claude Code's status line to the status bar, or disconnects it.
    Claude,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Text,
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// The band behind the tabs, darker than the page; it leaves the selected tab out.
    Strip,
    /// A tab of the strip: the selected one is of the colour of the page and joins it.
    Tab { selected: bool, hover: bool },
    Row { focus: bool },
    Button { selected: bool, hover: bool },
    Field { editing: bool },
    Switch { on: bool, hover: bool },
    /// A slider, its thumb `share` of the way (0 to 1) along the track; the rectangle is the one that takes the presses.
    Slider { share: f32 },
    Radio { on: bool, hover: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Visual {
    None,
    Plate(Fill),
    Label { text: String, size: f32, tone: Tone, bold: bool, align: Align },
    Glyph { ch: char, size: f32, tone: Tone },
    /// The logo of a shell, before the name of a profile.
    Mark(Mark),
    /// A line of text in a field, with its caret and selection when it is the one being edited (`cursor`), or what an empty
    /// one says (`placeholder`).
    Edit { text: String, cursor: Option<crate::textfield::Cursor>, placeholder: String },
    /// A color scheme: its background with its name and a few of its colours.
    Tile { scheme: usize, selected: bool, hover: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub rect: Rect,
    pub part: Option<Part>,
    pub visual: Visual,
    /// The rectangle moves with the scrolling and is cut off at the rows' area.
    pub scrolls: bool,
}

/// A row on show.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    Setting(&'static Descriptor),
    Key(Command),
    /// Claude Code: where it is and whether its status line goes to the status bar.
    Claude,
}

pub struct Frame {
    pub items: Vec<Item>,
    /// Where the rows show, in the coordinates of the window.
    pub viewport: Rect,
    /// The height of all the rows together.
    pub content_height: f32,
    /// The rectangle of each entry, scrolled to the top.
    pub rows: Vec<Rect>,
    pub entries: Vec<Entry>,
}

pub struct View<'a> {
    pub lang: Lang,
    pub settings: &'a Settings,
    pub keymap: &'a Keymap,
    pub profiles: &'a [Profile],
    pub claude: &'a crate::agent::Claude,
    /// The area of the page in the coordinates of the window, in DIP.
    pub area: Rect,
}

impl Frame {
    /// What is under a point of the window.
    pub fn part_at(&self, scroll: f32, x: f32, y: f32) -> Option<Part> {
        self.items.iter().rev().find_map(|item| {
            let part = item.part?;
            let rect = self.on_screen(item, scroll);
            let visible = !item.scrolls || self.viewport.contains(x, y);
            (visible && rect.contains(x, y)).then_some(part)
        })
    }

    pub fn on_screen(&self, item: &Item, scroll: f32) -> Rect {
        if item.scrolls { Rect::new(item.rect.x, item.rect.y - scroll, item.rect.w, item.rect.h) } else { item.rect }
    }

    /// The scroll offset that shows entry `i` whole, moving as little as possible from `scroll`.
    pub fn reveal(&self, i: usize, scroll: f32) -> f32 {
        let Some(row) = self.rows.get(i) else { return scroll };
        let top = row.y - self.viewport.y;
        let bottom = top + row.h;
        let view = self.viewport.h;
        if top < scroll {
            top
        } else if bottom > scroll + view {
            bottom - view
        } else {
            scroll
        }
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.viewport.h).max(0.0)
    }
}

/// The rows on show: the matches of the query, or the section.
pub fn entries(page: &Page, lang: Lang, keymap: &Keymap) -> Vec<Entry> {
    if !page.query.trim().is_empty() {
        return settings_ui::search(&page.query, lang, keymap)
            .into_iter()
            .map(|row| match row {
                settings_ui::Row::Setting(descriptor) => Entry::Setting(descriptor),
                settings_ui::Row::Key(command) => Entry::Key(command),
            })
            .collect();
    }
    let section = Section::ALL[page.section.min(Section::ALL.len() - 1)];
    if section == Section::Keys {
        settings_ui::key_rows().map(Entry::Key).collect()
    } else {
        // The Agent page starts with the agents it knows, before the rows of the table.
        let agents = (section == Section::Agents).then_some(Entry::Claude);
        agents.into_iter().chain(settings_ui::in_section(section).map(Entry::Setting)).collect()
    }
}

/// What a row says at its right: the option, the number, the name of the scheme or profile.
pub fn value_text(row: &Descriptor, settings: &Settings, profiles: &[Profile], lang: Lang) -> String {
    match (row.control, (row.get)(settings)) {
        (Control::Toggle, Value::Bool(on)) => (if on { Text::ToggleOn } else { Text::ToggleOff }).get(lang).to_owned(),
        (Control::Choice(options), Value::Index(i)) => options.get(i).map(|text| text.get(lang).to_owned()).unwrap_or_default(),
        (Control::Slider(_), Value::Number(n)) => format!("{n:.0}%"),
        (Control::Number(range), Value::Number(n)) => number_text(range, n),
        (Control::Profiles, Value::Text(id)) => profiles.iter().find(|p| p.id == id).map_or(id, |p| p.name.clone()),
        (_, Value::Text(text)) => text,
        _ => String::new(),
    }
}

fn number_text(range: Range, n: f64) -> String {
    if range.step.fract() == 0.0 { format!("{n:.0}") } else { format!("{n:.2}") }
}

/// The value one step (or one large step) after or before the one the row has.
pub fn stepped(row: &Descriptor, settings: &Settings, profiles: &[Profile], forward: bool, large: bool) -> Option<Value> {
    let step: isize = if forward { 1 } else { -1 };
    let cycle = |len: usize, at: usize| (at as isize + step).rem_euclid(len.max(1) as isize) as usize;
    Some(match (row.control, (row.get)(settings)) {
        (Control::Toggle, Value::Bool(on)) => Value::Bool(!on),
        (Control::Choice(options), Value::Index(i)) => Value::Index(cycle(options.len(), i)),
        (Control::Slider(range) | Control::Number(range), Value::Number(n)) => {
            let amount = if large { range.large } else { range.step };
            // Rounded to hundredths so that repeated steps of 0.05 do not drift.
            let moved = (n + step as f64 * amount).clamp(range.min, range.max);
            Value::Number((moved * 100.0).round() / 100.0)
        }
        (Control::Themes, Value::Text(name)) => {
            let names: Vec<&str> = themes::theme_names().collect();
            let at = names.iter().position(|n| *n == name).unwrap_or(0);
            Value::Text(names[cycle(names.len(), at)].to_owned())
        }
        (Control::Profiles, Value::Text(id)) => {
            let at = profiles.iter().position(|p| p.id == id).unwrap_or(0);
            Value::Text(profiles.get(cycle(profiles.len(), at))?.id.clone())
        }
        _ => return None,
    })
}

pub use crate::widgets::{slider_track, slider_value};

/// Where options of these widths go when they may not be wider together than room (with gap between them): the line of
/// each and how far it is from the left.
fn wrap(widths: &[f32], room: f32, gap: f32) -> Vec<(usize, f32)> {
    let (mut line, mut at) = (0, 0.0);
    widths
        .iter()
        .map(|width| {
            if at > 0.0 && at + width > room {
                line += 1;
                at = 0.0;
            }
            let place = (line, at);
            at += width + gap;
            place
        })
        .collect()
}

fn item(items: &mut Vec<Item>, rect: Rect, part: Option<Part>, visual: Visual, scrolls: bool) {
    items.push(Item { rect, part, visual, scrolls });
}

fn label(text: impl Into<String>, size: f32, tone: Tone, bold: bool, align: Align) -> Visual {
    Visual::Label { text: text.into(), size, tone, bold, align }
}

pub fn build(page: &Page, view: &View, measure: &mut dyn FnMut(&str, f32, bool) -> f32) -> Frame {
    let lang = view.lang;
    let Rect { x: ox, y: oy, w: width, h: height } = view.area;
    let mut items = Vec::new();
    let hover = page.hover;

    // The strip: the sections as tabs side by side, as tall as the strip but for the space above them.
    let searching = !page.query.trim().is_empty();
    item(&mut items, Rect::new(ox, oy, width, STRIP), None, Visual::Plate(Fill::Strip), false);
    let mut tabs_end = ox + STRIP_EDGE;
    for (i, section) in Section::ALL.iter().enumerate() {
        let text = section.text().get(lang);
        let rect = Rect::new(tabs_end, oy + TAB_TOP, measure(text, 13.0, false) + 2.0 * TAB_PAD, STRIP - TAB_TOP);
        let selected = !searching && i == page.section;
        let part = Part::Nav(i);
        item(&mut items, rect, Some(part), Visual::Plate(Fill::Tab { selected, hover: hover == Some(part) }), false);
        let tone = if selected { Tone::Text } else { Tone::Quiet };
        item(&mut items, rect, None, label(text, 13.0, tone, false, Align::Center), false);
        tabs_end = rect.x + rect.w;
    }

    // The search line and, left of it, the button for the file: what room the tabs leave.
    let search_width = (ox + width - STRIP_EDGE - (tabs_end + 12.0)).clamp(0.0, SEARCH_WIDTH);
    let search = Rect::new(ox + width - STRIP_EDGE - search_width, oy + TAB_TOP + (STRIP - TAB_TOP - SEARCH_HEIGHT) / 2.0, search_width, SEARCH_HEIGHT);
    let file_text = Text::SettingsFile.get(lang);
    let file_width = measure(file_text, 13.0, false) + 24.0;
    let file = Rect::new(search.x - 8.0 - file_width, search.y, file_width, SEARCH_HEIGHT);
    if file.x >= tabs_end + 12.0 {
        item(&mut items, file, Some(Part::File), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(Part::File) }), false);
        item(&mut items, file, None, label(file_text, 13.0, Tone::Text, false, Align::Center), false);
    }

    let content_width = (width - 2.0 * PAD).clamp(0.0, CONTENT_MAX);
    let x0 = ox + (width - content_width) / 2.0;
    item(&mut items, search, Some(Part::Search), Visual::Plate(Fill::Field { editing: page.editing.is_none() }), false);
    item(&mut items, Rect::new(search.x + 12.0, search.y, 20.0, search.h), None, Visual::Glyph { ch: '\u{E721}', size: 14.0, tone: Tone::Quiet }, false);
    let text_x = search.x + 40.0;
    let edit = Visual::Edit { text: page.query.clone(), cursor: page.editing.is_none().then_some(page.cursor), placeholder: Text::SettingsSearch.get(lang).to_owned() };
    item(&mut items, Rect::new(text_x, search.y, (search.w - 52.0).max(0.0), search.h), None, edit, false);

    // The rows.
    let entries = entries(page, lang, view.keymap);
    let top = oy + CONTENT_TOP;
    let viewport = Rect::new(ox, top, width, (oy + height - top).max(0.0));
    let mut rows = Vec::new();
    let mut y = top;
    if entries.is_empty() {
        item(&mut items, Rect::new(x0, y, content_width, ROW), None, label(Text::SettingsEmpty.get(lang), 13.0, Tone::Quiet, false, Align::Left), true);
        y += ROW;
    }
    let theme_names: Vec<&str> = themes::theme_names().collect();
    for (n, entry) in entries.iter().enumerate() {
        let focus = page.focus == Some(n);
        let start = y;
        match *entry {
            Entry::Key(command) => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                let capturing = page.capturing == Some(command);
                item(&mut items, rect, Some(Part::Binding(command)), Visual::Plate(Fill::Row { focus: focus || capturing }), true);
                item(&mut items, Rect::new(rect.x + MARGIN, rect.y, (rect.w - 2.0 * MARGIN) / 2.0, rect.h), None, label(command.label(lang), 13.0, Tone::Text, false, Align::Left), true);
                let mut right = rect.x + rect.w - MARGIN - RESET_ROOM;
                if nekominal_model::keybindings::is_modified(&view.settings.keybindings, command) {
                    let reset = Rect::new(rect.x + rect.w - MARGIN - 24.0, rect.y + (rect.h - 24.0) / 2.0, 24.0, 24.0);
                    let part = Part::ResetKey(command);
                    item(&mut items, reset, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, reset, None, Visual::Glyph { ch: '\u{E777}', size: 11.0, tone: Tone::Quiet }, true);
                }
                if capturing {
                    let text = if page.reserved { Text::KeyReserved } else { Text::KeysPress };
                    item(&mut items, Rect::new(rect.x + rect.w / 2.0, rect.y, rect.w / 2.0 - MARGIN - RESET_ROOM, rect.h), None, label(text.get(lang), 12.0, Tone::Quiet, false, Align::Right), true);
                    y += ROW;
                    rows.push(Rect::new(x0, start, content_width, y - start));
                    continue;
                }
                for shortcut in view.keymap.shortcuts(command).iter().rev() {
                    let text = shortcut.to_string();
                    let chip = Rect::new(right - measure(&text, 12.0, false) - 20.0, rect.y + (rect.h - 24.0) / 2.0, measure(&text, 12.0, false) + 20.0, 24.0);
                    item(&mut items, chip, None, Visual::Plate(Fill::Button { selected: false, hover: false }), true);
                    item(&mut items, chip, None, label(text, 12.0, Tone::Quiet, false, Align::Center), true);
                    right = chip.x - GAP;
                }
                y += ROW;
            }
            Entry::Claude => {
                let claude = view.claude;
                let rect = Rect::new(x0, y, content_width, CLAUDE_ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                let mark = Rect::new(rect.x + MARGIN, rect.y + (rect.h - 28.0) / 2.0, 28.0, 28.0);
                item(&mut items, mark, None, Visual::Mark(Mark::Badge { label: "C", color: (217, 119, 87) }), true);
                let text_x = mark.x + 28.0 + 2.0 * MARK_GAP;
                // A button to connect or disconnect when there is a Claude Code and nobody else's status line is in the way.
                let action = match claude.link {
                    _ if claude.exe.is_none() => None,
                    crate::agent::Link::Linked => Some(Text::ClaudeUnlink),
                    crate::agent::Link::Unlinked => Some(Text::ClaudeLink),
                    crate::agent::Link::Foreign => None,
                };
                let mut text_end = rect.x + rect.w - MARGIN;
                if let Some(action) = action {
                    let w = measure(action.get(lang), 13.0, false) + 32.0;
                    let button = Rect::new(text_end - w, rect.y + (rect.h - CONTROL) / 2.0, w, CONTROL);
                    item(&mut items, button, Some(Part::Claude), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(Part::Claude) }), true);
                    item(&mut items, button, None, label(action.get(lang), 13.0, Tone::Text, false, Align::Center), true);
                    text_end = button.x - 2.0 * GAP;
                }
                let room = (text_end - text_x).max(0.0);
                let title = match &claude.version {
                    Some(version) => format!("Claude Code  {version}"),
                    None => "Claude Code".to_owned(),
                };
                item(&mut items, Rect::new(text_x, rect.y + 8.0, room, 22.0), None, label(title, 13.0, Tone::Text, true, Align::Left), true);
                let note = match claude.link {
                    _ if claude.exe.is_none() => Text::ClaudeMissing,
                    crate::agent::Link::Linked => Text::ClaudeLinked,
                    crate::agent::Link::Unlinked => Text::ClaudeUnlinked,
                    crate::agent::Link::Foreign => Text::ClaudeForeign,
                };
                item(&mut items, Rect::new(text_x, rect.y + rect.h - 30.0, room, 22.0), None, label(note.get(lang), 12.0, Tone::Quiet, false, Align::Left), true);
                y += CLAUDE_ROW;
            }
            Entry::Setting(row) => {
                let id = row.id;
                // A control that does not fit beside the label goes on a line of its own under it.
                let label_width = measure(row.label(lang), 13.0, false);
                let control_width = match row.control {
                    Control::Toggle => SWITCH.0,
                    Control::Choice(options) => options.iter().map(|option| measure(option.get(lang), 13.0, false) + 30.0).sum::<f32>() - GAP,
                    Control::Slider(_) => 48.0 + 12.0 + SLIDER_WIDTH,
                    Control::Number(_) => 2.0 * CONTROL + 72.0,
                    Control::Text => FIELD_WIDTH,
                    Control::Themes | Control::Profiles => 0.0,
                };
                let stacked = control_width > 0.0 && label_width + control_width + 48.0 + RESET_ROOM > content_width;
                let drop = if stacked { STACK } else { 0.0 };
                // What fits on the line of the control under the label: the options of a choice go on more lines when they do not.
                let room = content_width - 32.0 - RESET_ROOM;
                let option_widths: Vec<f32> = match row.control {
                    Control::Choice(options) => options.iter().map(|option| measure(option.get(lang), 13.0, false) + 24.0).collect(),
                    _ => Vec::new(),
                };
                let option_lines = if stacked { wrap(&option_widths, room, GAP) } else { Vec::new() };
                let lines = option_lines.iter().map(|(line, _)| *line + 1).max().unwrap_or(1);
                let extra = (lines - 1) as f32 * (CONTROL + GAP);
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP + drop + extra);
                let line = Rect::new(rect.x, rect.y + drop, rect.w, ROW - ROW_GAP);
                let mid = line.y + (line.h - CONTROL) / 2.0;
                let right = rect.x + rect.w - MARGIN - RESET_ROOM;
                let value = (row.get)(view.settings);
                // A toggle is pressed anywhere on its row.
                let row_part = matches!(row.control, Control::Toggle).then_some(Part::Toggle(id));
                item(&mut items, rect, row_part, Visual::Plate(Fill::Row { focus }), true);
                let label_room = if stacked { rect.w - 32.0 - RESET_ROOM } else { (rect.w - 32.0 - RESET_ROOM) / 2.0 };
                item(&mut items, Rect::new(rect.x + MARGIN, rect.y, label_room, ROW - ROW_GAP), None, label(row.label(lang), 13.0, Tone::Text, false, Align::Left), true);
                if row.is_modified(view.settings) {
                    let reset = Rect::new(rect.x + rect.w - MARGIN - 24.0, rect.y + (ROW - ROW_GAP - 24.0) / 2.0, 24.0, 24.0);
                    let part = Part::Reset(id);
                    item(&mut items, reset, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, reset, None, Visual::Glyph { ch: '\u{E777}', size: 11.0, tone: Tone::Quiet }, true);
                }
                match (row.control, value) {
                    (Control::Toggle, Value::Bool(on)) => {
                        let switch = Rect::new(right - SWITCH.0, line.y + (line.h - SWITCH.1) / 2.0, SWITCH.0, SWITCH.1);
                        item(&mut items, switch, None, Visual::Plate(Fill::Switch { on, hover: hover == Some(Part::Toggle(id)) }), true);
                        y += ROW + drop;
                    }
                    (Control::Choice(options), Value::Index(chosen)) => {
                        let mut at = right;
                        for (i, option) in options.iter().enumerate().rev() {
                            let text = option.get(lang);
                            let w = option_widths[i];
                            let button = match option_lines.get(i) {
                                Some((line, offset)) => Rect::new(rect.x + MARGIN + offset, mid + *line as f32 * (CONTROL + GAP), w, CONTROL),
                                None => Rect::new(at - w, mid, w, CONTROL),
                            };
                            let part = Part::Option(id, i);
                            item(&mut items, button, Some(part), Visual::Plate(Fill::Button { selected: i == chosen, hover: hover == Some(part) }), true);
                            item(&mut items, button, None, label(text, 13.0, if i == chosen { Tone::Text } else { Tone::Quiet }, false, Align::Center), true);
                            at = button.x - GAP;
                        }
                        y += ROW + drop + extra;
                    }
                    (Control::Slider(range), Value::Number(n)) => {
                        let value_box = Rect::new(right - 48.0, line.y, 48.0, line.h);
                        item(&mut items, value_box, None, label(format!("{n:.0}%"), 13.0, Tone::Quiet, false, Align::Right), true);
                        let track_width = SLIDER_WIDTH.min(room - 60.0).max(60.0);
                        let share = (((n - range.min) / (range.max - range.min)) as f32).clamp(0.0, 1.0);
                        let track_x = value_box.x - 12.0 - track_width;
                        item(&mut items, Rect::new(track_x - THUMB / 2.0, line.y, track_width + THUMB, line.h), Some(Part::Slider(id)), Visual::Plate(Fill::Slider { share }), true);
                        y += ROW + drop;
                    }
                    (Control::Number(range), Value::Number(n)) => {
                        let plus = Rect::new(right - CONTROL, mid, CONTROL, CONTROL);
                        let value_box = Rect::new(plus.x - 72.0, mid, 72.0, CONTROL);
                        let minus = Rect::new(value_box.x - CONTROL, mid, CONTROL, CONTROL);
                        for (button, part, ch) in [(minus, Part::Minus(id), '\u{E738}'), (plus, Part::Plus(id), '\u{E710}')] {
                            item(&mut items, button, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                            item(&mut items, button, None, Visual::Glyph { ch, size: 11.0, tone: Tone::Text }, true);
                        }
                        item(&mut items, value_box, None, label(number_text(range, n), 13.0, Tone::Text, false, Align::Center), true);
                        y += ROW + drop;
                    }
                    (Control::Text, Value::Text(text)) => {
                        let field_width = FIELD_WIDTH.min(room);
                        let field = Rect::new(right - field_width, mid, field_width, CONTROL);
                        let editing = page.editing.as_ref().filter(|(edit_id, _)| *edit_id == id).map(|(_, text)| text.clone());
                        item(&mut items, field, Some(Part::Field(id)), Visual::Plate(Fill::Field { editing: editing.is_some() }), true);
                        let inner = Rect::new(field.x + 10.0, field.y, field.w - 20.0, field.h);
                        if let Some(text) = editing {
                            item(&mut items, inner, None, Visual::Edit { text, cursor: Some(page.edit_cursor), placeholder: String::new() }, true);
                        } else {
                            item(&mut items, inner, None, label(text, 13.0, Tone::Quiet, false, Align::Left), true);
                        }
                        y += ROW + drop;
                    }
                    (Control::Themes, Value::Text(current)) => {
                        y += ROW;
                        let columns = (((content_width + TILE_GAP) / (TILE.0 + TILE_GAP)).floor() as usize).max(1);
                        for (i, name) in theme_names.iter().enumerate() {
                            let (col, line) = (i % columns, i / columns);
                            let tile = Rect::new(x0 + col as f32 * (TILE.0 + TILE_GAP), y + line as f32 * (TILE.1 + TILE_GAP), TILE.0, TILE.1);
                            let part = Part::Tile(i);
                            item(&mut items, tile, Some(part), Visual::Tile { scheme: i, selected: *name == current, hover: hover == Some(part) }, true);
                        }
                        y += theme_names.len().div_ceil(columns) as f32 * (TILE.1 + TILE_GAP);
                    }
                    (Control::Profiles, Value::Text(current)) => {
                        y += ROW;
                        let chosen = view.profiles.iter().position(|p| p.id == current).unwrap_or(0);
                        for (i, profile) in view.profiles.iter().enumerate() {
                            let line = Rect::new(x0, y, content_width, RADIO_ROW - ROW_GAP);
                            let part = Part::Profile(i);
                            item(&mut items, line, Some(part), Visual::Plate(Fill::Row { focus: false }), true);
                            let radio = Rect::new(line.x + MARGIN, line.y + (line.h - 16.0) / 2.0, 16.0, 16.0);
                            item(&mut items, radio, None, Visual::Plate(Fill::Radio { on: i == chosen, hover: hover == Some(part) }), true);
                            // The logo of the shell, then its name.
                            let mark = Rect::new(line.x + 44.0, line.y + (line.h - MARK_SIZE) / 2.0, MARK_SIZE, MARK_SIZE);
                            item(&mut items, mark, None, Visual::Mark(profile_mark(profile.icon)), true);
                            let name_x = mark.x + MARK_SIZE + MARK_GAP;
                            item(&mut items, Rect::new(name_x, line.y, (line.x + line.w - MARGIN - name_x).max(0.0), line.h), None, label(profile.name.as_str(), 13.0, Tone::Text, false, Align::Left), true);
                            y += RADIO_ROW;
                        }
                    }
                    _ => y += ROW,
                }
            }
        }
        rows.push(Rect::new(x0, start, content_width, y - start));
    }
    Frame { items, viewport, content_height: y - top + PAD, rows, entries }
}

#[cfg(test)]
mod tests {
    use nekominal_model::{commands::default_bindings, keymap::build_keymap, settings_ui::Section};

    use super::*;

    const CLAUDE: crate::agent::Claude = crate::agent::Claude { exe: None, version: None, link: crate::agent::Link::Unlinked };

    fn keymap() -> Keymap {
        build_keymap(default_bindings(), &Default::default()).keymap
    }

    fn frame(page: &Page, settings: &Settings) -> Frame {
        let keymap = keymap();
        let view = View { lang: Lang::EnUs, settings, keymap: &keymap, profiles: &[], claude: &CLAUDE, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
        build(page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5)
    }

    fn row(id: &str) -> &'static Descriptor {
        settings_ui::find(id).expect("the row exists")
    }

    #[test]
    fn a_section_shows_exactly_its_rows_and_the_shortcuts_page_shows_the_commands() {
        let settings = Settings::default();
        for (i, section) in Section::ALL.iter().enumerate() {
            let page = Page { section: i, ..Page::default() };
            let shown = frame(&page, &settings).entries.len();
            // The Agent page starts with the row of Claude Code.
            let agents = usize::from(*section == Section::Agents);
            let expected = if *section == Section::Keys { settings_ui::key_rows().count() } else { settings_ui::in_section(*section).count() + agents };
            assert_eq!(shown, expected, "{section:?}");
        }
    }

    #[test]
    fn a_query_finds_rows_across_the_sections_and_a_nonsense_query_finds_none() {
        let settings = Settings::default();
        let found = frame(&Page { query: "opacity".into(), ..Page::default() }, &settings);
        assert!(found.entries.iter().any(|e| matches!(e, Entry::Setting(row) if row.id == "appearance.opacity")));
        assert!(frame(&Page { query: "zzzzqqq".into(), ..Page::default() }, &settings).entries.is_empty());
    }

    #[test]
    fn every_control_is_reachable_by_a_part_inside_the_rows_area() {
        let settings = Settings::default();
        for i in 0..Section::ALL.len() {
            let page = Page { section: i, ..Page::default() };
            let frame = frame(&page, &settings);
            for entry in &frame.entries {
                let Entry::Setting(row) = entry else { continue };
                let part = frame.items.iter().filter_map(|item| item.part).any(|part| match part {
                    Part::Toggle(id) | Part::Option(id, _) | Part::Plus(id) | Part::Slider(id) | Part::Field(id) => id == row.id,
                    Part::Tile(_) => matches!(row.control, Control::Themes),
                    // The profile rows come from the profiles found at run time; there are none in this test.
                    Part::Profile(_) => false,
                    _ => false,
                });
                assert!(part || matches!(row.control, Control::Profiles), "{} has nothing to press", row.id);
            }
        }
    }

    #[test]
    fn the_sections_are_tabs_side_by_side_and_the_one_shown_is_the_one_selected() {
        let settings = Settings::default();
        let tab_items = |frame: &Frame| -> Vec<Item> { frame.items.iter().filter(|item| matches!(item.part, Some(Part::Nav(_)))).cloned().collect() };
        let selected = |items: &[Item]| -> Vec<usize> {
            items.iter().enumerate().filter(|(_, item)| matches!(item.visual, Visual::Plate(Fill::Tab { selected: true, .. }))).map(|(i, _)| i).collect()
        };
        let shown = tab_items(&frame(&Page { section: 2, ..Page::default() }, &settings));
        assert_eq!(shown.len(), Section::ALL.len());
        assert!(shown.windows(2).all(|pair| (pair[0].rect.x + pair[0].rect.w - pair[1].rect.x).abs() < 0.01 && pair[0].rect.y == pair[1].rect.y), "side by side");
        assert_eq!(selected(&shown), vec![2]);
        // What a search finds comes from every section, so no tab is the one shown.
        let found = tab_items(&frame(&Page { query: "opacity".into(), ..Page::default() }, &settings));
        assert!(selected(&found).is_empty());
    }

    #[test]
    fn the_search_line_and_the_file_button_stay_in_the_strip_and_clear_of_the_tabs() {
        let settings = Settings::default();
        let keymap = keymap();
        for width in [crate::settings_window::MIN_SIZE.0, 900.0, 1400.0] {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], claude: &CLAUDE, area: Rect::new(0.0, 0.0, width, 650.0) };
            let frame = build(&Page::default(), &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.55);
            let tabs_end = frame.items.iter().filter(|item| matches!(item.part, Some(Part::Nav(_)))).map(|item| item.rect.x + item.rect.w).fold(0.0, f32::max);
            // The search line is always there; the button for the file goes when the tabs leave no room for it.
            let search = frame.items.iter().find(|item| item.part == Some(Part::Search)).expect("the search line is there");
            assert!(search.rect.w >= 100.0, "the search line is too narrow to use at {width}");
            for part in [Part::Search, Part::File] {
                let Some(item) = frame.items.iter().find(|item| item.part == Some(part)) else { continue };
                assert!(item.rect.x >= tabs_end - 0.01, "{part:?} runs into the tabs at {width}");
                assert!(item.rect.x + item.rect.w <= width + 0.01 && item.rect.y + item.rect.h <= STRIP, "{part:?} sticks out of the strip at {width}");
            }
            if width >= 1400.0 {
                assert!(frame.items.iter().any(|item| item.part == Some(Part::File)), "the button for the file has room at {width}");
            }
        }
    }

    #[test]
    fn a_profile_shows_the_logo_of_its_shell_between_the_radio_and_its_name() {
        use nekominal_model::profiles::ProfileKind;
        let profile = |id: &str, name: &str, icon: &'static str| Profile { id: id.into(), name: name.into(), kind: ProfileKind::Other, icon, program: String::new(), args: Vec::new(), cwd: None, custom: false };
        let profiles = [profile("a", "PowerShell", "pwsh"), profile("b", "Ubuntu", "wsl"), profile("c", "Whatever", "terminal")];
        let settings = Settings::default();
        let keymap = keymap();
        let section = Section::ALL.iter().position(|s| *s == row("defaultProfile").section).unwrap();
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, claude: &CLAUDE, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
        let frame = build(&Page { section, ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        let marks: Vec<&Item> = frame.items.iter().filter(|item| matches!(item.visual, Visual::Mark(_))).collect();
        assert_eq!(marks.len(), 3, "a logo for each profile");
        for (item, profile) in marks.iter().zip(&profiles) {
            assert_eq!(item.visual, Visual::Mark(profile_mark(profile.icon)));
            let radio = frame.items.iter().find(|r| matches!(r.visual, Visual::Plate(Fill::Radio { .. })) && (r.rect.y - item.rect.y).abs() < MARK_SIZE).expect("the radio of the row");
            let name = frame.items.iter().find(|n| matches!(&n.visual, Visual::Label { text, .. } if *text == profile.name)).expect("the name of the profile");
            assert!(item.rect.x >= radio.rect.x + radio.rect.w, "{}: the logo is right of the radio", profile.name);
            assert!(name.rect.x >= item.rect.x + item.rect.w, "{}: the name is right of the logo", profile.name);
            assert!(item.rect.y >= name.rect.y && item.rect.y + item.rect.h <= name.rect.y + name.rect.h, "{}: the logo is within the line of the name", profile.name);
        }
    }

    #[test]
    fn a_toggle_is_pressed_anywhere_on_its_row_and_a_choice_by_its_option() {
        let settings = Settings::default();
        let page = Page { section: 1, ..Page::default() };
        let frame = frame(&page, &settings);
        let at = frame.entries.iter().position(|e| matches!(e, Entry::Setting(row) if row.id == "copyOnSelect")).unwrap();
        let rect = frame.rows[at];
        assert_eq!(frame.part_at(0.0, rect.x + 40.0, rect.y + 10.0), Some(Part::Toggle("copyOnSelect")));
        let at = frame.entries.iter().position(|e| matches!(e, Entry::Setting(row) if row.id == "rightClick")).unwrap();
        let rect = frame.rows[at];
        let option = frame.items.iter().find(|item| item.part == Some(Part::Option("rightClick", 1))).unwrap();
        assert!(rect.contains(option.rect.x + 2.0, option.rect.y + 2.0));
        assert_eq!(frame.part_at(0.0, option.rect.x + 2.0, option.rect.y + 2.0), Some(Part::Option("rightClick", 1)));
    }

    #[test]
    fn rows_scrolled_out_of_the_area_cannot_be_pressed_and_reveal_scrolls_the_least() {
        let settings = Settings::default();
        let frame = frame(&Page::default(), &settings);
        // A point over the title bar of the page is not in the rows' area, whatever scrolled there.
        let above = frame.viewport.y - 5.0;
        assert_eq!(frame.part_at(200.0, 400.0, above), None);
        let last = frame.rows.len() - 1;
        let scroll = frame.reveal(last, 0.0);
        assert!(scroll > 0.0 && scroll <= frame.max_scroll());
        assert_eq!(frame.reveal(0, scroll), 0.0);
        assert_eq!(frame.reveal(last, scroll), scroll);
    }

    #[test]
    fn stepping_moves_a_value_by_its_step_wraps_a_choice_and_stays_in_range() {
        let settings = Settings::default();
        let opacity = row("appearance.opacity");
        assert_eq!(stepped(opacity, &settings, &[], true, false), Some(Value::Number(73.0)));
        assert_eq!(stepped(opacity, &settings, &[], false, true), Some(Value::Number(67.0)));
        let backdrop = row("appearance.backdrop");
        assert_eq!(stepped(backdrop, &settings, &[], false, false), Some(Value::Index(3)), "before the first option is the last one");
        let size = row("font.size");
        let mut small = Settings::default();
        (size.set)(&mut small, Value::Number(1.0));
        assert_eq!(stepped(size, &small, &[], false, false), Some((size.get)(&small)), "the lower bound holds");
        assert_eq!(stepped(row("font.family"), &settings, &[], true, false), None, "a text row has no step");
        assert_eq!(stepped(row("copyOnSelect"), &settings, &[], true, false), Some(Value::Bool(!settings.copy_on_select)));
    }

    #[test]
    fn at_any_width_a_control_stays_in_its_row_and_clear_of_the_label() {
        let settings = Settings::default();
        let keymap = keymap();
        let measure = |text: &str, size: f32| text.chars().count() as f32 * size * 0.55;
        for width in [460.0, 560.0, 700.0, 1100.0, 1600.0] {
            for section in 0..Section::ALL.len() {
                let page = Page { section, ..Page::default() };
                let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], claude: &CLAUDE, area: Rect::new(0.0, 48.0, width, 650.0) };
                let frame = build(&page, &view, &mut |text, size, _| measure(text, size));
                for (entry, row) in frame.entries.iter().zip(&frame.rows) {
                    let Entry::Setting(setting) = entry else { continue };
                    if matches!(setting.control, Control::Themes | Control::Profiles) {
                        continue;
                    }
                    let in_row = |item: &&Item| item.scrolls && item.rect.y >= row.y - 0.01 && item.rect.y + item.rect.h <= row.y + row.h + 0.01;
                    let controls: Vec<&Item> = frame
                        .items
                        .iter()
                        .filter(in_row)
                        .filter(|item| matches!(item.part, Some(Part::Option(..) | Part::Plus(_) | Part::Minus(_) | Part::Slider(_) | Part::Field(_))) || matches!(item.visual, Visual::Plate(Fill::Switch { .. })))
                        .collect();
                    assert!(!controls.is_empty(), "{} has no control at {width}", setting.id);
                    let label = frame.items.iter().filter(in_row).find(|item| matches!(item.visual, Visual::Label { .. }) && (item.rect.x - (row.x + MARGIN)).abs() < 0.01).expect("the row has its label");
                    let Visual::Label { text, size, .. } = &label.visual else { unreachable!() };
                    let label_end = label.rect.x + measure(text, *size);
                    for control in &controls {
                        assert!(control.rect.x >= row.x - 0.01 && control.rect.x + control.rect.w <= row.x + row.w + 0.01, "{} sticks out of its row at {width}", setting.id);
                        let beside = control.rect.x >= label_end - 0.01;
                        let below = control.rect.y >= label.rect.y + label.rect.h - 0.01;
                        assert!(beside || below, "{} runs into its label at {width}: control at {}, label ends at {label_end}", setting.id, control.rect.x);
                    }
                }
            }
        }
    }}
