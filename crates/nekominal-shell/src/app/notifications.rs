//! The notifications of the window: what `notify` decided, shown as toasts (`toast`), or, with those off, the end of a
//! long command as the flash of the taskbar button; and pushed to a phone (`push`) when that is set.

use super::*;
use crate::{notify::Event, push, toast};

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
            let wanted = notify::wanted(&event, self.active, index == self.selected);
            let push = push::wanted(&self.settings.notifications.push, self.active, &event);
            let toast = wanted && self.settings.notifications.toast;
            // The toast and the push are one notification to the limiter: a flood the window drops, the phone does too.
            if (toast || push) && self.notifier.limiter.allow(now, uid, &event) {
                let (title, body) = (self.tab_label(index), event.body(lang));
                if toast && let Err(e) = toast::show(self.hwnd, uid, &title, &body) {
                    util::log(&format!("toast: {e}"));
                }
                if push {
                    push::send(&self.settings.notifications.push, push::Message::new(title, body, &event, uid));
                }
            }
            if wanted
                && !self.settings.notifications.toast
                && matches!(event, Event::Command { .. })
                && bell::flash_for_response(self.settings.bell.taskbar, self.active, self.last_alert.map(|at| at.elapsed()))
            {
                self.alert(BellEffects { taskbar_flash: true, ..BellEffects::default() });
            }
        }
    }

    /// The test button of the settings page: a push with the settings as they are, whose result the page shows.
    pub(super) fn send_test_push(&mut self) {
        let lang = self.language();
        push::send_test(&self.settings.notifications.push, nekominal_model::strings::APP_NAME.to_owned(), Text::PushTestBody.get(lang).to_owned());
    }

    /// A toast of the tab with this uid was clicked: selects the tab when this window has it.
    pub fn show_tab(&mut self, uid: u64) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.uid == uid) else { return false };
        self.select(index);
        true
    }
}
