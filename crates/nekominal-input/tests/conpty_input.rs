//! ConPTY input experiments against the bundled OpenConsole. Run with
//! `cargo test --test conpty_input -- --ignored --nocapture --test-threads=1`.
//! Each test prints the matrix it measured; the assertions pin the findings that the design
//! depends on.

#[path = "../experiments/matrix.rs"]
mod matrix;
// The typist of the key assembler tests: streams of Windows key events for the assembled-stream experiments.
#[allow(dead_code)]
#[path = "common/typist.rs"]
mod typist;

use matrix::*;

fn run_group(name: &str) -> Vec<(Planned, ScenarioResult)> {
    plan(name)
        .into_iter()
        .map(|planned| {
            let result = run(&planned);
            let report = result.report(&planned.name);
            print!("{report}");
            if let Ok(path) = std::env::var("INPUT_REPORT") {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
                file.write_all(report.as_bytes()).unwrap();
            }
            (planned, result)
        })
        .collect()
}

#[test]
#[ignore]
fn backend_identity() {
    let rig = Rig::spawn(Child::Node, b"", "identity");
    println!("backend: {}", rig.backend);
    assert!(rig.backend.starts_with("bundled"), "the test binary must load the bundled ConPTY: {}", rig.backend);
}

#[test]
#[ignore]
fn keyboard_node() {
    run_group("keyboard-node");
}

#[test]
#[ignore]
fn keyboard_win32() {
    run_group("keyboard-win32");
}

#[test]
#[ignore]
fn keyboard_vt() {
    run_group("keyboard-vt");
}

#[test]
#[ignore]
fn modes() {
    run_group("modes");
}

#[test]
#[ignore]
fn mouse() {
    run_group("mouse");
}

#[test]
#[ignore]
fn paste_focus() {
    run_group("paste-focus");
}

#[test]
#[ignore]
fn extra() {
    run_group("extra");
}

#[test]
#[ignore]
fn startup() {
    run_group("startup");
}

// ---- findings: assertions on what ConPTY does with the bytes the library produces ------------

mod findings {
    use super::matrix::*;
    use nekominal_input::win32::control_state::*;
    use nekominal_input::*;

    fn modes_with_kitty(bits: u8) -> InputModes {
        InputModes { kitty: KittyFlags::from_bits_truncate(bits), ..InputModes::default() }
    }

    /// Runs the steps against a child and returns the result.
    fn run(child: Child, setup: &[u8], tag: &str, steps: Vec<Step>) -> ScenarioResult {
        let result = run_scenario(child, setup, tag, &steps);
        assert!(result.ready, "{tag}: child did not start");
        print!("{}", result.report(tag));
        result
    }

    fn record(vk: u16, sc: u16, uc: u16, cs: u32, key_down: bool) -> Win32Key {
        Win32Key { virtual_key: vk, scan_code: sc, unicode: uc, key_down, control_state: cs, repeat_count: 1 }
    }

    /// Both halves of a key press through the ConPTY route.
    fn press(input: KeyInput, vk: u16, sc: u16, uc: u16, cs: u32, modes: &InputModes) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (event, down) in [(KeyEvent::Press, true), (KeyEvent::Release, false)] {
            let input = input.clone().with_event(event);
            bytes.extend(encode_key_conpty(&input, &record(vk, sc, uc, cs, down), modes).unwrap_or_default());
        }
        bytes
    }

    #[test]
    #[ignore]
    fn kitty_route_delivers_exact_bytes_to_a_vt_child() {
        let modes = modes_with_kitty(1);
        let key = |k: Key, m: Mods| KeyInput::new(k).with_mods(m);
        let cases: Vec<(&str, KeyInput, &str)> = vec![
            ("Shift+Enter", key(Key::Enter, Mods::SHIFT), r"\e[13;2u"),
            ("Ctrl+Enter", key(Key::Enter, Mods::CTRL), r"\e[13;5u"),
            ("Escape", key(Key::Escape, Mods::NONE), r"\e[27u"),
            ("Ctrl+[", key(Key::Char('['), Mods::CTRL), r"\e[91;5u"),
            ("Ctrl+I (not Tab)", key(Key::Char('i'), Mods::CTRL), r"\e[105;5u"),
            ("Ctrl+M (not Enter)", key(Key::Char('m'), Mods::CTRL), r"\e[109;5u"),
            ("Ctrl+Space", key(Key::Char(' '), Mods::CTRL), r"\e[32;5u"),
            ("Alt+Backspace", key(Key::Backspace, Mods::ALT), r"\e[127;3u"),
            ("F13", key(Key::F(13), Mods::NONE), r"\e[57376u"),
            ("a (text)", KeyInput::new(Key::Char('a')).with_text("a"), "a"),
            ("Enter (legacy byte)", key(Key::Enter, Mods::NONE), r"\r"),
        ];
        let steps: Vec<Step> = cases
            .iter()
            .map(|(name, input, _)| step(name, press(input.clone(), 0, 0, 0, 0, &modes)))
            .collect();
        for setup in [KITTY_D, b"".as_slice()] {
            let result = run(Child::Node, setup, "kitty-route-node", steps.iter().map(|s| Step { label: s.label.clone(), bytes: s.bytes.clone() }).collect());
            for (name, _, expected) in &cases {
                let got = result.step(name).node_chunks().concat();
                assert_eq!(show(&got), *expected, "{name} (child setup {:?})", show(setup));
            }
        }
    }

    #[test]
    #[ignore]
    fn win32_route_is_lossless_for_a_win32_console_child() {
        let modes = InputModes::default();
        let key = |k: Key, m: Mods| KeyInput::new(k).with_mods(m);
        let steps = vec![
            step("Shift+Enter", press(key(Key::Enter, Mods::SHIFT), 0x0d, 28, 13, SHIFT_PRESSED, &modes)),
            step("Ctrl+Space", press(key(Key::Char(' '), Mods::CTRL), 0x20, 57, 0x20, LEFT_CTRL_PRESSED, &modes)),
            step("Alt+Escape", press(key(Key::Escape, Mods::ALT), 0x1b, 1, 27, LEFT_ALT_PRESSED, &modes)),
            step("F13", press(key(Key::F(13), Mods::NONE), 0x7c, 100, 0, 0, &modes)),
            step("Menu", press(key(Key::Menu, Mods::NONE), 0x5d, 93, 0, ENHANCED_KEY, &modes)),
            step("Ctrl+Alt+A", press(key(Key::Char('a'), Mods::CTRL | Mods::ALT), 0x41, 30, 0, LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED, &modes)),
        ];
        let result = run(Child::Win32, b"", "win32-route-win32", steps);
        let first = |name: &str| result.step(name).key_records().into_iter().find(|r| r.1 != 0x10 && r.1 != 0x11 && r.1 != 0x12).unwrap();
        assert_eq!((first("Shift+Enter").0, first("Shift+Enter").1, first("Shift+Enter").4 & 0x10), (true, 0x0d, 0x10));
        assert_eq!((first("Ctrl+Space").1, first("Ctrl+Space").4 & 0x0c), (0x20, 0x08));
        // Alt+Escape is a system key for conhost: the key-up arrives, the key-down does not.
        let alt_escape = result.step("Alt+Escape").key_records();
        assert!(!alt_escape.is_empty() && alt_escape.iter().all(|r| !r.0 && r.1 == 0x1b), "{alt_escape:?}");
        assert_eq!(first("F13").1, 0x7c);
        assert_eq!((first("Menu").1, first("Menu").4 & ENHANCED_KEY), (0x5d, ENHANCED_KEY));
        assert_eq!((first("Ctrl+Alt+A").1, first("Ctrl+Alt+A").3), (0x41, 0));
    }

    #[test]
    #[ignore]
    fn win32_route_renders_legacy_bytes_for_a_vt_child_and_loses_four_keys() {
        let modes = InputModes::default();
        let key = |k: Key, m: Mods| KeyInput::new(k).with_mods(m);
        let steps = vec![
            step("Shift+Enter", press(key(Key::Enter, Mods::SHIFT), 0x0d, 28, 13, SHIFT_PRESSED, &modes)),
            step("Ctrl+Enter", press(key(Key::Enter, Mods::CTRL), 0x0d, 28, 10, LEFT_CTRL_PRESSED, &modes)),
            step("Ctrl+Up", press(key(Key::Up, Mods::CTRL), 0x26, 72, 0, LEFT_CTRL_PRESSED | ENHANCED_KEY, &modes)),
            step("Ctrl+Space", press(key(Key::Char(' '), Mods::CTRL), 0x20, 57, 0x20, LEFT_CTRL_PRESSED, &modes)),
            step("Alt+Escape", press(key(Key::Escape, Mods::ALT), 0x1b, 1, 27, LEFT_ALT_PRESSED, &modes)),
            step("F24", press(key(Key::F(24), Mods::NONE), 0x87, 118, 0, 0, &modes)),
            step("Menu", press(key(Key::Menu, Mods::NONE), 0x5d, 93, 0, ENHANCED_KEY, &modes)),
        ];
        let result = run(Child::Node, b"", "win32-route-node", steps);
        let got = |name: &str| show(&result.step(name).node_chunks().concat());
        // Shift+Enter is only Enter: that is why a program that wants it asks for kitty flags.
        assert_eq!(got("Shift+Enter"), r"\r");
        assert_eq!(got("Ctrl+Enter"), r"\n");
        assert_eq!(got("Ctrl+Up"), r"\e[1;5A");
        for lost in ["Ctrl+Space", "Alt+Escape", "F24", "Menu"] {
            assert_eq!(got(lost), "", "{lost} has no VT form in conhost");
        }
    }

    #[test]
    #[ignore]
    fn kitty_bytes_are_typed_text_to_a_win32_console_child() {
        // Why kitty bytes must only follow a program's own request: a Win32 console child (cmd,
        // PSReadLine) sees the sequence as ordinary typed characters.
        let modes = modes_with_kitty(1);
        let bytes = encode_key(&KeyInput::new(Key::Enter).with_mods(Mods::SHIFT), &modes).unwrap();
        let result = run(Child::Win32, b"", "kitty-bytes-win32", vec![step("Shift+Enter kitty", bytes)]);
        let stream = result.step("Shift+Enter kitty").stream().concat();
        assert_eq!(show(&stream), r"\e[13;2u");
    }

    #[test]
    #[ignore]
    fn unicode_text_survives_raw_utf8() {
        let steps = vec![
            step("Zhuyin result", encode_text("你好，世界")),
            step("emoji and ZWJ", encode_text("😀 👨\u{200d}👩\u{200d}👧 ❤\u{fe0f}")),
            step("Bopomofo letters", encode_text("ㄋㄧˇㄏㄠˇ")),
        ];
        let node = run(Child::Node, b"", "unicode-node", steps.iter().map(|s| Step { label: s.label.clone(), bytes: s.bytes.clone() }).collect());
        let win = run(Child::Win32, b"", "unicode-win32", steps.iter().map(|s| Step { label: s.label.clone(), bytes: s.bytes.clone() }).collect());
        let vt = run(Child::VtInput, b"", "unicode-vt", steps.iter().map(|s| Step { label: s.label.clone(), bytes: s.bytes.clone() }).collect());
        for s in &steps {
            assert_eq!(node.step(&s.label).node_chunks().concat(), s.bytes, "node {}", s.label);
            assert_eq!(vt.step(&s.label).stream().concat(), s.bytes, "vt {}", s.label);
            // Win32 reads UTF-16 units; the S line is their UTF-8 again. Batches may split a pair.
            let text: Vec<u8> = win.step(&s.label).stream().concat();
            assert!(String::from_utf8_lossy(&text).chars().filter(|c| *c != '\u{fffd}').count() > 0);
        }
    }

    #[test]
    #[ignore]
    fn mouse_reports_follow_the_encodings_under_conpty() {
        let sgr_modes = InputModes { mouse_mode: MouseMode::Normal, mouse_encoding: MouseEncoding::Sgr, ..InputModes::default() };
        let click = MouseInput::new(MouseKind::Press(MouseButton::Left), 9, 4);
        let sgr = encode_mouse(&click, &sgr_modes, &mut MouseState::default()).unwrap();
        let utf8_modes = InputModes { mouse_encoding: MouseEncoding::Utf8, ..sgr_modes.clone() };
        let wide = MouseInput::new(MouseKind::Press(MouseButton::Left), 200, 4);
        let utf8 = encode_mouse(&wide, &utf8_modes, &mut MouseState::default()).unwrap();
        // The raw default encoding past column 95 (what the library refuses to send under ConPTY).
        let raw_modes = InputModes { mouse_encoding: MouseEncoding::Default, pty_decodes_utf8: false, ..sgr_modes.clone() };
        let default_wide = encode_mouse(&MouseInput::new(MouseKind::Press(MouseButton::Left), 100, 4), &raw_modes, &mut MouseState::default()).unwrap();
        let steps = || vec![step("sgr", sgr.clone()), step("utf8 col 200", utf8.clone()), step("default col 100", default_wide.clone())];

        let node = run(Child::Node, MOUSE_SGR, "mouse-node", steps());
        assert_eq!(node.step("sgr").node_chunks().concat(), sgr);
        assert_eq!(node.step("utf8 col 200").node_chunks().concat(), utf8);
        // One byte 0x84 is not UTF-8: ConPTY turns it into U+FFFD.
        assert_eq!(node.step("default col 100").node_chunks().concat(), b"\x1b[M \xef\xbf\xbd%");

        // A Win32 console child gets a MOUSE_EVENT_RECORD for the SGR report: 0-based cell, left button.
        let win = run(Child::Win32, b"", "mouse-win32", vec![step("sgr", sgr.clone())]);
        let mouse = win.step("sgr").mouse_records();
        assert_eq!(mouse.len(), 1);
        assert!(mouse[0].starts_with("M x=9 y=4 buttons=0x1"), "{mouse:?}");
    }

    #[test]
    #[ignore]
    fn focus_reports_are_gated_by_conpty() {
        let steps = || vec![step("in", b"\x1b[I".to_vec()), step("out", b"\x1b[O".to_vec())];
        let on = run(Child::Node, FOCUS_ON, "focus-node-on", steps());
        assert_eq!(on.step("in").node_chunks().concat(), b"\x1b[I");
        let off = run(Child::Node, b"", "focus-node-off", steps());
        assert!(off.step("in").node_chunks().is_empty() && off.step("out").node_chunks().is_empty());
        // A Win32 child always gets FOCUS_EVENT records.
        let win = run(Child::Win32, b"", "focus-win32", steps());
        assert_eq!(win.step("in").lines, vec!["F focus=1".to_string()]);
        assert_eq!(win.step("out").lines, vec!["F focus=0".to_string()]);
    }

    #[test]
    #[ignore]
    fn bracketed_paste_markers_reach_vt_children_only() {
        let paste = prepare_paste("one\ntwo 你好", true);
        let vt = run(Child::Node, PASTE_ON, "paste-node", vec![step("paste", paste.clone())]);
        assert_eq!(vt.step("paste").node_chunks().concat(), paste);
        let win = run(Child::Win32, b"", "paste-win32", vec![step("paste", paste)]);
        let stream = win.step("paste").stream().concat();
        assert_eq!(show(&stream), r"one\rtwo 你好", "the markers are swallowed for a Win32 child");
    }

    /// Ctrl+C must interrupt a cooked-mode console program whichever route carried it.
    #[test]
    #[ignore]
    fn ctrl_c_interrupts_a_console_program_on_both_routes() {
        use nekominal_pty::{
            conpty::SpawnConfig,
            session::{Event, Sessions, Sink},
        };
        use std::{
            collections::BTreeMap,
            sync::mpsc::channel,
            time::{Duration, Instant},
        };

        let routes: Vec<(&str, Vec<u8>)> = vec![
            ("raw 0x03", vec![0x03]),
            (
                "win32 record",
                [true, false]
                    .iter()
                    .flat_map(|down| encode_win32_input(&record(0x43, 46, 3, LEFT_CTRL_PRESSED, *down)))
                    .collect(),
            ),
        ];
        for (name, bytes) in routes {
            let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
            let config = SpawnConfig {
                program: std::path::PathBuf::from(system_root).join("System32").join("cmd.exe"),
                args: ["/d", "/c", "ping -n 60 127.0.0.1 >nul"].iter().map(|s| s.to_string()).collect(),
                cwd: None,
                env: BTreeMap::new(),
                cols: 100,
                rows: 30,
                width_model: nekominal_pty::conpty::WidthModel::Wcswidth,
                inherit_environment: true,
            };
            let (tx, rx) = channel();
            let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
            let sessions = Sessions::default();
            let (id, _) = sessions.spawn(&config, sink).unwrap();
            let session = sessions.get(id).unwrap();
            std::thread::sleep(Duration::from_millis(1000));
            let started = Instant::now();
            session.write(&bytes).unwrap();
            let mut exit = None;
            while started.elapsed() < Duration::from_secs(8) {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Event::Data(data)) => {
                        if data.windows(3).any(|w| w == b"\x1b[c") {
                            let _ = session.write(b"\x1b[?1;2c");
                        }
                        session.ack(data.len());
                    }
                    Ok(Event::Exit(code)) => {
                        exit = Some(code);
                        break;
                    }
                    Err(_) => {}
                }
            }
            println!("Ctrl+C via {name}: exit {:?} after {} ms", exit, started.elapsed().as_millis());
            assert!(exit.is_some(), "Ctrl+C via {name} did not end the program");
            sessions.close(id);
        }
    }

    /// PowerShell 7 with PSReadLine, the Win32-console child that matters most: Shift+Enter
    /// (AddLine) only works through win32-input-mode; Ctrl+Space (MenuComplete) works both ways.
    #[test]
    #[ignore]
    fn psreadline_needs_the_win32_route_for_shift_enter() {
        use nekominal_pty::{
            conpty::SpawnConfig,
            profiles::resolve_program,
            session::{Event, Session, Sessions, Sink},
        };
        use std::{
            collections::BTreeMap,
            sync::mpsc::{channel, Receiver},
            time::{Duration, Instant},
        };

        let Some(pwsh) = resolve_program("pwsh") else {
            println!("pwsh not installed, skipped");
            return;
        };

        fn pump(rx: &Receiver<Event>, session: &Session, raw: &mut Vec<u8>, duration: Duration) {
            let deadline = Instant::now() + duration;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                if let Ok(Event::Data(data)) = rx.recv_timeout(left.min(Duration::from_millis(20))) {
                    if data.windows(3).any(|w| w == b"\x1b[c") {
                        let _ = session.write(b"\x1b[?1;2c");
                    }
                    if data.windows(4).any(|w| w == b"\x1b[6n") {
                        let _ = session.write(b"\x1b[1;1R");
                    }
                    session.ack(data.len());
                    raw.extend_from_slice(&data);
                }
            }
        }

        // Returns the output produced after the typed prefix and the key.
        let run = |typed: &str, key: &[u8], then: &[u8]| -> String {
            let config = SpawnConfig {
                program: pwsh.clone(),
                args: ["-NoLogo", "-NoProfile"].iter().map(|s| s.to_string()).collect(),
                cwd: None,
                env: BTreeMap::new(),
                cols: 120,
                rows: 30,
                width_model: nekominal_pty::conpty::WidthModel::Wcswidth,
                inherit_environment: true,
            };
            let (tx, rx) = channel();
            let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
            let sessions = Sessions::default();
            let (id, _) = sessions.spawn(&config, sink).unwrap();
            let session = sessions.get(id).unwrap();
            let mut raw = Vec::new();
            // Wait for the prompt and for PSReadLine to settle.
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(25) && !String::from_utf8_lossy(&raw).contains("PS ") {
                pump(&rx, &session, &mut raw, Duration::from_millis(100));
            }
            pump(&rx, &session, &mut raw, Duration::from_millis(1200));
            session.write(&encode_text(typed)).unwrap();
            pump(&rx, &session, &mut raw, Duration::from_millis(500));
            let mark = raw.len();
            session.write(key).unwrap();
            pump(&rx, &session, &mut raw, Duration::from_millis(700));
            session.write(then).unwrap();
            pump(&rx, &session, &mut raw, Duration::from_millis(1200));
            sessions.close(id);
            show(&raw[mark..])
        };

        let ctrl = LEFT_CTRL_PRESSED;
        let both = |vk, sc, uc, cs| {
            let mut bytes = encode_win32_input(&record(vk, sc, uc, cs, true));
            bytes.extend(encode_win32_input(&record(vk, sc, uc, cs, false)));
            bytes
        };

        // Ctrl+Space: MenuComplete lists the candidates for "Get-Ch".
        let none = run("Get-EventS", b"", b"");
        let legacy_ctrl_space = run("Get-EventS", b"\x00", b"");
        let win32_ctrl_space = run("Get-EventS", &both(0x20, 57, 0x20, ctrl), b"");
        let wanted = "Get-EventSubscriber";
        // The whole stream of the key assembler for the same keys, modifier records included.
        let assembled = |build: &dyn Fn(&mut super::typist::Rig)| {
            let mut typist = super::typist::Rig::us();
            build(&mut typist);
            typist.out.clone()
        };
        let assembled_ctrl_space =
            run("Get-EventS", &assembled(&|r| r.chord(&[super::typist::Mod::Ctrl], PhysicalKey::Space)), b"");
        println!("Ctrl+Space assembled  -> output: {assembled_ctrl_space}");
        assert!(assembled_ctrl_space.contains(wanted), "the assembled Ctrl+Space did not complete");
        println!("no key               -> output: {none}");
        println!("Ctrl+Space legacy NUL  -> output: {legacy_ctrl_space}");
        println!("Ctrl+Space win32 record -> output: {win32_ctrl_space}");
        assert!(!none.contains(wanted));
        // PSReadLine binds Ctrl+@ to MenuComplete as well, and the legacy NUL reaches it as
        // Ctrl+Shift+2: both routes work for this child (other Win32 programs need not alias them).
        assert!(legacy_ctrl_space.contains(wanted), "legacy NUL did not complete");
        assert!(win32_ctrl_space.contains(wanted), "win32 Ctrl+Space did not complete");

        // Shift+Enter: AddLine shows the continuation prompt.
        let legacy_shift_enter = run("echo A", b"\r", b"");
        let win32_shift_enter = run("echo A", &both(0x0d, 28, 13, SHIFT_PRESSED), b"");
        let continuation = |text: &str| text.contains(">> ");
        println!("Shift+Enter legacy CR   -> continuation prompt: {}", continuation(&legacy_shift_enter));
        println!("Shift+Enter win32 record -> continuation prompt: {}", continuation(&win32_shift_enter));
        assert!(!continuation(&legacy_shift_enter));
        assert!(continuation(&win32_shift_enter));
        let assembled_shift_enter =
            run("echo A", &assembled(&|r| r.chord(&[super::typist::Mod::Shift], PhysicalKey::Enter)), b"");
        println!("Shift+Enter assembled  -> continuation prompt: {}", continuation(&assembled_shift_enter));
        assert!(continuation(&assembled_shift_enter), "the assembled Shift+Enter did not add a line");
    }

    /// The streams the key assembler produces for typing, through real ConPTY children: a VT child
    /// (node) gets the legacy bytes conhost renders for the records, a Win32 console child gets the
    /// records themselves.
    #[test]
    #[ignore]
    fn assembled_streams_reach_vt_and_win32_children() {
        use super::typist::{Layout, Mod, Rig as Typist};
        use PhysicalKey::*;

        let stream = |layout: Layout, build: &dyn Fn(&mut Typist)| {
            let mut typist = Typist::new(layout);
            build(&mut typist);
            typist.out.clone()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("AltGr+E German", stream(Layout::german(), &|r| r.chord(&[Mod::RightAlt], KeyE))),
            (
                "dead ^ then e French",
                stream(Layout::french(), &|r| {
                    r.tap(BracketLeft);
                    r.tap(KeyE);
                }),
            ),
            ("Shift+Enter", stream(Layout::us(), &|r| r.chord(&[Mod::Shift], Enter))),
            ("Ctrl+C", stream(Layout::us(), &|r| r.chord(&[Mod::Ctrl], KeyC))),
            ("Alt+A", stream(Layout::us(), &|r| r.chord(&[Mod::Alt], KeyA))),
            ("Ctrl+Up", stream(Layout::us(), &|r| r.chord(&[Mod::Ctrl], ArrowUp))),
            ("typing", stream(Layout::us(), &|r| r.type_text("Hello, World!"))),
            ("emoji (VK_PACKET)", stream(Layout::us(), &|r| r.inject_text("😀"))),
        ];
        let steps = || cases.iter().map(|(name, bytes)| step(name, bytes.clone())).collect::<Vec<_>>();

        let node = run(Child::Node, b"", "assembled-node", steps());
        let got = |name: &str| show(&node.step(name).node_chunks().concat());
        assert_eq!(got("AltGr+E German"), "€");
        assert_eq!(got("dead ^ then e French"), "ê");
        assert_eq!(got("Shift+Enter"), r"\r");
        assert_eq!(got("Ctrl+C"), r"\x03");
        assert_eq!(got("Alt+A"), r"\ea");
        assert_eq!(got("Ctrl+Up"), r"\e[1;5A");
        assert_eq!(got("typing"), "Hello, World!");
        assert_eq!(got("emoji (VK_PACKET)"), "😀");

        let win = run(Child::Win32, b"", "assembled-win32", steps());
        let downs = |name: &str| win.step(name).key_records().into_iter().filter(|r| r.0).collect::<Vec<_>>();
        // (key down, virtual key, scan code, character, control state)
        let altgr = downs("AltGr+E German").into_iter().find(|r| r.1 == 0x45).expect("the E record");
        assert_eq!((altgr.3, altgr.4 & (LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED)), (0x20AC, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED));
        let circumflex = downs("dead ^ then e French").into_iter().find(|r| r.1 == 0x45).expect("the E record");
        assert_eq!((circumflex.3, circumflex.4), (0xEA, 0));
        let enter = downs("Shift+Enter").into_iter().find(|r| r.1 == 0x0D).expect("the Enter record");
        assert_eq!(enter.4 & SHIFT_PRESSED, SHIFT_PRESSED);
        let ctrl_c = downs("Ctrl+C").into_iter().find(|r| r.1 == 0x43).expect("the C record");
        assert_eq!((ctrl_c.3, ctrl_c.4 & LEFT_CTRL_PRESSED), (3, LEFT_CTRL_PRESSED));
        let typed: Vec<u16> = downs("typing").into_iter().filter(|r| r.3 != 0).map(|r| r.3).collect();
        assert_eq!(String::from_utf16_lossy(&typed), "Hello, World!");
    }

    #[test]
    #[ignore]
    fn conpty_requests_win32_input_mode_and_focus_at_startup() {
        let node = run(Child::Node, b"", "startup-node", vec![]);
        let text = show(&node.startup_output);
        assert!(text.contains(r"\e[?9001h") && text.contains(r"\e[?1004h"), "{text}");
        // A Win32 child that enables mouse input makes ConPTY ask for any-event SGR tracking.
        let win = run(Child::Win32, b"", "startup-win32", vec![]);
        assert!(show(&win.startup_output).contains(r"\e[?1003;1006h"), "{}", show(&win.startup_output));
    }
}

/// One key as the toolkit would report it: the description for our encoder and the Win32 record.
struct Case {
    name: String,
    input: nekominal_input::KeyInput,
    record: nekominal_input::Win32Key,
}

fn cases() -> Vec<Case> {
    use nekominal_input::win32::control_state::*;
    use nekominal_input::{Key, KeyInput, KeypadKey, Mods, Win32Key};

    let cs = |mods: Mods, enhanced: bool| {
        Win32Key::control_state_from_mods(mods) | if enhanced { ENHANCED_KEY } else { 0 }
    };
    let mut cases = Vec::new();
    let mut add = |name: String, key: Key, mods: Mods, text: Option<&str>, vk: u16, uc: u16, enhanced: bool| {
        let mut input = KeyInput::new(key).with_mods(mods);
        if let Some(text) = text {
            input = input.with_text(text);
        }
        let record = Win32Key {
            virtual_key: vk,
            scan_code: 0,
            unicode: uc,
            key_down: true,
            control_state: cs(mods, enhanced),
            repeat_count: 1,
        };
        cases.push(Case { name, input, record });
    };

    for (i, c) in ('a'..='z').enumerate() {
        let vk = c.to_ascii_uppercase() as u16;
        let ctrl_uc = i as u16 + 1;
        add(format!("{c}"), Key::Char(c), Mods::NONE, Some(&c.to_string()), vk, c as u16, false);
        add(format!("Shift+{c}"), Key::Char(c), Mods::SHIFT, Some(&c.to_ascii_uppercase().to_string()), vk, c.to_ascii_uppercase() as u16, false);
        add(format!("Ctrl+{c}"), Key::Char(c), Mods::CTRL, None, vk, ctrl_uc, false);
        add(format!("Ctrl+Shift+{c}"), Key::Char(c), Mods::CTRL | Mods::SHIFT, None, vk, ctrl_uc, false);
        add(format!("Alt+{c}"), Key::Char(c), Mods::ALT, None, vk, c as u16, false);
        add(format!("Alt+Shift+{c}"), Key::Char(c), Mods::ALT | Mods::SHIFT, None, vk, c.to_ascii_uppercase() as u16, false);
        add(format!("Ctrl+Alt+{c}"), Key::Char(c), Mods::CTRL | Mods::ALT, None, vk, 0, false);
    }
    // Punctuation with a control character on a US layout.
    add("Ctrl+[".into(), Key::Char('['), Mods::CTRL, None, 0xdb, 0x1b, false);
    add("Ctrl+\\".into(), Key::Char('\\'), Mods::CTRL, None, 0xdc, 0x1c, false);
    add("Ctrl+]".into(), Key::Char(']'), Mods::CTRL, None, 0xdd, 0x1d, false);
    add("Ctrl+Shift+6 (^)".into(), Key::Char('6'), Mods::CTRL | Mods::SHIFT, None, 0x36, 0x1e, false);
    add("Ctrl+Shift+- (_)".into(), Key::Char('-'), Mods::CTRL | Mods::SHIFT, None, 0xbd, 0x1f, false);
    for (c, vk) in [('1', 0x31u16), ('.', 0xbe), (',', 0xbc), (';', 0xba), ('=', 0xbb)] {
        add(format!("{c}"), Key::Char(c), Mods::NONE, Some(&c.to_string()), vk, c as u16, false);
    }
    // Text that is not ASCII.
    add("é (dead key result)".into(), Key::Char('e'), Mods::NONE, Some("é"), 0x45, 'é' as u16, false);
    add("你 (IME)".into(), Key::Unidentified, Mods::NONE, Some("你"), 0, '你' as u16, false);
    add("AltGr € ".into(), Key::Char('€'), Mods::NONE, Some("€"), 0x45, '€' as u16, false);

    // C0 keys.
    for (mods, label) in [
        (Mods::NONE, ""),
        (Mods::SHIFT, "Shift+"),
        (Mods::CTRL, "Ctrl+"),
        (Mods::ALT, "Alt+"),
        (Mods::CTRL | Mods::SHIFT, "Ctrl+Shift+"),
        (Mods::CTRL | Mods::ALT, "Ctrl+Alt+"),
        (Mods::ALT | Mods::SHIFT, "Alt+Shift+"),
    ] {
        let ctrl = mods.contains(Mods::CTRL);
        add(format!("{label}Enter"), Key::Enter, mods, None, 0x0d, if ctrl { 10 } else { 13 }, false);
        add(format!("{label}Tab"), Key::Tab, mods, None, 0x09, 9, false);
        add(format!("{label}Backspace"), Key::Backspace, mods, None, 0x08, if ctrl { 0x7f } else { 8 }, false);
        add(format!("{label}Escape"), Key::Escape, mods, None, 0x1b, 27, false);
    }
    add("Space".into(), Key::Char(' '), Mods::NONE, Some(" "), 0x20, 0x20, false);
    add("Alt+Space".into(), Key::Char(' '), Mods::ALT, None, 0x20, 0x20, false);

    // Navigation, editing and function keys with every modifier combination Windows reports.
    let mods_set = [
        (Mods::NONE, ""),
        (Mods::SHIFT, "Shift+"),
        (Mods::ALT, "Alt+"),
        (Mods::CTRL, "Ctrl+"),
        (Mods::CTRL | Mods::SHIFT, "Ctrl+Shift+"),
        (Mods::CTRL | Mods::ALT, "Ctrl+Alt+"),
        (Mods::ALT | Mods::SHIFT, "Alt+Shift+"),
        (Mods::CTRL | Mods::ALT | Mods::SHIFT, "Ctrl+Alt+Shift+"),
    ];
    let nav = [
        (Key::Up, 0x26u16, "Up"),
        (Key::Down, 0x28, "Down"),
        (Key::Left, 0x25, "Left"),
        (Key::Right, 0x27, "Right"),
        (Key::Home, 0x24, "Home"),
        (Key::End, 0x23, "End"),
        (Key::Insert, 0x2d, "Insert"),
        (Key::Delete, 0x2e, "Delete"),
        (Key::PageUp, 0x21, "PageUp"),
        (Key::PageDown, 0x22, "PageDown"),
    ];
    for (mods, label) in mods_set {
        for (key, vk, name) in nav {
            add(format!("{label}{name}"), key, mods, None, vk, 0, true);
        }
        for n in 1..=20u8 {
            add(format!("{label}F{n}"), Key::F(n), mods, None, 0x6f + u16::from(n), 0, false);
        }
    }
    // The navigation half of the keypad (NumLock off): VK_LEFT etc. without ENHANCED_KEY.
    add("KP Left".into(), Key::Keypad(KeypadKey::Left), Mods::NONE, None, 0x25, 0, false);
    add("KP Home".into(), Key::Keypad(KeypadKey::Home), Mods::NONE, None, 0x24, 0, false);
    add("KP Delete".into(), Key::Keypad(KeypadKey::Delete), Mods::NONE, None, 0x2e, 0, false);
    add("KP Begin".into(), Key::Keypad(KeypadKey::Begin), Mods::NONE, None, 0x0c, 0, false);
    add("KP Enter".into(), Key::Keypad(KeypadKey::Enter), Mods::NONE, None, 0x0d, 13, true);
    add("KP 5 (NumLock)".into(), Key::Keypad(KeypadKey::Digit(5)), Mods::NONE, Some("5"), 0x65, '5' as u16, false);
    add("KP + (NumLock)".into(), Key::Keypad(KeypadKey::Add), Mods::NONE, Some("+"), 0x6b, '+' as u16, false);
    // Keys with no legacy VT form.
    add("Pause".into(), Key::Pause, Mods::NONE, None, 0x13, 0, false);
    add("Menu".into(), Key::Menu, Mods::NONE, None, 0x5d, 0, true);
    add("F21".into(), Key::F(21), Mods::NONE, None, 0x84, 0, false);
    add("F24".into(), Key::F(24), Mods::NONE, None, 0x87, 0, false);
    add("Ctrl+Space".into(), Key::Char(' '), Mods::CTRL, None, 0x20, 0x20, false);
    cases
}

/// The key assembler must produce, for every key of the OpenConsole differential below, the record
/// whose rendering by OpenConsole matched our legacy encoder: the same virtual key, character and
/// control state. Needs no ConPTY, so it runs with the unit tests.
#[test]
fn assembler_records_equal_the_records_proven_against_openconsole() {
    use nekominal_input::{win32::control_state::ENHANCED_KEY, InputModes, KeyAssembler, KeyStroke, KeyboardEvent, Mods};
    use typist::{parse_stream, Item};

    let modes = InputModes::default();
    let mut checked = 0;
    for case in cases() {
        // The AltGr row of the table has no Ctrl and Alt in its control state, the IME row no key.
        if case.name.starts_with("AltGr") || case.record.virtual_key == 0 {
            continue;
        }
        let mut asm = KeyAssembler::new();
        let mut out = Vec::new();
        let mut feed = |event: KeyboardEvent| out.extend(asm.handle(event, &modes).bytes);

        let mods = case.input.mods;
        let mut held = Mods::NONE;
        for (bit, vk, scan) in [(Mods::SHIFT, 0x10, 0x2A), (Mods::CTRL, 0x11, 0x1D), (Mods::ALT, 0x12, 0x38)] {
            if mods.contains(bit) {
                held |= bit;
                feed(KeyboardEvent::KeyDown(KeyStroke::new(vk, scan, false, held)));
            }
        }
        let extended = case.record.control_state & ENHANCED_KEY != 0;
        let stroke = KeyStroke::new(i32::from(case.record.virtual_key), 0, extended, mods);
        feed(KeyboardEvent::KeyDown(stroke));
        for unit in case.input.text.iter().flat_map(|text| text.encode_utf16()) {
            feed(KeyboardEvent::CharacterReceived(unit));
        }
        feed(KeyboardEvent::Flush);

        let records: Vec<_> = parse_stream(&out)
            .into_iter()
            .filter_map(|item| match item {
                Item::Record(rec) if rec.down && rec.vk == case.record.virtual_key => Some(rec),
                _ => None,
            })
            .collect();
        assert_eq!(records.len(), 1, "{}: {records:?}", case.name);
        assert_eq!(
            (records[0].unicode, records[0].control_state),
            (case.record.unicode, case.record.control_state),
            "{}",
            case.name
        );
        checked += 1;
    }
    // 477 cases, minus the AltGr row and the IME row.
    assert_eq!(checked, 475);
}

/// Our legacy encoder against OpenConsole's own `TerminalInput`: the same key as a Win32 record
/// goes in, conhost renders the legacy bytes for a VT-input child (node), and they are compared
/// with what `encode_key` produces for the same key.
#[test]
#[ignore]
fn legacy_encoder_against_openconsole() {
    use nekominal_input::{encode_key, win32::encode_win32_input, InputModes};
    use std::time::Duration;

    let modes = InputModes::default();
    let mut rig = Rig::spawn(Child::Node, b"", "differential");
    assert!(rig.wait_for(b"READY", Duration::from_secs(20)));
    rig.pump(Duration::from_millis(150));

    let mut agree = 0;
    let mut both_nothing = Vec::new();
    let mut disagree = Vec::new();
    let all = cases();
    let total = all.len();
    for case in all {
        let mut down = case.record;
        down.key_down = true;
        let mut up = case.record;
        up.key_down = false;
        let mut bytes = encode_win32_input(&down);
        bytes.extend(encode_win32_input(&up));
        let lines = rig.send(&bytes, Duration::from_millis(400), Duration::from_millis(40));
        let step = StepResult { label: case.name.clone(), sent: bytes, lines };
        let theirs: Vec<u8> = step.node_chunks().concat();
        let ours = encode_key(&case.input, &modes).unwrap_or_default();
        if ours == theirs {
            agree += 1;
            if ours.is_empty() {
                both_nothing.push(case.name.clone());
            }
        } else {
            disagree.push(format!("  {:<22} ours \"{}\"  openconsole \"{}\"", case.name, show(&ours), show(&theirs)));
        }
    }
    let report = format!(
        "legacy encoder vs OpenConsole (Win32 record -> VT bytes at a node child): {agree} of {total} agree ({} of them both empty: {:?}), {} differ\n{}\n",
        both_nothing.len(),
        both_nothing,
        disagree.len(),
        disagree.join("\n")
    );
    print!("{report}");
    if let Ok(path) = std::env::var("INPUT_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    // Known divergences, all on keys that conhost cannot render for a VT child.
    let unexpected: Vec<&String> = disagree
        .iter()
        .filter(|line| {
            !["Ctrl+Space", "Menu", "Pause", "F21", "F24", "Ctrl+Escape", "Alt+Escape", "Ctrl+Shift+Escape", "Ctrl+Alt+Escape", "Alt+Shift+Escape"]
                .iter()
                .any(|known| line.trim_start().starts_with(&format!("{known} ")))
        })
        .collect();
    assert!(unexpected.is_empty(), "unexpected differences:\n{}", unexpected.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// Writes `bytes` to a child in one go while pumping output, then compares what arrived.
fn paste_round_trip(child: Child, label: &str, bytes: &[u8], setup: &[u8]) -> (bool, std::time::Duration, usize) {
    use std::time::{Duration, Instant};

    let mut rig = Rig::spawn(child, setup, label);
    assert!(rig.wait_for(b"READY", Duration::from_secs(20)));
    rig.pump(Duration::from_millis(100));
    let session = rig.session.clone();
    let payload = bytes.to_vec();
    let started = Instant::now();
    let writer = std::thread::spawn(move || session.write(&payload).unwrap());

    let received = |rig: &Rig| -> Vec<u8> {
        let mut data = Vec::new();
        for line in rig.log_lines() {
            if let Some(rest) = line.strip_prefix("DATA ") {
                data.extend(unhex(rest.split(' ').nth(1).unwrap_or("")));
            } else if let Some(rest) = line.strip_prefix("S ") {
                data.extend(unhex(rest));
            }
        }
        data
    };
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        rig.pump(Duration::from_millis(100));
        got = received(&rig);
        if got.len() >= bytes.len() {
            break;
        }
    }
    let elapsed = started.elapsed();
    writer.join().unwrap();
    (got == bytes, elapsed, got.len())
}

#[test]
#[ignore]
fn large_paste() {
    let unit = "貼上的文字 line 你好 😀 ünï\r";
    let mut report = String::new();
    for (child, label, copies) in [
        (Child::Node, "node-4k", 150usize),
        (Child::Node, "node-64k", 2_400),
        (Child::Node, "node-1m", 38_000),
        (Child::VtInputQuiet, "vtq-64k", 2_400),
        (Child::VtInputQuiet, "vtq-1m", 38_000),
        (Child::Win32Quiet, "winq-64k", 2_400),
    ] {
        let text = unit.repeat(copies);
        let bytes = text.as_bytes().to_vec();
        let (identical, elapsed, got) = paste_round_trip(child, label, &bytes, b"");
        let line = format!(
            "{label:<10} sent {:>8} bytes, received {:>8}, identical={identical}, {:.0} ms\n",
            bytes.len(),
            got,
            elapsed.as_secs_f64() * 1000.0
        );
        print!("{line}");
        report.push_str(&line);
    }
    if let Ok(path) = std::env::var("INPUT_REPORT") {
        std::fs::write(path, report).unwrap();
    }
}
