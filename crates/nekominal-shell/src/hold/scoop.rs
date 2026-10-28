//! What the updater asks of Scoop and of the processes of the app: which version is installed, the update itself, and
//! the Nekominal that still run from the app's folder (which Scoop would refuse to update past).

use std::{
    io::{BufRead, BufReader},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What Scoop says when a program still runs from the app's folder (`test_running_process` of its `install.ps1`).
const STILL_RUNNING: &str = "are still running";

/// The version Scoop installed in `app` (`...\scoop\apps\nekominal`): what the manifest of `current` names.
pub fn installed(app: &Path) -> Option<String> {
    // Scoop keeps the manifest of an installed version as scoop-manifest.json; manifest.json is what older Scoops wrote.
    let current = app.join("current");
    let text = std::fs::read_to_string(current.join("scoop-manifest.json")).or_else(|_| std::fs::read_to_string(current.join("manifest.json"))).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    Some(manifest.get("version")?.as_str()?.to_owned())
}

/// Whether Scoop refused because something still runs from the app's folder.
pub fn refused_as_running(output: &[String]) -> bool {
    output.iter().any(|line| line.contains(STILL_RUNNING))
}

/// The PowerShell command that runs Scoop with `args`: the Scoop of the root the app is installed in (`<root>\apps\
/// nekominal`), so that an install of its own (a test one, `SCOOP` set elsewhere) updates itself and not the user's;
/// `scoop` from PATH when that root has none.
pub fn command(app: &Path, args: &str) -> String {
    let quote = |path: &Path| crate::update::ps_quote(&path.display().to_string());
    let root = app.parent().and_then(Path::parent);
    let own = root.map(|root| (root, root.join("apps").join("scoop").join("current").join("bin").join("scoop.ps1"))).filter(|(_, script)| script.is_file());
    let scoop = match &own {
        Some((root, script)) => format!("$env:SCOOP = {}; & {}", quote(root), quote(script)),
        None => "scoop".to_owned(),
    };
    // Every stream of Scoop's (it writes with Write-Host) as lines of text, in UTF-8.
    format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; & {{ {scoop} {args} }} *>&1 | ForEach-Object {{ \"$_\" }}")
}

/// Runs Scoop with `args` and hands each line it writes to `line` as it comes; all of them at the end.
pub fn run(app: &Path, args: &str, mut line: impl FnMut(&str)) -> std::io::Result<Vec<String>> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &command(app, args)])
        // Outside the app's folder, which Scoop replaces.
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let mut lines = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        for text in BufReader::new(stdout).split(b'\n').map_while(Result::ok) {
            let text = String::from_utf8_lossy(&text).trim_end().to_owned();
            if !text.trim().is_empty() {
                line(&text);
                lines.push(text);
            }
        }
    }
    child.wait()?;
    Ok(lines)
}

/// Whether `path` is inside the folder `dir` (any case, as Windows compares names).
pub fn is_inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (path.to_string_lossy().to_lowercase(), dir.to_string_lossy().to_lowercase());
    let dir = dir.trim_end_matches('\\');
    path.len() > dir.len() + 1 && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'\\'
}

/// The processes that run a program from inside `dir`, other than this one: their ids.
pub fn running_from(dir: &Path) -> Vec<u32> {
    let me = std::process::id();
    nekominal_pty::conpty::process_names()
        .unwrap_or_default()
        .into_iter()
        .filter(|(pid, _, _)| *pid != me)
        .filter(|(pid, _, _)| nekominal_pty::conpty::image_path(*pid).is_some_and(|image| is_inside(&image, dir)))
        .map(|(pid, _, _)| pid)
        .collect()
}

/// The programs to start the new Nekominal from, in order: the version Scoop now runs, the version that handed over
/// (Scoop keeps the folders of earlier versions), and the program that handed over itself; each once.
pub fn candidates(app: Option<&Path>, from_version: &str, from_exe: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();
    if let Some(app) = app {
        list.push(app.join("current").join("Nekominal.exe"));
        if !from_version.is_empty() {
            list.push(app.join(from_version).join("Nekominal.exe"));
        }
    }
    list.push(from_exe.to_path_buf());
    let mut seen: Vec<String> = Vec::new();
    list.retain(|path| {
        let key = path.to_string_lossy().to_lowercase();
        let new = !seen.contains(&key);
        seen.push(key);
        new
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_version_is_the_one_the_manifest_of_current_names() {
        let app = std::env::temp_dir().join(format!("nekominal-updater-app-{}", std::process::id()));
        std::fs::create_dir_all(app.join("current")).unwrap();
        std::fs::write(app.join("current").join("scoop-manifest.json"), "\u{feff}{ \"version\": \"0.2.4\", \"bin\": \"x\" }").unwrap();
        assert_eq!(installed(&app).as_deref(), Some("0.2.4"));
        std::fs::write(app.join("current").join("scoop-manifest.json"), "not json").unwrap();
        assert_eq!(installed(&app), None);
        std::fs::remove_dir_all(&app).unwrap();
        assert_eq!(installed(&app), None);
    }

    #[test]
    fn scoop_refusing_for_a_running_program_is_told_from_its_output() {
        let said = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert!(refused_as_running(&said(&["Updating 'nekominal' (0.2.3 -> 0.2.4)", "ERROR The following instances of \"nekominal\" are still running. Close them and try again."])));
        assert!(!refused_as_running(&said(&["'nekominal' (0.2.4) was installed successfully!"])));
    }

    #[test]
    fn scoop_is_the_one_of_the_apps_own_root_when_it_has_one() {
        let root = std::env::temp_dir().join(format!("nekominal-updater-root-{}", std::process::id()));
        let app = root.join("apps").join("nekominal");
        assert!(command(&app, "update nekominal").contains("& { scoop update nekominal }"), "no Scoop in the root: the one on PATH");
        let script = root.join(r"apps\scoop\current\bin\scoop.ps1");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "").unwrap();
        let command = command(&app, "update nekominal");
        assert!(command.contains(&format!("$env:SCOOP = '{}'", root.display())), "{command}");
        assert!(command.contains(&format!("& '{}' update nekominal", script.display())), "{command}");
        std::fs::remove_dir_all(&root).unwrap();
        let quoted = super::command(Path::new(r"C:\Users\o'neil\scoop\apps\nekominal"), "update");
        assert!(!quoted.contains("o'neil"), "a quote in a path is doubled: {quoted}");
    }

    #[test]
    fn a_path_is_inside_a_folder_only_below_it_whatever_the_case() {
        let app = Path::new(r"C:\Users\a\scoop\apps\nekominal");
        assert!(is_inside(Path::new(r"c:\users\A\Scoop\apps\nekominal\0.2.3\Nekominal.exe"), app));
        assert!(is_inside(Path::new(r"C:\Users\a\scoop\apps\nekominal\current\OpenConsole.exe"), Path::new(r"C:\Users\a\scoop\apps\nekominal\")));
        assert!(!is_inside(Path::new(r"C:\Users\a\scoop\apps\nekominal-next\x.exe"), app), "a folder whose name starts the same");
        assert!(!is_inside(app, app));
    }

    #[test]
    fn the_new_version_is_tried_first_then_the_one_that_handed_over() {
        let app = Path::new(r"C:\s\apps\nekominal");
        assert_eq!(
            candidates(Some(app), "0.2.3", Path::new(r"C:\s\apps\nekominal\current\Nekominal.exe")),
            [PathBuf::from(r"C:\s\apps\nekominal\current\Nekominal.exe"), PathBuf::from(r"C:\s\apps\nekominal\0.2.3\Nekominal.exe")],
            "the program that handed over ran from current: tried once"
        );
        assert_eq!(candidates(None, "0.2.3", Path::new(r"D:\n\Nekominal.exe")), [PathBuf::from(r"D:\n\Nekominal.exe")]);
    }
}
