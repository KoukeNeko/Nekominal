//! The badges of a tab: the pull request of the branch its folder is on (`github.rs`), and the TCP ports the programs of
//! its panes listen on (`ports.rs`). Read with the job tick, once a second. They are drawn as small pills on a third line
//! of a row of the list, and for the selected tab in the title bar; a click opens the pull request or the port.

use super::*;
use crate::{github, ports};

/// The text size and the height of a badge: on a row of the list, and in the title bar.
pub const SIDE: (f32, f32) = (10.5, 16.0);
pub const BAR: (f32, f32) = (11.0, 18.0);
/// What a line of badges adds to a row of the list.
const LINE: f32 = SIDE.1 + 4.0;
const GAP: f32 = 4.0;
const ICON: f32 = 11.0;

/// How a badge looks: a pull request by its state and its checks, or a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Passing,
    Failing,
    Running,
    /// Open without checks.
    Open,
    Draft,
    Merged,
    Closed,
    Port,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub look: Look,
    pub text: String,
    /// The failing checks, in a bubble after the number.
    pub count: Option<u32>,
    /// What a click opens.
    pub url: String,
}

/// The badges of a pull request and of ports: the pull request first, then each port.
pub fn chips(pr: Option<&github::PullRequest>, listening: &[u16]) -> Vec<Chip> {
    let pr = pr.map(|pr| {
        let checks = pr.checks;
        let look = match pr.state {
            github::State::Merged => Look::Merged,
            github::State::Closed => Look::Closed,
            github::State::Draft => Look::Draft,
            github::State::Open if checks.failing > 0 => Look::Failing,
            github::State::Open if checks.pending > 0 => Look::Running,
            github::State::Open if checks.passing > 0 => Look::Passing,
            github::State::Open => Look::Open,
        };
        Chip { look, text: format!("#{}", pr.number), count: (look == Look::Failing).then_some(checks.failing), url: pr.url.clone() }
    });
    let ports = listening.iter().map(|&port| Chip { look: Look::Port, text: port.to_string(), count: None, url: ports::url(port) });
    pr.into_iter().chain(ports).collect()
}

/// The width of a badge, its text measured by `measure`: the mark, the text, and the bubble of the count.
pub fn width(chip: &Chip, mut measure: impl FnMut(&str) -> f32) -> f32 {
    let count = chip.count.map_or(0.0, |n| 4.0 + measure(&n.to_string()) + 8.0);
    3.0 + ICON + 3.0 + measure(&chip.text) + count + 5.0
}

/// Where badges `widths` wide go in lines `room` wide, from the top left of their first line: one after the other, the next
/// line when one does not fit; a badge wider than the room has a line of its own.
pub fn place(widths: &[f32], room: f32) -> Vec<Rect> {
    let (mut x, mut y) = (0.0, 0.0);
    widths
        .iter()
        .map(|&w| {
            if x > 0.0 && x + w > room {
                (x, y) = (0.0, y + LINE);
            }
            let rect = Rect::new(x, y, w, SIDE.1);
            x += w + GAP;
            rect
        })
        .collect()
}

/// What placed badges add to a row: a line each, nothing without any.
pub fn height(placed: &[Rect]) -> f32 {
    placed.last().map_or(0.0, |last| last.y + LINE)
}

/// The badge under (`x`, `y`) among those drawn, by its place in `drawn`.
pub fn at(drawn: &[(Rect, String)], x: f32, y: f32) -> Option<usize> {
    drawn.iter().position(|(rect, _)| rect.contains(x, y))
}

/// The repository a tab is in: the main one for a worktree, so that the tabs of its worktrees go under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: std::path::PathBuf,
    pub worktree: bool,
}

impl Repo {
    pub fn name(&self) -> String {
        self.root.file_name().map_or_else(|| self.root.to_string_lossy().into_owned(), |name| name.to_string_lossy().into_owned())
    }
}

/// The repository of the folder `root` whose `.git` is a file saying `git_file` (a worktree or a submodule) or a folder
/// (`None`): a worktree's `.git` names `<main>/.git/worktrees/<name>`.
pub fn repo_of(root: &std::path::Path, git_file: Option<&str>) -> Repo {
    let main = git_file.and_then(|text| text.trim().strip_prefix("gitdir:")).and_then(|dir| {
        let dir = root.join(dir.trim());
        let worktrees = dir.parent()?;
        let git = worktrees.parent()?;
        (worktrees.file_name()? == "worktrees" && git.file_name()? == ".git").then(|| git.parent().map(std::path::Path::to_path_buf)).flatten()
    });
    match main {
        Some(main) => Repo { root: main, worktree: true },
        None => Repo { root: root.to_path_buf(), worktree: false },
    }
}

#[derive(Default)]
pub struct Badges {
    prs: github::PullRequests,
    /// By the uid of the tab: the repository of its focused pane's folder.
    pub repo: HashMap<u64, Repo>,
    /// By the uid of the tab: the pull request of the branch of its focused pane's folder.
    pub pull_request: HashMap<u64, github::PullRequest>,
    /// By the uid of the tab: the ports its panes listen on, ascending; open one at `ports::url`.
    pub ports: HashMap<u64, Vec<u16>>,
}

impl Badges {
    /// The badges of a tab as a line of text, for a row of the agents panel.
    pub fn text(&self, uid: u64) -> Option<String> {
        let ports = self.ports.get(&uid).map(|ports| ports.iter().map(|p| format!(":{p}")).collect::<Vec<_>>().join(" "));
        let parts: Vec<String> = [self.pull_request.get(&uid).map(github::PullRequest::summary), ports].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
    }

    pub fn chips(&self, uid: u64) -> Vec<Chip> {
        chips(self.pull_request.get(&uid), self.ports.get(&uid).map_or(&[], Vec::as_slice))
    }
}

/// Sets the badge of the tab `uid` in `map`, with a line in the log when it changed; true when it did.
fn set<T: PartialEq + std::fmt::Debug>(map: &mut HashMap<u64, T>, uid: u64, value: Option<T>, what: &str) -> bool {
    if map.get(&uid) == value.as_ref() {
        return false;
    }
    util::log(&format!("badges: tab {uid} {what} {value:?}"));
    match value {
        Some(value) => map.insert(uid, value),
        None => map.remove(&uid),
    };
    true
}

impl App {
    /// Reads the badges of every tab again; true when one changed.
    /// `processes` is the look at the processes of the system the tick took, which finds what a Store shell starts.
    pub(super) fn refresh_badges(&mut self, processes: &[(u32, u32, String)]) -> bool {
        let (badges, now) = (&mut self.badges, Instant::now());
        let listeners = ports::listeners();
        badges.prs.collect();
        let mut changed = false;
        let mut keys = Vec::new();
        for tab in &self.tabs {
            let pids: Vec<u32> = tab.panes.iter().flat_map(|entry| nekominal_pty::conpty::with_descendants(entry.pane.job_process_ids(), processes)).collect();
            let found = ports::ports_of(&listeners, &pids);
            changed |= set(&mut badges.ports, tab.uid, (!found.is_empty()).then_some(found), "ports");
            // The repository `refresh_usage` found this tick (git_branch reads only local disks: gh never runs for a share).
            let git = tab.focused_entry().and_then(|entry| entry.git.as_ref());
            let key = git.map(|git| (git.root.clone(), git.branch.clone()));
            let repo = git.map(|git| repo_of(&git.root, git.git_file.as_deref()));
            changed |= set(&mut badges.repo, tab.uid, repo, "repository");
            let pr = key.as_ref().and_then(|key| badges.prs.get(key, now).cloned());
            changed |= set(&mut badges.pull_request, tab.uid, pr, "pull request");
            keys.extend(key);
        }
        badges.prs.retain(|key| keys.contains(key));
        let open = |uid: &u64| self.tabs.iter().any(|tab| tab.uid == *uid);
        badges.ports.retain(|uid, _| open(uid));
        badges.pull_request.retain(|uid, _| open(uid));
        badges.repo.retain(|uid, _| open(uid));
        changed
    }

    /// The badges of tab `i` on its row of the list, placed in lines `room` wide from the top left of their first line.
    pub(super) fn row_badges(&mut self, i: usize, room: f32) -> Vec<(Chip, Rect)> {
        let chips = self.badges.chips(self.tabs[i].uid);
        let widths: Vec<f32> = chips.iter().map(|chip| width(chip, |text| self.ui.measure(text, SIDE.0, false))).collect();
        chips.into_iter().zip(place(&widths, room)).collect()
    }

    /// The badges of tab `i` on its row, in lines `room` wide from (`x`, `top`).
    pub(super) fn paint_row_badges(&mut self, i: usize, x: f32, top: f32, room: f32) {
        for (chip, rect) in self.row_badges(i, room) {
            self.paint_badge(&chip, Rect::new(x + rect.x, top + rect.y, rect.w, rect.h), SIDE.0);
        }
    }

    /// One badge in `rect`, its text `size` DIP: a pill of its colour with its mark and its text, the failing checks in a
    /// bubble. It is kept for a click.
    pub(super) fn paint_badge(&mut self, chip: &Chip, rect: Rect, size: f32) {
        let g = self.glass;
        let color = col(match chip.look {
            Look::Passing | Look::Open => g.mark(g.success),
            Look::Failing | Look::Closed => g.mark(g.danger),
            Look::Running => g.mark(g.warning),
            Look::Draft => g.text_secondary(),
            Look::Merged => g.mark(g.magenta),
            Look::Port => g.mark(g.cyan),
        });
        let tint = |alpha: u8| Color([color.0[0], color.0[1], color.0[2], alpha]);
        let hover = self.hover == Hit::Badge(self.badge_hits.len());
        self.ui.rrect(rect, rect.h / 2.0, tint(if hover { 64 } else { 36 }), None);
        let mark = Rect::new(rect.x + 3.0, rect.y + (rect.h - ICON) / 2.0, ICON, ICON);
        let (cx, cy, r) = (mark.x + ICON / 2.0, mark.y + ICON / 2.0, 3.2);
        match chip.look {
            Look::Running => {
                // A ring of dashes: checks that still run.
                for k in 0..8 {
                    let angle = k as f32 * std::f32::consts::FRAC_PI_4;
                    self.ui.rrect(Rect::new(cx + r * angle.cos() - 0.7, cy + r * angle.sin() - 0.7, 1.4, 1.4), 0.7, color, None);
                }
            }
            Look::Open | Look::Draft | Look::Closed => {
                self.ui.rrect(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, Color::TRANSPARENT, Some((color, 1.2)));
            }
            Look::Passing => self.icon_centered(mark, '\u{E73E}', 9.0, color),
            Look::Failing => self.icon_centered(mark, '\u{E711}', 8.0, color),
            Look::Merged => self.icon_centered(mark, '\u{E81C}', 9.0, color),
            Look::Port => self.icon_centered(mark, '\u{E774}', 9.0, color),
        }
        let line = self.ui.line_height(size);
        let x = mark.x + ICON + 3.0;
        let x = x + self.ui.text(x, rect.y + (rect.h - line) / 2.0, &chip.text, size, color, false);
        if let Some(n) = chip.count {
            let n = n.to_string();
            let bubble = Rect::new(x + 4.0, rect.y + 2.0, self.ui.measure(&n, size, false) + 8.0, rect.h - 4.0);
            self.ui.rrect(bubble, bubble.h / 2.0, tint(77), None);
            self.ui.text(bubble.x + 4.0, rect.y + (rect.h - line) / 2.0, &n, size, color, false);
        }
        self.badge_hits.push((rect, chip.url.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_goes_under_its_main_repository_and_anything_else_is_its_own() {
        let root = std::path::Path::new(r"D:\src\app-fix");
        let worktree = repo_of(root, Some("gitdir: D:/src/app/.git/worktrees/app-fix\n"));
        assert_eq!((worktree.name(), worktree.worktree), ("app".to_owned(), true));
        let relative = repo_of(root, Some("gitdir: ../app/.git/worktrees/app-fix"));
        assert!(relative.worktree && relative.root.ends_with("app"));
        for other in [None, Some("gitdir: ../.git/modules/sub"), Some("not a pointer")] {
            assert_eq!(repo_of(root, other), Repo { root: root.to_path_buf(), worktree: false }, "{other:?}");
        }
    }

    #[test]
    fn a_badge_is_set_once_and_cleared_with_none() {
        let mut map = HashMap::new();
        assert!(set(&mut map, 1, Some(vec![3000u16]), "ports"));
        assert!(!set(&mut map, 1, Some(vec![3000u16]), "ports"));
        assert!(set(&mut map, 1, None, "ports"));
        assert!(map.is_empty() && !set(&mut map, 1, None, "ports"));
    }

    #[test]
    fn the_text_of_the_badges_says_the_pull_request_then_the_ports() {
        let mut badges = Badges::default();
        assert_eq!(badges.text(1), None);
        badges.ports.insert(1, vec![3000, 5173]);
        assert_eq!(badges.text(1).as_deref(), Some(":3000 :5173"));
        let pr = github::parse(r#"{"number":4,"state":"MERGED","isDraft":false,"url":"u","statusCheckRollup":[]}"#).unwrap();
        badges.pull_request.insert(1, pr);
        assert_eq!(badges.text(1).as_deref(), Some("#4 merged \u{b7} :3000 :5173"));
    }

    fn pr(state: &str, draft: bool, rollup: &str) -> github::PullRequest {
        github::parse(&format!(r#"{{"number":142,"state":"{state}","isDraft":{draft},"url":"https://github.com/o/r/pull/142","statusCheckRollup":[{rollup}]}}"#)).unwrap()
    }

    #[test]
    fn a_pull_request_looks_like_its_checks_and_the_ports_follow_it() {
        let (pass, fail, run) = (r#"{"state":"SUCCESS"}"#, r#"{"state":"FAILURE"}"#, r#"{"state":"PENDING"}"#);
        let look = |pr: github::PullRequest| chips(Some(&pr), &[]).remove(0).look;
        assert_eq!(look(pr("OPEN", false, pass)), Look::Passing);
        assert_eq!(look(pr("OPEN", false, &format!("{pass},{run}"))), Look::Running);
        assert_eq!(look(pr("OPEN", false, &format!("{run},{fail}"))), Look::Failing, "a failure wins over what still runs");
        assert_eq!(look(pr("OPEN", false, "")), Look::Open);
        assert_eq!(look(pr("OPEN", true, fail)), Look::Draft);
        assert_eq!(look(pr("MERGED", false, fail)), Look::Merged);
        assert_eq!(look(pr("CLOSED", false, "")), Look::Closed);

        let failing = chips(Some(&pr("OPEN", false, &format!("{fail},{fail},{pass}"))), &[8080]);
        assert_eq!((failing[0].text.as_str(), failing[0].count), ("#142", Some(2)));
        assert_eq!(chips(Some(&pr("OPEN", false, pass)), &[])[0].count, None, "only failures are counted");
        assert_eq!(failing[1], Chip { look: Look::Port, text: "8080".into(), count: None, url: ports::url(8080) });
        assert_eq!(chips(None, &[3000, 5173]).iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["3000", "5173"]);
        assert!(chips(None, &[]).is_empty());
    }

    #[test]
    fn every_state_and_mix_of_checks_has_its_look() {
        let (pass, fail, run) = (r#"{"state":"SUCCESS"}"#, r#"{"state":"FAILURE"}"#, r#"{"state":"PENDING"}"#);
        // Every subset of passing, failing and running checks.
        for mask in 0..8 {
            let rollup = [(1, pass), (2, fail), (4, run)].iter().filter(|(bit, _)| mask & bit != 0).map(|(_, e)| *e).collect::<Vec<_>>().join(",");
            let open = match mask {
                0 => Look::Open,
                m if m & 2 != 0 => Look::Failing,
                m if m & 4 != 0 => Look::Running,
                _ => Look::Passing,
            };
            for (state, draft, look) in [("OPEN", false, open), ("OPEN", true, Look::Draft), ("MERGED", false, Look::Merged), ("CLOSED", false, Look::Closed)] {
                let chip = chips(Some(&pr(state, draft, &rollup)), &[]).remove(0);
                assert_eq!(chip.look, look, "{state} draft={draft} [{rollup}]");
                assert_eq!(chip.count.is_some(), look == Look::Failing, "only a failing badge counts");
            }
        }
    }

    #[test]
    fn badges_wrap_at_the_exact_edge_and_with_no_room() {
        let exact = place(&[50.0, 50.0, 1.0], 104.0);
        assert_eq!(exact.iter().map(|r| (r.x, r.y)).collect::<Vec<_>>(), [(0.0, 0.0), (54.0, 0.0), (0.0, LINE)], "a badge that ends on the edge stays");
        let none = place(&[20.0, 20.0], 0.0);
        assert_eq!(none.iter().map(|r| (r.x, r.y)).collect::<Vec<_>>(), [(0.0, 0.0), (0.0, LINE)], "a line each, none lost");
        assert_eq!(height(&none), 2.0 * LINE);
        let many = place(&vec![30.0; 20], 100.0);
        assert_eq!(height(&many), 7.0 * LINE, "three a line");
    }

    #[test]
    fn the_text_of_the_badges_with_a_pull_request_only_or_many_ports() {
        let mut badges = Badges::default();
        badges.pull_request.insert(1, pr("OPEN", false, r#"{"state":"FAILURE"}"#));
        assert_eq!(badges.text(1).as_deref(), Some("#142 open \u{b7} \u{2717}1"));
        badges.ports.insert(2, (3000..3006).collect());
        assert_eq!(badges.text(2).as_deref(), Some(":3000 :3001 :3002 :3003 :3004 :3005"));
        assert_eq!(badges.chips(2).len(), 6);
        assert_eq!(badges.text(3), None, "another tab");
    }

    #[test]
    fn a_click_on_a_badge_opens_its_pull_request_or_its_port() {
        let drawn: Vec<(Rect, String)> = chips(Some(&pr("OPEN", false, "")), &[3000])
            .into_iter()
            .zip(place(&[40.0, 40.0], 200.0))
            .map(|(chip, rect)| (rect, chip.url))
            .collect();
        assert_eq!(at(&drawn, 10.0, 8.0).map(|k| drawn[k].1.as_str()), Some("https://github.com/o/r/pull/142"));
        assert_eq!(at(&drawn, 50.0, 8.0).map(|k| drawn[k].1.as_str()), Some("http://localhost:3000"));
        assert_eq!(at(&drawn, 42.0, 8.0), None, "the gap between two badges");
        assert_eq!(at(&drawn, 10.0, 30.0), None);
    }

    #[test]
    fn badges_fill_a_line_then_wrap_and_add_a_line_each_to_the_row() {
        let measure = |text: &str| 6.0 * text.chars().count() as f32;
        let chip = chips(Some(&pr("OPEN", false, r#"{"state":"FAILURE"}"#)), &[]).remove(0);
        assert_eq!(width(&chip, measure), 3.0 + ICON + 3.0 + 24.0 + (4.0 + 6.0 + 8.0) + 5.0, "the count has its bubble");
        assert!(width(&Chip { count: None, ..chip.clone() }, measure) < width(&chip, measure));

        assert_eq!(height(&place(&[], 150.0)), 0.0, "no badges, no third line");
        let one = place(&[50.0, 50.0], 150.0);
        assert_eq!(one.iter().map(|r| (r.x, r.y)).collect::<Vec<_>>(), [(0.0, 0.0), (54.0, 0.0)]);
        assert_eq!(height(&one), LINE);
        let two = place(&[50.0, 50.0, 50.0, 50.0], 150.0);
        assert_eq!(two.iter().map(|r| (r.x, r.y)).collect::<Vec<_>>(), [(0.0, 0.0), (54.0, 0.0), (0.0, LINE), (54.0, LINE)]);
        assert!(two.iter().all(|r| r.x + r.w <= 150.0 && r.h == SIDE.1));
        assert_eq!(height(&two), 2.0 * LINE);
        let wide = place(&[200.0, 30.0], 150.0);
        assert_eq!((wide[0].x, wide[1].y), (0.0, LINE), "a badge wider than the row has a line of its own");
    }
}
