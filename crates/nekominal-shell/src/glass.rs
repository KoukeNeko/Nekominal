//! The colours of the glass chrome, derived from the active terminal colour scheme.
//!
//! The chrome never owns colours of its own: text and glyphs take the scheme foreground, the accent is the
//! scheme's blue, and a scheme whose background is light switches every translucent overlay from white to
//! black. Pure functions only; the shell turns the results into brushes.

use nekominal_model::{
    settings::Backdrop,
    themes::{Color, Scheme},
};
use nekominal_render as r;

/// Everything the top bar, tab chips, dividers and overlays are painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    /// The scheme background is light: overlays are black instead of white and the window theme is Light.
    pub light: bool,
    pub background: Color,
    pub foreground: Color,
    pub accent: Color,
    pub danger: Color,
    pub warning: Color,
    pub success: Color,
    /// A merged pull request, and a port that listens.
    pub magenta: Color,
    pub cyan: Color,
}

/// ANSI palette slots of the scheme the chrome borrows from.
const ANSI_RED: usize = 1;
const ANSI_GREEN: usize = 2;
const ANSI_YELLOW: usize = 3;
const ANSI_BLUE: usize = 4;
const ANSI_MAGENTA: usize = 5;
const ANSI_CYAN: usize = 6;

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..color }
}

impl Glass {
    pub fn from_scheme(scheme: &Scheme) -> Glass {
        Glass {
            light: scheme.background.is_light(),
            background: scheme.background,
            foreground: scheme.foreground,
            accent: scheme.ansi[ANSI_BLUE],
            danger: scheme.ansi[ANSI_RED],
            warning: scheme.ansi[ANSI_YELLOW],
            success: scheme.ansi[ANSI_GREEN],
            magenta: scheme.ansi[ANSI_MAGENTA],
            cyan: scheme.ansi[ANSI_CYAN],
        }
    }

    /// White on dark schemes, black on light ones: the colour translucent overlays are made of.
    fn overlay(&self) -> Color {
        if self.light { Color::rgb(0x000000) } else { Color::rgb(0xFFFFFF) }
    }

    /// A scheme colour for a mark that must stay visible on the chrome (a status glyph, a ring): moved toward black
    /// or white until it has [`MIN_MARK_CONTRAST`] against the scheme background, and left alone when it already has.
    pub fn mark(&self, color: Color) -> Color {
        let rgb = |c: Color| r::Rgb(c.r, c.g, c.b);
        let seen = r::contrast::ensure_min_contrast(rgb(color), rgb(self.background), MIN_MARK_CONTRAST);
        Color { r: seen.0, g: seen.1, b: seen.2, a: 255 }
    }

    pub fn text(&self) -> Color {
        self.foreground
    }

    pub fn text_secondary(&self) -> Color {
        with_alpha(self.foreground, 0.62)
    }

    /// Selected tab chip: a translucent lift of the glass.
    pub fn chip_selected(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.09 } else { 0.16 })
    }

    pub fn chip_selected_border(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.16 } else { 0.22 })
    }

    pub fn hover(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.06 } else { 0.09 })
    }

    pub fn pressed(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.10 } else { 0.14 })
    }

    /// The bar behind the tabs that are not selected: darker than the terminal, which the selected tab joins.
    pub fn tab_rest(&self) -> Color {
        with_alpha(Color::rgb(0x000000), if self.light { 0.06 } else { 0.28 })
    }

    /// The tool cluster and the other resting surfaces on the bar.
    pub fn cluster(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.05 } else { 0.07 })
    }

    /// 1 DIP divider between panes and the bar's lower edge.
    pub fn divider(&self) -> Color {
        with_alpha(self.foreground, 0.12)
    }

}

/// Alpha of the tint over the top bar. The terminal body is painted with the opacity setting; the bar uses
/// slightly less so the blur reads strongest at the top, but never so little that the title text loses
/// contrast on a bright backdrop. An opaque terminal has an opaque bar.
pub fn bar_tint_alpha(opacity: f32) -> f32 {
    if opacity >= 1.0 { 1.0 } else { (opacity * 0.85).clamp(0.40, 0.95) }
}

/// What the system does to the material behind the window, apart from the one that was asked for. Acrylic and Mica are
/// made one flat colour while the window is not active, and while transparency effects are off (the Windows setting,
/// which saving battery turns off as well): the chrome then has to look intentional on a flat colour of the system's
/// choosing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub window_active: bool,
    pub effects_on: bool,
}

impl Material {
    /// The window is active and the effects are on: the backdrop shows as asked.
    pub const NORMAL: Material = Material { window_active: true, effects_on: true };

    /// Nothing shows through whatever the backdrop is: it is none, or the system made it flat.
    pub fn is_flat(self, backdrop: Backdrop) -> bool {
        backdrop == Backdrop::None || !self.window_active || !self.effects_on
    }
}

/// The opacity the terminal is painted with: the setting while a backdrop shows through, and opaque once it is flat, so
/// the scheme keeps its own colour instead of mixing with the one the system chose.
pub fn terminal_opacity(backdrop: Backdrop, setting: f64, material: Material) -> f32 {
    if material.is_flat(backdrop) { 1.0 } else { setting.clamp(0.0, 1.0) as f32 }
}

/// Lowest contrast between a status mark and the background it sits on (WCAG non-text contrast).
pub const MIN_MARK_CONTRAST: f32 = 3.0;

/// Lowest contrast ratio between text and its background that the terminals keep (WCAG large-text AA). Text of a
/// program below it, such as PSReadLine's bright white on a light scheme, is moved toward black or white.
pub const MIN_TEXT_CONTRAST: f32 = 3.0;

/// The renderer's theme for a colour scheme at the given terminal opacity.
pub fn render_theme(scheme: &Scheme, opacity: f32) -> r::Theme {
    let rgb = |color: Color| r::Rgb(color.r, color.g, color.b);
    // A scheme may give its selection as a translucent overlay; the renderer wants the colour it looks like.
    let selection = if scheme.selection.a < 255 {
        scheme.background.mix(scheme.selection, f32::from(scheme.selection.a) / 255.0)
    } else {
        scheme.selection
    };
    let mut theme = r::Theme::from_parts(
        rgb(scheme.background),
        rgb(scheme.foreground),
        rgb(scheme.cursor),
        rgb(selection),
        scheme.ansi.map(rgb),
    );
    theme.set_background_opacity(opacity);
    theme.min_contrast = MIN_TEXT_CONTRAST;
    theme
}
#[cfg(test)]
mod tests {
    use nekominal_model::themes::{self, SCHEMES};

    use super::*;

    #[test]
    fn dark_schemes_get_white_overlays_and_light_ones_black() {
        let dark = Glass::from_scheme(themes::scheme("Tokyo Night"));
        let light = Glass::from_scheme(themes::scheme("One Half Light"));
        assert!(!dark.light && light.light);
        assert_eq!((dark.chip_selected().r, dark.chip_selected().a), (255, 41));
        assert_eq!((light.chip_selected().r, light.chip_selected().a), (0, 23));
    }

    #[test]
    fn accent_is_the_scheme_blue() {
        for scheme in &SCHEMES {
            assert_eq!(Glass::from_scheme(scheme).accent, scheme.ansi[4], "{}", scheme.name);
        }
    }

    #[test]
    fn every_scheme_is_classified_by_its_background() {
        for scheme in &SCHEMES {
            assert_eq!(Glass::from_scheme(scheme).light, scheme.background.luminance() > 0.5, "{}", scheme.name);
        }
    }

    #[test]
    fn selected_chip_is_stronger_than_hover_and_pressed_stronger_than_hover() {
        for scheme in &SCHEMES {
            let glass = Glass::from_scheme(scheme);
            assert!(glass.chip_selected().a > glass.hover().a, "{}", scheme.name);
            assert!(glass.pressed().a > glass.hover().a, "{}", scheme.name);
        }
    }

    #[test]
    fn bar_tint_keeps_a_floor_and_follows_an_opaque_terminal() {
        assert_eq!(bar_tint_alpha(1.0), 1.0);
        assert_eq!(bar_tint_alpha(0.2), 0.40);
        assert!(bar_tint_alpha(0.72) < 0.72);
        assert!(bar_tint_alpha(0.5) < bar_tint_alpha(0.9));
    }

    #[test]
    fn without_a_backdrop_the_terminal_is_opaque() {
        assert_eq!(terminal_opacity(Backdrop::None, 0.5, Material::NORMAL), 1.0);
        assert_eq!(terminal_opacity(Backdrop::Acrylic, 0.72, Material::NORMAL), 0.72);
        assert_eq!(terminal_opacity(Backdrop::Mica, 7.0, Material::NORMAL), 1.0);
    }

    #[test]
    fn a_flat_backdrop_is_painted_opaque_whatever_the_setting_and_the_slider_does_not_follow_the_window() {
        let inactive = Material { window_active: false, ..Material::NORMAL };
        let off = Material { effects_on: false, ..Material::NORMAL };
        for backdrop in [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt] {
            assert!(!Material::NORMAL.is_flat(backdrop), "{backdrop:?} shows through while the window is active");
            for flat in [inactive, off, Material { window_active: false, effects_on: false }] {
                assert!(flat.is_flat(backdrop), "{backdrop:?} {flat:?}");
                assert_eq!(terminal_opacity(backdrop, 0.5, flat), 1.0, "{backdrop:?} {flat:?}");
            }
            assert_eq!(terminal_opacity(backdrop, 0.5, Material::NORMAL), 0.5);
        }
        assert!(Material::NORMAL.is_flat(Backdrop::None), "no backdrop is flat in any case");
        // An opaque terminal has an opaque bar: the bar keeps the tint of the scheme and not the system's colour.
        assert_eq!(bar_tint_alpha(terminal_opacity(Backdrop::Acrylic, 0.5, inactive)), 1.0);
    }

    #[test]
    fn status_marks_keep_three_to_one_against_every_scheme_and_stay_in_their_hue_when_they_can() {
        for scheme in &SCHEMES {
            let glass = Glass::from_scheme(scheme);
            let background = r::Rgb(scheme.background.r, scheme.background.g, scheme.background.b);
            for (name, color) in [
                ("accent", glass.accent),
                ("warning", glass.warning),
                ("success", glass.success),
                ("danger", glass.danger),
            ] {
                let mark = glass.mark(color);
                let ratio = r::contrast::contrast_ratio(r::Rgb(mark.r, mark.g, mark.b), background);
                assert!(ratio >= MIN_MARK_CONTRAST - 0.01, "{} {name}: {ratio:.2}", scheme.name);
                let was = r::contrast::contrast_ratio(r::Rgb(color.r, color.g, color.b), background);
                if was >= MIN_MARK_CONTRAST {
                    assert_eq!(
                        mark,
                        Color { a: 255, ..color },
                        "{} {name} already passes and is left alone",
                        scheme.name
                    );
                }
            }
        }
    }

    #[test]
    fn terminals_keep_a_minimum_text_contrast_on_every_scheme() {
        for scheme in &SCHEMES {
            let theme = render_theme(scheme, 0.72);
            assert_eq!(theme.min_contrast, MIN_TEXT_CONTRAST, "{}", scheme.name);
            assert_eq!(theme.background_opacity, 0.72, "{}", scheme.name);
            assert_eq!(theme.background, r::Rgb(scheme.background.r, scheme.background.g, scheme.background.b));
        }
    }

    #[test]
    fn a_translucent_selection_is_mixed_into_the_background() {
        let campbell = themes::scheme("Campbell");
        assert!(campbell.selection.a < 255, "Campbell gives its selection as an overlay");
        let theme = render_theme(campbell, 1.0);
        let mixed = campbell.background.mix(campbell.selection, f32::from(campbell.selection.a) / 255.0);
        assert_eq!(theme.selection_bg, r::Rgb(mixed.r, mixed.g, mixed.b));
    }
}
