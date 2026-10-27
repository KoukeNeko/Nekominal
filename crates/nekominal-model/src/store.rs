//! Where the settings and the saved session live (`%APPDATA%\Nekominal`) and how they are read and written:
//! atomically, with the previous version kept as a `.bak`, and for the settings the first original kept as an `.orig`.
//! The only part of the model that touches the file system; what the files contain is the business of `settings` and
//! `session`.

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const APP_DIR: &str = "Nekominal";
const SETTINGS_FILE: &str = "settings.json";
const SESSION_FILE: &str = "session.json";

/// `%APPDATA%\Nekominal`; `None` when the profile has no `APPDATA` (a service account, for one).
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join(APP_DIR))
}

pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(SETTINGS_FILE))
}

pub fn session_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(SESSION_FILE))
}

/// The text of the file, or `None` when it does not exist. Accepts what editors write: UTF-8 with or without
/// a byte order mark, and UTF-16 (old Notepad's "Unicode"). Anything else is an `InvalidData` error.
pub fn load(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => decode(&bytes).map(Some).map_err(|reason| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {reason}", path.display()),
            )
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn decode(bytes: &[u8]) -> Result<String, &'static str> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| -> Result<String, &'static str> {
        let (pairs, tail) = rest.as_chunks::<2>();
        if !tail.is_empty() {
            return Err("UTF-16 text with an odd number of bytes");
        }
        String::from_utf16(&pairs.iter().map(|pair| unit(*pair)).collect::<Vec<u16>>())
            .map_err(|_| "invalid UTF-16 text")
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|_| "not UTF-8 text")
        }
        _ => String::from_utf8(bytes.to_vec()).map_err(|_| "not UTF-8 text"),
    }
}

/// Replaces the file with `text`: the new contents are written next to it and flushed to disk, the old
/// file is copied to `<name>.bak`, and the new file is renamed over it, so a crash leaves either version
/// whole. Creates the directory when needed. A file that is a link (into a folder of dotfiles) is saved where it points,
/// and the link stays.
pub fn save(path: &Path, text: &str) -> io::Result<()> {
    let target = fs::symlink_metadata(path).ok().filter(|meta| meta.file_type().is_symlink()).and_then(|_| fs::canonicalize(path).ok());
    let path = target.as_deref().unwrap_or(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // This process's own: two windows' processes saving at once must not write into one temp file and move each other's.
    let temp = sibling(path, &format!(".{}.tmp", std::process::id()))?;
    let result = write_synced(&temp, text).and_then(|()| {
        if path.exists() {
            fs::copy(path, sibling(path, ".bak")?)?;
        }
        fs::rename(&temp, path)
    });
    if result.is_err() {
        // The save already failed; a leftover temp file is harmless and a failure to remove it adds nothing.
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Like [`save`] for a file that the application rewrites from what it holds, which loses what it does not hold: the
/// comments of a hand-written `settings.json` and the keys this version does not know cannot survive the rewrite. So the
/// first time this replaces a file that exists, that file is copied to `<name>.orig` byte for byte, and the copy is never
/// touched again (the `.bak` of [`save`] is only the version before the last write). When the original cannot be kept
/// nothing is written.
pub fn save_keeping_original(path: &Path, text: &str) -> io::Result<()> {
    let original = sibling(path, ".orig")?;
    if path.exists() && !original.exists() {
        fs::copy(path, &original)?;
    }
    save(path, text)
}

fn write_synced(path: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// `settings.json` -> `settings.json.bak`, in the same directory.
fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name: OsString = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?
        .to_owned();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp directory, removed by the test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nekominal-model-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_files_live_under_appdata_nekominal() {
        // Read-only: the real environment names the same directory these functions do.
        match std::env::var_os("APPDATA") {
            Some(appdata) => {
                let dir = PathBuf::from(appdata).join("Nekominal");
                assert_eq!(config_dir(), Some(dir.clone()));
                assert_eq!(settings_path(), Some(dir.join("settings.json")));
                assert_eq!(session_path(), Some(dir.join("session.json")));
            }
            None => assert_eq!(
                (config_dir(), settings_path(), session_path()),
                (None, None, None)
            ),
        }
    }

    #[test]
    fn a_missing_file_loads_as_none() {
        let dir = temp_dir("missing");
        assert_eq!(load(&dir.join("settings.json")).unwrap(), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_creates_the_directory_and_leaves_only_the_file() {
        let dir = temp_dir("create");
        let path = dir.join("nested").join("deeper").join("settings.json");
        save(&path, "{}\n").unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some("{}\n"));
        assert_eq!(
            names_in(path.parent().unwrap()),
            ["settings.json"],
            "no temp file and no backup of nothing"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_again_keeps_the_previous_version_as_a_backup() {
        let dir = temp_dir("backup");
        let path = dir.join("settings.json");
        save(&path, "one").unwrap();
        save(&path, "two").unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some("two"));
        assert_eq!(
            fs::read_to_string(dir.join("settings.json.bak")).unwrap(),
            "one"
        );
        save(&path, "three").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("settings.json.bak")).unwrap(),
            "two",
            "only one generation is kept"
        );
        assert_eq!(names_in(&dir), ["settings.json", "settings.json.bak"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_first_rewrite_of_a_file_keeps_the_original_and_no_later_one_touches_it() {
        let dir = temp_dir("orig");
        let path = dir.join("settings.json");
        let original = "// my notes\r\n{ \"theme\": \"Nord\", \"future\": { \"x\": 1 } } // 設定\r\n";
        fs::write(&path, original).unwrap();
        save_keeping_original(&path, "{}\n").unwrap();
        assert_eq!(fs::read_to_string(dir.join("settings.json.orig")).unwrap(), original, "byte for byte");
        assert_eq!(fs::read_to_string(dir.join("settings.json.bak")).unwrap(), original);
        assert_eq!(load(&path).unwrap().as_deref(), Some("{}\n"));
        save_keeping_original(&path, "two").unwrap();
        save_keeping_original(&path, "three").unwrap();
        assert_eq!(fs::read_to_string(dir.join("settings.json.orig")).unwrap(), original, "the original stays");
        assert_eq!(fs::read_to_string(dir.join("settings.json.bak")).unwrap(), "two", "the backup is one generation");
        assert_eq!(names_in(&dir), ["settings.json", "settings.json.bak", "settings.json.orig"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_did_not_exist_has_no_original_and_a_plain_save_never_makes_one() {
        let dir = temp_dir("no-orig");
        let path = dir.join("settings.json");
        save_keeping_original(&path, "{}\n").unwrap();
        assert_eq!(names_in(&dir), ["settings.json"], "nothing to keep");
        // The second write is the first rewrite of a file that exists: now the file is kept.
        save_keeping_original(&path, "{ \"a\": 1 }\n").unwrap();
        assert_eq!(fs::read_to_string(dir.join("settings.json.orig")).unwrap(), "{}\n");
        let session = dir.join("session.json");
        save(&session, "one").unwrap();
        save(&session, "two").unwrap();
        assert_eq!(
            names_in(&dir),
            ["session.json", "session.json.bak", "settings.json", "settings.json.bak", "settings.json.orig"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_is_written_when_the_original_cannot_be_kept() {
        let dir = temp_dir("orig-fails");
        // Something that exists but cannot be copied as a file: a directory where the settings file should be.
        let path = dir.join("settings.json");
        fs::create_dir(&path).unwrap();
        assert!(save_keeping_original(&path, "new").is_err());
        assert_eq!(names_in(&dir), ["settings.json"], "no original, no backup, no temp file");
        assert!(path.is_dir(), "and the thing that was there is untouched");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn text_round_trips_exactly_including_chinese_and_line_endings() {
        let dir = temp_dir("text");
        let path = dir.join("session.json");
        let text = "{\r\n  \"cwd\": \"C:\\\\使用者\\\\你好 😀\"\r\n}\r\n";
        save(&path, text).unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some(text));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_save_leaves_the_old_file_and_no_temp_file() {
        let dir = temp_dir("failed");
        let path = dir.join("settings.json");
        save(&path, "old").unwrap();
        // The backup cannot be written when a directory sits where `settings.json.bak` should go.
        fs::create_dir(dir.join("settings.json.bak")).unwrap();
        assert!(save(&path, "new").is_err());
        assert_eq!(load(&path).unwrap().as_deref(), Some("old"));
        assert_eq!(
            names_in(&dir),
            ["settings.json", "settings.json.bak"],
            "the temp file is cleaned up"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_temp_file_is_this_processs_own_so_another_process_writing_its_own_does_not_get_in_the_way() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp_dir("temp-per-process");
        let path = dir.join("settings.json");
        // Another process in the middle of its save: its temp file is open and shares nothing.
        let other = File::options().write(true).create(true).truncate(true).share_mode(0).open(dir.join("settings.json.4242.tmp")).unwrap();
        save(&path, "mine").unwrap();
        drop(other);
        assert_eq!(load(&path).unwrap().as_deref(), Some("mine"));
        assert_eq!(names_in(&dir), ["settings.json", "settings.json.4242.tmp"], "its file is left to it, and none of this one's");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_path_without_a_file_name_is_refused() {
        let error = save(Path::new(".."), "x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn loading_accepts_what_editors_write() {
        let dir = temp_dir("encodings");
        let text = "{ \"theme\": \"Nord\", \"note\": \"設定\" }";
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let cases: [(&str, Vec<u8>); 4] = [
            ("utf8", text.as_bytes().to_vec()),
            (
                "utf8-bom",
                [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
            ),
            (
                "utf16le-bom",
                [
                    &[0xFF, 0xFE][..],
                    &utf16
                        .iter()
                        .flat_map(|u| u.to_le_bytes())
                        .collect::<Vec<u8>>(),
                ]
                .concat(),
            ),
            (
                "utf16be-bom",
                [
                    &[0xFE, 0xFF][..],
                    &utf16
                        .iter()
                        .flat_map(|u| u.to_be_bytes())
                        .collect::<Vec<u8>>(),
                ]
                .concat(),
            ),
        ];
        for (name, bytes) in cases {
            let path = dir.join(format!("{name}.json"));
            fs::write(&path, bytes).unwrap();
            assert_eq!(load(&path).unwrap().as_deref(), Some(text), "{name}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loading_rejects_text_it_cannot_decode_instead_of_guessing() {
        let dir = temp_dir("invalid");
        for (name, bytes) in [
            ("latin1", vec![b'{', 0xE9, b'}']),
            ("odd-utf16", vec![0xFF, 0xFE, 0x41]),
            ("lone-surrogate", vec![0xFF, 0xFE, 0x00, 0xD8]),
        ] {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap();
            let error = load(&path).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{name}");
            assert!(
                error.to_string().contains(name),
                "the message names the file: {error}"
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_empty_file_loads_as_empty_text() {
        let dir = temp_dir("empty");
        let path = dir.join("settings.json");
        fs::write(&path, b"").unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some(""));
        fs::remove_dir_all(dir).unwrap();
    }
}
