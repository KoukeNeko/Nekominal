//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a Direct3D 11 swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod access;
mod app;
mod find;
mod gpu;
mod layout;
mod popup;
mod status;
mod tab;
mod win;

/// What the shared modules below call to write to the log: the file named by NEKOMINAL_LOG, when there is one.
mod util {
    pub fn log(text: &str) {
        if let Some(path) = std::env::var_os("NEKOMINAL_LOG") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{text}");
            }
        }
    }
}

// The pure logic and the colours of the chrome are shared with the WinUI shell until that one is deleted; the files
// then move here.
#[path = "../../nekominal-app/src/clipboard.rs"]
mod clipboard;
#[path = "../../nekominal-app/src/open.rs"]
mod open;
#[path = "../../nekominal-app/src/glass.rs"]
mod glass;
#[path = "../../nekominal-app/src/logic.rs"]
#[allow(dead_code)]
mod logic;

fn main() {
    if let Err(e) = win::run() {
        std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
    }
}
