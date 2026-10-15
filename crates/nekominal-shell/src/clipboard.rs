//! Clipboard text through the Win32 clipboard (`CF_UNICODETEXT`).

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::util::log;

/// Another program may hold the clipboard for a moment (a clipboard manager copying it): a few short retries.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// The text on the clipboard, with line breaks as the source wrote them. `None` when there is no text or
/// the clipboard is held by another program.
pub fn read_text() -> Option<String> {
    // SAFETY: the clipboard is opened and closed around the reads; the global block is locked while its
    // NUL-terminated UTF-16 contents are copied out.
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
        if let Err(e) = OpenClipboard(None) {
            log(&format!("OpenClipboard: {e}"));
            return None;
        }
        let text = (|| {
            let handle: HANDLE = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
            let global = HGLOBAL(handle.0);
            let data = GlobalLock(global).cast::<u16>();
            if data.is_null() {
                return None;
            }
            let mut len = 0;
            while *data.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(data, len));
            let _ = GlobalUnlock(global);
            Some(text)
        })();
        let _ = CloseClipboard();
        text
    }
}

/// `text` as the clipboard wants it: NUL-terminated UTF-16 with every line break written as CR LF (Windows
/// programs that read the clipboard expect that, and a lone LF shows as one line in some of them). A NUL inside
/// the text would end it there, so it is dropped.
fn clipboard_units(text: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(text.len() + 1);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\0' => {}
            '\r' | '\n' => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                units.extend([u16::from(b'\r'), u16::from(b'\n')]);
            }
            other => {
                let mut buffer = [0u16; 2];
                units.extend_from_slice(other.encode_utf16(&mut buffer));
            }
        }
    }
    units.push(0);
    units
}

/// Puts `text` on the clipboard, replacing what is there. False when the clipboard stays held by another
/// program or the system refuses; the text itself is never logged.
pub fn write_text(text: &str) -> bool {
    let units = clipboard_units(text);
    // SAFETY: the clipboard is opened and closed around the write. The global block is ours until
    // `SetClipboardData` succeeds, and the system's after that.
    unsafe {
        let mut opened = OpenClipboard(None);
        for _ in 1..OPEN_ATTEMPTS {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(OPEN_RETRY);
            opened = OpenClipboard(None);
        }
        if let Err(e) = opened {
            log(&format!("OpenClipboard: {e}"));
            return false;
        }
        let stored = (|| -> windows::core::Result<()> {
            EmptyClipboard()?;
            let global = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)?;
            let data = GlobalLock(global).cast::<u16>();
            if data.is_null() {
                let error = windows::core::Error::from_thread();
                let _ = GlobalFree(Some(global));
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len());
            let _ = GlobalUnlock(global);
            if let Err(error) = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(global.0))) {
                let _ = GlobalFree(Some(global));
                return Err(error);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        match stored {
            Ok(()) => true,
            Err(e) => {
                log(&format!("write clipboard: {e}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clipboard_units;

    fn decoded(text: &str) -> String {
        let units = clipboard_units(text);
        assert_eq!(units.last(), Some(&0), "NUL-terminated");
        String::from_utf16(&units[..units.len() - 1]).unwrap()
    }

    #[test]
    fn every_line_break_is_written_as_cr_lf_once() {
        assert_eq!(decoded("a\nb"), "a\r\nb");
        assert_eq!(decoded("a\r\nb"), "a\r\nb");
        assert_eq!(decoded("a\rb"), "a\r\nb");
        assert_eq!(decoded("a\n\nb\n"), "a\r\n\r\nb\r\n");
    }

    #[test]
    fn text_survives_whole_and_a_nul_does_not_cut_it_short() {
        assert_eq!(decoded("你好 😀 e\u{301}"), "你好 😀 e\u{301}");
        assert_eq!(decoded("one\0two"), "onetwo");
        assert_eq!(clipboard_units(""), [0]);
    }
}
