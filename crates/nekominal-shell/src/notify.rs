//! When the window tells the user about what happens out of sight with a notification of Windows (`toast.rs`): an agent
//! that finished, waits for an answer or failed, a bell, a program's own notification (OSC 9 or OSC 777), and a command
//! that ran long and ended. What to tell, when and in which words is decided here, without a window, so it can be tested.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use nekominal_model::{layout::PaneId, strings::{Lang, Text}};
use nekominal_term::osc::PromptMark;

use crate::logic::TabState;

/// Something worth a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The tab reached this state: done, blocked (waits for an answer) or error. What the agent said with it, if anything.
    Agent(TabState, Option<String>),
    Bell,
    /// The text of a program's notification.
    Message(String),
    /// A command that ran at least the time of the setting ended: what was typed (when the shell marked it), its exit
    /// code (when the shell said it) and how long it ran.
    Command { line: Option<String>, exit: Option<i64>, seconds: u64 },
}

impl Event {
    /// A program can ring or notify as often as it likes; an agent's state and the end of a long command come seldom.
    pub fn chatty(&self) -> bool {
        matches!(self, Event::Bell | Event::Message(_))
    }

    /// What the notification says under the title of the tab.
    pub fn body(&self, lang: Lang) -> String {
        match self {
            Event::Agent(state, message) => {
                let what = match state {
                    TabState::Blocked => Text::AgentBlocked,
                    TabState::Error => Text::AgentError,
                    _ => Text::AgentDone,
                };
                match message {
                    Some(message) => format!("{}\n{message}", what.get(lang)),
                    None => what.get(lang).to_owned(),
                }
            }
            Event::Bell => Text::AgentBell.get(lang).to_owned(),
            Event::Message(text) => text.clone(),
            Event::Command { line, exit, seconds } => {
                let what = match exit {
                    Some(code) if *code != 0 => Text::ToastCommandFailed.format(lang, &[code, seconds]),
                    _ => Text::ToastCommandDone.format(lang, &[seconds]),
                };
                match line {
                    Some(line) => format!("{what}\n{line}"),
                    None => what,
                }
            }
        }
    }
}

/// Whether an event is worth telling where the user is: never about the tab they look at in the active window. The
/// window-wide ones are for a window in the background only, as its taskbar flash is; the end of a long command is
/// also told while another tab of the active window is in front, which is the point of waiting for it elsewhere.
pub fn wanted(event: &Event, window_active: bool, tab_in_view: bool) -> bool {
    match event {
        Event::Command { .. } => !(window_active && tab_in_view),
        _ => !window_active,
    }
}

/// The state a tab has newly reached that is worth telling: done, waiting for an answer or failed, from any other. A
/// tab seen for the first time (new, or moved in from another window) tells nothing: what it shows is not news.
pub fn agent_event(before: Option<TabState>, now: TabState) -> bool {
    before.is_some_and(|before| before != now) && matches!(now, TabState::Done | TabState::Blocked | TabState::Error)
}

/// The command took long enough for the setting, in seconds; 0 turns the notification off.
pub fn long_enough(elapsed: Duration, threshold: u32) -> bool {
    threshold > 0 && elapsed >= Duration::from_secs(u64::from(threshold))
}

/// At most this many notifications in [`BURST_WINDOW`], for the whole window: agents that all finish at once make a few,
/// not a column of them.
pub const BURST: usize = 3;
pub const BURST_WINDOW: Duration = Duration::from_secs(10);
/// A bell or a program's notification from a tab that was told about less than this ago is dropped: a program that
/// prints OSC 9 in a loop makes one.
pub const CHATTY_GAP: Duration = Duration::from_secs(30);

/// What has been shown lately, to keep the notifications few.
#[derive(Debug, Default)]
pub struct Limiter {
    recent: VecDeque<Instant>,
    last_by_tab: HashMap<u64, Instant>,
}

impl Limiter {
    /// Whether a notification of `event` from tab `tab` may be shown at `now`; one that may is counted as shown.
    pub fn allow(&mut self, now: Instant, tab: u64, event: &Event) -> bool {
        while self.recent.front().is_some_and(|at| now.duration_since(*at) >= BURST_WINDOW) {
            self.recent.pop_front();
        }
        let too_soon = event.chatty() && self.last_by_tab.get(&tab).is_some_and(|at| now.duration_since(*at) < CHATTY_GAP);
        if too_soon || self.recent.len() >= BURST {
            return false;
        }
        self.recent.push_back(now);
        self.last_by_tab.insert(tab, now);
        true
    }
}

/// The notifications of one window: what each tab showed last, the commands running, what waits to be told.
#[derive(Debug, Default)]
pub struct Notifier {
    states: HashMap<u64, TabState>,
    /// When the output of a command started (OSC 133 `C`) in a pane, by tab and pane, and what was typed for it.
    // shortcut: a pane closed while its command runs leaves its entry behind (a few bytes); prune by the live panes if
    // that ever matters.
    running: HashMap<(u64, PaneId), (Instant, Option<String>)>,
    pub limiter: Limiter,
    /// Events by the uid of their tab, for `App` to decide on and show after it has read the output of every pane.
    pub pending: Vec<(u64, Event)>,
}

impl Notifier {
    pub fn queue(&mut self, tab: u64, event: Event) {
        self.pending.push((tab, event));
    }

    /// A prompt mark of a pane: the output of a command starts its clock (the line typed is read then, by `line`), and
    /// its end queues the event when it ran at least `threshold` seconds. A command whose start was not marked (a shell
    /// without the `C` mark) is not timed: from the prompt (`B`) the time would count the typing too.
    pub fn on_mark(&mut self, tab: u64, pane: PaneId, mark: PromptMark, now: Instant, threshold: u32, line: impl FnOnce() -> Option<String>) {
        match mark {
            PromptMark::OutputStart if threshold > 0 => {
                self.running.insert((tab, pane), (now, line()));
            }
            PromptMark::CommandEnd { exit_code } => {
                if let Some((started, line)) = self.running.remove(&(tab, pane)) {
                    let elapsed = now.duration_since(started);
                    if long_enough(elapsed, threshold) {
                        self.queue(tab, Event::Command { line, exit: exit_code, seconds: elapsed.as_secs() });
                    }
                }
            }
            _ => {}
        }
    }

    /// The state the tab shows now: queues the event when it is news (`agent_event`).
    pub fn observe(&mut self, tab: u64, now: TabState, message: Option<String>) {
        if agent_event(self.states.insert(tab, now), now) {
            self.queue(tab, Event::Agent(now, message));
        }
    }

    /// Forgets the tabs that are no longer in the window.
    pub fn retain(&mut self, alive: impl Fn(u64) -> bool) {
        self.states.retain(|tab, _| alive(*tab));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_told_about_only_when_the_user_is_not_looking_at_it() {
        let done = Event::Agent(TabState::Done, None);
        assert!(wanted(&done, false, true), "the window is in the background");
        assert!(!wanted(&done, true, true));
        assert!(!wanted(&done, true, false), "an agent's state is shown by its tab in an active window");
        let command = Event::Command { line: None, exit: Some(0), seconds: 12 };
        assert!(wanted(&command, true, false), "a long command ended in another tab");
        assert!(wanted(&command, false, true));
        assert!(!wanted(&command, true, true), "the user watched it end");
    }

    #[test]
    fn an_agent_state_is_news_when_it_changes_to_done_blocked_or_error() {
        assert!(agent_event(Some(TabState::Working), TabState::Done));
        assert!(agent_event(Some(TabState::Working), TabState::Blocked));
        assert!(agent_event(Some(TabState::Idle), TabState::Error));
        assert!(!agent_event(Some(TabState::Done), TabState::Done), "no change");
        assert!(!agent_event(Some(TabState::Done), TabState::Working));
        assert!(!agent_event(Some(TabState::Idle), TabState::Attention), "a bell or a notification tells itself");
        assert!(!agent_event(None, TabState::Blocked), "a tab first seen is not news");
    }

    #[test]
    fn the_notifier_queues_an_agent_change_once_and_not_for_a_tab_it_meets() {
        let mut notifier = Notifier::default();
        notifier.observe(7, TabState::Done, None);
        assert!(notifier.pending.is_empty());
        notifier.observe(7, TabState::Working, None);
        notifier.observe(7, TabState::Blocked, Some("Allow edit?".into()));
        notifier.observe(7, TabState::Blocked, Some("Allow edit?".into()));
        assert_eq!(notifier.pending, [(7, Event::Agent(TabState::Blocked, Some("Allow edit?".into())))]);
        notifier.retain(|_| false);
        notifier.observe(7, TabState::Done, None);
        assert_eq!(notifier.pending.len(), 1, "a forgotten tab is met again");
    }

    #[test]
    fn a_command_is_timed_from_its_output_to_its_end_against_the_threshold() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut notifier = Notifier::default();
        let end = |code| PromptMark::CommandEnd { exit_code: Some(code) };
        notifier.on_mark(1, 1, PromptMark::OutputStart, at(0), 10, || Some("cargo build".into()));
        notifier.on_mark(1, 1, end(0), at(9), 10, || None);
        assert!(notifier.pending.is_empty(), "nine seconds is short");
        notifier.on_mark(1, 1, PromptMark::OutputStart, at(20), 10, || Some("cargo test".into()));
        notifier.on_mark(1, 2, end(0), at(40), 10, || None);
        assert!(notifier.pending.is_empty(), "another pane's end is not this command's");
        notifier.on_mark(1, 1, end(101), at(35), 10, || None);
        assert_eq!(notifier.pending, [(1, Event::Command { line: Some("cargo test".into()), exit: Some(101), seconds: 15 })]);
        // An end without a marked start (the first prompt, Ctrl+C at a prompt) and a threshold of 0 tell nothing.
        notifier.on_mark(1, 1, end(0), at(99), 10, || None);
        notifier.on_mark(1, 1, PromptMark::OutputStart, at(100), 0, || unreachable!("not read when off"));
        notifier.on_mark(1, 1, end(0), at(200), 0, || None);
        assert_eq!(notifier.pending.len(), 1);
        assert!(long_enough(Duration::from_secs(10), 10));
        assert!(!long_enough(Duration::from_secs(3600), 0));
    }

    #[test]
    fn the_limiter_drops_a_flood_from_one_tab_and_caps_bursts() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut limiter = Limiter::default();
        let note = Event::Message("tick".into());
        assert!(limiter.allow(at(0), 1, &note));
        assert!(!limiter.allow(at(1), 1, &note), "a program that notifies in a loop");
        assert!(!limiter.allow(at(29), 1, &Event::Bell));
        assert!(limiter.allow(at(30), 1, &note));
        // An agent's state is not held back by the tab's last notification, only by the burst.
        let done = Event::Agent(TabState::Done, None);
        assert!(limiter.allow(at(31), 1, &done));
        assert!(limiter.allow(at(32), 2, &done));
        assert!(!limiter.allow(at(33), 3, &done), "three in ten seconds already");
        assert!(limiter.allow(at(41), 3, &done));
    }

    #[test]
    fn the_text_says_what_happened_in_the_language_of_the_interface() {
        let blocked = Event::Agent(TabState::Blocked, Some("Allow edit?".into()));
        assert_eq!(blocked.body(Lang::EnUs), "Waiting for input\nAllow edit?");
        assert_eq!(Event::Agent(TabState::Error, None).body(Lang::ZhTw), "錯誤");
        assert_eq!(Event::Agent(TabState::Done, None).body(Lang::EnUs), "Done");
        assert_eq!(Event::Bell.body(Lang::EnUs), "Bell");
        assert_eq!(Event::Message("Build finished".into()).body(Lang::ZhTw), "Build finished");
        let ok = Event::Command { line: Some("cargo build".into()), exit: Some(0), seconds: 42 };
        assert_eq!(ok.body(Lang::EnUs), "Command finished after 42 s\ncargo build");
        assert_eq!(ok.body(Lang::ZhTw), "指令完成，耗時 42 秒\ncargo build");
        let failed = Event::Command { line: None, exit: Some(2), seconds: 11 };
        assert_eq!(failed.body(Lang::EnUs), "Command failed with exit code 2 after 11 s");
        assert_eq!(failed.body(Lang::ZhTw), "指令失敗，結束代碼 2，耗時 11 秒");
        let unknown = Event::Command { line: None, exit: None, seconds: 11 };
        assert_eq!(unknown.body(Lang::EnUs), "Command finished after 11 s", "no code said is no failure");
    }
}
