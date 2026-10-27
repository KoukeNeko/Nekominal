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

/// The PowerShell script that updates the app from its own console window, once process `pid` (this one) has closed:
/// it waits for the process and for the OpenConsole it ran the shells on (Scoop does not replace a file that runs),
/// brings Scoop and its buckets up to date (without that, the bucket on this computer may not know the new version
/// yet), updates the app, and starts it again. When the version did not change it says so and waits for a key, so that
/// what Scoop said stays on the screen.
pub fn update_script(pid: u32, app_dir: &Path) -> String {
    let quoted = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "''"));
    let dir = quoted(app_dir);
    let current = quoted(&app_dir.join("current"));
    format!(
        r#"$Host.UI.RawUI.WindowTitle = 'Nekominal update'
# Out of the folder Scoop is to replace: a console whose working folder is in it holds it, and Scoop cannot unlink it.
Set-Location -LiteralPath $env:USERPROFILE
$dir = {dir}
# The folder Scoop runs the app from: `current`, unless Scoop is set to keep no junction, then the version's own.
function Prefix {{ $p = scoop prefix nekominal 2>$null 6>$null | Select-Object -Last 1; if ($p -and (Test-Path -LiteralPath $p)) {{ $p }} else {{ {current} }} }}
function Installed {{ try {{ (Get-Content -LiteralPath (Join-Path (Prefix) 'manifest.json') -Raw -ErrorAction Stop | ConvertFrom-Json).version }} catch {{ '' }} }}
$before = Installed
Write-Host "Waiting for Nekominal $before to close..."
Wait-Process -Id {pid} -Timeout 120 -ErrorAction SilentlyContinue
# Closing can be called off (a pane is busy and the user says no): then this update is called off too, rather than
# running whenever Nekominal next closes.
if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{
    Write-Host 'Nekominal is still open: the update is called off.'
    Start-Sleep -Seconds 3
    exit
}}
# Every program that runs from the app's folder holds it: the OpenConsole of the shells, and Nekominal itself as an
# agent's status line or a cli call. They are waited for, a while; what still runs then is named.
$held = {{ Get-Process -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne $PID -and $_.Path -and $_.Path.StartsWith("$dir\", [StringComparison]::OrdinalIgnoreCase) }} }}
& $held | Wait-Process -Timeout 60 -ErrorAction SilentlyContinue
& $held | ForEach-Object {{ Write-Host "Still running from the Nekominal folder: $($_.Name) ($($_.Id)). Close it if the update fails." }}
scoop update
scoop update nekominal
$after = Installed
if ($after -eq $before) {{
    Write-Host ''
    Write-Host "Nekominal is still $before."
    Read-Host 'Press Enter to start it again'
}}
Start-Process -FilePath (Join-Path (Prefix) 'Nekominal.exe')
"#
    )
}

/// `script` as `powershell.exe -EncodedCommand` takes it: UTF-16LE in base64, which no path can break the quoting of.
pub fn encoded(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
    fn the_update_script_refreshes_the_buckets_before_the_app_and_quotes_its_paths() {
        let script = update_script(4242, Path::new(r"C:\Users\o'neil\scoop\apps\nekominal"));
        // By line, so that the line ends of a checkout (CRLF on Windows) do not matter.
        let lines: Vec<&str> = script.lines().map(str::trim).collect();
        let bucket = lines.iter().position(|line| *line == "scoop update").expect("the buckets are brought up to date");
        let app = lines.iter().position(|line| *line == "scoop update nekominal").expect("the app is updated");
        assert!(bucket < app, "the buckets first: without them the new version is unknown");
        assert!(script.contains("Wait-Process -Id 4242 -Timeout"), "a close that is called off calls the update off");
        assert!(!script.contains("-like"), "a [ in the folder is no wildcard");
        assert!(script.contains(r"'C:\Users\o''neil\scoop\apps\nekominal\current'"), "a quote in a path is doubled");
        assert!(script.contains("scoop prefix nekominal"), "the folder Scoop runs the app from, junction or not");
        let leave = lines.iter().position(|line| line.starts_with("Set-Location -LiteralPath $env:USERPROFILE")).expect("the script leaves the app's folder");
        assert!(leave < bucket, "before Scoop replaces it");
        assert!(!script.contains("Get-Process OpenConsole"), "every program from the folder is waited for, not the console only");
        // The base64 of UTF-16LE: "a" is 61 00.
        assert_eq!(encoded("a"), "YQA=");
    }

    #[test]
    fn windows_powershell_reads_the_encoded_update_script() {
        // Decoded and compiled by the PowerShell the app starts, as it would be, but not run: a script that does not
        // parse would close the app and update nothing.
        let script = encoded(&update_script(1, Path::new(r"C:\Users\a b\scoop\apps\nekominal")));
        let check = format!("$s = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{script}')); [void][scriptblock]::Create($s)");
        let status = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &check]).status().expect("powershell.exe runs");
        assert!(status.success(), "the update script does not parse");
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
