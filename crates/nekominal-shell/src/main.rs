//! Nekominal with a self-drawn shell: a plain Win32 window whose content is a Direct3D 11 swap chain on a DirectComposition
//! visual over a DWM backdrop. The chrome is drawn by `nekominal-ui`, the terminal panes by `nekominal-pane`.
#![windows_subsystem = "windows"]

mod access;
mod agent;
mod api;
mod app;
mod buffers;
mod cli;
mod clipboard;
mod design;
mod editor;
mod find;
mod glass;
mod handoff;
mod hold;
mod github;
mod gpu;
mod hooks;
mod icons;
mod integration;
mod layout;
mod logic;
mod mark;
mod motion;
mod notify;
mod open;
mod paste_files;
mod pick;
mod panels;
mod popup;
mod push;
mod ports;
mod preview;
mod remove_dialog;
mod search_tabs;
mod settings_page;
mod spend;
mod status;
mod subagents;
mod tab;
mod textfield;
mod toast;
mod triggers;
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

/// `nekominal cli …`: one request to the automation API of a running instance, its answer printed as JSON.
fn cli_main(args: &[String]) -> i32 {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // The program has no console of its own (it is a window program): the output goes to the console it was started
    // from, unless it is redirected, to a pipe or a file. A shell does not wait for a window program, so its prompt can
    // come before the output; `| Out-String` (or any pipe) makes it wait.
    // SAFETY: plain calls on the handles of this process.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| handle.is_invalid() || handle.0.is_null()) {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    let stdin = || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text);
        text
    };
    match cli::api_request(args, std::env::var(api::PANE_VAR).ok(), stdin) {
        Ok((pipe, request)) => api::run_cli(pipe.as_deref(), &request),
        Err(e) => {
            println!("{}", api::reply(&serde_json::Value::Null, &Err(e)));
            2
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
    // Claude Code's status line command and hooks are this program too: they hand the session over and are gone.
    // A copy of this program holding the tabs while Scoop updates the app (`hold`).
    if hold::hold_main() || agent::statusline_main() || agent::claude_hook_main() || hooks::hook_main() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|first| first == "cli") {
        std::process::exit(cli_main(&args[1..]));
    }
    // Started by the copy holding the tabs to take the windows of the Nekominal it replaced.
    let args = handoff::strip_adopt(args);
    cli::set_launch(cli::parse(&args));
    std::thread::spawn(paste_files::clean_old);
    std::thread::spawn(agent::clean_stale);
    if let Err(e) = win::run()
        && e != app::HANDED_OFF
    {
        std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
    }
}
