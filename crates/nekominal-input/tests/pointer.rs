//! MouseAssembler: pixels to cells, who owns the pointer, selection gestures, the wheel, and every
//! mouse mode and encoding through the assembler.

mod common;
use common::*;
use nekominal_input::*;

const LEFT: MouseButton = MouseButton::Left;

/// 10 x 20 pixel cells, 80 x 24.
fn grid() -> PointerGrid {
    PointerGrid { cell_width: 10.0, cell_height: 20.0, columns: 80, rows: 24 }
}

fn modes(mode: MouseMode, encoding: MouseEncoding) -> InputModes {
    InputModes { mouse_mode: mode, mouse_encoding: encoding, ..InputModes::default() }
}

fn press(button: MouseButton, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(PointerKind::Press(button), x, y)
}

fn release(button: MouseButton, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(PointerKind::Release(button), x, y)
}

fn moved(x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(PointerKind::Move, x, y)
}

fn wheel(delta: i32, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(PointerKind::Wheel { delta, horizontal: false }, x, y)
}

fn hwheel(delta: i32, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(PointerKind::Wheel { delta, horizontal: true }, x, y)
}

fn run(asm: &mut MouseAssembler, event: PointerEvent, modes: &InputModes) -> Vec<PointerOutcome> {
    asm.handle(&event, &grid(), modes)
}

/// The reports among the outcomes, printable.
fn reports(outcomes: &[PointerOutcome]) -> Vec<String> {
    outcomes
        .iter()
        .filter_map(|o| match o {
            PointerOutcome::Report(bytes) => Some(show(bytes)),
            _ => None,
        })
        .collect()
}

fn point(col: u16, row: u16, side: Side) -> CellPoint {
    CellPoint { col, row, side }
}

// ---- cells ------------------------------------------------------------------------------------------

#[test]
fn pixels_become_zero_based_cells() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    // (95, 50): column 9, row 2: 1-based 10;3 on the wire.
    assert_eq!(reports(&run(&mut asm, press(LEFT, 95.0, 50.0), &m)), [r"\e[<0;10;3M"]);
    assert_eq!(reports(&run(&mut asm, release(LEFT, 0.0, 0.0), &m)), [r"\e[<0;1;1m"]);
}

#[test]
fn the_program_gets_the_pointer_only_while_it_tracks_the_mouse_and_shift_is_up() {
    let tracking = modes(MouseMode::Normal, MouseEncoding::Sgr);
    assert!(tracking.program_gets_pointer(Mods::NONE));
    assert!(tracking.program_gets_pointer(Mods::CTRL), "only Shift is the bypass");
    assert!(!tracking.program_gets_pointer(Mods::SHIFT));
    assert!(!InputModes::default().program_gets_pointer(Mods::NONE));
}

#[test]
fn cell_at_names_the_cell_under_a_position_and_clamps_to_the_grid() {
    assert_eq!(cell_at(95.0, 50.0, &grid()), (9, 2));
    assert_eq!(cell_at(10.0, 20.0, &grid()), (1, 1), "a boundary belongs to the next cell");
    assert_eq!(cell_at(-30.0, -1.0, &grid()), (0, 0));
    assert_eq!(cell_at(5000.0, 5000.0, &grid()), (79, 23));
}

#[test]
fn a_cell_boundary_belongs_to_the_next_cell() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, press(LEFT, 9.99, 19.99), &m)), [r"\e[<0;1;1M"]);
    run(&mut asm, release(LEFT, 9.99, 19.99), &m);
    assert_eq!(reports(&run(&mut asm, press(LEFT, 10.0, 20.0), &m)), [r"\e[<0;2;2M"]);
}

#[test]
fn positions_outside_the_grid_are_clamped_to_its_edge() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, press(LEFT, -50.0, -5.0), &m)), [r"\e[<0;1;1M"]);
    run(&mut asm, release(LEFT, 0.0, 0.0), &m);
    assert_eq!(reports(&run(&mut asm, press(LEFT, 5000.0, 5000.0), &m)), [r"\e[<0;80;24M"]);
}

#[test]
fn hostile_coordinates_and_grids_do_not_panic() {
    let m = modes(MouseMode::Any, MouseEncoding::SgrPixels);
    let mut asm = MouseAssembler::default();
    for (x, y) in [(f32::NAN, 1.0), (f32::INFINITY, f32::NEG_INFINITY), (-1.0e30, 1.0e30)] {
        asm.handle(&moved(x, y), &grid(), &m);
        asm.handle(&press(LEFT, x, y), &grid(), &m);
        asm.handle(&release(LEFT, x, y), &grid(), &m);
    }
    let empty = PointerGrid { cell_width: 0.0, cell_height: -3.0, columns: 0, rows: 0 };
    asm.handle(&press(LEFT, 5.0, 5.0), &empty, &m);
    asm.handle(&wheel(120, 5.0, 5.0), &empty, &m);
}

#[test]
fn sgr_pixels_report_pixel_positions_clamped_to_the_grid() {
    let m = modes(MouseMode::Normal, MouseEncoding::SgrPixels);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, press(LEFT, 95.0, 50.0), &m)), [r"\e[<0;95;50M"]);
    run(&mut asm, release(LEFT, 0.0, 0.0), &m);
    assert_eq!(reports(&run(&mut asm, press(LEFT, -4.0, 9999.0), &m)), [r"\e[<0;0;479M"]);
}

// ---- every mode and encoding -----------------------------------------------------------------------------

#[test]
fn one_press_in_every_encoding() {
    let expected = [
        (MouseEncoding::Default, r"\e[M *#"),
        (MouseEncoding::Utf8, r"\e[M *#"),
        (MouseEncoding::Sgr, r"\e[<0;10;3M"),
        (MouseEncoding::Urxvt, r"\e[32;10;3M"),
        (MouseEncoding::SgrPixels, r"\e[<0;95;50M"),
    ];
    for (encoding, bytes) in expected {
        let mut asm = MouseAssembler::default();
        let m = modes(MouseMode::Normal, encoding);
        assert_eq!(reports(&run(&mut asm, press(LEFT, 95.0, 50.0), &m)), [bytes], "{encoding:?}");
    }
}

#[test]
fn one_release_in_every_encoding() {
    // Only SGR names the button that went up.
    let expected = [
        (MouseEncoding::Default, r"\e[M#*#"),
        (MouseEncoding::Utf8, r"\e[M#*#"),
        (MouseEncoding::Sgr, r"\e[<0;10;3m"),
        (MouseEncoding::Urxvt, r"\e[35;10;3M"),
        (MouseEncoding::SgrPixels, r"\e[<0;95;50m"),
    ];
    for (encoding, bytes) in expected {
        let mut asm = MouseAssembler::default();
        let m = modes(MouseMode::Normal, encoding);
        run(&mut asm, press(LEFT, 95.0, 50.0), &m);
        assert_eq!(reports(&run(&mut asm, release(LEFT, 95.0, 50.0), &m)), [bytes], "{encoding:?}");
    }
}

#[test]
fn each_mode_reports_its_events() {
    // press, drag to the next cell, release, motion without a button, wheel up.
    let sequence = |mode: MouseMode| -> Vec<String> {
        let m = modes(mode, MouseEncoding::Sgr);
        let mut asm = MouseAssembler::default();
        let mut sent = Vec::new();
        sent.extend(reports(&run(&mut asm, press(LEFT, 5.0, 5.0), &m)));
        sent.extend(reports(&run(&mut asm, moved(15.0, 5.0), &m)));
        sent.extend(reports(&run(&mut asm, release(LEFT, 15.0, 5.0), &m)));
        sent.extend(reports(&run(&mut asm, moved(25.0, 5.0), &m)));
        sent.extend(reports(&run(&mut asm, wheel(120, 25.0, 5.0), &m)));
        sent
    };
    assert_eq!(sequence(MouseMode::X10), [r"\e[<0;1;1M"]);
    assert_eq!(sequence(MouseMode::Normal), [r"\e[<0;1;1M", r"\e[<0;2;1m", r"\e[<64;3;1M"]);
    assert_eq!(sequence(MouseMode::Button), [r"\e[<0;1;1M", r"\e[<32;2;1M", r"\e[<0;2;1m", r"\e[<64;3;1M"]);
    assert_eq!(
        sequence(MouseMode::Any),
        [r"\e[<0;1;1M", r"\e[<32;2;1M", r"\e[<0;2;1m", r"\e[<35;3;1M", r"\e[<64;3;1M"]
    );
    assert!(sequence(MouseMode::None).is_empty());
}

#[test]
fn modifiers_ride_on_the_report() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 5.0, 5.0).with_mods(Mods::CTRL), &m);
    assert_eq!(reports(&out), [r"\e[<16;1;1M"]);
    run(&mut asm, release(LEFT, 5.0, 5.0).with_mods(Mods::CTRL), &m);
    let out = run(&mut asm, press(MouseButton::Right, 5.0, 5.0).with_mods(Mods::ALT), &m);
    assert_eq!(reports(&out), [r"\e[<10;1;1M"]);
}

#[test]
fn motion_within_a_cell_is_reported_once() {
    let m = modes(MouseMode::Any, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, moved(1.0, 1.0), &m)).len(), 1);
    assert!(run(&mut asm, moved(2.0, 3.0), &m).is_empty());
    assert!(run(&mut asm, moved(9.0, 19.0), &m).is_empty());
    assert_eq!(reports(&run(&mut asm, moved(10.0, 19.0), &m)).len(), 1);
}

#[test]
fn a_new_mode_starts_a_new_report_history() {
    let mut asm = MouseAssembler::default();
    let sgr = modes(MouseMode::Any, MouseEncoding::Sgr);
    assert_eq!(reports(&run(&mut asm, moved(1.0, 1.0), &sgr)).len(), 1);
    assert!(run(&mut asm, moved(2.0, 2.0), &sgr).is_empty());
    // The same cell under another encoding is a new report: the last position was written another way.
    let urxvt = modes(MouseMode::Any, MouseEncoding::Urxvt);
    assert_eq!(reports(&run(&mut asm, moved(2.0, 2.0), &urxvt)).len(), 1);
}

#[test]
fn back_and_forward_buttons_are_reported() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, press(MouseButton::Back, 5.0, 5.0), &m)), [r"\e[<128;1;1M"]);
    assert_eq!(reports(&run(&mut asm, release(MouseButton::Back, 5.0, 5.0), &m)), [r"\e[<128;1;1m"]);
}

#[test]
fn a_drag_with_the_middle_button_names_it() {
    let m = modes(MouseMode::Button, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    run(&mut asm, press(MouseButton::Middle, 5.0, 5.0), &m);
    assert_eq!(reports(&run(&mut asm, moved(15.0, 5.0), &m)), [r"\e[<33;2;1M"]);
}

#[test]
fn a_release_of_a_button_that_was_never_pressed_is_ignored() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert!(run(&mut asm, release(LEFT, 5.0, 5.0), &m).is_empty());
}

// ---- who owns the pointer ----------------------------------------------------------------------------------------

#[test]
fn shift_hands_the_pointer_to_the_terminal_while_the_program_tracks_it() {
    let m = modes(MouseMode::Any, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 22.0, 45.0).with_mods(Mods::SHIFT), &m);
    assert!(reports(&out).is_empty());
    assert_eq!(
        out,
        [PointerOutcome::SelectionStart { point: point(2, 2, Side::Left), kind: SelectionKind::Simple, extend: true }]
    );
}

#[test]
fn the_owner_is_kept_until_every_button_is_up() {
    let m = modes(MouseMode::Button, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    // The program owns the gesture: Shift pressed during it changes nothing.
    assert_eq!(reports(&run(&mut asm, press(LEFT, 5.0, 5.0), &m)).len(), 1);
    assert_eq!(reports(&run(&mut asm, moved(15.0, 5.0).with_mods(Mods::SHIFT), &m)).len(), 1);
    assert_eq!(reports(&run(&mut asm, release(LEFT, 15.0, 5.0).with_mods(Mods::SHIFT), &m)).len(), 1);
    // The terminal owns this one: letting go of Shift mid-drag does not turn it into reports.
    run(&mut asm, press(LEFT, 5.0, 5.0).with_mods(Mods::SHIFT), &m);
    let out = run(&mut asm, moved(55.0, 5.0), &m);
    assert!(reports(&out).is_empty());
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionUpdate { .. }]));
}

#[test]
fn a_second_button_follows_the_owner_of_the_first() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    run(&mut asm, press(LEFT, 5.0, 5.0), &m);
    assert_eq!(reports(&run(&mut asm, press(MouseButton::Right, 5.0, 5.0).with_mods(Mods::SHIFT), &m)).len(), 1);
}

#[test]
fn without_tracking_the_terminal_owns_the_pointer() {
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 22.0, 45.0), &InputModes::default());
    assert_eq!(
        out,
        [PointerOutcome::SelectionStart { point: point(2, 2, Side::Left), kind: SelectionKind::Simple, extend: false }]
    );
}

#[test]
fn other_buttons_do_nothing_for_the_terminal() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    assert!(run(&mut asm, press(MouseButton::Right, 5.0, 5.0), &m).is_empty());
    assert!(run(&mut asm, release(MouseButton::Right, 5.0, 5.0), &m).is_empty());
}

// ---- selection -----------------------------------------------------------------------------------------------------

#[test]
fn a_drag_selects_and_a_release_ends_it() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 22.0, 5.0), &m);
    let out = run(&mut asm, moved(61.0, 5.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionUpdate { point: point(6, 0, Side::Left), overshoot_rows: 0 }]);
    let out = run(&mut asm, release(LEFT, 68.0, 5.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionEnd { point: point(6, 0, Side::Right), dragged: true }]);
}

#[test]
fn the_side_of_a_cell_is_its_half() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    let sides = |asm: &mut MouseAssembler, x: f32| match run(asm, press(LEFT, x, 5.0), &m).as_slice() {
        [PointerOutcome::SelectionStart { point, .. }] => point.side,
        other => panic!("{other:?}"),
    };
    assert_eq!(sides(&mut asm, 31.0), Side::Left);
    run(&mut asm, release(LEFT, 31.0, 5.0), &m);
    assert_eq!(sides(&mut asm, 35.0), Side::Right);
}

#[test]
fn jitter_inside_the_threshold_is_a_click() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 20.0, 20.0), &m);
    assert!(run(&mut asm, moved(22.0, 21.0), &m).is_empty());
    assert!(run(&mut asm, moved(23.0, 22.0), &m).is_empty());
    let out = run(&mut asm, release(LEFT, 23.0, 22.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionEnd { point: point(2, 1, Side::Left), dragged: false }]);
}

#[test]
fn the_threshold_is_the_distance_from_the_press_not_per_move() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 20.0, 20.0), &m);
    // Each step is small; together they pass the threshold of 4 pixels.
    assert!(run(&mut asm, moved(21.0, 20.0), &m).is_empty());
    assert!(run(&mut asm, moved(22.0, 20.0), &m).is_empty());
    assert!(run(&mut asm, moved(23.0, 20.0), &m).is_empty());
    assert!(run(&mut asm, moved(24.0, 20.0), &m).is_empty(), "4 pixels over the same half cell: no change to report");
    let out = run(&mut asm, moved(26.0, 20.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionUpdate { point: point(2, 1, Side::Right), overshoot_rows: 0 }]);
}

#[test]
fn a_drag_threshold_from_the_system_applies() {
    let mut asm = MouseAssembler::new(PointerConfig { drag_threshold: 20.0, ..PointerConfig::default() });
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 20.0, 20.0), &m);
    assert!(run(&mut asm, moved(30.0, 20.0), &m).is_empty());
    assert!(!run(&mut asm, moved(40.0, 20.0), &m).is_empty());
}

#[test]
fn updates_are_only_reported_when_the_point_changes() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 20.0, 5.0), &m);
    assert_eq!(run(&mut asm, moved(80.0, 5.0), &m).len(), 1);
    assert!(run(&mut asm, moved(81.0, 6.0), &m).is_empty());
    assert_eq!(run(&mut asm, moved(86.0, 6.0), &m).len(), 1);
}

#[test]
fn a_drag_outside_the_grid_reports_the_overshoot() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 20.0, 40.0), &m);
    let out = run(&mut asm, moved(20.0, -45.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionUpdate { point: point(2, 0, Side::Left), overshoot_rows: -3 }]);
    let out = run(&mut asm, moved(20.0, 480.0 + 5.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionUpdate { point: point(2, 23, Side::Left), overshoot_rows: 1 }]);
    let out = run(&mut asm, moved(900.0, 480.0 + 5.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionUpdate { point: point(79, 23, Side::Right), overshoot_rows: 1 }]);
}

#[test]
fn double_and_triple_clicks_pick_words_and_lines() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    let click = |asm: &mut MouseAssembler, time: u32| {
        let out = run(asm, press(LEFT, 25.0, 5.0).with_time(time), &m);
        run(asm, release(LEFT, 25.0, 5.0).with_time(time + 30), &m);
        match out.as_slice() {
            [PointerOutcome::SelectionStart { kind, .. }] => *kind,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(click(&mut asm, 1_000), SelectionKind::Simple);
    assert_eq!(click(&mut asm, 1_200), SelectionKind::Word);
    assert_eq!(click(&mut asm, 1_400), SelectionKind::Line);
    // The fourth click starts over.
    assert_eq!(click(&mut asm, 1_600), SelectionKind::Simple);
}

#[test]
fn a_slow_second_click_or_one_in_another_cell_is_a_new_click() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    let kind = |asm: &mut MouseAssembler, x: f32, time: u32| {
        let out = run(asm, press(LEFT, x, 5.0).with_time(time), &m);
        run(asm, release(LEFT, x, 5.0).with_time(time + 10), &m);
        match out.as_slice() {
            [PointerOutcome::SelectionStart { kind, .. }] => *kind,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(kind(&mut asm, 25.0, 1_000), SelectionKind::Simple);
    assert_eq!(kind(&mut asm, 25.0, 1_501), SelectionKind::Simple, "after the double-click time");
    assert_eq!(kind(&mut asm, 25.0, 1_700), SelectionKind::Word);
    assert_eq!(kind(&mut asm, 55.0, 1_800), SelectionKind::Simple, "another cell");
}

#[test]
fn a_drag_does_not_count_towards_a_double_click() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 25.0, 5.0).with_time(1_000), &m);
    run(&mut asm, moved(75.0, 5.0), &m);
    run(&mut asm, release(LEFT, 75.0, 5.0).with_time(1_100), &m);
    // A press in the cell the drag started in, soon after: still a single click.
    let out = run(&mut asm, press(LEFT, 25.0, 5.0).with_time(1_200), &m);
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Simple, .. }]));
}

#[test]
fn the_double_click_time_from_the_system_applies() {
    let mut asm = MouseAssembler::new(PointerConfig { multi_click_ms: 100, ..PointerConfig::default() });
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 25.0, 5.0).with_time(1_000), &m);
    run(&mut asm, release(LEFT, 25.0, 5.0).with_time(1_020), &m);
    let out = run(&mut asm, press(LEFT, 25.0, 5.0).with_time(1_150), &m);
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Simple, .. }]));
}

#[test]
fn the_click_clock_wraps_with_the_tick_count() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 25.0, 5.0).with_time(u32::MAX - 100), &m);
    run(&mut asm, release(LEFT, 25.0, 5.0).with_time(u32::MAX - 90), &m);
    let out = run(&mut asm, press(LEFT, 25.0, 5.0).with_time(100), &m);
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Word, .. }]));
}

#[test]
fn alt_drags_a_block() {
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 25.0, 5.0).with_mods(Mods::ALT), &InputModes::default());
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Block, .. }]));
}

#[test]
fn ctrl_click_on_a_link_opens_it_on_release_in_the_same_place() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    let out = run(&mut asm, press(LEFT, 25.0, 45.0).with_mods(Mods::CTRL).over_link(), &m);
    assert!(out.is_empty(), "nothing is selected yet");
    let out = run(&mut asm, release(LEFT, 27.0, 46.0).with_mods(Mods::CTRL).over_link(), &m);
    assert_eq!(out, [PointerOutcome::OpenLink { col: 2, row: 2 }]);
}

#[test]
fn ctrl_click_away_from_a_link_is_a_plain_click() {
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 25.0, 45.0).with_mods(Mods::CTRL), &InputModes::default());
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Simple, .. }]));
}

#[test]
fn dragging_off_a_link_selects_from_where_the_button_went_down() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 22.0, 45.0).with_mods(Mods::CTRL).over_link(), &m);
    let out = run(&mut asm, moved(75.0, 45.0).with_mods(Mods::CTRL), &m);
    assert_eq!(
        out,
        [
            PointerOutcome::SelectionStart {
                point: point(2, 2, Side::Left),
                kind: SelectionKind::Simple,
                extend: false
            },
            PointerOutcome::SelectionUpdate { point: point(7, 2, Side::Right), overshoot_rows: 0 },
        ]
    );
    let out = run(&mut asm, release(LEFT, 75.0, 45.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionEnd { point: point(7, 2, Side::Right), dragged: true }]);
}

#[test]
fn a_link_is_not_opened_when_the_program_has_the_pointer() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, press(LEFT, 25.0, 45.0).with_mods(Mods::CTRL).over_link(), &m);
    assert_eq!(reports(&out), [r"\e[<16;3;3M"]);
}

#[test]
fn a_motion_without_a_button_selects_nothing() {
    let mut asm = MouseAssembler::default();
    assert!(run(&mut asm, moved(25.0, 45.0), &InputModes::default()).is_empty());
}

// ---- wheel -------------------------------------------------------------------------------------------------------------

#[test]
fn the_wheel_scrolls_the_scrollback_by_lines() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    assert_eq!(run(&mut asm, wheel(120, 5.0, 5.0), &m), [PointerOutcome::Scroll(3)]);
    assert_eq!(run(&mut asm, wheel(-240, 5.0, 5.0), &m), [PointerOutcome::Scroll(-6)]);
}

#[test]
fn lines_per_notch_comes_from_the_system_setting() {
    let mut asm = MouseAssembler::new(PointerConfig { lines_per_notch: 1, ..PointerConfig::default() });
    assert_eq!(run(&mut asm, wheel(120, 5.0, 5.0), &InputModes::default()), [PointerOutcome::Scroll(1)]);
}

#[test]
fn a_precision_touchpad_scrolls_smoothly_and_loses_nothing() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    let mut lines = Vec::new();
    for _ in 0..8 {
        if let [PointerOutcome::Scroll(n)] = run(&mut asm, wheel(30, 5.0, 5.0), &m).as_slice() {
            lines.push(*n);
        }
    }
    // 8 x 30 = 240 = two notches = six lines, spread over the events instead of in a lump.
    assert_eq!(lines.iter().sum::<i32>(), 6);
    assert!(lines.len() > 2, "{lines:?}");
}

#[test]
fn reversing_the_wheel_drops_the_leftover() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    assert!(run(&mut asm, wheel(20, 5.0, 5.0), &m).is_empty());
    assert_eq!(run(&mut asm, wheel(-60, 5.0, 5.0), &m), [PointerOutcome::Scroll(-1)]);
}

#[test]
fn the_horizontal_wheel_does_not_scroll_the_scrollback() {
    let mut asm = MouseAssembler::default();
    assert!(run(&mut asm, hwheel(120, 5.0, 5.0), &InputModes::default()).is_empty());
}

#[test]
fn the_wheel_goes_to_a_program_that_tracks_the_mouse() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, wheel(120, 25.0, 45.0), &m)), [r"\e[<64;3;3M"]);
    assert_eq!(reports(&run(&mut asm, wheel(-120, 25.0, 45.0), &m)), [r"\e[<65;3;3M"]);
    assert_eq!(reports(&run(&mut asm, hwheel(120, 25.0, 45.0), &m)), [r"\e[<67;3;3M"]);
    assert_eq!(reports(&run(&mut asm, hwheel(-120, 25.0, 45.0), &m)), [r"\e[<66;3;3M"]);
}

#[test]
fn several_notches_in_one_event_are_repeated() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, wheel(360, 5.0, 5.0), &m)), [r"\e[<64;1;1M".repeat(3)]);
}

#[test]
fn a_program_gets_whole_notches_from_a_precision_wheel() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    let mut count = 0;
    for _ in 0..8 {
        count += reports(&run(&mut asm, wheel(30, 5.0, 5.0), &m)).len();
    }
    assert_eq!(count, 2);
}

#[test]
fn shift_wheel_scrolls_the_scrollback_even_while_the_program_tracks_the_mouse() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    let out = run(&mut asm, wheel(120, 5.0, 5.0).with_mods(Mods::SHIFT), &m);
    assert_eq!(out, [PointerOutcome::Scroll(3)]);
}

#[test]
fn alternate_scroll_turns_the_wheel_into_cursor_keys() {
    let m = InputModes { alt_screen: true, alternate_scroll: true, ..InputModes::default() };
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, wheel(120, 5.0, 5.0), &m)), [r"\e[A"]);
    assert_eq!(reports(&run(&mut asm, wheel(-240, 5.0, 5.0), &m)), [r"\e[B\e[B"]);
    let app = InputModes { app_cursor: true, ..m.clone() };
    assert_eq!(reports(&run(&mut asm, wheel(120, 5.0, 5.0), &app)), [r"\eOA"]);
}

#[test]
fn alternate_scroll_needs_the_alternate_screen() {
    let m = InputModes { alt_screen: false, alternate_scroll: true, ..InputModes::default() };
    let mut asm = MouseAssembler::default();
    assert_eq!(run(&mut asm, wheel(120, 5.0, 5.0), &m), [PointerOutcome::Scroll(3)]);
}

#[test]
fn mouse_tracking_wins_over_alternate_scroll() {
    let m = InputModes {
        mouse_mode: MouseMode::Normal,
        mouse_encoding: MouseEncoding::Sgr,
        alt_screen: true,
        alternate_scroll: true,
        ..InputModes::default()
    };
    let mut asm = MouseAssembler::default();
    assert_eq!(reports(&run(&mut asm, wheel(120, 5.0, 5.0), &m)), [r"\e[<64;1;1M"]);
}

// ---- cancel --------------------------------------------------------------------------------------------------------------------

#[test]
fn losing_the_capture_releases_the_buttons_the_program_saw_pressed() {
    let m = modes(MouseMode::Normal, MouseEncoding::Sgr);
    let mut asm = MouseAssembler::default();
    run(&mut asm, press(LEFT, 25.0, 45.0), &m);
    run(&mut asm, press(MouseButton::Right, 25.0, 45.0), &m);
    let out = run(&mut asm, PointerEvent::new(PointerKind::Cancel, 0.0, 0.0), &m);
    assert_eq!(reports(&out), [r"\e[<0;3;3m", r"\e[<2;3;3m"]);
    // The gesture is over: a later release is a stray.
    assert!(run(&mut asm, release(LEFT, 25.0, 45.0), &m).is_empty());
}

#[test]
fn losing_the_capture_ends_a_selection_where_the_pointer_was() {
    let mut asm = MouseAssembler::default();
    let m = InputModes::default();
    run(&mut asm, press(LEFT, 25.0, 45.0), &m);
    run(&mut asm, moved(75.0, 45.0), &m);
    let out = run(&mut asm, PointerEvent::new(PointerKind::Cancel, 0.0, 0.0), &m);
    assert_eq!(out, [PointerOutcome::SelectionEnd { point: point(7, 2, Side::Right), dragged: true }]);
    // A new gesture starts clean.
    let out = run(&mut asm, press(LEFT, 25.0, 45.0).with_time(5_000), &m);
    assert!(matches!(out.as_slice(), [PointerOutcome::SelectionStart { kind: SelectionKind::Simple, .. }]));
}

#[test]
fn a_cancel_with_nothing_down_does_nothing() {
    let mut asm = MouseAssembler::default();
    assert!(run(&mut asm, PointerEvent::new(PointerKind::Cancel, 0.0, 0.0), &InputModes::default()).is_empty());
}

#[test]
fn the_pty_limit_of_the_default_encoding_applies_through_the_assembler() {
    let m = modes(MouseMode::Normal, MouseEncoding::Default);
    let wide = PointerGrid { columns: 200, ..grid() };
    let mut asm = MouseAssembler::default();
    // Column 96 cannot be written in 7-bit ASCII: not reported rather than corrupted.
    assert!(reports(&asm.handle(&press(LEFT, 955.0, 5.0), &wide, &m)).is_empty());
    asm.handle(&release(LEFT, 955.0, 5.0), &wide, &m);
    assert_eq!(reports(&asm.handle(&press(LEFT, 945.0, 5.0), &wide, &m)).len(), 1);
}
