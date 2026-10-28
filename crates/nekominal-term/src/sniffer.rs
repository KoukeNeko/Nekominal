//! The sniffer: a second `vte::Parser` that watches the same byte stream as the emulator and turns the
//! sequences alacritty_terminal ignores into typed events.
//!
//! * It only implements `osc_dispatch`, `csi_dispatch`, `esc_dispatch` and `hook` of `vte::Perform`;
//!   printing and C0 controls are no-ops, so it costs little next to the emulator.
//! * vte keeps parser state between `advance` calls, so a sequence split across PTY reads (or across
//!   one byte at a time) is reported exactly once.
//! * `terminated()` makes `Parser::advance_until_terminated` return right after a sequence that needs
//!   ordering against the emulator (queries that need a reply, OSC 133 marks). The caller feeds the
//!   emulator the same prefix, collects its replies, then the sniffer's, and continues. That keeps
//!   replies in stream order (e.g. `CSI ? u` answered before the `CSI c` that follows it).
//! * A Sixel string is decoded while it streams through `put` (`sixel`). APC strings, which vte reads and drops,
//!   are cut out of the stream beside it (`Apc`): the kitty graphics protocol travels in them.

use alacritty_terminal::vte::{Params, Parser, Perform};
use memchr::{memchr, memchr3};

use crate::agent_status::{parse_report, Report, MAX_SEQUENCE_LENGTH};
use crate::kitty::MAX_TRANSFER;
use crate::osc::{
    parse_osc133, parse_osc7, parse_osc777, parse_osc9, strip_control, Notification, Osc9,
    Progress, PromptMark,
};
use crate::sixel::{default_palette, SixelDecoder, SixelImage};

/// A graphics feature a program asked for that the terminal answers with an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// kitty graphics animation (`a=f`, `a=a`, `a=c`).
    KittyAnimation,
    /// kitty graphics Unicode placeholders (`U=1`).
    KittyPlaceholder,
}

/// A Sixel mode alacritty_terminal does not know: DECSET 80 (DECSDM, Sixel display mode: pictures go to the top left
/// and nothing scrolls) and 8452 (the cursor ends right of a picture instead of below it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SixelMode {
    Display,
    CursorRight,
}

/// A mouse reporting mode alacritty_terminal ignores: DECSET 9 (X10 reporting), 1015 (urxvt encoding) and
/// 1016 (SGR encoding with pixel positions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseExtension {
    X10,
    Urxvt,
    SgrPixels,
}

impl MouseExtension {
    fn from_mode(mode: u16) -> Option<Self> {
        match mode {
            9 => Some(Self::X10),
            1015 => Some(Self::Urxvt),
            1016 => Some(Self::SgrPixels),
            _ => None,
        }
    }
}

/// Something the emulator does not surface, in typed form.
#[derive(Clone, Debug, PartialEq)]
pub enum Sniffed {
    /// OSC 7 (`file://` URI) or ConEmu OSC 9;9.
    Cwd(String),
    /// OSC 9 free text (empty title) or OSC 777 `notify`.
    Notification(Notification),
    /// ConEmu OSC 9;4. `None` clears.
    Progress(Option<Progress>),
    /// OSC 133 semantic prompt mark.
    PromptMark(PromptMark),
    /// OSC 7501 `set` or `clear` (queries are reported as `AgentQuery`).
    Agent(Report),
    /// OSC 7501 ; ? : the terminal must reply `OSC 7501 ; ? ST` before any later DA1 reply.
    AgentQuery { bell_terminated: bool },
    /// `CSI > q` (XTVERSION).
    XtVersionQuery,
    /// `CSI ? 996 n` (colour scheme query).
    ColorSchemeQuery,
    /// `DCS $ q Pt ST` (DECRQSS).
    DecrqssQuery,
    /// `DCS + q Pt ST` (XTGETTCAP).
    XtgettcapQuery,
    /// `ESC c` (RIS). The emulator resets itself; the host must reset what it keeps on the side.
    FullReset,
    /// `CSI ? 9001 h/l` (win32-input-mode). ConPTY asks for it at startup; alacritty ignores the mode.
    Win32InputMode(bool),
    /// `CSI ? 9|1015|1016 h/l`.
    MouseExtension(MouseExtension, bool),
    /// `CSI ? 1000|1002|1003 h/l`: alacritty_terminal tracks these, but they replace X10 reporting.
    MouseTracking,
    /// `CSI ? 9|1015|1016 $ p` (DECRQM): alacritty_terminal answers "not recognized"; the host answers.
    MouseModeQuery(MouseExtension),
    /// `CSI ! p` (DECSTR). alacritty_terminal ignores it; the host applies a soft reset.
    SoftReset,
    /// `CSI 3 J`: the scrollback is cleared.
    ClearScrollback,
    /// A Sixel picture (`DCS P1 ; P2 ; P3 q ... ST`), decoded; it goes at the cursor.
    Sixel(SixelImage),
    /// `CSI ? 80|8452 h/l`.
    SixelMode(SixelMode, bool),
    /// An APC string that starts with `G`: a kitty graphics command, without the `G`.
    KittyGraphics(Vec<u8>),
    /// `CSI ? Pi ; Pa ; Pv S` (XTSMGRAPHICS): a query of the colour registers (`Pi` 1) or the Sixel geometry (2).
    GraphicsAttributes { item: u16, action: u16 },
    /// `CSI 16 t`: the size of a cell in pixels.
    CellSizeQuery,
}

impl Sniffed {
    /// Events after which the caller must let the emulator catch up before the next byte.
    fn needs_ordering(&self) -> bool {
        matches!(
            self,
            Sniffed::PromptMark(_)
                | Sniffed::AgentQuery { .. }
                | Sniffed::XtVersionQuery
                | Sniffed::ColorSchemeQuery
                | Sniffed::DecrqssQuery
                | Sniffed::XtgettcapQuery
                | Sniffed::MouseModeQuery(_)
                | Sniffed::SoftReset
                | Sniffed::Sixel(_)
                | Sniffed::KittyGraphics(_)
                | Sniffed::GraphicsAttributes { .. }
                | Sniffed::CellSizeQuery
        )
    }
}

#[derive(Default)]
struct Collector {
    events: Vec<Sniffed>,
    stop: bool,
    /// The Sixel string being read.
    sixel: Option<SixelDecoder>,
    /// DECRST 1070: the colour registers are shared, and a picture starts with the ones the last one left.
    shared_registers: bool,
    shared_palette: Option<Vec<[u8; 4]>>,
    sixel_budget: SixelBudget,
}

/// The pixel writes Sixel pictures may make, refilled with time: a few bytes of Sixel can ask for millions of pixels, and
/// a stream of such pictures would hold the terminal for minutes. A whole 4096 x 4096 picture fits several times over.
struct SixelBudget {
    left: f64,
    at: Option<std::time::Instant>,
}

const SIXEL_PIXELS_PER_SECOND: f64 = 64.0 * 1024.0 * 1024.0;
const SIXEL_PIXELS_AT_ONCE: f64 = 128.0 * 1024.0 * 1024.0;

impl Default for SixelBudget {
    fn default() -> Self {
        Self { left: SIXEL_PIXELS_AT_ONCE, at: None }
    }
}

impl SixelBudget {
    /// What a picture starting now may write.
    fn allowance(&mut self) -> usize {
        let now = std::time::Instant::now();
        if let Some(at) = self.at {
            self.left = (self.left + now.duration_since(at).as_secs_f64() * SIXEL_PIXELS_PER_SECOND).min(SIXEL_PIXELS_AT_ONCE);
        }
        self.at = Some(now);
        self.left as usize
    }

    fn spend(&mut self, cost: usize) {
        self.left = (self.left - cost as f64).max(0.0);
    }
}

impl Collector {
    fn push(&mut self, event: Sniffed) {
        self.stop |= event.needs_ordering();
        self.events.push(event);
    }
}

/// OSC parameters are split on `;` by vte; the legacy parsers expect the original payload.
fn join_params(params: &[&[u8]]) -> Vec<u8> {
    params.join(&b';')
}

fn first_param(params: &Params) -> Option<u16> {
    params.iter().next().and_then(|sub| sub.first().copied())
}

impl Perform for Collector {
    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        let Some((code, rest)) = params.split_first() else {
            return;
        };
        match *code {
            b"7" => {
                let payload = String::from_utf8_lossy(&join_params(rest)).into_owned();
                if let Some(path) = parse_osc7(&payload) {
                    self.push(Sniffed::Cwd(path));
                }
            }
            b"9" => {
                let payload = String::from_utf8_lossy(&join_params(rest)).into_owned();
                match parse_osc9(&payload) {
                    Osc9::Cwd(path) => self.push(Sniffed::Cwd(path)),
                    Osc9::Progress(progress) => self.push(Sniffed::Progress(progress)),
                    Osc9::Notification(text) => self.push(Sniffed::Notification(Notification {
                        title: String::new(),
                        body: strip_control(&text),
                    })),
                    Osc9::Ignored => {}
                }
            }
            b"133" => {
                let payload = String::from_utf8_lossy(&join_params(rest)).into_owned();
                if let Some(mark) = parse_osc133(&payload) {
                    self.push(Sniffed::PromptMark(mark));
                }
            }
            b"777" => {
                let payload = String::from_utf8_lossy(&join_params(rest)).into_owned();
                if let Some(n) = parse_osc777(&payload) {
                    self.push(Sniffed::Notification(Notification {
                        title: strip_control(&n.title),
                        body: strip_control(&n.body),
                    }));
                }
            }
            b"7501" => {
                // Strict UTF-8: a report with invalid bytes is malformed, not "repaired".
                let raw = join_params(rest);
                if raw.len() > MAX_SEQUENCE_LENGTH {
                    return;
                }
                let Ok(payload) = String::from_utf8(raw) else {
                    return;
                };
                match parse_report(&payload) {
                    Some(Report::Query) => self.push(Sniffed::AgentQuery { bell_terminated }),
                    Some(report) => self.push(Sniffed::Agent(report)),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match (action, intermediates) {
            // XTVERSION: `CSI > q` or `CSI > 0 q`.
            ('q', [b'>']) if first_param(params).unwrap_or(0) == 0 => {
                self.push(Sniffed::XtVersionQuery)
            }
            // DECSET / DECRST of the modes alacritty_terminal does not track, possibly among other modes.
            ('h' | 'l', [b'?']) => {
                let on = action == 'h';
                if params.iter().any(|p| p.first() == Some(&9001)) {
                    self.push(Sniffed::Win32InputMode(on));
                }
                for mode in params.iter().filter_map(|p| p.first().copied()) {
                    if let Some(extension) = MouseExtension::from_mode(mode) {
                        self.push(Sniffed::MouseExtension(extension, on));
                    } else if matches!(mode, 1000 | 1002 | 1003) {
                        self.push(Sniffed::MouseTracking);
                    } else if mode == 80 {
                        self.push(Sniffed::SixelMode(SixelMode::Display, on));
                    } else if mode == 8452 {
                        self.push(Sniffed::SixelMode(SixelMode::CursorRight, on));
                    } else if mode == 1070 {
                        // Set: every picture has registers of its own (xterm's default).
                        self.shared_registers = !on;
                        self.shared_palette = None;
                    }
                }
            }
            ('S', [b'?']) => {
                let mut values = params.iter().map(|p| p.first().copied().unwrap_or(0));
                let (item, action) = (values.next().unwrap_or(0), values.next().unwrap_or(0));
                self.push(Sniffed::GraphicsAttributes { item, action });
            }
            ('t', []) if first_param(params) == Some(16) => self.push(Sniffed::CellSizeQuery),
            // DECRQM for a mouse mode alacritty_terminal does not know.
            ('p', [b'?', b'$']) => {
                if let Some(extension) = first_param(params).and_then(MouseExtension::from_mode) {
                    self.push(Sniffed::MouseModeQuery(extension));
                }
            }
            ('p', [b'!']) => self.push(Sniffed::SoftReset),
            ('J', []) if first_param(params) == Some(3) => self.push(Sniffed::ClearScrollback),
            // Colour scheme query (contour / kitty extension): `CSI ? 996 n`.
            ('n', [b'?']) if first_param(params) == Some(996) => {
                self.push(Sniffed::ColorSchemeQuery)
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if !ignore && intermediates.is_empty() && byte == b'c' {
            self.shared_registers = false;
            self.shared_palette = None;
            self.push(Sniffed::FullReset);
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match (intermediates, action) {
            ([], 'q') => {
                let p2 = params.iter().nth(1).and_then(|p| p.first().copied()).unwrap_or(0);
                let palette = self.shared_palette.take().filter(|_| self.shared_registers).unwrap_or_else(default_palette);
                self.sixel = Some(SixelDecoder::with_allowance(p2, palette, self.sixel_budget.allowance()));
            }
            ([b'$'], 'q') => self.push(Sniffed::DecrqssQuery),
            ([b'+'], 'q') => self.push(Sniffed::XtgettcapQuery),
            _ => {}
        }
    }

    fn put(&mut self, byte: u8) {
        if let Some(decoder) = &mut self.sixel {
            decoder.put(byte);
        }
    }

    fn unhook(&mut self) {
        let Some(decoder) = self.sixel.take() else { return };
        if self.shared_registers {
            self.shared_palette = Some(decoder.palette().to_vec());
        }
        self.sixel_budget.spend(decoder.cost());
        if let Some(image) = decoder.finish() {
            self.push(Sniffed::Sixel(image));
        }
    }

    fn terminated(&self) -> bool {
        self.stop
    }
}

const ESC: u8 = 0x1b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApcState {
    Ground,
    Escape,
    String,
}

/// Finds APC strings (`ESC _ ... ESC \`) the way vte does: ESC starts an escape anywhere, `_` after it opens the string,
/// and the next ESC (the ST), CAN or SUB ends it. vte has no callback for them, so their bytes are collected here.
struct Apc {
    state: ApcState,
    payload: Vec<u8>,
    /// The string is a kitty graphics command (it starts with `G`) that is still under `MAX_TRANSFER`.
    keep: bool,
}

impl Apc {
    /// Runs the state machine over `bytes` until an APC string ends: returns the index just past the byte that ended it
    /// and whether it was ESC (a string that CAN or SUB cut short is dropped), or None when none ended. `collect` gets
    /// the bytes of the open string.
    fn scan(state: &mut ApcState, bytes: &[u8], mut collect: impl FnMut(&[u8])) -> Option<(usize, bool)> {
        let mut i = 0;
        while i < bytes.len() {
            match *state {
                ApcState::Ground => {
                    i += memchr(ESC, &bytes[i..])? + 1;
                    *state = ApcState::Escape;
                }
                ApcState::Escape => {
                    *state = match bytes[i] {
                        b'_' => ApcState::String,
                        ESC => ApcState::Escape,
                        _ => ApcState::Ground,
                    };
                    i += 1;
                }
                ApcState::String => {
                    let rest = &bytes[i..];
                    let Some(at) = memchr3(ESC, 0x18, 0x1a, rest) else {
                        collect(rest);
                        return None;
                    };
                    collect(&rest[..at]);
                    let complete = rest[at] == ESC;
                    *state = if complete { ApcState::Escape } else { ApcState::Ground };
                    return Some((i + at + 1, complete));
                }
            }
        }
        None
    }

    /// Where the next APC string in `bytes` ends (see `scan`), without moving.
    fn end_in(&self, bytes: &[u8]) -> Option<usize> {
        let mut state = self.state;
        Self::scan(&mut state, bytes, |_| {}).map(|(end, _)| end)
    }

    /// Moves over `bytes`, which end no later than the first APC string that ends in them; returns the kitty command
    /// that string carried, if it ended there.
    fn advance(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        // Outside a string the payload is empty and `keep` set: a string that ends resets both.
        let Self { state, payload, keep } = self;
        let (_, complete) = Self::scan(state, bytes, |chunk| {
            if payload.is_empty() && chunk.first().is_some_and(|byte| *byte != b'G') {
                *keep = false;
            }
            if *keep && payload.len() + chunk.len() <= MAX_TRANSFER + 1 {
                payload.extend_from_slice(chunk);
            } else {
                *keep = false;
                payload.clear();
            }
        })?;
        let mut command = std::mem::take(payload);
        let kept = std::mem::replace(keep, true);
        (complete && kept && command.first() == Some(&b'G')).then(|| {
            command.remove(0);
            command
        })
    }
}

pub struct Sniffer {
    parser: Parser,
    collector: Collector,
    apc: Apc,
}

impl Default for Sniffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sniffer {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            collector: Collector::default(),
            apc: Apc { state: ApcState::Ground, payload: Vec::new(), keep: true },
        }
    }

    /// Parses `bytes` until the end or until just after a sequence that needs ordering against the
    /// emulator. Returns the number of bytes consumed; call again with the remainder.
    pub fn advance(&mut self, bytes: &[u8]) -> usize {
        self.collector.stop = false;
        // An APC string that ends in `bytes` ends the segment, so the command it carries comes in stream order.
        let end = self.apc.end_in(bytes).unwrap_or(bytes.len());
        let consumed = self.parser.advance_until_terminated(&mut self.collector, &bytes[..end]);
        if let Some(command) = self.apc.advance(&bytes[..consumed]) {
            self.collector.push(Sniffed::KittyGraphics(command));
        }
        consumed
    }

    /// Takes the events produced since the last call.
    pub fn drain(&mut self) -> Vec<Sniffed> {
        std::mem::take(&mut self.collector.events)
    }

    /// Convenience for tests and tools: sniff a whole buffer and return every event.
    pub fn sniff_all(&mut self, bytes: &[u8]) -> Vec<Sniffed> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < bytes.len() {
            pos += self.advance(&bytes[pos..]);
            out.extend(self.drain());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::AgentState;
    use crate::osc::ProgressState;

    fn sniff(bytes: &[u8]) -> Vec<Sniffed> {
        Sniffer::new().sniff_all(bytes)
    }

    #[test]
    fn osc7_cwd_with_st_and_bel_terminators() {
        let st = sniff(b"\x1b]7;file://PC/C:/Users/me\x1b\\");
        let bel = sniff(b"\x1b]7;file://PC/C:/Users/me\x07");
        assert_eq!(st, vec![Sniffed::Cwd("C:\\Users\\me".into())]);
        assert_eq!(st, bel);
    }

    #[test]
    fn osc9_cwd_progress_and_text() {
        assert_eq!(sniff(b"\x1b]9;9;\"C:\\work dir\"\x07"), vec![Sniffed::Cwd("C:\\work dir".into())]);
        assert_eq!(
            sniff(b"\x1b]9;4;1;42\x07"),
            vec![Sniffed::Progress(Some(Progress::Value { state: ProgressState::Normal, value: 42.0 }))]
        );
        assert_eq!(sniff(b"\x1b]9;4;0\x07"), vec![Sniffed::Progress(None)]);
        assert_eq!(
            sniff(b"\x1b]9;Build finished\x07"),
            vec![Sniffed::Notification(Notification { title: String::new(), body: "Build finished".into() })]
        );
    }

    #[test]
    fn osc777_keeps_semicolons_in_the_body() {
        assert_eq!(
            sniff(b"\x1b]777;notify;Claude;Needs input; now\x07"),
            vec![Sniffed::Notification(Notification { title: "Claude".into(), body: "Needs input; now".into() })]
        );
    }

    #[test]
    fn osc133_marks_include_the_exit_code() {
        let events = sniff(b"\x1b]133;A\x07\x1b]133;D;3\x07");
        assert_eq!(
            events,
            vec![
                Sniffed::PromptMark(PromptMark::PromptStart),
                Sniffed::PromptMark(PromptMark::CommandEnd { exit_code: Some(3) })
            ]
        );
    }

    #[test]
    fn osc7501_set_clear_and_query() {
        let events = sniff(b"\x1b]7501;state=working:progress=5:id=a\x1b\\\x1b]7501;state=clear:id=a\x07\x1b]7501;?\x1b\\");
        assert_eq!(events.len(), 3);
        match &events[0] {
            Sniffed::Agent(Report::Set(r)) => {
                assert_eq!(r.state, AgentState::Working);
                assert_eq!(r.progress, Some(5));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(events[1], Sniffed::Agent(Report::Clear { id: "a".into() }));
        assert_eq!(events[2], Sniffed::AgentQuery { bell_terminated: false });
        // malformed and oversized reports produce nothing
        assert!(sniff(b"\x1b]7501;state=sleeping\x07").is_empty());
        let mut huge = b"\x1b]7501;state=working:app=".to_vec();
        huge.extend(std::iter::repeat_n(b'x', 5000));
        huge.push(7);
        assert!(sniff(&huge).is_empty());
    }

    #[test]
    fn xtversion_decrqss_xtgettcap_and_sixel() {
        assert_eq!(sniff(b"\x1b[>q"), vec![Sniffed::XtVersionQuery]);
        assert_eq!(sniff(b"\x1b[>0q"), vec![Sniffed::XtVersionQuery]);
        assert!(sniff(b"\x1b[>1q").is_empty());
        assert_eq!(sniff(b"\x1bP$qm\x1b\\"), vec![Sniffed::DecrqssQuery]);
        assert_eq!(sniff(b"\x1bP+q544e\x1b\\"), vec![Sniffed::XtgettcapQuery]);
        let events = sniff(b"\x1bP0;0;0q\"1;1;4;4#0;2;0;0;0~-\x1b\\");
        let [Sniffed::Sixel(image)] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((image.width, image.height, image.transparent_background), (4, 6, false));
        assert_eq!(sniff(b"\x1b[?996n"), vec![Sniffed::ColorSchemeQuery]);
        assert!(sniff(b"\x1b[?6n").is_empty());
    }

    #[test]
    fn graphics_queries_and_sixel_modes() {
        assert_eq!(sniff(b"\x1b[?1;1;0S"), vec![Sniffed::GraphicsAttributes { item: 1, action: 1 }]);
        assert_eq!(sniff(b"\x1b[?2;4S"), vec![Sniffed::GraphicsAttributes { item: 2, action: 4 }]);
        assert!(sniff(b"\x1b[2S").is_empty(), "scroll up is the emulator's");
        assert_eq!(sniff(b"\x1b[16t"), vec![Sniffed::CellSizeQuery]);
        assert!(sniff(b"\x1b[14t\x1b[18t").is_empty(), "alacritty answers those");
        assert_eq!(
            sniff(b"\x1b[?80;8452h\x1b[?80l"),
            vec![
                Sniffed::SixelMode(SixelMode::Display, true),
                Sniffed::SixelMode(SixelMode::CursorRight, true),
                Sniffed::SixelMode(SixelMode::Display, false)
            ]
        );
    }

    #[test]
    fn shared_colour_registers_carry_over_to_the_next_picture_only_after_decrst_1070() {
        let red_then_reuse = |mode: &[u8]| {
            let mut stream = mode.to_vec();
            stream.extend_from_slice(b"\x1bP;1q#3;2;100;0;0#3@\x1b\\\x1bP;1q#3@\x1b\\");
            sniff(&stream)
                .into_iter()
                .filter_map(|event| if let Sniffed::Sixel(image) = event { Some(image.pixels[..4].to_vec()) } else { None })
                .collect::<Vec<_>>()
        };
        assert_eq!(red_then_reuse(b"\x1b[?1070l")[1], [255, 0, 0, 255]);
        assert_ne!(red_then_reuse(b"")[1], [255, 0, 0, 255], "private registers start from the VT340 colours");
    }

    #[test]
    fn kitty_commands_are_cut_out_of_apc_strings_and_other_apc_strings_are_dropped() {
        assert_eq!(sniff(b"a\x1b_Ga=q,i=1;AAAA\x1b\\b"), vec![Sniffed::KittyGraphics(b"a=q,i=1;AAAA".to_vec())]);
        assert!(sniff(b"\x1b_Xsomething else\x1b\\").is_empty());
        assert!(sniff(b"\x1b_Ga=q\x18").is_empty(), "CAN cuts the string short");
        assert!(sniff(b"\x1b]2;title\x1b_\x1b\\").is_empty(), "an empty APC after an OSC ends it");
        let mut sniffer = Sniffer::new();
        let data = b"x\x1b_Gi=3\x1b\\yz";
        let consumed = sniffer.advance(data);
        assert_eq!(&data[..consumed], b"x\x1b_Gi=3\x1b", "the segment ends with the string, at its ST");
        assert_eq!(sniffer.drain(), vec![Sniffed::KittyGraphics(b"i=3".to_vec())]);
    }

    #[test]
    fn an_apc_string_over_the_transfer_limit_is_dropped_and_the_next_one_works() {
        let mut stream = b"\x1b_Gi=1;".to_vec();
        stream.extend(std::iter::repeat_n(b'A', MAX_TRANSFER + 8));
        stream.extend_from_slice(b"\x1b\\\x1b_Gi=2\x1b\\");
        assert_eq!(sniff(&stream), vec![Sniffed::KittyGraphics(b"i=2".to_vec())]);
    }

    #[test]
    fn win32_input_mode_is_tracked_among_other_modes() {
        assert_eq!(sniff(b"\x1b[?9001h"), vec![Sniffed::Win32InputMode(true)]);
        assert_eq!(sniff(b"\x1b[?1004;9001l"), vec![Sniffed::Win32InputMode(false)]);
        assert!(sniff(b"\x1b[?1004h\x1b[?25l").is_empty());
    }

    #[test]
    fn ris_and_decstr_are_reported() {
        assert_eq!(sniff(b"\x1bc"), vec![Sniffed::FullReset]);
        assert_eq!(sniff(b"\x1b[!p"), vec![Sniffed::SoftReset]);
        // DECSCUSR and other intermediates before `p` are different sequences.
        assert!(sniff(b"\x1b[0 p").is_empty());
        assert!(sniff(b"\x1b[?1049p").is_empty());
    }

    #[test]
    fn mouse_extension_modes_are_tracked_among_other_modes() {
        use MouseExtension::{SgrPixels, Urxvt, X10};
        assert_eq!(sniff(b"\x1b[?9h"), vec![Sniffed::MouseExtension(X10, true)]);
        assert_eq!(sniff(b"\x1b[?9l"), vec![Sniffed::MouseExtension(X10, false)]);
        assert_eq!(sniff(b"\x1b[?1015h"), vec![Sniffed::MouseExtension(Urxvt, true)]);
        assert_eq!(
            sniff(b"\x1b[?1000;1016;25h"),
            vec![Sniffed::MouseTracking, Sniffed::MouseExtension(SgrPixels, true)],
            "several modes in one sequence, others ignored"
        );
        assert_eq!(sniff(b"\x1b[?1003l"), vec![Sniffed::MouseTracking]);
        assert_eq!(sniff(b"\x1b[?1002h"), vec![Sniffed::MouseTracking]);
        // Not mouse modes, and the same numbers without the `?` are ANSI modes.
        assert!(sniff(b"\x1b[?1006h\x1b[?1004h\x1b[9h").is_empty());
    }

    #[test]
    fn decrqm_for_mouse_extensions_needs_ordering() {
        use MouseExtension::{SgrPixels, X10};
        assert_eq!(sniff(b"\x1b[?1016$p"), vec![Sniffed::MouseModeQuery(SgrPixels)]);
        assert_eq!(sniff(b"\x1b[?9$p"), vec![Sniffed::MouseModeQuery(X10)]);
        assert!(sniff(b"\x1b[?2004$p").is_empty(), "other modes are alacritty's to answer");
        assert!(sniff(b"\x1b[1016$p").is_empty(), "ANSI (non-private) mode query");

        let mut sniffer = Sniffer::new();
        let data = b"ab\x1b[?1016$pcd";
        let consumed = sniffer.advance(data);
        assert_eq!(&data[..consumed], b"ab\x1b[?1016$p", "the stream is cut after the query");
    }

    #[test]
    fn ed3_clears_the_scrollback_and_other_erase_modes_do_not() {
        assert_eq!(sniff(b"\x1b[3J"), vec![Sniffed::ClearScrollback]);
        assert!(sniff(b"\x1b[J\x1b[0J\x1b[1J\x1b[2J").is_empty());
        assert!(sniff(b"\x1b[?3J").is_empty(), "DECSED is selective erase, not ED");
    }

    #[test]
    fn soft_reset_ends_the_segment_so_later_bytes_see_the_reset_state() {
        let mut sniffer = Sniffer::new();
        let data = b"x\x1b[!pyz";
        let consumed = sniffer.advance(data);
        assert_eq!(&data[..consumed], b"x\x1b[!p");
        assert_eq!(sniffer.drain(), vec![Sniffed::SoftReset]);
    }

    #[test]
    fn events_are_identical_for_every_chunking_of_the_stream() {
        let mut stream = Vec::new();
        stream.extend_from_slice("héllo 你好 \u{1f468}\u{200d}\u{1f469}".as_bytes());
        stream.extend_from_slice(b"\x1b]7;file:///C:/a%20b\x1b\\");
        stream.extend_from_slice(b"\x1b[31mred\x1b[0m");
        stream.extend_from_slice(b"\x1b]7501;state=blocked:kind=auth:id=x/y\x07");
        stream.extend_from_slice(b"\x1b]777;notify;t;b\x1b\\\x1b[>q\x1bP$qm\x1b\\");
        stream.extend_from_slice("\x1b]9;你好,通知\x07".as_bytes());
        stream.extend_from_slice(b"\x1b]133;D;0\x07\x1b]7501;?\x07");
        stream.extend_from_slice(b"\x1b[?9;1016h\x1b[?1016$p\x1b[!p\x1b[3J\x1b[?1000l");
        stream.extend_from_slice(b"\x1b_Ga=T,f=24,s=1,v=1;AAAA\x1b\\\x1bPq#1;2;100;0;0!3~-@\x1b\\\x1b_Gi=9\x1b\\");
        let whole = sniff(&stream);
        assert_eq!(whole.len(), 17);

        // every possible single split point, and byte-at-a-time
        for split in 0..=stream.len() {
            let mut sniffer = Sniffer::new();
            let mut events = sniffer.sniff_all(&stream[..split]);
            events.extend(sniffer.sniff_all(&stream[split..]));
            assert_eq!(events, whole, "split at {split}");
        }
        let mut sniffer = Sniffer::new();
        let mut events = Vec::new();
        for byte in &stream {
            events.extend(sniffer.sniff_all(std::slice::from_ref(byte)));
        }
        assert_eq!(events, whole);
    }

    #[test]
    fn advance_stops_after_sequences_that_need_ordering() {
        let mut sniffer = Sniffer::new();
        let data = b"abc\x1b[>qdef\x1b]133;A\x07tail";
        let first = sniffer.advance(data);
        assert_eq!(&data[..first], b"abc\x1b[>q");
        assert_eq!(sniffer.drain(), vec![Sniffed::XtVersionQuery]);
        let second = sniffer.advance(&data[first..]);
        assert_eq!(&data[first..first + second], b"def\x1b]133;A\x07");
        assert_eq!(sniffer.drain(), vec![Sniffed::PromptMark(PromptMark::PromptStart)]);
        let third = sniffer.advance(&data[first + second..]);
        assert_eq!(third, 4);
        assert!(sniffer.drain().is_empty());
    }

    #[test]
    fn control_characters_are_stripped_from_notification_text() {
        let events = sniff(b"\x1b]777;notify;ti\x08tle;bo\xe2\x80\xaedy\x07");
        assert_eq!(
            events,
            vec![Sniffed::Notification(Notification { title: "title".into(), body: "body".into() })]
        );
    }
}
