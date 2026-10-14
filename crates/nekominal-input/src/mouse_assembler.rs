//! `MouseAssembler`: pointer events in pixels in, mouse reports for the program or typed outcomes for
//! the pane out.
//!
//! The assembler owns the policy that sits between the pointer and the encoders of [`crate::mouse`]:
//!
//! * **Who gets the pointer.** While the program tracks the mouse it gets every press, release, motion
//!   and wheel event, except when Shift is held: Shift hands the pointer to the terminal (selection,
//!   scrollback). The owner is decided at the first press and kept until every button is up, so a
//!   Shift that is pressed or released mid-drag cannot split a gesture between the two.
//! * **Cells.** Positions arrive in pixels of the terminal grid and become cells (and pixels for
//!   SGR-pixels reporting); a position outside the grid is clamped, and a selection drag reports how
//!   far outside it is so the pane can scroll.
//! * **Selection gestures** while the terminal owns the pointer: press starts, motion beyond a drag
//!   threshold updates, release ends; double and triple clicks pick words and lines, Alt picks a
//!   block, Shift extends, Ctrl+click on a hyperlink opens it.
//! * **The wheel**: whole notches for the program (a precision touchpad's small deltas add up), lines
//!   for the scrollback, cursor keys for alternate scroll.
//!
//! It reads no clock and no OS setting: the host supplies a timestamp with each event and the system's
//! double-click time, drag distance and scroll lines in [`PointerConfig`].

use crate::{
    key::Mods,
    modes::{InputModes, MouseEncoding, MouseMode},
    mouse::{MouseButton, MouseInput, MouseKind, MouseState, WheelAccumulator, WheelDirection, encode_mouse},
};

/// The system settings the gestures depend on (`GetDoubleClickTime`, `SM_CXDRAG`,
/// `SPI_GETWHEELSCROLLLINES`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerConfig {
    /// Pixels the pointer must travel from the press before a drag starts selecting.
    pub drag_threshold: f32,
    /// Longest time between two presses in one cell that still count as a double click.
    pub multi_click_ms: u32,
    /// Lines the scrollback moves for one wheel notch.
    pub lines_per_notch: u32,
}

impl Default for PointerConfig {
    fn default() -> Self {
        PointerConfig { drag_threshold: 4.0, multi_click_ms: 500, lines_per_notch: 3 }
    }
}

/// The terminal grid in pixels, relative to the pointer's coordinate origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerGrid {
    pub cell_width: f32,
    pub cell_height: f32,
    pub columns: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind {
    Press(MouseButton),
    Release(MouseButton),
    Move,
    /// A wheel delta in 120ths of a notch, positive away from the user (up) and to the right.
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    /// The pointer capture was lost or the window lost focus: end any gesture.
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub mods: Mods,
    /// Pixels from the top-left corner of the grid; may lie outside it while a button is down.
    pub x: f32,
    pub y: f32,
    /// Milliseconds on any monotonic clock; only differences of nearby presses are used.
    pub time_ms: u32,
    /// The pointer is over a hyperlink (the pane knows; the assembler does not).
    pub over_link: bool,
}

impl PointerEvent {
    pub fn new(kind: PointerKind, x: f32, y: f32) -> Self {
        PointerEvent { kind, mods: Mods::NONE, x, y, time_ms: 0, over_link: false }
    }

    pub fn with_mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    pub fn with_time(mut self, time_ms: u32) -> Self {
        self.time_ms = time_ms;
        self
    }

    pub fn over_link(mut self) -> Self {
        self.over_link = true;
        self
    }
}

/// Which half of a cell a position falls in: a selection includes a cell only when the pointer is
/// past its middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPoint {
    pub col: u16,
    pub row: u16,
    pub side: Side,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Word,
    Line,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerOutcome {
    /// Bytes for the program: a mouse report, or the cursor keys of alternate scroll.
    Report(Vec<u8>),
    /// A selection starts here. `extend`: Shift was held, so an existing selection grows to this
    /// point instead of starting a new one.
    SelectionStart { point: CellPoint, kind: SelectionKind, extend: bool },
    /// The drag moved to another cell or half cell. `overshoot_rows` is how many rows the pointer is
    /// above (negative) or below (positive) the grid, for scrolling while the drag is held there.
    SelectionUpdate { point: CellPoint, overshoot_rows: i32 },
    /// The button went up. `dragged` is false for a click that never left the drag threshold: a
    /// single click clears the selection, a double or triple click keeps its word or line.
    SelectionEnd { point: CellPoint, dragged: bool },
    /// Scroll the scrollback by this many lines; positive is up, towards older output.
    Scroll(i32),
    /// Ctrl+click on a hyperlink, released in the cell it was pressed in.
    OpenLink { col: u16, row: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Program,
    Terminal,
}

#[derive(Debug)]
struct Drag {
    origin_pixel: (f32, f32),
    origin: CellPoint,
    kind: SelectionKind,
    extend: bool,
    /// A Ctrl+click on a link: nothing is selected unless the pointer drags away.
    link: bool,
    moved: bool,
    last: (CellPoint, i32),
}

#[derive(Debug)]
struct Clicks {
    time_ms: u32,
    cell: (u16, u16),
    count: u8,
}

/// Where a pixel position falls.
#[derive(Debug)]
struct Location {
    point: CellPoint,
    overshoot_rows: i32,
    pixel: (u32, u32),
}

fn locate(x: f32, y: f32, grid: &PointerGrid) -> Location {
    let finite = |v: f32| if v.is_finite() { v.clamp(-1.0e6, 1.0e6) } else { 0.0 };
    let (x, y) = (finite(x), finite(y));
    let (cell_width, cell_height) = (grid.cell_width.max(1.0), grid.cell_height.max(1.0));
    let (columns, rows) = (grid.columns.max(1), grid.rows.max(1));
    let (width, height) = (f32::from(columns) * cell_width, f32::from(rows) * cell_height);

    let col = ((x / cell_width).floor().max(0.0) as u32).min(u32::from(columns) - 1) as u16;
    let row = ((y / cell_height).floor().max(0.0) as u32).min(u32::from(rows) - 1) as u16;
    let side = if x < 0.0 {
        Side::Left
    } else if x >= width || x - f32::from(col) * cell_width >= cell_width / 2.0 {
        Side::Right
    } else {
        Side::Left
    };
    let overshoot_rows = if y < 0.0 {
        -((-y / cell_height).ceil() as i32)
    } else if y >= height {
        ((y - height) / cell_height).floor() as i32 + 1
    } else {
        0
    };
    let pixel = (x.max(0.0).min(width - 1.0) as u32, y.max(0.0).min(height - 1.0) as u32);
    Location { point: CellPoint { col, row, side }, overshoot_rows, pixel }
}

/// The cell a pixel position falls in, as `(column, row)`; a position outside the grid is clamped to its edge.
/// The pane uses it to ask what lies under the pointer without a gesture (hover).
pub fn cell_at(x: f32, y: f32, grid: &PointerGrid) -> (u16, u16) {
    let at = locate(x, y, grid);
    (at.point.col, at.point.row)
}

/// Stateful translation of [`PointerEvent`]s. One per pane.
#[derive(Debug)]
pub struct MouseAssembler {
    config: PointerConfig,
    state: MouseState,
    last_modes: (MouseMode, MouseEncoding),
    /// Buttons pressed and not yet released, in press order.
    down: Vec<MouseButton>,
    owner: Option<Owner>,
    drag: Option<Drag>,
    clicks: Option<Clicks>,
    last_point: Option<(Location, Mods)>,
    wheel_vertical: WheelAccumulator,
    wheel_horizontal: WheelAccumulator,
    scroll: WheelAccumulator,
}

impl Default for MouseAssembler {
    fn default() -> Self {
        MouseAssembler::new(PointerConfig::default())
    }
}

impl MouseAssembler {
    pub fn new(config: PointerConfig) -> Self {
        MouseAssembler {
            config,
            state: MouseState::default(),
            last_modes: (MouseMode::None, MouseEncoding::Default),
            down: Vec::new(),
            owner: None,
            drag: None,
            clicks: None,
            last_point: None,
            wheel_vertical: WheelAccumulator::default(),
            wheel_horizontal: WheelAccumulator::default(),
            scroll: WheelAccumulator::default(),
        }
    }

    pub fn set_config(&mut self, config: PointerConfig) {
        self.config = config;
    }

    /// Feeds one pointer event and returns what to do about it, in order.
    pub fn handle(&mut self, event: &PointerEvent, grid: &PointerGrid, modes: &InputModes) -> Vec<PointerOutcome> {
        // A new mode starts a new report history: the position of the last report means nothing.
        if self.last_modes != (modes.mouse_mode, modes.mouse_encoding) {
            self.last_modes = (modes.mouse_mode, modes.mouse_encoding);
            self.state.reset();
        }
        let at = locate(event.x, event.y, grid);
        let mut out = Vec::new();
        let program_wants_pointer = modes.program_gets_pointer(event.mods);

        match event.kind {
            PointerKind::Press(button) => {
                if self.down.contains(&button) {
                    return out;
                }
                if self.down.is_empty() {
                    self.owner = Some(if program_wants_pointer { Owner::Program } else { Owner::Terminal });
                }
                self.down.push(button);
                match self.owner {
                    Some(Owner::Program) => self.report(MouseKind::Press(button), event, &at, modes, &mut out),
                    _ if button == MouseButton::Left => self.begin_selection(event, &at, &mut out),
                    _ => {}
                }
            }
            PointerKind::Release(button) => {
                let Some(index) = self.down.iter().position(|down| *down == button) else {
                    return out;
                };
                self.down.remove(index);
                match self.owner {
                    Some(Owner::Program) => self.report(MouseKind::Release(button), event, &at, modes, &mut out),
                    _ if button == MouseButton::Left => self.end_selection(&at, &mut out),
                    _ => {}
                }
                if self.down.is_empty() {
                    self.owner = None;
                }
            }
            PointerKind::Move => match self.owner {
                Some(Owner::Program) => {
                    let held = self.down.first().copied();
                    self.report(MouseKind::Move { held }, event, &at, modes, &mut out);
                }
                Some(Owner::Terminal) => self.drag_selection(event, &at, &mut out),
                None if program_wants_pointer => {
                    self.report(MouseKind::Move { held: None }, event, &at, modes, &mut out);
                }
                None => {}
            },
            PointerKind::Wheel { delta, horizontal } => {
                self.wheel(delta, horizontal, event, &at, modes, &mut out);
            }
            // A cancel carries no meaningful position: the gesture ends where the pointer last was.
            PointerKind::Cancel => self.cancel(modes, &mut out),
        }
        if event.kind != PointerKind::Cancel {
            self.last_point = Some((at, event.mods));
        }
        out
    }

    fn report(
        &mut self,
        kind: MouseKind,
        event: &PointerEvent,
        at: &Location,
        modes: &InputModes,
        out: &mut Vec<PointerOutcome>,
    ) {
        let input =
            MouseInput::new(kind, at.point.col, at.point.row).with_mods(event.mods).with_pixel(at.pixel.0, at.pixel.1);
        out.extend(encode_mouse(&input, modes, &mut self.state).map(PointerOutcome::Report));
    }

    // ---- selection ----------------------------------------------------------------------------

    fn begin_selection(&mut self, event: &PointerEvent, at: &Location, out: &mut Vec<PointerOutcome>) {
        let cell = (at.point.col, at.point.row);
        let count = match &self.clicks {
            Some(last)
                if last.cell == cell && event.time_ms.wrapping_sub(last.time_ms) <= self.config.multi_click_ms =>
            {
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.clicks = Some(Clicks { time_ms: event.time_ms, cell, count });

        let kind = match count {
            2 => SelectionKind::Word,
            3 => SelectionKind::Line,
            _ if event.mods.contains(Mods::ALT) => SelectionKind::Block,
            _ => SelectionKind::Simple,
        };
        let extend = event.mods.contains(Mods::SHIFT);
        let link = kind == SelectionKind::Simple && event.mods.contains(Mods::CTRL) && event.over_link;
        self.drag = Some(Drag {
            origin_pixel: (event.x, event.y),
            origin: at.point,
            kind,
            extend,
            link,
            moved: false,
            last: (at.point, 0),
        });
        if !link {
            out.push(PointerOutcome::SelectionStart { point: at.point, kind, extend });
        }
    }

    fn drag_selection(&mut self, event: &PointerEvent, at: &Location, out: &mut Vec<PointerOutcome>) {
        let threshold = self.config.drag_threshold.max(0.0);
        let Some(drag) = &mut self.drag else {
            return;
        };
        if !drag.moved {
            let (dx, dy) = (event.x - drag.origin_pixel.0, event.y - drag.origin_pixel.1);
            if dx * dx + dy * dy < threshold * threshold {
                return;
            }
            drag.moved = true;
            // A Ctrl+click on a link turned into a drag: the selection starts where the button went down.
            if drag.link {
                out.push(PointerOutcome::SelectionStart { point: drag.origin, kind: drag.kind, extend: drag.extend });
            }
        }
        if drag.last != (at.point, at.overshoot_rows) {
            drag.last = (at.point, at.overshoot_rows);
            out.push(PointerOutcome::SelectionUpdate { point: at.point, overshoot_rows: at.overshoot_rows });
        }
    }

    fn end_selection(&mut self, at: &Location, out: &mut Vec<PointerOutcome>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        // A drag is not a click: the next press starts a new chain.
        if drag.moved {
            self.clicks = None;
        }
        if drag.link && !drag.moved {
            out.push(PointerOutcome::OpenLink { col: drag.origin.col, row: drag.origin.row });
        } else {
            out.push(PointerOutcome::SelectionEnd { point: at.point, dragged: drag.moved });
        }
    }

    // ---- wheel ---------------------------------------------------------------------------------

    fn wheel(
        &mut self,
        delta: i32,
        horizontal: bool,
        event: &PointerEvent,
        at: &Location,
        modes: &InputModes,
        out: &mut Vec<PointerOutcome>,
    ) {
        let tracking = modes.mouse_tracking();
        let alternate_scroll = !tracking && modes.alt_screen && modes.alternate_scroll;
        if (tracking && !event.mods.contains(Mods::SHIFT)) || alternate_scroll {
            let accumulator = if horizontal { &mut self.wheel_horizontal } else { &mut self.wheel_vertical };
            let notches = accumulator.push(delta);
            if notches == 0 {
                return;
            }
            let direction = match (horizontal, notches > 0) {
                (false, true) => WheelDirection::Up,
                (false, false) => WheelDirection::Down,
                (true, true) => WheelDirection::Right,
                (true, false) => WheelDirection::Left,
            };
            let count = u16::try_from(notches.unsigned_abs()).unwrap_or(u16::MAX);
            let input = MouseInput::new(MouseKind::Wheel(direction), at.point.col, at.point.row)
                .with_mods(event.mods)
                .with_pixel(at.pixel.0, at.pixel.1)
                .with_notches(count);
            out.extend(encode_mouse(&input, modes, &mut self.state).map(PointerOutcome::Report));
        } else if !horizontal {
            // Scaling the delta by the lines per notch lets a precision touchpad scroll smoothly.
            let scaled = delta.saturating_mul(i32::try_from(self.config.lines_per_notch).unwrap_or(i32::MAX));
            let lines = self.scroll.push(scaled);
            if lines != 0 {
                out.push(PointerOutcome::Scroll(lines));
            }
        }
    }

    // ---- cancel --------------------------------------------------------------------------------

    /// Ends every gesture: the program is told its buttons went up, a selection drag ends where the
    /// pointer last was.
    fn cancel(&mut self, modes: &InputModes, out: &mut Vec<PointerOutcome>) {
        let last = self.last_point.take();
        let owner = self.owner.take();
        let down = std::mem::take(&mut self.down);
        if let (Some((at, mods)), Some(Owner::Program)) = (&last, owner) {
            for button in down {
                let input = MouseInput::new(MouseKind::Release(button), at.point.col, at.point.row)
                    .with_mods(*mods)
                    .with_pixel(at.pixel.0, at.pixel.1);
                out.extend(encode_mouse(&input, modes, &mut self.state).map(PointerOutcome::Report));
            }
        }
        if let (Some(drag), Some((at, _))) = (self.drag.take(), &last) {
            out.push(PointerOutcome::SelectionEnd { point: at.point, dragged: drag.moved });
        }
        self.last_point = last;
    }
}
