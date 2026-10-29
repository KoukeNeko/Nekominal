//! The icons of programs, taken from their files: what a tab, the list of profiles and the settings show for a shell or
//! a program, as Explorer would show it.

use std::path::Path;

use windows::{
    Win32::{
        Foundation::CloseHandle,
        Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, HBITMAP},
        Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, OPEN_EXISTING},
        System::{IO::DeviceIoControl, Ioctl::FSCTL_GET_REPARSE_POINT},
        UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW},
    },
    core::HSTRING,
};

/// A number that names the icon of `path` for the cache of the pictures: the same path, whatever its case, is one icon.
pub fn key(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    hasher.finish()
}

/// The first icon of the program at `path`, `px` pixels square, as premultiplied RGBA; `None` when the file has none.
/// An app execution alias (the `pwsh.exe` of the Store version in `WindowsApps`) is an empty file: the icon is that of the
/// program it stands for.
pub fn load(path: &Path, px: u32) -> Option<Vec<u8>> {
    let path = &plain(path);
    from_file(path, px).or_else(|| from_file(&alias_target(path)?, px)).or_else(|| from_file(&sibling(path)?, px))
}

/// What a tab shows for a program whose file has no icon of its own, in this order: the icon of the program installed
/// beside it that people know it by, its mark, its glyph.
struct Program {
    /// Its file names without the extension, in lower case.
    names: &'static [&'static str],
    /// The program beside it, looked for in up to this many folders above it (`Git\usr\bin\bash.exe` → `Git\git-bash.exe`).
    sibling: Option<(&'static str, usize)>,
    mark: Option<Mark>,
    /// Of Segoe Fluent Icons.
    glyph: char,
}

/// A mark from Simple Icons, kept unchanged in `assets/brands` and named in THIRD-PARTY-NOTICES.md with its licence.
struct Mark {
    file: &'static str,
    svg: &'static str,
    /// The brand's colour; none for a dark one, drawn in the colour of the text, which a dark tab bar would hide.
    rgb: Option<[u8; 3]>,
    license: License,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum License {
    Cc0,
    CcBy3,
    CcBy4,
}

macro_rules! mark {
    ($file:literal, $rgb:expr, $license:ident) => {
        Some(Mark { file: $file, svg: include_str!(concat!("../../../assets/brands/", $file)), rgb: $rgb, license: License::$license })
    };
}

const EDITOR: char = '\u{E70F}';
const NETWORK: char = '\u{E774}';
const PAGER: char = '\u{E8A5}';
const GIT: char = '\u{E81C}';
const FILES: char = '\u{E838}';
const CODE: char = '\u{E943}';
/// The terminal's, for a shell and a program nobody knows.
const TERMINAL: char = '\u{E756}';

#[rustfmt::skip]
const PROGRAMS: &[Program] = &[
    Program { names: &["git"], sibling: Some(("git-bash.exe", 3)), mark: mark!("git.svg", Some([0xF0, 0x3C, 0x2E]), CcBy3), glyph: GIT },
    Program { names: &["bash"], sibling: Some(("git-bash.exe", 3)), mark: None, glyph: TERMINAL },
    Program { names: &["gh"], sibling: None, mark: mark!("github.svg", None, Cc0), glyph: GIT },
    Program { names: &["lazygit", "tig", "gitui"], sibling: None, mark: None, glyph: GIT },
    Program { names: &["npm", "npx"], sibling: Some(("node.exe", 1)), mark: mark!("npm.svg", Some([0xCB, 0x38, 0x37]), Cc0), glyph: CODE },
    Program { names: &["pnpm", "pnpx"], sibling: Some(("node.exe", 1)), mark: mark!("pnpm.svg", Some([0xF6, 0x92, 0x20]), Cc0), glyph: CODE },
    Program { names: &["yarn"], sibling: Some(("node.exe", 1)), mark: mark!("yarn.svg", Some([0x2C, 0x8E, 0xBB]), CcBy4), glyph: CODE },
    Program { names: &["corepack"], sibling: Some(("node.exe", 1)), mark: None, glyph: CODE },
    Program { names: &["uv", "uvx"], sibling: None, mark: mark!("uv.svg", Some([0xDE, 0x5F, 0xE9]), Cc0), glyph: CODE },
    Program { names: &["esbuild"], sibling: None, mark: mark!("esbuild.svg", Some([0xFF, 0xCF, 0x00]), Cc0), glyph: TERMINAL },
    Program { names: &["vite"], sibling: None, mark: mark!("vite.svg", Some([0x91, 0x35, 0xFF]), Cc0), glyph: TERMINAL },
    Program { names: &["curl"], sibling: None, mark: mark!("curl.svg", None, Cc0), glyph: NETWORK },
    Program { names: &["7z", "7za", "7zr"], sibling: Some(("7zFM.exe", 1)), mark: mark!("7zip.svg", None, Cc0), glyph: TERMINAL },
    // The Rust and Neovim marks are share-alike: cargo and nvim keep their glyph.
    Program { names: &["node", "bun", "deno", "pip", "py", "python", "cargo", "rustc", "go", "dotnet", "java", "ruby", "php", "make", "cmake",
                       "claude", "codex", "gemini", "aider", "opencode"], sibling: None, mark: None, glyph: CODE },
    Program { names: &["vim", "vi", "nvim", "gvim", "nvim-qt", "nano", "micro", "hx", "helix", "kak", "emacs", "code", "notepad"], sibling: None, mark: None, glyph: EDITOR },
    Program { names: &["ssh", "mosh", "telnet", "scp", "sftp", "wget", "ping", "tracert", "nc"], sibling: None, mark: None, glyph: NETWORK },
    Program { names: &["less", "more", "man", "bat", "glow", "cat", "type"], sibling: None, mark: None, glyph: PAGER },
    Program { names: &["yazi", "ranger", "nnn", "mc", "lf", "explorer"], sibling: None, mark: None, glyph: FILES },
];

/// The row of a program by its file name, whatever its case and extension (`NPM.CMD`, `python3.12.exe`).
fn program(file_name: &str) -> Option<&'static Program> {
    let lower = file_name.to_lowercase();
    let name = ["exe", "cmd", "bat", "com"]
        .iter()
        .find_map(|extension| lower.strip_suffix(extension).and_then(|rest| rest.strip_suffix('.')))
        .unwrap_or(&lower);
    let name = if name.starts_with("python") { "python" } else { name };
    PROGRAMS.iter().find(|program| program.names.contains(&name))
}

/// The glyph a tab shows for the program it runs when it has neither an icon nor a mark.
pub fn glyph(file_name: &str) -> char {
    program(file_name).map_or(TERMINAL, |program| program.glyph)
}

/// The mark of a program by its file name: its SVG and its colour, none for the colour of the text.
pub fn brand(file_name: &str) -> Option<(&'static str, Option<[u8; 3]>)> {
    program(file_name)?.mark.as_ref().map(|mark| (mark.svg, mark.rgb))
}

/// A mark of [`brand`] `px` pixels square in `rgb`, as premultiplied RGBA: its one path, on a square of 24 units.
pub fn brand_pixels(svg: &str, rgb: [u8; 3], px: u32) -> Option<Vec<u8>> {
    let data = svg.split_once(" d=\"")?.1.split('"').next()?;
    let scale = px as f32 / 24.0;
    let (coverage, _) = zeno::Mask::new(data).size(px, px).transform(Some(zeno::Transform::scale(scale, scale))).render();
    let premultiplied = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    Some(coverage.iter().flat_map(|&a| [premultiplied(rgb[0], a), premultiplied(rgb[1], a), premultiplied(rgb[2], a), a]).collect())
}

/// `path` with its `.` and `..` worked out: a program can be started by such a path (Git's `bin\..\usr\bin\bash.exe`), and
/// the call that takes the icons out of a file does not work them out itself.
fn plain(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The program of the table installed beside `path` (the `git-bash.exe` of the Git a `git.exe` belongs to).
fn sibling(path: &Path) -> Option<std::path::PathBuf> {
    let (beside, up) = program(&path.file_name()?.to_string_lossy())?.sibling?;
    path.ancestors().skip(1).take(up).map(|dir| dir.join(beside)).find(|launcher| launcher.is_file())
}

fn from_file(path: &Path, px: u32) -> Option<Vec<u8>> {
    let mut name = [0u16; 260];
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().collect();
    if wide.is_empty() || wide.len() >= name.len() {
        return None;
    }
    name[..wide.len()].copy_from_slice(&wide);
    let mut icons = [HICON::default()];
    // SAFETY: the name is NUL-terminated in a buffer of the length the call takes; the icon it makes is destroyed below.
    let found = unsafe { PrivateExtractIconsW(&name, 0, px as i32, px as i32, Some(&mut icons), None, 0) };
    if found == 0 || found == u32::MAX || icons[0].is_invalid() {
        return None;
    }
    let pixels = icon_pixels(icons[0], px);
    // SAFETY: an icon this function made.
    unsafe {
        let _ = DestroyIcon(icons[0]);
    }
    pixels
}

/// The program an app execution alias stands for. The alias is a reparse point (`IO_REPARSE_TAG_APPEXECLINK`) whose data, after
/// the header and a version of 3, is NUL-terminated strings: the package, the app and the path of the program.
fn alias_target(path: &Path) -> Option<std::path::PathBuf> {
    const APPEXECLINK: u32 = 0x8000_001B;
    let mut data = vec![0u8; 16 * 1024];
    let mut read = 0u32;
    // SAFETY: a handle opened here and closed below; the buffer is as long as the size passed.
    unsafe {
        let file = CreateFileW(&HSTRING::from(path.as_os_str()), 0, FILE_SHARE_READ, None, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, None).ok()?;
        let asked = DeviceIoControl(file, FSCTL_GET_REPARSE_POINT, None, 0, Some(data.as_mut_ptr().cast()), data.len() as u32, Some(&mut read), None);
        let _ = CloseHandle(file);
        asked.ok()?;
    }
    alias_path(data.get(..read as usize)?, APPEXECLINK)
}

/// The path in the reparse data of an alias: its third string.
fn alias_path(data: &[u8], tag: u32) -> Option<std::path::PathBuf> {
    let word = |at: usize| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
    if word(0)? != tag || word(8)? != 3 {
        return None;
    }
    let units: Vec<u16> = data.get(12..)?.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    let target = units.split(|unit| *unit == 0).nth(2)?;
    (!target.is_empty()).then(|| std::path::PathBuf::from(String::from_utf16_lossy(target)))
}

/// The rows of a bitmap of the icon as 32-bit BGRA, top down.
fn bitmap_rows(bitmap: HBITMAP, px: u32) -> Option<Vec<u8>> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: px as i32,
            biHeight: -(px as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut data = vec![0u8; (px * px * 4) as usize];
    // SAFETY: a memory DC of our own, deleted below; the buffer holds the rows the header asks for.
    unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, px, Some(data.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        (lines == px as i32).then_some(data)
    }
}

fn icon_pixels(icon: HICON, px: u32) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    // SAFETY: a valid icon; the bitmaps it hands back are deleted below.
    unsafe { GetIconInfo(icon, &mut info).ok()? };
    let colour = (!info.hbmColor.is_invalid()).then(|| bitmap_rows(info.hbmColor, px)).flatten();
    let mask = (!info.hbmMask.is_invalid()).then(|| bitmap_rows(info.hbmMask, px)).flatten();
    // SAFETY: the bitmaps GetIconInfo made for us.
    unsafe {
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
    }
    Some(premultiplied_rgba(colour?, mask.as_deref()))
}

/// BGRA with straight alpha to premultiplied RGBA. An old icon without alpha has the transparency in its mask instead
/// (white where it is clear).
fn premultiplied_rgba(mut bgra: Vec<u8>, mask: Option<&[u8]>) -> Vec<u8> {
    let has_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
    for (i, p) in bgra.chunks_exact_mut(4).enumerate() {
        let alpha = if has_alpha {
            p[3]
        } else {
            match mask {
                Some(mask) if mask[i * 4] != 0 => 0,
                _ => 255,
            }
        };
        let (b, g, r) = (p[0], p[1], p[2]);
        let scale = |c: u8| ((u16::from(c) * u16::from(alpha) + 127) / 255) as u8;
        p.copy_from_slice(&[scale(r), scale(g), scale(b), alpha]);
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_premultiplied_and_turned_into_rgba() {
        let half_red = vec![0, 0, 255, 128];
        assert_eq!(premultiplied_rgba(half_red, None), [128, 0, 0, 128]);
        // No alpha anywhere: the mask says what is clear.
        let opaque_blue_and_clear = vec![255, 0, 0, 0, 255, 0, 0, 0];
        let mask = [0, 0, 0, 0, 255, 255, 255, 0];
        assert_eq!(premultiplied_rgba(opaque_blue_and_clear, Some(&mask)), [0, 0, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn the_bash_of_git_takes_the_icon_of_git_bash() {
        let root = std::env::temp_dir().join(format!("nekominal-git-{}", std::process::id()));
        for dir in ["bin", "usr\\bin"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("git-bash.exe"), b"").unwrap();
        assert_eq!(sibling(&root.join("bin\\bash.exe")), Some(root.join("git-bash.exe")));
        assert_eq!(sibling(&root.join("usr\\bin\\BASH.EXE")), Some(root.join("git-bash.exe")));
        assert_eq!(sibling(&root.join("cmd\\git.exe")), Some(root.join("git-bash.exe")), "git takes Git Bash's icon too");
        assert_eq!(sibling(&root.join("bin\\sh.exe")), None, "only bash");
        assert_eq!(sibling(Path::new("C:\\Windows\\System32\\bash.exe")), None, "the bash of WSL has no Git above it");
        // The bash that Git's launcher starts, by the path it starts it with.
        assert_eq!(plain(Path::new("C:\\git\\2.5\\bin\\..\\usr\\.\\bin\\bash.exe")), Path::new("C:\\git\\2.5\\usr\\bin\\bash.exe"));
        assert_eq!(sibling(&plain(&root.join("bin\\..\\usr\\bin\\bash.exe"))), Some(root.join("git-bash.exe")));
        // npm beside node, and only beside it.
        std::fs::write(root.join("bin\\node.exe"), b"").unwrap();
        assert_eq!(sibling(&root.join("bin\\npm.exe")), Some(root.join("bin\\node.exe")));
        assert_eq!(sibling(&root.join("usr\\bin\\npx.exe")), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_alias_names_its_program_in_its_third_string() {
        let mut data = 0x8000_001Bu32.to_le_bytes().to_vec();
        data.extend([0u8; 4]);
        data.extend(3u32.to_le_bytes());
        for text in ["Microsoft.PowerShell_8wekyb3d8bbwe", "Microsoft.PowerShell_8wekyb3d8bbwe!App", r"C:\Program Files\WindowsApps\P\pwsh.exe", "0"] {
            data.extend(text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes));
        }
        assert_eq!(alias_path(&data, 0x8000_001B), Some(std::path::PathBuf::from(r"C:\Program Files\WindowsApps\P\pwsh.exe")));
        assert_eq!(alias_path(&data, 0xA000_000C), None, "another kind of reparse point");
        assert_eq!(alias_path(&data[..20], 0x8000_001B), None, "cut short");
    }

    #[test]
    fn a_mark_is_its_path_filled_on_the_square_the_right_way_up() {
        // The top left half of the square: SVG's y runs down.
        let pixels = brand_pixels(r#"<svg viewBox="0 0 24 24"><path d="M0 0H24L0 24Z"/></svg>"#, [255, 0, 0], 24).unwrap();
        assert_eq!(pixels.len(), 24 * 24 * 4);
        assert_eq!(&pixels[..4], [255, 0, 0, 255], "the top left is covered");
        assert_eq!(&pixels[pixels.len() - 4..], [0, 0, 0, 0], "the bottom right is clear");
        for name in ["git.exe", "GH.EXE", "npx.cmd", "uvx.exe", "esbuild.exe", "7z.exe"] {
            let (svg, rgb) = brand(name).unwrap_or_else(|| panic!("{name} has a mark"));
            let pixels = brand_pixels(svg, rgb.unwrap_or([255; 3]), 20).unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[3] == 255) && pixels.chunks_exact(4).any(|p| p[3] < 128), "{name} is drawn");
        }
        assert_eq!(brand("pwsh.exe"), None, "a program with an icon of its own needs none");
    }

    #[test]
    fn a_tab_shows_the_glyph_of_the_program_it_runs_whatever_the_case_or_the_extension() {
        assert_eq!(glyph("vim.exe"), glyph("NVIM.EXE"));
        assert_eq!(glyph("ssh.exe"), NETWORK);
        assert_eq!(glyph("git"), GIT);
        assert_eq!(glyph("python3.12.exe"), glyph("py.exe"));
        assert_eq!(glyph("npm.cmd"), glyph("node.exe"));
        // A shell and a program nobody knows have the terminal's glyph; a name that only ends like an extension is no extension.
        assert_eq!(glyph("pwsh.exe"), TERMINAL);
        assert_eq!(glyph("whatever.exe"), TERMINAL);
        assert_ne!(glyph("exe"), EDITOR);
    }

    #[test]
    fn every_name_is_in_one_row_and_every_mark_is_a_file_with_its_licence_noticed() {
        let names: Vec<&str> = PROGRAMS.iter().flat_map(|program| program.names.iter().copied()).collect();
        for name in &names {
            assert_eq!(names.iter().filter(|other| *other == name).count(), 1, "{name} is in one row");
            assert_eq!(*name, name.to_lowercase(), "{name} is in lower case");
        }
        let marks: Vec<&Mark> = PROGRAMS.iter().filter_map(|program| program.mark.as_ref()).collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brands");
        let mut files: Vec<String> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        let mut used: Vec<String> = marks.iter().map(|mark| mark.file.to_owned()).collect();
        files.sort();
        used.sort();
        used.dedup();
        assert_eq!(files, used, "every file of assets/brands is a mark of the table, and no other");
        let notices = include_str!("../../../THIRD-PARTY-NOTICES.md");
        for mark in marks {
            let stem = mark.file.trim_end_matches(".svg");
            let named = match mark.license {
                License::Cc0 => notices.contains(&format!("`{stem}`")),
                License::CcBy3 => notices.contains(&format!("`{}`", mark.file)) && notices.contains("CC BY 3.0"),
                License::CcBy4 => notices.contains(&format!("`{}`", mark.file)) && notices.contains("CC BY 4.0"),
            };
            assert!(named, "THIRD-PARTY-NOTICES.md names {} under its licence ({:?})", mark.file, mark.license);
        }
    }

    #[test]
    fn a_program_of_the_system_has_an_icon_of_the_size_asked_for() {
        let cmd = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into())).join("System32\\cmd.exe");
        let pixels = load(&cmd, 24).expect("cmd.exe has an icon");
        assert_eq!(pixels.len(), 24 * 24 * 4);
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 255), "some of it is opaque");
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 0), "and some of it is clear");
        assert_eq!(key(Path::new("C:\\A\\B.exe")), key(Path::new("c:\\a\\b.EXE")));
        assert!(load(Path::new("C:\\no\\such\\program.exe"), 24).is_none());
    }
}
