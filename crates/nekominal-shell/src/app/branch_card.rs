//! The branch card: under the branch of the title bar while the pointer is on it, what git says of the focused pane's
//! folder (`worktree::inspect`): the branch, whether the folder is the main checkout or a worktree, its changes with the
//! first of their files, whether its commits are pushed and whether it is merged, in the lines the dialog of Remove this
//! worktree starts with. git is asked on a thread of its own when the card opens, again when what it said is a few
//! seconds old.

use std::path::{Path, PathBuf};

use super::*;
use crate::design::RADIUS;
use crate::remove_dialog;
use crate::worktree;

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
/// Where the text of a line starts, after its icon.
const TEXT_LEFT: f32 = 22.0;
/// How long what git said stands before the card asks again.
const FRESH: Duration = Duration::from_secs(5);

#[derive(Clone)]
enum Answer {
    Status(worktree::Status),
    Failed,
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
            let answer = worktree::inspect(&root).map_or(Answer::Failed, Answer::Status);
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

        // What it says, laid out first so that the card is as tall as it.
        let pr = self.tabs.get(self.selected).and_then(|tab| self.badges.pull_request.get(&tab.uid)).map(|pr| pr.number);
        let (kind, checks, note) = match answer {
            Some(Answer::Status(status)) => {
                let kind = match (status.main, &status.branch) {
                    (_, None) => Text::BranchDetached,
                    (true, _) => Text::BranchMain,
                    (false, _) => Text::BranchWorktree,
                };
                (Some(kind), remove_dialog::status_checks(&status, pr, lang), None)
            }
            Some(Answer::Failed) => (None, Vec::new(), Some(Text::BranchFailed)),
            None => (None, Vec::new(), Some(Text::BranchReading)),
        };
        let home = std::env::var("USERPROFILE").ok();
        let folder = logic::folder_label(&git.root.to_string_lossy(), home.as_deref());
        let mut height = PAD + line + 4.0 + small + 12.0;
        for check in &checks {
            height += line + 6.0;
            if !check.files.is_empty() {
                height += check.files.len() as f32 * small + 12.0 + 6.0;
            }
        }
        if note.is_some() {
            height += line + 6.0;
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
        self.ui.push_clip(Rect::new(x, card.y, right - x, card.h));
        let mut y = card.y + PAD;

        // The branch, what the folder is at the right, and the folder under them.
        let kind_w = kind.map_or(0.0, |kind| self.ui.measure(kind.get(lang), SMALL_SIZE, false));
        self.ui.push_clip(Rect::new(x, y, (right - x - kind_w - 10.0).max(0.0), line));
        self.ui.text(x, y, &git.branch, TEXT_SIZE, text, true);
        self.ui.pop_clip();
        if let Some(kind) = kind {
            self.ui.text(right - kind_w, y + (line - small) / 2.0, kind.get(lang), SMALL_SIZE, quiet, false);
        }
        y += line + 4.0;
        self.ui.mono(x, y, &folder, SMALL_SIZE, quiet);
        y += small + 6.0;
        self.ui.fill(Rect::new(x, y, right - x, 1.0), col(g.divider()));
        y += 6.0;

        // A line each, as the dialog has them: its icon, its words, and the files under the line of changes.
        let icon_line = self.ui.line_height(SMALL_SIZE);
        for check in &checks {
            let (glyph, color) = remove_dialog::mark_glyph(&g, check.mark);
            self.ui.icon(x, y + (line - icon_line) / 2.0, glyph, SMALL_SIZE, color);
            let words = if check.mark == remove_dialog::Mark::Note { quiet } else { text };
            self.ui.text(x + TEXT_LEFT, y, &check.text, TEXT_SIZE, words, false);
            y += line + 6.0;
            if !check.files.is_empty() {
                let rect = Rect::new(x + TEXT_LEFT, y - 2.0, right - x - TEXT_LEFT, check.files.len() as f32 * small + 12.0);
                self.ui.rrect(rect, RADIUS, col(g.tab_rest()), None);
                for (i, (code, path)) in check.files.iter().enumerate() {
                    let row = rect.y + 6.0 + i as f32 * small;
                    self.ui.mono(rect.x + 10.0, row, &code.to_string(), SMALL_SIZE, col(g.mark(g.warning)));
                    self.ui.mono(rect.x + 28.0, row, path, SMALL_SIZE, quiet);
                }
                y += rect.h + 6.0;
            }
        }
        if let Some(note) = note {
            self.ui.text(x, y, note.get(lang), TEXT_SIZE, quiet, false);
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
}
