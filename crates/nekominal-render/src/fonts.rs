//! Font stack: family chains, per-character fallback, cell metrics.
//!
//! Resolution order for a character (first face whose cmap maps it wins):
//!   1. the primary family list (user's monospace font, then e.g. Consolas)
//!   2. a class-specific chain: emoji -> emoji fonts, CJK -> Traditional Chinese
//!      fonts, everything else -> symbol fonts
//!   3. the remaining chains
//!   4. DirectWrite `MapCharacters` with an explicit Han locale ("zh-TW")
//!
//! Results are cached per (char, style, class).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use rustc_hash::FxHashMap;
use swash::{FontRef, tag_from_bytes};

#[cfg(windows)]
use crate::locator::{Located, Locator};

pub type FaceId = u16;

/// Font file bytes: a memory map of an installed font, or bytes embedded in the application.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

impl Style {
    pub const REGULAR: Style = Style { bold: false, italic: false };
    pub fn index(self) -> usize {
        (self.bold as usize) | ((self.italic as usize) << 1)
    }
}

/// What to render a glyph from: a face plus the style adjustments needed
/// because the face may not have the requested weight/slant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FontChoice {
    pub face: FaceId,
    /// Variable-font weight axis value to apply (None for static faces).
    pub wght: Option<f32>,
    pub synth_bold: bool,
    pub synth_italic: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CharClass {
    pub wide: bool,
    /// Followed by U+FE0F (emoji presentation selector).
    pub emoji_vs: bool,
}

pub struct Face {
    pub id: FaceId,
    pub family: String,
    pub path: Option<PathBuf>,
    data: FontData,
    pub index: u32,
    offset: u32,
    key: swash::CacheKey,
    pub weight: u16,
    pub italic: bool,
    /// (min, default, max) of the `wght` axis for variable fonts.
    pub wght_axis: Option<(f32, f32, f32)>,
    pub has_colr: bool,
    pub has_color_bitmap: bool,
    hb: OnceLock<Option<harfrust::font::Font>>,
}

impl Face {
    pub fn data(&self) -> &[u8] {
        (*self.data).as_ref()
    }

    pub fn font_ref(&self) -> FontRef<'_> {
        FontRef { data: self.data(), offset: self.offset, key: self.key }
    }

    pub fn glyph(&self, ch: char) -> u16 {
        self.font_ref().charmap().map(ch)
    }

    /// Lazily parsed HarfBuzz-port font for shaping multi-codepoint clusters.
    pub fn shaping_font(&self) -> Option<&harfrust::font::Font> {
        self.hb
            .get_or_init(|| {
                let blob = harfrust::font::Blob::from(self.data.clone());
                harfrust::font::Font::new(blob, self.index)
            })
            .as_ref()
    }
}

#[derive(Clone)]
pub struct FontConfig {
    /// Primary text chain, e.g. ["Cascadia Mono", "Consolas"].
    pub text: Vec<String>,
    /// Traditional Chinese first, then other CJK fonts.
    pub cjk: Vec<String>,
    pub emoji: Vec<String>,
    pub symbols: Vec<String>,
    /// Fonts supplied as bytes (bundled Nerd Font symbols etc.), appended to the
    /// symbols chain. (family label, data)
    pub embedded: Vec<(String, FontData)>,
    /// Locale handed to MapCharacters; decides Han glyph variants.
    pub han_locale: String,
}

impl Default for FontConfig {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        FontConfig {
            text: s(&["Cascadia Mono", "Consolas"]),
            cjk: s(&["Microsoft JhengHei UI", "Microsoft JhengHei", "MingLiU", "Noto Sans TC"]),
            emoji: s(&["Segoe UI Emoji"]),
            symbols: s(&["Segoe UI Symbol", "Segoe Fluent Icons"]),
            embedded: Vec::new(),
            han_locale: "zh-TW".into(),
        }
    }
}

pub struct FontStack {
    pub cfg: FontConfig,
    faces: Vec<Face>,
    by_file: HashMap<(PathBuf, u32), FaceId>,
    family_cache: HashMap<(String, Style), Option<FontChoice>>,
    char_cache: FxHashMap<(char, Style, CharClass), Option<FontChoice>>,
    embedded_faces: Vec<FaceId>,
    #[cfg(windows)]
    locator: Locator,
    pub stats: FontStats,
}

#[derive(Default, Clone, Debug)]
pub struct FontStats {
    pub faces_loaded: usize,
    pub os_fallback_calls: usize,
    pub char_resolutions: usize,
}

pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x2E80..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F | 0x3130..=0x318F | 0x31A0..=0x31FF |
        0x3200..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF |
        0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

impl FontStack {
    pub fn new(cfg: FontConfig) -> Result<Self, String> {
        let mut stack = FontStack {
            #[cfg(windows)]
            locator: Locator::new().map_err(|e| format!("DirectWrite init failed: {e}"))?,
            cfg,
            faces: Vec::new(),
            by_file: HashMap::new(),
            family_cache: HashMap::new(),
            char_cache: FxHashMap::default(),
            embedded_faces: Vec::new(),
            stats: FontStats::default(),
        };
        let embedded = std::mem::take(&mut stack.cfg.embedded);
        for (name, data) in &embedded {
            if let Some(id) = stack.add_face(name.clone(), None, data.clone(), 0, 400, false) {
                stack.embedded_faces.push(id);
            }
        }
        stack.cfg.embedded = embedded;
        // The primary regular face must exist: everything else is derived from it.
        if stack.primary(Style::REGULAR).is_none() {
            return Err(format!("none of the primary families {:?} could be loaded", stack.cfg.text));
        }
        Ok(stack)
    }

    pub fn face(&self, id: FaceId) -> &Face {
        &self.faces[id as usize]
    }

    #[cfg(windows)]
    pub fn locator(&self) -> &Locator {
        &self.locator
    }

    fn add_face(
        &mut self,
        family: String,
        path: Option<PathBuf>,
        data: FontData,
        index: u32,
        weight: u16,
        italic: bool,
    ) -> Option<FaceId> {
        let id = self.faces.len() as FaceId;
        let (offset, key, wght_axis, has_colr, has_bitmap) = {
            let bytes: &[u8] = (*data).as_ref();
            let font = FontRef::from_index(bytes, index as usize)?;
            let wght = tag_from_bytes(b"wght");
            let axis =
                font.variations().find(|v| v.tag() == wght).map(|v| (v.min_value(), v.default_value(), v.max_value()));
            let colr = font.table(tag_from_bytes(b"COLR")).is_some();
            let bitmap = font.table(tag_from_bytes(b"CBDT")).is_some() || font.table(tag_from_bytes(b"sbix")).is_some();
            (font.offset, font.key, axis, colr, bitmap)
        };
        self.faces.push(Face {
            id,
            family,
            path,
            data,
            index,
            offset,
            key,
            weight,
            italic,
            wght_axis,
            has_colr,
            has_color_bitmap: has_bitmap,
            hb: OnceLock::new(),
        });
        self.stats.faces_loaded += 1;
        Some(id)
    }

    #[cfg(windows)]
    fn load_located(&mut self, loc: &Located) -> Option<FaceId> {
        let key = (loc.path.clone(), loc.index);
        if let Some(&id) = self.by_file.get(&key) {
            return Some(id);
        }
        let file = std::fs::File::open(&loc.path).ok()?;
        // Safety: system font files are not modified while mapped (the OS locks
        // installed fonts); a concurrent rewrite would at worst corrupt glyphs.
        let map = unsafe { memmap2::Mmap::map(&file).ok()? };
        let data: FontData = Arc::new(map);
        let id = self.add_face(loc.family.clone(), Some(loc.path.clone()), data, loc.index, loc.weight, loc.italic)?;
        self.by_file.insert(key, id);
        Some(id)
    }

    fn choice_for_face(&self, id: FaceId, style: Style) -> FontChoice {
        let face = &self.faces[id as usize];
        match face.wght_axis {
            Some((min, _default, max)) => FontChoice {
                face: id,
                wght: Some(if style.bold { 700.0f32 } else { 400.0f32 }.clamp(min, max)),
                synth_bold: false,
                synth_italic: style.italic && !face.italic,
            },
            None => FontChoice {
                face: id,
                wght: None,
                synth_bold: style.bold && face.weight < 600,
                synth_italic: style.italic && !face.italic,
            },
        }
    }

    /// Family name -> face for a style, with weight/slant adjustment decided.
    pub fn family_choice(&mut self, family: &str, style: Style) -> Option<FontChoice> {
        if let Some(c) = self.family_cache.get(&(family.to_string(), style)) {
            return *c;
        }
        let mut choice = None;
        #[cfg(windows)]
        if let Some(loc) = self.locator.resolve(family, style.bold, style.italic)
            && let Some(id) = self.load_located(&loc)
        {
            choice = Some(self.choice_for_face(id, style));
        }
        // Embedded fonts are addressed by their label.
        if choice.is_none()
            && let Some(&id) = self.embedded_faces.iter().find(|&&id| self.faces[id as usize].family == family)
        {
            choice = Some(self.choice_for_face(id, style));
        }
        self.family_cache.insert((family.to_string(), style), choice);
        choice
    }

    /// First usable face of the primary text chain for a style.
    pub fn primary(&mut self, style: Style) -> Option<FontChoice> {
        let families = self.cfg.text.clone();
        families.iter().find_map(|f| self.family_choice(f, style))
    }

    fn chain_lookup(&mut self, chain: &[String], ch: char, style: Style) -> Option<FontChoice> {
        for fam in chain {
            if let Some(c) = self.family_choice(fam, style)
                && self.faces[c.face as usize].glyph(ch) != 0
            {
                return Some(c);
            }
        }
        None
    }

    /// Resolve the face that should draw `ch`.
    pub fn choice_for(&mut self, ch: char, style: Style, class: CharClass) -> Option<FontChoice> {
        if let Some(c) = self.char_cache.get(&(ch, style, class)) {
            return *c;
        }
        self.stats.char_resolutions += 1;
        let text = self.cfg.text.clone();
        let cjk = self.cfg.cjk.clone();
        let emoji = self.cfg.emoji.clone();
        let mut symbols = self.cfg.symbols.clone();
        for &id in &self.embedded_faces {
            symbols.push(self.faces[id as usize].family.clone());
        }

        let emoji_like = class.emoji_vs || (class.wide && !is_cjk(ch) && (ch as u32) >= 0x2300);
        let order: [&[String]; 4] = if emoji_like {
            [&emoji, &symbols, &text, &cjk]
        } else if is_cjk(ch) {
            [&text, &cjk, &symbols, &emoji]
        } else {
            [&text, &symbols, &cjk, &emoji]
        };
        let mut found = None;
        for chain in order {
            if let Some(c) = self.chain_lookup(chain, ch, style) {
                found = Some(c);
                break;
            }
        }

        #[cfg(windows)]
        if found.is_none() {
            self.stats.os_fallback_calls += 1;
            let base = self.cfg.text.first().cloned().unwrap_or_default();
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            if let Some(loc) = self.locator.fallback(s, &self.cfg.han_locale.clone(), &base, style.bold, style.italic)
                && let Some(id) = self.load_located(&loc)
                && self.faces[id as usize].glyph(ch) != 0
            {
                found = Some(self.choice_for_face(id, style));
            }
        }
        self.char_cache.insert((ch, style, class), found);
        found
    }

    pub fn clear_char_cache(&mut self) {
        self.char_cache.clear();
    }
}

// ---------------------------------------------------------------------------
// Cell metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: u32,
    pub height: u32,
    /// Distance from the top of the cell to the baseline.
    pub baseline: u32,
    pub underline_top: u32,
    pub underline_thickness: u32,
    pub strike_top: u32,
    pub strike_thickness: u32,
    pub px_size: f32,
    /// Unrounded advance of '0' (for diagnostics).
    pub advance: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

/// Integer-pixel cell geometry from the primary regular face.
///
/// `line_height` is a multiplier on the font's natural line height (1.0 = font
/// metrics). Extra leading is split above/below the text so glyphs stay
/// vertically centered in the cell.
pub fn compute_metrics(face: &Face, px: f32, line_height: f32) -> CellMetrics {
    let font = face.font_ref();
    let m = font.metrics(&[]).scale(px);
    let zero = font.charmap().map('0');
    let advance = font.glyph_metrics(&[]).scale(px).advance_width(zero);
    let width = advance.round().max(1.0) as u32;

    let ascent = m.ascent.ceil();
    let descent = m.descent.abs().ceil();
    let leading = m.leading.round();
    let natural = ascent + descent + leading;
    let height = (natural * line_height).round().max(1.0);
    let extra = height - natural;
    let baseline = (ascent + (leading / 2.0).floor() + (extra / 2.0).floor()).clamp(1.0, height - 1.0);

    let thickness = m.stroke_size.round().max(1.0);
    // underline_offset is negative below the baseline in OpenType terms.
    let ul_center = baseline - m.underline_offset.round();
    let underline_top =
        (ul_center - (thickness / 2.0).floor()).clamp(baseline + 1.0, (height - thickness).max(baseline + 1.0));
    let strike_top = (baseline - m.strikeout_offset.round() - (thickness / 2.0).floor()).max(0.0);

    CellMetrics {
        width,
        height: height as u32,
        baseline: baseline as u32,
        underline_top: underline_top.min(height - thickness) as u32,
        underline_thickness: thickness as u32,
        strike_top: strike_top as u32,
        strike_thickness: thickness as u32,
        px_size: px,
        advance,
        ascent: m.ascent,
        descent: m.descent,
        leading: m.leading,
    }
}
