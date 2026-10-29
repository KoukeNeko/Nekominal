//! Paths on a mapped network drive, as the share the drive stands for.
//!
//! A drive letter is a name in the logon session of the user. A process run as administrator has a session of its own
//! and none of the drives the user mapped: `Z:\SOP` is no folder to it, though the share behind it is as open to it as to
//! anyone. So a folder on such a drive is told by its share (`\\server\share\SOP`), which every session sees.

use std::{
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use windows_sys::Win32::{
    Foundation::NO_ERROR,
    NetworkManagement::WNet::WNetGetConnectionW,
    System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
};

/// The drive letter a path starts with (`Z` of `Z:\SOP`), in upper case.
fn letter(path: &Path) -> Option<char> {
    let mut chars = path.to_str()?.chars();
    let (letter, colon) = (chars.next()?, chars.next()?);
    (letter.is_ascii_alphabetic() && colon == ':').then(|| letter.to_ascii_uppercase())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn text_of(buffer: &[u16]) -> Option<String> {
    let len = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    let text = std::ffi::OsString::from_wide(&buffer[..len]).into_string().ok()?;
    (!text.is_empty()).then_some(text)
}

/// The share of the drive as this process sees it: only a process of the session that mapped it does.
fn connected_share(letter: char) -> Option<String> {
    let local = wide(&format!("{letter}:"));
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    // SAFETY: a NUL-terminated name and a buffer as long as `len` says.
    let status = unsafe { WNetGetConnectionW(local.as_ptr(), buffer.as_mut_ptr(), &mut len) };
    if status == NO_ERROR { text_of(&buffer) } else { None }
}

/// The share of the drive as the user's saved connections have it (`HKCU\Network\<letter>\RemotePath`, written for a drive
/// that is reconnected at sign-in): the user's registry is the same for the session of an administrator.
fn saved_share(letter: char) -> Option<String> {
    let key = wide(&format!(r"Network\{letter}"));
    let value = wide("RemotePath");
    let mut buffer = vec![0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: NUL-terminated names and a buffer as large as `bytes` says.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes) };
    if status == NO_ERROR { text_of(&buffer) } else { None }
}

/// `path` with its drive letter replaced by `share`.
fn under_share(share: &str, path: &Path) -> Option<PathBuf> {
    let rest = path.to_str()?.get(2..)?;
    Some(PathBuf::from(format!("{}{rest}", share.trim_end_matches('\\'))))
}

/// The path on the share behind a mapped network drive (`Z:\SOP` → `\\server\share\SOP`); none for a path that is not on
/// a mapped drive (a local drive, a path already on a share, a relative one).
pub fn unc_of(path: &Path) -> Option<PathBuf> {
    let letter = letter(path)?;
    let share = connected_share(letter).or_else(|| saved_share(letter))?;
    under_share(&share, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_on_a_drive_when_it_starts_with_a_letter_and_a_colon() {
        assert_eq!(letter(Path::new(r"z:\SOP")), Some('Z'));
        assert_eq!(letter(Path::new("C:")), Some('C'));
        assert_eq!(letter(Path::new(r"\\server\share\x")), None, "already on a share");
        assert_eq!(letter(Path::new(r"relative\x")), None);
        assert_eq!(letter(Path::new("")), None);
        assert_eq!(letter(Path::new("1:\\x")), None, "a drive is a letter");
    }

    #[test]
    fn the_share_replaces_the_drive_and_keeps_the_rest() {
        assert_eq!(under_share(r"\\tw-1.zentring.net\KMT", Path::new(r"Z:\SOP\StandardOperationProcedure")), Some(PathBuf::from(r"\\tw-1.zentring.net\KMT\SOP\StandardOperationProcedure")));
        assert_eq!(under_share(r"\\server\share\", Path::new(r"Z:\")), Some(PathBuf::from(r"\\server\share\")), "no doubled backslash at the root");
        assert_eq!(under_share(r"\\server\share", Path::new("Z:")), Some(PathBuf::from(r"\\server\share")));
    }

    #[test]
    fn a_local_drive_is_no_network_drive() {
        let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        assert_eq!(unc_of(&windows), None, "the system drive has no share behind it");
        assert_eq!(unc_of(Path::new(r"\\server\share\x")), None);
    }
}
