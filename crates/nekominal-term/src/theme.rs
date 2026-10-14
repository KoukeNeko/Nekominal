//! The colours the terminal itself must be able to report. alacritty_terminal stores `Color::{Named, Indexed,
//! Spec}` per cell and keeps only *overrides* (OSC 4/10/11/12) in `Term::colors()`.
//!
//! This is not where cell colours are decided: the renderer's `Theme` resolves them (INVERSE, bold-is-bright,
//! faint, hidden, contrast). `Palette` only answers programs that ask "what colour is slot N" (OSC 4/10/11/12
//! queries and the colour-scheme report), so the host should set it to the theme the user sees.

use alacritty_terminal::{
    term::color::Colors,
    vte::ansi::{Color, Rgb},
};

#[derive(Clone, Debug)]
pub struct Palette {
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

const fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

impl Default for Palette {
    /// Windows Terminal's "Campbell" scheme.
    fn default() -> Self {
        Self {
            ansi: [
                rgb(0x0c0c0c),
                rgb(0xc50f1f),
                rgb(0x13a10e),
                rgb(0xc19c00),
                rgb(0x0037da),
                rgb(0x881798),
                rgb(0x3a96dd),
                rgb(0xcccccc),
                rgb(0x767676),
                rgb(0xe74856),
                rgb(0x16c60c),
                rgb(0xf9f1a5),
                rgb(0x3b78ff),
                rgb(0xb4009e),
                rgb(0x61d6d6),
                rgb(0xf2f2f2),
            ],
            foreground: rgb(0xcccccc),
            background: rgb(0x0c0c0c),
            cursor: rgb(0xffffff),
        }
    }
}

impl Palette {
    /// xterm 256-colour table: 0-15 from the scheme, 16-231 the 6x6x6 cube, 232-255 the grey ramp.
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let n = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Rgb { r: level(n / 36), g: level((n / 6) % 6), b: level(n % 6) }
            }
            232..=255 => {
                let v = 8 + 10 * (index - 232);
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    /// Colour for an index into `Term::colors()` (0-255 palette, 256 foreground, 257 background, 258
    /// cursor). Overrides set by the program win.
    pub fn slot(&self, overrides: &Colors, slot: usize) -> Rgb {
        if let Some(color) = overrides[slot] {
            return color;
        }
        match slot {
            0..=255 => self.indexed(slot as u8),
            257 => self.background,
            258 => self.cursor,
            _ => self.foreground,
        }
    }

    /// A stored cell colour as plain RGB, flags ignored.
    pub fn color(&self, overrides: &Colors, color: Color) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Named(named) => self.slot(overrides, named as usize),
            Color::Indexed(index) => self.slot(overrides, usize::from(index)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grey_ramp_match_xterm() {
        let palette = Palette::default();
        assert_eq!(palette.indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(palette.indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(palette.indexed(21), Rgb { r: 0, g: 0, b: 255 });
        assert_eq!(palette.indexed(231), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(palette.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(palette.indexed(255), Rgb { r: 238, g: 238, b: 238 });
    }
}
