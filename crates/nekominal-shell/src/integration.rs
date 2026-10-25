//! Nekominal in Windows: "Open in Nekominal" in Explorer's right-click menu of a folder (and of the inside of one), and
//! the program those entries start. Everything is written under the current user, never for the whole machine.

use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW};
use windows::core::{HSTRING, PCWSTR};

/// The menus of a folder and of the empty part of a folder's window.
const MENU_KEYS: [&str; 2] = [r"Software\Classes\Directory\shell\Nekominal", r"Software\Classes\Directory\Background\shell\Nekominal"];

/// The program an entry outside the app starts: Scoop's `current` copy when Scoop installed this one, so that an update
/// does not leave the entry pointing at a folder that is gone; this program otherwise.
pub fn launch_exe() -> Option<PathBuf> {
    match crate::update::scoop_dir() {
        Some(dir) => Some(dir.join("current").join("Nekominal.exe")),
        None => std::env::current_exe().ok(),
    }
}

/// The command of the menu entry: the program with the folder clicked (`%V`) as its starting folder.
pub fn explorer_command(exe: &Path) -> String {
    format!("\"{}\" -d \"%V\"", exe.display())
}

/// Puts "Open in Nekominal" (`label`) in Explorer's menus of a folder, or takes it out.
pub fn set_explorer_menu(on: bool, label: &str) -> Result<(), String> {
    if !on {
        for key in MENU_KEYS {
            // SAFETY: deletes a key of this user's tree; a key that is not there is no error worth telling.
            let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
        }
        return Ok(());
    }
    let exe = launch_exe().ok_or("the program's path is not known")?;
    for key in MENU_KEYS {
        write_string(key, None, label)?;
        write_string(key, Some("Icon"), &exe.display().to_string())?;
        write_string(&format!(r"{key}\command"), None, &explorer_command(&exe))?;
    }
    Ok(())
}

/// Writes the string value `name` (the default value when none) of the key at `path` under the current user, making the
/// key when it is not there.
fn write_string(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let mut key = HKEY::default();
    let units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let name = name.map(HSTRING::from);
    // SAFETY: the key is opened, written and closed here; the strings live through the calls.
    unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None).ok().map_err(|e| format!("{path}: {e}"))?;
        let written = RegSetValueExW(key, name.as_ref().map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr())), None, REG_SZ, Some(&bytes)).ok();
        let _ = RegCloseKey(key);
        written.map_err(|e| format!("{path}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_written_under_the_current_user_reads_back_and_its_key_goes_again() {
        // A key of its own for the test, removed at the end.
        let path = format!(r"Software\Nekominal-test-{}", std::process::id());
        write_string(&format!(r"{path}\command"), None, r#""C:\x y\N.exe" -d "%V""#).expect("written");
        write_string(&path, Some("Icon"), "設定.exe").expect("written");
        let query = std::process::Command::new("reg").args(["query", &format!(r"HKCU\{path}"), "/s"]).output().expect("reg runs");
        let text = String::from_utf8_lossy(&query.stdout);
        // SAFETY: removes the test's own key.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path.as_str())) };
        assert!(text.contains(r#""C:\x y\N.exe" -d "%V""#), "{text}");
        assert!(text.contains("Icon"), "{text}");
        let gone = std::process::Command::new("reg").args(["query", &format!(r"HKCU\{path}")]).output().expect("reg runs");
        assert!(!gone.status.success(), "the key is gone again");
    }

    #[test]
    fn the_menu_entry_starts_the_program_in_the_folder_that_was_clicked() {
        let command = explorer_command(Path::new(r"C:\Users\a b\scoop\apps\nekominal\current\Nekominal.exe"));
        assert_eq!(command, r#""C:\Users\a b\scoop\apps\nekominal\current\Nekominal.exe" -d "%V""#);
        // What the command line makes of it once Explorer puts the folder in.
        let args: Vec<String> = ["-d", r"D:\src\my project"].map(str::to_owned).to_vec();
        assert!(matches!(&crate::cli::parse(&args)[..], [crate::cli::Action::NewTab(target)] if target.directory.as_deref() == Some(r"D:\src\my project")));
    }
}
