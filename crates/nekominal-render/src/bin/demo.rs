//! Headless renderer demo: renders test scenes to PNG and measures frame cost.
//!
//! usage: demo [--adapter real|warp] [--out DIR] [--scale 1.0] [--format bgra|rgba]
//!             [--lcd on|off] [--hint on|off] [--bench] [--ligatures] [--scene NAME]

use std::path::PathBuf;
use std::time::Instant;

use nekominal_render::offscreen::{self, AdapterKind, GpuContext, OffscreenTarget};
use nekominal_render::*;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ---------------------------------------------------------------------------
// Scene building helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct St {
    fg: Color,
    bg: Color,
    flags: CellFlags,
    ulc: Option<Color>,
}

impl St {
    fn new() -> Self {
        St { fg: Color::Default, bg: Color::Default, flags: CellFlags(0), ulc: None }
    }
    fn fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    fn bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    fn on(self, r: u8, g: u8, b: u8) -> Self {
        self.bg(Color::Rgb(r, g, b))
    }
    fn f(mut self, bit: u16) -> Self {
        self.flags = self.flags.with(bit);
        self
    }
    fn ul(mut self, u: Underline) -> Self {
        self.flags = self.flags.with_underline(u);
        self
    }
    fn ulc(mut self, c: Color) -> Self {
        self.ulc = Some(c);
        self
    }
}

struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl Grid {
    fn new(cols: usize, rows: usize) -> Self {
        Grid { cols, rows, cells: vec![Cell::default(); cols * rows] }
    }

    /// Emulator-style grapheme placement: wide clusters take two cells, zero
    /// width marks and ZWJ sequences are attached to the base cell.
    fn put(&mut self, r: usize, mut c: usize, s: &str, st: St) -> usize {
        for g in s.graphemes(true) {
            let mut chars = g.chars();
            let base = chars.next().unwrap();
            let rest: Vec<char> = chars.collect();
            let w = if rest.is_empty() { base.width().unwrap_or(1) } else { UnicodeWidthStr::width(g) }.clamp(1, 2);
            if r >= self.rows || c + w > self.cols {
                break;
            }
            let mut cell = Cell { ch: base, fg: st.fg, bg: st.bg, flags: st.flags, extra: None };
            if !rest.is_empty() {
                cell = cell.zerowidth(&rest);
            }
            if let Some(u) = st.ulc {
                cell = cell.underline_color(u);
            }
            if w == 2 {
                cell.flags = cell.flags.with(CellFlags::WIDE);
            }
            self.cells[r * self.cols + c] = cell;
            if w == 2 {
                self.cells[r * self.cols + c + 1] =
                    Cell { ch: ' ', fg: st.fg, bg: st.bg, flags: CellFlags(CellFlags::WIDE_SPACER), extra: None };
            }
            c += w;
        }
        c
    }

    fn fill_row(&mut self, r: usize, st: St) {
        for c in 0..self.cols {
            self.cells[r * self.cols + c] = Cell { ch: ' ', fg: st.fg, bg: st.bg, flags: st.flags, extra: None };
        }
    }
}

fn idx(i: u8) -> Color {
    Color::Indexed(i)
}

fn showcase_80x24() -> Grid {
    let mut g = Grid::new(80, 24);
    let white = Color::Rgb(255, 255, 255);

    // Powerline status bar
    let seg = |g: &mut Grid, c: usize, text: &str, fg: Color, bg: Color, next_bg: Color| -> usize {
        let c = g.put(0, c, text, St::new().fg(fg).bg(bg).f(CellFlags::BOLD));
        g.put(0, c, "\u{e0b0}", St::new().fg(bg).bg(next_bg))
    };
    g.fill_row(0, St::new().bg(Color::Rgb(0x1f, 0x1f, 0x1f)));
    let mut c = 0;
    c = seg(&mut g, c, " NEKOMINAL ", white, Color::Rgb(0x00, 0x78, 0xd4), Color::Rgb(0x2d, 0x7d, 0x46));
    c = seg(&mut g, c, " main ", white, Color::Rgb(0x2d, 0x7d, 0x46), Color::Rgb(0xc1, 0x9c, 0x00));
    c = seg(&mut g, c, " +3 ~2 ", Color::Rgb(0, 0, 0), Color::Rgb(0xc1, 0x9c, 0x00), Color::Rgb(0x1f, 0x1f, 0x1f));
    let _ = g.put(0, c + 1, "\u{e0b2}", St::new().fg(Color::Rgb(0x88, 0x17, 0x98)).bg(Color::Rgb(0x1f, 0x1f, 0x1f)));
    g.put(0, c + 2, " 繁體中文 ", St::new().fg(white).bg(Color::Rgb(0x88, 0x17, 0x98)));
    g.put(0, c + 12, "\u{e0b4}", St::new().fg(Color::Rgb(0x88, 0x17, 0x98)).bg(Color::Rgb(0x1f, 0x1f, 0x1f)));

    // ANSI 16
    g.put(2, 0, "ANSI", St::new().f(CellFlags::BOLD));
    for i in 0..8u8 {
        g.put(2, 6 + i as usize * 9, &format!("color{i}   "), St::new().fg(idx(i)));
        g.put(3, 6 + i as usize * 9, &format!("bright{i}  "), St::new().fg(idx(i + 8)));
        g.put(4, 6 + i as usize * 9, "         ", St::new().bg(idx(i)));
        g.put(5, 6 + i as usize * 9, "         ", St::new().bg(idx(i + 8)));
    }
    // 256 color cube + grayscale ramp
    g.put(6, 0, "256", St::new().f(CellFlags::BOLD));
    for n in 0..216usize {
        let (row, col) = (6 + n / 72, 6 + n % 72);
        if col < 80 {
            g.put(row, col, " ", St::new().bg(idx(16 + n as u8)));
        }
    }
    for n in 0..24usize {
        g.put(9, 6 + n, " ", St::new().bg(idx(232 + n as u8)));
    }
    g.put(9, 32, "grayscale ramp 232..255", St::new().fg(idx(8)));
    // truecolor gradient
    g.put(10, 0, "RGB", St::new().f(CellFlags::BOLD));
    for x in 0..74usize {
        let t = x as f32 / 73.0;
        let (r, gr, b) = ((255.0 * (1.0 - t)) as u8, (255.0 * (1.0 - (2.0 * t - 1.0).abs())) as u8, (255.0 * t) as u8);
        g.put(10, 6 + x, " ", St::new().on(r, gr, b));
    }

    // text styles
    let mut c = 0;
    for (text, st) in [
        ("bold", St::new().f(CellFlags::BOLD)),
        ("italic", St::new().f(CellFlags::ITALIC)),
        ("bold-italic", St::new().f(CellFlags::BOLD).f(CellFlags::ITALIC)),
        ("underline", St::new().ul(Underline::Single)),
        ("double", St::new().ul(Underline::Double)),
        ("curly", St::new().ul(Underline::Curly).ulc(Color::Rgb(255, 85, 85))),
        ("dotted", St::new().ul(Underline::Dotted).ulc(idx(10))),
        ("dashed", St::new().ul(Underline::Dashed).ulc(idx(12))),
        ("strike", St::new().f(CellFlags::STRIKEOUT)),
        ("faint", St::new().f(CellFlags::FAINT)),
        ("inverse", St::new().f(CellFlags::INVERSE)),
    ] {
        c = g.put(11, c, text, st) + 1;
    }

    // CJK, Traditional Chinese, IME-related scripts
    g.put(12, 0, "繁體中文：終端機模擬器渲染測試，骨直令刃返", St::new());
    g.put(13, 0, "注音：ㄓㄨˋ ㄧㄣ  倉頡：日月金木水  拼音：pīn yīn  ＦＵＬＬ ＷＩＤＴＨ", St::new().fg(idx(14)));
    g.put(14, 0, "日本語・한국어 mixed: コンパイル完了 ✓ 빌드 성공  Ünïcödé  e\u{301}  ǹ", St::new().fg(idx(11)));
    // emoji (incl. ZWJ, skin tone, flags, VS16)
    g.put(15, 0, "😀 👍🏽 👨‍👩‍👧‍👦 🇹🇼 🇯🇵 ❤️ 🧑‍💻 🚀 ✅ ⭐ ✨ 🐱 ⚡ 🔥", St::new());
    // box drawing
    let bx = St::new().fg(idx(12));
    g.put(16, 0, "┌────────────┬─────────┐ ┏━━━━━━┳━━━┓ ╔═══════╦═══╗ ╭────╮ ┌┄┄┐", bx);
    g.put(17, 0, "│ light      │ cells   │ ┃heavy ┃ x ┃ ║ double║ x ║ │    │ ┆  ┆", bx);
    g.put(18, 0, "├────────────┼─────────┤ ┗━━━━━━┻━━━┛ ╠═══════╬═══╣ ╰────╯ └┄┄┘", bx);
    g.put(19, 0, "└────────────┴─────────┘ ╒═══╤═══╕ ╓───╥───╖ ╚═══════╩═══╝ ╱╲╳ ╴╶╵╷ ╼╽╾╿", bx);
    // blocks, shades, braille
    g.put(20, 0, "█▉▊▋▌▍▎▏ ▀▄▔▕ ░▒▓ ▖▗▘▙▚▛▜▝▞▟  ⠁⠃⠇⡇⣇⣧⣷⣿ ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏", St::new().fg(idx(10)));
    // Powerline variants
    let mut c = 0;
    for (i, ch) in ('\u{e0b0}'..='\u{e0bf}').enumerate() {
        let col = [idx(1), idx(2), idx(3), idx(4), idx(5), idx(6)][i % 6];
        c = g.put(21, c, &ch.to_string(), St::new().fg(col).bg(Color::Rgb(0x30, 0x30, 0x30)));
        c = g.put(21, c, " ", St::new().bg(Color::Rgb(0x30, 0x30, 0x30)));
    }
    g.put(21, c + 1, "selection / search / link / cursor:", St::new().fg(idx(8)));
    // selection + search rows
    g.put(22, 2, "selected text region here", St::new());
    g.put(22, 32, "match  match  [current]  match", St::new());
    g.put(22, 66, "https://nekominal.dev", St::new().fg(Color::Rgb(0x3b, 0x78, 0xff)));
    // prompt
    g.put(23, 0, "❯", St::new().fg(idx(10)).f(CellFlags::BOLD));
    g.put(23, 2, "cargo run --release", St::new());
    g
}

fn showcase_140x40() -> Grid {
    let mut g = Grid::new(140, 40);
    let dim = St::new().fg(idx(8));
    let accent = St::new().fg(Color::Rgb(0xd9, 0x77, 0x57));
    // header (rounded box, agent-CLI style)
    g.put(0, 0, &format!("╭{}╮", "─".repeat(60)), accent);
    g.put(1, 0, "│", accent);
    g.put(1, 2, "✻ Welcome to Nekominal", St::new().f(CellFlags::BOLD));
    g.put(1, 61, "│", accent);
    g.put(2, 0, "│", accent);
    g.put(2, 2, "/help for help, /status for your current setup", dim);
    g.put(2, 61, "│", accent);
    g.put(3, 0, "│", accent);
    g.put(3, 2, "cwd: C:\\Users\\doeshing\\Desktop\\Nekominal", dim);
    g.put(3, 61, "│", accent);
    g.put(4, 0, &format!("╰{}╯", "─".repeat(60)), accent);

    g.put(
        6,
        0,
        "> 請幫我把 renderer 的 atlas 改成可以成長，並確認 CJK 與 emoji 都能正確顯示",
        St::new().f(CellFlags::BOLD),
    );
    g.put(8, 0, "●", St::new().fg(idx(10)));
    g.put(8, 2, "我會先檢查目前的 atlas 實作，然後加入 etagere 的 grow 路徑。", St::new());
    g.put(9, 0, "  ⎿ ", dim);
    g.put(9, 4, "Read(src/atlas.rs)", St::new().f(CellFlags::BOLD));
    g.put(10, 4, "Read 142 lines", dim);

    // diff block
    let del = St::new().fg(Color::Rgb(0xff, 0x9a, 0x9a)).on(0x4b, 0x1d, 0x1d);
    let add = St::new().fg(Color::Rgb(0xa6, 0xf0, 0xa6)).on(0x1d, 0x4b, 0x25);
    let ctx = St::new().fg(idx(7));
    let lines: [(&str, St, &str); 8] = [
        ("  41", ctx, "    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {"),
        ("  42", ctx, "        loop {"),
        ("  43", del, "-           return self.alloc.allocate(size2(w as i32, h as i32));"),
        ("  43", add, "+           if let Some(a) = self.alloc.allocate(size2(w as i32, h as i32)) {"),
        ("  44", add, "+               return Some((a.rectangle.min.x as u32, a.rectangle.min.y as u32));"),
        ("  45", add, "+           }"),
        ("  46", add, "+           self.grow(device, queue, (self.size * 2).min(self.max_size));"),
        ("  47", ctx, "        }"),
    ];
    for (i, (no, st, text)) in lines.iter().enumerate() {
        let r = 12 + i;
        g.put(r, 0, no, dim);
        let text_st = *st;
        for c in 4..110 {
            g.cells[r * 140 + c] = Cell { ch: ' ', fg: text_st.fg, bg: text_st.bg, flags: CellFlags(0), extra: None };
        }
        g.put(r, 4, text, text_st);
    }

    // table
    let tb = St::new().fg(idx(12));
    g.put(21, 0, "┌──────────────────┬──────────────┬──────────────┬───────────────────────┐", tb);
    g.put(22, 0, "│ 項目             │ 全螢幕重繪   │ 單行更新     │ 備註                  │", tb);
    g.put(23, 0, "├──────────────────┼──────────────┼──────────────┼───────────────────────┤", tb);
    g.put(24, 0, "│ CPU 編碼 (µs)    │ 0000.0       │ 0000.0       │ update + encode       │", tb);
    g.put(25, 0, "│ GPU 時間 (µs)    │ 0000.0       │ 0000.0       │ timestamp query       │", tb);
    g.put(26, 0, "└──────────────────┴──────────────┴──────────────┴───────────────────────┘", tb);

    // progress + spinner
    g.put(28, 0, "⠹", St::new().fg(idx(11)));
    g.put(28, 2, "Compiling nekominal_render v0.0.0", St::new().fg(idx(11)));
    g.put(29, 0, "[", dim);
    g.put(29, 1, &"█".repeat(40), St::new().fg(idx(10)));
    g.put(29, 41, "▌", St::new().fg(idx(10)));
    g.put(29, 42, &"░".repeat(20), dim);
    g.put(29, 62, "] 66%", dim);

    // wide text paragraph with mixed content
    g.put(31, 0, "Emoji: 😀 😎 🤖 👨‍👩‍👧‍👦 🧑‍💻 🇹🇼 🇺🇸 ❤️ ✅ ❌ ⚠️ 🔥   CJK: 蘋果 香蕉 橘子 葡萄  ｱｲｳｴｵ ＡＢＣ", St::new());
    g.put(
        32,
        0,
        "Math: ∑ ∫ ∞ ≠ ≤ ≥ → ⇒ ← ↑ ↓   Symbols: ✓ ✗ ★ ☆ ● ○ ▶ ◀ ■ □ ◆ ◇   Cyrillic: Привет  Greek: αβγδ",
        St::new().fg(idx(6)),
    );

    // status line + input box
    g.put(34, 0, &format!("╭{}╮", "─".repeat(138)), St::new().fg(idx(8)));
    g.put(35, 0, "│ > ", St::new().fg(idx(8)));
    g.put(35, 4, "試試看輸入中文，例如：ㄅㄆㄇㄈ", St::new());
    g.put(35, 139, "│", St::new().fg(idx(8)));
    g.put(36, 0, &format!("╰{}╯", "─".repeat(138)), St::new().fg(idx(8)));
    g.fill_row(38, St::new().bg(Color::Rgb(0x26, 0x26, 0x26)));
    g.put(38, 0, " ? for shortcuts", St::new().fg(idx(8)).bg(Color::Rgb(0x26, 0x26, 0x26)));
    g.put(38, 100, "Opus  ctx 42%  ⏵⏵ accept edits on", St::new().fg(idx(8)).bg(Color::Rgb(0x26, 0x26, 0x26)));
    g.put(39, 0, " NORMAL ", St::new().fg(idx(0)).bg(idx(4)).f(CellFlags::BOLD));
    g.put(39, 8, "\u{e0b0}", St::new().fg(idx(4)).bg(idx(8)));
    g.put(39, 9, " renderer.rs ", St::new().fg(idx(15)).bg(idx(8)));
    g.put(39, 22, "\u{e0b0}", St::new().fg(idx(8)));
    g
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Args {
    adapter: AdapterKind,
    out: PathBuf,
    scale: f32,
    bgra: bool,
    lcd: bool,
    hint: bool,
    bench: bool,
    ligatures: bool,
    scene: Option<String>,
    size_pt: f32,
    tag: String,
    font: Option<String>,
    atlas_initial: u32,
    atlas_max: u32,
    opacity: f32,
    correction: bool,
    gamma: f32,
}

fn parse_args() -> Args {
    let mut a = Args {
        adapter: AdapterKind::Real,
        out: PathBuf::from("out"),
        scale: 1.0,
        bgra: true,
        lcd: true,
        hint: false,
        bench: false,
        ligatures: false,
        scene: None,
        size_pt: 12.0,
        tag: String::new(),
        font: None,
        atlas_initial: 1024,
        atlas_max: 4096,
        opacity: 1.0,
        correction: true,
        gamma: 1.8,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().unwrap_or_default();
        match k.as_str() {
            "--adapter" => a.adapter = if val() == "warp" { AdapterKind::Warp } else { AdapterKind::Real },
            "--out" => a.out = PathBuf::from(val()),
            "--scale" => a.scale = val().parse().unwrap_or(1.0),
            "--format" => a.bgra = val() != "rgba",
            "--lcd" => a.lcd = val() != "off",
            "--hint" => a.hint = val() == "on",
            "--size" => a.size_pt = val().parse().unwrap_or(12.0),
            "--tag" => a.tag = val(),
            "--font" => a.font = Some(val()),
            "--atlas-initial" => a.atlas_initial = val().parse().unwrap_or(1024),
            "--atlas-max" => a.atlas_max = val().parse().unwrap_or(4096),
            "--opacity" => a.opacity = val().parse().unwrap_or(1.0),
            "--correction" => a.correction = val() != "off",
            "--gamma" => a.gamma = val().parse().unwrap_or(1.8),
            "--scene" => a.scene = Some(val()),
            "--bench" => a.bench = true,
            "--ligatures" => a.ligatures = true,
            _ => eprintln!("unknown arg {k}"),
        }
    }
    a
}

struct Harness {
    ctx: GpuContext,
    renderer: Renderer,
    format: wgpu::TextureFormat,
    query: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer)>,
    last_pixels: Option<Vec<u8>>,
    clear_alpha: f64,
}

#[derive(Default, Clone, Copy, Debug)]
struct Times {
    update_us: f64,
    record_us: f64,
    submit_us: f64,
    wall_us: f64,
    gpu_us: Option<f64>,
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}
fn p95(v: &[f64]) -> f64 {
    v[(v.len() as f64 * 0.95) as usize]
}

impl Harness {
    fn new(args: &Args, cfg: RendererConfig) -> Result<Self, String> {
        let ctx = offscreen::create_gpu(args.adapter)?;
        let format = if args.bgra { wgpu::TextureFormat::Bgra8Unorm } else { wgpu::TextureFormat::Rgba8Unorm };
        let renderer = Renderer::new(&ctx.device, &ctx.queue, format, cfg)?;
        let query = ctx.device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
            let qs = ctx.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("nk ts"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
            let resolve = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nk ts resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nk ts read"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            (qs, resolve, read)
        });
        Ok(Harness { ctx, renderer, format, query, last_pixels: None, clear_alpha: 1.0 })
    }

    fn target(&self, w: u32, h: u32) -> OffscreenTarget {
        OffscreenTarget::new(&self.ctx.device, w, h, self.format)
    }

    fn clear_color(&self, bg: Rgb) -> wgpu::Color {
        // Premultiplied: required for composition swap chains with alpha.
        let a = self.clear_alpha;
        wgpu::Color { r: bg.0 as f64 / 255.0 * a, g: bg.1 as f64 / 255.0 * a, b: bg.2 as f64 / 255.0 * a, a }
    }

    /// Record + submit + wait. Optionally save a PNG.
    fn draw(&mut self, target: &OffscreenTarget, panes: &[PaneDraw<'_>], bg: Rgb, save: Option<&PathBuf>) -> Times {
        let t0 = Instant::now();
        let mut enc =
            self.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("nk frame") });
        let ts = self.query.as_ref().map(|(qs, _, _)| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        });
        let clear = self.clear_color(bg);
        self.renderer.draw(&mut enc, &target.view, Some(clear), panes, ts);
        if let Some((qs, resolve, read)) = &self.query {
            enc.resolve_query_set(qs, 0..2, resolve, 0);
            enc.copy_buffer_to_buffer(resolve, 0, read, 0, 16);
        }
        if save.is_some() {
            target.encode_readback(&mut enc);
        }
        let cb = enc.finish();
        let record_us = t0.elapsed().as_secs_f64() * 1e6;
        self.ctx.queue.submit([cb]);
        let submit_us = t0.elapsed().as_secs_f64() * 1e6;
        let mut gpu_us = None;
        self.ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let wall_us = t0.elapsed().as_secs_f64() * 1e6;
        if let Some((_, _, read)) = &self.query {
            let slice = read.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = slice.get_mapped_range().unwrap();
            let ticks: [u64; 2] = [
                u64::from_le_bytes(data[0..8].try_into().unwrap()),
                u64::from_le_bytes(data[8..16].try_into().unwrap()),
            ];
            drop(data);
            read.unmap();
            gpu_us = Some((ticks[1].saturating_sub(ticks[0])) as f64 * self.ctx.timestamp_period as f64 / 1000.0);
        }
        if let Some(path) = save {
            let rgba = target.read_rgba(&self.ctx.device).expect("readback");
            offscreen::write_png(path, target.width, target.height, &rgba).expect("png");
            println!("  wrote {}", path.display());
            self.last_pixels = Some(rgba);
        }
        Times { update_us: 0.0, record_us, submit_us, wall_us, gpu_us }
    }
}

fn pane_size(m: &fonts::CellMetrics, cols: usize, rows: usize) -> (u32, u32) {
    (cols as u32 * m.width, rows as u32 * m.height)
}

fn main() {
    let args = parse_args();
    std::fs::create_dir_all(&args.out).ok();

    let t_start = Instant::now();
    let mut fonts = fonts::FontConfig::default();
    // Optional Nerd Font (patched Cascadia from the Rio repo checkout) for PUA icons.
    if let Ok(bytes) = std::fs::read(std::env::var("NK_NERD_FONT").unwrap_or_default()) {
        fonts.embedded.push(("CaskaydiaCove NF".into(), std::sync::Arc::new(bytes)));
    }
    if let Some(f) = &args.font {
        fonts.text.insert(0, f.clone());
    }
    let cfg = RendererConfig {
        fonts,
        font_size_pt: args.size_pt,
        scale: args.scale,
        lcd: args.lcd,
        hint: args.hint,
        ligatures: args.ligatures,
        atlas_initial: args.atlas_initial,
        atlas_max: args.atlas_max,
        text_correction: args.correction,
        gamma: args.gamma,
        ..Default::default()
    };
    let t_cfg = t_start.elapsed();
    let mut h = Harness::new(&args, cfg).expect("harness");
    let t_init = t_start.elapsed();
    let m = h.renderer.metrics();
    println!(
        "adapter: {} ({:?}) backend {:?}  features: dual_source={} timestamp={}",
        h.ctx.info.name,
        h.ctx.info.device_type,
        h.ctx.info.backend,
        h.ctx.device.features().contains(wgpu::Features::DUAL_SOURCE_BLENDING),
        h.ctx.device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
    );
    println!("init total: {:?}", t_init - t_cfg);
    for (n, d) in h.ctx.timings.iter().chain(h.renderer.init_timings.iter()) {
        println!("  init {:<56} {:?}", n, d);
    }
    println!(
        "metrics @ {}pt x{}: px={:.2} cell {}x{} baseline {} underline@{}+{} strike@{} advance={:.3} asc={:.2} desc={:.2} lead={:.2}  lcd_active={}",
        args.size_pt,
        args.scale,
        m.px_size,
        m.width,
        m.height,
        m.baseline,
        m.underline_top,
        m.underline_thickness,
        m.strike_top,
        m.advance,
        m.ascent,
        m.descent,
        m.leading,
        h.renderer.lcd_active()
    );

    let theme = Theme::campbell();
    h.renderer.set_theme(&theme);

    let want = |name: &str| args.scene.as_deref().is_none_or(|s| s == name);
    let suffix = format!("{}{}", if args.tag.is_empty() { String::new() } else { format!("-{}", args.tag) }, "");

    // ---- scene 1: 80x24 ------------------------------------------------
    if want("80x24") {
        let grid = showcase_80x24();
        let sel = [RowSpan { row: 22, start: 2, end: 27 }];
        let hl = [
            Highlight { span: RowSpan { row: 22, start: 32, end: 37 }, current: false },
            Highlight { span: RowSpan { row: 22, start: 39, end: 44 }, current: false },
            Highlight { span: RowSpan { row: 22, start: 46, end: 55 }, current: true },
            Highlight { span: RowSpan { row: 22, start: 57, end: 62 }, current: false },
        ];
        let mut view = GridView::plain(80, 24, &grid.cells);
        view.selection = &sel;
        view.highlights = &hl;
        view.cursor =
            Some(CursorView { col: 22, row: 23, shape: CursorShape::Block, visible: true, focused: true, wide: false });
        view.hover_link = Some(RowSpan { row: 22, start: 66, end: 80 });
        let mut pane = PaneState::new();
        let (w, hh) = pane_size(&m, 80, 24);
        let target = h.target(w, hh);

        let t = Instant::now();
        let st = h.renderer.update_pane(&mut pane, &view);
        let cold_update = t.elapsed();
        let path = args.out.join(format!("grid_80x24{suffix}.png"));
        let times = h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, Some(&path));
        println!(
            "80x24 {}x{}px  cold first frame: update {:?} (rows {}, bg {} glyph {} lcd {} deco {}, uploaded {} B) + draw wall {:.0}us gpu {:?}us",
            w,
            hh,
            cold_update,
            st.rows_rebuilt,
            st.bg_instances,
            st.glyph_instances,
            st.lcd_instances,
            st.deco_instances,
            st.uploaded_bytes,
            times.wall_us,
            times.gpu_us
        );
        println!("  atlas: {}", h.renderer.atlas_info());
        println!("  stats: {:?} fonts: {:?}", h.renderer.stats, h.renderer.fonts.stats);
        println!(
            "  cold-frame breakdown: update {:?} = font resolve {:?} + raster {:?} + atlas upload {:?} + rest",
            cold_update, h.renderer.stats.font_time, h.renderer.stats.raster_time, h.renderer.stats.upload_time
        );
    }

    // ---- scene 2: 140x40 -----------------------------------------------
    let mut grid140 = showcase_140x40();
    if want("140x40") {
        let mut view = GridView::plain(140, 40, &grid140.cells);
        view.cursor = Some(CursorView {
            col: 4 + 14 * 2 + 2,
            row: 35,
            shape: CursorShape::Bar,
            visible: true,
            focused: true,
            wide: false,
        });
        let scroll = Scrollbar { top: 0.2, height: 0.35, hovered: false };
        view.scrollbar = Some(scroll);
        let mut pane = PaneState::new();
        let (w, hh) = pane_size(&m, 140, 40);
        let target = h.target(w, hh);
        let t = Instant::now();
        let st = h.renderer.update_pane(&mut pane, &view);
        let cold_update = t.elapsed();
        let path = args.out.join(format!("grid_140x40{suffix}.png"));
        let times = h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, Some(&path));
        println!(
            "140x40 {}x{}px  cold: update {:?} (rows {}, bg {} glyph {} lcd {} deco {} overlay {}) draw wall {:.0}us gpu {:?}us",
            w,
            hh,
            cold_update,
            st.rows_rebuilt,
            st.bg_instances,
            st.glyph_instances,
            st.lcd_instances,
            st.deco_instances,
            st.overlay_instances,
            times.wall_us,
            times.gpu_us
        );
        println!("  atlas: {}", h.renderer.atlas_info());
    }

    // ---- scene: atlas stress (growth + clear + recovery) -----------------
    if args.scene.as_deref() == Some("stress") {
        let (cols, rows) = (140usize, 40usize);
        let (w, hh) = pane_size(&m, cols, rows);
        let target = h.target(w, hh);
        let mut pane = PaneState::new();
        let t_all = Instant::now();
        const FRAMES: usize = 14;
        for frame in 0..FRAMES {
            let mut g = Grid::new(cols, rows);
            let base = 0x4E00u32 + frame as u32 * 2800;
            let mut k = 0u32;
            for r in 0..rows {
                let mut c = 0;
                while c + 2 <= cols {
                    if let Some(ch) = char::from_u32(base + k) {
                        g.put(r, c, &ch.to_string(), St::new().fg(idx(7 + (frame % 8) as u8)));
                    }
                    c += 2;
                    k += 1;
                }
            }
            let view = GridView::plain(cols, rows, &g.cells);
            let t = Instant::now();
            let st = h.renderer.update_pane(&mut pane, &view);
            let upd = t.elapsed();
            let save = (frame == FRAMES - 1).then(|| args.out.join(format!("stress_last{suffix}.png")));
            h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, save.as_ref());
            println!(
                "frame {frame:2}: update {:>8.2?}  restarts {}  glyph+lcd instances {}  | {}",
                upd,
                st.atlas_restarts,
                st.glyph_instances + st.lcd_instances,
                h.renderer.atlas_info()
            );
        }
        // Back to the normal showcase: caches must recover after clears.
        let grid = showcase_80x24();
        let view = GridView::plain(80, 24, &grid.cells);
        let mut p2 = PaneState::new();
        let (w2, h2) = pane_size(&m, 80, 24);
        let t2 = h.target(w2, h2);
        h.renderer.update_pane(&mut p2, &view);
        h.draw(
            &t2,
            &[PaneDraw { pane: &p2, rect: [0, 0, w2, h2] }],
            theme.background,
            Some(&args.out.join(format!("stress_recovered{suffix}.png"))),
        );
        println!(
            "stress total {:?}; glyphs rasterized {}; atlas clears (r8 {}, rgba {}); {}",
            t_all.elapsed(),
            h.renderer.stats.glyphs_rasterized,
            h.renderer.stats.atlas_clears,
            h.renderer.stats.atlas_clears,
            h.renderer.atlas_info()
        );
        return;
    }

    // ---- scene: device lost / recreate -------------------------------------
    if args.scene.as_deref() == Some("devicelost") {
        use std::sync::atomic::{AtomicBool, Ordering};
        let lost = std::sync::Arc::new(AtomicBool::new(false));
        let lost2 = lost.clone();
        h.ctx.device.set_device_lost_callback(move |reason, msg| {
            println!("  device-lost callback: reason={reason:?} message={msg:?}");
            lost2.store(true, Ordering::SeqCst);
        });
        let grid = showcase_80x24();
        let view = GridView::plain(80, 24, &grid.cells);
        let (w, hh) = pane_size(&m, 80, 24);
        let mut pane = PaneState::new();
        h.renderer.update_pane(&mut pane, &view);
        let target = h.target(w, hh);
        h.draw(
            &target,
            &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
            theme.background,
            Some(&args.out.join("devicelost_before.png")),
        );
        let before = h.last_pixels.take().unwrap();
        drop(target);
        // Simulate device loss.
        h.ctx.device.destroy();
        let _ = h.ctx.device.poll(wgpu::PollType::Poll);
        std::thread::sleep(std::time::Duration::from_millis(50));
        println!("  device destroyed; callback fired: {}", lost.load(Ordering::SeqCst));
        // Recover: new device on the same adapter, GPU objects rebuilt, CPU state kept.
        let t = Instant::now();
        let ctx2 = offscreen::create_gpu(args.adapter).expect("recreate gpu");
        let t_dev = t.elapsed();
        let t = Instant::now();
        h.renderer.recreate_gpu(&ctx2.device, &ctx2.queue);
        h.query = None;
        h.ctx = ctx2;
        let t_renderer = t.elapsed();
        let t = Instant::now();
        let st = h.renderer.update_pane(&mut pane, &view);
        let t_update = t.elapsed();
        let target = h.target(w, hh);
        h.draw(
            &target,
            &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
            theme.background,
            Some(&args.out.join("devicelost_after.png")),
        );
        let after = h.last_pixels.take().unwrap();
        println!(
            "  recovery: new device {:?}, renderer.recreate_gpu {:?}, first update {:?} (rows rebuilt {}); pixels identical: {}",
            t_dev,
            t_renderer,
            t_update,
            st.rows_rebuilt,
            before == after
        );
        return;
    }

    // ---- scene: transparent background (Mica/acrylic style, premultiplied alpha) ------
    if args.scene.as_deref() == Some("transparent") {
        let mut th = theme.clone();
        th.background_opacity = args.opacity.min(0.99);
        h.renderer.set_theme(&th);
        h.clear_alpha = 0.0; // target starts fully transparent; the pane background rect supplies the opacity
        println!("  background_opacity {} -> lcd_active={}", th.background_opacity, h.renderer.lcd_active());
        let grid = showcase_80x24();
        let sel = [RowSpan { row: 22, start: 2, end: 27 }];
        let mut view = GridView::plain(80, 24, &grid.cells);
        view.selection = &sel;
        view.cursor =
            Some(CursorView { col: 22, row: 23, shape: CursorShape::Block, visible: true, focused: true, wide: false });
        let (w, hh) = pane_size(&m, 80, 24);
        let mut pane = PaneState::new();
        h.renderer.update_pane(&mut pane, &view);
        let target = h.target(w, hh);
        let tmp = args.out.join("transparent_raw.png");
        h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], th.background, Some(&tmp));
        let px = h.last_pixels.take().unwrap();
        // Composite premultiplied output over a checkerboard so the result is viewable.
        let mut out = vec![0u8; px.len()];
        for y in 0..hh as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                let checker = if ((x / 16) + (y / 16)) % 2 == 0 { 200u32 } else { 120u32 };
                let a = px[i + 3] as u32;
                for k in 0..3 {
                    out[i + k] = (px[i + k] as u32 + checker * (255 - a) / 255).min(255) as u8;
                }
                out[i + 3] = 255;
            }
        }
        let path = args.out.join(format!("transparent{suffix}.png"));
        offscreen::write_png(&path, w, hh, &out).unwrap();
        println!("  wrote {} (premultiplied alpha composited over checkerboard)", path.display());
        return;
    }

    // ---- scene: font resolution diagnostics -----------------------------------
    if args.scene.as_deref() == Some("diag") {
        let samples: &[(&str, &str, bool)] = &[
            ("latin", "A", false),
            ("latin bold", "A", false),
            ("accent", "é", false),
            ("arrow", "→", false),
            ("check", "✓", false),
            ("han (zh-TW)", "骨", true),
            ("bopomofo", "ㄅ", true),
            ("hangul", "한", true),
            ("hiragana", "あ", true),
            ("fullwidth latin", "Ａ", true),
            ("emoji 😀", "😀", true),
            ("heart + VS16", "❤\u{fe0f}", true),
            ("technologist ZWJ", "🧑\u{200d}💻", true),
            ("family ZWJ", "👨\u{200d}👩\u{200d}👧\u{200d}👦", true),
            ("flag TW", "🇹🇼", true),
            ("thumbs + skin tone", "👍🏽", true),
            ("box light", "┌", false),
            ("block", "█", false),
            ("braille", "⣿", false),
            ("powerline E0B0", "\u{e0b0}", false),
            ("nerd PUA F101", "\u{f101}", false),
            ("nerd PUA E5FA", "\u{e5fa}", false),
            ("symbol ✻", "✻", false),
            ("math ∑", "∑", false),
        ];
        let px = h.renderer.metrics().px_size;
        println!("{:<20} {:<28} {:<28} {:>6} {:>7} flags", "sample", "face family", "file", "glyph", "adv px");
        for (label, s, wide) in samples {
            let mut chars = s.chars();
            let base = chars.next().unwrap();
            let rest: Vec<char> = chars.collect();
            let style =
                if *label == "latin bold" { fonts::Style { bold: true, italic: false } } else { fonts::Style::REGULAR };
            let class = fonts::CharClass { wide: *wide, emoji_vs: rest.contains(&'\u{fe0f}') };
            match h.renderer.fonts.choice_for(base, style, class) {
                Some(c) => {
                    let f = h.renderer.fonts.face(c.face);
                    let g = f.glyph(base);
                    let adv = f.font_ref().glyph_metrics(&[]).scale(px).advance_width(g);
                    let mut note = String::new();
                    if !rest.is_empty()
                        && let Some(font) = f.shaping_font()
                    {
                        let sf = harfrust::ShaperFont::new(font);
                        let mut buf = harfrust::Buffer::new();
                        buf.push_str(s);
                        buf.guess_segment_properties();
                        let _ = harfrust::shape(&sf, &mut buf, harfrust::ShapeOptions::new());
                        note = format!(
                            "shaped {} cps -> {} glyphs {:?}",
                            s.chars().count(),
                            buf.len(),
                            buf.glyph_infos().iter().map(|i| i.glyph_id).collect::<Vec<_>>()
                        );
                    }
                    if f.has_colr || f.has_color_bitmap {
                        let mut rz = raster::Rasterizer::new();
                        if let Some(r) =
                            rz.rasterize(f, &c, g, px, 0.0, raster::RasterOptions { lcd: false, hint: false })
                        {
                            note.push_str(&format!(
                                " | ink at px={px:.0}: {}x{} left {} top {} kind {:?}",
                                r.width, r.height, r.left, r.top, r.kind
                            ));
                        }
                    }
                    println!(
                        "{:<20} {:<28} {:<28} {:>6} {:>7.2} wght={:?} synthB={} synthI={} colr={} bitmap={} {}",
                        label,
                        f.family,
                        f.path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "<embedded>".into()),
                        g,
                        adv,
                        c.wght,
                        c.synth_bold,
                        c.synth_italic,
                        f.has_colr,
                        f.has_color_bitmap,
                        note
                    );
                }
                None => println!("{:<20} NO FONT (sprite or missing)", label),
            }
        }
        println!("font stats: {:?}", h.renderer.fonts.stats);
        return;
    }
    // ---- scene: Nerd Font / PUA fallback -----------------------------------------
    if args.scene.as_deref() == Some("nerd") {
        let mut g = Grid::new(48, 6);
        g.put(0, 0, "PUA icons (embedded Nerd Font in symbols chain):", St::new().f(CellFlags::BOLD));
        g.put(2, 0, "\u{e0a0} branch  \u{e0a1} ln  \u{e0a2} lock  \u{f101} \u{f00c} \u{f07b} folder \u{f113} gh \u{e725} \u{e5fa} \u{f2db}", St::new().fg(idx(14)));
        g.put(
            3,
            0,
            "\u{e0b0}\u{e0b2} powerline from sprites; \u{e0b4}\u{e0b6} round; \u{ea60}\u{ea61}\u{ea62}",
            St::new().fg(idx(11)),
        );
        g.put(4, 0, "wide Nerd glyph needs 2 cells: \u{f1c0} \u{f121}", St::new().fg(idx(10)));
        let view = GridView::plain(48, 6, &g.cells);
        let mut pane = PaneState::new();
        let (w, hh) = pane_size(&m, 48, 6);
        let target = h.target(w, hh);
        h.renderer.update_pane(&mut pane, &view);
        h.draw(
            &target,
            &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
            theme.background,
            Some(&args.out.join(format!("nerd{suffix}.png"))),
        );
        println!("  missing glyphs: {}", h.renderer.stats.missing_glyphs);
        return;
    }
    // ---- scene: text gamma / contrast comparison ---------------------------------
    if args.scene.as_deref() == Some("gamma") {
        let mut g = Grid::new(44, 9);
        let rows: [(&str, Color, Color); 8] = [
            ("white on black   Hamburgefonstiv 0123", Color::Rgb(255, 255, 255), Color::Rgb(0, 0, 0)),
            ("black on white   Hamburgefonstiv 0123", Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255)),
            ("gray on dark     Hamburgefonstiv 0123", Color::Rgb(0x90, 0x90, 0x90), Color::Rgb(0x20, 0x20, 0x20)),
            ("dark on light    Hamburgefonstiv 0123", Color::Rgb(0x30, 0x30, 0x30), Color::Rgb(0xe0, 0xe0, 0xe0)),
            ("green on black   Hamburgefonstiv 0123", Color::Rgb(0x20, 0xd0, 0x20), Color::Rgb(0, 0, 0)),
            ("blue on black    Hamburgefonstiv 0123", Color::Rgb(0x40, 0x70, 0xff), Color::Rgb(0, 0, 0)),
            ("yellow on blue   Hamburgefonstiv 0123", Color::Rgb(0xff, 0xf0, 0x40), Color::Rgb(0x10, 0x20, 0x80)),
            ("red on white     Hamburgefonstiv 0123", Color::Rgb(0xc0, 0x10, 0x10), Color::Rgb(255, 255, 255)),
        ];
        for (i, (text, fg, bg)) in rows.iter().enumerate() {
            for c in 0..44 {
                g.cells[i * 44 + c] = Cell::default().fg(*fg).bg(*bg);
            }
            g.put(i, 0, text, St::new().fg(*fg).bg(*bg));
        }
        let view = GridView::plain(44, 9, &g.cells);
        let mut pane = PaneState::new();
        let (w, hh) = pane_size(&m, 44, 9);
        let target = h.target(w, hh);
        h.renderer.update_pane(&mut pane, &view);
        h.draw(
            &target,
            &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
            Rgb(0, 0, 0),
            Some(&args.out.join(format!("gamma{suffix}.png"))),
        );
        return;
    }
    // ---- scene: robustness / edge cases (must not panic) ---------------------------
    if args.scene.as_deref() == Some("edge") {
        let mut pane = PaneState::new();
        // 0x0 and 1x1 grids
        let empty: Vec<Cell> = Vec::new();
        let st0 = h.renderer.update_pane(&mut pane, &GridView::plain(0, 0, &empty));
        println!("  0x0 grid: rows_rebuilt {} needs_draw {}", st0.rows_rebuilt, st0.needs_draw());
        let one = vec![Cell::new('A')];
        let st1 = h.renderer.update_pane(&mut pane, &GridView::plain(1, 1, &one));
        println!("  1x1 grid: glyphs {} lcd {}", st1.glyph_instances, st1.lcd_instances);
        // wide char flagged in the last column, out-of-range cursor/selection/highlight/preedit
        let mut g = Grid::new(10, 3);
        g.put(0, 0, "abcdefghi", St::new());
        g.cells[9] = Cell::new('中').flag(CellFlags::WIDE);
        g.cells[10] = Cell { ch: ' ', flags: CellFlags(CellFlags::WIDE_SPACER), ..Default::default() };
        g.put(1, 0, "a\u{301}\u{302}\u{303} zero-width stack", St::new());
        g.cells[20] = Cell::new('\0');
        let sel = [RowSpan { row: 0, start: 7, end: 400 }, RowSpan { row: 99, start: 0, end: 3 }];
        let hl = [Highlight { span: RowSpan { row: 2, start: 5, end: 9999 }, current: true }];
        let pre = Preedit { row: 1, col: 8, text: "注音輸入法測試很長的預編輯字串".into() };
        let mut view = GridView::plain(10, 3, &g.cells);
        view.selection = &sel;
        view.highlights = &hl;
        view.preedit = Some(&pre);
        view.cursor = Some(CursorView {
            col: 500,
            row: 700,
            shape: CursorShape::Block,
            visible: true,
            focused: true,
            wide: true,
        });
        view.hover_link = Some(RowSpan { row: 0, start: 0, end: 60000 });
        view.scrollbar = Some(Scrollbar { top: 2.0, height: 5.0, hovered: false });
        let st = h.renderer.update_pane(&mut pane, &view);
        let (w, hh) = pane_size(&m, 10, 3);
        let target = h.target(w, hh);
        h.draw(
            &target,
            &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
            theme.background,
            Some(&args.out.join(format!("edge{suffix}.png"))),
        );
        println!("  edge cases ok: rows {} glyph instances {}", st.rows_total, st.glyph_instances + st.lcd_instances);
        // draw with an empty pane list and a pane that was never updated
        let never = PaneState::new();
        h.draw(&target, &[PaneDraw { pane: &never, rect: [0, 0, w, hh] }], theme.background, None);
        println!("  draw of never-updated pane ok");
        return;
    }
    // ---- scene: ligatures ------------------------------------------------
    if args.scene.as_deref() == Some("ligatures") {
        let mut g = Grid::new(64, 12);
        g.put(
            0,
            0,
            &format!("font: {}  ligatures: {}", args.font.as_deref().unwrap_or("default"), args.ligatures),
            St::new().f(CellFlags::BOLD),
        );
        g.put(2, 0, "-> => != == === !== <= >= <> :: ::: ++ -- /* */ // ///", St::new());
        g.put(3, 0, "<!-- --> |> <| <|> www ** ~= .. ... ?? := 0xFF ==> <== -<<", St::new());
        g.put(4, 0, "fn main() { let ok = a >= b && c != d || e == f; }", St::new());
        g.put(5, 0, "x |= 1; y <<= 2; z >>= 3; a ?? b; c ?. d; #[derive(Debug)]", St::new());
        g.put(6, 0, "-> => != bold ligatures", St::new().f(CellFlags::BOLD));
        g.put(7, 0, "-> => != italic ligatures", St::new().f(CellFlags::ITALIC));
        g.put(9, 0, "if (a<=b&&c>=d) {x=y;} // calt contexts: <= >= && ||", St::new().fg(idx(10)));
        let view = GridView::plain(64, 12, &g.cells);
        let mut pane = PaneState::new();
        let (w, hh) = pane_size(&m, 64, 12);
        let target = h.target(w, hh);
        let t = Instant::now();
        let st = h.renderer.update_pane(&mut pane, &view);
        println!("ligature scene: update {:?} glyphs {} lcd {}", t.elapsed(), st.glyph_instances, st.lcd_instances);
        let path = args.out.join(format!("ligatures{suffix}.png"));
        h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, Some(&path));
        return;
    }

    // ---- scene 3: two panes in one target (viewport + scissor) -----------
    if want("split") {
        let left = showcase_80x24();
        let mut right = Grid::new(60, 24);
        right.put(0, 0, "right pane: 60x24", St::new().f(CellFlags::BOLD));
        right.put(2, 0, "┌──────────────┐", St::new().fg(idx(11)));
        right.put(3, 0, "│ 獨立的視埠與 │", St::new().fg(idx(11)));
        right.put(4, 0, "│ scissor 裁切 │", St::new().fg(idx(11)));
        right.put(5, 0, "└──────────────┘", St::new().fg(idx(11)));
        right.put(7, 0, "unfocused pane: hollow cursor", St::new().fg(idx(8)));
        let mut vl = GridView::plain(80, 24, &left.cells);
        vl.cursor =
            Some(CursorView { col: 10, row: 23, shape: CursorShape::Block, visible: true, focused: true, wide: false });
        let mut vr = GridView::plain(60, 24, &right.cells);
        vr.cursor =
            Some(CursorView { col: 30, row: 7, shape: CursorShape::Block, visible: true, focused: false, wide: false });
        let (lw, lh) = pane_size(&m, 80, 24);
        let (rw, _) = pane_size(&m, 60, 24);
        let gap = 6u32;
        let (tw, th) = (lw + gap + rw, lh);
        let target = h.target(tw, th);
        let mut p1 = PaneState::new();
        let mut p2 = PaneState::new();
        h.renderer.update_pane(&mut p1, &vl);
        h.renderer.update_pane(&mut p2, &vr);
        let path = args.out.join(format!("split{suffix}.png"));
        h.draw(
            &target,
            &[PaneDraw { pane: &p1, rect: [0, 0, lw, lh] }, PaneDraw { pane: &p2, rect: [lw + gap, 0, rw, lh] }],
            Rgb(0x30, 0x30, 0x30),
            Some(&path),
        );
    }

    // ---- scene 4: cursor shapes / preedit / scrollbar --------------------
    if want("overlays") {
        let mut g = Grid::new(60, 8);
        g.put(0, 0, "cursor shapes: block bar underline hollow(unfocused)", St::new());
        g.put(2, 0, "IME preedit: 注音輸入中 → ", St::new());
        g.put(4, 0, "wide cursor over CJK: 中文", St::new());
        g.put(6, 0, "scrollbar thumb at right edge", St::new());
        let pre = Preedit { row: 2, col: 27, text: "ㄓㄨˋ ㄧㄣ".into() };
        let (w, hh) = pane_size(&m, 60, 8);
        let target = h.target(w, hh);
        let mut shots = Vec::new();
        for (i, (shape, focused)) in [
            (CursorShape::Block, true),
            (CursorShape::Bar, true),
            (CursorShape::Underline, true),
            (CursorShape::Block, false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut view = GridView::plain(60, 8, &g.cells);
            view.cursor =
                Some(CursorView { col: 15 + i as u16 * 6, row: 0, shape, visible: true, focused, wide: false });
            view.preedit = Some(&pre);
            view.scrollbar = Some(Scrollbar { top: 0.1, height: 0.4, hovered: true });
            if i == 0 {
                view.cursor = Some(CursorView {
                    col: 22,
                    row: 4,
                    shape: CursorShape::Block,
                    visible: true,
                    focused: true,
                    wide: true,
                });
            }
            shots.push((shape, focused));
            let mut pane = PaneState::new();
            h.renderer.update_pane(&mut pane, &view);
            if i == 0 {
                let path = args.out.join(format!("overlays{suffix}.png"));
                h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, Some(&path));
            }
        }
        // multiple cursors: draw all four shapes in one frame via 4 panes stacked horizontally is overkill; second image
        let mut view = GridView::plain(60, 8, &g.cells);
        view.cursor =
            Some(CursorView { col: 15, row: 0, shape: CursorShape::Bar, visible: true, focused: true, wide: false });
        let mut pane = PaneState::new();
        h.renderer.update_pane(&mut pane, &view);
        let path = args.out.join(format!("overlays_bar{suffix}.png"));
        h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, Some(&path));
    }

    // ---- bench ----------------------------------------------------------
    if args.bench {
        for (name, cols, rows) in [("80x24", 80usize, 24usize), ("140x40", 140, 40)] {
            let mut grid = if cols == 80 { showcase_80x24() } else { std::mem::replace(&mut grid140, Grid::new(1, 1)) };
            let (w, hh) = pane_size(&m, cols, rows);
            let target = h.target(w, hh);
            let mut pane = PaneState::new();
            {
                let view = GridView::plain(cols, rows, &grid.cells);
                h.renderer.update_pane(&mut pane, &view);
            }
            for _ in 0..400 {
                // Warm up (wake the GPU from its idle clocks, fill caches).
                h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, None);
            }
            const N: usize = 300;
            let mut results: Vec<(&str, Vec<Times>, usize)> = Vec::new();

            // A. full repaint: all rows rebuilt (glyph caches warm), full upload + draw.
            let mut v = Vec::new();
            let mut rows_rebuilt = 0;
            for _ in 0..N {
                h.renderer.invalidate_all();
                let t = Instant::now();
                let view = GridView::plain(cols, rows, &grid.cells);
                let st = h.renderer.update_pane(&mut pane, &view);
                rows_rebuilt = st.rows_rebuilt;
                let upd = t.elapsed().as_secs_f64() * 1e6;
                let mut tm = h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, None);
                tm.update_us = upd;
                v.push(tm);
            }
            results.push(("full repaint (all rows rebuilt)", v, rows_rebuilt));

            // B. one-line change.
            let mut v = Vec::new();
            for i in 0..N {
                let digit = char::from(b'0' + (i % 10) as u8);
                grid.cells[(rows - 1) * cols + 30] = Cell::new(digit).fg(Color::Indexed(10));
                let t = Instant::now();
                let view = GridView::plain(cols, rows, &grid.cells);
                let st = h.renderer.update_pane(&mut pane, &view);
                debug_assert_eq!(st.rows_rebuilt, 1);
                let upd = t.elapsed().as_secs_f64() * 1e6;
                let mut tm = h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, None);
                tm.update_us = upd;
                v.push(tm);
            }
            results.push(("1-line change (1 row rebuilt)", v, 1));

            // C. nothing changed: only damage detection.
            let mut v = Vec::new();
            for _ in 0..N {
                let t = Instant::now();
                let view = GridView::plain(cols, rows, &grid.cells);
                let st = h.renderer.update_pane(&mut pane, &view);
                debug_assert_eq!(st.rows_rebuilt, 0);
                v.push(Times { update_us: t.elapsed().as_secs_f64() * 1e6, ..Default::default() });
            }
            results.push(("no change (damage check only, no draw)", v, 0));

            // D. redraw without update (blink/overlay-only frames, GPU cost only).
            let mut v = Vec::new();
            for _ in 0..N {
                v.push(h.draw(&target, &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }], theme.background, None));
            }
            results.push(("draw only (buffers unchanged)", v, 0));

            // E. pipelined: update + encode + submit back to back, one wait at the end (sustained CPU cost per frame).
            {
                const P: usize = 600;
                let t = Instant::now();
                for i in 0..P {
                    let digit = char::from(b'0' + (i % 10) as u8);
                    grid.cells[(rows - 1) * cols + 30] = Cell::new(digit).fg(Color::Indexed(10));
                    let view = GridView::plain(cols, rows, &grid.cells);
                    h.renderer.update_pane(&mut pane, &view);
                    let mut enc = h.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                    h.renderer.draw(
                        &mut enc,
                        &target.view,
                        Some(h.clear_color(theme.background)),
                        &[PaneDraw { pane: &pane, rect: [0, 0, w, hh] }],
                        None,
                    );
                    h.ctx.queue.submit([enc.finish()]);
                }
                let cpu = t.elapsed();
                h.ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let total = t.elapsed();
                println!(
                    "pipelined 1-line-change frames ({name}): {} frames, CPU submit side {:.0} us/frame, until GPU idle {:.0} us/frame",
                    P,
                    cpu.as_secs_f64() * 1e6 / P as f64,
                    total.as_secs_f64() * 1e6 / P as f64
                );
            }

            println!("\n=== bench {name} ({w}x{hh} px, {N} iterations, median / p95, microseconds) ===");
            println!("{:<42} {:>9} {:>9} {:>9} {:>9} {:>9}", "case", "update", "record", "submit", "wall", "gpu");
            for (label, mut v, rebuilt) in results {
                let col = |f: &dyn Fn(&Times) -> f64| -> (f64, f64) {
                    let mut x: Vec<f64> = v.iter().map(f).collect();
                    let m = median(&mut x);
                    (m, p95(&x))
                };
                let upd = col(&|t| t.update_us);
                let rec = col(&|t| t.record_us);
                let sub = col(&|t| t.submit_us);
                let wall = col(&|t| t.wall_us);
                let gpu = if v[0].gpu_us.is_some() { col(&|t| t.gpu_us.unwrap_or(0.0)) } else { (f64::NAN, f64::NAN) };
                let _ = &mut v;
                println!(
                    "{:<42} {:>4.0}/{:<4.0} {:>4.0}/{:<4.0} {:>4.0}/{:<4.0} {:>4.0}/{:<4.0} {:>4.0}/{:<4.0}   (rows rebuilt {})",
                    label, upd.0, upd.1, rec.0, rec.1, sub.0, sub.1, wall.0, wall.1, gpu.0, gpu.1, rebuilt
                );
            }
            let (g8, grgba) = h.renderer.atlas_memory_bytes();
            println!(
                "atlas memory: r8 {} KiB, rgba {} KiB  | pane instance buffers {} KiB",
                g8 / 1024,
                grgba / 1024,
                pane.gpu_bytes() / 1024
            );
            println!("{}", h.renderer.atlas_info());
        }
    }
}
