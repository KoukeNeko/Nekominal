//! The clipboard of X11 and of Wayland (`arboard`); with a compositor that gives a program no access to it (GNOME), the
//! `wl-copy` and `wl-paste` of wl-clipboard when they are installed.

use std::{
    io::Write,
    process::{Command, Stdio},
    sync::Mutex,
};

pub use crate::clipboard_html::html_fragment;
use crate::util::log;

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Runs `f` on the clipboard, made when it is first needed and kept (on X11 the program that put text there serves it).
fn with_clipboard<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Option<T> {
    let mut guard = CLIPBOARD.lock().ok()?;
    if guard.is_none() {
        match arboard::Clipboard::new() {
            Ok(clipboard) => *guard = Some(clipboard),
            Err(e) => {
                log(&format!("clipboard: {e}"));
                return None;
            }
        }
    }
    match f(guard.as_mut()?) {
        Ok(value) => Some(value),
        Err(arboard::Error::ContentNotAvailable) => None,
        Err(e) => {
            log(&format!("clipboard: {e}"));
            None
        }
    }
}

fn wayland_tool(program: &str) -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() && crate::util::on_path(program)
}

/// The text on the clipboard, with line breaks as the source wrote them. `None` when there is no text.
pub fn read_text() -> Option<String> {
    with_clipboard(|clipboard| clipboard.get_text()).or_else(|| {
        let output = wayland_tool("wl-paste").then(|| Command::new("wl-paste").args(["--no-newline", "--type", "text"]).output().ok()).flatten()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    })
}

/// The files on the clipboard (copied in a file manager). `None` when there are none.
pub fn read_files() -> Option<Vec<std::path::PathBuf>> {
    let files = with_clipboard(|clipboard| clipboard.get().file_list())?;
    (!files.is_empty()).then_some(files)
}

/// The picture on the clipboard as a PNG file's bytes. `None` when it holds no picture.
pub fn read_png() -> Option<Vec<u8>> {
    let image = with_clipboard(|clipboard| clipboard.get_image())?;
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, u32::try_from(image.width).ok()?, u32::try_from(image.height).ok()?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&image.bytes).ok()?;
    Some(png)
}

/// Puts `text` on the clipboard, replacing what is there. False when the system refuses; the text itself is never logged.
pub fn write_text(text: &str) -> bool {
    write(text, None)
}

/// Puts `text` on the clipboard together with `html` (a fragment), for the programs that paste the colours and the style
/// along with the text; the others take the text.
pub fn write_rich(text: &str, html: &str) -> bool {
    write(text, Some(html))
}

fn write(text: &str, html: Option<&str>) -> bool {
    let stored = with_clipboard(|clipboard| match html {
        Some(html) => clipboard.set_html(html, Some(text)),
        None => clipboard.set_text(text),
    });
    if stored.is_some() {
        return true;
    }
    if wayland_tool("wl-copy") {
        let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() else { return false };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        return child.wait().is_ok_and(|status| status.success()) && written;
    }
    false
}
