//! The dialog of "Remove this worktree": what was checked (changes, pushed commits, merged branch, the tabs and the agent in
//! the folder), the files changed, the choice between deleting the branch with the folder and keeping it, and Cancel beside
//! the main button. A worktree with changes or commits not pushed offers neither choice and no force: the main button shows
//! the tab in the folder instead.
//!
//! [`model`] turns what git said into the dialog; [`place`] and [`paint`] lay it out and draw it inside a pop-up of
//! `popup::Kind::Confirm`, whose two rows are the buttons.

use nekominal_model::strings::{Lang, Text};
use nekominal_ui::{Color, Rect, Ui};

use crate::{
    design::RADIUS,
    glass::Glass,
    popup::{DIALOG_WIDTH, Row},
    widgets::{self, col},
    worktree::{Refusal, Status},
};

/// The index of each choice.
pub const DELETE_BRANCH: usize = 0;
pub const ARCHIVE: usize = 1;
/// The changed files the dialog names at most.
const FILES: usize = 5;

/// The icon of a line of what was checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Warn,
    /// Something to know that is neither (the tabs that close, the agent that runs).
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub mark: Mark,
    pub text: String,
    /// The changed files under the line of changes: the letter of their state and their path.
    pub files: Vec<(char, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    /// What the choice does, or why it cannot be chosen.
    pub note: String,
    pub enabled: bool,
}

/// What the main button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    Remove,
    /// Shows the tab in the folder, to commit or push there.
    ShowTab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub title: String,
    pub path: String,
    pub checks: Vec<Check>,
    pub choices: [Choice; 2],
    /// The choice taken; none when neither can be.
    pub chosen: Option<usize>,
    pub primary: Primary,
}

impl Dialog {
    pub fn delete_branch(&self) -> bool {
        self.chosen == Some(DELETE_BRANCH)
    }

    /// Takes the choice `i` when it can be taken; true when that changed it.
    pub fn choose(&mut self, i: usize) -> bool {
        let take = self.choices.get(i).is_some_and(|c| c.enabled) && self.chosen != Some(i);
        if take {
            self.chosen = Some(i);
        }
        take
    }

    /// The other choice (Tab and the arrows), when it can be taken.
    pub fn toggle(&mut self) -> bool {
        self.chosen.is_some_and(|i| i < 2) && self.choose(1 - self.chosen.unwrap_or(0))
    }
}

/// What the window knows of the folder besides git: the pull request of its branch, the tabs in it and the agent running
/// there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub pr: Option<u64>,
    pub tabs: usize,
    pub agent: Option<String>,
}

/// The dialog for the worktree `status` describes; the main checkout and a worktree on no branch are refused before it.
pub fn model(status: &Status, facts: &Facts, lang: Lang) -> Result<Dialog, Refusal> {
    if status.main {
        return Err(Refusal::Main);
    }
    let Some(branch) = status.branch.as_deref() else { return Err(Refusal::Detached) };
    let line = |mark, text| Check { mark, text, files: Vec::new() };
    let dirty = status.dirty();
    // With no remote there is nowhere to push to: nothing waits on it.
    let unpushed = status.remote.is_some() && status.unpushed.is_some_and(|n| n > 0);
    let merged = status.merged == Some(true);

    let mut checks = Vec::new();
    checks.push(if dirty {
        let files = status.files.iter().take(FILES).cloned().collect();
        Check { mark: Mark::Warn, text: Text::WorktreeChanged.format(lang, &[&(status.changed + status.untracked)]), files }
    } else {
        line(Mark::Ok, Text::WorktreeClean.get(lang).to_owned())
    });
    match (&status.remote, status.unpushed) {
        (None, _) => checks.push(line(Mark::Note, Text::WorktreeNoRemote.get(lang).to_owned())),
        (Some(_), Some(n)) if n > 0 => checks.push(line(Mark::Warn, Text::WorktreeUnpushed.format(lang, &[&n]))),
        (Some(remote), Some(_)) => checks.push(line(Mark::Ok, Text::WorktreePushed.format(lang, &[remote]))),
        (Some(_), None) => {}
    }
    match (&status.default_branch, status.merged, facts.pr) {
        (Some(default), Some(true), Some(pr)) => checks.push(line(Mark::Ok, Text::WorktreeMergedPr.format(lang, &[default, &pr]))),
        (Some(default), Some(true), None) => checks.push(line(Mark::Ok, Text::WorktreeMerged.format(lang, &[default]))),
        (Some(default), Some(false), _) => checks.push(line(Mark::Note, Text::WorktreeNotMerged.format(lang, &[default]))),
        _ => {}
    }
    if facts.tabs > 0 {
        checks.push(line(Mark::Note, Text::WorktreeTabsClose.format(lang, &[&facts.tabs])));
    }
    if let Some(agent) = &facts.agent {
        checks.push(line(Mark::Note, Text::WorktreeAgentRuns.format(lang, &[agent])));
    }

    let blocked = dirty || unpushed;
    let delete_note = match (dirty, unpushed) {
        (true, true) => Text::WorktreeCommitPushFirst.get(lang).to_owned(),
        (true, false) => Text::WorktreeCommitFirst.get(lang).to_owned(),
        (false, true) => Text::WorktreePushFirst.get(lang).to_owned(),
        (false, false) if merged => Text::WorktreeNothingLost.get(lang).to_owned(),
        (false, false) => Text::WorktreeOnlyMerged.get(lang).to_owned(),
    };
    let archive_note = if dirty {
        Text::WorktreeChangesLost.get(lang).to_owned()
    } else if unpushed {
        Text::WorktreePushFirst.get(lang).to_owned()
    } else {
        Text::WorktreeKeepsBranch.format(lang, &[&branch])
    };
    Ok(Dialog {
        title: Text::DialogRemoveWorktree.format(lang, &[&branch]),
        path: status.path.display().to_string(),
        checks,
        choices: [
            Choice { label: Text::WorktreeDeleteBranch.get(lang).to_owned(), note: delete_note, enabled: !blocked },
            Choice { label: Text::WorktreeArchive.get(lang).to_owned(), note: archive_note, enabled: !blocked },
        ],
        chosen: (!blocked).then_some(if merged { DELETE_BRANCH } else { ARCHIVE }),
        primary: if blocked { Primary::ShowTab } else { Primary::Remove },
    })
}

/// The dialog with where its parts go.
pub struct View {
    pub dialog: Dialog,
    pub layout: Layout,
}

/// Where the parts of the dialog go, in DIP of the pop-up.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The top of each line of what was checked.
    pub checks: Vec<f32>,
    pub choices: [Rect; 2],
    /// The top of the buttons, and the height of the whole.
    pub buttons: f32,
    pub height: f32,
}

const PAD: f32 = 20.0;
const TITLE_SIZE: f32 = 15.0;
const TEXT_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 12.0;
/// Where the text of a line of what was checked starts, after its icon.
const TEXT_LEFT: f32 = PAD + 24.0;
const ICON_SIZE: f32 = 13.0;
const BUTTON_HEIGHT: f32 = 32.0;

pub fn place(ui: &mut Ui, dialog: &Dialog) -> Layout {
    let (title, text, small) = (ui.line_height(TITLE_SIZE), ui.line_height(TEXT_SIZE), ui.line_height(SMALL_SIZE));
    let mut y = PAD + title + 4.0 + small + 14.0;
    let mut checks = Vec::new();
    for check in &dialog.checks {
        checks.push(y);
        y += text + 6.0;
        if !check.files.is_empty() {
            y += check.files.len() as f32 * small + 12.0 + 6.0;
        }
    }
    y += 8.0;
    let height = 18.0 + text + small;
    let choices = [0, 1].map(|_| {
        let rect = Rect::new(PAD, y, DIALOG_WIDTH - 2.0 * PAD, height);
        y += height + 6.0;
        rect
    });
    let buttons = y + 10.0;
    Layout { checks, choices, buttons, height: buttons + BUTTON_HEIGHT + 16.0 }
}

/// The rectangles of the buttons, Cancel then the main one, at the right, below the top of the buttons (`Layout::buttons`).
pub fn buttons(ui: &mut Ui, rows: &[Row]) -> Vec<Rect> {
    let mut right = DIALOG_WIDTH - PAD;
    let mut rects: Vec<Rect> = rows
        .iter()
        .rev()
        .map(|row| {
            let width = (ui.measure(&row.label, TEXT_SIZE, false) + 40.0).max(88.0);
            right -= width;
            let rect = Rect::new(right, 0.0, width, BUTTON_HEIGHT);
            right -= 8.0;
            rect
        })
        .collect();
    rects.reverse();
    rects
}

/// A colour at `share` of its opacity, for a choice that cannot be taken.
fn faded(color: Color, share: f32) -> Color {
    let [r, g, b, a] = color.0;
    Color([r, g, b, (f32::from(a) * share) as u8])
}

/// Draws the dialog; `rows` are its buttons at `rects` below `layout.buttons`, `hover` the one under the pointer.
pub fn paint(ui: &mut Ui, g: &Glass, view: &View, rows: &[Row], rects: &[Rect], hover: Option<usize>) {
    let (dialog, layout) = (&view.dialog, &view.layout);
    let (text_color, quiet) = (col(g.text()), col(g.text_secondary()));
    let (title, text, small) = (ui.line_height(TITLE_SIZE), ui.line_height(TEXT_SIZE), ui.line_height(SMALL_SIZE));
    let inner = DIALOG_WIDTH - 2.0 * PAD;

    ui.push_clip(Rect::new(PAD, 0.0, inner, layout.height));
    ui.text(PAD, PAD, &dialog.title, TITLE_SIZE, text_color, true);
    ui.mono(PAD, PAD + title + 4.0, &dialog.path, SMALL_SIZE, quiet);
    for (check, &y) in dialog.checks.iter().zip(&layout.checks) {
        let (glyph, color) = match check.mark {
            Mark::Ok => ('\u{E73E}', col(g.mark(g.success))),
            Mark::Warn => ('\u{E7BA}', col(g.mark(g.warning))),
            Mark::Note => ('\u{E737}', quiet),
        };
        let icon_line = ui.line_height(ICON_SIZE);
        ui.icon(PAD, y + (text - icon_line) / 2.0, glyph, ICON_SIZE, color);
        ui.text(TEXT_LEFT, y, &check.text, TEXT_SIZE, if check.mark == Mark::Note { quiet } else { text_color }, false);
        if !check.files.is_empty() {
            let top = y + text + 4.0;
            let rect = Rect::new(TEXT_LEFT, top, PAD + inner - TEXT_LEFT, check.files.len() as f32 * small + 12.0);
            ui.rrect(rect, RADIUS, col(g.tab_rest()), None);
            for (i, (code, path)) in check.files.iter().enumerate() {
                let line_y = top + 6.0 + i as f32 * small;
                ui.mono(rect.x + 10.0, line_y, &code.to_string(), SMALL_SIZE, col(g.mark(g.warning)));
                ui.mono(rect.x + 28.0, line_y, path, SMALL_SIZE, quiet);
            }
        }
    }
    for (i, (choice, &rect)) in dialog.choices.iter().zip(&layout.choices).enumerate() {
        let on = dialog.chosen == Some(i);
        if on {
            ui.rrect(rect, 6.0, col(g.chip_selected()), Some((col(g.accent), 1.0)));
        } else {
            ui.rrect(rect, 6.0, col(g.cluster()), None);
        }
        let share = if choice.enabled { 1.0 } else { 0.4 };
        widgets::radio(ui, g, Rect::new(rect.x + 12.0, rect.y + 9.0 + (text - 14.0) / 2.0, 14.0, 14.0), on, false);
        ui.text(rect.x + 36.0, rect.y + 9.0, &choice.label, TEXT_SIZE, faded(text_color, share), false);
        ui.text(rect.x + 36.0, rect.y + 9.0 + text, &choice.note, SMALL_SIZE, faded(quiet, share), false);
    }
    ui.pop_clip();

    for (i, (row, rect)) in rows.iter().zip(rects).enumerate() {
        let rect = Rect::new(rect.x, rect.y + layout.buttons, rect.w, rect.h);
        let primary = row.action == Some(crate::popup::Action::Confirm);
        let label_color = if primary && dialog.primary == Primary::Remove {
            let fill = if hover == Some(i) { faded(col(g.accent), 0.85) } else { col(g.accent) };
            ui.rrect(rect, RADIUS, fill, None);
            col(g.background)
        } else if primary {
            widgets::button(ui, g, rect, widgets::State::hover(hover == Some(i)));
            text_color
        } else {
            if hover == Some(i) {
                widgets::plate(ui, rect, col(g.hover()));
            }
            quiet
        };
        let x = rect.x + (rect.w - ui.measure(&row.label, TEXT_SIZE, false)) / 2.0;
        ui.text(x, rect.y + (rect.h - text) / 2.0, &row.label, TEXT_SIZE, label_color, false);
    }
}

/// What a screen reader reads as the text of the dialog: its title, its path and what was checked.
pub fn spoken(dialog: &Dialog) -> String {
    let mut lines = vec![dialog.title.clone(), dialog.path.clone()];
    for check in &dialog.checks {
        lines.push(check.text.clone());
        lines.extend(check.files.iter().map(|(code, path)| format!("{code} {path}")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn status() -> Status {
        Status {
            path: PathBuf::from(r"D:\src\app-fix-login"),
            branch: Some("fix/login".into()),
            main: false,
            changed: 0,
            untracked: 0,
            files: Vec::new(),
            remote: Some("origin".into()),
            unpushed: Some(0),
            default_branch: Some("main".into()),
            merged: Some(true),
        }
    }

    fn texts(dialog: &Dialog) -> Vec<(Mark, &str)> {
        dialog.checks.iter().map(|c| (c.mark, c.text.as_str())).collect()
    }

    #[test]
    fn a_clean_merged_worktree_offers_to_delete_the_branch_first() {
        let facts = Facts { pr: Some(143), tabs: 1, agent: None };
        let dialog = model(&status(), &facts, Lang::ZhTw).unwrap();
        assert_eq!(dialog.title, "移除工作樹 fix/login？");
        assert_eq!(dialog.path, r"D:\src\app-fix-login");
        assert_eq!(texts(&dialog), [
            (Mark::Ok, "沒有未提交的變更"),
            (Mark::Ok, "所有提交都已推送到 origin"),
            (Mark::Ok, "已合併到 main（PR #143）"),
            (Mark::Note, "1 個分頁在這個資料夾，會一起關閉"),
        ]);
        assert!(dialog.choices.iter().all(|c| c.enabled));
        assert_eq!(dialog.choices[DELETE_BRANCH].note, "分支已合併，不會遺失任何東西");
        assert_eq!(dialog.choices[ARCHIVE].note, "保留分支 fix/login，之後可以再建立工作樹");
        assert_eq!((dialog.chosen, dialog.primary), (Some(DELETE_BRANCH), Primary::Remove));
        assert!(dialog.delete_branch());
    }

    #[test]
    fn a_clean_worktree_not_merged_keeps_its_branch_unless_asked() {
        let status = Status { merged: Some(false), ..status() };
        let mut dialog = model(&status, &Facts::default(), Lang::EnUs).unwrap();
        assert_eq!(texts(&dialog)[2], (Mark::Note, "Not merged into main"));
        assert_eq!(dialog.choices[DELETE_BRANCH].note, "git deletes the branch only once it is merged");
        assert!(dialog.choices.iter().all(|c| c.enabled));
        assert_eq!((dialog.chosen, dialog.primary), (Some(ARCHIVE), Primary::Remove));
        assert!(!dialog.delete_branch());
        assert!(dialog.toggle() && dialog.delete_branch(), "Tab moves to the other choice");
        assert!(!dialog.choose(DELETE_BRANCH), "taken already");
    }

    #[test]
    fn changes_disable_both_choices_and_the_main_button_shows_the_tab() {
        let files: Vec<(char, String)> = (0..7).map(|i| ('M', format!("src/{i}.rs"))).collect();
        let status = Status { changed: 6, untracked: 1, files, unpushed: Some(2), merged: Some(false), ..status() };
        let facts = Facts { pr: None, tabs: 0, agent: Some("Claude Code".into()) };
        let mut dialog = model(&status, &facts, Lang::ZhTw).unwrap();
        assert_eq!(texts(&dialog), [
            (Mark::Warn, "7 個檔案有未提交的變更"),
            (Mark::Warn, "2 個提交還沒推送"),
            (Mark::Note, "還沒合併到 main"),
            (Mark::Note, "Claude Code 正在這個資料夾執行"),
        ]);
        assert_eq!(dialog.checks[0].files.len(), FILES, "a short list of the files");
        assert!(dialog.choices.iter().all(|c| !c.enabled));
        assert_eq!(dialog.choices[DELETE_BRANCH].note, "要先提交並推送");
        assert_eq!(dialog.choices[ARCHIVE].note, "未提交的變更會遺失");
        assert_eq!((dialog.chosen, dialog.primary), (None, Primary::ShowTab));
        assert!(!dialog.toggle() && !dialog.choose(ARCHIVE), "nothing can be chosen");
    }

    #[test]
    fn commits_not_pushed_disable_both_choices_with_their_reason() {
        let status = Status { unpushed: Some(3), ..status() };
        let dialog = model(&status, &Facts::default(), Lang::EnUs).unwrap();
        assert_eq!(texts(&dialog)[..2], [(Mark::Ok, "No uncommitted changes"), (Mark::Warn, "3 commits are not pushed")]);
        assert!(dialog.choices.iter().all(|c| !c.enabled && c.note == "Push first"));
        assert_eq!((dialog.chosen, dialog.primary), (None, Primary::ShowTab));
    }

    #[test]
    fn a_repository_with_no_remote_has_nothing_to_push_to() {
        let status = Status { remote: None, unpushed: Some(4), ..status() };
        let dialog = model(&status, &Facts::default(), Lang::EnUs).unwrap();
        assert_eq!(texts(&dialog)[1], (Mark::Note, "The repository has no remote"));
        assert_eq!(dialog.primary, Primary::Remove);
    }

    #[test]
    fn the_main_checkout_and_a_detached_head_are_refused_before_the_dialog() {
        assert_eq!(model(&Status { main: true, ..status() }, &Facts::default(), Lang::ZhTw), Err(Refusal::Main));
        assert_eq!(model(&Status { branch: None, ..status() }, &Facts::default(), Lang::ZhTw), Err(Refusal::Detached));
    }

    #[test]
    fn every_combination_of_checks_enables_the_choices_only_when_nothing_is_lost() {
        let files = |n: usize| (0..n).map(|i| ('M', format!("{i}.rs"))).collect::<Vec<_>>();
        for (changed, untracked) in [(0, 0), (2, 0), (0, 1)] {
            for remote in [None, Some("origin")] {
                for unpushed in [None, Some(0), Some(3)] {
                    for (default, merged) in [(None, None), (Some("main"), Some(true)), (Some("main"), Some(false))] {
                        for pr in [None, Some(7)] {
                            for (tabs, agent) in [(0, None), (2, Some("Codex"))] {
                                let status = Status {
                                    changed,
                                    untracked,
                                    files: files(changed + untracked),
                                    remote: remote.map(str::to_owned),
                                    unpushed,
                                    default_branch: default.map(str::to_owned),
                                    merged,
                                    ..status()
                                };
                                let facts = Facts { pr, tabs, agent: agent.map(str::to_owned) };
                                let case = format!("{status:?} {facts:?}");
                                let dialog = model(&status, &facts, Lang::EnUs).unwrap();
                                let dirty = changed + untracked > 0;
                                let waits = remote.is_some() && unpushed.is_some_and(|n| n > 0);
                                let free = !dirty && !waits;

                                assert!(dialog.choices.iter().all(|c| c.enabled == free), "{case}");
                                let chosen = free.then_some(if merged == Some(true) { DELETE_BRANCH } else { ARCHIVE });
                                assert_eq!(dialog.chosen, chosen, "{case}");
                                assert_eq!(dialog.primary, if free { Primary::Remove } else { Primary::ShowTab }, "{case}");

                                // A line for the changes, one for pushing unless git could not tell, one for merging when
                                // there is a default branch, one for the tabs and one for the agent when there are.
                                let lines = 1 + usize::from(remote.is_none() || unpushed.is_some()) + usize::from(default.is_some()) + usize::from(tabs > 0) + usize::from(agent.is_some());
                                assert_eq!(dialog.checks.len(), lines, "{case}");
                                let warnings = dialog.checks.iter().filter(|c| c.mark == Mark::Warn).count();
                                assert_eq!(warnings, usize::from(dirty) + usize::from(waits), "{case}: only what would be lost warns");
                                assert_eq!(dialog.checks[0].files.len(), changed + untracked, "{case}");
                                assert!(dialog.checks[1..].iter().all(|c| c.files.is_empty()));
                                assert_eq!(dialog.checks.iter().any(|c| c.text.contains("PR #7")), pr.is_some() && merged == Some(true), "{case}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_file_list_shows_five_files_and_the_count_says_all() {
        for (n, shown) in [(1, 1), (5, 5), (6, 5), (40, 5)] {
            let files = (0..n).map(|i| ('M', format!("{i}.rs"))).collect();
            let dialog = model(&Status { changed: n, files, ..status() }, &Facts::default(), Lang::EnUs).unwrap();
            assert_eq!(dialog.checks[0].files.len(), shown);
            assert_eq!(dialog.checks[0].text, format!("{n} files have uncommitted changes"));
            assert_eq!(dialog.checks[0].files[0].1, "0.rs", "in the order git lists them");
        }
        // Counted but not named (git said more than the lines could tell): the line with no list under it.
        let unnamed = model(&Status { untracked: 2, ..status() }, &Facts::default(), Lang::EnUs).unwrap();
        assert_eq!((unnamed.checks[0].mark, unnamed.checks[0].files.len()), (Mark::Warn, 0));
    }

    #[test]
    fn the_choice_moves_only_between_choices_that_can_be_taken() {
        let mut dialog = model(&status(), &Facts::default(), Lang::EnUs).unwrap();
        assert_eq!(dialog.chosen, Some(DELETE_BRANCH));
        assert!(!dialog.choose(2) && !dialog.choose(usize::MAX), "no third choice");
        assert!(dialog.toggle() && dialog.chosen == Some(ARCHIVE));
        assert!(dialog.toggle() && dialog.chosen == Some(DELETE_BRANCH), "and back");
        assert!(dialog.choose(ARCHIVE) && !dialog.choose(ARCHIVE));
        // One choice that cannot be taken keeps the other.
        dialog.choices[DELETE_BRANCH].enabled = false;
        assert!(!dialog.toggle() && !dialog.choose(DELETE_BRANCH));
        assert_eq!(dialog.chosen, Some(ARCHIVE));
    }

    #[test]
    fn a_screen_reader_hears_the_title_path_checks_and_files() {
        let files = vec![('M', "src/a.rs".to_owned()), ('?', "my notes.txt".to_owned())];
        let status = Status { changed: 1, untracked: 1, files, ..status() };
        let dialog = model(&status, &Facts { pr: None, tabs: 1, agent: None }, Lang::EnUs).unwrap();
        let spoken = spoken(&dialog);
        let lines: Vec<&str> = spoken.lines().collect();
        assert_eq!(lines[..5], [dialog.title.as_str(), r"D:\src\app-fix-login", "2 files have uncommitted changes", "M src/a.rs", "? my notes.txt"]);
        assert_eq!(lines.len(), 2 + dialog.checks.len() + 2);
    }

    #[test]
    fn the_files_under_a_check_push_the_lines_below_them_down() {
        use nekominal_render::offscreen::{AdapterKind, create_gpu};
        let gpu = create_gpu(AdapterKind::Warp).expect("D3D11 WARP");
        let mut ui = Ui::new(&gpu).expect("UI");
        let clean = model(&Status { unpushed: Some(2), ..status() }, &Facts::default(), Lang::EnUs).unwrap();
        let files = (0..9).map(|i| ('M', format!("{i}.rs"))).collect();
        let dirty = model(&Status { changed: 9, files, unpushed: Some(2), ..status() }, &Facts::default(), Lang::EnUs).unwrap();
        let (a, b) = (place(&mut ui, &clean), place(&mut ui, &dirty));
        assert_eq!(a.checks[0], b.checks[0]);
        let list = FILES as f32 * ui.line_height(SMALL_SIZE) + 18.0;
        assert!((b.checks[1] - a.checks[1] - list).abs() < 0.01, "five lines of files, not nine");
        assert!((b.height - a.height - list).abs() < 0.01);
        assert!(b.choices[0].y > b.checks[b.checks.len() - 1] && b.buttons > b.choices[1].y + b.choices[1].h);
    }

    #[test]
    fn the_dialog_renders_its_lines_choices_and_buttons() {
        use nekominal_render::offscreen::{AdapterKind, OffscreenTarget, create_gpu, write_png};
        let gpu = create_gpu(AdapterKind::Warp).expect("D3D11 WARP");
        let glass = Glass::from_scheme(nekominal_model::themes::scheme("Tokyo Night"));
        let files = vec![('M', "src/auth/oauth.rs".to_owned()), ('?', "src/auth/pkce.rs".to_owned())];
        let status = Status { changed: 1, untracked: 1, files, unpushed: Some(2), ..status() };
        let dialog = model(&status, &Facts { pr: None, tabs: 2, agent: None }, Lang::ZhTw).unwrap();
        let row = |label: &str, action| Row { label: label.into(), hint: String::new(), action: Some(action), current: false, mark: None, program: None };
        let rows = vec![row("取消", crate::popup::Action::Cancel), row("開啟分頁查看", crate::popup::Action::Confirm)];
        let mut ui = Ui::new(&gpu).expect("UI");
        let layout = place(&mut ui, &dialog);
        let rects = buttons(&mut ui, &rows);
        assert!(rects[0].x + rects[0].w < rects[1].x && rects[1].x + rects[1].w <= DIALOG_WIDTH - PAD, "Cancel then the main button, at the right");
        assert!(layout.choices[0].y > *layout.checks.last().unwrap() && layout.buttons > layout.choices[1].y + layout.choices[1].h);
        let view = View { dialog, layout };
        let size = (DIALOG_WIDTH as u32, view.layout.height as u32);
        let target = OffscreenTarget::new(&gpu, size.0, size.1);
        gpu.clear(&target.view, [0.03, 0.03, 0.03, 1.0]);
        ui.begin_frame(size, 1.0);
        paint(&mut ui, &glass, &view, &rows, &rects, None);
        ui.end_frame(&target.view);
        let rgba = target.read_rgba(&gpu).expect("readback");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-test/remove-worktree.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_png(&path, size.0, size.1, &rgba).unwrap();
        let ink = |rect: Rect| {
            let mut count = 0;
            for y in rect.y as u32..(rect.y + rect.h) as u32 {
                for x in rect.x as u32..(rect.x + rect.w) as u32 {
                    let at = ((y * size.0 + x) * 4) as usize;
                    if rgba[at..at + 3].iter().any(|c| *c > 120) { count += 1; }
                }
            }
            count
        };
        assert!(ink(Rect::new(PAD, PAD, 200.0, 18.0)) > 30, "title missing");
        assert!(ink(Rect::new(TEXT_LEFT, view.layout.checks[0], 200.0, 16.0)) > 20, "first check missing");
        let button = Rect::new(rects[1].x + 8.0, view.layout.buttons + 6.0, rects[1].w - 16.0, 20.0);
        assert!(ink(button) > 10, "main button label missing");
    }
}
