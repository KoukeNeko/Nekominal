//! Editing the profiles on the settings page. A change is made to the entries of `settings.json` (`Settings::profiles`):
//! a profile found on the machine gets an entry with its id the first time it is changed, which `profiles::merge` lays
//! over it.

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
    Scheme,
    FontSize,
    Elevate,
    Env,
}

/// What the text fields of a variable of the environment are known by while they are edited.
pub const ENV_KEY_ID: &str = "profile.env.key";
pub const ENV_VALUE_ID: &str = "profile.env.value";

impl Field {
    pub const ALL: [Field; 8] = [Field::Name, Field::Program, Field::Args, Field::Cwd, Field::Scheme, Field::FontSize, Field::Elevate, Field::Env];

    /// What the text field of the row is known by while it is edited.
    pub fn id(self) -> &'static str {
        match self {
            Field::Name => "profile.name",
            Field::Program => "profile.program",
            Field::Args => "profile.args",
            Field::Cwd => "profile.cwd",
            Field::Scheme => "profile.colorScheme",
            Field::FontSize => "profile.fontSize",
            Field::Elevate => "profile.elevate",
            Field::Env => "profile.env",
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
            Field::Scheme => Text::SettingsTheme,
            Field::FontSize => Text::SettingsFontSize,
            Field::Elevate => Text::ProfileElevate,
            Field::Env => Text::ProfileEnv,
        }
    }

    /// A line of text typed in a field; the other rows have controls of their own.
    pub fn is_text(self) -> bool {
        matches!(self, Field::Name | Field::Program | Field::Args | Field::Cwd)
    }

    /// A file or a folder can be chosen for it in a dialog.
    pub fn browses(self) -> bool {
        matches!(self, Field::Program | Field::Cwd)
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

/// What the field of a profile reads; for the rows without a text field, what a screen reader is told.
pub fn text(profile: &Profile, field: Field) -> String {
    match field {
        Field::Name => profile.name.clone(),
        Field::Program => profile.program.clone(),
        Field::Args => join_words(&profile.args),
        Field::Cwd => profile.cwd.clone().unwrap_or_default(),
        Field::Scheme => profile.options.color_scheme.clone().unwrap_or_default(),
        Field::FontSize => profile.options.font_size.map(|size| size.to_string()).unwrap_or_default(),
        Field::Elevate => String::new(),
        Field::Env => join_words(&profile.options.env.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>()),
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

/// Gives a text field of `profile` what was typed. The error says why the profile cannot take it (an empty name or
/// program), and nothing is changed then. The rows that are no text field have functions of their own.
pub fn set(settings: &mut Settings, profile: &Profile, field: Field, typed: &str) -> Result<(), Text> {
    let typed = typed.trim();
    if matches!(field, Field::Name | Field::Program) && typed.is_empty() {
        return Err(Text::ProfileNeedsValue);
    }
    // A field left as it was (the page sets a field whenever it loses focus) must not make an entry.
    if !field.is_text() || typed == text(profile, field) {
        return Ok(());
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
        _ => entry.cwd = (!typed.is_empty()).then(|| typed.to_owned()),
    }
    Ok(())
}

/// Gives `profile` the scheme with this name, or the window's with none. A name there is no scheme of changes nothing.
pub fn set_scheme(settings: &mut Settings, profile: &Profile, scheme: Option<&str>) {
    if scheme.is_some_and(|name| !themes::is_known(name)) {
        return;
    }
    entry(settings, profile).options.color_scheme = scheme.map(str::to_owned);
}

/// One point larger or smaller than the size the profile has, or than the window's (`window`) while it has none.
pub fn step_font_size(settings: &mut Settings, profile: &Profile, larger: bool, window: f64) {
    let now = profile.options.font_size.unwrap_or(window);
    let size = (now.round() + if larger { 1.0 } else { -1.0 }).clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1);
    entry(settings, profile).options.font_size = Some(size);
}

/// The profile's text is the window's size again.
pub fn follow_window_font_size(settings: &mut Settings, profile: &Profile) {
    entry(settings, profile).options.font_size = None;
}

/// Turns running as administrator on or off for `profile`.
pub fn toggle_elevate(settings: &mut Settings, profile: &Profile) {
    let entry = entry(settings, profile);
    entry.options.elevate = !entry.options.elevate;
}

/// The variable of the environment at `index`, in the order the editor lists them (by name).
pub fn env_at(profile: &Profile, index: usize) -> Option<(&String, &String)> {
    profile.options.env.iter().nth(index)
}

fn valid_name(name: &str) -> Result<(), Text> {
    if name.is_empty() {
        Err(Text::ProfileNeedsValue)
    } else if name.contains(|c: char| c == '=' || c.is_whitespace()) {
        Err(Text::ProfileBadVariable)
    } else {
        Ok(())
    }
}

/// Renames the variable at `index`, keeping its value.
pub fn rename_env(settings: &mut Settings, profile: &Profile, index: usize, name: &str) -> Result<(), Text> {
    let name = name.trim();
    let Some((old, value)) = env_at(profile, index).map(|(k, v)| (k.clone(), v.clone())) else { return Ok(()) };
    if name == old {
        return Ok(());
    }
    valid_name(name)?;
    if profile.options.env.contains_key(name) {
        return Err(Text::ProfileDuplicateVariable);
    }
    let env = &mut entry(settings, profile).options.env;
    env.remove(&old);
    env.insert(name.to_owned(), value);
    Ok(())
}

/// Gives the variable at `index` this value; an empty value is a variable set to nothing.
pub fn set_env_value(settings: &mut Settings, profile: &Profile, index: usize, value: &str) {
    if let Some(name) = env_at(profile, index).map(|(k, _)| k.clone()) {
        entry(settings, profile).options.env.insert(name, value.to_owned());
    }
}

pub fn remove_env(settings: &mut Settings, profile: &Profile, index: usize) {
    if let Some(name) = env_at(profile, index).map(|(k, _)| k.clone()) {
        entry(settings, profile).options.env.remove(&name);
    }
}

/// Adds a variable with a name no other has (`NAME`, `NAME2`, …) and no value; its name, for the editor to rename.
pub fn add_env(settings: &mut Settings, profile: &Profile) -> String {
    let name = (1..)
        .map(|n| if n == 1 { "NAME".to_owned() } else { format!("NAME{n}") })
        .find(|name| !profile.options.env.contains_key(name))
        .unwrap_or_default();
    entry(settings, profile).options.env.insert(name.clone(), String::new());
    name
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

    /// Runs a change on the PowerShell profile as the settings now make it.
    fn with<R>(settings: &mut Settings, change: impl FnOnce(&mut Settings, &Profile) -> R) -> R {
        let profile = pwsh(settings);
        change(settings, &profile)
    }

    fn pwsh(settings: &Settings) -> Profile {
        profiles(settings)[0].clone()
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
        assert_eq!(with(&mut settings, |s, p| set(s, p, Field::Cwd, "D:\\work")), Ok(()));
        assert_eq!(settings.profiles[0].program, "", "only the folder is adjusted");
        let merged = profiles(&settings);
        assert_eq!((merged.len(), merged[0].program.as_str(), merged[0].cwd.as_deref()), (1, "pwsh.exe", Some("D:\\work")));
        assert_eq!(set(&mut settings, &merged[0], Field::Args, "-NoProfile"), Ok(()));
        assert_eq!((settings.profiles[0].program.as_str(), &settings.profiles[0].args[..]), ("pwsh.exe", &["-NoProfile".to_owned()][..]));
        assert_eq!(pwsh(&settings).kind, crate::profiles::ProfileKind::PowerShell, "still the shell found");
        remove(&mut settings, "pwsh");
        assert_eq!(pwsh(&settings).args, ["-NoLogo"], "put back as found");
    }

    #[test]
    fn what_a_profile_cannot_take_is_refused_with_the_reason_and_changes_nothing() {
        let mut settings = Settings::default();
        assert_eq!(with(&mut settings, |s, p| set(s, p, Field::Name, " ")), Err(Text::ProfileNeedsValue));
        assert_eq!(with(&mut settings, |s, p| set(s, p, Field::Program, "")), Err(Text::ProfileNeedsValue));
        with(&mut settings, |s, p| set_scheme(s, p, Some("No Such Scheme")));
        assert!(settings.profiles.is_empty(), "nothing refused leaves an entry");
    }

    #[test]
    fn leaving_a_field_as_it_was_makes_no_entry() {
        let mut settings = Settings::default();
        for field in [Field::Name, Field::Program, Field::Args, Field::Cwd] {
            let p = pwsh(&settings);
            assert_eq!(set(&mut settings, &p, field, &format!(" {} ", text(&p, field))), Ok(()));
        }
        assert!(settings.profiles.is_empty());
    }

    #[test]
    fn the_scheme_the_font_size_and_elevation_are_chosen_and_followed_back_to_the_window() {
        let mut settings = Settings::default();
        with(&mut settings, |s, p| set_scheme(s, p, Some("Nord")));
        with(&mut settings, |s, p| step_font_size(s, p, true, 12.0));
        assert_eq!(pwsh(&settings).options.font_size, Some(13.0), "a step from the window's size");
        with(&mut settings, |s, p| step_font_size(s, p, false, 12.0));
        with(&mut settings, |s, p| step_font_size(s, p, false, 12.0));
        with(&mut settings, |s, p| toggle_elevate(s, p));
        let p = pwsh(&settings);
        assert_eq!((p.options.color_scheme.as_deref(), p.options.font_size, p.options.elevate), (Some("Nord"), Some(11.0), true));
        set_scheme(&mut settings, &p, None);
        with(&mut settings, |s, p| follow_window_font_size(s, p));
        let p = pwsh(&settings);
        assert_eq!((p.options.color_scheme.as_deref(), p.options.font_size), (None, None));
        for _ in 0..200 {
            with(&mut settings, |s, p| step_font_size(s, p, true, 12.0));
        }
        assert_eq!(pwsh(&settings).options.font_size, Some(FONT_SIZE_RANGE.1), "kept in range");
    }

    #[test]
    fn variables_are_added_renamed_given_values_and_removed_one_by_one() {
        let mut settings = Settings::default();
        assert_eq!(with(&mut settings, |s, p| add_env(s, p)), "NAME");
        assert_eq!(with(&mut settings, |s, p| add_env(s, p)), "NAME2");
        assert_eq!(with(&mut settings, |s, p| rename_env(s, p, 0, "LANG")), Ok(()));
        // By name: LANG, NAME2.
        with(&mut settings, |s, p| set_env_value(s, p, 0, "zh_TW"));
        assert_eq!(with(&mut settings, |s, p| rename_env(s, p, 1, "LANG")), Err(Text::ProfileDuplicateVariable));
        assert_eq!(with(&mut settings, |s, p| rename_env(s, p, 1, "A B")), Err(Text::ProfileBadVariable));
        assert_eq!(with(&mut settings, |s, p| rename_env(s, p, 1, "")), Err(Text::ProfileNeedsValue));
        assert_eq!(with(&mut settings, |s, p| rename_env(s, p, 1, "PATH")), Ok(()));
        with(&mut settings, |s, p| set_env_value(s, p, 1, "C:\\a b;C:\\c"));
        assert_eq!(text(&pwsh(&settings), Field::Env), r#"LANG=zh_TW "PATH=C:\a b;C:\c""#);
        with(&mut settings, |s, p| remove_env(s, p, 0));
        assert_eq!(pwsh(&settings).options.env.keys().collect::<Vec<_>>(), ["PATH"]);
        let written = crate::settings::parse(Some(&crate::settings::serialize(&settings))).settings;
        assert_eq!(written.profiles, settings.profiles, "the file reads back what the page wrote");
    }

    #[test]
    fn an_added_profile_runs_what_its_base_runs_and_can_be_deleted() {
        let mut settings = Settings::default();
        let base = pwsh(&settings);
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
