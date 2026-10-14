//! ConPTY experiments, part 2: which sequences a child writes survive ConPTY and reach the host.
//! Run with: `cargo test --test conpty_forward -- --ignored --nocapture --test-threads=1`

mod common;

use std::time::Duration;

use common::*;

const ESC: &str = "\x1b";

fn probes() -> Vec<(&'static str, String)> {
    let e = ESC;
    let st = "\x1b\\";
    let bel = "\x07";
    vec![
        ("osc0-title", format!("{e}]0;nk-title{bel}")),
        ("osc2-title-st", format!("{e}]2;nk-title2{st}")),
        ("osc7-cwd", format!("{e}]7;file://PC/C:/Users/me/%E4%BD%A0{st}")),
        ("osc8-link", format!("{e}]8;id=a;https://example.com{st}link{e}]8;;{st}")),
        ("osc9-text", format!("{e}]9;hello-notify{bel}")),
        ("osc9-4-progress", format!("{e}]9;4;1;42{bel}")),
        ("osc9-9-cwd", format!("{e}]9;9;\"C:\\x y\"{bel}")),
        ("osc52-set", format!("{e}]52;c;aGVsbG8={bel}")),
        ("osc133-A", format!("{e}]133;A{bel}")),
        ("osc133-D", format!("{e}]133;D;3{bel}")),
        ("osc777-notify", format!("{e}]777;notify;T;B{bel}")),
        ("osc7501-set", format!("{e}]7501;state=working:id=a{st}")),
        ("osc7501-query", format!("{e}]7501;?{st}")),
        ("osc11-query", format!("{e}]11;?{bel}")),
        ("osc4-set", format!("{e}]4;1;rgb:ff/00/00{bel}")),
        ("osc1337-custom", format!("{e}]1337;SetMark{bel}")),
        ("osc99-kitty-notify", format!("{e}]99;i=1:d=0;hello{st}")),
        ("decset-2004", format!("{e}[?2004h")),
        ("decset-1006", format!("{e}[?1006h")),
        ("decset-1000", format!("{e}[?1000h")),
        ("decset-1003", format!("{e}[?1003h")),
        ("decset-1049", format!("{e}[?1049h{e}[?1049l")),
        ("decset-2026", format!("{e}[?2026h{e}[?2026l")),
        ("decset-2027", format!("{e}[?2027h")),
        ("decset-25l", format!("{e}[?25l{e}[?25h")),
        ("decrqm-2026", format!("{e}[?2026$p")),
        ("kitty-push", format!("{e}[>1u")),
        ("kitty-query", format!("{e}[?u")),
        ("kitty-set", format!("{e}[=5;1u")),
        ("kitty-pop", format!("{e}[<1u")),
        ("xtversion", format!("{e}[>q")),
        ("da1", format!("{e}[c")),
        ("da2", format!("{e}[>c")),
        ("dsr-5", format!("{e}[5n")),
        ("xtwinops-18t", format!("{e}[18t")),
        ("xtwinops-22t", format!("{e}[22t{e}[23t")),
        ("decscusr-beam", format!("{e}[5 q")),
        ("sgr-underline-curly", format!("{e}[4:3mx{e}[0m")),
        ("sgr-colon-truecolor", format!("{e}[38:2::1:2:3mx{e}[0m")),
        ("sgr-58-underline-color", format!("{e}[4;58;2;1;2;3mx{e}[0m")),
        ("sgr-blink-strike", format!("{e}[5;9mx{e}[0m")),
        ("sixel-dcs", format!("{e}Pq#0;2;0;0;0~-{st}")),
        ("decrqss", format!("{e}P$qm{st}")),
        ("xtgettcap", format!("{e}P+q544e{st}")),
        ("apc-kitty-graphics", format!("{e}_Ga=q,i=31,s=1,v=1,t=d,f=24;AAAA{e}\\")),
        ("bel", "\x07".to_owned()),
        ("keypad-app", format!("{e}={e}>")),
        ("decstr", format!("{e}[!p")),
        ("zwj-emoji", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}".to_owned()),
        ("cjk", "你好，世界".to_owned()),
    ]
}

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from.min(haystack.len())..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

#[test]
#[ignore = "spawns ConPTY"]
fn which_sequences_survive_conpty() {
    let probes = probes();
    let file = scratch("probes.tsv");
    let tsv: String = probes.iter().map(|(tag, seq)| format!("{tag}\t{}\n", hex(seq))).collect();
    std::fs::write(&file, tsv).unwrap();

    let args = vec!["-NoLogo".into(), "-NoProfile".into(), "-File".into(), script("forward.ps1"), file.to_string_lossy().into_owned()];
    let mut host = Host::spawn(pwsh(), &args, HostOptions::default());
    host.pump_until_exit(Duration::from_secs(40));
    println!("exit: {:?}; raw bytes: {}", host.exit, host.raw.len());

    let mut verbatim = 0;
    for (tag, seq) in &probes {
        let begin = format!("<<B:{tag}>>").into_bytes();
        let end = format!("<<E:{tag}>>").into_bytes();
        let Some(b) = find(&host.raw, &begin, 0) else {
            println!("{tag:24} MARKER MISSING");
            continue;
        };
        let start = b + begin.len();
        let Some(e) = find(&host.raw, &end, start) else {
            println!("{tag:24} END MARKER MISSING");
            continue;
        };
        let between = &host.raw[start..e];
        let same = between == seq.as_bytes();
        let contains = find(between, seq.as_bytes(), 0).is_some();
        if same {
            verbatim += 1;
        }
        let status = if same { "verbatim" } else if contains { "contains" } else if between.is_empty() { "SWALLOWED" } else { "CHANGED" };
        println!("{tag:24} {status:9} -> {}", escape(between));
    }
    println!("verbatim: {verbatim}/{}", probes.len());
    println!("replies produced by the emulator for the probes: {}", escape(&host.replies));
    println!("core events (everything except PtyWrite):");
    for event in &host.events {
        println!("   {event}");
    }
}
