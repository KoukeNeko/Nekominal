//! The silence policy: no routine action of the interface makes a sound, and the only code that may ask
//! Windows for one is `crates/nekominal-app/src/sound.rs` (the opt-in audible bell). This is the static half of
//! the invariant; `tools/slice-drive/sound-witness.ps1` is the runtime half.

use std::fs;
use std::path::{Path, PathBuf};

/// Names that make or enable a sound. `Beep(` also covers `MessageBeep(`.
const SOUND_API: &[&str] = &[
    "MessageBeep",
    "PlaySound",
    "sndPlaySound",
    "Beep(",
    "ElementSoundPlayer",
    "Win32::Media::Audio",
    "Win32_Media_Audio",
    "Win32_Media_Multimedia",
];

/// The files allowed to name them, relative to the `crates` directory: the one sound module and this list.
const ALLOWED: &[&str] = &["nekominal-app/src/sound.rs", "nekominal-app/tests/silence.rs"];

/// Generated bindings declare every WinUI type; they make no sound until some other file calls them.
const SKIPPED_CRATES: &[&str] = &["nekominal-winui-sys"];

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                collect(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") || path.file_name().is_some_and(|n| n == "Cargo.toml")
        {
            out.push(path);
        }
    }
}

#[test]
fn only_the_sound_module_names_a_sound_api() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates directory");
    let mut files = Vec::new();
    for entry in fs::read_dir(crates).expect("crates is readable") {
        let path = entry.expect("directory entry").path();
        let skipped = path.file_name().is_some_and(|name| SKIPPED_CRATES.iter().any(|s| name == *s));
        if path.is_dir() && !skipped {
            collect(&path, &mut files);
        }
    }
    assert!(files.len() > 50, "the scan found only {} files", files.len());
    let mut offences = Vec::new();
    for path in files {
        let relative = path.strip_prefix(crates).expect("under crates").to_string_lossy().replace('\\', "/");
        if ALLOWED.contains(&relative.as_str()) {
            continue;
        }
        let text = fs::read_to_string(&path).expect("source is UTF-8");
        for (number, line) in text.lines().enumerate() {
            // A comment cannot make a sound.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(api) = SOUND_API.iter().find(|api| line.contains(**api)) {
                offences.push(format!("{relative}:{}: {api}", number + 1));
            }
        }
    }
    assert!(offences.is_empty(), "sound APIs outside sound.rs:\n{}", offences.join("\n"));
}
