//! Renderer input data model: cells, colors, theme, overlays.
//!
//! The emulator owns the real grid; the renderer only ever sees a `GridView`
//! snapshot (row-major cells + a few overlay descriptions). Everything here is
//! plain data so the renderer crate never depends on the emulator crate.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::contrast::{lerp, readable_on};

/// Cell color as the emulator stores it (palette references stay symbolic so a
/// theme change does not require touching the grid).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Color {
    /// Theme foreground/background depending on the slot.
    Default,
    /// 0..=15 ANSI, 16..=231 6x6x6 cube, 232..=255 grayscale ramp.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct CellFlags(pub u16);

impl CellFlags {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const FAINT: u16 = 1 << 2;
    pub const INVERSE: u16 = 1 << 3;
    pub const HIDDEN: u16 = 1 << 4;
    // Bits 5 and 7 are unused: alacritty_terminal keeps no blink (SGR 5) or overline (SGR 53) attribute,
    // so the renderer has nothing to draw for them.
    pub const STRIKEOUT: u16 = 1 << 6;
    /// First cell of a double-width character.
    pub const WIDE: u16 = 1 << 8;
    /// Second cell of a double-width character (carries no glyph).
    pub const WIDE_SPACER: u16 = 1 << 9;
    // Underline styles are mutually exclusive: 3 bits at 10..=12.
    const UL_SHIFT: u16 = 10;
    const UL_MASK: u16 = 0b111 << Self::UL_SHIFT;

    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
    pub fn with(mut self, bit: u16) -> Self {
        self.0 |= bit;
        self
    }
    pub fn underline(self) -> Underline {
        match (self.0 & Self::UL_MASK) >> Self::UL_SHIFT {
            1 => Underline::Single,
            2 => Underline::Double,
            3 => Underline::Curly,
            4 => Underline::Dotted,
            5 => Underline::Dashed,
            _ => Underline::None,
        }
    }
    pub fn with_underline(mut self, u: Underline) -> Self {
        self.0 = (self.0 & !Self::UL_MASK) | ((u as u16) << Self::UL_SHIFT);
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Underline {
    None = 0,
    Single = 1,
    Double = 2,
    Curly = 3,
    Dotted = 4,
    Dashed = 5,
}

/// Rarely used per-cell data, boxed behind an `Arc` so `Cell` stays small and
/// cloning a row is cheap (mirrors alacritty_terminal's `CellExtra`).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CellExtra {
    /// Combining marks, variation selectors, ZWJ and the following emoji parts.
    pub zerowidth: Vec<char>,
    pub underline_color: Option<Color>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
    pub extra: Option<Arc<CellExtra>>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: Color::Default, bg: Color::Default, flags: CellFlags(0), extra: None }
    }
}

impl Hash for Cell {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ch.hash(state);
        self.fg.hash(state);
        self.bg.hash(state);
        self.flags.hash(state);
        if let Some(e) = &self.extra {
            e.hash(state);
        }
    }
}

impl Cell {
    pub fn new(ch: char) -> Self {
        Cell { ch, ..Default::default() }
    }
    pub fn fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    pub fn flag(mut self, bit: u16) -> Self {
        self.flags = self.flags.with(bit);
        self
    }
    pub fn underline(mut self, u: Underline) -> Self {
        self.flags = self.flags.with_underline(u);
        self
    }
    pub fn zerowidth(mut self, chars: &[char]) -> Self {
        let mut e = self.extra.as_deref().cloned().unwrap_or_default();
        e.zerowidth.extend_from_slice(chars);
        self.extra = Some(Arc::new(e));
        self
    }
    pub fn underline_color(mut self, c: Color) -> Self {
        let mut e = self.extra.as_deref().cloned().unwrap_or_default();
        e.underline_color = Some(c);
        self.extra = Some(Arc::new(e));
        self
    }
    pub fn zerowidth_chars(&self) -> &[char] {
        self.extra.as_deref().map(|e| e.zerowidth.as_slice()).unwrap_or(&[])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn from_hex(hex: u32) -> Self {
        Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }
}

/// Resolved colors and rendering policy. Colors are sRGB-encoded bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Rgb,
    pub background: Rgb,
    /// Background alpha for `Color::Default` cells (1.0 = opaque). Values < 1.0
    /// disable LCD text because the backdrop is no longer known (Mica/acrylic).
    pub background_opacity: f32,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection_bg: Rgb,
    /// `None` keeps each cell's own foreground inside the selection.
    pub selection_fg: Option<Rgb>,
    pub search_match_bg: Rgb,
    pub search_match_fg: Rgb,
    pub search_current_bg: Rgb,
    pub search_current_fg: Rgb,
    pub preedit_bg: Rgb,
    pub link_underline: Rgb,
    pub scrollbar_thumb: Rgb,
    pub scrollbar_track_alpha: u8,
    pub ansi: [Rgb; 16],
    /// OSC 4 overrides for the palette slots 16..=255 (index = slot - 16). Slots 0..=15 live in `ansi`.
    pub indexed_overrides: [Option<Rgb>; 240],
    /// SGR 1 on ANSI 0..=7 selects the bright variant. This is the only place that decides it.
    pub bold_is_bright: bool,
    /// 0.0..=1.0: how much SGR 2 (faint) pulls the foreground toward the background. The only place that
    /// decides how faint text looks.
    pub faint_amount: f32,
    /// Lowest WCAG contrast ratio between a cell's text and its background (1.0 = off, 4.5 = WCAG AA).
    /// Text below it is moved toward white or black. Box drawing and Powerline glyphs, selection, search
    /// highlights, the cursor and faint text are exempt.
    pub min_contrast: f32,
    /// Roughly what is behind a translucent window (the blurred Mica or acrylic average, for example a dark
    /// grey for a dark system theme). Only `min_contrast` uses it, to judge default-background text against
    /// the colour the user actually sees. `None` judges against `background`.
    pub backdrop_hint: Option<Rgb>,
}

impl Theme {
    /// Campbell (Windows Terminal default scheme).
    pub fn campbell() -> Self {
        let h = Rgb::from_hex;
        Theme::from_parts(
            h(0x0C0C0C),
            h(0xCCCCCC),
            h(0xFFFFFF),
            h(0xFFFFFF),
            [
                h(0x0C0C0C),
                h(0xC50F1F),
                h(0x13A10E),
                h(0xC19C00),
                h(0x0037DA),
                h(0x881798),
                h(0x3A96DD),
                h(0xCCCCCC),
                h(0x767676),
                h(0xE74856),
                h(0x16C60C),
                h(0xF9F1A5),
                h(0x3B78FF),
                h(0xB4009E),
                h(0x61D6D6),
                h(0xF2F2F2),
            ],
        )
    }

    /// A full theme from the five values a colour scheme carries. Everything else is derived so the
    /// result is legible for any scheme: text on the cursor and on the selection is whichever of the
    /// scheme's own colours is readable there, search highlights keep fixed mid-tone backgrounds.
    pub fn from_parts(background: Rgb, foreground: Rgb, cursor: Rgb, selection: Rgb, ansi: [Rgb; 16]) -> Self {
        let search_match_bg = Rgb::from_hex(0x3A5E87);
        let search_current_bg = Rgb::from_hex(0xF9A825);
        let text = [Rgb(255, 255, 255), Rgb(0, 0, 0)];
        Theme {
            foreground,
            background,
            background_opacity: 1.0,
            cursor,
            cursor_text: readable_on(cursor, &[background, foreground]),
            selection_bg: selection,
            selection_fg: Some(readable_on(selection, &[background, foreground])),
            search_match_bg,
            search_match_fg: readable_on(search_match_bg, &text),
            search_current_bg,
            search_current_fg: readable_on(search_current_bg, &text),
            preedit_bg: lerp(background, foreground, 0.16),
            link_underline: ansi[12],
            scrollbar_thumb: lerp(background, foreground, 0.74),
            scrollbar_track_alpha: 0x20,
            ansi,
            indexed_overrides: [None; 240],
            bold_is_bright: true,
            faint_amount: 0.5,
            min_contrast: 1.0,
            backdrop_hint: None,
        }
    }

    /// The colour a default-background cell shows: `background` mixed with `backdrop_hint` by the opacity.
    pub fn effective_background(&self) -> Rgb {
        match self.backdrop_hint {
            Some(hint) if self.background_opacity < 1.0 => lerp(hint, self.background, self.background_opacity),
            _ => self.background,
        }
    }

    /// Opacity of the default background, clamped to 0.0..=1.0. Below 1.0 the pane lets the window's
    /// backdrop (Mica, acrylic) show through and text is drawn with grayscale anti-aliasing.
    pub fn set_background_opacity(&mut self, opacity: f32) {
        self.background_opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
    }

    /// OSC 4: overrides palette slot `slot` (0..=15 replace the scheme colour, 16..=255 the xterm table).
    pub fn set_indexed(&mut self, slot: u8, color: Rgb) {
        match slot {
            0..=15 => self.ansi[slot as usize] = color,
            _ => self.indexed_overrides[slot as usize - 16] = Some(color),
        }
    }

    /// xterm 256-color formula for indices 16..=255, unless OSC 4 replaced the slot.
    pub fn indexed(&self, i: u8) -> Rgb {
        if let Some(color) = i.checked_sub(16).and_then(|slot| self.indexed_overrides[slot as usize]) {
            return color;
        }
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            232..=255 => {
                let v = 8 + 10 * (i - 232);
                Rgb(v, v, v)
            }
        }
    }

    pub fn resolve_fg(&self, c: Color) -> Rgb {
        match c {
            Color::Default => self.foreground,
            Color::Indexed(i) => self.indexed(i),
            Color::Rgb(r, g, b) => Rgb(r, g, b),
        }
    }

    pub fn resolve_bg(&self, c: Color) -> Rgb {
        match c {
            Color::Default => self.background,
            Color::Indexed(i) => self.indexed(i),
            Color::Rgb(r, g, b) => Rgb(r, g, b),
        }
    }
}

// ---------------------------------------------------------------------------
// Overlays: state that changes without the grid changing.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CursorView {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
    /// The blink phase, driven by the caller: false while the blink is in its "off" phase. The renderer
    /// has no timers; toggling this is enough to hide or show the cursor.
    pub visible: bool,
    /// Unfocused panes draw a hollow block regardless of `shape`.
    pub focused: bool,
    /// Cursor sits on the first half of a wide char: cover both cells.
    pub wide: bool,
}

/// Half-open column range `[start, end)` on one row.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RowSpan {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Highlight {
    pub span: RowSpan,
    /// The match the next/previous-result cursor is on.
    pub current: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Preedit {
    pub row: u16,
    pub col: u16,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Scrollbar {
    /// Thumb top/height as fractions of the track (0..=1).
    pub top: f32,
    pub height: f32,
    pub hovered: bool,
}

/// What a mark on the scrollbar stands for: a prompt the shell marked, a match of the find bar, the match it is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkKind {
    Prompt,
    /// A command that ended with a failure: where its output starts.
    Failed,
    Match,
    CurrentMatch,
}

/// A mark on the track of the scrollbar, `at` that fraction (0..=1) of the whole buffer from its top.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScrollMark {
    pub at: f32,
    pub kind: MarkKind,
}

pub struct GridView<'a> {
    pub cols: usize,
    pub rows: usize,
    /// Row-major, `cols * rows` long.
    pub cells: &'a [Cell],
    pub cursor: Option<CursorView>,
    pub selection: &'a [RowSpan],
    pub highlights: &'a [Highlight],
    /// The cells of the link under the pointer, one span per row it covers (a link that wraps covers several).
    pub hover_link: &'a [RowSpan],
    pub preedit: Option<&'a Preedit>,
    pub scrollbar: Option<Scrollbar>,
    /// Marks on the track of the scrollbar; drawn only with it.
    pub marks: &'a [ScrollMark],
    /// The top row is a header pinned over the grid (sticky scroll): a thin line along its bottom edge parts it from
    /// the rows under it.
    pub sticky_header: bool,
}

impl<'a> GridView<'a> {
    pub fn plain(cols: usize, rows: usize, cells: &'a [Cell]) -> Self {
        GridView {
            cols,
            rows,
            cells,
            cursor: None,
            selection: &[],
            highlights: &[],
            hover_link: &[],
            preedit: None,
            scrollbar: None,
            marks: &[],
            sticky_header: false,
        }
    }
}
