//! Notifications of Windows ("toasts") for an app that is not packaged (Scoop installs a plain folder): the app gets an
//! AppUserModelID of its own, registered under the current user with its name and icon, and shows its toasts with it.
//! A click on one posts [`WM_TOAST`] to the window that showed it, which brings forward the window that has the tab now.
//!
//! The process does not take the id as its own (SetCurrentProcessExplicitAppUserModelID): its windows would then stop
//! grouping with a taskbar pin or shortcut of the program, and the jump list would move to another id. The toasts do not
//! need it while the app runs; a click after the app closed does nothing, as no COM activator is registered.

use std::cell::RefCell;

use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{PostMessageW, WM_APP},
    },
    core::{HSTRING, IInspectable},
};

/// The AppUserModelID the toasts are shown with: `Company.Product`, as Windows asks.
pub const AUMID: &str = "KoukeNeko.Nekominal";
/// Posted to a window when a toast it showed is clicked; the wParam is the uid of the tab.
// Far from the WM_APP + n of the other modules, which count up from 1.
pub const WM_TOAST: u32 = WM_APP + 0x50;
/// The tab uid the toast of a newer version is shown with: no tab has it, and a click opens the About page.
pub const UPDATE: u64 = u64::MAX;
/// Toasts kept alive so their click still reaches the handler; the oldest go first.
const KEPT: usize = 16;

thread_local! {
    static SHOWN: RefCell<Vec<ToastNotification>> = const { RefCell::new(Vec::new()) };
}

/// The icon of the toasts, built into the program.
const ICON: &[u8] = include_bytes!("../../../assets/nekominal.ico");

/// The file name of an icon: `nekominal-` and the start of the SHA-256 of its bytes.
fn icon_name(icon: &[u8]) -> String {
    let digest = nekominal_pty::relocate::sha256(icon);
    format!("nekominal-{}.ico", digest[..6].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The folder of the icon: Nekominal under the roaming AppData of the user, asked of Windows and not of `APPDATA`. The id
/// the icon is registered under is the user's, one for every copy of the program: a copy run with a settings folder of its
/// own (a test, a portable copy) pointed it at that folder, and the icon went when the folder did.
fn icon_dir() -> Option<std::path::PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    // SAFETY: the path Windows allocates is read once, then freed.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let dir = path.to_string().ok().map(|p| std::path::PathBuf::from(p).join("Nekominal"));
        CoTaskMemFree(Some(path.0 as *const _));
        dir
    }
}

/// Registers the id under the current user, once a run: Windows reads the name and the icon of the toasts from there.
fn register() -> Result<(), String> {
    static DONE: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    DONE.get_or_init(|| {
        let key = format!(r"Software\Classes\AppUserModelId\{AUMID}");
        crate::integration::write_string(&key, Some("DisplayName"), nekominal_model::strings::APP_NAME)?;
        // The icon must be a file: the one built into the program is written under the user's AppData. It is named by its
        // content, as Windows keeps the picture of a path it has read: a new icon is a new path, and the old ones go.
        if let Some(dir) = icon_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let name = icon_name(ICON);
            let icon = dir.join(&name);
            if icon.exists() || std::fs::write(&icon, ICON).is_ok() {
                crate::integration::write_string(&key, Some("IconUri"), &icon.display().to_string())?;
                for old in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let file = old.file_name().to_string_lossy().into_owned();
                    if file != name && file.starts_with("nekominal") && file.ends_with(".ico") {
                        let _ = std::fs::remove_file(old.path());
                    }
                }
            }
        }
        Ok(())
    })
    .clone()
}

/// The text as the content of an XML element.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The XML of a toast with a title and a body. Each tab has one tag, so its next toast replaces the last in the
/// notification centre instead of piling up.
pub fn xml(title: &str, body: &str) -> String {
    let lines: String = body.lines().take(2).map(|line| format!("<text>{}</text>", escape(line))).collect();
    format!(r#"<toast><visual><binding template="ToastGeneric"><text>{}</text>{lines}</binding></visual></toast>"#, escape(title))
}

/// Shows a toast for the tab `tab` of the window `hwnd`.
pub fn show(hwnd: HWND, tab: u64, title: &str, body: &str) -> Result<(), String> {
    register()?;
    let shown = || -> windows::core::Result<ToastNotification> {
        // SAFETY: the thread may be in an apartment already (UI Automation): then this only counts it again.
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml(title, body)))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        toast.SetTag(&HSTRING::from(tab.to_string()))?;
        toast.SetGroup(&HSTRING::from("tabs"))?;
        let window = hwnd.0 as isize;
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
            // SAFETY: posting to a window; one that is gone makes the call fail, which is nothing to handle.
            unsafe { PostMessageW(Some(HWND(window as *mut _)), WM_TOAST, WPARAM(tab as usize), LPARAM(0)).ok() };
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)?;
        Ok(toast)
    };
    let toast = shown().map_err(|e| e.to_string())?;
    SHOWN.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push(toast);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_file_is_named_by_its_content_so_a_new_icon_is_a_new_path() {
        let name = icon_name(ICON);
        assert!(name.starts_with("nekominal-") && name.ends_with(".ico") && name.len() == "nekominal-".len() + 12 + 4, "{name}");
        assert_eq!(icon_name(ICON), name, "the same bytes, the same name");
        assert_ne!(icon_name(b"another icon"), name);
    }

    #[test]
    fn the_icon_goes_to_a_nekominal_folder_of_the_roaming_appdata_windows_knows() {
        let dir = icon_dir().expect("the roaming AppData of the user");
        assert!(dir.ends_with("Nekominal"), "{}", dir.display());
        assert!(dir.parent().is_some_and(std::path::Path::is_dir), "{}", dir.display());
    }

    #[test]
    fn the_xml_escapes_the_text_and_keeps_two_lines_of_the_body() {
        assert_eq!(
            xml("a <b> & \"c\"", "one\ntwo\nthree"),
            r#"<toast><visual><binding template="ToastGeneric"><text>a &lt;b&gt; &amp; &quot;c&quot;</text><text>one</text><text>two</text></binding></visual></toast>"#
        );
    }

    /// Shows a real toast: run with `cargo test -p nekominal-shell toast -- --ignored` and look at the screen. It also
    /// asks the notification centre whether the toast is there, which is what can be checked without eyes.
    #[test]
    #[ignore = "shows a notification on the desktop and writes the registration under HKCU"]
    fn a_toast_is_shown_and_reaches_the_notification_centre() {
        show(HWND::default(), 424242, "Nekominal test", "Command finished after 12 s\ncargo build").expect("shown");
        let history = ToastNotificationManager::History().unwrap().GetHistoryWithId(&HSTRING::from(AUMID)).unwrap();
        let tags: Vec<String> = history.into_iter().map(|toast| toast.Tag().unwrap().to_string()).collect();
        println!("in the notification centre: {tags:?}");
        assert!(tags.contains(&"424242".to_owned()), "{tags:?}");
    }
}
