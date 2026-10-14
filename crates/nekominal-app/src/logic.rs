//! Toolkit-free decisions of the shell: tab titles, tab drag targets and the pixel geometry of split panes.
//! Kept apart from the WinUI code so they can be tested without a window.

use nekominal_model::layout::{Axis, Rect, Splitter};
use nekominal_model::settings::OPACITY_RANGE;

/// A pixel rectangle on the physical pixel grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Thickness of the space between two panes, in physical pixels (1 DIP, never less than one pixel).
pub fn divider_px(scale: f64) -> i32 {
    (scale.round() as i32).max(1)
}

/// Pixel rectangle of a pane whose share of the tab area is `rect`. Edges are rounded on the pixel grid, so
/// neighbours tile exactly; every edge shared with another pane gives up half of the divider.
pub fn pane_px_rect(rect: Rect, area_px: (i32, i32), divider: i32) -> PxRect {
    const EDGE: f64 = 1e-6;
    let edge = |fraction: f64, total: i32| (fraction * f64::from(total)).round() as i32;
    let (left, right) = (edge(rect.x, area_px.0), edge(rect.x + rect.w, area_px.0));
    let (top, bottom) = (edge(rect.y, area_px.1), edge(rect.y + rect.h, area_px.1));
    let before = divider / 2;
    let after = divider - before;
    let x0 = left + if rect.x > EDGE { after } else { 0 };
    let x1 = right - if rect.x + rect.w < 1.0 - EDGE { before } else { 0 };
    let y0 = top + if rect.y > EDGE { after } else { 0 };
    let y1 = bottom - if rect.y + rect.h < 1.0 - EDGE { before } else { 0 };
    PxRect { x: x0, y: y0, w: (x1 - x0).max(1), h: (y1 - y0).max(1) }
}

/// The divider between a split's two children: the gap `pane_px_rect` leaves, over the extent of the split.
pub fn splitter_px_rect(splitter: &Splitter, area_px: (i32, i32), divider: i32) -> PxRect {
    let edge = |fraction: f64, total: i32| (fraction * f64::from(total)).round() as i32;
    let before = divider / 2;
    let (x0, x1) = (edge(splitter.rect.x, area_px.0), edge(splitter.rect.x + splitter.rect.w, area_px.0));
    let (y0, y1) = (edge(splitter.rect.y, area_px.1), edge(splitter.rect.y + splitter.rect.h, area_px.1));
    match splitter.axis {
        Axis::Row => {
            let at = edge(splitter.rect.x + splitter.rect.w * splitter.ratio, area_px.0) - before;
            PxRect { x: at, y: y0, w: divider, h: y1 - y0 }
        }
        Axis::Column => {
            let at = edge(splitter.rect.y + splitter.rect.h * splitter.ratio, area_px.1) - before;
            PxRect { x: x0, y: at, w: x1 - x0, h: divider }
        }
    }
}

/// The ratio a split takes when its divider is dragged to `position` (a fraction of the whole tab area along
/// the split's axis). The model clamps it; this only converts the coordinate system.
pub fn ratio_at(splitter: &Splitter, position: f64) -> f64 {
    match splitter.axis {
        Axis::Row => (position - splitter.rect.x) / splitter.rect.w,
        Axis::Column => (position - splitter.rect.y) / splitter.rect.h,
    }
}

/// Index a dragged tab would take. `widths` are the chips in their current order (without spacing),
/// `gap` the spacing between them, `from` the dragged chip and `center` where its middle currently is,
/// measured from the left edge of the strip.
pub fn reorder_target(widths: &[f64], gap: f64, from: usize, center: f64) -> usize {
    let mut left = 0.0;
    let centers: Vec<f64> = widths
        .iter()
        .map(|width| {
            let middle = left + width / 2.0;
            left += width + gap;
            middle
        })
        .collect();
    let mut target = from;
    for (index, middle) in centers.iter().enumerate().skip(from + 1) {
        if center > *middle {
            target = index;
        }
    }
    for (index, middle) in centers.iter().enumerate().take(from).rev() {
        if center < *middle {
            target = index;
        }
    }
    target
}

/// An unselected chip is never narrower than this, in DIP.
pub const CHIP_MIN_WIDTH: f64 = 48.0;
/// No chip is wider than this.
pub const CHIP_MAX_WIDTH: f64 = 216.0;
/// The selected chip keeps at least this much, so its title and close glyph stay usable.
pub const SELECTED_MIN_WIDTH: f64 = 100.0;
/// A chip narrower than this shows its number instead of its title.
pub const ORDINAL_BELOW: f64 = 64.0;
/// Below this strip width the position plate shows only the total.
pub const PLATE_TOTAL_ONLY_BELOW: f64 = 300.0;
/// Room kept around a chip that is scrolled into view.
pub const REVEAL_MARGIN: f64 = 12.0;
/// Width of what covers a cut edge of the strip: the block that counts the hidden chips (32) and its fade (16).
pub const COUNT_BLOCK_WIDTH: f64 = 48.0;

/// The width of every chip of the strip. `preferred` is what each chip wants (its title and chrome), `selected`
/// the index of the selected one, `available` the width of the strip and `gap` the space between chips.
///
/// The selected chip keeps its preferred width, clamped to [`SELECTED_MIN_WIDTH`]..[`CHIP_MAX_WIDTH`]. When
/// everything fits, every chip keeps its preferred width. Otherwise the others share what is left by water-filling:
/// a common level that short titles stay under (they are never enlarged) down to [`CHIP_MIN_WIDTH`]. Below that
/// floor the sum exceeds `available` and the strip scrolls.
pub fn tab_widths(preferred: &[f64], selected: usize, available: f64, gap: f64) -> Vec<f64> {
    let wanted: Vec<f64> = preferred
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let floor = if index == selected { SELECTED_MIN_WIDTH } else { CHIP_MIN_WIDTH };
            width.clamp(floor, CHIP_MAX_WIDTH)
        })
        .collect();
    let usable = available - gap * preferred.len().saturating_sub(1) as f64;
    if wanted.iter().sum::<f64>() <= usable {
        return wanted;
    }
    let mut widths = wanted.clone();
    let mut others: Vec<usize> = (0..wanted.len()).filter(|index| *index != selected).collect();
    others.sort_by(|a, b| wanted[*a].total_cmp(&wanted[*b]));
    let mut remaining = (usable - wanted.get(selected).copied().unwrap_or(0.0)).max(0.0);
    for (position, index) in others.iter().enumerate() {
        let level = remaining / (others.len() - position) as f64;
        if wanted[*index] <= level {
            remaining -= wanted[*index];
        } else {
            // This chip and every longer one get the level, but never less than the floor.
            for rest in &others[position..] {
                widths[*rest] = level.max(CHIP_MIN_WIDTH).min(wanted[*rest]);
            }
            break;
        }
    }
    widths
}

/// Whether a chip of this width shows its number instead of its title.
pub fn shows_ordinal(width: f64) -> bool {
    width < ORDINAL_BELOW
}

/// Width of the whole strip: the chips, the gaps between them and `padding` on both sides.
pub fn strip_extent(widths: &[f64], gap: f64, padding: f64) -> f64 {
    widths.iter().sum::<f64>() + gap * widths.len().saturating_sub(1) as f64 + padding
}

/// How many chips lie completely outside the visible part of the strip, to the left and to the right. A chip that
/// shows even partly counts as visible, so hidden plus visible chips are all of them.
pub fn hidden_counts(widths: &[f64], gap: f64, padding: f64, offset: f64, viewport: f64) -> (usize, usize) {
    let (mut left, mut right) = (0, 0);
    let mut edge = padding / 2.0;
    for width in widths {
        if edge + width <= offset {
            left += 1;
        } else if edge >= offset + viewport {
            right += 1;
        }
        edge += width + gap;
    }
    (left, right)
}

/// The scroll offset that shows the chip at `left` (strip coordinates) of `width`, with `margin` around it. An
/// edge with chips behind it is covered by a count block of `block` DIP, so the margin starts behind the block. The
/// offset stays as it is when the chip is already in view.
pub fn reveal_offset(
    (left, width): (f64, f64),
    offset: f64,
    viewport: f64,
    extent: f64,
    margin: f64,
    block: f64,
) -> f64 {
    let max_offset = (extent - viewport).max(0.0);
    let offset = offset.clamp(0.0, max_offset);
    let inset_left = margin + if offset > 0.0 { block } else { 0.0 };
    let inset_right = margin + if offset < max_offset { block } else { 0.0 };
    if left < offset + inset_left {
        let target = left - margin - block;
        return if target <= 0.0 { 0.0 } else { target.min(max_offset) };
    }
    if left + width > offset + viewport - inset_right {
        let target = left + width + margin + block - viewport;
        return if target >= max_offset { max_offset } else { target.max(0.0) };
    }
    offset
}

/// What the position plate says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plate {
    /// `current / total`, both one-based.
    Position { current: usize, total: usize },
    /// A narrow strip keeps only the count.
    Total(usize),
}

/// The plate for `total` tabs with the zero-based `selected` one: none for a single tab, the total alone when the
/// strip is narrower than [`PLATE_TOTAL_ONLY_BELOW`].
pub fn plate(selected: usize, total: usize, strip_width: f64) -> Option<Plate> {
    if total < 2 {
        None
    } else if strip_width < PLATE_TOTAL_ONLY_BELOW {
        Some(Plate::Total(total))
    } else {
        Some(Plate::Position { current: selected + 1, total })
    }
}

/// Tabs in most-recently-used order, and the walk through that order while the keys of next/previous tab are
/// held: every press steps through the order as it was before the first press, and the tab shown when the keys
/// come up becomes the most recent.
#[derive(Debug)]
pub struct Mru<T> {
    order: Vec<T>,
    walk: Option<(Vec<T>, usize)>,
}

impl<T: Copy + PartialEq> Default for Mru<T> {
    fn default() -> Self {
        Mru { order: Vec::new(), walk: None }
    }
}

impl<T: Copy + PartialEq> Mru<T> {
    /// `tab` was selected some other way than by the keys: it becomes the most recent and any walk ends.
    pub fn touch(&mut self, tab: T) {
        self.order.retain(|other| *other != tab);
        self.order.insert(0, tab);
        self.walk = None;
    }

    /// A closed tab leaves the order; the walk ends.
    pub fn remove(&mut self, tab: T) {
        self.order.retain(|other| *other != tab);
        self.walk = None;
    }

    pub fn most_recent(&self) -> Option<T> {
        self.order.first().copied()
    }

    /// One press of next (`step` 1) or previous (`step` -1): the tab to show, or `None` with fewer than two tabs.
    pub fn step(&mut self, step: isize) -> Option<T> {
        if self.order.len() < 2 {
            return None;
        }
        let (snapshot, position) = self.walk.get_or_insert_with(|| (self.order.clone(), 0));
        let count = snapshot.len() as isize;
        *position = (*position as isize + step).rem_euclid(count) as usize;
        Some(snapshot[*position])
    }

    /// The keys came up: the tab shown last becomes the most recent.
    pub fn commit(&mut self) {
        if let Some((snapshot, position)) = self.walk.take() {
            self.touch(snapshot[position]);
        }
    }

    #[cfg(test)]
    pub fn is_walking(&self) -> bool {
        self.walk.is_some()
    }
}

/// The last segment of a working directory as a tab shows it: `~` for the home folder, the drive (`C:`) for the
/// root of a drive, `/` for the root of a POSIX path (a WSL shell reports those), otherwise the folder name.
pub fn folder_label(path: &str, home: Option<&str>) -> String {
    let normal = |text: &str| text.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    let trimmed = path.trim_end_matches(['\\', '/']);
    if home.is_some_and(|home| !normal(home).is_empty() && normal(home) == normal(trimmed)) {
        return "~".to_owned();
    }
    if trimmed.is_empty() {
        return if path.is_empty() { String::new() } else { "/".to_owned() };
    }
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).to_owned()
}

/// The background opacity the sliders and commands asked for, kept until the next frame: a drag raises many value
/// changes per frame and only the last of them is worth applying.
#[derive(Debug, Default)]
pub struct PendingOpacity(Option<f64>);

impl PendingOpacity {
    /// Records a request, rounded to a whole percent and kept inside the valid range.
    pub fn request(&mut self, opacity: f64) {
        self.0 = Some(((opacity * 100.0).round() / 100.0).clamp(OPACITY_RANGE.0, OPACITY_RANGE.1));
    }

    /// What the controls show: the latest request, or `applied` when nothing waits.
    pub fn latest(&self, applied: f64) -> f64 {
        self.0.unwrap_or(applied)
    }

    /// The opacity to apply in this frame: the request, unless it is what `applied` already is.
    pub fn take(&mut self, applied: f64) -> Option<f64> {
        self.0.take().filter(|opacity| (opacity - applied).abs() > 0.001)
    }
}

/// The text a tab shows for the title a program set. Shells report the path of their own executable until
/// the first prompt (`C:\Program Files\PowerShell\7\pwsh.exe`): that becomes the program name. An empty
/// title falls back to `fallback`.
pub fn tab_title(raw: &str, fallback: &str) -> String {
    let title = raw.trim();
    let program = if let Some(quoted) = title.strip_prefix('"') {
        quoted.split('"').next().unwrap_or_default()
    } else {
        let lower = title.to_ascii_lowercase();
        lower.find(".exe").map_or(title, |end| &title[..end + 4])
    };
    let looks_like_executable = program.to_ascii_lowercase().ends_with(".exe")
        && (program.contains('\\') || program.contains('/') || program == title);
    let name = if looks_like_executable {
        let file = program.rsplit(['\\', '/']).next().unwrap_or(program);
        file.get(..file.len() - 4).unwrap_or(file)
    } else {
        title
    };
    if name.is_empty() { fallback.to_owned() } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use nekominal_model::layout::{Axis, Node, UNIT};

    use super::*;

    #[test]
    fn executable_paths_become_program_names() {
        assert_eq!(tab_title(r"C:\Program Files\PowerShell\7\pwsh.exe", "x"), "pwsh");
        assert_eq!(tab_title(r#""C:\Windows\System32\cmd.exe" /k echo"#, "x"), "cmd");
        assert_eq!(tab_title(r"C:\WINDOWS\system32\cmd.exe", "x"), "cmd");
        assert_eq!(tab_title("wsl.exe", "x"), "wsl");
    }

    #[test]
    fn titles_a_program_chose_are_kept() {
        assert_eq!(tab_title("  claude - Nekominal  ", "x"), "claude - Nekominal");
        assert_eq!(tab_title("vim main.rs", "x"), "vim main.rs");
        assert_eq!(tab_title(r"C:\Users\me\src", "x"), r"C:\Users\me\src");
    }

    #[test]
    fn an_empty_title_uses_the_fallback() {
        assert_eq!(tab_title("", "PowerShell"), "PowerShell");
        assert_eq!(tab_title("   ", "PowerShell"), "PowerShell");
    }

    #[test]
    fn dragging_a_tab_past_the_middle_of_a_neighbour_swaps_them() {
        let widths = [100.0, 100.0, 100.0];
        // Chip 0 has its middle at 50, chip 1 at 154, chip 2 at 258.
        assert_eq!(reorder_target(&widths, 4.0, 0, 60.0), 0);
        assert_eq!(reorder_target(&widths, 4.0, 0, 160.0), 1);
        assert_eq!(reorder_target(&widths, 4.0, 0, 300.0), 2);
        assert_eq!(reorder_target(&widths, 4.0, 2, 100.0), 1);
        assert_eq!(reorder_target(&widths, 4.0, 2, -40.0), 0);
        assert_eq!(reorder_target(&widths, 4.0, 1, 154.0), 1);
    }

    /// A small deterministic generator, so the property tests need no dependency and fail the same way twice.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn between(&mut self, low: f64, high: f64) -> f64 {
            low + (self.next() % 10_000) as f64 / 10_000.0 * (high - low)
        }

        fn below(&mut self, count: usize) -> usize {
            (self.next() % count as u64) as usize
        }
    }

    const GAP: f64 = 4.0;

    fn sum_with_gaps(widths: &[f64]) -> f64 {
        widths.iter().sum::<f64>() + GAP * widths.len().saturating_sub(1) as f64
    }

    #[test]
    fn five_long_titled_tabs_share_a_653_dip_strip_and_twelve_hit_the_floor() {
        let five = tab_widths(&[216.0; 5], 1, 649.0, GAP);
        assert_eq!(five[1], 216.0, "the selected chip keeps its width");
        for index in [0, 2, 3, 4] {
            assert!((five[index] - 104.25).abs() < 1e-9, "{five:?}");
        }
        assert!((sum_with_gaps(&five) - 649.0).abs() < 1e-9, "the strip is used exactly");

        let twelve = tab_widths(&[216.0; 12], 0, 649.0, GAP);
        assert_eq!(twelve[0], 216.0);
        assert!(twelve[1..].iter().all(|width| *width == CHIP_MIN_WIDTH), "{twelve:?}");
        assert!(sum_with_gaps(&twelve) > 649.0, "more than fits: the strip scrolls");
    }

    #[test]
    fn short_titles_are_never_enlarged_and_what_they_leave_goes_to_the_long_ones() {
        // Selected 216, one short (60) and two long (216): the short one keeps 60, the long ones split the rest.
        let widths = tab_widths(&[216.0, 60.0, 216.0, 216.0], 0, 500.0, GAP);
        assert_eq!(widths[1], 60.0);
        assert!((widths[2] - 106.0).abs() < 1e-9 && (widths[3] - 106.0).abs() < 1e-9, "{widths:?}");
        // With room for everyone nothing changes, except that the selected chip is at least 100 wide.
        assert_eq!(tab_widths(&[80.0, 80.0, 30.0], 0, 800.0, GAP), [100.0, 80.0, 48.0]);
        assert_eq!(tab_widths(&[400.0], 0, 800.0, GAP), [216.0]);
        assert_eq!(tab_widths(&[], 0, 800.0, GAP), Vec::<f64>::new());
    }

    #[test]
    fn tab_widths_hold_their_invariants_for_any_tabs_and_any_strip() {
        let mut random = Lcg(7);
        for _ in 0..3000 {
            let count = 1 + random.below(40);
            let preferred: Vec<f64> = (0..count).map(|_| random.between(20.0, 300.0)).collect();
            let selected = random.below(count);
            let available = random.between(100.0, 2000.0);
            let widths = tab_widths(&preferred, selected, available, GAP);

            assert_eq!(widths.len(), count);
            for (index, width) in widths.iter().enumerate() {
                let (floor, wanted) = if index == selected {
                    (SELECTED_MIN_WIDTH, preferred[index].clamp(SELECTED_MIN_WIDTH, CHIP_MAX_WIDTH))
                } else {
                    (CHIP_MIN_WIDTH, preferred[index].clamp(CHIP_MIN_WIDTH, CHIP_MAX_WIDTH))
                };
                assert!(*width >= floor - 1e-9 && *width <= wanted + 1e-9, "{index}: {width} not in {floor}..{wanted}");
            }
            assert_eq!(widths[selected], preferred[selected].clamp(SELECTED_MIN_WIDTH, CHIP_MAX_WIDTH));
            // Either everything fits, or every unselected chip is at the floor (and the strip scrolls).
            let fits = sum_with_gaps(&widths) <= available + 1e-6;
            let at_floor = widths.iter().enumerate().all(|(i, w)| i == selected || *w <= CHIP_MIN_WIDTH + 1e-9);
            assert!(fits || at_floor, "{widths:?} in {available}");

            // More room never narrows a chip.
            let wider = tab_widths(&preferred, selected, available + random.between(0.0, 500.0), GAP);
            for (before, after) in widths.iter().zip(&wider) {
                assert!(*after >= *before - 1e-9, "{before} became {after}");
            }

            // Another tab, with the selection where it was, never widens the others.
            let mut more = preferred.clone();
            more.push(random.between(20.0, 300.0));
            let with_new = tab_widths(&more, selected, available, GAP);
            for (before, after) in widths.iter().zip(&with_new) {
                assert!(*after <= *before + 1e-9, "{before} became {after}");
            }
        }
    }

    #[test]
    fn a_chip_narrower_than_64_dip_shows_its_number() {
        assert!(shows_ordinal(48.0) && shows_ordinal(63.9));
        assert!(!shows_ordinal(64.0) && !shows_ordinal(216.0));
    }

    #[test]
    fn hidden_and_visible_chips_add_up_to_all_of_them() {
        let widths = [100.0; 12];
        let extent = strip_extent(&widths, GAP, 4.0);
        assert_eq!(extent, 12.0 * 100.0 + 11.0 * GAP + 4.0);
        let viewport = 500.0;
        for offset in [0.0, 1.0, 104.0, 300.0, 520.0, extent - viewport] {
            let (left, right) = hidden_counts(&widths, GAP, 4.0, offset, viewport);
            let visible = widths
                .iter()
                .enumerate()
                .filter(|(i, w)| {
                    let edge = 2.0 + *i as f64 * (100.0 + GAP);
                    edge + **w > offset && edge < offset + viewport
                })
                .count();
            assert_eq!(left + visible + right, widths.len(), "offset {offset}");
        }
        assert_eq!(hidden_counts(&widths, GAP, 4.0, 0.0, viewport), (0, 7));
        assert_eq!(hidden_counts(&widths, GAP, 4.0, extent - viewport, viewport), (7, 0));
        // A chip cut by the edge still shows.
        assert_eq!(hidden_counts(&widths, GAP, 4.0, 50.0, viewport).0, 0);
    }

    #[test]
    fn revealing_a_chip_leaves_a_margin_and_makes_room_for_the_count_blocks() {
        let (viewport, extent) = (500.0, 1304.0);
        let reveal =
            |left, offset| reveal_offset((left, 100.0), offset, viewport, extent, REVEAL_MARGIN, COUNT_BLOCK_WIDTH);
        // Already in view: nothing moves.
        assert_eq!(reveal(200.0, 0.0), 0.0);
        assert_eq!(reveal(300.0, 100.0), 100.0);
        // The first chip of the strip stays flush at the left edge.
        assert_eq!(reveal(2.0, 300.0), 0.0);
        // A chip to the right scrolls in with the margin and the block behind it.
        assert_eq!(reveal(600.0, 0.0), 600.0 + 100.0 + REVEAL_MARGIN + COUNT_BLOCK_WIDTH - 500.0);
        // The last chip scrolls to the end, with the margin only.
        assert_eq!(reveal(1202.0, 0.0), extent - viewport);
        // A chip to the left of the view scrolls in from the left.
        assert_eq!(reveal(300.0, 700.0), 300.0 - REVEAL_MARGIN - COUNT_BLOCK_WIDTH);
        // A strip that fits has nothing to scroll.
        assert_eq!(reveal_offset((300.0, 100.0), 0.0, 500.0, 400.0, REVEAL_MARGIN, COUNT_BLOCK_WIDTH), 0.0);
    }

    #[test]
    fn a_revealed_chip_is_in_view_inside_its_margins_for_any_strip() {
        let mut random = Lcg(11);
        for _ in 0..3000 {
            let count = 2 + random.below(40);
            let widths: Vec<f64> = (0..count).map(|_| random.between(CHIP_MIN_WIDTH, CHIP_MAX_WIDTH)).collect();
            let extent = strip_extent(&widths, GAP, 4.0);
            // Wide enough for the widest chip between two count blocks and their margins.
            let viewport = random.between(400.0, 900.0);
            let index = random.below(count);
            let left = 2.0 + widths[..index].iter().sum::<f64>() + GAP * index as f64;
            let offset = random.between(0.0, (extent - viewport).max(0.0) + 50.0);
            let at = reveal_offset((left, widths[index]), offset, viewport, extent, REVEAL_MARGIN, COUNT_BLOCK_WIDTH);
            assert!(at >= 0.0 && at <= (extent - viewport).max(0.0) + 1e-9, "offset {at}");
            assert!(left >= at - 1e-9 && left + widths[index] <= at + viewport + 1e-9, "the chip is cut off");
            if extent > viewport {
                // Every scroll position has the chip clear of the blocks, unless it sits at an end of the strip.
                let margin_left = if at > 0.0 { REVEAL_MARGIN + COUNT_BLOCK_WIDTH } else { 0.0 };
                assert!(left >= at + margin_left - 1e-9 || index == 0, "left margin at {at}");
                let at_end = at + viewport >= extent - 1e-9;
                let margin_right = if at_end { 0.0 } else { REVEAL_MARGIN + COUNT_BLOCK_WIDTH };
                assert!(
                    left + widths[index] <= at + viewport - margin_right + 1e-9 || index == count - 1,
                    "right margin"
                );
            }
        }
    }

    #[test]
    fn the_plate_is_hidden_for_one_tab_and_total_only_on_a_narrow_strip() {
        assert_eq!(plate(0, 0, 800.0), None);
        assert_eq!(plate(0, 1, 800.0), None);
        assert_eq!(plate(2, 12, 800.0), Some(Plate::Position { current: 3, total: 12 }));
        assert_eq!(plate(1, 2, 300.0), Some(Plate::Position { current: 2, total: 2 }));
        assert_eq!(plate(2, 12, 299.9), Some(Plate::Total(12)));
    }

    #[test]
    fn next_tab_walks_the_most_recently_used_order_while_the_keys_are_held() {
        let mut mru = Mru::default();
        for tab in ['a', 'b', 'c'] {
            mru.touch(tab);
        }
        // Select A, B, C in turn: the order is C, B, A. One tap goes to B.
        assert_eq!(mru.step(1), Some('b'));
        mru.commit();
        assert!(!mru.is_walking());
        // Held: two more presses go to C and then A, and A is the most recent when the keys come up.
        assert_eq!(mru.step(1), Some('c'));
        assert_eq!(mru.step(1), Some('a'));
        mru.commit();
        assert_eq!(mru.most_recent(), Some('a'));
        // The order after that: A, B, C (B was used after C).
        assert_eq!(mru.step(1), Some('b'));
        assert_eq!(mru.step(1), Some('c'));
        assert_eq!(mru.step(1), Some('a'), "the walk wraps around");
        mru.commit();
        assert_eq!(mru.most_recent(), Some('a'));
    }

    #[test]
    fn previous_tab_walks_backwards_and_a_tap_toggles_between_two_tabs() {
        let mut mru = Mru::default();
        for tab in [1, 2, 3, 4] {
            mru.touch(tab);
        }
        assert_eq!(mru.step(-1), Some(1), "previous starts from the least recently used");
        mru.commit();
        assert_eq!(mru.most_recent(), Some(1));
        // Tapping twice with the keys released between returns to where it began.
        let mut pair = Mru::default();
        pair.touch('x');
        pair.touch('y');
        assert_eq!(pair.step(1), Some('x'));
        pair.commit();
        assert_eq!(pair.step(1), Some('y'));
        pair.commit();
        assert_eq!(pair.most_recent(), Some('y'));
    }

    #[test]
    fn selecting_a_tab_another_way_ends_the_walk_and_a_closed_tab_leaves_the_order() {
        let mut mru = Mru::default();
        for tab in [1, 2, 3] {
            mru.touch(tab);
        }
        assert_eq!(mru.step(1), Some(2));
        mru.touch(1);
        assert!(!mru.is_walking());
        assert_eq!(mru.most_recent(), Some(1));
        // Closing the most recent tab leaves the one used before it.
        mru.remove(1);
        assert_eq!(mru.most_recent(), Some(3));
        mru.remove(2);
        mru.remove(3);
        assert_eq!(mru.most_recent(), None);
        assert_eq!(mru.step(1), None);
        let mut single = Mru::default();
        single.touch(9);
        assert_eq!(single.step(1), None, "nothing to walk to");
    }

    #[test]
    fn a_folder_is_shown_by_its_last_segment_with_home_and_drives_spelled_short() {
        let home = Some(r"C:\Users\me");
        assert_eq!(folder_label(r"C:\Users\me\src\nekominal", home), "nekominal");
        assert_eq!(folder_label(r"C:\Users\me\src\nekominal\", home), "nekominal");
        assert_eq!(folder_label(r"C:\Users\me", home), "~");
        assert_eq!(folder_label(r"c:\users\ME\", home), "~", "paths of Windows ignore case and a trailing slash");
        assert_eq!(folder_label("C:/Users/me", home), "~", "OSC 7 may report forward slashes");
        assert_eq!(folder_label(r"C:\Users", home), "Users");
        assert_eq!(folder_label(r"C:\", home), "C:");
        assert_eq!(folder_label("D:", home), "D:");
        assert_eq!(folder_label(r"\\server\share\docs", home), "docs");
        assert_eq!(folder_label("/home/me/project", home), "project");
        assert_eq!(folder_label("/", home), "/");
        assert_eq!(folder_label("", home), "");
        assert_eq!(folder_label(r"C:\Users\me", None), "me");
        assert_eq!(folder_label("", Some("")), "", "an unknown home is not every folder");
    }

    #[test]
    fn many_opacity_requests_in_a_frame_become_one_apply_of_the_last() {
        let mut pending = PendingOpacity::default();
        assert_eq!(pending.take(0.72), None, "nothing was asked for");
        for percent in 20..=100 {
            pending.request(f64::from(percent) / 100.0);
        }
        pending.request(0.5);
        assert_eq!(pending.latest(0.72), 0.5);
        assert_eq!(pending.take(0.72), Some(0.5));
        assert_eq!(pending.take(0.72), None, "a request is applied once");
        assert_eq!(pending.latest(0.72), 0.72);
    }

    #[test]
    fn an_opacity_request_equal_to_the_applied_one_applies_nothing() {
        let mut pending = PendingOpacity::default();
        pending.request(0.72);
        assert_eq!(pending.take(0.72), None);
        // The slider's own echo of a value it was given arrives as the same request.
        pending.request(72.0 / 100.0);
        assert_eq!(pending.take(0.72), None);
    }

    #[test]
    fn opacity_requests_are_whole_percent_inside_the_valid_range() {
        let mut pending = PendingOpacity::default();
        for (asked, applied) in [(0.724, 0.72), (0.726, 0.73), (7.0, 1.0), (0.0, 0.2), (-3.0, 0.2)] {
            pending.request(asked);
            assert_eq!(pending.take(0.9), Some(applied), "asked for {asked}");
        }
    }

    #[test]
    fn a_single_pane_fills_the_area_without_a_divider() {
        let rect = pane_px_rect(UNIT, (1000, 600), 2);
        assert_eq!(rect, PxRect { x: 0, y: 0, w: 1000, h: 600 });
    }

    #[test]
    fn neighbours_tile_the_area_and_leave_exactly_the_divider_between_them() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Row, 2).split_pane(2, Axis::Column, 3);
        let rects: Vec<PxRect> =
            tree.layout_rects(UNIT).into_iter().map(|(_, rect)| pane_px_rect(rect, (1001, 601), 3)).collect();
        // Pane 1 is the left half, 2 and 3 stack on the right.
        let (left, top_right, bottom_right) = (rects[0], rects[1], rects[2]);
        assert_eq!(left.x, 0);
        assert_eq!(top_right.x - (left.x + left.w), 3, "divider between left and right");
        assert_eq!(top_right.x, bottom_right.x);
        assert_eq!(top_right.x + top_right.w, 1001);
        assert_eq!(bottom_right.y - (top_right.y + top_right.h), 3, "divider between top and bottom");
        assert_eq!(bottom_right.y + bottom_right.h, 601);
    }

    #[test]
    fn the_divider_sits_in_the_gap_between_the_panes() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Row, 2);
        let rects = tree.layout_rects(UNIT);
        let (a, b) = (pane_px_rect(rects[0].1, (1000, 500), 3), pane_px_rect(rects[1].1, (1000, 500), 3));
        let divider = splitter_px_rect(&tree.splitters(UNIT)[0], (1000, 500), 3);
        assert_eq!(divider.x, a.x + a.w);
        assert_eq!(divider.x + divider.w, b.x);
        assert_eq!((divider.y, divider.h), (0, 500));
    }

    #[test]
    fn a_dragged_divider_converts_back_to_a_ratio() {
        let tree = Node::Leaf(1).split_pane(1, Axis::Column, 2).split_pane(2, Axis::Row, 3);
        let inner = &tree.splitters(UNIT)[1];
        assert_eq!(inner.axis, Axis::Row);
        // The inner split spans the lower half; a divider at 0.25 of the area is a quarter of that half.
        assert!((ratio_at(inner, 0.25) - 0.25).abs() < 1e-9);
        let outer = &tree.splitters(UNIT)[0];
        assert!((ratio_at(outer, 0.3) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn divider_is_one_dip_in_pixels() {
        assert_eq!(divider_px(1.0), 1);
        assert_eq!(divider_px(1.5), 2);
        assert_eq!(divider_px(2.0), 2);
        assert_eq!(divider_px(0.0), 1);
    }
}
