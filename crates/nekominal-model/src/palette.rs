//! Entries of the command palette: every command, a "new tab" entry per profile and one per color scheme.

use crate::{
    commands::Command,
    fuzzy,
    keymap::Keymap,
    profiles::Profile,
    strings::{Lang, Text},
    themes,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    Run(Command),
    /// Open a tab with the profile of this id.
    NewTab(String),
    /// Switch to the color scheme with this name.
    SetColorScheme(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// Command id, `profile.<id>` or `theme.<name>`.
    pub id: String,
    pub label: String,
    /// The command's first shortcut; empty for the other entries and for unbound commands.
    pub shortcut: String,
    pub action: PaletteAction,
}

/// All entries in display order: commands, then profiles, then color schemes.
pub fn items(lang: Lang, keymap: &Keymap, profiles: &[Profile]) -> Vec<PaletteItem> {
    let commands = Command::ALL
        .iter()
        .filter(|command| command.in_palette())
        .map(|command| PaletteItem {
            id: command.id().to_owned(),
            label: command.label(lang),
            shortcut: keymap.shortcut_text(*command),
            action: PaletteAction::Run(*command),
        });
    let new_tabs = profiles.iter().map(|profile| PaletteItem {
        id: format!("profile.{}", profile.id),
        label: Text::PaletteNewTabWith.format(lang, &[&profile.name]),
        shortcut: String::new(),
        action: PaletteAction::NewTab(profile.id.clone()),
    });
    let schemes = themes::theme_names().map(|name| PaletteItem {
        id: format!("theme.{name}"),
        label: Text::PaletteColorScheme.format(lang, &[&name]),
        shortcut: String::new(),
        action: PaletteAction::SetColorScheme(name.to_owned()),
    });
    commands.chain(new_tabs).chain(schemes).collect()
}

/// The entries matching `query`, best match first; an empty query keeps the order of `items`.
pub fn search(items: Vec<PaletteItem>, query: &str) -> Vec<PaletteItem> {
    fuzzy::rank(items, query, |item| item.label.as_str())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::{
        commands::default_bindings,
        keymap::build_keymap,
        profiles::{DiscoveredProfile, merge},
        settings::CustomProfile,
    };

    fn profiles() -> Vec<Profile> {
        let found = DiscoveredProfile {
            id: "pwsh".into(),
            name: "PowerShell".into(),
            kind: "powershell",
            icon: "pwsh",
            program: "pwsh.exe".into(),
            args: vec![],
        };
        let mine = CustomProfile {
            id: "node".into(),
            name: "Node".into(),
            program: "node.exe".into(),
            args: vec![],
            cwd: None,
        };
        merge(&[found], &[mine])
    }

    fn default_items(lang: Lang) -> Vec<PaletteItem> {
        items(
            lang,
            &build_keymap(default_bindings(), &BTreeMap::new()).keymap,
            &profiles(),
        )
    }

    #[test]
    fn lists_commands_then_profiles_then_color_schemes() {
        let entries = default_items(Lang::ZhTw);
        let commands = Command::ALL.iter().filter(|c| c.in_palette()).count();
        assert_eq!(entries.len(), commands + 2 + themes::SCHEMES.len());
        assert_eq!(entries[0].id, "tab.new");
        assert_eq!(entries[commands].id, "profile.pwsh");
        assert_eq!(entries[commands + 1].id, "profile.node");
        assert_eq!(entries[commands + 2].id, "theme.Tokyo Night");
        let ids: BTreeSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len(), "every entry has its own id");
    }

    #[test]
    fn commands_carry_their_label_shortcut_and_action() {
        let entries = default_items(Lang::ZhTw);
        let new_tab = entries.iter().find(|e| e.id == "tab.new").unwrap();
        assert_eq!(new_tab.label, "新增分頁");
        assert_eq!(new_tab.shortcut, "Ctrl+Shift+T");
        assert_eq!(new_tab.action, PaletteAction::Run(Command::TabNew));
        let unbound = entries.iter().find(|e| e.id == "tab.close").unwrap();
        assert_eq!(unbound.shortcut, "");
    }

    #[test]
    fn hidden_commands_stay_out_and_user_bindings_show() {
        let overrides: BTreeMap<String, Vec<String>> =
            [("tab.new".to_owned(), vec!["Ctrl+T".to_owned()])].into();
        let keymap = build_keymap(default_bindings(), &overrides).keymap;
        let entries = items(Lang::EnUs, &keymap, &[]);
        assert!(entries.iter().all(|e| e.id != "edit.smartCopy"
            && e.id != "tab.profiles"
            && !e.id.starts_with("tab.goto")));
        assert_eq!(
            entries.iter().find(|e| e.id == "tab.new").unwrap().shortcut,
            "Ctrl+T"
        );
        assert!(entries.iter().all(|e| !e.id.starts_with("profile.")));
    }

    #[test]
    fn profile_and_color_scheme_entries_name_what_they_open() {
        let entries = default_items(Lang::ZhTw);
        let profile = entries.iter().find(|e| e.id == "profile.node").unwrap();
        assert_eq!(profile.label, "新增分頁：Node");
        assert_eq!(profile.action, PaletteAction::NewTab("node".into()));
        assert_eq!(profile.shortcut, "");
        let scheme = entries.iter().find(|e| e.id == "theme.Nord").unwrap();
        assert_eq!(scheme.label, "色彩配置：Nord");
        assert_eq!(scheme.action, PaletteAction::SetColorScheme("Nord".into()));
        let english = default_items(Lang::EnUs);
        assert_eq!(
            english
                .iter()
                .find(|e| e.id == "profile.node")
                .unwrap()
                .label,
            "New tab: Node"
        );
    }

    #[test]
    fn a_profile_named_with_braces_is_shown_literally() {
        let weird = DiscoveredProfile {
            id: "x".into(),
            name: "{0} {1}".into(),
            kind: "cmd",
            icon: "cmd",
            program: "cmd.exe".into(),
            args: vec![],
        };
        let entries = items(Lang::EnUs, &Keymap::default(), &merge(&[weird], &[]));
        assert!(entries.iter().any(|e| e.label == "New tab: {0} {1}"));
    }

    #[test]
    fn searching_filters_and_ranks_by_label() {
        let all = default_items(Lang::ZhTw);
        assert_eq!(search(all.clone(), "  "), all);
        // The exact label first, then the profile entries, the shorter one before the longer.
        let found: Vec<String> = search(all.clone(), "新增分頁")
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(found, ["tab.new", "profile.node", "profile.pwsh"]);
        assert_eq!(search(all.clone(), "zzzzqqqq"), Vec::new());
        let nord = search(all, "nord");
        assert_eq!(nord.len(), 1);
        assert_eq!(nord[0].id, "theme.Nord");
    }
}
