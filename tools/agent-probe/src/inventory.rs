//! Inventory of the control sequences a program emitted: every mode switch, query, OSC, DCS and ESC sequence
//! with a count, what it is for, whether the emulator supports it and, for queries, whether it answered.
//!
//! The inventory parses the raw output stream with its own `vte` parser, independent of the emulator, so a
//! sequence the emulator ignores is counted too. Replies are classified from the bytes the emulator produced
//! (`CoreEvent::PtyWrite`), so "answered" is measured, not assumed.

use std::{collections::BTreeMap, fmt::Write as _, time::Duration};

use nekominal_term::alacritty_terminal::vte::{Params, Parser, Perform};

/// What a query expects back. One group per reply shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Da1,
    Da2,
    Da3,
    Dsr5,
    Dsr6,
    /// `CSI ? 6 n`.
    CprPrivate,
    DecrqmPrivate(u16),
    DecrqmAnsi(u16),
    KittyKeyboard,
    /// `CSI 14 t`, `CSI 16 t`, `CSI 18 t`, `CSI 19 t`.
    Xtwinops(u16),
    /// OSC 4 / 10 / 11 / 12 colour query, by slot.
    OscColor(u16),
    Osc52,
    Osc7501,
    Xtversion,
    Decrqss,
    Xtgettcap,
    ColorScheme,
    /// `CSI ? Pi ; Pa ; Pv S` (XTSMGRAPHICS).
    GraphicsQuery,
}

impl Group {
    pub fn name(self) -> String {
        match self {
            Group::Da1 => "DA1".into(),
            Group::Da2 => "DA2".into(),
            Group::Da3 => "DA3".into(),
            Group::Dsr5 => "DSR 5".into(),
            Group::Dsr6 => "DSR 6 (CPR)".into(),
            Group::CprPrivate => "DECXCPR".into(),
            Group::DecrqmPrivate(mode) => format!("DECRQM ?{mode}"),
            Group::DecrqmAnsi(mode) => format!("DECRQM {mode}"),
            Group::KittyKeyboard => "kitty keyboard query".into(),
            Group::Xtwinops(op) => format!("XTWINOPS {op}"),
            Group::OscColor(slot) => format!("OSC {slot} colour query"),
            Group::Osc52 => "OSC 52 read".into(),
            Group::Osc7501 => "OSC 7501 support query".into(),
            Group::Xtversion => "XTVERSION".into(),
            Group::Decrqss => "DECRQSS".into(),
            Group::Xtgettcap => "XTGETTCAP".into(),
            Group::ColorScheme => "colour scheme query".into(),
            Group::GraphicsQuery => "XTSMGRAPHICS".into(),
        }
    }
}

/// How well the emulator handles a sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Yes,
    /// Accepted and parsed, no visible effect on a headless or GPU terminal (documented reason).
    Ignored(&'static str),
    No(&'static str),
    Unknown,
}

impl Support {
    pub fn label(self) -> String {
        match self {
            Support::Yes => "yes".into(),
            Support::Ignored(why) => format!("ignored ({why})"),
            Support::No(why) => format!("no ({why})"),
            Support::Unknown => "not classified".into(),
        }
    }
}

/// DEC private mode: name, support. Verified against the emulator by `private_mode_table_matches_the_emulator`.
pub fn private_mode_info(mode: u16) -> (&'static str, Support) {
    match mode {
        1 => ("application cursor keys (DECCKM)", Support::Yes),
        3 => ("132/80 column mode (DECCOLM)", Support::Ignored("the grid size follows the window")),
        4 => ("smooth scroll (DECSCLM)", Support::Ignored("no effect")),
        5 => ("reverse video (DECSCNM)", Support::No("not implemented by alacritty_terminal")),
        6 => ("origin mode (DECOM)", Support::Yes),
        7 => ("auto wrap (DECAWM)", Support::Yes),
        8 => ("auto repeat", Support::Ignored("no effect")),
        9 => ("X10 mouse reporting", Support::Yes),
        12 => ("cursor blink (att610)", Support::Yes),
        25 => ("show cursor (DECTCEM)", Support::Yes),
        45 => ("reverse wraparound", Support::No("not implemented")),
        47 | 1047 => {
            ("alternate screen (legacy)", Support::No("only 1049 is implemented; ConPTY and modern apps use 1049"))
        }
        66 => ("application keypad (DECNKM)", Support::Yes),
        69 => ("left/right margins (DECLRMM)", Support::No("not implemented")),
        1000 => ("mouse tracking: clicks", Support::Yes),
        1001 => ("mouse highlight tracking", Support::No("not implemented")),
        1002 => ("mouse tracking: drag", Support::Yes),
        1003 => ("mouse tracking: all motion", Support::Yes),
        1004 => ("focus reporting", Support::Yes),
        1005 => ("mouse encoding: UTF-8", Support::Yes),
        1006 => ("mouse encoding: SGR", Support::Yes),
        1007 => ("alternate scroll", Support::Yes),
        1015 => ("mouse encoding: urxvt", Support::Yes),
        1016 => ("mouse encoding: SGR pixels", Support::Yes),
        1042 => ("urgency on bell", Support::Yes),
        1048 => ("save cursor (DECSC alias)", Support::No("not implemented")),
        1049 => ("alternate screen with cursor save", Support::Yes),
        2004 => ("bracketed paste", Support::Yes),
        2026 => ("synchronized output", Support::Yes),
        2027 => ("grapheme clustering", Support::No("cells are code-point based; ConPTY runs in wcswidth mode")),
        2031 => ("colour scheme change notifications", Support::No("only the CSI ? 996 n query is answered")),
        9001 => ("win32-input-mode", Support::Yes),
        _ => ("unknown mode", Support::Unknown),
    }
}

#[derive(Clone, Debug)]
pub enum Kind {
    Mode,
    Query(Group),
    Other,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub key: String,
    pub purpose: String,
    pub support: Support,
    pub kind: Kind,
}

fn param(params: &Params, index: usize) -> Option<u16> {
    params.iter().nth(index).and_then(|sub| sub.first().copied())
}

fn all_params(params: &Params) -> Vec<u16> {
    params.iter().filter_map(|sub| sub.first().copied()).collect()
}

fn intermediates_text(intermediates: &[u8]) -> String {
    intermediates.iter().map(|&b| char::from(b)).collect()
}

fn other(key: String, purpose: &str, support: Support) -> Vec<Item> {
    vec![Item { key, purpose: purpose.to_owned(), support, kind: Kind::Other }]
}

fn query(key: &str, purpose: &str, group: Group, support: Support) -> Vec<Item> {
    vec![Item { key: key.to_owned(), purpose: purpose.to_owned(), support, kind: Kind::Query(group) }]
}

fn classify_csi(params: &Params, intermediates: &[u8], action: char) -> Vec<Item> {
    let first = param(params, 0);
    match (action, intermediates) {
        ('h' | 'l', [b'?']) => all_params(params)
            .into_iter()
            .map(|mode| {
                let (name, support) = private_mode_info(mode);
                Item {
                    key: format!("{} ?{mode}", if action == 'h' { "DECSET" } else { "DECRST" }),
                    purpose: name.to_owned(),
                    support,
                    kind: Kind::Mode,
                }
            })
            .collect(),
        ('h' | 'l', []) => all_params(params)
            .into_iter()
            .map(|mode| Item {
                key: format!("{} {mode}", if action == 'h' { "SM" } else { "RM" }),
                purpose: "ANSI mode".to_owned(),
                support: if matches!(mode, 4 | 20) { Support::Yes } else { Support::Ignored("unknown ANSI mode") },
                kind: Kind::Mode,
            })
            .collect(),
        ('c', []) if first.unwrap_or(0) == 0 => {
            query("CSI c (DA1)", "primary device attributes", Group::Da1, Support::Yes)
        }
        ('c', [b'>']) => query("CSI > c (DA2)", "secondary device attributes", Group::Da2, Support::Yes),
        ('c', [b'=']) => {
            query("CSI = c (DA3)", "tertiary device attributes", Group::Da3, Support::No("never answered"))
        }
        ('n', []) if first == Some(5) => query("CSI 5 n (DSR)", "device status", Group::Dsr5, Support::Yes),
        ('n', []) if first == Some(6) => query("CSI 6 n (CPR)", "cursor position report", Group::Dsr6, Support::Yes),
        ('n', [b'?']) if first == Some(6) => {
            query("CSI ? 6 n (DECXCPR)", "extended cursor position", Group::CprPrivate, Support::No("never answered"))
        }
        ('n', [b'?']) if first == Some(996) => {
            query("CSI ? 996 n", "colour scheme (dark/light) query", Group::ColorScheme, Support::Yes)
        }
        ('p', [b'?', b'$']) => {
            let mode = first.unwrap_or(0);
            let (name, _) = private_mode_info(mode);
            vec![Item {
                key: format!("DECRQM ?{mode}"),
                purpose: format!("mode query: {name}"),
                support: Support::Yes,
                kind: Kind::Query(Group::DecrqmPrivate(mode)),
            }]
        }
        ('p', [b'$']) => {
            let mode = first.unwrap_or(0);
            vec![Item {
                key: format!("DECRQM {mode}"),
                purpose: "ANSI mode query".to_owned(),
                support: Support::Yes,
                kind: Kind::Query(Group::DecrqmAnsi(mode)),
            }]
        }
        ('p', [b'!']) => other("CSI ! p (DECSTR)".into(), "soft terminal reset", Support::Yes),
        ('u', [b'?']) => query("CSI ? u", "kitty keyboard: query flags", Group::KittyKeyboard, Support::Yes),
        ('u', [b'>']) => other(format!("CSI > {} u", first.unwrap_or(0)), "kitty keyboard: push flags", Support::Yes),
        ('u', [b'<']) => other("CSI < u".into(), "kitty keyboard: pop flags", Support::Yes),
        ('u', [b'=']) => other("CSI = u".into(), "kitty keyboard: set flags", Support::Yes),
        ('q', [b'>']) => query("CSI > q (XTVERSION)", "terminal name and version", Group::Xtversion, Support::Yes),
        ('q', [b' ']) => other("CSI SP q (DECSCUSR)".into(), "cursor shape", Support::Yes),
        ('t', []) => {
            let op = first.unwrap_or(0);
            match op {
                14 | 16 | 18 | 19 => {
                    let answered = matches!(op, 14 | 18);
                    query(
                        &format!("CSI {op} t"),
                        match op {
                            14 => "window size in pixels",
                            16 => "cell size in pixels",
                            18 => "text area size in cells",
                            _ => "screen size in cells",
                        },
                        Group::Xtwinops(op),
                        if answered { Support::Yes } else { Support::No("never answered") },
                    )
                }
                22 | 23 => {
                    other(format!("CSI {op} t"), "title stack push/pop", Support::Ignored("title stack not kept"))
                }
                _ => other(format!("CSI {op} t"), "window manipulation", Support::Ignored("no effect")),
            }
        }
        ('S', [b'?']) => query(
            "CSI ? S (XTSMGRAPHICS)",
            "graphics attribute query",
            Group::GraphicsQuery,
            Support::No("never answered; no sixel or kitty graphics"),
        ),
        ('m', []) => other("CSI m (SGR)".into(), "colours and attributes", Support::Yes),
        ('m', [b'>']) | ('m', [b'?']) => other(
            format!("CSI {} m", intermediates_text(intermediates)),
            "key modifier options (xterm)",
            Support::Ignored("no effect"),
        ),
        ('H' | 'f', []) => other("CSI H (CUP)".into(), "cursor position", Support::Yes),
        ('A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G' | 'd', []) => {
            other(format!("CSI {action}"), "cursor movement", Support::Yes)
        }
        ('J', []) => other("CSI J (ED)".into(), "erase in display", Support::Yes),
        ('K', []) => other("CSI K (EL)".into(), "erase in line", Support::Yes),
        ('L' | 'M' | 'P' | '@' | 'X', []) => {
            other(format!("CSI {action}"), "insert/delete/erase characters or lines", Support::Yes)
        }
        ('S' | 'T', []) => other(format!("CSI {action}"), "scroll up/down", Support::Yes),
        ('r', []) => other("CSI r (DECSTBM)".into(), "scroll region", Support::Yes),
        ('s' | 'u', []) => other(format!("CSI {action}"), "save/restore cursor (ANSI.SYS)", Support::Yes),
        ('g' | 'I' | 'Z' | 'b', []) => other(format!("CSI {action}"), "tabs and repeat", Support::Yes),
        (action, intermediates) => {
            other(format!("CSI {}{action}", intermediates_text(intermediates)), "other CSI", Support::Unknown)
        }
    }
}

fn classify_osc(params: &[&[u8]]) -> Vec<Item> {
    let Some((code, rest)) = params.split_first() else {
        return Vec::new();
    };
    let code_text = String::from_utf8_lossy(code).into_owned();
    let payload = rest.first().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default();
    let is_query = payload == "?";
    match code_text.as_str() {
        "0" | "1" | "2" => other(format!("OSC {code_text}"), "window title", Support::Yes),
        "4" if rest.iter().any(|p| *p == b"?") => {
            query("OSC 4 ?", "palette colour query", Group::OscColor(4), Support::Yes)
        }
        "4" => other("OSC 4".into(), "set palette colour", Support::Yes),
        "7" => other("OSC 7".into(), "working directory", Support::Yes),
        "8" => other("OSC 8".into(), "hyperlink", Support::Yes),
        "9" => match payload.as_str() {
            "4" => other("OSC 9;4".into(), "progress bar", Support::Yes),
            "9" => other("OSC 9;9".into(), "working directory (ConEmu)", Support::Yes),
            _ => other("OSC 9".into(), "desktop notification", Support::Yes),
        },
        "10" | "11" | "12" if is_query => {
            let slot: u16 = code_text.parse().unwrap_or(0);
            query(
                &format!("OSC {code_text} ?"),
                match slot {
                    10 => "foreground colour query",
                    11 => "background colour query",
                    _ => "cursor colour query",
                },
                Group::OscColor(slot),
                Support::Yes,
            )
        }
        "10" | "11" | "12" => other(format!("OSC {code_text}"), "set dynamic colour", Support::Yes),
        "52" if rest.get(1).is_some_and(|p| *p == b"?") => {
            query("OSC 52 ?", "clipboard read", Group::Osc52, Support::No("policy: write only, a read gets no reply"))
        }
        "52" => other("OSC 52".into(), "clipboard write", Support::Yes),
        "99" => other("OSC 99".into(), "kitty desktop notification", Support::No("not implemented")),
        "104" | "110" | "111" | "112" => other(format!("OSC {code_text}"), "reset colour", Support::Yes),
        "133" => other("OSC 133".into(), "shell prompt marks", Support::Yes),
        "777" => other("OSC 777".into(), "desktop notification (notify)", Support::Yes),
        "1337" => other("OSC 1337".into(), "iTerm2 proprietary", Support::No("not implemented")),
        "7501" if is_query => {
            query("OSC 7501 ?", "program status protocol support query", Group::Osc7501, Support::Yes)
        }
        "7501" => other("OSC 7501 set".into(), "program status report (agent state)", Support::Yes),
        _ => other(format!("OSC {code_text}"), "other OSC", Support::Unknown),
    }
}

#[derive(Default)]
struct Collector {
    items: Vec<Item>,
}

impl Perform for Collector {
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if !ignore {
            self.items.extend(classify_csi(params, intermediates, action));
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.items.extend(classify_osc(params));
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        let text = format!("ESC {}{}", intermediates_text(intermediates), char::from(byte));
        let (purpose, support) = match (intermediates, byte) {
            ([], b'7') => ("save cursor (DECSC)", Support::Yes),
            ([], b'8') => ("restore cursor (DECRC)", Support::Yes),
            ([], b'=') => ("application keypad (DECKPAM)", Support::Yes),
            ([], b'>') => ("numeric keypad (DECKPNM)", Support::Yes),
            ([], b'c') => ("full reset (RIS)", Support::Yes),
            ([], b'M') => ("reverse index", Support::Yes),
            ([], b'D' | b'E') => ("index / next line", Support::Yes),
            ([b'(' | b')' | b'*' | b'+'], _) => ("character set designation", Support::Yes),
            _ => ("other ESC", Support::Unknown),
        };
        self.items.push(Item { key: text, purpose: purpose.to_owned(), support, kind: Kind::Other });
    }

    fn hook(&mut self, _params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        let items = match (intermediates, action) {
            ([], 'q') => {
                other("DCS q (sixel)".into(), "sixel graphics", Support::No("recognised and dropped; nothing is drawn"))
            }
            ([b'$'], 'q') => query(
                "DCS $ q (DECRQSS)",
                "setting query",
                Group::Decrqss,
                Support::Ignored("answered with 'invalid request'"),
            ),
            ([b'+'], 'q') => query(
                "DCS + q (XTGETTCAP)",
                "terminfo capability query",
                Group::Xtgettcap,
                Support::Ignored("answered with 'invalid request'"),
            ),
            _ => other(format!("DCS {}{action}", intermediates_text(intermediates)), "other DCS", Support::Unknown),
        };
        self.items.extend(items);
    }
}

/// Classifies the replies the emulator wrote. One call per reply, as `CoreEvent::PtyWrite` delivers them.
pub fn classify_reply(bytes: &[u8]) -> Vec<Group> {
    #[derive(Default)]
    struct Replies(Vec<Group>);
    impl Perform for Replies {
        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
            let first = param(params, 0);
            let group = match (action, intermediates) {
                ('c', [b'?']) => Group::Da1,
                ('c', [b'>']) => Group::Da2,
                ('n', []) if first == Some(0) => Group::Dsr5,
                ('R', []) => Group::Dsr6,
                ('y', [b'?', b'$']) => Group::DecrqmPrivate(first.unwrap_or(0)),
                ('y', [b'$']) => Group::DecrqmAnsi(first.unwrap_or(0)),
                ('u', [b'?']) => Group::KittyKeyboard,
                ('t', []) if first == Some(4) => Group::Xtwinops(14),
                ('t', []) if first == Some(8) => Group::Xtwinops(18),
                ('n', [b'?']) if first == Some(997) => Group::ColorScheme,
                _ => return,
            };
            self.0.push(group);
        }

        fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
            let code = params.first().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default();
            match code.as_str() {
                "4" | "10" | "11" | "12" => self.0.push(Group::OscColor(code.parse().unwrap_or(0))),
                "52" => self.0.push(Group::Osc52),
                "7501" => self.0.push(Group::Osc7501),
                _ => {}
            }
        }

        fn hook(&mut self, _params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
            match (intermediates, action) {
                ([b'>'], '|') => self.0.push(Group::Xtversion),
                ([b'$'], 'r') => self.0.push(Group::Decrqss),
                ([b'+'], 'r') => self.0.push(Group::Xtgettcap),
                _ => {}
            }
        }
    }
    let mut parser = Parser::new();
    let mut replies = Replies::default();
    parser.advance(&mut replies, bytes);
    replies.0
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub purpose: String,
    pub support: Support,
    pub kind: Kind,
    pub count: u64,
    pub first_at: Duration,
}

/// How the emulator handled the queries of one group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Not a query.
    NotAQuery,
    Answered {
        queries: u64,
        replies: u64,
    },
    Unanswered {
        queries: u64,
    },
}

impl Answer {
    pub fn label(self) -> String {
        match self {
            Answer::NotAQuery => "-".into(),
            Answer::Answered { queries, replies } if replies >= queries => format!("answered {replies}/{queries}"),
            Answer::Answered { queries, replies } => format!("partly answered {replies}/{queries}"),
            Answer::Unanswered { queries } => format!("NOT answered (0/{queries})"),
        }
    }
}

/// A reply the emulator wrote, in order, for ordering checks.
#[derive(Clone, Debug)]
pub struct ReplyRecord {
    pub at: Duration,
    pub groups: Vec<Group>,
    pub len: usize,
}

const ORDER_LOG_LIMIT: usize = 4000;

pub struct Inventory {
    parser: Parser,
    collector: Collector,
    entries: BTreeMap<String, Entry>,
    /// Every sequence in stream order (capped), as (time, key).
    order: Vec<(Duration, String)>,
    /// Queries per group, in stream order (capped), for the order check.
    query_order: Vec<(Duration, Group)>,
    queries: BTreeMap<Group, u64>,
    replies: BTreeMap<Group, u64>,
    reply_log: Vec<ReplyRecord>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self::new()
    }
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            collector: Collector::default(),
            entries: BTreeMap::new(),
            order: Vec::new(),
            query_order: Vec::new(),
            queries: BTreeMap::new(),
            replies: BTreeMap::new(),
            reply_log: Vec::new(),
        }
    }

    /// Feeds raw output bytes seen at `at` (time since the spawn).
    pub fn feed(&mut self, bytes: &[u8], at: Duration) {
        self.parser.advance(&mut self.collector, bytes);
        for item in std::mem::take(&mut self.collector.items) {
            if let Kind::Query(group) = item.kind {
                *self.queries.entry(group).or_default() += 1;
                if self.query_order.len() < ORDER_LOG_LIMIT {
                    self.query_order.push((at, group));
                }
            }
            if self.order.len() < ORDER_LOG_LIMIT {
                self.order.push((at, item.key.clone()));
            }
            self.entries.entry(item.key.clone()).and_modify(|entry| entry.count += 1).or_insert(Entry {
                key: item.key,
                purpose: item.purpose,
                support: item.support,
                kind: item.kind,
                count: 1,
                first_at: at,
            });
        }
    }

    /// Records one reply the emulator wrote.
    pub fn note_reply(&mut self, bytes: &[u8], at: Duration) {
        let groups = classify_reply(bytes);
        for group in &groups {
            *self.replies.entry(*group).or_default() += 1;
        }
        self.reply_log.push(ReplyRecord { at, groups, len: bytes.len() });
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key)
    }

    pub fn count(&self, key: &str) -> u64 {
        self.entries.get(key).map_or(0, |entry| entry.count)
    }

    /// Sequences in the order the program sent them (capped at the first few thousand).
    pub fn order(&self) -> &[(Duration, String)] {
        &self.order
    }

    pub fn replies(&self) -> &[ReplyRecord] {
        &self.reply_log
    }

    pub fn answer_for(&self, group: Group) -> Answer {
        let queries = self.queries.get(&group).copied().unwrap_or(0);
        if queries == 0 {
            return Answer::NotAQuery;
        }
        match self.replies.get(&group).copied().unwrap_or(0) {
            0 => Answer::Unanswered { queries },
            replies => Answer::Answered { queries, replies },
        }
    }

    pub fn answer(&self, entry: &Entry) -> Answer {
        match entry.kind {
            Kind::Query(group) => self.answer_for(group),
            _ => Answer::NotAQuery,
        }
    }

    /// Queries that got no reply at all.
    pub fn unanswered(&self) -> Vec<Group> {
        self.queries
            .keys()
            .copied()
            .filter(|group| matches!(self.answer_for(*group), Answer::Unanswered { .. }))
            .collect()
    }

    /// Whether the replies that exist came in the order of the queries that asked for them (every query that
    /// was answered, compared as sequences of groups).
    pub fn replies_follow_query_order(&self) -> bool {
        let answered = |group: &Group| self.replies.contains_key(group);
        let wanted: Vec<Group> = self.query_order.iter().map(|(_, g)| *g).filter(answered).collect();
        let got: Vec<Group> = self.reply_log.iter().flat_map(|r| r.groups.iter().copied()).collect();
        // The reply stream may hold more of a group than the first N queries; compare the common prefix.
        wanted.iter().zip(got.iter()).all(|(a, b)| a == b)
    }

    /// Plain-text table for test output.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{:<26} {:>6}  {:<46} {:<28} {}", "sequence", "count", "purpose", "emulator", "answer");
        for entry in self.entries() {
            let _ = writeln!(
                out,
                "{:<26} {:>6}  {:<46} {:<28} {}",
                entry.key,
                entry.count,
                entry.purpose,
                entry.support.label(),
                self.answer(entry).label()
            );
        }
        out
    }

    /// Markdown table for the user-facing doc: sequence, purpose, supported. No payloads are included.
    pub fn markdown_rows(&self) -> Vec<(String, String, String, u64)> {
        self.entries()
            .map(|entry| {
                let mut support = entry.support.label();
                if let Answer::Unanswered { .. } = self.answer(entry) {
                    support = format!("{support}; no reply");
                }
                (entry.key.clone(), entry.purpose.clone(), support, entry.count)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(inventory: &mut Inventory, bytes: &[u8]) {
        inventory.feed(bytes, Duration::from_millis(1));
    }

    #[test]
    fn counts_modes_queries_and_osc_with_their_support() {
        let mut inv = Inventory::new();
        feed(
            &mut inv,
            b"\x1b[?1049h\x1b[?2004h\x1b[?1000;1006h\x1b[?2027h\x1b[?25l\x1b[?2026h\x1b[?2026l\x1b[?2026h\x1b[?2026l",
        );
        feed(
            &mut inv,
            b"\x1b]0;title\x07\x1b]7501;state=working\x07\x1b]7501;?\x07\x1b]11;?\x07\x1b]52;c;?\x07\x1b]9;4;3\x07",
        );
        feed(
            &mut inv,
            b"\x1b[?u\x1b[>1u\x1b[c\x1b[>q\x1b[?996n\x1b[?2026$p\x1b[18t\x1b[16t\x1bP$qm\x1b\\\x1bP+q544e\x1b\\",
        );
        assert_eq!(inv.count("DECSET ?2026"), 2);
        assert_eq!(inv.count("DECRST ?2026"), 2);
        assert_eq!(
            inv.entry("DECSET ?2027").unwrap().support,
            Support::No("cells are code-point based; ConPTY runs in wcswidth mode")
        );
        assert_eq!(inv.entry("DECSET ?1006").unwrap().support, Support::Yes);
        assert_eq!(inv.count("OSC 7501 set"), 1);
        assert_eq!(inv.count("OSC 7501 ?"), 1);
        assert_eq!(inv.count("OSC 9;4"), 1);
        assert_eq!(inv.count("CSI > 1 u"), 1);
        assert_eq!(inv.count("CSI ? u"), 1);
        assert_eq!(inv.count("DCS $ q (DECRQSS)"), 1);
        assert_eq!(inv.count("DCS + q (XTGETTCAP)"), 1);
        assert_eq!(inv.count("CSI 16 t"), 1);
    }

    #[test]
    fn a_split_stream_counts_the_same_as_a_whole_one() {
        let stream = b"\x1b[?1049h\x1b[?2004h\x1b]0;t\x07\x1b[c";
        let mut whole = Inventory::new();
        feed(&mut whole, stream);
        let mut split = Inventory::new();
        for byte in stream {
            feed(&mut split, std::slice::from_ref(byte));
        }
        let keys = |inv: &Inventory| inv.entries().map(|e| (e.key.clone(), e.count)).collect::<Vec<_>>();
        assert_eq!(keys(&whole), keys(&split));
    }

    #[test]
    fn replies_are_matched_to_queries_by_group() {
        let mut inv = Inventory::new();
        feed(&mut inv, b"\x1b[?u\x1b[c\x1b[>q\x1b]7501;?\x07\x1b[16t\x1b[?6n");
        let at = Duration::from_millis(2);
        inv.note_reply(b"\x1b[?0u", at);
        inv.note_reply(b"\x1b[?6c", at);
        inv.note_reply(b"\x1bP>|Nekominal 0.1.0\x1b\\", at);
        inv.note_reply(b"\x1b]7501;?\x1b\\", at);
        assert_eq!(inv.answer_for(Group::KittyKeyboard), Answer::Answered { queries: 1, replies: 1 });
        assert_eq!(inv.answer_for(Group::Da1), Answer::Answered { queries: 1, replies: 1 });
        assert_eq!(inv.answer_for(Group::Xtwinops(16)), Answer::Unanswered { queries: 1 });
        assert_eq!(inv.answer_for(Group::CprPrivate), Answer::Unanswered { queries: 1 });
        assert_eq!(inv.answer_for(Group::Dsr5), Answer::NotAQuery);
        assert!(inv.replies_follow_query_order());
        let mut unanswered = inv.unanswered();
        unanswered.sort();
        assert_eq!(unanswered, vec![Group::CprPrivate, Group::Xtwinops(16)]);
    }

    #[test]
    fn out_of_order_replies_are_detected() {
        let mut inv = Inventory::new();
        feed(&mut inv, b"\x1b[?u\x1b[c");
        let at = Duration::ZERO;
        inv.note_reply(b"\x1b[?6c", at);
        inv.note_reply(b"\x1b[?0u", at);
        assert!(!inv.replies_follow_query_order());
    }

    #[test]
    fn reply_shapes_are_recognised() {
        assert_eq!(classify_reply(b"\x1b[?6c"), vec![Group::Da1]);
        assert_eq!(classify_reply(b"\x1b[>0;2600;1c"), vec![Group::Da2]);
        assert_eq!(classify_reply(b"\x1b[0n"), vec![Group::Dsr5]);
        assert_eq!(classify_reply(b"\x1b[3;7R"), vec![Group::Dsr6]);
        assert_eq!(classify_reply(b"\x1b[?2004;2$y"), vec![Group::DecrqmPrivate(2004)]);
        assert_eq!(classify_reply(b"\x1b[8;30;100t"), vec![Group::Xtwinops(18)]);
        assert_eq!(classify_reply(b"\x1b[4;480;800t"), vec![Group::Xtwinops(14)]);
        assert_eq!(classify_reply(b"\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\"), vec![Group::OscColor(11)]);
        assert_eq!(classify_reply(b"\x1b[?997;1n"), vec![Group::ColorScheme]);
        assert_eq!(classify_reply(b"\x1bP0$r\x1b\\"), vec![Group::Decrqss]);
        assert_eq!(classify_reply(b"\x1bP0+r\x1b\\"), vec![Group::Xtgettcap]);
        assert!(classify_reply(b"plain text").is_empty());
    }
}
