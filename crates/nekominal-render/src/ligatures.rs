//! Font ligatures: `=>`, `>=`, `->` drawn the way the font draws them (the setting `font.ligatures`).
//!
//! With [`RendererConfig::ligatures`](crate::RendererConfig::ligatures) on, `build_row` first asks
//! [`Renderer::ligature_row`] for the row. It splits the row into runs of cells the primary face draws in one style
//! and one set of colours: plain text only, so no wide CJK or emoji, no box drawing, no combining marks, and nothing
//! under the cursor, selected or highlighted (those cells break a ligature, as in WezTerm and kitty). Each run is
//! shaped with the font's `calt` and `liga` features (`dlig` off) and every glyph is placed from the left edge of the
//! cell its cluster starts in, so a ligature never moves another cell. Coding fonts (Cascadia Code, Fira Code,
//! JetBrains Mono) make most of theirs with `calt`: one glyph per cell whose ink reaches into its neighbours. A
//! `liga` glyph that stands for N characters is drawn once from the first of its N cells; the others draw none.
//!
//! A run whose shaping keeps each character's own glyph in place (any run in Consolas, a plain word in Cascadia Code)
//! is cached as such and drawn by the per-cell path, so turning ligatures on changes nothing else on the screen.
//! Shaped runs are cached per (text, face, style) and cleared with the other glyph caches; a steady screen shapes
//! nothing. Rows are damaged whole, so a change in any cell of a run redraws the run.

use std::ops::Range;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::atlas::AtlasFull;
use crate::cell::{Cell, CellFlags, Color, GridView};
use crate::fonts::{CharClass, Face, FaceId, Style};
use crate::renderer::{PlacedGlyph, Renderer, style_of};
use crate::sprites;

/// Distinct runs are bounded by what programs print, not by the atlas: like the cluster cache, start over past this.
const LIGATURE_CACHE_MAX: usize = 16384;

/// The glyphs of a shaped run, cell by cell, relative to each cell's top-left corner (a cell a ligature covers holds
/// none).
pub(crate) type ShapedRun = Arc<[Arc<[PlacedGlyph]>]>;

#[derive(Default)]
pub(crate) struct LigatureCache {
    /// `None`: shaping changed nothing, the cells draw one by one.
    runs: FxHashMap<(Box<str>, FaceId, Style), Option<ShapedRun>>,
    /// Runs shaped so far (cache misses).
    shaped: usize,
}

impl LigatureCache {
    pub fn clear(&mut self) {
        self.runs.clear();
    }
}

/// What cells must share to be shaped together. Same `fg`, `bg` and flags give the same drawn colours, so a ligature
/// never takes the colour of only one of the cells it covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct RunKey {
    pub face: FaceId,
    pub style: Style,
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
}

/// The runs worth shaping: consecutive cells with equal keys, two or more of them. `None` is a cell drawn on its own.
pub(crate) fn runs<K: PartialEq>(keys: &[Option<K>]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < keys.len() {
        let mut end = start + 1;
        if keys[start].is_some() {
            while end < keys.len() && keys[end] == keys[start] {
                end += 1;
            }
            if end - start >= 2 {
                out.push(start..end);
            }
        }
        start = end;
    }
    out
}

/// One glyph as the shaper hands it over, in font units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawGlyph {
    pub glyph: u16,
    /// Byte offset in the run's text of the first character the glyph stands for.
    pub cluster: u32,
    pub advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// A glyph placed on the grid: `x` from the left edge of `cell`, `y` up from the baseline, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShapedGlyph {
    pub cell: usize,
    pub glyph: u16,
    pub x: f32,
    pub y: f32,
}

/// Puts shaped glyphs on the cells of `text`, one character per cell: each glyph starts at the left edge of the cell
/// its cluster begins in, moved by its offsets and by the advances of the glyphs before it in the same cluster (never
/// by the advances of other clusters, so the columns stay where they are). `scale` turns font units into pixels.
/// `None` when every character kept its own glyph (`cmap`) in place: the run looks as it does without ligatures.
pub(crate) fn place(text: &str, raw: &[RawGlyph], cmap: impl Fn(char) -> u16, scale: f32) -> Option<Vec<ShapedGlyph>> {
    let starts: Vec<(usize, char)> = text.char_indices().collect();
    let unchanged = raw.len() == starts.len()
        && raw.iter().zip(&starts).all(|(g, &(at, ch))| {
            g.cluster as usize == at && g.glyph == cmap(ch) && g.x_offset == 0 && g.y_offset == 0
        });
    if unchanged || starts.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut cluster = u32::MAX;
    let mut pen = 0;
    for g in raw {
        if g.cluster != cluster {
            cluster = g.cluster;
            pen = 0;
        }
        let cell = starts.partition_point(|&(at, _)| at <= g.cluster as usize).saturating_sub(1);
        out.push(ShapedGlyph {
            cell,
            glyph: g.glyph,
            x: (pen + g.x_offset) as f32 * scale,
            y: g.y_offset as f32 * scale,
        });
        pen += g.advance;
    }
    Some(out)
}

/// Shapes `text` left to right with the coding ligature features. Empty when the face cannot be shaped.
fn shape(face: &Face, text: &str) -> Vec<RawGlyph> {
    let Some(font) = face.shaping_font() else { return Vec::new() };
    let features = [
        harfrust::Feature::new(harfrust::Tag::new(b"calt"), 1, ..),
        harfrust::Feature::new(harfrust::Tag::new(b"liga"), 1, ..),
        harfrust::Feature::new(harfrust::Tag::new(b"dlig"), 0, ..),
    ];
    let sf = harfrust::ShaperFont::new(font);
    let mut buf = harfrust::Buffer::new();
    buf.push_str(text);
    // The grid runs left to right whatever the script: a terminal shows characters in the order they were written.
    buf.set_direction(harfrust::Direction::LeftToRight);
    buf.guess_segment_properties();
    if harfrust::shape(&sf, &mut buf, harfrust::ShapeOptions::new().features(&features)).is_err() {
        return Vec::new();
    }
    buf.glyph_infos()
        .iter()
        .zip(buf.glyph_positions())
        .map(|(i, p)| RawGlyph {
            glyph: i.glyph_id as u16,
            cluster: i.cluster,
            advance: p.x_advance,
            x_offset: p.x_offset,
            y_offset: p.y_offset,
        })
        .collect()
}

impl Renderer {
    /// Turns font ligatures on or off; every row is drawn again.
    pub fn set_ligatures(&mut self, on: bool) {
        if self.cfg.ligatures != on {
            self.cfg.ligatures = on;
            self.epoch += 1;
        }
    }

    /// Runs shaped since the renderer was made (cache misses): a frame that shaped nothing leaves it as it was.
    pub fn ligature_runs_shaped(&self) -> usize {
        self.ligatures.shaped
    }

    /// The shaped runs of row `r` whose glyphs differ from drawing the cells one by one, with the cells they cover.
    pub(crate) fn ligature_row(
        &mut self,
        r: usize,
        cells: &[Cell],
        view: &GridView<'_>,
    ) -> Result<Vec<(Range<usize>, ShapedRun)>, AtlasFull> {
        // The cursor breaks a ligature whatever its shape or blink phase, so the glyphs do not change as it blinks.
        let cursor_col = view.cursor.filter(|c| c.row as usize == r).map(|c| c.col as usize);
        let in_span = |s: &crate::cell::RowSpan, c: usize| s.row as usize == r && (s.start as usize) <= c && c < s.end as usize;
        let mut primary: [Option<Option<FaceId>>; 4] = [None; 4];
        let mut keys = Vec::with_capacity(cells.len());
        for (c, cell) in cells.iter().enumerate() {
            let plain = cell.ch != ' '
                && cell.ch != '\0'
                && cell.flags.0 & (CellFlags::HIDDEN | CellFlags::WIDE | CellFlags::WIDE_SPACER) == 0
                && cell.zerowidth_chars().is_empty()
                && !sprites::is_sprite_char(cell.ch)
                && cursor_col != Some(c)
                && !view.selection.iter().any(|s| in_span(s, c))
                && !view.highlights.iter().any(|h| in_span(&h.span, c));
            let key = if plain {
                let style = style_of(cell.flags);
                let primary_face =
                    *primary[style.index()].get_or_insert_with(|| self.fonts.primary(style).map(|p| p.face));
                let face = self.fonts.choice_for(cell.ch, style, CharClass { wide: false, emoji_vs: false }).map(|f| f.face);
                (face.is_some() && face == primary_face).then(|| RunKey {
                    face: face.unwrap_or_default(),
                    style,
                    fg: cell.fg,
                    bg: cell.bg,
                    flags: cell.flags.0,
                })
            } else {
                None
            };
            keys.push(key);
        }
        let mut out = Vec::new();
        let mut text = String::new();
        for run in runs(&keys) {
            let Some(key) = keys[run.start] else { continue };
            text.clear();
            text.extend(cells[run.clone()].iter().map(|c| c.ch));
            if let Some(shaped) = self.shaped_run(&text, key.face, key.style)? {
                out.push((run, shaped));
            }
        }
        Ok(out)
    }

    fn shaped_run(&mut self, text: &str, face: FaceId, style: Style) -> Result<Option<ShapedRun>, AtlasFull> {
        let key = (Box::<str>::from(text), face, style);
        if let Some(v) = self.ligatures.runs.get(&key) {
            return Ok(v.clone());
        }
        self.ligatures.shaped += 1;
        let m = self.metrics;
        let placed = {
            let f = self.fonts.face(face);
            let scale = m.px_size / f.font_ref().metrics(&[]).units_per_em.max(1) as f32;
            place(text, &shape(f, text), |ch| f.glyph(ch), scale)
        };
        let run = match (placed, self.fonts.primary(style)) {
            (Some(glyphs), Some(choice)) => {
                let mut cells: Vec<Vec<PlacedGlyph>> = vec![Vec::new(); text.chars().count()];
                for g in glyphs {
                    if g.glyph == 0 {
                        continue;
                    }
                    let cached = self.cached_glyph(choice, g.glyph, m.px_size, 0)?;
                    if let Some(bitmap) = cached.bitmap {
                        cells[g.cell].push(PlacedGlyph {
                            bitmap,
                            dx: g.x.round() as i32 + cached.left,
                            dy: m.baseline as i32 - cached.top - g.y.round() as i32,
                        });
                    }
                }
                Some(cells.into_iter().map(Arc::from).collect())
            }
            _ => None,
        };
        if self.ligatures.runs.len() > LIGATURE_CACHE_MAX {
            self.ligatures.runs.clear();
        }
        self.ligatures.runs.insert(key, run.clone());
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(glyph: u16, cluster: u32) -> RawGlyph {
        RawGlyph { glyph, cluster, advance: 600, x_offset: 0, y_offset: 0 }
    }

    /// A pretend cmap: the glyph of a character is its code.
    fn cmap(ch: char) -> u16 {
        ch as u16
    }

    #[test]
    fn runs_are_two_or_more_equal_neighbours_and_a_lone_cell_or_a_break_is_drawn_on_its_own() {
        let keys = [Some(1), Some(1), None, Some(1), Some(2), Some(2), Some(2), Some(1), None, Some(3), Some(3)];
        assert_eq!(runs(&keys), [0..2, 4..7, 9..11]);
        assert_eq!(runs::<u8>(&[]), Vec::<Range<usize>>::new());
        assert_eq!(runs(&[Some(1)]), Vec::<Range<usize>>::new());
    }

    #[test]
    fn a_run_that_keeps_every_glyph_in_place_is_left_to_the_per_cell_path() {
        let raw = [glyph('a' as u16, 0), glyph('=' as u16, 1), glyph('>' as u16, 2)];
        assert_eq!(place("a=>", &raw, cmap, 0.01), None);
    }

    #[test]
    fn contextual_alternates_stay_one_glyph_per_cell_at_the_left_edge_of_their_own_cell() {
        // Fira Code / Cascadia Code style: `=>` becomes two new glyphs, each in its own cluster; the second reaches
        // left into the first cell with its ink, not with its position.
        let raw = [glyph(900, 0), glyph(901, 1)];
        let placed = place("=>", &raw, cmap, 0.02).expect("changed");
        assert_eq!(
            placed,
            [ShapedGlyph { cell: 0, glyph: 900, x: 0.0, y: 0.0 }, ShapedGlyph { cell: 1, glyph: 901, x: 0.0, y: 0.0 }]
        );
    }

    #[test]
    fn a_ligature_of_three_characters_is_drawn_once_from_its_first_cell_and_the_cells_after_it_keep_their_columns() {
        // `a===b`: one glyph for `===` (cluster at byte 1, three cells wide), then `b` at cell 4.
        let raw = [glyph('a' as u16, 0), RawGlyph { glyph: 777, cluster: 1, advance: 1800, x_offset: 0, y_offset: 0 }, glyph('b' as u16, 4)];
        let placed = place("a===b", &raw, cmap, 0.01).expect("changed");
        assert_eq!(placed.iter().map(|g| (g.cell, g.glyph)).collect::<Vec<_>>(), [(0, 'a' as u16), (1, 777), (4, 'b' as u16)]);
        assert!(placed.iter().all(|g| g.x == 0.0), "every glyph starts at its own cell: {placed:?}");
    }

    #[test]
    fn glyphs_of_one_cluster_follow_each_other_and_offsets_move_them_in_pixels() {
        let raw = [
            RawGlyph { glyph: 5, cluster: 0, advance: 300, x_offset: 0, y_offset: 0 },
            RawGlyph { glyph: 6, cluster: 0, advance: 300, x_offset: 50, y_offset: -100 },
            glyph('x' as u16, 2),
        ];
        let placed = place("<-x", &raw, cmap, 0.01).expect("changed");
        assert_eq!(placed[1], ShapedGlyph { cell: 0, glyph: 6, x: 3.5, y: -1.0 });
        assert_eq!(placed[2], ShapedGlyph { cell: 2, glyph: 'x' as u16, x: 0.0, y: 0.0 });
    }

    #[test]
    fn clusters_map_to_cells_by_characters_not_bytes() {
        // `é` takes two bytes but one cell.
        let raw = [glyph(1, 0), glyph(2, 2), glyph(3, 3)];
        let placed = place("é=>", &raw, cmap, 0.01).expect("changed");
        assert_eq!(placed.iter().map(|g| g.cell).collect::<Vec<_>>(), [0, 1, 2]);
    }
}
