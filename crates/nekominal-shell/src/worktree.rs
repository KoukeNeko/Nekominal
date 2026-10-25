//! A git worktree for an agent: a second checkout of the repository on a branch of its own, so that two agents (or an agent
//! and the user) work on one repository without touching each other's files.
//!
//! Where it goes: beside the main checkout, `<parent>\<repo>-<branch>`, so `D:\src\app` and the branch `fix/login` make
//! `D:\src\app-fix-login`. That is the layout of the examples of `git worktree` itself (`../hotfix`) and what most worktree
//! helpers default to. A folder inside the repository (`.worktrees\<branch>`, or Claude Code's `.claude\worktrees`) needs
//! an ignore rule in every repository, or the `git status`, the searches and the file watchers of the main checkout all see
//! the second one.
//!
//! git runs as a process with its arguments, never through a shell, and without a console window.

use std::{
    ffi::OsString,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

/// `CREATE_NO_WINDOW`: git is a console program, and this one has no console for it to borrow.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The folder is not in a git repository (or git is not installed).
    NotRepo,
    /// What git said when it refused.
    Git(String),
}

/// Whether git takes `name` as the name of a new branch: the rules of `git check-ref-format --branch`, checked as the name
/// is typed, before git is asked (which it is again when the worktree is made).
pub fn valid_branch(name: &str) -> bool {
    let bad_char = |c: char| c.is_ascii_control() || " ~^:?*[\\".contains(c);
    !name.is_empty()
        && name != "HEAD"
        && name != "@"
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.chars().any(bad_char)
        && name.split('/').all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

/// The folder of the worktree for `branch` beside the repository at `root`: `<root>-<branch>`, with what a folder name
/// cannot hold (the `/` of `fix/login`, and `<>|"`, which git allows) made `-`; `-2`, `-3`… after it while one exists.
pub fn worktree_path(root: &Path, branch: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let repo = root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".to_owned());
    let folder: String = branch.chars().map(|c| if "/<>|\"".contains(c) { '-' } else { c }).collect();
    let base = format!("{repo}-{folder}");
    let parent = root.parent().unwrap_or(root);
    (1..)
        .map(|n| parent.join(if n == 1 { base.clone() } else { format!("{base}-{n}") }))
        .find(|path| !exists(path))
        .expect("an unused name is found")
}

/// The arguments of `git` that make the worktree at `path` on the new branch `branch`.
pub fn add_args(path: &Path, branch: &str) -> Vec<OsString> {
    vec!["worktree".into(), "add".into(), "-b".into(), branch.into(), "--".into(), path.into()]
}

/// Runs git in `dir` with `args`: what it wrote, or the last line of what it complained of.
fn git<I, S>(dir: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new("git").arg("-C").arg(dir).args(args).creation_flags(CREATE_NO_WINDOW).output().map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("git failed").trim().to_owned())
}

/// The folder of the main checkout of the repository `dir` is in, also when `dir` is in one of its worktrees: the worktrees
/// of a repository then all sit beside the same folder.
pub fn repository_root(dir: &Path) -> Result<PathBuf, Error> {
    let common = git(dir, ["rev-parse", "--path-format=absolute", "--git-common-dir"]).map_err(|_| Error::NotRepo)?;
    let common = PathBuf::from(common);
    match common.parent() {
        Some(root) if common.file_name().is_some_and(|name| name == ".git") => Ok(root.to_path_buf()),
        // A bare repository or a separate git folder: the top of the checkout `dir` is in.
        _ => git(dir, ["rev-parse", "--show-toplevel"]).map(PathBuf::from).map_err(|_| Error::NotRepo),
    }
}

/// Makes a worktree on the new branch `branch` for the repository `dir` is in; its folder.
// shortcut: runs on the thread of the window, which waits while git checks the files out; move it to a thread if a large
// repository makes that wait felt.
pub fn create(dir: &Path, branch: &str) -> Result<PathBuf, Error> {
    let root = repository_root(dir)?;
    git(&root, ["check-ref-format", "--branch", branch]).map_err(Error::Git)?;
    let path = worktree_path(&root, branch, Path::exists);
    git(&root, add_args(&path, branch)).map_err(Error::Git)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_follow_the_rules_of_git_check_ref_format() {
        for good in ["fix", "fix/login", "agent-1", "feature/新功能", "v1.2", "a@b", "x.lockx"] {
            assert!(valid_branch(good), "{good}");
        }
        for bad in ["", "-x", "HEAD", "@", "a..b", "a b", "a~1", "a^", "a:b", "a?", "a*", "a[b", r"a\b", "a/", "/a", "a//b", ".a", "a/.b", "a.lock", "a/b.lock/c", "a.", "a@{1}", "a\tb", "a\u{7f}"] {
            assert!(!valid_branch(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_worktree_goes_beside_the_repository_named_after_it_and_the_branch() {
        let root = Path::new(r"D:\src\app");
        assert_eq!(worktree_path(root, "fix", |_| false), Path::new(r"D:\src\app-fix"));
        assert_eq!(worktree_path(root, "fix/login", |_| false), Path::new(r"D:\src\app-fix-login"), "a slash is not a folder");
        assert_eq!(worktree_path(root, "a<b>|\"c", |_| false), Path::new(r"D:\src\app-a-b---c"));
        let taken = [Path::new(r"D:\src\app-fix").to_path_buf(), Path::new(r"D:\src\app-fix-2").to_path_buf()];
        assert_eq!(worktree_path(root, "fix", |p| taken.iter().any(|t| t == p)), Path::new(r"D:\src\app-fix-3"), "a folder that exists is left alone");
    }

    #[test]
    fn git_is_asked_for_a_new_branch_and_the_path_after_the_end_of_the_options() {
        let args = add_args(Path::new(r"D:\src\app-fix"), "fix");
        assert_eq!(args, ["worktree", "add", "-b", "fix", "--", r"D:\src\app-fix"].map(OsString::from));
    }

    /// Makes a repository with a commit in a fresh temporary folder, makes a worktree of it and removes it again. Skipped
    /// where git is not on the PATH.
    #[test]
    fn a_real_worktree_is_made_beside_the_repository_and_can_be_removed() {
        if Command::new("git").arg("--version").creation_flags(CREATE_NO_WINDOW).output().is_err() {
            eprintln!("git is not on the PATH: skipped");
            return;
        }
        let base = std::env::temp_dir().join(format!("nekominal-worktree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, ["init", "-q"]).unwrap();
        git(&repo, ["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "start"]).unwrap();
        let sub = repo.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(create(&base, "x"), Err(Error::NotRepo), "a folder outside the repository");

        let made = create(&sub, "agent/one").unwrap();
        assert_eq!(made.file_name().unwrap(), "repo-agent-one");
        assert!(made.join(".git").is_file(), "a worktree has a .git file");
        assert_eq!(git(&made, ["branch", "--show-current"]).unwrap(), "agent/one");
        // From inside the worktree the next one still goes beside the main checkout.
        assert_eq!(repository_root(&made).unwrap().canonicalize().unwrap(), repo.canonicalize().unwrap());
        assert!(matches!(create(&made, "agent/one"), Err(Error::Git(_))), "the branch exists");
        assert!(matches!(create(&repo, "bad..name"), Err(Error::Git(_))), "git checks the name too");

        git(&repo, [OsString::from("worktree"), "remove".into(), made.clone().into()]).unwrap();
        assert!(!made.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
