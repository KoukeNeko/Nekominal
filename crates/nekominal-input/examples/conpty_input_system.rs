//! The same ConPTY input matrix against the in-box (kernel32) ConPTY. An example binary does not
//! sit next to the staged conpty.dll, so `nekominal-pty` falls back to the system implementation.
//! Usage: `cargo run --example conpty_input_system -- <group>...` (groups: keyboard-node,
//! keyboard-win32, keyboard-vt, modes, mouse, paste-focus).

#[path = "../experiments/matrix.rs"]
mod matrix;

use matrix::*;

fn main() {
    let groups: Vec<String> = std::env::args().skip(1).collect();
    let probe = Rig::spawn(Child::Node, b"", "identity-system");
    println!("backend: {}", probe.backend);
    drop(probe);
    assert!(
        !probe_backend_is_bundled(),
        "this example must run on the in-box ConPTY"
    );
    for group in groups {
        for planned in plan(&group) {
            let result = run(&planned);
            let report = result.report(&format!("SYSTEM {}", planned.name));
            print!("{report}");
            if let Ok(path) = std::env::var("INPUT_REPORT") {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
                file.write_all(report.as_bytes()).unwrap();
            }
        }
    }
}

fn probe_backend_is_bundled() -> bool {
    nekominal_pty::conpty::backend().map(|b| b.kind == "bundled").unwrap_or(false)
}
