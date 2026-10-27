//! What the badges of a tab will show (nothing draws them yet): the pull request of the branch its folder is on
//! (`github.rs`), and the TCP ports the programs of its panes listen on (`ports.rs`). Read with the job tick, once a second.

use super::*;
use crate::{agent, github, ports};

#[derive(Default)]
pub struct Badges {
    prs: github::PullRequests,
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
    pub(super) fn refresh_badges(&mut self) -> bool {
        let (badges, now) = (&mut self.badges, Instant::now());
        let listeners = ports::listeners();
        badges.prs.collect();
        let mut changed = false;
        let mut keys = Vec::new();
        for tab in &self.tabs {
            let pids: Vec<u32> = tab.panes.iter().flat_map(|entry| entry.pane.job_process_ids()).collect();
            let found = ports::ports_of(&listeners, &pids);
            changed |= set(&mut badges.ports, tab.uid, (!found.is_empty()).then_some(found), "ports");
            // git_branch reads only local disks: gh never runs for a share.
            let key = tab.focused_entry().and_then(|entry| {
                let dir = entry.usage.as_ref().and_then(|u| u.cwd.clone()).or_else(|| entry.cwd.clone())?;
                let dir = std::path::Path::new(&dir);
                let branch = agent::git_branch(dir)?;
                Some((github::repo_root(dir)?, branch))
            });
            let pr = key.as_ref().and_then(|key| badges.prs.get(key, now).cloned());
            changed |= set(&mut badges.pull_request, tab.uid, pr, "pull request");
            keys.extend(key);
        }
        badges.prs.retain(|key| keys.contains(key));
        let open = |uid: &u64| self.tabs.iter().any(|tab| tab.uid == *uid);
        badges.ports.retain(|uid, _| open(uid));
        badges.pull_request.retain(|uid, _| open(uid));
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
