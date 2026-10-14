use std::{env, fs, path::PathBuf};

fn main() {
    stage_conpty();
}

/// Copies the bundled conpty.dll / OpenConsole.exe pair next to the built executables (and the
/// test binaries in `deps`) so `cargo run` and `cargo test` load the same matched pair that a
/// release ships. Run `scripts/fetch-conpty.ps1` to populate `resources/`.
fn stage_conpty() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest_dir.join("resources");
    println!("cargo:rerun-if-changed={}", source.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || !source.is_dir() {
        return;
    }

    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out; executables live in target/<profile>.
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    let deps_dir = profile_dir.join("deps");

    for name in ["conpty.dll", "OpenConsole.exe", "VERSION"] {
        let from = source.join(name);
        if !from.is_file() {
            continue;
        }
        for target in [profile_dir, deps_dir.as_path()] {
            if target.is_dir() {
                fs::copy(&from, target.join(name))
                    .unwrap_or_else(|e| panic!("failed to stage {name}: {e}"));
            }
        }
    }
}
