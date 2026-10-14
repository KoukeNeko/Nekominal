//! The snapshot hands the renderer colours as the program set them. Resolution happens once, in the
//! renderer's `Theme`; INVERSE / HIDDEN / DIM in `flags` must therefore reach it unapplied.

use nekominal_term::{
    CellColor, CoreConfig, Snapshot, TerminalCore,
    alacritty_terminal::{term::cell::Flags, vte::ansi::Rgb},
};

fn snapshot_of(bytes: &[u8]) -> Snapshot {
    let mut core = TerminalCore::new(20, 3, CoreConfig::default());
    core.feed(bytes);
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot
}

#[test]
fn sgr_colours_stay_symbolic() {
    let s = snapshot_of(b"\x1b[31ma\x1b[91mb\x1b[38;5;200mc\x1b[38;2;1;2;3md\x1b[0me\x1b[44mf");
    let fg = |col| s.cell(0, col).fg_raw;
    assert_eq!(fg(0), CellColor::Indexed(1));
    assert_eq!(fg(1), CellColor::Indexed(9));
    assert_eq!(fg(2), CellColor::Indexed(200));
    assert_eq!(fg(3), CellColor::Rgb(1, 2, 3));
    assert_eq!(fg(4), CellColor::Default);
    assert_eq!(s.cell(0, 4).bg_raw, CellColor::Default);
    assert_eq!(s.cell(0, 5).bg_raw, CellColor::Indexed(4));
}

#[test]
fn inverse_hidden_and_dim_are_flags_not_applied_colours() {
    let s = snapshot_of(b"\x1b[7ma\x1b[0m\x1b[8mb\x1b[0m\x1b[2mc");
    assert!(s.cell(0, 0).flags.contains(Flags::INVERSE));
    assert_eq!((s.cell(0, 0).fg_raw, s.cell(0, 0).bg_raw), (CellColor::Default, CellColor::Default));
    assert!(s.cell(0, 1).flags.contains(Flags::HIDDEN));
    assert!(s.cell(0, 2).flags.contains(Flags::DIM));
}

#[test]
fn osc_palette_overrides_are_exported() {
    let s = snapshot_of(b"\x1b]4;1;rgb:ff/80/00\x07\x1b]11;rgb:10/20/30\x07");
    assert!(s.overrides.contains(&(1, Rgb { r: 0xff, g: 0x80, b: 0 })));
    assert!(s.overrides.contains(&(257, Rgb { r: 0x10, g: 0x20, b: 0x30 })));
}
