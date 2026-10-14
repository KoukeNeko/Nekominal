//! Paste and focus.

use crate::{key::is_control, modes::InputModes};

pub const PASTE_START: &[u8] = b"\x1b[200~";
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// Prepares pasted text for the PTY.
///
/// * C0 controls except TAB, CR and LF, DEL, and C1 controls are removed. Removing ESC is what
///   neutralizes an embedded `ESC [ 201 ~` (it degrades to the harmless text `[201~`) and an
///   embedded 8-bit CSI (U+009B).
/// * Line breaks (`CRLF`, `LF`) become CR, what Enter sends.
/// * With `bracketed`, the result is framed by `ESC [ 200 ~` and `ESC [ 201 ~`.
///
/// Whether to write in one piece or in chunks is up to the writer thread; the bytes are valid
/// UTF-8 and may be split at character boundaries.
pub fn prepare_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let mut out = String::with_capacity(text.len());
    let mut after_cr = false;
    for ch in text.chars() {
        match ch {
            '\r' => {
                out.push('\r');
                after_cr = true;
                continue;
            }
            '\n' => {
                if !after_cr {
                    out.push('\r');
                }
            }
            '\t' => out.push('\t'),
            c if is_control(c) => {}
            c => out.push(c),
        }
        after_cr = false;
    }

    let body = out.into_bytes();
    if !bracketed {
        return body;
    }
    let mut framed = Vec::with_capacity(body.len() + PASTE_START.len() + PASTE_END.len());
    framed.extend_from_slice(PASTE_START);
    framed.extend_from_slice(&body);
    framed.extend_from_slice(PASTE_END);
    framed
}

/// `CSI I` / `CSI O`, only while the application asked for focus reports (DECSET 1004).
///
/// Under ConPTY the emulator always sees `?1004h` (ConPTY requests it itself at startup) and
/// ConPTY consumes the reports unless the client enabled the mode, so sending is harmless.
pub fn encode_focus(focused: bool, modes: &InputModes) -> Option<&'static [u8]> {
    if !modes.focus_reporting {
        return None;
    }
    Some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}
