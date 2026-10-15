//! The application of one window: tabs with a terminal pane each, the chrome drawn around them, and the input.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nekominal_input::{KeyEvent, KeyInput, Mods, Utf16Assembler, vk};
use nekominal_model::{
    commands::{self, Command},
    keymap::{self, Keymap, Modifiers},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, Settings},
    store,
    themes::{self, Color as SchemeColor},
};
use nekominal_pane::{Pane, PaneRenderer, UiEvent};
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_render as r;
use nekominal_ui::{Color, Rect, Ui};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    Graphics::Gdi::InvalidateRect,
    UI::{
        Input::KeyboardAndMouse::GetKeyState,
        WindowsAndMessaging::{PostMessageW, WM_APP},
    },
};

use crate::{
    glass::{self, Glass},
    gpu::{self, Gpu, Surface},
    layout::{self, Hit, Layout},
    logic,
};

/// Posted by a reader thread when a pane has output to draw.
pub const WM_WAKE: u32 = WM_APP + 1;

/// Corner radius of every rounded rectangle, in DIP (the same number as `design.rs` of the WinUI shell).
pub const CORNER_RADIUS: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 10.0;

static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

struct Tab {
    title: String,
    pane: Pane,
    view: PaneRenderer,
    grid: (u16, u16),
    exited: bool,
}

pub struct App {
    hwnd: HWND,
    gpu: Gpu,
    surface: Surface,
    ui: Ui,
    scale: f32,
    maximized: bool,
    active: bool,
    settings: Settings,
    keymap: Keymap,
    glass: Glass,
    theme: r::Theme,
    profile: Profile,
    tabs: Vec<Tab>,
    selected: usize,
    layout: Layout,
    /// Width in DIP of the caption buttons DWM draws at the top right.
    caption: f32,
    hover: Hit,
    pressed: Hit,
    text: Utf16Assembler,
}

fn col(c: SchemeColor) -> Color {
    Color([c.r, c.g, c.b, c.a])
}

fn discovered(found: &nekominal_pty::profiles::Profile) -> DiscoveredProfile {
    DiscoveredProfile {
        id: found.id.clone(),
        name: found.name.clone(),
        kind: found.kind,
        icon: found.icon,
        program: found.program.clone(),
        args: found.args.clone(),
    }
}

fn held() -> Modifiers {
    let down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
    Modifiers { ctrl: down(0x11), shift: down(0x10), alt: down(0x12), meta: down(0x5B) || down(0x5C) }
}

fn input_mods(m: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    for (on, flag) in [(m.shift, Mods::SHIFT), (m.ctrl, Mods::CTRL), (m.alt, Mods::ALT), (m.meta, Mods::SUPER)] {
        if on {
            mods |= flag;
        }
    }
    mods
}

impl App {
    pub fn new(hwnd: HWND, size_px: (u32, u32), scale: f32) -> Result<App, String> {
        let text = store::settings_path().and_then(|path| store::load(&path).ok().flatten());
        let settings = settings::parse(text.as_deref()).settings;
        let keymap = keymap::build_keymap(commands::default_bindings(), &settings.keybindings).keymap;
        let scheme = themes::scheme(&settings.theme);
        let glass = Glass::from_scheme(scheme);
        let opacity = glass::terminal_opacity(settings.appearance.backdrop, settings.appearance.opacity, glass::Material::NORMAL);
        let theme = glass::render_theme(scheme, opacity);

        let found: Vec<DiscoveredProfile> = nekominal_pty::profiles::discover().iter().map(discovered).collect();
        let all = profiles::merge(&found, &settings.profiles);
        let profile = profiles::resolve_default(&all, &settings.default_profile)
            .or(all.first())
            .cloned()
            .ok_or("no shell was found")?;

        let (gpu, surface) = gpu::create(hwnd, size_px)?;
        let ui = Ui::new(&gpu.device, &gpu.queue, gpu::FORMAT)?;
        let mut app = App {
            hwnd,
            gpu,
            surface,
            ui,
            scale,
            maximized: false,
            active: true,
            settings,
            keymap,
            glass,
            theme,
            profile,
            tabs: Vec::new(),
            selected: 0,
            layout: Layout::new(1.0, 1.0, 0, 0.0),
            caption: 138.0,
            hover: Hit::None,
            pressed: Hit::None,
            text: Utf16Assembler::default(),
        };
        app.relayout();
        app.new_tab()?;
        Ok(app)
    }

    pub fn backdrop(&self) -> Backdrop {
        self.settings.appearance.backdrop
    }

    pub fn dark(&self) -> bool {
        !self.glass.light
    }

    // --- tabs ---------------------------------------------------------------------------------------------------

    fn spawn_config(&self, cols: u16, rows: u16) -> SpawnConfig {
        let mut env = nekominal_pty::profiles::default_environment();
        if self.settings.agent_environment {
            env.extend(nekominal_pty::profiles::agent_environment());
        }
        let program = nekominal_pty::profiles::resolve_program(&self.profile.program)
            .unwrap_or_else(|| self.profile.program.clone().into());
        SpawnConfig {
            program,
            args: profiles::launch_args(&self.profile, self.settings.shell_integration),
            cwd: self.profile.cwd.as_deref().map(std::path::PathBuf::from).filter(|dir| dir.is_dir()),
            env,
            cols,
            rows,
            width_model: WidthModel::default(),
        }
    }

    fn renderer_config(&self) -> r::RendererConfig {
        let mut fonts = r::fonts::FontConfig::default();
        if !self.settings.font.family.is_empty() {
            fonts.text = self.settings.font.family.clone();
        }
        r::RendererConfig {
            fonts,
            font_size_pt: self.settings.font.size as f32,
            scale: self.scale,
            line_height: self.settings.font.line_height as f32,
            lcd: self.theme.background_opacity >= 1.0,
            ..Default::default()
        }
    }

    fn new_tab(&mut self) -> Result<(), String> {
        let mut view = PaneRenderer::new(
            &self.gpu.device,
            &self.gpu.queue,
            gpu::FORMAT,
            self.renderer_config(),
            self.theme.clone(),
        )?;
        let pane_px = self.pane_px();
        let (cols, rows) = view.grid_for(pane_px);
        let hwnd = self.hwnd.0 as isize;
        let wake: nekominal_pane::Wake = Arc::new(move || {
            // One posted message per burst of output: the handler clears the flag when it draws.
            if !WAKE_PENDING.swap(true, Ordering::AcqRel) {
                // SAFETY: posting to a window handle from another thread is allowed; a dead window drops it.
                unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0)).ok() };
            }
        });
        let config = self.spawn_config(cols as u16, rows as u16);
        let core = logic::core_config(&self.settings);
        let pane = Pane::spawn_woken(config, core, wake).map_err(|e| format!("pty: {e}"))?;
        view.set_focused(true);
        self.tabs.push(Tab { title: self.profile.name.clone(), pane, view, grid: (cols as u16, rows as u16), exited: false });
        self.selected = self.tabs.len() - 1;
        self.relayout();
        self.redraw();
        Ok(())
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        tab.pane.shutdown();
        if self.tabs.is_empty() {
            // SAFETY: ends the message loop of this thread through the window.
            unsafe { windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(self.hwnd), windows::Win32::UI::WindowsAndMessaging::WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
            return;
        }
        if self.selected > index || self.selected >= self.tabs.len() {
            self.selected = self.selected.saturating_sub(1);
        }
        self.relayout();
        self.redraw();
    }

    fn select(&mut self, index: usize) {
        if index < self.tabs.len() && index != self.selected {
            self.selected = index;
            self.tabs[index].view.invalidate();
            self.relayout();
            self.redraw();
        }
    }

    fn pane_px(&self) -> (u32, u32) {
        let s = self.scale;
        let (w, h) = (self.surface.size().0 as f32, self.surface.size().1 as f32);
        let top = (layout::TITLE_HEIGHT * s).round();
        (w.max(1.0) as u32, (h - top).max(1.0) as u32)
    }

    // --- events from the window ---------------------------------------------------------------------------------

    pub fn on_size(&mut self, size_px: (u32, u32), maximized: bool) {
        self.maximized = maximized;
        self.surface.resize(&self.gpu, size_px);
        self.relayout();
        self.redraw();
    }

    pub fn on_dpi(&mut self, scale: f32) {
        self.scale = scale;
        for tab in &mut self.tabs {
            let _ = tab.view.set_scale(scale);
        }
        self.relayout();
        self.redraw();
    }

    /// The system caption buttons are width_px wide: the chips keep clear of them.
    pub fn on_caption(&mut self, width_px: f32) {
        self.caption = width_px / self.scale;
        self.relayout();
        self.redraw();
    }

    pub fn on_activate(&mut self, active: bool) {
        self.active = active;
        if let Some(tab) = self.tabs.get_mut(self.selected) {
            tab.view.set_focused(active);
            tab.pane.set_focused(active);
        }
        self.redraw();
    }

    fn relayout(&mut self) {
        let (w, h) = self.surface.size();
        self.layout = Layout::new(w as f32 / self.scale, h as f32 / self.scale, self.tabs.len(), self.caption);
        let px = self.pane_px();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i != self.selected {
                continue;
            }
            let (cols, rows) = tab.view.grid_for(px);
            if (cols as u16, rows as u16) != tab.grid {
                tab.grid = (cols as u16, rows as u16);
                tab.pane.resize(cols as u16, rows as u16);
                tab.view.invalidate();
            }
        }
    }

    pub fn redraw(&self) {
        // SAFETY: invalidating our own window; WM_PAINT arrives when the queue is otherwise empty.
        unsafe { InvalidateRect(Some(self.hwnd), None, false).ok() };
    }

    /// Output arrived (or a pane's program ended): take the events and ask for a frame.
    pub fn on_wake(&mut self) {
        WAKE_PENDING.store(false, Ordering::Release);
        let mut closed = Vec::new();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            while let Ok(event) = tab.pane.events.try_recv() {
                match event {
                    UiEvent::Title(title) => tab.title = logic::clean_title(&title),
                    UiEvent::Exit(code) => {
                        tab.exited = true;
                        if code == 0 {
                            closed.push(i);
                        }
                    }
                    _ => {}
                }
            }
        }
        for i in closed.into_iter().rev() {
            self.close_tab(i);
        }
        self.redraw();
    }

    // --- mouse --------------------------------------------------------------------------------------------------

    fn point(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        let (x, y) = self.point(x, y);
        self.layout.hit(x, y)
    }

    pub fn on_mouse_move(&mut self, x: i32, y: i32) {
        let hit = self.hit_test(x, y);
        if hit != self.hover {
            self.hover = hit;
            self.redraw();
        }
    }

    pub fn on_mouse_leave(&mut self) {
        if self.hover != Hit::None {
            self.hover = Hit::None;
            self.redraw();
        }
    }

    pub fn on_button(&mut self, x: i32, y: i32, down: bool) {
        let hit = self.hit_test(x, y);
        if down {
            self.pressed = hit;
            self.redraw();
            return;
        }
        let pressed = std::mem::replace(&mut self.pressed, Hit::None);
        self.redraw();
        if pressed != hit {
            return;
        }
        match hit {
            Hit::Chip(i) => self.select(i),
            Hit::ChipClose(i) => self.close_tab(i),
            Hit::NewTab => {
                if let Err(e) = self.new_tab() {
                    eprintln!("new tab: {e}");
                }
            }
            _ => {}
        }
    }

    pub fn on_wheel(&mut self, delta: i16) {
        if let Some(tab) = self.tabs.get(self.selected) {
            tab.pane.scroll_lines(i32::from(delta) / 120 * 3);
            self.redraw();
        }
    }

    // --- keyboard -----------------------------------------------------------------------------------------------

    pub fn run(&mut self, command: Command) {
        match command {
            Command::TabNew => {
                if let Err(e) = self.new_tab() {
                    eprintln!("new tab: {e}");
                }
            }
            Command::TabClose => self.close_tab(self.selected),
            Command::TabNext => self.select((self.selected + 1) % self.tabs.len().max(1)),
            Command::TabPrevious => self.select((self.selected + self.tabs.len().max(1) - 1) % self.tabs.len().max(1)),
            other => eprintln!("command {} is not wired yet", other.id()),
        }
    }

    /// A key went down. `true` when the window handled it (a shortcut, or the terminal encoded it).
    pub fn on_key_down(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        let held = held();
        if let Some(name) = nekominal_input::physical_key(scan, extended).map(|key| key.name())
            && let Some(command) = self.keymap.find(held, name)
        {
            if !repeat {
                self.run(command);
            }
            return true;
        }
        if vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET {
            return false;
        }
        let mods = input_mods(held);
        let Some(key) = vk::key_for_vk_scan(vk_code, scan, extended, mods) else { return false };
        let Some(tab) = self.tabs.get(self.selected) else { return false };
        let event = if repeat { KeyEvent::Repeat } else { KeyEvent::Press };
        tab.pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(event));
        true
    }

    /// One UTF-16 unit of text from the keyboard or the IME.
    pub fn on_char(&mut self, unit: u16) {
        if unit < 0x20 || unit == 0x7F {
            return;
        }
        if let Some(ch) = self.text.push(unit)
            && let Some(tab) = self.tabs.get(self.selected)
        {
            tab.pane.send_text(&ch.to_string());
        }
    }

    // --- drawing ------------------------------------------------------------------------------------------------

    pub fn paint(&mut self) {
        let size = self.surface.size();
        let Some(frame) = self.surface.acquire(&self.gpu) else {
            self.redraw();
            return;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        let s = self.scale;
        let top = (layout::TITLE_HEIGHT * s).round() as u32;
        let rect = [0, top.min(size.1), size.0, size.1.saturating_sub(top)];
        match self.tabs.get_mut(self.selected) {
            Some(tab) => {
                let prepared = tab.view.prepare(&tab.pane.term);
                let _ = prepared;
                tab.view.draw_in(&mut encoder, &view, rect);
            }
            None => drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            })),
        }

        self.ui.begin_frame(size, s);
        self.paint_chrome();
        self.ui.end_frame(&mut encoder, &view);
        self.gpu.queue.submit([encoder.finish()]);
        self.surface.present(&self.gpu, frame);
    }

    fn paint_chrome(&mut self) {
        let g = self.glass;
        let width = self.layout.size.0;
        let tint = {
            let opacity = glass::terminal_opacity(self.settings.appearance.backdrop, self.settings.appearance.opacity, glass::Material { window_active: self.active, effects_on: true });
            let alpha = glass::bar_tint_alpha(opacity);
            Color([g.background.r, g.background.g, g.background.b, (alpha * 255.0).round() as u8])
        };
        self.ui.fill(Rect::new(0.0, 0.0, width, layout::TITLE_HEIGHT), tint);
        self.ui.fill(Rect::new(0.0, layout::TITLE_HEIGHT - 1.0, width, 1.0), col(g.divider()));

        let line = self.ui.line_height(TEXT_SIZE);
        for (i, chip) in self.layout.chips.clone().into_iter().enumerate() {
            let selected = i == self.selected;
            let hover = matches!(self.hover, Hit::Chip(n) | Hit::ChipClose(n) if n == i);
            let (fill, border) = if selected {
                (col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)))
            } else if hover {
                (col(g.hover()), None)
            } else {
                (Color::TRANSPARENT, None)
            };
            self.ui.rrect(chip, CORNER_RADIUS, fill, border);
            let title = if logic::shows_ordinal(f64::from(chip.w)) {
                (i + 1).to_string()
            } else {
                self.tabs[i].title.clone()
            };
            let close = Layout::close_button(chip).filter(|_| selected || hover);
            let right = close.map_or(chip.x + chip.w - 10.0, |c| c.x - 4.0);
            self.ui.push_clip(Rect::new(chip.x, chip.y, (right - chip.x).max(0.0), chip.h));
            let color = if selected { col(g.text()) } else { col(g.text_secondary()) };
            self.ui.text(chip.x + 12.0, chip.y + (chip.h - line) / 2.0, &title, TEXT_SIZE, color, false);
            self.ui.pop_clip();
            if let Some(close) = close {
                if self.hover == Hit::ChipClose(i) {
                    self.ui.rrect(close, CORNER_RADIUS, col(g.pressed()), None);
                }
                self.icon_centered(close, '\u{E711}', ICON_SIZE, col(g.text()));
            }
        }
        let plus = self.layout.new_tab;
        if self.hover == Hit::NewTab {
            self.ui.rrect(plus, CORNER_RADIUS, col(g.hover()), None);
        }
        self.icon_centered(plus, '\u{E710}', 12.0, col(g.text()));
    }

    fn icon_centered(&mut self, rect: Rect, ch: char, size: f32, color: Color) {
        let line = self.ui.line_height(size);
        let width = self.ui.measure_icon(ch, size);
        self.ui.icon(rect.x + (rect.w - width) / 2.0, rect.y + (rect.h - line) / 2.0, ch, size, color);
    }
}
