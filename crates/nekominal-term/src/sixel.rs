//! A streaming Sixel decoder: the sniffer hands it the bytes of a `DCS P1 ; P2 ; P3 q ... ST` string one by one
//! (vte's `put`), so a picture is never held twice, once as text and once as pixels.
//!
//! What it reads: data characters `?`..`~` (six pixels of a column each), repeat `! Pn`, carriage return `$`, next
//! band `-`, raster attributes `" Pan ; Pad ; Ph ; Pv` (pixel aspect and size) and colour registers `# Pc` (select) /
//! `# Pc ; Pu ; Px ; Py ; Pz` (define in HLS, `Pu` 1, or RGB, `Pu` 2, both in percent). Anything else is skipped, as a
//! VT340 does. A register is resolved when a pixel is drawn, not when the picture ends: the encoders in use define their
//! colours first, and the pixels need no second pass.
//!
//! Output is untrusted: the picture is cut at `MAX_SIDE` pixels each way (pixels past it are dropped, not stored), and a
//! repeat count, a register number or a parameter never grows anything past that.

use std::fmt;

/// The widest and tallest picture kept, in pixels; the rest is dropped. 4096 x 4096 RGBA is 64 MiB.
pub const MAX_SIDE: usize = 4096;
/// Colour registers of a picture (xterm's default for a VT340 is 16; it allows up to 1024).
pub const REGISTERS: usize = 1024;

/// The VT340's 16 start colours in percent, as xterm sets them; the other registers start black.
const VT340: [[u32; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

/// The palette a picture starts with when registers are private (DECSET 1070, the default).
pub fn default_palette() -> Vec<[u8; 4]> {
    let mut palette = vec![[0, 0, 0, 255]; REGISTERS];
    for (slot, [r, g, b]) in palette.iter_mut().zip(VT340) {
        *slot = [percent(r), percent(g), percent(b), 255];
    }
    palette
}

fn percent(value: u32) -> u8 {
    ((value.min(100) * 255 + 50) / 100) as u8
}

/// DEC HLS (hue 0 is blue, 120 red, 240 green; lightness and saturation in percent) to RGB.
fn hls(hue: u32, lightness: u32, saturation: u32) -> [u8; 4] {
    let h = ((hue % 360) as f32 + 240.0) % 360.0 / 360.0;
    let (l, s) = (lightness.min(100) as f32 / 100.0, saturation.min(100) as f32 / 100.0);
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return [v, v, v, 255];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0), 255]
}

/// A decoded picture: RGBA, straight alpha. Pixels no sixel set are transparent; `transparent_background` false (P2 0
/// or 2) asks the terminal to fill them with its background colour.
#[derive(Clone, PartialEq, Eq)]
pub struct SixelImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub transparent_background: bool,
}

impl fmt::Debug for SixelImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SixelImage({}x{}, transparent: {})", self.width, self.height, self.transparent_background)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Repeat,
    Raster,
    Color,
}

pub struct SixelDecoder {
    palette: Vec<[u8; 4]>,
    color: [u8; 4],
    /// One row of pixels per picture row, each as long as the rightmost pixel set in it.
    rows: Vec<Vec<[u8; 4]>>,
    x: usize,
    /// Top pixel row of the current band of six.
    top: usize,
    /// Pixel aspect from the raster attributes: each sixel pixel is drawn `wide` x `tall` pixels.
    wide: usize,
    tall: usize,
    /// Size the raster attributes declared (the picture is at least that big).
    declared: (usize, usize),
    state: State,
    params: [u32; 5],
    count: usize,
    transparent_background: bool,
}

impl SixelDecoder {
    /// A picture of a `DCS P1 ; P2 ; P3 q` string with `palette` (`default_palette`, or the one the last picture left
    /// when the registers are shared). P1 (an aspect ratio) is ignored as xterm does: the raster attributes give it.
    pub fn new(p2: u16, mut palette: Vec<[u8; 4]>) -> Self {
        palette.resize(REGISTERS, [0, 0, 0, 255]);
        let color = palette[0];
        Self {
            palette,
            color,
            rows: Vec::new(),
            x: 0,
            top: 0,
            wide: 1,
            tall: 1,
            declared: (0, 0),
            state: State::Data,
            params: [0; 5],
            count: 0,
            transparent_background: p2 == 1,
        }
    }

    /// The palette as the picture left it (for shared registers).
    pub fn palette(&self) -> &[[u8; 4]] {
        &self.palette
    }

    pub fn put(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' if self.state != State::Data => {
                if self.count == 0 {
                    self.count = 1;
                }
                let slot = &mut self.params[self.count - 1];
                *slot = slot.saturating_mul(10).saturating_add(u32::from(byte - b'0'));
            }
            b';' if self.state != State::Data => {
                if self.count == 0 {
                    self.count = 1;
                }
                self.count = (self.count + 1).min(self.params.len());
                self.params[self.count - 1] = 0;
            }
            _ => {
                let repeat = self.end_parameters();
                match byte {
                    b'?'..=b'~' => self.sixel(byte - b'?', repeat),
                    b'!' => self.start(State::Repeat),
                    b'"' => self.start(State::Raster),
                    b'#' => self.start(State::Color),
                    b'$' => self.x = 0,
                    b'-' => {
                        self.x = 0;
                        self.top = self.top.saturating_add(6 * self.tall).min(MAX_SIDE);
                    }
                    _ => {}
                }
            }
        }
    }

    fn start(&mut self, state: State) {
        self.state = state;
        self.params = [0; 5];
        self.count = 0;
    }

    /// Applies the parameters of a repeat, raster or colour introducer that just ended; returns the repeat count for
    /// the data character that follows (1 when there is none).
    fn end_parameters(&mut self) -> usize {
        let state = std::mem::replace(&mut self.state, State::Data);
        let p = self.params;
        match state {
            State::Data => 1,
            State::Repeat => (p[0] as usize).clamp(1, MAX_SIDE),
            State::Raster => {
                let (pan, pad) = (p[0].max(1) as usize, p[1].max(1) as usize);
                // Pan : Pad is the pixel's height to its width.
                (self.tall, self.wide) = if pan >= pad { (((pan + pad / 2) / pad).min(16), 1) } else { (1, ((pad + pan / 2) / pan).min(16)) };
                if self.count >= 4 {
                    self.declared = ((p[2] as usize).min(MAX_SIDE), (p[3] as usize).min(MAX_SIDE));
                }
                1
            }
            State::Color => {
                let register = p[0] as usize;
                if register < REGISTERS {
                    if self.count >= 5 {
                        match p[1] {
                            1 => self.palette[register] = hls(p[2], p[3], p[4]),
                            2 => self.palette[register] = [percent(p[2]), percent(p[3]), percent(p[4]), 255],
                            _ => {}
                        }
                    }
                    self.color = self.palette[register];
                }
                1
            }
        }
    }

    fn sixel(&mut self, bits: u8, repeat: usize) {
        let (wide, tall) = (self.wide, self.tall);
        let start = self.x;
        let end = start.saturating_add(repeat.saturating_mul(wide)).min(MAX_SIDE);
        self.x = end;
        if bits == 0 || start >= end {
            return;
        }
        for bit in 0..6 {
            if bits & (1 << bit) == 0 {
                continue;
            }
            for dy in 0..tall {
                let y = self.top + bit * tall + dy;
                if y >= MAX_SIDE {
                    return;
                }
                if self.rows.len() <= y {
                    self.rows.resize_with(y + 1, Vec::new);
                }
                let row = &mut self.rows[y];
                if row.len() < end {
                    row.resize(end, [0; 4]);
                }
                row[start..end].fill(self.color);
            }
        }
    }

    /// The picture; None when it has no pixels at all.
    pub fn finish(mut self) -> Option<SixelImage> {
        self.end_parameters();
        let width = self.rows.iter().map(Vec::len).max().unwrap_or(0).max(self.declared.0);
        let height = self.rows.len().max(self.declared.1);
        if width == 0 || height == 0 {
            return None;
        }
        let mut pixels = vec![0; width * height * 4];
        for (y, row) in self.rows.iter().enumerate() {
            let at = y * width * 4;
            pixels[at..at + row.len() * 4].copy_from_slice(row.as_flattened());
        }
        Some(SixelImage { width: width as u32, height: height as u32, pixels, transparent_background: self.transparent_background })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(p2: u16, data: &[u8]) -> SixelImage {
        let mut decoder = SixelDecoder::new(p2, default_palette());
        for byte in data {
            decoder.put(*byte);
        }
        decoder.finish().expect("a picture")
    }

    fn px(image: &SixelImage, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width + x) * 4) as usize;
        image.pixels[i..i + 4].try_into().unwrap()
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const CLEAR: [u8; 4] = [0; 4];

    #[test]
    fn a_two_by_twelve_golden_picture_with_rgb_registers_bands_and_carriage_return() {
        // Band 1: red column of six, then (after `$`) green in the second column's top three rows. Band 2: a red pixel
        // at the top of the second column.
        let image = decode(1, b"#1;2;100;0;0#2;2;0;100;0#1~$?#2F-#1?@");
        assert_eq!((image.width, image.height), (2, 7), "as tall as the lowest pixel set");
        for y in 0..6 {
            assert_eq!(px(&image, 0, y), RED, "row {y}");
        }
        assert_eq!(px(&image, 1, 0), GREEN);
        assert_eq!(px(&image, 1, 2), GREEN);
        assert_eq!(px(&image, 1, 3), CLEAR);
        assert_eq!(px(&image, 1, 6), RED, "second band starts at row 6");
        assert_eq!(px(&image, 0, 6), CLEAR);
        assert!(image.transparent_background);
    }

    #[test]
    fn repeat_fills_a_run_and_raster_attributes_set_size_and_aspect() {
        let image = decode(0, b"\"2;1;10;4#1;2;0;100;0!3~");
        // Declared 10 x 4, but the pixels are twice as tall as wide, so the band covers 12 rows.
        assert_eq!((image.width, image.height), (10, 12));
        assert_eq!(px(&image, 2, 11), GREEN);
        assert_eq!(px(&image, 3, 0), CLEAR, "past the run, inside the declared width");
        assert!(!image.transparent_background, "P2 0 asks for the background");
    }

    #[test]
    fn hls_colours_follow_the_dec_hue_circle() {
        assert_eq!(hls(120, 50, 100), RED, "hue 120 is red");
        assert_eq!(hls(240, 50, 100), GREEN);
        assert_eq!(hls(0, 50, 100), [0, 0, 255, 255], "hue 0 is blue");
        assert_eq!(hls(0, 100, 0), [255, 255, 255, 255]);
        let image = decode(1, b"#5;1;120;50;100#5@");
        assert_eq!(px(&image, 0, 0), RED);
    }

    #[test]
    fn default_registers_are_the_vt340_colours() {
        let image = decode(1, b"#2@");
        assert_eq!(px(&image, 0, 0), [204, 33, 33, 255], "register 2 is the VT340's red (80, 13, 13 %)");
    }

    #[test]
    fn oversized_input_is_cut_at_the_limit_instead_of_growing() {
        let mut data = b"#1;2;100;0;0!999999999~".to_vec();
        data.extend(std::iter::repeat_n(b'-', 2000));
        data.extend_from_slice(b"~\"1;1;99999;99999");
        let image = decode(1, &data);
        assert_eq!((image.width as usize, image.height as usize), (MAX_SIDE, MAX_SIDE));
    }

    #[test]
    fn garbage_never_panics() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..200 {
            let mut decoder = SixelDecoder::new(0, default_palette());
            for _ in 0..2000 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                // Mostly the sixel alphabet, with digits and introducers mixed in.
                let alphabet = b"0123456789;#!\"$-?~@ABCDxyz\x00\xff";
                decoder.put(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = decoder.finish();
        }
    }

    #[test]
    fn a_string_without_pixels_is_no_picture() {
        let decoder = SixelDecoder::new(0, default_palette());
        assert!(decoder.finish().is_none());
        let mut decoder = SixelDecoder::new(0, default_palette());
        for byte in b"#1;2;1;2;3$-" {
            decoder.put(*byte);
        }
        assert!(decoder.finish().is_none());
    }
}
