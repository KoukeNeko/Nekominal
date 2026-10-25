//! Hint mode: the URLs, file paths and git hashes on the screen of a pane, each with a short label the keyboard picks
//! it by, as kitty's hints and WezTerm's quick select do. Pure: it reads a snapshot.

use nekominal_term::{Snapshot, alacritty_terminal::term::cell::Flags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Url,
    /// A path as a compiler or a tool prints it, with its `:line` or `:line:column` when it has one.
    Path,
    Hash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Where its first character is on the screen.
    pub row: usize,
    pub column: usize,
    pub text: String,
    pub kind: HintKind,
    pub label: String,
}

/// The letters labels are made of, those under the fingers first.
const ALPHABET: &str = "fjdkslaghrueiwoqptyvbcnxmz";

/// The hints on the screen, the one nearest the bottom (the latest output) with the first label. One text found twice
/// has one label; past 26 texts every label is two letters, so that no label is the start of another.
pub fn find(snapshot: &Snapshot) -> Vec<Hint> {
    let mut found: Vec<Hint> = Vec::new();
    for line in logical_lines(snapshot) {
        for (start, token) in tokens(&line) {
            let text: String = token.iter().map(|(c, _, _)| *c).collect();
            let Some((kind, length)) = classify(&text) else { continue };
            let text: String = text.chars().take(length).collect();
            let (_, row, column) = token[0];
            let _ = start;
            found.push(Hint { row, column, text, kind, label: String::new() });
        }
    }
    found.sort_by(|a, b| b.row.cmp(&a.row).then(a.column.cmp(&b.column)));
    let mut texts: Vec<String> = Vec::new();
    for hint in &found {
        if !texts.contains(&hint.text) {
            texts.push(hint.text.clone());
        }
    }
    let labels = labels(texts.len());
    for hint in &mut found {
        let at = texts.iter().position(|text| *text == hint.text).unwrap_or(0);
        hint.label = labels[at].clone();
    }
    found
}

fn labels(count: usize) -> Vec<String> {
    let letters: Vec<char> = ALPHABET.chars().collect();
    if count <= letters.len() {
        return letters.iter().take(count).map(char::to_string).collect();
    }
    letters.iter().flat_map(|a| letters.iter().map(move |b| format!("{a}{b}"))).take(count).collect()
}

/// The characters of the screen with where each is, a soft-wrapped line joined to the row it goes on to. The blank a
/// wide character leaves after itself is no character.
fn logical_lines(snapshot: &Snapshot) -> Vec<Vec<(char, usize, usize)>> {
    let mut lines = Vec::new();
    let mut line = Vec::new();
    for row in 0..snapshot.rows {
        let cells = &snapshot.cells[row * snapshot.columns..(row + 1) * snapshot.columns];
        for (column, cell) in cells.iter().enumerate() {
            if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                line.push((cell.ch, row, column));
            }
        }
        if !cells.last().is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE)) {
            lines.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The runs of characters between blanks, quotes and brackets, with where each starts in the line.
fn tokens(line: &[(char, usize, usize)]) -> Vec<(usize, &[(char, usize, usize)])> {
    let breaks = |c: char| c.is_whitespace() || "\"'`<>()[]{}|".contains(c);
    let mut out = Vec::new();
    let mut start = None;
    for (i, (c, _, _)) in line.iter().enumerate() {
        match (breaks(*c), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

/// What a token is, and how many of its characters are it: the punctuation a sentence puts after a URL or a path is not.
fn classify(token: &str) -> Option<(HintKind, usize)> {
    let trimmed = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    let length = trimmed.chars().count();
    let lower = trimmed.to_ascii_lowercase();
    if ["http://", "https://", "ftp://", "file://"].iter().any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len()) {
        return Some((HintKind::Url, length));
    }
    let hex = trimmed.chars().all(|c| c.is_ascii_hexdigit());
    if hex && (7..=40).contains(&trimmed.len()) && trimmed.chars().any(|c| c.is_ascii_digit()) && trimmed.chars().any(|c| c.is_ascii_alphabetic()) {
        return Some((HintKind::Hash, length));
    }
    let (path, _) = split_position(trimmed);
    let bytes = path.as_bytes();
    let rooted = (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'))
        || ["\\\\", "./", "../", ".\\", "..\\", "~/", "~\\"].iter().any(|start| path.starts_with(start));
    let last = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let nested = path.contains(['/', '\\']) && last.contains('.') && !last.starts_with('.') && !last.ends_with('.');
    ((rooted && path.len() > 3) || nested).then_some((HintKind::Path, length))
}

/// A path and the `:line` or `:line:column` after it (as `src/main.rs:12:5`), the position empty when there is none.
pub fn split_position(text: &str) -> (&str, &str) {
    let mut path = text;
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((before, number)) if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) && !before.is_empty() => path = before,
            _ => break,
        }
    }
    (path, &text[path.len()..])
}

#[cfg(test)]
mod tests {
    use nekominal_term::{CoreConfig, TerminalCore};

    use super::*;

    fn screen(columns: usize, text: &str) -> Snapshot {
        let mut core = TerminalCore::new(columns, 6, CoreConfig::default());
        core.feed(text.as_bytes());
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        snapshot
    }

    fn texts(hints: &[Hint]) -> Vec<(&str, HintKind, &str)> {
        hints.iter().map(|hint| (hint.text.as_str(), hint.kind, hint.label.as_str())).collect()
    }

    #[test]
    fn urls_paths_with_their_positions_and_hashes_are_found_the_latest_first() {
        let hints = find(&screen(80, "see https://example.com/a?b=1. now\r\nerror at src\\main.rs:12:5 and C:\\work\\x.txt\r\ncommit 3f2a9c1d (HEAD)"));
        assert_eq!(
            texts(&hints),
            [
                ("3f2a9c1d", HintKind::Hash, "f"),
                ("src\\main.rs:12:5", HintKind::Path, "j"),
                ("C:\\work\\x.txt", HintKind::Path, "d"),
                ("https://example.com/a?b=1", HintKind::Url, "k"),
            ]
        );
        assert_eq!((hints[0].row, hints[0].column), (2, 7));
    }

    #[test]
    fn plain_words_numbers_and_versions_are_not_hints() {
        assert!(find(&screen(80, "hello world 1234567 deadbeef 0.0.21 v1.2 ... a.b")).is_empty());
    }

    #[test]
    fn a_url_that_wraps_onto_the_next_row_is_one_hint() {
        let hints = find(&screen(20, "go https://example.com/a/long/path end"));
        assert_eq!(texts(&hints), [("https://example.com/a/long/path", HintKind::Url, "f")]);
        assert_eq!((hints[0].row, hints[0].column), (0, 3));
    }

    #[test]
    fn one_text_twice_has_one_label_and_past_twenty_six_every_label_is_two_letters() {
        let twice = find(&screen(80, "a/b.rs\r\na/b.rs"));
        assert_eq!(twice.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["f", "f"]);
        let many: String = (0..30).map(|i| format!("d{i}/f.rs ")).collect();
        let labels: Vec<String> = find(&screen(400, &many)).into_iter().map(|h| h.label).collect();
        assert_eq!(labels.len(), 30);
        assert!(labels.iter().all(|label| label.chars().count() == 2));
        assert!(labels.iter().all(|label| labels.iter().filter(|other| *other == label).count() == 1));
    }

    #[test]
    fn the_position_after_a_path_is_split_from_it() {
        assert_eq!(split_position("src/main.rs:12:5"), ("src/main.rs", ":12:5"));
        assert_eq!(split_position("src/main.rs:12"), ("src/main.rs", ":12"));
        assert_eq!(split_position("C:\\x.txt"), ("C:\\x.txt", ""));
    }
}
