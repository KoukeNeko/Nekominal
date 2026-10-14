//! WCAG 2.x contrast math on sRGB bytes: used for `Theme::min_contrast` and for choosing readable
//! text colours when a `Theme` is built from a handful of scheme colours.

use std::sync::OnceLock;

use crate::cell::Rgb;

const WHITE: Rgb = Rgb(255, 255, 255);
const BLACK: Rgb = Rgb(0, 0, 0);

fn linear_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0.0; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        lut
    })
}

/// WCAG relative luminance, 0.0 (black) to 1.0 (white).
pub fn luminance(c: Rgb) -> f32 {
    let lut = linear_lut();
    0.2126 * lut[c.0 as usize] + 0.7152 * lut[c.1 as usize] + 0.0722 * lut[c.2 as usize]
}

/// WCAG contrast ratio, 1.0 (identical) to 21.0 (black on white).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// `fg` moved toward white or black (in sRGB, so hue is mostly kept) until it reaches `min` contrast
/// against `bg`. The direction that keeps `fg`'s side of `bg` is tried first; when neither extreme reaches
/// `min` (for example 21:1 on mid grey) the better extreme is returned. Returns `fg` unchanged when it
/// already passes.
pub fn ensure_min_contrast(fg: Rgb, bg: Rgb, min: f32) -> Rgb {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let order = if luminance(fg) >= luminance(bg) { [WHITE, BLACK] } else { [BLACK, WHITE] };
    for target in order {
        if contrast_ratio(target, bg) < min {
            continue;
        }
        // Scan from `fg` toward the extreme, then refine the last step: the ratio is not monotonic when
        // the path crosses `bg`'s luminance, so a plain bisection would be wrong there.
        const STEPS: u32 = 32;
        let mut hi = 1.0;
        let mut lo = 0.0;
        for step in 1..=STEPS {
            let t = step as f32 / STEPS as f32;
            if contrast_ratio(mix(fg, target, t), bg) >= min {
                hi = t;
                break;
            }
            lo = t;
        }
        for _ in 0..5 {
            let mid = (lo + hi) / 2.0;
            if contrast_ratio(mix(fg, target, mid), bg) >= min {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        return mix(fg, target, hi);
    }
    if contrast_ratio(WHITE, bg) >= contrast_ratio(BLACK, bg) { WHITE } else { BLACK }
}

/// The first candidate readable on `bg` (contrast of at least 4.5, WCAG AA for normal text), else the
/// candidate or black/white with the highest contrast.
pub fn readable_on(bg: Rgb, candidates: &[Rgb]) -> Rgb {
    const AA: f32 = 4.5;
    if let Some(&c) = candidates.iter().find(|&&c| contrast_ratio(c, bg) >= AA) {
        return c;
    }
    candidates
        .iter()
        .copied()
        .chain([WHITE, BLACK])
        .max_by(|&a, &b| contrast_ratio(a, bg).total_cmp(&contrast_ratio(b, bg)))
        .unwrap_or(WHITE)
}

/// Linear interpolation between two colours (`t` = 0 gives `a`).
pub fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    mix(a, b, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_reference_values() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-6);
        // #767676 on white is the classic 4.54:1 AA threshold example.
        let r = contrast_ratio(Rgb(0x76, 0x76, 0x76), WHITE);
        assert!((r - 4.54).abs() < 0.02, "{r}");
    }

    #[test]
    fn passing_colours_are_returned_untouched() {
        let fg = Rgb(0xCC, 0xCC, 0xCC);
        assert_eq!(ensure_min_contrast(fg, Rgb(0x0C, 0x0C, 0x0C), 4.5), fg);
    }

    #[test]
    fn low_contrast_text_is_lightened_on_a_dark_background_and_reaches_the_floor() {
        let bg = Rgb(0x00, 0x00, 0x40);
        let fg = Rgb(0x00, 0x37, 0xDA); // Campbell blue on a dark blue
        assert!(contrast_ratio(fg, bg) < 4.5);
        let out = ensure_min_contrast(fg, bg, 4.5);
        assert!(contrast_ratio(out, bg) >= 4.5, "{out:?}");
        assert!(luminance(out) > luminance(fg), "kept its side of the background");
        // Smallest adjustment: one scan step less would not have passed (within the refinement resolution).
        assert!(contrast_ratio(out, bg) < 4.7, "{}", contrast_ratio(out, bg));
    }

    #[test]
    fn light_background_darkens_the_text() {
        let bg = Rgb(0xF2, 0xF2, 0xF2);
        let fg = Rgb(0xE0, 0xE0, 0x00);
        let out = ensure_min_contrast(fg, bg, 4.5);
        assert!(contrast_ratio(out, bg) >= 4.5);
        assert!(luminance(out) < luminance(bg));
    }

    #[test]
    fn crossing_to_the_other_side_when_the_near_side_cannot_reach_the_floor() {
        // Light grey text on a light background: white is not enough, so the text goes dark.
        let bg = Rgb(0xE8, 0xE8, 0xE8);
        let fg = Rgb(0xF0, 0xF0, 0xF0);
        let out = ensure_min_contrast(fg, bg, 4.5);
        assert!(contrast_ratio(out, bg) >= 4.5, "{out:?}");
        assert!(luminance(out) < luminance(bg));
    }

    #[test]
    fn unreachable_floor_returns_the_better_extreme() {
        let bg = Rgb(0x77, 0x77, 0x77);
        assert_eq!(ensure_min_contrast(Rgb(0x70, 0x70, 0x70), bg, 21.0), BLACK);
    }

    #[test]
    fn readable_on_prefers_the_first_passing_candidate() {
        let bg = Rgb(0xFF, 0xFF, 0xFF);
        assert_eq!(readable_on(bg, &[Rgb(0xCC, 0xCC, 0xCC), Rgb(0x20, 0x20, 0x20)]), Rgb(0x20, 0x20, 0x20));
        // Nothing passes: black or white wins over a worse candidate.
        assert_eq!(readable_on(Rgb(0x80, 0x80, 0x80), &[Rgb(0x90, 0x90, 0x90)]), BLACK);
    }
}
