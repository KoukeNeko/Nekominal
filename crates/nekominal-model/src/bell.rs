//! What the bell of a program (BEL) does: whether the tab shows an attention mark, the taskbar button flashes and
//! a sound plays. The decision is a pure function of the settings and of where the bell rang, so it needs no
//! window to be tested.
//!
//! Nothing else in the interface is audible: this is the only caller-visible source of a sound, and the default
//! settings keep it silent.

use std::time::Duration;

use crate::settings::{Bell, BellSound};

/// A taskbar flash or a sound happens at most once in this time per window; the first one is immediate. A program
/// that prints a binary file would otherwise ring the whole screen.
pub const ALERT_INTERVAL: Duration = Duration::from_secs(1);

/// Where the bell rang.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BellContext {
    pub window_active: bool,
    /// The pane is on screen and has the focus: its tab is selected, it is not hidden by a zoomed sibling.
    pub pane_in_view: bool,
    /// Time since this window last flashed its taskbar button or played a sound; `None` if it never did.
    pub since_last_alert: Option<Duration>,
    /// The system asks for sounds to be shown (`SM_SHOWSOUNDS`).
    pub show_sounds: bool,
}

/// What a bell causes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BellEffects {
    /// The tab gets an attention mark.
    pub chip: bool,
    pub taskbar_flash: bool,
    pub sound: bool,
}

pub fn bell_effects(settings: &Bell, context: &BellContext) -> BellEffects {
    // The user is looking at the pane that rang: there is nothing to point them to.
    let watching = context.window_active && context.pane_in_view;
    let mut chip = settings.visual && !watching;
    let mut flash = settings.taskbar && !context.window_active;
    let mut play = match settings.sound {
        BellSound::None => false,
        BellSound::Background => !watching,
        BellSound::Always => true,
    };
    // A system that shows sounds wants every audible alert to have a visual equivalent.
    if play && context.show_sounds {
        chip |= !watching;
        flash |= !context.window_active;
    }
    if !alert_due(context.since_last_alert) {
        flash = false;
        play = false;
    }
    BellEffects {
        chip,
        taskbar_flash: flash,
        sound: play,
    }
}

/// Whether a taskbar flash or a sound may happen now: the first is immediate, then one in each [`ALERT_INTERVAL`].
pub fn alert_due(since_last_alert: Option<Duration>) -> bool {
    since_last_alert.is_none_or(|since| since >= ALERT_INTERVAL)
}

/// Whether the taskbar button flashes for a tab that has started to need a response (an agent that waits for an answer
/// or failed, a notification). Like a bell it flashes only while the window is not active, and it shares the
/// limit of one a second with the bell and the sound. The setting is the one of the bell: it is the taskbar alert.
pub fn flash_for_response(
    taskbar: bool,
    window_active: bool,
    since_last_alert: Option<Duration>,
) -> bool {
    taskbar && !window_active && alert_due(since_last_alert)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WATCHING: BellContext = BellContext {
        window_active: true,
        pane_in_view: true,
        since_last_alert: None,
        show_sounds: false,
    };
    const HIDDEN_TAB: BellContext = BellContext {
        pane_in_view: false,
        ..WATCHING
    };
    const INACTIVE_WINDOW: BellContext = BellContext {
        window_active: false,
        ..WATCHING
    };
    const NOTHING: BellEffects = BellEffects {
        chip: false,
        taskbar_flash: false,
        sound: false,
    };

    fn settings(visual: bool, taskbar: bool, sound: BellSound) -> Bell {
        Bell {
            visual,
            taskbar,
            sound,
        }
    }

    #[test]
    fn the_pane_being_looked_at_rings_for_nobody() {
        for sound in [BellSound::None, BellSound::Background] {
            assert_eq!(
                bell_effects(&settings(true, true, sound), &WATCHING),
                NOTHING
            );
        }
    }

    #[test]
    fn by_default_a_hidden_tab_gets_a_mark_and_an_inactive_window_a_flash_and_nothing_is_heard() {
        let defaults = Bell {
            visual: true,
            taskbar: true,
            sound: BellSound::None,
        };
        assert_eq!(
            bell_effects(&defaults, &HIDDEN_TAB),
            BellEffects {
                chip: true,
                taskbar_flash: false,
                sound: false
            }
        );
        assert_eq!(
            bell_effects(&defaults, &INACTIVE_WINDOW),
            BellEffects {
                chip: true,
                taskbar_flash: true,
                sound: false
            }
        );
        let hidden_and_inactive = BellContext {
            window_active: false,
            pane_in_view: false,
            ..WATCHING
        };
        assert_eq!(
            bell_effects(&defaults, &hidden_and_inactive),
            BellEffects {
                chip: true,
                taskbar_flash: true,
                sound: false
            }
        );
    }

    #[test]
    fn the_taskbar_flashes_only_while_the_window_is_not_active() {
        let flash =
            |context| bell_effects(&settings(false, true, BellSound::None), &context).taskbar_flash;
        assert!(!flash(WATCHING) && !flash(HIDDEN_TAB));
        assert!(flash(INACTIVE_WINDOW));
        assert!(!bell_effects(&settings(true, false, BellSound::None), &INACTIVE_WINDOW).taskbar_flash);
    }

    #[test]
    fn a_sound_needs_the_setting_and_background_stays_out_of_the_pane_being_watched() {
        let heard = |sound, context| bell_effects(&settings(true, true, sound), &context).sound;
        for context in [WATCHING, HIDDEN_TAB, INACTIVE_WINDOW] {
            assert!(!heard(BellSound::None, context));
        }
        assert!(!heard(BellSound::Background, WATCHING));
        assert!(heard(BellSound::Background, HIDDEN_TAB));
        assert!(heard(BellSound::Background, INACTIVE_WINDOW));
        assert!(heard(BellSound::Always, WATCHING));
    }

    #[test]
    fn taskbar_and_sound_are_limited_to_one_per_second_the_first_at_once_and_the_mark_is_not() {
        let loud = settings(true, true, BellSound::Always);
        let at = |millis| BellContext {
            since_last_alert: Some(Duration::from_millis(millis)),
            ..INACTIVE_WINDOW
        };
        assert_eq!(
            bell_effects(&loud, &INACTIVE_WINDOW),
            BellEffects {
                chip: true,
                taskbar_flash: true,
                sound: true
            }
        );
        assert_eq!(
            bell_effects(&loud, &at(0)),
            BellEffects {
                chip: true,
                taskbar_flash: false,
                sound: false
            }
        );
        assert_eq!(
            bell_effects(&loud, &at(999)),
            BellEffects {
                chip: true,
                taskbar_flash: false,
                sound: false
            }
        );
        assert_eq!(
            bell_effects(&loud, &at(1000)),
            BellEffects {
                chip: true,
                taskbar_flash: true,
                sound: true
            }
        );
    }

    #[test]
    fn a_new_response_flashes_the_taskbar_of_an_inactive_window_once_a_second() {
        let second = ALERT_INTERVAL;
        let just_before = ALERT_INTERVAL - Duration::from_millis(1);
        // Needs the setting and a window that is not active.
        assert!(flash_for_response(true, false, None));
        assert!(!flash_for_response(false, false, None));
        assert!(!flash_for_response(true, true, None));
        // The first is immediate, then one in each interval, bells and sounds counted in.
        assert!(!flash_for_response(true, false, Some(Duration::ZERO)));
        assert!(!flash_for_response(true, false, Some(just_before)));
        assert!(flash_for_response(true, false, Some(second)));
        assert!(alert_due(None) && !alert_due(Some(just_before)) && alert_due(Some(second)));
    }

    #[test]
    fn when_the_system_shows_sounds_an_audible_alert_has_a_visual_equivalent() {
        let show = |context: BellContext| BellContext {
            show_sounds: true,
            ..context
        };
        let silent_visuals = settings(false, false, BellSound::Background);
        assert_eq!(
            bell_effects(&silent_visuals, &show(HIDDEN_TAB)),
            BellEffects {
                chip: true,
                taskbar_flash: false,
                sound: true
            }
        );
        assert_eq!(
            bell_effects(&silent_visuals, &show(INACTIVE_WINDOW)),
            BellEffects {
                chip: true,
                taskbar_flash: true,
                sound: true
            }
        );
        // Without the system setting the same bell has no picture.
        assert_eq!(
            bell_effects(&silent_visuals, &HIDDEN_TAB),
            BellEffects {
                chip: false,
                taskbar_flash: false,
                sound: true
            }
        );
    }
}
