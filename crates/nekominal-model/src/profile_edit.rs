//! Editing the profiles on the settings page. A change is made to the entries of `settings.json` (`Settings::profiles`):
//! a profile found on the machine gets an entry with its id the first time it is changed, which `profiles::merge` lays
//! over it.

use std::collections::BTreeMap;

use crate::{
    profiles::Profile,
    settings::{CustomProfile, FONT_SIZE_RANGE, Settings},
    strings::Text,
    themes,
};

/// What the editor of a profile shows, a row each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Program,
    Args,
    Cwd,
    Env,
    Scheme,
    FontSize,
    Elevate,
}

impl Field {
    pub const ALL: [Field; 8] = [Field::Name, Field::Program, Field::Args, Field::Cwd, Field::Env, Field::Scheme, Field::FontSize, Field::Elevate];

    /// What the text field of the row is known by while it is edited.
    pub fn id(self) -> &'static str {
        match self {
            Field::Name => "profile.name",
            Field::Program => "profile.program",
            Field::Args => "profile.args",
            Field::Cwd => "profile.cwd",
            Field::Env => "profile.env",
            Field::Scheme => "profile.colorScheme",
            Field::FontSize => "profile.fontSize",
            Field::Elevate => "profile.elevate",
        }
    }

    pub fn from_id(id: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|field| field.id() == id)
    }

    pub fn label(self) -> Text {
        match self {
            Field::Name => Text::ProfileName,
            Field::Program => Text::ProfileProgram,
            Field::Args => Text::ProfileArgs,
            Field::Cwd => Text::ProfileCwd,
            Field::Env => Text::ProfileEnv,
            Field::Scheme => Text::SettingsTheme,
            Field::FontSize => Text::SettingsFontSize,
            Field::Elevate => Text::ProfileElevate,
        }
    }

    /// What an empty field means, said in it: the window's scheme and font size, or nothing.
    pub fn placeholder(self) -> Option<Text> {
        match self {
            Field::Scheme | Field::FontSize => Some(Text::ProfileSameAsWindow),
            _ => None,
        }
    }
}

/// Words joined by spaces, a word with a space or a quote in it (or an empty one) in double quotes.
pub fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| if word.is_empty() || word.contains([' ', '"']) { format!("\"{}\"", word.replace('"', "")) } else { word.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words of a line split at spaces; double quotes keep spaces in a word.
pub fn split_words(line: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted, mut started) = (Vec::new(), String::new(), false, false);
    for c in line.chars() {
        match c {
            '"' => (quoted, started) = (!quoted, true),
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn env_text(env: &BTreeMap<String, String>) -> String {
    join_words(&env.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>())
}

/// `KEY=value` words; `None` when a word has no name before its `=`.
fn parse_env(line: &str) -> Option<BTreeMap<String, String>> {
    split_words(line)
        .into_iter()
        .map(|word| {
            let (key, value) = word.split_once('=')?;
            (!key.trim().is_empty()).then(|| (key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

/// What the field of a profile reads.
pub fn text(profile: &Profile, field: Field) -> String {
    match field {
        Field::Name => profile.name.clone(),
        Field::Program => profile.program.clone(),
        Field::Args => join_words(&profile.args),
        Field::Cwd => profile.cwd.clone().unwrap_or_default(),
        Field::Env => env_text(&profile.options.env),
        Field::Scheme => profile.options.color_scheme.clone().unwrap_or_default(),
        Field::FontSize => profile.options.font_size.map(|size| size.to_string()).unwrap_or_default(),
        Field::Elevate => String::new(),
    }
}

/// The entry of `profile` in the settings, made from what the profile is when it has none.
fn entry<'a>(settings: &'a mut Settings, profile: &Profile) -> &'a mut CustomProfile {
    let at = match settings.profiles.iter().position(|entry| entry.id == profile.id) {
        Some(at) => at,
        None => {
            // A profile found on the machine is adjusted by an entry without a program.
            settings.profiles.push(CustomProfile {
                id: profile.id.clone(),
                name: profile.name.clone(),
                program: String::new(),
                args: Vec::new(),
                cwd: profile.cwd.clone(),
                options: profile.options.clone(),
            });
            settings.profiles.len() - 1
        }
    };
    &mut settings.profiles[at]
}

/// Gives the field of `profile` what was typed. False, with nothing changed, when the profile cannot take it: an empty
/// name or program, an environment word without a name, a scheme there is none of, a font size out of range.
pub fn set(settings: &mut Settings, profile: &Profile, field: Field, typed: &str) -> bool {
    let typed = typed.trim();
    let env = if field == Field::Env { match parse_env(typed) { Some(env) => Some(env), None => return false } } else { None };
    let size = match (field, typed) {
        (Field::FontSize, "") => None,
        (Field::FontSize, typed) => match typed.parse::<f64>() {
            Ok(size) if (FONT_SIZE_RANGE.0..=FONT_SIZE_RANGE.1).contains(&size) => Some(size),
            _ => return false,
        },
        _ => None,
    };
    match field {
        Field::Name | Field::Program if typed.is_empty() => return false,
        Field::Scheme if !typed.is_empty() && !themes::is_known(typed) => return false,
        _ => {}
    }
    let entry = entry(settings, profile);
    // An entry without a program cannot change the command of the shell it adjusts: it becomes the whole command.
    if matches!(field, Field::Program | Field::Args) && entry.program.is_empty() {
        entry.program = profile.program.clone();
        entry.args = profile.args.clone();
    }
    match field {
        Field::Name => entry.name = typed.to_owned(),
        Field::Program => entry.program = typed.to_owned(),
        Field::Args => entry.args = split_words(typed),
        Field::Cwd => entry.cwd = (!typed.is_empty()).then(|| typed.to_owned()),
        Field::Env => entry.options.env = env.unwrap_or_default(),
        Field::Scheme => entry.options.color_scheme = (!typed.is_empty()).then(|| typed.to_owned()),
        Field::FontSize => entry.options.font_size = size,
        Field::Elevate => entry.options.elevate = !entry.options.elevate,
    }
    true
}

/// Turns running as administrator on or off for `profile`.
pub fn toggle_elevate(settings: &mut Settings, profile: &Profile) {
    set(settings, profile, Field::Elevate, "");
}

/// Adds a profile that runs what `base` runs, called `name`; its id.
pub fn add(settings: &mut Settings, base: &Profile, taken: &[Profile], name: &str) -> String {
    let id = (1..)
        .map(|n| format!("profile-{n}"))
        .find(|id| !taken.iter().any(|p| p.id == *id) && !settings.profiles.iter().any(|p| p.id == *id))
        .unwrap_or_default();
    settings.profiles.push(CustomProfile {
        id: id.clone(),
        name: name.to_owned(),
        program: base.program.clone(),
        args: base.args.clone(),
        cwd: None,
        options: Default::default(),
    });
    id
}

/// Takes the entry of profile `id` out of the settings: a profile written there is gone, one found on the machine is as
/// it was found. The default profile naming a profile that is gone falls back to the first one (`resolve_default`).
pub fn remove(settings: &mut Settings, id: &str) {
    settings.profiles.retain(|entry| entry.id != id);
}

/// Whether the settings change profile `id`: it then has something to delete or to put back.
pub fn has_entry(settings: &Settings, id: &str) -> bool {
    settings.profiles.iter().any(|entry| entry.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{DiscoveredProfile, merge};

    fn found() -> Vec<DiscoveredProfile> {
        vec![DiscoveredProfile { id: "pwsh".into(), name: "PowerShell".into(), kind: "powershell", icon: "pwsh", program: "pwsh.exe".into(), args: vec!["-NoLogo".into()] }]
    }

    fn profiles(settings: &Settings) -> Vec<Profile> {
        merge(&found(), &settings.profiles)
    }

    #[test]
    fn words_split_at_spaces_and_quotes_keep_them_and_join_back() {
        assert_eq!(split_words(r#"-c  "echo hi" """#), ["-c", "echo hi", ""]);
        let words = split_words(r#"-NoLogo "C:\Program Files\x" """#);
        assert_eq!(split_words(&join_words(&words)), words);
    }

    #[test]
    fn changing_a_found_profile_adjusts_it_and_the_command_then_becomes_its_own() {
        let mut settings = Settings::default();
        let pwsh = profiles(&settings)[0].clone();
        assert!(set(&mut settings, &pwsh, Field::Cwd, "D:\\work"));
        assert_eq!(settings.profiles[0].program, "", "only the folder is adjusted");
        let merged = profiles(&settings);
        assert_eq!((merged.len(), merged[0].program.as_str(), merged[0].cwd.as_deref()), (1, "pwsh.exe", Some("D:\\work")));
        assert!(set(&mut settings, &merged[0], Field::Args, "-NoProfile"));
        assert_eq!((settings.profiles[0].program.as_str(), &settings.profiles[0].args[..]), ("pwsh.exe", &["-NoProfile".to_owned()][..]));
        assert_eq!(profiles(&settings)[0].kind, crate::profiles::ProfileKind::PowerShell, "still the shell found");
        remove(&mut settings, "pwsh");
        assert_eq!(profiles(&settings)[0].args, ["-NoLogo"], "put back as found");
    }

    #[test]
    fn options_are_checked_before_they_are_kept() {
        let mut settings = Settings::default();
        let pwsh = profiles(&settings)[0].clone();
        for (field, bad) in [(Field::Name, " "), (Field::Program, ""), (Field::Env, "=x"), (Field::Scheme, "No Such Scheme"), (Field::FontSize, "500"), (Field::FontSize, "big")] {
            assert!(!set(&mut settings, &pwsh, field, bad), "{field:?} {bad:?}");
        }
        assert!(settings.profiles.is_empty(), "nothing refused leaves an entry");
        assert!(set(&mut settings, &pwsh, Field::Env, r#"LANG=zh_TW "PATH=C:\a b;C:\c""#));
        assert!(set(&mut settings, &pwsh, Field::Scheme, "Nord"));
        assert!(set(&mut settings, &pwsh, Field::FontSize, "14"));
        toggle_elevate(&mut settings, &pwsh);
        let merged = &profiles(&settings)[0];
        assert_eq!(merged.options.env["PATH"], "C:\\a b;C:\\c");
        assert_eq!(text(merged, Field::Env), r#"LANG=zh_TW "PATH=C:\a b;C:\c""#);
        assert_eq!((merged.options.color_scheme.as_deref(), merged.options.font_size, merged.options.elevate), (Some("Nord"), Some(14.0), true));
        assert!(set(&mut settings, merged, Field::FontSize, ""));
        assert_eq!(profiles(&settings)[0].options.font_size, None, "an empty field follows the window");
        let written = crate::settings::parse(Some(&crate::settings::serialize(&settings))).settings;
        assert_eq!(written.profiles, settings.profiles, "the file reads back what the page wrote");
    }

    #[test]
    fn an_added_profile_runs_what_its_base_runs_and_can_be_deleted() {
        let mut settings = Settings::default();
        let base = profiles(&settings)[0].clone();
        let taken = profiles(&settings);
        let id = add(&mut settings, &base, &taken, "New profile");
        let taken = profiles(&settings);
        let again = add(&mut settings, &base, &taken, "New profile");
        assert_ne!(id, again);
        let merged = profiles(&settings);
        assert_eq!(merged.len(), 3);
        assert!(merged[1].custom && merged[1].program == "pwsh.exe" && merged[1].name == "New profile");
        remove(&mut settings, &id);
        assert!(!has_entry(&settings, &id) && profiles(&settings).len() == 2);
    }
}
