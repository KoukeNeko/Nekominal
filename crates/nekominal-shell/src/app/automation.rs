//! What the automation API (`api.rs`) asks of the windows, answered on the UI thread: the windows, tabs and panes, and
//! the commands on a pane, found in whichever window has it. And the events of the panes for the subscribers, from the
//! output a window reads (`on_wake`).

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::atomic::Ordering, time::Instant};

use nekominal_model::{layout::{Axis, PaneId}, strings::Text};
use nekominal_term::{agent_status::AgentState, osc::PromptMark};
use serde_json::{Value, json};
use windows::Win32::Foundation::HWND;

use super::{App, Confirmation};
use crate::{
    api::{self, Direction, Request, Topic},
    cli::{Action, Target},
    logic, notify,
    tab::PaneEntry,
};

thread_local! {
    /// The agent state each pane was last seen in, by its id, while someone follows `agent-state`.
    // shortcut: a closed pane leaves its entry (a few bytes) until the process ends; prune by the live panes if that matters.
    static STATES: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
}

/// The answer to a request that would ask the user while the window already asks something.
const QUESTION_OPEN: &str = "a question is already open in the window";

fn state_name(state: Option<AgentState>) -> &'static str {
    match state {
        None => "none",
        Some(AgentState::Idle) => "idle",
        Some(AgentState::Working) => "working",
        Some(AgentState::Done) => "done",
        Some(AgentState::Blocked) => "blocked",
        Some(AgentState::Error) => "error",
    }
}

/// The last `lines` lines of `text`.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Answers a request from the windows of the process (the one in front first); `None` when a window that may hold the
/// answer is busy (the request came while one of its handlers runs), for the request to be tried again.
pub fn answer(windows: &[(HWND, Rc<RefCell<App>>)], request: &Request) -> Option<Result<Value, String>> {
    let pane = match request {
        Request::List => {
            let mut listed = Vec::new();
            for (_, app) in windows {
                listed.push(app.try_borrow().ok()?.api_window());
            }
            return Some(Ok(json!({ "pid": std::process::id(), "windows": listed })));
        }
        Request::Subscribe { .. } => return Some(Err("subscribe is answered by the pipe".to_owned())),
        Request::NewTab { profile, cwd, command } => {
            let (_, app) = windows.first()?;
            return Some(app.try_borrow_mut().ok()?.api_new_tab(profile.clone(), cwd.clone(), command.clone()));
        }
        Request::Notify { pane: None, title, body } => {
            let (_, app) = windows.first()?;
            let mut app = app.try_borrow_mut().ok()?;
            let selected = app.selected;
            return Some(app.api_notify(title, body, selected));
        }
        Request::Notify { pane: Some(pane), .. }
        | Request::SendText { pane, .. }
        | Request::GetText { pane, .. }
        | Request::Split { pane, .. }
        | Request::Focus { pane }
        | Request::Close { pane }
        | Request::SetTitle { pane, .. } => pane,
    };
    // Each window is looked into on its own, and only the one with the pane is borrowed to change it: a window that
    // comes forward (focus) is sent its activation while the others are free to take it.
    for (_, app) in windows {
        let found = app.try_borrow().ok()?.api_find(pane);
        if let Some(at) = found {
            return Some(app.try_borrow_mut().ok()?.api_on_pane(request, at));
        }
    }
    Some(Err(format!("no pane {pane}")))
}

impl App {
    fn api_pane(&self, tab: usize, entry: &PaneEntry) -> Value {
        let record = entry.pane.agent_summary();
        json!({
            "id": entry.session,
            "title": entry.title,
            "cwd": entry.cwd,
            "job": entry.job,
            "profile": entry.profile,
            "agent": record.as_ref().and_then(|record| record.app.clone()),
            "agentState": state_name(record.map(|record| record.state)),
            "focused": tab == self.selected && entry.id == self.tabs[tab].focused,
            "exited": entry.exited,
        })
    }

    /// The window, its tabs and their panes, for `list`.
    fn api_window(&self) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(t, tab)| {
                let panes: Vec<Value> = tab.panes.iter().map(|entry| self.api_pane(t, entry)).collect();
                json!({ "id": tab.uid, "title": self.tab_label(t), "selected": t == self.selected, "panes": panes })
            })
            .collect();
        json!({ "id": self.hwnd.0 as isize, "title": self.window_title, "active": self.active, "tabs": tabs })
    }

    /// Where the pane with this id is: the position of its tab and its id there.
    fn api_find(&self, pane: &str) -> Option<(usize, PaneId)> {
        self.tabs.iter().enumerate().find_map(|(t, tab)| tab.panes.iter().find(|entry| entry.session == pane).map(|entry| (t, entry.id)))
    }

    fn api_session(&self, tab: usize) -> Option<String> {
        self.tabs.get(tab)?.focused_entry().map(|entry| entry.session.clone())
    }

    /// A request about the pane `id` of tab `t`.
    fn api_on_pane(&mut self, request: &Request, (t, id): (usize, PaneId)) -> Result<Value, String> {
        let gone = || "the pane is gone".to_owned();
        match request {
            Request::SendText { text, .. } => {
                let entry = self.tabs[t].entry(id).ok_or_else(gone)?;
                // The check of a paste of the clipboard (see `api.rs`): what it would ask about, the user is asked.
                let title = match logic::paste_warning(text, entry.pane.input_modes().bracketed_paste, &self.settings.paste) {
                    None => {
                        entry.pane.paste(text);
                        return Ok(json!({ "sent": true }));
                    }
                    Some(logic::PasteWarning::Lines(lines)) => Text::DialogPasteLines.format(self.language(), &[&lines]),
                    Some(logic::PasteWarning::Bytes(bytes)) => Text::DialogPasteSize.format(self.language(), &[&logic::size_label(bytes)]),
                };
                // The window asks one question at a time: this one would not be shown, and the text never sent.
                if self.confirmation.is_some() {
                    return Err(QUESTION_OPEN.to_owned());
                }
                self.show_pane(self.tabs[t].uid, id);
                self.confirm(title, Text::EditPaste, Confirmation::Paste { tab: t, pane: id, text: text.clone() });
                Ok(json!({ "sent": false, "asked": true }))
            }
            Request::GetText { lines, .. } => {
                let entry = self.tabs[t].entry(id).ok_or_else(gone)?;
                let text = entry.pane.term.lock().all_text();
                Ok(json!({ "text": tail(&text, lines.unwrap_or(usize::from(entry.grid.1))) }))
            }
            Request::Split { direction, cwd, command, .. } => {
                // A split is of the focused pane of the selected tab: the pane is made that first.
                self.select(t);
                self.tabs[t].focus(id);
                let target = Target { directory: cwd.clone(), profile: None, command: command.clone() };
                let profile = self.target_profile(&target);
                let before = self.tabs[t].panes.len();
                self.split_with(if *direction == Direction::Right { Axis::Row } else { Axis::Column }, &profile, cwd.as_deref());
                if self.tabs[t].panes.len() == before {
                    return Err("the pane could not be started (see the log)".to_owned());
                }
                Ok(json!({ "pane": self.api_session(t) }))
            }
            Request::Focus { .. } => {
                self.show_pane(self.tabs[t].uid, id);
                Ok(json!({}))
            }
            Request::Close { pane } => {
                let asking = self.confirmation.is_some();
                // As the close button does: a pane whose program is busy asks first, as the settings say. While another
                // question is open that one is not asked, and the pane stays.
                self.request_close_pane(t, id);
                if asking && self.api_find(pane).is_some() {
                    return Err(QUESTION_OPEN.to_owned());
                }
                Ok(json!({ "asked": !asking && self.confirmation.is_some() }))
            }
            Request::SetTitle { title, .. } => {
                self.tabs[t].name = (!title.is_empty()).then(|| title.clone());
                self.update_window_title();
                self.redraw();
                Ok(json!({}))
            }
            Request::Notify { title, body, .. } => self.api_notify(title, body, t),
            Request::List | Request::NewTab { .. } | Request::Subscribe { .. } => Err("not a request about a pane".to_owned()),
        }
    }

    /// A tab, as the command line opens one: a profile (by name or id), a folder, a program.
    fn api_new_tab(&mut self, profile: Option<String>, cwd: Option<String>, command: Vec<String>) -> Result<Value, String> {
        if let Some(wanted) = &profile
            && !self.profiles.iter().any(|p| p.id.eq_ignore_ascii_case(wanted) || p.name.eq_ignore_ascii_case(wanted))
        {
            return Err(format!("no profile {wanted}"));
        }
        let before = self.tabs.len();
        let elevated = self.launch(&[Action::NewTab(Target { directory: cwd, profile, command })]);
        self.redraw();
        if self.tabs.len() > before {
            let t = self.tabs.len() - 1;
            Ok(json!({ "tab": self.tabs[t].uid, "pane": self.api_session(t) }))
        } else if elevated {
            Ok(json!({ "elevated": true }))
        } else {
            Err("the tab could not be started (see the log)".to_owned())
        }
    }

    /// A notification of Windows, as the window shows its own: only with toasts on, and within their limits.
    fn api_notify(&mut self, title: &str, body: &str, tab: usize) -> Result<Value, String> {
        if !self.settings.notifications.toast {
            return Err("notifications are turned off in the settings".to_owned());
        }
        let uid = self.tabs.get(tab).ok_or("the window has no tab")?.uid;
        if !self.notifier.limiter.allow(Instant::now(), uid, &notify::Event::Message(format!("{title} {body}"))) {
            return Ok(json!({ "shown": false }));
        }
        crate::toast::show(self.hwnd, uid, title, body)?;
        Ok(json!({ "shown": true }))
    }

    /// At the start of `on_wake`, while the panes that woke still say so: their `output` events, and the agents whose
    /// state changed. Nothing is worked out while nobody follows.
    pub(super) fn api_observe(&self) {
        let (output, agents) = (api::wants(Topic::Output), api::wants(Topic::AgentState));
        if !agents {
            STATES.with(|states| states.borrow_mut().clear());
        }
        if !output && !agents {
            return;
        }
        for entry in self.tabs.iter().flat_map(|tab| &tab.panes) {
            if output && entry.wake.pending.load(Ordering::Acquire) {
                api::publish(Topic::Output, &entry.session, json!({}));
            }
            if agents {
                let state = state_name(entry.pane.agent_summary().map(|record| record.state));
                // A pane first seen is not news, as for the notifications.
                let before = STATES.with(|states| states.borrow_mut().insert(entry.session.clone(), state));
                if before.is_some_and(|before| before != state) {
                    api::publish(Topic::AgentState, &entry.session, json!({ "state": state }));
                }
            }
        }
    }

    /// A prompt mark of a pane: the end of a command is an event.
    pub(super) fn api_mark(entry: &PaneEntry, mark: PromptMark) {
        if let PromptMark::CommandEnd { exit_code } = mark
            && api::wants(Topic::CommandFinished)
        {
            let command = entry.pane.term.lock().last_command_line();
            api::publish(Topic::CommandFinished, &entry.session, json!({ "exit": exit_code, "command": command }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_text_gives_the_last_lines_asked_for_or_all_there_are() {
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail("a\nb", 10), "a\nb");
        assert_eq!(tail("", 3), "");
    }

    #[test]
    fn an_agent_state_has_the_name_its_osc_uses_and_none_without_an_agent() {
        assert_eq!(state_name(None), "none");
        assert_eq!(state_name(Some(AgentState::Blocked)), "blocked");
        assert_eq!(state_name(Some(AgentState::Working)), "working");
    }
}
