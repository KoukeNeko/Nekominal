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

use alacritty_terminal::vte::{Params, Parser, Perform};

use crate::agent_status::{parse_report, Report, MAX_SEQUENCE_LENGTH};
use crate::osc::{
    parse_osc133, parse_osc7, parse_osc777, parse_osc9, strip_control, Notification, Osc9,
    Progress, PromptMark,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// `DCS ... q` (sixel). The emulator swallows the whole string; nothing is drawn.
    Sixel,
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
    Unsupported(Unsupported),
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
        )
    }
}

#[derive(Default)]
struct Collector {
    events: Vec<Sniffed>,
    stop: bool,
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
                    }
                }
            }
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
            self.push(Sniffed::FullReset);
        }
    }

    fn hook(&mut self, _params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match (intermediates, action) {
            ([], 'q') => self.push(Sniffed::Unsupported(Unsupported::Sixel)),
            ([b'$'], 'q') => self.push(Sniffed::DecrqssQuery),
            ([b'+'], 'q') => self.push(Sniffed::XtgettcapQuery),
            _ => {}
        }
    }

    fn terminated(&self) -> bool {
        self.stop
    }
}

pub struct Sniffer {
    parser: Parser,
    collector: Collector,
}

impl Default for Sniffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sniffer {
    pub fn new() -> Self {
        Self { parser: Parser::new(), collector: Collector::default() }
    }

    /// Parses `bytes` until the end or until just after a sequence that needs ordering against the
    /// emulator. Returns the number of bytes consumed; call again with the remainder.
    pub fn advance(&mut self, bytes: &[u8]) -> usize {
        self.collector.stop = false;
        self.parser.advance_until_terminated(&mut self.collector, bytes)
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
        assert_eq!(sniff(b"\x1bP0;0;0q\"1;1;4;4#0;2;0;0;0~-\x1b\\"), vec![Sniffed::Unsupported(Unsupported::Sixel)]);
        assert_eq!(sniff(b"\x1b[?996n"), vec![Sniffed::ColorSchemeQuery]);
        assert!(sniff(b"\x1b[?6n").is_empty());
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
        let whole = sniff(&stream);
        assert_eq!(whole.len(), 14);

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
