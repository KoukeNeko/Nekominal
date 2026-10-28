//! Toolkit-free decisions of the shell: tab titles, tab drag targets, the pixel geometry of split panes and the
//! terminal setup (OSC 52 policy, when a paste asks first). Kept apart from the WinUI code so they can be tested
//! without a window.

use nekominal_model::layout::{Axis, Rect, Splitter};
use nekominal_model::settings::{
    Cursor, CursorStyle as CursorShapeSetting, Osc52Policy, PasteSettings, Settings, TITLE_SEPARATOR, TitlePart,
    TitleSlot,
};
use nekominal_term::CoreConfig;
use nekominal_term::agent_status::{AgentRecord, AgentState};
use nekominal_term::alacritty_terminal::term::Osc52;
use nekominal_term::alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
use nekominal_term::osc::is_control_or_bidi;

/// A pixel rectangle on the physical pixel grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Thickness of the space between two panes, in physical pixels (1 DIP, never less than one pixel).
pub fn divider_px(scale: f64) -> i32 {
    (scale.round() as i32).max(1)
}

/// Pixel rectangle of a pane whose share of the tab area is `rect`. Edges are rounded on the pixel grid, so
/// neighbours tile exactly; every edge shared with another pane gives up half of the divider.
pub fn pane_px_rect(rect: Rect, area_px: (i32, i32), divider: i32) -> PxRect {
    const EDGE: f64 = 1e-6;
    let edge = |fraction: f64, total: i32| (fraction * f64::from(total)).round() as i32;
    let (left, right) = (edge(rect.x, area_px.0), edge(rect.x + rect.w, area_px.0));
    let (top, bottom) = (edge(rect.y, area_px.1), edge(rect.y + rect.h, area_px.1));
    let before = divider / 2;
    let after = divider - before;
    let x0 = left + if rect.x > EDGE { after } else { 0 };
    let x1 = right - if rect.x + rect.w < 1.0 - EDGE { before } else { 0 };
    let y0 = top + if rect.y > EDGE { after } else { 0 };
    let y1 = bottom - if rect.y + rect.h < 1.0 - EDGE { before } else { 0 };
    PxRect { x: x0, y: y0, w: (x1 - x0).max(1), h: (y1 - y0).max(1) }
}

/// The divider between a split's two children: the gap `pane_px_rect` leaves, over the extent of the split.
pub fn splitter_px_rect(splitter: &Splitter, area_px: (i32, i32), divider: i32) -> PxRect {
    let edge = |fraction: f64, total: i32| (fraction * f64::from(total)).round() as i32;
    let before = divider / 2;
    let (x0, x1) = (edge(splitter.rect.x, area_px.0), edge(splitter.rect.x + splitter.rect.w, area_px.0));
    let (y0, y1) = (edge(splitter.rect.y, area_px.1), edge(splitter.rect.y + splitter.rect.h, area_px.1));
    match splitter.axis {
        Axis::Row => {
            let at = edge(splitter.rect.x + splitter.rect.w * splitter.ratio, area_px.0) - before;
            PxRect { x: at, y: y0, w: divider, h: y1 - y0 }
        }
        Axis::Column => {
            let at = edge(splitter.rect.y + splitter.rect.h * splitter.ratio, area_px.1) - before;
            PxRect { x: x0, y: at, w: x1 - x0, h: divider }
        }
    }
}

/// The ratio a split takes when its divider is dragged to `position` (a fraction of the whole tab area along
/// the split's axis). The model clamps it; this only converts the coordinate system.
pub fn ratio_at(splitter: &Splitter, position: f64) -> f64 {
    match splitter.axis {
        Axis::Row => (position - splitter.rect.x) / splitter.rect.w,
        Axis::Column => (position - splitter.rect.y) / splitter.rect.h,
    }
}

/// Index a dragged tab would take. `widths` are the chips in their current order (without spacing),
/// `gap` the spacing between them, `from` the dragged chip and `center` where its middle currently is,
/// measured from the left edge of the strip.
pub fn reorder_target(widths: &[f64], gap: f64, from: usize, center: f64) -> usize {
    let mut left = 0.0;
    let centers: Vec<f64> = widths
        .iter()
        .map(|width| {
            let middle = left + width / 2.0;
            left += width + gap;
            middle
        })
        .collect();
    let mut target = from;
    for (index, middle) in centers.iter().enumerate().skip(from + 1) {
        if center > *middle {
            target = index;
        }
    }
    for (index, middle) in centers.iter().enumerate().take(from).rev() {
        if center < *middle {
            target = index;
        }
    }
    target
}

/// What a tab shows about its panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabState {
    #[default]
    Idle,
    /// An agent works (OSC 7501), or a program reports progress (OSC 9;4).
    Working,
    /// An agent waits for an answer.
    Blocked,
    Done,
    Error,
    /// A bell or a notification rang out of sight.
    Attention,
}

/// The groups of the sidebar when it lists the tabs by state, in their order: what waits for the user (an agent's
/// question, a failure, a bell or a notification out of sight), what works, what is done (an agent that finished and
/// waits for the next message), and the rest.
pub const SIDEBAR_GROUPS: usize = 4;

/// The group of [`SIDEBAR_GROUPS`] a tab in this state goes in.
pub fn sidebar_group(state: TabState) -> usize {
    match state {
        TabState::Blocked | TabState::Error | TabState::Attention => 0,
        TabState::Working => 1,
        TabState::Done => 2,
        TabState::Idle => 3,
    }
}

/// A reason for a pane to want attention that no agent record gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    Bell,
    Notification,
    /// The program ended with a non-zero exit code.
    Exit,
}

/// Progress a program reports with OSC 9;4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reported {
    /// Busy, no amount known.
    Indeterminate,
    Percent(u8),
}

/// Everything one pane tells its tab right now.
#[derive(Clone, Debug, Default)]
pub struct PaneSignal {
    /// The most urgent OSC 7501 record, read live from the emulator.
    pub agent: Option<AgentRecord>,
    /// OSC 9;4 progress, for a program that reports only that.
    pub progress: Option<Reported>,
    pub attention: Option<Attention>,
    /// What the notification said.
    pub note: Option<String>,
}

/// What a tab shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabStatus {
    pub state: TabState,
    /// Percent, for a working tab that reports it.
    pub progress: Option<u8>,
    /// Why the tab wants attention, for the words of its tooltip.
    pub attention: Option<Attention>,
    /// What the agent or the notification said, already cleaned of control characters.
    pub message: Option<String>,
}

/// A bell or a notification ranks above nothing and below every agent state.
const ATTENTION_RANK: u8 = 1;

/// The status of a tab: that of its most urgent pane. Agent states rank by `AgentState::urgency`, the one order of
/// the application, doubled so a bell fits between idle and done; a program that exited with an error ranks as
/// an agent error (also over a progress report it left behind), a progress report as working. The first of equally
/// urgent panes wins.
pub fn tab_status(panes: &[PaneSignal]) -> TabStatus {
    let rank = |state: AgentState| state.urgency() * 2;
    let mut best: Option<(u8, TabStatus)> = None;
    for pane in panes {
        let candidate = if let Some(record) = &pane.agent {
            let state = match record.state {
                AgentState::Blocked => TabState::Blocked,
                AgentState::Error => TabState::Error,
                AgentState::Working => TabState::Working,
                AgentState::Done => TabState::Done,
                AgentState::Idle => continue,
            };
            let message = record.message.clone().or_else(|| record.title.clone());
            (rank(record.state), TabStatus { state, progress: record.progress, attention: None, message })
        } else {
            match (pane.attention, pane.progress) {
                (Some(Attention::Exit), _) => (
                    rank(AgentState::Error),
                    TabStatus { state: TabState::Error, progress: None, attention: pane.attention, message: None },
                ),
                (_, Some(reported)) => {
                    let progress = match reported {
                        Reported::Percent(percent) => Some(percent),
                        Reported::Indeterminate => None,
                    };
                    (
                        rank(AgentState::Working),
                        TabStatus { state: TabState::Working, progress, attention: None, message: None },
                    )
                }
                (Some(kind), None) => (
                    ATTENTION_RANK,
                    TabStatus {
                        state: TabState::Attention,
                        progress: None,
                        attention: Some(kind),
                        message: pane.note.clone(),
                    },
                ),
                (None, None) => continue,
            }
        };
        if best.as_ref().is_none_or(|(best_rank, _)| candidate.0 > *best_rank) {
            best = Some(candidate);
        }
    }
    best.map(|(_, status)| status).unwrap_or_default()
}

/// How much a state needs the user, higher first. It is `tab_status`'s order, which is `AgentState::urgency` doubled
/// so that a bell or a notification fits between idle and done; the list of tabs sorts by it.
pub fn state_rank(state: TabState) -> u8 {
    let rank = |state: AgentState| state.urgency() * 2;
    match state {
        TabState::Blocked => rank(AgentState::Blocked),
        TabState::Error => rank(AgentState::Error),
        TabState::Working => rank(AgentState::Working),
        TabState::Done => rank(AgentState::Done),
        TabState::Attention => ATTENTION_RANK,
        TabState::Idle => rank(AgentState::Idle),
    }
}

/// A tab is blocked or failed: the states the count on the position plate is of, and the ones a window-level alert is
/// raised for besides a notification.
pub fn is_urgent(state: TabState) -> bool {
    matches!(state, TabState::Blocked | TabState::Error)
}

/// A tab as the count in the window title sees it.
#[derive(Clone, Copy, Debug)]
pub struct TabAttention {
    /// A pane of the tab waits for the user.
    pub waiting: bool,
}

/// How many tabs wait for the user now, seen or not: the count in the title of a window that is not active.
pub fn waiting_count(tabs: &[TabAttention]) -> usize {
    tabs.iter().filter(|tab| tab.waiting).count()
}

/// The mark before a count of tabs that need a response, on the position plate and in the window title. Text on
/// purpose: nothing in the interface is a circle or a badge.
pub const ATTENTION_MARK: char = '!';

/// `!2`; nothing for none.
pub fn attention_text(count: usize) -> Option<String> {
    (count > 0).then(|| format!("{ATTENTION_MARK}{count}"))
}

/// The title of the window. While it is not active it starts with the count of the tabs that wait for the user, which
/// the taskbar button and Alt+Tab show.
pub fn window_title(app: &str, waiting: usize, window_active: bool) -> String {
    match attention_text(waiting) {
        Some(count) if !window_active => format!("{count} {app}"),
        _ => app.to_owned(),
    }
}

/// The tab that "next tab needing a response" goes to: the first after `selected`, in strip order and wrapping, that
/// waits for the user. `None` when no other tab does.
pub fn next_waiting(waiting: &[bool], selected: usize) -> Option<usize> {
    let count = waiting.len();
    (1..count).map(|step| (selected + step) % count).find(|index| waiting[*index])
}

/// No chip is narrower than this, in DIP.
pub const CHIP_MIN_WIDTH: f64 = 48.0;
/// No chip is wider than this.
pub const CHIP_MAX_WIDTH: f64 = 216.0;

/// The commands of a PowerShell history file (PSReadLine's `ConsoleHost_history.txt`), the latest first, each once, at
/// most `limit` of them. The lines that go on a command (it ends with a backtick) are joined to it.
pub fn recent_commands(history: &str, limit: usize) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    let mut pending = String::new();
    for line in history.lines() {
        if let Some(head) = line.strip_suffix('`') {
            pending.push_str(head);
            pending.push('\n');
            continue;
        }
        pending.push_str(line);
        let command = std::mem::take(&mut pending).trim().to_owned();
        if !command.is_empty() {
            commands.push(command);
        }
    }
    let mut seen = std::collections::HashSet::new();
    commands.into_iter().rev().filter(|command| seen.insert(command.clone())).take(limit).collect()
}

/// How much of the height of the screen the drop-down window takes.
pub const QUAKE_SHARE: f32 = 0.45;

/// The colours a tab can be given, with their names: clear on the dark bar and on a light one alike.
pub const TAB_COLORS: [(nekominal_model::strings::Text, (u8, u8, u8)); 8] = {
    use nekominal_model::strings::Text;
    [
        (Text::ColorRed, (232, 93, 93)),
        (Text::ColorOrange, (240, 140, 60)),
        (Text::ColorYellow, (228, 192, 64)),
        (Text::ColorGreen, (110, 190, 100)),
        (Text::ColorTeal, (60, 180, 180)),
        (Text::ColorBlue, (90, 140, 240)),
        (Text::ColorPurple, (160, 110, 230)),
        (Text::ColorPink, (230, 110, 170)),
    ]
};

/// The file name "export the text" offers for a tab called `label`: the label without what Windows forbids in a name,
/// "terminal" when nothing is left, with `.txt`.
pub fn export_file_name(label: &str) -> String {
    let clean: String = label.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').trim();
    format!("{}.txt", if clean.is_empty() { "terminal" } else { clean })
}

/// The text as Windows' own editors want it in a file: lines ended by CR LF.
pub fn with_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The tabs "close the other tabs" closes when tab `keep` of `count` stays: every other one, the last first, so that
/// closing one does not move the places of those still to close.
pub fn tabs_other_than(count: usize, keep: usize) -> Vec<usize> {
    (0..count).rev().filter(|i| *i != keep).collect()
}

/// The tabs "close the tabs to the right" closes: those after tab `from`, the last first.
pub fn tabs_right_of(count: usize, from: usize) -> Vec<usize> {
    (from.saturating_add(1)..count).rev().collect()
}

/// The width of every chip of the strip, for `count` tabs in a strip `available` DIP wide with `gap` between chips: an
/// equal share of the strip, kept between [`CHIP_MIN_WIDTH`] and [`CHIP_MAX_WIDTH`]. Only these two numbers decide it.
/// Which tab is selected and what the titles say do not, so nothing in the strip moves when a tab is selected or a
/// program retitles one. Once the floor is reached the chips together are wider than the strip, and it scrolls.
pub fn tab_width(count: usize, available: f64, gap: f64) -> f64 {
    if count == 0 {
        return CHIP_MAX_WIDTH;
    }
    ((available - gap * (count - 1) as f64) / count as f64).clamp(CHIP_MIN_WIDTH, CHIP_MAX_WIDTH)
}

/// The glyph (of the icon font) a tab shows for the program it runs, by its file name. The glyphs are those of the
/// icon font; a program with none of its own gets the terminal's, as a shell does.
pub fn job_icon(file_name: &str) -> char {
    let lower = file_name.to_lowercase();
    let name = ["exe", "cmd", "bat", "com"]
        .iter()
        .find_map(|extension| lower.strip_suffix(extension).and_then(|rest| rest.strip_suffix('.')))
        .unwrap_or(lower.as_str());
    match name {
        "vim" | "vi" | "nvim" | "gvim" | "nvim-qt" | "nano" | "micro" | "hx" | "helix" | "kak" | "emacs" | "code" | "notepad" => '\u{E70F}',
        "ssh" | "mosh" | "telnet" | "scp" | "sftp" | "curl" | "wget" | "ping" | "tracert" | "nc" => '\u{E774}',
        "less" | "more" | "man" | "bat" | "glow" | "cat" | "type" => '\u{E8A5}',
        "git" | "lazygit" | "gh" | "tig" | "gitui" => '\u{E81C}',
        "yazi" | "ranger" | "nnn" | "mc" | "lf" | "explorer" => '\u{E838}',
        "node" | "npm" | "npx" | "pnpm" | "yarn" | "bun" | "deno" | "pip" | "uv" | "cargo" | "rustc" | "go" | "dotnet" | "java" | "ruby"
        | "php" | "make" | "cmake" | "claude" | "codex" | "gemini" | "aider" | "opencode" => '\u{E943}',
        _ if name.starts_with("python") || name == "py" => '\u{E943}',
        _ => '\u{E756}',
    }
}

/// What a row of a list shows before its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A small square of a colour with a short label in it, the way a shell is known by its logo.
    Badge { label: &'static str, color: (u8, u8, u8) },
    /// A glyph of the icon font.
    Glyph(char),
}

/// The mark of a profile, by the icon key of the profile: one of `nekominal_pty::profiles::ICON_KEYS` or the generic one.
pub fn profile_mark(icon: &str) -> Mark {
    match icon {
        "pwsh" => Mark::Badge { label: "PS", color: (0x3A, 0x78, 0xD4) },
        "windows-powershell" => Mark::Badge { label: "PS", color: (0x1B, 0x3F, 0x8B) },
        "cmd" => Mark::Badge { label: ">_", color: (0x4A, 0x4A, 0x4A) },
        "git-bash" => Mark::Badge { label: "$", color: (0xD0, 0x4A, 0x28) },
        "wsl" => Mark::Badge { label: "L", color: (0x3F, 0x8F, 0x5A) },
        "visual-studio" => Mark::Badge { label: "VS", color: (0x68, 0x21, 0x7A) },
        "ssh" => Mark::Glyph('\u{E774}'),
        _ => Mark::Glyph('\u{E756}'),
    }
}

/// The last segment of a working directory as a tab shows it: `~` for the home folder, the drive (`C:`) for the
/// root of a drive, `/` for the root of a POSIX path (a WSL shell reports those), otherwise the folder name, cleaned
/// like any title (a shell reports whatever the path holds).
pub fn folder_label(path: &str, home: Option<&str>) -> String {
    let normal = |text: &str| text.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    let trimmed = path.trim_end_matches(['\\', '/']);
    if home.is_some_and(|home| !normal(home).is_empty() && normal(home) == normal(trimmed)) {
        return "~".to_owned();
    }
    if trimmed.is_empty() {
        return if path.is_empty() { String::new() } else { "/".to_owned() };
    }
    clean_title(trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed))
}

/// The most characters a title shows. A program may set a title of megabytes, and every chip is measured on every
/// layout of the strip.
pub const MAX_TITLE_CHARS: usize = 120;

/// The one cleaning of every title a tab shows, whatever it came from (a program, an agent, the user, a folder name):
/// no control, directional or noncharacter code points (a title is never markup and never reorders what is around
/// it), no blanks at either end, at most [`MAX_TITLE_CHARS`] characters, cut between two characters.
pub fn clean_title(raw: &str) -> String {
    clean_text(raw, MAX_TITLE_CHARS)
}

/// [`clean_title`] with a limit of its own.
pub fn clean_text(raw: &str, max: usize) -> String {
    let kept: String =
        raw.chars().filter(|c| !is_control_or_bidi(*c)).skip_while(|c| c.is_whitespace()).take(max).collect();
    kept.trim_end().to_owned()
}

/// The last segment of a working directory and the one before it as `parent/last`, for the tabs whose last
/// segments are the same. `None` where there is no parent worth naming: the home folder, a drive root, `/`.
pub fn folder_label_with_parent(path: &str, home: Option<&str>) -> Option<String> {
    let last = folder_label(path, home);
    if last == "~" || last == "/" || last.ends_with(':') || last.is_empty() {
        return None;
    }
    let trimmed = path.trim_end_matches(['\\', '/']);
    let parent_path = &trimmed[..trimmed.rfind(['\\', '/']).unwrap_or(0)];
    let parent = folder_label(parent_path, home);
    // A parent that is a root says nothing a folder name does not; a drive still tells where the folder is.
    (!parent.is_empty() && parent != "/").then(|| clean_title(&format!("{parent}/{last}")))
}

/// Where the default title of a tab comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleSource {
    /// The name the user gave the tab.
    User,
    /// Made from the parts of the title format; `folder` is whether the working directory is one of them, so that
    /// tabs with the same title can be told apart by the folder above it.
    Parts { folder: bool },
    /// Nothing the format asks for had anything to say.
    Untitled,
}

/// What the title of a tab is made from: the focused pane of the tab, its profile and the user.
#[derive(Clone, Debug, Default)]
pub struct TitleParts {
    /// The name the user gave the tab; it wins over everything.
    pub user: Option<String>,
    /// The title the program set (OSC 0 or 2).
    pub program: String,
    /// What an agent reported (OSC 7501): its title and its app id.
    pub agent_title: Option<String>,
    pub agent_app: Option<String>,
    pub cwd: Option<String>,
    pub profile: String,
    /// The file name of the pane's program without `.exe`: `pwsh`.
    pub shell: String,
    pub home: Option<String>,
    /// The title of last resort.
    pub untitled: String,
}

impl TitleParts {
    /// A program title says nothing the tab does not know already when it is the name of the profile, the id of
    /// the agent app or the name of the shell itself (a shell titles itself with its own path until its first
    /// prompt, and PowerShell never retitles itself after it).
    fn is_generic(&self, title: &str) -> bool {
        let same = |other: &str| !other.is_empty() && title.eq_ignore_ascii_case(other);
        title.is_empty() || same(&self.profile) || same(&self.shell) || self.agent_app.as_deref().is_some_and(same)
    }
}

/// What a part of the title says about a tab: nothing (an empty text) when it has nothing to say.
fn part_text(part: TitlePart, parts: &TitleParts, with_parent: bool) -> String {
    match part {
        // What an agent called the session, else what the terminal title says, unless it says nothing new.
        TitlePart::Title => parts
            .agent_title
            .iter()
            .map(String::as_str)
            .chain([parts.program.as_str()])
            .map(|title| tab_title(&clean_title(title), ""))
            .find(|title| !parts.is_generic(title))
            .unwrap_or_default(),
        TitlePart::Folder => {
            let Some(cwd) = parts.cwd.as_deref().filter(|cwd| !cwd.is_empty()) else { return String::new() };
            let home = parts.home.as_deref();
            let with_parent = if with_parent { folder_label_with_parent(cwd, home) } else { None };
            with_parent.unwrap_or_else(|| folder_label(cwd, home))
        }
        TitlePart::Profile => clean_title(&parts.profile),
        TitlePart::Shell => clean_title(&parts.shell),
        TitlePart::Agent => parts.agent_app.as_deref().map(clean_title).unwrap_or_default(),
    }
}

/// The title of a tab: the name the user gave it; else what `format` makes of it: each slot says the first of its parts
/// that has something to say, the slots that say nothing are left out and the rest are joined by [`TITLE_SEPARATOR`];
/// else `untitled`. `with_parent` adds the folder above the working directory, for tabs whose titles collide.
pub fn default_title(parts: &TitleParts, format: &[TitleSlot], with_parent: bool) -> (String, TitleSource) {
    if let Some(user) = parts.user.as_deref().map(clean_title).filter(|name| !name.is_empty()) {
        return (user, TitleSource::User);
    }
    let mut said = Vec::new();
    let mut folder = false;
    for slot in format {
        let found = slot.0.iter().find_map(|part| {
            let text = part_text(*part, parts, with_parent);
            (!text.is_empty()).then_some((*part, text))
        });
        if let Some((part, text)) = found {
            folder |= part == TitlePart::Folder;
            said.push(text);
        }
    }
    let title = clean_title(&said.join(TITLE_SEPARATOR));
    if title.is_empty() {
        (parts.untitled.clone(), TitleSource::Untitled)
    } else {
        (title, TitleSource::Parts { folder })
    }
}

/// The text a tab shows for the title a program set. Shells report the path of their own executable until
/// the first prompt (`C:\Program Files\PowerShell\7\pwsh.exe`): that becomes the program name. An empty
/// title falls back to `fallback`.
pub fn tab_title(raw: &str, fallback: &str) -> String {
    let title = raw.trim();
    let program = if let Some(quoted) = title.strip_prefix('"') {
        quoted.split('"').next().unwrap_or_default()
    } else {
        let lower = title.to_ascii_lowercase();
        lower.find(".exe").map_or(title, |end| &title[..end + 4])
    };
    let looks_like_executable = program.to_ascii_lowercase().ends_with(".exe")
        && (program.contains('\\') || program.contains('/') || program == title);
    let name = if looks_like_executable {
        let file = program.rsplit(['\\', '/']).next().unwrap_or(program);
        file.get(..file.len() - 4).unwrap_or(file)
    } else {
        title
    };
    if name.is_empty() { fallback.to_owned() } else { name.to_owned() }
}

/// The `file:///` URI of a Windows path, with what a URI cannot hold as it is escaped (a space, `%`, `#`, `?`).
pub use nekominal_term::links::file_uri;

/// Why a paste asks first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteWarning {
    /// The text has this many lines: a shell runs every one of them.
    Lines(usize),
    /// The text is this big, in bytes.
    Bytes(usize),
}

/// Whether pasting `text` asks first. Several lines ask unless the program turned on bracketed paste (it then
/// takes the paste as data, not as typing); a size over `warn_large_bytes` asks always. A limit of 0 turns the size
/// check off.
pub fn paste_warning(text: &str, bracketed: bool, settings: &PasteSettings) -> Option<PasteWarning> {
    // The same line breaks `prepare_paste` turns into Enter: CR LF, LF and CR each count once.
    let breaks = text.chars().filter(|c| *c == '\n').count()
        + text.char_indices().filter(|(at, c)| *c == '\r' && text.as_bytes().get(at + 1) != Some(&b'\n')).count();
    if settings.warn_multiline && !bracketed && breaks > 0 {
        return Some(PasteWarning::Lines(breaks + 1));
    }
    (settings.warn_large_bytes > 0 && text.len() as u64 > settings.warn_large_bytes)
        .then_some(PasteWarning::Bytes(text.len()))
}

/// A size for a dialog title: `812 B`, `5.3 KiB`, `12 MiB`.
pub fn size_label(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let size = bytes as f64;
    let (value, unit) = if size < KIB {
        return format!("{bytes} B");
    } else if size < KIB * KIB {
        (size / KIB, "KiB")
    } else {
        (size / (KIB * KIB), "MiB")
    };
    if value < 10.0 { format!("{} {unit}", (value * 10.0).round() / 10.0) } else { format!("{value:.0} {unit}") }
}

/// What the emulator lets OSC 52 do under a policy of the settings.
pub fn osc52_for(policy: Osc52Policy) -> Osc52 {
    match policy {
        Osc52Policy::Off => Osc52::Disabled,
        Osc52Policy::Write => Osc52::OnlyCopy,
        Osc52Policy::ReadWrite => Osc52::CopyPaste,
    }
}

/// The cursor of a program that chose none.
pub fn cursor_style_for(cursor: &Cursor) -> CursorStyle {
    let shape = match cursor.style {
        CursorShapeSetting::Block => CursorShape::Block,
        CursorShapeSetting::Bar => CursorShape::Beam,
        CursorShapeSetting::Underline => CursorShape::Underline,
    };
    CursorStyle { shape, blinking: cursor.blink }
}

/// The emulator setup of a new pane.
pub fn core_config(settings: &Settings) -> CoreConfig {
    CoreConfig {
        osc52: osc52_for(settings.osc52),
        cursor_style: cursor_style_for(&settings.cursor),
        scrollback_lines: settings.scrollback as usize,
        scrollback_bytes: nekominal_term::scrollback_bytes_for(settings.scrollback as usize),
        kitty_keyboard: settings.keyboard.kitty_protocol,
        sticky_header: settings.sticky_header,
        triggers: crate::triggers::rules(&settings.triggers),
        // Says Sixel, so programs that draw pictures (chafa, timg, yazi) use it.
        da1_reply: Some(nekominal_term::DA1_WITH_SIXEL.to_owned()),
        ..CoreConfig::default()
    }
}

// -------------------------------------------------------------------------------------------------
// The settings drawer
// -------------------------------------------------------------------------------------------------

/// What a change of the settings puts in effect at once. The rest is read when it is used (a paste, a close) or when a
/// new pane starts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// The colour scheme or the backdrop: the chrome and the terminals are painted again.
    pub glass: bool,
    /// The size, the family, the line height or the ligatures of the font.
    pub font: bool,
    pub osc52: bool,
    /// The shape of the cursor a program that chose none gets, and how many lines of scrollback a pane keeps.
    pub cursor: bool,
    pub scrollback: bool,
    pub sticky_header: bool,
    /// How the title of a tab is made.
    pub titles: bool,
}

/// What `now` changes of the screen compared with `before`. The opacity is not part of it: it has its own way.
pub fn effects(before: &Settings, now: &Settings) -> Effects {
    Effects {
        glass: before.theme != now.theme || before.appearance.backdrop != now.appearance.backdrop,
        font: before.font.size != now.font.size
            || before.font.family != now.font.family
            || before.font.line_height != now.font.line_height
            || before.font.ligatures != now.font.ligatures,
        osc52: before.osc52 != now.osc52,
        cursor: before.cursor != now.cursor,
        scrollback: before.scrollback != now.scrollback,
        sticky_header: before.sticky_header != now.sticky_header,
        titles: before.tabs != now.tabs,
    }
}

#[cfg(test)]
mod tests {
    use nekominal_model::layout::{Axis, Node, UNIT};
    use nekominal_model::settings::TabTitle;

    #[test]
    fn a_finished_agent_has_a_group_of_its_own_and_a_bell_goes_with_what_waits() {
        assert_eq!(sidebar_group(TabState::Done), 2, "not with the idle shells");
        assert_eq!(sidebar_group(TabState::Attention), sidebar_group(TabState::Blocked));
        assert_eq!(sidebar_group(TabState::Error), 0);
        assert_eq!(sidebar_group(TabState::Working), 1);
        assert_eq!(sidebar_group(TabState::Idle), SIDEBAR_GROUPS - 1);
    }

    #[test]
    fn a_path_becomes_a_file_uri_the_link_checks_take() {
        assert_eq!(file_uri(r"C:\My Files\a#1.txt"), "file:///C:/My%20Files/a%231.txt");
        let dir = std::env::temp_dir().join("nekominal hint test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let opened = nekominal_term::links::openable(&file_uri(&file.to_string_lossy()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(opened.is_some_and(|target| target.ends_with("notes.txt")), "a file that is there opens");
    }

    use super::*;

    #[test]
    fn closing_the_others_or_those_to_the_right_closes_the_last_first_and_keeps_the_one_asked_for() {
        assert_eq!(tabs_other_than(4, 1), [3, 2, 0]);
        assert_eq!(tabs_other_than(1, 0), Vec::<usize>::new(), "a lone tab has no others");
        assert_eq!(tabs_right_of(4, 1), [3, 2]);
        assert_eq!(tabs_right_of(4, 3), Vec::<usize>::new(), "nothing is right of the last tab");
        // Closed in that order, each index is still the tab it named when the list was made.
        let mut tabs = vec!["a", "b", "c", "d"];
        for i in tabs_other_than(tabs.len(), 2) {
            tabs.remove(i);
        }
        assert_eq!(tabs, ["c"]);
    }

    #[test]
    fn an_exported_file_is_named_after_its_tab_without_what_windows_forbids() {
        assert_eq!(export_file_name("Nekominal"), "Nekominal.txt");
        assert_eq!(export_file_name(r#"C:\src: "build" <2>"#), "C__src_ _build_ _2_.txt");
        assert_eq!(export_file_name("  ..  "), "terminal.txt", "nothing usable left");
        assert_eq!(export_file_name("設定"), "設定.txt");
        assert_eq!(with_crlf("a\nb\r\nc\n"), "a\r\nb\r\nc\r\n", "every line once, never CR CR LF");
    }

    #[test]
    fn recent_commands_are_the_latest_first_each_once_with_continued_lines_joined() {
        let history = "git status\ncargo build\ngit status\nGet-ChildItem `\n  -Recurse\n\ncargo test\n";
        assert_eq!(recent_commands(history, 10), ["cargo test", "Get-ChildItem \n  -Recurse", "git status", "cargo build"]);
        assert_eq!(recent_commands(history, 2), ["cargo test", "Get-ChildItem \n  -Recurse"]);
        assert!(recent_commands("", 5).is_empty());
    }

    #[test]
    fn the_tab_colours_have_names_of_their_own_and_differ() {
        let mut names: Vec<_> = TAB_COLORS.iter().map(|(name, _)| name.id()).collect();
        names.dedup();
        assert_eq!(names.len(), TAB_COLORS.len());
        let mut rgb: Vec<_> = TAB_COLORS.iter().map(|(_, rgb)| *rgb).collect();
        rgb.sort_unstable();
        rgb.dedup();
        assert_eq!(rgb.len(), TAB_COLORS.len());
    }

    fn paste_settings() -> PasteSettings {
        Settings::default().paste
    }

    #[test]
    fn a_paste_of_several_lines_asks_unless_the_program_took_bracketed_paste() {
        let settings = paste_settings();
        assert_eq!(paste_warning("ls", false, &settings), None);
        assert_eq!(paste_warning("a\nb", false, &settings), Some(PasteWarning::Lines(2)));
        assert_eq!(paste_warning("a\r\nb\r\nc", false, &settings), Some(PasteWarning::Lines(3)), "CR LF is one break");
        assert_eq!(paste_warning("a\rb", false, &settings), Some(PasteWarning::Lines(2)));
        assert_eq!(paste_warning("ls\n", false, &settings), Some(PasteWarning::Lines(2)), "a final break runs the line");
        assert_eq!(paste_warning("a\nb", true, &settings), None, "bracketed paste is data");
        let quiet = PasteSettings { warn_multiline: false, ..settings };
        assert_eq!(paste_warning("a\nb", false, &quiet), None);
    }

    #[test]
    fn a_big_paste_asks_even_to_a_program_with_bracketed_paste_and_zero_turns_it_off() {
        let settings = paste_settings();
        let big = "x".repeat(settings.warn_large_bytes as usize + 1);
        assert_eq!(paste_warning(&big, true, &settings), Some(PasteWarning::Bytes(5121)));
        assert_eq!(paste_warning(&big[1..], true, &settings), None, "exactly the limit does not ask");
        let lines = format!("{big}\n{big}");
        assert_eq!(paste_warning(&lines, false, &settings), Some(PasteWarning::Lines(2)), "lines are named first");
        let off = PasteSettings { warn_large_bytes: 0, ..settings };
        assert_eq!(paste_warning(&big, true, &off), None);
    }

    #[test]
    fn sizes_are_short_and_use_binary_units() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(812), "812 B");
        assert_eq!(size_label(1024), "1 KiB");
        assert_eq!(size_label(5121), "5 KiB");
        assert_eq!(size_label(5400), "5.3 KiB");
        assert_eq!(size_label(12 * 1024 + 300), "12 KiB");
        assert_eq!(size_label(1024 * 1024), "1 MiB");
        assert_eq!(size_label(37 * 1024 * 1024), "37 MiB");
    }

    #[test]
    fn the_osc52_policy_decides_what_the_emulator_lets_a_program_do() {
        assert_eq!(osc52_for(Osc52Policy::Off), Osc52::Disabled);
        assert_eq!(osc52_for(Osc52Policy::Write), Osc52::OnlyCopy);
        assert_eq!(osc52_for(Osc52Policy::ReadWrite), Osc52::CopyPaste);
        let defaults = core_config(&Settings::default());
        assert_eq!(defaults.osc52, Osc52::OnlyCopy, "a new pane may copy and may not read");
        let reading = core_config(&Settings { osc52: Osc52Policy::ReadWrite, ..Settings::default() });
        assert_eq!(reading.osc52, Osc52::CopyPaste);
    }

    #[test]
    fn executable_paths_become_program_names() {
        assert_eq!(tab_title(r"C:\Program Files\PowerShell\7\pwsh.exe", "x"), "pwsh");
        assert_eq!(tab_title(r#""C:\Windows\System32\cmd.exe" /k echo"#, "x"), "cmd");
        assert_eq!(tab_title(r"C:\WINDOWS\system32\cmd.exe", "x"), "cmd");
        assert_eq!(tab_title("wsl.exe", "x"), "wsl");
    }

    #[test]
    fn titles_a_program_chose_are_kept() {
        assert_eq!(tab_title("  claude - Nekominal  ", "x"), "claude - Nekominal");
        assert_eq!(tab_title("vim main.rs", "x"), "vim main.rs");
        assert_eq!(tab_title(r"C:\Users\me\src", "x"), r"C:\Users\me\src");
    }

    #[test]
    fn an_empty_title_uses_the_fallback() {
        assert_eq!(tab_title("", "PowerShell"), "PowerShell");
        assert_eq!(tab_title("   ", "PowerShell"), "PowerShell");
    }

    #[test]
    fn dragging_a_tab_past_the_middle_of_a_neighbour_swaps_them() {
        let widths = [100.0, 100.0, 100.0];
        // Chip 0 has its middle at 50, chip 1 at 154, chip 2 at 258.
        assert_eq!(reorder_target(&widths, 4.0, 0, 60.0), 0);
        assert_eq!(reorder_target(&widths, 4.0, 0, 160.0), 1);
        assert_eq!(reorder_target(&widths, 4.0, 0, 300.0), 2);
        assert_eq!(reorder_target(&widths, 4.0, 2, 100.0), 1);
        assert_eq!(reorder_target(&widths, 4.0, 2, -40.0), 0);
        assert_eq!(reorder_target(&widths, 4.0, 1, 154.0), 1);
    }

    /// A small deterministic generator, so the property tests need no dependency and fail the same way twice.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn between(&mut self, low: f64, high: f64) -> f64 {
            low + (self.next() % 10_000) as f64 / 10_000.0 * (high - low)
        }

        fn below(&mut self, count: usize) -> usize {
            (self.next() % count as u64) as usize
        }
    }

    const GAP: f64 = 4.0;

    fn sum_with_gaps(widths: &[f64]) -> f64 {
        widths.iter().sum::<f64>() + GAP * widths.len().saturating_sub(1) as f64
    }

    #[test]
    fn five_tabs_share_a_649_dip_strip_and_thirty_hit_the_floor() {
        let five = tab_width(5, 649.0, GAP);
        assert!((five - 126.6).abs() < 1e-9, "{five}");
        assert!((sum_with_gaps(&[five; 5]) - 649.0).abs() < 1e-9, "the strip is used exactly");
        let twelve = tab_width(12, 649.0, GAP);
        assert!((twelve - 605.0 / 12.0).abs() < 1e-9, "{twelve}");
        let thirty = tab_width(30, 649.0, GAP);
        assert_eq!(thirty, CHIP_MIN_WIDTH, "more than fits: the chips keep their floor and the strip scrolls");
        assert!(sum_with_gaps(&[thirty; 30]) > 649.0);
        // Few tabs stop at the ceiling, and no tab at all is not a division by zero.
        assert_eq!(tab_width(1, 800.0, GAP), CHIP_MAX_WIDTH);
        assert_eq!(tab_width(2, 800.0, GAP), CHIP_MAX_WIDTH);
        assert_eq!(tab_width(0, 800.0, GAP), CHIP_MAX_WIDTH);
    }

    #[test]
    fn only_the_strip_and_the_count_decide_the_width_of_the_chips() {
        // The rule has neither a selected tab nor a title to look at, so selecting every tab in turn, or a program
        // retitling one, cannot change a width. What is left to hold it to is what it does with the two numbers it takes.
        let mut random = Lcg(7);
        for _ in 0..3000 {
            let count = 1 + random.below(40);
            let available = random.between(100.0, 2000.0);
            let width = tab_width(count, available, GAP);

            assert!((CHIP_MIN_WIDTH..=CHIP_MAX_WIDTH).contains(&width), "{count} in {available}: {width}");
            let total = sum_with_gaps(&vec![width; count]);
            // The chips fit the strip, unless they are at the floor: only then does it scroll.
            assert!(total <= available + 1e-6 || width == CHIP_MIN_WIDTH, "{count} in {available}: {width}");
            // And they use all of it, unless they are at the ceiling.
            assert!((total - available).abs() < 1e-6 || width == CHIP_MIN_WIDTH || width == CHIP_MAX_WIDTH);

            // More room never narrows a chip, another tab never widens one.
            let wider = tab_width(count, available + random.between(0.0, 500.0), GAP);
            assert!(wider >= width - 1e-9, "{width} became {wider}");
            let with_new = tab_width(count + 1, available, GAP);
            assert!(with_new <= width + 1e-9, "{width} became {with_new}");
        }
    }
    fn agent(state: AgentState) -> PaneSignal {
        let record =
            AgentRecord { id: String::new(), state, kind: None, progress: None, app: None, title: None, message: None };
        PaneSignal { agent: Some(record), ..PaneSignal::default() }
    }

    #[test]
    fn a_tab_shows_its_most_urgent_pane_by_the_urgency_of_the_emulator() {
        use AgentState::*;
        let state_of = |states: &[AgentState]| tab_status(&states.iter().map(|s| agent(*s)).collect::<Vec<_>>()).state;
        assert_eq!(state_of(&[]), TabState::Idle);
        assert_eq!(state_of(&[Done, Working]), TabState::Working);
        assert_eq!(state_of(&[Working, Blocked, Done]), TabState::Blocked);
        assert_eq!(state_of(&[Working, Error]), TabState::Error);
        assert_eq!(state_of(&[Done]), TabState::Done);
        // The order is the emulator's, not a copy of it: whatever it ranks higher wins.
        let all = [Idle, Working, Done, Blocked, Error];
        for a in all {
            for b in all {
                let won = tab_status(&[agent(a), agent(b)]).state;
                let expected = if b.urgency() > a.urgency() { b } else { a };
                let expected = match expected {
                    Idle => TabState::Idle,
                    Working => TabState::Working,
                    Done => TabState::Done,
                    Blocked => TabState::Blocked,
                    Error => TabState::Error,
                };
                assert_eq!(won, expected, "{a:?} against {b:?}");
            }
        }
    }

    #[test]
    fn equally_urgent_panes_resolve_to_the_first_and_the_status_carries_its_progress_and_message() {
        let mut first = agent(AgentState::Working);
        first.agent.as_mut().unwrap().progress = Some(40);
        first.agent.as_mut().unwrap().message = Some("tests".into());
        let mut second = agent(AgentState::Working);
        second.agent.as_mut().unwrap().progress = Some(90);
        let status = tab_status(&[first, second]);
        assert_eq!(
            (status.state, status.progress, status.message.as_deref()),
            (TabState::Working, Some(40), Some("tests"))
        );
    }

    #[test]
    fn a_bell_a_notification_and_an_exit_are_the_fallbacks_below_what_an_agent_reports() {
        let attention =
            |kind| PaneSignal { attention: Some(kind), note: Some("built".into()), ..PaneSignal::default() };
        assert_eq!(tab_status(&[attention(Attention::Bell)]).state, TabState::Attention);
        let notified = tab_status(&[attention(Attention::Notification)]);
        assert_eq!(
            (notified.state, notified.attention, notified.message.as_deref()),
            (TabState::Attention, Some(Attention::Notification), Some("built"))
        );
        assert_eq!(tab_status(&[attention(Attention::Exit)]).state, TabState::Error, "a failed exit is an error");
        // Done outranks a bell, working outranks done, an exit error outranks working.
        assert_eq!(tab_status(&[attention(Attention::Bell), agent(AgentState::Done)]).state, TabState::Done);
        assert_eq!(tab_status(&[agent(AgentState::Done), attention(Attention::Exit)]).state, TabState::Error);
        let progress = PaneSignal { progress: Some(Reported::Percent(30)), ..PaneSignal::default() };
        let status = tab_status(&[attention(Attention::Bell), progress.clone()]);
        assert_eq!((status.state, status.progress), (TabState::Working, Some(30)));
        assert_eq!(tab_status(&[progress, agent(AgentState::Blocked)]).state, TabState::Blocked);
        let busy = PaneSignal { progress: Some(Reported::Indeterminate), ..PaneSignal::default() };
        assert_eq!(tab_status(&[busy]), TabStatus { state: TabState::Working, ..TabStatus::default() });
    }

    #[test]
    fn a_failed_exit_outranks_the_progress_the_program_left_behind() {
        let left_behind = |attention| PaneSignal {
            progress: Some(Reported::Percent(60)),
            attention,
            ..PaneSignal::default()
        };
        let failed = tab_status(&[left_behind(Some(Attention::Exit))]);
        assert_eq!((failed.state, failed.progress, failed.attention), (TabState::Error, None, Some(Attention::Exit)));
        // Of the other reasons for attention, progress outranks them: the program is still at work.
        for kind in [Attention::Bell, Attention::Notification] {
            let status = tab_status(&[left_behind(Some(kind))]);
            assert_eq!((status.state, status.progress), (TabState::Working, Some(60)), "{kind:?}");
        }
        // And across panes: the failed one wins over another pane that works.
        let other = PaneSignal { progress: Some(Reported::Indeterminate), ..PaneSignal::default() };
        assert_eq!(tab_status(&[other, left_behind(Some(Attention::Exit))]).state, TabState::Error);
    }

    #[test]
    fn an_agent_record_that_says_idle_shows_nothing() {
        assert_eq!(tab_status(&[agent(AgentState::Idle)]), TabStatus::default());
    }

    fn tab_state_of(state: AgentState) -> TabState {
        match state {
            AgentState::Idle => TabState::Idle,
            AgentState::Working => TabState::Working,
            AgentState::Done => TabState::Done,
            AgentState::Blocked => TabState::Blocked,
            AgentState::Error => TabState::Error,
        }
    }

    #[test]
    fn states_are_ranked_by_the_urgency_of_the_emulator_with_a_bell_between_idle_and_done() {
        let all = [AgentState::Idle, AgentState::Working, AgentState::Done, AgentState::Blocked, AgentState::Error];
        for a in all {
            for b in all {
                assert_eq!(
                    state_rank(tab_state_of(a)).cmp(&state_rank(tab_state_of(b))),
                    a.urgency().cmp(&b.urgency()),
                    "{a:?} against {b:?}"
                );
            }
        }
        assert!(state_rank(TabState::Idle) < state_rank(TabState::Attention));
        assert!(state_rank(TabState::Attention) < state_rank(TabState::Done));
        // The status of a tab is the one of its most urgent pane, by this rank.
        let mixed = [agent(AgentState::Done), agent(AgentState::Working)];
        assert_eq!(tab_status(&mixed).state, TabState::Working);
        assert!(state_rank(TabState::Working) > state_rank(TabState::Done));
    }

    #[test]
    fn the_window_title_counts_every_tab_that_waits_the_selected_and_the_seen_ones_included() {
        let tabs = [true, true, false, true, false].map(|waiting| TabAttention { waiting });
        assert_eq!(waiting_count(&tabs), 3);
        assert_eq!(waiting_count(&[]), 0);
    }

    #[test]
    fn a_count_is_text_with_a_mark_and_is_not_there_for_none() {
        assert_eq!(attention_text(0), None);
        assert_eq!(attention_text(3).as_deref(), Some("!3"));
        assert_eq!(attention_text(12).as_deref(), Some("!12"));
        assert_eq!(window_title("Nekominal", 2, false), "!2 Nekominal");
        assert_eq!(window_title("Nekominal", 2, true), "Nekominal", "an active window shows no count");
        assert_eq!(window_title("Nekominal", 0, false), "Nekominal");
    }

    #[test]
    fn the_next_tab_that_waits_follows_the_strip_order_and_wraps() {
        let waiting = [false, true, false, true, true, false];
        assert_eq!(next_waiting(&waiting, 0), Some(1));
        assert_eq!(next_waiting(&waiting, 1), Some(3));
        assert_eq!(next_waiting(&waiting, 3), Some(4));
        assert_eq!(next_waiting(&waiting, 4), Some(1), "after the last one it wraps to the first");
        assert_eq!(next_waiting(&waiting, 5), Some(1));
        assert_eq!(next_waiting(&[false, true], 1), None, "the only tab that waits is the one shown");
        assert_eq!(next_waiting(&[false, false, false], 1), None);
        assert_eq!(next_waiting(&[true], 0), None);
        assert_eq!(next_waiting(&[], 0), None);
        // Visiting them in turn from the first returns to where it began.
        let mut at = 0;
        let mut seen = Vec::new();
        for _ in 0..3 {
            at = next_waiting(&waiting, at).unwrap();
            seen.push(at);
        }
        assert_eq!(seen, [1, 3, 4]);
    }

    #[test]
    fn a_tab_shows_the_icon_of_the_program_it_runs_whatever_the_case_or_the_extension() {
        assert_eq!(job_icon("vim.exe"), job_icon("NVIM.EXE"));
        assert_eq!(job_icon("ssh.exe"), '\u{E774}');
        assert_eq!(job_icon("git"), '\u{E81C}');
        assert_eq!(job_icon("python3.12.exe"), job_icon("py.exe"));
        assert_eq!(job_icon("npm.cmd"), job_icon("node.exe"));
        // A shell and a program nobody knows have the terminal's glyph; a name that only ends like an extension is no extension.
        assert_eq!(job_icon("pwsh.exe"), '\u{E756}');
        assert_eq!(job_icon("whatever.exe"), '\u{E756}');
        assert_ne!(job_icon("exe"), '\u{E70F}');
        assert_ne!(job_icon("vim.exe"), job_icon("pwsh.exe"));
    }

    #[test]
    fn every_profile_has_a_mark_and_the_shells_are_told_apart() {
        let marks: Vec<Mark> = nekominal_pty::profiles::ICON_KEYS.iter().map(|key| profile_mark(key)).collect();
        for (i, mark) in marks.iter().enumerate() {
            assert!(marks[i + 1..].iter().all(|other| other != mark), "{} looks like another profile", nekominal_pty::profiles::ICON_KEYS[i]);
        }
        assert_eq!(profile_mark("terminal"), profile_mark("something else"), "a profile nobody knows is generic");
        assert!(!marks.contains(&profile_mark("terminal")), "a known profile is not the generic one");
    }

    #[test]
    fn a_folder_is_shown_by_its_last_segment_with_home_and_drives_spelled_short() {
        let home = Some(r"C:\Users\me");
        assert_eq!(folder_label(r"C:\Users\me\src\nekominal", home), "nekominal");
        assert_eq!(folder_label(r"C:\Users\me\src\nekominal\", home), "nekominal");
        assert_eq!(folder_label(r"C:\Users\me", home), "~");
        assert_eq!(folder_label(r"c:\users\ME\", home), "~", "paths of Windows ignore case and a trailing slash");
        assert_eq!(folder_label("C:/Users/me", home), "~", "OSC 7 may report forward slashes");
        assert_eq!(folder_label(r"C:\Users", home), "Users");
        assert_eq!(folder_label(r"C:\", home), "C:");
        assert_eq!(folder_label("D:", home), "D:");
        assert_eq!(folder_label(r"\\server\share\docs", home), "docs");
        assert_eq!(folder_label("/home/me/project", home), "project");
        assert_eq!(folder_label("/", home), "/");
        assert_eq!(folder_label("", home), "");
        assert_eq!(folder_label(r"C:\Users\me", None), "me");
        assert_eq!(folder_label("", Some("")), "", "an unknown home is not every folder");
    }

    /// The title of a tab by the default format: what a program says, else the folder, else the profile.
    fn titled(parts: &TitleParts, with_parent: bool) -> (String, TitleSource) {
        default_title(parts, &TabTitle::default().parts, with_parent)
    }

    /// The title by a format written as it is in the settings: `["folder", "title|profile"]`.
    fn titled_by(texts: &[&str], parts: &TitleParts) -> (String, TitleSource) {
        let format: Vec<TitleSlot> = texts.iter().map(|text| TitleSlot::parse(text).expect("known parts")).collect();
        default_title(parts, &format, false)
    }
    fn parts(program: &str, cwd: Option<&str>) -> TitleParts {
        TitleParts {
            program: program.into(),
            cwd: cwd.map(Into::into),
            profile: "PowerShell".into(),
            shell: "pwsh".into(),
            home: Some(r"C:\Users\me".into()),
            untitled: "終端機".into(),
            ..TitleParts::default()
        }
    }

    #[test]
    fn a_format_joins_the_slots_that_have_something_to_say_and_skips_the_others() {
        let mut p = parts("build", Some(r"C:\src\nekominal"));
        p.agent_app = Some("claude".into());
        let built = |texts: &[&str], p: &TitleParts| titled_by(texts, p).0;
        assert_eq!(built(&["folder", "title"], &p), "nekominal · build");
        assert_eq!(built(&["title", "folder", "profile"], &p), "build · nekominal · PowerShell");
        assert_eq!(built(&["agent", "folder"], &p), "claude · nekominal");
        assert_eq!(built(&["shell"], &p), "pwsh");
        // The first part of a slot that has something to say fills it; the others of that slot stay silent.
        assert_eq!(built(&["title|folder"], &p), "build");
        p.program.clear();
        assert_eq!(built(&["title|folder"], &p), "nekominal");
        // A slot that says nothing leaves no separator behind, at the start, in the middle or at the end.
        assert_eq!(built(&["title", "folder"], &p), "nekominal");
        assert_eq!(built(&["folder", "title"], &p), "nekominal");
        assert_eq!(
            built(&["agent", "title", "folder", "title|agent", "profile"], &p),
            "claude · nekominal · claude · PowerShell"
        );
        p.cwd = None;
        assert_eq!(built(&["folder", "title"], &p), "終端機", "nothing to say: the title of last resort");
        assert_eq!(titled_by(&["folder", "title"], &p).1, TitleSource::Untitled);
        // The user's name wins over any format, and the folder is in the title only when a part says it.
        p.user = Some("mine".into());
        assert_eq!(titled_by(&["folder"], &p), ("mine".to_owned(), TitleSource::User));
    }

    #[test]
    fn a_title_that_says_its_folder_can_take_the_parent_and_one_that_does_not_cannot() {
        let p = parts("build", Some(r"C:\work\api"));
        let format =
            |texts: &[&str]| -> Vec<TitleSlot> { texts.iter().map(|text| TitleSlot::parse(text).unwrap()).collect() };
        assert_eq!(
            default_title(&p, &format(&["folder", "title"]), false),
            ("api · build".to_owned(), TitleSource::Parts { folder: true })
        );
        assert_eq!(default_title(&p, &format(&["folder", "title"]), true).0, "work/api · build");
        let without = default_title(&p, &format(&["title", "profile"]), true);
        assert_eq!(without, ("build · PowerShell".to_owned(), TitleSource::Parts { folder: false }));
        // A fallback that did not reach the folder does not count as saying it.
        assert_eq!(
            default_title(&p, &format(&["title|folder"]), true),
            ("build".to_owned(), TitleSource::Parts { folder: false })
        );
    }

    #[test]
    fn a_title_is_made_of_cleaned_parts_and_stays_within_the_limit() {
        let mut p = parts("pay\u{202e}load", Some("C:\\work\\a\u{7}pi"));
        p.agent_app = Some("cla\u{202e}ude".into());
        assert_eq!(titled_by(&["title", "folder", "agent"], &p).0, "payload · api · claude");
        let long = TitleParts { program: "x".repeat(200), ..parts("", Some(r"C:\work\api")) };
        let title = titled_by(&["title", "folder"], &long).0;
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        assert!(title.starts_with("xxx") && !title.contains('·'), "the cut keeps the head");
    }
    #[test]
    fn a_title_comes_from_the_user_then_the_program_then_the_folder_then_the_profile_then_nothing() {
        let mut p = parts("build", Some(r"C:\src\nekominal"));
        p.user = Some("  mine ".into());
        assert_eq!(titled(&p, false), ("mine".to_owned(), TitleSource::User));
        p.user = None;
        assert_eq!(titled(&p, false), ("build".to_owned(), TitleSource::Parts { folder: false }));
        p.program.clear();
        assert_eq!(titled(&p, false), ("nekominal".to_owned(), TitleSource::Parts { folder: true }));
        p.cwd = None;
        assert_eq!(titled(&p, false), ("PowerShell".to_owned(), TitleSource::Parts { folder: false }));
        p.profile.clear();
        assert_eq!(titled(&p, false), ("終端機".to_owned(), TitleSource::Untitled));
        // A name of only invisible characters is no name.
        p.user = Some("\u{202e}\u{7}  ".into());
        assert_eq!(titled(&p, false).1, TitleSource::Untitled);
    }

    #[test]
    fn titles_that_only_name_the_shell_the_profile_or_the_agent_app_are_generic() {
        let folder = Some(r"C:\work\api");
        for generic in [r"C:\Program Files\PowerShell\7\pwsh.exe", "pwsh.exe", "PWSH", "powershell", "", "  "] {
            let mut p = parts(generic, folder);
            p.profile = "PowerShell".into();
            assert_eq!(titled(&p, false), ("api".to_owned(), TitleSource::Parts { folder: true }), "{generic:?}");
        }
        let mut agent = parts("claude", folder);
        agent.agent_app = Some("claude".into());
        assert_eq!(titled(&agent, false).0, "api", "the app id of the agent says nothing");
        agent.agent_title = Some("Fix the build".into());
        assert_eq!(titled(&agent, false), ("Fix the build".to_owned(), TitleSource::Parts { folder: false }));
        // Titles a program chose are kept as they are, an executable's path or not.
        for chosen in ["build", "vim main.rs", "claude - Nekominal", "✳ Claude Code"] {
            assert_eq!(
                titled(&parts(chosen, folder), false),
                (chosen.to_owned(), TitleSource::Parts { folder: false })
            );
        }
    }

    #[test]
    fn a_profile_named_like_the_title_is_generic_even_when_it_is_not_the_shell() {
        let mut p = parts("Windows PowerShell", Some(r"C:\work\api"));
        p.profile = "Windows PowerShell".into();
        p.shell = "powershell".into();
        assert_eq!(titled(&p, false).0, "api");
    }

    #[test]
    fn titles_lose_control_and_bidirectional_characters() {
        assert_eq!(clean_title("  build\u{202e}evil\u{7}\u{1b}[31m "), "buildevil[31m");
        assert_eq!(clean_title("a\u{2066}b\u{2069}c"), "abc");
        assert_eq!(clean_title("終端\u{200b}機"), "終端\u{200b}機", "zero-width joiners of real text stay");
        assert_eq!(titled(&parts("pay\u{202e}load", None), false).0, "payload");
        assert_eq!(titled(&TitleParts { agent_title: Some("x\u{202e}y".into()), ..parts("", None) }, false).0, "xy");
    }

    #[test]
    fn a_title_is_cut_between_characters_at_a_fixed_length_whatever_its_size() {
        let cjk = "終".repeat(500);
        let cut = clean_title(&cjk);
        assert_eq!(cut.chars().count(), MAX_TITLE_CHARS);
        assert!(cut.chars().all(|c| c == '終'), "no character is split");
        assert_eq!(clean_title(&"😀".repeat(MAX_TITLE_CHARS + 1)).chars().count(), MAX_TITLE_CHARS);
        let exact = "x".repeat(MAX_TITLE_CHARS);
        assert_eq!(clean_title(&exact), exact, "a title that fits is kept whole");
        // A title of 4 MiB, the most an OSC payload may carry, ends up as short as any other.
        assert_eq!(clean_title(&"y".repeat(4 * 1024 * 1024)).len(), MAX_TITLE_CHARS);
        // Blanks and invisible characters at the start do not use up the length.
        let padded = format!("{}\u{200e}\u{7}{}", " ".repeat(300), "z".repeat(300));
        assert_eq!(clean_title(&padded), "z".repeat(MAX_TITLE_CHARS));
        assert_eq!(clean_title(&format!("name{}", " ".repeat(300))), "name", "and the end is trimmed");
    }

    #[test]
    fn directional_marks_separators_and_noncharacters_are_cleaned_out_of_titles() {
        assert_eq!(clean_title("a\u{200e}b\u{200f}c\u{61c}d\u{2028}e\u{2029}f\u{fffe}g\u{ffff}h\u{fdd0}i"), "abcdefghi");
        assert_eq!(clean_title("終端\u{200d}機"), "終端\u{200d}機", "a joiner is part of real text");
    }

    #[test]
    fn the_folder_of_a_working_directory_is_cleaned_like_any_title() {
        // A shell reports whatever the path holds: its last segment never reaches a chip raw.
        assert_eq!(folder_label("C:\\work\\pay\u{202e}load", None), "payload");
        assert_eq!(folder_label("C:\\work\\a\u{7}b\u{1b}[31m", None), "ab[31m");
        assert_eq!(folder_label("/home/me/x\u{2028}y\u{fffe}", None), "xy");
        let long = format!("C:\\work\\{}", "d".repeat(400));
        assert_eq!(folder_label(&long, None).chars().count(), MAX_TITLE_CHARS);
        assert_eq!(folder_label("C:\\work\\\u{202e}\u{7}", None), "", "a folder of invisible characters has no name");
        // The same cleaning reaches the title of a tab, with a parent or not.
        let at = |cwd: &str, with_parent| titled(&parts("", Some(cwd)), with_parent).0;
        assert_eq!(at("C:\\Users\\me\\src\\pay\u{202e}load", false), "payload");
        assert_eq!(at("C:\\Users\\me\\sr\u{7}c\\pay\u{202e}load", true), "src/payload");
        assert_eq!(titled(&parts("", Some("C:\\a\\\u{202e}\u{7}")), false).1, TitleSource::Parts { folder: false });
        assert!(at(&long, true).chars().count() <= MAX_TITLE_CHARS);
    }

    #[test]
    fn the_folder_is_the_last_segment_and_gets_its_parent_only_when_asked() {
        let at = |cwd| parts("", Some(cwd));
        assert_eq!(titled(&at(r"C:\Users\me"), false).0, "~");
        assert_eq!(titled(&at(r"C:\"), false).0, "C:");
        assert_eq!(titled(&at(r"C:\Users\me\nekominal\src"), false).0, "src");
        assert_eq!(titled(&at(r"C:\Users\me\nekominal\src"), true).0, "nekominal/src");
        assert_eq!(titled(&at(r"C:\work"), true).0, "C:/work");
        assert_eq!(titled(&at("/home/me/api"), true).0, "me/api");
        // There is no parent to add to these.
        assert_eq!(titled(&at(r"C:\Users\me"), true).0, "~");
        assert_eq!(titled(&at(r"C:\"), true).0, "C:");
        assert_eq!(titled(&at("/"), true).0, "/");
        assert_eq!(titled(&at("/usr"), true).0, "usr");
    }

    #[test]
    fn a_single_pane_fills_the_area_without_a_divider() {
        let rect = pane_px_rect(UNIT, (1000, 600), 2);
        assert_eq!(rect, PxRect { x: 0, y: 0, w: 1000, h: 600 });
    }

    #[test]
    fn neighbours_tile_the_area_and_leave_exactly_the_divider_between_them() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Row, 2).split_pane(2, Axis::Column, 3);
        let rects: Vec<PxRect> =
            tree.layout_rects(UNIT).into_iter().map(|(_, rect)| pane_px_rect(rect, (1001, 601), 3)).collect();
        // Pane 1 is the left half, 2 and 3 stack on the right.
        let (left, top_right, bottom_right) = (rects[0], rects[1], rects[2]);
        assert_eq!(left.x, 0);
        assert_eq!(top_right.x - (left.x + left.w), 3, "divider between left and right");
        assert_eq!(top_right.x, bottom_right.x);
        assert_eq!(top_right.x + top_right.w, 1001);
        assert_eq!(bottom_right.y - (top_right.y + top_right.h), 3, "divider between top and bottom");
        assert_eq!(bottom_right.y + bottom_right.h, 601);
    }

    #[test]
    fn the_divider_sits_in_the_gap_between_the_panes() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Row, 2);
        let rects = tree.layout_rects(UNIT);
        let (a, b) = (pane_px_rect(rects[0].1, (1000, 500), 3), pane_px_rect(rects[1].1, (1000, 500), 3));
        let divider = splitter_px_rect(&tree.splitters(UNIT)[0], (1000, 500), 3);
        assert_eq!(divider.x, a.x + a.w);
        assert_eq!(divider.x + divider.w, b.x);
        assert_eq!((divider.y, divider.h), (0, 500));
    }

    #[test]
    fn a_dragged_divider_converts_back_to_a_ratio() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Column, 2).split_pane(2, Axis::Row, 3);
        let inner = &tree.splitters(UNIT)[1];
        assert_eq!(inner.axis, Axis::Row);
        // The inner split spans the lower half; a divider at 0.25 of the area is a quarter of that half.
        assert!((ratio_at(inner, 0.25) - 0.25).abs() < 1e-9);
        let outer = &tree.splitters(UNIT)[0];
        assert!((ratio_at(outer, 0.3) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn divider_is_one_dip_in_pixels() {
        assert_eq!(divider_px(1.0), 1);
        assert_eq!(divider_px(1.5), 2);
        assert_eq!(divider_px(2.0), 2);
        assert_eq!(divider_px(0.0), 1);
    }

    #[test]
    fn a_change_of_the_settings_puts_in_effect_only_what_it_changes() {
        use nekominal_model::settings::{Backdrop, Osc52Policy};
        use nekominal_model::settings::CursorStyle;
        let before = Settings::default();
        let after = |change: fn(&mut Settings)| {
            let mut now = before.clone();
            change(&mut now);
            effects(&before, &now)
        };
        assert_eq!(after(|_| {}), Effects::default());
        assert_eq!(after(|s| s.theme = "Nord".into()), Effects { glass: true, ..Effects::default() });
        assert_eq!(after(|s| s.appearance.backdrop = Backdrop::Mica), Effects { glass: true, ..Effects::default() });
        assert_eq!(after(|s| s.font.size = 16.0), Effects { font: true, ..Effects::default() });
        assert_eq!(after(|s| s.font.line_height = 1.3), Effects { font: true, ..Effects::default() });
        assert_eq!(after(|s| s.font.family = vec!["Fira Code".into()]), Effects { font: true, ..Effects::default() });
        assert_eq!(after(|s| s.font.ligatures = false), Effects { font: true, ..Effects::default() });
        assert_eq!(after(|s| s.osc52 = Osc52Policy::Off), Effects { osc52: true, ..Effects::default() });
        assert_eq!(after(|s| s.cursor.style = CursorStyle::Bar), Effects { cursor: true, ..Effects::default() });
        assert_eq!(after(|s| s.scrollback = 500), Effects { scrollback: true, ..Effects::default() });
        assert_eq!(after(|s| s.sticky_header = false), Effects { sticky_header: true, ..Effects::default() });
        // The opacity has its own way (the next frame applies it), and the rest is read where it is used.
        assert_eq!(after(|s| s.appearance.opacity = 0.5), Effects::default());
        assert_eq!(after(|s| s.confirm_close = false), Effects::default());
        assert_eq!(after(|s| s.keyboard.kitty_protocol = false), Effects::default());
    }

    #[test]
    fn a_new_pane_starts_with_the_cursor_the_scrollback_and_the_keyboard_protocol_of_the_settings() {
        use nekominal_model::settings::Keyboard;
        let defaults = core_config(&Settings::default());
        assert_eq!((defaults.cursor_style.shape, defaults.cursor_style.blinking), (CursorShape::Block, true));
        assert_eq!((defaults.scrollback_lines, defaults.kitty_keyboard), (10_000, true));
        let mut settings = Settings::default();
        settings.cursor = Cursor { style: CursorShapeSetting::Bar, blink: false, blink_rate: 530 };
        settings.scrollback = 250;
        settings.keyboard = Keyboard { kitty_protocol: false };
        let config = core_config(&settings);
        assert_eq!((config.cursor_style.shape, config.cursor_style.blinking), (CursorShape::Beam, false));
        assert_eq!((config.scrollback_lines, config.kitty_keyboard), (250, false));
        assert_eq!(cursor_style_for(&Cursor { style: CursorShapeSetting::Underline, blink: true, blink_rate: 530 }).shape, CursorShape::Underline);
    }
}
