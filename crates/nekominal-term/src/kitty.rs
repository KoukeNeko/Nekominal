//! The kitty graphics protocol (`APC G <control> ; <payload> ST`): the control keys, and turning a transmission into
//! RGBA pixels. What a command does to the pane (store, place, delete, reply) is `terminal::graphics`.
//!
//! Every input is untrusted: sizes are checked before anything is allocated for them (`MAX_IMAGE_BYTES`), a compressed
//! payload is inflated only up to that size, and a file is read only when it is a regular file of a bounded size. A
//! temporary file (`t=t`) is deleted after it is read only under kitty's own rule: it is in the temporary folder and its
//! name says `tty-graphics-protocol`, so a program cannot have the terminal delete anything else.

use std::io::{Read, Seek, SeekFrom};

use base64::Engine;

use crate::images::{MAX_IMAGE_BYTES, MAX_IMAGE_SIDE};

/// The most base64 one image may take while its chunks arrive: an image at the size limit, as base64, and room for
/// the PNG or zlib framing around it.
pub const MAX_TRANSFER: usize = MAX_IMAGE_BYTES / 3 * 4 + 64 * 1024;

/// One command's control keys. Absent keys are 0 (kitty's defaults: action `t`, medium `d`, format 32, delete `a`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// `a`
    pub action: u8,
    /// `q`: 1 drops the OK replies, 2 every reply.
    pub quiet: u32,
    /// `i`, `I`, `p`
    pub id: u32,
    pub number: u32,
    pub placement: u32,
    /// `f`, `t`, `o`
    pub format: u32,
    pub medium: u8,
    pub compression: u8,
    /// `s`, `v`: pixel size of raw data; `S`, `O`: bytes to read from a file and where.
    pub width: u32,
    pub height: u32,
    pub size: u32,
    pub offset: u32,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `x`, `y`, `w`, `h`: the part of the image shown.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// `X`, `Y`: pixel offset inside the first cell; `c`, `r`: cells to fit the image in.
    pub cell_x: u32,
    pub cell_y: u32,
    pub columns: u32,
    pub rows: u32,
    pub z: i32,
    /// `C=1`: the cursor stays.
    pub no_move: bool,
    /// `U=1`: a Unicode placeholder placement.
    pub placeholder: bool,
    /// `d`
    pub delete: u8,
}

impl Command {
    /// Parses the control keys (`a=T,f=100,i=7`). Unknown keys are skipped; a key with a value that is not one is an
    /// error, as in kitty.
    pub fn parse(control: &[u8]) -> Option<Self> {
        let mut command = Command::default();
        for pair in control.split(|byte| *byte == b',').filter(|pair| !pair.is_empty()) {
            let [key, b'=', value @ ..] = pair else { return None };
            let number = || -> Option<u32> { std::str::from_utf8(value).ok()?.parse().ok() };
            let letter = || -> Option<u8> { if let [one] = value { Some(*one) } else { None } };
            match key {
                b'a' => command.action = letter()?,
                b'q' => command.quiet = number()?,
                b'i' => command.id = number()?,
                b'I' => command.number = number()?,
                b'p' => command.placement = number()?,
                b'f' => command.format = number()?,
                b't' => command.medium = letter()?,
                b'o' => command.compression = letter()?,
                b's' => command.width = number()?,
                b'v' => command.height = number()?,
                b'S' => command.size = number()?,
                b'O' => command.offset = number()?,
                b'm' => command.more = number()? == 1,
                b'x' => command.x = number()?,
                b'y' => command.y = number()?,
                b'w' => command.w = number()?,
                b'h' => command.h = number()?,
                b'X' => command.cell_x = number()?,
                b'Y' => command.cell_y = number()?,
                b'c' => command.columns = number()?,
                b'r' => command.rows = number()?,
                b'z' => command.z = std::str::from_utf8(value).ok()?.parse().ok()?,
                b'C' => command.no_move = number()? == 1,
                b'U' => command.placeholder = number()? == 1,
                b'd' => command.delete = letter()?,
                _ => {}
            }
        }
        if command.action == 0 {
            command.action = b't';
        }
        Some(command)
    }
}

/// Why a command failed: an errno name and a message, as kitty replies them (`ENOENT:no such image`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

pub fn fail<T>(code: &'static str, message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure { code, message: message.into() })
}

/// Pixels a transmission carried.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn base64(data: &[u8]) -> Result<Vec<u8>, Failure> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let joined: Vec<u8> = data.iter().copied().filter(|byte| !matches!(byte, b'\r' | b'\n')).collect();
    engine.decode(&joined).or_else(|_| fail("EINVAL", "bad base64 data"))
}

/// The pixels of a complete transmission: `data` is the base64 payload of all its chunks.
pub fn decode(command: &Command, data: &[u8]) -> Result<Decoded, Failure> {
    let bytes = match command.medium {
        0 | b'd' => base64(data)?,
        b'f' | b't' => {
            let path = String::from_utf8(base64(data)?).or_else(|_| fail("EINVAL", "the file name is not UTF-8"))?;
            read_file(&path, command)?
        }
        b's' => return fail("EINVAL", "shared memory is not supported"),
        _ => return fail("EINVAL", "unknown transmission medium"),
    };
    let bytes = match command.compression {
        0 => bytes,
        b'z' => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&bytes, MAX_IMAGE_BYTES)
            .or_else(|_| fail("EINVAL", "the zlib data is corrupt or too large"))?,
        _ => return fail("EINVAL", "unknown compression"),
    };
    match command.format {
        0 | 32 => raw(command, &bytes, 4),
        24 => raw(command, &bytes, 3),
        100 => png(&bytes),
        _ => fail("EINVAL", "unknown format"),
    }
}

fn check_size(width: u32, height: u32) -> Result<(), Failure> {
    if width == 0 || height == 0 {
        return fail("EINVAL", "the image has no size");
    }
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE || width as usize * height as usize * 4 > MAX_IMAGE_BYTES {
        return fail("EFBIG", format!("the image is larger than {MAX_IMAGE_SIDE}x{MAX_IMAGE_SIDE}"));
    }
    Ok(())
}

fn raw(command: &Command, bytes: &[u8], channels: usize) -> Result<Decoded, Failure> {
    let (width, height) = (command.width, command.height);
    check_size(width, height)?;
    let pixels = width as usize * height as usize;
    if bytes.len() < pixels * channels {
        return fail("ENODATA", "insufficient image data");
    }
    let rgba = if channels == 4 {
        bytes[..pixels * 4].to_vec()
    } else {
        bytes[..pixels * 3].as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
    };
    Ok(Decoded { width, height, rgba })
}

fn png(bytes: &[u8]) -> Result<Decoded, Failure> {
    let bad = |e: png::DecodingError| Failure { code: "EINVAL", message: format!("bad PNG data: {e}") };
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: MAX_IMAGE_BYTES });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(bad)?;
    let (width, height) = (reader.info().width, reader.info().height);
    check_size(width, height)?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(|| Failure { code: "EFBIG", message: "PNG too large".into() })?];
    let info = reader.next_frame(&mut buffer).map_err(bad)?;
    buffer.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buffer.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
        png::ColorType::Indexed => return fail("EINVAL", "unexpanded palette PNG"),
    };
    if rgba.len() != width as usize * height as usize * 4 {
        return fail("EINVAL", "bad PNG data");
    }
    Ok(Decoded { width, height, rgba })
}

/// True for a path the terminal may delete after reading (`t=t`): kitty's rule, a file in the temporary folder whose
/// path says `tty-graphics-protocol`. Both read from the file the path leads to: Windows takes `..` away before it opens
/// one, even after a folder that is not there, so `%TEMP%\tty-graphics-protocol\..\x` is `%TEMP%\x`.
fn deletable(path: &std::path::Path) -> bool {
    let (Ok(file), Ok(temp)) = (path.canonicalize(), std::env::temp_dir().canonicalize()) else { return false };
    file.to_string_lossy().contains("tty-graphics-protocol") && file.starts_with(temp)
}

/// Reads the bytes a `t=f` / `t=t` transmission names. Only a file the protocol itself makes is read: in the temporary
/// folder, with `tty-graphics-protocol` in its path. A program is output, possibly from another machine (ssh): with any
/// path, the answers (no such file, a folder, too large, read) would tell it which of the user's files exist and how big
/// they are, so every other path gets the same refusal before anything is looked at. Never a share either (a `\\server`
/// path would make Windows connect to it and send the user's credentials).
fn read_file(path: &str, command: &Command) -> Result<Vec<u8>, Failure> {
    let path_ref = std::path::Path::new(path);
    if path.starts_with("\\\\") || path.starts_with("//") || !path_ref.is_absolute() || !deletable(path_ref) {
        return fail("EPERM", "only the protocol's own temporary files are read");
    }
    let metadata = std::fs::metadata(path_ref).or_else(|e| fail("ENOENT", format!("cannot read the file: {e}")))?;
    if !metadata.is_file() {
        return fail("EBADF", "not a regular file");
    }
    let available = metadata.len().saturating_sub(u64::from(command.offset));
    let wanted = if command.size > 0 { u64::from(command.size).min(available) } else { available };
    if wanted > MAX_TRANSFER as u64 {
        return fail("EFBIG", "the file is too large");
    }
    let mut file = std::fs::File::open(path_ref).or_else(|e| fail("ENOENT", format!("cannot read the file: {e}")))?;
    file.seek(SeekFrom::Start(u64::from(command.offset))).or_else(|e| fail("EBADF", e.to_string()))?;
    let mut bytes = Vec::with_capacity(wanted as usize);
    // `take` consumes the file, so it is closed before the delete below.
    file.take(wanted).read_to_end(&mut bytes).or_else(|e| fail("EBADF", e.to_string()))?;
    if command.medium == b't' && deletable(path_ref) {
        let _ = std::fs::remove_file(path_ref);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.encode(bytes).into_bytes()
    }

    #[test]
    fn control_keys_parse_with_kittys_defaults() {
        let command = Command::parse(b"a=T,f=24,s=2,v=1,i=7,p=3,z=-5,C=1,q=2,X=4").unwrap();
        assert_eq!((command.action, command.format, command.width, command.height), (b'T', 24, 2, 1));
        assert_eq!((command.id, command.placement, command.z, command.no_move, command.quiet, command.cell_x), (7, 3, -5, true, 2, 4));
        assert_eq!(Command::parse(b"").unwrap().action, b't', "transmit is the default action");
        assert!(Command::parse(b"i=x").is_none());
        assert!(Command::parse(b"a=TT").is_none());
        assert!(Command::parse(b"i").is_none());
        assert_eq!(Command::parse(b"K=1,i=2").unwrap().id, 2, "unknown keys are skipped");
    }

    #[test]
    fn raw_rgb_and_rgba_become_rgba() {
        let rgb = Command::parse(b"f=24,s=2,v=1").unwrap();
        let decoded = decode(&rgb, &b64(&[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!(decoded.rgba, [1, 2, 3, 255, 4, 5, 6, 255]);
        let rgba = Command::parse(b"f=32,s=1,v=1").unwrap();
        assert_eq!(decode(&rgba, &b64(&[9, 8, 7, 6])).unwrap().rgba, [9, 8, 7, 6]);
        assert_eq!(decode(&rgba, &b64(&[9, 8])).err().unwrap().code, "ENODATA");
        let none = Command::parse(b"f=32").unwrap();
        assert_eq!(decode(&none, &b64(&[1, 2, 3, 4])).err().unwrap().code, "EINVAL", "raw data needs s and v");
    }

    #[test]
    fn zlib_and_png_payloads_decode() {
        let pixels = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let zipped = miniz_oxide::deflate::compress_to_vec_zlib(&pixels, 6);
        let command = Command::parse(b"f=32,s=2,v=1,o=z").unwrap();
        assert_eq!(decode(&command, &b64(&zipped)).unwrap().rgba, pixels);

        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let decoded = decode(&Command::parse(b"f=100").unwrap(), &b64(&png_bytes)).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn sizes_past_the_limit_are_refused_before_anything_is_allocated() {
        let command = Command::parse(b"f=32,s=100000,v=100000").unwrap();
        assert_eq!(decode(&command, &b64(&[0; 4])).err().unwrap().code, "EFBIG");
        // A zlib bomb: a few KiB that inflate past the limit.
        let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0; MAX_IMAGE_BYTES + 1], 10);
        let command = Command::parse(b"f=32,s=4096,v=4096,o=z").unwrap();
        assert_eq!(decode(&command, &b64(&bomb)).err().unwrap().code, "EINVAL");
    }

    #[test]
    fn only_the_protocols_own_temp_files_are_read_and_every_other_path_is_refused_alike() {
        let folder = std::env::temp_dir().join(format!("nekominal-kitty-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let other = folder.join("pixels.rgba");
        std::fs::write(&other, [1, 2, 3, 4]).unwrap();
        let mine = folder.join("tty-graphics-protocol-0.rgba");
        std::fs::write(&mine, [1, 2, 3, 4]).unwrap();
        let temp = folder.join("tty-graphics-protocol-1.rgba");
        std::fs::write(&temp, [5, 6, 7, 8]).unwrap();
        let path = |p: &std::path::Path| b64(p.to_str().unwrap().as_bytes());

        let file = Command::parse(b"f=32,s=1,v=1,t=f").unwrap();
        assert_eq!(decode(&file, &path(&mine)).unwrap().rgba, [1, 2, 3, 4]);
        assert!(mine.exists(), "a file transmission leaves the file");
        let as_temp = Command::parse(b"f=32,s=1,v=1,t=t").unwrap();
        assert_eq!(decode(&as_temp, &path(&temp)).unwrap().rgba, [5, 6, 7, 8]);
        assert!(!temp.exists(), "the protocol's own temp file is deleted after the read");

        // A file that exists, one that does not, a folder, a share, a relative path: one answer, nothing to tell apart.
        // And a path that names the marker on its way to another file: Windows goes up the `..` whether the folder is there or not.
        let around = folder.join("tty-graphics-protocol").join("..").join("pixels.rgba");
        let refused = [path(&other), path(&folder.join("missing")), path(&folder), b64(b"\\\\server\\share\\x.png"), b64(b"relative.png"), path(&around)];
        for payload in refused {
            assert_eq!(decode(&file, &payload).err().unwrap().code, "EPERM");
        }
        assert_eq!(decode(&as_temp, &path(&around)).err().unwrap().code, "EPERM");
        assert!(other.exists(), "not deleted by a path that only names the marker");
        let shm = Command::parse(b"t=s,f=32,s=1,v=1").unwrap();
        assert_eq!(decode(&shm, &b64(b"name")).err().unwrap().code, "EINVAL");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn garbage_payloads_fail_without_panicking() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for format in [b"f=100".as_slice(), b"f=32,s=3,v=3,o=z", b"f=24,s=2,v=2"] {
            let command = Command::parse(format).unwrap();
            for length in 0..64 {
                let bytes: Vec<u8> = (0..length * 7)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed as u8
                    })
                    .collect();
                let _ = decode(&command, &b64(&bytes));
                let _ = decode(&command, &bytes);
                let _ = Command::parse(&bytes);
            }
        }
    }
}
