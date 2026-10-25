//! Pane update: damage detection per row, row instance building, glyph/cluster
//! resolution, overlays (cursor, scrollbar).

use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rustc_hash::FxHasher;
use unicode_width::UnicodeWidthChar;

use crate::atlas::AtlasFull;
use crate::cell::{Cell, CellFlags, Color, CursorShape, GridView, Rgb, RowSpan, Theme, Underline};
use crate::contrast::ensure_min_contrast;
use crate::fonts::{CharClass, FontChoice, Style};
use crate::gpu::{GlyphInstance, KIND_COLOR, KIND_LCD, KIND_MASK, RectInstance};
use crate::raster::GlyphKind;
use crate::renderer::{
    CachedBitmap, CachedGlyph, ClusterKey, GlyphKey, PaneState, PlacedGlyph, Renderer, RowInstances, SpriteKey,
    UpdateStats, style_of,
};
use crate::sprites;

const CLUSTER_CACHE_MAX: usize = 65536;

fn rgba(c: Rgb, a: u8) -> [u8; 4] {
    [c.0, c.1, c.2, a]
}

fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

fn span_contains(s: &RowSpan, row: usize, col: usize) -> bool {
    s.row as usize == row && (s.start as usize) <= col && col < s.end as usize
}

impl Renderer {
    /// Bring a pane's instance buffers up to date with `view`. Only rows whose
    /// content or overlay state changed are rebuilt; GPU uploads happen only
    /// when something changed. Call for every visible pane before `draw`.
    pub fn update_pane(&mut self, pane: &mut PaneState, view: &GridView<'_>) -> UpdateStats {
        let mut restarts = 0;
        loop {
            match self.update_pane_inner(pane, view) {
                Ok(mut s) => {
                    s.atlas_restarts = restarts;
                    return s;
                }
                Err(AtlasFull) => {
                    // Everything cached points into the atlas: drop it all and
                    // rebuild this pane from scratch.
                    self.reset_glyph_caches();
                    restarts += 1;
                    if restarts >= 3 {
                        // One frame needs more glyphs than the atlas holds: raise the limit rather than draw wrong glyphs.
                        if self.cfg.atlas_max >= 16384 {
                            return UpdateStats { rows_total: view.rows, atlas_restarts: restarts, ..Default::default() };
                        }
                        self.cfg.atlas_max = (self.cfg.atlas_max * 2).min(16384);
                        self.atlas_gray.set_max_size(self.cfg.atlas_max);
                        self.atlas_rgba.set_max_size(self.cfg.atlas_max);
                    }
                }
            }
        }
    }

    fn update_pane_inner(&mut self, pane: &mut PaneState, view: &GridView<'_>) -> Result<UpdateStats, AtlasFull> {
        debug_assert_eq!(view.cells.len(), view.cols * view.rows);
        let theme = self.theme.clone();
        let gpu_stale = pane.gpu.is_none() || pane.gpu_gen != self.gpu_gen;
        if gpu_stale {
            // The device may be gone (it is made again then): no buffers, and the pane is not drawn until it is.
            let Ok(buffers) = self.pane_gpu() else {
                pane.gpu = None;
                return Ok(UpdateStats { rows_total: view.rows, ..Default::default() });
            };
            pane.gpu = Some(buffers);
            pane.gpu_gen = self.gpu_gen;
        }
        let mut force = gpu_stale || pane.epoch != self.epoch;
        if pane.cols != view.cols || pane.rows != view.rows {
            pane.cols = view.cols;
            pane.rows = view.rows;
            pane.row_keys = vec![0; view.rows];
            pane.row_data = (0..view.rows).map(|_| RowInstances::new()).collect();
            force = true;
        }

        let mut stats = UpdateStats { rows_total: view.rows, ..Default::default() };
        // Which instance streams (bg, glyph, lcd, deco) actually changed: only those are re-uploaded.
        let mut dirty = [force; 4];
        for r in 0..view.rows {
            let cells = &view.cells[r * view.cols..(r + 1) * view.cols];
            let key = self.row_key(r, cells, view);
            if force || pane.row_keys[r] != key {
                let mut nb = RowInstances::new();
                self.build_row(r, cells, view, &theme, &mut nb)?;
                let old = &pane.row_data[r];
                dirty[0] |= old.bg != nb.bg;
                dirty[1] |= old.glyphs != nb.glyphs;
                dirty[2] |= old.lcd != nb.lcd;
                dirty[3] |= old.deco != nb.deco;
                pane.row_data[r] = nb;
                pane.row_keys[r] = key;
                stats.rows_rebuilt += 1;
            }
        }
        pane.epoch = self.epoch;

        let PaneState { row_data, scratch, gpu, overlay_cpu, overlay_prev, .. } = pane;
        let gpu = gpu.as_mut().expect("pane gpu");
        {
            let g = &self.gpu;
            if dirty[0] {
                stats.uploaded_bytes += upload_stream(row_data, |r| &r.bg, scratch, &mut gpu.bg, g);
            }
            if dirty[1] {
                stats.uploaded_bytes += upload_stream(row_data, |r| &r.glyphs, scratch, &mut gpu.glyphs, g);
            }
            if dirty[2] {
                stats.uploaded_bytes += upload_stream(row_data, |r| &r.lcd, scratch, &mut gpu.lcd, g);
            }
            if dirty[3] {
                stats.uploaded_bytes += upload_stream(row_data, |r| &r.deco, scratch, &mut gpu.deco, g);
            }
            stats.bg_instances = gpu.bg.count;
            stats.glyph_instances = gpu.glyphs.count;
            stats.lcd_instances = gpu.lcd.count;
            stats.deco_instances = gpu.deco.count;
        }

        overlay_cpu.clear();
        build_overlay(&self.metrics, &theme, view, overlay_cpu);
        if force || overlay_cpu != overlay_prev {
            gpu.overlay.write(&self.gpu, bytemuck::cast_slice(overlay_cpu), overlay_cpu.len() as u32);
            stats.uploaded_bytes += overlay_cpu.len() * 20;
            // An overlay that became empty (a blinking bar cursor going off) uploads nothing but still changes
            // the picture.
            stats.overlay_changed = true;
            std::mem::swap(overlay_cpu, overlay_prev);
        }
        stats.overlay_instances = gpu.overlay.count;
        let t = std::time::Instant::now();
        self.atlas_gray.flush(&self.gpu);
        self.atlas_rgba.flush(&self.gpu);
        self.stats.upload_time += t.elapsed();
        Ok(stats)
    }

    fn row_key(&self, r: usize, cells: &[Cell], view: &GridView<'_>) -> u64 {
        let mut h = FxHasher::default();
        cells.hash(&mut h);
        for s in view.selection {
            if s.row as usize == r {
                s.hash(&mut h);
            }
        }
        for hl in view.highlights {
            if hl.span.row as usize == r {
                hl.hash(&mut h);
            }
        }
        for l in view.hover_link {
            if l.row as usize == r {
                l.hash(&mut h);
            }
        }
        if let Some(c) = view.cursor {
            // Only the focused visible block cursor changes the row's text colors;
            // other shapes are overlay rects.
            if c.row as usize == r && c.visible && c.focused && c.shape == CursorShape::Block {
                c.hash(&mut h);
            }
        }
        if let Some(p) = view.preedit
            && p.row as usize == r
        {
            p.hash(&mut h);
        }
        h.finish()
    }

    fn build_row(
        &mut self,
        r: usize,
        cells: &[Cell],
        view: &GridView<'_>,
        theme: &Theme,
        out: &mut RowInstances,
    ) -> Result<(), AtlasFull> {
        let m = self.metrics;
        let (cw, ch) = (m.width as i32, m.height as i32);
        let y0 = r as i32 * ch;

        // IME preedit replaces cells on its row with the composition string.
        let cells: Cow<'_, [Cell]> = match view.preedit {
            Some(p) if p.row as usize == r => Cow::Owned(apply_preedit(cells, p.col as usize, &p.text, theme)),
            _ => Cow::Borrowed(cells),
        };

        let lcd_on = self.lcd_active();
        let mut bg_run: Option<(usize, [u8; 4])> = None;
        let mut ul_rects: Vec<RectInstance> = Vec::new();
        let mut ul2_rects: Vec<RectInstance> = Vec::new();
        let mut strike_rects: Vec<RectInstance> = Vec::new();
        let mut link_rects: Vec<RectInstance> = Vec::new();
        let mut sprite_ul: Vec<GlyphInstance> = Vec::new();
        // (text, background, adjusted text) of the last min-contrast lookup: rows repeat colour pairs.
        let mut contrast_memo: Option<(Rgb, Rgb, Rgb)> = None;

        let flush_bg = |run: &mut Option<(usize, [u8; 4])>, end: usize, out: &mut RowInstances| {
            if let Some((start, color)) = run.take() {
                out.bg.push(RectInstance {
                    rect: [(start as i32 * cw) as f32, y0 as f32, ((end - start) as i32 * cw) as f32, ch as f32],
                    color,
                });
            }
        };

        let cursor_here =
            view.cursor.filter(|c| c.row as usize == r && c.visible && c.focused && c.shape == CursorShape::Block);

        let mut c = 0usize;
        while c < cells.len() {
            let cell = &cells[c];
            let wide = cell.flags.has(CellFlags::WIDE) && c + 1 < cells.len();
            let span = if wide { 2usize } else { 1 };

            // ---- colors -------------------------------------------------
            let mut fg_idx = cell.fg;
            if theme.bold_is_bright
                && cell.flags.has(CellFlags::BOLD)
                && let Color::Indexed(i @ 0..8) = fg_idx
            {
                fg_idx = Color::Indexed(i + 8);
            }
            let mut fg = theme.resolve_fg(fg_idx);
            let mut bg = theme.resolve_bg(cell.bg);
            let mut bg_is_default = matches!(cell.bg, Color::Default);
            if cell.flags.has(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
                bg_is_default = false;
            }
            let hidden = cell.flags.has(CellFlags::HIDDEN);
            let zw = cell.zerowidth_chars();
            let blank = (cell.ch == ' ' || cell.ch == '\0') && zw.is_empty();
            let faint = cell.flags.has(CellFlags::FAINT);
            // Powerline separators and box drawing rely on exact colour matches with their neighbours.
            let graphic = zw.is_empty() && sprites::is_sprite_char(cell.ch);
            if theme.min_contrast > 1.0 && !hidden && !blank && !faint && !graphic {
                // A translucent default background shows the backdrop through it: judge against that mix.
                let shown_bg = if bg_is_default { theme.effective_background() } else { bg };
                fg = match contrast_memo {
                    Some((from_fg, from_bg, adjusted)) if from_fg == fg && from_bg == shown_bg => adjusted,
                    _ => {
                        let adjusted = ensure_min_contrast(fg, shown_bg, theme.min_contrast);
                        contrast_memo = Some((fg, shown_bg, adjusted));
                        adjusted
                    }
                };
            }
            if faint {
                fg = lerp_rgb(fg, bg, theme.faint_amount);
            }

            let in_sel = view.selection.iter().any(|s| (0..span).any(|k| span_contains(s, r, c + k)));
            let hl = view.highlights.iter().find(|h| (0..span).any(|k| span_contains(&h.span, r, c + k)));
            if in_sel {
                bg = theme.selection_bg;
                bg_is_default = false;
                if let Some(sf) = theme.selection_fg {
                    fg = sf;
                }
            }
            if let Some(h) = hl {
                // Search highlight wins over selection so the current match stays visible.
                if h.current {
                    bg = theme.search_current_bg;
                    fg = theme.search_current_fg;
                } else {
                    bg = theme.search_match_bg;
                    fg = theme.search_match_fg;
                }
                bg_is_default = false;
            }
            if let Some(cur) = cursor_here
                && c == cur.col as usize
            {
                bg = theme.cursor;
                fg = theme.cursor_text;
                bg_is_default = false;
            }
            let underline_rgb = match cell.extra.as_deref().and_then(|e| e.underline_color) {
                Some(col) => theme.resolve_fg(col),
                None => fg,
            };

            // ---- background run ------------------------------------------
            // Default-background cells are covered by the pane background rect drawn
            // first in `draw` (opacity applied exactly once); only explicit colors get rects.
            let bg_color = if bg_is_default { None } else { Some(rgba(bg, 255)) };
            match (bg_color, &mut bg_run) {
                (Some(col), Some((_, rc))) if *rc == col => {}
                (Some(col), _) => {
                    flush_bg(&mut bg_run, c, out);
                    bg_run = Some((c, col));
                }
                (None, _) => flush_bg(&mut bg_run, c, out),
            }
            // A wide char's spacer cell is skipped; the run covers it implicitly.
            let next_c = c + span;

            // ---- glyph ---------------------------------------------------
            let x0 = c as i32 * cw;
            if !hidden && !blank {
                let fg_px = rgba(fg, 255);
                let style = style_of(cell.flags);
                let placed = self.cluster_glyphs(cell.ch, zw, style, span as u8)?;
                for g in placed.iter() {
                    let mut dst_y = y0 + g.dy;
                    let mut src_y = g.bitmap.src[1] as i32;
                    let mut h = g.bitmap.size[1] as i32;
                    // Clip to the row so rows stay independent (damage tracking).
                    if dst_y < y0 {
                        let cut = y0 - dst_y;
                        dst_y += cut;
                        src_y += cut;
                        h -= cut;
                    }
                    if dst_y + h > y0 + ch {
                        h = y0 + ch - dst_y;
                    }
                    if h <= 0 {
                        continue;
                    }
                    let kind = match g.bitmap.kind {
                        GlyphKind::Mask => KIND_MASK,
                        GlyphKind::Lcd => KIND_LCD,
                        GlyphKind::Color => KIND_COLOR,
                    };
                    let inst = GlyphInstance {
                        dst: [x0 + g.dx, dst_y],
                        size: [g.bitmap.size[0], h as u16],
                        src: [g.bitmap.src[0], src_y as u16],
                        fg: fg_px,
                        kind,
                    };
                    if kind == KIND_LCD && lcd_on {
                        out.lcd.push(inst);
                    } else {
                        out.glyphs.push(inst);
                    }
                }
            }

            // ---- decorations ---------------------------------------------
            let span_w = (span as i32 * cw) as f32;
            let ul_col = rgba(underline_rgb, 255);
            let fg_col = rgba(fg, 255);
            match cell.flags.underline() {
                Underline::None => {}
                Underline::Single => ul_rects.push(RectInstance {
                    rect: [x0 as f32, (y0 + m.underline_top as i32) as f32, span_w, m.underline_thickness as f32],
                    color: ul_col,
                }),
                Underline::Double => {
                    let t = m.underline_thickness as i32;
                    let low = m.underline_top as i32;
                    let high = (low - 2 * t).max(0);
                    ul_rects
                        .push(RectInstance { rect: [x0 as f32, (y0 + low) as f32, span_w, t as f32], color: ul_col });
                    ul2_rects
                        .push(RectInstance { rect: [x0 as f32, (y0 + high) as f32, span_w, t as f32], color: ul_col });
                }
                kind @ (Underline::Curly | Underline::Dotted | Underline::Dashed) => {
                    if let Some(bm) = self.sprite_underline(kind)? {
                        for k in 0..span as i32 {
                            sprite_ul.push(GlyphInstance {
                                dst: [x0 + k * cw, y0],
                                size: bm.size,
                                src: bm.src,
                                fg: ul_col,
                                kind: KIND_MASK,
                            });
                        }
                    }
                }
            }
            if cell.flags.has(CellFlags::STRIKEOUT) {
                strike_rects.push(RectInstance {
                    rect: [x0 as f32, (y0 + m.strike_top as i32) as f32, span_w, m.strike_thickness as f32],
                    color: fg_col,
                });
            }
            if view.hover_link.iter().any(|l| (0..span).any(|k| span_contains(l, r, c + k))) {
                link_rects.push(RectInstance {
                    rect: [x0 as f32, (y0 + m.underline_top as i32) as f32, span_w, m.underline_thickness as f32],
                    color: rgba(theme.link_underline, 255),
                });
            }

            c = next_c;
        }
        flush_bg(&mut bg_run, cells.len(), out);

        for mut v in [ul_rects, ul2_rects, strike_rects, link_rects] {
            merge_adjacent(&mut v);
            out.deco.extend(v);
        }
        out.glyphs.extend(sprite_ul);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Glyph / cluster resolution
    // -----------------------------------------------------------------------

    pub(crate) fn cluster_glyphs(
        &mut self,
        ch: char,
        zw: &[char],
        style: Style,
        span: u8,
    ) -> Result<Arc<[PlacedGlyph]>, AtlasFull> {
        let key = ClusterKey { ch, zw: zw.into(), style, span };
        if let Some(v) = self.cluster_cache.get(&key) {
            return Ok(v.clone());
        }
        let placed: Arc<[PlacedGlyph]> = self.build_cluster(ch, zw, style, span)?.into();
        // Distinct combining sequences (Zalgo text) never fill the atlas, so nothing else would bound this cache.
        if self.cluster_cache.len() > CLUSTER_CACHE_MAX {
            self.cluster_cache.clear();
        }
        self.cluster_cache.insert(key, placed.clone());
        Ok(placed)
    }

    fn build_cluster(&mut self, ch: char, zw: &[char], style: Style, span: u8) -> Result<Vec<PlacedGlyph>, AtlasFull> {
        if zw.is_empty()
            && sprites::is_sprite_char(ch)
            && let Some(bitmap) = self.sprite_char(ch)?
        {
            return Ok(vec![PlacedGlyph { bitmap, dx: 0, dy: 0 }]);
        }
        let class = CharClass { wide: span == 2, emoji_vs: zw.contains(&'\u{FE0F}') };
        let t = std::time::Instant::now();
        let resolved = self.fonts.choice_for(ch, style, class);
        self.stats.font_time += t.elapsed();
        let Some(choice) = resolved else {
            self.stats.missing_glyphs += 1;
            return Ok(Vec::new());
        };
        if zw.is_empty() {
            let glyph = self.fonts.face(choice.face).glyph(ch);
            return self.place_single(choice, glyph, style, span);
        }
        let mut text = String::with_capacity(4 + zw.len() * 4);
        text.push(ch);
        text.extend(zw.iter());
        self.build_shaped(choice, &text, style, span)
    }

    fn place_single(
        &mut self,
        choice: FontChoice,
        glyph: u16,
        style: Style,
        span: u8,
    ) -> Result<Vec<PlacedGlyph>, AtlasFull> {
        if glyph == 0 {
            self.stats.missing_glyphs += 1;
            return Ok(Vec::new());
        }
        let m = self.metrics;
        let px = m.px_size;
        let span_w = span as f32 * m.width as f32;
        let is_primary = self.fonts.primary(style).is_some_and(|p| p.face == choice.face);
        let face = self.fonts.face(choice.face);
        let adv = face.font_ref().glyph_metrics(&[]).scale(px).advance_width(glyph);
        let color_face = face.has_colr || face.has_color_bitmap;

        let mut px_eff = px;
        let mut g = self.cached_glyph(choice, glyph, px_eff, 0)?;
        if let Some(b) = g.bitmap {
            if b.kind == GlyphKind::Color && g.width > 0 && g.height > 0 {
                // Emoji: scale the bitmap so its ink fits the allotted cells
                // exactly (re-rasterized at the new size, never GPU-scaled).
                let s = (span_w / g.width as f32).min(m.height as f32 / g.height as f32);
                if !(0.98..=1.02).contains(&s) {
                    px_eff = px * s;
                    g = self.cached_glyph(choice, glyph, px_eff, 0)?;
                }
            } else if !is_primary && !color_face && g.width as f32 > span_w + 0.25 * m.width as f32 {
                // Fallback glyph wider than its cells: shrink (WezTerm uses the same 0.25 cell slack).
                let s = (span_w + 0.25 * m.width as f32) / g.width as f32;
                px_eff = px * s;
                g = self.cached_glyph(choice, glyph, px_eff, 0)?;
            }
        }
        let Some(bitmap) = g.bitmap else { return Ok(Vec::new()) };
        let (dx, dy) = if bitmap.kind == GlyphKind::Color {
            ((span_w as i32 - g.width as i32) / 2, (m.height as i32 - g.height as i32) / 2)
        } else {
            let adv_eff = adv * px_eff / px;
            let center = if span == 2 && adv_eff < span_w { ((span_w - adv_eff) / 2.0).round() as i32 } else { 0 };
            (g.left + center, m.baseline as i32 - g.top)
        };
        Ok(vec![PlacedGlyph { bitmap, dx, dy }])
    }

    /// Shape a multi-codepoint cluster (combining marks, VS16, ZWJ emoji).
    fn build_shaped(
        &mut self,
        choice: FontChoice,
        text: &str,
        style: Style,
        span: u8,
    ) -> Result<Vec<PlacedGlyph>, AtlasFull> {
        let m = self.metrics;
        let px = m.px_size;
        let face = self.fonts.face(choice.face);
        let upem = face.font_ref().metrics(&[]).units_per_em.max(1) as f32;
        let mut shaped: Vec<(u16, f32, f32, f32)> = Vec::new(); // glyph, advance, x_off, y_off (px)
        if let Some(font) = face.shaping_font() {
            let sf = harfrust::ShaperFont::new(font);
            let mut buf = harfrust::Buffer::new();
            buf.push_str(text);
            buf.guess_segment_properties();
            if harfrust::shape(&sf, &mut buf, harfrust::ShapeOptions::new()).is_ok() {
                let k = px / upem;
                for (i, p) in buf.glyph_infos().iter().zip(buf.glyph_positions()) {
                    shaped.push((
                        i.glyph_id as u16,
                        p.x_advance as f32 * k,
                        p.x_offset as f32 * k,
                        p.y_offset as f32 * k,
                    ));
                }
            }
        }
        if shaped.is_empty() {
            let base = text.chars().next().unwrap_or(' ');
            let glyph = face.glyph(base);
            return self.place_single(choice, glyph, style, span);
        }
        if shaped.len() == 1 {
            return self.place_single(choice, shaped[0].0, style, span);
        }
        // Several glyphs: lay them out along the shaper advances, compressing
        // uniformly when they exceed the cells they own.
        let span_w = span as f32 * m.width as f32;
        let face_is_color = {
            let f = self.fonts.face(choice.face);
            f.has_colr || f.has_color_bitmap
        };
        if face_is_color {
            // Un-ligated emoji sequence (e.g. a ZWJ family the font has no glyph
            // for): show the visible components side by side inside the span,
            // each scaled to its slot.
            let visible: Vec<u16> = shaped.iter().map(|g| g.0).filter(|&g| g != 0).collect();
            let mut rasters = Vec::new();
            for glyph in visible {
                let g = self.cached_glyph(choice, glyph, px, 0)?;
                if g.bitmap.is_some() {
                    rasters.push((glyph, g));
                }
            }
            let n = rasters.len().max(1) as f32;
            let slot = span_w / n;
            let mut out = Vec::new();
            for (i, (glyph, g)) in rasters.into_iter().enumerate() {
                let s = (slot / g.width as f32).min(m.height as f32 / g.height as f32);
                let g2 = if (0.98..=1.02).contains(&s) { g } else { self.cached_glyph(choice, glyph, px * s, 0)? };
                if let Some(b2) = g2.bitmap {
                    out.push(PlacedGlyph {
                        bitmap: b2,
                        dx: (slot * i as f32 + (slot - g2.width as f32) / 2.0).round() as i32,
                        dy: (m.height as i32 - g2.height as i32) / 2,
                    });
                }
            }
            return Ok(out);
        }
        let total: f32 = shaped.iter().map(|g| g.1).sum();
        let scale = if total > span_w + 0.25 * m.width as f32 { (span_w + 0.25 * m.width as f32) / total } else { 1.0 };
        let px_eff = px * scale;
        let mut out = Vec::new();
        let mut pen = 0.0f32;
        for (glyph, adv, xo, yo) in shaped {
            if glyph != 0 {
                let x = (pen + xo) * scale;
                let g = self.cached_glyph(choice, glyph, px_eff, 0)?;
                if let Some(bitmap) = g.bitmap {
                    out.push(PlacedGlyph {
                        bitmap,
                        dx: (x.floor() as i32) + g.left,
                        dy: m.baseline as i32 - g.top - (yo * scale).round() as i32,
                    });
                }
            }
            pen += adv;
        }
        Ok(out)
    }

    pub(crate) fn cached_glyph(
        &mut self,
        choice: FontChoice,
        glyph: u16,
        px: f32,
        subpx: u8,
    ) -> Result<CachedGlyph, AtlasFull> {
        let opts = self.raster_options();
        let key = GlyphKey {
            face: choice.face,
            glyph,
            px_q: (px * 64.0).round() as u32,
            wght: choice.wght.unwrap_or(0.0) as u16,
            flags: (choice.synth_bold as u8)
                | ((choice.synth_italic as u8) << 1)
                | ((opts.lcd as u8) << 2)
                | ((opts.hint as u8) << 3),
            subpx,
        };
        if let Some(g) = self.glyph_cache.get(&key) {
            return Ok(*g);
        }
        let face = self.fonts.face(choice.face);
        let t = std::time::Instant::now();
        let r = self.raster.rasterize(face, &choice, glyph, px, subpx as f32 / 4.0, opts);
        self.stats.raster_time += t.elapsed();
        let cached = match r {
            Some(r) if r.width > 0 && r.height > 0 => {
                let src = self.insert_bitmap(r.kind, r.width, r.height, &r.data)?;
                CachedGlyph {
                    bitmap: Some(CachedBitmap { kind: r.kind, src, size: [r.width as u16, r.height as u16] }),
                    left: r.left,
                    top: r.top,
                    width: r.width,
                    height: r.height,
                }
            }
            Some(r) => CachedGlyph { bitmap: None, left: r.left, top: r.top, width: 0, height: 0 },
            None => CachedGlyph { bitmap: None, left: 0, top: 0, width: 0, height: 0 },
        };
        self.stats.glyphs_rasterized = self.raster.glyphs_rasterized;
        self.glyph_cache.insert(key, cached);
        Ok(cached)
    }

    fn sprite_char(&mut self, ch: char) -> Result<Option<CachedBitmap>, AtlasFull> {
        let key = (SpriteKey::Char(ch), self.metrics.width, self.metrics.height);
        if let Some(c) = self.sprite_cache.get(&key) {
            return Ok(*c);
        }
        let bm = match sprites::draw_char(ch, &self.metrics) {
            Some(s) => {
                let src = self.insert_bitmap(GlyphKind::Mask, s.w, s.h, &s.data)?;
                self.stats.sprites_drawn += 1;
                Some(CachedBitmap { kind: GlyphKind::Mask, src, size: [s.w as u16, s.h as u16] })
            }
            None => None,
        };
        self.sprite_cache.insert(key, bm);
        Ok(bm)
    }

    fn sprite_underline(&mut self, ul: Underline) -> Result<Option<CachedBitmap>, AtlasFull> {
        let key = (SpriteKey::Underline(ul as u8), self.metrics.width, self.metrics.height);
        if let Some(c) = self.sprite_cache.get(&key) {
            return Ok(*c);
        }
        let bm = match sprites::draw_underline(ul, &self.metrics) {
            Some(s) => {
                let src = self.insert_bitmap(GlyphKind::Mask, s.w, s.h, &s.data)?;
                self.stats.sprites_drawn += 1;
                Some(CachedBitmap { kind: GlyphKind::Mask, src, size: [s.w as u16, s.h as u16] })
            }
            None => None,
        };
        self.sprite_cache.insert(key, bm);
        Ok(bm)
    }
}

/// Concatenate one instance stream of every row and upload it with a single
/// `Map`. Returns the number of bytes written.
fn upload_stream<T: bytemuck::Pod>(
    rows: &[RowInstances],
    pick: fn(&RowInstances) -> &Vec<T>,
    scratch: &mut Vec<u8>,
    buf: &mut crate::gpu::GrowBuffer,
    gpu: &crate::d3d::Gpu,
) -> usize {
    scratch.clear();
    let mut n = 0usize;
    for rd in rows {
        let v = pick(rd);
        scratch.extend_from_slice(bytemuck::cast_slice(v));
        n += v.len();
    }
    buf.write(gpu, scratch, n as u32);
    scratch.len()
}

/// Merge consecutive rects that touch horizontally with equal y/h/color.
fn merge_adjacent(v: &mut Vec<RectInstance>) {
    let mut out: Vec<RectInstance> = Vec::with_capacity(v.len());
    for r in v.drain(..) {
        if let Some(last) = out.last_mut()
            && last.rect[1] == r.rect[1]
            && last.rect[3] == r.rect[3]
            && last.color == r.color
            && last.rect[0] + last.rect[2] == r.rect[0]
        {
            last.rect[2] += r.rect[2];
            continue;
        }
        out.push(r);
    }
    *v = out;
}

fn apply_preedit(cells: &[Cell], col: usize, text: &str, theme: &Theme) -> Vec<Cell> {
    let mut row = cells.to_vec();
    let mut c = col;
    let bg = Color::Rgb(theme.preedit_bg.0, theme.preedit_bg.1, theme.preedit_bg.2);
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if w == 0 {
            if c > col {
                let prev = row[c - 1].clone();
                row[c - 1] = prev.zerowidth(&[ch]);
            }
            continue;
        }
        if c + w > row.len() {
            break;
        }
        let mut cell = Cell::new(ch).bg(bg).underline(Underline::Single);
        if w == 2 {
            cell = cell.flag(CellFlags::WIDE);
        }
        row[c] = cell;
        if w == 2 {
            row[c + 1] = Cell { ch: ' ', bg, flags: CellFlags(CellFlags::WIDE_SPACER), ..Default::default() };
        }
        c += w;
    }
    row
}

/// Cursor shapes that do not recolor text (bar, underline, hollow) and the
/// scrollbar. Rebuilt every update; cheap and independent of row caches.
fn build_overlay(m: &crate::fonts::CellMetrics, theme: &Theme, view: &GridView<'_>, out: &mut Vec<RectInstance>) {
    let (cw, ch) = (m.width as f32, m.height as f32);
    if let Some(c) = view.cursor
        && c.visible
    {
        let x = c.col as f32 * cw;
        let y = c.row as f32 * ch;
        let w = if c.wide { 2.0 * cw } else { cw };
        let col = rgba(theme.cursor, 255);
        let t = (m.underline_thickness as f32).max(1.0);
        if !c.focused {
            out.push(RectInstance { rect: [x, y, w, t], color: col });
            out.push(RectInstance { rect: [x, y + ch - t, w, t], color: col });
            out.push(RectInstance { rect: [x, y + t, t, ch - 2.0 * t], color: col });
            out.push(RectInstance { rect: [x + w - t, y + t, t, ch - 2.0 * t], color: col });
        } else {
            match c.shape {
                CursorShape::Block => {}
                CursorShape::Bar => {
                    let bw = (cw / 8.0).round().max(1.0);
                    out.push(RectInstance { rect: [x, y, bw, ch], color: col });
                }
                CursorShape::Underline => {
                    out.push(RectInstance { rect: [x, y + ch - t, w, t], color: col });
                }
            }
        }
    }
    if view.sticky_header {
        let t = (m.underline_thickness as f32).max(1.0);
        out.push(RectInstance { rect: [0.0, ch - t, view.cols as f32 * cw, t], color: rgba(theme.scrollbar_thumb, 0x60) });
    }
    if let Some(sb) = view.scrollbar {
        let track_h = view.rows as f32 * ch;
        let right = view.cols as f32 * cw;
        let w = (cw * 0.6).round().max(4.0);
        let x = right - w;
        out.push(RectInstance {
            rect: [x, 0.0, w, track_h],
            color: rgba(theme.scrollbar_thumb, theme.scrollbar_track_alpha),
        });
        let th = (sb.height * track_h).max(16.0).min(track_h);
        let ty = (sb.top * track_h).clamp(0.0, track_h - th);
        let a = if sb.hovered { 230 } else { 150 };
        out.push(RectInstance { rect: [x, ty, w, th], color: rgba(theme.scrollbar_thumb, a) });
        // The marks over the thumb: a line across the track at each prompt, a shorter one, in the colour of the find,
        // at each match.
        for mark in view.marks {
            let y = (mark.at.clamp(0.0, 1.0) * track_h).min(track_h - 2.0).round();
            let (left, width, color) = match mark.kind {
                crate::cell::MarkKind::Prompt => (x, w, rgba(theme.foreground, 170)),
                // The red of the scheme, as a shell paints an error.
                crate::cell::MarkKind::Failed => (x, w, rgba(theme.ansi[1], 255)),
                crate::cell::MarkKind::Match => (x + w / 2.0, w / 2.0, rgba(theme.search_match_bg, 255)),
                crate::cell::MarkKind::CurrentMatch => (x, w, rgba(theme.search_current_bg, 255)),
            };
            out.push(RectInstance { rect: [left, y, width, 2.0], color });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d3d::{AdapterKind, Gpu};
    use crate::renderer::RendererConfig;

    #[test]
    fn the_cluster_cache_is_cleared_past_its_cap() {
        let gpu = Gpu::new(AdapterKind::Warp).expect("WARP adapter");
        let mut renderer = Renderer::new(&gpu, RendererConfig::default()).expect("renderer");
        let empty: Arc<[PlacedGlyph]> = Arc::from([]);
        for ch in (0u32..).filter_map(char::from_u32).take(CLUSTER_CACHE_MAX + 1) {
            let key = ClusterKey { ch, zw: Box::new(['\u{301}']), style: Style::REGULAR, span: 1 };
            renderer.cluster_cache.insert(key, empty.clone());
        }
        renderer.cluster_glyphs('x', &['\u{302}'], Style::REGULAR, 1).expect("room in the atlas");
        assert_eq!(renderer.cluster_cache.len(), 1);
    }
}
