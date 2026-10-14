//! The candidate sequences and the scenario groups of the ConPTY input experiment.

#![allow(dead_code)]

#[path = "rig.rs"]
pub mod rig;

use nekominal_input::win32::control_state::*;
use nekominal_input::*;
pub use rig::*;

// ---- setups: what the "application" asks the terminal for -----------------------------------

pub const KITTY_D: &[u8] = b"\x1b[>1u";
pub const KITTY_ALL: &[u8] = b"\x1b[>31u";
pub const APP_MODES: &[u8] = b"\x1b[?1h\x1b=";
pub const MOUSE_SGR: &[u8] = b"\x1b[?1000h\x1b[?1006h";
pub const MOUSE_DEFAULT: &[u8] = b"\x1b[?1000h";
pub const MOUSE_UTF8: &[u8] = b"\x1b[?1000h\x1b[?1005h";
pub const MOUSE_URXVT: &[u8] = b"\x1b[?1000h\x1b[?1015h";
pub const MOUSE_ANY_SGR: &[u8] = b"\x1b[?1003h\x1b[?1006h";
pub const FOCUS_ON: &[u8] = b"\x1b[?1004h";
pub const PASTE_ON: &[u8] = b"\x1b[?2004h";

fn from_input(label: &str, input: KeyInput, modes: &InputModes) -> Step {
    let bytes = encode_key(&input, modes).unwrap_or_else(|| panic!("{label}: encoder returned nothing"));
    step(label, bytes)
}

fn win_key(label: &str, vk: u16, sc: u16, uc: u16, cs: u32) -> Step {
    let mut bytes = Vec::new();
    for key_down in [true, false] {
        bytes.extend(encode_win32_input(&Win32Key {
            virtual_key: vk,
            scan_code: sc,
            unicode: uc,
            key_down,
            control_state: cs,
            repeat_count: 1,
        }));
    }
    step(label, bytes)
}

/// Bytes our encoder produces in legacy mode.
pub fn keyboard_legacy() -> Vec<Step> {
    let m = InputModes::default();
    let app = InputModes { app_cursor: true, ..InputModes::default() };
    let key = |k: Key, mods: Mods| KeyInput::new(k).with_mods(mods);
    let typed = |c: char| KeyInput::new(Key::Char(c)).with_text(c.to_string());
    vec![
        from_input("a", typed('a'), &m),
        step("CJK text", encode_text("你好😀")),
        from_input("Enter", key(Key::Enter, Mods::NONE), &m),
        from_input("Shift+Enter", key(Key::Enter, Mods::SHIFT), &m),
        from_input("Ctrl+Enter (LF)", key(Key::Enter, Mods::CTRL), &m),
        from_input("Alt+Enter", key(Key::Enter, Mods::ALT), &m),
        from_input("Tab", key(Key::Tab, Mods::NONE), &m),
        from_input("Shift+Tab", key(Key::Tab, Mods::SHIFT), &m),
        from_input("Backspace", key(Key::Backspace, Mods::NONE), &m),
        from_input("Ctrl+Backspace", key(Key::Backspace, Mods::CTRL), &m),
        from_input("Alt+Backspace", key(Key::Backspace, Mods::ALT), &m),
        from_input("Escape (lone ESC)", key(Key::Escape, Mods::NONE), &m),
        from_input("Alt+a", key(Key::Char('a'), Mods::ALT), &m),
        from_input("Ctrl+A", key(Key::Char('a'), Mods::CTRL), &m),
        from_input("Ctrl+C", key(Key::Char('c'), Mods::CTRL), &m),
        from_input("Ctrl+Space (NUL)", key(Key::Char(' '), Mods::CTRL), &m),
        from_input("Up", key(Key::Up, Mods::NONE), &m),
        from_input("Up (app cursor SS3)", key(Key::Up, Mods::NONE), &app),
        from_input("Shift+Up", key(Key::Up, Mods::SHIFT), &m),
        from_input("Ctrl+Right", key(Key::Right, Mods::CTRL), &m),
        from_input("Home", key(Key::Home, Mods::NONE), &m),
        from_input("Delete", key(Key::Delete, Mods::NONE), &m),
        from_input("PageUp", key(Key::PageUp, Mods::NONE), &m),
        from_input("F1 (SS3)", key(Key::F(1), Mods::NONE), &m),
        from_input("F5", key(Key::F(5), Mods::NONE), &m),
        from_input("Shift+F5", key(Key::F(5), Mods::SHIFT), &m),
        from_input("F13", key(Key::F(13), Mods::NONE), &m),
    ]
}

/// Bytes our encoder produces under kitty flags (sent regardless of what the child pushed).
pub fn keyboard_kitty() -> Vec<Step> {
    let d = InputModes { kitty: KittyFlags::DISAMBIGUATE, ..InputModes::default() };
    let full = InputModes { kitty: KittyFlags::ALL, ..InputModes::default() };
    let events = InputModes {
        kitty: KittyFlags::DISAMBIGUATE | KittyFlags::REPORT_EVENTS,
        ..InputModes::default()
    };
    let key = |k: Key, mods: Mods| KeyInput::new(k).with_mods(mods);
    let typed = |c: char| KeyInput::new(Key::Char(c)).with_text(c.to_string());
    vec![
        from_input("k:Shift+Enter", key(Key::Enter, Mods::SHIFT), &d),
        from_input("k:Ctrl+Enter", key(Key::Enter, Mods::CTRL), &d),
        from_input("k:Alt+Enter", key(Key::Enter, Mods::ALT), &d),
        from_input("k:Escape", key(Key::Escape, Mods::NONE), &d),
        from_input("k:Ctrl+[", key(Key::Char('['), Mods::CTRL), &d),
        from_input("k:Ctrl+I", key(Key::Char('i'), Mods::CTRL), &d),
        from_input("k:Ctrl+M", key(Key::Char('m'), Mods::CTRL), &d),
        from_input("k:Alt+a", key(Key::Char('a'), Mods::ALT), &d),
        from_input("k:Ctrl+Shift+A", key(Key::Char('a'), Mods::CTRL | Mods::SHIFT), &d),
        from_input("k:Shift+Tab", key(Key::Tab, Mods::SHIFT), &d),
        from_input("k:Up (CSI form)", key(Key::Up, Mods::NONE), &d),
        from_input("k:release a", typed('a').with_event(KeyEvent::Release), &events),
        from_input("k:repeat Up", key(Key::Up, Mods::NONE).with_event(KeyEvent::Repeat), &events),
        from_input("k:all keys a", typed('a'), &full),
        from_input("k:text a", typed('a'), &full),
        from_input(
            "k:Cyrillic Ctrl+C",
            key(Key::Char('с'), Mods::CTRL).with_base_layout('c'),
            &InputModes { kitty: KittyFlags::DISAMBIGUATE | KittyFlags::REPORT_ALTERNATES, ..InputModes::default() },
        ),
        from_input("k:F13", key(Key::F(13), Mods::NONE), &d),
        from_input("k:KP0 (all keys)", KeyInput::new(Key::Keypad(KeypadKey::Digit(0))), &full),
        from_input("k:KP Enter", key(Key::Keypad(KeypadKey::Enter), Mods::NONE), &d),
        from_input("k:LeftShift (all keys)", KeyInput::new(Key::Modifier(ModifierKey::LeftShift)), &full),
    ]
}

/// win32-input-mode records.
pub fn keyboard_win32() -> Vec<Step> {
    let shift = SHIFT_PRESSED;
    let ctrl = LEFT_CTRL_PRESSED;
    vec![
        win_key("w:a", 0x41, 30, 'a' as u16, 0),
        win_key("w:Enter", 0x0d, 28, 13, 0),
        win_key("w:Shift+Enter", 0x0d, 28, 13, shift),
        win_key("w:Ctrl+Enter", 0x0d, 28, 10, ctrl),
        win_key("w:KP Enter", 0x0d, 28, 13, ENHANCED_KEY),
        win_key("w:Tab", 0x09, 15, 9, 0),
        win_key("w:Shift+Tab", 0x09, 15, 9, shift),
        win_key("w:Backspace", 0x08, 14, 8, 0),
        win_key("w:Ctrl+Backspace", 0x08, 14, 0x7f, ctrl),
        win_key("w:Escape", 0x1b, 1, 27, 0),
        win_key("w:Ctrl+[", 0xdb, 26, 27, ctrl),
        win_key("w:Ctrl+A", 0x41, 30, 1, ctrl),
        win_key("w:Ctrl+C", 0x43, 46, 3, ctrl),
        win_key("w:Alt+a", 0x41, 30, 'a' as u16, LEFT_ALT_PRESSED),
        win_key("w:Up (enhanced)", 0x26, 72, 0, ENHANCED_KEY),
        win_key("w:Ctrl+Up", 0x26, 72, 0, ENHANCED_KEY | ctrl),
        win_key("w:F5", 0x74, 63, 0, 0),
        win_key("w:Shift+F5", 0x74, 63, 0, shift),
        win_key("w:Delete (enhanced)", 0x2e, 83, 0, ENHANCED_KEY),
        win_key("w:AltGr euro (text)", 0x45, 18, 0x20ac, ctrl | RIGHT_ALT_PRESSED),
        win_key("w:KP5 NumLock", 0x65, 76, '5' as u16, NUMLOCK_ON),
        step("w:CJK text (vk0 down+up)", encode_win32_text("你")),
        step("w:emoji text (surrogates)", encode_win32_text("😀")),
        step("w:CJK down only", {
            encode_win32_input(&Win32Key {
                virtual_key: 0,
                scan_code: 0,
                unicode: '好' as u16,
                key_down: true,
                control_state: 0,
                repeat_count: 1,
            })
        }),
    ]
}

pub fn mouse_candidates() -> Vec<Step> {
    let sgr = |kind, col, row| {
        let modes = InputModes { mouse_mode: MouseMode::Any, mouse_encoding: MouseEncoding::Sgr, ..InputModes::default() };
        encode_mouse(&MouseInput::new(kind, col, row), &modes, &mut MouseState::default()).unwrap()
    };
    let with = |encoding, kind, col, row| {
        // Raw 8-bit reports on purpose: the experiment measures what ConPTY does with them.
        let modes = InputModes { mouse_mode: MouseMode::Any, mouse_encoding: encoding, pty_decodes_utf8: false, ..InputModes::default() };
        encode_mouse(&MouseInput::new(kind, col, row), &modes, &mut MouseState::default()).unwrap()
    };
    vec![
        step("m:SGR press", sgr(MouseKind::Press(MouseButton::Left), 9, 4)),
        step("m:SGR release", sgr(MouseKind::Release(MouseButton::Left), 9, 4)),
        step("m:SGR right press", sgr(MouseKind::Press(MouseButton::Right), 9, 4)),
        step("m:SGR wheel up", sgr(MouseKind::Wheel(WheelDirection::Up), 9, 4)),
        step("m:SGR wheel down", sgr(MouseKind::Wheel(WheelDirection::Down), 9, 4)),
        step("m:SGR motion (no button)", sgr(MouseKind::Move { held: None }, 10, 4)),
        step("m:SGR drag (left)", sgr(MouseKind::Move { held: Some(MouseButton::Left) }, 11, 4)),
        step("m:SGR press col 150", sgr(MouseKind::Press(MouseButton::Left), 150, 4)),
        step("m:default press", with(MouseEncoding::Default, MouseKind::Press(MouseButton::Left), 9, 4)),
        step("m:default release", with(MouseEncoding::Default, MouseKind::Release(MouseButton::Left), 9, 4)),
        step("m:default press col 90 (<=95)", with(MouseEncoding::Default, MouseKind::Press(MouseButton::Left), 90, 4)),
        step("m:default press col 100 (>95)", with(MouseEncoding::Default, MouseKind::Press(MouseButton::Left), 100, 4)),
        step("m:default press col 200", with(MouseEncoding::Default, MouseKind::Press(MouseButton::Left), 200, 4)),
        step("m:utf8 press col 200", with(MouseEncoding::Utf8, MouseKind::Press(MouseButton::Left), 200, 4)),
        step("m:urxvt press", with(MouseEncoding::Urxvt, MouseKind::Press(MouseButton::Left), 9, 4)),
    ]
}

pub fn paste_candidates() -> Vec<Step> {
    vec![
        step("p:bracketed ascii", prepare_paste("one\ntwo", true)),
        step("p:bracketed CJK+emoji", prepare_paste("你好\n世界 😀", true)),
        step("p:unbracketed multiline", prepare_paste("one\ntwo", false)),
        step("p:start marker alone", b"\x1b[200~".to_vec()),
        step("p:end marker alone", b"\x1b[201~".to_vec()),
        step("p:focus in", b"\x1b[I".to_vec()),
        step("p:focus out", b"\x1b[O".to_vec()),
    ]
}

pub fn app_mode_candidates() -> Vec<Step> {
    vec![
        step("a:Up CSI", b"\x1b[A".to_vec()),
        step("a:Up SS3", b"\x1bOA".to_vec()),
        step("a:Home CSI", b"\x1b[H".to_vec()),
        step("a:Home SS3", b"\x1bOH".to_vec()),
        step("a:F1 SS3", b"\x1bOP".to_vec()),
        step("a:KP0 SS3 p", b"\x1bOp".to_vec()),
        step("a:digit 0", b"0".to_vec()),
        step("a:KP Enter SS3 M", b"\x1bOM".to_vec()),
    ]
}

/// Keys that the legacy bytes cannot carry faithfully: how do the children see them?
pub fn extra_keys() -> Vec<Step> {
    let ctrl = LEFT_CTRL_PRESSED;
    vec![
        step("e:Ctrl+Space raw NUL", b"\x00".to_vec()),
        step("e:Ctrl+Space kitty", b"\x1b[32;5u".to_vec()),
        win_key("e:w Ctrl+Space vk20 uc20", 0x20, 0x39, 0x20, ctrl),
        win_key("e:w Ctrl+Space vk20 uc00", 0x20, 0x39, 0x00, ctrl),
        win_key("e:w Space", 0x20, 0x39, 0x20, 0),
        win_key("e:w Alt+Space", 0x20, 0x39, 0x20, LEFT_ALT_PRESSED),
        win_key("e:w Shift+Space", 0x20, 0x39, 0x20, SHIFT_PRESSED),
        win_key("e:w Ctrl+Shift+A", 0x41, 30, 1, ctrl | SHIFT_PRESSED),
        win_key("e:w Ctrl+Alt+A (chord)", 0x41, 30, 0, ctrl | LEFT_ALT_PRESSED),
        win_key("e:w Pause", 0x13, 69, 0, 0),
        win_key("e:w Menu (VK_APPS)", 0x5d, 93, 0, ENHANCED_KEY),
        win_key("e:w Insert (enh)", 0x2d, 82, 0, ENHANCED_KEY),
        win_key("e:w PageDown (enh)", 0x22, 81, 0, ENHANCED_KEY),
        win_key("e:w F13", 0x7c, 100, 0, 0),
        win_key("e:w F24", 0x87, 118, 0, 0),
        win_key("e:w KP Left (non-enh)", 0x25, 75, 0, 0),
        win_key("e:w KP Clear (VK_CLEAR)", 0x0c, 76, 0, 0),
        win_key("e:w Ctrl+Backspace uc08", 0x08, 14, 0x08, ctrl),
        win_key("e:w Alt+Enter", 0x0d, 28, 13, LEFT_ALT_PRESSED),
        win_key("e:w Ctrl+Shift+Enter", 0x0d, 28, 10, ctrl | SHIFT_PRESSED),
        win_key("e:w Escape+Alt", 0x1b, 1, 27, LEFT_ALT_PRESSED),
    ]
}

/// A group of scenarios: (name, child, setup, steps).
pub struct Planned {
    pub name: String,
    pub child: Child,
    pub setup: Vec<u8>,
    pub steps: Vec<Step>,
}

fn all_keyboard() -> Vec<Step> {
    let mut steps = keyboard_legacy();
    steps.extend(keyboard_kitty());
    steps.extend(keyboard_win32());
    steps
}

pub fn plan(group: &str) -> Vec<Planned> {
    let planned = |name: &str, child, setup: &[u8], steps: Vec<Step>| Planned {
        name: name.to_string(),
        child,
        setup: setup.to_vec(),
        steps,
    };
    match group {
        "keyboard-node" => vec![
            planned("node/no-setup", Child::Node, b"", all_keyboard()),
            planned("node/kitty-1", Child::Node, KITTY_D, all_keyboard()),
            planned("node/kitty-31", Child::Node, KITTY_ALL, all_keyboard()),
        ],
        "keyboard-win32" => vec![
            planned("win32/no-setup", Child::Win32, b"", all_keyboard()),
            planned("win32/kitty-1", Child::Win32, KITTY_D, all_keyboard()),
        ],
        "keyboard-vt" => vec![
            planned("vt/no-setup", Child::VtInput, b"", all_keyboard()),
            planned("vt/kitty-1", Child::VtInput, KITTY_D, all_keyboard()),
            planned("vt/kitty-31", Child::VtInput, KITTY_ALL, all_keyboard()),
        ],
        "modes" => vec![
            planned("vt/app-modes", Child::VtInput, APP_MODES, app_mode_candidates()),
            planned("node/app-modes", Child::Node, APP_MODES, app_mode_candidates()),
            planned("win32/app-modes", Child::Win32, APP_MODES, app_mode_candidates()),
        ],
        "extra" => vec![
            planned("node/extra", Child::Node, b"", extra_keys()),
            planned("vt/extra", Child::VtInput, b"", extra_keys()),
            planned("win32/extra", Child::Win32, b"", extra_keys()),
        ],
        "startup" => vec![
            planned("node/startup", Child::Node, b"", vec![]),
            planned("win32/startup", Child::Win32, b"", vec![]),
            planned("vt/startup", Child::VtInput, b"", vec![]),
        ],
        "mouse" => vec![
            planned("node/mouse-sgr", Child::Node, MOUSE_SGR, mouse_candidates()),
            planned("node/mouse-default", Child::Node, MOUSE_DEFAULT, mouse_candidates()),
            planned("node/mouse-utf8", Child::Node, MOUSE_UTF8, mouse_candidates()),
            planned("node/mouse-urxvt", Child::Node, MOUSE_URXVT, mouse_candidates()),
            planned("node/mouse-any-sgr", Child::Node, MOUSE_ANY_SGR, mouse_candidates()),
            planned("node/mouse-off", Child::Node, b"", mouse_candidates()),
            planned("vt/mouse-sgr", Child::VtInput, MOUSE_SGR, mouse_candidates()),
            planned("vt/mouse-off", Child::VtInput, b"", mouse_candidates()),
            planned("win32/mouse-input", Child::Win32, MOUSE_SGR, mouse_candidates()),
        ],
        "paste-focus" => vec![
            planned("node/paste+focus on", Child::Node, b"\x1b[?2004h\x1b[?1004h", paste_candidates()),
            planned("node/off", Child::Node, b"", paste_candidates()),
            planned("vt/paste+focus on", Child::VtInput, b"\x1b[?2004h\x1b[?1004h", paste_candidates()),
            planned("vt/off", Child::VtInput, b"", paste_candidates()),
            planned("win32/off", Child::Win32, b"", paste_candidates()),
            planned("win32/paste+focus on", Child::Win32, b"\x1b[?2004h\x1b[?1004h", paste_candidates()),
        ],
        other => panic!("unknown group {other}"),
    }
}

pub fn run(planned: &Planned) -> ScenarioResult {
    run_scenario(planned.child, &planned.setup, &planned.name.replace('/', "-"), &planned.steps)
}
