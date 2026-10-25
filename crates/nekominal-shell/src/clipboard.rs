//! Clipboard text through the Win32 clipboard (`CF_UNICODETEXT`), and with it, for a copy with formatting, the same text
//! as HTML ("HTML Format").

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::util::log;

/// Another program may hold the clipboard for a moment (a clipboard manager copying it): a few short retries.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// Opens the clipboard, with the retries; false (and logged) when it stays held.
fn open() -> bool {
    // SAFETY: opening the clipboard for this thread; the callers close it.
    unsafe {
        let mut opened = OpenClipboard(None);
        for _ in 1..OPEN_ATTEMPTS {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(OPEN_RETRY);
            opened = OpenClipboard(None);
        }
        if let Err(e) = &opened {
            log(&format!("OpenClipboard: {e}"));
        }
        opened.is_ok()
    }
}

/// The text on the clipboard, with line breaks as the source wrote them. `None` when there is no text or
/// the clipboard is held by another program.
pub fn read_text() -> Option<String> {
    // SAFETY: the clipboard is opened and closed around the reads; the global block is locked while its
    // NUL-terminated UTF-16 contents are copied out.
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
        if !open() {
            return None;
        }
        let text = (|| {
            let handle: HANDLE = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
            let global = HGLOBAL(handle.0);
            let data = GlobalLock(global).cast::<u16>();
            if data.is_null() {
                return None;
            }
            // Up to the NUL, but never past the block: another program may have put text there without one.
            let units = GlobalSize(global) / 2;
            let len = (0..units).take_while(|&i| *data.add(i) != 0).count();
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
    write(text, None)
}

/// Puts `text` on the clipboard together with `html` (a fragment, see `cf_html`), for the programs that paste the
/// colours and the style along with the text (Word, Outlook, OneNote); the others take the text.
pub fn write_rich(text: &str, html: &str) -> bool {
    write(text, Some(html))
}

/// The "HTML Format" of the Windows clipboard around `fragment`: a header that says where the HTML and the fragment start
/// and end, in bytes of UTF-8 from the start of the header, then the document.
pub fn cf_html(fragment: &str) -> String {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let before = "<html><body>\r\n<!--StartFragment-->";
    let after = "<!--EndFragment-->\r\n</body></html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + before.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + after.len();
    format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{before}{fragment}{after}"
    )
}

fn write(text: &str, html: Option<&str>) -> bool {
    let units = clipboard_units(text);
    // SAFETY: the clipboard is opened and closed around the write. The global block is ours until
    // `SetClipboardData` succeeds, and the system's after that.
    unsafe {
        if !open() {
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
            // The formatted copy rides along; when it cannot be stored, the text still is.
            if let Some(html) = html {
                let format = RegisterClipboardFormatW(windows::core::w!("HTML Format"));
                let mut bytes = cf_html(html).into_bytes();
                bytes.push(0);
                if format != 0
                    && let Ok(block) = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
                {
                    let data = GlobalLock(block).cast::<u8>();
                    if data.is_null() {
                        let _ = GlobalFree(Some(block));
                    } else {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
                        let _ = GlobalUnlock(block);
                        if SetClipboardData(format, Some(HANDLE(block.0))).is_err() {
                            let _ = GlobalFree(Some(block));
                        }
                    }
                }
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

/// The HTML of a copy with formatting: the lines of runs of the selection in a `<pre>` in the terminal's font and
/// colours, each run in its own colours and style. `resolve` turns the symbolic colours into RGB (foreground, background).
pub fn html_fragment(lines: &[Vec<nekominal_term::StyledRun>], font: &str, resolve: impl Fn(&nekominal_term::StyledRun) -> ((u8, u8, u8), (u8, u8, u8)), default: ((u8, u8, u8), (u8, u8, u8))) -> String {
    let hex = |(r, g, b): (u8, u8, u8)| format!("#{r:02x}{g:02x}{b:02x}");
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut html = format!(
        "<pre style=\"font-family:'{}',Consolas,monospace;color:{};background-color:{};padding:8px;white-space:pre\">",
        font.replace(['\'', '"', '<', '>'], ""),
        hex(default.0),
        hex(default.1)
    );
    for (n, runs) in lines.iter().enumerate() {
        if n > 0 {
            html.push('\n');
        }
        for run in runs {
            let (fg, bg) = resolve(run);
            let mut style = String::new();
            if fg != default.0 {
                style += &format!("color:{};", hex(fg));
            }
            if bg != default.1 {
                style += &format!("background-color:{};", hex(bg));
            }
            for (on, css) in [(run.bold, "font-weight:bold;"), (run.italic, "font-style:italic;"), (run.underline, "text-decoration:underline;")] {
                if on {
                    style += css;
                }
            }
            if style.is_empty() {
                html += &escape(&run.text);
            } else {
                html += &format!("<span style=\"{style}\">{}</span>", escape(&run.text));
            }
        }
    }
    html += "</pre>";
    html
}

#[cfg(test)]
mod tests {
    use super::{cf_html, clipboard_units, html_fragment};
    use nekominal_term::{CellColor, StyledRun};

    #[test]
    fn the_html_format_says_where_its_parts_are_in_bytes() {
        let fragment = "<b>設定</b>";
        let all = cf_html(fragment);
        let field = |name: &str| -> usize { all.split(&format!("{name}:")).nth(1).unwrap()[..10].parse().unwrap() };
        let (start, end) = (field("StartFragment"), field("EndFragment"));
        assert_eq!(&all.as_bytes()[start..end], fragment.as_bytes(), "the fragment is where the header says, counted in bytes of UTF-8");
        assert!(all[field("StartHTML")..].starts_with("<html>"));
        assert_eq!(field("EndHTML"), all.len());
    }

    #[test]
    fn a_formatted_copy_keeps_the_colours_and_style_of_each_run_and_escapes_the_text() {
        let run = |text: &str, fg, bold| StyledRun { text: text.into(), fg, bg: CellColor::Default, bold, italic: false, underline: false };
        let lines = vec![vec![run("ok ", CellColor::Default, false), run("<error>", CellColor::Indexed(1), true)], vec![]];
        let html = html_fragment(&lines, "Cascadia Mono", |r| (if r.fg == CellColor::Indexed(1) { (255, 0, 0) } else { (200, 200, 200) }, (20, 20, 20)), ((200, 200, 200), (20, 20, 20)));
        assert!(html.starts_with("<pre style=\"font-family:'Cascadia Mono',Consolas,monospace;color:#c8c8c8;background-color:#141414;"));
        assert!(html.contains("ok <span style=\"color:#ff0000;font-weight:bold;\">&lt;error&gt;</span>\n</pre>"), "{html}");
    }

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
