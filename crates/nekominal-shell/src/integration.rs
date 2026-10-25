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

/// The arguments of a task of the jump list: the profile with this id.
pub fn profile_arguments(id: &str) -> String {
    format!("-p \"{}\"", id.replace('"', ""))
}

/// Starts this program elevated (Windows asks the user) with `args`: a profile set to run as administrator cannot run
/// inside a window that is not.
pub fn run_elevated(args: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // SAFETY: the strings live through the call.
    let result = unsafe { ShellExecuteW(None, windows::core::w!("runas"), &HSTRING::from(exe.as_os_str()), &HSTRING::from(args), None, SW_SHOWNORMAL) };
    let code = result.0 as isize;
    if code > 32 { Ok(()) } else { Err(format!("ShellExecute returned {code}")) }
}

/// Fills the tasks of the jump list (the right-click menu of the taskbar button) with the profiles, up to as many as
/// Windows shows: each starts a window with its profile.
pub fn set_jump_list(profiles: &[(String, String, Option<PathBuf>)]) -> Result<(), String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};
    use windows::core::Interface;
    let exe = launch_exe().ok_or("the program's path is not known")?;
    let exe = HSTRING::from(exe.as_os_str());
    // SAFETY: COM objects of this thread, released when they go out of scope; the strings live through the calls.
    unsafe {
        // The thread may be in an apartment already (UI Automation): then this only counts it again.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots).map_err(|e| e.to_string())?;
        let tasks: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        for (id, name, program) in profiles.iter().take(slots.max(1) as usize) {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
            link.SetPath(&exe).map_err(|e| e.to_string())?;
            link.SetArguments(&HSTRING::from(profile_arguments(id))).map_err(|e| e.to_string())?;
            let icon = program.as_ref().map_or_else(|| exe.clone(), |path| HSTRING::from(path.as_os_str()));
            let _ = link.SetIconLocation(&icon, 0);
            let store: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
            store.SetValue(&PKEY_Title, &lpwstr_value(name)?).map_err(|e| e.to_string())?;
            store.Commit().map_err(|e| e.to_string())?;
            tasks.AddObject(&link).map_err(|e| e.to_string())?;
        }
        let array: IObjectArray = tasks.cast().map_err(|e| e.to_string())?;
        list.AddUserTasks(&array).map_err(|e| e.to_string())?;
        list.CommitList().map_err(|e| e.to_string())
    }
}

/// `text` as the value of a string property of the shell (VT_LPWSTR, which the title of a link is): the string in memory of
/// COM, which the value frees when it is dropped.
fn lpwstr_value(text: &str) -> Result<windows::Win32::System::Com::StructuredStorage::PROPVARIANT, String> {
    use windows::Win32::System::Com::CoTaskMemAlloc;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
    use windows::Win32::System::Variant::VT_LPWSTR;
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a block of the size of the string, filled before it is handed to the value, which owns it from then on.
    unsafe {
        let block = CoTaskMemAlloc(units.len() * 2).cast::<u16>();
        if block.is_null() {
            return Err("out of memory".into());
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), block, units.len());
        Ok(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 { vt: VT_LPWSTR, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: PROPVARIANT_0_0_0 { pwszVal: windows::core::PWSTR(block) } }),
            },
        })
    }
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
    fn a_task_of_the_jump_list_names_its_profile_and_its_title_is_a_string_of_the_shell() {
        assert_eq!(profile_arguments("wsl-Ubuntu"), r#"-p "wsl-Ubuntu""#);
        assert_eq!(profile_arguments(r#"a"b"#), r#"-p "ab""#, "a quote cannot end the argument early");
        let args: Vec<String> = ["-p", "wsl-Ubuntu"].map(str::to_owned).to_vec();
        assert!(matches!(&crate::cli::parse(&args)[..], [crate::cli::Action::NewTab(t)] if t.profile.as_deref() == Some("wsl-Ubuntu")));
        let value = lpwstr_value("設定檔").expect("a value");
        assert_eq!(value.vt(), windows::Win32::System::Variant::VT_LPWSTR);
        assert_eq!(value.to_string(), "設定檔");
    }

    #[test]
    fn the_jump_list_takes_the_profiles() {
        // The list of this test program, not of Nekominal: it is written and read by Windows like any other.
        set_jump_list(&[("pwsh".into(), "PowerShell".into(), None)]).expect("the jump list is written");
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
