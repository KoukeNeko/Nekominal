//! Adapter from `nekominal_term::Snapshot` to `nekominal_render::GridView` (seed of nekominal-pane::view).
//!
//! Colours stay symbolic (`CellColor`): the renderer's `Theme` is the only place that resolves them, so
//! INVERSE / HIDDEN / FAINT / bold-is-bright are applied exactly once and `Default` background cells can be
//! drawn through the single pane background rect.

use std::sync::Arc;

use nekominal_render as r;
use nekominal_term::{
    CellColor, Snapshot,
    alacritty_terminal::{term::cell::Flags, vte::ansi::CursorShape},
};

fn color(c: CellColor) -> r::Color {
    match c {
        CellColor::Default => r::Color::Default,
        CellColor::Indexed(i) => r::Color::Indexed(i),
        CellColor::Rgb(red, green, blue) => r::Color::Rgb(red, green, blue),
    }
}

fn flags(f: Flags) -> r::CellFlags {
    let mut out = r::CellFlags(0);
    for (from, to) in [
        (Flags::BOLD, r::CellFlags::BOLD),
        (Flags::ITALIC, r::CellFlags::ITALIC),
        (Flags::DIM, r::CellFlags::FAINT),
        (Flags::INVERSE, r::CellFlags::INVERSE),
        (Flags::HIDDEN, r::CellFlags::HIDDEN),
        (Flags::STRIKEOUT, r::CellFlags::STRIKEOUT),
        (Flags::WIDE_CHAR, r::CellFlags::WIDE),
        (Flags::WIDE_CHAR_SPACER, r::CellFlags::WIDE_SPACER),
    ] {
        if f.contains(from) {
            out = out.with(to);
        }
    }
    let underline = if f.contains(Flags::UNDERCURL) {
        r::Underline::Curly
    } else if f.contains(Flags::DOUBLE_UNDERLINE) {
        r::Underline::Double
    } else if f.contains(Flags::DOTTED_UNDERLINE) {
        r::Underline::Dotted
    } else if f.contains(Flags::DASHED_UNDERLINE) {
        r::Underline::Dashed
    } else if f.contains(Flags::UNDERLINE) {
        r::Underline::Single
    } else {
        r::Underline::None
    };
    out.with_underline(underline)
}

/// Fills `cells` (reused between frames) with the renderer's view of the snapshot.
pub fn fill_cells(snapshot: &Snapshot, cells: &mut Vec<r::Cell>) {
    cells.clear();
    cells.reserve(snapshot.cells.len());
    for cell in &snapshot.cells {
        let leading_spacer = cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER);
        let zerowidth = cell.zerowidth.as_deref().filter(|z| !z.is_empty());
        let underline_color = cell.underline_color.map(color);
        let extra = (zerowidth.is_some() || underline_color.is_some()).then(|| {
            Arc::new(r::CellExtra { zerowidth: zerowidth.map(<[char]>::to_vec).unwrap_or_default(), underline_color })
        });
        cells.push(r::Cell {
            ch: if leading_spacer { ' ' } else { cell.ch },
            fg: color(cell.fg_raw),
            bg: color(cell.bg_raw),
            flags: flags(cell.flags),
            extra,
        });
    }
}

pub fn cursor(snapshot: &Snapshot, focused: bool, visible: bool) -> Option<r::CursorView> {
    let (row, col) = snapshot.cursor.position?;
    let shape = match snapshot.cursor.shape {
        CursorShape::Hidden => return None,
        CursorShape::Block | CursorShape::HollowBlock => r::CursorShape::Block,
        CursorShape::Underline => r::CursorShape::Underline,
        CursorShape::Beam => r::CursorShape::Bar,
    };
    let wide = snapshot.cell(row, col).flags.contains(Flags::WIDE_CHAR);
    Some(r::CursorView { col: col as u16, row: row as u16, shape, visible, focused, wide })
}

/// How far an unfocused pane's text colours move toward its background.
const DIM_AMOUNT: f32 = 0.38;

fn toward(color: r::Rgb, target: r::Rgb) -> r::Rgb {
    let channel = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * DIM_AMOUNT).round() as u8;
    r::Rgb(channel(color.0, target.0), channel(color.1, target.1), channel(color.2, target.2))
}

/// The theme of an unfocused pane: foreground and the whole palette pulled toward the background. Cells with
/// their own 24-bit colour keep it, so the shape of the text (not only its colour) must still tell which
/// pane has focus: the shell also draws an accent line on the focused pane.
pub fn dimmed(theme: &r::Theme) -> r::Theme {
    let mut dimmed = theme.clone();
    dimmed.foreground = toward(theme.foreground, theme.background);
    for color in &mut dimmed.ansi {
        *color = toward(*color, theme.background);
    }
    for slot in 16..=255u8 {
        dimmed.indexed_overrides[usize::from(slot) - 16] = Some(toward(theme.indexed(slot), theme.background));
    }
    dimmed
}

/// Applies the program's OSC 4 / 10 / 11 / 12 overrides to a copy of the user's theme.
pub fn themed(base: &r::Theme, snapshot: &Snapshot) -> r::Theme {
    let mut theme = base.clone();
    for &(slot, rgb) in &snapshot.overrides {
        let value = r::Rgb(rgb.r, rgb.g, rgb.b);
        match slot {
            0..=255 => theme.set_indexed(slot as u8, value),
            256 => theme.foreground = value,
            257 => theme.background = value,
            258 => theme.cursor = value,
            _ => {}
        }
    }
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(a: r::Rgb, b: r::Rgb) -> i32 {
        (i32::from(a.0) - i32::from(b.0)).abs()
            + (i32::from(a.1) - i32::from(b.1)).abs()
            + (i32::from(a.2) - i32::from(b.2)).abs()
    }

    #[test]
    fn dimming_pulls_text_colours_toward_the_background_and_keeps_the_background() {
        let theme = r::Theme::campbell();
        let dimmed = dimmed(&theme);
        assert_eq!(dimmed.background, theme.background);
        assert!(distance(dimmed.foreground, theme.background) < distance(theme.foreground, theme.background));
        for slot in 0..=255u8 {
            let (before, after) = (theme.indexed(slot), dimmed.indexed(slot));
            assert!(distance(after, theme.background) <= distance(before, theme.background), "slot {slot}");
        }
    }

    #[test]
    fn dimming_leaves_the_original_theme_untouched() {
        let theme = r::Theme::campbell();
        let before = theme.clone();
        let _ = dimmed(&theme);
        assert_eq!(theme, before);
    }
}
