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
use nekominal_input::{KeyEvent, KeyInput, vk};
use nekominal_model::{layout::PaneId, strings::Text};
use nekominal_pane::{Pane, PaneRenderer, UiEvent};
use nekominal_pty::conpty::SpawnConfig;
use nekominal_render as r;
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

struct Render {
    surface: PanelSurface,
    renderer: Option<PaneRenderer>,
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
        profile_id: String,
        look: Rc<Look>,
        window: &Window,
        root: &FrameworkElement,
        on_focus: Rc<dyn Fn(PaneId, bool)>,
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
        let pane = Pane::spawn_with(config)
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
                    grid: (0, 0),
                    frames: 0,
                    dimmed: false,
                    last_size_dip: (0.0, 0.0),
                    last_text_update: Instant::now(),
                    text_stale: false,
                }),
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
            cb!("pane pointer", |_, _| {
                if let Some(view) = weak.upgrade() {
                    let _ = ui(&view.host).and_then(|host| host.Focus(FocusState::Pointer));
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
                let delta = args
                    .GetCurrentPoint(&view.host)
                    .and_then(|p| p.Properties())
                    .and_then(|p| p.MouseWheelDelta())
                    .unwrap_or(0);
                view.pane.scroll_lines(i32::from(delta > 0) * 3 - i32::from(delta < 0) * 3);
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
