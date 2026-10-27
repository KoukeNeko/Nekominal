//! The agents panel: every pane of every window of this process that runs an agent, with what it is doing and what it
//! said about its session, the ones that wait for the user first, and what the agents cost today at the bottom. Choosing
//! a row shows that pane, in whichever window it is.
//!
//! A child of `app`, so that it reads the window's tabs where they are; the window code (`win.rs`) gathers the panes of
//! every window, since a window cannot borrow the others while it handles a key.

use nekominal_model::{layout::PaneId, strings::{Lang, Text}};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, PostMessageW, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow},
};

use super::App;
use crate::{agent::Usage, hooks, logic::{self, Mark, TabState}, popup, status};

/// The glyph of a row whose agent has said nothing of its state: the one of the programs that are agents.
const AGENT_GLYPH: char = '\u{E943}';

/// A pane that runs an agent, as one window sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// The window (its handle), the tab (its uid) and the pane, for showing it.
    pub window: isize,
    pub tab: u64,
    pub pane: PaneId,
    /// Where it is: the order of its window (the one that asks first), then of its tab in the strip.
    pub place: (usize, usize),
    /// The pane is the one the window shows with the keyboard.
    pub focused: bool,
    pub agent: String,
    pub title: String,
    pub folder: Option<String>,
    pub state: TabState,
    /// How long the agent has run: its own count when it says one, else since the pane was first seen running it.
    pub time_ms: Option<u64>,
    pub usage: Option<Usage>,
    /// The pull request and listening ports of its tab, as text (`app/badges.rs`).
    pub badges: Option<String>,
}

/// `ms` as the bar shows a time: `m:ss`, or `h:mm:ss` from an hour.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// The rows of the panel for the panes `seen` from the window `here`: the most urgent state first (a question or a
/// failure, then work, then done), each kind in the order of the windows and of their tabs; then what today cost.
pub fn rows(mut seen: Vec<Seen>, here: isize, today: f64, lang: Lang) -> Vec<popup::Row> {
    seen.sort_by_key(|s| (std::cmp::Reverse(logic::state_rank(s.state)), s.place));
    let row = |label: String, hint: String, action: popup::Action| popup::Row { label, hint, action: Some(action), current: false, mark: None, program: None };
    let mut rows: Vec<popup::Row> = seen
        .into_iter()
        .map(|s| {
            let label = [Some(s.agent), Some(s.title), s.folder].into_iter().flatten().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
            let usage = s.usage.unwrap_or_default();
            let hint = [
                super::state_text(s.state).map(|text| text.get(lang).to_owned()),
                s.time_ms.map(clock),
                usage.model,
                usage.context.map(|share| format!("{share:.0}%")),
                usage.cost.map(|cost| format!("${cost:.2}")),
                s.badges,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            popup::Row {
                current: s.focused && s.window == here,
                mark: Some(Mark::Glyph(status::glyph(s.state).unwrap_or(AGENT_GLYPH))),
                ..row(label, hint, popup::Action::Agent { window: s.window, tab: s.tab, pane: s.pane })
            }
        })
        .collect();
    if rows.is_empty() {
        rows.push(row(Text::AgentsNone.get(lang).to_owned(), String::new(), popup::Action::Cancel));
    }
    rows.push(popup::Row::separator());
    rows.push(row(Text::AgentsToday.format(lang, &[&format!("${today:.2}")]), String::new(), popup::Action::Cancel));
    rows
}

impl App {
    /// The panes of this window that run an agent; `window` is the order of this window among those asked.
    pub fn agents_seen(&self, window: usize) -> Vec<Seen> {
        let home = std::env::var("USERPROFILE").ok();
        let mut seen = Vec::new();
        for (t, tab) in self.tabs.iter().enumerate() {
            for entry in tab.panes.iter().filter(|entry| Self::runs_agent(entry)) {
                let record = entry.pane.agent_summary();
                // Read here, not from the window's copy: the window keeps only the focused pane's up to date.
                let usage = crate::agent::read_usage(&entry.session).or_else(|| entry.usage.clone());
                let agent = record
                    .as_ref()
                    .and_then(|r| r.app.as_deref())
                    .or_else(|| entry.job.as_deref().and_then(hooks::agent_of_program))
                    .or(usage.is_some().then_some("claude"))
                    .map_or_else(|| "Agent".to_owned(), hooks::agent_name);
                let signal = logic::PaneSignal { agent: record, progress: entry.progress, attention: entry.attention, note: entry.note.clone() };
                let cwd = usage.as_ref().and_then(|u| u.cwd.clone()).or_else(|| entry.cwd.clone());
                seen.push(Seen {
                    window: self.hwnd.0 as isize,
                    tab: tab.uid,
                    pane: entry.id,
                    place: (window, t),
                    focused: t == self.selected && entry.id == tab.focused,
                    agent,
                    title: self.tab_label(t),
                    folder: cwd.map(|cwd| logic::folder_label(&cwd, home.as_deref())),
                    state: logic::tab_status(&[signal]).state,
                    time_ms: usage.as_ref().and_then(|u| u.duration_ms).or_else(|| entry.agent_since.map(|at| at.elapsed().as_millis() as u64)),
                    usage,
                    badges: self.badges.text(tab.uid),
                });
            }
        }
        seen
    }

    /// Opens the panel with the panes `seen` in all the windows, the focused one under the pointer.
    pub fn open_agents(&mut self, seen: Vec<Seen>) {
        let rows = rows(seen, self.hwnd.0 as isize, crate::spend::today_total(), self.language());
        let at = rows.iter().position(|row| row.current);
        let anchor = self.screen_point((self.layout.size.0 + popup::PALETTE_WIDTH) / 2.0 * self.scale, (crate::layout::TITLE_HEIGHT + 16.0) * self.scale);
        self.open_popup(rows, popup::Kind::Wide, anchor);
        if let (Some(popup), Some(at)) = (self.popup.as_mut(), at) {
            popup.hover = Some(at);
            popup.reveal(at);
        }
    }

    /// Shows a pane a row of the panel chose: here, or by asking the window that has it.
    pub fn show_agent(&mut self, window: isize, tab: u64, pane: PaneId) {
        if window == self.hwnd.0 as isize {
            self.show_pane(tab, pane);
        } else {
            // SAFETY: posting to a window of this thread; a window gone since does not take it.
            unsafe { let _ = PostMessageW(Some(HWND(window as *mut _)), crate::win::WM_SHOW_PANE, WPARAM(tab as usize), LPARAM(pane as isize)); }
        }
    }

    /// Selects the tab with this uid and focuses its pane, with the window brought up from wherever it is.
    pub fn show_pane(&mut self, tab: u64, pane: PaneId) {
        let Some(index) = self.tabs.iter().position(|t| t.uid == tab) else { return };
        self.select(index);
        if self.tabs[index].focus(pane) {
            self.sync_focus();
            self.redraw();
        }
        // SAFETY: plain calls on a window of this thread.
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            } else if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(place: (usize, usize), state: TabState) -> Seen {
        Seen {
            window: place.0 as isize + 100,
            tab: place.1 as u64,
            pane: 1,
            place,
            focused: false,
            agent: "Codex".into(),
            title: format!("tab {}-{}", place.0, place.1),
            folder: None,
            state,
            time_ms: None,
            usage: None,
            badges: None,
        }
    }

    fn labels(rows: &[popup::Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn the_panes_that_wait_come_first_then_work_then_done_each_in_window_and_tab_order() {
        let list = vec![
            seen((0, 2), TabState::Done),
            seen((1, 0), TabState::Working),
            seen((0, 1), TabState::Working),
            seen((1, 3), TabState::Blocked),
            seen((0, 0), TabState::Idle),
            seen((0, 4), TabState::Error),
        ];
        let rows = rows(list, 100, 0.0, Lang::EnUs);
        let order: Vec<&str> = labels(&rows).into_iter().map(|label| label.trim_start_matches("Codex · ")).collect();
        assert_eq!(order, ["tab 1-3", "tab 0-4", "tab 0-1", "tab 1-0", "tab 0-2", "tab 0-0", "", "Spent today: $0.00"]);
        assert!(rows[6].is_separator());
    }

    #[test]
    fn a_row_says_the_agent_tab_and_folder_and_its_state_time_model_context_and_cost() {
        let full = Seen {
            folder: Some("~\\repo".into()),
            focused: true,
            time_ms: Some(3_725_000),
            usage: Some(Usage { model: Some("Opus".into()), context: Some(42.4), cost: Some(0.8412), duration_ms: Some(3_725_000), cwd: None }),
            agent: "Claude Code".into(),
            ..seen((0, 0), TabState::Working)
        };
        let rows = rows(vec![full.clone()], 100, 3.456, Lang::EnUs);
        assert_eq!(rows[0].label, "Claude Code · tab 0-0 · ~\\repo");
        assert_eq!(rows[0].hint, "Running · 1:02:05 · Opus · 42% · $0.84");
        assert_eq!(rows[0].action, Some(popup::Action::Agent { window: 100, tab: 0, pane: 1 }));
        assert!(rows[0].current, "the pane the asking window shows");
        assert_eq!(rows[0].mark, Some(Mark::Glyph(status::glyph(TabState::Working).unwrap())));
        assert_eq!(rows.last().unwrap().label, "Spent today: $3.46");
        let elsewhere = super::rows(vec![full], 200, 0.0, Lang::ZhTw);
        assert!(!elsewhere[0].current, "focused in another window is not current here");
        assert_eq!(elsewhere[0].hint, "執行中 · 1:02:05 · Opus · 42% · $0.84");
    }

    #[test]
    fn an_agent_that_said_nothing_shows_only_what_is_known() {
        let quiet = Seen { time_ms: Some(65_000), ..seen((0, 0), TabState::Idle) };
        let rows = rows(vec![quiet.clone(), Seen { badges: Some("#4 open \u{b7} :3000".into()), ..quiet }], 100, 0.0, Lang::EnUs);
        assert_eq!(rows[0].hint, "1:05");
        assert_eq!(rows[1].hint, "1:05 \u{b7} #4 open \u{b7} :3000", "the badges of its tab come last");
        assert_eq!(rows[0].mark, Some(Mark::Glyph(AGENT_GLYPH)));
    }

    #[test]
    fn without_agents_the_panel_says_so_above_the_total() {
        let rows = rows(Vec::new(), 100, 0.0, Lang::EnUs);
        assert_eq!(labels(&rows), ["No agents are running", "", "Spent today: $0.00"]);
        assert_eq!(rows[0].action, Some(popup::Action::Cancel));
        assert_eq!(rows[2].action, Some(popup::Action::Cancel), "the total is a row that closes the panel, not a separator");
    }

    #[test]
    fn times_read_as_the_bar_writes_them() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(59_999), "0:59");
        assert_eq!(clock(600_000), "10:00");
        assert_eq!(clock(3_600_000), "1:00:00");
    }
}
