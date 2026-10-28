//! Files typed into a pane as their paths: dropped on it from Explorer, or pasted when the clipboard holds files or an
//! image and no text. Apart from `app.rs`, which only hands over to it.

use crate::paste_files::{self, Target};

use super::*;

/// Where the program of the pane looks for files, and how its shell quotes: a WSL pane (its program is `wsl.exe`) sees
/// the drives under `/mnt`.
fn target(entry: &PaneEntry) -> Target<'_> {
    target_of(&entry.shell, &entry.profile_id, entry.job.as_deref())
}

fn target_of<'a>(shell: &str, profile_id: &'a str, job: Option<&str>) -> Target<'a> {
    if shell.eq_ignore_ascii_case("wsl") {
        // The profiles found on the machine are `wsl-<distribution>`; others leave the distribution unknown.
        return Target::Wsl(profile_id.strip_prefix("wsl-"));
    }
    // The program in front reads what is typed, not the shell of the profile: a cmd tab that runs ssh or wsl gets the
    // path quoted for bash, where `$(…)` in a file name would run inside the double quotes cmd's are.
    match job.map(program_name).as_deref() {
        Some("wsl") => Target::Wsl(None),
        Some("ssh" | "bash" | "sh" | "zsh" | "fish") => Target::Bash,
        _ => Target::of_shell(shell),
    }
}

/// A program's file name without `.exe`, in lower case.
fn program_name(file: &str) -> String {
    let lower = file.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_in_front_decides_how_a_path_is_quoted() {
        assert_eq!(target_of("cmd", "cmd", Some("cmd.exe")), Target::of_shell("cmd"));
        assert_eq!(target_of("cmd", "cmd", Some("ssh.exe")), Target::Bash, "ssh in a cmd tab reads bash's quoting");
        assert_eq!(target_of("pwsh", "pwsh", Some("WSL.EXE")), Target::Wsl(None));
        assert_eq!(target_of("cmd", "cmd", None), Target::of_shell("cmd"));
        assert_eq!(target_of("wsl", "wsl-Ubuntu", Some("bash")), Target::Wsl(Some("Ubuntu")), "a WSL profile keeps its distribution");
        let typed = crate::paste_files::path_text(&[std::path::PathBuf::from(r"C:\dl\notes $(rm -rf ~).txt")], target_of("cmd", "cmd", Some("ssh.exe")));
        assert!(typed.starts_with('\'') && typed.ends_with('\''), "single quotes, where nothing runs: {typed}");
    }
}

impl App {
    /// The text a paste into the focused pane types when the clipboard has no text: the paths of its files or image.
    pub(super) fn clipboard_files_text(&self) -> Option<String> {
        if !self.settings.paste.images {
            return None;
        }
        paste_files::clipboard_text(target(self.focused()?))
    }

    /// Files dropped at a pixel of the window: their paths are pasted into the pane there. A drop elsewhere (the tab strip)
    /// does nothing.
    pub fn drop_files(&mut self, x: i32, y: i32, paths: &[std::path::PathBuf]) {
        let Some(tab) = self.tab() else { return };
        let Some(entry) = tab.pane_at(x, y).and_then(|id| tab.entry(id)) else { return };
        if !paths.is_empty() && !entry.exited {
            entry.pane.paste(&paste_files::path_text(paths, target(entry)));
        }
    }
}
