//! How a tab shows what its panes ask for: a glyph of Segoe Fluent Icons and a colour. The state itself (which pane
//! wins, what a bell is worth) is `logic::tab_status`.

use nekominal_model::themes::Color;

use crate::{glass::Glass, logic::TabState};

/// Progress lasts until the program clears it or returns control to the shell.
pub fn progress_after(current: Option<crate::logic::Reported>, event: &nekominal_pane::UiEvent) -> Option<crate::logic::Reported> {
    use crate::logic::Reported;
    use nekominal_pane::UiEvent;
    use nekominal_term::osc::Progress;
    match event {
        UiEvent::Progress(progress) => progress.map(|p| match p {
            Progress::Value { value, .. } => Reported::Percent(value.round() as u8),
            Progress::Indeterminate => Reported::Indeterminate,
        }),
        UiEvent::Exit(_) => None,
        UiEvent::PromptMark(mark) if nekominal_pane::host::Activity::after_mark(*mark) == nekominal_pane::host::Activity::AtPrompt => None,
        _ => current,
    }
}

/// The glyph that tells a state apart without its colour; none for a tab at rest.
pub fn glyph(state: TabState) -> Option<char> {
    match state {
        TabState::Idle => None,
        TabState::Working => Some('\u{E895}'),
        TabState::Blocked => Some('\u{E897}'),
        TabState::Done => Some('\u{E73E}'),
        TabState::Error => Some('\u{E7BA}'),
        TabState::Attention => Some('\u{EA8F}'),
    }
}

/// The colour of the glyph: the accent while the tab works, amber for what waits for the user, green for done and red
/// for an error, all kept legible against the scheme.
pub fn color(glass: Glass, state: TabState) -> Color {
    match state {
        TabState::Idle => glass.accent,
        TabState::Working => glass.mark(glass.accent),
        TabState::Blocked | TabState::Attention => glass.mark(glass.warning),
        TabState::Done => glass.mark(glass.success),
        TabState::Error => glass.mark(glass.danger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_ends_on_exit_or_a_prompt_but_not_during_command_output() {
        use crate::logic::Reported;
        use nekominal_pane::UiEvent;
        use nekominal_term::osc::{Progress, PromptMark};
        let busy = Some(Reported::Indeterminate);
        assert_eq!(progress_after(None, &UiEvent::Progress(Some(Progress::Indeterminate))), busy);
        assert_eq!(progress_after(busy, &UiEvent::Progress(None)), None);
        assert_eq!(progress_after(busy, &UiEvent::PromptMark(PromptMark::OutputStart)), busy);
        assert_eq!(progress_after(busy, &UiEvent::Bell), busy);
        for event in [UiEvent::Exit(0), UiEvent::Exit(1),
            UiEvent::PromptMark(PromptMark::PromptStart),
            UiEvent::PromptMark(PromptMark::CommandStart),
            UiEvent::PromptMark(PromptMark::CommandEnd { exit_code: Some(1) })] {
            assert_eq!(progress_after(busy, &event), None, "{event:?}");
        }
    }

    #[test]
    fn a_tab_at_rest_has_no_glyph_and_every_other_state_has_its_own() {
        let states = [TabState::Working, TabState::Blocked, TabState::Done, TabState::Error, TabState::Attention];
        assert_eq!(glyph(TabState::Idle), None);
        let glyphs: Vec<char> = states.iter().filter_map(|s| glyph(*s)).collect();
        assert_eq!(glyphs.len(), states.len());
        let mut unique = glyphs.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), glyphs.len(), "no two states share a glyph");
    }
}
