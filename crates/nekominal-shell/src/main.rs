//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a Direct3D 11 swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod access;
mod agent;
mod app;
mod buffers;
mod cli;
mod clipboard;
mod design;
mod editor;
mod find;
mod glass;
mod github;
mod gpu;
mod hooks;
mod icons;
mod integration;
mod layout;
#[allow(dead_code)]
mod logic;
mod mark;
mod motion;
mod notify;
mod open;
mod pick;
mod popup;
mod ports;
mod search_tabs;
mod settings_page;
mod settings_window;
mod spend;
mod status;
mod tab;
mod textfield;
mod toast;
mod update;
mod welcome;
mod widgets;
mod win;
mod worktree;

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
    // Started from PowerShell 7, the shells and the update window would inherit its module folders, which break Windows
    // PowerShell (and Scoop in it).
    if let Some(value) = std::env::var_os("PSModulePath") {
        let kept = nekominal_pty::profiles::windows_powershell_module_path(&value.to_string_lossy());
        // SAFETY: the first thing the program does, before any other thread runs.
        unsafe {
            match kept {
                Some(kept) => std::env::set_var("PSModulePath", kept),
                None => std::env::remove_var("PSModulePath"),
            }
        }
    }
    // Claude Code's status line command is this program too: it hands the session over and is gone.
    if agent::statusline_main() || hooks::hook_main() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli::set_launch(cli::parse(&args));
    if let Err(e) = win::run()
        && e != app::HANDED_OFF
    {
        std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
    }
}
