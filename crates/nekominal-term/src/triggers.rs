//! Triggers: regular expressions matched against the lines of the output (iTerm2's triggers). The setting and the
//! actions are the shell's; here a line is matched, and what matched is reported (`CoreEvent::Trigger`) and, for a
//! rule with a colour, kept as a highlight of its cells.
//!
//! Which lines: a line of the primary screen is matched once, when it is complete, that is when the cursor has gone
//! below it (a wrapped line is one line, joined, matched when its last row is done). An `instant` rule matches the
//! line the cursor is on instead, as it is written, and fires once per line: a prompt such as `Proceed? [y/N]` ends
//! without a line break. Nothing on the alternate screen is matched: a full-screen program redraws its rows in place,
//! so they never complete and the same text would match on every frame.
//!
//! Known limit: a line is not matched again when a program rewrites it after the cursor went below it (the live
//! region of an Ink app such as Claude Code redraws rows it already passed): only its first text is seen.
//!
//! Cost: nothing at all without triggers. With them, each new line is read from the grid once (at most
//! [`MAX_LINE_CHARS`]) and one `RegexSet` tells which rules match; only those run again for captures and ranges.

use std::{ops::Range, sync::Arc};

use regex::{Regex, RegexSet};

/// The most characters of a line that are matched: the rest of a longer one is not.
pub const MAX_LINE_CHARS: usize = 2048;

/// One rule as the emulator needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerRule {
    /// What the shell knows the rule by (its index among the settings' triggers); comes back in the hits.
    pub id: usize,
    pub pattern: String,
    pub instant: bool,
    /// Only while the pane's program is this one (see [`program_matches`]).
    pub program: Option<String>,
    /// The background of the matched cells, if the rule highlights.
    pub highlight: Option<(u8, u8, u8)>,
}

/// The rules of a pane, compiled once.
#[derive(Debug)]
pub struct Triggers {
    rules: Vec<TriggerRule>,
    patterns: Vec<Arc<str>>,
    regexes: Vec<Regex>,
    set: RegexSet,
    any_instant: bool,
}

/// A line matched a rule.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerHit {
    /// The rule's [`TriggerRule::id`], and its pattern, so the shell can tell a hit of rules it has since changed.
    pub rule: usize,
    pub pattern: Arc<str>,
    /// The first match: `$0`, then the groups (empty for a group that did not take part).
    pub groups: Vec<String>,
}

/// What a rule found in one line: the hit, and the byte ranges of every match when the rule highlights.
#[derive(Clone, Debug, PartialEq)]
pub struct LineHit {
    pub hit: TriggerHit,
    pub highlight: Option<((u8, u8, u8), Vec<Range<usize>>)>,
}

impl Triggers {
    /// Compiles `rules`; the error names the first pattern that is not a regular expression.
    pub fn new(rules: Vec<TriggerRule>) -> Result<Triggers, String> {
        let set = RegexSet::new(rules.iter().map(|rule| &rule.pattern)).map_err(|e| e.to_string())?;
        let regexes = rules.iter().map(|rule| Regex::new(&rule.pattern)).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        Ok(Triggers {
            patterns: rules.iter().map(|rule| Arc::from(rule.pattern.as_str())).collect(),
            any_instant: rules.iter().any(|rule| rule.instant),
            rules,
            regexes,
            set,
        })
    }

    pub fn rules(&self) -> &[TriggerRule] {
        &self.rules
    }

    pub(crate) fn any_instant(&self) -> bool {
        self.any_instant
    }

    /// The rules that match `line`: the instant ones or the others, as `instant` says, and of those with a program
    /// only the ones of `program`.
    pub fn matches(&self, line: &str, instant: bool, program: Option<&str>) -> Vec<LineHit> {
        let mut hits = Vec::new();
        for index in self.set.matches(line).iter() {
            let rule = &self.rules[index];
            if rule.instant != instant || rule.program.as_deref().is_some_and(|filter| !program.is_some_and(|p| program_matches(filter, p))) {
                continue;
            }
            let regex = &self.regexes[index];
            let Some(captures) = regex.captures(line) else { continue };
            let groups = captures.iter().map(|group| group.map_or_else(String::new, |m| m.as_str().to_owned())).collect();
            let highlight = rule.highlight.map(|color| {
                (color, regex.find_iter(line).map(|m| m.range()).filter(|range| !range.is_empty()).collect())
            });
            hits.push(LineHit { hit: TriggerHit { rule: rule.id, pattern: self.patterns[index].clone(), groups }, highlight });
        }
        hits
    }
}

/// Whether the program `filter` names is `program` (a file name such as `claude.exe`): case does not count, and either
/// may leave out `.exe`.
pub fn program_matches(filter: &str, program: &str) -> bool {
    let stem = |name: &str| {
        let name = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let lower = name.to_ascii_lowercase();
        lower.strip_suffix(".exe").map(str::to_owned).unwrap_or(lower)
    };
    stem(filter) == stem(program)
}

/// A line of the grid as text, a wrapped line joined: where each character of the text is on the grid.
#[derive(Debug, Default)]
pub(crate) struct GridLine {
    pub text: String,
    /// For each character: its first byte in `text`, its grid line, its column and how many columns it takes.
    pub cells: Vec<(usize, i32, usize, usize)>,
}

impl GridLine {
    pub fn push(&mut self, ch: char, zerowidth: &[char], line: i32, column: usize, width: usize) {
        self.cells.push((self.text.len(), line, column, width));
        self.text.push(ch);
        self.text.extend(zerowidth);
    }

    /// Blanks at the end are the cells after the text, not text.
    pub fn trim_end(&mut self) {
        let length = self.text.trim_end().len();
        self.text.truncate(length);
        let kept = self.cells.partition_point(|cell| cell.0 < length);
        self.cells.truncate(kept);
    }

    /// The cells of the bytes `range` of the text, one span per grid line: (line, first column, column after the last).
    pub fn spans(&self, range: &Range<usize>) -> Vec<(i32, usize, usize)> {
        // The character the range starts in: the last one that starts at or before it.
        let first = self.cells.partition_point(|cell| cell.0 <= range.start).saturating_sub(1);
        let mut spans: Vec<(i32, usize, usize)> = Vec::new();
        for &(_, line, column, width) in self.cells[first..].iter().take_while(|cell| cell.0 < range.end) {
            match spans.last_mut() {
                Some(span) if span.0 == line => span.2 = column + width,
                _ => spans.push((line, column, column + width)),
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: usize, pattern: &str) -> TriggerRule {
        TriggerRule { id, pattern: pattern.into(), instant: false, program: None, highlight: None }
    }

    #[test]
    fn the_set_tells_which_rules_match_and_each_hit_has_its_captures() {
        let triggers = Triggers::new(vec![rule(0, r"error: (\w+)"), rule(5, "warning"), rule(7, r"(x)?done")]).unwrap();
        let hits = triggers.matches("error: disk done", false, None);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].hit.rule, hits[0].hit.groups.clone()), (0, vec!["error: disk".to_owned(), "disk".to_owned()]));
        assert_eq!((hits[1].hit.rule, hits[1].hit.groups.clone()), (7, vec!["done".to_owned(), String::new()]), "a group that took no part is empty");
        assert_eq!(&*hits[1].hit.pattern, "(x)?done");
        assert!(triggers.matches("all fine", false, None).is_empty());
        assert!(Triggers::new(vec![rule(0, "(")]).is_err());
    }

    #[test]
    fn an_instant_rule_matches_only_the_line_being_written_and_the_others_only_complete_lines() {
        let triggers = Triggers::new(vec![TriggerRule { instant: true, ..rule(1, r"\[y/N\]") }, rule(2, "N")]).unwrap();
        let ids = |instant| triggers.matches("Proceed? [y/N]", instant, None).iter().map(|h| h.hit.rule).collect::<Vec<_>>();
        assert_eq!(ids(true), [1]);
        assert_eq!(ids(false), [2]);
        assert!(triggers.any_instant());
    }

    #[test]
    fn a_rule_with_a_program_matches_only_while_that_program_runs() {
        let triggers = Triggers::new(vec![TriggerRule { program: Some("claude.exe".into()), ..rule(0, "proceed") }]).unwrap();
        assert_eq!(triggers.matches("proceed", false, Some("claude.exe")).len(), 1);
        assert_eq!(triggers.matches("proceed", false, Some("Claude")).len(), 1);
        assert!(triggers.matches("proceed", false, Some("pwsh.exe")).is_empty());
        assert!(triggers.matches("proceed", false, None).is_empty(), "an unknown program is not the one asked for");
        assert!(program_matches("claude", r"C:\tools\CLAUDE.EXE"));
        assert!(!program_matches("claude", "claude-code.exe"));
    }

    #[test]
    fn the_program_filter_ignores_case_folders_and_exe_on_either_side() {
        assert!(program_matches("CLAUDE.EXE", "claude"));
        assert!(program_matches("claude.exe", "claude.exe"));
        assert!(program_matches(r"C:\x\claude.exe", "/usr/bin/claude"), "only the file name counts");
        assert!(!program_matches("claude.exe.exe", "claude"), "one .exe is left out, not every one");
        assert!(!program_matches("node", "claude"));
    }

    #[test]
    fn a_highlight_rule_gives_the_range_of_every_match() {
        let triggers = Triggers::new(vec![TriggerRule { highlight: Some((1, 2, 3)), ..rule(0, "ab|x*") }]).unwrap();
        let hits = triggers.matches("ab-ab", false, None);
        assert_eq!(hits[0].highlight, Some(((1, 2, 3), vec![0..2, 3..5])), "empty matches are no cells");
    }

    #[test]
    fn the_spans_of_a_match_follow_wide_characters_and_wrapped_rows() {
        // "a設b" on line 4 (設 takes two columns), wrapped onto "cd" on line 5.
        let mut line = GridLine::default();
        line.push('a', &[], 4, 0, 1);
        line.push('設', &[], 4, 1, 2);
        line.push('b', &[], 4, 3, 1);
        line.push('c', &[], 5, 0, 1);
        line.push('d', &[], 5, 1, 1);
        line.push(' ', &[], 5, 2, 1);
        line.trim_end();
        assert_eq!(line.text, "a設bcd");
        let range = |needle: &str| {
            let start = line.text.find(needle).unwrap();
            start..start + needle.len()
        };
        assert_eq!(line.spans(&range("設")), [(4, 1, 3)]);
        assert_eq!(line.spans(&range("bc")), [(4, 3, 4), (5, 0, 1)]);
        assert_eq!(line.spans(&range("a設bcd")), [(4, 0, 4), (5, 0, 2)]);
        // A combining mark stays with its base character.
        let mut marked = GridLine::default();
        marked.push('e', &['\u{301}'], 0, 0, 1);
        marked.push('x', &[], 0, 1, 1);
        assert_eq!(marked.text, "e\u{301}x");
        assert_eq!(marked.spans(&(0..3)), [(0, 0, 1)]);
        assert_eq!(marked.spans(&(3..4)), [(0, 1, 2)]);
    }
}
