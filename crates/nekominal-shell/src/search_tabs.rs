//! Search all tabs: the find bar's search run over every pane of every tab, its matches listed in the palette (the line
//! of each with the title of its tab), and the one chosen shown in its pane, selected.

use nekominal_model::layout::PaneId;
use nekominal_term::{SearchMatch, TerminalCore};

use crate::{find, popup, tab::Tab};

/// Most matches one pane gives the list, and the most the list shows: enough to find something, few enough to draw on
/// every keystroke.
const PER_PANE: usize = 1000;
pub const MAX_RESULTS: usize = 200;
/// Characters of the line a row shows.
const SNIPPET: usize = 90;

/// A match in the list: the tab (by its `uid`, which survives the tabs being moved or closed) and pane it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub tab: u64,
    pub pane: PaneId,
    pub found: SearchMatch,
}

/// Where a match comes in the list: the selected tab first, then the others in the order of the strip; in a tab its panes
/// in their order; in a pane the newest output first, as the find bar starts from the last match. `tab` and `pane` are
/// positions, `found` the match.
pub fn rank(selected: usize, tab: usize, pane: usize, found: &SearchMatch) -> (bool, usize, usize, std::cmp::Reverse<(i32, usize)>) {
    (tab != selected, tab, pane, std::cmp::Reverse(found.start))
}

/// The part of `line` a row shows: all of it when it is short, else from a little before the match at `column`, with an
/// ellipsis for what was left out. The blanks around it go.
pub fn snippet(line: &str, column: usize) -> String {
    let chars: Vec<char> = line.trim_end().chars().collect();
    let first = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let start = if chars.len() - first <= SNIPPET { first } else { column.saturating_sub(SNIPPET / 4).clamp(first, chars.len().saturating_sub(SNIPPET)) };
    let end = (start + SNIPPET).min(chars.len());
    let mut out = String::new();
    if start > first {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// The matches of `query` in every pane of `tabs`, in the order of [`rank`] and at most [`MAX_RESULTS`], with the rows
/// the palette shows for them. `title` is what the strip calls tab `i`.
pub fn search(tabs: &[Tab], selected: usize, query: &str, options: find::Options, title: impl Fn(usize) -> String) -> (Vec<Hit>, Vec<popup::Row>) {
    if query.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let pattern = find::pattern(query, options);
    let mut found = Vec::new();
    for (t, tab) in tabs.iter().enumerate() {
        for (p, entry) in tab.panes.iter().enumerate() {
            let core = entry.pane.term.lock();
            // A pattern that does not compile finds nothing, as in the find bar.
            for m in core.search_all(&pattern, PER_PANE).unwrap_or_default() {
                let line = core.text_between((m.start.0, 0), (m.start.0, core.columns().saturating_sub(1)));
                found.push((rank(selected, t, p, &m), Hit { tab: tab.uid, pane: entry.id, found: m }, snippet(&line, m.start.1), t));
            }
        }
    }
    found.sort_by_key(|f| f.0);
    found.truncate(MAX_RESULTS);
    let rows = found
        .iter()
        .enumerate()
        .map(|(i, (_, _, line, t))| popup::Row { label: line.clone(), hint: title(*t), action: Some(popup::Action::SearchHit(i)), current: *t == selected, mark: None, program: None })
        .collect();
    (found.into_iter().map(|(_, hit, _, _)| hit).collect(), rows)
}

/// Scrolls the pane to `found` when it is out of view, as the find bar does, and selects it.
pub fn reveal(core: &mut TerminalCore, found: SearchMatch) {
    let (rows, top) = (core.rows() as i32, -(core.display_offset() as i32));
    if found.start.0 < top || found.end.0 > top + rows - 1 {
        core.scroll_to_line(found.start.0 - rows / 2);
    }
    core.select_between(found.start, found.end);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: i32, column: usize) -> SearchMatch {
        SearchMatch { start: (line, column), end: (line, column + 2) }
    }

    #[test]
    fn the_selected_tab_comes_first_then_the_strip_and_the_newest_output_of_a_pane_first() {
        // (tab, pane, match) of five matches, the second tab selected.
        let mut found = vec![(0, 0, at(-5, 0)), (1, 1, at(3, 0)), (2, 0, at(0, 0)), (1, 0, at(-2, 4)), (1, 0, at(7, 1)), (1, 0, at(7, 0))];
        found.sort_by_key(|(tab, pane, m)| rank(1, *tab, *pane, m));
        let order: Vec<(usize, usize, (i32, usize))> = found.iter().map(|(t, p, m)| (*t, *p, m.start)).collect();
        assert_eq!(order, [(1, 0, (7, 1)), (1, 0, (7, 0)), (1, 0, (-2, 4)), (1, 1, (3, 0)), (0, 0, (-5, 0)), (2, 0, (0, 0))]);
    }

    #[test]
    fn a_long_line_shows_the_part_around_the_match_and_says_what_was_left_out() {
        assert_eq!(snippet("    cargo build  ", 4), "cargo build");
        let long: String = (0..300).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let shown = snippet(&long, 150);
        assert!(shown.starts_with('…') && shown.ends_with('…'), "{shown}");
        assert!(shown.contains(&long[148..152]), "the match is in view");
        assert_eq!(snippet(&long, 0).chars().next(), Some('a'), "a match near the start shows the start");
        assert!(snippet(&long, 299).ends_with(&long[290..]), "a match near the end shows the end");
    }
}
