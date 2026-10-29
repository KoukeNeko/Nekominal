//! A git worktree for an agent: a second checkout of the repository on a branch of its own, so that two agents (or an agent
//! and the user) work on one repository without touching each other's files.
//!
//! Where it goes: beside the main checkout, `<parent>\<repo>-<branch>`, so `D:\src\app` and the branch `fix/login` make
//! `D:\src\app-fix-login`. That is the layout of the examples of `git worktree` itself (`../hotfix`) and what most worktree
//! helpers default to. A folder inside the repository (`.worktrees\<branch>`, or Claude Code's `.claude\worktrees`) needs
//! an ignore rule in every repository, or the `git status`, the searches and the file watchers of the main checkout all see
//! the second one.
//!
//! A new worktree gets the files git ignores that `.worktreeinclude` names (`.env`…: see [`copy_included`]), and its tab
//! starts with the command of the setup script of the repository, `.nekominal\setup.ps1` or `.nekominal\setup.cmd`, if
//! there is one, typed for the user to run ([`setup_command`]).
//!
//! A worktree is removed (`git worktree remove`, then its branch with `git branch -d`) or archived (the folder goes, the
//! branch stays): never the main checkout, and never with changes not committed unless forced ([`check_remove`]).
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
    /// Asked of nobody: the rules below say no before git is run.
    Refused(Refusal),
    /// A part of the branch name is a name Windows keeps for a device (`aux`, `con`…), which git cannot store.
    Reserved(String),
}

/// Why a worktree is not removed or archived.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The main checkout: it holds the repository itself, and is never removed.
    Main,
    /// Changes not committed and files not tracked, which removing the folder would lose.
    Dirty { changed: usize, untracked: usize },
    /// On no branch: nothing would keep its commits once the folder goes.
    Detached,
}

/// What a worktree holds, as git sees it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The top of the worktree.
    pub path: PathBuf,
    /// None when HEAD is on no branch.
    pub branch: Option<String>,
    /// The main checkout of the repository, not a worktree added to it.
    pub main: bool,
    /// Lines of `git status --porcelain`: changed tracked files (staged or not), and files not tracked.
    pub changed: usize,
    pub untracked: usize,
    /// Files and folders git ignores (`!!` of `--ignored`: `.env`, build output): no change to git, but the removal of the
    /// folder deletes them too.
    pub ignored: usize,
    /// Those files with the letter of their state (`M`, `A`, `D`, `R`, `?` for untracked…), in the order git lists them.
    pub files: Vec<(char, String)>,
    /// The remote the branch pushes to (its upstream's, else the first one); None when the repository has none.
    pub remote: Option<String>,
    /// Commits ahead of the upstream, or, with no upstream, on no remote branch; None when git could not tell.
    pub unpushed: Option<usize>,
    /// Commits the upstream has that HEAD has not; None with no upstream.
    pub behind: Option<usize>,
    /// The default branch (`origin/HEAD`, else `main` or `master`) and whether HEAD is merged into it.
    pub default_branch: Option<String>,
    pub merged: Option<bool>,
}

impl Status {
    pub fn dirty(&self) -> bool {
        self.changed + self.untracked > 0
    }
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

/// The part of `name` that is a name Windows keeps for a device (`aux`, `con`, `nul`, `prn`, `com1`…`com9`, `lpt1`…`lpt9`,
/// in any case, with any extension): git stores a branch as files named after its parts, and no file can have such a name.
pub fn reserved_part(name: &str) -> Option<&str> {
    name.split('/').find(|part| {
        let stem = part.split('.').next().unwrap_or(part).trim_end().to_ascii_lowercase();
        matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
            || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0')
    })
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

/// Which of `paths` git ignores in the repository `root`, separated by NUL. The paths go to git on its standard input, as
/// many as there are. Exit status 1 is "none of them is ignored"; any other failure is an error, not an empty answer.
fn check_ignore(root: &Path, paths: &[&str]) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    let input = paths.join("\0").into_bytes();
    let mut stdin = child.stdin.take().expect("piped");
    // Written from a thread of its own: git answers while it reads, and a full pipe both ways would hold both.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    let _ = writer.join();
    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        Some(1) => Ok(String::new()),
        _ => Err(String::from_utf8_lossy(&output.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("git check-ignore failed").trim().to_owned()),
    }
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
pub fn create(dir: &Path, branch: &str) -> Result<(PathBuf, Option<String>), Error> {
    let root = repository_root(dir)?;
    git(&root, ["check-ref-format", "--branch", branch]).map_err(Error::Git)?;
    if let Some(part) = reserved_part(branch) {
        return Err(Error::Reserved(part.to_owned()));
    }
    let path = worktree_path(&root, branch, Path::exists);
    git(&root, add_args(&path, branch)).map_err(Error::Git)?;
    let copy_failed = match copy_included(&root, &path) {
        Ok(copied) => {
            if !copied.is_empty() {
                crate::util::log(&format!("worktree: copied {} files", copied.len()));
            }
            None
        }
        Err(e) => {
            crate::util::log(&format!("worktree: .worktreeinclude: {e}"));
            Some(e)
        }
    };
    Ok((path, copy_failed))
}

/// Copies into the new worktree `dest` the files of the main checkout `root` that `.worktreeinclude` names and git ignores
/// (`.env` and the like, which a checkout does not bring); a file already there is left alone. The files copied, as git
/// names them.
///
/// `.worktreeinclude` is the convention of Claude Code: patterns in the syntax of `.gitignore`, relative to the root of the
/// repository. git matches them: the files not tracked that the patterns take, and of those the ones git ignores.
pub fn copy_included(root: &Path, dest: &Path) -> Result<Vec<String>, String> {
    let include = root.join(".worktreeinclude");
    if !include.is_file() {
        return Ok(Vec::new());
    }
    let mut exclude_from = OsString::from("--exclude-from=");
    exclude_from.push(&include);
    let listed = git(root, [OsString::from("ls-files"), "-z".into(), "--others".into(), "--ignored".into(), exclude_from])?;
    let candidates: Vec<&str> = listed.split('\0').filter(|p| !p.is_empty()).collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    // On stdin, not the command line: that holds about 32 000 characters, a few hundred paths of node_modules.
    let ignored = check_ignore(root, &candidates)?;
    let mut copied = Vec::new();
    for file in ignored.split('\0').filter(|p| !p.is_empty()) {
        let to = dest.join(file);
        if to.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{file}: {e}"))?;
        }
        std::fs::copy(root.join(file), &to).map_err(|e| format!("{file}: {e}"))?;
        copied.push(file.to_owned());
    }
    Ok(copied)
}

/// What to type in the first tab of a new worktree to set it up, when the repository has a setup script: `.nekominal\setup.ps1`
/// (run by Windows PowerShell, whatever the execution policy) or else `.nekominal\setup.cmd`. Typed into the shell, not run
/// hidden, so the user sees it, and not run until the user presses Enter: the script comes with the repository.
///
/// One command line for every shell a profile runs: cmd, PowerShell, Git Bash and WSL. A forward slash, as bash eats a
/// backslash; and the `.cmd` goes through PowerShell too, as cmd takes `/` in a program's path for a switch and Git Bash
/// turns cmd's `/c` into `C:/`.
pub fn setup_command(worktree: &Path) -> Option<String> {
    let dir = worktree.join(".nekominal");
    if dir.join("setup.ps1").is_file() {
        Some("powershell.exe -NoProfile -ExecutionPolicy Bypass -File .nekominal/setup.ps1".to_owned())
    } else if dir.join("setup.cmd").is_file() {
        Some("powershell.exe -NoProfile -Command .nekominal/setup.cmd".to_owned())
    } else {
        None
    }
}

/// Changed and untracked files in the output of `git status --porcelain`; the ignored ones (`!!`) are counted apart.
pub fn parse_porcelain(text: &str) -> (usize, usize) {
    let lines = text.lines().filter(|line| !line.trim().is_empty() && !ignored_line(line));
    lines.fold((0, 0), |(changed, untracked), line| if line.starts_with("??") { (changed, untracked + 1) } else { (changed + 1, untracked) })
}

/// The files of `git status --porcelain` with the letter of their state: the first of the two columns that is set, `?` for
/// a file not tracked. Read after `git` trims the output, which takes the leading space of the first line.
///
/// git puts a path with a space in quotes (`"my file.txt"`, and each side of a rename); they are taken off. The escapes
/// inside such quotes are for `"`, `\` and control characters, none of which a Windows file name holds, nor the `>` of
/// the ` -> ` of a rename.
pub fn parse_files(text: &str) -> Vec<(char, String)> {
    let unquote = |path: &str| path.strip_prefix('"').and_then(|p| p.strip_suffix('"')).unwrap_or(path).to_owned();
    text.lines()
        .filter(|line| !ignored_line(line))
        .filter_map(|line| {
            let (code, path) = line.trim_start().split_once(' ')?;
            let path = path.trim_start().split(" -> ").map(unquote).collect::<Vec<_>>().join(" -> ");
            Some((code.chars().next()?, path))
        })
        .collect()
}

/// A line of `git status --porcelain --ignored` for a file or folder git ignores.
fn ignored_line(line: &str) -> bool {
    line.trim_start().starts_with("!!")
}

/// (behind, ahead) in the output of `git rev-list --left-right --count @{upstream}...HEAD`.
pub fn parse_ahead_behind(text: &str) -> Option<(usize, usize)> {
    let mut counts = text.split_whitespace().map(str::parse::<usize>);
    match (counts.next(), counts.next(), counts.next()) {
        (Some(Ok(behind)), Some(Ok(ahead)), None) => Some((behind, ahead)),
        _ => None,
    }
}

/// The last commit of HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    /// When it was committed, in seconds since 1970.
    pub at: u64,
}

/// What the card of the branch in the title bar shows of a folder: what [`inspect`] finds, with where the branch pushes and
/// how far from it it is, the last commit and the stashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub status: Status,
    /// The address of the remote (`https://github.com/a/b.git`, `git@github.com:a/b.git`).
    pub url: Option<String>,
    /// The branch it pushes to (`origin/master`).
    pub upstream: Option<String>,
    pub last: Option<Commit>,
    pub stashes: usize,
}

/// [`inspect`] and the rest of what the card shows; a few more runs of git, read where nothing waits on them. What git said
/// when the folder is not one it can read (no repository, a folder it does not trust), as it said it.
pub fn overview(dir: &Path) -> Result<Overview, String> {
    git(dir, ["rev-parse", "--show-toplevel"])?;
    let status = inspect(dir).map_err(|e| match e {
        Error::Git(said) => said,
        other => format!("{other:?}"),
    })?;
    let path = status.path.clone();
    let url = status.remote.as_ref().and_then(|remote| git(&path, ["remote", "get-url", remote]).ok()).filter(|url| !url.is_empty());
    let upstream = git(&path, ["rev-parse", "--abbrev-ref", "@{upstream}"]).ok().filter(|name| !name.is_empty());
    let last = git(&path, ["log", "-1", "--format=%h%x1f%s%x1f%an%x1f%ct"]).ok().and_then(|line| parse_commit(&line));
    // No stash is no refs/stash, which git reports as an error.
    let stashes = git(&path, ["rev-list", "--walk-reflogs", "--count", "refs/stash"]).ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    Ok(Overview { status, url, upstream, last, stashes })
}

/// A commit from the line `git log --format=%h%x1f%s%x1f%an%x1f%ct` printed.
fn parse_commit(line: &str) -> Option<Commit> {
    let mut parts = line.split('\u{1f}');
    let (hash, subject, author, at) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(Commit { hash: hash.to_owned(), subject: subject.to_owned(), author: author.to_owned(), at: at.trim().parse().ok()? })
}

/// Where a remote is, as people name it: its host and its path without `.git` (`github.com`, `KoukeNeko/Nekominal`), from
/// an https or ssh address or the `user@host:path` form of scp; a local folder is its path, with no host.
pub fn remote_place(url: &str) -> (Option<&str>, &str) {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        // Without the user (`git@`, a token) and the port.
        let host = authority.rsplit('@').next().unwrap_or(authority);
        return (Some(host.split(':').next().unwrap_or(host)).filter(|host| !host.is_empty()), path);
    }
    // `git@github.com:a/b`, not a drive (`C:\repo`, `C:/repo`).
    match url.split_once(':') {
        Some((host, path)) if host.len() > 1 && !host.contains(['/', '\\']) => (Some(host.rsplit_once('@').map_or(host, |(_, h)| h)), path),
        _ => (None, url),
    }
}

/// Inspects the worktree `dir` is in.
pub fn inspect(dir: &Path) -> Result<Status, Error> {
    let dirs = git(dir, ["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"]).map_err(|_| Error::NotRepo)?;
    let [top, git_dir, common] = dirs.lines().collect::<Vec<_>>()[..] else { return Err(Error::NotRepo) };
    let path = PathBuf::from(top.replace('/', "\\"));
    let branch = git(&path, ["branch", "--show-current"]).ok().filter(|b| !b.is_empty());
    let porcelain = git(&path, ["-c", "core.quotePath=false", "status", "--porcelain", "--ignored"]).map_err(Error::Git)?;
    let (changed, untracked) = parse_porcelain(&porcelain);
    let ignored = porcelain.lines().filter(|line| ignored_line(line)).count();
    let remote = branch
        .as_ref()
        .and_then(|b| git(&path, ["config", "--get", &format!("branch.{b}.remote")]).ok())
        .or_else(|| git(&path, ["remote"]).ok().and_then(|names| names.lines().next().map(str::to_owned)))
        .filter(|name| !name.is_empty());
    let (unpushed, behind) = match git(&path, ["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]) {
        Ok(counts) => parse_ahead_behind(&counts).map_or((None, None), |(behind, ahead)| (Some(ahead), Some(behind))),
        Err(_) => (git(&path, ["rev-list", "--count", "HEAD", "--not", "--remotes"]).ok().and_then(|n| n.parse().ok()), None),
    };
    let default_branch = git(&path, ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .or_else(|| ["main", "master"].into_iter().find(|b| git(&path, ["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok()).map(str::to_owned));
    let merged = default_branch.as_ref().map(|b| git(&path, ["merge-base", "--is-ancestor", "HEAD", b]).is_ok());
    Ok(Status { path, branch, main: same_path(git_dir, common), changed, untracked, ignored, files: parse_files(&porcelain), remote, unpushed, behind, default_branch, merged })
}

/// Paths git wrote, compared as Windows does: either slash, any case, no trailing separator.
fn normalize(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn same_path(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

/// Whether the folder `dir` (a pane's working directory) is `root` or inside it.
pub fn contains(root: &Path, dir: &str) -> bool {
    let (root, dir) = (normalize(&root.to_string_lossy()), normalize(dir));
    dir == root || dir.strip_prefix(&root).is_some_and(|rest| rest.starts_with('\\'))
}

/// Whether the worktree may be removed: never the main checkout, and not with changes unless `force`.
pub fn check_remove(status: &Status, force: bool) -> Result<(), Refusal> {
    if status.main {
        return Err(Refusal::Main);
    }
    if status.dirty() && !force {
        return Err(Refusal::Dirty { changed: status.changed, untracked: status.untracked });
    }
    Ok(())
}

/// Whether the worktree may be archived (its folder goes, its branch stays): never forced, so never with changes, and
/// only on a branch, which keeps the work.
pub fn check_archive(status: &Status) -> Result<(), Refusal> {
    check_remove(status, false)?;
    if status.branch.is_none() {
        return Err(Refusal::Detached);
    }
    Ok(())
}

/// Removes the worktree `status` describes, with `git worktree remove` (`--force` when `force`), and then its branch when
/// `delete_branch`: `git branch -d`, which keeps a branch not merged, or `-D` when `force`. What git said when it kept the
/// branch, or None.
pub fn remove(status: &Status, force: bool, delete_branch: bool) -> Result<Option<String>, Error> {
    check_remove(status, force).map_err(Error::Refused)?;
    // From the main checkout: the folder being removed is no place to run git in.
    let root = repository_root(&status.path)?;
    let mut args: Vec<OsString> = vec!["worktree".into(), "remove".into()];
    if force {
        args.push("--force".into());
    }
    args.push(status.path.clone().into());
    if let Err(said) = git(&root, args) {
        // A folder a process still has as its working directory: git removed the files and the worktree, then failed on
        // the folder itself. The worktree is gone (its `.git` file with it), so the branch goes on as asked; the empty
        // folder stays while the process holds it.
        if status.path.join(".git").exists() {
            return Err(Error::Git(said));
        }
        let _ = std::fs::remove_dir(&status.path);
    }
    match &status.branch {
        // Without the advice git writes after "not fully merged", which would be the last line, the one told.
        Some(branch) if delete_branch => Ok(git(&root, ["-c", "advice.forceDeleteBranch=false", "branch", if force { "-D" } else { "-d" }, branch]).err()),
        _ => Ok(None),
    }
}

/// Archives the worktree: removes its folder and keeps its branch, so the work stays in git.
pub fn archive(status: &Status) -> Result<(), Error> {
    check_archive(status).map_err(Error::Refused)?;
    remove(status, false, false).map(|_| ())
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
    fn the_folder_of_a_worktree_keeps_spaces_and_unicode_and_a_drive_root_has_one_too() {
        let none = |_: &Path| false;
        assert_eq!(worktree_path(Path::new(r"D:\my src\應用 app"), "功能/登入", none), Path::new(r"D:\my src\應用 app-功能-登入"));
        assert_eq!(worktree_path(Path::new(r"D:\"), "fix", none), Path::new(r"D:\repo-fix"), "a repository at the root of a drive");
        assert_eq!(worktree_path(Path::new(r"\\nas\share\app"), "fix", none), Path::new(r"\\nas\share\app-fix"));
    }

    /// The branch names to hold `valid_branch` and git to: those of the test above, and a few more.
    const NAMES: &[&str] = &[
        "fix", "fix/login", "agent-1", "feature/新功能", "v1.2", "a@b", "x.lockx", "a\"b", "a<b>|c", "a/b/c", "UPPER", "", "-x", "HEAD", "a..b",
        "a b", "a~1", "a^", "a:b", "a?", "a*", "a[b", r"a\b", "a/", "/a", "a//b", ".a", "a/.b", "a.lock", "a/b.lock/c", "a.", "a@{1}", "@{-1}",
        "a\tb", "a\u{7f}", "--force",
    ];

    #[test]
    fn branch_names_agree_with_git_check_ref_format() {
        let Some((base, repo)) = temp_repo("refs") else { return };
        let long = "a".repeat(300);
        for name in NAMES.iter().copied().chain([long.as_str()]) {
            let git_says = git(&repo, ["check-ref-format", "--branch", name]).is_ok();
            assert_eq!(valid_branch(name), git_says, "{name:?}");
        }
        // The one difference: git reads `@` as the branch checked out, which is no name for a new one.
        assert!(git(&repo, ["check-ref-format", "--branch", "@"]).is_ok() && !valid_branch("@"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_branch_name_too_long_for_windows_is_refused_by_git_and_leaves_nothing_behind() {
        let Some((base, repo)) = temp_repo("long") else { return };
        let long = "a".repeat(260);
        assert!(valid_branch(&long));
        assert!(matches!(create(&repo, &long), Err(Error::Git(_))));
        let folders: Vec<_> = std::fs::read_dir(&base).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(folders, ["repo"], "no folder for the worktree");
        assert_eq!(git(&repo, ["branch", "--list"]).unwrap(), "* main", "no branch");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_part_named_for_a_windows_device_is_found_whatever_its_case_or_extension() {
        for name in ["aux", "AUX", "aux.txt", "fix/aux", "x/con/y", "com1", "lpt9", "nul ", "Prn.c"] {
            assert!(reserved_part(name).is_some(), "{name}");
        }
        assert_eq!(reserved_part("fix/Aux.v2"), Some("Aux.v2"));
        for name in ["com0", "auxx", "fix", "my-con", "lpt", "conx/a"] {
            assert_eq!(reserved_part(name), None, "{name}");
        }
    }

    #[test]
    fn a_branch_named_for_a_windows_device_is_refused_and_leaves_nothing_behind() {
        let Some((base, repo)) = temp_repo("reserved") else { return };
        assert_eq!(create(&repo, "fix/aux"), Err(Error::Reserved("aux".to_owned())));
        let folders: Vec<_> = std::fs::read_dir(&base).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(folders, ["repo"], "no folder for the worktree");
        assert_eq!(git(&repo, ["branch", "--list"]).unwrap(), "* main", "no branch");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_real_worktree_takes_the_next_free_folder_in_a_path_with_spaces_and_unicode() {
        let Some((base, repo)) = temp_repo("my repo 新") else { return };
        std::fs::create_dir_all(base.join("repo-feature-新")).unwrap();
        std::fs::create_dir_all(base.join("repo-feature-新-2")).unwrap();
        let made = create(&repo, "feature/新").unwrap().0;
        assert_eq!(made.file_name().unwrap(), "repo-feature-新-3");
        let status = inspect(&made).unwrap();
        assert_eq!((status.branch.as_deref(), status.main), (Some("feature/新"), false));
        assert!(contains(&status.path, &made.to_string_lossy()));
        std::fs::write(made.join("筆記 one.txt"), "x").unwrap();
        assert_eq!(inspect(&made).unwrap().files, [('?', "筆記 one.txt".to_owned())]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_repository_with_no_commits_yet_has_no_counts_and_no_default_branch() {
        let Some((base, _)) = temp_repo("unborn") else { return };
        let repo = base.join("empty");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, ["init", "-q", "-b", "main"]).unwrap();
        let status = inspect(&repo).unwrap();
        assert!(status.main && !status.dirty());
        assert_eq!(status.branch.as_deref(), Some("main"), "the branch to be");
        assert_eq!((status.unpushed, &status.default_branch, status.merged), (None, &None, None));
        // git 2.42 and later make an orphan branch; older ones refuse. Either way nothing breaks.
        match create(&repo, "first").map(|(path, _)| path) {
            Ok(made) => assert_eq!(inspect(&made).unwrap().branch.as_deref(), Some("first")),
            Err(e) => assert!(matches!(e, Error::Git(_)), "{e:?}"),
        }
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_bare_repository_is_not_a_place_for_a_worktree() {
        let Some((base, _)) = temp_repo("bare") else { return };
        let bare = base.join("bare.git");
        git(&base, ["init", "-q", "--bare", "bare.git"]).unwrap();
        assert_eq!(create(&bare, "x"), Err(Error::NotRepo));
        assert_eq!(inspect(&bare), Err(Error::NotRepo));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_submodule_is_the_main_checkout_of_its_own_repository() {
        let Some((base, repo)) = temp_repo("submodule") else { return };
        let lib = base.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        git(&lib, ["init", "-q", "-b", "main"]).unwrap();
        commit(&lib, "lib");
        // A local path as the URL, which git allows only when asked.
        git(&repo, [OsString::from("-c"), "protocol.file.allow=always".into(), "submodule".into(), "add".into(), "-q".into(), lib.clone().into(), "lib".into()]).unwrap();
        let sub = repo.join("lib");
        let status = inspect(&sub).unwrap();
        assert!(status.main, "a submodule is never removed as a worktree");
        assert_eq!(check_remove(&status, true), Err(Refusal::Main));
        assert_eq!(repository_root(&sub).unwrap().canonicalize().unwrap(), sub.canonicalize().unwrap());
        let made = create(&sub, "x").unwrap().0;
        assert_eq!(made.parent().unwrap().canonicalize().unwrap(), repo.canonicalize().unwrap(), "beside the folder of the submodule");
        git(&sub, [OsString::from("worktree"), "remove".into(), made.into()]).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_detached_head_has_no_branch_and_is_not_archived() {
        let Some((base, repo)) = temp_repo("detached") else { return };
        let made = create(&repo, "agent/detached").unwrap().0;
        git(&made, ["checkout", "-q", "--detach"]).unwrap();
        let status = inspect(&made).unwrap();
        assert_eq!((status.branch.as_deref(), status.main), (None, false));
        assert_eq!(archive(&status), Err(Error::Refused(Refusal::Detached)));
        assert!(made.exists());
        git(&repo, [OsString::from("worktree"), "remove".into(), made.into()]).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn every_kind_of_change_is_counted_with_its_letter() {
        let Some((base, repo)) = temp_repo("changes") else { return };
        for (file, text) in [("a.txt", "a"), ("b.txt", "b"), ("old name.txt", "o"), (".gitignore", "*.log\n")] {
            std::fs::write(repo.join(file), text).unwrap();
        }
        git(&repo, ["add", "-A"]).unwrap();
        commit(&repo, "files");
        std::fs::write(repo.join("build.log"), "x").unwrap();
        let ignored = inspect(&repo).unwrap();
        assert!(!ignored.dirty(), "files git ignores are no changes");

        std::fs::write(repo.join("a.txt"), "changed").unwrap();
        std::fs::write(repo.join("new.txt"), "n").unwrap();
        git(&repo, ["add", "new.txt"]).unwrap();
        git(&repo, ["mv", "old name.txt", "new name.txt"]).unwrap();
        std::fs::write(repo.join("my notes.txt"), "u").unwrap();
        let status = inspect(&repo).unwrap();
        assert_eq!((status.changed, status.untracked), (3, 1));
        for file in [('M', "a.txt"), ('A', "new.txt"), ('R', "old name.txt -> new name.txt"), ('?', "my notes.txt")] {
            assert!(status.files.contains(&(file.0, file.1.to_owned())), "{file:?} in {:?}", status.files);
        }

        git(&repo, ["reset", "-q", "--hard"]).unwrap();
        std::fs::remove_file(repo.join("my notes.txt")).unwrap();
        git(&repo, ["checkout", "-q", "-b", "side"]).unwrap();
        std::fs::write(repo.join("b.txt"), "side").unwrap();
        git(&repo, ["add", "b.txt"]).unwrap();
        commit(&repo, "side");
        git(&repo, ["checkout", "-q", "main"]).unwrap();
        std::fs::write(repo.join("b.txt"), "main").unwrap();
        git(&repo, ["add", "b.txt"]).unwrap();
        commit(&repo, "main");
        assert!(git(&repo, ["-c", "user.name=t", "-c", "user.email=t@example.com", "merge", "-q", "side"]).is_err(), "a conflict");
        let conflicted = inspect(&repo).unwrap();
        assert_eq!((conflicted.changed, &conflicted.files[..]), (1, &[('U', "b.txt".to_owned())][..]));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn pushed_ahead_behind_and_unpushed_commits_are_read_against_the_remote() {
        let Some((base, repo)) = temp_repo("remote") else { return };
        git(&base, ["init", "-q", "--bare", "remote.git"]).unwrap();
        git(&repo, [OsString::from("remote"), "add".into(), "origin".into(), base.join("remote.git").into()]).unwrap();
        git(&repo, ["push", "-q", "-u", "origin", "main"]).unwrap();
        let pushed = inspect(&repo).unwrap();
        assert_eq!((pushed.remote.as_deref(), pushed.unpushed), (Some("origin"), Some(0)));
        assert_eq!((pushed.default_branch.as_deref(), pushed.merged), (Some("main"), Some(true)), "no origin/HEAD: the local main");

        commit(&repo, "one");
        commit(&repo, "two");
        assert_eq!(inspect(&repo).unwrap().unpushed, Some(2), "ahead of the upstream");
        git(&repo, ["push", "-q"]).unwrap();
        git(&repo, ["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        assert_eq!(inspect(&repo).unwrap().unpushed, Some(0), "only behind: nothing to push");
        git(&repo, ["pull", "-q", "--ff-only"]).unwrap();

        // A branch never pushed: no upstream, so the commits on no remote branch.
        let made = create(&repo, "agent/remote").unwrap().0;
        let fresh = inspect(&made).unwrap();
        assert_eq!((fresh.remote.as_deref(), fresh.unpushed), (Some("origin"), Some(0)));
        commit(&made, "work");
        assert_eq!(inspect(&made).unwrap().unpushed, Some(1));

        git(&repo, ["remote", "set-head", "origin", "main"]).unwrap();
        let unmerged = inspect(&made).unwrap();
        assert_eq!((unmerged.default_branch.as_deref(), unmerged.merged), (Some("origin/main"), Some(false)));
        git(&made, ["push", "-q", "origin", "HEAD:main"]).unwrap();
        git(&repo, ["fetch", "-q"]).unwrap();
        assert_eq!(inspect(&made).unwrap().merged, Some(true), "merged into origin/main");
        assert_eq!(inspect(&made).unwrap().unpushed, Some(0), "on a remote branch now");
        git(&repo, [OsString::from("worktree"), "remove".into(), made.into()]).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
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

        let made = create(&sub, "agent/one").unwrap().0;
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

    #[test]
    fn porcelain_counts_changed_and_untracked_files() {
        assert_eq!(parse_porcelain(""), (0, 0));
        assert_eq!(parse_porcelain("M a.rs\n M b.rs\nA  c.rs\nR  d -> e\n?? new.txt\n?? dir/\n"), (4, 2));
        assert_eq!(parse_porcelain("UU conflict.rs\n\n"), (1, 0));
        assert_eq!(parse_porcelain("!! .env\n M a.rs\n!! build/\n"), (1, 0), "ignored files are no changes");
        assert_eq!(parse_files("!! .env\n M a.rs"), [('M', "a.rs".to_owned())]);
    }

    #[test]
    fn porcelain_files_keep_their_state_letter_and_path() {
        let files = parse_files("M a.rs\n M b.rs\nA  c.rs\nR  d -> e\n?? 新.txt\n\n");
        let expected = [('M', "a.rs"), ('M', "b.rs"), ('A', "c.rs"), ('R', "d -> e"), ('?', "新.txt")];
        assert_eq!(files, expected.map(|(c, p)| (c, p.to_owned())));
    }

    #[test]
    fn porcelain_paths_lose_the_quotes_git_puts_around_spaces() {
        // What git 2.56 writes for these, with core.quotePath=false.
        let files = parse_files("?? \"my file.txt\"\nR  \"old name.rs\" -> \"new name.rs\"\nR  a.rs -> \"b c.rs\"\n M 金.txt");
        let expected = [('?', "my file.txt"), ('R', "old name.rs -> new name.rs"), ('R', "a.rs -> b c.rs"), ('M', "金.txt")];
        assert_eq!(files, expected.map(|(c, p)| (c, p.to_owned())));
    }

    #[test]
    fn ahead_and_behind_are_read_from_rev_list() {
        assert_eq!(parse_ahead_behind("2\t5"), Some((2, 5)));
        assert_eq!(parse_ahead_behind("0\t0\n"), Some((0, 0)));
        for bad in ["", "3", "a\tb", "1\t2\t3"] {
            assert_eq!(parse_ahead_behind(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_folder_is_in_a_worktree_whatever_its_slashes_and_case() {
        let root = Path::new(r"D:\src\app-fix");
        assert!(contains(root, r"D:\src\app-fix"));
        assert!(contains(root, r"d:/SRC/app-fix/sub/"));
        assert!(!contains(root, r"D:\src\app-fix-2"), "a sibling with the same start");
        assert!(!contains(root, r"D:\src"));
    }

    #[test]
    fn a_folder_is_in_a_worktree_only_below_it() {
        let app = Path::new(r"D:\src\app");
        assert!(!contains(app, r"D:\src\app2"), "a name that starts the same");
        assert!(!contains(app, r"D:\src\ap"));
        assert!(contains(app, r"D:\src\app\deep\er"));
        assert!(contains(Path::new(r"D:\src\app\"), r"D:\src\app"), "a trailing separator on the worktree");
        assert!(contains(Path::new("D:/src/app"), r"d:\src\APP\x"), "git's slashes");
        assert!(contains(Path::new(r"D:\"), r"D:\anything"), "a worktree at the root of a drive");
        assert!(contains(Path::new(r"D:\Ärger"), r"d:\ärger\x"), "case of letters beyond ASCII");
        assert!(!contains(app, ""));
    }

    fn status(main: bool, branch: Option<&str>, changed: usize, untracked: usize) -> Status {
        Status { path: PathBuf::from(r"D:\src\app-x"), branch: branch.map(str::to_owned), main, changed, untracked, ignored: 0, files: Vec::new(), remote: None, unpushed: Some(0), behind: None, default_branch: None, merged: None }
    }

    #[test]
    fn the_main_checkout_is_never_removed_and_changes_only_by_force() {
        assert_eq!(check_remove(&status(true, Some("main"), 0, 0), true), Err(Refusal::Main));
        assert_eq!(check_archive(&status(true, Some("main"), 0, 0)), Err(Refusal::Main));
        assert_eq!(check_remove(&status(false, Some("x"), 1, 2), false), Err(Refusal::Dirty { changed: 1, untracked: 2 }));
        assert_eq!(check_remove(&status(false, Some("x"), 1, 2), true), Ok(()));
        assert_eq!(check_remove(&status(false, None, 0, 0), false), Ok(()));
        assert_eq!(check_archive(&status(false, Some("x"), 0, 1)), Err(Refusal::Dirty { changed: 0, untracked: 1 }), "an archive is never forced");
        assert_eq!(check_archive(&status(false, None, 0, 0)), Err(Refusal::Detached));
        assert_eq!(check_archive(&status(false, Some("x"), 0, 0)), Ok(()));
    }

    /// A fresh repository with one commit on `main` in a temporary folder named `name`; None where git is not on the PATH.
    fn temp_repo(name: &str) -> Option<(PathBuf, PathBuf)> {
        if Command::new("git").arg("--version").creation_flags(CREATE_NO_WINDOW).output().is_err() {
            eprintln!("git is not on the PATH: skipped");
            return None;
        }
        let base = std::env::temp_dir().join(format!("nekominal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, ["init", "-q", "-b", "main"]).unwrap();
        commit(&repo, "start");
        Some((base, repo))
    }

    fn commit(dir: &Path, message: &str) {
        git(dir, ["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", message]).unwrap();
    }

    #[test]
    fn a_remote_is_named_by_its_host_and_path_whatever_its_form() {
        assert_eq!(remote_place("https://github.com/KoukeNeko/Nekominal.git"), (Some("github.com"), "KoukeNeko/Nekominal"));
        assert_eq!(remote_place("https://x-token:ghp_secret@github.com:443/a/b/"), (Some("github.com"), "a/b"), "no user, token or port");
        assert_eq!(remote_place("git@github.com:a/b.git"), (Some("github.com"), "a/b"));
        assert_eq!(remote_place("ssh://git@gitlab.example.com:2222/group/sub/repo.git"), (Some("gitlab.example.com"), "group/sub/repo"));
        assert_eq!(remote_place(r"C:\repos\shared.git"), (None, r"C:\repos\shared"), "a drive is no host");
        assert_eq!(remote_place("D:/repos/x"), (None, "D:/repos/x"));
        assert_eq!(parse_commit("003c739\u{1f}feat: a\u{1f}b c\u{1f}1700000000"), Some(Commit { hash: "003c739".into(), subject: "feat: a".into(), author: "b c".into(), at: 1_700_000_000 }));
        assert_eq!(parse_commit("003c739\u{1f}no time"), None);
    }

    #[test]
    fn the_overview_has_the_last_commit_the_stashes_and_how_far_from_upstream() {
        let Some((base, repo)) = temp_repo("overview") else { return };
        commit(&repo, "second");
        let seen = overview(&repo).unwrap();
        assert_eq!(seen.last.as_ref().map(|c| (c.subject.as_str(), c.author.as_str())), Some(("second", "t")));
        assert_eq!((seen.upstream.as_deref(), seen.status.behind, seen.stashes, seen.url.as_deref()), (None, None, 0, None), "no remote, no upstream, no stash");
        // A clone of it is one commit behind once the original moves on.
        let clone = base.join("clone");
        git(&base, [OsString::from("clone"), "-q".into(), repo.clone().into(), clone.clone().into()]).unwrap();
        commit(&repo, "third");
        git(&clone, ["fetch", "-q"]).unwrap();
        std::fs::write(clone.join("a.txt"), "x").unwrap();
        git(&clone, ["-c", "user.name=t", "-c", "user.email=t@example.com", "stash", "-q", "-u"]).unwrap();
        let seen = overview(&clone).unwrap();
        assert_eq!((seen.upstream.as_deref(), seen.status.behind, seen.stashes), (Some("origin/main"), Some(1), 1));
        assert!(seen.url.is_some(), "the address of origin");
        assert!(overview(&base).unwrap_err().contains("not a git repository"), "git's words when it is no repository");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_dirty_worktree_is_refused_until_clean_then_removed_with_its_merged_branch() {
        let Some((base, repo)) = temp_repo("remove") else { return };
        let main = inspect(&repo).unwrap();
        assert!(main.main && !main.dirty());
        assert_eq!(remove(&main, true, false), Err(Error::Refused(Refusal::Main)));

        let made = create(&repo, "agent/two").unwrap().0;
        let fresh = inspect(&made).unwrap();
        assert!(!fresh.main);
        assert_eq!(fresh.branch.as_deref(), Some("agent/two"));
        assert_eq!(fresh.default_branch.as_deref(), Some("main"));
        assert_eq!((fresh.unpushed, fresh.merged), (Some(1), Some(true)), "no remote: every commit is unpushed");

        std::fs::write(made.join("note.txt"), "x").unwrap();
        let dirty = inspect(&made).unwrap();
        assert_eq!((dirty.changed, dirty.untracked), (0, 1));
        assert_eq!((&dirty.files[..], &dirty.remote), (&[('?', "note.txt".to_owned())][..], &None));
        assert_eq!(remove(&dirty, false, true), Err(Error::Refused(Refusal::Dirty { changed: 0, untracked: 1 })));
        assert!(made.exists());

        std::fs::remove_file(made.join("note.txt")).unwrap();
        commit(&made, "work");
        let ahead = inspect(&made).unwrap();
        assert_eq!((ahead.unpushed, ahead.merged), (Some(2), Some(false)));
        let kept = remove(&ahead, false, true).unwrap().expect("-d keeps a branch not merged");
        // What git said of the branch, in whatever language, not the advice it adds after it.
        assert!(kept.contains("agent/two") && !kept.contains("advice."), "{kept}");
        assert!(!made.exists());
        assert!(git(&repo, ["rev-parse", "--verify", "refs/heads/agent/two"]).is_ok());

        let merged = create(&repo, "agent/three").unwrap().0;
        assert_eq!(remove(&inspect(&merged).unwrap(), false, true), Ok(None));
        assert!(git(&repo, ["rev-parse", "--verify", "refs/heads/agent/three"]).is_err(), "a merged branch goes");
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The shell of a pane that has not exited yet: Windows keeps a folder that is a process's working directory.
    #[test]
    fn a_worktree_a_process_still_sits_in_loses_its_branch_as_asked() {
        let Some((base, repo)) = temp_repo("held") else { return };
        let made = create(&repo, "agent/held").unwrap().0;
        let status = inspect(&made).unwrap();
        // ping, not cmd running it: killing cmd would leave its ping in the folder.
        let mut shell = Command::new("ping.exe").args(["-n", "30", "127.0.0.1"]).stdout(std::process::Stdio::null()).current_dir(&made).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
        let removed = remove(&status, false, true);
        shell.kill().unwrap();
        shell.wait().unwrap();
        assert_eq!(removed, Ok(None), "git removed the worktree and only failed on the folder itself");
        assert!(git(&repo, ["rev-parse", "--verify", "refs/heads/agent/held"]).is_err(), "the branch goes as asked");
        assert!(!git(&repo, ["worktree", "list"]).unwrap().contains("agent/held"));
        assert!(std::fs::read_dir(&made).map_or(true, |mut entries| entries.next().is_none()), "at most an empty folder is left");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn ignored_files_are_counted_apart_from_changes_as_the_removal_deletes_them() {
        let Some((base, repo)) = temp_repo("ignored") else { return };
        std::fs::write(repo.join(".gitignore"), ".env\nbuild/\n").unwrap();
        git(&repo, ["add", ".gitignore"]).unwrap();
        commit(&repo, "ignore");
        let made = create(&repo, "agent/ignored").unwrap().0;
        assert_eq!(inspect(&made).unwrap().ignored, 0);
        std::fs::write(made.join(".env"), "SECRET=1").unwrap();
        std::fs::create_dir_all(made.join("build")).unwrap();
        std::fs::write(made.join("build").join("a.bin"), "x").unwrap();
        std::fs::write(made.join("build").join("b.bin"), "x").unwrap();
        let status = inspect(&made).unwrap();
        assert_eq!((status.changed, status.untracked, status.ignored), (0, 0, 2), "the file and the folder, no change");
        assert!(status.files.is_empty() && !status.dirty());
        assert_eq!(remove(&status, false, true), Ok(None), "git removes them without asking");
        assert!(!made.join(".env").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_archive_keeps_the_branch_and_refuses_changes() {
        let Some((base, repo)) = temp_repo("archive") else { return };
        let made = create(&repo, "agent/four").unwrap().0;
        commit(&made, "work");
        std::fs::write(made.join("note.txt"), "x").unwrap();
        git(&made, ["add", "note.txt"]).unwrap();
        assert_eq!(archive(&inspect(&made).unwrap()), Err(Error::Refused(Refusal::Dirty { changed: 1, untracked: 0 })));
        git(&made, ["rm", "-q", "--cached", "note.txt"]).unwrap();
        std::fs::remove_file(made.join("note.txt")).unwrap();
        archive(&inspect(&made).unwrap()).unwrap();
        assert!(!made.exists());
        assert_eq!(git(&repo, ["log", "-1", "--format=%s", "agent/four"]).unwrap(), "work", "the commit is kept on its branch");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_ignored_files_worktreeinclude_names_are_copied_into_a_new_worktree() {
        let Some((base, repo)) = temp_repo("include") else { return };
        let write = |rel: &str, text: &str| {
            let path = repo.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(".gitignore", ".env*\nsecrets/\nbuild/\n");
        write(".worktreeinclude", "# the local settings\n.env\n.env.local\nsecrets/*.key\nnotes.txt\n");
        git(&repo, ["add", ".gitignore", ".worktreeinclude"]).unwrap();
        commit(&repo, "ignore");
        write(".env", "A=1");
        write(".env.local", "B=2");
        write(".env.other", "not named");
        write("secrets/a.key", "k");
        write("secrets/b.txt", "not a key");
        write("build/out.bin", "ignored, not named");
        write("notes.txt", "named but not ignored");

        let made = create(&repo, "agent/five").unwrap().0;
        let read = |rel: &str| std::fs::read_to_string(made.join(rel)).ok();
        assert_eq!(read(".env").as_deref(), Some("A=1"));
        assert_eq!(read(".env.local").as_deref(), Some("B=2"));
        assert_eq!(read("secrets/a.key").as_deref(), Some("k"));
        for absent in [".env.other", "secrets/b.txt", "build/out.bin", "notes.txt"] {
            assert_eq!(read(absent), None, "{absent}");
        }
        write(".env", "A=2");
        assert_eq!(copy_included(&repo, &made).unwrap(), Vec::<String>::new(), "a file already there is kept");
        assert_eq!(read(".env").as_deref(), Some("A=1"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_missing_or_empty_worktreeinclude_copies_nothing() {
        let Some((base, repo)) = temp_repo("include-empty") else { return };
        std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(repo.join(".env"), "A=1").unwrap();
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        assert_eq!(copy_included(&repo, &dest), Ok(Vec::new()), "no .worktreeinclude");
        for text in ["", "\n\n", "# only a comment\n"] {
            std::fs::write(repo.join(".worktreeinclude"), text).unwrap();
            assert_eq!(copy_included(&repo, &dest), Ok(Vec::new()), "{text:?}");
        }
        assert!(!dest.join(".env").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Thousands of paths are more than a command line holds: they go to git on its standard input.
    #[test]
    fn a_worktreeinclude_naming_thousands_of_files_copies_them_all() {
        let Some((base, repo)) = temp_repo("include-many") else { return };
        std::fs::write(repo.join(".gitignore"), "cache/\n").unwrap();
        std::fs::write(repo.join(".worktreeinclude"), "cache/\n").unwrap();
        for i in 0..1500 {
            let dir = repo.join("cache").join(format!("a fairly long folder name {}", i / 100));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("entry number {i}.bin")), "x").unwrap();
        }
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        assert_eq!(copy_included(&repo, &dest).unwrap().len(), 1500);
        assert!(dest.join("cache").join("a fairly long folder name 14").join("entry number 1499.bin").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn worktreeinclude_negations_nested_folders_and_odd_names_are_followed() {
        let Some((base, repo)) = temp_repo("include-more") else { return };
        let write = |rel: &str| {
            let path = repo.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, rel).unwrap();
        };
        std::fs::write(repo.join(".gitignore"), ".env*\nsecrets/\n*.key\n").unwrap();
        std::fs::write(repo.join(".worktreeinclude"), "**/.env\n.env*\n!.env.example\nsecrets/\n!secrets/old/\n*.key\n").unwrap();
        for rel in [".env", ".env.example", "apps/web/config/.env", "secrets/a.txt", "secrets/deep/b.txt", "secrets/old/c.txt", "my keys/金鑰 1.key"] {
            write(rel);
        }
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let mut copied = copy_included(&repo, &dest).unwrap();
        copied.sort();
        // `!secrets/old/` takes nothing back: as in a .gitignore, a file whose folder is excluded cannot be re-included.
        assert_eq!(copied, [".env", "apps/web/config/.env", "my keys/金鑰 1.key", "secrets/a.txt", "secrets/deep/b.txt", "secrets/old/c.txt"]);
        assert_eq!(std::fs::read_to_string(dest.join("my keys").join("金鑰 1.key")).unwrap(), "my keys/金鑰 1.key");
        assert!(!dest.join(".env.example").exists(), "a negation after the pattern it excepts");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_setup_script_must_be_a_file() {
        let dir = std::env::temp_dir().join(format!("nekominal-setup-odd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".nekominal"), "a file, not a folder").unwrap();
        assert_eq!(setup_command(&dir), None);
        std::fs::remove_file(dir.join(".nekominal")).unwrap();
        std::fs::create_dir_all(dir.join(".nekominal").join("setup.ps1")).unwrap();
        assert_eq!(setup_command(&dir), None, "a folder named setup.ps1");
        std::fs::write(dir.join(".nekominal").join("setup.cmd"), "").unwrap();
        assert_eq!(setup_command(&dir).as_deref(), Some("powershell.exe -NoProfile -Command .nekominal/setup.cmd"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_setup_script_is_found_and_powershell_comes_first() {
        let dir = std::env::temp_dir().join(format!("nekominal-setup-{}", std::process::id()));
        let scripts = dir.join(".nekominal");
        std::fs::create_dir_all(&scripts).unwrap();
        assert_eq!(setup_command(&dir), None);
        std::fs::write(scripts.join("setup.cmd"), "").unwrap();
        assert_eq!(setup_command(&dir).as_deref(), Some("powershell.exe -NoProfile -Command .nekominal/setup.cmd"));
        std::fs::write(scripts.join("setup.ps1"), "").unwrap();
        assert!(setup_command(&dir).unwrap().ends_with("-File .nekominal/setup.ps1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The command line as cmd, PowerShell and Git Bash (where installed) read it, run in a folder with the scripts.
    #[test]
    fn the_setup_command_runs_the_script_in_cmd_powershell_and_git_bash_alike() {
        let dir = std::env::temp_dir().join(format!("nekominal-setup-shells-{}", std::process::id()));
        let scripts = dir.join(".nekominal");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("setup.cmd"), "@echo cmd-setup-ran").unwrap();
        let bash = [("ProgramFiles", r"Git\bin\bash.exe"), ("USERPROFILE", r"scoop\apps\git\current\bin\bash.exe")]
            .into_iter()
            .find_map(|(var, rel)| std::env::var_os(var).map(|p| PathBuf::from(p).join(rel)).filter(|p| p.is_file()));
        for script in ["cmd", "ps1"] {
            if script == "ps1" {
                std::fs::write(scripts.join("setup.ps1"), "Write-Output ps1-setup-ran").unwrap();
            }
            let line = setup_command(&dir).unwrap();
            let mut shells = vec![Command::new("cmd.exe").args(["/d", "/c", &line]).current_dir(&dir).creation_flags(CREATE_NO_WINDOW).output().unwrap()];
            shells.push(Command::new("powershell.exe").args(["-NoProfile", "-Command", &line]).current_dir(&dir).creation_flags(CREATE_NO_WINDOW).output().unwrap());
            if let Some(bash) = &bash {
                shells.push(Command::new(bash).args(["-c", &line]).current_dir(&dir).creation_flags(CREATE_NO_WINDOW).output().unwrap());
            }
            for output in shells {
                assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("{script}-setup-ran")), "{line}: {output:?}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
