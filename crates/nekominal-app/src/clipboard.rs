//! Clipboard text through the Win32 clipboard (`CF_UNICODETEXT`).

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::util::log;

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
