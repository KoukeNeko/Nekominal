//! `TerminalCore`: alacritty_terminal's `Term` driven directly from the PTY forwarder (no
//! `EventLoop`, no `tty` module), with a chunk-safe sniffer on the same byte stream.

use std::{
    fmt, mem,
    sync::{Arc, Mutex},
    time::Instant,
};

use alacritty_terminal::{
    event::{Event as AlacEvent, EventListener, WindowSize},
    grid::{Dimensions, Grid, Row, Scroll},
    index::{Boundary, Column, Direction, Line, Point, Side},
    selection::{Selection, SelectionRange, SelectionType},
    term::{
        cell::{Cell, Flags, Hyperlink, LineLength},
        point_to_viewport,
        search::{Match, RegexIter, RegexSearch},
        viewport_to_point, ClipboardType, Config, Osc52, Term, TermDamage, TermMode,
        MIN_COLUMNS, MIN_SCREEN_LINES,
    },
    vte::ansi::{self, Color, CursorShape, CursorStyle, Handler, Rgb},
};

use crate::{
    agent_status::{AgentRecord, AgentStatus, Report},
    guard::StringGuard,
    links::{self, Link},
    osc::{is_control_or_bidi, strip_control, Notification, Progress, PromptMark},
    sniffer::{MouseExtension, Sniffed, Sniffer, Unsupported},
    theme::Palette,
};

/// Dimensions for `Term::new` / `Term::resize`. Only `columns` and `screen_lines` are read by them;
/// `total_lines` is for the trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermDims {
    pub columns: usize,
    pub screen_lines: usize,
}

impl TermDims {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        Self { columns: columns.max(MIN_COLUMNS), screen_lines: screen_lines.max(MIN_SCREEN_LINES) }
    }
}

impl Dimensions for TermDims {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// `EventListener` for `Term`. `send_event` runs inside `Processor::advance` while the caller owns
/// the terminal, so it only queues; `TerminalCore` drains the queue after every parsed segment.
#[derive(Clone, Default)]
struct EventProxy(Arc<Mutex<Vec<AlacEvent>>>);

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        self.0.lock().expect("event queue poisoned").push(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardKind {
    Clipboard,
    Selection,
}

impl From<ClipboardType> for ClipboardKind {
    fn from(value: ClipboardType) -> Self {
        match value {
            ClipboardType::Clipboard => Self::Clipboard,
            ClipboardType::Selection => Self::Selection,
        }
    }
}

/// Turns clipboard text into the `OSC 52` reply that must be written to the PTY.
pub type ClipboardFormatter = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Identity of a grid line that survives scrolling: a line keeps its id while it moves from the screen into
/// scrollback, and ids grow downward, so comparing two ids orders the lines. An id stops resolving
/// (`TerminalCore::line_of` returns `None`) when its line is trimmed from the scrollback, and every id of
/// an older `epoch` stops resolving at once when the buffer is rewritten under it (reflow after a column
/// change, `ED 3`, RIS); `CoreEvent::LineIdsReset` announces that.
///
/// Ids follow rows, not text, wherever text moves without entering the scrollback: a scroll region that does
/// not start at the top of the screen (DECSTBM), scroll down (`CSI n T`) and a reverse index on the top
/// row. Text moved there changes under a fixed id. Shells do not do that at a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineId {
    pub epoch: u32,
    pub index: u64,
}

impl LineId {
    /// The id of the line `rows` below this one (negative: above) in the same epoch.
    pub fn offset(self, rows: i64) -> Self {
        Self { epoch: self.epoch, index: self.index.saturating_add_signed(rows) }
    }
}

/// Where the cursor was when an OSC 133 mark arrived (grid coordinates: line 0 is the top of the
/// screen, negative lines are scrollback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkCursor {
    pub line: i32,
    pub column: usize,
    pub history_size: usize,
    /// Stable identity of `line` for prompt navigation; `None` on the alternate screen.
    pub line_id: Option<LineId>,
}

/// Mouse reporting modes alacritty_terminal does not track, kept by the sniffer. They complete
/// `TermMode`: DECSET 9 selects X10 reporting (it replaces 1000/1002/1003 and is replaced by them), 1015 the
/// urxvt encoding and 1016 SGR positions in pixels. A host builds its input modes from both: `x10` set means
/// 9 came after the last 1000/1002/1003 and wins over the `TermMode` tracking mode, and `urxvt` /
/// `sgr_pixels` pick the encoding in place of the one `TermMode` reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseExtensions {
    pub x10: bool,
    pub urxvt: bool,
    pub sgr_pixels: bool,
}

pub enum CoreEvent {
    /// Bytes to write to the PTY, in the order the terminal produced them.
    PtyWrite(Vec<u8>),
    Title(String),
    ResetTitle,
    Bell,
    /// OSC 52 write (allowed by `Osc52::OnlyCopy | CopyPaste`).
    ClipboardStore { clipboard: ClipboardKind, text: String },
    /// OSC 52 read (only with `Osc52::OnlyPaste | CopyPaste`): the host asks the user, then writes
    /// `reply(&clipboard_text)` to the PTY.
    ClipboardLoad { clipboard: ClipboardKind, reply: ClipboardFormatter },
    CursorBlinkingChange,
    MouseCursorDirty,
    Cwd(String),
    Notification(Notification),
    Progress(Option<Progress>),
    PromptMark { mark: PromptMark, cursor: Option<MarkCursor> },
    /// Every `LineId` handed out before this event no longer resolves: the buffer was rewritten (reflow after
    /// a column change, clearing the scrollback, RIS). Drop prompt marks held by id.
    LineIdsReset,
    /// An OSC 7501 report changed the pane's agent records.
    Agent(Report),
    /// The records changed for another reason (prompt, RIS, child exit).
    AgentRecordsDropped,
    Unsupported(Unsupported),
    /// win32-input-mode (DECSET 9001) was switched; `TerminalCore::win32_input_mode` has the state.
    Win32InputMode(bool),
    /// alacritty_terminal panicked while parsing (an upstream bug, for example the 4097th kitty keyboard
    /// push). The terminal was reset and keeps running; the screen content is lost.
    EmulatorPanic(String),
}

impl fmt::Debug for CoreEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PtyWrite(bytes) => write!(f, "PtyWrite({:?})", String::from_utf8_lossy(bytes)),
            Self::Title(t) => write!(f, "Title({t:?})"),
            Self::ResetTitle => write!(f, "ResetTitle"),
            Self::Bell => write!(f, "Bell"),
            // The text is what the user copied: it stays out of every log.
            Self::ClipboardStore { clipboard, text } => write!(f, "ClipboardStore({clipboard:?}, {} bytes)", text.len()),
            Self::ClipboardLoad { clipboard, .. } => write!(f, "ClipboardLoad({clipboard:?})"),
            Self::CursorBlinkingChange => write!(f, "CursorBlinkingChange"),
            Self::MouseCursorDirty => write!(f, "MouseCursorDirty"),
            Self::Cwd(p) => write!(f, "Cwd({p:?})"),
            Self::Notification(n) => write!(f, "Notification({n:?})"),
            Self::Progress(p) => write!(f, "Progress({p:?})"),
            Self::PromptMark { mark, cursor } => write!(f, "PromptMark({mark:?}, {cursor:?})"),
            Self::LineIdsReset => write!(f, "LineIdsReset"),
            Self::Agent(r) => write!(f, "Agent({r:?})"),
            Self::AgentRecordsDropped => write!(f, "AgentRecordsDropped"),
            Self::Unsupported(u) => write!(f, "Unsupported({u:?})"),
            Self::Win32InputMode(on) => write!(f, "Win32InputMode({on})"),
            Self::EmulatorPanic(message) => write!(f, "EmulatorPanic({message:?})"),
        }
    }
}

/// Default memory budget of the scrollback of one pane.
pub const DEFAULT_SCROLLBACK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct CoreConfig {
    /// Most lines kept in scrollback. The memory budget below can allow fewer.
    pub scrollback_lines: usize,
    /// Memory budget of the scrollback. A row costs `columns * size_of::<Cell>()` plus its header, so a wide
    /// terminal keeps fewer lines than a narrow one (see `TerminalCore::scrollback_limit`). Combining marks
    /// and hyperlinks allocate beyond that and are not counted, and alacritty keeps up to 1,000 spare rows
    /// allocated (measured: 10,000 history lines at 80 columns, 18.6 MiB of rows, take 20.7 MiB of heap).
    pub scrollback_bytes: usize,
    /// Kitty keyboard protocol bookkeeping (stack, query reply). Off by default in alacritty_terminal.
    pub kitty_keyboard: bool,
    pub osc52: Osc52,
    /// Characters that end a double-click (semantic) selection.
    pub semantic_escape_chars: String,
    pub terminal_name: String,
    pub terminal_version: String,
    pub palette: Palette,
    pub cell_size_px: (u16, u16),
    pub cursor_style: CursorStyle,
    /// Replaces alacritty_terminal's fixed DA1 reply (`CSI ? 6 c`) when set.
    pub da1_reply: Option<String>,
    /// Columns added on each side of every damaged range in `Snapshot::damage`. alacritty's own damage
    /// misses the base cell of a wide glyph that gets overwritten through its spacer and the cell a
    /// combining mark attaches to; 2 covers both (see the damage audit tests).
    pub damage_margin: usize,
}

/// Default semantic escape characters plus the full-width punctuation Traditional Chinese text uses.
pub const SEMANTIC_ESCAPE_CHARS_CJK: &str =
    ",│`|:\"' ()[]{}<>\t，。、；：？！…「」『』（）［］｛｝【】《》〈〉“”‘’";

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            scrollback_bytes: DEFAULT_SCROLLBACK_BYTES,
            kitty_keyboard: true,
            osc52: Osc52::OnlyCopy,
            semantic_escape_chars: SEMANTIC_ESCAPE_CHARS_CJK.to_owned(),
            terminal_name: "Nekominal".to_owned(),
            terminal_version: "0.1.0".to_owned(),
            palette: Palette::default(),
            cell_size_px: (8, 16),
            cursor_style: CursorStyle::default(),
            da1_reply: None,
            damage_margin: 2,
        }
    }
}

/// A cell colour as the program set it, before any palette lookup. The renderer owns resolution (theme,
/// bold-is-bright, faint, inverse, hidden); resolving here as well would apply INVERSE and HIDDEN twice and
/// lose Default, which the renderer needs to draw the pane background once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CellColor {
    #[default]
    Default,
    /// 0-15 ANSI, 16-231 cube, 232-255 grey ramp.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

fn cell_color(color: Color) -> CellColor {
    match color {
        Color::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => CellColor::Indexed(index),
        Color::Named(named) => match named as usize {
            index @ 0..=15 => CellColor::Indexed(index as u8),
            // DimBlack..DimWhite: alacritty only produces them while rendering, never in a stored cell.
            index @ 259..=266 => CellColor::Indexed((index - 259) as u8),
            _ => CellColor::Default,
        },
    }
}

/// One visible cell. Colours are exactly what the program set; BOLD, DIM, INVERSE and HIDDEN stay in `flags`
/// unapplied, for the renderer to resolve once.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapCell {
    pub fg_raw: CellColor,
    pub bg_raw: CellColor,
    pub ch: char,
    /// Combining marks, ZWJ, variation selectors that followed `ch` (alacritty stores them in the cell).
    pub zerowidth: Option<Box<[char]>>,
    pub flags: Flags,
    /// SGR 58, symbolic like the other colours.
    pub underline_color: Option<CellColor>,
    pub hyperlink: Option<Hyperlink>,
}

impl Default for SnapCell {
    fn default() -> Self {
        Self {
            fg_raw: CellColor::Default,
            bg_raw: CellColor::Default,
            ch: ' ',
            zerowidth: None,
            flags: Flags::empty(),
            underline_color: None,
            hyperlink: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDamage {
    /// Viewport row.
    pub row: usize,
    pub left: usize,
    pub right: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Damage {
    #[default]
    Full,
    Lines(Vec<LineDamage>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapCursor {
    /// Viewport position, `None` when the cursor is below the viewport (scrolled back).
    pub position: Option<(usize, usize)>,
    pub shape: CursorShape,
    pub blinking: bool,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub columns: usize,
    pub rows: usize,
    pub display_offset: usize,
    pub history_size: usize,
    /// `rows * columns`, row-major.
    pub cells: Vec<SnapCell>,
    pub cursor: SnapCursor,
    /// Selection in grid coordinates (see `Snapshot::is_selected`).
    pub selection: Option<SelectionRange>,
    pub mode: TermMode,
    pub damage: Damage,
    /// OSC 4 / 10 / 11 / 12 overrides set by the program: slot (0-255 palette, 256 fg, 257 bg, 258 cursor) and colour.
    pub overrides: Vec<(usize, Rgb)>,
    /// Identity of the top viewport row. Row `r` below it is `top_line.offset(r)`; `None` on the alternate screen.
    pub top_line: Option<LineId>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            columns: 0,
            rows: 0,
            display_offset: 0,
            history_size: 0,
            cells: Vec::new(),
            cursor: SnapCursor { position: None, shape: CursorShape::Block, blinking: false },
            selection: None,
            mode: TermMode::empty(),
            damage: Damage::Full,
            overrides: Vec::new(),
            top_line: None,
        }
    }
}

impl Snapshot {
    pub fn cell(&self, row: usize, column: usize) -> &SnapCell {
        &self.cells[row * self.columns + column]
    }

    /// Text of a viewport row without trailing blanks. Wide-char spacers are skipped, combining
    /// marks are appended to their base character.
    pub fn row_text(&self, row: usize) -> String {
        let mut text = String::new();
        for column in 0..self.columns {
            let cell = self.cell(row, column);
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            text.push(cell.ch);
            if let Some(zerowidth) = &cell.zerowidth {
                text.extend(zerowidth.iter());
            }
        }
        text.trim_end().to_owned()
    }

    /// All rows, trailing blank rows removed.
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = (0..self.rows).map(|row| self.row_text(row)).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub fn is_selected(&self, row: usize, column: usize) -> bool {
        self.selection.is_some_and(|range| {
            range.contains(viewport_to_point(self.display_offset, Point::new(row, Column(column))))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

impl From<SelectionKind> for SelectionType {
    fn from(kind: SelectionKind) -> Self {
        match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Semantic => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        }
    }
}

/// Which half of the cell the pointer was in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellSide {
    Left,
    Right,
}

impl From<CellSide> for Side {
    fn from(side: CellSide) -> Self {
        match side {
            CellSide::Left => Side::Left,
            CellSide::Right => Side::Right,
        }
    }
}

/// A search hit in grid coordinates (`line < 0` is scrollback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: (i32, usize),
    pub end: (i32, usize),
}

impl From<Match> for SearchMatch {
    fn from(m: Match) -> Self {
        SearchMatch { start: (m.start().line.0, m.start().column.0), end: (m.end().line.0, m.end().column.0) }
    }
}

/// A compiled search pattern (four lazy DFAs). Keep it while the find bar is open: building it is the
/// expensive part of a search.
pub struct SearchSession {
    regex: RegexSearch,
}

pub struct TerminalCore {
    term: Term<EventProxy>,
    processor: ansi::Processor,
    sniffer: Sniffer,
    guard: StringGuard,
    proxy: EventProxy,
    agent: AgentStatus,
    win32_input_mode: bool,
    mouse_extensions: MouseExtensions,
    events: Vec<CoreEvent>,
    config: CoreConfig,
    dirty: bool,
    /// Lines the scrollback may hold (`scrollback_limit`). alacritty's own limit is this plus
    /// `HISTORY_SLACK`, so lines are only ever dropped by `enforce_scrollback`, which counts them.
    history_cap: usize,
    /// The limit alacritty's grid currently has (`history_cap + HISTORY_SLACK`, or `history_cap` between the
    /// two calls of a trim).
    history_limit: usize,
    /// Lines trimmed from the top of the scrollback in this epoch: `LineId::index` is
    /// `line_base + history_size + grid line`, which does not change when a line scrolls.
    line_base: u64,
    line_epoch: u32,
    /// The lines the shell marked as the start of a prompt (OSC 133 `A`) that are still in the buffer, oldest
    /// first. Held by `LineId`, so they stay on their text while it scrolls into the scrollback, go when the
    /// scrollback trims them, and all go with the ids when the buffer is rewritten.
    prompts: Vec<LineId>,
}

/// How far the scrollback may overshoot its cap between two `enforce_scrollback` calls. One feed chunk adds
/// at most one line per byte, so it is larger than `FEED_CHUNK`.
const HISTORY_SLACK: usize = 2048;

/// Most rows above and below a row that `link_at` joins into one line to look for a URL: a line of this many rows
/// is thousands of characters, and the search runs for every pointer move.
const MAX_LINK_ROWS: i32 = 50;

/// `feed` hands the parsers at most this many bytes at a time, so the scrollback can be trimmed (and the
/// trimmed lines counted) before it outgrows `HISTORY_SLACK`.
const FEED_CHUNK: usize = 1024;

/// Bytes one scrollback row of `columns` cells occupies.
fn history_row_bytes(columns: usize) -> usize {
    columns * std::mem::size_of::<Cell>() + std::mem::size_of::<Row<Cell>>()
}

/// Sequences that make up DECSTR, fed to the emulator (it ignores `CSI ! p`): text cursor on, replace mode,
/// absolute origin, autowrap, normal cursor keys, numeric keypad, full scroll region, default attributes,
/// ASCII in all four character sets, and the program-chosen input modes (bracketed paste and mouse
/// reporting) off. Modes the host protocol relies on (focus reporting, win32 input, kitty keyboard) stay.
const SOFT_RESET: &[u8] = b"\x1b[?25h\x1b[4l\x1b[?6l\x1b[?7h\x1b[?1l\x1b>\x1b[r\x1b[0m\x0f\
    \x1b(B\x1b)B\x1b*B\x1b+B\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l";

impl TerminalCore {
    /// The options of alacritty's `Term` for `config`, with `history` lines of scrollback allowed.
    fn term_config(config: &CoreConfig, history: usize) -> Config {
        Config {
            scrolling_history: history,
            default_cursor_style: config.cursor_style,
            semantic_escape_chars: config.semantic_escape_chars.clone(),
            kitty_keyboard: config.kitty_keyboard,
            osc52: config.osc52,
            ..Config::default()
        }
    }

    pub fn new(columns: usize, rows: usize, config: CoreConfig) -> Self {
        let proxy = EventProxy::default();
        let history_cap = Self::scrollback_limit_for(&config, TermDims::new(columns, rows).columns);
        let term_config = Self::term_config(&config, history_cap + HISTORY_SLACK);
        let mut term = Term::new(term_config, &TermDims::new(columns, rows), proxy.clone());
        term.is_focused = true;
        Self {
            term,
            processor: ansi::Processor::new(),
            sniffer: Sniffer::new(),
            guard: StringGuard::default(),
            proxy,
            agent: AgentStatus::default(),
            win32_input_mode: false,
            mouse_extensions: MouseExtensions::default(),
            events: Vec::new(),
            config,
            dirty: true,
            history_cap,
            history_limit: history_cap + HISTORY_SLACK,
            line_base: 0,
            line_epoch: 0,
            prompts: Vec::new(),
        }
    }

    fn scrollback_limit_for(config: &CoreConfig, columns: usize) -> usize {
        config.scrollback_lines.min(config.scrollback_bytes / history_row_bytes(columns))
    }

    /// Lines of scrollback kept at the current width: the smaller of `scrollback_lines` and what
    /// `scrollback_bytes` buys.
    pub fn scrollback_limit(&self) -> usize {
        self.history_cap
    }

    // --- byte stream --------------------------------------------------------------------------

    /// Feeds PTY output. The sniffer and the emulator see exactly the same bytes in the same order;
    /// the stream is cut after every sequence that needs ordering so replies stay in stream order.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.flush_expired_sync(Instant::now());
        let received = bytes.len();
        let filtered = self.guard.filter(bytes);
        let bytes: &[u8] = &filtered;
        let mut pos = 0;
        while pos < bytes.len() {
            let end = (pos + FEED_CHUNK).min(bytes.len());
            let consumed = self.sniffer.advance(&bytes[pos..end]);
            self.advance_emulator(&bytes[pos..pos + consumed]);
            self.pump();
            self.enforce_scrollback();
            pos += consumed;
        }
        // Same rule as alacritty's own event loop: bytes parked in a synchronized update do not
        // change what is on screen.
        if received > 0 && self.processor.sync_bytes_count() < received {
            self.dirty = true;
        }
    }

    /// alacritty_terminal has reachable panics (see the notes). A panic inside the parser must not take the
    /// PTY forwarder thread down, so it is caught here: the emulator is reset and the host is told.
    /// Requires `panic = "unwind"` (the default) in the release profile.
    fn recover_from_panic(&mut self, payload: Box<dyn std::any::Any + Send>) {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_owned());
        self.term.reset_state();
        self.processor = ansi::Processor::new();
        self.proxy.0.lock().expect("event queue poisoned").clear();
        self.events.push(CoreEvent::EmulatorPanic(message));
        self.reset_line_ids();
        self.dirty = true;
    }

    /// Runs `bytes` through the emulator, containing upstream panics.
    fn advance_emulator(&mut self, bytes: &[u8]) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.processor.advance(&mut self.term, bytes)
        }));
        if let Err(payload) = outcome {
            self.recover_from_panic(payload);
        }
    }

    // --- scrollback and line identity ----------------------------------------------------------

    /// Trims the scrollback to `history_cap`, counting what it drops so `LineId`s stay valid. alacritty
    /// itself drops lines only past `history_cap + HISTORY_SLACK`; if it got there (one huge synchronized
    /// update, thousands of `CSI n S` in one chunk) the count is unknown and the ids are reset instead.
    fn enforce_scrollback(&mut self) {
        // The alternate screen has no scrollback, and the primary grid is not the active one.
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let limit = self.history_cap + HISTORY_SLACK;
        let history = self.term.history_size();
        if history >= self.history_limit {
            self.reset_line_ids();
        }
        if history > self.history_cap {
            self.line_base += (history - self.history_cap) as u64;
            self.term.grid_mut().update_history(self.history_cap);
            self.history_limit = self.history_cap;
            // Ids grow downward, so the prompts that were trimmed are the oldest ones.
            let trimmed = self.prompts.partition_point(|id| id.index < self.line_base);
            self.prompts.drain(..trimmed);
        }
        if self.history_limit != limit {
            self.term.grid_mut().update_history(limit);
            self.history_limit = limit;
        }
    }

    fn reset_line_ids(&mut self) {
        self.line_epoch = self.line_epoch.wrapping_add(1);
        self.line_base = 0;
        self.prompts.clear();
        self.events.push(CoreEvent::LineIdsReset);
    }

    /// Identity of grid line `line` (0 is the top of the screen, negative lines are scrollback). `None` on
    /// the alternate screen and for lines outside the buffer.
    pub fn line_id(&self, line: i32) -> Option<LineId> {
        let history = self.term.history_size() as i64;
        let in_buffer = (-history..self.term.screen_lines() as i64).contains(&i64::from(line));
        if self.term.mode().contains(TermMode::ALT_SCREEN) || !in_buffer {
            return None;
        }
        let index = self.line_base as i64 + history + i64::from(line);
        Some(LineId { epoch: self.line_epoch, index: index as u64 })
    }

    /// The grid line that currently holds `id`; `None` once it was trimmed from the scrollback, belongs to an
    /// older epoch, or the alternate screen is active.
    pub fn line_of(&self, id: LineId) -> Option<i32> {
        if id.epoch != self.line_epoch || self.term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }
        let history = self.term.history_size() as i64;
        let line = id.index as i64 - self.line_base as i64 - history;
        (-history..self.term.screen_lines() as i64).contains(&line).then_some(line as i32)
    }

    /// The oldest line still in the scrollback (or the top screen row when there is none).
    pub fn oldest_line_id(&self) -> Option<LineId> {
        self.line_id(-(self.term.history_size() as i32))
    }

    /// Scrolls the view so grid line `line` is the top row, as far as the buffer allows.
    pub fn scroll_to_line(&mut self, line: i32) {
        let target = i64::from(-line).clamp(0, self.term.history_size() as i64);
        let delta = target - self.display_offset() as i64;
        self.scroll_display(Scroll::Delta(delta as i32));
    }

    /// Scrolls the view to the nearest prompt the shell marked above the top row of the view (`forward` false) or
    /// below it, so the prompt becomes the top row, as far as the buffer allows (a prompt on the live screen can only
    /// bring the view back to the bottom). With no prompt below, `forward` goes to the live screen. True when the
    /// view moved. The alternate screen has no marks: nothing moves there.
    pub fn scroll_to_prompt(&mut self, forward: bool) -> bool {
        let top = -(self.display_offset() as i32);
        let lines = self.prompts.iter().filter_map(|id| self.line_of(*id));
        let target =
            if forward { lines.filter(|line| *line > top).min() } else { lines.filter(|line| *line < top).max() };
        let before = self.display_offset();
        match target {
            Some(line) => self.scroll_to_line(line),
            None if forward && !self.term.mode().contains(TermMode::ALT_SCREEN) => self.scroll_display(Scroll::Bottom),
            None => {}
        }
        self.display_offset() != before
    }

    /// The lines of the prompts the shell marked that are still in the buffer, oldest first (grid lines: negative
    /// ones are in the scrollback).
    pub fn prompt_lines(&self) -> Vec<i32> {
        self.prompts.iter().filter_map(|id| self.line_of(*id)).collect()
    }

    /// Clears the screen and the scrollback ("Clear buffer"). The alternate screen is left alone: a full-screen
    /// program owns it, has no scrollback and is the one to redraw. False when nothing was cleared.
    pub fn clear_buffer(&mut self) -> bool {
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return false;
        }
        // Home, erase the display (alacritty keeps it as history), erase the history: through `feed`, so the
        // sniffer sees the last one and the line ids and prompt marks go with the lines.
        self.feed(b"\x1b[H\x1b[2J\x1b[3J");
        true
    }

    /// Mouse modes beyond `mode()`; see `MouseExtensions`.
    pub fn mouse_extensions(&self) -> MouseExtensions {
        self.mouse_extensions
    }

    /// DECSTR (`CSI ! p`), which alacritty_terminal does not implement. Cursor position and screen content stay.
    fn soft_reset(&mut self) {
        let (line, column) = self.cursor_cell();
        self.advance_emulator(SOFT_RESET);
        // DECSTBM homes the cursor; DECSTR must not move it.
        self.advance_emulator(format!("\x1b[{};{}H", line + 1, column + 1).as_bytes());
        self.mouse_extensions = MouseExtensions::default();
    }

    /// When the current synchronized update (DEC mode 2026) must be flushed, if one is open.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    pub fn is_sync_pending(&self) -> bool {
        self.sync_deadline().is_some()
    }

    /// Applies a synchronized update whose 150 ms timeout has passed. Call it from the host's timer
    /// at `sync_deadline()`; `feed` also calls it first, because alacritty only checks the deadline
    /// when its poll times out. Returns whether anything was flushed.
    pub fn flush_expired_sync(&mut self, now: Instant) -> bool {
        match self.sync_deadline() {
            Some(deadline) if deadline <= now => {
                self.processor.stop_sync(&mut self.term);
                self.pump();
                self.enforce_scrollback();
                self.dirty = true;
                true
            }
            _ => false,
        }
    }

    fn pump(&mut self) {
        let queued = std::mem::take(&mut *self.proxy.0.lock().expect("event queue poisoned"));
        for event in queued {
            self.translate(event);
        }
        for sniffed in self.sniffer.drain() {
            self.handle_sniffed(sniffed);
        }
    }

    fn reply(&mut self, text: impl Into<String>) {
        self.events.push(CoreEvent::PtyWrite(text.into().into_bytes()));
    }

    fn translate(&mut self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => match (&self.config.da1_reply, text.as_str()) {
                (Some(custom), "\x1b[?6c") => {
                    let custom = custom.clone();
                    self.reply(custom);
                }
                // alacritty answers DECRQM for the mouse modes only the sniffer tracks with "not recognized";
                // `Sniffed::MouseModeQuery` sends the real answer.
                (_, "\x1b[?9;0$y" | "\x1b[?1015;0$y" | "\x1b[?1016;0$y") => {}
                _ => self.reply(text),
            },
            AlacEvent::Title(title) => self.events.push(CoreEvent::Title(strip_control(&title))),
            AlacEvent::ResetTitle => self.events.push(CoreEvent::ResetTitle),
            AlacEvent::Bell => self.events.push(CoreEvent::Bell),
            AlacEvent::ClipboardStore(ty, text) => {
                self.events.push(CoreEvent::ClipboardStore { clipboard: ty.into(), text })
            }
            AlacEvent::ClipboardLoad(ty, reply) => {
                self.events.push(CoreEvent::ClipboardLoad { clipboard: ty.into(), reply })
            }
            AlacEvent::ColorRequest(slot, format) => {
                let color = self.config.palette.slot(self.term.colors(), slot);
                self.reply(format(color));
            }
            AlacEvent::TextAreaSizeRequest(format) => {
                let (cell_width, cell_height) = self.config.cell_size_px;
                self.reply(format(WindowSize {
                    num_lines: self.term.screen_lines() as u16,
                    num_cols: self.term.columns() as u16,
                    cell_width,
                    cell_height,
                }));
            }
            AlacEvent::CursorBlinkingChange => self.events.push(CoreEvent::CursorBlinkingChange),
            AlacEvent::MouseCursorDirty => self.events.push(CoreEvent::MouseCursorDirty),
            // Only alacritty's own EventLoop sends these; Term never does.
            AlacEvent::Wakeup | AlacEvent::Exit | AlacEvent::ChildExit(_) => {}
        }
    }

    fn handle_sniffed(&mut self, sniffed: Sniffed) {
        match sniffed {
            Sniffed::Cwd(path) => self.events.push(CoreEvent::Cwd(path)),
            Sniffed::Notification(n) => self.events.push(CoreEvent::Notification(n)),
            Sniffed::Progress(p) => self.events.push(CoreEvent::Progress(p)),
            Sniffed::PromptMark(mark) => {
                if mark == PromptMark::PromptStart && self.agent.drop_active() {
                    self.events.push(CoreEvent::AgentRecordsDropped);
                }
                // While a synchronized update is open the emulator has not applied the bytes yet,
                // so its cursor is stale.
                let cursor = (!self.is_sync_pending()).then(|| {
                    let point = self.term.grid().cursor.point;
                    MarkCursor {
                        line: point.line.0,
                        column: point.column.0,
                        history_size: self.term.history_size(),
                        line_id: self.line_id(point.line.0),
                    }
                });
                if mark == PromptMark::PromptStart
                    && let Some(id) = cursor.and_then(|cursor| cursor.line_id)
                    && let Err(at) = self.prompts.binary_search(&id)
                {
                    self.prompts.insert(at, id);
                }
                self.events.push(CoreEvent::PromptMark { mark, cursor });
            }
            Sniffed::Agent(report) => {
                if self.agent.apply(&report) {
                    self.events.push(CoreEvent::Agent(report));
                }
            }
            Sniffed::AgentQuery { bell_terminated } => {
                let terminator = if bell_terminated { "\x07" } else { "\x1b\\" };
                self.reply(format!("\x1b]7501;?{terminator}"));
            }
            Sniffed::XtVersionQuery => {
                let identity = format!("{} {}", self.config.terminal_name, self.config.terminal_version);
                self.reply(format!("\x1bP>|{identity}\x1b\\"));
            }
            Sniffed::ColorSchemeQuery => {
                let dark = self.config.palette.background.luminance() < 0.5;
                self.reply(format!("\x1b[?997;{}n", if dark { 1 } else { 2 }));
            }
            // "Invalid request" replies: programs that probe get an answer instead of timing out.
            Sniffed::DecrqssQuery => self.reply("\x1bP0$r\x1b\\"),
            Sniffed::XtgettcapQuery => self.reply("\x1bP0+r\x1b\\"),
            Sniffed::FullReset => {
                self.win32_input_mode = false;
                self.mouse_extensions = MouseExtensions::default();
                self.reset_line_ids();
                if self.agent.clear() {
                    self.events.push(CoreEvent::AgentRecordsDropped);
                }
            }
            Sniffed::SoftReset => self.soft_reset(),
            Sniffed::ClearScrollback => {
                // Only the primary screen has scrollback to clear.
                if !self.term.mode().contains(TermMode::ALT_SCREEN) {
                    self.reset_line_ids();
                }
            }
            Sniffed::MouseExtension(mode, on) => match mode {
                MouseExtension::X10 => self.mouse_extensions.x10 = on,
                MouseExtension::Urxvt => self.mouse_extensions.urxvt = on,
                MouseExtension::SgrPixels => self.mouse_extensions.sgr_pixels = on,
            },
            // DECSET or DECRST of 1000/1002/1003 replaces X10 reporting.
            Sniffed::MouseTracking => self.mouse_extensions.x10 = false,
            Sniffed::MouseModeQuery(mode) => {
                let (number, set) = match mode {
                    MouseExtension::X10 => (9, self.mouse_extensions.x10),
                    MouseExtension::Urxvt => (1015, self.mouse_extensions.urxvt),
                    MouseExtension::SgrPixels => (1016, self.mouse_extensions.sgr_pixels),
                };
                self.reply(format!("\x1b[?{number};{}$y", if set { 1 } else { 2 }));
            }
            Sniffed::Win32InputMode(on) => {
                self.win32_input_mode = on;
                self.events.push(CoreEvent::Win32InputMode(on));
            }
            Sniffed::Unsupported(feature) => self.events.push(CoreEvent::Unsupported(feature)),
        }
    }

    /// Takes everything the host must act on, in order.
    pub fn take_events(&mut self) -> Vec<CoreEvent> {
        std::mem::take(&mut self.events)
    }

    /// True when the visible content may have changed since the consumer last called `take_dirty`.
    /// The producer thread peeks with this; only the thread that draws should clear the flag.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears and returns the dirty flag (call it from the drawing side).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    // --- size, scroll, state ------------------------------------------------------------------

    /// Resizes the grid (reflowing the primary screen). The caller must resize the PTY as well.
    ///
    /// A narrower screen wraps its lines into more rows, and alacritty keeps the bottom of the screen: the top rows
    /// go into the scrollback even when blank rows under the cursor could have taken the new ones. A program that
    /// redraws from the top left (Claude Code in classic mode, anything that clears and redraws) then finds its old
    /// screen above the new one. `reflow_into_blank_rows` gives those rows back to the screen. The alternate screen
    /// is never reflowed; the primary screen hidden under it is, in the same way as when it is shown.
    pub fn resize(&mut self, columns: usize, rows: usize) {
        let old_columns = self.term.columns();
        let target = TermDims::new(columns, rows);
        let narrower = target.columns < old_columns;
        if narrower && self.term.mode().contains(TermMode::ALT_SCREEN) {
            self.resize_hidden_primary(target);
        } else {
            if narrower {
                self.clear_rows_the_cut_drops(target.screen_lines);
                self.reflow_into_blank_rows(target.columns);
            }
            self.term.resize(target);
        }
        if self.term.columns() != old_columns {
            // Reflow re-wraps the buffer, so rows no longer mean what they did; the budget buys a different
            // number of lines at the new width.
            self.reset_line_ids();
            self.history_cap = Self::scrollback_limit_for(&self.config, self.term.columns());
        }
        // Fewer rows push lines into the scrollback, and a new cap needs a new grid limit.
        self.enforce_scrollback();
        self.dirty = true;
    }

    /// Narrows the active (primary) screen to `columns` at its current height. The reflow pushes the rows that no
    /// longer fit into the scrollback; of those, as many as there are blank rows at the bottom of the screen, below
    /// the cursor, are pulled back: the screen shrinks by that many rows, which drops the blank ones without
    /// pushing anything, and grows again, which takes the same number of lines from the scrollback. The content
    /// ends where it would have if the program had drawn it at the new width, with no more scrollback than that.
    fn reflow_into_blank_rows(&mut self, columns: usize) {
        let rows = self.term.screen_lines();
        let screen_pushes = self.rows_pushed_by_the_screen(columns);
        let history = self.term.history_size();
        self.term.resize(TermDims::new(columns, rows));
        let pushed = self.term.history_size().saturating_sub(history).min(screen_pushes);
        let grid = self.term.grid();
        let cursor_row = usize::try_from(grid.cursor.point.line.0).unwrap_or(0);
        let blank = (cursor_row + 1..rows).rev().take_while(|row| grid[Line(*row as i32)].is_clear()).count();
        let given_back = pushed.min(blank);
        if given_back > 0 {
            self.term.resize(TermDims::new(columns, rows - given_back));
            self.term.resize(TermDims::new(columns, rows));
        }
    }

    /// The rows the screen alone pushes into the scrollback when it narrows to `columns` at its current height.
    /// Reflowing the real grid also re-wraps the scrollback, and the rows that adds are not rows the screen gave
    /// up: counting them would pull old scrollback back onto a screen that has blank rows (and the program's next
    /// redraw from the top left then erases it). The screen is reflowed on a grid of its own for the count.
    fn rows_pushed_by_the_screen(&self, columns: usize) -> usize {
        let grid = self.term.grid();
        let (rows, old_columns) = (grid.screen_lines(), grid.columns());
        let mut screen: Grid<Cell> = Grid::new(rows, old_columns, rows * old_columns);
        for row in 0..rows {
            let line = Line(row as i32);
            screen[line] = grid[line].clone();
        }
        screen.cursor = grid.cursor.clone();
        screen.saved_cursor = grid.saved_cursor.clone();
        screen.resize(true, rows, columns);
        screen.history_size()
    }

    /// Clears the rows under the cursor that a cut to `rows` rows drops anyway. alacritty changes the rows before
    /// the columns, so those rows never wrap; reflowing them first would wrap the ones that hold something into the
    /// scrollback, and the blank ones that could have taken the wrapping of the rows above would be spent by the cut.
    fn clear_rows_the_cut_drops(&mut self, rows: usize) {
        let cursor_row = usize::try_from(self.term.grid().cursor.point.line.0).unwrap_or(0);
        for row in rows.max(cursor_row + 1)..self.term.screen_lines() {
            self.term.grid_mut()[Line(row as i32)].reset(&Cell::default());
        }
    }

    /// `resize` to a narrower `target` while the alternate screen is shown: the primary screen under it is reflowed
    /// like a shown one. `swap_alt` clears nothing while the alternate screen is shown, so the alternate grid is taken
    /// out (a blank one holds its place) and the primary grid becomes the active one for the work. Swapping back
    /// clears the blank grid that goes inactive; the alternate grid then returns, cut or padded to the new size and
    /// not reflowed, with its content and cursor as the program left them.
    fn resize_hidden_primary(&mut self, target: TermDims) {
        let blank = Grid::new(self.term.screen_lines(), self.term.columns(), 0);
        let mut alternate = mem::replace(self.term.grid_mut(), blank);
        self.term.swap_alt();
        self.clear_rows_the_cut_drops(target.screen_lines);
        self.reflow_into_blank_rows(target.columns);
        self.term.resize(target);
        self.term.swap_alt();
        alternate.resize(false, target.screen_lines, target.columns);
        *self.term.grid_mut() = alternate;
    }

    pub fn scroll_display(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
        self.pump();
        self.dirty = true;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.term.is_focused = focused;
        self.dirty = true;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.config.palette = palette;
        self.dirty = true;
    }

    /// Changes what OSC 52 may do in a terminal that already runs (the setting changed).
    pub fn set_osc52(&mut self, osc52: Osc52) {
        self.config.osc52 = osc52;
        self.term.set_options(Self::term_config(&self.config, self.history_limit));
        self.dirty = true;
    }

    /// Changes how many lines of scrollback are kept, in a terminal that already runs (the setting changed): a smaller
    /// number drops the oldest lines at once, like a narrower terminal with a smaller budget does.
    pub fn set_scrollback_lines(&mut self, lines: usize) {
        self.config.scrollback_lines = lines;
        self.history_cap = Self::scrollback_limit_for(&self.config, self.term.columns());
        self.enforce_scrollback();
        self.dirty = true;
    }

    /// Changes the cursor a program that chose none gets, in a terminal that already runs (the setting changed). A
    /// cursor a program set with DECSCUSR stays as it set it until it asks for the default again.
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.config.cursor_style = style;
        self.term.set_options(Self::term_config(&self.config, self.history_limit));
        self.dirty = true;
    }

    pub fn set_cell_size_px(&mut self, width: u16, height: u16) {
        self.config.cell_size_px = (width, height);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Whether the program (ConPTY, in practice) asked for win32-input-mode key events.
    pub fn win32_input_mode(&self) -> bool {
        self.win32_input_mode
    }

    /// Active kitty keyboard flags in protocol bit order (1 disambiguate, 2 event types,
    /// 4 alternate keys, 8 all keys as escape codes, 16 associated text).
    pub fn kitty_flags(&self) -> u8 {
        let mode = *self.term.mode();
        [
            (TermMode::DISAMBIGUATE_ESC_CODES, 1),
            (TermMode::REPORT_EVENT_TYPES, 2),
            (TermMode::REPORT_ALTERNATE_KEYS, 4),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
            (TermMode::REPORT_ASSOCIATED_TEXT, 16),
        ]
        .into_iter()
        .filter(|(flag, _)| mode.contains(*flag))
        .fold(0, |bits, (_, bit)| bits | bit)
    }

    /// Pops the whole kitty keyboard stack of the active screen (a crashed program leaves its flags).
    pub fn reset_keyboard_modes(&mut self) {
        self.term.pop_keyboard_modes(u16::MAX);
    }

    /// Cursor in viewport coordinates `(row, column)`, ignoring display_offset.
    pub fn cursor_cell(&self) -> (usize, usize) {
        let point = self.term.grid().cursor.point;
        (point.line.0 as usize, point.column.0)
    }

    pub fn columns(&self) -> usize {
        self.term.columns()
    }

    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.history_size()
    }

    /// Another program takes the place of the one that ran. What the old one left in the emulator goes: the alternate
    /// screen, the modes it set (mouse reports, bracketed paste, a changed scroll region), a keyboard protocol it
    /// pushed and the reports of an agent. The screen it drew moves into the scrollback, so it stays to be read, and
    /// the new program starts on a clear screen: it draws only the cells it means to, and anything left there would
    /// stay.
    pub fn reset_for_new_session(&mut self) {
        self.feed(b"\x1b[?1049l\x1b[!p\x1b[?25h\x1b[0m\x1b[H\x1b[2J");
        self.reset_keyboard_modes();
        if self.agent.clear() {
            self.events.push(CoreEvent::AgentRecordsDropped);
        }
    }

    /// The child exited: `working` / `blocked` records and a stale keyboard mode do not survive it.
    pub fn on_child_exit(&mut self) {
        if self.agent.drop_active() {
            self.events.push(CoreEvent::AgentRecordsDropped);
        }
        self.reset_keyboard_modes();
    }

    pub fn agent_summary(&self) -> Option<&AgentRecord> {
        self.agent.summary()
    }

    pub fn agent_mut(&mut self) -> &mut AgentStatus {
        &mut self.agent
    }

    // --- rendering ----------------------------------------------------------------------------

    /// Copies the visible grid into `out` (buffers are reused) and consumes the damage.
    ///
    /// This deliberately does not call `Term::renderable_content()`: its `RenderableCursor::new` does
    /// `point.column -= 1` when the cell under the cursor is a wide-char spacer, which underflows at
    /// column 0 (reachable with `ESC[P` on a wide glyph) and panics in debug builds.
    pub fn snapshot(&mut self, out: &mut Snapshot) {
        let columns = self.term.columns();
        let rows = self.term.screen_lines();
        let margin = self.config.damage_margin;
        out.damage = match self.term.damage() {
            TermDamage::Full => Damage::Full,
            TermDamage::Partial(lines) => Damage::Lines(
                lines
                    .map(|l| LineDamage {
                        row: l.line,
                        left: l.left.saturating_sub(margin),
                        right: (l.right + margin).min(columns - 1),
                    })
                    .collect(),
            ),
        };
        self.term.reset_damage();

        let grid = self.term.grid();
        let display_offset = grid.display_offset();
        let colors = self.term.colors();
        let mode = *self.term.mode();

        out.columns = columns;
        out.rows = rows;
        out.display_offset = display_offset;
        out.mode = mode;
        out.selection = self.term.selection.as_ref().and_then(|s| s.to_range(&self.term));
        out.cells.clear();
        out.cells.resize(rows * columns, SnapCell::default());
        for indexed in grid.display_iter() {
            let Some(viewport) = point_to_viewport(display_offset, indexed.point) else {
                continue;
            };
            if viewport.line < rows && viewport.column.0 < columns {
                out.cells[viewport.line * columns + viewport.column.0] = snap_cell(indexed.cell);
            }
        }

        let vi_mode = mode.contains(TermMode::VI);
        let mut point = if vi_mode { self.term.vi_mode_cursor.point } else { grid.cursor.point };
        if point.column.0 > 0 && grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column -= 1;
        }
        let style = self.term.cursor_style();
        out.cursor = SnapCursor {
            position: point_to_viewport(display_offset, point)
                .filter(|p| p.line < rows && p.column.0 < columns)
                .map(|p| (p.line, p.column.0)),
            shape: if !vi_mode && !mode.contains(TermMode::SHOW_CURSOR) { CursorShape::Hidden } else { style.shape },
            blinking: style.blinking,
        };
        out.history_size = self.term.history_size();
        out.top_line = self.line_id(-(display_offset as i32));
        out.overrides.clear();
        out.overrides.extend((0..alacritty_terminal::term::color::COUNT).filter_map(|slot| colors[slot].map(|rgb| (slot, rgb))));
    }
    // --- selection ----------------------------------------------------------------------------

    fn grid_point(&self, row: usize, column: usize) -> Point {
        viewport_to_point(self.display_offset(), Point::new(row, Column(column)))
    }

    pub fn selection_start(&mut self, kind: SelectionKind, row: usize, column: usize, side: CellSide) {
        let point = self.grid_point(row, column);
        self.term.selection = Some(Selection::new(kind.into(), point, side.into()));
        self.dirty = true;
    }

    pub fn selection_update(&mut self, row: usize, column: usize, side: CellSide) {
        let point = self.grid_point(row, column);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side.into());
            self.dirty = true;
        }
    }

    pub fn selection_clear(&mut self) {
        self.term.selection = None;
        self.dirty = true;
    }

    /// A selection exists, even an empty one that a drag is still growing.
    pub fn selection_active(&self) -> bool {
        self.term.selection.is_some()
    }

    /// The selected text as the clipboard gets it, `None` when nothing is selected. It comes from the cells, so no
    /// escape sequence can be in it: wide characters appear once, a soft-wrapped line continues without a line
    /// break, trailing blanks are dropped from every line, and a line-wise selection (triple click) carries no
    /// final line break (pasting it would run it). Control and bidirectional-override characters that ended up
    /// in cells are dropped; line feed and tab stay.
    pub fn selection_text(&self) -> Option<String> {
        let text = self.term.selection_to_string()?;
        let by_line = self.term.selection.as_ref().is_some_and(|selection| selection.ty == SelectionType::Lines);
        let text = if by_line { text.strip_suffix('\n').unwrap_or(&text) } else { &text };
        Some(text.chars().filter(|c| matches!(c, '\n' | '\t') || !is_control_or_bidi(*c)).collect())
    }

    /// Selects the whole buffer, scrollback included, down to the last line that has text: the blank rows under
    /// the prompt are not part of what anyone copies. A buffer without text selects nothing.
    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let last_text = (self.term.topmost_line().0..=self.term.bottommost_line().0)
            .rev()
            .find(|line| grid[Line(*line)].line_length().0 > 0);
        self.term.selection = last_text.map(|last| {
            let top = Point::new(self.term.topmost_line(), Column(0));
            let mut selection = Selection::new(SelectionType::Simple, top, Side::Left);
            selection.update(Point::new(Line(last), self.term.last_column()), Side::Right);
            selection
        });
        self.dirty = true;
    }

    /// Text between two grid points, for example a search match or an OSC 8 link.
    pub fn text_between(&self, start: (i32, usize), end: (i32, usize)) -> String {
        let clamp = |p: (i32, usize)| Point::new(Line(p.0), Column(p.1)).grid_clamp(&self.term, Boundary::Grid);
        let (start, end) = (clamp(start), clamp(end));
        if start > end {
            return String::new();
        }
        self.term.bounds_to_string(start, end)
    }

    // --- search -------------------------------------------------------------------------------

    /// Every match in the scrollback and screen, top to bottom. The pattern is a regex; it is
    /// case-insensitive unless it contains an uppercase letter (alacritty's smart case).
    pub fn search_all(&self, pattern: &str, limit: usize) -> Result<Vec<SearchMatch>, String> {
        let mut regex = RegexSearch::new(pattern).map_err(|e| e.to_string())?;
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(self.term.bottommost_line(), self.term.last_column());
        Ok(RegexIter::new(start, end, Direction::Right, &self.term, &mut regex)
            .take(limit)
            .map(SearchMatch::from)
            .collect())
    }

    pub fn search_start(&self, pattern: &str) -> Result<SearchSession, String> {
        RegexSearch::new(pattern).map(|regex| SearchSession { regex }).map_err(|e| e.to_string())
    }

    /// The next match at or after `origin` (grid coordinates, `line < 0` is scrollback), or the previous
    /// one when `forward` is false. Wraps around the whole buffer, so `None` means no match at all.
    pub fn search_next(&self, session: &mut SearchSession, origin: (i32, usize), forward: bool) -> Option<SearchMatch> {
        // Term::search_next indexes the grid with the origin as given and panics outside it.
        let origin = Point::new(Line(origin.0), Column(origin.1)).grid_clamp(&self.term, Boundary::Grid);
        let direction = if forward { Direction::Right } else { Direction::Left };
        self.term.search_next(&mut session.regex, origin, direction, Side::Left, None).map(SearchMatch::from)
    }

    // --- cells --------------------------------------------------------------------------------

    pub fn hyperlink_at(&self, row: usize, column: usize) -> Option<Hyperlink> {
        if row >= self.rows() || column >= self.columns() {
            return None;
        }
        let point = self.grid_point(row, column);
        self.term.grid()[point].hyperlink()
    }

    /// The link under a cell of the viewport, when it is one that may be opened (`links::open_target`): a
    /// hyperlink a program set (OSC 8), with every cell of the viewport that holds it, or else a URL in the text,
    /// which covers several rows when its line wraps.
    pub fn link_at(&self, row: usize, column: usize) -> Option<Link> {
        let (rows, columns) = (self.rows(), self.columns());
        if row >= rows || column >= columns {
            return None;
        }
        let offset = self.display_offset() as i32;
        let grid = self.term.grid();
        let line_of = |row: usize| Line(row as i32 - offset);

        if let Some(hyperlink) = grid[line_of(row)][Column(column)].hyperlink() {
            links::open_target(hyperlink.uri()).ok()?;
            let mut spans = Vec::new();
            for r in 0..rows {
                let cells = &grid[line_of(r)];
                let mut start = None;
                for c in 0..=columns {
                    let linked = c < columns && cells[Column(c)].hyperlink().as_ref() == Some(&hyperlink);
                    match (linked, start) {
                        (true, None) => start = Some(c),
                        (false, Some(first)) => {
                            spans.push((r, first, c));
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
            return Some(Link { uri: hyperlink.uri().to_owned(), spans });
        }

        // A row whose last cell wraps continues on the next row: they are one line of text.
        let wraps = |line: i32| grid[Line(line)][Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        let here = line_of(row).0;
        let (mut first, mut last) = (here, here);
        while here - first < MAX_LINK_ROWS && first > self.term.topmost_line().0 && wraps(first - 1) {
            first -= 1;
        }
        while last - here < MAX_LINK_ROWS && last < self.term.bottommost_line().0 && wraps(last) {
            last += 1;
        }
        let text: Vec<char> = (first..=last)
            .flat_map(|line| {
                let cells = &grid[Line(line)];
                (0..columns).map(move |c| {
                    let cell = &cells[Column(c)];
                    if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                        '\0'
                    } else {
                        cell.c
                    }
                })
            })
            .collect();
        let at = (here - first) as usize * columns + column;
        let found = links::find_urls(&text).into_iter().find(|range| range.contains(&at))?;
        let uri: String = text[found.clone()].iter().collect();
        links::open_target(&uri).ok()?;
        let spans = (0..rows)
            .filter_map(|r| {
                let line = line_of(r).0;
                let base = usize::try_from(line - first).ok()? * columns;
                let (start, end) = (found.start.max(base), found.end.min(base + columns));
                (line <= last && start < end).then(|| (r, start - base, end - base))
            })
            .collect();
        Some(Link { uri, spans })
    }
}

fn snap_cell(cell: &Cell) -> SnapCell {
    SnapCell {
        fg_raw: cell_color(cell.fg),
        bg_raw: cell_color(cell.bg),
        ch: cell.c,
        zerowidth: cell
            .zerowidth()
            .filter(|marks| !marks.is_empty())
            .map(|marks| marks.to_vec().into_boxed_slice()),
        flags: cell.flags,
        underline_color: cell.underline_color().map(cell_color),
        hyperlink: cell.hyperlink(),
    }
}
