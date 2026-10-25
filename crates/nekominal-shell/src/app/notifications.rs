//! The notifications of the window: what `notify` decided, shown as toasts (`toast`), or, with those off, the end of a
//! long command as the flash of the taskbar button.

use super::*;
use crate::{notify::Event, toast};

impl App {
    /// After the output of every pane was read: queues the states of the tabs that changed, and tells what is worth it.
    pub(super) fn notify(&mut self) {
        for tab in &self.tabs {
            let status = tab.status();
            self.notifier.observe(tab.uid, status.state, status.message);
        }
        self.notifier.retain(|uid| self.tabs.iter().any(|tab| tab.uid == uid));
        let (lang, now) = (self.language(), Instant::now());
        let mut pending = std::mem::take(&mut self.notifier.pending);
        // An agent that waits often also sends a notification of its own (Claude Code does): its state goes first, and the
        // limit of a chatty tab then drops the repeat.
        pending.sort_by_key(|(_, event)| event.chatty());
        for (uid, event) in pending {
            let Some(index) = self.tabs.iter().position(|tab| tab.uid == uid) else { continue };
            if !notify::wanted(&event, self.active, index == self.selected) {
                continue;
            }
            if self.settings.notifications.toast {
                if self.notifier.limiter.allow(now, uid, &event)
                    && let Err(e) = toast::show(self.hwnd, uid, &self.tab_label(index), &event.body(lang))
                {
                    util::log(&format!("toast: {e}"));
                }
            } else if matches!(event, Event::Command { .. })
                && bell::flash_for_response(self.settings.bell.taskbar, self.active, self.last_alert.map(|at| at.elapsed()))
            {
                self.alert(BellEffects { taskbar_flash: true, ..BellEffects::default() });
            }
        }
    }

    /// A toast of the tab with this uid was clicked: selects the tab when this window has it.
    pub fn show_tab(&mut self, uid: u64) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.uid == uid) else { return false };
        self.select(index);
        true
    }
}
