//! The pictures of one pane (Sixel and the kitty graphics protocol): their pixels, and where they are placed.
//!
//! A placement is held by the `LineId` of its top row on the primary screen, like the highlights of triggers, so it
//! scrolls with its text into the scrollback and goes when the scrollback trims its last row or the ids are reset (RIS,
//! `ED 3`). On the alternate screen, which has no scrollback, it is held by its screen row.
//!
//! Output is untrusted: one picture holds at most `MAX_IMAGE_BYTES`, the pane at most `MAX_PANE_IMAGE_BYTES` (the oldest
//! pictures go first) and `MAX_PLACEMENTS` placements.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::terminal::LineId;

/// The widest and tallest picture kept, in pixels.
pub const MAX_IMAGE_SIDE: u32 = crate::sixel::MAX_SIDE as u32;
/// One picture's pixels at most: 4096 x 4096 RGBA.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// All the pictures of a pane at most; past it the oldest are dropped with their placements.
pub const MAX_PANE_IMAGE_BYTES: usize = 256 * 1024 * 1024;
/// Placements of a pane at most; past it the oldest goes.
pub const MAX_PLACEMENTS: usize = 4096;

/// Keys are unique in the process, so a renderer that caches textures by key never mistakes one picture for another.
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct Image {
    pub key: u64,
    /// The kitty image id (`i`) and number (`I`); 0 for none. A picture with neither (a Sixel, an anonymous kitty
    /// transmission) lives only as long as it is placed.
    pub id: u32,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    /// RGBA, straight alpha.
    pub pixels: Arc<[u8]>,
}

impl Image {
    fn anonymous(&self) -> bool {
        self.id == 0 && self.number == 0
    }
}

/// The row a placement's top is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// A line of the primary screen or its scrollback.
    Line(LineId),
    /// A row of the alternate screen.
    Screen(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image: u64,
    /// The kitty placement id (`p`); 0 for none.
    pub id: u32,
    pub anchor: Anchor,
    pub column: usize,
    /// Pixels into the first cell.
    pub offset: [u32; 2],
    /// The part of the picture shown: x, y, width, height in its pixels.
    pub src: [u32; 4],
    /// Cells the picture is scaled into; None draws it at its own size.
    pub cells: Option<[u32; 2]>,
    /// Columns and rows it covers.
    pub span: [u32; 2],
    pub z: i32,
    /// A Sixel: text written over it erases it, as in xterm. A kitty placement stays until it is deleted.
    pub erasable: bool,
}

/// A picture in view, as `Snapshot::images` hands it to the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapImage {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub src: [u32; 4],
    /// Viewport row of its top (negative: it starts above the view) and column.
    pub row: i32,
    pub column: i32,
    pub offset: [u32; 2],
    pub cells: Option<[u32; 2]>,
    pub z: i32,
}

#[derive(Default)]
pub struct ImageStore {
    /// Oldest first.
    images: Vec<Image>,
    /// Oldest first.
    placements: Vec<Placement>,
    bytes: usize,
    next_id: u32,
}

/// The rows a placement covers, `top..bottom`, counted as `LineId::index` on the primary screen and as screen rows on
/// the alternate one; None for a placement of the other screen or an older epoch.
fn rows_of(placement: &Placement, alt: bool, epoch: u32) -> Option<(i64, i64)> {
    let top = match placement.anchor {
        Anchor::Line(id) if !alt && id.epoch == epoch => id.index as i64,
        Anchor::Screen(row) if alt => i64::from(row),
        _ => return None,
    };
    Some((top, top + i64::from(placement.span[1])))
}

impl ImageStore {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    pub fn has_placements(&self) -> bool {
        !self.placements.is_empty()
    }

    /// Bytes of pixels held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    pub fn image(&self, key: u64) -> Option<&Image> {
        self.images.iter().find(|image| image.key == key)
    }

    pub fn by_id(&self, id: u32) -> Option<&Image> {
        self.images.iter().find(|image| id != 0 && image.id == id)
    }

    /// The newest picture with image number `number`.
    pub fn by_number(&self, number: u32) -> Option<&Image> {
        self.images.iter().rev().find(|image| number != 0 && image.number == number)
    }

    /// An image id no picture has, for a transmission that gave only a number.
    pub fn free_id(&mut self) -> u32 {
        loop {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if self.by_id(self.next_id).is_none() {
                return self.next_id;
            }
        }
    }

    /// Keeps a picture; one with the same id is replaced, its placements with it. Returns its key.
    pub fn insert(&mut self, id: u32, number: u32, width: u32, height: u32, pixels: Vec<u8>) -> u64 {
        debug_assert!(pixels.len() <= MAX_IMAGE_BYTES);
        if id != 0 {
            self.remove_images(|image| image.id == id);
        }
        self.sweep();
        let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        self.bytes += pixels.len();
        self.images.push(Image { key, id, number, width, height, pixels: pixels.into() });
        while self.bytes > MAX_PANE_IMAGE_BYTES && self.images.len() > 1 {
            let oldest = self.images[0].key;
            self.remove_images(|image| image.key == oldest);
        }
        key
    }

    /// Adds a placement. One of the same picture with the same placement id is replaced.
    pub fn place(&mut self, placement: Placement) {
        if placement.id != 0 {
            self.placements.retain(|old| !(old.image == placement.image && old.id == placement.id));
        }
        if self.placements.len() >= MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        self.placements.push(placement);
    }

    /// Drops the pictures `doomed` picks and all their placements. True when any went.
    pub fn remove_images(&mut self, doomed: impl Fn(&Image) -> bool) -> bool {
        let keys: Vec<u64> = self.images.iter().filter(|image| doomed(image)).map(|image| image.key).collect();
        if keys.is_empty() {
            return false;
        }
        self.placements.retain(|placement| !keys.contains(&placement.image));
        self.images.retain(|image| {
            let gone = keys.contains(&image.key);
            if gone {
                self.bytes -= image.pixels.len();
            }
            !gone
        });
        true
    }

    /// Drops the placements `doomed` picks; with `free` (an upper-case kitty delete) also the pictures of those that are
    /// left with no placement. True when any went.
    pub fn remove_placements(&mut self, doomed: impl Fn(&Placement, &Image) -> bool, free: bool) -> bool {
        let mut touched = Vec::new();
        let images = &self.images;
        self.placements.retain(|placement| {
            let gone = images.iter().find(|image| image.key == placement.image).is_some_and(|image| doomed(placement, image));
            if gone {
                touched.push(placement.image);
            }
            !gone
        });
        if free {
            let orphaned: Vec<u64> =
                touched.iter().copied().filter(|key| !self.placements.iter().any(|p| p.image == *key)).collect();
            self.remove_images(|image| orphaned.contains(&image.key));
        }
        !touched.is_empty()
    }

    /// Drops the pictures nothing can show again: anonymous ones without a placement.
    pub fn sweep(&mut self) {
        let placements = &self.placements;
        let unplaced: Vec<u64> = self
            .images
            .iter()
            .filter(|image| image.anonymous() && !placements.iter().any(|p| p.image == image.key))
            .map(|image| image.key)
            .collect();
        if !unplaced.is_empty() {
            self.remove_images(|image| unplaced.contains(&image.key));
        }
    }

    /// Everything goes (RIS).
    pub fn clear(&mut self) {
        *self = Self { next_id: self.next_id, ..Self::default() };
    }

    /// Drops the placements of the primary screen (`alt` false) or of the alternate one. True when any went.
    pub fn clear_screen(&mut self, alt: bool) -> bool {
        let before = self.placements.len();
        self.placements.retain(|placement| matches!(placement.anchor, Anchor::Screen(_)) != alt);
        self.placements.len() != before
    }

    /// Takes the placements of the primary screen out (to be put back with new anchors after a reflow).
    pub fn take_primary(&mut self) -> Vec<Placement> {
        let (primary, rest) = self.placements.drain(..).partition(|p| matches!(p.anchor, Anchor::Line(_)));
        self.placements = rest;
        primary
    }

    /// The scrollback dropped every line above `line_base`: placements whose last row was among them go.
    pub fn trim(&mut self, line_base: u64) -> bool {
        let before = self.placements.len();
        self.placements.retain(|placement| match placement.anchor {
            Anchor::Line(id) => id.index + u64::from(placement.span[1]) > line_base,
            Anchor::Screen(_) => true,
        });
        self.placements.len() != before
    }

    /// Text was written over `cells` (rows counted as `rows_of` does, columns `start..end`): the Sixels under it go, or
    /// with `all` every picture there.
    pub fn erase(&mut self, cells: &[(i64, usize, usize)], alt: bool, epoch: u32, all: bool) -> bool {
        self.remove_placements(
            |placement, _| {
                let Some((top, bottom)) = rows_of(placement, alt, epoch).filter(|_| all || placement.erasable) else { return false };
                let (left, right) = (placement.column, placement.column + placement.span[0] as usize);
                cells.iter().any(|&(row, start, end)| (top..bottom).contains(&row) && start < right && left < end)
            },
            false,
        )
    }

    /// Drops the Sixels a new picture at `new` covers whole, on the same screen: a program that draws a picture
    /// where another one was meant to replace it.
    pub fn drop_covered_sixels(&mut self, new: &Placement, alt: bool, epoch: u32) {
        let Some((top, bottom)) = rows_of(new, alt, epoch) else { return };
        let (left, right) = (new.column, new.column + new.span[0] as usize);
        self.remove_placements(
            |old, _| {
                old.erasable
                    && rows_of(old, alt, epoch).is_some_and(|(t, b)| top <= t && b <= bottom)
                    && left <= old.column
                    && old.column + old.span[0] as usize <= right
            },
            false,
        );
    }

    /// The rows of a placement relative to grid line 0 of the live screen, `top..bottom`: `screen_top` is the
    /// `LineId::index` of that line (the alternate screen has none).
    pub fn screen_rows(placement: &Placement, alt: bool, epoch: u32, screen_top: i64) -> Option<(i64, i64)> {
        rows_of(placement, alt, epoch).map(|(top, bottom)| if alt { (top, bottom) } else { (top - screen_top, bottom - screen_top) })
    }

    /// The placements in a view of `rows` rows whose top row is `top` (None on the alternate screen), with their
    /// pictures, oldest first.
    pub fn visible(&self, top: Option<LineId>, alt: bool, rows: usize, out: &mut Vec<SnapImage>) {
        let (epoch, first) = top.map_or((0, 0), |top| (top.epoch, top.index as i64));
        for placement in &self.placements {
            let Some((start, end)) = rows_of(placement, alt, epoch) else { continue };
            if !alt && top.is_none() {
                continue;
            }
            let (start, end) = (start - first, end - first);
            if end <= 0 || start >= rows as i64 {
                continue;
            }
            let Some(image) = self.image(placement.image) else { continue };
            out.push(SnapImage {
                key: image.key,
                width: image.width,
                height: image.height,
                pixels: image.pixels.clone(),
                src: placement.src,
                row: start as i32,
                column: placement.column as i32,
                offset: placement.offset,
                cells: placement.cells,
                z: placement.z,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(index: u64) -> Anchor {
        Anchor::Line(LineId { epoch: 0, index })
    }

    fn placement(image: u64, anchor: Anchor, span: [u32; 2], erasable: bool) -> Placement {
        Placement { image, id: 0, anchor, column: 2, offset: [0; 2], src: [0, 0, 1, 1], cells: None, span, z: 0, erasable }
    }

    #[test]
    fn a_transmission_with_the_same_id_replaces_the_picture_and_its_placements() {
        let mut store = ImageStore::default();
        let first = store.insert(7, 0, 1, 1, vec![0; 4]);
        store.place(placement(first, line(0), [1, 1], false));
        let second = store.insert(7, 0, 1, 1, vec![1; 4]);
        assert_ne!(first, second);
        assert!(store.image(first).is_none());
        assert!(store.placements().is_empty());
        assert_eq!(store.bytes(), 4);
        assert_eq!(store.by_id(7).map(|image| image.key), Some(second));
    }

    #[test]
    fn the_pane_budget_drops_the_oldest_pictures_first() {
        let mut store = ImageStore::default();
        let side = MAX_IMAGE_SIDE;
        let keys: Vec<u64> = (1..=5).map(|id| store.insert(id, 0, side, side, vec![0; MAX_IMAGE_BYTES])).collect();
        assert_eq!(store.bytes(), 4 * MAX_IMAGE_BYTES, "four fit in 256 MiB");
        assert!(store.image(keys[0]).is_none());
        assert!(store.image(keys[4]).is_some());
    }

    #[test]
    fn anonymous_pictures_live_only_while_placed() {
        let mut store = ImageStore::default();
        let sixel = store.insert(0, 0, 1, 1, vec![0; 4]);
        store.place(placement(sixel, line(3), [1, 1], true));
        let kept = store.insert(9, 0, 1, 1, vec![0; 4]);
        assert!(store.image(sixel).is_some(), "placed");
        assert!(store.erase(&[(3, 0, 80)], false, 0, false));
        store.sweep();
        assert!(store.image(sixel).is_none(), "unplaced and nameless");
        assert!(store.image(kept).is_some(), "an image with an id waits to be placed");
    }

    #[test]
    fn text_over_a_sixel_erases_it_and_text_beside_it_does_not() {
        let mut store = ImageStore::default();
        let key = store.insert(0, 0, 1, 1, vec![0; 4]);
        store.place(placement(key, line(10), [3, 2], true));
        assert!(!store.erase(&[(9, 0, 80), (12, 0, 80), (10, 0, 2), (11, 5, 9)], false, 0, false), "above, below, left, right");
        assert!(!store.erase(&[(10, 2, 5)], true, 0, false), "a write on the other screen");
        assert!(!store.erase(&[(10, 2, 5)], false, 1, false), "a write in another epoch");
        assert!(store.erase(&[(11, 4, 5)], false, 0, false));
        let kitty = store.insert(1, 0, 1, 1, vec![0; 4]);
        store.place(placement(kitty, line(10), [3, 2], false));
        assert!(!store.erase(&[(10, 0, 80)], false, 0, false), "kitty placements stay under text");
        assert!(store.erase(&[(11, 0, usize::MAX)], false, 0, true), "and go with a cleared row");
    }

    #[test]
    fn placements_go_when_the_scrollback_drops_their_last_row() {
        let mut store = ImageStore::default();
        let key = store.insert(1, 0, 1, 1, vec![0; 4]);
        store.place(placement(key, line(5), [1, 3], false));
        assert!(!store.trim(7), "row 7 is still there");
        assert!(store.trim(8));
    }

    #[test]
    fn the_view_gets_the_placements_that_reach_into_it_with_rows_relative_to_its_top() {
        let mut store = ImageStore::default();
        let key = store.insert(1, 0, 1, 1, vec![0; 4]);
        store.place(placement(key, line(98), [1, 4], false)); // rows 98..102
        store.place(placement(key, line(110), [1, 1], false)); // below a 10-row view at 100
        store.place(placement(key, Anchor::Screen(0), [1, 1], false)); // alternate screen
        let mut out = Vec::new();
        store.visible(Some(LineId { epoch: 0, index: 100 }), false, 10, &mut out);
        assert_eq!(out.iter().map(|image| image.row).collect::<Vec<_>>(), [-2]);
        out.clear();
        store.visible(None, true, 10, &mut out);
        assert_eq!(out.iter().map(|image| image.row).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn a_placement_id_is_replaced_and_the_count_is_capped() {
        let mut store = ImageStore::default();
        let key = store.insert(1, 0, 1, 1, vec![0; 4]);
        store.place(Placement { id: 4, ..placement(key, line(0), [1, 1], false) });
        store.place(Placement { id: 4, ..placement(key, line(9), [1, 1], false) });
        assert_eq!(store.placements().len(), 1);
        assert_eq!(store.placements()[0].anchor, line(9));
        for row in 0..MAX_PLACEMENTS as u64 + 10 {
            store.place(placement(key, line(row), [1, 1], false));
        }
        assert_eq!(store.placements().len(), MAX_PLACEMENTS);
    }
}
