//! Files typed into a pane as their paths: dropped on it from Explorer, or pasted when the clipboard holds files or an
//! image and no text. Apart from `app.rs`, which only hands over to it.

use crate::paste_files::{self, Target};

use super::*;

/// Where the program of the pane looks for files, and how its shell quotes: a WSL pane (its program is `wsl.exe`) sees
/// the drives under `/mnt`.
fn target(entry: &PaneEntry) -> Target<'_> {
    if entry.shell.eq_ignore_ascii_case("wsl") {
        // The profiles found on the machine are `wsl-<distribution>`; others leave the distribution unknown.
        Target::Wsl(entry.profile_id.strip_prefix("wsl-"))
    } else {
        Target::of_shell(&entry.shell)
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
