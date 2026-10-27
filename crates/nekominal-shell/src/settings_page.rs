//! The settings page: a column of the sections at the left, with the search line on top of it, and at the right the title
//! of the section and its rows.
//!
//! The page is a display list. `build` turns the state of the page and the settings into rectangles with what is drawn
//! in them and what they do when pressed (`Part`), so that drawing and hit testing read the same geometry. The rows are
//! those of `settings_ui::TABLE`; nothing here knows what a setting means. Pure apart from the text measure the caller
//! hands in.

use nekominal_model::{
    commands::Command,
    keymap::{Keymap, Shortcut},
    profile_edit::{self, Field},
    profiles::Profile,
    settings::Settings,
    settings_ui::{self, Control, Descriptor, Range, Section, Value},
    strings::{Lang, Text},
    themes,
    triggers::{self, Action, Trigger},
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
const HEADING: f32 = 42.0;
/// The miniatures of the layouts of the tabs: at most this wide, three in a line, as tall as this share of their width.
const LAYOUT_THUMB: f32 = 170.0;
const LAYOUT_RATIO: f32 = 0.6;
/// The one inset of the page: the title, the labels, the controls, the grids and the reset buttons all keep this far from
/// the ends of the hairlines between the rows, left and right.
const INNER: f32 = MARGIN;
/// The items of a section are plates of their own with a gap between them (the approved page section design).
const ROW: f32 = 48.0;
const ROW_GAP: f32 = 4.0;
/// Below a grid (the colour schemes, the layouts) before the next row, as much as above the text of a row.
const GRID_BOTTOM: f32 = INNER;
const CONTROL: f32 = 28.0;
const SWITCH: (f32, f32) = (44.0, 24.0);
const SLIDER_WIDTH: f32 = 200.0;
const FIELD_WIDTH: f32 = 280.0;
const WIDE_FIELD: f32 = 320.0;
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
    /// The command whose shortcut the next chord pressed becomes, which of its shortcuts it takes the place of, and the
    /// last chord pressed when it was one a program needs (pressed again, it is taken all the same).
    pub capturing: Option<Command>,
    pub slot: KeySlot,
    pub reserved: Option<Shortcut>,
    /// The caret and the selection of the search line, and of the text field being edited.
    pub cursor: crate::textfield::Cursor,
    pub edit_cursor: crate::textfield::Cursor,
    /// The id of the profile whose editor the Profiles page shows instead of the list.
    pub profile: Option<String>,
    /// The variable of the profile's environment whose name or value is being edited, by its place in the list.
    pub env_row: usize,
    /// Why what was typed in the field being edited (its id) was not taken: it stays open until it is fixed or given up.
    pub error: Option<(&'static str, Text)>,
    /// The names of the saved layouts, read when the page opens and after each change to them, not at each frame.
    pub layouts: Vec<String>,
    /// The rule of the Triggers page that is open in place, by its index; one past the last is the one being added.
    pub trigger: Option<usize>,
}

/// Which shortcut of the command being recorded the chord pressed is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeySlot {
    /// The row was pressed: the first shortcut, and Delete takes them all away.
    #[default]
    First,
    /// A shortcut was pressed: that one, and Delete takes only it away.
    Chip(Shortcut),
    /// The add button was pressed: one more.
    New,
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
    /// Deletes the scheme of the user's own at this index of `themes::theme_names`.
    DeleteScheme(usize),
    /// Imports the colour schemes of Windows Terminal, and iTerm2 presets the user picks.
    ImportSchemes,
    ImportItermColors,
    /// The profile at this index.
    Profile(usize),
    Reset(&'static str),
    /// The row of a command on the shortcuts page: pressing it waits for the new shortcut.
    Binding(Command),
    /// Puts the shortcuts of a command back to the default.
    ResetKey(Command),
    /// The shortcut at this index of a command's: pressing it waits for the chord that takes its place.
    Chip(Command, usize),
    /// Waits for one more shortcut of a command.
    AddKey(Command),
    /// Puts every shortcut back to the default.
    ResetKeys,
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
    /// The button that chooses a file or a folder for a field in a dialog.
    Browse(Field),
    /// The scheme at this index of `themes::theme_names`, or the window's.
    ProfileScheme(Option<usize>),
    /// A point larger (true) or smaller, and the window's size again.
    ProfileFont(bool),
    ProfileFontReset,
    /// The variables of the environment: adding one, and the name, the value and the delete button of the one at this place.
    EnvAdd,
    EnvKey(usize),
    EnvValue(usize),
    EnvDelete(usize),
    /// The snippets: adding one, and the name, the text and the delete button of the one at this place.
    SnippetAdd,
    SnippetName(usize),
    SnippetInput(usize),
    SnippetDelete(usize),
    /// The saved layouts: saving this window's under a new name, and the name, opening here and deleting the one at this
    /// place of `Page::layouts`.
    LayoutSave,
    LayoutName(usize),
    LayoutOpen(usize),
    LayoutDelete(usize),
    /// Sends a test push with the settings of the Terminal page.
    PushTest,
    /// The Triggers page: adding a rule, and of the rule at this index its plate (which opens and closes it), its switch,
    /// the fields of its form, its "before the line ends" chip, the action and the colour at an index, and the ready-made
    /// rule at an index of `triggers::PRESETS`.
    TriggerAdd,
    TriggerOpen(usize),
    TriggerSwitch(usize),
    TriggerName(usize),
    TriggerPattern(usize),
    TriggerInstant(usize),
    TriggerProgram(usize),
    TriggerText(usize),
    TriggerAction(usize, usize),
    TriggerColor(usize, usize),
    TriggerPreset(usize),
}

/// The ids the fields of a rule of the Triggers page are edited under (`Page::editing`); `Page::env_row` is the rule's
/// index, one past the last for the one being added.
pub const TRIGGER_NAME_ID: &str = "trigger.name";
pub const TRIGGER_PATTERN_ID: &str = "trigger.pattern";
pub const TRIGGER_PROGRAM_ID: &str = "trigger.program";
pub const TRIGGER_TEXT_ID: &str = "trigger.text";

/// The id a field of a rule is edited under, and the index of its rule.
pub fn trigger_field(part: Part) -> Option<(&'static str, usize)> {
    match part {
        Part::TriggerName(i) => Some((TRIGGER_NAME_ID, i)),
        Part::TriggerPattern(i) => Some((TRIGGER_PATTERN_ID, i)),
        Part::TriggerProgram(i) => Some((TRIGGER_PROGRAM_ID, i)),
        Part::TriggerText(i) => Some((TRIGGER_TEXT_ID, i)),
        _ => None,
    }
}

/// The rule at `index` of the settings, or the one being added past the last: a highlight, as the file's default.
pub fn trigger_at(settings: &Settings, index: usize) -> Trigger {
    settings.triggers.get(index).cloned().unwrap_or_else(|| Trigger {
        name: String::new(),
        pattern: String::new(),
        enabled: true,
        action: Action::Highlight { color: triggers::DEFAULT_HIGHLIGHT.to_owned() },
        program: None,
        instant: false,
    })
}

/// What the tag of a rule says it does: a send also says what it types, its Enter as ⏎.
pub fn action_text(action: &Action, lang: Lang) -> String {
    let name = ACTIONS[action.index()].get(lang);
    match action {
        Action::Send { text, .. } => format!("{name} {}", text.replace('\r', "\u{23CE}")),
        _ => name.to_owned(),
    }
}

/// The names of the actions, in the order of `Action::index`.
const ACTIONS: [Text; 5] = [Text::TriggerHighlight, Text::AgentNotification, Text::TriggerBadge, Text::TriggerSend, Text::TriggerBell];

/// A secret as its field shows it while it is not edited: its first three and last four characters, the rest dots, and
/// a short one all dots.
pub fn masked(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 10 {
        return "\u{2022}".repeat(chars.len());
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}{}{tail}", "\u{2022}".repeat(chars.len() - 7))
}

/// The id the name of a saved layout is edited under; `Page::env_row` is its place, one past the last for a new one.
pub const LAYOUT_NAME_ID: &str = "layout.name";

/// The ids the fields of the snippets are edited under (`Page::editing`); `Page::env_row` is the snippet's place, one past
/// the last for the one being added.
pub const SNIPPET_NAME_ID: &str = "snippet.name";
pub const SNIPPET_INPUT_ID: &str = "snippet.input";

/// A snippet's text as its field shows it: the Enter that runs it written `\r`, which a line of text cannot hold.
pub fn snippet_shown(input: &str) -> String {
    input.replace('\r', "\\r")
}

/// The text of a snippet from its field: `\r` back to the Enter it stands for.
// shortcut: a snippet that types a backslash and an r cannot be written in the field; the file still takes it.
pub fn snippet_input(shown: &str) -> String {
    shown.replace("\\r", "\r")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Text,
    Quiet,
    /// What says why a value was not taken.
    Error,
    /// What says something went well: the test push got through.
    Good,
}

/// The look of a small pill: a pattern in a rule's plate, or the tag of its action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Code,
    Plain,
    /// A send: it types into a program.
    Warn,
    /// A highlight: it marks the text.
    Mark,
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
    /// An item of a section: a plate of its own, lighter while it has the keyboard.
    Row { focus: bool },
    /// A line across the page: where rows scrolled under the title are cut off.
    Rule,
    Button { selected: bool, hover: bool },
    Field { editing: bool },
    Switch { on: bool, hover: bool },
    /// A slider, its thumb `share` of the way (0 to 1) along the track; the rectangle is the one that takes the presses.
    Slider { share: f32 },
    Radio { on: bool, hover: bool },
    Tag(Tag),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Visual {
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
    /// A colour of a highlight (`#RRGGBB`) to choose.
    Swatch { color: &'static str, selected: bool, hover: bool },
    /// The last lines of the pane on a dark box, the byte ranges a pattern matches in each marked.
    Sample { lines: Vec<(String, Vec<(usize, usize)>)> },
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
    /// The heading of a section of the page, over the rows that follow it (`settings_ui::heading_before`).
    Heading(Text),
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
    /// At the end of the Terminal page: the snippets, a line each, and the button that adds one.
    Snippets,
    /// Under the profiles: the saved layouts of windows, a line each, and the button that saves this window's.
    Layouts,
    /// Under the phone notifications: the button that sends a test, and what the last one came to.
    PushTest,
    /// The rule of the Triggers page at this index (one past the last is the one being added), and the ready-made ones.
    Trigger(usize),
    TriggerPresets,
}

impl Entry {
    pub fn section(&self) -> Section {
        match self {
            Entry::Heading(text) => settings_ui::heading_section(*text),
            Entry::Setting(row) => row.section,
            Entry::Key(_) => Section::Keys,
            Entry::Claude => Section::Agents,
            Entry::Snippets | Entry::PushTest => Section::Terminal,
            Entry::Trigger(_) | Entry::TriggerPresets => Section::Triggers,
            Entry::Layouts => Section::Profiles,
            Entry::About | Entry::Repo | Entry::Contributors => Section::About,
            Entry::NewProfile | Entry::ProfileHeader | Entry::ProfileField(_) => Section::Profiles,
        }
    }
}

/// What Enter presses on a row that is not a setting or a command: the button the row stands for, if it has one.
pub fn enter_part(entry: &Entry) -> Option<Part> {
    match entry {
        Entry::NewProfile => Some(Part::NewProfile),
        Entry::Snippets => Some(Part::SnippetAdd),
        Entry::Layouts => Some(Part::LayoutSave),
        Entry::ProfileHeader => Some(Part::ProfileBack),
        // The variables: add one. The schemes and the font size are many buttons, none of which is the row's.
        Entry::ProfileField(Field::Env) => Some(Part::EnvAdd),
        Entry::ProfileField(Field::Scheme | Field::FontSize) => None,
        Entry::ProfileField(field) => Some(Part::ProfileField(*field)),
        Entry::PushTest => Some(Part::PushTest),
        Entry::Trigger(i) => Some(Part::TriggerOpen(*i)),
        _ => None,
    }
}

/// The row the keyboard moves to from `focus`, down or up, over the headings: a heading has no plate to show it has the
/// keyboard, nor anything to press. At either end it stays; None when there is no row.
pub fn step_focus(entries: &[Entry], focus: Option<usize>, down: bool) -> Option<usize> {
    let rows = || entries.iter().enumerate().filter(|(_, entry)| !matches!(entry, Entry::Heading(_))).map(|(i, _)| i);
    match (focus, down) {
        (None, true) => rows().next(),
        (None, false) => rows().last(),
        (Some(at), true) => rows().find(|i| *i > at).or_else(|| rows().last()),
        (Some(at), false) => rows().filter(|i| *i < at).last().or_else(|| rows().next()),
    }
}

/// What a rule is called on its plate and to a screen reader: its name, or for the one being added what adds it.
pub fn trigger_title(rule: &Trigger, lang: Lang) -> String {
    if rule.name.is_empty() { Text::TriggerAdd.get(lang).to_owned() } else { rule.name.clone() }
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
        Section::Triggers => '\u{E945}',
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
    /// What the last test push came to, and how long ago it was sent.
    pub push_test: Option<(std::time::Duration, crate::push::Test)>,
    /// The last lines of the focused pane that are not blank, which the form of a rule matches its pattern against.
    pub sample: &'a [String],
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
pub fn entries(page: &Page, lang: Lang, keymap: &Keymap, settings: &Settings) -> Vec<Entry> {
    if !page.query.trim().is_empty() {
        let mut found: Vec<Entry> = settings_ui::search_with(&page.query, lang, keymap, &settings.schemes)
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
    } else if section == Section::Triggers {
        // The rules, the one being added after them while it is open, then the ready-made ones.
        let count = settings.triggers.len() + usize::from(page.trigger == Some(settings.triggers.len()));
        (0..count).map(Entry::Trigger).chain([Entry::TriggerPresets]).collect()
    } else if section == Section::Profiles && page.profile.is_some() {
        std::iter::once(Entry::ProfileHeader).chain(Field::ALL.map(Entry::ProfileField)).collect()
    } else {
        // The Agent page starts with the agents it knows, before the rows of the table.
        let agents = (section == Section::Agents).then_some(Entry::Claude);
        let about = if section == Section::About { vec![Entry::About, Entry::Repo, Entry::Contributors] } else { Vec::new() };
        let new_profile = (section == Section::Profiles).then_some(Entry::NewProfile).into_iter().chain((section == Section::Profiles).then_some(Entry::Layouts));
        // The snippets are a section of their own, not the last rows' (the push rows end the Terminal page before them).
        let snippets = (section == Section::Terminal).then_some([Entry::Heading(Text::SettingsSnippetsHeading), Entry::Snippets]).into_iter().flatten();
        // A row that starts a section of the page comes under its heading; the test of the push comes after its rows.
        let rows = settings_ui::in_section(section).flat_map(|row| {
            let test = (row.id == "notifications.push.events").then_some(Entry::PushTest);
            settings_ui::heading_before(row.id).map(Entry::Heading).into_iter().chain([Entry::Setting(row)]).chain(test)
        });
        agents.into_iter().chain(about).chain(rows).chain(new_profile).chain(snippets).collect()
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
        (Control::Secret, Value::Text(text)) => masked(&text),
        (Control::Flags(options), Value::Flags(bits)) => options.iter().enumerate().filter(|(i, _)| bits & 1 << i != 0).map(|(_, text)| text.get(lang)).collect::<Vec<_>>().join(", "),
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
            let names: Vec<&str> = themes::theme_names(&settings.schemes).collect();
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

/// A button with a glyph before its text; `primary` for the one a page stands for (add a rule).
fn icon_button(items: &mut Vec<Item>, rect: Rect, part: Part, ch: char, text: &str, primary: bool, hover: Option<Part>) {
    item(items, rect, Some(part), Visual::Plate(Fill::Button { selected: primary, hover: hover == Some(part) }), true);
    item(items, Rect::new(rect.x + 8.0, rect.y, 16.0, rect.h), None, Visual::Glyph { ch, size: 11.0, tone: Tone::Text }, true);
    item(items, Rect::new(rect.x + 28.0, rect.y, (rect.w - 32.0).max(0.0), rect.h), None, label(text, 13.0, Tone::Text, false, Align::Left), true);
}

/// A line of text that is edited when pressed: what is typed while it is (diting), else what it holds, or placeholder
/// greyed while it holds nothing.
fn text_field(items: &mut Vec<Item>, rect: Rect, part: Part, editing: Option<String>, cursor: crate::textfield::Cursor, shown: &str, placeholder: &str) {
    item(items, rect, Some(part), Visual::Plate(Fill::Field { editing: editing.is_some() }), true);
    let inner = Rect::new(rect.x + 10.0, rect.y, (rect.w - 20.0).max(0.0), rect.h);
    let visual = match editing {
        Some(text) => Visual::Edit { text, cursor: Some(cursor), placeholder: placeholder.to_owned() },
        None if shown.is_empty() => label(placeholder, 13.0, Tone::Quiet, false, Align::Left),
        None => label(shown, 13.0, Tone::Quiet, false, Align::Left),
    };
    item(items, inner, None, visual, true);
}

/// The profile whose editor the page shows.
pub fn edited<'a>(page: &Page, view: &View<'a>) -> Option<&'a Profile> {
    let id = page.profile.as_deref()?;
    view.profiles.iter().find(|p| p.id == id)
}

fn label(text: impl Into<String>, size: f32, tone: Tone, bold: bool, align: Align) -> Visual {
    Visual::Label { text: text.into(), size, tone, bold, align }
}

// --- the parts every page is made of: a section's heading, and items, each a plate with its label at the left and its
// controls placed from the right end inward. A page that draws a row draws it with these, so every row looks the same.

/// The heading of a section of a page, over its items; its height.
fn section_heading(items: &mut Vec<Item>, x: f32, y: f32, width: f32, text: &str) -> f32 {
    // More room above than below, so that the heading goes with the items under it, not the ones before.
    item(items, Rect::new(x + INNER, y + HEADING - 24.0, (width - 2.0 * INNER).max(0.0), 18.0), None, label(text, 12.0, Tone::Quiet, true, Align::Left), true);
    HEADING
}

/// The plate of an item, taking the presses of `part` anywhere on it (a toggle), or none.
fn row_plate(items: &mut Vec<Item>, rect: Rect, part: Option<Part>, focus: bool) {
    item(items, rect, part, Visual::Plate(Fill::Row { focus }), true);
}

/// The label of an item at the left of its first line, `room` wide, with what it is for under it in smaller words.
fn row_label(items: &mut Vec<Item>, rect: Rect, room: f32, text: &str, note: Option<&str>, tone: Tone) {
    let line = ROW - ROW_GAP;
    match note {
        Some(note) => {
            item(items, Rect::new(rect.x + INNER, rect.y + 5.0, room, line / 2.0), None, label(text, 13.0, tone, false, Align::Left), true);
            item(items, Rect::new(rect.x + INNER, rect.y + line / 2.0 - 1.0, room, line / 2.0), None, label(note, 12.0, Tone::Quiet, false, Align::Left), true);
        }
        None => item(items, Rect::new(rect.x + INNER, rect.y, room, line), None, label(text, 13.0, tone, false, Align::Left), true),
    }
}

/// A button with words; `primary` for the one a row stands for (update).
fn text_button(items: &mut Vec<Item>, rect: Rect, part: Part, text: &str, primary: bool, hover: Option<Part>) {
    item(items, rect, Some(part), Visual::Plate(Fill::Button { selected: primary, hover: hover == Some(part) }), true);
    item(items, rect, None, label(text, 13.0, Tone::Text, false, Align::Center), true);
}

/// An option of a choice: the chosen one stands out, the others are quiet.
fn option_button(items: &mut Vec<Item>, rect: Rect, part: Part, text: &str, chosen: bool, hover: Option<Part>) {
    item(items, rect, Some(part), Visual::Plate(Fill::Button { selected: chosen, hover: hover == Some(part) }), true);
    item(items, rect, None, label(text, 13.0, if chosen { Tone::Text } else { Tone::Quiet }, false, Align::Center), true);
}

/// A square button with a glyph: add, delete, open, put back.
fn glyph_button(items: &mut Vec<Item>, rect: Rect, part: Part, ch: char, hover: Option<Part>) {
    item(items, rect, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
    item(items, rect, None, Visual::Glyph { ch, size: 11.0, tone: Tone::Quiet }, true);
}

/// Places the controls of an item's line from its right end inward, a gap between each.
struct Controls {
    /// Where the next control ends.
    right: f32,
    /// The top of a control of the usual height, centred on the line.
    mid: f32,
}

impl Controls {
    fn new(rect: Rect) -> Self {
        Controls { right: rect.x + rect.w - INNER, mid: rect.y + (ROW - ROW_GAP - CONTROL) / 2.0 }
    }

    /// The rectangle of the next control, `width` wide and `height` high, centred on the line.
    fn take(&mut self, width: f32, height: f32) -> Rect {
        let rect = Rect::new(self.right - width, self.mid + (CONTROL - height) / 2.0, width, height);
        self.right = rect.x - GAP;
        rect
    }

    /// A button with words, as wide as they are.
    fn button(&mut self, measure: &mut dyn FnMut(&str, f32, bool) -> f32, text: &str) -> Rect {
        self.take(measure(text, 13.0, false) + 28.0, CONTROL)
    }
}

/// What the test row of the push says: under its label when the last test was sent, and beside its button what it came to.
fn push_test_texts(test: Option<&(std::time::Duration, crate::push::Test)>, lang: Lang) -> (Option<String>, Option<(String, Tone)>) {
    use crate::push::Test;
    let Some((age, test)) = test else { return (None, None) };
    let minutes = age.as_secs() / 60;
    let ago = if minutes == 0 { Text::PushJustNow.get(lang).to_owned() } else { Text::PushMinutesAgo.format(lang, &[&minutes]) };
    match test {
        Test::Sending => (None, Some((Text::PushSending.get(lang).to_owned(), Tone::Quiet))),
        Test::Sent => (Some(Text::PushLastSent.format(lang, &[&ago])), Some((format!("\u{25CF} {}", Text::PushDelivered.get(lang)), Tone::Good))),
        Test::Failed(why) => (Some(Text::PushLastTried.format(lang, &[&ago])), Some((why.clone(), Tone::Error))),
    }
}

/// `text` cut to `room` with an ellipsis when it is wider.
fn fit(text: &str, room: f32, size: f32, measure: &mut dyn FnMut(&str, f32, bool) -> f32) -> String {
    if measure(text, size, false) <= room {
        return text.to_owned();
    }
    let mut cut: String = text.to_owned();
    while !cut.is_empty() && measure(&format!("{cut}\u{2026}"), size, false) > room {
        cut.pop();
    }
    format!("{cut}\u{2026}")
}

/// The width of the labels of the form of a rule, left of its fields.
const FORM_LABEL: f32 = 110.0;
/// One line of the form of a rule: a field and the gap under it.
const FORM_LINE: f32 = CONTROL + 2.0 * GAP;
/// A line of the sample of the pane in the form of a rule.
const SAMPLE_LINE: f32 = 18.0;

/// The plate of the rule at `index` of the Triggers page: its name, its pattern, the tag of its action and its switch,
/// and, while it is open, its form under them. Its height.
fn trigger_entry(items: &mut Vec<Item>, page: &Page, view: &View, measure: &mut dyn FnMut(&str, f32, bool) -> f32, index: usize, at: Rect, focus: bool) -> f32 {
    let (lang, hover) = (view.lang, page.hover);
    let rule = trigger_at(view.settings, index);
    let open = page.trigger == Some(index);
    let plate = items.len();
    row_plate(items, Rect::new(at.x, at.y, at.w, 0.0), None, focus || open);
    let head = Rect::new(at.x, at.y, at.w, ROW - ROW_GAP);
    // The line of the name opens and closes the form; a press on the form between its fields does nothing.
    item(items, Rect::new(head.x + INNER, head.y, (head.w - 2.0 * INNER).max(0.0), head.h), Some(Part::TriggerOpen(index)), label("", 13.0, Tone::Quiet, false, Align::Left), true);
    let mut controls = Controls::new(head);
    let switch = controls.take(SWITCH.0, SWITCH.1);
    item(items, switch, Some(Part::TriggerSwitch(index)), Visual::Plate(Fill::Switch { on: rule.is_on(), hover: hover == Some(Part::TriggerSwitch(index)) }), true);
    let tag_text = action_text(&rule.action, lang);
    let tag = controls.take(measure(&tag_text, 12.0, false) + 18.0, 22.0);
    let look = match rule.action {
        Action::Highlight { .. } => Tag::Mark,
        Action::Send { .. } => Tag::Warn,
        _ => Tag::Plain,
    };
    item(items, tag, None, Visual::Plate(Fill::Tag(look)), true);
    item(items, tag, None, label(tag_text, 12.0, Tone::Text, false, Align::Center), true);
    // The name, then the pattern in a pill, both cut to what is left of the line.
    let left = head.x + INNER;
    let room = (controls.right - left).max(0.0);
    let name = trigger_title(&rule, lang);
    let name_w = measure(&name, 13.0, true).min(room * 0.5);
    item(items, Rect::new(left, head.y, name_w, head.h), None, label(fit(&name, name_w, 13.0, measure), 13.0, if rule.name.is_empty() { Tone::Quiet } else { Tone::Text }, true, Align::Left), true);
    let code_x = left + name_w + 10.0;
    let code_w = (measure(&rule.pattern, 12.0, false) + 16.0).min((controls.right - code_x).max(0.0));
    if !rule.pattern.is_empty() && code_w > 24.0 {
        let code = Rect::new(code_x, head.y + (head.h - 22.0) / 2.0, code_w, 22.0);
        item(items, code, None, Visual::Plate(Fill::Tag(Tag::Code)), true);
        item(items, Rect::new(code.x + 8.0, code.y, code.w - 16.0, code.h), None, label(fit(&rule.pattern, code.w - 16.0, 12.0, measure), 12.0, Tone::Quiet, false, Align::Left), true);
    }
    let mut height = ROW;
    if open {
        // The form: a label at the left of each line, its field at the right of it.
        let right = at.x + at.w - INNER;
        let field_x = left + FORM_LABEL + 14.0;
        let field_w = (right - field_x).max(0.0);
        let mut line_y = at.y + ROW;
        let editing_of = |id: &str| page.editing.as_ref().filter(|(edit_id, _)| *edit_id == id && page.env_row == index).map(|(_, text)| text.clone());
        let form_label = |items: &mut Vec<Item>, line_y: f32, text: Text| item(items, Rect::new(left, line_y, FORM_LABEL, CONTROL), None, label(text.get(lang), 13.0, Tone::Quiet, false, Align::Left), true);
        form_label(items, line_y, Text::ProfileName);
        text_field(items, Rect::new(field_x, line_y, field_w, CONTROL), Part::TriggerName(index), editing_of(TRIGGER_NAME_ID), page.edit_cursor, &rule.name, "");
        line_y += FORM_LINE;
        form_label(items, line_y, Text::TriggerPattern);
        let chip_text = Text::TriggerInstant.get(lang);
        let chip = Rect::new(right - measure(chip_text, 13.0, false) - 24.0, line_y, measure(chip_text, 13.0, false) + 24.0, CONTROL);
        option_button(items, chip, Part::TriggerInstant(index), chip_text, rule.instant, hover);
        let typed = editing_of(TRIGGER_PATTERN_ID);
        let pattern = typed.clone().unwrap_or_else(|| rule.pattern.clone());
        text_field(items, Rect::new(field_x, line_y, (chip.x - GAP - field_x).max(0.0), CONTROL), Part::TriggerPattern(index), typed, page.edit_cursor, &rule.pattern, "");
        line_y += FORM_LINE;
        if let Some((_, error)) = page.error.filter(|(id, _)| *id == TRIGGER_PATTERN_ID && page.env_row == index) {
            item(items, Rect::new(field_x, line_y - 2.0 * GAP + 2.0, field_w, 18.0), None, label(error.get(lang), 12.0, Tone::Error, false, Align::Left), true);
            line_y += 20.0;
        }
        // The last lines of the pane, what the pattern being typed matches in them marked.
        if !view.sample.is_empty() {
            let hits = triggers::sample_hits(&pattern, view.sample);
            let lines: Vec<(String, Vec<(usize, usize)>)> = view.sample.iter().cloned().zip(hits).collect();
            let box_h = lines.len() as f32 * SAMPLE_LINE + 16.0;
            item(items, Rect::new(field_x, line_y, field_w, box_h), None, Visual::Sample { lines }, true);
            line_y += box_h + 2.0 * GAP;
        }
        form_label(items, line_y, Text::TriggerProgram);
        let note = Text::TriggerProgramNote.get(lang);
        let note_w = measure(note, 12.0, false).min(field_w / 2.0);
        item(items, Rect::new(right - note_w, line_y, note_w, CONTROL), None, label(note, 12.0, Tone::Quiet, false, Align::Right), true);
        let program = rule.program.clone().unwrap_or_default();
        text_field(items, Rect::new(field_x, line_y, (right - note_w - GAP - field_x).max(0.0), CONTROL), Part::TriggerProgram(index), editing_of(TRIGGER_PROGRAM_ID), page.edit_cursor, &program, "");
        line_y += FORM_LINE;
        form_label(items, line_y, Text::TriggerAction);
        let widths: Vec<f32> = ACTIONS.iter().map(|text| measure(text.get(lang), 13.0, false) + 24.0).collect();
        let places = wrap(&widths, field_w, GAP);
        for (k, (text, (line, offset))) in ACTIONS.iter().zip(&places).enumerate() {
            let button = Rect::new(field_x + offset, line_y + *line as f32 * (CONTROL + GAP), widths[k], CONTROL);
            option_button(items, button, Part::TriggerAction(index, k), text.get(lang), rule.action.index() == k, hover);
        }
        line_y += places.iter().map(|(line, _)| *line + 1).max().unwrap_or(1) as f32 * (CONTROL + GAP) + GAP;
        // What the action needs: the text of a notification or of a send, the colour of a highlight.
        match &rule.action {
            Action::Notify { text } => {
                form_label(items, line_y, Text::TriggerNotifyText);
                text_field(items, Rect::new(field_x, line_y, field_w, CONTROL), Part::TriggerText(index), editing_of(TRIGGER_TEXT_ID), page.edit_cursor, text, "$0");
                line_y += FORM_LINE;
            }
            Action::Send { text, .. } => {
                form_label(items, line_y, Text::TriggerSendText);
                text_field(items, Rect::new(field_x, line_y, field_w, CONTROL), Part::TriggerText(index), editing_of(TRIGGER_TEXT_ID), page.edit_cursor, &snippet_shown(text), Text::SnippetInput.get(lang));
                line_y += FORM_LINE;
            }
            Action::Highlight { color } => {
                form_label(items, line_y, Text::TriggerColor);
                for (k, swatch) in triggers::HIGHLIGHTS.iter().enumerate() {
                    let part = Part::TriggerColor(index, k);
                    let rect = Rect::new(field_x + k as f32 * (CONTROL + GAP), line_y, CONTROL, CONTROL);
                    item(items, rect, Some(part), Visual::Swatch { color: swatch, selected: color.eq_ignore_ascii_case(swatch), hover: hover == Some(part) }, true);
                }
                line_y += FORM_LINE;
            }
            Action::Badge | Action::Bell => {}
        }
        height = line_y - at.y - 2.0 * GAP + GRID_BOTTOM;
    }
    items[plate].rect.h = height - ROW_GAP;
    height
}

pub fn build(page: &Page, view: &View, measure: &mut dyn FnMut(&str, f32, bool) -> f32) -> Frame {
    let lang = view.lang;
    let Rect { x: ox, y: oy, w: width, h: height } = view.area;
    let mut items = Vec::new();
    let hover = page.hover;

    // The column of the sections at the left: the search line on top, a line a section (with what a search finds in it),
    // and the button for the file at the bottom.
    let searching = !page.query.trim().is_empty();
    let entries = entries(page, lang, view.keymap, view.settings);
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
    // Beside the title of the shortcuts page, while any differs from its default: all of them back at once.
    if !searching && Section::ALL.get(page.section) == Some(&Section::Keys) && !view.settings.keybindings.is_empty() {
        let text = Text::KeysResetAll.get(lang);
        let button = Rect::new(x0 + content_width - INNER - measure(text, 13.0, false) - 28.0, oy + 18.0, measure(text, 13.0, false) + 28.0, CONTROL);
        text_button(&mut items, button, Part::ResetKeys, text, false, hover);
        // The title does not scroll: neither does the button beside it.
        for fixed in items.iter_mut().rev().take(2) {
            fixed.scrolls = false;
        }
    }
    // At the right end of the title of the Triggers page the button that adds a rule, as the shortcuts page has its reset,
    // and under the title what a rule is.
    if !searching && Section::ALL.get(page.section) == Some(&Section::Triggers) {
        let text = Text::TriggerAdd.get(lang);
        let width = measure(text, 13.0, false) + 40.0;
        let button = Rect::new(x0 + content_width - INNER - width, oy + 18.0, width, CONTROL);
        icon_button(&mut items, button, Part::TriggerAdd, '\u{E710}', text, true, hover);
        item(&mut items, Rect::new(x0 + INNER, oy + 46.0, (content_width - 2.0 * INNER).max(0.0), 18.0), None, label(Text::TriggersIntro.get(lang), 12.0, Tone::Quiet, false, Align::Left), false);
        for fixed in items.iter_mut().rev().take(4) {
            fixed.scrolls = false;
        }
    }
    // The Triggers page has a line under its title: its rows start under it.
    let intro = !searching && Section::ALL.get(page.section) == Some(&Section::Triggers);
    let top = oy + CONTENT_TOP + if intro { 14.0 } else { 0.0 };
    let viewport = Rect::new(page_x, top, (width - nav.w).max(0.0), (oy + height - top).max(0.0));
    let mut rows = Vec::new();
    let mut y = top;
    if entries.is_empty() {
        item(&mut items, Rect::new(x0 + INNER, y, (content_width - 2.0 * INNER).max(0.0), ROW), None, label(Text::SettingsEmpty.get(lang), 13.0, Tone::Quiet, false, Align::Left), true);
        y += ROW;
    }
    let theme_names: Vec<&str> = themes::theme_names(&view.settings.schemes).collect();
    let mut last_section = None;
    // The bottom of the left row of a pair that shares a line, while the right one is still to come.
    let mut pair_bottom: Option<f32> = None;
    for (n, entry) in entries.iter().enumerate() {
        let focus = page.focus == Some(n);
        // What a search finds comes by section, under the name of each.
        if searching && last_section != Some(entry.section()) {
            last_section = Some(entry.section());
            y += section_heading(&mut items, x0, y, content_width, entry.section().text().get(lang));
        }
        let start = y;
        let first = items.len();
        // Where the entry is across the page: all of it, or a half of a pair's line.
        let mut slot = (x0, content_width);
        // The bottom of the entry when it is not where the next one starts (the left of a pair).
        let mut end = None;
        match *entry {
            Entry::Heading(text) => y += section_heading(&mut items, x0, y, content_width, text.get(lang)),
            Entry::PushTest => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                let (note, status) = push_test_texts(view.push_test.as_ref(), lang);
                let mut controls = Controls::new(rect);
                let text = Text::PushTestSend.get(lang);
                let button = controls.take(measure(text, 13.0, false) + 40.0, CONTROL);
                icon_button(&mut items, button, Part::PushTest, '\u{E72A}', text, false, hover);
                if let Some((status, tone)) = status {
                    let at = controls.take(measure(&status, 13.0, false).min((controls.right - rect.x - INNER) / 2.0).max(0.0), CONTROL);
                    item(&mut items, at, None, label(status, 13.0, tone, false, Align::Right), true);
                }
                row_label(&mut items, rect, (controls.right - rect.x - INNER).max(0.0), Text::PushTest.get(lang), note.as_deref(), Tone::Text);
                y += ROW;
            }
            Entry::Trigger(i) => y += trigger_entry(&mut items, page, view, measure, i, Rect::new(x0, y, content_width, 0.0), focus),
            Entry::TriggerPresets => {
                // A chip a ready-made rule, as many in a line as fit inside the inset.
                let texts: Vec<&str> = triggers::PRESETS.iter().map(|text| text.get(lang)).collect();
                let widths: Vec<f32> = texts.iter().map(|text| measure(text, 13.0, false) + 40.0).collect();
                let places = wrap(&widths, (content_width - 2.0 * INNER).max(0.0), GAP);
                let lines = places.iter().map(|(line, _)| *line + 1).max().unwrap_or(1);
                let rect = Rect::new(x0, y, content_width, INNER + lines as f32 * (CONTROL + GAP) - GAP + GRID_BOTTOM);
                // The ready-made rules are offered on the page itself, under the rules, not as one more item: their plate
                // shows only while they have the keyboard.
                if focus {
                    row_plate(&mut items, rect, None, focus);
                }
                for (i, (text, (line, offset))) in texts.iter().zip(&places).enumerate() {
                    let chip = Rect::new(rect.x + INNER + offset, rect.y + INNER + *line as f32 * (CONTROL + GAP), widths[i], CONTROL);
                    icon_button(&mut items, chip, Part::TriggerPreset(i), '\u{E710}', text, false, hover);
                }
                y += rect.h;
            }
            Entry::Key(command) => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                let capturing = page.capturing == Some(command);
                row_plate(&mut items, rect, Some(Part::Binding(command)), focus || capturing);
                row_label(&mut items, rect, (rect.w - 2.0 * INNER) / 2.0, &command.label(lang), None, Tone::Text);
                let mut controls = Controls::new(rect);
                if capturing {
                    let text = if page.reserved.is_some() { Text::KeyReserved } else { Text::KeysPress };
                    item(&mut items, Rect::new(rect.x + rect.w / 2.0, rect.y, rect.w / 2.0 - INNER, rect.h), None, label(text.get(lang), 12.0, Tone::Quiet, false, Align::Right), true);
                    y += ROW;
                    rows.push(Rect::new(x0, start, content_width, y - start));
                    continue;
                }
                // Each shortcut is a button that records the chord taking its place; the add button records one more.
                for (index, shortcut) in view.keymap.shortcuts(command).iter().enumerate().rev() {
                    let text = shortcut.to_string();
                    let chip = controls.take(measure(&text, 12.0, false) + 20.0, 24.0);
                    let part = Part::Chip(command, index);
                    item(&mut items, chip, Some(part), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(part) }), true);
                    item(&mut items, chip, None, label(text, 12.0, Tone::Quiet, false, Align::Center), true);
                }
                glyph_button(&mut items, controls.take(RESET, RESET), Part::AddKey(command), '\u{E710}', hover);
                // The reset button left of the shortcuts it puts back.
                if nekominal_model::keybindings::is_modified(&view.settings.keybindings, command) {
                    glyph_button(&mut items, controls.take(RESET, RESET), Part::ResetKey(command), '\u{E777}', hover);
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
                    text_button(&mut items, button, Part::Update, text, true, hover);
                    at = button.x + button.w + GAP;
                }
                let text = Text::UpdateCheck.get(lang);
                let button = Rect::new(at, buttons_y, measure(text, 13.0, false) + 28.0, CONTROL);
                text_button(&mut items, button, Part::CheckUpdate, text, false, hover);
                if newer && !view.scoop {
                    let x = button.x + button.w + 12.0;
                    item(&mut items, Rect::new(x, buttons_y, (rect.x + rect.w - INNER - x).max(0.0), CONTROL), None, label(Text::UpdateNotScoop.get(lang), 12.0, Tone::Quiet, false, Align::Left), true);
                }
                y += ABOUT_ROW;
            }
            Entry::Repo => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                row_label(&mut items, rect, (rect.w - 2.0 * INNER) / 3.0, Text::SettingsRepo.get(lang), None, Tone::Text);
                let text = Text::CommonOpen.get(lang);
                let button = Controls::new(rect).button(measure, text);
                text_button(&mut items, button, Part::OpenRepo, text, false, hover);
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
                row_plate(&mut items, rect, None, focus);
                row_label(&mut items, rect, rect.w - 2.0 * INNER, Text::SettingsContributors.get(lang), None, Tone::Text);
                for (i, (login, (line, offset))) in people.iter().zip(&places).enumerate() {
                    let chip = Rect::new(rect.x + INNER + offset, rect.y + ROW + *line as f32 * (CONTROL + GAP), widths[i], CONTROL);
                    text_button(&mut items, chip, Part::Contributor(i), login, false, hover);
                }
                y += height;
            }
            Entry::Snippets => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                let right = rect.x + rect.w - INNER;
                let room = (right - x0 - INNER).max(0.0);
                let text = Text::SnippetsAdd.get(lang);
                let add = Controls::new(rect).take(measure(text, 13.0, false) + 40.0, CONTROL);
                row_label(&mut items, rect, (add.x - GAP - rect.x - INNER).max(0.0), Text::SettingsSnippets.get(lang), None, Tone::Text);
                icon_button(&mut items, add, Part::SnippetAdd, '\u{E710}', text, false, hover);
                // A line a snippet: its name, what it types and the button that deletes it; then the one being added.
                let mut line_y = y + ROW - ROW_GAP;
                let count = view.settings.snippets.len();
                let adding = page.editing.as_ref().is_some_and(|(id, _)| *id == SNIPPET_INPUT_ID) && page.env_row == count;
                let lines = view.settings.snippets.iter().map(|s| (s.name.as_str(), snippet_shown(&s.input))).chain(adding.then(|| ("", String::new())));
                for (i, (name, shown)) in lines.enumerate() {
                    let name_w = ((room - CONTROL - 2.0 * GAP) * 0.3).max(0.0);
                    let name_rect = Rect::new(x0 + INNER, line_y, name_w, CONTROL);
                    let input_rect = Rect::new(name_rect.x + name_w + GAP, line_y, (room - name_w - CONTROL - 2.0 * GAP).max(0.0), CONTROL);
                    let this_row = page.env_row == i;
                    for (rect, part, id, shown, placeholder) in [
                        (name_rect, Part::SnippetName(i), SNIPPET_NAME_ID, name, Text::ProfileName.get(lang)),
                        (input_rect, Part::SnippetInput(i), SNIPPET_INPUT_ID, shown.as_str(), Text::SnippetInput.get(lang)),
                    ] {
                        let editing = page.editing.as_ref().filter(|(edit_id, _)| this_row && *edit_id == id).map(|(_, text)| text.clone());
                        text_field(&mut items, rect, part, editing, page.edit_cursor, shown, placeholder);
                    }
                    if i < count {
                        glyph_button(&mut items, Rect::new(right - CONTROL, line_y, CONTROL, CONTROL), Part::SnippetDelete(i), '\u{E711}', hover);
                    }
                    line_y += CONTROL + GAP;
                }
                y = if line_y > y + ROW - ROW_GAP { line_y - GAP + GRID_BOTTOM } else { y + ROW };
            }
            Entry::Layouts => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                let right = rect.x + rect.w - INNER;
                let text = Text::AppSaveLayout.get(lang);
                let save = Controls::new(rect).take(measure(text, 13.0, false) + 40.0, CONTROL);
                row_label(&mut items, rect, (save.x - GAP - rect.x - INNER).max(0.0), Text::SettingsLayouts.get(lang), None, Tone::Text);
                icon_button(&mut items, save, Part::LayoutSave, '\u{E74E}', text, false, hover);
                // A line a layout: its name (pressed, it is renamed), the buttons that open it here and delete it; then the
                // one being saved.
                let mut line_y = y + ROW - ROW_GAP;
                let count = page.layouts.len();
                let saving = page.editing.as_ref().is_some_and(|(id, _)| *id == LAYOUT_NAME_ID) && page.env_row == count;
                let names = page.layouts.iter().map(String::as_str).chain(saving.then_some(""));
                for (i, name) in names.enumerate() {
                    let open = Rect::new(right - 2.0 * CONTROL - GAP, line_y, CONTROL, CONTROL);
                    let name_rect = Rect::new(x0 + INNER, line_y, (open.x - GAP - x0 - INNER).max(0.0), CONTROL);
                    let editing = page.editing.as_ref().filter(|(id, _)| page.env_row == i && *id == LAYOUT_NAME_ID).map(|(_, text)| text.clone());
                    text_field(&mut items, name_rect, Part::LayoutName(i), editing, page.edit_cursor, name, Text::ProfileName.get(lang));
                    if i < count {
                        glyph_button(&mut items, open, Part::LayoutOpen(i), '\u{E8A7}', hover);
                        glyph_button(&mut items, Rect::new(right - CONTROL, line_y, CONTROL, CONTROL), Part::LayoutDelete(i), '\u{E74D}', hover);
                    }
                    line_y += CONTROL + GAP;
                }
                y = if line_y > y + ROW - ROW_GAP { line_y - GAP + GRID_BOTTOM } else { y + ROW };
            }
            Entry::NewProfile => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                let text = Text::ProfileNew.get(lang);
                let button = Rect::new(rect.x + INNER, rect.y + (rect.h - CONTROL) / 2.0, measure(text, 13.0, false) + 40.0, CONTROL);
                icon_button(&mut items, button, Part::NewProfile, '\u{E710}', text, false, hover);
                y += ROW;
            }
            Entry::ProfileHeader => {
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP);
                row_plate(&mut items, rect, None, focus);
                let mut controls = Controls::new(rect);
                let text = Text::ProfileBack.get(lang);
                let back = Rect::new(rect.x + INNER, controls.mid, measure(text, 13.0, false) + 40.0, CONTROL);
                icon_button(&mut items, back, Part::ProfileBack, '\u{E72B}', text, false, hover);
                let profile = edited(page, view);
                // A profile written in the settings is deleted; the entry of a shell found on the machine is taken out, which
                // puts it back as found. A shell nothing changes has neither.
                if let Some(profile) = profile.filter(|p| profile_edit::has_entry(view.settings, &p.id)) {
                    let action = if view.found.iter().any(|f| f.id == profile.id) { Text::ProfileRestore } else { Text::ProfileDelete };
                    let text = action.get(lang);
                    let button = controls.button(measure, text);
                    text_button(&mut items, button, Part::DeleteProfile, text, false, hover);
                }
                let name_x = back.x + back.w + 2.0 * GAP;
                let name = profile.map_or(String::new(), |p| p.name.clone());
                item(&mut items, Rect::new(name_x, rect.y, (controls.right - name_x).max(0.0), rect.h), None, label(name, 13.0, Tone::Text, true, Align::Left), true);
                y += ROW;
            }
            Entry::ProfileField(field) => {
                let profile = edited(page, view);
                let part = Part::ProfileField(field);
                let room = content_width - 2.0 * INNER;
                let right = x0 + content_width - INNER;
                let mid = y + (ROW - ROW_GAP - CONTROL) / 2.0;
                // The plate goes under what the row holds; its height is known once they are placed.
                let plate = items.len();
                item(&mut items, Rect::new(x0, y, content_width, 0.0), (field == Field::Elevate).then_some(part), Visual::Plate(Fill::Row { focus }), true);
                item(&mut items, Rect::new(x0 + INNER, y, room / 3.0, ROW - ROW_GAP), None, label(field.label().get(lang), 13.0, Tone::Text, false, Align::Left), true);
                let mut height = ROW;
                let error_of = |id: &str| page.error.filter(|(error_id, _)| *error_id == id).map(|(_, text)| text.get(lang));
                match field {
                    Field::Elevate => {
                        let switch = Rect::new(right - SWITCH.0, y + (ROW - ROW_GAP - SWITCH.1) / 2.0, SWITCH.0, SWITCH.1);
                        let on = profile.is_some_and(|p| p.options.elevate);
                        item(&mut items, switch, None, Visual::Plate(Fill::Switch { on, hover: hover == Some(part) }), true);
                    }
                    Field::Scheme => {
                        // The window's scheme beside the label, then the schemes as tiles, as on the Appearance page.
                        let current = profile.and_then(|p| p.options.color_scheme.as_deref());
                        let text = Text::ProfileSameAsWindow.get(lang);
                        let same = Rect::new(right - measure(text, 13.0, false) - 28.0, mid, measure(text, 13.0, false) + 28.0, CONTROL);
                        option_button(&mut items, same, Part::ProfileScheme(None), text, current.is_none(), hover);
                        let columns = (((room + TILE_GAP) / (TILE.0 + TILE_GAP)).floor() as usize).max(1);
                        let tile_w = ((room - TILE_GAP * (columns - 1) as f32) / columns as f32).max(0.0);
                        for (i, name) in theme_names.iter().enumerate() {
                            let (col, line) = (i % columns, i / columns);
                            let tile = Rect::new(x0 + INNER + col as f32 * (tile_w + TILE_GAP), y + ROW + line as f32 * (TILE.1 + TILE_GAP), tile_w, TILE.1);
                            let tile_part = Part::ProfileScheme(Some(i));
                            item(&mut items, tile, Some(tile_part), Visual::Tile { scheme: i, selected: current == Some(*name), hover: hover == Some(tile_part) }, true);
                        }
                        height += theme_names.len().div_ceil(columns) as f32 * (TILE.1 + TILE_GAP) - TILE_GAP + GRID_BOTTOM;
                    }
                    Field::FontSize => {
                        // The number steps like the font size of the window; while the profile has none, the window's shows greyed.
                        let own = profile.and_then(|p| p.options.font_size);
                        let plus = Rect::new(right - CONTROL, mid, CONTROL, CONTROL);
                        let value_box = Rect::new(plus.x - 72.0, mid, 72.0, CONTROL);
                        let minus = Rect::new(value_box.x - CONTROL, mid, CONTROL, CONTROL);
                        for (button, step, ch) in [(minus, Part::ProfileFont(false), '\u{E738}'), (plus, Part::ProfileFont(true), '\u{E710}')] {
                            item(&mut items, button, Some(step), Visual::Plate(Fill::Button { selected: false, hover: hover == Some(step) }), true);
                            item(&mut items, button, None, Visual::Glyph { ch, size: 11.0, tone: Tone::Text }, true);
                        }
                        let size = own.unwrap_or(view.settings.font.size);
                        item(&mut items, value_box, None, label(format!("{size:.0}"), 13.0, if own.is_some() { Tone::Text } else { Tone::Quiet }, false, Align::Center), true);
                        if own.is_some() {
                            let reset = Rect::new(minus.x - GAP - RESET, y + (ROW - ROW_GAP - RESET) / 2.0, RESET, RESET);
                            glyph_button(&mut items, reset, Part::ProfileFontReset, '\u{E777}', hover);
                        } else {
                            let x = x0 + INNER + room / 3.0;
                            item(&mut items, Rect::new(x, y, (minus.x - GAP - x).max(0.0), ROW - ROW_GAP), None, label(Text::ProfileSameAsWindow.get(lang), 12.0, Tone::Quiet, false, Align::Right), true);
                        }
                    }
                    Field::Env => {
                        let text = Text::ProfileAddVariable.get(lang);
                        let add = Rect::new(right - measure(text, 13.0, false) - 40.0, mid, measure(text, 13.0, false) + 40.0, CONTROL);
                        icon_button(&mut items, add, Part::EnvAdd, '\u{E710}', text, false, hover);
                        // A line a variable: its name, its value and the button that deletes it.
                        let mut line_y = y + ROW - ROW_GAP;
                        let vars: Vec<(String, String)> = profile.map(|p| p.options.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                        for (i, (key, value)) in vars.iter().enumerate() {
                            let key_w = ((room - CONTROL - 2.0 * GAP) * 0.35).max(0.0);
                            let key_rect = Rect::new(x0 + INNER, line_y, key_w, CONTROL);
                            let value_rect = Rect::new(key_rect.x + key_w + GAP, line_y, (room - key_w - CONTROL - 2.0 * GAP).max(0.0), CONTROL);
                            let delete = Rect::new(right - CONTROL, line_y, CONTROL, CONTROL);
                            let this_row = page.env_row == i;
                            for (rect, cell_part, id, shown, placeholder) in [
                                (key_rect, Part::EnvKey(i), profile_edit::ENV_KEY_ID, key, ""),
                                (value_rect, Part::EnvValue(i), profile_edit::ENV_VALUE_ID, value, Text::ProfileValue.get(lang)),
                            ] {
                                let editing = page.editing.as_ref().filter(|(edit_id, _)| this_row && *edit_id == id).map(|(_, text)| text.clone());
                                text_field(&mut items, rect, cell_part, editing, page.edit_cursor, shown, placeholder);
                            }
                            glyph_button(&mut items, delete, Part::EnvDelete(i), '\u{E711}', hover);
                            line_y += CONTROL + GAP;
                            if let Some(error) = error_of(profile_edit::ENV_KEY_ID).filter(|_| this_row) {
                                item(&mut items, Rect::new(key_rect.x, line_y - GAP + 2.0, room, 18.0), None, label(error, 12.0, Tone::Error, false, Align::Left), true);
                                line_y += 20.0;
                            }
                        }
                        height = line_y - y + if vars.is_empty() { 0.0 } else { GRID_BOTTOM - GAP };
                        height = height.max(ROW);
                    }
                    _ => {
                        // A line of text: as wide as two thirds of the row, less the button that browses for a file or folder.
                        let mut field_right = right;
                        if field.browses() {
                            let text = Text::ProfileBrowse.get(lang);
                            let browse = Rect::new(right - measure(text, 13.0, false) - 28.0, mid, measure(text, 13.0, false) + 28.0, CONTROL);
                            text_button(&mut items, browse, Part::Browse(field), text, false, hover);
                            field_right = browse.x - GAP;
                        }
                        let left = x0 + INNER + room / 3.0 + GAP;
                        let field_rect = Rect::new(left, mid, (field_right - left).max(0.0), CONTROL);
                        let editing = page.editing.as_ref().filter(|(id, _)| *id == field.id()).map(|(_, text)| text.clone());
                        let shown = profile.map(|p| profile_edit::text(p, field)).unwrap_or_default();
                        text_field(&mut items, field_rect, part, editing, page.edit_cursor, &shown, "");
                        if let Some(error) = error_of(field.id()) {
                            item(&mut items, Rect::new(field_rect.x, mid + CONTROL + 2.0, field_rect.w, 18.0), None, label(error, 12.0, Tone::Error, false, Align::Left), true);
                            height += 20.0;
                        }
                    }
                }
                items[plate].rect.h = height - ROW_GAP;
                y += height;
            }
            Entry::Setting(row) => {
                let id = row.id;
                // Two rows of a pair share a line, a half each; outside a search, where they may not be found together.
                let left = !searching && settings_ui::paired_with_next(id);
                let right_half = !searching && pair_bottom.is_some();
                let half = ((content_width - GAP) / 2.0).max(0.0);
                let (x0, content_width) = if left {
                    (x0, half)
                } else if right_half {
                    (x0 + content_width - half, half)
                } else {
                    (x0, content_width)
                };
                slot = (x0, content_width);
                let note = settings_ui::note_of(id).map(|text| text.get(lang));
                let error = page.error.filter(|(error_id, _)| *error_id == id).map(|(_, text)| text.get(lang));
                let error_room = if error.is_some() { 20.0 } else { 0.0 };
                // A control that does not fit beside the label goes on a line of its own under it.
                let label_width = measure(row.label(lang), 13.0, false);
                let control_width = match row.control {
                    Control::Toggle => SWITCH.0,
                    Control::Choice(options) | Control::Flags(options) => options.iter().map(|option| measure(option.get(lang), 13.0, false) + 30.0).sum::<f32>() - GAP,
                    Control::Slider(_) => 48.0 + 12.0 + SLIDER_WIDTH,
                    Control::Number(_) => 2.0 * CONTROL + 72.0,
                    // A line of text with a note under its label has the room of a long one: an address, a token.
                    Control::Text | Control::Secret if note.is_some() => WIDE_FIELD,
                    Control::Text | Control::Secret => FIELD_WIDTH,
                    Control::Themes | Control::Profiles | Control::Layouts(_) => 0.0,
                };
                // A setting that does nothing with the others as they are is greyed and cannot be pressed.
                let applies = row.applies(view.settings);
                let stacked = control_width > 0.0 && label_width + control_width + 2.0 * INNER + 16.0 + RESET_ROOM > content_width;
                let drop = if stacked { STACK } else { 0.0 };
                // What fits on the line of the control under the label: the options of a choice go on more lines when they do not.
                let room = content_width - 2.0 * INNER;
                let option_widths: Vec<f32> = match row.control {
                    Control::Choice(options) | Control::Flags(options) => options.iter().map(|option| measure(option.get(lang), 13.0, false) + 24.0).collect(),
                    _ => Vec::new(),
                };
                let option_lines = if stacked { wrap(&option_widths, room, GAP) } else { Vec::new() };
                let lines = option_lines.iter().map(|(line, _)| *line + 1).max().unwrap_or(1);
                let extra = (lines - 1) as f32 * (CONTROL + GAP) + error_room;
                let rect = Rect::new(x0, y, content_width, ROW - ROW_GAP + drop + extra);
                let line = Rect::new(rect.x, rect.y + drop, rect.w, ROW - ROW_GAP);
                let mid = line.y + (line.h - CONTROL) / 2.0;
                // Every control ends at the same inset from the right end of the hairline.
                let right = rect.x + rect.w - INNER;
                let value = (row.get)(view.settings);
                // A toggle is pressed anywhere on its row.
                let row_part = (applies && matches!(row.control, Control::Toggle)).then_some(Part::Toggle(id));
                item(&mut items, rect, row_part, Visual::Plate(Fill::Row { focus }), true);
                // Beside its control the label has half the row, or less where the control is wider; a note under it, a
                // sentence rather than a name, has all the room up to the control (and the reset button left of it).
                let beside = rect.w - 2.0 * INNER - RESET_ROOM - control_width - GAP;
                let label_room = if stacked {
                    rect.w - 2.0 * INNER - RESET_ROOM
                } else if note.is_some() {
                    beside.max(label_width)
                } else {
                    ((rect.w - 2.0 * INNER - RESET_ROOM) / 2.0).min(beside).max(label_width)
                };
                let tone = if applies { Tone::Text } else { Tone::Quiet };
                row_label(&mut items, rect, label_room, row.label(lang), note, tone);
                // Where the control beside the label starts: the reset button goes left of it. A control under its label (or a
                // grid) leaves the reset button at the right end of the label's line.
                let mut control_left = right;
                match (row.control, value.clone()) {
                    (Control::Toggle, Value::Bool(on)) => {
                        let switch = Rect::new(right - SWITCH.0, line.y + (line.h - SWITCH.1) / 2.0, SWITCH.0, SWITCH.1);
                        item(&mut items, switch, None, Visual::Plate(Fill::Switch { on: on && applies, hover: hover == Some(Part::Toggle(id)) }), true);
                        control_left = switch.x;
                        y += ROW + drop;
                    }
                    (Control::Choice(options), Value::Index(_)) | (Control::Flags(options), Value::Flags(_)) => {
                        // A choice has one option chosen; flags any number, each pressed on and off on its own.
                        let chosen = |i: usize| match value {
                            Value::Index(chosen) => i == chosen,
                            Value::Flags(bits) => bits & 1 << i != 0,
                            _ => false,
                        };
                        let mut at = right;
                        for (i, option) in options.iter().enumerate().rev() {
                            let text = option.get(lang);
                            let w = option_widths[i];
                            let button = match option_lines.get(i) {
                                Some((line, offset)) => Rect::new(rect.x + INNER + offset, mid + *line as f32 * (CONTROL + GAP), w, CONTROL),
                                None => Rect::new(at - w, mid, w, CONTROL),
                            };
                            option_button(&mut items, button, Part::Option(id, i), text, chosen(i), hover);
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
                        glyph_button(&mut items, minus, Part::Minus(id), '\u{E738}', hover);
                        glyph_button(&mut items, plus, Part::Plus(id), '\u{E710}', hover);
                        item(&mut items, value_box, None, label(number_text(range, n), 13.0, Tone::Text, false, Align::Center), true);
                        control_left = minus.x;
                        y += ROW + drop;
                    }
                    (Control::Text | Control::Secret, Value::Text(text)) => {
                        let field_width = control_width.min(room - RESET_ROOM);
                        let field = Rect::new(right - field_width, mid, field_width, CONTROL);
                        control_left = field.x;
                        let editing = page.editing.as_ref().filter(|(edit_id, _)| *edit_id == id).map(|(_, text)| text.clone());
                        // A secret shows masked unless it is being edited.
                        let shown = if row.control == Control::Secret { masked(&text) } else { text };
                        text_field(&mut items, field, Part::Field(id), editing, page.edit_cursor, &shown, "");
                        if let Some(error) = error {
                            item(&mut items, Rect::new(field.x, mid + CONTROL + 2.0, field.w, 18.0), None, label(error, 12.0, Tone::Error, false, Align::Left), true);
                        }
                        y += ROW + drop + error_room;
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
                            let delete = Part::DeleteScheme(i);
                            let hovered = hover == Some(part) || hover == Some(delete);
                            item(&mut items, tile, Some(part), Visual::Tile { scheme: i, selected: *name == current, hover: hovered }, true);
                            // A scheme of the user's own (imported or written) can go again; the built-in ones cannot.
                            if hovered && view.settings.schemes.iter().any(|scheme| scheme.name == *name) {
                                glyph_button(&mut items, Rect::new(tile.x + tile.w - RESET - 6.0, tile.y + 6.0, RESET, RESET), delete, '\u{E74D}', hover);
                            }
                        }
                        y += theme_names.len().div_ceil(columns) as f32 * (TILE.1 + TILE_GAP) - TILE_GAP + 12.0;
                        // Schemes come in from Windows Terminal's settings and from iTerm2 presets here, as from the palette; the
                        // second button goes under the first where they do not fit side by side.
                        let buttons = [(Part::ImportSchemes, Text::AppImportSchemes.get(lang)), (Part::ImportItermColors, Text::AppImportItermColors.get(lang))];
                        let widths: Vec<f32> = buttons.iter().map(|(_, text)| measure(text, 13.0, false) + 28.0).collect();
                        let places = wrap(&widths, inner, GAP);
                        for ((part, text), (&(line, offset), w)) in buttons.into_iter().zip(places.iter().zip(&widths)) {
                            text_button(&mut items, Rect::new(x0 + INNER + offset, y + line as f32 * (CONTROL + GAP), *w, CONTROL), part, text, false, hover);
                        }
                        let lines = places.iter().map(|(line, _)| *line + 1).max().unwrap_or(1);
                        y += lines as f32 * (CONTROL + GAP) - GAP + GRID_BOTTOM;
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
                            glyph_button(&mut items, edit, Part::EditProfile(i), '\u{E70F}', hover);
                            y += RADIO_ROW;
                        }
                    }
                    _ => y += ROW,
                }
                if row.is_modified(view.settings) {
                    let beside = !stacked && control_width > 0.0;
                    let x = if beside { control_left - GAP - RESET } else { right - RESET };
                    let reset = Rect::new(x, rect.y + (ROW - ROW_GAP - RESET) / 2.0, RESET, RESET);
                    glyph_button(&mut items, reset, Part::Reset(id), '\u{E777}', hover);
                }
                // The left row of a pair leaves the line to the right one; the line is as tall as the taller of them.
                if left {
                    (end, pair_bottom) = (Some(y), Some(y));
                    y = start;
                } else if let Some(bottom) = pair_bottom.take() {
                    y = y.max(bottom);
                }
            }
        }
        // An item's plate holds all of its lines (the snippets, the saved layouts, the tiles under the colour scheme), not only
        // the first; an item that lays plates of its own inside it (the profiles) keeps its plate to its first line.
        let bottom = end.unwrap_or(y) - ROW_GAP;
        let plates: Vec<usize> = (first..items.len()).filter(|&i| matches!(items[i].visual, Visual::Plate(Fill::Row { .. }))).collect();
        if let [only] = plates[..]
            && items[only].rect.y == start
            && items[only].rect.y + items[only].rect.h < bottom
        {
            items[only].rect.h = bottom - start;
        }
        rows.push(Rect::new(slot.0, start, slot.1, end.unwrap_or(y) - start));
    }
    // Rows scrolled under the title are cut off by a line, not at no edge at all.
    if page.scroll > 0.0 {
        item(&mut items, Rect::new(x0, top - 1.0, content_width, 1.0), None, Visual::Plate(Fill::Rule), false);
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
        let view = View { lang: Lang::EnUs, settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
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
                // The snippets, and the test of the push.
                // The snippets, under a heading of their own.
                Section::Terminal => 3,
                Section::Profiles => 2,
                // No rules yet: the ready-made ones.
                Section::Triggers => 1,
                _ => 0,
            };
            // And a heading over each row that starts a section of the page.
            let headings = settings_ui::in_section(*section).filter(|row| settings_ui::heading_before(row.id).is_some()).count();
            let expected = if *section == Section::Keys { settings_ui::key_rows().count() } else { settings_ui::in_section(*section).count() + own + headings };
            assert_eq!(shown, expected, "{section:?}");
        }
    }

    #[test]
    fn a_heading_stands_over_the_row_that_starts_its_section_and_takes_no_presses() {
        let terminal = Section::ALL.iter().position(|s| *s == Section::Terminal).unwrap();
        let frame = frame(&Page { section: terminal, ..Page::default() }, &Settings::default());
        let at = frame.entries.iter().position(|e| matches!(e, Entry::Heading(_))).expect("the Terminal page has a heading");
        assert!(matches!(frame.entries[at + 1], Entry::Setting(row) if row.id == "notifications.toast"));
        let heading = frame.rows[at];
        assert!(frame.items.iter().filter(|item| item.part.is_some()).all(|item| !heading.contains(item.rect.x + 1.0, item.rect.y + 1.0) || item.rect.y >= heading.y + heading.h));
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
        for (width, height) in [(760.0, 480.0), (900.0, 650.0), (1400.0, 900.0)] {
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, width, height) };
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
                // A row of a pair is half the line: its control ends at the inset of its half.
                let right = row.x + row.w - INNER;
                let in_row = |item: &&Item| item.scrolls && item.rect.y >= row.y && item.rect.y + item.rect.h <= row.y + row.h && item.rect.x >= row.x && item.rect.x + item.rect.w <= row.x + row.w + 0.01;
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
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], area: Rect::new(0.0, 0.0, 1000.0, 700.0), update, scoop };
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
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], area: Rect::new(0.0, 0.0, 1000.0, 700.0), update: Check::Latest, scoop: true };
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
    fn each_shortcut_of_a_command_is_its_own_button_with_an_add_button_and_reset_all_shows_once_one_changed() {
        let keys = Section::ALL.iter().position(|s| *s == Section::Keys).unwrap();
        let page = Page { section: keys, ..Page::default() };
        let parts = |settings: &Settings| frame(&page, settings).items.iter().filter_map(|item| item.part).collect::<Vec<_>>();
        let mut settings = Settings::default();
        let all = parts(&settings);
        let paste = keymap().shortcuts(Command::EditPaste).len();
        assert!(paste > 1, "paste has more than one shortcut by default");
        assert_eq!(all.iter().filter(|p| matches!(p, Part::Chip(Command::EditPaste, _))).count(), paste);
        assert!(all.contains(&Part::AddKey(Command::TabClose)), "a command without a shortcut can be given one");
        assert!(all.contains(&Part::AddKey(Command::TabGoto(3))) && all.contains(&Part::AddKey(Command::EditSmartCopy)));
        assert!(!all.contains(&Part::ResetKeys), "nothing to reset");
        settings.keybindings.insert("tab.close".into(), vec!["Ctrl+Shift+Q".into()]);
        assert!(parts(&settings).contains(&Part::ResetKeys));
    }

    #[test]
    fn the_schemes_can_be_imported_from_the_page_and_only_ones_own_can_be_deleted_from_its_tile() {
        let appearance = Section::ALL.iter().position(|s| *s == Section::Appearance).unwrap();
        let mut settings = Settings::default();
        let mut own = themes::scheme("Nord").clone();
        own.name = "Mine".into();
        settings.schemes.push(own);
        let names: Vec<String> = themes::theme_names(&settings.schemes).map(str::to_owned).collect();
        let (mine, builtin) = (names.iter().position(|n| n == "Mine").unwrap(), names.iter().position(|n| n == "Nord").unwrap());
        let parts = |hover: Option<Part>| {
            let page = Page { section: appearance, hover, ..Page::default() };
            frame(&page, &settings).items.iter().filter_map(|item| item.part).collect::<Vec<_>>()
        };
        let idle = parts(None);
        assert!(idle.contains(&Part::ImportSchemes) && idle.contains(&Part::ImportItermColors));
        assert!(!idle.iter().any(|p| matches!(p, Part::DeleteScheme(_))), "the delete button waits for the pointer");
        assert!(parts(Some(Part::Tile(mine))).contains(&Part::DeleteScheme(mine)));
        assert!(!parts(Some(Part::Tile(builtin))).contains(&Part::DeleteScheme(builtin)), "a built-in scheme stays");
    }

    #[test]
    fn the_snippets_are_lines_of_fields_on_the_terminal_page_and_one_being_added_gets_a_line_of_its_own() {
        let terminal = Section::ALL.iter().position(|s| *s == Section::Terminal).unwrap();
        let mut settings = Settings::default();
        settings.snippets.push(nekominal_model::settings::Snippet { name: "build".into(), input: "cargo build\r".into() });
        let parts = |page: &Page| frame(page, &settings).items.iter().filter_map(|item| item.part).collect::<Vec<_>>();
        let page = Page { section: terminal, ..Page::default() };
        assert!(matches!(frame(&page, &settings).entries.last(), Some(Entry::Snippets)));
        let shown = parts(&page);
        for part in [Part::SnippetAdd, Part::SnippetName(0), Part::SnippetInput(0), Part::SnippetDelete(0)] {
            assert!(shown.contains(&part), "{part:?}");
        }
        assert!(!shown.contains(&Part::SnippetInput(1)));
        let adding = Page { section: terminal, editing: Some((SNIPPET_INPUT_ID, String::new())), env_row: 1, ..Page::default() };
        let shown = parts(&adding);
        assert!(shown.contains(&Part::SnippetInput(1)) && !shown.contains(&Part::SnippetDelete(1)), "the new line has nothing to delete yet");
    }

    #[test]
    fn the_saved_layouts_are_listed_under_the_profiles_with_a_save_button_and_their_own_buttons() {
        let profiles = Section::ALL.iter().position(|s| *s == Section::Profiles).unwrap();
        let settings = Settings::default();
        let page = Page { section: profiles, layouts: vec!["work".into(), "home".into()], ..Page::default() };
        let parts: Vec<Part> = frame(&page, &settings).items.iter().filter_map(|item| item.part).collect();
        assert!(parts.contains(&Part::LayoutSave));
        for i in 0..2 {
            assert!(parts.contains(&Part::LayoutName(i)) && parts.contains(&Part::LayoutOpen(i)) && parts.contains(&Part::LayoutDelete(i)));
        }
        let saving = Page { editing: Some((LAYOUT_NAME_ID, String::new())), env_row: 2, ..page };
        let parts: Vec<Part> = frame(&saving, &settings).items.iter().filter_map(|item| item.part).collect();
        assert!(parts.contains(&Part::LayoutName(2)) && !parts.contains(&Part::LayoutOpen(2)), "the name being given has nothing to open yet");
    }

    #[test]
    fn a_snippets_enter_is_written_as_backslash_r_in_its_field_and_back() {
        assert_eq!(snippet_shown("cargo build\r"), "cargo build\\r");
        assert_eq!(snippet_input("cargo build\\r"), "cargo build\r");
        assert_eq!(snippet_input(&snippet_shown("a\rb\r")), "a\rb\r");
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
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, found: &[], programs: &[None, None, None], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
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
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, found: &found, programs: &[None, None], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
            build(page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5)
        };
        let parts = |frame: &Frame| frame.items.iter().filter_map(|item| item.part).collect::<Vec<_>>();
        let list = build_for(&Page { section, ..Page::default() });
        assert!(matches!(list.entries[list.entries.len() - 2..], [Entry::NewProfile, Entry::Layouts]));
        assert!(parts(&list).contains(&Part::EditProfile(0)) && parts(&list).contains(&Part::EditProfile(1)) && parts(&list).contains(&Part::NewProfile));
        let edit = list.items.iter().find(|item| item.part == Some(Part::EditProfile(1))).unwrap();
        assert_eq!(list.part_at(0.0, edit.rect.x + 2.0, edit.rect.y + 2.0), Some(Part::EditProfile(1)), "the pencil wins over the row under it");
        let mine = build_for(&Page { section, profile: Some("mine".into()), ..Page::default() });
        assert_eq!(mine.entries.len(), 1 + Field::ALL.len());
        for field in Field::ALL {
            let wanted = match field {
                Field::Scheme => Part::ProfileScheme(None),
                Field::FontSize => Part::ProfileFont(true),
                Field::Env => Part::EnvAdd,
                _ => Part::ProfileField(field),
            };
            assert!(parts(&mine).contains(&wanted), "{field:?}");
        }
        assert!(parts(&mine).contains(&Part::Browse(Field::Program)) && parts(&mine).contains(&Part::Browse(Field::Cwd)));
        assert!(parts(&mine).contains(&Part::ProfileScheme(Some(0))), "the schemes are tiles to choose");
        // A variable is a line of its own: its name, its value and its delete button; a refused name says why under it.
        let mut settings = settings.clone();
        settings.profiles[0].options.env.insert("LANG".into(), "zh_TW".into());
        let profiles = nekominal_model::profiles::merge(&found, &settings.profiles);
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &profiles, found: &found, programs: &[None, None], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 0.0, 1000.0, 700.0) };
        let page = Page { section, profile: Some("mine".into()), editing: Some((profile_edit::ENV_KEY_ID, "A B".into())), error: Some((profile_edit::ENV_KEY_ID, Text::ProfileBadVariable)), ..Page::default() };
        let with_var = build(&page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        for part in [Part::EnvKey(0), Part::EnvValue(0), Part::EnvDelete(0)] {
            assert!(parts(&with_var).contains(&part), "{part:?}");
        }
        assert!(with_var.items.iter().any(|item| matches!(&item.visual, Visual::Label { tone: Tone::Error, text, .. } if text == Text::ProfileBadVariable.get(Lang::EnUs))));
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

    fn section_of(section: Section) -> usize {
        Section::ALL.iter().position(|s| *s == section).unwrap()
    }

    fn parts(frame: &Frame) -> Vec<Part> {
        frame.items.iter().filter_map(|item| item.part).collect()
    }

    fn texts(frame: &Frame) -> Vec<String> {
        frame.items.iter().filter_map(|item| match &item.visual { Visual::Label { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    fn at_entry(frame: &Frame, wanted: impl Fn(&Entry) -> bool) -> Rect {
        frame.rows[frame.entries.iter().position(wanted).expect("the entry is on the page")]
    }

    #[test]
    fn the_phone_notifications_are_under_their_heading_with_every_part_of_the_preview() {
        let terminal = Page { section: section_of(Section::Terminal), ..Page::default() };
        let frame = frame(&terminal, &Settings::default());
        let heading = frame.entries.iter().position(|e| matches!(e, Entry::Heading(Text::SettingsPushHeading))).expect("the heading");
        let ids: Vec<&str> = frame.entries[heading + 1..].iter().filter_map(|e| if let Entry::Setting(row) = e { Some(row.id) } else { None }).collect();
        assert_eq!(ids, ["notifications.push.url", "notifications.push.kind", "notifications.push.when", "notifications.push.token", "notifications.push.events"]);
        assert!(matches!(frame.entries[heading + 6], Entry::PushTest), "the test row after the events");
        let shown = parts(&frame);
        for part in [Part::Field("notifications.push.url"), Part::Option("notifications.push.kind", 1), Part::Option("notifications.push.when", 1), Part::Field("notifications.push.token"), Part::Option("notifications.push.events", 4), Part::PushTest] {
            assert!(shown.contains(&part), "{part:?}");
        }
        let words = texts(&frame);
        for text in [Text::NotePushUrl, Text::NotePushToken, Text::PushTestSend, Text::PushTest, Text::PushKindNtfy, Text::PushWhenAway, Text::BellSoundAlways, Text::GroupWaiting, Text::PushEventMessage] {
            assert!(words.iter().any(|w| w == text.get(Lang::EnUs)), "{text:?}");
        }
        // The format and the time to send share a line, a half each.
        let kind = at_entry(&frame, |e| matches!(e, Entry::Setting(row) if row.id == "notifications.push.kind"));
        let when = at_entry(&frame, |e| matches!(e, Entry::Setting(row) if row.id == "notifications.push.when"));
        assert_eq!(kind.y, when.y);
        assert!(kind.x + kind.w <= when.x && kind.w < frame.rows[0].w / 2.0 + 0.01);
        let token = at_entry(&frame, |e| matches!(e, Entry::Setting(row) if row.id == "notifications.push.token"));
        assert!(token.y >= kind.y + kind.h, "the next row is under the pair");
    }

    #[test]
    fn the_token_shows_masked_unless_it_is_being_edited() {
        assert_eq!(masked("tk_0123456789abcdefa91c"), "tk_\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}a91c");
        assert_eq!(masked("short"), "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}", "a short secret shows nothing of itself");
        assert_eq!(masked(""), "");
        let mut settings = Settings::default();
        settings.notifications.push.token = "tk_0123456789abcdefa91c".into();
        let terminal = Page { section: section_of(Section::Terminal), ..Page::default() };
        let words = texts(&frame(&terminal, &settings));
        assert!(words.iter().any(|w| *w == masked("tk_0123456789abcdefa91c")));
        assert!(!words.iter().any(|w| w.contains("0123456789")), "the token itself is not on the page");
        assert_eq!(value_text(row("notifications.push.token"), &settings, &[], Lang::EnUs), masked("tk_0123456789abcdefa91c"), "nor read out");
        let editing = Page { editing: Some(("notifications.push.token", "tk_0123456789abcdefa91c".into())), ..terminal };
        assert!(frame(&editing, &settings).items.iter().any(|item| matches!(&item.visual, Visual::Edit { text, .. } if text == "tk_0123456789abcdefa91c")), "edited, it is the text");
    }

    #[test]
    fn the_events_are_flags_each_shown_chosen_on_its_own() {
        let terminal = Page { section: section_of(Section::Terminal), ..Page::default() };
        let chosen = |settings: &Settings| -> Vec<bool> {
            let frame = frame(&terminal, settings);
            (0..5).map(|i| frame.items.iter().any(|item| item.part == Some(Part::Option("notifications.push.events", i)) && matches!(item.visual, Visual::Plate(Fill::Button { selected: true, .. })))).collect()
        };
        assert_eq!(chosen(&Settings::default()), [true, true, true, false, false], "waiting, failed and done by default");
        let mut settings = Settings::default();
        (row("notifications.push.events").set)(&mut settings, Value::Flags(0b10010));
        assert_eq!(chosen(&settings), [false, true, false, false, true]);
        assert_eq!(value_text(row("notifications.push.events"), &settings, &[], Lang::EnUs), "Failed, Program notifications");
    }

    #[test]
    fn a_refused_address_says_why_under_its_field() {
        let page = Page { section: section_of(Section::Terminal), editing: Some(("notifications.push.url", "http://ntfy.sh/x".into())), error: Some(("notifications.push.url", Text::PushBadUrl)), ..Page::default() };
        let frame = frame(&page, &Settings::default());
        let error = frame.items.iter().find(|item| matches!(&item.visual, Visual::Label { tone: Tone::Error, text, .. } if text == Text::PushBadUrl.get(Lang::EnUs))).expect("the reason");
        let row = at_entry(&frame, |e| matches!(e, Entry::Setting(row) if row.id == "notifications.push.url"));
        assert!(row.contains(error.rect.x + 1.0, error.rect.y + 1.0) && error.rect.y + error.rect.h <= row.y + row.h, "inside its row, which grew for it");
    }

    #[test]
    fn the_test_row_says_what_the_last_test_came_to() {
        use crate::push::Test;
        use std::time::Duration;
        assert_eq!(push_test_texts(None, Lang::EnUs), (None, None));
        let (note, status) = push_test_texts(Some(&(Duration::from_secs(5), Test::Sent)), Lang::ZhTw);
        assert_eq!(note.as_deref(), Some("上次傳送：剛才，手機已收到"));
        assert_eq!(status, Some(("\u{25CF} 已送達".to_owned(), Tone::Good)));
        let (note, status) = push_test_texts(Some(&(Duration::from_secs(150), Test::Failed("curl.exe exited with 6".into()))), Lang::EnUs);
        assert_eq!(note.as_deref(), Some("Last sent 2 min ago"));
        assert_eq!(status, Some(("curl.exe exited with 6".to_owned(), Tone::Error)));
        assert_eq!(push_test_texts(Some(&(Duration::ZERO, Test::Sending)), Lang::EnUs).1.map(|(text, _)| text), Some("Sending".to_owned()));
    }

    fn rules() -> Settings {
        let mut settings = Settings::default();
        settings.triggers = vec![
            triggers::preset(0, Lang::EnUs).unwrap(),
            Trigger { name: "Approve".into(), pattern: "Do you want to (proceed|make this edit)".into(), enabled: true, action: Action::Notify { text: "$TAB: $0".into() }, program: Some("claude.exe".into()), instant: true },
            Trigger { name: "npm".into(), pattern: r"Ok to proceed\? \(y\)".into(), enabled: false, action: Action::Send { text: "y\r".into(), allow_send: false }, program: None, instant: false },
        ];
        settings
    }

    #[test]
    fn the_triggers_page_is_a_plate_a_rule_with_its_name_pattern_tag_and_switch_and_the_presets() {
        let page = Page { section: section_of(Section::Triggers), ..Page::default() };
        let settings = rules();
        let frame = frame(&page, &settings);
        assert!(matches!(frame.entries[..], [Entry::Trigger(0), Entry::Trigger(1), Entry::Trigger(2), Entry::TriggerPresets]));
        let shown = parts(&frame);
        assert!(shown.contains(&Part::TriggerAdd));
        for i in 0..3 {
            assert!(shown.contains(&Part::TriggerOpen(i)) && shown.contains(&Part::TriggerSwitch(i)));
            assert!(!shown.contains(&Part::TriggerPattern(i)), "closed, a rule shows no form");
        }
        assert_eq!(shown.iter().filter(|p| matches!(p, Part::TriggerPreset(_))).count(), triggers::PRESETS.len());
        let words = texts(&frame);
        for word in ["Approve", "npm", "Send text y\u{23CE}", "Notification", "Highlight", Text::TriggersIntro.get(Lang::EnUs), Text::TriggerAdd.get(Lang::EnUs)] {
            assert!(words.iter().any(|w| w == word), "{word}");
        }
        let switches: Vec<bool> = frame.items.iter().filter_map(|item| match item.visual { Visual::Plate(Fill::Switch { on, .. }) => Some(on), _ => None }).collect();
        assert_eq!(switches, [true, true, false], "a send not allowed to send is off");
        let tags: Vec<Tag> = frame.items.iter().filter_map(|item| match item.visual { Visual::Plate(Fill::Tag(tag)) if tag != Tag::Code => Some(tag), _ => None }).collect();
        assert_eq!(tags, [Tag::Mark, Tag::Plain, Tag::Warn]);
    }

    #[test]
    fn an_open_rule_shows_its_form_in_place_with_the_sample_matched() {
        let settings = rules();
        let keymap = keymap();
        let sample = ["  Do you want to proceed?".to_owned(), "  \u{276F} 1. Yes  2. No".to_owned()];
        let page = Page { section: section_of(Section::Triggers), trigger: Some(1), ..Page::default() };
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &sample, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
        let open = build(&page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        let shown = parts(&open);
        for part in [Part::TriggerName(1), Part::TriggerPattern(1), Part::TriggerInstant(1), Part::TriggerProgram(1), Part::TriggerText(1)] {
            assert!(shown.contains(&part), "{part:?}");
        }
        assert_eq!(shown.iter().filter(|p| matches!(p, Part::TriggerAction(1, _))).count(), 5, "the five actions");
        assert!(!shown.contains(&Part::TriggerName(0)), "only the open rule has a form");
        let chosen = |part: Part| open.items.iter().any(|item| item.part == Some(part) && matches!(item.visual, Visual::Plate(Fill::Button { selected: true, .. })));
        assert!(chosen(Part::TriggerAction(1, 1)) && chosen(Part::TriggerInstant(1)), "notify and before the line ends");
        let words = texts(&open);
        for text in [Text::ProfileName, Text::TriggerPattern, Text::TriggerProgram, Text::TriggerProgramNote, Text::TriggerAction, Text::TriggerNotifyText] {
            assert!(words.iter().any(|w| w == text.get(Lang::EnUs)), "{text:?}");
        }
        let lines = open.items.iter().find_map(|item| match &item.visual { Visual::Sample { lines } => Some(lines.clone()), _ => None }).expect("the sample");
        assert_eq!(lines[0].1, [(2, 24)], "the match is marked");
        assert!(lines[1].1.is_empty());
        // The open rule grew; the rules after it moved down.
        let closed = frame(&Page { section: section_of(Section::Triggers), ..Page::default() }, &settings);
        assert!(open.rows[1].h > closed.rows[1].h && open.rows[2].y > closed.rows[2].y);
        // A highlight offers its colours instead of a text.
        let highlight = build(&Page { trigger: Some(0), ..page.clone() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        assert_eq!(parts(&highlight).iter().filter(|p| matches!(p, Part::TriggerColor(0, _))).count(), triggers::HIGHLIGHTS.len());
        assert!(!parts(&highlight).contains(&Part::TriggerText(0)));
    }

    #[test]
    fn a_pattern_that_is_not_a_regular_expression_says_so_under_its_field_and_matches_nothing() {
        let settings = rules();
        let keymap = keymap();
        let sample = ["  Do you want to proceed?".to_owned()];
        let page = Page { section: section_of(Section::Triggers), trigger: Some(1), env_row: 1, editing: Some((TRIGGER_PATTERN_ID, "Do you (want".into())), error: Some((TRIGGER_PATTERN_ID, Text::TriggerBadPattern)), ..Page::default() };
        let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &sample, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
        let frame = build(&page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
        assert!(frame.items.iter().any(|item| matches!(&item.visual, Visual::Label { tone: Tone::Error, text, .. } if text == Text::TriggerBadPattern.get(Lang::EnUs))));
        assert!(frame.items.iter().any(|item| matches!(&item.visual, Visual::Edit { text, .. } if text == "Do you (want")), "the field stays open with what was typed");
        let lines = frame.items.iter().find_map(|item| match &item.visual { Visual::Sample { lines } => Some(lines.clone()), _ => None }).unwrap();
        assert!(lines[0].1.is_empty(), "the sample follows what is typed, which matches nothing");
    }

    #[test]
    fn a_rule_being_added_is_a_form_of_its_own_after_the_rules() {
        let settings = rules();
        let page = Page { section: section_of(Section::Triggers), trigger: Some(3), env_row: 3, editing: Some((TRIGGER_PATTERN_ID, String::new())), ..Page::default() };
        let frame = frame(&page, &settings);
        assert!(matches!(frame.entries[..], [.., Entry::Trigger(3), Entry::TriggerPresets]));
        assert!(parts(&frame).contains(&Part::TriggerPattern(3)));
        assert_eq!(trigger_at(&settings, 3).pattern, "", "it is in the settings once its pattern is given");
    }

    /// Every page in a state that shows its rows that are buttons: rules, snippets, layouts, the editor of a profile.
    fn every_page(settings: &Settings, width: f32, sample: &[String], build_page: &mut dyn FnMut(&Page, &View)) {
        use nekominal_model::profiles::ProfileKind;
        let keymap = keymap();
        let profiles = [Profile { id: "mine".into(), name: "mine".into(), kind: ProfileKind::Other, icon: "terminal", program: "x.exe".into(), args: Vec::new(), cwd: None, custom: true, options: Default::default() }];
        let view = View { lang: Lang::EnUs, settings, keymap: &keymap, profiles: &profiles, found: &[], programs: &[None], claude: &CLAUDE, push_test: None, sample, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, width, 650.0) };
        let layouts = vec!["work".to_owned()];
        for section in 0..Section::ALL.len() {
            build_page(&Page { section, layouts: layouts.clone(), ..Page::default() }, &view);
        }
        build_page(&Page { section: section_of(Section::Profiles), profile: Some("mine".into()), ..Page::default() }, &view);
        for open in 0..settings.triggers.len() {
            build_page(&Page { section: section_of(Section::Triggers), trigger: Some(open), ..Page::default() }, &view);
        }
    }

    fn full_settings() -> Settings {
        let mut settings = rules();
        settings.triggers.push(Trigger { name: "ding".into(), pattern: "ding".into(), enabled: true, action: Action::Bell, program: None, instant: false });
        settings.snippets.push(nekominal_model::settings::Snippet { name: "build".into(), input: "cargo build\r".into() });
        settings.notifications.push.token = "tk_0123456789abcdefa91c".into();
        settings
    }

    #[test]
    fn enter_on_a_row_presses_a_button_that_is_on_it() {
        let settings = full_settings();
        let mut checked = 0;
        every_page(&settings, 1100.0, &[], &mut |page, view| {
            let frame = build(page, view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
            for (entry, row) in frame.entries.iter().zip(&frame.rows) {
                let Some(part) = enter_part(entry) else { continue };
                let on_row = frame.items.iter().any(|item| item.part == Some(part) && row.contains(item.rect.x + 1.0, item.rect.y + 1.0));
                assert!(on_row, "Enter on {entry:?} presses {part:?}, which is not on its row");
                checked += 1;
            }
        });
        assert!(checked >= 8, "the rows that are buttons were on the pages: {checked}");
    }

    #[test]
    fn at_any_width_every_part_of_the_new_rows_stays_inside_its_row() {
        let mut settings = full_settings();
        // A long name and pattern, to be cut rather than run into the tag.
        settings.triggers[2].name = "a rule with a name far longer than its plate has room for".into();
        settings.triggers[2].pattern = r"Ok to proceed\? \(y\) and a great deal more of a pattern than fits".into();
        let sample = ["  Do you want to proceed?".to_owned(), "error[E0308]: mismatched types".to_owned()];
        for width in [760.0, 900.0, 1100.0, 1600.0] {
            let check = |frame: &Frame, what: &str| {
                for item in frame.items.iter().filter(|item| item.scrolls && item.part.is_some()) {
                    let part = item.part.unwrap();
                    assert!(item.rect.w > 0.0 && item.rect.h > 0.0, "{what} at {width}: {part:?} has no room");
                    let inside = frame.rows.iter().any(|row| {
                        item.rect.x >= row.x - 0.01 && item.rect.x + item.rect.w <= row.x + row.w + 0.01 && item.rect.y >= row.y - 0.01 && item.rect.y + item.rect.h <= row.y + row.h + 0.01
                    });
                    assert!(inside, "{what} at {width}: {part:?} at {:?} is outside every row", item.rect);
                }
                for (entry, row) in frame.entries.iter().zip(&frame.rows) {
                    let Entry::Trigger(_) = entry else { continue };
                    // On the line of the name: the name and the pattern end before the tag, the tag before the switch.
                    let head = |item: &&Item| item.scrolls && item.rect.y >= row.y - 0.01 && item.rect.y + item.rect.h <= row.y + ROW + 0.01;
                    let tag = frame.items.iter().filter(head).find(|item| matches!(item.visual, Visual::Plate(Fill::Tag(tag)) if tag != Tag::Code)).expect("the tag");
                    let switch = frame.items.iter().filter(head).find(|item| matches!(item.visual, Visual::Plate(Fill::Switch { .. }))).expect("the switch");
                    assert!(tag.rect.x + tag.rect.w <= switch.rect.x + 0.01, "{what} at {width}: the tag runs into the switch");
                    for left in frame.items.iter().filter(head).filter(|item| matches!(item.visual, Visual::Label { bold: true, .. } | Visual::Plate(Fill::Tag(Tag::Code)))) {
                        assert!(left.rect.x + left.rect.w <= tag.rect.x + 0.01, "{what} at {width}: {:?} runs into the tag", left.visual);
                    }
                }
            };
            every_page(&settings, width, &sample, &mut |page, view| {
                let frame = build(page, view, &mut |text, size, _| text.chars().count() as f32 * size * 0.55);
                check(&frame, &format!("section {} rule {:?}", page.section, page.trigger));
            });
            // A refused pattern says why under its field, inside the rule's plate.
            let keymap = keymap();
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: Some((std::time::Duration::from_secs(90), crate::push::Test::Failed("curl.exe exited with 6".into()))), sample: &sample, update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, width, 650.0) };
            let page = Page { section: section_of(Section::Triggers), trigger: Some(1), env_row: 1, editing: Some((TRIGGER_PATTERN_ID, "(".into())), error: Some((TRIGGER_PATTERN_ID, Text::TriggerBadPattern)), ..Page::default() };
            let frame = build(&page, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.55);
            check(&frame, "refused pattern");
            let error = frame.items.iter().find(|item| matches!(&item.visual, Visual::Label { tone: Tone::Error, .. })).expect("the reason");
            assert!(frame.rows[1].contains(error.rect.x + 1.0, error.rect.y + 1.0) && error.rect.y + error.rect.h <= frame.rows[1].y + frame.rows[1].h, "the reason is inside the plate at {width}");
            let terminal = build(&Page { section: section_of(Section::Terminal), ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.55);
            check(&terminal, "push test that failed");
        }
    }

    #[test]
    fn a_note_sits_under_its_label_inside_the_row() {
        let frame = frame(&Page { section: section_of(Section::Terminal), ..Page::default() }, &Settings::default());
        for id in ["notifications.push.url", "notifications.push.token"] {
            let row = at_entry(&frame, |e| matches!(e, Entry::Setting(row) if row.id == id));
            let in_row = |item: &&Item| row.contains(item.rect.x + 1.0, item.rect.y + 1.0);
            let label = frame.items.iter().filter(in_row).find(|item| matches!(&item.visual, Visual::Label { text, .. } if text == settings_ui::find(id).unwrap().label(Lang::EnUs))).expect("the label");
            let note_text = settings_ui::note_of(id).unwrap().get(Lang::EnUs);
            let note = frame.items.iter().filter(in_row).find(|item| matches!(&item.visual, Visual::Label { text, size, .. } if text == note_text && *size < 13.0)).expect("the note, smaller");
            assert!(note.rect.y > label.rect.y && note.rect.x == label.rect.x, "{id}: under the label");
            assert!(note.rect.y + note.rect.h <= row.y + row.h + 0.01, "{id}: inside the row");
        }
    }

    #[test]
    fn a_secret_shows_its_ends_only_past_ten_characters() {
        assert_eq!(masked("0123456789"), "\u{2022}".repeat(10));
        assert_eq!(masked("0123456789a"), format!("012{}789a", "\u{2022}".repeat(4)));
        // Counted in characters, so a secret of wide characters is not cut inside one.
        assert_eq!(masked("一二三四五六七八九十甲"), format!("一二三{}八九十甲", "\u{2022}".repeat(4)));
    }

    #[test]
    fn no_flag_and_every_flag_are_shown_as_they_are() {
        let terminal = Page { section: section_of(Section::Terminal), ..Page::default() };
        let events = row("notifications.push.events");
        let Control::Flags(options) = events.control else { panic!("the events are flags") };
        for bits in [0, (1 << options.len()) - 1] {
            let mut settings = Settings::default();
            (events.set)(&mut settings, Value::Flags(bits));
            let frame = frame(&terminal, &settings);
            let chosen = (0..options.len()).filter(|i| frame.items.iter().any(|item| item.part == Some(Part::Option(events.id, *i)) && matches!(item.visual, Visual::Plate(Fill::Button { selected: true, .. })))).count();
            assert_eq!(chosen, bits.count_ones() as usize, "{bits:b}");
            let read = value_text(events, &settings, &[], Lang::EnUs);
            assert_eq!(read.is_empty(), bits == 0, "{bits:b}: {read}");
        }
    }

    #[test]
    fn each_action_shows_only_the_parts_it_needs() {
        let keymap = keymap();
        for (k, wanted) in [(0, "colours"), (1, "text"), (2, "nothing"), (3, "text"), (4, "nothing")] {
            let mut settings = rules();
            settings.triggers[0].action = Action::at(k).unwrap();
            let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, 1100.0, 650.0) };
            let frame = build(&Page { section: section_of(Section::Triggers), trigger: Some(0), ..Page::default() }, &view, &mut |text, size, _| text.chars().count() as f32 * size * 0.5);
            let shown = parts(&frame);
            let colours = shown.iter().filter(|p| matches!(p, Part::TriggerColor(0, _))).count();
            let text = shown.contains(&Part::TriggerText(0));
            assert_eq!((colours > 0, text), (wanted == "colours", wanted == "text"), "action {k}");
            for part in [Part::TriggerName(0), Part::TriggerPattern(0), Part::TriggerInstant(0), Part::TriggerProgram(0)] {
                assert!(shown.contains(&part), "action {k}: {part:?}");
            }
            assert!(!frame.items.iter().any(|item| matches!(item.visual, Visual::Sample { .. })), "no pane, no sample");
        }
        // The text of a send shows its Enter as \r, which a line of text cannot hold.
        let mut settings = rules();
        settings.triggers[2].enabled = true;
        let frame = frame(&Page { section: section_of(Section::Triggers), trigger: Some(2), ..Page::default() }, &settings);
        assert!(texts(&frame).iter().any(|t| t == "y\\r"));
    }

    #[test]
    fn every_heading_is_on_its_page_once_and_never_found_alone() {
        let settings = Settings::default();
        for section in 0..Section::ALL.len() {
            let frame = frame(&Page { section, ..Page::default() }, &settings);
            let headings: Vec<Text> = frame.entries.iter().filter_map(|e| if let Entry::Heading(text) = e { Some(*text) } else { None }).collect();
            let unique: std::collections::BTreeSet<&str> = headings.iter().map(|t| t.get(Lang::EnUs)).collect();
            assert_eq!(unique.len(), headings.len(), "{section}: a heading twice");
            for (i, entry) in frame.entries.iter().enumerate() {
                if let Entry::Heading(text) = entry {
                    assert_eq!(settings_ui::heading_section(*text), Section::ALL[section], "{text:?} is on the page it belongs to");
                    assert!(frame.entries.get(i + 1).is_some_and(|next| !matches!(next, Entry::Heading(_))), "{text:?} stands over a row");
                }
            }
        }
        for text in [Text::SettingsNotificationsHeading, Text::SettingsPushHeading, Text::SettingsSnippetsHeading] {
            let found = frame(&Page { query: text.get(Lang::EnUs).into(), ..Page::default() }, &settings);
            assert!(!found.entries.iter().any(|e| matches!(e, Entry::Heading(_))), "{text:?}: a search shows rows, not headings");
        }
    }

    #[test]
    fn a_search_finds_the_push_rows_and_never_a_heading_alone() {
        let settings = rules();
        let found = |query: &str| frame(&Page { query: query.into(), ..Page::default() }, &settings).entries;
        let ids = |entries: Vec<Entry>| entries.into_iter().filter_map(|e| if let Entry::Setting(row) = e { Some(row.id) } else { None }).collect::<Vec<_>>();
        assert!(ids(found("token")).contains(&"notifications.push.token"));
        assert!(ids(found("ntfy")).contains(&"notifications.push.kind"), "by an option");
        assert!(found("Approve").is_empty(), "the rules are not rows of the settings: their names find nothing");
        // A result comes under the name of its section, drawn as a heading over it, never as a result of its own.
        let frame = frame(&Page { query: "token".into(), ..Page::default() }, &settings);
        assert!(frame.entries.iter().all(|e| !matches!(e, Entry::Heading(_))));
        assert!(texts(&frame).iter().any(|t| t == Section::Terminal.text().get(Lang::EnUs)));
    }

    #[test]
    fn a_rule_without_a_name_is_shown_and_read_out_as_the_rule_to_add() {
        let settings = rules();
        let adding = trigger_at(&settings, settings.triggers.len());
        assert_eq!(trigger_title(&adding, Lang::EnUs), Text::TriggerAdd.get(Lang::EnUs), "a screen reader has a name to say");
        assert_eq!(trigger_title(&settings.triggers[1], Lang::EnUs), "Approve");
        let page = Page { section: section_of(Section::Triggers), trigger: Some(3), ..Page::default() };
        let frame = frame(&page, &settings);
        let plate = frame.rows[3];
        let shown = frame.items.iter().any(|item| matches!(&item.visual, Visual::Label { text, bold: true, .. } if *text == trigger_title(&adding, Lang::EnUs)) && plate.contains(item.rect.x + 1.0, item.rect.y + 1.0));
        assert!(shown, "its plate says the same");
    }

    #[test]
    fn the_keyboard_focus_skips_the_headings() {
        let frame = frame(&Page { section: section_of(Section::Terminal), ..Page::default() }, &Settings::default());
        let entries = &frame.entries;
        let headings: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Heading(_))).map(|(i, _)| i).collect();
        assert!(headings.len() >= 3, "the Terminal page has its headings");
        // Down from the first row to the last, then up again: never on a heading, every other row once.
        let mut seen = Vec::new();
        let mut focus = None;
        while let Some(next) = step_focus(entries, focus, true).filter(|next| Some(*next) != focus) {
            seen.push(next);
            focus = Some(next);
        }
        assert_eq!(seen.len(), entries.len() - headings.len());
        assert!(seen.iter().all(|i| !headings.contains(i)));
        assert_eq!(step_focus(entries, focus, true), focus, "the last row stays");
        let up = step_focus(entries, Some(headings[1] + 1), false);
        assert_eq!(up, Some(headings[1] - 1), "up over a heading to the row above it");
        assert_eq!(step_focus(entries, Some(seen[0]), false), Some(seen[0]), "the first row stays");
        assert_eq!(step_focus(&[], None, true), None, "a search that found nothing has no row");
        assert_eq!(step_focus(&[Entry::Heading(Text::SettingsSnippetsHeading)], None, false), None);
    }

    #[test]
    fn at_any_width_a_control_stays_in_its_row_and_clear_of_the_label() {
        let settings = Settings::default();
        let keymap = keymap();
        let measure = |text: &str, size: f32| text.chars().count() as f32 * size * 0.55;
        // From the smallest size the window of the settings had, column of the sections included.
        for width in [760.0, 900.0, 1100.0, 1600.0] {
            for section in 0..Section::ALL.len() {
                let page = Page { section, ..Page::default() };
                let view = View { lang: Lang::EnUs, settings: &settings, keymap: &keymap, profiles: &[], found: &[], programs: &[], claude: &CLAUDE, push_test: None, sample: &[], update: crate::update::Check::Idle, scoop: false, area: Rect::new(0.0, 48.0, width, 650.0) };
                let frame = build(&page, &view, &mut |text, size, _| measure(text, size));
                for (entry, row) in frame.entries.iter().zip(&frame.rows) {
                    let Entry::Setting(setting) = entry else { continue };
                    if matches!(setting.control, Control::Themes | Control::Profiles | Control::Layouts(_)) {
                        continue;
                    }
                    let in_row = |item: &&Item| item.scrolls && item.rect.y >= row.y - 0.01 && item.rect.y + item.rect.h <= row.y + row.h + 0.01 && item.rect.x >= row.x - 0.01 && item.rect.x <= row.x + row.w;
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
