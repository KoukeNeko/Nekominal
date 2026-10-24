//! Links: URLs found in plain text, and the one place that decides which link may be opened.
//!
//! A link comes from a program (an OSC 8 hyperlink stored on cells) or from the text itself ([`find_urls`]).
//! Either way it is only a link to the user, hovered and opened, when [`open_target`] accepts it: a short list of
//! schemes, nothing that names a program, a script or a shortcut, and a `file:` link only when it is a path on this
//! machine. Everything else stays text.

use std::ops::Range;

/// The most bytes a link may have. Browsers cope with much more; a link that long is not one a person follows.
const MAX_LINK_BYTES: usize = 2048;

/// Schemes [`find_urls`] looks for, written with what must follow the scheme: `//` for the ones with an authority.
const URL_PREFIXES: [&str; 5] = ["https://", "http://", "ftp://", "file://", "mailto:"];

/// What a link points at once it passed [`open_target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// `http`, `https` or `ftp`: the browser.
    Web,
    /// `mailto`: the mail program.
    Mail,
    /// `file`: a local file or folder.
    File,
}

/// What to hand to `ShellExecute` for an allowed link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTarget {
    pub kind: LinkKind,
    /// The URL as written, or for a file the Windows path it names (`C:\Users\me\a.txt`).
    pub target: String,
}

/// Why a link was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A scheme that is not on the list.
    Scheme,
    /// Control characters, spaces, backslashes, no host, too long, or another shape no link has.
    Malformed,
    /// A `file:` link that is not a path on this machine: a network share, a path of another system, a device.
    NotLocal,
    /// A file whose type runs code: a program, a script, an installer, a shortcut.
    Executable,
}

/// File types that `ShellExecute` runs, or that run code once opened: programs, scripts, installers, shortcuts,
/// registry and help files, ClickOnce and the other things Explorer treats as launchers.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "ps1", "ps1xml", "psm1", "psd1", "psc1", "vbs", "vbe", "js", "jse", "wsf",
    "wsh", "ws", "hta", "msi", "msp", "mst", "msc", "cpl", "dll", "sys", "lnk", "url", "reg", "inf", "scf", "jar",
    "application", "gadget", "appref-ms", "settingcontent-ms", "library-ms", "search-ms", "chm", "hlp", "iqy", "slk",
    "docm", "dotm", "xlsm", "xltm", "xlam", "pptm", "potm", "ppam", "ppsm", "sldm",
];

/// Characters that show nothing or reorder what is shown around them: a link that has one is not what it looks like.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

fn is_url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c)
}

fn starts_with_ignore_case(cells: &[char], at: usize, prefix: &str) -> bool {
    prefix.chars().enumerate().all(|(offset, p)| cells.get(at + offset).is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

/// The URLs in one line of text, one `char` per cell (a cell that holds no character, such as the spacer of a wide
/// character, is `'\0'`), as ranges of those cells. Simple and conservative: a known scheme at the start of a word,
/// then ASCII URL characters; the punctuation that ends a sentence, and a closing bracket nothing opened, are not
/// part of it. A wide or any other non-ASCII character ends a URL.
pub fn find_urls(cells: &[char]) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < cells.len() {
        let word_start = at == 0 || !(cells[at - 1].is_ascii_alphanumeric() || "+-.".contains(cells[at - 1]));
        let prefix = word_start.then(|| URL_PREFIXES.iter().find(|p| starts_with_ignore_case(cells, at, p))).flatten();
        let Some(prefix) = prefix else {
            at += 1;
            continue;
        };
        let mut end = at + prefix.len();
        while end < cells.len() && is_url_char(cells[end]) {
            end += 1;
        }
        // Trimming never eats into the scheme: `mailto:` alone loses its colon otherwise.
        let end = (at + trim_end(&cells[at..end])).max(at + prefix.len());
        let rest = &cells[at + prefix.len()..end];
        let has_body = match *prefix {
            "mailto:" => rest.contains(&'@'),
            // `file:///C:/x` has an empty host: the third slash is where the path starts.
            "file://" => !rest.is_empty(),
            _ => rest.first().is_some_and(|c| c.is_ascii_alphanumeric() || *c == '['),
        };
        if has_body {
            found.push(at..end);
            at = end;
        } else {
            at += prefix.len();
        }
    }
    found
}

/// Length of `url` without the characters that end a sentence or close a bracket nothing opened.
fn trim_end(url: &[char]) -> usize {
    let mut end = url.len();
    while end > 0 {
        let c = url[end - 1];
        let unopened = |open: char| url[..end].iter().filter(|x| **x == c).count() > url[..end].iter().filter(|x| **x == open).count();
        let trim = match c {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
            ')' => unopened('('),
            ']' => unopened('['),
            _ => false,
        };
        if !trim {
            break;
        }
        end -= 1;
    }
    end
}

/// Full-width and CJK punctuation, the ideographic space and the general punctuation of running text: no address a
/// person writes goes on through them, though the letters around them may be any script.
fn ends_text(c: char) -> bool {
    c.is_whitespace()
        || matches!(c, '\u{2010}'..='\u{2027}' | '\u{2030}'..='\u{205e}' | '\u{3000}'..='\u{303f}' | '\u{30fb}' | '\u{fe10}'..='\u{fe6f}'
            | '\u{ff01}'..='\u{ff0f}' | '\u{ff1a}'..='\u{ff20}' | '\u{ff3b}'..='\u{ff40}' | '\u{ff5b}'..='\u{ff65}')
}

/// How many of `cells` an OSC 8 link keeps when its text is its own address: a program that marks the URLs of its text
/// itself (Claude Code does) can run one on into the sentence after it, `https://a.b/c・結束・`. The link then ends
/// before the first full-width or CJK punctuation, without the punctuation that ends a sentence, as [`find_urls`]
/// would end it. `None` when the text is not the address (a link with words of its own) or nothing has to go.
/// `cells` is one `char` per cell, `'\0'` for the spacer of a wide character.
pub fn autolink_end(cells: &[char], uri: &str) -> Option<usize> {
    let text: String = cells.iter().filter(|c| **c != '\0').collect();
    if text != uri && percent_decode(uri).as_deref() != Some(text.as_str()) {
        return None;
    }
    let prefix = URL_PREFIXES.iter().find(|p| starts_with_ignore_case(cells, 0, p))?;
    let cut = cells.iter().position(|c| ends_text(*c)).unwrap_or(cells.len());
    let end = trim_end(&cells[..cut]);
    (end < cells.len() && end > prefix.len()).then_some(end)
}

/// The only gate a link passes before anything is opened. See the module documentation.
pub fn open_target(uri: &str) -> Result<OpenTarget, Refusal> {
    if uri.len() > MAX_LINK_BYTES || uri.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\' || is_invisible(c)) {
        return Err(Refusal::Malformed);
    }
    let (scheme, rest) = uri.split_once(':').ok_or(Refusal::Scheme)?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "ftp" => {
            let authority = rest.strip_prefix("//").ok_or(Refusal::Malformed)?;
            let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
            let host = authority.rsplit('@').next().unwrap_or_default();
            if host.is_empty() || host.starts_with(':') {
                return Err(Refusal::Malformed);
            }
            Ok(OpenTarget { kind: LinkKind::Web, target: uri.to_owned() })
        }
        "mailto" => {
            let address = rest.split('?').next().unwrap_or_default();
            // Some mail programs attach the file `attach=` names to the message.
            let attaches = rest.to_ascii_lowercase().contains("attach=");
            if address.is_empty() || rest.starts_with("//") || attaches {
                return Err(Refusal::Malformed);
            }
            Ok(OpenTarget { kind: LinkKind::Mail, target: uri.to_owned() })
        }
        "file" => local_path(rest).map(|target| OpenTarget { kind: LinkKind::File, target }),
        _ => Err(Refusal::Scheme),
    }
}

/// The Windows path a `file:` link names, when it is a path on this machine and not a file that runs code.
/// `rest` is what follows `file:`.
fn local_path(rest: &str) -> Result<String, Refusal> {
    let after_slashes = rest.strip_prefix("//").ok_or(Refusal::NotLocal)?;
    // `file:///C:/x` has an empty authority, `file://localhost/C:/x` names this machine; any other host is a share.
    let (authority, path) = after_slashes.split_at(after_slashes.find('/').unwrap_or(after_slashes.len()));
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return Err(Refusal::NotLocal);
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(path).ok_or(Refusal::Malformed)?;
    if decoded.contains('\\') || decoded.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err(Refusal::Malformed);
    }
    let bytes = decoded.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if !drive || bytes.get(3).is_some_and(|b| *b != b'/') {
        return Err(Refusal::NotLocal);
    }
    let windows = decoded[1..].replace('/', "\\");
    let mut components = windows.split('\\');
    let drive = components.next().unwrap_or_default();
    // A colon after the drive names an alternate data stream; the rest are not valid in a file name.
    let names: Vec<&str> = components.collect();
    if names.iter().any(|name| name.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))) {
        return Err(Refusal::Malformed);
    }
    // Windows ignores trailing dots and spaces when it opens a file, so `a.exe.` runs a.exe.
    let last = names.iter().rev().find(|name| !name.is_empty()).copied().unwrap_or_default();
    let last = last.trim_end_matches(['.', ' ']);
    if let Some((_, extension)) = last.rsplit_once('.')
        && EXECUTABLE_EXTENSIONS.iter().any(|runs| extension.eq_ignore_ascii_case(runs))
    {
        return Err(Refusal::Executable);
    }
    let path: Vec<&str> = std::iter::once(drive).chain(names.iter().copied()).collect();
    Ok(if names.iter().all(|name| name.is_empty()) { format!("{drive}\\") } else { path.join("\\") })
}

/// `decodeURIComponent` of a path: an invalid escape, or a result that is not UTF-8, is an error.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let high = (*bytes.get(at + 1)? as char).to_digit(16)?;
            let low = (*bytes.get(at + 2)? as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A link under a cell: where it goes, and the cells it covers in the viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// What the link points at, as the program or the text wrote it.
    pub uri: String,
    /// The cells it covers, one span per row of the viewport: (row, first column, end column), the end exclusive.
    pub spans: Vec<(usize, usize, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<String> {
        let cells: Vec<char> = text.chars().collect();
        find_urls(&cells).into_iter().map(|range| cells[range].iter().collect()).collect()
    }

    #[test]
    fn a_url_ends_before_the_punctuation_that_ends_a_sentence() {
        assert_eq!(urls("see https://example.com/a?b=c#d."), ["https://example.com/a?b=c#d"]);
        assert_eq!(urls("go to http://a.b/c, then http://d.e/f; or http://g.h/i: yes"), ["http://a.b/c", "http://d.e/f", "http://g.h/i"]);
        assert_eq!(urls("really http://a.b/c?!"), ["http://a.b/c"]);
        assert_eq!(urls("'https://a.b/c'"), ["https://a.b/c"]);
    }

    #[test]
    fn brackets_belong_to_a_url_only_when_balanced() {
        assert_eq!(urls("(https://a.b/c)"), ["https://a.b/c"]);
        assert_eq!(urls("(see https://en.wikipedia.org/wiki/Rust_(language))"), ["https://en.wikipedia.org/wiki/Rust_(language)"]);
        assert_eq!(urls("[https://a.b/c]"), ["https://a.b/c"]);
        assert_eq!(urls("https://a.b/c[1]"), ["https://a.b/c[1]"]);
        assert_eq!(urls("<https://a.b/c> and \"https://d.e/f\""), ["https://a.b/c", "https://d.e/f"]);
    }

    #[test]
    fn only_the_known_schemes_at_the_start_of_a_word_are_urls() {
        assert_eq!(urls("HTTPS://EXAMPLE.COM/X"), ["HTTPS://EXAMPLE.COM/X"]);
        assert_eq!(urls("ftp://files.example.com/pub file:///C:/Users/me/a.txt"), ["ftp://files.example.com/pub", "file:///C:/Users/me/a.txt"]);
        assert_eq!(urls("mailto:someone@example.com"), ["mailto:someone@example.com"]);
        assert_eq!(urls("mailto:nobody"), Vec::<String>::new());
        assert_eq!(urls("xhttps://a.b nothttp://a.b ssh://a.b javascript:alert(1)"), Vec::<String>::new());
        assert_eq!(urls("http:// https://. http://"), Vec::<String>::new(), "a scheme alone is not a link");
        assert_eq!(urls("http://[::1]:8080/x http://localhost:3000"), ["http://[::1]:8080/x", "http://localhost:3000"]);
        assert_eq!(urls("https://a.b/x https://c.d/y"), ["https://a.b/x", "https://c.d/y"]);
    }

    #[test]
    fn a_wide_or_other_non_ascii_character_ends_a_url_and_a_missing_cell_does_too() {
        assert_eq!(urls("https://example.com/貓 下一句"), ["https://example.com/"]);
        assert_eq!(urls("https://example.com，下一句"), ["https://example.com"]);
        let cells: Vec<char> = "https://ex\0ample.com".chars().collect();
        assert_eq!(find_urls(&cells), [0..10]);
    }

    #[test]
    fn a_program_s_autolink_ends_at_full_width_punctuation_and_keeps_letters_of_any_script() {
        let end = |text: &str, uri: &str| autolink_end(&text.chars().collect::<Vec<_>>(), uri);
        let text = "https://example.com/full・結束・";
        assert_eq!(end(text, text), Some("https://example.com/full".chars().count()));
        assert_eq!(end(text, "https://example.com/full%E3%83%BB%E7%B5%90%E6%9D%9F%E3%83%BB"), Some(24), "an encoded address is the same address");
        assert_eq!(end("https://example.com/end，然後", "https://example.com/end，然後"), Some(23));
        assert_eq!(end("https://example.com/end.", "https://example.com/end."), Some(23));
        assert_eq!(end("https://example.com/in-parens)", "https://example.com/in-parens)"), Some(29));
        let wiki = "https://zh.wikipedia.org/wiki/中華民國";
        assert_eq!(end(wiki, wiki), None, "a path in Chinese is kept whole");
        assert_eq!(end("可點的文字連結", "https://example.com/"), None, "a link with words of its own is the program's to say");
        assert_eq!(end("https://example.com/x", "https://example.com/x"), None);
        assert_eq!(end("https://・", "https://・"), None, "a scheme alone is not a link");
        let spaced: Vec<char> = "https://a.b/貓\0。".chars().collect();
        assert_eq!(autolink_end(&spaced, "https://a.b/貓。"), Some(14), "the spacer of a wide letter stays with it");
    }

    #[test]
    fn the_ranges_are_cell_positions_in_the_text() {
        let cells: Vec<char> = "ab https://a.b/c d".chars().collect();
        assert_eq!(find_urls(&cells), [3..16]);
        assert!(find_urls(&[]).is_empty());
    }

    fn accepted(uri: &str) -> OpenTarget {
        open_target(uri).unwrap_or_else(|why| panic!("{uri}: {why:?}"))
    }

    #[test]
    fn web_mail_and_local_file_links_are_accepted() {
        for uri in ["http://example.com", "https://example.com/a?b=c#d", "HTTPS://Example.com", "ftp://files.example.com/pub", "http://[::1]:8080/", "https://user@example.com/x", "http://localhost:3000"] {
            assert_eq!(accepted(uri), OpenTarget { kind: LinkKind::Web, target: uri.to_owned() });
        }
        assert_eq!(accepted("mailto:me@example.com?subject=hi").kind, LinkKind::Mail);
        assert_eq!(accepted("file:///C:/Users/me/a.txt"), OpenTarget { kind: LinkKind::File, target: "C:\\Users\\me\\a.txt".to_owned() });
        assert_eq!(accepted("file:///C:/Users/me/My%20Docs/a%20b.txt").target, "C:\\Users\\me\\My Docs\\a b.txt");
        assert_eq!(accepted("file://localhost/D:/x/y.txt").target, "D:\\x\\y.txt");
        assert_eq!(accepted("FILE:///c:/work/").target, "c:\\work\\", "a folder opens in Explorer");
        assert_eq!(accepted("file:///C:/").target, "C:\\");
        assert_eq!(accepted("file:///C:/%E4%BD%A0%E5%A5%BD.txt").target, "C:\\你好.txt");
        assert_eq!(accepted("file:///C:/a.txt#line5").target, "C:\\a.txt");
    }

    #[test]
    fn every_other_scheme_is_refused() {
        for uri in [
            "javascript:alert(1)", "data:text/html,<script>1</script>", "vbscript:msgbox(1)", "ms-msdt:/id%20PCWDiagnostic", "search-ms:query=x",
            "ssh://host", "steam://run/1", "telnet://host", "custom:thing", "ms-settings:network", "ldap://host", "calc.exe", "C:\\Windows\\System32\\calc.exe", "no scheme at all",
        ] {
            assert!(matches!(open_target(uri), Err(Refusal::Scheme | Refusal::Malformed)), "{uri}");
        }
        assert_eq!(open_target("javascript:alert(1)"), Err(Refusal::Scheme));
        assert_eq!(open_target("calc.exe"), Err(Refusal::Scheme));
    }

    #[test]
    fn a_web_link_needs_a_host_and_no_tricks() {
        for uri in ["http://", "https:///path", "http:example.com", "https://:80/x", "https://user@/x", "https://a.b\\@c.d", "https://a b.c", "https://a.b/\u{7}", "https://a.b/\u{202e}x"] {
            assert_eq!(open_target(uri), Err(Refusal::Malformed), "{uri:?}");
        }
        assert_eq!(open_target(&format!("https://a.b/{}", "x".repeat(MAX_LINK_BYTES))), Err(Refusal::Malformed));
        assert!(open_target(&format!("https://a.b/{}", "x".repeat(MAX_LINK_BYTES - 12))).is_ok(), "the limit itself is allowed");
    }

    #[test]
    fn a_mail_link_may_not_attach_a_file() {
        assert_eq!(open_target("mailto:"), Err(Refusal::Malformed));
        assert_eq!(open_target("mailto://x"), Err(Refusal::Malformed));
        assert_eq!(open_target("mailto:a@b.c?attach=C:/secret.txt"), Err(Refusal::Malformed));
        assert_eq!(open_target("mailto:a@b.c?subject=x&ATTACH=C:/s"), Err(Refusal::Malformed));
    }

    #[test]
    fn a_file_link_must_name_a_path_on_this_machine() {
        for uri in ["file://server/share/x.txt", "file:////server/share/x.txt", "file:///home/me/x.txt", "file:///x.txt", "file://C:/x.txt", "file:C:/x.txt", "file:///C:x.txt", "file:///1:/x.txt"] {
            assert_eq!(open_target(uri), Err(Refusal::NotLocal), "{uri}");
        }
        for uri in ["file:///C:/a%zz", "file:///C:/a%FF", "file:///C:/a/b:c.txt", "file:///C:/a/b%3Ac.txt", "file:///C:/a/b%00c", "file:///C:/a/b|c", "file:///C:/a/%5Cb"] {
            assert_eq!(open_target(uri), Err(Refusal::Malformed), "{uri}");
        }
    }

    #[test]
    fn a_file_that_runs_code_is_never_opened_however_it_is_dressed() {
        for uri in [
            "file:///C:/Windows/System32/calc.exe", "file:///C:/x/run.BAT", "file:///C:/x/a.cmd", "file:///C:/x/a.ps1", "file:///C:/x/a.vbs", "file:///C:/x/a.js",
            "file:///C:/x/a.msi", "file:///C:/x/a.lnk", "file:///C:/x/a.url", "file:///C:/x/a.hta", "file:///C:/x/a.reg", "file:///C:/x/a.jar", "file:///C:/x/a.docm",
            "file:///C:/x/a.exe.", "file:///C:/x/a.exe%20", "file:///C:/x/a.exe%20.", "file:///C:/x/a.Exe", "file:///C:/x/a.txt.exe", "file:///C:/x/a.scr", "file:///C:/x/a.settingcontent-ms",
        ] {
            assert_eq!(open_target(uri), Err(Refusal::Executable), "{uri}");
        }
        assert!(open_target("file:///C:/x/a.exe.txt").is_ok());
        assert!(open_target("file:///C:/x/exe").is_ok());
        assert!(open_target("file:///C:/x/folder.d/").is_ok());
        assert!(open_target("file:///C:/x/a.pdf").is_ok());
    }
}
