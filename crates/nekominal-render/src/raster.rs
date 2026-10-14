//! Glyph rasterization through swash (outline, LCD, COLR v0 and bitmap strikes).

use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Transform, Vector};

use crate::fonts::{Face, FontChoice};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlyphKind {
    /// R8 coverage mask tinted with the foreground color.
    Mask,
    /// RGBA: per-channel coverage for LCD subpixel blending.
    Lcd,
    /// RGBA premultiplied color bitmap (emoji).
    Color,
}

pub struct Rasterized {
    pub kind: GlyphKind,
    pub width: u32,
    pub height: u32,
    /// Pixels from the glyph origin to the left edge of the bitmap.
    pub left: i32,
    /// Pixels from the baseline up to the top edge of the bitmap.
    pub top: i32,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct RasterOptions {
    pub lcd: bool,
    pub hint: bool,
}

pub struct Rasterizer {
    ctx: ScaleContext,
    pub glyphs_rasterized: usize,
}

/// Faux italic shear (tan 14 degrees ~ 0.25, same as Skia/Chromium).
const SYNTH_ITALIC_DEGREES: f32 = 14.0;

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rasterizer {
    pub fn new() -> Self {
        Rasterizer { ctx: ScaleContext::new(), glyphs_rasterized: 0 }
    }

    /// Rasterize one glyph at `px` pixels per em. `subpx` is the horizontal
    /// sub-pixel offset in [0, 1) (0 for grid-aligned monospace glyphs).
    pub fn rasterize(
        &mut self,
        face: &Face,
        choice: &FontChoice,
        glyph: u16,
        px: f32,
        subpx: f32,
        opts: RasterOptions,
    ) -> Option<Rasterized> {
        self.glyphs_rasterized += 1;
        let font = face.font_ref();
        let wght = choice.wght.map(|w| [("wght", w)]);
        let mut builder = self.ctx.builder(font).size(px).hint(opts.hint);
        if let Some(w) = &wght {
            builder = builder.variations(w.iter().copied());
        }
        let mut scaler = builder.build();

        let color_face = face.has_colr || face.has_color_bitmap;
        let sources_color = [Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline];
        let sources_plain = [Source::Outline];
        let sources: &[Source] = if color_face { &sources_color } else { &sources_plain };

        let mut render = Render::new(sources);
        render.format(if opts.lcd { Format::Subpixel } else { Format::Alpha });
        render.offset(Vector::new(subpx, 0.0));
        // COLR layers that reference the foreground color: white so the
        // result is neutral if a font uses them (Segoe UI Emoji does not).
        render.default_color([255, 255, 255, 255]);
        if choice.synth_bold {
            render.embolden(px / 48.0);
        }
        if choice.synth_italic {
            render
                .transform(Some(Transform::skew(Angle::from_degrees(SYNTH_ITALIC_DEGREES), Angle::from_degrees(0.0))));
        }
        let image = render.render(&mut scaler, glyph)?;
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            return Some(Rasterized {
                kind: GlyphKind::Mask,
                width: 0,
                height: 0,
                left: p.left,
                top: p.top,
                data: Vec::new(),
            });
        }
        let (kind, data) = match image.content {
            Content::Mask => (GlyphKind::Mask, image.data),
            Content::SubpixelMask => (GlyphKind::Lcd, image.data),
            Content::Color => {
                let mut d = image.data;
                for px in d.as_chunks_mut::<4>().0 {
                    let a = px[3] as u32;
                    px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
                    px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
                    px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
                }
                (GlyphKind::Color, d)
            }
        };
        Some(Rasterized { kind, width: p.width, height: p.height, left: p.left, top: p.top, data })
    }
}
