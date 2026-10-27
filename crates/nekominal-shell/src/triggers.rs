//! The actions of the triggers (`settings.json` `triggers`): the emulator matches the lines and highlights
//! (`nekominal_term::triggers`); a hit comes here as a `UiEvent::Trigger` and becomes a bell, a notification, a mark on
//! the tab or text sent to the program.

use std::{sync::Arc, time::Instant};

use nekominal_model::triggers::{substitute, Action, SendGuard, Trigger};
use nekominal_pane::UiEvent;
use nekominal_term::{
    osc::Notification,
    triggers::{TriggerHit, TriggerRule, Triggers},
};

use crate::{logic::Attention, tab::PaneEntry};

/// The rules the emulator matches: the enabled triggers, each known by its index. None when there are none.
pub fn rules(triggers: &[Trigger]) -> Option<Arc<Triggers>> {
    let rules: Vec<TriggerRule> = triggers
        .iter()
        .enumerate()
        .filter(|(_, trigger)| trigger.enabled)
        .map(|(id, trigger)| TriggerRule {
            id,
            pattern: trigger.pattern.clone(),
            instant: trigger.instant,
            program: trigger.program.clone(),
            highlight: trigger.action.highlight().map(|color| (color.r, color.g, color.b)),
        })
        .collect();
    if rules.is_empty() {
        return None;
    }
    // The patterns were checked when the settings were read, so this fails only past a limit of the regex crate.
    Triggers::new(rules).map_err(|e| crate::util::log(&format!("triggers: {e}"))).ok().map(Arc::new)
}

/// What a hit does.
#[derive(Debug, PartialEq)]
enum Effect {
    Nothing,
    Bell,
    Notify { title: String, body: String },
    Badge(String),
    Send(String),
}

/// What `hit` does under `triggers`. A hit of a rule that has since changed does nothing, and a send needs the rule's
/// `allowSend` and the pane's `guard` (the emulator already fires a rule at most once per line).
fn effect(hit: &TriggerHit, triggers: &[Trigger], guard: &mut SendGuard, now: Instant) -> Effect {
    let Some(trigger) = triggers.get(hit.rule).filter(|trigger| trigger.enabled && *trigger.pattern == *hit.pattern) else {
        return Effect::Nothing;
    };
    match &trigger.action {
        Action::Highlight { .. } => Effect::Nothing,
        Action::Notify { text } => Effect::Notify { title: trigger.name.clone(), body: substitute(text, &hit.groups) },
        Action::Badge => Effect::Badge(trigger.name.clone()),
        Action::Send { text, allow_send: true } if guard.allow(now, hit.rule) => Effect::Send(text.clone()),
        Action::Send { .. } => Effect::Nothing,
        Action::Bell => Effect::Bell,
    }
}

/// Does what a trigger's hit asks of the pane in `entry`, or hands back the event that does it for the rest of the
/// output's handling (a bell, a notification); any other event comes back as it is. `watched`: the user looks at the
/// tab, which then gets no mark.
pub fn take(event: UiEvent, triggers: &[Trigger], entry: &mut PaneEntry, watched: bool) -> Option<UiEvent> {
    let UiEvent::Trigger(hit) = event else { return Some(event) };
    match effect(&hit, triggers, &mut entry.sends, Instant::now()) {
        Effect::Nothing => None,
        Effect::Bell => Some(UiEvent::Bell),
        Effect::Notify { title, body } => Some(UiEvent::Notification(Notification { title, body })),
        Effect::Badge(name) => {
            if !watched && entry.attention != Some(Attention::Exit) {
                entry.attention = Some(Attention::Notification);
                entry.note = Some(name);
            }
            None
        }
        Effect::Send(text) => {
            entry.pane.send(text.into_bytes());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nekominal_model::settings::parse;

    use super::*;

    fn triggers() -> Vec<Trigger> {
        parse(Some(
            r##"{ "triggers": [
                { "name": "fail", "pattern": "FAIL (\\w+)", "action": "notify", "text": "$1 failed" },
                { "pattern": "\\[y/N\\]", "action": "send", "text": "y\r", "allowSend": true },
                { "pattern": "Proceed", "action": "send", "text": "y\r" },
                { "pattern": "ding", "action": "bell" },
                { "name": "done", "pattern": "done", "action": "badge" },
                { "pattern": "off", "action": "bell", "enabled": false },
                { "pattern": "x", "action": "highlight", "color": "#102030" }
            ] }"##,
        ))
        .settings
        .triggers
    }

    fn hit(rule: usize, pattern: &str, groups: &[&str]) -> TriggerHit {
        TriggerHit { rule, pattern: Arc::from(pattern), groups: groups.iter().map(|g| (*g).to_owned()).collect() }
    }

    #[test]
    fn the_enabled_triggers_become_the_emulators_rules_by_index() {
        let rules = rules(&triggers()).unwrap();
        let ids: Vec<usize> = rules.rules().iter().map(|rule| rule.id).collect();
        assert_eq!(ids, [0, 1, 2, 3, 4, 6], "the disabled one is left out and the rest keep their index");
        assert_eq!(rules.rules()[5].highlight, Some((0x10, 0x20, 0x30)));
        assert!(super::rules(&[]).is_none());
    }

    #[test]
    fn each_action_does_its_own_thing() {
        let (triggers, mut guard, now) = (triggers(), SendGuard::default(), Instant::now());
        let mut effect = |hit: TriggerHit| effect(&hit, &triggers, &mut guard, now);
        assert_eq!(effect(hit(0, r"FAIL (\w+)", &["FAIL build", "build"])), Effect::Notify { title: "fail".into(), body: "build failed".into() });
        assert_eq!(effect(hit(3, "ding", &["ding"])), Effect::Bell);
        assert_eq!(effect(hit(4, "done", &["done"])), Effect::Badge("done".into()));
        assert_eq!(effect(hit(6, "x", &["x"])), Effect::Nothing, "the emulator drew it");
        assert_eq!(effect(hit(5, "off", &["off"])), Effect::Nothing, "a hit of a rule turned off since");
        assert_eq!(effect(hit(3, "dong", &["dong"])), Effect::Nothing, "a hit of a rule changed since");
        assert_eq!(effect(hit(42, "ding", &["ding"])), Effect::Nothing);
    }

    #[test]
    fn a_send_needs_the_rule_to_allow_it_and_is_rate_limited() {
        let (triggers, mut guard, now) = (triggers(), SendGuard::default(), Instant::now());
        assert_eq!(effect(&hit(2, "Proceed", &["Proceed"]), &triggers, &mut guard, now), Effect::Nothing, "no allowSend");
        let yes = hit(1, r"\[y/N\]", &["[y/N]"]);
        assert_eq!(effect(&yes, &triggers, &mut guard, now), Effect::Send("y\r".into()));
        assert_eq!(effect(&yes, &triggers, &mut guard, now + Duration::from_millis(100)), Effect::Nothing, "too soon");
        assert_eq!(effect(&yes, &triggers, &mut guard, now + Duration::from_secs(3)), Effect::Send("y\r".into()));
    }

    #[test]
    fn all_rules_off_is_no_matcher_at_all() {
        let mut off = triggers();
        off.iter_mut().for_each(|rule| rule.enabled = false);
        assert!(rules(&off).is_none(), "nothing to match costs nothing");
    }

    #[test]
    fn a_rule_that_matches_what_it_sent_is_held_to_the_pane_limit() {
        // The program echoes the answer and asks again at once, line after line, for a minute.
        let (triggers, mut guard, now) = (triggers(), SendGuard::default(), Instant::now());
        let yes = hit(1, r"\[y/N\]", &["[y/N]"]);
        let sent = (0..600).filter(|k| effect(&yes, &triggers, &mut guard, now + Duration::from_millis(100 * k)) != Effect::Nothing).count();
        assert_eq!(sent, nekominal_model::triggers::SEND_BURST);
    }

    #[test]
    fn a_queued_hit_does_what_its_rule_does_now() {
        let (mut triggers, mut guard, now) = (triggers(), SendGuard::default(), Instant::now());
        let queued = hit(0, r"FAIL (\w+)", &["FAIL build", "build"]);
        triggers[0].action = Action::Badge;
        assert_eq!(effect(&queued, &triggers, &mut guard, now), Effect::Badge("fail".into()), "the pattern is the same: the rule as it is now");
        // A hit is known by its rule's index and pattern: another rule of the same pattern that took the index acts.
        triggers[0] = Trigger { name: "other".into(), pattern: r"FAIL (\w+)".into(), enabled: true, action: Action::Badge, program: None, instant: false };
        assert_eq!(effect(&queued, &triggers, &mut guard, now), Effect::Badge("other".into()));
        // A notification without the group it names says nothing there.
        let notify = Trigger { action: Action::Notify { text: "[$2]".into() }, ..triggers[0].clone() };
        triggers[0] = notify;
        assert_eq!(effect(&queued, &triggers, &mut guard, now), Effect::Notify { title: "other".into(), body: "[]".into() });
    }
}
