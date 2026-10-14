//! Golden images of one dense scene, dark and light, opaque and translucent (0.72 and 0.5).
//!
//! The scene has bold / italic / underline styles / strikeout / faint / reverse / ANSI colours, Traditional
//! Chinese, emoji, box drawing, blocks, selection, search matches and a cursor. Translucent frames are stored
//! composited over a dark and a light backdrop (left and right panel), because that is what the user sees; the
//! raw premultiplied output is covered by `translucent.rs`.
//!
//! Regenerate after an intended change: `NEKOMINAL_BLESS_GOLDEN=1 cargo test -p nekominal-render --test headless
//! golden`, then look at `tests/golden/*.png`.

use nekominal_render::{CellFlags, CursorShape, CursorView, Highlight, RowSpan, Theme};

use super::common::*;

fn scene() -> Scene {
    let mut s = Scene::new(44, 15);
    let plain = Style::new();
    let bold = Style::new().flag(CellFlags::BOLD);
    let mut c = s.put(0, 0, "Regular ", plain);
    c = s.put(0, c, "Bold ", bold);
    c = s.put(0, c, "Italic ", Style::new().flag(CellFlags::ITALIC));
    s.put(0, c, "BoldItalic", bold.flag(CellFlags::ITALIC));

    let mut c = s.put(1, 0, "single ", plain.underline(Underline::Single));
    c = s.put(1, c, "double ", plain.underline(Underline::Double));
    c = s.put(1, c, "curly ", plain.underline(Underline::Curly).underline_color(Color::Rgb(255, 85, 85)));
    c = s.put(1, c, "dotted ", plain.underline(Underline::Dotted).underline_color(idx(10)));
    c = s.put(1, c, "dashed ", plain.underline(Underline::Dashed).underline_color(idx(12)));
    s.put(1, c, "strike", plain.flag(CellFlags::STRIKEOUT));

    let mut c = s.put(2, 0, "faint ", plain.flag(CellFlags::FAINT));
    c = s.put(2, c, "reverse ", plain.flag(CellFlags::INVERSE));
    for i in 1..=6u8 {
        c = s.put(2, c, "ab ", plain.fg(idx(i)));
    }
    for i in 9..=14u8 {
        c = s.put(2, c, "ab ", bold.fg(idx(i - 8)));
    }
    let _ = c;

    let mut c = s.put(3, 0, " ERROR ", plain.fg(idx(15)).bg(idx(1)));
    c = s.put(3, c + 1, " OK ", plain.fg(idx(0)).bg(idx(2)));
    c = s.put(3, c + 1, " WARN ", plain.fg(idx(0)).bg(idx(3)).flag(CellFlags::BOLD));
    s.put(3, c + 1, " truecolor ", plain.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(0x6A, 0x3D, 0x9A)));

    s.put(4, 0, "繁體中文：終端機模擬器渲染測試，骨直令刃返", plain);
    s.put(5, 0, "注音：ㄓㄨˋ ㄧㄣ 倉頡：日月金木水 Ünïcödé e\u{301}", plain.fg(idx(6)));
    s.put(6, 0, "😀 👍🏽 ❤️ 🧑‍💻 🚀 ✅ ⭐ ✨ 🐱 ⚡ 🔥", plain);

    let frame = plain.fg(idx(4));
    s.put(7, 0, "┌────────┬───────┐ ┏━━━━┳━━┓ ╔═════╦══╗ ╭───╮", frame);
    s.put(8, 0, "│ light  │ cells │ ┃heav┃ x┃ ║doubl║ x║ │   │", frame);
    s.put(9, 0, "├────────┼───────┤ ┗━━━━┻━━┛ ╠═════╬══╣ ╰───╯", frame);
    s.put(10, 0, "└────────┴───────┘ ╒═══╤═══╕ ╚═════╩══╝ ╱╲╳", frame);
    s.put(11, 0, "█▉▊▋▌▍▎▏ ▀▄▔▕ ░▒▓ ▖▗▘▙▚▛▜▝▞▟ ⠁⠃⠇⡇⣇⣧⣷⣿", plain.fg(idx(2)));

    s.put(12, 2, "selected text region", plain);
    s.selection.push(RowSpan { row: 12, start: 2, end: 22 });
    let mut c = s.put(13, 2, "match ", plain);
    c = s.put(13, c, "match ", plain);
    s.put(13, c, "current", plain);
    s.highlights.push(Highlight { span: RowSpan { row: 13, start: 2, end: 7 }, current: false });
    s.highlights.push(Highlight { span: RowSpan { row: 13, start: 8, end: 13 }, current: false });
    s.highlights.push(Highlight { span: RowSpan { row: 13, start: 14, end: 21 }, current: true });

    s.put(14, 0, "❯", plain.fg(idx(2)).flag(CellFlags::BOLD));
    let end = s.put(14, 2, "cargo test --workspace", plain);
    s.cursor = Some(CursorView {
        col: end as u16,
        row: 14,
        shape: CursorShape::Block,
        visible: true,
        focused: true,
        wide: false,
    });
    s
}

/// Dark panel on a dark backdrop, light panel on a light backdrop, side by side. Opaque frames are one panel.
fn compose(theme: &Theme) -> (u32, u32, Vec<u8>) {
    let frame = render(&scene(), theme, test_config());
    if theme.background_opacity >= 1.0 {
        let opaque: Vec<u8> = frame.pixels().iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        return (frame.width, frame.height, opaque);
    }
    let left = frame.over(rgb(0x20262E));
    let right = frame.over(rgb(0xDCE2EA));
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut out = Vec::with_capacity(w * h * 8);
    for y in 0..h {
        out.extend_from_slice(&left[y * w * 4..(y + 1) * w * 4]);
        out.extend_from_slice(&right[y * w * 4..(y + 1) * w * 4]);
    }
    (frame.width * 2, frame.height, out)
}

fn check(name: &str, base: Theme, opacity: f32) {
    let theme = theme_with_opacity(base, opacity);
    let (w, h, rgba) = compose(&theme);
    assert_golden(name, w, h, &rgba);
}

#[test]
fn dark_opaque() {
    check("dark_opaque", Theme::campbell(), 1.0);
}

#[test]
fn dark_translucent_072() {
    check("dark_072", Theme::campbell(), 0.72);
}

#[test]
fn dark_translucent_050() {
    check("dark_050", Theme::campbell(), 0.5);
}

#[test]
fn light_opaque() {
    check("light_opaque", light_theme(), 1.0);
}

#[test]
fn light_translucent_072() {
    check("light_072", light_theme(), 0.72);
}

#[test]
fn light_translucent_050() {
    check("light_050", light_theme(), 0.5);
}
