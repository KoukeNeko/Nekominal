//! Removes personal data from text that leaves `target/agent-probe/`: failure messages, notes and docs.
//!
//! Claude Code's screens show account details (e-mail, organisation, plan). Raw dumps stay in the ignored
//! `target` directory; everything else goes through [`scrub`] first.

use std::sync::OnceLock;

use regex::{Captures, Regex};

fn email() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}").unwrap())
}

fn uuid() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap())
}

fn long_hex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{24,}\b").unwrap())
}

fn token_candidate() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/_=\-]{32,}").unwrap())
}

/// A line that names an account detail. Claude Code prints these on its start screen and in `/status`.
fn account_label() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(organi[sz]ation|account|login method|e-?mail|welcome back|logged in|signed in|subscription|\bplan\b|\bMax\b|\bPro\b|\bTeam\b|\bEnterprise\b|api key|auth token|\blimit\b|\bresets\b|\busage\b|\bcredits?\b|\bquota\b)",
        )
        .unwrap()
    })
}

/// Replaces e-mail addresses, UUIDs, long hex strings, long base64/URL-safe tokens and each of `names` (a user
/// or machine name, matched case-insensitively as a whole word) with placeholders.
pub fn scrub_with(text: &str, names: &[&str]) -> String {
    let mut out = email().replace_all(text, "<email>").into_owned();
    out = uuid().replace_all(&out, "<uuid>").into_owned();
    out = long_hex().replace_all(&out, "<hex>").into_owned();
    // A token mixes lower-case letters and digits; a run of dashes, a long word or an upper-case variable name
    // such as CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 is not one.
    out = token_candidate()
        .replace_all(&out, |caps: &Captures| {
            let candidate = &caps[0];
            let lower = candidate.chars().any(|c| c.is_ascii_lowercase());
            let digits = candidate.chars().any(|c| c.is_ascii_digit());
            if lower && digits { "<token>".to_owned() } else { candidate.to_owned() }
        })
        .into_owned();
    for name in names.iter().filter(|name| name.len() >= 3) {
        let pattern = format!(r"(?i)\b{}\b", regex::escape(name));
        if let Ok(re) = Regex::new(&pattern) {
            out = re.replace_all(&out, "<user>").into_owned();
        }
    }
    out
}

/// [`scrub_with`] for this machine: the current user name and computer name.
pub fn scrub(text: &str) -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    let names: Vec<&str> = [user.as_str(), host.as_str()].into_iter().filter(|name| !name.is_empty()).collect();
    scrub_with(text, &names)
}

/// [`scrub_with`] plus masking of whole lines that look like account details, for screen dumps that are
/// printed or quoted. The unmasked screen is only ever written under `target/agent-probe/`.
pub fn scrub_screen_with(screen: &str, names: &[&str]) -> String {
    screen
        .lines()
        .map(|line| {
            if account_label().is_match(line) {
                // Keep the layout (leading box-drawing and spaces), drop the words.
                let kept: String = line.chars().take_while(|c| !c.is_alphanumeric()).collect();
                format!("{kept}<account line masked>")
            } else {
                scrub_with(line, names)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn scrub_screen(screen: &str) -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    let names: Vec<&str> = [user.as_str(), host.as_str()].into_iter().filter(|name| !name.is_empty()).collect();
    scrub_screen_with(screen, &names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_uuids_hex_and_tokens_are_replaced() {
        // Assembled at runtime so no token-shaped literal exists in the source.
        let token = ["sk", "ant", "api03", "AbCdEf0123456789AbCdEf0123456789XyZ"].join("-");
        let text = format!(
            "mail jane.doe+tag@example.co.uk id 123e4567-e89b-12d3-a456-426614174000 \
             sha 0123456789abcdef0123456789abcdef01 tok {token} end"
        );
        let scrubbed = scrub_with(&text, &[]);
        assert_eq!(scrubbed, "mail <email> id <uuid> sha <hex> tok <token> end");
    }

    #[test]
    fn environment_variable_names_with_digits_are_not_tokens() {
        let text = "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 and ANTHROPIC_BASE_URL_OVERRIDE_FOR_TESTING_2";
        assert_eq!(scrub_with(text, &[]), text);
    }

    #[test]
    fn the_user_name_is_replaced_in_paths_and_prose() {
        let scrubbed = scrub_with("C:\\Users\\Jdoe\\Desktop and jdoe again, not jdoes", &["jdoe"]);
        assert_eq!(scrubbed, "C:\\Users\\<user>\\Desktop and <user> again, not jdoes");
    }

    #[test]
    fn rules_of_dashes_and_plain_words_survive() {
        let rule = "─".repeat(40) + &"-".repeat(40) + " internationalization-and-localization-considerations";
        assert_eq!(scrub_with(&rule, &[]), rule);
        assert_eq!(scrub_with("short names such as ab and x", &["ab"]), "short names such as ab and x");
    }

    #[test]
    fn account_lines_of_a_screen_are_masked_wholesale() {
        let screen =
            "╭──────╮\n│ Welcome back Jane! │\n│ Organization: Acme Inc │\n│ /help for help │\n│ Opus plan │\n╰──────╯";
        let scrubbed = scrub_screen_with(screen, &[]);
        assert!(!scrubbed.contains("Jane") && !scrubbed.contains("Acme") && !scrubbed.contains("Opus"), "{scrubbed}");
        assert!(scrubbed.contains("/help for help"));
        assert_eq!(scrubbed.lines().count(), 6, "the layout keeps its line count");
    }

    #[test]
    fn usage_and_limit_lines_are_masked() {
        let screen = "  You've used 85% of your weekly limit · resets Oct 18\n  Opus 4 · Claude Max\n  ctrl+g to edit";
        let scrubbed = scrub_screen_with(screen, &[]);
        assert!(
            !scrubbed.contains("85%") && !scrubbed.contains("Opus") && scrubbed.contains("ctrl+g to edit"),
            "{scrubbed}"
        );
    }

    #[test]
    fn scrub_screen_still_removes_mail_and_names_on_other_lines() {
        let screen = "build by jane@example.com in C:\\Users\\jdoe\\src";
        assert_eq!(scrub_screen_with(screen, &["jdoe"]), "build by <email> in C:\\Users\\<user>\\src");
    }
}
