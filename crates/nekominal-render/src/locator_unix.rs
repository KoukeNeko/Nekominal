//! System font discovery through fontconfig (Linux only), the counterpart of the DirectWrite locator.
//!
//! fontconfig is used for *discovery only*: family name -> (file, face index, actual weight/slant) and a per-character
//! fallback sorted by closeness to a base family and a language. Rasterization and shaping stay in swash/harfrust. The
//! library is opened when the program runs (`libfontconfig.so.1`), so building needs no development package.

use std::{
    ffi::{CStr, CString, c_char, c_int},
    path::PathBuf,
    ptr::null_mut,
};

use fontconfig_sys::{
    FcCharSet, FcFontSet, FcMatchPattern, FcPattern, FcResult, FcResultMatch, constants::{FC_CHARSET, FC_FAMILY, FC_FILE, FC_INDEX, FC_LANG, FC_SLANT, FC_WEIGHT},
    statics::LIB,
};

#[derive(Clone, Debug)]
pub struct Located {
    pub path: PathBuf,
    pub index: u32,
    pub family: String,
    /// Weight of the font fontconfig picked (400 regular, 700 bold, ...).
    pub weight: u16,
    /// True only when the picked font is a real italic/oblique face.
    pub italic: bool,
}

pub struct Locator;

const FC_WEIGHT_REGULAR: c_int = 80;
const FC_WEIGHT_BOLD: c_int = 200;
const FC_SLANT_ITALIC: c_int = 100;

/// Names fontconfig resolves by its configuration (the user's monospace font) instead of by a family of that name.
const GENERIC_FAMILIES: [&str; 6] = ["monospace", "sans-serif", "serif", "emoji", "system-ui", "ui-monospace"];

/// OpenType weight of a fontconfig weight (fontconfig's scale is its own: 80 is regular, 200 is bold).
fn css_weight(fc: c_int) -> u16 {
    match fc {
        ..=0 => 100,
        1..=40 => 200,
        41..=50 => 300,
        51..=80 => 400,
        81..=100 => 500,
        101..=180 => 600,
        181..=200 => 700,
        201..=205 => 800,
        _ => 900,
    }
}

/// A fontconfig pattern, destroyed when dropped.
struct Pattern(*mut FcPattern);

impl Pattern {
    fn new() -> Option<Pattern> {
        // SAFETY: plain call; null is the failure.
        let pattern = unsafe { (LIB.FcPatternCreate)() };
        (!pattern.is_null()).then_some(Pattern(pattern))
    }

    fn add_string(&self, object: &CStr, value: &str) {
        let Ok(value) = CString::new(value) else { return };
        // SAFETY: the pattern is live and copies the string.
        unsafe { (LIB.FcPatternAddString)(self.0, object.as_ptr(), value.as_ptr().cast()) };
    }

    fn add_integer(&self, object: &CStr, value: c_int) {
        // SAFETY: the pattern is live.
        unsafe { (LIB.FcPatternAddInteger)(self.0, object.as_ptr(), value) };
    }

    /// The preferences and the defaults of the system, as a match needs them.
    fn complete(&self) {
        // SAFETY: the pattern is live; a null configuration is the current one.
        unsafe {
            (LIB.FcConfigSubstitute)(null_mut(), self.0, FcMatchPattern);
            (LIB.FcDefaultSubstitute)(self.0);
        }
    }
}

impl Drop for Pattern {
    fn drop(&mut self) {
        // SAFETY: made by FcPatternCreate and destroyed once.
        unsafe { (LIB.FcPatternDestroy)(self.0) };
    }
}

fn string_of(pattern: *mut FcPattern, object: &CStr, n: c_int) -> Option<String> {
    let mut value: *mut u8 = null_mut();
    // SAFETY: the pattern is live; the string is owned by it and copied here.
    unsafe {
        if (LIB.FcPatternGetString)(pattern, object.as_ptr(), n, &mut value) != FcResultMatch || value.is_null() {
            return None;
        }
        Some(CStr::from_ptr(value.cast::<c_char>()).to_string_lossy().into_owned())
    }
}

fn integer_of(pattern: *mut FcPattern, object: &CStr) -> Option<c_int> {
    let mut value = 0;
    // SAFETY: the pattern is live.
    (unsafe { (LIB.FcPatternGetInteger)(pattern, object.as_ptr(), 0, &mut value) } == FcResultMatch).then_some(value)
}

/// The file and face of a font pattern that a match gave.
fn describe(font: *mut FcPattern) -> Option<Located> {
    let path = PathBuf::from(string_of(font, FC_FILE, 0)?);
    Some(Located {
        path,
        // The high half of the index names an instance of a variable font; the face is the low half.
        index: (integer_of(font, FC_INDEX).unwrap_or(0) & 0xFFFF) as u32,
        family: string_of(font, FC_FAMILY, 0)?,
        weight: css_weight(integer_of(font, FC_WEIGHT).unwrap_or(FC_WEIGHT_REGULAR)),
        italic: integer_of(font, FC_SLANT).unwrap_or(0) >= FC_SLANT_ITALIC,
    })
}

fn base_pattern(family: &str, bold: bool, italic: bool) -> Option<Pattern> {
    let pattern = Pattern::new()?;
    pattern.add_string(FC_FAMILY, family);
    pattern.add_integer(FC_WEIGHT, if bold { FC_WEIGHT_BOLD } else { FC_WEIGHT_REGULAR });
    pattern.add_integer(FC_SLANT, if italic { FC_SLANT_ITALIC } else { 0 });
    Some(pattern)
}

impl Locator {
    pub fn new() -> Result<Self, String> {
        // SAFETY: loads the library and the configuration; both are kept for the life of the program.
        unsafe {
            fontconfig_sys::statics::LIB_RESULT.as_ref().map_err(|e| format!("libfontconfig: {e}"))?;
            if (LIB.FcInit)() == 0 {
                return Err("FcInit failed".into());
            }
        }
        Ok(Locator)
    }

    /// Family name -> best matching face for the requested weight/style. None when no installed family has that name
    /// (fontconfig answers with the closest font whatever is asked), except for the generic names it resolves itself.
    pub fn resolve(&self, family: &str, bold: bool, italic: bool) -> Option<Located> {
        let pattern = base_pattern(family, bold, italic)?;
        pattern.complete();
        let mut result: FcResult = 0;
        // SAFETY: the pattern is live; the match is destroyed below.
        let font = unsafe { (LIB.FcFontMatch)(null_mut(), pattern.0, &mut result) };
        if font.is_null() {
            return None;
        }
        let located = describe(font).filter(|_| {
            GENERIC_FAMILIES.iter().any(|generic| generic.eq_ignore_ascii_case(family))
                || (0..).map_while(|n| string_of(font, FC_FAMILY, n)).any(|name| name.eq_ignore_ascii_case(family))
        });
        // SAFETY: made by FcFontMatch and destroyed once.
        unsafe { (LIB.FcPatternDestroy)(font) };
        located
    }

    /// Last-resort fallback: which installed font should draw the first character of `text`? `locale` steers Han
    /// unification ("zh-TW" picks a Traditional Chinese face where there is one).
    pub fn fallback(&self, text: &str, locale: &str, base_family: &str, bold: bool, italic: bool) -> Option<Located> {
        let ch = text.chars().next()?;
        let pattern = base_pattern(base_family, bold, italic)?;
        pattern.add_string(FC_LANG, locale);
        // SAFETY: the charset is destroyed below; the pattern copies it.
        let charset = unsafe { (LIB.FcCharSetCreate)() };
        if charset.is_null() {
            return None;
        }
        // SAFETY: the charset and the pattern are live.
        unsafe {
            (LIB.FcCharSetAddChar)(charset, u32::from(ch));
            (LIB.FcPatternAddCharSet)(pattern.0, FC_CHARSET.as_ptr(), charset);
            (LIB.FcCharSetDestroy)(charset);
        }
        pattern.complete();
        let mut result: FcResult = 0;
        // SAFETY: the pattern is live; the set is destroyed below.
        let set: *mut FcFontSet = unsafe { (LIB.FcFontSort)(null_mut(), pattern.0, 1, null_mut(), &mut result) };
        if set.is_null() {
            return None;
        }
        // SAFETY: the set is live; its fonts are owned by it and described before it is destroyed.
        let located = unsafe {
            let fonts = std::slice::from_raw_parts((*set).fonts, (*set).nfont as usize);
            fonts.iter().copied().find(|&font| covers(font, ch)).and_then(|font| describe(font))
        };
        // SAFETY: made by FcFontSort and destroyed once.
        unsafe { (LIB.FcFontSetDestroy)(set) };
        located
    }
}

/// Whether the font of a pattern has a glyph for `ch`.
fn covers(font: *mut FcPattern, ch: char) -> bool {
    let mut charset: *mut FcCharSet = null_mut();
    // SAFETY: the pattern is live; the charset is owned by it.
    unsafe {
        (LIB.FcPatternGetCharSet)(font, FC_CHARSET.as_ptr(), 0, &mut charset) == FcResultMatch
            && !charset.is_null()
            && (LIB.FcCharSetHasChar)(charset, u32::from(ch)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_that_is_installed_is_found_and_one_that_is_not_is_not() {
        let locator = Locator::new().expect("fontconfig");
        let mono = locator.resolve("monospace", false, false).expect("the system has a monospace font");
        assert!(mono.path.is_file(), "{mono:?}");
        assert!(locator.resolve("No Such Family Nekominal", false, false).is_none());
    }

    #[test]
    fn bold_asks_for_a_heavier_face_of_the_same_family() {
        let locator = Locator::new().expect("fontconfig");
        let regular = locator.resolve("monospace", false, false).unwrap();
        let bold = locator.resolve("monospace", true, false).unwrap();
        assert!(bold.weight >= regular.weight, "{regular:?} {bold:?}");
    }

    #[test]
    fn a_character_no_listed_family_draws_gets_a_font_that_has_it() {
        let locator = Locator::new().expect("fontconfig");
        let Some(found) = locator.fallback("\u{4E2D}", "zh-TW", "monospace", false, false) else {
            eprintln!("skipped: no installed font has Han characters");
            return;
        };
        assert!(found.path.is_file(), "{found:?}");
    }
}
