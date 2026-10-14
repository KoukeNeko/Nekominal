//! One terminal pane on screen: a focusable host control around a `SwapChainPanel`, the ConPTY session and
//! emulator behind it (`nekominal_pane::Pane`), the renderer that draws into the panel, and the IME endpoint.
//!
//! The pane knows nothing about tabs or layout: the shell positions `host`, tells it about focus and drives
//! `tick` from `CompositionTarget.Rendering`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use nekominal_input::Utf16Assembler;
use nekominal_input::{KeyEvent, KeyInput, MouseButton, PointerEvent, PointerGrid, PointerKind, vk};
use nekominal_model::{layout::PaneId, strings::Text};
use nekominal_pane::{Pane, PaneRenderer, UiEvent};
use nekominal_pty::conpty::SpawnConfig;
use nekominal_render as r;
use nekominal_term::CoreConfig;
use nekominal_winui::{
    ime::{self, ImeHost},
    surface::{FORMAT, MatrixMode, PanelSurface, Presented, SurfaceOptions},
    window,
};
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};

use crate::i18n::t;
use crate::keyboard;
use crate::logic::{Attention, Reported};
use crate::terminal_control::{self, TextModel};
use crate::util::{log, ui};

/// How the terminals look right now: shared by every pane, changed by the settings and the zoom commands.
pub struct Look {
    pub theme: RefCell<r::Theme>,
    pub font_pt: Cell<f32>,
    pub line_height: Cell<f32>,
    pub families: RefCell<Vec<String>>,
}

impl Look {
    fn renderer_config(&self, scale: f32) -> r::RendererConfig {
        let mut fonts = r::fonts::FontConfig::default();
        let families = self.families.borrow();
        if !families.is_empty() {
            fonts.text = families.clone();
        }
        r::RendererConfig {
            fonts,
            font_size_pt: self.font_pt.get(),
            scale,
            line_height: self.line_height.get(),
            lcd: self.theme.borrow().background_opacity >= 1.0,
            ..Default::default()
        }
    }
}

/// What a pane asks of the shell when the pointer does something the pane cannot finish alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneRequest {
    /// A gesture ended with text selected (copy on select).
    Selected,
    /// The right button went down where the terminal owns the pointer; the position is in DIP from the pane's
    /// top-left corner.
    RightClick { x: f64, y: f64 },
}

/// Time between two steps of the scrolling a selection drag held outside the pane asks for.
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(50);

/// The buttons the pointer can have down, in the order of [`PaneView::buttons`].
const BUTTONS: [MouseButton; 3] = [MouseButton::Left, MouseButton::Middle, MouseButton::Right];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Press,
    Move,
    Release,
}

struct Render {
    surface: PanelSurface,
    renderer: Option<PaneRenderer>,
    /// Physical pixels per DIP of the panel.
    scale: f32,
    grid: (usize, usize),
    frames: u64,
    dimmed: bool,
    last_size_dip: (f64, f64),
    last_text_update: Instant,
    /// The screen changed since the text for UI Automation was refreshed.
    text_stale: bool,
}

/// How often the text exposed to UI Automation is refreshed while the screen changes.
const TEXT_MODEL_INTERVAL: Duration = Duration::from_millis(100);

pub struct PaneView {
    pub id: PaneId,
    pub host: UserControl,
    pub panel: SwapChainPanel,
    pub pane: Pane,
    /// The title the program set (OSC 0 or 2), cleaned; empty until it sets one.
    pub title: RefCell<String>,
    /// The file name of the program without `.exe` (`pwsh`): what a shell titles itself with before it settles.
    pub shell_name: String,
    /// The profile the session was started from, so a split or a duplicate starts the same shell.
    pub profile_id: String,
    /// Working directory the shell last reported.
    pub cwd: RefCell<Option<PathBuf>>,
    /// The program behind the pane ended (its exit code), so the pane no longer counts as running.
    pub exited: Cell<Option<u32>>,
    /// Why the pane wants attention without an agent saying so (a bell, a notification, a failed exit) until the
    /// user has seen it.
    pub attention: Cell<Option<Attention>>,
    /// What the notification said.
    pub note: RefCell<Option<String>>,
    /// The pane waits for the user (`logic::needs_response`): while it is not the focused one of its tab it has a
    /// warning line.
    pub needs_response: Cell<bool>,
    /// Progress the program reports with OSC 9;4.
    pub progress: Cell<Option<Reported>>,
    look: Rc<Look>,
    render: RefCell<Render>,
    on_request: Rc<dyn Fn(PaneId, PaneRequest)>,
    /// Which of the left, middle and right buttons are down, as the last pointer event saw them.
    buttons: Cell<[bool; 3]>,
    last_drag_scroll: Cell<Instant>,
    ime: Option<ImeHost>,
    text_model: Rc<TextModel>,
    window: Window,
    root: FrameworkElement,
    revokers: RefCell<Vec<EventRevoker>>,
}

/// Quoted the way PowerShell and cmd want a path with spaces.
pub fn quote_path(path: &str) -> String {
    if path.contains(' ') { format!("\"{path}\"") } else { path.to_owned() }
}

impl PaneView {
    /// Creates the host and starts the session. `on_focus(id, focused)` reports keyboard focus entering and
    /// leaving the host.
    pub fn new(
        id: PaneId,
        config: SpawnConfig,
        core: CoreConfig,
        profile_id: String,
        look: Rc<Look>,
        window: &Window,
        root: &FrameworkElement,
        on_focus: Rc<dyn Fn(PaneId, bool)>,
        on_request: Rc<dyn Fn(PaneId, PaneRequest)>,
    ) -> Result<Rc<PaneView>> {
        let panel = SwapChainPanel::new()?;
        let panel_fe: FrameworkElement = panel.cast()?;
        panel_fe.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
        panel_fe.SetVerticalAlignment(VerticalAlignment::Stretch)?;

        let text_model = Rc::new(TextModel::default());
        let host = terminal_control::create(t(Text::TabUntitled), text_model.clone())?;
        host.SetContent(&panel)?;
        let host_control: Control = host.cast()?;
        host_control.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;
        host_control.SetVerticalContentAlignment(VerticalAlignment::Stretch)?;
        let host_fe: FrameworkElement = host.cast()?;
        host_fe.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
        host_fe.SetVerticalAlignment(VerticalAlignment::Stretch)?;
        let host_ui: UIElement = host.cast()?;
        host_ui.SetIsTabStop(true)?;
        host_ui.SetUseSystemFocusVisuals(false)?;

        let shell_name = config.program.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let pane = Pane::spawn_with(config, core)
            .map_err(|e| windows_core::Error::new(windows_core::HRESULT(-1), format!("pty: {e}")))?;
        let surface = PanelSurface::new(
            panel.clone(),
            SurfaceOptions { premultiplied: true, matrix: MatrixMode::Always, ..Default::default() },
        );

        let view = Rc::new_cyclic(|weak: &Weak<PaneView>| {
            let ime = ImeHost::new(
                {
                    let weak = weak.clone();
                    move || weak.upgrade().and_then(|view| view.ime_layout())
                },
                {
                    let weak = weak.clone();
                    move |text| {
                        if let Some(view) = weak.upgrade() {
                            view.pane.send_text(&text);
                        }
                    }
                },
            )
            .map_err(|e| log(&format!("IME unavailable for pane {id}: {e}")))
            .ok();
            PaneView {
                id,
                host,
                panel,
                pane,
                title: RefCell::new(String::new()),
                shell_name,
                profile_id,
                cwd: RefCell::new(None),
                exited: Cell::new(None),
                attention: Cell::new(None),
                note: RefCell::new(None),
                needs_response: Cell::new(false),
                progress: Cell::new(None),
                look,
                render: RefCell::new(Render {
                    surface,
                    renderer: None,
                    scale: 1.0,
                    grid: (0, 0),
                    frames: 0,
                    dimmed: false,
                    last_size_dip: (0.0, 0.0),
                    last_text_update: Instant::now(),
                    text_stale: false,
                }),
                on_request,
                buttons: Cell::new([false; 3]),
                last_drag_scroll: Cell::new(Instant::now()),
                ime,
                text_model,
                window: window.clone(),
                root: root.clone(),
                revokers: RefCell::new(Vec::new()),
            }
        });
        view.attach_events(&host_ui, &panel_fe, on_focus)?;
        Ok(view)
    }

    fn attach_events(
        self: &Rc<Self>,
        host: &UIElement,
        panel: &FrameworkElement,
        on_focus: Rc<dyn Fn(PaneId, bool)>,
    ) -> Result<()> {
        let mut revokers = Vec::new();
        let weak = Rc::downgrade(self);

        revokers.push(panel.SizeChanged({
            let weak = weak.clone();
            cb!("pane size", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                if let Ok(size) = args.NewSize() {
                    view.on_size(f64::from(size.width), f64::from(size.height));
                }
            })
        })?);
        revokers.push(self.panel.CompositionScaleChanged({
            let weak = weak.clone();
            cb!("pane scale", |_, _| {
                let Some(view) = weak.upgrade() else { return };
                let (width, height) = (panel_size(&view.panel).0, panel_size(&view.panel).1);
                view.on_size(width, height);
            })
        })?);

        revokers.push(host.GotFocus({
            let (weak, on_focus) = (weak.clone(), on_focus.clone());
            cb!("pane got focus", |_, _| {
                let Some(view) = weak.upgrade() else { return };
                if let Some(ime) = &view.ime {
                    ime.focus_enter();
                }
                view.pane.set_focused(true);
                on_focus(view.id, true);
            })
        })?);
        revokers.push(host.LostFocus({
            let (weak, on_focus) = (weak.clone(), on_focus);
            cb!("pane lost focus", |_, _| {
                let Some(view) = weak.upgrade() else { return };
                if let Some(ime) = &view.ime {
                    ime.focus_leave();
                }
                view.pane.set_focused(false);
                on_focus(view.id, false);
            })
        })?);
        revokers.push(host.PointerPressed({
            let weak = weak.clone();
            cb!("pane pointer pressed", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                view.on_pointer(args, Phase::Press);
            })
        })?);
        revokers.push(host.PointerReleased({
            let weak = weak.clone();
            cb!("pane pointer released", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                view.on_pointer(args, Phase::Release);
            })
        })?);
        revokers.push(host.PointerMoved({
            let weak = weak.clone();
            cb!("pane pointer moved", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                view.on_pointer(args, Phase::Move);
            })
        })?);
        // The pointer capture ended (the window lost it, a menu opened): a held button is no longer held.
        revokers.push(host.PointerCaptureLost({
            let weak = weak.clone();
            cb!("pane pointer capture lost", |_, _| {
                if let Some(view) = weak.upgrade() {
                    view.on_pointer_lost();
                }
            })
        })?);

        // Application shortcuts already ran at the window root. Non-text keys and Ctrl/Alt chords are
        // encoded from this event; text keys are left to CharacterReceived and the IME.
        revokers.push(host.PreviewKeyDown({
            let weak = weak.clone();
            cb!("pane key", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                let code = args.Key().map_or(0, |key| key.0);
                if code == vk::VK_PROCESSKEY || code == vk::VK_PACKET {
                    return;
                }
                let status = args.KeyStatus().unwrap_or_default();
                let mods = keyboard::input_mods(keyboard::held_modifiers());
                let scan = u16::try_from(status.scan_code).unwrap_or(0);
                let Some(key) = vk::key_for_vk_scan(code, scan, status.is_extended_key, mods) else { return };
                let event = if status.was_key_down { KeyEvent::Repeat } else { KeyEvent::Press };
                view.pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(event));
                let _ = args.SetHandled(true);
            })
        })?);
        // CharacterReceived delivers UTF-16 code units: an astral character arrives as two events.
        let assembler = RefCell::new(Utf16Assembler::default());
        revokers.push(host.CharacterReceived({
            let weak = weak.clone();
            cb!("pane character", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                if let Some(ch) = assembler.borrow_mut().push(args.Character().unwrap_or(0)) {
                    view.pane.send_text(&ch.to_string());
                }
                let _ = args.SetHandled(true);
            })
        })?);

        // Files dropped on the pane are pasted as paths.
        host.SetAllowDrop(true)?;
        revokers.push(host.DragOver(cb!("pane drag over", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let _ = args.SetAcceptedOperation(DataPackageOperation::Copy);
            if let Ok(overlay) = args.DragUIOverride() {
                let _ = overlay.SetCaption(t(Text::DropPastePath));
            }
        }))?);
        revokers.push(host.Drop({
            let weak = weak.clone();
            cb!("pane drop", |_, args| {
                let Some(view) = weak.upgrade() else { return };
                let Some(view_data) = args.as_ref().and_then(|a| a.DataView().ok()) else { return };
                let Ok(operation) = view_data.GetStorageItemsAsync() else { return };
                // The completion closure must be `Send`: it carries the pane id and looks the pane up again.
                let id = view.id;
                let _ = operation.when(move |items| {
                    let paths: Vec<String> = items
                        .map(|items| {
                            (0..items.Size().unwrap_or(0))
                                .filter_map(|i| items.GetAt(i).ok())
                                .filter_map(|item| item.Path().ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    if paths.is_empty() {
                        return;
                    }
                    let quoted: Vec<String> = paths.iter().map(|path| quote_path(path)).collect();
                    crate::shell::with_pane(id, |view| view.pane.send_text(&quoted.join(" ")));
                });
            })
        })?);

        revokers.push(host.PointerWheelChanged({
            let weak = weak.clone();
            cb!("pane wheel", |_, args| {
                let (Some(view), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                view.on_wheel(args);
                let _ = args.SetHandled(true);
            })
        })?);

        *self.revokers.borrow_mut() = revokers;
        Ok(())
    }

    fn ime_layout(&self) -> Option<ime::Layout> {
        let render = self.render.try_borrow().ok()?;
        let (cx, cy, cw, ch) = render.renderer.as_ref()?.cursor_cell_px(&self.pane.term);
        let panel: FrameworkElement = self.panel.cast().ok()?;
        let (x, y, w, h) = window::element_screen_rect(&self.window, &self.root, &panel)?;
        Some(ime::Layout { cursor_cell: (x + cx, y + cy, cw, ch), pane: (x, y, w, h) })
    }

    pub fn focus(&self) {
        let _ = ui(&self.host).and_then(|host| host.Focus(FocusState::Programmatic));
    }

    /// The grid in physical pixels, as the pointer needs it, and the pixels per DIP. `None` before the first frame.
    fn pointer_grid(&self) -> Option<(PointerGrid, f32)> {
        let render = self.render.try_borrow().ok()?;
        let metrics = render.renderer.as_ref()?.metrics();
        let grid = PointerGrid {
            cell_width: metrics.width as f32,
            cell_height: metrics.height as f32,
            columns: u16::try_from(render.grid.0).ok()?,
            rows: u16::try_from(render.grid.1).ok()?,
        };
        Some((grid, render.scale))
    }

    /// A pointer event of the host: press, move or release of the mouse buttons. The pane applies it (selection,
    /// reports to the program); what the shell has to do about it is requested.
    fn on_pointer(&self, args: &PointerRoutedEventArgs, phase: Phase) {
        let Some((grid, scale)) = self.pointer_grid() else { return };
        let Ok(point) = args.GetCurrentPoint(&self.panel) else { return };
        let (Ok(position), Ok(properties)) = (point.Position(), point.Properties()) else { return };
        let down = [
            properties.IsLeftButtonPressed().unwrap_or(false),
            properties.IsMiddleButtonPressed().unwrap_or(false),
            properties.IsRightButtonPressed().unwrap_or(false),
        ];
        let before = self.buttons.replace(down);
        let mods = keyboard::input_mods(keyboard::held_modifiers());
        let event = |kind| {
            PointerEvent::new(kind, position.x * scale, position.y * scale)
                .with_mods(mods)
                .with_time((point.Timestamp().unwrap_or(0) / 1000) as u32)
        };
        let mut kinds = Vec::new();
        match phase {
            Phase::Press => {
                let _ = ui(&self.host).and_then(|host| host.Focus(FocusState::Pointer));
                // Moves outside the pane keep coming while a button is held, which a selection drag needs.
                if let Ok(pointer) = args.Pointer() {
                    let _ = ui(&self.host).and_then(|host| host.CapturePointer(&pointer));
                }
                kinds.extend((0..3).filter(|i| down[*i] && !before[*i]).map(|i| PointerKind::Press(BUTTONS[i])));
            }
            Phase::Move => kinds.push(PointerKind::Move),
            Phase::Release => {
                kinds.extend((0..3).filter(|i| before[*i] && !down[*i]).map(|i| PointerKind::Release(BUTTONS[i])));
                if !down.contains(&true)
                    && let Ok(pointer) = args.Pointer()
                {
                    let _ = ui(&self.host).and_then(|host| host.ReleasePointerCapture(&pointer));
                }
            }
        }
        for kind in kinds {
            if kind == PointerKind::Press(MouseButton::Right)
                && !self.pane.input_modes().program_gets_pointer(mods)
            {
                let (x, y) = (f64::from(position.x), f64::from(position.y));
                (self.on_request)(self.id, PaneRequest::RightClick { x, y });
            }
            let effects = self.pane.pointer(&event(kind), &grid);
            if effects.selected {
                (self.on_request)(self.id, PaneRequest::Selected);
            }
        }
        if phase != Phase::Move {
            let _ = args.SetHandled(true);
        }
    }

    /// The pointer capture ended: held buttons go up for the program and a selection drag ends where it was.
    fn on_pointer_lost(&self) {
        self.buttons.set([false; 3]);
        if let Some((grid, _)) = self.pointer_grid() {
            let effects = self.pane.pointer_cancel(&grid);
            if effects.selected {
                (self.on_request)(self.id, PaneRequest::Selected);
            }
        }
    }

    /// The wheel: notches for a program that tracks the mouse (or cursor keys of alternate scroll), lines of the
    /// scrollback otherwise.
    fn on_wheel(&self, args: &PointerRoutedEventArgs) {
        let Some((grid, scale)) = self.pointer_grid() else { return };
        let Ok(point) = args.GetCurrentPoint(&self.panel) else { return };
        let (Ok(position), Ok(properties)) = (point.Position(), point.Properties()) else { return };
        let delta = properties.MouseWheelDelta().unwrap_or(0);
        let mods = keyboard::input_mods(keyboard::held_modifiers());
        let event = PointerEvent::new(
            PointerKind::Wheel { delta, horizontal: false },
            position.x * scale,
            position.y * scale,
        )
        .with_mods(mods);
        self.pane.pointer(&event, &grid);
    }

    /// `SizeChanged` / `CompositionScaleChanged` of the panel (DIP). Creates the surface and the renderer on
    /// the first non-empty size and keeps the terminal grid in step with the pixels.
    pub fn on_size(&self, width_dip: f64, height_dip: f64) {
        let Ok(mut render) = self.render.try_borrow_mut() else { return };
        render.last_size_dip = (width_dip, height_dip);
        let metrics = match render.surface.on_metrics(width_dip, height_dip) {
            Ok(Some(metrics)) => metrics,
            Ok(None) => return,
            Err(e) => {
                log(&format!("pane {} surface: {e}", self.id));
                return;
            }
        };
        render.scale = metrics.scale.0;
        if render.renderer.is_none() {
            let Some(gpu) = render.surface.gpu() else { return };
            let config = self.look.renderer_config(metrics.scale.0);
            let theme = self.look.theme.borrow().clone();
            match PaneRenderer::new(&gpu.device, &gpu.queue, FORMAT, config, theme) {
                Ok(mut renderer) => {
                    renderer.set_focused(!render.dimmed);
                    render.renderer = Some(renderer);
                }
                Err(e) => {
                    log(&format!("pane {} renderer: {e}", self.id));
                    return;
                }
            }
        } else if metrics.scale_changed
            && let Some(renderer) = render.renderer.as_mut()
            && let Err(e) = renderer.set_scale(metrics.scale.0)
        {
            log(&format!("pane {} set_scale: {e}", self.id));
        }
        let Render { renderer, grid, .. } = &mut *render;
        let Some(renderer) = renderer.as_mut() else { return };
        let new_grid = renderer.grid_for(metrics.size_px);
        if new_grid != *grid {
            *grid = new_grid;
            self.pane.resize(new_grid.0 as u16, new_grid.1 as u16);
            let cell = renderer.metrics();
            let (cols, rows) = new_grid;
            log(&format!("pane {} grid {cols}x{rows} cells of {}x{} px", self.id, cell.width, cell.height));
        }
        // A new surface starts empty: everything must be drawn again.
        renderer.invalidate();
    }

    /// Hidden panes give their swap chain back until they are shown again.
    pub fn suspend(&self) {
        if let Ok(mut render) = self.render.try_borrow_mut() {
            render.surface.suspend();
        }
    }

    /// Dimmed panes are the unfocused ones of a tab with several panes.
    pub fn set_dimmed(&self, dimmed: bool) {
        if let Ok(mut render) = self.render.try_borrow_mut() {
            render.dimmed = dimmed;
            if let Some(renderer) = render.renderer.as_mut() {
                renderer.set_focused(!dimmed);
            }
        }
    }

    /// Only the theme changed (colour scheme, background opacity): the renderer redraws with it, and the
    /// glyph atlases, the font and the swap chain are left alone.
    pub fn apply_theme(&self) {
        let Ok(mut render) = self.render.try_borrow_mut() else { return };
        if let Some(renderer) = render.renderer.as_mut() {
            renderer.set_theme(self.look.theme.borrow().clone());
        }
    }

    /// The theme, font size or line height changed (settings, zoom). A new font family needs a new renderer,
    /// which `on_size` builds when the old one is dropped here.
    pub fn apply_look(&self, new_font_family: bool) {
        let Ok(mut render) = self.render.try_borrow_mut() else { return };
        if new_font_family {
            render.renderer = None;
            render.grid = (0, 0);
            let (w, h) = render.last_size_dip;
            drop(render);
            self.on_size(w, h);
            return;
        }
        if let Some(renderer) = render.renderer.as_mut() {
            renderer.set_theme(self.look.theme.borrow().clone());
            if let Err(e) = renderer.set_font_size(self.look.font_pt.get()) {
                log(&format!("pane {} font size: {e}", self.id));
            }
        }
        let (w, h) = render.last_size_dip;
        drop(render);
        self.on_size(w, h);
    }

    /// Closing the pane would end work: its program runs a command or an agent and has not ended.
    pub fn is_busy(&self) -> bool {
        self.exited.get().is_none() && self.pane.is_busy()
    }

    /// Events from the program (title, exit, working directory), oldest first.
    pub fn poll_events(&self) -> Vec<UiEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.pane.events.try_recv() {
            events.push(event);
        }
        events
    }

    /// One vsync-paced step: snapshot, update the instance buffers, draw and present when something changed.
    pub fn tick(&self) {
        // A selection drag held above or below the pane keeps scrolling while the pointer rests there.
        if self.buttons.get()[0] && self.last_drag_scroll.get().elapsed() >= DRAG_SCROLL_INTERVAL {
            self.last_drag_scroll.set(Instant::now());
            self.pane.drag_scroll();
        }
        let Ok(mut render) = self.render.try_borrow_mut() else { return };
        let Render { surface, renderer, frames, last_text_update, text_stale, .. } = &mut *render;
        let Some(renderer) = renderer.as_mut() else { return };
        renderer.set_preedit(self.ime.as_ref().and_then(|ime| ime.preedit()));
        let prepared = renderer.prepare(&self.pane.term);
        // The last change before the screen goes quiet must reach UI Automation too, not only the changes
        // that happen to fall outside the refresh interval.
        if *text_stale && last_text_update.elapsed() >= TEXT_MODEL_INTERVAL {
            *last_text_update = Instant::now();
            *text_stale = false;
            self.update_text_model(renderer);
        }
        if !prepared.needs_draw {
            return;
        }
        let outcome = surface.present(|device, queue, view, size| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            renderer.draw(&mut encoder, view, size);
            queue.submit([encoder.finish()]);
        });
        match outcome {
            Presented::Drawn => {
                *frames += 1;
                if *frames == 1 {
                    log(&format!("pane {} first frame", self.id));
                }
                *text_stale = true;
            }
            Presented::Reconfigured => renderer.invalidate(),
            Presented::Skipped => {}
        }
    }

    /// The visible text for UI Automation.
    fn update_text_model(&self, renderer: &PaneRenderer) {
        let snapshot = renderer.snapshot();
        let text = snapshot.text();
        let (cursor_row, cursor_col) = snapshot.cursor.position.unwrap_or((0, 0));
        let before: usize =
            (0..cursor_row.min(snapshot.rows)).map(|row| snapshot.row_text(row).chars().count() + 1).sum();
        let in_row = snapshot.row_text(cursor_row.min(snapshot.rows.saturating_sub(1))).chars().count().min(cursor_col);
        let caret = i32::try_from((before + in_row).min(text.chars().count())).unwrap_or(0);
        *self.text_model.text.borrow_mut() = text;
        self.text_model.caret.set((caret, caret));
    }

    pub fn shutdown(&self) {
        self.pane.shutdown();
    }
}

fn panel_size(panel: &SwapChainPanel) -> (f64, f64) {
    match panel.cast::<FrameworkElement>() {
        Ok(fe) => (fe.ActualWidth().unwrap_or(0.0), fe.ActualHeight().unwrap_or(0.0)),
        Err(_) => (0.0, 0.0),
    }
}
