//! The profiles a new tab can open: the shells found on the machine plus the ones written in `settings.json`.

use crate::{settings::{CustomProfile, ProfileOptions}, shell_integration};

/// Icon of a profile whose program is not one of the known shells.
pub const GENERIC_ICON: &str = "terminal";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKind {
    PowerShell,
    Cmd,
    Bash,
    Wsl,
    Ssh,
    /// Another interactive shell a custom profile starts (`zsh`, `fish`, `nu`).
    Shell,
    /// A program that is not a known shell, or a kind this version does not know.
    Other,
}

impl ProfileKind {
    /// `name` is the `kind` string the discovery step reports (`powershell`, `cmd`, `bash`, `wsl`, `ssh`).
    pub fn from_name(name: &str) -> ProfileKind {
        match name {
            "powershell" => ProfileKind::PowerShell,
            "cmd" => ProfileKind::Cmd,
            "bash" => ProfileKind::Bash,
            "wsl" => ProfileKind::Wsl,
            "ssh" => ProfileKind::Ssh,
            _ => ProfileKind::Other,
        }
    }

    /// An interactive shell: its pane closes when the shell exits cleanly (`exit`). A profile of any other program,
    /// a coding agent for one, keeps its last screen when the program ends, because that output is what the
    /// session was for.
    pub fn closes_pane_on_clean_exit(self) -> bool {
        !matches!(self, ProfileKind::Other)
    }
}

/// A shell found by `nekominal_pty::profiles::discover`, as plain data so this crate does not depend on
/// the PTY crate: copy the fields of `nekominal_pty::profiles::Profile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredProfile {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub icon: &'static str,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Unique and stable across runs; what `defaultProfile` and a saved session refer to.
    pub id: String,
    pub name: String,
    pub kind: ProfileKind,
    /// Key the shell maps to a glyph: one of the discovery step's icon keys or [`GENERIC_ICON`].
    pub icon: &'static str,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Written in `settings.json` rather than found on the machine (an entry replacing a found shell is not custom).
    pub custom: bool,
    /// What `settings.json` sets for this profile alone: its environment, scheme, font size and elevation.
    pub options: ProfileOptions,
}

fn from_discovered(found: &DiscoveredProfile) -> Profile {
    Profile {
        id: found.id.clone(),
        name: found.name.clone(),
        kind: ProfileKind::from_name(found.kind),
        icon: found.icon,
        program: found.program.clone(),
        args: found.args.clone(),
        cwd: None,
        custom: false,
        options: ProfileOptions::default(),
    }
}

fn from_custom(entry: &CustomProfile) -> Profile {
    Profile {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: kind_for_program(&entry.program),
        icon: icon_for_program(&entry.program),
        program: entry.program.clone(),
        args: entry.args.clone(),
        cwd: entry.cwd.clone(),
        custom: true,
        options: entry.options.clone(),
    }
}

/// The file name of a program in lower case, without `.exe` (`C:\Program Files\PowerShell\7\pwsh.exe` is `pwsh`).
fn program_name(program: &str) -> String {
    let file = program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    file.strip_suffix(".exe").unwrap_or(&file).to_owned()
}

/// The icon of a known shell, from the file name of the program (`C:\Program Files\PowerShell\7\pwsh.exe`).
pub fn icon_for_program(program: &str) -> &'static str {
    match program_name(program).as_str() {
        "pwsh" => "pwsh",
        "powershell" => "windows-powershell",
        "cmd" => "cmd",
        "wsl" => "wsl",
        "ssh" => "ssh",
        _ => GENERIC_ICON,
    }
}

/// The kind of a custom profile, from the file name of its program like its icon: the names the discovery step gives
/// its kinds to, then the other interactive shells. Any other program is [`ProfileKind::Other`].
pub fn kind_for_program(program: &str) -> ProfileKind {
    match program_name(program).as_str() {
        "pwsh" | "powershell" => ProfileKind::PowerShell,
        "cmd" => ProfileKind::Cmd,
        "bash" => ProfileKind::Bash,
        "wsl" => ProfileKind::Wsl,
        "ssh" => ProfileKind::Ssh,
        "sh" | "dash" | "ksh" | "zsh" | "fish" | "nu" => ProfileKind::Shell,
        _ => ProfileKind::Other,
    }
}

/// The discovered profiles in their own order, then the custom ones in the order of `settings.json`.
///
/// A custom profile with the id of a discovered one replaces it in place: that is how a built-in is given
/// other arguments. Of several custom profiles with one id only the first counts.
pub fn merge(discovered: &[DiscoveredProfile], custom: &[CustomProfile]) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = discovered.iter().map(from_discovered).collect();
    let mut taken: Vec<&str> = Vec::new();
    for entry in custom {
        if taken.contains(&entry.id.as_str()) {
            continue;
        }
        taken.push(&entry.id);
        // An entry without a program adjusts the profile found on the machine that it names: its name and its folder.
        // One that names none has nothing to run and is left out.
        if entry.program.is_empty() {
            if let Some(found) = profiles.iter_mut().find(|existing| !existing.custom && existing.id == entry.id) {
                if !entry.name.is_empty() {
                    found.name = entry.name.clone();
                }
                found.cwd = entry.cwd.clone().or(found.cwd.take());
                found.options = entry.options.clone();
            }
            continue;
        }
        let mut profile = from_custom(entry);
        match profiles
            .iter()
            .position(|existing| !existing.custom && existing.id == entry.id)
        {
            Some(index) => {
                // A built-in given other arguments is still that shell: it keeps how its sessions behave and its
                // shell integration.
                profile.kind = profiles[index].kind;
                profile.custom = false;
                profiles[index] = profile;
            }
            None => profiles.push(profile),
        }
    }
    profiles
}

pub fn find<'a>(profiles: &'a [Profile], id: &str) -> Option<&'a Profile> {
    profiles.iter().find(|profile| profile.id == id)
}

/// The profile a new tab opens: the one `defaultProfile` names, else the first (also for an empty or
/// stale name, so a removed shell never leaves the user without one).
pub fn resolve_default<'a>(profiles: &'a [Profile], default_id: &str) -> Option<&'a Profile> {
    find(profiles, default_id).or_else(|| profiles.first())
}

/// The arguments to start the profile with. A discovered PowerShell gets the prompt marks and working
/// directory reports when `shell_integration` is on; custom profiles are launched exactly as written.
pub fn launch_args(profile: &Profile, shell_integration: bool) -> Vec<String> {
    // The program decides, not only the kind: a found PowerShell given another program (bash) keeps its kind for how its
    // sessions behave, and must not get PowerShell's arguments.
    if shell_integration && profile.kind == ProfileKind::PowerShell && !profile.custom && kind_for_program(&profile.program) == ProfileKind::PowerShell {
        shell_integration::powershell_args(&profile.args)
    } else {
        profile.args.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovered(
        id: &str,
        kind: &'static str,
        icon: &'static str,
        program: &str,
    ) -> DiscoveredProfile {
        DiscoveredProfile {
            id: id.into(),
            name: id.to_uppercase(),
            kind,
            icon,
            program: program.into(),
            args: vec!["-NoLogo".into()],
        }
    }

    #[test]
    fn an_entry_without_a_program_adjusts_the_profile_found_with_its_id() {
        let found = discovered("pwsh", "PowerShell", "pwsh", "pwsh.exe");
        let adjust = CustomProfile { id: "pwsh".into(), name: "Work shell".into(), program: String::new(), args: vec![], cwd: Some("D:\\work".into()), options: ProfileOptions { elevate: true, ..Default::default() } };
        let merged = merge(&[found], &[adjust]);
        assert_eq!(merged.len(), 1, "it adds no profile");
        assert_eq!((merged[0].name.as_str(), merged[0].program.as_str(), merged[0].cwd.as_deref()), ("Work shell", "pwsh.exe", Some("D:\\work")));
        assert!(!merged[0].custom, "it is still the shell found on the machine");
        assert!(merged[0].options.elevate, "it takes the options of the entry");
        let stray = CustomProfile { id: "nope".into(), name: "nope".into(), program: String::new(), args: vec![], cwd: None, options: Default::default() };
        assert!(merge(&[], &[stray]).is_empty(), "naming no profile, it has nothing to run");
        // A found PowerShell given another program keeps its kind, but the program gets no PowerShell arguments.
        let bash = CustomProfile { id: "pwsh".into(), name: "Bash".into(), program: r"C:\Program Files\Git\bin\bash.exe".into(), args: vec!["-l".into()], cwd: None, options: Default::default() };
        let merged = merge(&[discovered("pwsh", "PowerShell", "pwsh", "pwsh.exe")], &[bash]);
        assert_eq!(launch_args(&merged[0], true), ["-l"]);
        // A name equal to the id is a rename like any other; an entry that gives none keeps the name found.
        let to_id = CustomProfile { id: "pwsh".into(), name: "pwsh".into(), program: String::new(), args: vec![], cwd: None, options: Default::default() };
        assert_eq!(merge(&[discovered("pwsh", "PowerShell", "pwsh", "pwsh.exe")], &[to_id])[0].name, "pwsh");
        let parsed = crate::settings::parse(Some(r#"{ "profiles": [ { "id": "pwsh", "cwd": "D:\\" } ] }"#)).settings;
        let as_found = merge(&[discovered("pwsh", "PowerShell", "pwsh", "pwsh.exe")], &[])[0].name.clone();
        assert_eq!(merge(&[discovered("pwsh", "PowerShell", "pwsh", "pwsh.exe")], &parsed.profiles)[0].name, as_found);
    }

    fn custom(id: &str, program: &str) -> CustomProfile {
        CustomProfile {
            id: id.into(),
            name: format!("my {id}"),
            program: program.into(),
            args: vec!["-x".into()],
            cwd: Some("C:\\work".into()),
            options: Default::default(),
        }
    }

    fn shells() -> Vec<DiscoveredProfile> {
        vec![
            discovered("pwsh", "powershell", "pwsh", "C:\\pwsh.exe"),
            discovered("cmd", "cmd", "cmd", "C:\\cmd.exe"),
            discovered("wsl-Ubuntu", "wsl", "wsl", "C:\\wsl.exe"),
        ]
    }

    fn ids(profiles: &[Profile]) -> Vec<&str> {
        profiles.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn discovered_profiles_keep_their_order_and_custom_ones_follow() {
        let merged = merge(&shells(), &[custom("b", "b.exe"), custom("a", "a.exe")]);
        assert_eq!(ids(&merged), ["pwsh", "cmd", "wsl-Ubuntu", "b", "a"]);
        assert!(merged[..3].iter().all(|p| !p.custom));
        assert!(merged[3..].iter().all(|p| p.custom));
        assert_eq!(merged[0].kind, ProfileKind::PowerShell);
        assert_eq!(merged[0].icon, "pwsh");
        assert_eq!(merged[0].cwd, None);
        assert_eq!(merged[3].cwd.as_deref(), Some("C:\\work"));
        assert_eq!(merged[3].kind, ProfileKind::Other);
    }

    #[test]
    fn merging_the_same_input_twice_gives_the_same_list() {
        let again = || merge(&shells(), &[custom("b", "b.exe"), custom("a", "a.exe")]);
        assert_eq!(again(), again());
        assert_eq!(merge(&[], &[]), Vec::new());
        assert_eq!(ids(&merge(&[], &[custom("only", "x.exe")])), ["only"]);
        assert_eq!(ids(&merge(&shells(), &[])), ["pwsh", "cmd", "wsl-Ubuntu"]);
    }

    #[test]
    fn a_custom_profile_with_a_discovered_id_replaces_it_in_place() {
        let merged = merge(&shells(), &[custom("cmd", "C:\\tools\\cmd.exe")]);
        assert_eq!(ids(&merged), ["pwsh", "cmd", "wsl-Ubuntu"]);
        let replaced = &merged[1];
        assert!(!replaced.custom, "still the shell found");
        assert_eq!(replaced.program, "C:\\tools\\cmd.exe");
        assert_eq!(replaced.args, ["-x"]);
    }

    #[test]
    fn of_several_custom_profiles_with_one_id_the_first_wins() {
        let merged = merge(
            &shells(),
            &[
                custom("a", "first.exe"),
                custom("a", "second.exe"),
                custom("pwsh", "p1.exe"),
                custom("pwsh", "p2.exe"),
            ],
        );
        assert_eq!(ids(&merged), ["pwsh", "cmd", "wsl-Ubuntu", "a"]);
        assert_eq!(merged[3].program, "first.exe");
        assert_eq!(merged[0].program, "p1.exe");
    }

    #[test]
    fn icons_come_from_the_discovery_step_or_the_program_name() {
        for (program, icon) in [
            ("pwsh.exe", "pwsh"),
            ("C:\\Program Files\\PowerShell\\7\\PWSH.EXE", "pwsh"),
            ("powershell", "windows-powershell"),
            ("C:/Windows/System32/cmd.exe", "cmd"),
            // The Git logo belongs to Git Bash only, which discovery names itself.
            ("bash.exe", GENERIC_ICON),
            ("C:\\Windows\\System32\\wsl.exe", "wsl"),
            ("ssh", "ssh"),
            ("node.exe", GENERIC_ICON),
            ("C:\\tools\\pwsh-preview\\run.cmd", GENERIC_ICON),
            ("", GENERIC_ICON),
        ] {
            assert_eq!(icon_for_program(program), icon, "{program}");
        }
        assert_eq!(merge(&[], &[custom("n", "node.exe")])[0].icon, GENERIC_ICON);
        assert_eq!(merge(&[], &[custom("p", "pwsh.exe")])[0].icon, "pwsh");
    }

    #[test]
    fn kinds_follow_the_names_of_the_discovery_step() {
        for (name, kind) in [
            ("powershell", ProfileKind::PowerShell),
            ("cmd", ProfileKind::Cmd),
            ("bash", ProfileKind::Bash),
            ("wsl", ProfileKind::Wsl),
            ("ssh", ProfileKind::Ssh),
            ("something-new", ProfileKind::Other),
        ] {
            assert_eq!(ProfileKind::from_name(name), kind, "{name}");
        }
    }

    #[test]
    fn a_shell_closes_its_pane_on_a_clean_exit_and_any_other_program_keeps_its_last_screen() {
        for kind in [
            ProfileKind::PowerShell,
            ProfileKind::Cmd,
            ProfileKind::Bash,
            ProfileKind::Wsl,
            ProfileKind::Ssh,
            ProfileKind::Shell,
        ] {
            assert!(kind.closes_pane_on_clean_exit(), "{kind:?}");
        }
        assert!(!ProfileKind::Other.closes_pane_on_clean_exit());
    }

    #[test]
    fn a_custom_profile_is_classified_by_its_program_like_its_icon() {
        for (program, kind) in [
            ("pwsh.exe", ProfileKind::PowerShell),
            ("C:\\Program Files\\PowerShell\\7\\PWSH.EXE", ProfileKind::PowerShell),
            ("powershell", ProfileKind::PowerShell),
            ("C:/Windows/System32/cmd.exe", ProfileKind::Cmd),
            ("bash.exe", ProfileKind::Bash),
            ("C:\\Windows\\System32\\wsl.exe", ProfileKind::Wsl),
            ("ssh", ProfileKind::Ssh),
            ("zsh", ProfileKind::Shell),
            ("C:\\tools\\nu.exe", ProfileKind::Shell),
            ("/usr/bin/fish", ProfileKind::Shell),
            // Anything else is a program whose last screen is what the session was for.
            ("claude.exe", ProfileKind::Other),
            ("node.exe", ProfileKind::Other),
            ("C:\\tools\\pwsh-preview\\run.cmd", ProfileKind::Other),
            ("", ProfileKind::Other),
        ] {
            assert_eq!(kind_for_program(program), kind, "{program}");
            // An entry without a program adjusts a profile found on the machine; it is no profile of its own.
            let merged = merge(&[], &[custom("mine", program)]);
            if program.is_empty() {
                assert!(merged.is_empty());
            } else {
                assert_eq!(merged[0].kind, kind, "{program}");
            }
        }
    }

    #[test]
    fn a_custom_shell_closes_its_pane_when_it_exits_and_an_agent_keeps_its_screen() {
        let merged = merge(
            &[],
            &[
                custom("pwsh7", "C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
                custom("nushell", "nu.exe"),
                custom("agent", "claude.exe"),
            ],
        );
        let closes: Vec<bool> = merged
            .iter()
            .map(|p| p.kind.closes_pane_on_clean_exit())
            .collect();
        assert_eq!(closes, [true, true, false]);
    }

    #[test]
    fn a_built_in_given_other_arguments_keeps_its_kind_whatever_program_it_names() {
        let merged = merge(
            &shells(),
            &[
                custom("pwsh", "C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
                custom("cmd", "C:\\tools\\launcher.exe"),
            ],
        );
        assert_eq!(merged[0].kind, ProfileKind::PowerShell);
        assert_eq!(&launch_args(&merged[0], true)[1..3], ["-NoExit", "-EncodedCommand"], "keeps its shell integration");
        assert_eq!(merged[1].kind, ProfileKind::Cmd);
        assert!(merged[1].kind.closes_pane_on_clean_exit());
        // The profile that replaces nothing has only its program to go by.
        assert_eq!(
            merge(&shells(), &[custom("launcher", "C:\\tools\\launcher.exe")])[3].kind,
            ProfileKind::Other
        );
    }

    #[test]
    fn the_default_is_the_named_profile_else_the_first() {
        let merged = merge(&shells(), &[]);
        assert_eq!(
            resolve_default(&merged, "cmd").map(|p| p.id.as_str()),
            Some("cmd")
        );
        assert_eq!(
            resolve_default(&merged, "").map(|p| p.id.as_str()),
            Some("pwsh")
        );
        assert_eq!(
            resolve_default(&merged, "removed").map(|p| p.id.as_str()),
            Some("pwsh")
        );
        assert_eq!(resolve_default(&[], "cmd"), None);
        assert_eq!(
            find(&merged, "wsl-Ubuntu").map(|p| p.name.as_str()),
            Some("WSL-UBUNTU")
        );
        assert_eq!(find(&merged, "nope"), None);
    }

    #[test]
    fn only_a_discovered_powershell_gets_the_shell_integration() {
        let merged = merge(&shells(), &[custom("mine", "pwsh.exe")]);
        let pwsh = &merged[0];
        let with = launch_args(pwsh, true);
        assert_eq!(&with[..3], ["-NoLogo", "-NoExit", "-EncodedCommand"]);
        assert_eq!(with.len(), 4);
        assert_eq!(launch_args(pwsh, false), ["-NoLogo"]);
        assert_eq!(
            launch_args(&merged[1], true),
            ["-NoLogo"],
            "cmd has no integration"
        );
        assert_eq!(
            launch_args(&merged[2], true),
            ["-NoLogo"],
            "wsl has no integration"
        );
        assert_eq!(
            launch_args(&merged[3], true),
            ["-x"],
            "custom profiles run as written"
        );
    }

    #[test]
    fn a_powershell_that_runs_its_own_command_is_left_alone() {
        let mut developer = discovered(
            "vs-1-powershell",
            "powershell",
            "visual-studio",
            "C:\\powershell.exe",
        );
        developer.args = vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            "&{Import-Module x}".into(),
        ];
        let merged = merge(&[developer.clone()], &[]);
        assert_eq!(launch_args(&merged[0], true), developer.args);
    }
}
