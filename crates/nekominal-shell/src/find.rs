//! The find bar's pure parts: what the user types is plain text, and the search wants a regular expression.

/// The regular expression that matches `text` literally (the search is case-insensitive unless the text holds an
/// uppercase letter, which is the search's own rule).
pub fn literal_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if r"\.+*?()|[]{}^$#&-~".contains(ch) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

/// How the bar searches: whether the case of the letters counts (Alt+C), and whether what is typed is a regular expression
/// (Alt+R) rather than text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub match_case: bool,
    pub regex: bool,
}

/// The pattern the search takes for `query`: the text as it is, or escaped to be matched literally, with the case of its
/// letters counting or not. The flag is said in the pattern, so that the search's own rule (ignore the case unless there
/// is a capital) does not decide.
pub fn pattern(query: &str, options: Options) -> String {
    let body = if options.regex { query.to_owned() } else { literal_pattern(query) };
    format!("{}{body}", if options.match_case { "(?-i)" } else { "(?i)" })
}

/// What the bar says of the search: nothing before anything is typed, the count when there are matches.
pub fn status(query: &str, current: usize, total: usize) -> String {
    if query.is_empty() {
        String::new()
    } else if total == 0 {
        "0".to_owned()
    } else {
        format!("{} / {total}", current + 1)
    }
}

/// `status` with the options that are on before it: "Aa" for the case, ".*" for a regular expression.
pub fn status_with(options: Options, count: &str) -> String {
    let flags: Vec<&str> = [(options.match_case, "Aa"), (options.regex, ".*")].into_iter().filter(|(on, _)| *on).map(|(_, flag)| flag).collect();
    match (flags.is_empty(), count.is_empty()) {
        (true, _) => count.to_owned(),
        (false, true) => flags.join(" "),
        (false, false) => format!("{}  {count}", flags.join(" ")),
    }
}

/// The match after (`forward`) or before `current` among `total`, wrapping at the ends.
pub fn step(current: usize, total: usize, forward: bool) -> usize {
    if total == 0 {
        0
    } else if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_character_with_a_meaning_in_a_pattern_is_escaped() {
        assert_eq!(literal_pattern("a.b*c"), r"a\.b\*c");
        assert_eq!(literal_pattern("f(x) [1]"), r"f\(x\) \[1\]");
        assert_eq!(literal_pattern("設定 ABC"), "設定 ABC");
        assert_eq!(literal_pattern(r"C:\dir"), r"C:\\dir");
    }

    #[test]
    fn the_options_decide_the_case_and_whether_the_query_is_escaped() {
        let plain = Options::default();
        assert_eq!(pattern("a.b", plain), r"(?i)a\.b", "text, any case");
        assert_eq!(pattern("Ab", Options { match_case: true, ..plain }), "(?-i)Ab", "text, its case");
        assert_eq!(pattern(r"err(or)?\d+", Options { regex: true, ..plain }), r"(?i)err(or)?\d+", "a regular expression as typed");
        assert_eq!(status_with(plain, "3 / 12"), "3 / 12");
        assert_eq!(status_with(Options { match_case: true, regex: true }, "3 / 12"), "Aa .*  3 / 12");
        assert_eq!(status_with(Options { regex: true, ..plain }, ""), ".*", "the options show before anything is typed");
    }

    #[test]
    fn the_status_is_empty_until_there_is_a_query_and_counts_from_one() {
        assert_eq!(status("", 0, 0), "");
        assert_eq!(status("x", 0, 0), "0");
        assert_eq!(status("x", 2, 12), "3 / 12");
    }

    #[test]
    fn stepping_wraps_at_both_ends_and_survives_no_matches() {
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(1, 3, true), 2);
        assert_eq!(step(0, 0, true), 0);
    }
}
