//! Parsers for OSC payloads that alacritty_terminal does not surface (port of
//! `legacy/tauri-xterm/src/osc.ts`). Every function takes the payload *after* the OSC number, with
//! the parameters that vte split on `;` joined back together.

/// `OSC 7 ; file://host/C:/dir` to a Windows path, or `None` when it is not a local file URI.
/// A POSIX path (for example inside WSL) is returned unchanged.
pub fn parse_osc7(data: &str) -> Option<String> {
    let rest = strip_prefix_ignore_case(data, "file://")?;
    // authority ends at the first '/', the path runs until '?' or '#'.
    let slash = rest.find('/')?;
    let path_and_more = &rest[slash..];
    let end = path_and_more.find(['?', '#']).unwrap_or(path_and_more.len());
    let path = percent_decode(&path_and_more[..end])?;
    let bytes = path.as_bytes();
    // "/C:/Users/me" has a drive letter; anything else is a POSIX path.
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        Some(path[1..].replace('/', "\\"))
    } else {
        Some(path)
    }
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// `decodeURIComponent`: invalid escapes or non-UTF-8 results are an error.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = (*bytes.get(i + 1)? as char).to_digit(16)?;
            let lo = (*bytes.get(i + 2)? as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressState {
    Normal,
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Value { state: ProgressState, value: f64 },
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Osc9 {
    Cwd(String),
    /// `None` clears the progress indicator.
    Progress(Option<Progress>),
    Notification(String),
    Ignored,
}

/// JavaScript `Number(text)` for the inputs that matter here: blank is 0, junk is NaN (`None`).
fn js_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    trimmed.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `OSC 9` carries three unrelated things: iTerm2 notifications (`text`), ConEmu working
/// directory (`9;"path"`) and ConEmu progress (`4;state;value`).
pub fn parse_osc9(data: &str) -> Osc9 {
    let mut parts = data.split(';');
    let command = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();

    if command == "9" && !rest.is_empty() {
        let joined = rest.join(";");
        let path = joined
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(&joined);
        return Osc9::Cwd(path.to_owned());
    }
    if command == "4" {
        let state = js_number(rest.first().copied().unwrap_or("0")).filter(|v| v.fract() == 0.0).map(|v| v as i64);
        let value = js_number(rest.get(1).copied().unwrap_or("0"))
            .unwrap_or(0.0)
            .clamp(0.0, 100.0);
        let progress = |state| Osc9::Progress(Some(Progress::Value { state, value }));
        return match state {
            Some(0) => Osc9::Progress(None),
            Some(1) => progress(ProgressState::Normal),
            Some(2) => progress(ProgressState::Error),
            Some(3) => Osc9::Progress(Some(Progress::Indeterminate)),
            Some(4) => progress(ProgressState::Warning),
            _ => Osc9::Ignored,
        };
    }
    // Other numeric ConEmu commands (sleep, tab title, ...) are not notifications.
    if !command.is_empty() && command.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty() {
        return Osc9::Ignored;
    }
    if data.is_empty() {
        Osc9::Ignored
    } else {
        Osc9::Notification(data.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// `OSC 777 ; notify ; title ; body`
pub fn parse_osc777(data: &str) -> Option<Notification> {
    let mut parts = data.split(';');
    if parts.next()? != "notify" {
        return None;
    }
    let title = parts.next().unwrap_or("").to_owned();
    let body = parts.collect::<Vec<_>>().join(";");
    Some(Notification { title, body })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptMark {
    PromptStart,
    CommandStart,
    OutputStart,
    CommandEnd { exit_code: Option<i64> },
}

/// `OSC 133 ; A|B|C|D[;exit]` semantic prompt marks.
pub fn parse_osc133(data: &str) -> Option<PromptMark> {
    let mut parts = data.split(';');
    match parts.next()? {
        "A" => Some(PromptMark::PromptStart),
        "B" => Some(PromptMark::CommandStart),
        "C" => Some(PromptMark::OutputStart),
        "D" => {
            let exit_code = parts.find_map(|param| {
                let digits = param.strip_prefix('-').unwrap_or(param);
                (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| param.parse::<i64>().ok())
                    .flatten()
            });
            Some(PromptMark::CommandEnd { exit_code })
        }
        _ => None,
    }
}

/// Shells title themselves with their executable path before the first prompt; show `fallback` then.
pub fn display_title(title: &str, fallback: &str) -> String {
    let trimmed = title.trim();
    let lower = trimmed.strip_suffix(['"', '\'']).unwrap_or(trimmed).to_ascii_lowercase();
    if trimmed.is_empty() || lower.ends_with(".exe") {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Removes C0/C1 controls and bidi overrides from text that came from a program and will be shown
/// in the UI (titles, notification bodies).
pub fn strip_control(text: &str) -> String {
    text.chars().filter(|c| !is_control_or_bidi(*c)).collect()
}

/// What text from a program must not carry into the interface: C0, DEL and C1 controls, the directional marks
/// (ALM, LRM, RLM) and the embeddings, overrides and isolates (U+202A..202E, U+2066..2069), the line and paragraph
/// separators, and the noncharacters (U+FDD0..FDEF and the last two code points of every plane). The joiners that
/// real text and emoji need (U+200B..200D) stay.
pub fn is_control_or_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{fdd0}'..='\u{fdef}'
    ) || (u32::from(c) & 0xFFFE) == 0xFFFE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_from_a_program_loses_controls_directional_marks_separators_and_noncharacters() {
        for c in [
            '\u{0}', '\u{1b}', '\u{7f}', '\u{9b}', '\u{61c}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}',
            '\u{202e}', '\u{2066}', '\u{2069}', '\u{fdd0}', '\u{fdef}', '\u{fffe}', '\u{ffff}', '\u{1fffe}', '\u{10ffff}',
        ] {
            assert!(is_control_or_bidi(c), "U+{:04X} is removed", u32::from(c));
        }
        // Text, the joiners of emoji and the neighbours of every removed range stay.
        for c in [
            'a', ' ', '終', '\u{a0}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2027}', '\u{202f}', '\u{2060}', '\u{fdcf}',
            '\u{fdf0}', '\u{fffd}', '\u{1f600}', '\u{10fffd}',
        ] {
            assert!(!is_control_or_bidi(c), "U+{:04X} stays", u32::from(c));
        }
        assert_eq!(strip_control("a\u{200e}b\u{2028}c\u{fffe}d\u{202e}e"), "abcde");
    }

    #[test]
    fn osc7_converts_windows_file_uri() {
        assert_eq!(parse_osc7("file://PC/C:/Users/me/My%20Docs").as_deref(), Some("C:\\Users\\me\\My Docs"));
        assert_eq!(parse_osc7("file:///D:/x").as_deref(), Some("D:\\x"));
    }

    #[test]
    fn osc7_keeps_posix_paths_and_rejects_garbage() {
        assert_eq!(parse_osc7("file://host/home/me").as_deref(), Some("/home/me"));
        assert_eq!(parse_osc7("https://example.com/x"), None);
        assert_eq!(parse_osc7("not a uri"), None);
        assert_eq!(parse_osc7("file://host/bad%zz"), None);
        assert_eq!(parse_osc7("file://host/%E4%BD%A0%E5%A5%BD").as_deref(), Some("/你好"));
    }

    #[test]
    fn osc9_forms() {
        assert_eq!(parse_osc9("9;\"C:\\work dir\""), Osc9::Cwd("C:\\work dir".into()));
        assert_eq!(parse_osc9("9;C:\\work"), Osc9::Cwd("C:\\work".into()));
        assert_eq!(
            parse_osc9("4;1;42"),
            Osc9::Progress(Some(Progress::Value { state: ProgressState::Normal, value: 42.0 }))
        );
        assert_eq!(parse_osc9("4;3"), Osc9::Progress(Some(Progress::Indeterminate)));
        assert_eq!(parse_osc9("4;0"), Osc9::Progress(None));
        assert_eq!(
            parse_osc9("4;1;250"),
            Osc9::Progress(Some(Progress::Value { state: ProgressState::Normal, value: 100.0 }))
        );
        assert_eq!(parse_osc9("Build finished"), Osc9::Notification("Build finished".into()));
        assert_eq!(parse_osc9("1;5"), Osc9::Ignored);
        assert_eq!(parse_osc9(""), Osc9::Ignored);
    }

    #[test]
    fn osc777_and_133() {
        assert_eq!(
            parse_osc777("notify;Claude;Needs input; now"),
            Some(Notification { title: "Claude".into(), body: "Needs input; now".into() })
        );
        assert_eq!(parse_osc777("precmd"), None);
        assert_eq!(parse_osc133("A"), Some(PromptMark::PromptStart));
        assert_eq!(parse_osc133("D;127;aid=1"), Some(PromptMark::CommandEnd { exit_code: Some(127) }));
        assert_eq!(parse_osc133("D"), Some(PromptMark::CommandEnd { exit_code: None }));
        assert_eq!(parse_osc133("D;-1"), Some(PromptMark::CommandEnd { exit_code: Some(-1) }));
        assert_eq!(parse_osc133("Z"), None);
    }

    #[test]
    fn display_title_replaces_executable_paths() {
        assert_eq!(display_title("C:\\Program Files\\PowerShell\\7\\pwsh.exe", "PowerShell"), "PowerShell");
        assert_eq!(display_title("\"C:\\Windows\\System32\\cmd.exe\"", "cmd"), "cmd");
        assert_eq!(display_title("  ", "PowerShell"), "PowerShell");
        assert_eq!(display_title("npm run build", "PowerShell"), "npm run build");
    }
}
