//! The pointer of one pane: the gestures of `MouseAssembler` applied to the emulator (selection, scrolling) and
//! reported to the program (mouse reports), without any toolkit.
//!
//! Who gets a gesture is the assembler's rule: while the program tracks the mouse it gets every press, drag and
//! wheel notch, and Shift hands the pointer to the terminal. What the terminal does with it is decided here:
//!
//! * a press starts a selection (cell, word on a double click, line on a triple click, block with Alt) in grid
//!   coordinates, so it follows its text when the view scrolls and when output pushes lines into the scrollback;
//! * Shift+click grows the selection that exists instead of starting another;
//! * a click that did not drag clears the selection; a double or triple click keeps its word or line;
//! * a drag held above or below the grid scrolls the view one step per [`Pointer::drag_scroll`].

use nekominal_input::{
    CellPoint, InputModes, Mods, MouseAssembler, MouseButton, PointerEvent, PointerGrid, PointerKind, PointerOutcome,
    SelectionKind as Gesture, Side, cell_at,
};
use nekominal_term::{CellSide, Link, SelectionKind, TerminalCore, alacritty_terminal::grid::Scroll};

/// Most lines one [`Pointer::drag_scroll`] moves, however far outside the grid the pointer is.
const MAX_DRAG_SCROLL: i32 = 4;

/// What the host has to do after an event, beyond what was done to the emulator.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PointerEffects {
    /// Bytes for the program, in order: mouse reports or the cursor keys of alternate scroll.
    pub reports: Vec<Vec<u8>>,
    /// A selection gesture ended with text selected: copy on select copies it now.
    pub selected: bool,
    /// Ctrl+click on a link that may be opened, released in the cell it was pressed in. What is opened is its checked
    /// `target`; its `uri` is the text as written, which for a path holds the `:line:column` to go to.
    pub open_link: Option<Link>,
}

/// Moves the view to the display offset `offset`, when it is not there.
fn scroll_to(core: &mut TerminalCore, offset: usize) {
    let delta = offset as i32 - core.display_offset() as i32;
    if delta != 0 {
        core.scroll_display(Scroll::Delta(delta));
    }
}

fn kind_of(gesture: Gesture) -> SelectionKind {
    match gesture {
        Gesture::Simple => SelectionKind::Simple,
        Gesture::Word => SelectionKind::Semantic,
        Gesture::Line => SelectionKind::Lines,
        Gesture::Block => SelectionKind::Block,
    }
}

fn side_of(side: Side) -> CellSide {
    match side {
        Side::Left => CellSide::Left,
        Side::Right => CellSide::Right,
    }
}

/// How the last press started a selection.
#[derive(Clone, Copy, Debug)]
struct Started {
    gesture: Gesture,
    /// It grew the selection that existed (Shift+click) instead of starting one.
    extended: bool,
}

#[derive(Debug, Default)]
pub struct Pointer {
    assembler: MouseAssembler,
    started: Option<Started>,
    /// Where the drag is held: the cell it is over (on the nearest row of the grid) and how many rows beyond the
    /// grid the pointer is, negative above and positive below; zero while it is inside.
    drag: Option<(CellPoint, i32)>,
    /// The left button went down on the pinned prompt line: the view went to its prompt and the release is swallowed.
    on_header: bool,
    /// The left button holds the thumb of the scrollbar, this many pixels below the top of the thumb.
    thumb_grab: Option<f32>,
    /// The find bar has matches: the scrollbar shows at the bottom too, and can be taken there.
    finding: bool,
}

/// The scrollbar as it is drawn (`nekominal_render::rows`), in pixels of the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bar {
    left: f32,
    track: f32,
    thumb_top: f32,
    thumb: f32,
    history: usize,
}

impl Bar {
    /// The scrollbar the pane shows now: none at the bottom without a find, or without a scrollback.
    fn of(core: &TerminalCore, grid: &PointerGrid, finding: bool) -> Option<Bar> {
        let (history, offset, rows) = (core.history_size(), core.display_offset(), usize::from(grid.rows));
        if history == 0 || (offset == 0 && !finding) {
            return None;
        }
        let total = (history + rows) as f32;
        let track = f32::from(grid.rows) * grid.cell_height;
        let width = (grid.cell_width * 0.6).round().max(4.0);
        let thumb = (rows as f32 / total * track).max(16.0).min(track);
        let top = (history - offset.min(history)) as f32 / total;
        let thumb_top = (top * track).clamp(0.0, track - thumb);
        Some(Bar { left: f32::from(grid.columns) * grid.cell_width - width, track, thumb_top, thumb, history })
    }

    /// The point is on the scrollbar; a few pixels to its left count, as it is thin.
    fn holds(&self, x: f32, y: f32) -> bool {
        x >= self.left - 3.0 && y >= 0.0 && y < self.track
    }

    /// The display offset that puts the top of the thumb at `top` pixels: the thumb's way down the track is the
    /// scrollback, from its oldest line at the top to the screen at the bottom.
    fn offset_at(&self, top: f32) -> usize {
        let travel = (self.track - self.thumb).max(1.0);
        let down = (top.clamp(0.0, travel) / travel * self.history as f32).round() as usize;
        self.history - down.min(self.history)
    }
}

impl Pointer {
    /// Feeds one event of the toolkit. The selection and the view change in `core`; the rest is returned.
    pub fn handle(
        &mut self,
        core: &mut TerminalCore,
        event: &PointerEvent,
        grid: &PointerGrid,
        modes: &InputModes,
    ) -> PointerEffects {
        let mut effects = PointerEffects::default();
        // The left button on the scrollbar takes its thumb, whoever has the mouse otherwise: a press on the track first
        // brings the middle of the thumb there. The thumb follows the pointer until the button goes up.
        match event.kind {
            PointerKind::Press(MouseButton::Left) => {
                if let Some(bar) = Bar::of(core, grid, self.finding).filter(|bar| bar.holds(event.x, event.y)) {
                    let on_thumb = event.y >= bar.thumb_top && event.y < bar.thumb_top + bar.thumb;
                    let grab = if on_thumb { event.y - bar.thumb_top } else { bar.thumb / 2.0 };
                    self.thumb_grab = Some(grab);
                    if !on_thumb {
                        scroll_to(core, bar.offset_at(event.y - grab));
                    }
                    return effects;
                }
            }
            PointerKind::Move => {
                if let Some(grab) = self.thumb_grab {
                    if let Some(bar) = Bar::of(core, grid, true) {
                        scroll_to(core, bar.offset_at(event.y - grab));
                    }
                    return effects;
                }
            }
            PointerKind::Release(MouseButton::Left) if self.thumb_grab.take().is_some() => return effects,
            PointerKind::Cancel => self.thumb_grab = None,
            _ => {}
        }
        // A plain left click on the pinned prompt line goes to that prompt; neither the press nor its release reaches the
        // assembler, so it neither selects nor reports. The pinned line hides the row under it, so nothing is lost there.
        match event.kind {
            PointerKind::Press(MouseButton::Left)
                if event.mods == Mods::NONE && event.y >= 0.0 && event.y < grid.cell_height && core.scroll_to_sticky_prompt() =>
            {
                self.on_header = true;
                return effects;
            }
            PointerKind::Release(MouseButton::Left) if std::mem::take(&mut self.on_header) => return effects,
            PointerKind::Cancel => self.on_header = false,
            _ => {}
        }
        // Only a press with Ctrl can be the start of a click on a link: the assembler asks nothing else.
        let event = match event.kind {
            PointerKind::Press(_) if event.mods.contains(Mods::CTRL) => {
                let (col, row) = cell_at(event.x, event.y, grid);
                PointerEvent { over_link: core.link_at(usize::from(row), usize::from(col)).is_some(), ..*event }
            }
            _ => *event,
        };
        for outcome in self.assembler.handle(&event, grid, modes) {
            match outcome {
                PointerOutcome::Report(bytes) => effects.reports.push(bytes),
                PointerOutcome::Scroll(lines) => core.scroll_display(Scroll::Delta(lines)),
                PointerOutcome::OpenLink { col, row } => {
                    effects.open_link = core.link_at(usize::from(row), usize::from(col));
                }
                PointerOutcome::SelectionStart { point, kind, extend } => {
                    let extended = extend && core.selection_active();
                    if extended {
                        core.selection_update(usize::from(point.row), usize::from(point.col), side_of(point.side));
                    } else {
                        core.selection_start(
                            kind_of(kind),
                            usize::from(point.row),
                            usize::from(point.col),
                            side_of(point.side),
                        );
                    }
                    self.started = Some(Started { gesture: kind, extended });
                    self.drag = Some((point, 0));
                }
                PointerOutcome::SelectionUpdate { point, overshoot_rows } => {
                    core.selection_update(usize::from(point.row), usize::from(point.col), side_of(point.side));
                    self.drag = Some((point, overshoot_rows));
                }
                PointerOutcome::SelectionEnd { point, dragged } => {
                    self.drag = None;
                    let started = self.started.take();
                    if dragged {
                        core.selection_update(usize::from(point.row), usize::from(point.col), side_of(point.side));
                    }
                    // A click that moved nothing selects nothing, unless it picked a word or a line or grew a selection.
                    let keeps = dragged
                        || started.is_some_and(|s| s.extended || matches!(s.gesture, Gesture::Word | Gesture::Line));
                    if !keeps {
                        core.selection_clear();
                    }
                    effects.selected = keeps && core.selection_text().is_some();
                }
            }
        }
        effects
    }

    /// One step of scrolling for a drag that is held above or below the grid: the view moves toward the older or
    /// newer lines and the selection grows to the edge row that is now showing others. True when it did.
    pub fn drag_scroll(&mut self, core: &mut TerminalCore) -> bool {
        let Some((point, overshoot)) = self.drag.filter(|(_, overshoot)| *overshoot != 0) else { return false };
        let lines = overshoot.abs().min(MAX_DRAG_SCROLL);
        core.scroll_display(Scroll::Delta(if overshoot < 0 { lines } else { -lines }));
        core.selection_update(usize::from(point.row), usize::from(point.col), side_of(point.side));
        true
    }

    /// The find bar has matches or not: with them the scrollbar shows, and takes the pointer, at the bottom too.
    pub fn set_finding(&mut self, finding: bool) {
        self.finding = finding;
    }

    /// The left button holds the thumb of the scrollbar.
    pub fn holds_thumb(&self) -> bool {
        self.thumb_grab.is_some()
    }

    /// The pointer left (capture lost, focus lost): every held button goes up and a drag ends where it was.
    pub fn cancel(&mut self, core: &mut TerminalCore, grid: &PointerGrid, modes: &InputModes) -> PointerEffects {
        self.handle(core, &PointerEvent::new(PointerKind::Cancel, 0.0, 0.0), grid, modes)
    }
}

#[cfg(test)]
mod tests {
    use nekominal_input::{MouseButton, Mods};
    use nekominal_term::CoreConfig;

    use super::*;
    use crate::host::input_modes_of;

    /// A 20 x 5 terminal with 10 x 20 pixel cells and a pointer on it.
    struct Rig {
        core: TerminalCore,
        pointer: Pointer,
        clock: u32,
    }

    const GRID: PointerGrid = PointerGrid { cell_width: 10.0, cell_height: 20.0, columns: 20, rows: 5 };

    /// The left half of a cell, which starts a selection at that cell; `right` is its right half.
    fn left(col: u16, row: u16) -> (f32, f32) {
        (f32::from(col) * 10.0 + 2.0, f32::from(row) * 20.0 + 5.0)
    }

    fn right(col: u16, row: u16) -> (f32, f32) {
        (f32::from(col) * 10.0 + 8.0, f32::from(row) * 20.0 + 5.0)
    }

    /// A rig with 45 lines written, most of them in the scrollback.
    fn scrolled_back() -> Rig {
        let text: String = (0..45).map(|i| format!("line{i:02}\r\n")).collect();
        Rig::new(&text)
    }

    /// On the scrollbar, which is the right 6 pixels of the 200 of the grid (0.6 of a cell, at least 4).
    const BAR_X: f32 = 197.0;

    #[test]
    fn the_thumb_of_the_scrollbar_follows_the_pointer_from_the_oldest_line_to_the_screen() {
        let mut rig = scrolled_back();
        let history = rig.core.history_size();
        assert!(history > 30, "{history}");
        rig.core.scroll_display(Scroll::Top);
        // The track is the 100 pixels of the 5 rows; the thumb, 16 at least, is at its top.
        rig.press((BAR_X, 5.0));
        assert!(rig.pointer.holds_thumb());
        rig.drag((BAR_X, 5.0 + 84.0));
        assert_eq!(rig.core.display_offset(), 0, "the thumb at the bottom shows the screen");
        rig.drag((BAR_X, 5.0 + 42.0));
        assert_eq!(rig.core.display_offset(), history - (history as f32 / 2.0).round() as usize, "half way, half the scrollback");
        rig.drag((BAR_X, -300.0));
        assert_eq!(rig.core.display_offset(), history, "above the track: the oldest line");
        rig.release((BAR_X, -300.0));
        assert!(!rig.pointer.holds_thumb());
        assert_eq!(rig.text(), None, "dragging the thumb selects nothing");
    }

    #[test]
    fn a_press_on_the_track_brings_the_thumb_there_and_a_drag_goes_on_from_it() {
        let mut rig = scrolled_back();
        let history = rig.core.history_size();
        rig.core.scroll_display(Scroll::Top);
        // The middle of the 16-pixel thumb goes to 60: its top to 52 of the 84 it can travel.
        rig.press((BAR_X, 60.0));
        assert_eq!(rig.core.display_offset(), history - (52.0 / 84.0 * history as f32).round() as usize);
        rig.drag((BAR_X, 100.0));
        assert_eq!(rig.core.display_offset(), 0);
        rig.release((BAR_X, 100.0));
    }

    #[test]
    fn without_a_scrollbar_the_right_edge_selects_and_with_the_find_it_scrolls() {
        let mut rig = scrolled_back();
        assert_eq!(rig.core.display_offset(), 0);
        // At the bottom there is no scrollbar: the press starts a selection as anywhere else.
        rig.press((BAR_X, 50.0));
        assert!(!rig.pointer.holds_thumb());
        rig.release((BAR_X, 50.0));
        // The find bar shows the scrollbar at the bottom too, and it can be taken there.
        rig.pointer.set_finding(true);
        rig.press((BAR_X, 10.0));
        assert!(rig.pointer.holds_thumb());
        assert!(rig.core.display_offset() > 0, "the thumb went up to the press");
    }

    impl Rig {
        fn new(text: &str) -> Rig {
            let mut core = TerminalCore::new(20, 5, CoreConfig::default());
            core.feed(text.as_bytes());
            Rig { core, pointer: Pointer::default(), clock: 1000 }
        }

        fn send(&mut self, kind: PointerKind, at: (f32, f32), mods: Mods) -> PointerEffects {
            self.clock += 30;
            let event = PointerEvent::new(kind, at.0, at.1).with_mods(mods).with_time(self.clock);
            let modes = input_modes_of(&self.core);
            self.pointer.handle(&mut self.core, &event, &GRID, &modes)
        }

        fn press(&mut self, at: (f32, f32)) -> PointerEffects {
            self.send(PointerKind::Press(MouseButton::Left), at, Mods::NONE)
        }

        fn release(&mut self, at: (f32, f32)) -> PointerEffects {
            self.send(PointerKind::Release(MouseButton::Left), at, Mods::NONE)
        }

        fn drag(&mut self, to: (f32, f32)) -> PointerEffects {
            self.send(PointerKind::Move, to, Mods::NONE)
        }

        /// A press and a release in the same place.
        fn click(&mut self, at: (f32, f32)) -> PointerEffects {
            self.press(at);
            self.release(at)
        }

        fn text(&self) -> Option<String> {
            self.core.selection_text()
        }
    }

    #[test]
    fn dragging_selects_from_the_press_to_the_release_and_reports_it_on_release() {
        let mut rig = Rig::new("hello world, again");
        assert_eq!(rig.press(left(6, 0)), PointerEffects::default());
        assert_eq!(rig.text(), None, "nothing is selected until the pointer moves");
        rig.drag(right(10, 0));
        assert_eq!(rig.text().as_deref(), Some("world"));
        let effects = rig.release(right(10, 0));
        assert!(effects.selected && effects.reports.is_empty());
        assert_eq!(rig.text().as_deref(), Some("world"));
    }

    #[test]
    fn a_pointer_past_the_middle_of_a_cell_includes_it_and_before_the_middle_does_not() {
        let mut rig = Rig::new("abcdef");
        rig.press(right(1, 0));
        rig.drag(left(4, 0));
        rig.release(left(4, 0));
        assert_eq!(rig.text().as_deref(), Some("cd"), "starts after the middle of b, ends before the middle of e");
    }

    #[test]
    fn a_click_without_a_drag_clears_the_selection() {
        let mut rig = Rig::new("hello world");
        rig.press(left(0, 0));
        rig.drag(right(4, 0));
        rig.release(right(4, 0));
        assert_eq!(rig.text().as_deref(), Some("hello"));
        let effects = rig.click(left(8, 1));
        assert!(!effects.selected);
        assert!(!rig.core.selection_active(), "the click dismissed it, and no empty selection is left behind");
    }

    #[test]
    fn a_double_click_selects_the_word_and_a_triple_click_the_line() {
        let mut rig = Rig::new("hello world, again");
        rig.click(left(7, 0));
        let effects = rig.click(left(7, 0));
        assert_eq!(rig.text().as_deref(), Some("world"));
        assert!(effects.selected, "a picked word is a selection to copy");
        let effects = rig.click(left(7, 0));
        assert_eq!(rig.text().as_deref(), Some("hello world, again"), "the line, without its line break");
        assert!(effects.selected);
        // Far apart in time, the next click starts over.
        rig.clock += 5000;
        rig.click(left(7, 0));
        assert_eq!(rig.text(), None);
    }

    #[test]
    fn alt_drag_selects_a_block() {
        let mut rig = Rig::new("abcdef\r\nghijkl\r\nmnopqr");
        rig.send(PointerKind::Press(MouseButton::Left), left(1, 0), Mods::ALT);
        rig.send(PointerKind::Move, right(3, 2), Mods::ALT);
        rig.send(PointerKind::Release(MouseButton::Left), right(3, 2), Mods::ALT);
        assert_eq!(rig.text().as_deref(), Some("bcd\nhij\nnop"));
    }

    #[test]
    fn shift_click_grows_the_selection_and_starts_one_when_there_is_none() {
        let mut rig = Rig::new("hello world, again");
        rig.send(PointerKind::Press(MouseButton::Left), left(0, 0), Mods::SHIFT);
        rig.send(PointerKind::Release(MouseButton::Left), left(0, 0), Mods::SHIFT);
        assert_eq!(rig.text(), None, "a shift click with nothing to extend is a plain click");

        rig.clock += 2000;
        rig.press(left(0, 0));
        rig.drag(right(4, 0));
        rig.release(right(4, 0));
        assert_eq!(rig.text().as_deref(), Some("hello"));
        rig.send(PointerKind::Press(MouseButton::Left), right(10, 0), Mods::SHIFT);
        let effects = rig.send(PointerKind::Release(MouseButton::Left), right(10, 0), Mods::SHIFT);
        assert_eq!(rig.text().as_deref(), Some("hello world"), "the end moved, the start stayed");
        assert!(effects.selected, "the grown selection is still a selection to copy");
    }

    #[test]
    fn a_program_that_tracks_the_mouse_gets_the_drag_and_shift_takes_it_back() {
        let mut rig = Rig::new("\x1b[?1000h\x1b[?1002h\x1b[?1006hhello world");
        let down = rig.press(left(6, 0));
        assert_eq!(down.reports, [b"\x1b[<0;7;1M".to_vec()]);
        let motion = rig.drag(right(10, 0));
        assert_eq!(motion.reports, [b"\x1b[<32;11;1M".to_vec()]);
        let up = rig.release(right(10, 0));
        assert_eq!(up.reports, [b"\x1b[<0;11;1m".to_vec()]);
        assert!(!rig.core.selection_active(), "the program owns the pointer: nothing was selected natively");

        rig.send(PointerKind::Press(MouseButton::Left), left(6, 0), Mods::SHIFT);
        rig.send(PointerKind::Move, right(10, 0), Mods::SHIFT);
        let up = rig.send(PointerKind::Release(MouseButton::Left), right(10, 0), Mods::SHIFT);
        assert!(up.reports.is_empty() && up.selected, "shift bypasses the program");
        assert_eq!(rig.text().as_deref(), Some("world"));
    }

    #[test]
    fn x10_urxvt_and_pixel_reports_follow_the_mode_the_program_chose() {
        let mut rig = Rig::new("\x1b[?9h");
        assert_eq!(rig.press(left(0, 0)).reports, [b"\x1b[M !!".to_vec()], "X10: the press only");
        rig.release(left(0, 0));
        assert!(rig.drag(left(5, 0)).reports.is_empty());
        let mut rig = Rig::new("\x1b[?1000h\x1b[?1015h");
        assert_eq!(rig.press(left(0, 0)).reports, [b"\x1b[32;1;1M".to_vec()], "urxvt");
    }

    #[test]
    fn the_wheel_scrolls_the_scrollback_unless_the_program_wants_it() {
        let mut text = String::new();
        for i in 0..30 {
            text.push_str(&format!("line{i}\r\n"));
        }
        let mut rig = Rig::new(&text);
        let effects = rig.send(PointerKind::Wheel { delta: 120, horizontal: false }, left(0, 0), Mods::NONE);
        assert!(effects.reports.is_empty());
        assert_eq!(rig.core.display_offset(), 3, "one notch is three lines toward the older ones");
        rig.send(PointerKind::Wheel { delta: -120, horizontal: false }, left(0, 0), Mods::NONE);
        assert_eq!(rig.core.display_offset(), 0);

        rig.core.feed(b"\x1b[?1000h\x1b[?1006h");
        let effects = rig.send(PointerKind::Wheel { delta: 120, horizontal: false }, left(2, 1), Mods::NONE);
        assert_eq!(effects.reports, [b"\x1b[<64;3;2M".to_vec()]);
        assert_eq!(rig.core.display_offset(), 0, "the program scrolls, not the terminal");
        rig.send(PointerKind::Wheel { delta: 120, horizontal: false }, left(2, 1), Mods::SHIFT);
        assert_eq!(rig.core.display_offset(), 3, "shift scrolls the terminal's own history");
    }

    #[test]
    fn a_drag_held_above_the_grid_scrolls_and_extends_the_selection() {
        let mut text = String::new();
        for i in 0..30 {
            text.push_str(&format!("line{i:02}\r\n"));
        }
        let mut rig = Rig::new(&text);
        // The live screen shows line26 to line29 and the empty cursor row.
        assert_eq!(rig.core.display_offset(), 0);
        rig.press(left(0, 3));
        let above = (left(0, 0).0, -25.0);
        rig.drag(above);
        assert!(rig.pointer.drag_scroll(&mut rig.core));
        assert!(rig.pointer.drag_scroll(&mut rig.core));
        assert_eq!(rig.core.display_offset(), 2 * 2, "overshoot of two rows moves two lines a step");
        let expected = ["line22", "line23", "line24", "line25", "line26", "line27", "line28"].join("\n");
        assert_eq!(rig.text(), Some(expected), "the selection grew into the lines scrolled into view");
        // Back inside the grid: nothing more to scroll.
        rig.drag(left(2, 1));
        assert!(!rig.pointer.drag_scroll(&mut rig.core));
        let effects = rig.release(left(2, 1));
        assert!(effects.selected);
    }

    #[test]
    fn a_click_on_the_pinned_prompt_line_scrolls_to_that_prompt_and_selects_nothing() {
        let mut text = String::from("\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
        for i in 0..10 {
            text.push_str(&format!("file{i}\r\n"));
        }
        text.push_str("\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        let mut rig = Rig::new(&text);
        // Twelve lines on five rows: the prompt of `ls` is seven lines up, so three back it is above the view.
        rig.core.scroll_display(Scroll::Delta(3));
        let effects = rig.click(left(4, 0));
        assert_eq!(effects, PointerEffects::default());
        assert_eq!(rig.core.display_offset(), 7, "the prompt is the top row");
        assert_eq!(rig.text(), None, "the click selected nothing");
        // Now the prompt is in sight: the top row is a row like the others again.
        rig.clock += 2000;
        rig.press(left(0, 0));
        rig.drag(right(1, 0));
        rig.release(right(1, 0));
        assert_eq!(rig.text().as_deref(), Some("$ "));
    }

    #[test]
    fn a_drag_held_below_the_grid_scrolls_toward_the_newer_lines() {
        let mut text = String::new();
        for i in 0..30 {
            text.push_str(&format!("line{i:02}\r\n"));
        }
        let mut rig = Rig::new(&text);
        rig.core.scroll_display(Scroll::Delta(10));
        rig.press(left(0, 0));
        rig.drag((left(1, 4).0, 5.0 * 20.0 + 30.0));
        assert!(rig.pointer.drag_scroll(&mut rig.core));
        assert_eq!(rig.core.display_offset(), 8, "overshoot of two rows moves two lines");
        assert!(rig.text().is_some());
    }

    #[test]
    fn ctrl_click_on_a_link_opens_it_and_every_other_gesture_selects() {
        let mut rig = Rig::new("go https://a.b/x ok\r\n\x1b]8;;javascript:alert(1)\x07script\x1b]8;;\x07");
        let ctrl = |rig: &mut Rig, kind, at| rig.send(kind, at, Mods::CTRL);
        let down = PointerKind::Press(MouseButton::Left);
        let up = PointerKind::Release(MouseButton::Left);

        ctrl(&mut rig, down, left(8, 0));
        let effects = ctrl(&mut rig, up, left(8, 0));
        assert_eq!(effects.open_link.map(|link| link.target).as_deref(), Some("https://a.b/x"));
        assert!(!effects.selected && rig.text().is_none(), "opening a link selects nothing");

        rig.clock += 2000;
        assert_eq!(rig.click(left(8, 0)).open_link, None, "without Ctrl a click on a link is a plain click");
        rig.clock += 2000;
        ctrl(&mut rig, down, left(0, 0));
        assert_eq!(ctrl(&mut rig, up, left(0, 0)).open_link, None, "Ctrl+click off a link opens nothing");
        rig.clock += 2000;
        ctrl(&mut rig, down, left(2, 1));
        assert_eq!(ctrl(&mut rig, up, left(2, 1)).open_link, None, "a link that may not be opened is text");

        // A real mouse adds a move to the same place around a click: that is still a click.
        rig.clock += 2000;
        ctrl(&mut rig, PointerKind::Move, left(8, 0));
        ctrl(&mut rig, down, left(8, 0));
        ctrl(&mut rig, PointerKind::Move, left(8, 0));
        let effects = ctrl(&mut rig, up, left(8, 0));
        assert_eq!(effects.open_link.map(|link| link.target).as_deref(), Some("https://a.b/x"));

        // Starting a drag on a link selects instead.
        rig.clock += 2000;
        ctrl(&mut rig, down, left(8, 0));
        ctrl(&mut rig, PointerKind::Move, right(12, 0));
        let effects = ctrl(&mut rig, up, right(12, 0));
        assert_eq!((effects.open_link, effects.selected), (None, true));
        assert_eq!(rig.text().as_deref(), Some("://a."));
    }

    #[test]
    fn ctrl_click_on_a_file_link_opens_the_checked_path_and_not_the_address_as_written() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "\\Cargo.toml");
        let uri = format!("file:///{}", path.replace('\\', "/").replace(' ', "%20"));
        let mut rig = Rig::new(&format!("\x1b]8;;{uri}\x07file\x1b]8;;\x07"));
        let down = PointerKind::Press(MouseButton::Left);
        let up = PointerKind::Release(MouseButton::Left);
        rig.send(down, left(1, 0), Mods::CTRL);
        assert_eq!(rig.send(up, left(1, 0), Mods::CTRL).open_link.map(|link| link.target).as_deref(), Some(path));
    }

    #[test]
    fn ctrl_click_on_a_printed_path_with_its_line_opens_the_file_in_the_folder_the_shell_reported() {
        let folder = env!("CARGO_MANIFEST_DIR");
        let osc7 = format!("\x1b]7;file:///{}\x07", folder.replace('\\', "/").replace(' ', "%20"));
        let mut rig = Rig::new(&format!("{osc7}at src\\pointer.rs:12:5. x\r\nsrc\\missing.rs:1"));
        let down = PointerKind::Press(MouseButton::Left);
        let up = PointerKind::Release(MouseButton::Left);
        rig.send(down, left(5, 0), Mods::CTRL);
        let link = rig.send(up, left(5, 0), Mods::CTRL).open_link.expect("the path is a link");
        assert_eq!(link.uri, "src\\pointer.rs:12:5", "the text keeps its position and loses the full stop");
        assert_eq!(link.target, std::fs::canonicalize(format!("{folder}\\src\\pointer.rs")).unwrap().to_str().unwrap().trim_start_matches(r"\\?\"));
        rig.clock += 2000;
        // The first line wraps at 20 columns: the second path is on the third row.
        rig.send(down, left(2, 2), Mods::CTRL);
        assert_eq!(rig.send(up, left(2, 2), Mods::CTRL).open_link, None, "a file that is not there is text");
    }

    #[test]
    fn cancelling_ends_the_drag_where_it_was_and_tells_the_program_the_buttons_are_up() {
        let mut rig = Rig::new("hello world");
        rig.press(left(0, 0));
        rig.drag(right(4, 0));
        let modes = input_modes_of(&rig.core);
        let effects = rig.pointer.cancel(&mut rig.core, &GRID, &modes);
        assert!(effects.selected && effects.reports.is_empty());
        assert_eq!(rig.text().as_deref(), Some("hello"));

        let mut rig = Rig::new("\x1b[?1000h\x1b[?1006h");
        rig.press(left(2, 1));
        let modes = input_modes_of(&rig.core);
        let effects = rig.pointer.cancel(&mut rig.core, &GRID, &modes);
        assert_eq!(effects.reports, [b"\x1b[<0;3;2m".to_vec()]);
    }
}
