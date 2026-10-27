//! The pull request of the branch a tab is on, with its checks and its review, as GitHub's `gh` tells it in the folder of
//! the repository. `gh` runs on a thread of its own, at most once a minute for a repository and branch (and at once for a
//! branch not seen before); without `gh`, signed out of it, outside GitHub or without a pull request, there is none.

use std::{
    collections::HashMap,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};

/// `CREATE_NO_WINDOW`: gh is a console program, and this one has no console for it to borrow.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How long what gh said holds before it is asked again.
const FRESH: Duration = Duration::from_secs(60);
const FIELDS: &str = "number,state,isDraft,url,reviewDecision,statusCheckRollup,mergeStateStatus";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Draft,
    Merged,
    Closed,
}

/// The checks of the head commit, counted by how they stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    pub pending: u32,
    pub passing: u32,
    pub failing: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub state: State,
    pub checks: Checks,
    /// `APPROVED`, `CHANGES_REQUESTED` or `REVIEW_REQUIRED`; none when the repository asks for no review.
    pub review: Option<String>,
    /// `CLEAN`, `BEHIND`, `DIRTY`, `BLOCKED` and the like.
    pub merge_state: Option<String>,
    pub url: String,
}

impl PullRequest {
    /// A short line for a row: `#12 open · ✓3 ✗1 …2 · APPROVED`.
    pub fn summary(&self) -> String {
        let state = match self.state {
            State::Open => "open",
            State::Draft => "draft",
            State::Merged => "merged",
            State::Closed => "closed",
        };
        let Checks { pending, passing, failing } = self.checks;
        let checks = [(passing, '\u{2713}'), (failing, '\u{2717}'), (pending, '\u{2026}')]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, mark)| format!("{mark}{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        [Some(format!("#{} {state}", self.number)), (!checks.is_empty()).then_some(checks), self.review.clone()].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ")
    }
}

/// Counts the entries of `statusCheckRollup`: check runs (a `status`, then a `conclusion` once completed) and commit
/// statuses (a `state`). As `gh pr checks` counts them: what is not a pass or a failure is pending, a check that went stale
/// or a state GitHub adds later among them.
fn checks(rollup: &[serde_json::Value]) -> Checks {
    let mut checks = Checks::default();
    for item in rollup {
        let text = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let verdict = if item.get("status").is_some() {
            if text("status") != "COMPLETED" { "PENDING" } else { text("conclusion") }
        } else {
            text("state")
        };
        match verdict {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => checks.passing += 1,
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => checks.failing += 1,
            _ => checks.pending += 1,
        }
    }
    checks
}

/// What `gh pr view --json` printed; none for anything else.
pub fn parse(json: &str) -> Option<PullRequest> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
    let state = match text("state")?.as_str() {
        "MERGED" => State::Merged,
        "CLOSED" => State::Closed,
        _ if value.get("isDraft").and_then(|v| v.as_bool()) == Some(true) => State::Draft,
        _ => State::Open,
    };
    Some(PullRequest {
        number: value.get("number")?.as_u64()?,
        state,
        checks: checks(value.get("statusCheckRollup").and_then(|v| v.as_array()).map_or(&[], Vec::as_slice)),
        review: text("reviewDecision"),
        merge_state: text("mergeStateStatus"),
        url: text("url")?,
    })
}

/// Asks gh in `repo` for the pull request of the branch checked out there. Blocks: called on a thread of its own.
fn ask_gh(repo: &Path) -> Option<PullRequest> {
    answer(Command::new("gh").args(["pr", "view", "--json", FIELDS]).current_dir(repo).env("GH_PROMPT_DISABLED", "1").stdin(Stdio::null()).creation_flags(CREATE_NO_WINDOW).output())
}

/// What a run of gh came to: a pull request only when gh ran, succeeded and printed one. Not installed, signed out, a
/// remote that is not GitHub's and a branch without a pull request are all none.
fn answer(output: std::io::Result<std::process::Output>) -> Option<PullRequest> {
    let output = output.ok().filter(|output| output.status.success())?;
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// The folder of the repository that holds `dir`: the nearest one with a `.git` (a folder, or the file of a worktree).
/// The window takes it from `agent::git_branch`, which finds the same folder on its way; the tests here keep this one.
#[cfg(test)]
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|folder| folder.join(".git").exists()).map(Path::to_path_buf)
}

/// A repository and the branch it is on.
pub type Key = (PathBuf, String);

struct Entry {
    asked: Instant,
    running: bool,
    found: Option<PullRequest>,
}

/// What gh said, by repository and branch, with the answers of the threads that ask it still to come.
pub struct PullRequests {
    entries: HashMap<Key, Entry>,
    sender: Sender<(Key, Option<PullRequest>)>,
    answers: Receiver<(Key, Option<PullRequest>)>,
}

impl Default for PullRequests {
    fn default() -> Self {
        let (sender, answers) = channel();
        PullRequests { entries: HashMap::new(), sender, answers }
    }
}

impl PullRequests {
    /// The pull request known for `key`; asks gh for it again when what it said is older than a minute.
    pub fn get(&mut self, key: &Key, now: Instant) -> Option<&PullRequest> {
        if self.due(key, now) {
            let (sender, key) = (self.sender.clone(), key.clone());
            std::thread::spawn(move || {
                let found = ask_gh(&key.0);
                let _ = sender.send((key, found));
            });
        }
        self.entries.get(key)?.found.as_ref()
    }

    /// Whether gh is to be asked for `key` now, which marks it asked.
    fn due(&mut self, key: &Key, now: Instant) -> bool {
        let new = !self.entries.contains_key(key);
        let entry = self.entries.entry(key.clone()).or_insert(Entry { asked: now, running: false, found: None });
        let due = !entry.running && (new || now.duration_since(entry.asked) >= FRESH);
        if due {
            (entry.asked, entry.running) = (now, true);
        }
        due
    }

    /// Takes the answers that came; true when one changed what is known.
    pub fn collect(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, found)) = self.answers.try_recv() {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.running = false;
                changed |= entry.found != found;
                entry.found = found;
            }
        }
        changed
    }

    /// Forgets the repositories and branches no tab is on any more.
    pub fn retain(&mut self, keep: impl Fn(&Key) -> bool) {
        self.entries.retain(|key, _| keep(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: &str = r#"{"isDraft":false,"mergeStateStatus":"BLOCKED","number":12,"reviewDecision":"REVIEW_REQUIRED","state":"OPEN",
        "statusCheckRollup":[
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","workflowName":"CI"},
            {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":"","workflowName":"CI"},
            {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING"}],
        "url":"https://github.com/o/r/pull/12"}"#;
    const DRAFT: &str = r#"{"isDraft":true,"mergeStateStatus":"DRAFT","number":3,"reviewDecision":"","state":"OPEN","statusCheckRollup":[],"url":"https://github.com/o/r/pull/3"}"#;
    const FAILING: &str = r#"{"isDraft":false,"mergeStateStatus":"UNSTABLE","number":7,"reviewDecision":"CHANGES_REQUESTED","state":"OPEN",
        "statusCheckRollup":[
            {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE"},
            {"__typename":"CheckRun","name":"e2e","status":"COMPLETED","conclusion":"TIMED_OUT"},
            {"__typename":"StatusContext","context":"deploy","state":"ERROR"},
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"}],
        "url":"https://github.com/o/r/pull/7"}"#;
    const PASSING: &str = r#"{"isDraft":false,"mergeStateStatus":"CLEAN","number":8,"reviewDecision":"APPROVED","state":"OPEN",
        "statusCheckRollup":[
            {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED"},
            {"__typename":"StatusContext","context":"ci","state":"SUCCESS"}],
        "url":"https://github.com/o/r/pull/8"}"#;
    const MERGED: &str = r#"{"isDraft":false,"mergeStateStatus":"UNKNOWN","number":5,"reviewDecision":"APPROVED","state":"MERGED","statusCheckRollup":null,"url":"https://github.com/o/r/pull/5"}"#;

    #[test]
    fn an_open_pull_request_counts_its_checks_by_how_they_stand() {
        let pr = parse(OPEN).unwrap();
        assert_eq!((pr.number, pr.state), (12, State::Open));
        assert_eq!(pr.checks, Checks { pending: 2, passing: 1, failing: 0 });
        assert_eq!(pr.review.as_deref(), Some("REVIEW_REQUIRED"));
        assert_eq!(pr.merge_state.as_deref(), Some("BLOCKED"));
        assert_eq!(pr.url, "https://github.com/o/r/pull/12");
        assert_eq!(pr.summary(), "#12 open \u{b7} \u{2713}1 \u{2026}2 \u{b7} REVIEW_REQUIRED");
    }

    #[test]
    fn a_draft_failing_passing_and_merged_pull_request_read_as_such() {
        let draft = parse(DRAFT).unwrap();
        assert_eq!((draft.state, draft.checks, draft.review.as_deref()),(State::Draft, Checks::default(), None));
        assert_eq!(draft.summary(), "#3 draft");
        let failing = parse(FAILING).unwrap();
        assert_eq!(failing.checks, Checks { pending: 0, passing: 1, failing: 3 });
        assert_eq!(failing.summary(), "#7 open \u{b7} \u{2713}1 \u{2717}3 \u{b7} CHANGES_REQUESTED");
        let passing = parse(PASSING).unwrap();
        assert_eq!(passing.checks, Checks { pending: 0, passing: 3, failing: 0 });
        let merged = parse(MERGED).unwrap();
        assert_eq!((merged.state, merged.checks), (State::Merged, Checks::default()));
        assert_eq!(parse(&MERGED.replace("MERGED", "CLOSED")).unwrap().state, State::Closed);
    }

    /// The rollup of a pull request with the one entry `entry`.
    fn one(entry: &str) -> Checks {
        parse(&format!(r#"{{"number":1,"state":"OPEN","url":"u","statusCheckRollup":[{entry}]}}"#)).unwrap().checks
    }

    #[test]
    fn every_check_run_and_status_context_state_is_counted_as_gh_counts_it() {
        let (pass, fail, wait) = (Checks { passing: 1, ..Checks::default() }, Checks { failing: 1, ..Checks::default() }, Checks { pending: 1, ..Checks::default() });
        let run = |status: &str, conclusion: &str| one(&format!(r#"{{"__typename":"CheckRun","status":"{status}","conclusion":"{conclusion}"}}"#));
        for conclusion in ["SUCCESS", "NEUTRAL", "SKIPPED"] {
            assert_eq!(run("COMPLETED", conclusion), pass, "{conclusion}");
        }
        for conclusion in ["FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"] {
            assert_eq!(run("COMPLETED", conclusion), fail, "{conclusion}");
        }
        // A check GitHub gave up on, and what it may add one day: not a red badge.
        for conclusion in ["STALE", "", "SOMETHING_NEW"] {
            assert_eq!(run("COMPLETED", conclusion), wait, "{conclusion:?}");
        }
        for status in ["QUEUED", "IN_PROGRESS", "PENDING", "REQUESTED", "WAITING", ""] {
            assert_eq!(run(status, "SUCCESS"), wait, "{status:?}: what has not completed has no verdict yet");
        }
        assert_eq!(one(r#"{"__typename":"CheckRun","status":"COMPLETED","conclusion":null}"#), wait);
        assert_eq!(one(r#"{"__typename":"CheckRun","status":null}"#), wait);

        let context = |state: &str| one(&format!(r#"{{"__typename":"StatusContext","context":"ci","state":"{state}"}}"#));
        assert_eq!(context("SUCCESS"), pass);
        assert_eq!((context("FAILURE"), context("ERROR")), (fail, fail));
        assert_eq!((context("PENDING"), context("EXPECTED"), context("NEW_STATE")), (wait, wait, wait));
        assert_eq!(one(r#"{"__typename":"StatusContext","context":"ci"}"#), wait, "no state");
        assert_eq!(one("{}"), wait, "an entry of neither kind");
    }

    #[test]
    fn what_is_not_a_pull_request_is_none() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("no pull requests found for branch \"main\""), None);
        assert_eq!(parse(r#"{"state":"OPEN","url":"x"}"#), None, "no number");
    }

    #[test]
    fn missing_fields_and_nulls_read_as_nothing_or_as_defaults() {
        // Only what a badge cannot do without: the number, the state and the address a click opens.
        let bare = parse(r#"{"number":9,"state":"OPEN","url":"u"}"#).unwrap();
        assert_eq!((bare.state, bare.checks, &bare.review, &bare.merge_state), (State::Open, Checks::default(), &None, &None));
        let nulls = parse(r#"{"number":9,"state":"OPEN","url":"u","isDraft":null,"reviewDecision":null,"mergeStateStatus":null,"statusCheckRollup":null}"#).unwrap();
        assert_eq!(nulls, bare);
        assert_eq!(parse(r#"{"number":9,"state":"OPEN","url":"u","statusCheckRollup":{"nodes":[]}}"#).unwrap().checks, Checks::default(), "a rollup that is not a list");
        assert_eq!(parse(r#"{"number":9,"state":"MERGED","isDraft":true,"url":"u"}"#).unwrap().state, State::Merged, "the state comes before the draft");
        for bad in [
            r#"{"number":"9","state":"OPEN","url":"u"}"#,
            r#"{"number":-9,"state":"OPEN","url":"u"}"#,
            r#"{"number":9.5,"state":"OPEN","url":"u"}"#,
            r#"{"number":9,"state":null,"url":"u"}"#,
            r#"{"number":9,"state":"","url":"u"}"#,
            r#"{"number":9,"state":"OPEN","url":""}"#,
            r#"{"number":9,"state":"OPEN"}"#,
            r#"[{"number":9,"state":"OPEN","url":"u"}]"#,
            "null",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_unknown_state_reads_as_open() {
        assert_eq!(parse(r#"{"number":9,"state":"LOCKED","url":"u"}"#).unwrap().state, State::Open);
        assert_eq!(parse(r#"{"number":9,"state":"LOCKED","isDraft":true,"url":"u"}"#).unwrap().state, State::Draft);
    }

    #[test]
    fn a_huge_rollup_is_counted_in_full() {
        let entries: Vec<&str> = [r#"{"status":"COMPLETED","conclusion":"SUCCESS"}"#, r#"{"state":"FAILURE"}"#, r#"{"status":"QUEUED"}"#].into_iter().cycle().take(30_000).collect();
        let json = format!(r#"{{"number":1,"state":"OPEN","url":"u","statusCheckRollup":[{}]}}"#, entries.join(","));
        assert_eq!(parse(&json).unwrap().checks, Checks { pending: 10_000, passing: 10_000, failing: 10_000 });
    }

    #[test]
    fn gh_missing_signed_out_or_outside_github_gives_no_pull_request() {
        use std::{os::windows::process::ExitStatusExt, process::{ExitStatus, Output}};
        let run = |code: u32, stdout: &str| Ok(Output { status: ExitStatus::from_raw(code), stdout: stdout.as_bytes().to_vec(), stderr: Vec::new() });
        assert_eq!(answer(Err(std::io::ErrorKind::NotFound.into())), None, "gh is not installed");
        // What gh says signed out, outside GitHub, and on a branch without one: on stderr, with exit status 1.
        assert_eq!(answer(run(1, "")), None);
        assert_eq!(answer(run(1, OPEN)), None, "a failed run is not believed");
        assert_eq!(answer(run(0, "")), None);
        assert_eq!(answer(run(0, OPEN)).map(|pr| pr.number), Some(12));
    }

    #[test]
    fn an_answer_for_a_branch_no_tab_is_on_is_dropped_and_none_clears_what_was_known() {
        let mut prs = PullRequests::default();
        let (key, gone): (Key, Key) = ((PathBuf::from(r"C:\repo"), "main".into()), (PathBuf::from(r"C:\repo"), "old".into()));
        let t0 = Instant::now();
        assert!(prs.due(&key, t0) && prs.due(&gone, t0));
        prs.retain(|k| *k == key);
        prs.sender.send((gone.clone(), parse(OPEN))).unwrap();
        assert!(!prs.collect(), "the tab moved on before gh answered");
        assert!(!prs.entries.contains_key(&gone));

        prs.sender.send((key.clone(), parse(OPEN))).unwrap();
        assert!(prs.collect());
        assert!(prs.due(&key, t0 + FRESH));
        prs.sender.send((key.clone(), None)).unwrap();
        assert!(prs.collect(), "the pull request went away");
        assert_eq!(prs.entries[&key].found, None);
        // A gh that never answers keeps the branch from being asked again: there is no timeout.
        assert!(prs.due(&key, t0 + FRESH * 2));
        assert!(!prs.due(&key, t0 + FRESH * 100));
    }

    #[test]
    fn the_repository_of_a_worktree_is_its_own_folder() {
        let root = std::env::temp_dir().join(format!("nekominal-repo-{}", crate::agent::new_session()));
        let tree = root.join("app-fix");
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: ../.git/worktrees/app-fix\n").unwrap();
        assert_eq!(repo_root(&tree.join("src")).as_deref(), Some(tree.as_path()), "the `.git` file of a worktree, not the folder above");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gh_is_asked_at_once_then_not_again_for_a_minute_and_never_twice_at_a_time() {
        let mut prs = PullRequests::default();
        let key: Key = (PathBuf::from(r"C:\repo"), "main".into());
        let t0 = Instant::now();
        assert!(prs.due(&key, t0), "a branch not seen before is asked for at once");
        assert!(!prs.due(&key, t0 + FRESH * 2), "not while the last question runs");
        prs.sender.send((key.clone(), parse(OPEN))).unwrap();
        assert!(prs.collect());
        assert_eq!(prs.entries[&key].found.as_ref().map(|pr| pr.number), Some(12));
        assert!(!prs.due(&key, t0 + Duration::from_secs(59)));
        assert!(prs.due(&key, t0 + FRESH), "a minute on");
        prs.sender.send((key.clone(), parse(OPEN))).unwrap();
        assert!(!prs.collect(), "the same answer changes nothing");
        let other: Key = (PathBuf::from(r"C:\repo"), "feature".into());
        assert!(prs.due(&other, t0 + Duration::from_secs(61)), "a new branch is asked for at once");
        prs.retain(|k| *k == other);
        assert!(!prs.entries.contains_key(&key));
    }

    #[test]
    fn the_repository_of_a_folder_is_the_nearest_with_a_git() {
        let root = std::env::temp_dir().join(format!("nekominal-repo-{}", crate::agent::new_session()));
        let deep = root.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(repo_root(&deep).as_deref(), Some(root.as_path()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "runs gh, which needs a network and a sign-in"]
    fn gh_tells_the_pull_request_of_this_repository() {
        let repo = repo_root(&std::env::current_dir().unwrap()).unwrap();
        println!("{:?}", ask_gh(&repo));
    }
}
