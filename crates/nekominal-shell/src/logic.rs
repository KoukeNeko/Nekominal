//! Toolkit-free decisions of the shell: tab titles, tab drag targets, the pixel geometry of split panes and the
//! terminal setup (OSC 52 policy, when a paste asks first). Kept apart from the WinUI code so they can be tested
//! without a window.

use nekominal_model::commands::Command;
use nekominal_model::layout::{Axis, Rect, Splitter};
use nekominal_model::settings::{
    Cursor, CursorStyle as CursorShapeSetting, OPACITY_RANGE, Osc52Policy, PasteSettings, Settings, TITLE_SEPARATOR,
    TitlePart, TitleSlot,
};
use nekominal_model::settings_ui::Range;
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

/// The pane wants an answer or a look: a bell, a notification or a failed exit that nobody has seen, or an agent
/// that waits for an answer or failed. A pane in the tab that is not the focused one then gets a warning line.
pub fn needs_response(pane: &PaneSignal) -> bool {
    pane.attention.is_some()
        || pane.agent.as_ref().is_some_and(|r| matches!(r.state, AgentState::Blocked | AgentState::Error))
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

/// What the panes hidden by a zoom ask for: the status of those that wait for the user (`needs_response`), or `None`
/// when nothing is zoomed or none of the hidden panes waits. `zoomed` is the index in `panes` of the zoomed pane, which
/// is on screen and needs no such mark. The corner label of the zoomed pane shows it.
pub fn hidden_alert(panes: &[PaneSignal], zoomed: Option<usize>) -> Option<TabStatus> {
    let zoomed = zoomed?;
    let waiting: Vec<PaneSignal> = panes
        .iter()
        .enumerate()
        .filter(|(index, pane)| *index != zoomed && needs_response(pane))
        .map(|(_, pane)| pane.clone())
        .collect();
    (!waiting.is_empty()).then(|| tab_status(&waiting))
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

/// What a window-level alert (an announcement to assistive technology, a flash of the taskbar button) is raised for.
/// A bell is not one: it has its own settings (`bell_effects`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Blocked,
    Error,
    Notification,
}

pub fn alert_kind(status: &TabStatus) -> Option<AlertKind> {
    match status.state {
        TabState::Blocked => Some(AlertKind::Blocked),
        TabState::Error => Some(AlertKind::Error),
        TabState::Attention if status.attention == Some(Attention::Notification) => Some(AlertKind::Notification),
        _ => None,
    }
}

/// What a tab asks of the user at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Need {
    /// A pane of the tab waits for the user (`needs_response`).
    pub waiting: bool,
    /// The reason, when it is one a window-level alert is raised for.
    pub alert: Option<AlertKind>,
}

impl Need {
    pub fn of(panes: &[PaneSignal], status: &TabStatus) -> Need {
        Need { waiting: panes.iter().any(needs_response), alert: alert_kind(status) }
    }
}

/// How many alerts may be outstanding at once: a third tab that needs a response while two alerts are still waiting
/// for the user gets its mark, its unread flag and its place in the counts, but no announcement and no flash.
pub const MAX_ALERTS: usize = 2;

/// What the user has been shown of the response a tab needs. The flag is separate from the status the chip draws, which
/// is read live from the panes: it says only that the response has not been looked at.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    need: Need,
    unread: bool,
    alerted: bool,
}

impl Watch {
    /// A response nobody has looked at yet: it began while the tab was out of sight (another tab was selected, or the
    /// window was not active), and neither viewing the tab nor the end of the need has come since.
    pub fn unread(&self) -> bool {
        self.unread
    }

    /// A window-level alert was raised for it and is still outstanding.
    pub fn alerted(&self) -> bool {
        self.alerted
    }

    /// The tab asks for `now`. `watched` is whether the user is looking at it: it is the selected tab of the active
    /// window. `other_alerts` is how many other tabs have an alert that is still outstanding. Returns the alert to
    /// raise, if this is the moment for one.
    ///
    /// A need that begins (or turns into a more urgent kind) while the tab is out of sight is unread. Watching the
    /// tab, or the need ending, clears it; the tab being left again afterwards does not bring it back.
    pub fn observe(&mut self, now: Need, watched: bool, other_alerts: usize) -> Option<AlertKind> {
        let before = std::mem::replace(&mut self.need, now);
        if !now.waiting || watched {
            self.unread = false;
            self.alerted = false;
            return None;
        }
        let fresh = !before.waiting || (now.alert.is_some() && now.alert != before.alert);
        self.unread |= fresh;
        let raise = if fresh && other_alerts < MAX_ALERTS { now.alert } else { None };
        self.alerted |= raise.is_some();
        raise
    }
}

/// A tab as the counts of the position plate and of the window title see it.
#[derive(Clone, Copy, Debug)]
pub struct TabAttention {
    pub state: TabState,
    pub unread: bool,
    pub selected: bool,
    /// A pane of the tab waits for the user.
    pub waiting: bool,
}

/// The `!N` of the position plate: the tabs that are not selected, are blocked or failed, and have a response that
/// nobody has looked at.
pub fn plate_count(tabs: &[TabAttention]) -> usize {
    tabs.iter().filter(|tab| tab.unread && !tab.selected && is_urgent(tab.state)).count()
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

/// Where the pane that waits for the user is among the panes of its tab, as (position, count) one-based; the first
/// such pane in `waiting` (the panes in the order of the layout). Nothing for a tab with a single pane.
pub fn waiting_pane(waiting: &[bool]) -> Option<(usize, usize)> {
    if waiting.len() < 2 {
        return None;
    }
    waiting.iter().position(|pane| *pane).map(|index| (index + 1, waiting.len()))
}

/// The heading of a group of rows in the list of every tab: the state its tabs are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heading {
    pub state: TabState,
    /// Told apart for a state of `Attention` only: a notification and a bell are two groups.
    pub attention: Option<Attention>,
}

/// The order of the rows of the list of every tab, as (index of the tab in strip order, heading it opens). While some
/// tab is unread (`any_unread`) the rows are grouped by state, the most urgent group first (`state_rank`), each group in
/// strip order, and the first row of a group carries its heading. Otherwise the rows keep the strip order and have
/// none. `tabs` are the state and the attention of every tab in strip order.
pub fn list_order(tabs: &[(TabState, Option<Attention>)], any_unread: bool) -> Vec<(usize, Option<Heading>)> {
    if !any_unread {
        return (0..tabs.len()).map(|index| (index, None)).collect();
    }
    let heading = |index: usize| {
        let (state, attention) = tabs[index];
        Heading { state, attention: if state == TabState::Attention { attention } else { None } }
    };
    let kind_order = |heading: Heading| match heading.attention {
        Some(Attention::Notification) => 0,
        Some(_) => 1,
        None => 2,
    };
    let mut order: Vec<usize> = (0..tabs.len()).collect();
    order.sort_by_key(|index| (std::cmp::Reverse(state_rank(tabs[*index].0)), kind_order(heading(*index)), *index));
    let mut previous = None;
    order
        .into_iter()
        .map(|index| {
            let current = heading(index);
            let opens = previous != Some(current);
            previous = Some(current);
            (index, opens.then_some(current))
        })
        .collect()
}

/// No chip is narrower than this, in DIP.
pub const CHIP_MIN_WIDTH: f64 = 48.0;
/// No chip is wider than this.
pub const CHIP_MAX_WIDTH: f64 = 216.0;
/// A chip narrower than this shows its number instead of its title.
pub const ORDINAL_BELOW: f64 = 64.0;
/// Below this strip width the position plate shows only the total.
pub const PLATE_TOTAL_ONLY_BELOW: f64 = 300.0;
/// Room kept around a chip that is scrolled into view.
pub const REVEAL_MARGIN: f64 = 12.0;
/// Width of what covers a cut edge of the strip: the block that counts the hidden chips (32) and its fade (16).
pub const COUNT_BLOCK_WIDTH: f64 = 48.0;

/// The width of every chip of the strip, for `count` tabs in a strip `available` DIP wide with `gap` between chips: an
/// equal share of the strip, kept between [`CHIP_MIN_WIDTH`] and [`CHIP_MAX_WIDTH`]. Only these two numbers decide it.
/// Which tab is selected and what the titles say do not, so nothing in the strip moves when a tab is selected or a
/// program retitles one. Once the floor is reached the chips together are wider than the strip, and it scrolls.
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

/// The tabs "close the other tabs" closes when tab `keep` of `count` stays: every other one, the last first, so that
/// closing one does not move the places of those still to close.
pub fn tabs_other_than(count: usize, keep: usize) -> Vec<usize> {
    (0..count).rev().filter(|i| *i != keep).collect()
}

/// The tabs "close the tabs to the right" closes: those after tab `from`, the last first.
pub fn tabs_right_of(count: usize, from: usize) -> Vec<usize> {
    (from.saturating_add(1)..count).rev().collect()
}

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

/// Whether a chip of this width shows its number instead of its title.
pub fn shows_ordinal(width: f64) -> bool {
    width < ORDINAL_BELOW
}

/// Width of the whole strip: the chips, the gaps between them and `padding` on both sides.
pub fn strip_extent(widths: &[f64], gap: f64, padding: f64) -> f64 {
    widths.iter().sum::<f64>() + gap * widths.len().saturating_sub(1) as f64 + padding
}

/// How many chips lie completely outside the visible part of the strip, to the left and to the right. A chip that
/// shows even partly counts as visible, so hidden plus visible chips are all of them.
pub fn hidden_counts(widths: &[f64], gap: f64, padding: f64, offset: f64, viewport: f64) -> (usize, usize) {
    let (mut left, mut right) = (0, 0);
    let mut edge = padding / 2.0;
    for width in widths {
        if edge + width <= offset {
            left += 1;
        } else if edge >= offset + viewport {
            right += 1;
        }
        edge += width + gap;
    }
    (left, right)
}

/// The scroll offset that shows the chip at `left` (strip coordinates) of `width`, with `margin` around it. An
/// edge with chips behind it is covered by a count block of `block` DIP, so the margin starts behind the block. The
/// offset stays as it is when the chip is already in view.
pub fn reveal_offset(
    (left, width): (f64, f64),
    offset: f64,
    viewport: f64,
    extent: f64,
    margin: f64,
    block: f64,
) -> f64 {
    let max_offset = (extent - viewport).max(0.0);
    let offset = offset.clamp(0.0, max_offset);
    let inset_left = margin + if offset > 0.0 { block } else { 0.0 };
    let inset_right = margin + if offset < max_offset { block } else { 0.0 };
    if left < offset + inset_left {
        let target = left - margin - block;
        return if target <= 0.0 { 0.0 } else { target.min(max_offset) };
    }
    if left + width > offset + viewport - inset_right {
        let target = left + width + margin + block - viewport;
        return if target >= max_offset { max_offset } else { target.max(0.0) };
    }
    offset
}

/// What the position plate says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plate {
    /// `current / total`, both one-based.
    Position { current: usize, total: usize },
    /// A narrow strip keeps only the count.
    Total(usize),
}

/// The plate for `total` tabs with the zero-based `selected` one: none for a single tab, the total alone when the
/// strip is narrower than [`PLATE_TOTAL_ONLY_BELOW`] or the bar is in its narrow tier (the room the plate would
/// take is what the window cannot spare).
pub fn plate(selected: usize, total: usize, strip_width: f64, tier: BarTier) -> Option<Plate> {
    if total < 2 {
        None
    } else if strip_width < PLATE_TOTAL_ONLY_BELOW || tier == BarTier::Narrow {
        Some(Plate::Total(total))
    } else {
        Some(Plate::Position { current: selected + 1, total })
    }
}

/// Which of the controls on the right of the top bar the width of the window leaves room for. The new-tab and
/// profile buttons on the left never fold, and the menu button on the right never does either.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BarTier {
    /// Only the menu button: the rest is in the menu.
    Narrow,
    /// The palette and the menu button.
    Medium,
    /// The split pair, the palette and the menu button.
    Wide,
}

/// From this window width (DIP) the bar is in the medium tier, and from [`WIDE_FROM`] in the wide one.
pub const MEDIUM_FROM: f64 = 800.0;
pub const WIDE_FROM: f64 = 1200.0;
/// A window that gets narrower keeps its tier for this much below the width a tier starts at, so dragging an edge
/// across a boundary does not flicker.
pub const TIER_HYSTERESIS: f64 = 24.0;

/// The commands the right-hand group of the bar can show, with the tier from which each shows. This is the order of
/// `nekominal_model::menu::TOOLBAR` (a test keeps them one list); the split pair folds together, before the palette.
pub const TOOLBAR_TIERS: [(Command, BarTier); 3] = [
    (Command::PaneSplitRight, BarTier::Wide),
    (Command::PaneSplitDown, BarTier::Wide),
    (Command::AppPalette, BarTier::Medium),
];

impl BarTier {
    /// The width from which the tier starts when the window grows.
    fn starts_at(self) -> f64 {
        match self {
            BarTier::Narrow => 0.0,
            BarTier::Medium => MEDIUM_FROM,
            BarTier::Wide => WIDE_FROM,
        }
    }

    /// The tier of a window `width` DIP wide that had `previous`. A window that grows takes the tier of its width at
    /// once; one that shrinks keeps `previous` until it is [`TIER_HYSTERESIS`] below where that tier starts.
    pub fn at(width: f64, previous: Option<BarTier>) -> BarTier {
        let mut tier = BarTier::Narrow;
        for candidate in [BarTier::Medium, BarTier::Wide] {
            let held = previous.is_some_and(|previous| previous >= candidate)
                && width + TIER_HYSTERESIS >= candidate.starts_at();
            if width >= candidate.starts_at() || held {
                tier = candidate;
            }
        }
        tier
    }

    /// Whether the right-hand group of the bar shows `command` in this tier.
    pub fn shows(self, command: Command) -> bool {
        TOOLBAR_TIERS.iter().any(|(shown, from)| *shown == command && self >= *from)
    }

    /// The divider between the split pair and the rest of the group shows when both are there.
    pub fn shows_divider(self) -> bool {
        self == BarTier::Wide
    }

    /// How many controls the right-hand group has, the menu button included.
    pub fn control_count(self) -> usize {
        1 + TOOLBAR_TIERS.iter().filter(|(command, _)| self.shows(*command)).count()
    }
}

/// Width of a tool button of the bar, of the padding of the group around them and of the divider with its margins, in
/// DIP (`resources.xaml`, `shell.xaml`; a test keeps them in step).
pub const TOOL_BUTTON_WIDTH: f64 = 32.0;
pub const CLUSTER_PADDING: f64 = 4.0;
pub const DIVIDER_WIDTH: f64 = 7.0;

/// The width of the group on the right of the bar in a tier.
pub fn cluster_width(tier: BarTier) -> f64 {
    CLUSTER_PADDING
        + TOOL_BUTTON_WIDTH * tier.control_count() as f64
        + if tier.shows_divider() { DIVIDER_WIDTH } else { 0.0 }
}

/// The smallest the drag area of the window gets, however narrow the window.
pub const MIN_DRAG_WIDTH: f64 = 48.0;

/// What the bar spends on neither the strip, the position plate, the caption buttons nor the group on the right: the
/// app mark and its margins (12 + 22 + 8), the margin and the new-tab and profile buttons (4 + 32 + 22), the margins
/// of the group on the right (8 + 8) and the smallest drag area (`shell.xaml`; a test keeps them in step).
pub const BAR_FIXED_WIDTH: f64 = 12.0 + 22.0 + 8.0 + 4.0 + 32.0 + 22.0 + 8.0 + 8.0 + MIN_DRAG_WIDTH;

/// The smallest the strip gets, however narrow the window.
pub const STRIP_MIN_WIDTH: f64 = 120.0;

/// The width the strip and the position plate may share on a bar `bar_width` wide whose caption buttons (and the room
/// the system keeps on the left) take `insets`.
pub fn strip_budget(bar_width: f64, insets: f64, tier: BarTier) -> f64 {
    (bar_width - insets - cluster_width(tier) - BAR_FIXED_WIDTH).max(STRIP_MIN_WIDTH)
}

/// Tabs in most-recently-used order, and the walk through that order while the keys of next/previous tab are
/// held: every press steps through the order as it was before the first press, and the tab shown when the keys
/// come up becomes the most recent.
#[derive(Debug)]
pub struct Mru<T> {
    order: Vec<T>,
    walk: Option<(Vec<T>, usize)>,
}

impl<T: Copy + PartialEq> Default for Mru<T> {
    fn default() -> Self {
        Mru { order: Vec::new(), walk: None }
    }
}

impl<T: Copy + PartialEq> Mru<T> {
    /// `tab` was selected some other way than by the keys: it becomes the most recent and any walk ends.
    pub fn touch(&mut self, tab: T) {
        self.order.retain(|other| *other != tab);
        self.order.insert(0, tab);
        self.walk = None;
    }

    /// A closed tab leaves the order; the walk ends.
    pub fn remove(&mut self, tab: T) {
        self.order.retain(|other| *other != tab);
        self.walk = None;
    }

    pub fn most_recent(&self) -> Option<T> {
        self.order.first().copied()
    }

    /// `closed` was closed while `selected` (if any) was the tab shown. It leaves the order, and the tab to show
    /// next is returned: the selected one stays, and closing the selected one hands over to the tab used most
    /// recently before it, not to a neighbour.
    pub fn close(&mut self, closed: T, selected: Option<T>) -> Option<T> {
        self.remove(closed);
        if selected == Some(closed) { self.most_recent() } else { selected }
    }

    /// One press of next (`step` 1) or previous (`step` -1): the tab to show, or `None` with fewer than two tabs.
    pub fn step(&mut self, step: isize) -> Option<T> {
        if self.order.len() < 2 {
            return None;
        }
        let (snapshot, position) = self.walk.get_or_insert_with(|| (self.order.clone(), 0));
        let count = snapshot.len() as isize;
        *position = (*position as isize + step).rem_euclid(count) as usize;
        Some(snapshot[*position])
    }

    /// The keys came up: the tab shown last becomes the most recent.
    pub fn commit(&mut self) {
        if let Some((snapshot, position)) = self.walk.take() {
            self.touch(snapshot[position]);
        }
    }

    #[cfg(test)]
    pub fn is_walking(&self) -> bool {
        self.walk.is_some()
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

/// The most characters of a working directory a tooltip shows: the whole of any path Windows allows.
pub const MAX_PATH_CHARS: usize = 520;

/// [`clean_title`] with a limit of its own.
pub fn clean_text(raw: &str, max: usize) -> String {
    let kept: String =
        raw.chars().filter(|c| !is_control_or_bidi(*c)).skip_while(|c| c.is_whitespace()).take(max).collect();
    kept.trim_end().to_owned()
}

/// The parts of a line of a tooltip that have something to say, joined by [`TITLE_SEPARATOR`]; none when none has.
pub fn tip_line(parts: &[Option<&str>]) -> Option<String> {
    let said: Vec<&str> = parts.iter().flatten().copied().filter(|part| !part.is_empty()).collect();
    (!said.is_empty()).then(|| said.join(TITLE_SEPARATOR))
}

/// The tooltip of a chip, in at most four lines, each of them left out when it has nothing to say: the whole title (the
/// chip cuts it short), the state when the tab is not at rest, the whole working directory, and the profile with the
/// panes. The command line is not in it: its arguments may hold secrets.
pub fn tab_tooltip(title: &str, state: Option<String>, cwd: Option<&str>, place: Option<String>) -> String {
    [Some(title.to_owned()), state, cwd.map(str::to_owned), place]
        .into_iter()
        .flatten()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The width of the zone at each end of the strip in which a dragged chip makes the strip scroll, in DIP.
pub const DRAG_EDGE: f64 = 24.0;
/// How fast the strip scrolls (DIP a second) at the inner border of that zone, and at the edge of the viewer or
/// beyond it.
pub const DRAG_SCROLL_SLOW: f64 = 200.0;
pub const DRAG_SCROLL_FAST: f64 = 900.0;

/// The scroll speed (DIP a second; negative toward the start of the strip, zero in the middle) for a dragged chip whose
/// pointer is `x` DIP from the left edge of a viewer `viewport` DIP wide. It grows with the depth in the zone, so the
/// strip starts at once and the user can slow it by moving back.
pub fn drag_scroll_speed(x: f64, viewport: f64) -> f64 {
    let ramp =
        |depth: f64| DRAG_SCROLL_SLOW + (DRAG_SCROLL_FAST - DRAG_SCROLL_SLOW) * (depth / DRAG_EDGE).clamp(0.0, 1.0);
    if x < DRAG_EDGE && x <= viewport / 2.0 {
        -ramp(DRAG_EDGE - x)
    } else if x > viewport - DRAG_EDGE {
        ramp(x - (viewport - DRAG_EDGE))
    } else {
        0.0
    }
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

/// What the agent of a pane called its session and itself, kept after its record is gone. Seeing a finished agent
/// dismisses its record, and a tab must not change its name for that; the label ends with the agent, when the shell
/// is at its prompt again or the pane restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLabel {
    /// The session title of the last report that had one, cleaned like any title.
    pub title: Option<String>,
    /// The app id of the last report that had one.
    pub app: Option<String>,
}

impl AgentLabel {
    /// A report was set: what it names replaces the old name; what it leaves out keeps it.
    pub fn remember(&mut self, record: &AgentRecord) {
        if let Some(title) = record.title.as_deref().map(clean_title).filter(|title| !title.is_empty()) {
            self.title = Some(title);
        }
        if let Some(app) = &record.app {
            self.app = Some(app.clone());
        }
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

/// For every tab whose title says its folder (`Some(title)`), whether another such tab has the same title, so it needs
/// the parent folder to be told apart. Titles compare without case.
pub fn colliding_folders(labels: &[Option<String>]) -> Vec<bool> {
    labels
        .iter()
        .map(|label| {
            label.as_deref().is_some_and(|label| {
                labels.iter().flatten().filter(|other| other.eq_ignore_ascii_case(label)).count() > 1
            })
        })
        .collect()
}

/// The entries of a list for which `add` put a row into it, in order. `add` leaves the list as it was when it
/// returns false, so the row at an index of the list is always the row of the entry at that index: what Enter picks
/// is what the row says, however many rows could not be built.
pub fn keep_built<T>(entries: Vec<T>, mut add: impl FnMut(&T) -> bool) -> Vec<T> {
    entries.into_iter().filter(|entry| add(entry)).collect()
}

/// The background opacity the sliders and commands asked for, kept until the next frame: a drag raises many value
/// changes per frame and only the last of them is worth applying.
#[derive(Debug, Default)]
pub struct PendingOpacity(Option<f64>);

impl PendingOpacity {
    /// Records a request, rounded to a whole percent and kept inside the valid range.
    pub fn request(&mut self, opacity: f64) {
        self.0 = Some(((opacity * 100.0).round() / 100.0).clamp(OPACITY_RANGE.0, OPACITY_RANGE.1));
    }

    /// What the controls show: the latest request, or `applied` when nothing waits.
    pub fn latest(&self, applied: f64) -> f64 {
        self.0.unwrap_or(applied)
    }

    /// The opacity to apply in this frame: the request, unless it is what `applied` already is.
    pub fn take(&mut self, applied: f64) -> Option<f64> {
        self.0.take().filter(|opacity| (opacity - applied).abs() > 0.001)
    }
}

/// Whether the opacity of the background could be put in effect. A failure while the slider moves is only logged (the
/// next change applies again); one that is still there when the slider has rested (the settings are about to be
/// written) is shown in the row of the opacity, until the next change.
#[derive(Debug, Default)]
pub struct OpacityApply {
    failed: bool,
    settled: bool,
}

impl OpacityApply {
    /// The latest opacity is in effect.
    pub fn applied(&mut self) {
        *self = OpacityApply::default();
    }

    /// Applying the latest opacity failed.
    pub fn failed(&mut self) {
        *self = OpacityApply { failed: true, settled: false };
    }

    /// The slider (or a command) asked for another opacity: whatever the row said is out of date.
    pub fn requested(&mut self) {
        self.settled = false;
    }

    /// The slider has rested: a failure that is still there is now worth saying.
    pub fn rested(&mut self) {
        self.settled = self.failed;
    }

    /// The row of the opacity says `Not applied` instead of the percent.
    pub fn shows_label(&self) -> bool {
        self.failed && self.settled
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
        kitty_keyboard: settings.keyboard.kitty_protocol,
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
    /// The size, the family or the line height of the font.
    pub font: bool,
    pub osc52: bool,
    /// The shape of the cursor a program that chose none gets, and how many lines of scrollback a pane keeps.
    pub cursor: bool,
    pub scrollback: bool,
    /// How the title of a tab is made.
    pub titles: bool,
}

/// What `now` changes of the screen compared with `before`. The opacity is not part of it: it has its own way.
pub fn effects(before: &Settings, now: &Settings) -> Effects {
    Effects {
        glass: before.theme != now.theme || before.appearance.backdrop != now.appearance.backdrop,
        font: before.font.size != now.font.size
            || before.font.family != now.font.family
            || before.font.line_height != now.font.line_height,
        osc52: before.osc52 != now.osc52,
        cursor: before.cursor != now.cursor,
        scrollback: before.scrollback != now.scrollback,
        titles: before.tabs != now.tabs,
    }
}

/// Width of the drawer, the narrowest it gets and the space between it and the edges of the window, in DIP.
pub const DRAWER_WIDTH: f64 = 480.0;
pub const DRAWER_MIN_WIDTH: f64 = 320.0;
pub const DRAWER_MARGIN: f64 = 8.0;

/// The width of the drawer in a body of `body_width` DIP: 480, less when the window is narrower than that plus its
/// margins, never under 320.
pub fn drawer_width(body_width: f64) -> f64 {
    (body_width - 2.0 * DRAWER_MARGIN).clamp(DRAWER_MIN_WIDTH, DRAWER_WIDTH)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// The item a navigation key moves to, from `current` among `count` items laid out `columns` to a row (a row of
/// choices is `columns == count`, a list is 1). `None` when the key leads nowhere: the first item and Left, the last
/// row and Down.
pub fn roving_target(current: usize, count: usize, columns: usize, key: Arrow) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let columns = columns.clamp(1, count);
    let target = match key {
        Arrow::Left => current.checked_sub(1)?,
        Arrow::Right => current + 1,
        Arrow::Up => current.checked_sub(columns)?,
        Arrow::Down => current + columns,
        Arrow::Home => 0,
        Arrow::End => count - 1,
    };
    (target < count && target != current).then_some(target)
}

/// Which colour scheme tile the user points at: the one under the pointer or the one with the keyboard focus,
/// whichever of the two happened last. The scheme is previewed while there is one and put back when there is none.
#[derive(Debug, Default)]
pub struct Peek {
    hover: Option<usize>,
    focus: Option<usize>,
    hover_latest: bool,
}

impl Peek {
    pub fn hovered(&mut self, tile: Option<usize>) {
        self.hover = tile;
        self.hover_latest |= tile.is_some();
    }

    pub fn focused(&mut self, tile: Option<usize>) {
        self.focus = tile;
        if tile.is_some() {
            self.hover_latest = false;
        }
    }

    pub fn tile(&self) -> Option<usize> {
        if self.hover_latest { self.hover.or(self.focus) } else { self.focus.or(self.hover) }
    }

    pub fn clear(&mut self) {
        *self = Peek::default();
    }
}

/// A number as a row shows it: whole numbers without a fraction, the others with at most two decimals.
pub fn number_text(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 { format!("{rounded:.0}") } else { format!("{rounded:.2}").trim_end_matches('0').to_owned() }
}

/// The number typed into a field; `None` when the text is not one.
pub fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// `value` moved by `steps` steps of `range`, kept inside it and free of floating point drift.
pub fn step_number(value: f64, steps: i32, range: Range) -> f64 {
    let moved = ((value + f64::from(steps) * range.step) * 1e6).round() / 1e6;
    moved.clamp(range.min, range.max)
}

#[cfg(test)]
mod tests {
    use nekominal_model::layout::{Axis, Node, UNIT};
    use nekominal_model::settings::TabTitle;

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
    #[test]
    fn the_tooltip_has_four_lines_at_most_and_leaves_out_the_ones_with_nothing_to_say() {
        let full = tab_tooltip(
            "build the thing",
            tip_line(&[Some("Waiting for input"), Some("40%"), Some("Allow edit?")]),
            Some(r"C:\work\nekominal"),
            tip_line(&[Some("PowerShell"), Some("3 panes"), Some("Zoomed")]),
        );
        assert_eq!(
            full.lines().collect::<Vec<_>>(),
            [
                "build the thing",
                "Waiting for input · 40% · Allow edit?",
                r"C:\work\nekominal",
                "PowerShell · 3 panes · Zoomed"
            ]
        );
        // A tab at rest has no state line, and a tab that never reported a folder has no folder line.
        assert_eq!(tab_tooltip("t", None, None, tip_line(&[Some("PowerShell"), None, None])), "t\nPowerShell");
        assert_eq!(tab_tooltip("t", None, Some(""), None), "t");
        assert_eq!(tip_line(&[None, Some(""), None]), None);
        assert_eq!(tip_line(&[Some("a"), None, Some("b")]).as_deref(), Some("a · b"));
        // What a program sent never adds a line: a break in it is cleaned out before it gets here.
        let sent = clean_text("C:\\a\nb\u{202e}\r\nc", MAX_PATH_CHARS);
        assert_eq!(tab_tooltip("t", None, Some(&sent), None).lines().count(), 2);
        assert_eq!(clean_text(&"x".repeat(2000), MAX_PATH_CHARS).chars().count(), MAX_PATH_CHARS);
        assert_eq!(clean_title("  a\u{7}b "), clean_text("  a\u{7}b ", MAX_TITLE_CHARS));
    }

    #[test]
    fn a_dragged_chip_scrolls_the_strip_from_the_edge_zones_and_only_there() {
        let viewport = 440.0;
        // Nothing in the middle, and not on the very border of a zone.
        for x in [DRAG_EDGE, 100.0, 220.0, viewport - DRAG_EDGE] {
            assert_eq!(drag_scroll_speed(x, viewport), 0.0, "{x}");
        }
        // Toward the start at the left, toward the end at the right, as soon as the pointer is in the zone.
        assert!(drag_scroll_speed(DRAG_EDGE - 0.1, viewport) <= -DRAG_SCROLL_SLOW);
        assert!(drag_scroll_speed(viewport - DRAG_EDGE + 0.1, viewport) >= DRAG_SCROLL_SLOW);
        // Faster the deeper, and as fast as it gets at the edge and beyond; the two ends are alike.
        assert!(drag_scroll_speed(2.0, viewport) < drag_scroll_speed(14.0, viewport));
        assert_eq!(drag_scroll_speed(0.0, viewport), -DRAG_SCROLL_FAST);
        assert_eq!(drag_scroll_speed(-300.0, viewport), -DRAG_SCROLL_FAST);
        assert_eq!(drag_scroll_speed(viewport, viewport), DRAG_SCROLL_FAST);
        assert_eq!(drag_scroll_speed(viewport + 300.0, viewport), DRAG_SCROLL_FAST);
        for x in [0.0, 5.0, 12.0, 23.0] {
            assert_eq!(drag_scroll_speed(x, viewport), -drag_scroll_speed(viewport - x, viewport), "{x}");
        }
        // A viewer too narrow for two zones scrolls toward the nearer end.
        assert!(drag_scroll_speed(10.0, 40.0) < 0.0 && drag_scroll_speed(30.0, 40.0) > 0.0);
        // The strip crosses 22 chips of 52 DIP in under two seconds at the edge, and in under six at the border of
        // the zone: the drag of the study (tab 3 to the 25th place among 30) fits its three seconds.
        let distance = 22.0 * (CHIP_MIN_WIDTH + 4.0);
        assert!(distance / DRAG_SCROLL_FAST < 2.0 && distance / DRAG_SCROLL_SLOW < 6.0);
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
    fn a_pane_needs_a_response_for_a_bell_a_notification_an_exit_or_an_agent_that_waits_or_failed() {
        assert!(!needs_response(&PaneSignal::default()));
        for kind in [Attention::Bell, Attention::Notification, Attention::Exit] {
            assert!(needs_response(&PaneSignal { attention: Some(kind), ..PaneSignal::default() }), "{kind:?}");
        }
        assert!(needs_response(&agent(AgentState::Blocked)));
        assert!(needs_response(&agent(AgentState::Error)));
        for state in [AgentState::Working, AgentState::Done, AgentState::Idle] {
            assert!(!needs_response(&agent(state)), "{state:?}");
        }
        assert!(!needs_response(&PaneSignal { progress: Some(Reported::Percent(10)), ..PaneSignal::default() }));
    }

    #[test]
    fn a_zoom_warns_about_the_hidden_panes_that_wait_and_only_those() {
        let bell = PaneSignal { attention: Some(Attention::Bell), ..PaneSignal::default() };
        let quiet = PaneSignal::default();
        // Nothing is zoomed: the warning lines of the panes say it, not the label.
        assert_eq!(hidden_alert(&[quiet.clone(), bell.clone()], None), None);
        // The pane that rang is hidden: the label warns with the state of a bell.
        let alert = hidden_alert(&[quiet.clone(), bell.clone(), quiet.clone()], Some(0)).expect("a hidden pane rang");
        assert_eq!((alert.state, alert.attention), (TabState::Attention, Some(Attention::Bell)));
        // The pane that rang is the zoomed one: it is on screen, nothing is hidden from the user.
        assert_eq!(hidden_alert(&[quiet.clone(), bell.clone()], Some(1)), None);
        // A hidden pane that only works, or finished, does not wait for anybody.
        assert_eq!(hidden_alert(&[quiet.clone(), agent(AgentState::Working), agent(AgentState::Done)], Some(0)), None);
        // Of several that wait the most urgent state shows: a blocked agent over a bell.
        let both = hidden_alert(&[quiet.clone(), bell, agent(AgentState::Blocked)], Some(0)).expect("two wait");
        assert_eq!(both.state, TabState::Blocked);
        assert_eq!(hidden_alert(&[], Some(0)), None);
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

    fn waiting(alert: Option<AlertKind>) -> Need {
        Need { waiting: true, alert }
    }

    const QUIET: Need = Need { waiting: false, alert: None };

    #[test]
    fn a_response_that_begins_out_of_sight_is_unread_and_raises_one_alert() {
        let mut watch = Watch::default();
        assert_eq!(watch.observe(QUIET, false, 0), None);
        assert!(!watch.unread() && !watch.alerted());
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), false, 0), Some(AlertKind::Blocked));
        assert!(watch.unread() && watch.alerted());
        // The same need seen again raises nothing more and stays unread.
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), false, 0), None);
        assert!(watch.unread());
        // A tab that is selected in the active window is watched: viewing it clears the flag and frees the alert.
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), true, 0), None);
        assert!(!watch.unread() && !watch.alerted());
    }

    #[test]
    fn leaving_a_tab_whose_response_was_seen_does_not_make_it_unread_again() {
        let mut watch = Watch::default();
        watch.observe(waiting(Some(AlertKind::Blocked)), false, 0);
        watch.observe(waiting(Some(AlertKind::Blocked)), true, 0);
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), false, 0), None);
        assert!(!watch.unread(), "it still waits, but the user has seen it");
        // A new need after the old one ended is unread again.
        watch.observe(QUIET, false, 0);
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), false, 0), Some(AlertKind::Blocked));
        assert!(watch.unread());
    }

    #[test]
    fn the_need_ending_clears_the_flag_without_anybody_looking() {
        let mut watch = Watch::default();
        watch.observe(waiting(Some(AlertKind::Error)), false, 0);
        assert!(watch.unread());
        watch.observe(QUIET, false, 0);
        assert!(!watch.unread() && !watch.alerted());
    }

    #[test]
    fn a_more_urgent_kind_is_a_new_need_and_a_bell_is_unread_without_an_alert() {
        let mut watch = Watch::default();
        // A bell: unread, but it has its own settings and raises no alert here.
        assert_eq!(watch.observe(waiting(None), false, 0), None);
        assert!(watch.unread() && !watch.alerted());
        // The same tab then blocks: that raises one.
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), false, 0), Some(AlertKind::Blocked));
        // Blocked turning into an error is announced again; an error staying one is not.
        assert_eq!(watch.observe(waiting(Some(AlertKind::Error)), false, 0), Some(AlertKind::Error));
        assert_eq!(watch.observe(waiting(Some(AlertKind::Error)), false, 0), None);
    }

    #[test]
    fn at_most_two_alerts_are_outstanding_and_the_third_tab_is_unread_all_the_same() {
        let mut third = Watch::default();
        assert_eq!(third.observe(waiting(Some(AlertKind::Blocked)), false, MAX_ALERTS), None);
        assert!(third.unread(), "it counts for the plate and the list");
        assert!(!third.alerted(), "but it holds no alert");
        // A slot freed later does not announce what was already waiting.
        assert_eq!(third.observe(waiting(Some(AlertKind::Blocked)), false, 0), None);
        let mut second = Watch::default();
        assert_eq!(second.observe(waiting(Some(AlertKind::Blocked)), false, MAX_ALERTS - 1), Some(AlertKind::Blocked));
    }

    #[test]
    fn a_need_in_the_window_the_user_is_looking_at_raises_nothing() {
        let mut watch = Watch::default();
        assert_eq!(watch.observe(waiting(Some(AlertKind::Blocked)), true, 0), None);
        assert!(!watch.unread() && !watch.alerted());
    }

    #[test]
    fn alerts_are_raised_for_blocked_failed_and_notified_tabs_but_not_for_a_bell() {
        let of = |panes: &[PaneSignal]| alert_kind(&tab_status(panes));
        assert_eq!(of(&[agent(AgentState::Blocked)]), Some(AlertKind::Blocked));
        assert_eq!(of(&[agent(AgentState::Error)]), Some(AlertKind::Error));
        let exit = PaneSignal { attention: Some(Attention::Exit), ..PaneSignal::default() };
        assert_eq!(of(&[exit]), Some(AlertKind::Error));
        let note = PaneSignal { attention: Some(Attention::Notification), ..PaneSignal::default() };
        assert_eq!(of(&[note]), Some(AlertKind::Notification));
        let bell = PaneSignal { attention: Some(Attention::Bell), ..PaneSignal::default() };
        assert_eq!(of(&[bell.clone()]), None);
        assert_eq!(of(&[agent(AgentState::Working), agent(AgentState::Done)]), None);
        // A tab needs a response for a bell, though.
        let status = tab_status(&[bell.clone()]);
        assert_eq!(Need::of(&[bell], &status), Need { waiting: true, alert: None });
        assert_eq!(Need::of(&[agent(AgentState::Working)], &TabStatus::default()), QUIET);
    }

    fn attention(state: TabState, unread: bool, selected: bool) -> TabAttention {
        TabAttention {
            state,
            unread,
            selected,
            waiting: matches!(state, TabState::Blocked | TabState::Error | TabState::Attention),
        }
    }

    #[test]
    fn the_plate_counts_unread_blocked_and_failed_tabs_that_are_not_selected() {
        let tabs = [
            attention(TabState::Blocked, true, false),
            attention(TabState::Error, true, false),
            attention(TabState::Blocked, true, true),
            attention(TabState::Blocked, false, false),
            attention(TabState::Attention, true, false),
            attention(TabState::Done, true, false),
            attention(TabState::Working, true, false),
            attention(TabState::Idle, false, false),
        ];
        assert_eq!(plate_count(&tabs), 2, "a blocked one that is selected, seen, a bell and the rest are not counted");
        assert_eq!(plate_count(&[]), 0);
        // The window title counts everything that waits, the selected tab and the seen ones included.
        assert_eq!(waiting_count(&tabs), 5);
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
    fn the_pane_that_waits_is_named_by_its_place_among_several() {
        assert_eq!(waiting_pane(&[]), None);
        assert_eq!(waiting_pane(&[true]), None, "a single pane needs no place");
        assert_eq!(waiting_pane(&[false, true, false]), Some((2, 3)));
        assert_eq!(waiting_pane(&[true, true]), Some((1, 2)), "the first of those that wait");
        assert_eq!(waiting_pane(&[false, false]), None);
    }

    #[test]
    fn the_list_groups_tabs_by_state_most_urgent_first_when_one_is_unread_and_keeps_the_strip_order_otherwise() {
        let tabs = [
            (TabState::Idle, None),
            (TabState::Done, None),
            (TabState::Blocked, None),
            (TabState::Working, None),
            (TabState::Error, None),
            (TabState::Blocked, None),
            (TabState::Attention, Some(Attention::Bell)),
            (TabState::Attention, Some(Attention::Notification)),
        ];
        let plain = list_order(&tabs, false);
        assert_eq!(plain.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(plain.iter().all(|(_, heading)| heading.is_none()));

        let grouped = list_order(&tabs, true);
        let order: Vec<usize> = grouped.iter().map(|(index, _)| *index).collect();
        // Blocked (2, 5), error (4), working (3), done (1), a notification (7), a bell (6), the rest (0).
        assert_eq!(order, [2, 5, 4, 3, 1, 7, 6, 0]);
        let headings: Vec<(usize, TabState)> =
            grouped.iter().filter_map(|(index, heading)| heading.map(|heading| (*index, heading.state))).collect();
        assert_eq!(
            headings,
            [
                (2, TabState::Blocked),
                (4, TabState::Error),
                (3, TabState::Working),
                (1, TabState::Done),
                (7, TabState::Attention),
                (6, TabState::Attention),
                (0, TabState::Idle)
            ]
        );
        let notification = grouped[5].1.unwrap();
        assert_eq!(notification.attention, Some(Attention::Notification));
        assert_eq!(grouped[6].1.unwrap().attention, Some(Attention::Bell));
        assert_eq!(list_order(&[], true), Vec::new());
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
    fn a_chip_narrower_than_64_dip_shows_its_number() {
        assert!(shows_ordinal(48.0) && shows_ordinal(63.9));
        assert!(!shows_ordinal(64.0) && !shows_ordinal(216.0));
    }

    #[test]
    fn hidden_and_visible_chips_add_up_to_all_of_them() {
        let widths = [100.0; 12];
        let extent = strip_extent(&widths, GAP, 4.0);
        assert_eq!(extent, 12.0 * 100.0 + 11.0 * GAP + 4.0);
        let viewport = 500.0;
        for offset in [0.0, 1.0, 104.0, 300.0, 520.0, extent - viewport] {
            let (left, right) = hidden_counts(&widths, GAP, 4.0, offset, viewport);
            let visible = widths
                .iter()
                .enumerate()
                .filter(|(i, w)| {
                    let edge = 2.0 + *i as f64 * (100.0 + GAP);
                    edge + **w > offset && edge < offset + viewport
                })
                .count();
            assert_eq!(left + visible + right, widths.len(), "offset {offset}");
        }
        assert_eq!(hidden_counts(&widths, GAP, 4.0, 0.0, viewport), (0, 7));
        assert_eq!(hidden_counts(&widths, GAP, 4.0, extent - viewport, viewport), (7, 0));
        // A chip cut by the edge still shows.
        assert_eq!(hidden_counts(&widths, GAP, 4.0, 50.0, viewport).0, 0);
    }

    #[test]
    fn revealing_a_chip_leaves_a_margin_and_makes_room_for_the_count_blocks() {
        let (viewport, extent) = (500.0, 1304.0);
        let reveal =
            |left, offset| reveal_offset((left, 100.0), offset, viewport, extent, REVEAL_MARGIN, COUNT_BLOCK_WIDTH);
        // Already in view: nothing moves.
        assert_eq!(reveal(200.0, 0.0), 0.0);
        assert_eq!(reveal(300.0, 100.0), 100.0);
        // The first chip of the strip stays flush at the left edge.
        assert_eq!(reveal(2.0, 300.0), 0.0);
        // A chip to the right scrolls in with the margin and the block behind it.
        assert_eq!(reveal(600.0, 0.0), 600.0 + 100.0 + REVEAL_MARGIN + COUNT_BLOCK_WIDTH - 500.0);
        // The last chip scrolls to the end, with the margin only.
        assert_eq!(reveal(1202.0, 0.0), extent - viewport);
        // A chip to the left of the view scrolls in from the left.
        assert_eq!(reveal(300.0, 700.0), 300.0 - REVEAL_MARGIN - COUNT_BLOCK_WIDTH);
        // A strip that fits has nothing to scroll.
        assert_eq!(reveal_offset((300.0, 100.0), 0.0, 500.0, 400.0, REVEAL_MARGIN, COUNT_BLOCK_WIDTH), 0.0);
    }

    #[test]
    fn a_revealed_chip_is_in_view_inside_its_margins_for_any_strip() {
        let mut random = Lcg(11);
        for _ in 0..3000 {
            let count = 2 + random.below(40);
            let widths: Vec<f64> = (0..count).map(|_| random.between(CHIP_MIN_WIDTH, CHIP_MAX_WIDTH)).collect();
            let extent = strip_extent(&widths, GAP, 4.0);
            // Wide enough for the widest chip between two count blocks and their margins.
            let viewport = random.between(400.0, 900.0);
            let index = random.below(count);
            let left = 2.0 + widths[..index].iter().sum::<f64>() + GAP * index as f64;
            let offset = random.between(0.0, (extent - viewport).max(0.0) + 50.0);
            let at = reveal_offset((left, widths[index]), offset, viewport, extent, REVEAL_MARGIN, COUNT_BLOCK_WIDTH);
            assert!(at >= 0.0 && at <= (extent - viewport).max(0.0) + 1e-9, "offset {at}");
            assert!(left >= at - 1e-9 && left + widths[index] <= at + viewport + 1e-9, "the chip is cut off");
            if extent > viewport {
                // Every scroll position has the chip clear of the blocks, unless it sits at an end of the strip.
                let margin_left = if at > 0.0 { REVEAL_MARGIN + COUNT_BLOCK_WIDTH } else { 0.0 };
                assert!(left >= at + margin_left - 1e-9 || index == 0, "left margin at {at}");
                let at_end = at + viewport >= extent - 1e-9;
                let margin_right = if at_end { 0.0 } else { REVEAL_MARGIN + COUNT_BLOCK_WIDTH };
                assert!(
                    left + widths[index] <= at + viewport - margin_right + 1e-9 || index == count - 1,
                    "right margin"
                );
            }
        }
    }

    #[test]
    fn the_plate_is_hidden_for_one_tab_and_total_only_on_a_narrow_strip_or_in_the_narrow_tier() {
        use BarTier::*;
        assert_eq!(plate(0, 0, 800.0, Wide), None);
        assert_eq!(plate(0, 1, 800.0, Wide), None);
        assert_eq!(plate(2, 12, 800.0, Wide), Some(Plate::Position { current: 3, total: 12 }));
        assert_eq!(plate(1, 2, 300.0, Medium), Some(Plate::Position { current: 2, total: 2 }));
        assert_eq!(plate(2, 12, 299.9, Medium), Some(Plate::Total(12)));
        // The narrow tier says the total whatever the strip: the bar has no room for more.
        assert_eq!(plate(2, 12, 455.0, Narrow), Some(Plate::Total(12)));
        assert_eq!(plate(0, 1, 455.0, Narrow), None);
    }

    #[test]
    fn the_bar_shows_four_two_or_one_control_on_the_right_at_the_widths_of_the_spec() {
        for (width, controls) in [(1440.0, 4), (1200.0, 4), (1199.0, 2), (1000.0, 2), (800.0, 2), (799.0, 1), (700.0, 1), (500.0, 1)] {
            assert_eq!(BarTier::at(width, None).control_count(), controls, "{width} DIP");
        }
        use BarTier::*;
        assert_eq!(BarTier::at(1440.0, None), Wide);
        assert_eq!(BarTier::at(1000.0, None), Medium);
        assert_eq!(BarTier::at(700.0, None), Narrow);
        // What each tier shows, and the divider only where the split pair and the rest are both there.
        let shown = |tier: BarTier| TOOLBAR_TIERS.iter().filter(|(c, _)| tier.shows(*c)).map(|(c, _)| c.id()).collect::<Vec<_>>();
        assert_eq!(shown(Wide), ["pane.splitRight", "pane.splitDown", "app.palette"]);
        assert_eq!(shown(Medium), ["app.palette"]);
        assert_eq!(shown(Narrow), Vec::<&str>::new());
        assert!(Wide.shows_divider() && !Medium.shows_divider() && !Narrow.shows_divider());
        assert!(!Wide.shows(Command::EditFind) && !Wide.shows(Command::AppSettings), "find and settings have no button");
    }

    #[test]
    fn the_tiers_of_the_bar_are_the_toolbar_of_the_model_in_its_order() {
        let commands: Vec<Command> = TOOLBAR_TIERS.iter().map(|(command, _)| *command).collect();
        assert_eq!(commands, nekominal_model::menu::TOOLBAR);
        // The split pair folds together, and before the palette.
        assert_eq!(TOOLBAR_TIERS[0].1, TOOLBAR_TIERS[1].1);
        assert!(TOOLBAR_TIERS[0].1 > TOOLBAR_TIERS[2].1);
    }

    #[test]
    fn a_tier_boundary_never_goes_the_wrong_way_and_a_shrinking_window_keeps_its_tier_for_24_dip() {
        // Growing: every width gets the tier it starts at, at once, and a wider window never has less.
        let mut previous = None;
        let mut last = BarTier::Narrow;
        for tenth in 0..=30_000 {
            let width = f64::from(tenth) / 10.0;
            let tier = BarTier::at(width, previous);
            assert!(tier >= last, "{width} DIP has {tier:?} after {last:?}");
            assert_eq!(tier, BarTier::at(width, None), "growing needs no memory: {width}");
            (previous, last) = (Some(tier), tier);
        }
        // Shrinking: a narrower window never has more, and a tier is kept until 24 DIP below where it starts.
        let mut previous = Some(BarTier::Wide);
        let mut last = BarTier::Wide;
        let mut switches = Vec::new();
        for tenth in (0..=30_000).rev() {
            let width = f64::from(tenth) / 10.0;
            let tier = BarTier::at(width, previous);
            assert!(tier <= last, "{width} DIP has {tier:?} after {last:?}");
            if tier != last {
                switches.push((width, tier));
            }
            (previous, last) = (Some(tier), tier);
        }
        assert_eq!(switches, [(1175.9, BarTier::Medium), (775.9, BarTier::Narrow)]);
        // The edges of the margin, from either side.
        assert_eq!(BarTier::at(1176.0, Some(BarTier::Wide)), BarTier::Wide);
        assert_eq!(BarTier::at(1175.9, Some(BarTier::Wide)), BarTier::Medium);
        assert_eq!(BarTier::at(1199.9, Some(BarTier::Medium)), BarTier::Medium, "growing waits for the boundary");
        assert_eq!(BarTier::at(1200.0, Some(BarTier::Medium)), BarTier::Wide);
        assert_eq!(BarTier::at(776.0, Some(BarTier::Medium)), BarTier::Medium);
        assert_eq!(BarTier::at(775.9, Some(BarTier::Medium)), BarTier::Narrow);
        assert_eq!(BarTier::at(790.0, Some(BarTier::Narrow)), BarTier::Narrow, "a window that grew from narrow has no margin");
        // A jump over a whole tier lands in the tier of its width, held at most one margin.
        assert_eq!(BarTier::at(700.0, Some(BarTier::Wide)), BarTier::Narrow);
        assert_eq!(BarTier::at(790.0, Some(BarTier::Wide)), BarTier::Medium);
    }

    #[test]
    fn the_group_on_the_right_gets_narrower_with_every_tier() {
        use BarTier::*;
        assert!(cluster_width(Wide) > cluster_width(Medium) && cluster_width(Medium) > cluster_width(Narrow));
        // 4 padding, 32 per control, and the divider of the wide tier with its margins.
        assert_eq!((cluster_width(Wide), cluster_width(Medium), cluster_width(Narrow)), (139.0, 68.0, 36.0));
    }

    #[test]
    fn at_500_dip_the_narrow_bar_leaves_the_strip_its_minimum_and_a_plate_and_the_drag_area_theirs() {
        // 138 DIP of caption buttons. What is left for the strip and the plate: the window minus the caption buttons,
        // the single control on the right and everything else that is fixed (including the smallest drag area).
        let budget = strip_budget(500.0, 138.0, BarTier::at(500.0, None));
        assert_eq!(budget, 500.0 - 138.0 - 36.0 - BAR_FIXED_WIDTH);
        // The plate of the narrow tier is the total in a button of the minimum width (32) with a 4 DIP margin.
        assert!(budget >= STRIP_MIN_WIDTH + 36.0 + 4.0, "{budget}");
        // The more room, the more strip; and never less than the minimum.
        assert!(strip_budget(1000.0, 138.0, BarTier::Medium) > strip_budget(900.0, 138.0, BarTier::Medium));
        assert_eq!(strip_budget(100.0, 138.0, BarTier::Narrow), STRIP_MIN_WIDTH);
        // A wider tier takes room from the strip at the same width.
        assert!(strip_budget(1300.0, 138.0, BarTier::Medium) > strip_budget(1300.0, 138.0, BarTier::Wide));
    }

    #[test]
    fn next_tab_walks_the_most_recently_used_order_while_the_keys_are_held() {
        let mut mru = Mru::default();
        for tab in ['a', 'b', 'c'] {
            mru.touch(tab);
        }
        // Select A, B, C in turn: the order is C, B, A. One tap goes to B.
        assert_eq!(mru.step(1), Some('b'));
        mru.commit();
        assert!(!mru.is_walking());
        // Held: two more presses go to C and then A, and A is the most recent when the keys come up.
        assert_eq!(mru.step(1), Some('c'));
        assert_eq!(mru.step(1), Some('a'));
        mru.commit();
        assert_eq!(mru.most_recent(), Some('a'));
        // The order after that: A, B, C (B was used after C).
        assert_eq!(mru.step(1), Some('b'));
        assert_eq!(mru.step(1), Some('c'));
        assert_eq!(mru.step(1), Some('a'), "the walk wraps around");
        mru.commit();
        assert_eq!(mru.most_recent(), Some('a'));
    }

    #[test]
    fn previous_tab_walks_backwards_and_a_tap_toggles_between_two_tabs() {
        let mut mru = Mru::default();
        for tab in [1, 2, 3, 4] {
            mru.touch(tab);
        }
        assert_eq!(mru.step(-1), Some(1), "previous starts from the least recently used");
        mru.commit();
        assert_eq!(mru.most_recent(), Some(1));
        // Tapping twice with the keys released between returns to where it began.
        let mut pair = Mru::default();
        pair.touch('x');
        pair.touch('y');
        assert_eq!(pair.step(1), Some('x'));
        pair.commit();
        assert_eq!(pair.step(1), Some('y'));
        pair.commit();
        assert_eq!(pair.most_recent(), Some('y'));
    }

    #[test]
    fn selecting_a_tab_another_way_ends_the_walk_and_a_closed_tab_leaves_the_order() {
        let mut mru = Mru::default();
        for tab in [1, 2, 3] {
            mru.touch(tab);
        }
        assert_eq!(mru.step(1), Some(2));
        mru.touch(1);
        assert!(!mru.is_walking());
        assert_eq!(mru.most_recent(), Some(1));
        // Closing the most recent tab leaves the one used before it.
        mru.remove(1);
        assert_eq!(mru.most_recent(), Some(3));
        mru.remove(2);
        mru.remove(3);
        assert_eq!(mru.most_recent(), None);
        assert_eq!(mru.step(1), None);
        let mut single = Mru::default();
        single.touch(9);
        assert_eq!(single.step(1), None, "nothing to walk to");
    }

    #[test]
    fn closing_the_selected_tab_shows_the_one_used_before_it_and_closing_another_keeps_the_selection() {
        // Four tabs in strip order a b c d, used in the order a, d, b, c, d, b: b is selected, d was used before it.
        let used = || {
            let mut mru = Mru::default();
            for tab in ['a', 'd', 'b', 'c', 'd', 'b'] {
                mru.touch(tab);
            }
            mru
        };
        let mut mru = used();
        // Closing the selected tab hands over to the one used before it, not to its neighbour in the strip (c).
        let mut closing = used();
        assert_eq!(closing.close('b', Some('b')), Some('d'));
        assert_eq!(closing.most_recent(), Some('d'));
        // Closing a tab that is not selected leaves the selection and takes the closed one out of the order.
        assert_eq!(mru.close('c', Some('b')), Some('b'));
        assert_eq!(mru.close('a', Some('b')), Some('b'));
        assert_eq!(mru.close('b', Some('b')), Some('d'), "c and a are gone: d is the last one used before b");
        assert_eq!(mru.close('d', Some('d')), None, "the last tab leaves nothing to show");
        // A tab that was never selected is not an error, and no selection stays none.
        assert_eq!(Mru::<char>::default().close('x', None), None);
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
        // Tabs whose whole titles are the same are told apart, those that only share a folder are not.
        let same = ["api · build".to_owned(), "api · build".to_owned(), "api · logs".to_owned()];
        let labels: Vec<Option<String>> = same.iter().cloned().map(Some).collect();
        assert_eq!(colliding_folders(&labels), [true, true, false]);
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
    fn the_label_of_an_agent_outlives_its_record_and_follows_the_latest_report() {
        let report = |title: Option<&str>, app: Option<&str>| AgentRecord {
            id: String::new(),
            state: AgentState::Done,
            kind: None,
            progress: None,
            app: app.map(Into::into),
            title: title.map(Into::into),
            message: None,
        };
        let mut label = AgentLabel::default();
        label.remember(&report(Some("Fix the build"), Some("claude")));
        assert_eq!((label.title.as_deref(), label.app.as_deref()), (Some("Fix the build"), Some("claude")));
        // A report that names neither leaves both: the record that carried them may be gone by now.
        label.remember(&report(None, None));
        assert_eq!((label.title.as_deref(), label.app.as_deref()), (Some("Fix the build"), Some("claude")));
        label.remember(&report(Some("  Write\u{202e}the docs "), None));
        assert_eq!((label.title.as_deref(), label.app.as_deref()), (Some("Writethe docs"), Some("claude")));
        // A title of nothing visible is no title.
        label.remember(&report(Some("\u{7}  "), None));
        assert_eq!(label.title.as_deref(), Some("Writethe docs"));
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
    fn five_tabs_in_different_folders_have_five_names_and_the_same_name_gets_a_parent() {
        let folders = [r"C:\Users\me", r"C:\work\api", r"C:\work\web", r"D:\other", r"C:\Users\me\src\api"];
        let names: Vec<String> = folders.iter().map(|f| titled(&parts("pwsh", Some(f)), false).0).collect();
        assert_eq!(names, ["~", "api", "web", "other", "api"]);
        let labels: Vec<Option<String>> = names.iter().cloned().map(Some).collect();
        let collide = colliding_folders(&labels);
        assert_eq!(collide, [false, true, false, false, true]);
        let told_apart: Vec<String> = folders
            .iter()
            .zip(&collide)
            .map(|(f, with_parent)| titled(&parts("pwsh", Some(f)), *with_parent).0)
            .collect();
        assert_eq!(told_apart, ["~", "work/api", "web", "other", "src/api"]);
        // Tabs that are not named after a folder never collide, and the comparison ignores case.
        assert_eq!(
            colliding_folders(&[None, Some("Api".into()), None, Some("api".into())]),
            [false, true, false, true]
        );
        assert_eq!(colliding_folders(&[]), Vec::<bool>::new());
    }

    #[test]
    fn a_row_that_cannot_be_built_leaves_the_rows_and_the_entries_of_a_list_in_step() {
        let entries = vec!["build", "{0}", "logs", "bad\u{fffe}", "claude"];
        let mut rows: Vec<String> = Vec::new();
        let kept = keep_built(entries, |entry| {
            if entry.starts_with('{') || entry.contains('\u{fffe}') {
                return false;
            }
            rows.push(format!("row of {entry}"));
            true
        });
        assert_eq!(kept, ["build", "logs", "claude"]);
        assert_eq!(rows.len(), kept.len());
        for (row, entry) in rows.iter().zip(&kept) {
            assert_eq!(*row, format!("row of {entry}"), "the row at an index is the one of the entry at that index");
        }
        assert_eq!(keep_built(vec![1, 2, 3], |_| true), [1, 2, 3]);
        assert_eq!(keep_built(Vec::<u8>::new(), |_| false), Vec::<u8>::new());
    }

    #[test]
    fn many_opacity_requests_in_a_frame_become_one_apply_of_the_last() {
        let mut pending = PendingOpacity::default();
        assert_eq!(pending.take(0.72), None, "nothing was asked for");
        for percent in 20..=100 {
            pending.request(f64::from(percent) / 100.0);
        }
        pending.request(0.5);
        assert_eq!(pending.latest(0.72), 0.5);
        assert_eq!(pending.take(0.72), Some(0.5));
        assert_eq!(pending.take(0.72), None, "a request is applied once");
        assert_eq!(pending.latest(0.72), 0.72);
    }

    #[test]
    fn an_opacity_request_equal_to_the_applied_one_applies_nothing() {
        let mut pending = PendingOpacity::default();
        pending.request(0.72);
        assert_eq!(pending.take(0.72), None);
        // The slider's own echo of a value it was given arrives as the same request.
        pending.request(72.0 / 100.0);
        assert_eq!(pending.take(0.72), None);
    }

    #[test]
    fn a_failed_opacity_is_said_in_its_row_only_once_the_slider_has_rested_and_until_the_next_change() {
        let mut state = OpacityApply::default();
        assert!(!state.shows_label(), "nothing failed");
        // While the slider moves a failure is only logged.
        state.failed();
        assert!(!state.shows_label());
        state.requested();
        state.failed();
        assert!(!state.shows_label(), "still moving");
        // The slider rests with the last apply failed: the row says so.
        state.rested();
        assert!(state.shows_label());
        // Resting again changes nothing, and so does a request that has not been applied yet.
        state.rested();
        assert!(state.shows_label());
        state.requested();
        assert!(!state.shows_label(), "moving again hides it");
        // A request that applies puts the percent back for good; resting then has nothing to say.
        state.applied();
        state.rested();
        assert!(!state.shows_label());
        // A slider that rested without a failure says nothing either.
        let mut fine = OpacityApply::default();
        fine.rested();
        assert!(!fine.shows_label());
    }

    #[test]
    fn opacity_requests_are_whole_percent_inside_the_valid_range() {
        let mut pending = PendingOpacity::default();
        for (asked, applied) in [(0.724, 0.72), (0.726, 0.73), (7.0, 1.0), (0.0, 0.2), (-3.0, 0.2)] {
            pending.request(asked);
            assert_eq!(pending.take(0.9), Some(applied), "asked for {asked}");
        }
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
        assert_eq!(after(|s| s.osc52 = Osc52Policy::Off), Effects { osc52: true, ..Effects::default() });
        assert_eq!(after(|s| s.cursor.style = CursorStyle::Bar), Effects { cursor: true, ..Effects::default() });
        assert_eq!(after(|s| s.scrollback = 500), Effects { scrollback: true, ..Effects::default() });
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

    #[test]
    fn the_drawer_is_480_wide_and_gives_way_to_a_narrow_window_down_to_320() {
        assert_eq!(drawer_width(1440.0), 480.0);
        assert_eq!(drawer_width(1000.0), 480.0);
        assert_eq!(drawer_width(700.0), 480.0);
        assert_eq!(drawer_width(496.0), 480.0, "480 plus both margins fits exactly");
        assert_eq!(drawer_width(495.0), 479.0);
        assert_eq!(drawer_width(400.0), 384.0);
        assert_eq!(drawer_width(336.0), 320.0);
        assert_eq!(drawer_width(200.0), 320.0, "never narrower than 320");
        assert_eq!(drawer_width(0.0), 320.0, "before the first layout");
        for width in (0..2000).step_by(7) {
            let drawer = drawer_width(f64::from(width));
            assert!((DRAWER_MIN_WIDTH..=DRAWER_WIDTH).contains(&drawer));
            assert!(drawer + 2.0 * DRAWER_MARGIN <= f64::from(width) || drawer == DRAWER_MIN_WIDTH, "{width}");
        }
    }

    #[test]
    fn navigation_keys_move_through_a_row_a_list_and_a_grid() {
        use Arrow::*;
        // A row of four choices: sideways only, and the ends stay put.
        assert_eq!(roving_target(0, 4, 4, Right), Some(1));
        assert_eq!(roving_target(3, 4, 4, Right), None);
        assert_eq!(roving_target(0, 4, 4, Left), None);
        assert_eq!(roving_target(2, 4, 4, Left), Some(1));
        assert_eq!(roving_target(1, 4, 4, Down), None);
        assert_eq!(roving_target(1, 4, 4, Up), None);
        assert_eq!(roving_target(2, 4, 4, Home), Some(0));
        assert_eq!(roving_target(0, 4, 4, End), Some(3));
        assert_eq!(roving_target(0, 4, 4, Home), None, "already there");
        // A list: up and down.
        assert_eq!(roving_target(1, 3, 1, Down), Some(2));
        assert_eq!(roving_target(1, 3, 1, Up), Some(0));
        assert_eq!(roving_target(2, 3, 1, Down), None);
        // Ten colour tiles in five columns: Down from Tokyo Night (0) is Nord (5).
        assert_eq!(roving_target(0, 10, 5, Down), Some(5));
        assert_eq!(roving_target(5, 10, 5, Up), Some(0));
        assert_eq!(roving_target(7, 10, 5, Down), None);
        assert_eq!(roving_target(4, 10, 5, Right), Some(5), "the end of a row continues in the next");
        assert_eq!(roving_target(0, 0, 5, Right), None);
        assert_eq!(roving_target(0, 1, 5, Down), None, "fewer items than columns");
    }

    #[test]
    fn the_peek_follows_whichever_of_pointer_and_focus_moved_last() {
        let mut peek = Peek::default();
        assert_eq!(peek.tile(), None);
        peek.focused(Some(2));
        assert_eq!(peek.tile(), Some(2));
        peek.hovered(Some(5));
        assert_eq!(peek.tile(), Some(5), "the pointer moved last");
        peek.focused(Some(3));
        assert_eq!(peek.tile(), Some(3), "now the keyboard did");
        peek.focused(None);
        assert_eq!(peek.tile(), Some(5), "the pointer is still on a tile");
        peek.hovered(None);
        assert_eq!(peek.tile(), None, "left the grid: the committed scheme is put back");
        peek.hovered(Some(1));
        peek.clear();
        assert_eq!(peek.tile(), None);
    }

    #[test]
    fn numbers_are_shown_short_typed_loosely_and_stepped_without_drift() {
        assert_eq!(number_text(14.0), "14");
        assert_eq!(number_text(1.1), "1.1");
        assert_eq!(number_text(1.05), "1.05");
        assert_eq!(number_text(1.1500000000000001), "1.15");
        assert_eq!(number_text(10000.0), "10000");
        assert_eq!(parse_number(" 16 "), Some(16.0));
        assert_eq!(parse_number("1.25"), Some(1.25));
        for text in ["", "x", "1e999", "NaN", "inf", "1,5"] {
            assert_eq!(parse_number(text), None, "{text:?}");
        }
        let line_height = Range { min: 1.0, max: 2.0, step: 0.05, large: 0.25 };
        assert_eq!(step_number(1.1, 1, line_height), 1.15);
        assert_eq!(step_number(1.1, -2, line_height), 1.0);
        assert_eq!(step_number(1.95, 3, line_height), 2.0, "stays inside the range");
        let mut value = 1.0;
        for _ in 0..20 {
            value = step_number(value, 1, line_height);
        }
        assert_eq!(value, 2.0, "twenty steps of 0.05 land on 2 exactly");
    }
}
