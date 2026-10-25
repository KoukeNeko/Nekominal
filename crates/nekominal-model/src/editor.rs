//! The editor a `file:line:column` opens in: the setting `editor` is `auto` or a command line with `{file}`, `{line}`
//! and `{column}` in it, as `code -g "{file}:{line}:{column}"`. Pure: finding the programs and starting one is the
//! shell's.
//!
//! The command line is split into the program and its arguments here, once, and the file is put into the arguments it
//! names afterwards: what the file is called never changes how the line splits, and no shell ever reads it.

/// The value of the setting that looks for one of [`KNOWN`].
pub const AUTO: &str = "auto";

/// The editors `auto` looks for, the first found wins: the name of the program, and the command it is run with.
pub const KNOWN: &[(&str, &str)] = &[
    ("code", r#"code -g "{file}:{line}:{column}""#),
    ("cursor", r#"cursor -g "{file}:{line}:{column}""#),
    ("windsurf", r#"windsurf -g "{file}:{line}:{column}""#),
    ("zed", r#"zed "{file}:{line}:{column}""#),
];

/// The command of the first of [`KNOWN`] among the names of the programs that were `found`.
pub fn pick(found: &[&str]) -> Option<&'static str> {
    KNOWN.iter().find(|(name, _)| found.contains(name)).map(|(_, template)| *template)
}

/// The words of a command line: split at blanks, a pair of double quotes holding blanks in one word (and nothing else
/// special, so `C:\Program Files\x` keeps its backslashes). `None` when a quote is not closed or there is no word.
pub fn words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                word.get_or_insert_with(String::new);
            }
            c if c.is_whitespace() && !quoted => words.extend(word.take()),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    (!quoted && !words.is_empty()).then_some(words)
}

/// The program and the arguments that open `file` at `line` and `column` with `template`. `None` for a command line that
/// does not split, that does not name `{file}` in an argument, or whose program is a placeholder: the file must never be
/// what runs.
pub fn command(template: &str, file: &str, line: u32, column: u32) -> Option<(String, Vec<String>)> {
    let mut words = words(template)?;
    let program = words.remove(0);
    if program.contains('{') || !words.iter().any(|word| word.contains("{file}")) {
        return None;
    }
    let (line, column) = (line.to_string(), column.to_string());
    let args = words.iter().map(|word| word.replace("{line}", &line).replace("{column}", &column).replace("{file}", file)).collect();
    Some((program, args))
}

/// Whether the setting can be used: `auto`, empty (the default program of the file), or a command [`command`] takes.
pub fn is_valid(setting: &str) -> bool {
    let setting = setting.trim();
    setting.is_empty() || setting == AUTO || command(setting, "f", 1, 1).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_line_splits_at_blanks_and_quotes_hold_a_word_together() {
        assert_eq!(words(r#"code -g "{file}:{line}""#).unwrap(), ["code", "-g", "{file}:{line}"]);
        assert_eq!(words(r#""C:\Program Files\Zed\zed.exe"  {file}"#).unwrap(), [r"C:\Program Files\Zed\zed.exe", "{file}"]);
        assert_eq!(words(r#"a "" b"#).unwrap(), ["a", "", "b"], "empty quotes are an empty argument");
        assert_eq!(words(r#"code "{file}"#), None, "a quote that is not closed");
        assert_eq!(words("   "), None);
    }

    #[test]
    fn the_file_goes_into_its_argument_whole_whatever_it_is_called() {
        let (program, args) = command(r#"code -g "{file}:{line}:{column}""#, r"C:\My Docs\a b.rs", 12, 5).unwrap();
        assert_eq!((program.as_str(), args), ("code", vec!["-g".to_owned(), r"C:\My Docs\a b.rs:12:5".to_owned()]));
        // A name that looks like a placeholder or a quote is not read again.
        let (_, args) = command("vim +{line} {file}", r#"C:\x\{line} "q".txt"#, 3, 1).unwrap();
        assert_eq!(args, ["+3", r#"C:\x\{line} "q".txt"#]);
    }

    #[test]
    fn a_command_must_name_the_file_in_an_argument_and_never_run_it() {
        assert_eq!(command("code -g", "f", 1, 1), None);
        assert_eq!(command("{file}", "f", 1, 1), None);
        assert_eq!(command("{file} {file}", "f", 1, 1), None);
        assert!(is_valid("auto") && is_valid("") && is_valid("notepad {file}"));
        assert!(!is_valid("notepad") && !is_valid(r#"code "{file}"#));
    }

    #[test]
    fn auto_takes_vs_code_first_then_cursor_windsurf_and_zed() {
        assert_eq!(pick(&["zed", "cursor", "code"]), Some(KNOWN[0].1));
        assert_eq!(pick(&["zed", "cursor"]), Some(KNOWN[1].1));
        assert_eq!(pick(&["zed", "windsurf"]), Some(KNOWN[2].1));
        assert_eq!(pick(&["zed"]), Some(KNOWN[3].1));
        assert_eq!(pick(&["notepad"]), None);
        assert_eq!(pick(&[]), None);
        for (name, template) in KNOWN {
            assert_eq!(command(template, "f", 1, 1).map(|(program, _)| program).as_deref(), Some(*name));
        }
    }
}
