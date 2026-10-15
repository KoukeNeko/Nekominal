//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a wgpu swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod app;
mod gpu;
mod layout;
mod win;

// The pure logic and the colours of the chrome are shared with the WinUI shell until that one is deleted; the files
// then move here.
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
