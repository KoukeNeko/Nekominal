//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a Direct3D 11 swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod access;
mod agent;
mod app;
mod cli;
mod clipboard;
mod design;
mod find;
mod glass;
mod gpu;
mod icons;
mod integration;
mod layout;
#[allow(dead_code)]
mod logic;
mod mark;
mod motion;
mod open;
mod pick;
mod popup;
mod settings_page;
mod settings_window;
mod status;
mod tab;
mod textfield;
mod update;
mod welcome;
mod widgets;
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
    // Claude Code's status line command is this program too: it hands the session over and is gone.
    if agent::statusline_main() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli::set_launch(cli::parse(&args));
    if let Err(e) = win::run() {
        std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
    }
}
