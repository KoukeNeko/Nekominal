//! The preview of an image whose path a pane shows: while the pointer is on the path (the link it underlines), the picture
//! hangs under it, scaled to fit, with the file's name and its size in pixels. Passive: it opens and closes with the
//! underline, and nothing in it takes the pointer.

use std::path::{Path, PathBuf};

use super::*;
use crate::preview;

/// As the other cards have it: the acrylic shows around the picture.
const PAD: f32 = 16.0;
/// The narrowest card, so that a small icon still has room for its name.
const MIN_W: f32 = 200.0;

/// The image under the pointer: its file, its size in pixels, a key for the picture that changes with the file, and where
/// the link is (its first row, in DIP).
#[derive(Clone, Debug, PartialEq)]
pub struct Tip {
    path: PathBuf,
    size: (u32, u32),
    key: u64,
    anchor: Rect,
}

impl Tip {
    /// The tip for the file `target` a link names, under `anchor`; none when it is no image Windows can read.
    pub fn of(target: &str, anchor: Rect, last: Option<&Tip>) -> Option<Tip> {
        let path = Path::new(target);
        if !path.is_absolute() || !preview::is_image(path) {
            return None;
        }
        // The same file as the last tip: its header is not read again on every move of the pointer along the path.
        if let Some(last) = last.filter(|last| last.path == path) {
            return Some(Tip { anchor, ..last.clone() });
        }
        let size = preview::size(path)?;
        use std::hash::{Hash, Hasher};
        let mut key = std::collections::hash_map::DefaultHasher::new();
        (path, std::fs::metadata(path).and_then(|m| m.modified()).ok()).hash(&mut key);
        Some(Tip { path: path.to_owned(), size, key: key.finish(), anchor })
    }
}

impl App {
    pub(super) fn paint_image_tip(&mut self) {
        let Some(tip) = self.image_tip.clone() else { return };
        let g = self.glass;
        let small = self.ui.line_height(SMALL_SIZE);
        let (width, window_h) = (self.layout.size.0, self.layout.size.1);
        // The longest side the settings give, within the store of the UI's pictures (at most 1024 pixels on a side) at any
        // scale of the window, and within a window that is lower than that.
        let side = self.settings.previews.size as f32;
        let room_w = side.min(1000.0 / self.scale).min(width - 2.0 * PAD - 16.0);
        let room_h = side.min(1000.0 / self.scale).min(window_h - 3.0 * PAD - small - 6.0 - 16.0).max(40.0);
        let (w, h) = preview::fit(tip.size.0, tip.size.1, room_w.max(40.0), room_h, self.scale);
        let card_w = (w + 2.0 * PAD).max(MIN_W);
        let card_h = PAD + h + 6.0 + small + PAD;
        // Under the link, or over it when the window has no room below, as the other cards hang from what opens them.
        let (card, _) = hang(tip.anchor, (card_w, card_h), (width, window_h));
        let (x, y) = (card.x, card.y);
        if self.panels_failed {
            self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        }
        self.ui.rrect(card, CORNER_RADIUS, self.panel_fill(252), Some((col(g.divider()), 1.0)));
        self.panel_shapes.push((card, CORNER_RADIUS));
        // A checkerboard would say "transparent" louder than the picture: the card's own colour is behind it.
        let path = tip.path.clone();
        self.ui.picture(tip.key, x + (card_w - w) / 2.0, y + PAD, w, h, |pw, ph| preview::pixels(&path, pw, ph));
        let dims = format!("{} \u{d7} {}", tip.size.0, tip.size.1);
        let dims_w = self.ui.measure(&dims, SMALL_SIZE, false);
        let name = tip.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = subagents_card::fit(&mut self.ui, &name, SMALL_SIZE, false, card_w - 2.0 * PAD - dims_w - 10.0);
        let text_y = y + PAD + h + 6.0;
        self.ui.text(x + PAD, text_y, &name, SMALL_SIZE, col(g.text()), false);
        self.ui.text(x + card_w - PAD - dims_w, text_y, &dims, SMALL_SIZE, col(g.text_secondary()), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::LinkAction::{Copy, Open, Reveal};

    #[test]
    fn the_menu_of_a_file_can_show_it_and_copy_its_path_and_that_of_an_address_only_copies() {
        assert_eq!(link_menu_items(true), [(Text::CommonOpen, Open), (Text::LinkReveal, Reveal), (Text::LinkCopyPath, Copy)]);
        assert_eq!(link_menu_items(false), [(Text::CommonOpen, Open), (Text::CardCopyLink, Copy)], "no file to show for an address");
    }

    #[test]
    fn only_an_image_that_windows_reads_gets_a_tip_and_the_same_file_is_not_read_again() {
        let anchor = Rect::new(10.0, 20.0, 100.0, 18.0);
        assert_eq!(Tip::of(r"C:\notes.txt", anchor, None), None, "not an image");
        assert_eq!(Tip::of("shots/a.png", anchor, None), None, "a relative path is no checked file");
        assert_eq!(Tip::of(r"C:\no\such\picture.png", anchor, None), None, "nothing Windows can read");
        // The last tip's file is taken as it was, only moved: its header is not read again.
        let last = Tip { path: PathBuf::from(r"C:\gone\a.png"), size: (40, 30), key: 7, anchor };
        let moved = Rect::new(50.0, 20.0, 100.0, 18.0);
        assert_eq!(Tip::of(r"C:\gone\a.png", moved, Some(&last)), Some(Tip { anchor: moved, ..last }));
    }
}
