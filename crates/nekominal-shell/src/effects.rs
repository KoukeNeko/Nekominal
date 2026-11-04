//! Whether the system draws a backdrop (Mica, acrylic) behind the window at all.
//!
//! It does not where `DWMWA_SYSTEMBACKDROP_TYPE` is missing (a Windows Server before 2025, build 20348 for 2022), in a
//! Remote Desktop session (a Server is mostly used through one), and while the user has "Transparency effects" off.
//! Asking DWM for a backdrop there changes nothing and the frame it extends over the client area shows through where
//! the terminal is painted translucent, so the window is made flat instead: see `glass::Material`.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
    Win32::{
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION},
    },
    core::w,
};

/// The first build with `DWMWA_SYSTEMBACKDROP_TYPE`: Windows 11 22H2.
const MIN_BUILD: u32 = 22621;

static ON: AtomicBool = AtomicBool::new(true);

/// Whether the backdrop shows, as of the last [`refresh`].
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Asks the system again: before a window is made, and when the system says its settings or its displays changed (a
/// Remote Desktop session ends or begins with one).
pub fn refresh() {
    // SAFETY: a plain query.
    let remote = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
    ON.store(available(nekominal_pty::conpty::os_build_number(), remote, transparency_setting()), Ordering::Relaxed);
}

fn available(build: u32, remote: bool, transparency: bool) -> bool {
    build >= MIN_BUILD && !remote && transparency
}

/// "Transparency effects" of the Colors page of Settings; on when the value is not there.
fn transparency_setting() -> bool {
    let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
    // SAFETY: the value is as large as `size` says, and lives through the call.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    read.is_err() || value != 0
}

#[cfg(test)]
mod tests {
    use super::available;

    #[test]
    fn the_backdrop_needs_the_api_a_local_session_and_the_transparency_setting() {
        assert!(available(26100, false, true), "Windows 11, Server 2025");
        assert!(available(22621, false, true), "the first build with the API");
        assert!(!available(20348, false, true), "Server 2022");
        assert!(!available(26100, true, true), "Remote Desktop");
        assert!(!available(26100, false, false), "transparency effects off");
        assert!(!available(0, false, true), "the build could not be asked");
    }
}
