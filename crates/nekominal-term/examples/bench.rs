//! Throughput and memory numbers for the emulator core.
//!
//! `cargo run --release --example bench` (optionally `-- 200` for the megabytes per workload).

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use nekominal_term::{
    alacritty_terminal::{
        event::VoidListener,
        term::{test::TermSize, Config, Term},
        vte::ansi::Processor,
    },
    CoreConfig, Sniffer, Snapshot, TerminalCore,
};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which `System` shares.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by this allocator with `layout`, which is `System`'s.
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` and `layout` come from this allocator, which is `System`'s.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                let now = CURRENT.fetch_add(new_size - layout.size(), Ordering::Relaxed) + new_size - layout.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn heap() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: [&str; 24] = [
    "the", "terminal", "emulator", "render", "cursor", "scrollback", "buffer", "error", "warning", "build", "finished",
    "src/main.rs", "cargo", "test", "passed", "failed", "let", "mut", "fn", "async", "Result", "Option", "TODO", "0x7fff",
];
const CJK: [&str; 8] = ["終端機", "你好世界", "測試", "繁體中文", "注音輸入", "倉頡", "游標", "捲動緩衝區"];

/// A pseudo log/compiler output stream: coloured words, wide text, the odd cursor and OSC sequence.
fn colored_text_mix(bytes: usize) -> Vec<u8> {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut out = Vec::with_capacity(bytes + 4096);
    let mut line = 0usize;
    while out.len() < bytes {
        let mut cols = 0;
        while cols < 100 {
            match rng.below(14) {
                0 => out.extend_from_slice(format!("\x1b[{}m", 31 + rng.below(7)).as_bytes()),
                1 => out.extend_from_slice(format!("\x1b[38;5;{}m", rng.below(256)).as_bytes()),
                2 => out.extend_from_slice(format!("\x1b[38;2;{};{};{}m", rng.below(256), rng.below(256), rng.below(256)).as_bytes()),
                3 => out.extend_from_slice(b"\x1b[1m"),
                4 => out.extend_from_slice(b"\x1b[0m"),
                5 => {
                    let w = CJK[rng.below(CJK.len())];
                    out.extend_from_slice(w.as_bytes());
                    cols += w.chars().count() * 2;
                    continue;
                }
                _ => {}
            }
            let w = WORDS[rng.below(WORDS.len())];
            out.extend_from_slice(w.as_bytes());
            out.push(b' ');
            cols += w.len() + 1;
        }
        out.extend_from_slice(b"\x1b[0m\x1b[K\r\n");
        line += 1;
        if line.is_multiple_of(200) {
            out.extend_from_slice(b"\x1b]8;;https://example.com/a\x1b\\link\x1b]8;;\x1b\\ \x1b]7;file:///C:/work\x1b\\\x1b]133;A\x07$ \x1b]133;B\x07\x1b]2;title\x07\r\n");
        }
    }
    out
}

fn plain_ascii(bytes: usize) -> Vec<u8> {
    let mut rng = Rng(7);
    let mut out = Vec::with_capacity(bytes + 256);
    while out.len() < bytes {
        for _ in 0..16 {
            out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
            out.push(b' ');
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A TUI that repaints the whole 200x50 screen: CUP, SGR, text, EL, per row.
fn tui_repaint(bytes: usize) -> Vec<u8> {
    let mut rng = Rng(99);
    let mut out = Vec::with_capacity(bytes + 65536);
    while out.len() < bytes {
        out.extend_from_slice(b"\x1b[?2026h\x1b[H");
        for row in 1..=50 {
            out.extend_from_slice(format!("\x1b[{row};1H\x1b[38;5;{}m\x1b[48;5;{}m", rng.below(256), rng.below(16)).as_bytes());
            for _ in 0..12 {
                out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
                out.push(b' ');
            }
            out.extend_from_slice(b"\x1b[0m\x1b[K");
        }
        out.extend_from_slice(b"\x1b[?2026l");
    }
    out
}

fn cjk_heavy(bytes: usize) -> Vec<u8> {
    let mut rng = Rng(5);
    let mut out = Vec::with_capacity(bytes + 256);
    while out.len() < bytes {
        for _ in 0..12 {
            out.extend_from_slice(CJK[rng.below(CJK.len())].as_bytes());
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

const CHUNK: usize = 64 * 1024; // nekominal-pty FRAME_MAX

fn mbps(bytes: usize, secs: f64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / secs
}

fn run_emulator_only(data: &[u8], total: usize) -> f64 {
    let mut term = Term::new(Config::default(), &TermSize::new(120, 40), VoidListener);
    let mut processor: Processor = Processor::new();
    let start = Instant::now();
    let mut fed = 0;
    while fed < total {
        for chunk in data.chunks(CHUNK) {
            processor.advance(&mut term, chunk);
            fed += chunk.len();
        }
    }
    mbps(fed, start.elapsed().as_secs_f64())
}

fn run_sniffer_only(data: &[u8], total: usize) -> f64 {
    let mut sniffer = Sniffer::new();
    let start = Instant::now();
    let mut fed = 0;
    while fed < total {
        for chunk in data.chunks(CHUNK) {
            let _ = sniffer.sniff_all(chunk);
            fed += chunk.len();
        }
    }
    mbps(fed, start.elapsed().as_secs_f64())
}

fn run_core(data: &[u8], total: usize, snapshot_every: Option<usize>) -> (f64, usize) {
    let mut core = TerminalCore::new(120, 40, CoreConfig::default());
    let mut snapshot = Snapshot::default();
    let start = Instant::now();
    let mut fed = 0;
    let mut frames = 0;
    while fed < total {
        for chunk in data.chunks(CHUNK) {
            core.feed(chunk);
            core.take_events();
            fed += chunk.len();
            if let Some(every) = snapshot_every
                && (fed / CHUNK).is_multiple_of(every)
            {
                core.snapshot(&mut snapshot);
                frames += 1;
            }
        }
    }
    (mbps(fed, start.elapsed().as_secs_f64()), frames)
}

fn memory_per_scrollback(columns: usize, lines: usize, style: &str) -> (usize, usize) {
    let before = heap();
    let mut core = TerminalCore::new(columns, 40, CoreConfig { scrollback_lines: 10_000, ..CoreConfig::default() });
    let empty = heap() - before;
    let mut rng = Rng(11);
    let mut line = Vec::new();
    for i in 0..lines {
        line.clear();
        match style {
            "plain" => {
                for _ in 0..columns / 6 {
                    line.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
                    line.push(b' ');
                }
            }
            "colored" => {
                for _ in 0..columns / 8 {
                    line.extend_from_slice(format!("\x1b[38;5;{}m", rng.below(256)).as_bytes());
                    line.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
                    line.push(b' ');
                }
            }
            "links+combining" => {
                line.extend_from_slice(format!("\x1b]8;;https://example.com/{i}\x1b\\").as_bytes());
                for _ in 0..columns / 10 {
                    line.extend_from_slice("e\u{301}x ".as_bytes());
                }
                line.extend_from_slice(b"\x1b]8;;\x1b\\");
            }
            _ => unreachable!(),
        }
        line.extend_from_slice(b"\x1b[0m\r\n");
        core.feed(&line);
    }
    core.take_events();
    (empty, heap() - before)
}

fn main() {
    let megabytes: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(200);
    let total = megabytes * 1024 * 1024;
    let block = 8 * 1024 * 1024;

    println!("== throughput, {megabytes} MiB per row, 64 KiB chunks, 120x40 terminal, release build");
    println!("{:<34} {:>14} {:>14} {:>16} {:>14}", "workload", "emulator only", "sniffer only", "TerminalCore", "core+snapshots");
    let workloads: [(&str, Vec<u8>); 4] = [
        ("coloured text mix (headline)", colored_text_mix(block)),
        ("plain ASCII text", plain_ascii(block)),
        ("TUI repaint 200x50 in sync frames", tui_repaint(block)),
        ("CJK-heavy (all wide cells)", cjk_heavy(block)),
    ];
    for (name, data) in &workloads {
        let emulator = run_emulator_only(data, total);
        let sniffer = run_sniffer_only(data, total);
        let (core, _) = run_core(data, total, None);
        let (with_snapshots, frames) = run_core(data, total, Some(4));
        println!(
            "{name:<34} {emulator:>11.0} MB/s {sniffer:>11.0} MB/s {core:>13.0} MB/s {with_snapshots:>11.0} MB/s ({frames} frames)"
        );
    }

    println!("\n== snapshot cost (copy visible grid, colours stay symbolic)");
    for (columns, rows) in [(80, 24), (120, 40), (200, 60), (300, 80)] {
        let mut core = TerminalCore::new(columns, rows, CoreConfig::default());
        let data = colored_text_mix(1 << 20);
        core.feed(&data);
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let start = Instant::now();
        let runs = 2000;
        for _ in 0..runs {
            core.snapshot(&mut snapshot);
        }
        let per = start.elapsed().as_secs_f64() / runs as f64;
        println!("{columns:>3}x{rows:<3} {:>8.1} us per snapshot ({} cells, {:.0} ns/cell)", per * 1e6, columns * rows, per * 1e9 / (columns * rows) as f64);
    }

    println!("\n== regex search over a full 10 000-line scrollback (120 columns, 1.2 M cells)");
    {
        let mut core = TerminalCore::new(120, 40, CoreConfig::default());
        core.feed(&colored_text_mix(14 * 1024 * 1024));
        println!("history {} lines", core.history_size());
        for pattern in ["TODO", "zzzzqqq", "Option", "error|warning", r"(?-u:\b)fn \w+ \d+", r"0x[0-9a-f]+ .* failed", "終端機", ".*passed.*"] {
            let start = Instant::now();
            let mut session = core.search_start(pattern).unwrap();
            let compile = start.elapsed();
            let start = Instant::now();
            let all = core.search_all(pattern, usize::MAX).unwrap();
            let all_time = start.elapsed();
            let start = Instant::now();
            let next = core.search_next(&mut session, (-(core.history_size() as i32), 0), true);
            let next_time = start.elapsed();
            let start = Instant::now();
            let prev = core.search_next(&mut session, (39, 119), false);
            let prev_time = start.elapsed();
            println!(
                "{pattern:<28} compile {:>6.2} ms | all matches {:>6} in {:>8.1} ms | next from top {:>7.2} ms ({}) | previous from bottom {:>7.2} ms ({})",
                compile.as_secs_f64() * 1e3,
                all.len(),
                all_time.as_secs_f64() * 1e3,
                next_time.as_secs_f64() * 1e3,
                if next.is_some() { "hit" } else { "miss" },
                prev_time.as_secs_f64() * 1e3,
                if prev.is_some() { "hit" } else { "miss" },
            );
        }
    }

    println!("\n== memory: heap bytes for a fresh terminal, then with the 10 000-line scrollback full");
    println!("{:<18} {:>6} {:>16} {:>22} {:>16}", "content", "cols", "empty terminal", "10 000+40 lines", "per history line");
    for style in ["plain", "colored", "links+combining"] {
        for columns in [80, 120, 200] {
            let (empty, filled) = memory_per_scrollback(columns, 10_040, style);
            let per_line = (filled - empty) as f64 / 10_040.0;
            println!(
                "{style:<18} {columns:>6} {:>12.2} MiB {:>18.2} MiB {per_line:>13.0} B",
                empty as f64 / 1048576.0,
                filled as f64 / 1048576.0
            );
        }
    }
    println!("Cell is {} bytes; Row adds a usize.", std::mem::size_of::<nekominal_term::alacritty_terminal::term::cell::Cell>());

    println!("\n== runaway OSC payload: heap growth while an OSC string never terminates (vte keeps the payload in an unbounded Vec; StringGuard caps it at 4 MiB)");
    let mut core = TerminalCore::new(80, 24, CoreConfig::default());
    let before = heap();
    core.feed(b"\x1b]2;");
    let filler = vec![b'x'; 1 << 20];
    for _ in 0..64 {
        core.feed(&filler);
    }
    println!("64 MiB of unterminated OSC payload: heap +{:.0} MiB with the guard (+257 MiB measured before it existed; two parsers hold a copy each)", (heap() - before) as f64 / 1048576.0);
    core.feed(b"\x07");
}
