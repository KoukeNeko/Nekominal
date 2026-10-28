//! The parts the chrome is made of: the button, the button of the bar, the switch, the radio, the slider, the field and the line
//! of text in it, the row, the row of a menu, the tab and the logo of a shell. Each is drawn here once, in every state it has
//! (at rest, under the pointer, pressed, chosen, being edited), with the colours of the scheme and the measures of `design.rs`.
//! The bar, the pop-ups and the settings call these and draw none of them again, so that one control looks the same wherever
//! it is. Nothing here knows what a control does: the caller says what state it is in.

use nekominal_model::{settings_ui::Range, themes::Color as SchemeColor};
use nekominal_ui::{Color, Rect, Ui};

use crate::{
    design::RADIUS,
    glass::Glass,
    logic::Mark,
    textfield::{self, Cursor},
};

/// The thickness of the track of a slider, and the diameter of its thumb, in DIP.
pub const TRACK: f32 = 4.0;
pub const THUMB: f32 = 16.0;
/// The side of the square a logo is drawn in.
pub const MARK_SIZE: f32 = 18.0;

pub fn col(c: SchemeColor) -> Color {
    Color([c.r, c.g, c.b, c.a])
}

/// What the pointer is doing to a control, and whether it is the one chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub hover: bool,
    pub pressed: bool,
    pub chosen: bool,
}

impl State {
    pub fn hover(hover: bool) -> State {
        State { hover, ..State::default() }
    }

    pub fn chosen(chosen: bool, hover: bool) -> State {
        State { hover, chosen, ..State::default() }
    }
}

/// The height of an item on a plate of its own, wherever one is listed (a row of the settings, a profile, an option of a
/// dialog), the gap under it included, and that gap: one size across the app.
pub const ITEM: f32 = 48.0;
pub const ITEM_GAP: f32 = 4.0;

/// A plain rounded plate of one colour.
pub fn plate(ui: &mut Ui, rect: Rect, fill: Color) {
    ui.rrect(rect, RADIUS, fill, None);
}

/// A button: at rest a plate of the cluster, lighter under the pointer, lighter still pressed; the one chosen (an option of a
/// choice, the button of the dialog the keyboard is on) is lifted off the glass and outlined.
pub fn button(ui: &mut Ui, g: &Glass, rect: Rect, state: State) {
    if state.chosen {
        ui.rrect(rect, RADIUS, col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)));
    } else if state.pressed {
        plate(ui, rect, col(g.pressed()));
    } else {
        plate(ui, rect, if state.hover { col(g.hover()) } else { col(g.cluster()) });
    }
}

/// A button of the bar (a caption button, the menu, the new tab): nothing at rest, a plate as square as the bar under the
/// pointer, while it is pressed and while the pop-up that hangs from it is open (`chosen`).
pub fn bar_button(ui: &mut Ui, g: &Glass, rect: Rect, state: State) {
    if state.pressed {
        ui.fill(rect, col(g.pressed()));
    } else if state.hover || state.chosen {
        ui.fill(rect, col(g.hover()));
    }
}

/// A row of a menu or a list: lit under the pointer, lifted and outlined when it is the current one.
pub fn menu_row(ui: &mut Ui, g: &Glass, rect: Rect, hover: bool, current: bool) {
    if hover {
        plate(ui, rect, col(g.hover()));
    } else if current {
        ui.rrect(rect, RADIUS, col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)));
    }
}

/// The knob of a switch in `switch`: a disc with a gap of 3 to the track, at the end the switch is on or off.
pub fn switch_knob(rect: Rect, on: bool) -> Rect {
    let knob = rect.h - 6.0;
    Rect::new(if on { rect.x + rect.w - knob - 3.0 } else { rect.x + 3.0 }, rect.y + 3.0, knob, knob)
}

/// A switch: a pill that is the accent when it is on, with the knob at its end.
pub fn switch(ui: &mut Ui, g: &Glass, rect: Rect, on: bool, hover: bool) {
    let fill = if on { col(g.accent) } else if hover { col(g.hover()) } else { col(g.pressed()) };
    ui.rrect(rect, rect.h / 2.0, fill, Some((col(g.divider()), 1.0)));
    let knob = switch_knob(rect, on);
    ui.rrect(knob, knob.w / 2.0, col(g.text()), None);
}

/// A radio: a ring that is the accent and has a dot in it when it is the one chosen.
pub fn radio(ui: &mut Ui, g: &Glass, rect: Rect, on: bool, hover: bool) {
    let ring = if on { col(g.accent) } else if hover { col(g.text()) } else { col(g.text_secondary()) };
    ui.rrect(rect, rect.w / 2.0, Color::TRANSPARENT, Some((ring, 1.5)));
    if on {
        ui.rrect(Rect::new(rect.x + 4.0, rect.y + 4.0, rect.w - 8.0, rect.h - 8.0), (rect.w - 8.0) / 2.0, col(g.accent), None);
    }
}

/// The track of a slider inside the rectangle that takes its presses (it reaches half a thumb beyond the track on each side).
pub fn slider_track(area: Rect) -> Rect {
    Rect::new(area.x + THUMB / 2.0, area.y + (area.h - TRACK) / 2.0, area.w - THUMB, TRACK)
}

/// The number a pointer at `x` chooses on a slider whose track is `track` wide, on the grid of the steps of `range`.
pub fn slider_value(range: Range, track: Rect, x: f32) -> f64 {
    let share = f64::from(((x - track.x) / track.w).clamp(0.0, 1.0));
    let steps = (share * (range.max - range.min) / range.step).round();
    (range.min + steps * range.step).clamp(range.min, range.max)
}

/// A slider in `area`, the share (0 to 1) of the way to the end its thumb is at.
pub fn slider(ui: &mut Ui, g: &Glass, area: Rect, share: f32) {
    let track = slider_track(area);
    let share = share.clamp(0.0, 1.0);
    ui.rrect(track, track.h / 2.0, col(g.divider()), None);
    ui.rrect(Rect::new(track.x, track.y, track.w * share, track.h), track.h / 2.0, col(g.accent), None);
    ui.rrect(Rect::new(track.x + track.w * share - THUMB / 2.0, area.y + (area.h - THUMB) / 2.0, THUMB, THUMB), THUMB / 2.0, col(g.text()), None);
}

/// The surface of a field: outlined by the accent while it is being edited.
pub fn field(ui: &mut Ui, g: &Glass, rect: Rect, light: f32) {
    // The faint divider turns into the accent, its alpha with it.
    let (rest, lit) = (col(g.divider()).0, col(g.accent).0);
    let ring = Color(std::array::from_fn(|i| (f32::from(rest[i]) + (f32::from(lit[i]) - f32::from(rest[i])) * light).round() as u8));
    ui.rrect(rect, RADIUS, col(g.cluster()), Some((ring, 1.0)));
}

/// A line of text and, when it is the one being edited, its caret and selection.
pub struct LineEdit<'a> {
    pub text: &'a str,
    /// The caret and the selection; `None` for a line that is not being edited, which is only drawn.
    pub cursor: Option<Cursor>,
    /// What the input method is composing at the caret, underlined.
    pub preedit: &'a str,
    /// What an empty line says in a quiet colour.
    pub placeholder: &'a str,
}

/// How far a line is scrolled left so that the caret, `caret` from its start, stays in `room`, with 4 to spare.
pub fn edit_scroll(caret: f32, room: f32) -> f32 {
    (caret + 4.0 - room).max(0.0)
}

/// Draws the line of text, `size` big, in `area` (clipped to it, centred in its height). The text scrolls so that the caret
/// stays in view. Returns where the caret is: its left, its top and its height, which is where the input method puts its window.
pub fn line_edit(ui: &mut Ui, g: &Glass, area: Rect, size: f32, edit: &LineEdit) -> (f32, f32, f32) {
    let line = ui.line_height(size);
    let y = area.y + (area.h - line) / 2.0;
    let Some(cursor) = edit.cursor else {
        ui.push_clip(area);
        if edit.text.is_empty() {
            ui.text(area.x, y, edit.placeholder, size, col(g.text_secondary()), false);
        } else {
            ui.text(area.x, y, edit.text, size, col(g.text()), false);
        }
        ui.pop_clip();
        return (area.x, y, line);
    };
    let before = textfield::prefix(edit.text, cursor.caret);
    let after = &edit.text[before.len()..];
    let width = ui.measure(before, size, false);
    let composing = ui.measure(edit.preedit, size, false);
    let start = area.x - edit_scroll(width + composing, area.w);
    ui.push_clip(Rect::new(area.x, y, area.w.max(0.0), line));
    if edit.text.is_empty() && edit.preedit.is_empty() {
        ui.text(area.x, y, edit.placeholder, size, col(g.text_secondary()), false);
    }
    if let Some((from, to)) = cursor.selection() {
        let (left, right) = (ui.measure(textfield::prefix(edit.text, from), size, false), ui.measure(textfield::prefix(edit.text, to), size, false));
        ui.fill(Rect::new(start + left, y, right - left, line), Color([g.accent.r, g.accent.g, g.accent.b, 90]));
    }
    ui.text(start, y, before, size, col(g.text()), false);
    ui.text(start + width, y, edit.preedit, size, col(g.text()), false);
    ui.text(start + width + composing, y, after, size, col(g.text()), false);
    if composing > 0.0 {
        ui.fill(Rect::new(start + width, y + line - 1.0, composing, 1.0), col(g.accent));
    }
    let caret = start + width + composing + 1.0;
    // The caret is a bar after the text that does not blink, so that the window needs no timer.
    ui.fill(Rect::new(caret, y, 1.5, line), col(g.text()));
    ui.pop_clip();
    (caret, y, line)
}

/// What stands before a row (the logo of a shell) in `square`: a badge of a colour with its letters in white, or a glyph of the
/// icon font in `glyph_color`.
pub fn mark(ui: &mut Ui, square: Rect, mark: Mark, glyph_color: Color) {
    match mark {
        Mark::Badge { label, color: (r, green, b) } => {
            ui.rrect(square, RADIUS, Color([r, green, b, 255]), None);
            let (size, label_line) = (9.0, ui.line_height(9.0));
            let width = ui.measure(label, size, true);
            ui.text(square.x + (square.w - width) / 2.0, square.y + (square.h - label_line) / 2.0, label, size, Color([255, 255, 255, 255]), true);
        }
        Mark::Glyph(ch) => {
            let (size, glyph_line) = (14.0, ui.line_height(14.0));
            let width = ui.measure_icon(ch, size);
            // The glyphs of the icon font are drawn above the middle of their line: 1.5 down puts the glyph in the middle.
            ui.icon(square.x + (square.w - width) / 2.0, square.y + (square.h - glyph_line) / 2.0 + 1.5, ch, size, glyph_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use nekominal_model::themes;
    use nekominal_render::offscreen::{AdapterKind, OffscreenTarget, create_gpu};

    use super::*;

    #[test]
    fn a_slider_takes_the_number_under_the_pointer_on_the_grid_of_its_steps() {
        let range = Range { min: 20.0, max: 100.0, step: 1.0, large: 5.0 };
        let track = Rect::new(100.0, 0.0, 200.0, 4.0);
        assert_eq!(slider_value(range, track, 100.0), 20.0);
        assert_eq!(slider_value(range, track, 300.0), 100.0);
        assert_eq!(slider_value(range, track, 200.0), 60.0);
        assert_eq!(slider_value(range, track, 50.0), 20.0, "left of the track is the minimum");
        assert_eq!(slider_value(range, track, 900.0), 100.0);
    }

    #[test]
    fn the_track_of_a_slider_leaves_half_a_thumb_of_the_area_at_each_end() {
        let area = Rect::new(10.0, 20.0, 216.0, 28.0);
        let track = slider_track(area);
        assert_eq!((track.x - area.x, area.x + area.w - (track.x + track.w)), (THUMB / 2.0, THUMB / 2.0));
        assert_eq!(track.y + track.h / 2.0, area.y + area.h / 2.0, "the track is in the middle of the height");
    }

    #[test]
    fn the_knob_of_a_switch_is_inside_it_at_the_end_it_is_on_or_off() {
        let rect = Rect::new(100.0, 10.0, 44.0, 24.0);
        let (off, on) = (switch_knob(rect, false), switch_knob(rect, true));
        assert_eq!((off.x - rect.x, rect.x + rect.w - (on.x + on.w)), (3.0, 3.0));
        assert_eq!((off.w, off.h), (18.0, 18.0));
        assert!(off.y >= rect.y && off.y + off.h <= rect.y + rect.h);
    }

    #[test]
    fn a_line_scrolls_only_once_the_caret_would_leave_the_room() {
        assert_eq!(edit_scroll(50.0, 200.0), 0.0);
        assert_eq!(edit_scroll(196.0, 200.0), 0.0, "four to spare is kept");
        assert_eq!(edit_scroll(250.0, 200.0), 54.0);
    }

    /// The mean of the three channels over a rectangle of a frame read back, in pixels.
    fn mean(rgba: &[u8], width: u32, rect: (u32, u32, u32, u32)) -> [f32; 3] {
        let mut sum = [0u64; 3];
        for y in rect.1..rect.1 + rect.3 {
            for x in rect.0..rect.0 + rect.2 {
                let at = ((y * width + x) * 4) as usize;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += u64::from(rgba[at + c]);
                }
            }
        }
        let n = (rect.2 * rect.3) as f32;
        sum.map(|s| s as f32 / n)
    }

    fn differ(a: [f32; 3], b: [f32; 3]) -> f32 {
        (0..3).map(|c| (a[c] - b[c]).abs()).sum()
    }

    #[test]
    fn every_state_of_a_control_looks_unlike_the_others_and_the_line_reports_its_caret() {
        let gpu = create_gpu(AdapterKind::Warp).expect("D3D11 WARP");
        let g = Glass::from_scheme(themes::scheme("Tokyo Night"));
        let size = (400u32, 300u32);
        let target = OffscreenTarget::new(&gpu, size.0, size.1);
        gpu.clear(&target.view, [0.1, 0.1, 0.13, 1.0]);
        let mut ui = Ui::new(&gpu).expect("UI");
        ui.begin_frame(size, 1.0);
        // One row of each control, in the states it has: the same size, one under the other.
        let cell = |i: f32, j: f32| Rect::new(10.0 + j * 100.0, 10.0 + i * 40.0, 80.0, 28.0);
        button(&mut ui, &g, cell(0.0, 0.0), State::default());
        button(&mut ui, &g, cell(0.0, 1.0), State::hover(true));
        button(&mut ui, &g, cell(0.0, 2.0), State { pressed: true, ..State::default() });
        button(&mut ui, &g, cell(0.0, 3.0), State::chosen(true, false));
        switch(&mut ui, &g, Rect::new(10.0, 50.0, 44.0, 24.0), false, false);
        switch(&mut ui, &g, Rect::new(110.0, 50.0, 44.0, 24.0), true, false);
        radio(&mut ui, &g, Rect::new(10.0, 90.0, 16.0, 16.0), false, false);
        radio(&mut ui, &g, Rect::new(110.0, 90.0, 16.0, 16.0), true, false);
        field(&mut ui, &g, cell(3.0, 0.0), 0.0);
        field(&mut ui, &g, cell(3.0, 1.0), 1.0);
        slider(&mut ui, &g, Rect::new(10.0, 170.0, 120.0, 28.0), 0.0);
        slider(&mut ui, &g, Rect::new(210.0, 170.0, 120.0, 28.0), 1.0);
        let edit_area = Rect::new(10.0, 210.0, 150.0, 28.0);
        let short = line_edit(&mut ui, &g, edit_area, 13.0, &LineEdit { text: "ab", cursor: Some(Cursor::at_end("ab")), preedit: "", placeholder: "" });
        let long = line_edit(&mut ui, &g, Rect::new(210.0, 210.0, 150.0, 28.0), 13.0, &LineEdit { text: "abcdefgh", cursor: Some(Cursor::at_end("abcdefgh")), preedit: "", placeholder: "" });
        line_edit(&mut ui, &g, Rect::new(10.0, 250.0, 150.0, 28.0), 13.0, &LineEdit { text: "", cursor: None, preedit: "", placeholder: "Search" });
        ui.end_frame(&target.view);
        let rgba = target.read_rgba(&gpu).expect("readback");
        let at = |r: Rect| (r.x as u32, r.y as u32, r.w as u32, r.h as u32);
        let m = |r: Rect| mean(&rgba, size.0, at(r));
        // The four states of a button are four colours.
        let states: Vec<[f32; 3]> = (0..4).map(|j| m(Rect::new(10.0 + j as f32 * 100.0 + 4.0, 14.0, 72.0, 20.0))).collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert!(differ(states[i], states[j]) > 1.0, "button states {i} and {j} look the same: {:?} {:?}", states[i], states[j]);
            }
        }
        // A switch that is on is the accent and one that is off is not; its knob is at the other end.
        let (off, on) = (m(Rect::new(10.0, 50.0, 44.0, 24.0)), m(Rect::new(110.0, 50.0, 44.0, 24.0)));
        assert!(differ(off, on) > 20.0, "a switch on and a switch off look the same");
        let (left, right) = (m(Rect::new(13.0, 53.0, 18.0, 18.0)), m(Rect::new(33.0, 53.0, 18.0, 18.0)));
        assert!(differ(left, right) > 20.0, "the knob of a switch off is at the left end");
        // The dot makes a radio that is on unlike one that is not.
        assert!(differ(m(Rect::new(12.0, 92.0, 12.0, 12.0)), m(Rect::new(112.0, 92.0, 12.0, 12.0))) > 20.0, "a radio on looks like a radio off");
        // The outline of a field being edited is the accent: its top edge differs from the quiet one.
        assert!(differ(m(Rect::new(110.0, 130.0, 80.0, 1.0)), m(Rect::new(10.0, 130.0, 80.0, 1.0))) > 5.0, "the outline of a field being edited is not told apart");
        // The thumb of a slider is a bright disc at the end the share says, and nothing but the thin track elsewhere.
        let (start_thumb, start_track) = (m(Rect::new(10.0, 176.0, 16.0, 16.0)), m(Rect::new(70.0, 176.0, 16.0, 16.0)));
        let (end_thumb, end_track) = (m(Rect::new(314.0, 176.0, 16.0, 16.0)), m(Rect::new(270.0, 176.0, 16.0, 16.0)));
        assert!(differ(start_thumb, start_track) > 20.0, "a slider at its start has no thumb there");
        assert!(differ(end_thumb, end_track) > 20.0, "a slider at its end has no thumb there");
        // The caret comes after the text; a line too long for its room scrolls and keeps the caret inside it.
        assert!(short.0 > edit_area.x && short.0 < edit_area.x + 40.0, "the caret of a short line is just after its text: {}", short.0);
        assert!(long.0 <= 210.0 + 150.0, "the caret of a long line stays inside its room: {}", long.0);
        assert!(long.2 > 8.0 && long.1 >= 210.0, "the caret has the height of the line");
        // The placeholder of an empty line shows.
        assert!(differ(m(Rect::new(10.0, 250.0, 150.0, 28.0)), [0.1 * 255.0, 0.1 * 255.0, 0.13 * 255.0]) > 1.0, "the placeholder is not drawn");
    }
}
