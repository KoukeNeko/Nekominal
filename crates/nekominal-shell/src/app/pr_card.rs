//! The card of a pull request: under its badge (on a row of the list, or in the title bar) while the pointer is on the
//! badge or on the card. What gh said of it (`github.rs`): its number, state and title, its branches, its lines and who
//! opened it, its checks with the names of those that failed, its review and what keeps it from merging; and three
//! buttons, to open it on GitHub, to copy its link, and to dismiss its badge.

use super::*;
use crate::github;

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
/// The lines of the title it shows at most.
const TITLE_LINES: usize = 3;
/// The names of failing checks it lists at most.
const FAILING: usize = 3;
const BUTTON: f32 = 26.0;

/// What keeps a pull request from merging, from GitHub's `mergeStateStatus`; none when nothing does or GitHub does not say.
fn merge_blocker(state: Option<&str>) -> Option<Text> {
    match state? {
        "BEHIND" => Some(Text::PrBehind),
        "DIRTY" => Some(Text::PrConflicts),
        "BLOCKED" => Some(Text::PrBlocked),
        _ => None,
    }
}

/// The review GitHub asks for or has, from its `reviewDecision`.
fn review(decision: Option<&str>) -> Option<Text> {
    match decision? {
        "APPROVED" => Some(Text::PrApproved),
        "CHANGES_REQUESTED" => Some(Text::PrChangesRequested),
        "REVIEW_REQUIRED" => Some(Text::PrReviewRequired),
        _ => None,
    }
}

impl App {
    /// The pull request the card shows, and the badge it hangs from: the badge under the pointer when it is a pull
    /// request's, else the one it showed while the pointer is on the card.
    fn pr_card_target(&mut self) -> Option<(github::PullRequest, Rect)> {
        let shown = match self.hover {
            Hit::Badge(k) => self.badge_hits.get(k).and_then(|(_, chip)| chip.tab).map(|tab| (tab, k)),
            Hit::PrCard | Hit::PrOpen | Hit::PrCopy | Hit::PrDismiss => self.pr_card,
            _ => None,
        };
        self.pr_card = shown;
        // The badge it hangs from: the one the pointer was on (a tab has one on its row and one in the title bar).
        let (tab, k) = shown?;
        let anchor = self.badge_hits.get(k).filter(|(_, chip)| chip.tab == Some(tab)).map(|(rect, _)| *rect)?;
        Some((self.badges.pull_request.get(&tab)?.clone(), anchor))
    }

    pub(super) fn paint_pr_card(&mut self) {
        (self.pr_card_rect, self.pr_buttons) = (None, None);
        let Some((pr, anchor)) = self.pr_card_target() else { return };
        let g = self.glass;
        let lang = self.language();
        let (text, quiet) = (col(g.text()), col(g.text_secondary()));
        let (line, small) = (self.ui.line_height(TEXT_SIZE), self.ui.line_height(SMALL_SIZE));
        let inner = WIDTH - 2.0 * PAD;

        // What it says, laid out first so that the card is as tall as it.
        let (state, state_color) = match pr.state {
            github::State::Open => (Text::PrStateOpen, g.mark(g.success)),
            github::State::Draft => (Text::PrStateDraft, g.text_secondary()),
            github::State::Merged => (Text::PrStateMerged, g.mark(g.magenta)),
            github::State::Closed => (Text::PrStateClosed, g.mark(g.danger)),
        };
        let title = if pr.title.is_empty() { Vec::new() } else { wrap(&mut self.ui, &pr.title, TEXT_SIZE, inner, TITLE_LINES) };
        let branches = pr.head.as_deref().zip(pr.base.as_deref()).map(|(head, base)| format!("{head} \u{2192} {base}"));
        let lines = pr.additions.zip(pr.deletions);
        let author = pr.author.as_deref().map(|login| Text::PrBy.format(lang, &[&login]));
        let github::Checks { passing, failing, pending } = pr.checks;
        let counts: Vec<(String, Color)> = [
            (passing, Text::PrChecksPassed, g.mark(g.success)),
            (failing, Text::PrChecksFailed, g.mark(g.danger)),
            (pending, Text::PrChecksRunning, g.mark(g.warning)),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(n, words, color)| (words.format(lang, &[&n]), col(color)))
        .collect();
        let failed: Vec<&String> = pr.failing.iter().take(FAILING).collect();
        let review = review(pr.review.as_deref()).map(|words| {
            let color = match words {
                Text::PrApproved => g.mark(g.success),
                Text::PrChangesRequested => g.mark(g.danger),
                _ => g.mark(g.warning),
            };
            (words.get(lang), col(color))
        });
        let blocker = merge_blocker(pr.merge_state.as_deref()).filter(|_| matches!(pr.state, github::State::Open | github::State::Draft));

        let mut height = PAD + line + 6.0 + title.len() as f32 * line;
        height += (usize::from(branches.is_some()) + usize::from(lines.is_some() || author.is_some())) as f32 * (small + 4.0);
        if !counts.is_empty() {
            height += 12.0 + line + failed.len() as f32 * (small + 4.0);
        }
        height += (usize::from(review.is_some()) + usize::from(blocker.is_some())) as f32 * (small + 6.0);
        height += 14.0 + BUTTON + PAD;

        // Where every card of the bar hangs.
        let (card, hit) = bar_card(anchor, (WIDTH, height), self.layout.size);
        self.pr_card_rect = Some(hit);
        if self.panels_failed {
            self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        }
        self.ui.rrect(card, CORNER_RADIUS, self.panel_fill(252), Some((col(g.divider()), 1.0)));
        self.panel_shapes.push((card, CORNER_RADIUS));
        let (x, right) = (card.x + PAD, card.x + card.w - PAD);
        let mut y = card.y + PAD;

        // The number and the state, the repository at the right.
        let number = format!("#{}", pr.number);
        let nw = self.ui.text(x, y, &number, TEXT_SIZE, text, true);
        let state_words = state.get(lang);
        let dot = Rect::new(x + nw + 10.0, y + line / 2.0 - 3.0, 6.0, 6.0);
        self.ui.rrect(dot, 3.0, col(state_color), None);
        self.ui.text(dot.x + 12.0, y + (line - small) / 2.0, state_words, SMALL_SIZE, col(state_color), false);
        if let Some(repo) = self.pr_card.and_then(|(tab, _)| self.badges.repo.get(&tab)).map(badges::Repo::name) {
            let rw = self.ui.measure(&repo, SMALL_SIZE, false);
            self.ui.text(right - rw, y + (line - small) / 2.0, &repo, SMALL_SIZE, quiet, false);
        }
        y += line + 6.0;
        for row in &title {
            self.ui.text(x, y, row, TEXT_SIZE, text, true);
            y += line;
        }
        if let Some(branches) = &branches {
            self.ui.text(x, y + 2.0, branches, SMALL_SIZE, quiet, false);
            y += small + 4.0;
        }
        if lines.is_some() || author.is_some() {
            let mut at = x;
            if let Some((added, deleted)) = lines {
                at += self.ui.text(at, y + 2.0, &format!("+{added}"), SMALL_SIZE, col(g.mark(g.success)), false) + 6.0;
                at += self.ui.text(at, y + 2.0, &format!("\u{2212}{deleted}"), SMALL_SIZE, col(g.mark(g.danger)), false) + 6.0;
            }
            if let Some(author) = &author {
                let author = if lines.is_some() { format!("\u{b7} {author}") } else { author.clone() };
                self.ui.text(at, y + 2.0, &author, SMALL_SIZE, quiet, false);
            }
            y += small + 4.0;
        }

        // The checks: how many stand how, and the names of those that failed.
        if !counts.is_empty() {
            y += 6.0;
            self.ui.fill(Rect::new(x, y, right - x, 1.0), col(g.divider()));
            y += 6.0;
            self.ui.text(x, y, Text::PrChecks.get(lang), TEXT_SIZE, text, false);
            let mut at = right;
            for (i, (words, color)) in counts.iter().enumerate().rev() {
                let w = self.ui.measure(words, SMALL_SIZE, false);
                at -= w;
                self.ui.text(at, y + (line - small) / 2.0, words, SMALL_SIZE, *color, false);
                if i > 0 {
                    let sep = self.ui.measure(" \u{b7} ", SMALL_SIZE, false);
                    at -= sep;
                    self.ui.text(at, y + (line - small) / 2.0, " \u{b7} ", SMALL_SIZE, quiet, false);
                }
            }
            y += line;
            for name in &failed {
                let row = format!("\u{2717} {name}");
                let shown = wrap(&mut self.ui, &row, SMALL_SIZE, inner, 1).into_iter().next().unwrap_or_default();
                self.ui.text(x, y + 2.0, &shown, SMALL_SIZE, col(g.mark(g.danger)), false);
                y += small + 4.0;
            }
        }
        for (words, color) in review.into_iter().chain(blocker.map(|t| (t.get(lang), col(g.mark(g.warning))))) {
            let dot = Rect::new(x, y + 3.0 + small / 2.0 - 3.0, 6.0, 6.0);
            self.ui.rrect(dot, 3.0, color, None);
            self.ui.text(x + 12.0, y + 3.0, words, SMALL_SIZE, text, false);
            y += small + 6.0;
        }

        // The buttons, from the right: dismiss, copy the link, open it.
        y += 14.0;
        let labels = [Text::PrOpenOnGitHub.get(lang), Text::CardCopyLink.get(lang), Text::PrDismiss.get(lang)];
        let widths = labels.map(|label| self.ui.measure(label, SMALL_SIZE, false) + 20.0);
        let dismiss = Rect::new(right - widths[2], y, widths[2], BUTTON);
        let copy = Rect::new(dismiss.x - 6.0 - widths[1], y, widths[1], BUTTON);
        let open = Rect::new(copy.x - 6.0 - widths[0], y, widths[0], BUTTON);
        for ((rect, label), hit) in [open, copy, dismiss].into_iter().zip(labels).zip([Hit::PrOpen, Hit::PrCopy, Hit::PrDismiss]) {
            widgets::button(&mut self.ui, &g, rect, State { hover: self.hover == hit, pressed: self.pressed == hit, chosen: false });
            let lw = self.ui.measure(label, SMALL_SIZE, false);
            self.ui.text(rect.x + (rect.w - lw) / 2.0, y + (BUTTON - small) / 2.0, label, SMALL_SIZE, text, false);
        }
        self.pr_buttons = Some([open, copy, dismiss]);
    }

    /// A button of the card was pressed: open the pull request, copy its link, or dismiss its badge.
    pub(super) fn pr_card_action(&mut self, hit: Hit) {
        let Some(url) = self.pr_card.and_then(|(tab, _)| self.badges.pull_request.get(&tab)).map(|pr| pr.url.clone()) else { return };
        match hit {
            Hit::PrOpen => open_link(&url),
            Hit::PrCopy => {
                clipboard::write_text(&url);
            }
            Hit::PrDismiss => {
                self.badges.dismissed.insert(url);
                self.pr_card = None;
                self.relayout();
                self.redraw();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_of_the_bar_has_its_top_at_one_height_whatever_opens_it_and_hangs_70_before_the_middle_of_it() {
        let window = (525.0, 600.0);
        // A badge, the meter and the branch sit differently high in the bar: their cards do not.
        for anchor in [Rect::new(62.0, 9.0, 40.0, 22.0), Rect::new(300.0, 12.0, 70.0, 16.0), Rect::new(200.0, 6.0, 60.0, 28.0)] {
            let (card, gap) = bar_card(anchor, (340.0, 200.0), window);
            assert_eq!(card.y, layout::TITLE_HEIGHT + 4.0, "{anchor:?}");
            assert_eq!((gap.y, gap.y + gap.h), (anchor.y + anchor.h, card.y + card.h), "the gap to the anchor is the card's too");
        }
        assert_eq!(bar_card(Rect::new(200.0, 6.0, 40.0, 28.0), (340.0, 200.0), window).0.x, 200.0 + 20.0 - 70.0);
        assert_eq!(bar_card(Rect::new(480.0, 6.0, 40.0, 28.0), (340.0, 200.0), window).0.x, 525.0 - 340.0 - 8.0, "kept inside on the right");
        assert_eq!(bar_card(Rect::new(5.0, 6.0, 40.0, 28.0), (340.0, 200.0), window).0.x, 8.0, "and on the left");
    }

    #[test]
    fn a_card_hangs_12_before_what_opens_it_and_6_below_or_above_it_when_there_is_no_room_and_inside_the_window() {
        let window = (525.0, 600.0);
        let (card, hit) = hang(Rect::new(62.0, 10.0, 40.0, 28.0), (340.0, 200.0), window);
        assert_eq!((card.x, card.y), (50.0, 44.0), "12 before it, 6 below it");
        assert_eq!((hit.x, hit.y, hit.h), (50.0, 38.0, 206.0), "the gap to the anchor is the card's too");
        let (card, hit) = hang(Rect::new(62.0, 500.0, 40.0, 28.0), (340.0, 200.0), window);
        assert_eq!(card.y, 294.0, "over it, 6 above, when it would run out of the window below");
        assert_eq!((hit.y, hit.h), (294.0, 206.0));
        assert_eq!(hang(Rect::new(480.0, 10.0, 40.0, 28.0), (340.0, 200.0), window).0.x, 525.0 - 340.0 - 8.0, "kept inside on the right");
        assert_eq!(hang(Rect::new(5.0, 10.0, 40.0, 28.0), (340.0, 200.0), window).0.x, 8.0, "and on the left");
    }

    #[test]
    fn the_card_names_what_keeps_a_pull_request_from_merging_and_its_review() {
        assert_eq!(merge_blocker(Some("BEHIND")), Some(Text::PrBehind));
        assert_eq!(merge_blocker(Some("DIRTY")), Some(Text::PrConflicts));
        assert_eq!(merge_blocker(Some("BLOCKED")), Some(Text::PrBlocked));
        assert_eq!(merge_blocker(Some("CLEAN")), None, "nothing to say of one that can merge");
        assert_eq!(merge_blocker(None), None);
        assert_eq!(review(Some("APPROVED")), Some(Text::PrApproved));
        assert_eq!(review(Some("CHANGES_REQUESTED")), Some(Text::PrChangesRequested));
        assert_eq!(review(Some("REVIEW_REQUIRED")), Some(Text::PrReviewRequired));
        assert_eq!(review(Some("")), None);
    }
}
