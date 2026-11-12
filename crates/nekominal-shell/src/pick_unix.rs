//! The dialogs that pick a program, a folder or color scheme files: `zenity` (GNOME and most desktops) or `kdialog` (KDE),
//! whichever the system has.

use std::process::{Command, Stdio};

use crate::{util::on_path, win::Hwnd};

/// What a dialog answers: the lines it printed, or nothing when it was cancelled or there is no dialog program.
fn ask(zenity: &[&str], kdialog: &[&str]) -> Vec<String> {
    let mut command = if on_path("zenity") {
        let mut command = Command::new("zenity");
        command.args(zenity);
        command
    } else if on_path("kdialog") {
        let mut command = Command::new("kdialog");
        command.args(kdialog);
        command
    } else {
        crate::util::log("no zenity or kdialog to pick with");
        return Vec::new();
    };
    let Ok(output) = command.stdin(Stdio::null()).stderr(Stdio::null()).output() else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

/// A program, starting in the folder of `current`.
pub fn program(_owner: Hwnd, current: &str) -> Option<String> {
    let start = if current.is_empty() { "/usr/bin/".to_owned() } else { current.to_owned() };
    ask(&["--file-selection", "--title=Program", "--filename", &start], &["--getopenfilename", &start]).into_iter().next()
}

/// A folder, starting at `current`.
pub fn folder(_owner: Hwnd, current: &str) -> Option<String> {
    let start = if current.is_empty() { "~/".to_owned() } else { format!("{}/", current.trim_end_matches('/')) };
    ask(&["--file-selection", "--directory", "--title=Folder", "--filename", &start], &["--getexistingdirectory", &start]).into_iter().next()
}

/// The JSON files of color schemes to add.
pub fn color_presets(_owner: Hwnd) -> Vec<String> {
    ask(
        &["--file-selection", "--multiple", "--separator=\n", "--title=Color schemes", "--file-filter=JSON | *.json"],
        &["--getopenfilename", "~/", "*.json", "--multiple", "--separate-output"],
    )
}

/// Where to save a text file, starting with `name`.
pub fn save_text(_owner: Hwnd, name: &str) -> Option<String> {
    ask(&["--file-selection", "--save", "--confirm-overwrite", "--title=Save", "--filename", name], &["--getsavefilename", name]).into_iter().next()
}
