//! The dialog of Windows that chooses a program file or a folder, for the fields of a profile.

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FILEOPENDIALOGOPTIONS, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH};
use windows::core::{HSTRING, w};

/// A program file the user chose, the dialog starting in the folder of `current`; None when it was closed.
pub fn program(owner: HWND, current: &str) -> Option<String> {
    choose(owner, current, false)
}

/// A folder the user chose, the dialog starting at `current`; None when it was closed.
pub fn folder(owner: HWND, current: &str) -> Option<String> {
    choose(owner, current, true)
}

fn choose(owner: HWND, current: &str, folders: bool) -> Option<String> {
    // SAFETY: COM objects of this thread, released when they go out of scope; the strings live through the calls, and the
    // path the dialog hands back is freed once copied.
    unsafe {
        // The thread may be in an apartment already (UI Automation): then this only counts it again.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let pick = if folders { FOS_PICKFOLDERS } else { FILEOPENDIALOGOPTIONS(0) };
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM | pick).ok()?;
        if !folders {
            let filters = [
                COMDLG_FILTERSPEC { pszName: w!("Programs (*.exe, *.cmd, *.bat)"), pszSpec: w!("*.exe;*.cmd;*.bat;*.com") },
                COMDLG_FILTERSPEC { pszName: w!("All files (*.*)"), pszSpec: w!("*.*") },
            ];
            dialog.SetFileTypes(&filters).ok()?;
        }
        let start = Path::new(current);
        if let Some(dir) = (if folders { Some(start) } else { start.parent() }).filter(|dir| dir.is_dir())
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        dialog.Show(Some(owner)).ok()?;
        let path = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        text
    }
}
