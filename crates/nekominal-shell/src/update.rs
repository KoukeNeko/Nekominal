//! Whether a newer Nekominal is out: the version the Scoop bucket offers, read in the background with the `curl.exe`
//! Windows ships, so the app takes no HTTP library for one request. Who has worked on it, as the build took it in. And
//! where a copy that Scoop installed lives.

use std::path::{Path, PathBuf};

/// The manifest of the app in the bucket the release workflow writes.
pub const MANIFEST: &str = "https://raw.githubusercontent.com/KoukeNeko/scoop-bucket/main/bucket/nekominal.json";
/// Where the source is.
pub const REPO: &str = "https://github.com/KoukeNeko/Nekominal";
/// Who has worked on it: the logins of the contributors of the repository, without bots, most commits first. The release
/// workflow writes the file from GitHub before it builds; a build elsewhere takes the copy in the repository.
const CONTRIBUTORS: &str = include_str!("../contributors.json");
/// The version of this build.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// Not asked yet.
    Idle,
    Checking,
    Latest,
    Newer(String),
    Failed,
}

/// Whether `latest` is a later version than `current`: dotted numbers compared one by one, a missing part counting as 0.
/// Something that is not a version is never later.
pub fn newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| v.trim().trim_start_matches('v').split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>();
    let (Ok(mut a), Ok(mut b)) = (parts(latest), parts(current)) else { return false };
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// The version a manifest names.
pub fn version_of(manifest: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(manifest).ok()?;
    Some(value.get("version")?.as_str()?.to_owned())
}

/// The body at `url`, or nothing when it cannot be read. Blocks for up to ten seconds: a thread of its own calls it.
fn fetch(url: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("curl.exe")
        .args(["--silent", "--fail", "--location", "--max-time", "10", url])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads the manifest and says what it means for this build.
pub fn check() -> Check {
    let Some(latest) = fetch(MANIFEST).as_deref().and_then(version_of) else { return Check::Failed };
    if newer(&latest, CURRENT) { Check::Newer(latest) } else { Check::Latest }
}

/// The people who have worked on the source, as the build took them in.
pub fn contributors() -> Vec<String> {
    serde_json::from_str(CONTRIBUTORS).unwrap_or_default()
}

/// The folder Scoop keeps the app in (`...\scoop\apps\nekominal`) when this program is the copy it installed, from
/// `...\apps\nekominal\<version or current>\Nekominal.exe`.
pub fn scoop_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    scoop_dir_of(&exe)
}

fn scoop_dir_of(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?;
    app.parent()?.file_name()?.eq_ignore_ascii_case("apps").then(|| app.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(newer("0.0.13", "0.0.12"));
        assert!(newer("0.0.10", "0.0.9"), "10 is after 9, though '1' sorts before '9'");
        assert!(newer("v0.1", "0.0.99"));
        assert!(!newer("0.0.12", "0.0.12") && !newer("0.0.12.0", "0.0.12"), "the same version is not newer");
        assert!(!newer("0.0.11", "0.0.12"));
        assert!(!newer("latest", "0.0.12") && !newer("", "0.0.12"), "what is not a version is never newer");
    }

    #[test]
    fn the_version_comes_from_the_manifest_and_the_scoop_folder_from_the_path() {
        assert_eq!(version_of(r#"{ "version": "0.0.13", "bin": "Nekominal.exe" }"#).as_deref(), Some("0.0.13"));
        assert_eq!(version_of("<html>404</html>"), None);
        assert_eq!(version_of(r#"{ "version": 13 }"#), None);
        let people = contributors();
        assert!(!people.is_empty() && people.iter().all(|login| !login.is_empty() && !login.contains(char::is_whitespace)), "the embedded list is logins: {people:?}");
        let scoop = Path::new(r"C:\Users\a\scoop\apps\nekominal\current\Nekominal.exe");
        assert_eq!(scoop_dir_of(scoop).as_deref(), Some(Path::new(r"C:\Users\a\scoop\apps\nekominal")));
        assert_eq!(scoop_dir_of(Path::new(r"D:\src\Nekominal\target\release\nekominal-next.exe")), None);
    }
}
