//! Files as pasted text: an image on the clipboard saved as a PNG file, files copied in Explorer, and files dropped on a
//! pane, all typed as their paths. Coding agents (Claude Code, Codex, Gemini CLI) take an image by its path in the prompt,
//! and that works the same over WSL and SSH, where an agent cannot read the clipboard of Windows itself.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::{clipboard, util::log};

/// Pasted images are kept this long: long enough for the agent to read them, then they are cleared at the next start.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// Where a pane's program looks for the files: Windows itself, or a WSL distribution (named when it is known), which
/// sees the drives under `/mnt`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target<'a> {
    Windows,
    Wsl(Option<&'a str>),
}

/// The folder the pasted images go in: `%TEMP%\Nekominal\paste`.
fn paste_dir() -> PathBuf {
    std::env::temp_dir().join("Nekominal").join("paste")
}

/// The paths of the files of a drop handle (a drop, or `CF_HDROP` on the clipboard).
pub fn drop_paths(drop: HDROP) -> Vec<PathBuf> {
    // SAFETY: the handle is valid for the caller; each name is read into a buffer as long as the call says it needs.
    unsafe {
        let count = DragQueryFileW(drop, u32::MAX, None);
        (0..count)
            .filter_map(|n| {
                let len = DragQueryFileW(drop, n, None) as usize;
                let mut name = vec![0u16; len + 1];
                let got = DragQueryFileW(drop, n, Some(&mut name)) as usize;
                (got > 0).then(|| PathBuf::from(String::from_utf16_lossy(&name[..got])))
            })
            .collect()
    }
}

/// What a paste types when the clipboard has no text: the paths of the files copied in Explorer, or the path of the image
/// on it, saved as a PNG file. `None` when there is neither.
pub fn clipboard_text(target: Target) -> Option<String> {
    if let Some(paths) = clipboard::read_files() {
        return Some(path_text(&paths, target));
    }
    let path = save_clipboard_image()?;
    Some(path_text(&[path], target))
}

/// The image on the clipboard as a PNG file in the paste folder. A PNG that a program put there is kept as it is (with its
/// transparency); a bitmap is encoded.
fn save_clipboard_image() -> Option<PathBuf> {
    // SAFETY: registering a format name has no other effect; the same name always gives the same number.
    let png_format = unsafe { RegisterClipboardFormatW(windows::core::w!("PNG")) };
    let (format, bytes) = clipboard::read_bytes(&[png_format, u32::from(CF_DIBV5.0), u32::from(CF_DIB.0)])?;
    let png = if format == png_format && bytes.starts_with(b"\x89PNG") {
        bytes
    } else {
        let Some((width, height, rgba)) = dib_to_rgba(&bytes) else {
            log("paste: a bitmap of a kind that is not read");
            return None;
        };
        encode_png(width, height, &rgba)?
    };
    let dir = paste_dir();
    let path = dir.join(format!("{}.png", timestamp()));
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, png)) {
        Ok(()) => Some(path),
        Err(e) => {
            log(&format!("paste: saving the image: {e}"));
            None
        }
    }
}

/// The local time to the millisecond, for the name of a file: `20261027-153012-123`.
fn timestamp() -> String {
    // SAFETY: plain query.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
}

/// Removes the pasted images older than a day. Run once at start, off the window's thread.
pub fn clean_old() {
    clean_old_in(&paste_dir(), SystemTime::now());
}

fn clean_old_in(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let modified = entry.metadata().and_then(|meta| meta.modified());
        if modified.is_ok_and(|modified| is_stale(modified, now)) && entry.path().extension().is_some_and(|ext| ext == "png") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A file written at `modified` is older than it is kept. A time in the future (a clock set back) is not.
fn is_stale(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified).is_ok_and(|age| age > KEEP)
}

/// The paths as typed text, separated by spaces, each converted for the target and quoted when it needs to be.
pub fn path_text(paths: &[PathBuf], target: Target) -> String {
    paths.iter().map(|path| one_path(path, target)).collect::<Vec<_>>().join(" ")
}

fn one_path(path: &Path, target: Target) -> String {
    let windows = path.to_string_lossy();
    match target {
        // A location WSL cannot reach (a network share) is typed as it is: better than nothing typed.
        Target::Wsl(distro) => match nekominal_pty::profiles::windows_path_to_wsl(path, distro) {
            Some(linux) if linux.chars().any(|c| c.is_whitespace() || "'\"\\$`!&()[]{};<>|*?#~^".contains(c)) => {
                format!("'{}'", linux.replace('\'', r"'\''"))
            }
            Some(linux) => linux,
            None => format!("\"{windows}\""),
        },
        // Double quotes are what cmd, PowerShell and the agents' prompts all read as one path; a Windows path cannot hold one.
        Target::Windows if windows.chars().any(|c| c.is_whitespace() || "&()[]{}^=;!'+,`$@#%~".contains(c)) => format!("\"{windows}\""),
        Target::Windows => windows.into_owned(),
    }
}

/// The pixels of a packed DIB (`CF_DIB`, `CF_DIBV5`: a header, the colour masks or table, then the rows) as RGBA, top row
/// first. 24 and 32 bits a pixel, the kinds screenshots and browsers put on the clipboard; `None` for the others.
fn dib_to_rgba(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |at: usize| dib.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    if header < 40 {
        return None;
    }
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]);
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)? as usize;
    // BI_RGB, BI_BITFIELDS, BI_ALPHABITFIELDS.
    let (bitfields, alpha_field) = match compression {
        0 => (false, false),
        3 => (true, false),
        6 => (true, true),
        _ => return None,
    };
    if width <= 0 || height == 0 || !(bits == 24 || bits == 32) || (bitfields && bits != 32) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w.checked_mul(h)? > 1 << 28 {
        return None;
    }
    // The masks are in a V4 or V5 header; after a plain one they follow it.
    let mut masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000];
    let mut start = header;
    if bitfields {
        let at = if header >= 52 { 40 } else { header };
        for (n, mask) in masks.iter_mut().enumerate().take(if alpha_field || header >= 56 { 4 } else { 3 }) {
            *mask = u32_at(at + 4 * n)?;
        }
        if header < 52 {
            start += if alpha_field { 16 } else { 12 };
        }
        if !alpha_field && header < 56 {
            masks[3] = 0;
        }
    }
    start += colors_used * 4;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let rows = dib.get(start..start.checked_add(stride.checked_mul(h)?)?)?;
    let channel = |pixel: u32, mask: u32| if mask == 0 { 0 } else { ((pixel & mask) >> mask.trailing_zeros()) as u8 };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        // Rows go from the bottom up unless the height is negative.
        let row = if height > 0 { h - 1 - y } else { y };
        let row = &rows[row * stride..][..stride];
        for x in 0..w {
            let pixel = if bits == 32 {
                u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]])
            } else {
                u32::from_le_bytes([row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 0])
            };
            let alpha = if bits == 32 { channel(pixel, masks[3]) } else { 255 };
            rgba.extend([channel(pixel, masks[0]), channel(pixel, masks[1]), channel(pixel, masks[2]), alpha]);
        }
    }
    // Most programs leave the fourth byte of a 32-bit bitmap at zero and mean it opaque: an image that is all transparent
    // was never meant.
    if rgba.chunks_exact(4).all(|px| px[3] == 0) {
        rgba.chunks_exact_mut(4).for_each(|px| px[3] = 255);
    }
    Some((w as u32, h as u32, rgba))
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let written = encoder.write_header().and_then(|mut writer| writer.write_image_data(rgba).and_then(|()| writer.finish()));
    if let Err(e) = written {
        log(&format!("paste: encoding the image: {e}"));
        return None;
    }
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_windows_path_is_quoted_only_when_a_shell_would_split_or_read_it() {
        assert_eq!(path_text(&paths(&[r"C:\shots\a.png"]), Target::Windows), r"C:\shots\a.png");
        assert_eq!(path_text(&paths(&[r"C:\Users\Jo Doe\a.png", r"D:\x&y\b.png"]), Target::Windows), r#""C:\Users\Jo Doe\a.png" "D:\x&y\b.png""#);
        assert_eq!(path_text(&paths(&[r"C:\圖片\截圖.png"]), Target::Windows), r"C:\圖片\截圖.png", "letters of any script need no quotes");
    }

    #[test]
    fn a_wsl_pane_gets_the_path_under_mnt_in_single_quotes_when_needed() {
        assert_eq!(path_text(&paths(&[r"C:\Users\jo\AppData\Local\Temp\Nekominal\paste\1.png"]), Target::Wsl(None)), "/mnt/c/Users/jo/AppData/Local/Temp/Nekominal/paste/1.png");
        assert_eq!(path_text(&paths(&[r"D:\Jo's files\a b.png"]), Target::Wsl(None)), r"'/mnt/d/Jo'\''s files/a b.png'");
        assert_eq!(path_text(&paths(&[r"\\wsl$\Ubuntu\home\jo\a.png"]), Target::Wsl(Some("Ubuntu"))), "/home/jo/a.png");
        assert_eq!(path_text(&paths(&[r"\\server\share\a.png"]), Target::Wsl(None)), r#""\\server\share\a.png""#, "a place WSL cannot reach stays as Windows names it");
    }

    #[test]
    fn every_kind_of_windows_path_is_typed_whole() {
        assert_eq!(path_text(&[], Target::Windows), "", "nothing dropped, nothing typed");
        assert_eq!(path_text(&paths(&[r"\\server\share\a b.png"]), Target::Windows), r#""\\server\share\a b.png""#);
        assert_eq!(path_text(&paths(&[r"\\server\share\a.png"]), Target::Windows), r"\\server\share\a.png");
        let long = format!(r"\\?\C:\{}\a.png", "deep\\".repeat(60));
        assert_eq!(path_text(&[PathBuf::from(&long)], Target::Windows), long, "a long path keeps its prefix");
        assert_eq!(path_text(&paths(&[r"C:\it's\(1).png", r"C:\a;b\x"]), Target::Windows), r#""C:\it's\(1).png" "C:\a;b\x""#);
    }

    #[test]
    fn a_wsl_pane_gets_a_path_its_shell_reads_back_as_it_was() {
        let wsl = |path: &str, distro| path_text(&paths(&[path]), Target::Wsl(distro));
        assert_eq!(wsl(r"C:\圖片\截圖 1.png", None), "'/mnt/c/圖片/截圖 1.png'");
        assert_eq!(wsl(r"C:\a\$(touch x)`id`.png", None), r"'/mnt/c/a/$(touch x)`id`.png'", "nothing in it runs");
        assert_eq!(wsl(r"\\?\D:\x\y.png", None), "/mnt/d/x/y.png");
        assert_eq!(wsl(r"\\wsl.localhost\Ubuntu\home\jo\a b.png", Some("Ubuntu")), "'/home/jo/a b.png'");
        assert_eq!(path_text(&paths(&[r"C:\a.png", r"D:\b c.png"]), Target::Wsl(None)), "/mnt/c/a.png '/mnt/d/b c.png'", "several, each on its own");
    }

    #[test]
    fn a_pasted_image_is_cleared_once_it_is_a_day_old_and_not_before() {
        let now = SystemTime::now();
        assert!(!is_stale(now - Duration::from_secs(60), now));
        assert!(!is_stale(now - Duration::from_secs(23 * 3600), now));
        assert!(is_stale(now - Duration::from_secs(25 * 3600), now));
        assert!(!is_stale(now + Duration::from_secs(3600), now), "a file from the future is kept");
        assert!(!is_stale(now - KEEP, now), "exactly a day is kept");
        assert!(is_stale(now - KEEP - Duration::from_secs(1), now));
    }

    #[test]
    fn the_cleanup_removes_only_old_pngs_and_gets_past_what_it_cannot_remove() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("nekominal-paste-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        clean_old_in(&dir, SystemTime::now());
        std::fs::create_dir_all(dir.join("old.png")).unwrap();
        let now = SystemTime::now();
        let file = |name: &str, age_hours: u64| {
            std::fs::write(dir.join(name), b"x").unwrap();
            std::fs::File::options().write(true).open(dir.join(name)).unwrap().set_modified(now - Duration::from_secs(age_hours * 3600)).unwrap();
        };
        for (name, hours) in [("a-old.png", 48), ("b-locked.png", 48), ("c-new.png", 1), ("d-old.txt", 48), ("e-OLD.PNG", 48), ("f-old.png", 30)] {
            file(name, hours);
        }
        // Held open by a program that shares nothing: it cannot be removed, and the ones after it still are.
        let held = std::fs::File::options().read(true).share_mode(0).open(dir.join("b-locked.png")).unwrap();
        clean_old_in(&dir, now);
        drop(held);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["b-locked.png", "c-new.png", "d-old.txt", "e-OLD.PNG", "old.png"], "only the pasted images of the app's own name go");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The header of a DIB: its size, width, height, bits and compression; the rest zero.
    fn header(size: u32, width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut dib = vec![0u8; size as usize];
        dib[..4].copy_from_slice(&size.to_le_bytes());
        dib[4..8].copy_from_slice(&width.to_le_bytes());
        dib[8..12].copy_from_slice(&height.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&bits.to_le_bytes());
        dib[16..20].copy_from_slice(&compression.to_le_bytes());
        dib
    }

    #[test]
    fn a_bitmap_that_claims_more_than_it_holds_is_refused_before_anything_is_allocated() {
        for (width, height) in [(i32::MAX, i32::MAX), (i32::MAX, i32::MIN), (1 << 15, 1 << 15), (100_000, 100_000), (1, i32::MAX), (10_000, 10_000)] {
            assert_eq!(dib_to_rgba(&header(40, width, height, 32, 0)), None, "{width}x{height}");
        }
        for (width, height, bits) in [(0, 2, 32), (-2, 2, 32), (2, 0, 32), (2, 2, 16), (2, 2, 8), (2, 2, 1)] {
            assert_eq!(dib_to_rgba(&[header(40, width, height, bits, 0), vec![0; 64]].concat()), None, "{width}x{height}x{bits}");
        }
        assert_eq!(dib_to_rgba(&[header(40, 1, 1, 32, 1), vec![0; 4]].concat()), None, "run-length compression is not read");
        assert_eq!(dib_to_rgba(&[header(40, 1, 1, 24, 3), vec![0; 16]].concat()), None, "masks only at 32 bits");
        let mut many_colours = [header(40, 1, 1, 32, 0), vec![0; 4]].concat();
        many_colours[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(dib_to_rgba(&many_colours), None, "a colour table longer than the data");
        let mut huge_header = header(40, 1, 1, 32, 0);
        huge_header[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(dib_to_rgba(&huge_header), None);
        for cut in 0..40 {
            assert_eq!(dib_to_rgba(&header(40, 1, 1, 32, 0)[..cut]), None, "a header cut at {cut}");
        }
    }

    #[test]
    fn a_v5_bitmap_with_masks_keeps_its_alpha_and_a_plain_one_takes_masks_after_the_header() {
        // DIBV5, 32 bits, BI_BITFIELDS: the masks at 40, alpha at 52; one pixel half transparent, stored as ARGB.
        let mut v5 = header(124, 1, 1, 32, 3);
        for (n, mask) in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000].iter().enumerate() {
            v5[40 + 4 * n..44 + 4 * n].copy_from_slice(&mask.to_le_bytes());
        }
        v5.extend(0x80_11_22_33u32.to_le_bytes());
        assert_eq!(dib_to_rgba(&v5), Some((1, 1, vec![0x11, 0x22, 0x33, 0x80])));
        // A plain header with BI_BITFIELDS: three masks after it (here RGB in the other order), no alpha.
        let mut plain = header(40, 1, 1, 32, 3);
        for mask in [0x0000_00FFu32, 0x0000_FF00, 0x00FF_0000] {
            plain.extend(mask.to_le_bytes());
        }
        plain.extend(0x00_11_22_33u32.to_le_bytes());
        assert_eq!(dib_to_rgba(&plain), Some((1, 1, vec![0x33, 0x22, 0x11, 255])));
        // BI_ALPHABITFIELDS after a plain header: four masks.
        let mut alpha = header(40, 1, 1, 32, 6);
        for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
            alpha.extend(mask.to_le_bytes());
        }
        alpha.extend(0x40_00_00_FFu32.to_le_bytes());
        assert_eq!(dib_to_rgba(&alpha), Some((1, 1, vec![0, 0, 255, 0x40])));
    }

    #[test]
    fn a_large_real_bitmap_is_read_and_encoded() {
        let (w, h) = (1920, 1080);
        let dib = [header(40, w, -h, 24, 0), vec![0x7F; (w as usize * 3).div_ceil(4) * 4 * h as usize]].concat();
        let (width, height, rgba) = dib_to_rgba(&dib).unwrap();
        assert_eq!((width, height, rgba.len()), (1920, 1080, 1920 * 1080 * 4));
        assert_eq!(decode(&encode_png(width, height, &rgba).unwrap()).0, 1920);
    }

    /// A 2x2 bitmap with a plain header, 32 bits a pixel, bottom row first, and a fourth byte of zero.
    fn dib_32(rows_bottom_up: [[[u8; 4]; 2]; 2], top_down: bool) -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend(40u32.to_le_bytes());
        dib.extend(2i32.to_le_bytes());
        dib.extend((if top_down { -2i32 } else { 2 }).to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(32u16.to_le_bytes());
        dib.extend([0u8; 24]);
        for row in rows_bottom_up {
            for bgra in row {
                dib.extend(bgra);
            }
        }
        dib
    }

    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(info.buffer_size());
        (info.width, info.height, buffer)
    }

    #[test]
    fn a_bitmap_round_trips_through_png_top_row_first_and_opaque() {
        let (red, green, blue, white) = ([0, 0, 255, 0], [0, 255, 0, 0], [255, 0, 0, 0], [255, 255, 255, 0]);
        let (w, h, rgba) = dib_to_rgba(&dib_32([[blue, white], [red, green]], false)).unwrap();
        let png = encode_png(w, h, &rgba).unwrap();
        assert_eq!(decode(&png), (2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]));
        let (_, _, top_down) = dib_to_rgba(&dib_32([[red, green], [blue, white]], true)).unwrap();
        assert_eq!(top_down, rgba, "a negative height is a bitmap stored from the top");
    }

    #[test]
    fn a_24_bit_bitmap_pads_its_rows_and_its_alpha_is_kept_when_there_is_one() {
        // 1x2 at 24 bits: each row of 3 bytes is padded to 4.
        let mut dib = dib_32([[[0; 4]; 2]; 2], false)[..40].to_vec();
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[14] = 24;
        dib.extend([1, 2, 3, 0, 4, 5, 6, 0]);
        assert_eq!(dib_to_rgba(&dib).unwrap(), (1, 2, vec![6, 5, 4, 255, 3, 2, 1, 255]));
        let half = dib_32([[[0, 0, 0, 128], [0, 0, 0, 255]], [[0, 0, 0, 0], [0, 0, 0, 255]]], true);
        assert_eq!(dib_to_rgba(&half).unwrap().2.iter().skip(3).step_by(4).copied().collect::<Vec<_>>(), [128, 255, 0, 255]);
        assert_eq!(dib_to_rgba(&dib[..44]), None, "a bitmap cut short is not read past its end");
    }

    #[test]
    #[ignore = "puts an image on the clipboard of this desktop"]
    fn an_image_on_the_clipboard_pastes_as_the_path_of_a_png() {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
        let dib = dib_32([[[255, 0, 0, 0]; 2]; 2], false);
        // SAFETY: a fresh block handed to the clipboard, which owns it after the call.
        unsafe {
            OpenClipboard(None).unwrap();
            EmptyClipboard().unwrap();
            let global = GlobalAlloc(GMEM_MOVEABLE, dib.len()).unwrap();
            std::ptr::copy_nonoverlapping(dib.as_ptr(), GlobalLock(global).cast::<u8>(), dib.len());
            let _ = GlobalUnlock(global);
            SetClipboardData(u32::from(CF_DIB.0), Some(HANDLE(global.0))).unwrap();
            CloseClipboard().unwrap();
        }
        let text = clipboard_text(Target::Windows).unwrap();
        let path = PathBuf::from(text.trim_matches('"'));
        assert!(path.starts_with(paste_dir()), "{text}");
        assert_eq!(decode(&std::fs::read(&path).unwrap()).0, 2);
        std::fs::remove_file(path).unwrap();
    }
}
