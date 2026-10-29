//! The branch card: under the branch of the title bar while the pointer is on it, what git says of the focused pane's
//! folder (`worktree::overview`). At the top where it pushes: the remote and how far the branch is from its upstream; then
//! the last commit; then its changes with the first of their files, the commits the remote has that it has not, whether it
//! is merged, its pull request and its stashes. When git cannot read the folder, what git said. git is asked on a thread
//! of its own when the card opens, again when what it said is a few seconds old.

use std::path::{Path, PathBuf};

use super::*;
use crate::design::RADIUS;
use crate::github;
use crate::remove_dialog::{self, Check, Mark};
use crate::worktree;

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
/// Where the text of a line starts, after its icon.
const TEXT_LEFT: f32 = 22.0;
/// The row of where it pushes and how far it is from there.
const SYNC: f32 = 30.0;
/// The lines of git's complaint it shows at most.
const ERROR_LINES: usize = 4;
/// How long what git said stands before the card asks again.
const FRESH: Duration = Duration::from_secs(5);

#[derive(Clone)]
enum Answer {
    Overview(Box<worktree::Overview>),
    /// What git said.
    Failed(String),
}

/// What git said of a folder, and whether it is being asked.
#[derive(Default)]
pub struct Read {
    root: Option<PathBuf>,
    asked: Option<Instant>,
    running: bool,
    answer: Option<Answer>,
    /// An answer came that the window has not drawn yet: `App::on_wake` takes it.
    pub answered: bool,
}

impl Read {
    /// Whether git is to be asked about `root` now: nothing is being asked, and it is another folder or what git said is
    /// no longer fresh.
    fn due(&self, root: &Path, now: Instant) -> bool {
        !self.running && (self.root.as_deref() != Some(root) || self.asked.is_none_or(|at| now.duration_since(at) >= FRESH))
    }
}

/// How long ago `at` (seconds since 1970) was, as the card says it.
fn ago(at: u64, now: u64, lang: Lang) -> String {
    match now.saturating_sub(at) {
        0..60 => Text::PushJustNow.get(lang).to_owned(),
        s @ 60..3_600 => Text::PushMinutesAgo.format(lang, &[&(s / 60)]),
        s @ 3_600..86_400 => Text::BranchHoursAgo.format(lang, &[&(s / 3_600)]),
        s => Text::BranchDaysAgo.format(lang, &[&(s / 86_400)]),
    }
}

/// The lines under the last commit: the changes, what the remote has that the branch has not, whether it is merged (not
/// for the default branch itself), the pull request and the stashes.
fn lines(seen: &worktree::Overview, pr: Option<&github::PullRequest>, lang: Lang) -> Vec<Check> {
    let status = &seen.status;
    let note = |text: String| Check { mark: Mark::Note, text, files: Vec::new() };
    let mut lines = vec![remove_dialog::changes_check(status, lang)];
    if let (Some(n @ 1..), Some(remote)) = (status.behind, &status.remote) {
        lines.push(Check { mark: Mark::Warn, text: Text::BranchBehind.format(lang, &[remote, &n]), files: Vec::new() });
    }
    let is_default = status.branch.as_ref().zip(status.default_branch.as_ref()).is_some_and(|(branch, default)| default == branch || default.ends_with(&format!("/{branch}")));
    if !is_default {
        lines.extend(remove_dialog::merge_check(status, pr.map(|pr| pr.number), lang));
    }
    if let Some(pr) = pr {
        let state = match pr.state {
            github::State::Open => Text::PrStateOpen,
            github::State::Draft => Text::PrStateDraft,
            github::State::Merged => Text::PrStateMerged,
            github::State::Closed => Text::PrStateClosed,
        };
        lines.push(note(format!("PR #{} \u{b7} {}", pr.number, state.get(lang))));
    }
    if seen.stashes > 0 {
        lines.push(note(Text::BranchStashes.format(lang, &[&seen.stashes])));
    }
    lines
}

impl App {
    /// Asks git about `root` on a thread of its own when it is due; the answer wakes this window.
    fn ask_git(&mut self, root: &Path) {
        let now = Instant::now();
        {
            let Ok(mut read) = self.branch_read.lock() else { return };
            if !read.due(root, now) {
                return;
            }
            // Another folder: what git said of the last one is not this one's.
            if read.root.as_deref() != Some(root) {
                read.answer = None;
            }
            (read.root, read.asked, read.running) = (Some(root.to_owned()), Some(now), true);
        }
        let (shared, root, hwnd) = (Arc::clone(&self.branch_read), root.to_owned(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let answer = match worktree::overview(&root) {
                Ok(seen) => Answer::Overview(Box::new(seen)),
                Err(said) => {
                    crate::util::log(&format!("branch card: git in {}: {said}", root.display()));
                    Answer::Failed(said)
                }
            };
            if let Ok(mut read) = shared.lock() {
                read.running = false;
                // The card went to another folder meanwhile: the answer is not its.
                if read.root.as_deref() == Some(root.as_path()) {
                    (read.answer, read.answered) = (Some(answer), true);
                }
            }
            // SAFETY: posting to a window that may be gone, which PostMessage reports and nothing else.
            unsafe { let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0)); }
        });
    }

    pub(super) fn paint_branch_card(&mut self) {
        // Open while the pointer is on the branch or on the card.
        self.branch_card_rect = None;
        if !matches!(self.hover, Hit::Branch | Hit::BranchCard) {
            return;
        }
        let (Some(anchor), Some(git)) = (self.branch_rect, self.focused().and_then(|entry| entry.git.clone())) else { return };
        self.ask_git(&git.root);
        let answer = self.branch_read.lock().ok().filter(|read| read.root.as_deref() == Some(git.root.as_path())).and_then(|read| read.answer.clone());
        let g = self.glass;
        let lang = self.language();
        let (text, quiet) = (col(g.text()), col(g.text_secondary()));
        let (line, small) = (self.ui.line_height(TEXT_SIZE), self.ui.line_height(SMALL_SIZE));
        let inner = WIDTH - 2.0 * PAD;
        let home = std::env::var("USERPROFILE").ok();
        let folder = logic::folder_label(&git.root.to_string_lossy(), home.as_deref());
        let pr = self.tabs.get(self.selected).and_then(|tab| self.badges.pull_request.get(&tab.uid)).cloned();

        // What it says, laid out first so that the card is as tall as it.
        let (kind, kind_color) = match &answer {
            Some(Answer::Overview(seen)) => (
                match (seen.status.main, &seen.status.branch) {
                    (_, None) => Text::BranchDetached,
                    (true, _) => Text::BranchMain,
                    (false, _) => Text::BranchWorktree,
                },
                quiet,
            ),
            Some(Answer::Failed(_)) => (Text::BranchFailed, col(g.mark(g.danger))),
            None => (Text::BranchReading, quiet),
        };
        let complaint = match &answer {
            Some(Answer::Failed(said)) => wrap(&mut self.ui, said, SMALL_SIZE, inner - 20.0, ERROR_LINES),
            _ => Vec::new(),
        };
        let checks = match &answer {
            Some(Answer::Overview(seen)) => lines(seen, pr.as_ref(), lang),
            _ => Vec::new(),
        };
        let mut height = PAD + line + 6.0;
        match &answer {
            Some(Answer::Overview(seen)) => {
                height += line + 6.0 + SYNC + 10.0;
                if seen.last.is_some() {
                    height += 1.0 + 10.0 + line + small + 10.0;
                }
                height += 1.0 + 8.0;
                for check in &checks {
                    height += line + 6.0;
                    if !check.files.is_empty() {
                        height += check.files.len() as f32 * small + 12.0 + 6.0;
                    }
                }
            }
            Some(Answer::Failed(_)) => height += small + 10.0 + complaint.len() as f32 * small + 16.0 + 4.0,
            None => height += small + 4.0,
        }
        height += PAD - 6.0;

        let width = self.layout.size.0;
        let mid = anchor.x + anchor.w / 2.0;
        let card = Rect::new((mid - 70.0).clamp(8.0, (width - WIDTH - 8.0).max(8.0)), layout::TITLE_HEIGHT + 4.0, WIDTH, height);
        // The gap between the branch and the card is the card's too: the pointer crosses it on the way down.
        self.branch_card_rect = Some(Rect::new(card.x, anchor.y + anchor.h, card.w, card.y + card.h - (anchor.y + anchor.h)));
        if self.panels_failed {
            self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        }
        self.ui.rrect(card, CORNER_RADIUS, self.panel_fill(252), Some((col(g.divider()), 1.0)));
        self.panel_shapes.push((card, CORNER_RADIUS));
        let (x, right) = (card.x + PAD, card.x + card.w - PAD);
        self.ui.push_clip(Rect::new(x, card.y, inner, card.h));
        let mut y = card.y + PAD;

        // The branch, and what the folder is (or that git could not read it) at the right.
        let kind_w = self.ui.measure(kind.get(lang), SMALL_SIZE, false);
        let branch = subagents_card::fit(&mut self.ui, &git.branch, TEXT_SIZE, true, inner - kind_w - 10.0);
        self.ui.text(x, y, &branch, TEXT_SIZE, text, true);
        self.ui.text(right - kind_w, y + (line - small) / 2.0, kind.get(lang), SMALL_SIZE, kind_color, false);
        y += line + 6.0;

        let seen = match answer {
            Some(Answer::Overview(seen)) => seen,
            Some(Answer::Failed(_)) => {
                self.ui.mono(x, y, &folder, SMALL_SIZE, quiet);
                y += small + 10.0;
                let danger = col(g.mark(g.danger));
                let box_h = complaint.len() as f32 * small + 16.0;
                let [r, gr, b, _] = danger.0;
                self.ui.rrect(Rect::new(x, y, inner, box_h), RADIUS, Color([r, gr, b, 26]), Some((Color([r, gr, b, 70]), 1.0)));
                for (n, row) in complaint.iter().enumerate() {
                    self.ui.text(x + 10.0, y + 8.0 + n as f32 * small, row, SMALL_SIZE, text, false);
                }
                self.ui.pop_clip();
                return;
            }
            None => {
                self.ui.mono(x, y, &folder, SMALL_SIZE, quiet);
                self.ui.pop_clip();
                return;
            }
        };

        // Where it pushes: the remote by its host and path, and the name git knows it by.
        let icon_line = self.ui.line_height(SMALL_SIZE);
        match (seen.url.as_deref().map(worktree::remote_place), seen.status.remote.as_deref()) {
            (Some((host, path)), _) => {
                self.ui.icon(x, y + (line - icon_line) / 2.0, '\u{E71B}', SMALL_SIZE, quiet);
                let where_ = [host, seen.status.remote.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ");
                let ww = self.ui.measure(&where_, SMALL_SIZE, false);
                self.ui.text(right - ww, y + (line - small) / 2.0, &where_, SMALL_SIZE, quiet, false);
                let path = subagents_card::fit(&mut self.ui, path, TEXT_SIZE, false, inner - TEXT_LEFT - ww - 10.0);
                self.ui.text(x + TEXT_LEFT, y, &path, TEXT_SIZE, col(g.accent), false);
            }
            // A remote whose address git does not give (only a push address set): its name.
            (None, Some(remote)) => {
                self.ui.icon(x, y + (line - icon_line) / 2.0, '\u{E71B}', SMALL_SIZE, quiet);
                self.ui.text(x + TEXT_LEFT, y, remote, TEXT_SIZE, col(g.accent), false);
            }
            (None, None) => {
                self.ui.text(x, y, Text::WorktreeNoRemote.get(lang), TEXT_SIZE, quiet, false);
            }
        }
        y += line + 6.0;

        // How far from its upstream: in step, or the commits each side has that the other has not.
        let sync = Rect::new(x, y, inner, SYNC);
        self.ui.rrect(sync, RADIUS, col(g.tab_rest()), None);
        let mid_y = y + (SYNC - line) / 2.0;
        let ahead = seen.status.unpushed.unwrap_or(0);
        match (&seen.upstream, seen.status.behind) {
            (Some(upstream), Some(0)) if ahead == 0 => {
                let ok = col(g.mark(g.success));
                self.ui.icon(x + 10.0, y + (SYNC - icon_line) / 2.0, '\u{E73E}', SMALL_SIZE, ok);
                let words = subagents_card::fit(&mut self.ui, &Text::BranchSynced.format(lang, &[upstream]), TEXT_SIZE, false, inner - 40.0);
                self.ui.text(x + 10.0 + TEXT_LEFT, mid_y, &words, TEXT_SIZE, ok, false);
            }
            (Some(upstream), behind) => {
                let mut at = x + 10.0;
                at += self.ui.mono(at, y + (SYNC - small) / 2.0, &format!("\u{2191}{ahead}"), SMALL_SIZE, col(g.accent)) + 10.0;
                at += self.ui.mono(at, y + (SYNC - small) / 2.0, &format!("\u{2193}{}", behind.unwrap_or(0)), SMALL_SIZE, col(g.mark(g.warning))) + 12.0;
                self.ui.push_clip(Rect::new(at, y, (right - 10.0 - at).max(0.0), SYNC));
                self.ui.mono(at, y + (SYNC - small) / 2.0, upstream, SMALL_SIZE, quiet);
                self.ui.pop_clip();
            }
            (None, _) => {
                let mut words = Text::BranchNoUpstream.get(lang).to_owned();
                if ahead > 0 {
                    words = format!("{words} \u{b7} {}", Text::WorktreeUnpushed.format(lang, &[&ahead]));
                }
                let words = subagents_card::fit(&mut self.ui, &words, TEXT_SIZE, false, inner - 20.0);
                self.ui.text(x + 10.0, mid_y, &words, TEXT_SIZE, quiet, false);
            }
        }
        y += SYNC + 10.0;

        // The last commit: its hash and subject, who made it and when.
        if let Some(last) = &seen.last {
            self.ui.fill(Rect::new(x, y, inner, 1.0), col(g.divider()));
            y += 1.0 + 10.0;
            let hw = self.ui.mono(x, y + (line - small) / 2.0, &last.hash, SMALL_SIZE, col(g.accent)) + 8.0;
            let subject = subagents_card::fit(&mut self.ui, &last.subject, TEXT_SIZE, false, inner - hw);
            self.ui.text(x + hw, y, &subject, TEXT_SIZE, text, false);
            y += line;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            self.ui.text(x, y, &format!("{} \u{b7} {}", last.author, ago(last.at, now, lang)), SMALL_SIZE, quiet, false);
            y += small + 10.0;
        }
        self.ui.fill(Rect::new(x, y, inner, 1.0), col(g.divider()));
        y += 1.0 + 8.0;

        // A line each, as the dialog of Remove this worktree has them, and the files under the line of changes.
        for check in &checks {
            let (glyph, color) = remove_dialog::mark_glyph(&g, check.mark);
            self.ui.icon(x, y + (line - icon_line) / 2.0, glyph, SMALL_SIZE, color);
            let words = if check.mark == Mark::Note { quiet } else { text };
            let shown = subagents_card::fit(&mut self.ui, &check.text, TEXT_SIZE, false, inner - TEXT_LEFT);
            self.ui.text(x + TEXT_LEFT, y, &shown, TEXT_SIZE, words, false);
            y += line + 6.0;
            if !check.files.is_empty() {
                let rect = Rect::new(x + TEXT_LEFT, y - 2.0, inner - TEXT_LEFT, check.files.len() as f32 * small + 12.0);
                self.ui.rrect(rect, RADIUS, col(g.tab_rest()), None);
                self.ui.push_clip(rect);
                for (i, (code, path)) in check.files.iter().enumerate() {
                    let row = rect.y + 6.0 + i as f32 * small;
                    self.ui.mono(rect.x + 10.0, row, &code.to_string(), SMALL_SIZE, col(g.mark(g.warning)));
                    self.ui.mono(rect.x + 28.0, row, path, SMALL_SIZE, quiet);
                }
                self.ui.pop_clip();
                y += rect.h + 6.0;
            }
        }
        self.ui.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_asked_again_for_another_folder_or_once_what_it_said_is_old_and_never_twice_at_once() {
        let (a, b) = (Path::new(r"C:\repo"), Path::new(r"C:\other"));
        let now = Instant::now();
        assert!(Read::default().due(a, now), "never asked");
        let read = Read { root: Some(a.to_owned()), asked: Some(now), ..Read::default() };
        assert!(!read.due(a, now + Duration::from_secs(1)), "fresh");
        assert!(read.due(a, now + FRESH), "old");
        assert!(read.due(b, now), "another folder");
        let running = Read { running: true, ..read };
        assert!(!running.due(b, now + FRESH), "one question at a time");
    }

    fn overview(branch: &str, default: &str) -> worktree::Overview {
        let status = worktree::Status {
            path: PathBuf::from(r"C:\repo"),
            branch: Some(branch.into()),
            main: true,
            changed: 0,
            untracked: 0,
            ignored: 0,
            files: Vec::new(),
            remote: Some("origin".into()),
            unpushed: Some(0),
            behind: Some(0),
            default_branch: Some(default.into()),
            merged: Some(true),
        };
        worktree::Overview { status, url: None, upstream: Some(format!("origin/{branch}")), last: None, stashes: 0 }
    }

    #[test]
    fn the_lines_say_what_the_remote_has_and_leave_out_the_merge_of_the_default_branch_into_itself() {
        let texts = |seen: &worktree::Overview| lines(seen, None, Lang::EnUs).into_iter().map(|c| c.text).collect::<Vec<_>>();
        assert_eq!(texts(&overview("master", "origin/master")), ["No uncommitted changes"], "master is not merged into itself");
        let mut behind = worktree::Overview { stashes: 2, ..overview("fix", "origin/master") };
        behind.status.behind = Some(3);
        assert_eq!(texts(&behind), ["No uncommitted changes", "origin has 3 new commits not pulled yet", "Merged into origin/master", "2 stashed"]);
    }

    #[test]
    fn an_age_is_said_in_the_largest_unit_it_fills() {
        let lang = Lang::EnUs;
        assert_eq!(ago(1_000, 1_030, lang), Text::PushJustNow.get(lang));
        assert_eq!(ago(1_000, 1_000 + 125, lang), Text::PushMinutesAgo.format(lang, &[&2]));
        assert_eq!(ago(0, 7_300, lang), "2 h ago");
        assert_eq!(ago(0, 3 * 86_400 + 5, lang), "3 days ago");
        assert_eq!(ago(2_000, 1_000, lang), Text::PushJustNow.get(lang), "a clock behind the commit is no future");
    }
}
