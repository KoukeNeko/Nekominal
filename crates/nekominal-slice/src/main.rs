//! Vertical slice on the proposed crate layout:
//!   nekominal-pty -> nekominal-term -> nekominal-pane (host, view adapter, PaneRenderer) -> nekominal-render
//!   nekominal-winui (bootstrap, PanelSurface, ImeHost) over nekominal-winui-sys (generated WinUI 3 bindings).
//!
//! Environment knobs (all optional):
//!   NEKOMINAL_LOG=<file>    append a timestamped log
//!   SLICE_OPACITY=<0..1>    < 1 selects PreMultiplied alpha + translucent theme (SwapChainPanel transparency probe)
//!   SLICE_MATRIX=always|once|never   when IDXGISwapChain2::SetMatrixTransform(1/scale) is applied
//!   SLICE_PROGRAM=<exe>     default pwsh.exe, falling back to cmd.exe
//!   SLICE_STDIN=<text>      bytes written to the PTY after SLICE_STDIN_MS (escapes: \r \n \e \t)
//!   SLICE_DUMP_MS=<ms>      log the visible text of the terminal after this delay
//!   SLICE_EXIT_MS=<ms>      exit the process after this delay
// The bindings keep the Windows metadata names (`SetXxx`, `IFoo`, enum constants).
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use nekominal_input::{KeyEvent, KeyInput, Mods, vk};
use nekominal_pane::{Pane, PaneRenderer};
use nekominal_render as r;
use nekominal_winui::{
    app,
    diag::log,
    ime::{self, ImeHost},
    surface::{FORMAT, MatrixMode, PanelSurface, Presented, SurfaceOptions},
    window,
};
use nekominal_winui_sys::*;
use windows_core::*;

struct Terminal {
    surface: PanelSurface,
    renderer: Option<PaneRenderer>,
    pane: Rc<Pane>,
    ime: Option<Rc<ImeHost>>,
    grid: (usize, usize),
    opacity: f32,
    frames: u64,
    started: Instant,
}

impl Terminal {
    fn on_metrics(&mut self, width_dip: f64, height_dip: f64) {
        let metrics = match self.surface.on_metrics(width_dip, height_dip) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => {
                log(&format!("FATAL surface: {e}"));
                return;
            }
        };
        if self.renderer.is_none() {
            let Some(gpu) = self.surface.gpu() else { return };
            let mut theme = r::Theme::campbell();
            theme.background_opacity = self.opacity;
            let t = Instant::now();
            let config = r::RendererConfig { scale: metrics.scale.0, lcd: self.opacity >= 1.0, ..Default::default() };
            match PaneRenderer::new(&gpu.device, &gpu.queue, FORMAT, config, theme) {
                Ok(renderer) => {
                    log(&format!(
                        "renderer ready in {:.1} ms, cell {}x{}, lcd_active={}",
                        t.elapsed().as_secs_f64() * 1000.0,
                        renderer.metrics().width,
                        renderer.metrics().height,
                        renderer.lcd_active()
                    ));
                    self.renderer = Some(renderer);
                }
                Err(e) => {
                    log(&format!("FATAL renderer: {e}"));
                    return;
                }
            }
        } else if metrics.scale_changed
            && let Some(renderer) = self.renderer.as_mut()
            && let Err(e) = renderer.set_scale(metrics.scale.0)
        {
            log(&format!("set_scale failed: {e}"));
        }
        let Some(renderer) = self.renderer.as_mut() else { return };
        let grid = renderer.grid_for(metrics.size_px);
        if grid != self.grid {
            let m = renderer.metrics();
            log(&format!("grid {}x{} (cell {}x{} px)", grid.0, grid.1, m.width, m.height));
            self.grid = grid;
            self.pane.resize(grid.0 as u16, grid.1 as u16);
        }
        renderer.invalidate();
    }

    /// One `CompositionTarget.Rendering` tick (vsync paced).
    fn tick(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        renderer.set_preedit(self.ime.as_ref().and_then(|ime| ime.preedit()));
        let prepared = renderer.prepare(&self.pane.term);
        if !prepared.needs_draw {
            return;
        }
        let outcome = self.surface.present(|device, queue, view, size| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            renderer.draw(&mut encoder, view, size);
            queue.submit([encoder.finish()]);
        });
        match outcome {
            Presented::Drawn => {
                self.frames += 1;
                if self.frames == 1 {
                    log(&format!("first frame presented at +{:.0} ms", self.started.elapsed().as_secs_f64() * 1000.0));
                }
                if self.frames <= 3 || self.frames.is_multiple_of(120) {
                    log(&format!(
                        "frame {} rows_rebuilt={}/{} update {:.0} us",
                        self.frames,
                        prepared.rows_rebuilt,
                        prepared.rows_total,
                        prepared.update.as_secs_f64() * 1e6
                    ));
                }
            }
            Presented::Reconfigured => renderer.invalidate(),
            Presented::Skipped => {}
        }
    }
}

struct Ui {
    _window: Window,
    _terminal: Rc<RefCell<Terminal>>,
    _pane: Rc<Pane>,
    _revokers: Vec<EventRevoker>,
    _rendering: EventRevoker,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn mods_now() -> Mods {
    let down = |code: i32| {
        InputKeyboardSource::GetKeyStateForCurrentThread(VirtualKey(code)).map(|s| (s.0 & 1) != 0).unwrap_or(false)
    };
    let mut m = Mods::NONE;
    if down(0x10) {
        m |= Mods::SHIFT;
    }
    if down(0x11) {
        m |= Mods::CTRL;
    }
    if down(0x12) {
        m |= Mods::ALT;
    }
    m
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn unescape(s: &str) -> Vec<u8> {
    s.replace("\\r", "\r").replace("\\n", "\n").replace("\\e", "\x1b").replace("\\t", "\t").into_bytes()
}

fn main() {
    nekominal_winui::diag::init();
    log("main start");
    let result = app::run(|| match build_ui() {
        Ok(ui) => {
            UI.with(|u| *u.borrow_mut() = Some(ui));
            Ok(())
        }
        Err(e) => {
            log(&format!("FATAL build_ui: {e}"));
            Err(e)
        }
    });
    log(&format!("app::run returned {result:?}"));
}

fn build_ui() -> Result<Ui> {
    let window = Window::new()?;
    window.SetTitle("Nekominal vertical slice")?;
    window.SetExtendsContentIntoTitleBar(true)?;
    let mica = MicaBackdrop::new()?;
    mica.SetKind(MicaKind::Base)?;
    window.cast::<IWindow2>()?.SetSystemBackdrop(&mica.cast::<SystemBackdrop>()?)?;

    // Root: row 0 = 48 DIP title bar (drag region), row 1 = terminal host.
    let root = Grid::new()?;
    let rows = root.RowDefinitions()?;
    for unit in [GridUnitType::Auto, GridUnitType::Star] {
        let row = RowDefinition::new()?;
        row.SetHeight(GridLength { value: 1.0, grid_unit_type: unit })?;
        rows.Append(&row)?;
    }
    let children = root.cast::<Panel>()?.Children()?;
    let drag = Grid::new()?;
    drag.cast::<FrameworkElement>()?.SetHeight(48.0)?;
    let transparent = SolidColorBrush::new()?;
    transparent.SetColor(Color { a: 0, r: 0, g: 0, b: 0 })?;
    drag.cast::<Panel>()?.SetBackground(&transparent)?;
    let title = TextBlock::new()?;
    title.SetText("Nekominal  (nekominal-pty + term + pane + render, nekominal-winui over generated bindings)")?;
    let title_fe: FrameworkElement = title.cast()?;
    title_fe.SetMargin(Thickness { left: 16.0, top: 0.0, right: 0.0, bottom: 0.0 })?;
    title_fe.SetVerticalAlignment(VerticalAlignment::Center)?;
    drag.cast::<Panel>()?.Children()?.Append(&title.cast::<UIElement>()?)?;
    Grid::SetRow(&drag.cast::<FrameworkElement>()?, 0)?;
    children.Append(&drag.cast::<UIElement>()?)?;
    window.SetTitleBar(&drag.cast::<UIElement>()?)?;

    // Terminal host: SwapChainPanel is a Grid and cannot take focus, so a focusable ContentControl wraps it.
    let panel = SwapChainPanel::new()?;
    let panel_fe: FrameworkElement = panel.cast()?;
    panel_fe.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
    panel_fe.SetVerticalAlignment(VerticalAlignment::Stretch)?;
    let host = ContentControl::new()?;
    let host_ui: UIElement = host.cast()?;
    host_ui.SetIsTabStop(true)?;
    host.SetContent(&panel)?;
    let host_control: Control = host.cast()?;
    host_control.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;
    host_control.SetVerticalContentAlignment(VerticalAlignment::Stretch)?;
    let host_fe: FrameworkElement = host.cast()?;
    host_fe.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
    host_fe.SetVerticalAlignment(VerticalAlignment::Stretch)?;
    Grid::SetRow(&host_fe, 1)?;
    children.Append(&host_ui)?;

    // Pane: ConPTY + emulator. Initial 80x24; the first panel metrics resize it to the real grid.
    let program = std::env::var_os("SLICE_PROGRAM")
        .map(std::path::PathBuf::from)
        .or_else(|| nekominal_pty::profiles::resolve_program("pwsh"))
        .unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into());
    let args = if program.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("pwsh")) {
        vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()]
    } else {
        Vec::new()
    };
    log(&format!("spawning {} {args:?}", program.display()));
    let pane = Rc::new(Pane::spawn(program, args, 80, 24).map_err(|e| Error::new(HRESULT(-1), format!("pty: {e}")))?);

    let opacity = env_f32("SLICE_OPACITY", 1.0);
    let matrix = match std::env::var("SLICE_MATRIX").as_deref() {
        Ok("once") => MatrixMode::Once,
        Ok("never") => MatrixMode::Never,
        _ => MatrixMode::Always,
    };
    log(&format!("opacity {opacity} matrix {matrix:?}"));
    let terminal = Rc::new(RefCell::new(Terminal {
        surface: PanelSurface::new(
            panel.clone(),
            SurfaceOptions { premultiplied: opacity < 1.0, matrix, ..Default::default() },
        ),
        renderer: None,
        pane: pane.clone(),
        ime: None,
        grid: (0, 0),
        opacity,
        frames: 0,
        started: Instant::now(),
    }));

    let mut revokers = Vec::new();
    revokers.push(window.Closed({
        let pane = pane.clone();
        move |_, _| {
            log("window closed: shutting the pane down");
            pane.shutdown();
        }
    })?);
    revokers.push(panel_fe.SizeChanged({
        let terminal = terminal.clone();
        move |_, args| {
            let Some(args) = args.as_ref() else { return };
            let Ok(size) = args.NewSize() else { return };
            if let Ok(mut t) = terminal.try_borrow_mut() {
                t.on_metrics(f64::from(size.width), f64::from(size.height));
            }
        }
    })?);
    revokers.push(panel.CompositionScaleChanged({
        let (terminal, panel) = (terminal.clone(), panel.clone());
        move |_, _| {
            let Ok(fe) = panel.cast::<FrameworkElement>() else { return };
            if let Ok(mut t) = terminal.try_borrow_mut() {
                t.on_metrics(fe.ActualWidth().unwrap_or(0.0), fe.ActualHeight().unwrap_or(0.0));
            }
        }
    })?);

    // IME: CoreTextEditContext on the host; commits go to the PTY, the preedit is drawn by the renderer.
    let root_fe: FrameworkElement = root.cast()?;
    let ime = Rc::new(ImeHost::new(
        {
            let (terminal, window, root_fe, panel_fe) = (terminal.clone(), window.clone(), root_fe.clone(), panel_fe.clone());
            move || {
                let t = terminal.try_borrow().ok()?;
                let (cx, cy, cw, ch) = t.renderer.as_ref()?.cursor_cell_px(&t.pane.term);
                let (x, y, w, h) = window::element_screen_rect(&window, &root_fe, &panel_fe)?;
                Some(ime::Layout { cursor_cell: (x + cx, y + cy, cw, ch), pane: (x, y, w, h) })
            }
        },
        {
            let pane = pane.clone();
            move |text| pane.send_text(&text)
        },
    )?);
    terminal.borrow_mut().ime = Some(ime.clone());
    revokers.push(host_ui.GotFocus({
        let ime = ime.clone();
        move |_, _| ime.focus_enter()
    })?);
    revokers.push(host_ui.LostFocus({
        let ime = ime.clone();
        move |_, _| ime.focus_leave()
    })?);

    // Keyboard: non-text keys and Ctrl/Alt chords from PreviewKeyDown, text from CharacterReceived.
    revokers.push(host_ui.PreviewKeyDown({
        let pane = pane.clone();
        move |_, args| {
            let Some(a) = args.as_ref() else { return };
            let code = a.Key().map_or(0, |k| k.0);
            let status = a.KeyStatus().unwrap_or_default();
            if code == vk::VK_PROCESSKEY || code == vk::VK_PACKET {
                return;
            }
            let mods = mods_now();
            let Some(key) = vk::key_for_vk(code, status.is_extended_key, mods) else { return };
            let event = if status.was_key_down { KeyEvent::Repeat } else { KeyEvent::Press };
            let sent = pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(event));
            log(&format!("key vk=0x{code:02X} {mods:?} sent={sent}"));
            let _ = a.SetHandled(true);
        }
    })?);
    // CharacterReceived delivers UTF-16 code units: an astral character arrives as two events.
    let utf16 = RefCell::new(nekominal_input::Utf16Assembler::default());
    revokers.push(host_ui.CharacterReceived({
        let pane = pane.clone();
        move |_, args| {
            let Some(a) = args.as_ref() else { return };
            if let Some(ch) = utf16.borrow_mut().push(a.Character().unwrap_or(0)) {
                log(&format!("char U+{:04X}", ch as u32));
                pane.send_text(&ch.to_string());
            }
            let _ = a.SetHandled(true);
        }
    })?);
    revokers.push(host_ui.PointerPressed({
        let host = host_ui.clone();
        move |_, _| {
            let _ = host.Focus(FocusState::Pointer);
        }
    })?);
    revokers.push(host_fe.Loaded({
        let host = host_ui.clone();
        move |_, _| {
            let ok = host.Focus(FocusState::Programmatic);
            log(&format!("host focus = {ok:?}"));
        }
    })?);

    // One vsync-paced tick drives snapshot -> update -> draw -> present.
    let rendering = CompositionTarget::Rendering({
        let terminal = terminal.clone();
        move |_, _| {
            if let Ok(mut t) = terminal.try_borrow_mut() {
                t.tick();
            }
        }
    })?;

    window.SetContent(&root)?;
    window.Activate()?;

    // Probe helpers.
    if let Ok(text) = std::env::var("SLICE_STDIN") {
        let delay: u64 = std::env::var("SLICE_STDIN_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2500);
        let tx = pane.sender();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay));
            let _ = tx.send(unescape(&text));
        });
    }
    if let Some(ms) = std::env::var("SLICE_DUMP_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        let term = pane.term.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            let mut snapshot = nekominal_term::Snapshot::default();
            term.lock().snapshot(&mut snapshot);
            log(&format!("TEXT DUMP {}x{}:\n{}", snapshot.columns, snapshot.rows, snapshot.text()));
        });
    }
    if let Some(ms) = std::env::var("SLICE_EXIT_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            log("exit requested");
            std::process::exit(0);
        });
    }
    log("ui built");
    Ok(Ui { _window: window, _terminal: terminal, _pane: pane, _revokers: revokers, _rendering: rendering })
}
