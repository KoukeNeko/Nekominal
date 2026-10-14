//! Nekominal: a Windows terminal in a frosted-glass WinUI 3 window.
//!
//! `nekominal-winui` boots the Windows App Runtime and the XAML application; the shell (window, top bar,
//! tabs, panes, overlays) lives in this crate and is driven from Rust over generated WinUI 3 bindings.
//! The terminal itself (ConPTY, emulator, wgpu renderer) is `nekominal-pane`.
#![windows_subsystem = "windows"]
// The bindings keep the Windows metadata names (`SetXxx`, `IFoo`, enum constants).
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

#[macro_use]
mod util;

mod chip;
mod clipboard;
mod cursor;
mod design;
mod findbar;
mod glass;
mod i18n;
mod keyboard;
mod logic;
mod palette;
mod pane_view;
mod settings_page;
mod shell;
mod sound;
mod tab;
mod terminal_control;
mod window;

fn main() {
    nekominal_winui::diag::init();
    util::log("main start");
    std::panic::set_hook(Box::new(|info| util::log(&format!("PANIC {info}"))));
    let result =
        nekominal_winui::app::run(|| shell::launch().inspect_err(|e| util::log(&format!("FATAL launch: {e}"))));
    util::log(&format!("app::run returned {result:?}"));
}
