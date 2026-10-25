//! The settings page: a column of the sections at the left, with the search line on top of it, and at the right the title
//! of the section and its rows.
//!
//! The page is a display list. `build` turns the state of the page and the settings into rectangles with what is drawn
//! in them and what they do when pressed (`Part`), so that drawing and hit testing read the same geometry. The rows are
//! those of `settings_ui::TABLE`; nothing here knows what a setting means. Pure apart from the text measure the caller
//! hands in.

use nekominal_model::{
    commands::Command,
    keymap::Keymap,
    profile_edit::{self, Field},
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

/// The column of the sections at the left of the page: its width, the room inside its edges and the height of a line, in
/// DIP. The search line is at its top and the button for the file at its bottom.
pub const NAV: f32 = 220.0;
const NAV_PAD: f32 = INSET;
const NAV_ITEM: f32 = 34.0;
const SEARCH_HEIGHT: f32 = 30.0;
const PAD: f32 = 32.0;
const CONTENT_MAX: f32 = 680.0;
/// Where the rows start below the top of the page: the title of the page is above them.
const CONTENT_TOP: f32 = 66.0;
/// The name of a section over what a search finds in it.
const HEADING: f32 = 34.0;
/// The miniatures of the layouts of the tabs: at most this wide, three in a line, as tall as this share of their width.
const LAYOUT_THUMB: f32 = 170.0;
const LAYOUT_RATIO: f32 = 0.6;
/// The one inset of the page: the title, the labels, the controls, the grids and the reset buttons all keep this far from
/// the ends of the hairlines between the rows, left and right.
const INNER: f32 = MARGIN;
/// The rows touch: a hairline above each separates them, so there is no gap between them.
const ROW: f32 = 48.0;
const ROW_GAP: f32 = 0.0;
/// Below a grid (the colour schemes, the layouts) before the next row, as much as above the text of a row.
const GRID_BOTTOM: f32 = INNER;
const CONTROL: f32 = 28.0;
const SWITCH: (f32, f32) = (44.0, 24.0);
const SLIDER_WIDTH: f32 = 200.0;
const FIELD_WIDTH: f32 = 280.0;
const TILE: (f32, f32) = (148.0, 64.0);
const TILE_GAP: f32 = 2.0 * GAP;
const RADIO_ROW: f32 = 40.0;
/// The reset button of a row, and the room the label leaves for it.
const RESET: f32 = 24.0;
const RESET_ROOM: f32 = RESET + GAP;
/// The row of an agent on the Agent page: its logo, its name and version, and a line that says how it is connected.
const CLAUDE_ROW: f32 = 64.0;
/// The row of the app on the About page: its logo, its name and version, what the look for a newer one found, and the
/// buttons under them.
const ABOUT_ROW: f32 = 108.0;
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
    /// The id of the profile whose editor the Profiles page shows instead of the list.
    pub profile: Option<String>,
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
    /// Looks again for a newer version, and updates to it and starts the app again.
    CheckUpdate,
    Update,
    /// Opens the repository, and the GitHub page of the contributor at this index of `update::contributors`.
    OpenRepo,
    Contributor(usize),
    /// The editor of the profile at this index, a new profile, back to the list, and deleting (or putting back) the one edited.
    EditProfile(usize),
    NewProfile,
    ProfileBack,
    DeleteProfile,
    /// A row of the editor of a profile: its text field, or the switch of running as administrator.
    ProfileField(Field),
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
    /// The column of the sections, darker than the page, with a hairline where it meets the page.
    Strip,
    /// A line of the column of the sections: the selected one on a plate.
    Tab { selected: bool, hover: bool },
    /// A row of the page: a hairline above it, and a plate while it has the keyboard.
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
    /// The icon of a program (a shell before the name of its profile, Claude Code), from its file at `path`; `mark` when
    /// there is no file or it has no icon.
    Program { path: Option<std::path::PathBuf>, mark: Mark },
    /// A line of text in a field, with its caret and selection when it is the one being edited (`cursor`), or what an empty
    /// one says (`placeholder`).
    Edit { text: String, cursor: Option<crate::textfield::Cursor>, placeholder: String },
    /// A color scheme: its background with its name and a few of its colours.
    Tile { scheme: usize, selected: bool, hover: bool },
    /// A layout of the tabs (an index into `TabLayout::ALL`): a window in a few rectangles.
    Layout { layout: usize, selected: bool, hover: bool },
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
    /// Nekominal itself: its version, whether a newer one is out, and the buttons that look and update.
    About,
    /// Where its source is, and who has worked on it.
    Repo,
    Contributors,
    /// Under the profiles: the button that adds one.
    NewProfile,
    /// The editor of a profile: the way back and its name on top, then a row a field.
    ProfileHeader,
    ProfileField(Field),
}

impl Entry {
    pub fn section(&self) -> Section {
        match self {
            Entry::Setting(row) => row.section,
            Entry::Key(_) => Section::Keys,
            Entry::Claude => Section::Agents,
            Entry::About | Entry::Repo | Entry::Contributors => Section::About,
            Entry::NewProfile | Entry::ProfileHeader | Entry::ProfileField(_) => Section::Profiles,
        }
    }
}

/// What the About page says of the look for a newer version.
pub fn update_note(check: &crate::update::Check, lang: Lang) -> String {
    use crate::update::Check;
    match check {
        Check::Idle | Check::Checking => Text::UpdateChecking.get(lang).to_owned(),
        Check::Latest => Text::UpdateLatest.get(lang).to_owned(),
        Check::Newer(version) => Text::UpdateNewer.format(lang, &[version]),
        Check::Failed => Text::UpdateFailed.get(lang).to_owned(),
    }
}

/// The glyph before the name of a section in the column of the sections.
fn section_glyph(section: Section) -> char {
    match section {
        Section::Appearance => '\u{E790}',
        Section::Terminal => '\u{E756}',
        Section::Profiles => '\u{E77B}',
        Section::Agents => '\u{E99A}',
        Section::Keys => '\u{E765}',
        Section::Advanced => '\u{E713}',
        Section::About => '\u{E946}',
    }
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
    /// The shells found on the machine: deleting the entry of one of them puts it back as found rather than removing it.
    pub found: &'a [nekominal_model::profiles::DiscoveredProfile],
    /// The program file of each profile, in the order of `profiles`, for its icon.
    pub programs: &'a [Option<std::path::PathBuf>],
    pub claude: &'a crate::agent::Claude,
    /// The area of the page in the coordinates of the window, in DIP.
    pub area: Rect,
    /// What the last look for a newer version found, and whether Scoop installed this copy (so that it can update it).
    pub update: crate::update::Check,
    pub scoop: bool,
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
        let mut found: Vec<Entry> = settings_ui::search(&page.query, lang, keymap)
            .into_iter()
            .map(|row| match row {
                settings_ui::Row::Setting(descriptor) => Entry::Setting(descriptor),
                settings_ui::Row::Key(command) => Entry::Key(command),
            })
            .collect();
        // By section, in the order of the sections; the best match first within each.
        found.sort_by_key(|entry| Section::ALL.iter().position(|s| *s == entry.section()));
        return found;
    }
    let section = Section::ALL[page.section.min(Section::ALL.len() - 1)];
    if section == Section::Keys {
        settings_ui::key_rows().map(Entry::Key).collect()
    } else if section == Section::Profiles && page.profile.is_some() {
        std::iter::once(Entry::ProfileHeader).chain(Field::ALL.map(Entry::ProfileField)).collect()
    } else {
        // The Agent page starts with the agents it knows, before the rows of the table.
        let agents = (section == Section::Agents).then_some(Entry::Claude);
        let about = if section == Section::About { vec![Entry::About, Entry::Repo, Entry::Contributors] } else { Vec::new() };
        let new_profile = (section == Section::Profiles).then_some(Entry::NewProfile);
        agents.into_iter().chain(about).chain(settings_ui::in_section(section).map(Entry::Setting)).chain(new_profile).collect()
    }
}

/// What the row of Claude Code says about the status bar getting its numbers.
pub fn claude_note(claude: &crate::agent::Claude, settings: &Settings) -> Text {
    if claude.exe.is_none() {
        Text::ClaudeMissing
    } else if claude.own_status_line {
        Text::ClaudeForeign
    } else if !settings.status_bar.show {
        Text::ClaudeBarOff
    } else {
        Text::ClaudeAuto
    }
}

/// What a row says at its right: the option, the number, the name of the scheme or profile.
pub fn value_text(row: &Descriptor, settings: &Settings, profiles: &[Profile], lang: Lang) -> String {
    match (row.control, (row.get)(settings)) {
        (Control::Toggle, Value::Bool(on)) => (if on { Text::ToggleOn } else { Text::ToggleOff }).get(lang).to_owned(),
        (Control::Choice(options) | Control::Layouts(options), Value::Index(i)) => options.get(i).map(|text| text.get(lang).to_owned()).unwrap_or_default(),
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
        (Control::Choice(options) | Control::Layouts(options), Value::Index(i)) => Value::Index(cycle(options.len(), i)),
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

/// A button with a glyph before its text.
fn icon_button(items: &mut Vec<Item>, rect: Rect, part: Part, ch: char, text: &str, hover: Option<Part>) {
    item(items, rect, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
    item(items, Rect::new(rect.x + 8.0, rect.y, 16.0, rect.h), None, Visual::Glyph { ch, size: 11.0, tone: Tone::Text }, true);
    item(items, Rect::new(rect.x + 28.0, rect.y, (rect.w - 32.0).max(0.0), rect.h), None, label(text, 13.0, Tone::Text, false, Align::Left), true);
}

/// The profile whose editor the page shows.
pub fn edited<'a>(page: &Page, view: &View<'a>) -> Option<&'a Profile> {
    let id = page.profile.as_deref()?;
    view.profiles.iter().find(|p| p.id == id)
}

fn label(text: impl Into<String>, size: f32, tone: Tone, bold: bool, align: Align) -> Visual {
    Visual::Label { text: text.into(), size, tone, bold, align }
}

pub fn build(page: &Page, view: &View, measure: &mut dyn FnMut(&str, f32, bool) -> f32) -> Frame {
    let lang = view.lang;
    let Rect { x: ox, y: oy, w: width, h: height } = view.area;
    let mut items = Vec::new();
    let hover = page.hover;

    // The column of the sections at the left: the search line on top, a line a section (with what a search finds in it),
    // and the button for the file at the bottom.
    let searching = !page.query.trim().is_empty();
    let entries = entries(page, lang, view.keymap);
    let nav = Rect::new(ox, oy, NAV.min(width / 2.0), height);
    item(&mut items, nav, None, Visual::Plate(Fill::Strip), false);
    let (inner_x, inner_w) = (nav.x + NAV_PAD, (nav.w - 2.0 * NAV_PAD).max(0.0));
    let search = Rect::new(inner_x, oy + NAV_PAD, inner_w, SEARCH_HEIGHT);
    item(&mut items, search, Some(Part::Search), Visual::Plate(Fill::Field { editing: page.editing.is_none() }), false);
    item(&mut items, Rect::new(search.x + 10.0, search.y, 16.0, search.h), None, Visual::Glyph { ch: '\u{E721}', size: 12.0, tone: Tone::Quiet }, false);
    let edit = Visual::Edit { text: page.query.clone(), cursor: page.editing.is_none().then_some(page.cursor), placeholder: Text::SettingsSearch.get(lang).to_owned() };
    item(&mut items, Rect::new(search.x + 32.0, search.y, (search.w - 40.0).max(0.0), search.h), None, edit, false);
    let mut nav_y = search.y + search.h + 12.0;
    for (i, section) in Section::ALL.iter().enumerate() {
        let rect = Rect::new(inner_x, nav_y, inner_w, NAV_ITEM);
        let selected = !searching && i == page.section;
        let part = Part::Nav(i);
        item(&mut items, rect, Some(part), Visual::Plate(Fill::Tab { selected, hover: hover == Some(part) }), false);
        let found = entries.iter().filter(|entry| entry.section() == *section).count();
        let tone = if selected || (searching && found > 0) { Tone::Text } else { Tone::Quiet };
        item(&mut items, Rect::new(rect.x + 8.0, rect.y, 18.0, rect.h), None, Visual::Glyph { ch: section_glyph(*section), size: 13.0, tone }, false);
        item(&mut items, Rect::new(rect.x + 34.0, rect.y, (rect.w - 64.0).max(0.0), rect.h), None, label(section.text().get(lang), 13.0, tone, false, Align::Left), false);
        if searching {
            item(&mut items, Rect::new(rect.x + rect.w - 40.0, rect.y, 30.0, rect.h), None, label(found.to_string(), 12.0, Tone::Quiet, false, Align::Right), false);
        }
        nav_y += NAV_ITEM + 2.0;
    }
    let file = Rect::new(inner_x, oy + height - NAV_PAD - NAV_ITEM, inner_w, NAV_ITEM);
    if file.y >= nav_y + 8.0 {
        item(&mut items, file, Some(Part::File), Visual::Plate(Fill::Tab { selected: false, hover: hover == Some(Part::File) }), false);
        item(&mut items, Rect::new(file.x + 8.0, file.y, 18.0, file.h), None, Visual::Glyph { ch: '\u{E8A5}', size: 13.0, tone: Tone::Quiet }, false);
        item(&mut items, Rect::new(file.x + 34.0, file.y, (file.w - 40.0).max(0.0), file.h), None, label(Text::SettingsFile.get(lang), 13.0, Tone::Quiet, false, Align::Left), false);
    }

    // The page: its title, then its rows, under it and scrolling.
    let page_x = nav.x + nav.w;
    let content_width = (width - nav.w - 2.0 * PAD).clamp(0.0, CONTENT_MAX);
    let x0 = page_x + PAD;
    let title = if searching { Text::SettingsResults } else { Section::ALL[page.section.min(Section::ALL.len() - 1)].text() };
    item(&mut items, Rect::new(x0 + INNER, oy + 18.0, (content_width - 2.0 * INNER).max(0.0), 32.0), None, label(title.get(lang), 20.0, Tone::Text, true, Align::Left), false);
    let top = oy + CONTENT_TOP;
    let viewport = Rect::new(page_x, top, (width - nav.w).max(0.0), (oy + height - top).max(0.0));
    let mut rows = Vec::new();
    let mut y = top;
    if entries.is_empty() {
        item(&mut items, Rect::new(x0 + INNER, y, (content_width - 2.0 * INNER).max(0.0), ROW), None, label(Text::SettingsEmpty.get(lang), 13.0, Tone::Quiet, false, Align::Left), true);
        y += ROW;
    }
    let theme_names: Vec<&str> = themes::theme_names().collect();
    let mut last_section = None;
    for (n, entry) in entries.iter().enumerate() {
        let focus = page.focus == Some(n);
        // What a search finds comes by section, under the name of each.
        if searching && last_section != Some(entry.section()) {
            last_section = Some(entry.section());
            item(&mut items, Rect::new(x0 + INNER, y + 8.0, (content_width - 2.0 * INNER).max(0.0), HEADING - 8.0), None, label(entry.section().text().get(lang), 12.0, Tone::Quiet, true, Align::Left), true);
            y += HEADING;
        }
        let start = y;
        match *entry {
            Entry::Key(command) => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                let capturing = page.capturing == Some(command);
                item(&mut items, rect, Some(Part::Binding(command)), Visual::Plate(Fill::Row { focus: focus || capturing }), true);
                item(&mut items, Rect::new(rect.x + INNER, rect.y, (rect.w - 2.0 * INNER) / 2.0, rect.h), None, label(command.label(lang), 13.0, Tone::Text, false, Align::Left), true);
                let mut right = rect.x + rect.w - INNER;
                if capturing {
                    let text = if page.reserved { Text::KeyReserved } else { Text::KeysPress };
                    item(&mut items, Rect::new(rect.x + rect.w / 2.0, rect.y, rect.w / 2.0 - INNER, rect.h), None, label(text.get(lang), 12.0, Tone::Quiet, false, Align::Right), true);
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
                // The reset button left of the shortcuts it puts back.
                if nekominal_model::keybindings::is_modified(&view.settings.keybindings, command) {
                    let reset = Rect::new(right - RESET, rect.y + (rect.h - RESET) / 2.0, RESET, RESET);
                    let part = Part::ResetKey(command);
                    item(&mut items, reset, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, reset, None, Visual::Glyph { ch: '\u{E777}', size: 11.0, tone: Tone::Quiet }, true);
                }
                y += ROW;
            }
            Entry::Claude => {
                let claude = view.claude;
                let rect = Rect::new(x0, y, content_width, CLAUDE_ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                let mark = Rect::new(rect.x + INNER, rect.y + (rect.h - 28.0) / 2.0, 28.0, 28.0);
                item(&mut items, mark, None, Visual::Program { path: claude.exe.clone(), mark: Mark::Badge { label: "C", color: (217, 119, 87) } }, true);
                let text_x = mark.x + 28.0 + 2.0 * MARK_GAP;
                let room = (rect.x + rect.w - INNER - text_x).max(0.0);
                let title = match &claude.version {
                    Some(version) => format!("Claude Code  {version}"),
                    None => "Claude Code".to_owned(),
                };
                item(&mut items, Rect::new(text_x, rect.y + 8.0, room, 22.0), None, label(title, 13.0, Tone::Text, true, Align::Left), true);
                let note = claude_note(claude, view.settings);
                item(&mut items, Rect::new(text_x, rect.y + rect.h - 30.0, room, 22.0), None, label(note.get(lang), 12.0, Tone::Quiet, false, Align::Left), true);
                y += CLAUDE_ROW;
            }
            Entry::About => {
                use crate::update::Check;
                let rect = Rect::new(x0, y, content_width, ABOUT_ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                let mark = Rect::new(rect.x + INNER, rect.y + 16.0, 36.0, 36.0);
                item(&mut items, mark, None, Visual::Program { path: std::env::current_exe().ok(), mark: Mark::Badge { label: "N", color: (122, 162, 247) } }, true);
                let text_x = mark.x + 36.0 + 2.0 * MARK_GAP;
                let room = (rect.x + rect.w - INNER - text_x).max(0.0);
                let title = format!("Nekominal  {}", crate::update::CURRENT);
                item(&mut items, Rect::new(text_x, rect.y + 12.0, room, 24.0), None, label(title, 15.0, Tone::Text, true, Align::Left), true);
                let newer = matches!(view.update, Check::Newer(_));
                let tone = if newer { Tone::Text } else { Tone::Quiet };
                item(&mut items, Rect::new(text_x, rect.y + 36.0, room, 20.0), None, label(update_note(&view.update, lang), 12.0, tone, false, Align::Left), true);
                // The buttons: update when a newer version is out and Scoop can put it in, and look again.
                let mut at = text_x;
                let buttons_y = rect.y + 66.0;
                if newer && view.scoop {
                    let text = Text::AppUpdate.get(lang);
                    let button = Rect::new(at, buttons_y, measure(text, 13.0, false) + 28.0, CONTROL);
                    item(&mut items, button, Some(Part::Update), Visual::Plate(Fill::Button { selected: true, hover: hover == Some(Part::Update) }), true);
                    item(&mut items, button, None, label(text, 13.0, Tone::Text, false, Align::Center), true);
                    at = button.x + button.w + GAP;
                }
                let text = Text::UpdateCheck.get(lang);
                let button = Rect::new(at, buttons_y, measure(text, 13.0, false) + 28.0, CONTROL);
                item(&mut items, button, Some(Part::CheckUpdate), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(Part::CheckUpdate) }), true);
                item(&mut items, button, None, label(text, 13.0, Tone::Text, false, Align::Center), true);
                if newer && !view.scoop {
                    let x = button.x + button.w + 12.0;
                    item(&mut items, Rect::new(x, buttons_y, (rect.x + rect.w - INNER - x).max(0.0), CONTROL), None, label(Text::UpdateNotScoop.get(lang), 12.0, Tone::Quiet, false, Align::Left), true);
                }
                y += ABOUT_ROW;
            }
            Entry::Repo => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                item(&mut items, Rect::new(rect.x + INNER, rect.y, (rect.w - 2.0 * INNER) / 3.0, rect.h), None, label(Text::SettingsRepo.get(lang), 13.0, Tone::Text, false, Align::Left), true);
                let text = Text::CommonOpen.get(lang);
                let button = Rect::new(rect.x + rect.w - INNER - measure(text, 13.0, false) - 28.0, rect.y + (rect.h - CONTROL) / 2.0, measure(text, 13.0, false) + 28.0, CONTROL);
                item(&mut items, button, Some(Part::OpenRepo), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(Part::OpenRepo) }), true);
                item(&mut items, button, None, label(text, 13.0, Tone::Text, false, Align::Center), true);
                let address = crate::update::REPO.trim_start_matches("https://");
                let left = rect.x + INNER + (rect.w - 2.0 * INNER) / 3.0;
                item(&mut items, Rect::new(left, rect.y, (button.x - GAP - 8.0 - left).max(0.0), rect.h), None, label(address, 12.0, Tone::Quiet, false, Align::Right), true);
                y += ROW;
            }
            Entry::Contributors => {
                // The label on its line, then a chip a contributor, as many in a line as fit inside the inset.
                let people = crate::update::contributors();
                let widths: Vec<f32> = people.iter().map(|login| measure(login, 13.0, false) + 24.0).collect();
                let places = wrap(&widths, (content_width - 2.0 * INNER).max(0.0), GAP);
                let lines = places.iter().map(|(line, _)| *line + 1).max().unwrap_or(0);
                let height = ROW + (lines as f32 * (CONTROL + GAP) - GAP).max(0.0) + if lines > 0 { GRID_BOTTOM } else { 0.0 };
                let rect = Rect::new(x0, y, content_width, height - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                item(&mut items, Rect::new(rect.x + INNER, rect.y, rect.w - 2.0 * INNER, ROW - ROW_GAP), None, label(Text::SettingsContributors.get(lang), 13.0, Tone::Text, false, Align::Left), true);
                for (i, (login, (line, offset))) in people.iter().zip(&places).enumerate() {
                    let chip = Rect::new(rect.x + INNER + offset, rect.y + ROW + *line as f32 * (CONTROL + GAP), widths[i], CONTROL);
                    let part = Part::Contributor(i);
                    item(&mut items, chip, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, chip, None, label(login.as_str(), 13.0, Tone::Text, false, Align::Center), true);
                }
                y += height;
            }
            Entry::NewProfile => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                let text = Text::ProfileNew.get(lang);
                let button = Rect::new(rect.x + INNER, rect.y + (rect.h - CONTROL) / 2.0, measure(text, 13.0, false) + 40.0, CONTROL);
                icon_button(&mut items, button, Part::NewProfile, '\u{E710}', text, hover);
                y += ROW;
            }
            Entry::ProfileHeader => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                item(&mut items, rect, None, Visual::Plate(Fill::Row { focus }), true);
                let mid = rect.y + (rect.h - CONTROL) / 2.0;
                let text = Text::ProfileBack.get(lang);
                let back = Rect::new(rect.x + INNER, mid, measure(text, 13.0, false) + 40.0, CONTROL);
                icon_button(&mut items, back, Part::ProfileBack, '\u{E72B}', text, hover);
                let profile = edited(page, view);
                let mut right = rect.x + rect.w - INNER;
                // A profile written in the settings is deleted; the entry of a shell found on the machine is taken out, which
                // puts it back as found. A shell nothing changes has neither.
                if let Some(profile) = profile.filter(|p| profile_edit::has_entry(view.settings, &p.id)) {
                    let action = if view.found.iter().any(|f| f.id == profile.id) { Text::ProfileRestore } else { Text::ProfileDelete };
                    let text = action.get(lang);
                    let button = Rect::new(right - measure(text, 13.0, false) - 28.0, mid, measure(text, 13.0, false) + 28.0, CONTROL);
                    item(&mut items, button, Some(Part::DeleteProfile), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(Part::DeleteProfile) }), true);
                    item(&mut items, button, None, label(text, 13.0, Tone::Text, false, Align::Center), true);
                    right = button.x - GAP;
                }
                let name_x = back.x + back.w + 2.0 * GAP;
                let name = profile.map_or(String::new(), |p| p.name.clone());
                item(&mut items, Rect::new(name_x, rect.y, (right - name_x).max(0.0), rect.h), None, label(name, 13.0, Tone::Text, true, Align::Left), true);
                y += ROW;
            }
            Entry::ProfileField(field) => {
                let profile = edited(page, view);
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                let part = Part::ProfileField(field);
                // The switch is pressed anywhere on its row, like a toggle of the table.
                item(&mut items, rect, (field == Field::Elevate).then_some(part), Visual::Plate(Fill::Row { focus }), true);
                let room = rect.w - 2.0 * INNER;
                item(&mut items, Rect::new(rect.x + INNER, rect.y, room / 3.0, rect.h), None, label(field.label().get(lang), 13.0, Tone::Text, false, Align::Left), true);
                let right = rect.x + rect.w - INNER;
                if field == Field::Elevate {
                    let switch = Rect::new(right - SWITCH.0, rect.y + (rect.h - SWITCH.1) / 2.0, SWITCH.0, SWITCH.1);
                    let on = profile.is_some_and(|p| p.options.elevate);
                    item(&mut items, switch, None, Visual::Plate(Fill::Switch { on, hover: hover == Some(part) }), true);
                } else {
                    let width = room * 2.0 / 3.0 - GAP;
                    let field_rect = Rect::new(right - width, rect.y + (rect.h - CONTROL) / 2.0, width, CONTROL);
                    let editing = page.editing.as_ref().filter(|(id, _)| *id == field.id()).map(|(_, text)| text.clone());
                    item(&mut items, field_rect, Some(part), Visual::Plate(Fill::Field { editing: editing.is_some() }), true);
                    let inner = Rect::new(field_rect.x + 10.0, field_rect.y, field_rect.w - 20.0, field_rect.h);
                    let shown = match editing {
                        Some(text) => Visual::Edit { text, cursor: Some(page.edit_cursor), placeholder: String::new() },
                        None => {
                            let text = profile.map(|p| profile_edit::text(p, field)).unwrap_or_default();
                            let text = if text.is_empty() { field.placeholder().map_or("", |t| t.get(lang)).to_owned() } else { text };
                            label(text, 13.0, Tone::Quiet, false, Align::Left)
                        }
                    };
                    item(&mut items, inner, None, shown, true);
                }
                y += ROW;
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
                    Control::Themes | Control::Profiles | Control::Layouts(_) => 0.0,
                };
                // A setting that does nothing with the others as they are is greyed and cannot be pressed.
                let applies = row.applies(view.settings);
                let stacked = control_width > 0.0 && label_width + control_width + 2.0 * INNER + 16.0 + RESET_ROOM > content_width;
                let drop = if stacked { STACK } else { 0.0 };
                // What fits on the line of the control under the label: the options of a choice go on more lines when they do not.
                let room = content_width - 2.0 * INNER;
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
                // Every control ends at the same inset from the right end of the hairline.
                let right = rect.x + rect.w - INNER;
                let value = (row.get)(view.settings);
                // A toggle is pressed anywhere on its row.
                let row_part = (applies && matches!(row.control, Control::Toggle)).then_some(Part::Toggle(id));
                item(&mut items, rect, row_part, Visual::Plate(Fill::Row { focus }), true);
                let label_room = if stacked { rect.w - 2.0 * INNER - RESET_ROOM } else { (rect.w - 2.0 * INNER - RESET_ROOM) / 2.0 };
                let tone = if applies { Tone::Text } else { Tone::Quiet };
                item(&mut items, Rect::new(rect.x + INNER, rect.y, label_room, ROW - ROW_GAP), None, label(row.label(lang), 13.0, tone, false, Align::Left), true);
                // Where the control beside the label starts: the reset button goes left of it. A control under its label (or a
                // grid) leaves the reset button at the right end of the label's line.
                let mut control_left = right;
                match (row.control, value) {
                    (Control::Toggle, Value::Bool(on)) => {
                        let switch = Rect::new(right - SWITCH.0, line.y + (line.h - SWITCH.1) / 2.0, SWITCH.0, SWITCH.1);
                        item(&mut items, switch, None, Visual::Plate(Fill::Switch { on: on && applies, hover: hover == Some(Part::Toggle(id)) }), true);
                        control_left = switch.x;
                        y += ROW + drop;
                    }
                    (Control::Choice(options), Value::Index(chosen)) => {
                        let mut at = right;
                        for (i, option) in options.iter().enumerate().rev() {
                            let text = option.get(lang);
                            let w = option_widths[i];
                            let button = match option_lines.get(i) {
                                Some((line, offset)) => Rect::new(rect.x + INNER + offset, mid + *line as f32 * (CONTROL + GAP), w, CONTROL),
                                None => Rect::new(at - w, mid, w, CONTROL),
                            };
                            let part = Part::Option(id, i);
                            item(&mut items, button, Some(part), Visual::Plate(Fill::Button { selected: i == chosen, hover: hover == Some(part) }), true);
                            item(&mut items, button, None, label(text, 13.0, if i == chosen { Tone::Text } else { Tone::Quiet }, false, Align::Center), true);
                            at = button.x - GAP;
                        }
                        control_left = at + GAP;
                        y += ROW + drop + extra;
                    }
                    (Control::Slider(range), Value::Number(n)) => {
                        let value_box = Rect::new(right - 48.0, line.y, 48.0, line.h);
                        item(&mut items, value_box, None, label(format!("{n:.0}%"), 13.0, Tone::Quiet, false, Align::Right), true);
                        let track_width = SLIDER_WIDTH.min(room - 60.0).max(60.0);
                        let share = (((n - range.min) / (range.max - range.min)) as f32).clamp(0.0, 1.0);
                        let track_x = value_box.x - 12.0 - track_width;
                        item(&mut items, Rect::new(track_x - THUMB / 2.0, line.y, track_width + THUMB, line.h), Some(Part::Slider(id)), Visual::Plate(Fill::Slider { share }), true);
                        control_left = track_x - THUMB / 2.0;
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
                        control_left = minus.x;
                        y += ROW + drop;
                    }
                    (Control::Text, Value::Text(text)) => {
                        let field_width = FIELD_WIDTH.min(room - RESET_ROOM);
                        let field = Rect::new(right - field_width, mid, field_width, CONTROL);
                        control_left = field.x;
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
                    (Control::Layouts(options), Value::Index(chosen)) => {
                        y += ROW;
                        let w = ((content_width - 2.0 * INNER - 2.0 * TILE_GAP) / 3.0).clamp(0.0, LAYOUT_THUMB);
                        for (i, option) in options.iter().enumerate() {
                            let thumb = Rect::new(x0 + INNER + i as f32 * (w + TILE_GAP), y, w, w * LAYOUT_RATIO);
                            let part = Part::Option(id, i);
                            item(&mut items, thumb, Some(part), Visual::Layout { layout: i, selected: i == chosen, hover: hover == Some(part) }, true);
                            let tone = if i == chosen { Tone::Text } else { Tone::Quiet };
                            item(&mut items, Rect::new(thumb.x, thumb.y + thumb.h + 4.0, w, 22.0), None, label(option.get(lang), 12.0, tone, false, Align::Center), true);
                        }
                        y += w * LAYOUT_RATIO + 4.0 + 22.0 + GRID_BOTTOM;
                    }
                    (Control::Themes, Value::Text(current)) => {
                        y += ROW;
                        // The tiles share the width inside the inset, as many in a line as fit at their size.
                        let inner = (content_width - 2.0 * INNER).max(0.0);
                        let columns = (((inner + TILE_GAP) / (TILE.0 + TILE_GAP)).floor() as usize).max(1);
                        let tile_w = ((inner - TILE_GAP * (columns - 1) as f32) / columns as f32).max(0.0);
                        for (i, name) in theme_names.iter().enumerate() {
                            let (col, line) = (i % columns, i / columns);
                            let tile = Rect::new(x0 + INNER + col as f32 * (tile_w + TILE_GAP), y + line as f32 * (TILE.1 + TILE_GAP), tile_w, TILE.1);
                            let part = Part::Tile(i);
                            item(&mut items, tile, Some(part), Visual::Tile { scheme: i, selected: *name == current, hover: hover == Some(part) }, true);
                        }
                        y += theme_names.len().div_ceil(columns) as f32 * (TILE.1 + TILE_GAP) - TILE_GAP + GRID_BOTTOM;
                    }
                    (Control::Profiles, Value::Text(current)) => {
                        y += ROW;
                        let chosen = view.profiles.iter().position(|p| p.id == current).unwrap_or(0);
                        for (i, profile) in view.profiles.iter().enumerate() {
                            let line = Rect::new(x0, y, content_width, RADIO_ROW - ROW_GAP);
                            let part = Part::Profile(i);
                            item(&mut items, line, Some(part), Visual::Plate(Fill::Row { focus: false }), true);
                            let radio = Rect::new(line.x + INNER, line.y + (line.h - 16.0) / 2.0, 16.0, 16.0);
                            item(&mut items, radio, None, Visual::Plate(Fill::Radio { on: i == chosen, hover: hover == Some(part) }), true);
                            // The logo of the shell, then its name.
                            let mark = Rect::new(radio.x + radio.w + 12.0, line.y + (line.h - MARK_SIZE) / 2.0, MARK_SIZE, MARK_SIZE);
                            item(&mut items, mark, None, Visual::Program { path: view.programs.get(i).cloned().flatten(), mark: profile_mark(profile.icon) }, true);
                            let name_x = mark.x + MARK_SIZE + MARK_GAP;
                            let edit = Rect::new(line.x + line.w - INNER - CONTROL, line.y + (line.h - CONTROL) / 2.0, CONTROL, CONTROL);
                            item(&mut items, Rect::new(name_x, line.y, (edit.x - GAP - name_x).max(0.0), line.h), None, label(profile.name.as_str(), 13.0, Tone::Text, false, Align::Left), true);
                            // Its editor, pressed over the row that makes it the default.
                            let part = Part::EditProfile(i);
                            item(&mut items, edit, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                            item(&mut items, edit, None, Visual::Glyph { ch: '\u{E70F}', size: 11.0, tone: Tone::Quiet }, true);
                            y += RADIO_ROW;
                        }
                    }
                    _ => y += ROW,
                }
                if row.is_modified(view.settings) {
                    let beside = !stacked && control_width > 0.0;
                    let x = if beside { control_left - GAP - RESET } else { right - RESET };
                    let reset = Rect::new(x, rect.y + (ROW - ROW_GAP - RESET) / 2.0, RESET, RESET);
                    let part = Part::Reset(id);
                    item(&mut items, reset, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, reset, None, Visual::Glyph { ch: '\u{E777}', size: 11.0, tone: Tone::Quiet }, true);
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

    const CLAUDE: crate::agent::Claude = crate::agent::Claude { exe: None, version: None, own_status_line: false };

    fn keymap() -> Keymap {
        build_keymap(default_bindings(), &Default::default()).keymap
    }

    fn frame(page: &Page, settings: &Settings) -> Frame {
        let keymap = keymap();
        let view = View { lang: Lang::EnUs, settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
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
            // The Agent page starts with the row of Claude Code; the About page is the rows of the app, its source and its
            // contributors.
            let own = match section {
                Section::Agents => 1,
                Section::About => 3,
                Section::Profiles => 1,
                _ => 0,
            };
            let expected = if *section == Section::Keys { settings_ui::key_rows().count() } else { settings_ui::in_section(*section).count() + own };
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
                if !row.applies(&settings) {
                    continue;
                }
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
    fn the_sections_are_a_column_and_the_one_shown_is_the_one_selected() {
        let settings = Settings::default();
        let nav_items = |frame: &Frame| -> Vec<Item> { frame.items.iter().filter(|item| matches!(item.part, Some(Part::Nav(_)))).cloned().collect() };
        let selected = |items: &[Item]| -> Vec<usize> {
            items.iter().enumerate().filter(|(_, item)| matches!(item.visual, Visual::Plate(Fill::Tab { selected: true, .. }))).map(|(i, _)| i).collect()
        };
        let shown = nav_items(&frame(&Page { section: 2, ..Page::default() }, &settings));
        assert_eq!(shown.len(), Section::ALL.len());
        assert!(shown.windows(2).all(|pair| pair[0].rect.x == pair[1].rect.x && pair[0].rect.y + pair[0].rect.h <= pair[1].rect.y), "one under the other");
        assert!(shown.iter().all(|item| item.rect.x + item.rect.w <= NAV), "in the column");
        assert_eq!(selected(&shown), vec![2]);
        // What a search finds comes from every section, so no section is the one shown.
        let found = nav_items(&frame(&Page { query: "opacity".into(), ..Page::default() }, &settings));
        assert!(selected(&found).is_empty());
    }

    #[test]
    fn the_search_line_and_the_file_button_stay_in_the_column_and_the_rows_right_of_it() {
        let settings = Settings::default();
        let keymap = keymap();
        for (width, height) in [crate::settings_window::MIN_SIZE, (900.0, 650.0), (1400.0, 900.0)] {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, width, height) };
            let frame = build(&Page::default(), &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.55);
            let nav_end = frame.items.iter().filter(|item| matches!(item.part, Some(Part::Nav(_)))).map(|item| item.rect.y + item.rect.h).fold(0.0, f32::max);
            let search = frame.items.iter().find(|item| item.part == Some(Part::Search)).expect("the search line is there");
            assert!(search.rect.w >= 100.0 && search.rect.x + search.rect.w <= NAV, "the search line at {width}");
            if let Some(file) = frame.items.iter().find(|item| item.part == Some(Part::File)) {
                assert!(file.rect.y >= nav_end && file.rect.y + file.rect.h <= height, "the button for the file at {width}");
            }
            assert!(frame.rows.iter().all(|row| row.x >= NAV.min(width / 2.0)), "the rows are right of the column at {width}");
        }
    }

    #[test]
    fn a_search_finds_by_section_and_counts_in_the_column() {
        let settings = Settings::default();
        let frame = frame(&Page { query: "tab".into(), ..Page::default() }, &settings);
        let order: Vec<usize> = frame.entries.iter().map(|e| Section::ALL.iter().position(|s| *s == e.section()).unwrap()).collect();
        assert!(order.windows(2).all(|pair| pair[0] <= pair[1]), "by section");
        let total: usize = frame.items.iter().filter(|item| item.rect.x < NAV && !item.scrolls).filter_map(|item| match &item.visual {
            Visual::Label { text, align: Align::Right, .. } => text.parse::<usize>().ok(),
            _ => None,
        }).sum();
        assert_eq!(total, frame.entries.len(), "the counts of the sections add up to what is found");
    }

    #[test]
    fn the_title_labels_controls_and_grids_keep_one_inset_from_both_ends_of_the_rows() {
        let mut settings = Settings::default();
        settings.copy_on_select = !settings.copy_on_select; // a reset button to place
        for section in 0..Section::ALL.len() {
            let frame = frame(&Page { section, ..Page::default() }, &settings);
            let Some(first) = frame.rows.first() else { continue };
            let (left, right) = (first.x + INNER, first.x + first.w - INNER);
            let title = frame.items.iter().find(|item| !item.scrolls && item.rect.x > NAV && matches!(item.visual, Visual::Label { size, .. } if size == 20.0)).expect("the title");
            assert_eq!(title.rect.x, left, "{section}: the title starts where the labels do");
            for item in frame.items.iter().filter(|item| item.scrolls && !matches!(item.visual, Visual::Plate(Fill::Row { .. }))) {
                assert!(item.rect.x >= left - 0.01 && item.rect.x + item.rect.w <= right + 0.01, "{section}: {:?} at {:?} is outside {left}..{right}", item.visual, item.rect);
            }
            for (row, entry) in frame.rows.iter().zip(&frame.entries) {
                let Entry::Setting(setting) = entry else { continue };
                if !matches!(setting.control, Control::Toggle | Control::Choice(_) | Control::Number(_)) {
                    continue;
                }
                let in_row = |item: &&Item| item.scrolls && item.rect.y >= row.y && item.rect.y + item.rect.h <= row.y + row.h;
                let end = frame.items.iter().filter(in_row).filter(|item| !matches!(item.visual, Visual::Plate(Fill::Row { .. }))).map(|item| item.rect.x + item.rect.w).fold(0.0, f32::max);
                assert!((end - right).abs() < 0.01, "{}: its control ends at {end}, not at {right}", setting.id);
            }
        }
        let terminal = Section::ALL.iter().position(|s| *s == row("copyOnSelect").section).unwrap();
        let frame = frame(&Page { section: terminal, ..Page::default() }, &settings);
        let reset = frame.items.iter().find(|item| item.part == Some(Part::Reset("copyOnSelect"))).expect("the reset button");
        let switch = frame.items.iter().find(|item| matches!(item.visual, Visual::Plate(Fill::Switch { .. })) && (item.rect.y - reset.rect.y).abs() < 12.0).expect("its switch");
        assert!(reset.rect.x + reset.rect.w <= switch.rect.x && switch.rect.x - (reset.rect.x + reset.rect.w) <= GAP + 0.01, "the reset button sits just left of its control");
    }

    #[test]
    fn the_about_page_offers_the_update_only_when_a_newer_version_is_out_and_scoop_can_install_it() {
        use crate::update::Check;
        let settings = Settings::default();
        let keymap = keymap();
        let about = Section::ALL.iter().position(|s| *s == Section::About).unwrap();
        let parts = |update: Check, scoop: bool| {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, area: Rect::new(0.0, 0.0, 1000.0, 700.0), update, scoop };
            let frame = build(&Page { section: about, ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
            let texts: Vec<String> = frame.items.iter().filter_map(|item| match &item.visual { Visual::Label { text, .. } => Some(text.clone()), _ => None }).collect();
            (frame.items.iter().filter_map(|item| item.part).filter(|p| matches!(p, Part::Update | Part::CheckUpdate)).collect::<Vec<_>>(), texts)
        };
        let (latest, texts) = parts(Check::Latest, true);
        assert_eq!(latest, [Part::CheckUpdate]);
        assert!(texts.iter().any(|t| t.starts_with("Nekominal  ") && t.ends_with(crate::update::CURRENT)), "the version is on the page");
        let (newer, texts) = parts(Check::Newer("9.9.9".into()), true);
        assert_eq!(newer, [Part::Update, Part::CheckUpdate]);
        assert!(texts.iter().any(|t| t.contains("9.9.9")));
        let all: Vec<Part> = {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, area: Rect::new(0.0, 0.0, 1000.0, 700.0), update: Check::Latest, scoop: true };
            build(&Page { section: about, ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5).items.iter().filter_map(|item| item.part).collect()
        };
        assert!(all.contains(&Part::OpenRepo), "the repository can be opened");
        let people = crate::update::contributors().len();
        assert_eq!(all.iter().filter(|p| matches!(p, Part::Contributor(_))).count(), people, "a chip for each contributor");
        let (elsewhere, texts) = parts(Check::Newer("9.9.9".into()), false);
        assert_eq!(elsewhere, [Part::CheckUpdate], "a copy Scoop did not install is not updated by it");
        assert!(texts.iter().any(|t| t == Text::UpdateNotScoop.get(Lang::EnUs)));
    }

    #[test]
    fn the_rows_of_the_sidebar_are_greyed_and_cannot_be_pressed_with_the_tabs_on_top() {
        let mut settings = Settings::default();
        let page = Page { section: 0, ..Page::default() };
        let can_press = |settings: &Settings, id: &'static str| frame(&page, settings).items.iter().any(|item| item.part == Some(Part::Toggle(id)));
        assert!(!can_press(&settings, "tabs.sidebarOpen") && !can_press(&settings, "tabs.groupByState"));
        settings.tabs.layout = nekominal_model::settings::TabLayout::Icons;
        assert!(can_press(&settings, "tabs.sidebarOpen") && !can_press(&settings, "tabs.groupByState"), "the groups are only for the list");
        settings.tabs.layout = nekominal_model::settings::TabLayout::List;
        assert!(can_press(&settings, "tabs.groupByState"));
        let thumbs = frame(&page, &settings).items.iter().filter(|item| matches!(item.visual, Visual::Layout { .. })).count();
        assert_eq!(thumbs, 3, "a miniature for each layout");
    }

    #[test]
    fn a_profile_shows_the_logo_of_its_shell_between_the_radio_and_its_name() {
        use nekominal_model::profiles::ProfileKind;
        let profile = |id: &str, name: &str, icon: &'static str| Profile { id: id.into(), name: name.into(), kind: ProfileKind::Other, icon, program: String::new(), args: Vec::new(), cwd: None, custom: false, options: Default::default() };
        let profiles = [profile("a", "PowerShell", "pwsh"), profile("b", "Ubuntu", "wsl"), profile("c", "Whatever", "terminal")];
        let settings = Settings::default();
        let keymap = keymap();
        let section = Section::ALL.iter().position(|s| *s == row("defaultProfile").section).unwrap();
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, found: &[], programs: &[None, None, None], claude: &CLAUDE, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
        let frame = build(&Page { section, ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        // The profiles come first on their page: the Agent page has a mark of its own.
        let marks: Vec<&Item> = frame.items.iter().filter(|item| matches!(item.visual, Visual::Program { .. })).collect();
        assert_eq!(marks.len(), 3, "a logo for each profile");
        for (item, profile) in marks.iter().zip(&profiles) {
            assert_eq!(item.visual, Visual::Program { path: None, mark: profile_mark(profile.icon) }, "no program file: the logo stands in");
            let radio = frame.items.iter().find(|r| matches!(r.visual, Visual::Plate(Fill::Radio { .. })) && (r.rect.y - item.rect.y).abs() < MARK_SIZE).expect("the radio of the row");
            let name = frame.items.iter().find(|n| matches!(&n.visual, Visual::Label { text, .. } if *text == profile.name)).expect("the name of the profile");
            assert!(item.rect.x >= radio.rect.x + radio.rect.w, "{}: the logo is right of the radio", profile.name);
            assert!(name.rect.x >= item.rect.x + item.rect.w, "{}: the name is right of the logo", profile.name);
            assert!(item.rect.y >= name.rect.y && item.rect.y + item.rect.h <= name.rect.y + name.rect.h, "{}: the logo is within the line of the name", profile.name);
        }
    }

    #[test]
    fn a_profile_is_edited_on_its_own_page_and_only_an_entry_of_the_settings_can_be_deleted() {
        use nekominal_model::profiles::{DiscoveredProfile, ProfileKind};
        let profile = |id: &str| Profile { id: id.into(), name: id.into(), kind: ProfileKind::Other, icon: "terminal", program: "x.exe".into(), args: Vec::new(), cwd: None, custom: false, options: Default::default() };
        let profiles = [profile("pwsh"), profile("mine")];
        let found = [DiscoveredProfile { id: "pwsh".into(), name: "pwsh".into(), kind: "powershell", icon: "pwsh", program: "x.exe".into(), args: Vec::new() }];
        let mut settings = Settings::default();
        settings.profiles.push(nekominal_model::settings::CustomProfile { id: "mine".into(), name: "mine".into(), program: "x.exe".into(), args: Vec::new(), cwd: None, options: Default::default() });
        let keymap = keymap();
        let section = Section::ALL.iter().position(|s| *s == Section::Profiles).unwrap();
        let build_for = |page: &Page| {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, found: &found, programs: &[None, None], claude: &CLAUDE, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
            build(page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5)
        };
        let parts = |frame: &Frame| frame.items.iter().filter_map(|item| item.part).collect::<Vec<_>>();
        let list = build_for(&Page { section, ..Page::default() });
        assert!(matches!(list.entries.last(), Some(Entry::NewProfile)));
        assert!(parts(&list).contains(&Part::EditProfile(0)) && parts(&list).contains(&Part::EditProfile(1)) && parts(&list).contains(&Part::NewProfile));
        let edit = list.items.iter().find(|item| item.part == Some(Part::EditProfile(1))).unwrap();
        assert_eq!(list.part_at(0.0, edit.rect.x + 2.0, edit.rect.y + 2.0), Some(Part::EditProfile(1)), "the pencil wins over the row under it");
        let mine = build_for(&Page { section, profile: Some("mine".into()), ..Page::default() });
        assert_eq!(mine.entries.len(), 1 + Field::ALL.len());
        for field in Field::ALL {
            assert!(parts(&mine).contains(&Part::ProfileField(field)), "{field:?}");
        }
        assert!(parts(&mine).contains(&Part::DeleteProfile) && parts(&mine).contains(&Part::ProfileBack));
        let pwsh = build_for(&Page { section, profile: Some("pwsh".into()), ..Page::default() });
        assert!(!parts(&pwsh).contains(&Part::DeleteProfile), "a shell nothing changes has nothing to put back");
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
        // The window is never narrower than its smallest size, column of the sections included.
        for width in [crate::settings_window::MIN_SIZE.0, 900.0, 1100.0, 1600.0] {
            for section in 0..Section::ALL.len() {
                let page = Page { section, ..Page::default() };
                let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, width, 650.0) };
                let frame = build(&page, &view, &mut |text, size, _| measure(text, size));
                for (entry, row) in frame.entries.iter().zip(&frame.rows) {
                    let Entry::Setting(setting) = entry else { continue };
                    if matches!(setting.control, Control::Themes | Control::Profiles | Control::Layouts(_)) {
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
