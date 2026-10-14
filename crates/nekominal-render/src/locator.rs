//! System font discovery through DirectWrite (Windows only).
//!
//! DirectWrite is used for *discovery only*: family name -> (file, face index,
//! actual weight/style) and `IDWriteFontFallback::MapCharacters` for the
//! last-resort per-character fallback. Rasterization and shaping stay in
//! swash/harfrust so the renderer is deterministic and testable on WARP/CI.

use std::path::PathBuf;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{BOOL, Interface, PCWSTR, Result, implement};

#[derive(Clone, Debug)]
pub struct Located {
    pub path: PathBuf,
    pub index: u32,
    pub family: String,
    /// Weight of the font DirectWrite picked (400 regular, 700 bold, ...).
    pub weight: u16,
    /// True only when the picked font is a real italic/oblique face.
    pub italic: bool,
}

pub struct Locator {
    factory: IDWriteFactory2,
    collection: IDWriteFontCollection,
    fallback: IDWriteFontFallback,
}

#[implement(IDWriteTextAnalysisSource)]
struct Source {
    text: Vec<u16>,
    locale: Vec<u16>, // NUL-terminated
}

#[allow(non_snake_case)]
impl IDWriteTextAnalysisSource_Impl for Source_Impl {
    fn GetTextAtPosition(&self, pos: u32, text: *mut *mut u16, len: *mut u32) -> Result<()> {
        unsafe {
            if (pos as usize) < self.text.len() {
                *text = self.text.as_ptr().add(pos as usize) as *mut u16;
                *len = (self.text.len() - pos as usize) as u32;
            } else {
                *text = std::ptr::null_mut();
                *len = 0;
            }
        }
        Ok(())
    }
    fn GetTextBeforePosition(&self, pos: u32, text: *mut *mut u16, len: *mut u32) -> Result<()> {
        unsafe {
            if pos > 0 && (pos as usize) <= self.text.len() {
                *text = self.text.as_ptr() as *mut u16;
                *len = pos;
            } else {
                *text = std::ptr::null_mut();
                *len = 0;
            }
        }
        Ok(())
    }
    fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
        DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
    }
    fn GetLocaleName(&self, _pos: u32, len: *mut u32, locale: *mut *mut u16) -> Result<()> {
        unsafe {
            *len = (self.text.len() as u32).max(1);
            *locale = self.locale.as_ptr() as *mut u16;
        }
        Ok(())
    }
    fn GetNumberSubstitution(
        &self,
        _pos: u32,
        len: *mut u32,
        sub: windows::core::OutRef<'_, IDWriteNumberSubstitution>,
    ) -> Result<()> {
        unsafe {
            *len = (self.text.len() as u32).max(1);
        }
        sub.write(None)?;
        Ok(())
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn english_family_name(font: &IDWriteFont) -> Option<String> {
    unsafe {
        let names = font.GetFontFamily().ok()?.GetFamilyNames().ok()?;
        let mut idx = 0u32;
        let mut exists = BOOL(0);
        let en = wide("en-us");
        let _ = names.FindLocaleName(PCWSTR(en.as_ptr()), &mut idx, &mut exists);
        if !exists.as_bool() {
            idx = 0;
        }
        let len = names.GetStringLength(idx).ok()?;
        let mut buf = vec![0u16; len as usize + 1];
        names.GetString(idx, &mut buf).ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn describe(font: &IDWriteFont) -> Option<Located> {
    unsafe {
        let face = font.CreateFontFace().ok()?;
        let mut n = 0u32;
        face.GetFiles(&mut n, None).ok()?;
        let mut files: Vec<Option<IDWriteFontFile>> = vec![None; n as usize];
        face.GetFiles(&mut n, Some(files.as_mut_ptr())).ok()?;
        let file = files.first()?.as_ref()?;
        let mut key = std::ptr::null_mut();
        let mut key_size = 0u32;
        file.GetReferenceKey(&mut key, &mut key_size).ok()?;
        let local: IDWriteLocalFontFileLoader = file.GetLoader().ok()?.cast().ok()?;
        let len = local.GetFilePathLengthFromKey(key, key_size).ok()?;
        let mut buf = vec![0u16; len as usize + 1];
        local.GetFilePathFromKey(key, key_size, &mut buf).ok()?;
        Some(Located {
            path: PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])),
            index: face.GetIndex(),
            family: english_family_name(font)?,
            weight: font.GetWeight().0 as u16,
            italic: font.GetStyle() != DWRITE_FONT_STYLE_NORMAL,
        })
    }
}

fn weight(bold: bool) -> DWRITE_FONT_WEIGHT {
    if bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL }
}

fn style(italic: bool) -> DWRITE_FONT_STYLE {
    if italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL }
}

impl Locator {
    pub fn new() -> Result<Self> {
        unsafe {
            let factory: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut collection = None;
            factory.GetSystemFontCollection(&mut collection, false)?;
            let fallback = factory.GetSystemFontFallback()?;
            Ok(Locator { factory, collection: collection.expect("system collection"), fallback })
        }
    }

    pub fn family_names(&self) -> Vec<String> {
        unsafe {
            let n = self.collection.GetFontFamilyCount();
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n {
                let Ok(fam) = self.collection.GetFontFamily(i) else { continue };
                let Ok(names) = fam.GetFamilyNames() else { continue };
                let mut idx = 0u32;
                let mut exists = BOOL(0);
                let en = wide("en-us");
                let _ = names.FindLocaleName(PCWSTR(en.as_ptr()), &mut idx, &mut exists);
                if !exists.as_bool() {
                    idx = 0;
                }
                let Ok(len) = names.GetStringLength(idx) else { continue };
                let mut buf = vec![0u16; len as usize + 1];
                if names.GetString(idx, &mut buf).is_ok() {
                    out.push(String::from_utf16_lossy(&buf[..len as usize]));
                }
            }
            out
        }
    }

    /// Family name -> best matching face for the requested weight/style.
    pub fn resolve(&self, family: &str, bold: bool, italic: bool) -> Option<Located> {
        unsafe {
            let name = wide(family);
            let mut idx = 0u32;
            let mut exists = BOOL(0);
            self.collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut idx, &mut exists).ok()?;
            if !exists.as_bool() {
                return None;
            }
            let fam = self.collection.GetFontFamily(idx).ok()?;
            let font = fam.GetFirstMatchingFont(weight(bold), DWRITE_FONT_STRETCH_NORMAL, style(italic)).ok()?;
            describe(&font)
        }
    }

    /// Last-resort fallback: which installed font should draw the first
    /// character of `text`? `locale` steers Han unification ("zh-TW" picks
    /// Microsoft JhengHei UI; "en-US" would pick Yu Gothic UI).
    pub fn fallback(&self, text: &str, locale: &str, base_family: &str, bold: bool, italic: bool) -> Option<Located> {
        unsafe {
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let src: IDWriteTextAnalysisSource = Source { text: utf16.clone(), locale: wide(locale) }.into();
            let base = wide(base_family);
            let mut mapped_len = 0u32;
            let mut mapped: Option<IDWriteFont> = None;
            let mut scale = 0f32;
            self.fallback
                .MapCharacters(
                    &src,
                    0,
                    utf16.len() as u32,
                    None,
                    PCWSTR(base.as_ptr()),
                    weight(bold),
                    style(italic),
                    DWRITE_FONT_STRETCH_NORMAL,
                    &mut mapped_len,
                    &mut mapped,
                    &mut scale,
                )
                .ok()?;
            describe(&mapped?)
        }
    }

    pub fn factory(&self) -> &IDWriteFactory2 {
        &self.factory
    }
}
