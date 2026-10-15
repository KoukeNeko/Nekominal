//! The silence policy: no routine action of the interface makes a sound, and the only code that may ask
//! Windows for one is `crates/nekominal-app/src/sound.rs` (the opt-in audible bell). This is the static half of
//! the invariant; `tools/slice-drive/sound-witness.ps1` is the runtime half, and `window.rs` has the test that the
//! one sound nobody asks for, the system beep of an Alt chord nothing handles, is answered.

use std::fs;
use std::path::{Path, PathBuf};

/// Names that make or enable a sound. `Beep(` also covers `MessageBeep(`. `ElementSoundMode` is the XAML attribute
/// (and property) with which a WinUI control plays its own sounds.
const SOUND_API: &[&str] = &[
    "MessageBeep",
    "PlaySound",
    "sndPlaySound",
    "Beep(",
    "ElementSoundPlayer",
    "ElementSoundMode",
    "Win32::Media::Audio",
    "Win32_Media_Audio",
    "Win32_Media_Multimedia",
];

/// The files allowed to name them, relative to the `crates` directory: the one sound module and this list.
const ALLOWED: &[&str] = &["nekominal-app/src/sound.rs", "nekominal-app/tests/silence.rs"];

/// Generated bindings declare every WinUI type; they make no sound until some other file calls them.
const SKIPPED_CRATES: &[&str] = &["nekominal-winui-sys"];

/// Source files the scan reads: Rust, manifests (a Cargo feature enables an API) and XAML (markup sets properties).
fn is_scanned(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "rs" || ext == "xaml") || path.file_name().is_some_and(|n| n == "Cargo.toml")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                collect(&path, out);
            }
        } else if is_scanned(&path) {
            out.push(path);
        }
    }
}

/// The lines of `text` that name a sound API, as (line number, the name). A comment cannot make a sound.
fn sound_lines(text: &str) -> Vec<(usize, &'static str)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .filter_map(|(number, line)| SOUND_API.iter().find(|api| line.contains(**api)).map(|api| (number + 1, *api)))
        .collect()
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
    assert!(files.iter().any(|path| path.extension().is_some_and(|ext| ext == "xaml")), "the scan reads markup too");
    let mut offences = Vec::new();
    for path in files {
        let relative = path.strip_prefix(crates).expect("under crates").to_string_lossy().replace('\\', "/");
        if ALLOWED.contains(&relative.as_str()) {
            continue;
        }
        let text = fs::read_to_string(&path).expect("source is UTF-8");
        for (number, api) in sound_lines(&text) {
            offences.push(format!("{relative}:{number}: {api}"));
        }
    }
    assert!(offences.is_empty(), "sound APIs outside sound.rs:\n{}", offences.join("\n"));
}

/// The scan is only worth something if it can find what it looks for.
#[test]
fn the_scan_finds_each_way_of_making_a_sound_and_ignores_comments() {
    for api in SOUND_API {
        let call = format!("let _ = unsafe {{ {api}0) }};");
        // Some names contain another (sndPlaySound has PlaySound): the line is found, whichever name is reported.
        let found = sound_lines(&call);
        assert!(matches!(found.as_slice(), [(1, _)]), "{api}: {found:?}");
    }
    assert_eq!(
        sound_lines("fine\n<Button ElementSoundMode=\"Off\"/>\nfine\n    MessageBeep(0);"),
        [(2, "ElementSoundMode"), (4, "MessageBeep")]
    );
    assert_eq!(sound_lines("// MessageBeep(0) is how the bell sounds\n  /// PlaySound too"), []);
    assert!(is_scanned(Path::new("a/shell.xaml")) && is_scanned(Path::new("a/Cargo.toml")));
    assert!(!is_scanned(Path::new("a/notes.md")));
}
