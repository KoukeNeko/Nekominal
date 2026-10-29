//! ConPTY experiments, part 4: which `PSEUDOCONSOLE_GLYPH_WIDTH_*` mode keeps the shell's cursor and
//! this emulator's cursor in the same column?
//!
//! For each case the child prints the text on an empty row and reports `[Console]::CursorLeft` (ConPTY's
//! own idea of the cursor). The same run feeds ConPTY's output to `TerminalCore` one byte at a time
//! and samples the emulator cursor when the child's title marker arrives.
//!
//! Run with: `cargo test --release --test conpty_widths -- --ignored --nocapture --test-threads=1`

mod common;

use std::{collections::BTreeMap, time::{Duration, Instant}};

use common::{raw_conpty::*, *};
use nekominal_term::{CoreConfig, CoreEvent, TerminalCore};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct Case {
    category: String,
    label: String,
    text: String,
}

fn cps(label: &str, category: &str, cps: &[u32]) -> Case {
    let text: String = cps.iter().map(|c| char::from_u32(*c).unwrap()).collect();
    Case { category: category.to_owned(), label: format!("{label} [{}]", cps.iter().map(|c| format!("{c:X}")).collect::<Vec<_>>().join(" ")), text }
}

fn cases() -> Vec<Case> {
    let mut cases = vec![
        cps("ascii", "curated", &[0x61, 0x62, 0x63]),
        cps("cjk ideographs", "curated", &[0x4F60, 0x597D]),
        cps("fullwidth comma", "curated", &[0xFF0C]),
        cps("corner brackets", "curated", &[0x300C, 0x300D]),
        cps("halfwidth katakana", "curated", &[0xFF71]),
        cps("hangul syllable", "curated", &[0xD55C]),
        cps("box drawing light", "curated", &[0x2500]),
        cps("full block", "curated", &[0x2588]),
        cps("degree sign (ambiguous)", "curated", &[0xB0]),
        cps("greek alpha (ambiguous)", "curated", &[0x3B1]),
        cps("circled digit 1 (ambiguous)", "curated", &[0x2460]),
        cps("bullet (ambiguous)", "curated", &[0x2022]),
        cps("right arrow (ambiguous)", "curated", &[0x2192]),
        cps("ellipsis (ambiguous)", "curated", &[0x2026]),
        cps("curly quotes (ambiguous)", "curated", &[0x201C, 0x201D]),
        cps("emoji grinning", "curated", &[0x1F600]),
        cps("emoji rocket", "curated", &[0x1F680]),
        cps("heart text-default", "curated", &[0x2764]),
        cps("heart + VS16", "curated", &[0x2764, 0xFE0F]),
        cps("sun + VS16", "curated", &[0x2600, 0xFE0F]),
        cps("check mark + VS16", "curated", &[0x2714, 0xFE0F]),
        cps("star (emoji-presentation)", "curated", &[0x2B50]),
        cps("zap (emoji-presentation)", "curated", &[0x26A1]),
        cps("keycap 1", "curated", &[0x31, 0xFE0F, 0x20E3]),
        cps("flag TW (2 RIs)", "curated", &[0x1F1F9, 0x1F1FC]),
        cps("thumbs up + skin tone", "curated", &[0x1F44D, 0x1F3FD]),
        cps("ZWJ family", "curated", &[0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467]),
        cps("ZWJ couple with heart", "curated", &[0x1F469, 0x200D, 0x2764, 0xFE0F, 0x200D, 0x1F468]),
        cps("ZWJ rainbow flag", "curated", &[0x1F3F3, 0xFE0F, 0x200D, 0x1F308]),
        cps("ZWJ technologist", "curated", &[0x1F9D1, 0x200D, 0x1F4BB]),
        cps("tag sequence England", "curated", &[0x1F3F4, 0xE0067, 0xE0062, 0xE0065, 0xE006E, 0xE0067, 0xE007F]),
        cps("e + combining acute", "curated", &[0x65, 0x301]),
        cps("combining acute alone", "curated", &[0x301]),
        cps("hangul jamo L+V+T", "curated", &[0x1112, 0x1161, 0x11AB]),
        cps("devanagari ka+virama+ssa", "curated", &[0x915, 0x94D, 0x937]),
        cps("thai ko kai + tone", "curated", &[0xE01, 0xE48]),
        cps("zero width space", "curated", &[0x200B]),
        cps("zero width joiner alone", "curated", &[0x200D]),
        cps("VS16 alone", "curated", &[0xFE0F]),
        cps("ideographic space", "curated", &[0x3000]),
        cps("arabic lam+alef", "curated", &[0x644, 0x627]),
        // Issue #3: Devanagari words, and an X typed at each place inside हिन्दी.
        cps("devanagari ka+i", "curated", &[0x915, 0x93F]),
        cps("devanagari namaste", "curated", &[0x928, 0x92E, 0x938, 0x94D, 0x924, 0x947]),
        cps("devanagari hindi", "curated", &[0x939, 0x93F, 0x928, 0x94D, 0x926, 0x940]),
        cps("devanagari ka+nukta", "curated", &[0x915, 0x93C]),
        cps("devanagari ha+candrabindu", "curated", &[0x939, 0x901]),
        cps("hindi X before ii", "curated", &[0x939, 0x93F, 0x928, 0x94D, 0x926, 0x58, 0x940]),
        cps("hindi X before da", "curated", &[0x939, 0x93F, 0x928, 0x94D, 0x58, 0x926, 0x940]),
        cps("hindi X before virama", "curated", &[0x939, 0x93F, 0x928, 0x58, 0x94D, 0x926, 0x940]),
        cps("hindi X before na", "curated", &[0x939, 0x93F, 0x58, 0x928, 0x94D, 0x926, 0x940]),
        cps("hindi X before i", "curated", &[0x939, 0x58, 0x93F, 0x928, 0x94D, 0x926, 0x940]),
        // A virama joins the consonant after it whatever stands before it; the vowel sign makes the join show in the width.
        cps("probe X virama da ii", "curated", &[0x58, 0x94D, 0x926, 0x940]),
        cps("probe ka virama da ii", "curated", &[0x915, 0x94D, 0x926, 0x940]),
        cps("probe virama da ii", "curated", &[0x94D, 0x926, 0x940]),
        cps("probe space virama da ii", "curated", &[0x20, 0x94D, 0x926, 0x940]),
        cps("probe a acute virama da ii", "curated", &[0x61, 0x301, 0x94D, 0x926, 0x940]),
        cps("probe X virama X ii", "curated", &[0x58, 0x94D, 0x58, 0x940]),
        cps("probe X virama bengali ka aa", "curated", &[0x58, 0x9CD, 0x995, 0x9BE]),
        cps("probe na X virama da ii", "curated", &[0x928, 0x58, 0x94D, 0x926, 0x940]),
    ];

    let sweeps: &[(&str, u32, u32)] = &[
        ("latin1-supplement", 0xA1, 0xFF),
        ("combining-diacritics", 0x300, 0x36F),
        ("greek", 0x391, 0x3C9),
        ("cyrillic", 0x410, 0x44F),
        ("hangul-jamo", 0x1100, 0x11FF),
        ("general-punctuation", 0x2010, 0x205E),
        ("arrows", 0x2190, 0x21FF),
        ("math-operators", 0x2200, 0x22FF),
        ("misc-technical", 0x2300, 0x23FF),
        ("enclosed-alphanumerics", 0x2460, 0x24FF),
        ("box-and-blocks", 0x2500, 0x259F),
        ("geometric-shapes", 0x25A0, 0x25FF),
        ("misc-symbols", 0x2600, 0x26FF),
        ("dingbats", 0x2700, 0x27BF),
        ("misc-symbols-arrows", 0x2B00, 0x2BFF),
        ("cjk-punctuation", 0x3000, 0x303F),
        ("kana", 0x3040, 0x30FF),
        ("variation-selectors", 0xFE00, 0xFE0F),
        ("halfwidth-fullwidth", 0xFF00, 0xFFEF),
        ("enclosed-alnum-supp", 0x1F100, 0x1F1FF),
        ("emoji-misc-pictographs", 0x1F300, 0x1F64F),
        ("emoji-transport", 0x1F680, 0x1F6FF),
        ("emoji-supplemental", 0x1F900, 0x1F9FF),
        ("emoji-extended-a", 0x1FA70, 0x1FAFF),
        ("cjk-ext-b", 0x20000, 0x2001F),
        ("tag-characters", 0xE0020, 0xE007F),
    ];
    for (name, start, end) in sweeps {
        for cp in *start..=*end {
            if let Some(c) = char::from_u32(cp) {
                cases.push(Case { category: format!("sweep:{name}"), label: format!("U+{cp:04X}"), text: c.to_string() });
            }
        }
    }
    // text-presentation emoji followed by VS16 (the sequences that decide narrow vs wide emoji)
    let vs16: Vec<u32> = (0x2600..=0x27BF).chain([0xA9, 0xAE, 0x203C, 0x2049, 0x2122, 0x2139, 0x231A, 0x231B, 0x2328, 0x23CF, 0x24C2, 0x25B6, 0x25C0, 0x2934, 0x2B05, 0x2B1B, 0x2B50, 0x2B55, 0x3030, 0x303D, 0x3297, 0x3299]).chain(0x2194..=0x2199).chain(0x23E9..=0x23F3).chain(0x25AA..=0x25AB).chain(0x25FB..=0x25FE).collect();
    for cp in vs16 {
        let c = char::from_u32(cp).unwrap();
        cases.push(Case { category: "sweep:base+VS16".into(), label: format!("U+{cp:04X}+FE0F"), text: format!("{c}\u{fe0f}") });
    }
    for n in [96usize, 97, 98, 99, 100] {
        let fill = "a".repeat(n);
        for (name, tail) in [("none", ""), ("wide CJK", "你"), ("emoji", "\u{1f600}"), ("ZWJ family", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"), ("e+acute", "e\u{301}"), ("wide then b", "你b"), ("b", "b")] {
            cases.push(Case { category: "wrap-edge".into(), label: format!("{n} x a + {name}"), text: format!("{fill}{tail}") });
        }
    }
    cases
}

struct Outcome {
    /// ConPTY's console buffer cursor column after the text.
    console: usize,
    console_top: usize,
    /// Emulator cursor column when the title marker arrived.
    emulator: Option<usize>,
    emulator_row: Option<usize>,
}

fn run_mode(flags: u32, cases: &[Case]) -> (Vec<Outcome>, Duration) {
    let started = Instant::now();
    let cases_file = scratch("width-cases.tsv");
    let out_file = scratch(&format!("width-out-{flags:02x}.tsv"));
    let _ = std::fs::remove_file(&out_file);
    let tsv: String = cases
        .iter()
        .map(|c| format!("{}\t{}\n", c.label.replace('\t', " "), c.text.chars().map(|ch| format!("{:X}", ch as u32)).collect::<Vec<_>>().join(" ")))
        .collect();
    std::fs::write(&cases_file, tsv).unwrap();

    let args = vec![
        "-NoLogo".to_owned(),
        "-NoProfile".to_owned(),
        "-File".to_owned(),
        script("widths.ps1"),
        cases_file.to_string_lossy().into_owned(),
        out_file.to_string_lossy().into_owned(),
    ];
    // NK_WIDTH_BACKEND=system uses the in-box kernel32 ConPTY instead of the bundled pair.
    let mut pty = if std::env::var("NK_WIDTH_BACKEND").as_deref() == Ok("system") {
        RawPty::spawn_system(&pwsh(), &args, 100, 30, flags).expect("spawn")
    } else {
        RawPty::spawn(&pwsh(), &args, 100, 30, flags).expect("spawn")
    };
    let mut core = TerminalCore::new(100, 30, CoreConfig::default());
    let mut markers: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    pty.run(Duration::from_secs(120), |chunk| {
        let mut reply = Vec::new();
        for byte in chunk {
            core.feed(std::slice::from_ref(byte));
            for event in core.take_events() {
                match event {
                    CoreEvent::PtyWrite(bytes) => reply.extend(bytes),
                    CoreEvent::Title(title) => {
                        if let Some(rest) = title.strip_prefix('W') {
                            let index: usize = rest.split(';').next().unwrap().parse().unwrap();
                            let (row, column) = core.cursor_cell();
                            markers.insert(index, (row, column));
                        }
                    }
                    _ => {}
                }
            }
        }
        reply
    });
    drop(pty);

    let out = std::fs::read_to_string(&out_file).unwrap_or_default();
    let outcomes = out
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let console: usize = line.split('\t').nth(1).unwrap().parse().unwrap();
            let console_top: usize = line.split('\t').nth(2).unwrap().parse().unwrap();
            let marker = markers.get(&index).copied();
            Outcome { console, console_top, emulator: marker.map(|m| m.1), emulator_row: marker.map(|m| m.0) }
        })
        .collect();
    (outcomes, started.elapsed())
}

fn emulator_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[test]
#[ignore = "spawns ConPTY, takes about a minute"]
fn glyph_width_modes_against_the_emulator() {
    let cases = cases();
    println!("{} cases", cases.len());
    let modes: [(&str, u32); 7] = [
        ("0x00 default", GLYPH_WIDTH_DEFAULT),
        ("0x08 graphemes", GLYPH_WIDTH_GRAPHEMES),
        ("0x10 wcswidth", GLYPH_WIDTH_WCSWIDTH),
        ("0x18 console", GLYPH_WIDTH_CONSOLE),
        ("0x28 graphemes+ambig-wide", GLYPH_WIDTH_GRAPHEMES | AMBIGUOUS_IS_WIDE),
        ("0x30 wcswidth+ambig-wide", GLYPH_WIDTH_WCSWIDTH | AMBIGUOUS_IS_WIDE),
        ("0x38 console+ambig-wide", GLYPH_WIDTH_CONSOLE | AMBIGUOUS_IS_WIDE),
    ];

    // NK_WIDTH_MODES=0x10,0x08 restricts the run (hex flag values).
    let only: Option<Vec<u32>> = std::env::var("NK_WIDTH_MODES").ok().map(|v| {
        v.split(',').map(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap()).collect()
    });
    let mut results = Vec::new();
    for (name, flags) in modes {
        if only.as_ref().is_some_and(|o| !o.contains(&flags)) {
            continue;
        }
        let (outcomes, took) = run_mode(flags, &cases);
        println!("mode {name}: {} results in {took:.1?}", outcomes.len());
        results.push((name, outcomes));
    }

    // Per category: how many cases disagree with the emulator's per-char sum, per mode.
    let mut categories: Vec<String> = cases.iter().map(|c| c.category.clone()).collect();
    categories.dedup();
    println!("\n== ConPTY console cursor vs emulator model (sum of unicode-width char widths); count of disagreements");
    print!("{:28} {:>5}", "category", "n");
    for (name, _) in &results {
        print!(" | {:>12}", name.split(' ').next().unwrap());
    }
    println!();
    for category in &categories {
        let indices: Vec<usize> = (0..cases.len()).filter(|i| &cases[*i].category == category).collect();
        print!("{category:28} {:>5}", indices.len());
        for (_, outcomes) in &results {
            let bad = indices.iter().filter(|&&i| outcomes.get(i).is_some_and(|o| o.console != emulator_width(&cases[i].text))).count();
            print!(" | {bad:>12}");
        }
        println!();
    }

    println!("\n== same, but against unicode-width's string width (VS16/ZWJ/keycap aware)");
    for category in &categories {
        let indices: Vec<usize> = (0..cases.len()).filter(|i| &cases[*i].category == category).collect();
        print!("{category:28} {:>5}", indices.len());
        for (_, outcomes) in &results {
            let bad = indices.iter().filter(|&&i| outcomes.get(i).is_some_and(|o| o.console != cases[i].text.width())).count();
            print!(" | {bad:>12}");
        }
        println!();
    }

    println!("\n== end to end: emulator cursor at the marker vs ConPTY console cursor; count of disagreements (missing markers in brackets)");
    for category in &categories {
        let indices: Vec<usize> = (0..cases.len()).filter(|i| &cases[*i].category == category).collect();
        print!("{category:28} {:>5}", indices.len());
        for (_, outcomes) in &results {
            let bad = indices.iter().filter(|&&i| outcomes.get(i).is_some_and(|o| o.emulator.is_some_and(|e| e != o.console))).count();
            let missing = indices.iter().filter(|&&i| outcomes.get(i).is_some_and(|o| o.emulator.is_none())).count();
            print!(" | {:>12}", if missing > 0 { format!("{bad} [{missing}]") } else { bad.to_string() });
        }
        println!();
    }

    println!("\n== curated cases: ConPTY console column per mode (emulator model / string model in front)");
    println!("{:46} {:>4} {:>4} | {}", "case", "emu", "str", results.iter().map(|(n, _)| n.split(' ').next().unwrap().to_owned()).collect::<Vec<_>>().join(" "));
    for (i, case) in cases.iter().enumerate().filter(|(_, c)| c.category == "curated") {
        let cols: Vec<String> = results
            .iter()
            .map(|(_, o)| o.get(i).map(|o| format!("{}{}", o.console, match o.emulator { Some(e) if e != o.console => format!("!{e}"), _ => String::new() })).unwrap_or_default())
            .collect();
        println!("{:46} {:>4} {:>4} | {}", case.label, emulator_width(&case.text), case.text.width(), cols.join("    "));
    }

    println!("\n== totals over all {} cases (disagreement with the emulator model)", cases.len());
    for (name, outcomes) in &results {
        let all = (0..cases.len()).filter(|&i| outcomes[i].console != emulator_width(&cases[i].text)).count();
        // wrap-edge cases are excluded: after a wrap the column no longer equals the width sum.
        let starts_with_base = |i: usize| cases[i].category != "wrap-edge" && cases[i].text.chars().next().and_then(|c| c.width()).unwrap_or(0) > 0;
        let with_base = (0..cases.len()).filter(|&i| starts_with_base(i)).filter(|&i| outcomes[i].console != emulator_width(&cases[i].text)).count();
        let total_base = (0..cases.len()).filter(|&i| starts_with_base(i)).count();
        println!("  {name:28} all: {all:>4}   cases that start with a visible base char: {with_base:>3} of {total_base}");
    }

    println!("\n== wrap at the right margin (100 columns): (row,col) ConPTY vs emulator; ! marks a disagreement");
    for (i, case) in cases.iter().enumerate().filter(|(_, c)| c.category == "wrap-edge") {
        let cols: Vec<String> = results
            .iter()
            .map(|(_, o)| {
                let o = &o[i];
                let console = (o.console_top, o.console);
                let emulator = o.emulator_row.zip(o.emulator);
                format!("{:?}{}", console, match emulator { Some(e) if e != console => format!("!{e:?}"), _ => String::new() })
            })
            .collect();
        println!("  {:34} {}", case.label, cols.join("  "));
    }

    println!("\n== wcswidth (0x10): every disagreement on a case that starts with a visible base char");
    for (name, outcomes) in results.iter().filter(|(n, _)| n.starts_with("0x10")) {
        for (i, case) in cases.iter().enumerate() {
            let starts_visible = case.text.chars().next().and_then(|c| c.width()).unwrap_or(0) > 0;
            if starts_visible && outcomes[i].console != emulator_width(&case.text) && case.category != "wrap-edge" {
                println!("  {name} {} {:?}: console {} emulator-model {} string-model {}", case.category, case.label, outcomes[i].console, emulator_width(&case.text), case.text.width());
            }
        }
    }

    println!("\n== first disagreements per mode (console vs emulator model), non-curated");
    for (name, outcomes) in &results {
        let mut shown = 0;
        for (i, case) in cases.iter().enumerate().filter(|(_, c)| c.category != "curated") {
            if let Some(o) = outcomes.get(i)
                && o.console != emulator_width(&case.text)
                && shown < 8
            {
                println!("  {name:28} {} {:?}: console {} emulator-model {}", case.category, case.label, o.console, emulator_width(&case.text));
                shown += 1;
            }
        }
    }
}
