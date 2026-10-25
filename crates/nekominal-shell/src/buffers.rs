//! The files of the panes' text between runs, one `.vt` file per pane in `%LOCALAPPDATA%\Nekominal\buffers`: written
//! when the session is saved with restoring on, taken (read, then deleted) when it is restored. They hold whatever the
//! panes showed, secrets included, so they stay in the user's local profile (not the roaming one, where the settings
//! are), get no `.bak`, are capped in size, and none outlives the next save: every save deletes the files it did not
//! write, all of them when restoring is off.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use nekominal_model::session;
use nekominal_term::{TerminalCore, restore::SAVED_BYTES};
use windows::{
    Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    },
    core::PCWSTR,
};

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join(nekominal_model::store::APP_DIR).join("buffers"))
}

/// Milliseconds since 1970: what the names of one save start with, so they never meet those of the save before.
pub fn stamp() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis())
}

/// Writes the text of `term` as the next file of the save `stamp`, whose names so far are `kept`. The file's name, or
/// None when the pane holds nothing or the file could not be written.
pub fn save(dir: &Path, stamp: u128, kept: &mut Vec<String>, term: &mut TerminalCore) -> Option<String> {
    let text = term.saved_text();
    if text.is_empty() {
        return None;
    }
    let name = format!("{stamp}-{}.vt", kept.len() + 1);
    match fs::create_dir_all(dir).and_then(|()| fs::write(dir.join(&name), text)) {
        Ok(()) => {
            kept.push(name.clone());
            Some(name)
        }
        Err(e) => {
            crate::util::log(&format!("buffers: {name}: {e}"));
            None
        }
    }
}

/// Deletes every file in `dir` but those named in `keep`.
pub fn prune(dir: &Path, keep: &[String]) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if entry.file_type().is_ok_and(|kind| kind.is_file()) && !keep.iter().any(|kept| *name == **kept) {
            if let Err(e) = fs::remove_file(entry.path()) {
                crate::util::log(&format!("buffers: {}: {e}", name.to_string_lossy()));
            }
        }
    }
}

/// The text of the file `name` (at most `SAVED_BYTES` of it) and when it was written; the file is deleted, since the
/// text now lives in its pane and the next save writes it again. None for a name that is not one of these files.
pub fn take(name: &str) -> Option<(String, SystemTime)> {
    if !session::is_buffer_name(name) {
        return None;
    }
    let path = dir()?.join(name);
    let read = (|| -> io::Result<(Vec<u8>, SystemTime)> {
        let file = File::open(&path)?;
        let written = file.metadata()?.modified()?;
        let mut bytes = Vec::new();
        file.take(SAVED_BYTES as u64).read_to_end(&mut bytes)?;
        Ok((bytes, written))
    })();
    let _ = fs::remove_file(&path);
    // A cut in the middle of a character costs that character, not the file.
    read.ok().map(|(bytes, written)| (String::from_utf8_lossy(&bytes).into_owned(), written))
}

/// `time` as the user's short date and time, in local time; empty when Windows cannot say.
pub fn local_time(time: SystemTime) -> String {
    let ticks = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() / 100) as u64 + 116_444_736_000_000_000;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    // SAFETY: plain conversions between structures that live through the calls.
    if unsafe { FileTimeToSystemTime(&file_time, &mut utc).and_then(|()| SystemTimeToTzSpecificLocalTime(None, &utc, &mut local)) }.is_err() {
        return String::new();
    }
    let (mut date, mut clock) = ([0u16; 64], [0u16; 64]);
    // SAFETY: the buffers are as long as the calls are told; null locale names mean the user's own.
    let (date_len, clock_len) = unsafe {
        (
            GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&local), PCWSTR::null(), Some(&mut date), PCWSTR::null()),
            GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&local), PCWSTR::null(), Some(&mut clock)),
        )
    };
    // The lengths count the terminating zero.
    let text = |buffer: &[u16], len: i32| String::from_utf16_lossy(&buffer[..(len.max(1) as usize - 1).min(buffer.len())]);
    format!("{} {}", text(&date, date_len), text(&clock, clock_len)).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_writes_one_file_per_pane_that_holds_text_and_pruning_deletes_the_rest() {
        let dir = std::env::temp_dir().join(format!("nekominal-buffers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1-1.vt"), "an older save").unwrap();

        let mut kept = Vec::new();
        let mut empty = TerminalCore::new(20, 4, Default::default());
        assert_eq!(save(&dir, 7, &mut kept, &mut empty), None, "an empty pane writes no file");
        let mut busy = TerminalCore::new(20, 4, Default::default());
        busy.feed(b"\x1b[31mhello\x1b[0m");
        assert_eq!(save(&dir, 7, &mut kept, &mut busy).as_deref(), Some("7-1.vt"));
        assert!(session::is_buffer_name(&kept[0]), "the name is one the session reads back");
        assert_eq!(fs::read_to_string(dir.join("7-1.vt")).unwrap(), "\x1b[0;31mhello\x1b[0m\r\n");

        prune(&dir, &kept);
        let names: Vec<String> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["7-1.vt"]);
        prune(&dir, &[]);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "restoring off keeps nothing");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_that_is_not_a_buffer_is_never_opened() {
        assert_eq!(take("..\\session.json"), None);
        assert_eq!(take("C:\\Windows\\win.ini"), None);
    }

    #[test]
    fn the_time_of_a_save_is_shown_in_local_time() {
        let text = local_time(UNIX_EPOCH + std::time::Duration::from_secs(1_761_400_000));
        assert!(text.chars().any(|c| c.is_ascii_digit()), "{text:?}");
    }
}
