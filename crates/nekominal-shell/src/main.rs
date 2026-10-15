//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a Direct3D 11 swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod access;
mod app;
mod clipboard;
mod find;
mod glass;
mod gpu;
mod layout;
#[allow(dead_code)]
mod logic;
mod open;
mod popup;
mod status;
mod tab;
mod win;

/// What the modules call to write to the log: the file named by NEKOMINAL_LOG, when there is one.
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

fn main() {
    if let Err(e) = win::run() {
        std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
    }
}
