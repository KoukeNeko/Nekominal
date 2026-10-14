//! Mouse reports, alternate scroll and the wheel accumulator.

mod common;
use common::*;
use nekominal_input::*;

fn modes(mode: MouseMode, encoding: MouseEncoding) -> InputModes {
    InputModes { mouse_mode: mode, mouse_encoding: encoding, ..InputModes::default() }
}

fn mouse(input: MouseInput, modes: &InputModes) -> String {
    let mut state = MouseState::default();
    match encode_mouse(&input, modes, &mut state) {
        Some(bytes) => show(&bytes),
        None => NOTHING.to_string(),
    }
}

fn at(kind: MouseKind, col: u16, row: u16) -> MouseInput {
    MouseInput::new(kind, col, row)
}

const LEFT: MouseButton = MouseButton::Left;

fn press(button: MouseButton) -> MouseKind {
    MouseKind::Press(button)
}

fn release(button: MouseButton) -> MouseKind {
    MouseKind::Release(button)
}

fn motion(held: Option<MouseButton>) -> MouseKind {
    MouseKind::Move { held }
}

// ---- default encoding -----------------------------------------------------------------------

#[test]
fn default_encoding_press_and_release() {
    let m = modes(MouseMode::Normal, MouseEncoding::Default);
    // Positions are 1-based and offset by 32: (0,0) is 33,33 = "!!".
    assert_eq!(mouse(at(press(LEFT), 0, 0), &m), r"\e[M !!");
    assert_eq!(mouse(at(press(MouseButton::Middle), 0, 0), &m), r"\e[M!!!");
    assert_eq!(mouse(at(press(MouseButton::Right), 0, 0), &m), "\\e[M\"!!");
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), r"\e[M *%");
    // Release cannot name the button: code 3 (+32 = '#') for all of them.
    assert_eq!(mouse(at(release(LEFT), 9, 4), &m), r"\e[M#*%");
    assert_eq!(mouse(at(release(MouseButton::Right), 9, 4), &m), r"\e[M#*%");
}

#[test]
fn default_encoding_modifiers() {
    let m = modes(MouseMode::Normal, MouseEncoding::Default);
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(SHIFT), &m), r"\e[M$!!");
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(ALT), &m), r"\e[M(!!");
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(CTRL), &m), r"\e[M0!!");
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(SHIFT | ALT | CTRL), &m), r"\e[M<!!");
    // The Windows key is not part of the protocol.
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(SUPER), &m), r"\e[M !!");
}

#[test]
fn default_encoding_wheel() {
    let m = modes(MouseMode::Normal, MouseEncoding::Default);
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0), &m), r"\e[M`!!");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Down), 0, 0), &m), r"\e[Ma!!");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Left), 0, 0), &m), r"\e[Mb!!");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Right), 0, 0), &m), r"\e[Mc!!");
}

#[test]
fn default_encoding_stays_ascii_under_conpty() {
    // ConPTY decodes its input as UTF-8: a lone byte above 0x7f would arrive as U+FFFD.
    let m = modes(MouseMode::Normal, MouseEncoding::Default);
    assert!(m.pty_decodes_utf8);
    // Position 95 is the last one whose byte (95 + 32 = 127) is still ASCII.
    assert_eq!(
        encode_mouse(&at(press(LEFT), 94, 94), &m, &mut MouseState::default()).unwrap(),
        [0x1b, b'[', b'M', 32, 127, 127]
    );
    assert_eq!(mouse(at(press(LEFT), 95, 0), &m), NOTHING);
    assert_eq!(mouse(at(press(LEFT), 0, 95), &m), NOTHING);
    // The extra buttons would need a button byte of 160.
    assert_eq!(mouse(at(press(MouseButton::Back), 0, 0), &m), NOTHING);
    // SGR and UTF-8 reports have no such limit.
    let sgr = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_ne!(mouse(at(press(LEFT), 200, 200), &sgr), NOTHING);
}

#[test]
fn default_encoding_reaches_223_when_the_pty_is_8_bit_clean() {
    let mut m = modes(MouseMode::Normal, MouseEncoding::Default);
    m.pty_decodes_utf8 = false;
    assert_eq!(
        encode_mouse(&at(press(LEFT), 222, 222), &m, &mut MouseState::default()).unwrap()[4..],
        [255, 255]
    );
    assert_eq!(mouse(at(press(LEFT), 223, 0), &m), NOTHING);
    assert_eq!(mouse(at(press(LEFT), 0, 223), &m), NOTHING);
    assert_eq!(
        encode_mouse(&at(press(MouseButton::Back), 0, 0), &m, &mut MouseState::default()).unwrap()[3],
        160
    );
}
#[test]
fn extra_buttons_use_the_128_range() {
    let sgr = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(mouse(at(press(MouseButton::Back), 0, 0), &sgr), r"\e[<128;1;1M");
    assert_eq!(mouse(at(press(MouseButton::Forward), 0, 0), &sgr), r"\e[<129;1;1M");
    assert_eq!(mouse(at(release(MouseButton::Back), 0, 0), &sgr), r"\e[<128;1;1m");
}

// ---- SGR (1006) -----------------------------------------------------------------------------

#[test]
fn sgr_press_release() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), r"\e[<0;10;5M");
    assert_eq!(mouse(at(release(LEFT), 9, 4), &m), r"\e[<0;10;5m");
    assert_eq!(mouse(at(press(MouseButton::Middle), 9, 4), &m), r"\e[<1;10;5M");
    // SGR names the released button.
    assert_eq!(mouse(at(release(MouseButton::Right), 9, 4), &m), r"\e[<2;10;5m");
}

#[test]
fn sgr_modifiers_and_wheel() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(CTRL), &m), r"\e[<16;1;1M");
    assert_eq!(mouse(at(press(LEFT), 0, 0).with_mods(SHIFT | ALT | CTRL), &m), r"\e[<28;1;1M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 9, 4), &m), r"\e[<64;10;5M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Down), 9, 4), &m), r"\e[<65;10;5M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Left), 9, 4), &m), r"\e[<66;10;5M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Right), 9, 4), &m), r"\e[<67;10;5M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 9, 4).with_mods(CTRL), &m), r"\e[<80;10;5M");
}

#[test]
fn sgr_has_no_position_limit() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(mouse(at(press(LEFT), 3000, 1999), &m), r"\e[<0;3001;2000M");
    assert_eq!(mouse(at(press(LEFT), u16::MAX, u16::MAX), &m), r"\e[<0;65536;65536M");
}

#[test]
fn wheel_notches_repeat_the_report() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(
        mouse(at(MouseKind::Wheel(WheelDirection::Down), 0, 0).with_notches(3), &m),
        r"\e[<65;1;1M\e[<65;1;1M\e[<65;1;1M"
    );
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Down), 0, 0).with_notches(0), &m), r"\e[<65;1;1M");
}

#[test]
fn a_runaway_wheel_delta_is_capped() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let report = r"\e[<65;1;1M";
    let wheel = at(MouseKind::Wheel(WheelDirection::Down), 0, 0).with_notches(u16::MAX);
    assert_eq!(mouse(wheel, &m), report.repeat(nekominal_input::mouse::MAX_NOTCHES));
    assert_eq!(mouse(wheel, &alt_scroll()), r"\e[B".repeat(nekominal_input::mouse::MAX_NOTCHES));
}

// ---- UTF-8 (1005), URXVT (1015), SGR-pixels (1016) ------------------------------------------

#[test]
fn utf8_encoding_carries_large_positions() {
    let m = modes(MouseMode::Normal, MouseEncoding::Utf8);
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), r"\e[M *%");
    // x = 201 -> 233 = U+00E9, two bytes.
    assert_eq!(mouse(at(press(LEFT), 200, 4), &m), "\\e[M é%");
    // Largest representable position: 2015.
    assert_eq!(
        encode_mouse(&at(press(LEFT), 2014, 0), &m, &mut MouseState::default()).unwrap(),
        [0x1b, b'[', b'M', b' ', 0xdf, 0xbf, b'!']
    );
    assert_eq!(mouse(at(press(LEFT), 2015, 0), &m), NOTHING);
}

#[test]
fn urxvt_encoding_is_decimal_with_the_x10_button_offset() {
    let m = modes(MouseMode::Normal, MouseEncoding::Urxvt);
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), r"\e[32;10;5M");
    assert_eq!(mouse(at(release(LEFT), 9, 4), &m), r"\e[35;10;5M");
    assert_eq!(mouse(at(press(LEFT), 300, 100), &m), r"\e[32;301;101M");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0), &m), r"\e[96;1;1M");
}

#[test]
fn sgr_pixels_reports_pixel_positions() {
    let m = modes(MouseMode::Normal, MouseEncoding::SgrPixels);
    assert_eq!(mouse(at(press(LEFT), 9, 4).with_pixel(123, 45), &m), r"\e[<0;123;45M");
    assert_eq!(mouse(at(release(LEFT), 9, 4).with_pixel(123, 45), &m), r"\e[<0;123;45m");
    // Without a pixel position the report cannot be made.
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), NOTHING);
}

// ---- which events each mode reports ---------------------------------------------------------

#[test]
fn no_mode_reports_nothing() {
    let m = modes(MouseMode::None, MouseEncoding::Sgr);
    assert_eq!(mouse(at(press(LEFT), 0, 0), &m), NOTHING);
    assert_eq!(mouse(at(release(LEFT), 0, 0), &m), NOTHING);
    assert_eq!(mouse(at(motion(None), 0, 0), &m), NOTHING);
    assert!(!m.mouse_tracking());
}

#[test]
fn x10_reports_presses_of_the_three_buttons_without_modifiers() {
    let m = modes(MouseMode::X10, MouseEncoding::Default);
    assert!(m.mouse_tracking());
    assert_eq!(mouse(at(press(LEFT), 9, 4), &m), r"\e[M *%");
    assert_eq!(mouse(at(press(LEFT), 9, 4).with_mods(CTRL | SHIFT), &m), r"\e[M *%");
    assert_eq!(mouse(at(release(LEFT), 9, 4), &m), NOTHING);
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 9, 4), &m), NOTHING);
    assert_eq!(mouse(at(motion(Some(LEFT)), 9, 4), &m), NOTHING);
    assert_eq!(mouse(at(press(MouseButton::Back), 9, 4), &m), NOTHING);
}

#[test]
fn normal_mode_reports_clicks_and_wheel_but_not_motion() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert_eq!(mouse(at(motion(Some(LEFT)), 1, 1), &m), NOTHING);
    assert_eq!(mouse(at(motion(None), 1, 1), &m), NOTHING);
    assert_ne!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 1, 1), &m), NOTHING);
}

#[test]
fn button_mode_reports_motion_only_while_a_button_is_held() {
    let m = modes(MouseMode::Button, MouseEncoding::Sgr);
    assert_eq!(mouse(at(motion(Some(LEFT)), 10, 4), &m), r"\e[<32;11;5M");
    assert_eq!(mouse(at(motion(Some(MouseButton::Right)), 10, 4), &m), r"\e[<34;11;5M");
    assert_eq!(mouse(at(motion(None), 10, 4), &m), NOTHING);
    // Default encoding: motion with the left button is '@' = 32 + 0 + 32.
    let d = modes(MouseMode::Button, MouseEncoding::Default);
    assert_eq!(mouse(at(motion(Some(LEFT)), 0, 0), &d), r"\e[M@!!");
    assert_eq!(mouse(at(motion(Some(MouseButton::Right)), 0, 0), &d), "\\e[MB!!");
}

#[test]
fn any_mode_reports_hover_as_button_3() {
    let m = modes(MouseMode::Any, MouseEncoding::Sgr);
    assert_eq!(mouse(at(motion(None), 10, 4), &m), r"\e[<35;11;5M");
    assert_eq!(mouse(at(motion(None), 10, 4).with_mods(CTRL), &m), r"\e[<51;11;5M");
    let d = modes(MouseMode::Any, MouseEncoding::Default);
    assert_eq!(mouse(at(motion(None), 0, 0), &d), r"\e[MC!!");
}

#[test]
fn motion_is_reported_once_per_cell() {
    let m = modes(MouseMode::Any, MouseEncoding::Sgr);
    let mut state = MouseState::default();
    let move_to = |col, row| MouseInput::new(MouseKind::Move { held: None }, col, row);
    assert!(encode_mouse(&move_to(3, 3), &m, &mut state).is_some());
    assert!(encode_mouse(&move_to(3, 3), &m, &mut state).is_none());
    assert!(encode_mouse(&move_to(4, 3), &m, &mut state).is_some());
    assert!(encode_mouse(&move_to(4, 3), &m, &mut state).is_none());
    // A click in between does not stop the next motion in the same cell being reported again
    // once the click moved the "last position" elsewhere.
    assert!(encode_mouse(&at(press(LEFT), 9, 9), &m, &mut state).is_some());
    assert!(encode_mouse(&move_to(4, 3), &m, &mut state).is_some());
    state.reset();
    assert!(encode_mouse(&move_to(4, 3), &m, &mut state).is_some());
}

#[test]
fn pixel_mode_reports_every_pixel_step() {
    let m = modes(MouseMode::Any, MouseEncoding::SgrPixels);
    let mut state = MouseState::default();
    let move_to = |x, y| MouseInput::new(MouseKind::Move { held: None }, 0, 0).with_pixel(x, y);
    assert!(encode_mouse(&move_to(10, 10), &m, &mut state).is_some());
    assert!(encode_mouse(&move_to(10, 10), &m, &mut state).is_none());
    assert!(encode_mouse(&move_to(11, 10), &m, &mut state).is_some());
}

#[test]
fn wheel_does_not_disturb_the_motion_filter() {
    let m = modes(MouseMode::Any, MouseEncoding::Sgr);
    let mut state = MouseState::default();
    let move_to = |col| MouseInput::new(MouseKind::Move { held: None }, col, 0);
    assert!(encode_mouse(&move_to(2), &m, &mut state).is_some());
    assert!(encode_mouse(&at(MouseKind::Wheel(WheelDirection::Up), 7, 0), &m, &mut state).is_some());
    assert!(encode_mouse(&move_to(2), &m, &mut state).is_none());
}

// ---- alternate scroll (1007) ----------------------------------------------------------------

fn alt_scroll() -> InputModes {
    InputModes { alt_screen: true, alternate_scroll: true, ..InputModes::default() }
}

#[test]
fn alternate_scroll_turns_the_wheel_into_cursor_keys_on_the_alternate_screen() {
    let m = alt_scroll();
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0), &m), r"\e[A");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Down), 0, 0), &m), r"\e[B");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Right), 0, 0), &m), r"\e[C");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Left), 0, 0), &m), r"\e[D");
    assert_eq!(
        mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0).with_notches(3), &m),
        r"\e[A\e[A\e[A"
    );
}

#[test]
fn alternate_scroll_honours_application_cursor_keys() {
    let m = InputModes { app_cursor: true, ..alt_scroll() };
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0), &m), r"\eOA");
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Down), 0, 0).with_notches(2), &m), r"\eOB\eOB");
}

#[test]
fn alternate_scroll_needs_the_alternate_screen_and_the_mode() {
    let wheel = at(MouseKind::Wheel(WheelDirection::Up), 0, 0);
    let main_screen = InputModes { alt_screen: false, ..alt_scroll() };
    assert_eq!(mouse(wheel, &main_screen), NOTHING);
    let mode_off = InputModes { alternate_scroll: false, ..alt_scroll() };
    assert_eq!(mouse(wheel, &mode_off), NOTHING);
}

#[test]
fn mouse_tracking_wins_over_alternate_scroll() {
    let m = InputModes { mouse_mode: MouseMode::Normal, mouse_encoding: MouseEncoding::Sgr, ..alt_scroll() };
    assert_eq!(mouse(at(MouseKind::Wheel(WheelDirection::Up), 0, 0), &m), r"\e[<64;1;1M");
}

#[test]
fn clicks_are_not_alternate_scroll() {
    assert_eq!(mouse(at(press(LEFT), 0, 0), &alt_scroll()), NOTHING);
}

// ---- wheel accumulator ----------------------------------------------------------------------

#[test]
fn wheel_accumulator_emits_whole_notches() {
    let mut acc = WheelAccumulator::default();
    assert_eq!(acc.push(120), 1);
    assert_eq!(acc.push(-120), -1);
    assert_eq!(acc.push(240), 2);
    assert_eq!(acc.push(-360), -3);
}

#[test]
fn wheel_accumulator_collects_precision_touchpad_deltas() {
    let mut acc = WheelAccumulator::default();
    let notches: i32 = (0..8).map(|_| acc.push(30)).sum();
    assert_eq!(notches, 2);
    assert_eq!(acc.push(60), 0);
    assert_eq!(acc.push(60), 1);
}

#[test]
fn wheel_accumulator_drops_the_remainder_on_direction_change() {
    let mut acc = WheelAccumulator::default();
    assert_eq!(acc.push(100), 0);
    assert_eq!(acc.push(-100), 0);
    // The +100 was discarded: -100 alone is not a notch, and neither is -100 + 100 (reset again).
    assert_eq!(acc.push(-20), -1);
}

#[test]
fn wheel_accumulator_ignores_zero_deltas() {
    let mut acc = WheelAccumulator::default();
    assert_eq!(acc.push(100), 0);
    assert_eq!(acc.push(0), 0);
    assert_eq!(acc.push(20), 1);
}
