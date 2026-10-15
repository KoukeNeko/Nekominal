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
}

/// ANSI palette slots of the scheme the chrome borrows from.
const ANSI_RED: usize = 1;
const ANSI_GREEN: usize = 2;
const ANSI_YELLOW: usize = 3;
const ANSI_BLUE: usize = 4;

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

    /// The tool cluster and the other resting surfaces on the bar.
    pub fn cluster(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.05 } else { 0.07 })
    }

    /// 1 DIP divider between panes and the bar's lower edge.
    pub fn divider(&self) -> Color {
        with_alpha(self.foreground, 0.12)
    }

    /// Status dot of a tab with nothing to report.
    pub fn dot_idle(&self) -> Color {
        with_alpha(self.foreground, 0.30)
    }

    /// Veil laid over the terminal while an overlay (palette) is open.
    pub fn veil(&self) -> Color {
        with_alpha(self.background, 0.45)
    }

    /// Surface of floating panels (palette, find bar, the settings drawer): the scheme background, nearly opaque so
    /// text stays readable over whatever the backdrop shows. The alpha is fixed: it does not follow the opacity of the
    /// terminal, so a panel is as readable at 20 % as at 100 %.
    pub fn panel(&self) -> Color {
        with_alpha(self.background, PANEL_ALPHA)
    }

    /// What the panel looks like over `backdrop`, which is whatever shows through it (the wallpaper, the terminal).
    pub fn panel_over(&self, backdrop: Color) -> Color {
        backdrop.mix(Color { a: 255, ..self.background }, PANEL_ALPHA)
    }

    /// `color` moved toward black or white, as far as it takes, until it has `min` contrast on the panel over every
    /// backdrop of [`WORST_BACKDROPS`]; a colour that has it already is left alone. The panel over the backdrop that is
    /// closest in brightness to the colour decides, and moving the colour only widens its distance to the others.
    fn legible(&self, color: Color, min: f32) -> Color {
        let rgb = |c: Color| r::Rgb(c.r, c.g, c.b);
        let ratio = |color: Color, panel: Color| r::contrast::contrast_ratio(rgb(color), rgb(panel));
        let mut color = Color { a: 255, ..color };
        for _ in 0..3 {
            let worst = WORST_BACKDROPS
                .iter()
                .map(|backdrop| self.panel_over(Color::rgb(*backdrop)))
                .min_by(|a, b| ratio(color, *a).total_cmp(&ratio(color, *b)))
                .unwrap_or(self.background);
            if ratio(color, worst) >= min {
                break;
            }
            let moved = r::contrast::ensure_min_contrast(rgb(color), rgb(worst), min);
            color = Color { r: moved.0, g: moved.1, b: moved.2, a: 255 };
        }
        color
    }

    pub fn panel_border(&self) -> Color {
        with_alpha(self.overlay(), if self.light { 0.14 } else { 0.18 })
    }

    /// Text on the settings drawer: the foreground, moved only as far as it takes to read ([`MIN_DRAWER_TEXT_CONTRAST`])
    /// on the panel over any backdrop.
    pub fn panel_text(&self) -> Color {
        self.legible(self.foreground, MIN_DRAWER_TEXT_CONTRAST)
    }

    /// Secondary text on the settings drawer: the text toward the background, opaque (what shows through the panel does
    /// not tint it), and still legible.
    pub fn panel_text_secondary(&self) -> Color {
        let text = self.panel_text();
        let softer = self.legible(text.mix(self.background, 0.35), MIN_DRAWER_TEXT_CONTRAST);
        // Solving for legibility can land a step past the text itself when the text is at the floor: never stronger.
        let distance = |color: Color| (color.luminance() - self.background.luminance()).abs();
        if distance(softer) > distance(text) { text } else { softer }
    }

    /// The outline of a control on the settings drawer ([`MIN_DRAWER_CONTROL_CONTRAST`]).
    pub fn panel_outline(&self) -> Color {
        self.legible(self.foreground.mix(self.background, 0.5), MIN_DRAWER_CONTROL_CONTRAST)
    }

    /// The accent of a choice that is on (its outline and check mark) and of the bar of a modified row.
    pub fn panel_accent(&self) -> Color {
        self.legible(self.mark(self.accent), MIN_DRAWER_CONTROL_CONTRAST)
    }

    /// The tag of a row whose value in the settings file was not valid: text, so as legible as the other text.
    pub fn panel_warning(&self) -> Color {
        self.legible(self.mark(self.warning), MIN_DRAWER_TEXT_CONTRAST)
    }
}

/// Alpha of the tint over the top bar. The terminal body is painted with the opacity setting; the bar uses
/// slightly less so the blur reads strongest at the top, but never so little that the title text loses
/// contrast on a bright backdrop. An opaque terminal has an opaque bar.
pub fn bar_tint_alpha(opacity: f32) -> f32 {
    if opacity >= 1.0 { 1.0 } else { (opacity * 0.85).clamp(0.40, 0.95) }
}

/// The opacity the terminal is painted with: the setting while a backdrop shows through, opaque without one.
pub fn terminal_opacity(backdrop: Backdrop, setting: f64) -> f32 {
    match backdrop {
        Backdrop::None => 1.0,
        _ => setting.clamp(0.0, 1.0) as f32,
    }
}

/// What an opacity slider shows (percent) and whether it can be moved. Without a backdrop nothing shows through,
/// so the slider reads 100% and is off, while the saved setting stays as it is for the next backdrop.
pub fn opacity_control(backdrop: Backdrop, setting: f64) -> (f64, bool) {
    let shown = (f64::from(terminal_opacity(backdrop, setting)) * 100.0).round();
    (shown, backdrop != Backdrop::None)
}

/// How opaque the floating panels are. Fixed, so that what a panel says stays readable whatever the terminal behind it
/// shows, and so that moving the opacity slider does not change the panel it is on. Measured over a striped window (the
/// worst thing that can show through): the contrast of the text of the drawer changed by 0.10 between a background
/// opacity of 20 % and 100 % at an alpha of 0.88, by 0.06 at 0.92 and by 0.00 at 0.96.
pub const PANEL_ALPHA: f32 = 0.92;

/// What the window can have behind a panel: white and black, the saturated primaries and secondaries. The contrast of
/// the drawer is solved for these (a wallpaper is between them).
pub const WORST_BACKDROPS: [u32; 8] = [0xFFFFFF, 0x000000, 0xFF0000, 0x00FF00, 0x0000FF, 0x00FFFF, 0xFF00FF, 0xFFFF00];

/// Lowest contrast of the text of the drawer against its panel (WCAG normal text, AA), and of the outline of a control
/// and of the mark of a choice that is on (WCAG non-text contrast), over any backdrop.
pub const MIN_DRAWER_TEXT_CONTRAST: f32 = 4.5;
pub const MIN_DRAWER_CONTROL_CONTRAST: f32 = 3.0;

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
        assert_eq!(terminal_opacity(Backdrop::None, 0.5), 1.0);
        assert_eq!(terminal_opacity(Backdrop::Acrylic, 0.72), 0.72);
        assert_eq!(terminal_opacity(Backdrop::Mica, 7.0), 1.0);
    }

    #[test]
    fn without_a_backdrop_the_slider_reads_one_hundred_and_is_off_but_the_setting_is_kept() {
        assert_eq!(opacity_control(Backdrop::None, 0.72), (100.0, false));
        assert_eq!(opacity_control(Backdrop::Acrylic, 0.72), (72.0, true));
        assert_eq!(opacity_control(Backdrop::Mica, 0.2), (20.0, true));
        assert_eq!(opacity_control(Backdrop::MicaAlt, 1.0), (100.0, true));
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

    fn rgb(color: Color) -> r::Rgb {
        r::Rgb(color.r, color.g, color.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        r::contrast::contrast_ratio(rgb(a), rgb(b))
    }

    /// Backdrops the rule was not solved for: every mix of 0, 128 and 255 in the three channels (27 colours, among them
    /// white, black and the saturated ones).
    fn backdrops() -> Vec<Color> {
        let levels = [0u32, 128, 255];
        let mut all = Vec::new();
        for red in levels {
            for green in levels {
                for blue in levels {
                    all.push(Color::rgb(red << 16 | green << 8 | blue));
                }
            }
        }
        all
    }

    #[test]
    fn drawer_text_and_controls_read_on_the_panel_over_every_backdrop_for_every_scheme() {
        for scheme in &SCHEMES {
            let glass = Glass::from_scheme(scheme);
            for backdrop in backdrops() {
                let panel = glass.panel_over(backdrop);
                let at = format!("{} over #{:02X}{:02X}{:02X}", scheme.name, backdrop.r, backdrop.g, backdrop.b);
                for (role, color) in [
                    ("text", glass.panel_text()),
                    ("secondary text", glass.panel_text_secondary()),
                    ("warning", glass.panel_warning()),
                ] {
                    let ratio = contrast(color, panel);
                    assert!(ratio >= MIN_DRAWER_TEXT_CONTRAST - 0.01, "{at}: {role} {ratio:.2}");
                }
                for (role, color) in [("outline", glass.panel_outline()), ("accent", glass.panel_accent())] {
                    let ratio = contrast(color, panel);
                    assert!(ratio >= MIN_DRAWER_CONTROL_CONTRAST - 0.01, "{at}: {role} {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn the_panel_fill_is_one_fixed_alpha_and_panel_over_is_that_fill_composited() {
        for scheme in &SCHEMES {
            let glass = Glass::from_scheme(scheme);
            assert_eq!(glass.panel().a, (PANEL_ALPHA * 255.0).round() as u8, "{}", scheme.name);
            let backdrop = Color::rgb(0x336699);
            let alpha = f32::from(glass.panel().a) / 255.0;
            let by_hand =
                |top: u8, under: u8| (f32::from(top) * alpha + f32::from(under) * (1.0 - alpha)).round() as i32;
            let over = glass.panel_over(backdrop);
            for (got, expected) in [
                (over.r, by_hand(glass.background.r, backdrop.r)),
                (over.g, by_hand(glass.background.g, backdrop.g)),
                (over.b, by_hand(glass.background.b, backdrop.b)),
            ] {
                assert!((i32::from(got) - expected).abs() <= 1, "{}", scheme.name);
            }
        }
    }

    #[test]
    fn tokyo_night_secondary_text_on_a_white_wallpaper_was_3_6_to_1_and_now_reads() {
        let glass = Glass::from_scheme(themes::scheme("Tokyo Night"));
        // The measured value of the rule before the drawer took its own: the foreground a third of the way to the
        // background on a panel of alpha 0.88 over white (the study estimated about 3.9 for an alpha blend; the opaque
        // mix measures 3.6).
        let old_panel = Color::rgb(0xFFFFFF).mix(glass.background, 0.88);
        let before = contrast(glass.foreground.mix(glass.background, 0.35), old_panel);
        assert!((3.5..3.7).contains(&before), "{before:.2}");
        let panel = glass.panel_over(Color::rgb(0xFFFFFF));
        let after = contrast(glass.panel_text_secondary(), panel);
        assert!(after >= MIN_DRAWER_TEXT_CONTRAST - 0.01, "{after:.2}");
        assert!(contrast(glass.panel_text(), panel) > 7.0, "the primary text reads without a change");
    }

    #[test]
    fn a_colour_that_reads_already_is_left_alone_and_the_others_move_toward_the_light_or_the_dark_by_the_same_rule() {
        let tokyo = Glass::from_scheme(themes::scheme("Tokyo Night"));
        assert_eq!(tokyo.panel_text(), Color { a: 255, ..tokyo.foreground }, "no per-scheme hack: it already reads");
        let dark = Glass::from_scheme(themes::scheme("Solarized Dark"));
        assert!(
            dark.panel_text().luminance() > dark.foreground.luminance(),
            "a dull foreground on a dark panel gets lighter"
        );
        let light = Glass::from_scheme(themes::scheme("Solarized Light"));
        assert!(light.panel_text().luminance() < light.foreground.luminance(), "and on a light panel darker");
        for scheme in &SCHEMES {
            let glass = Glass::from_scheme(scheme);
            let panel = glass.panel_over(Color::rgb(if glass.light { 0x000000 } else { 0xFFFFFF }));
            assert!(
                contrast(glass.panel_text_secondary(), panel) <= contrast(glass.panel_text(), panel) + 0.01,
                "{}: the secondary text is never stronger than the text",
                scheme.name
            );
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
