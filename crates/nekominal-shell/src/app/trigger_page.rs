//! The Triggers page of the settings: what a press on a rule or its form does, what a field of a rule takes, and the last
//! lines of the focused pane its form matches a pattern against.

use super::*;
use nekominal_model::triggers::{self, Action, Trigger};

/// How many lines of the pane the form of a rule shows.
const SAMPLE_LINES: usize = 3;

impl App {
    /// The last lines of the focused pane that are not blank, for the form of a rule to match its pattern against.
    pub(super) fn trigger_sample(&self) -> Vec<String> {
        let text = self.focused().map(|entry| entry.view.snapshot().text()).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).map(str::to_owned).collect();
        lines.drain(..lines.len().saturating_sub(SAMPLE_LINES));
        lines
    }

    /// Changes the rule at `index`, one past the last adding it, and keeps the settings.
    fn change_trigger(&mut self, index: usize, change: impl FnOnce(&mut Trigger)) {
        let before = self.settings.clone();
        let mut rule = settings_page::trigger_at(&self.settings, index);
        change(&mut rule);
        match self.settings.triggers.get_mut(index) {
            Some(slot) => *slot = rule,
            None => self.settings.triggers.push(rule),
        }
        self.apply_settings(&before);
    }

    /// Opens the rule at `index` in place, or closes the one open.
    fn open_trigger(&mut self, index: Option<usize>) {
        if let Some(page) = self.page.as_mut() {
            (page.trigger, page.error) = (index, None);
        }
    }

    /// What a press does on the Triggers page: add a rule, open or close one, switch it, choose for it, edit a field.
    pub(super) fn press_trigger_part(&mut self, part: Part) {
        let count = self.settings.triggers.len();
        let room = count < triggers::MAX_TRIGGERS;
        match part {
            // A new rule is a form of its own until its pattern is given: the file holds none without one.
            Part::TriggerAdd if room => {
                self.open_trigger(Some(count));
                self.start_profile_edit(settings_page::TRIGGER_PATTERN_ID, Some(String::new()), count);
            }
            Part::TriggerOpen(i) => {
                let open = self.page.as_ref().is_some_and(|page| page.trigger == Some(i));
                self.open_trigger((!open).then_some(i));
            }
            Part::TriggerSwitch(i) if i < count => self.change_trigger(i, |rule| rule.turn(!rule.is_on())),
            Part::TriggerInstant(i) if i < count => self.change_trigger(i, |rule| rule.instant = !rule.instant),
            Part::TriggerAction(i, k) if i < count => {
                if let Some(action) = Action::at(k).filter(|action| action.index() != self.settings.triggers[i].action.index()) {
                    self.change_trigger(i, |rule| rule.set_action(action));
                }
            }
            Part::TriggerColor(i, k) if i < count => {
                if let Some(color) = triggers::HIGHLIGHTS.get(k) {
                    self.change_trigger(i, |rule| rule.action = Action::Highlight { color: (*color).to_owned() });
                }
            }
            Part::TriggerPreset(k) if room => {
                if let Some(rule) = triggers::preset(k, self.language()) {
                    self.change_trigger(count, |slot| *slot = rule);
                    self.open_trigger(Some(count));
                }
            }
            Part::TriggerName(i) | Part::TriggerPattern(i) | Part::TriggerProgram(i) | Part::TriggerText(i) => {
                let Some((id, _)) = settings_page::trigger_field(part) else { return };
                let rule = settings_page::trigger_at(&self.settings, i);
                let text = match (id, &rule.action) {
                    (settings_page::TRIGGER_NAME_ID, _) => rule.name.clone(),
                    (settings_page::TRIGGER_PATTERN_ID, _) => rule.pattern.clone(),
                    (settings_page::TRIGGER_PROGRAM_ID, _) => rule.program.clone().unwrap_or_default(),
                    (_, Action::Notify { text }) => text.clone(),
                    (_, Action::Send { text, .. }) => settings_page::snippet_shown(text),
                    _ => return,
                };
                self.start_profile_edit(id, Some(text), i);
            }
            _ => {}
        }
        self.page_redraw();
    }

    /// What was typed in the field `id` of the rule at `index` becomes its name, pattern, program or text. A pattern that
    /// is not a regular expression, or a send without text, is refused; an empty pattern deletes the rule, and gives up
    /// the one being added.
    pub(super) fn commit_trigger(&mut self, id: &str, index: usize, text: &str) -> Result<(), Text> {
        let count = self.settings.triggers.len();
        if id == settings_page::TRIGGER_PATTERN_ID {
            if text.is_empty() {
                if index < count {
                    let before = self.settings.clone();
                    self.settings.triggers.remove(index);
                    self.apply_settings(&before);
                }
                self.open_trigger(None);
                return Ok(());
            }
            if triggers::pattern_problem(text).is_some() {
                return Err(Text::TriggerBadPattern);
            }
            let pattern = text.to_owned();
            self.change_trigger(index, |rule| rule.pattern = pattern);
            return Ok(());
        }
        // The other fields belong to a rule that is in the settings already.
        if index >= count {
            return Ok(());
        }
        let trimmed = text.trim().to_owned();
        match id {
            // A rule without a name is called by its pattern, as the file does.
            settings_page::TRIGGER_NAME_ID => self.change_trigger(index, |rule| rule.name = if trimmed.is_empty() { rule.pattern.clone() } else { trimmed }),
            settings_page::TRIGGER_PROGRAM_ID => self.change_trigger(index, |rule| rule.program = (!trimmed.is_empty()).then_some(trimmed)),
            settings_page::TRIGGER_TEXT_ID => match self.settings.triggers[index].action {
                Action::Send { .. } if settings_page::snippet_input(text).is_empty() => return Err(Text::ProfileNeedsValue),
                Action::Send { allow_send, .. } => {
                    let action = Action::Send { text: settings_page::snippet_input(text), allow_send };
                    self.change_trigger(index, |rule| rule.action = action);
                }
                Action::Notify { .. } => {
                    let action = Action::Notify { text: if text.is_empty() { "$0".to_owned() } else { text.to_owned() } };
                    self.change_trigger(index, |rule| rule.action = action);
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
}
