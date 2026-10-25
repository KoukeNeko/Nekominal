//! Entries of the command palette: every command, a "go to tab" entry per open tab, a "new tab" entry per
//! profile, one per color scheme and one per row of the settings drawer (`settings_ui::TABLE`).

use crate::{
    commands::Command,
    fuzzy,
    keymap::Keymap,
    profiles::Profile,
    settings_ui,
    strings::{Lang, Text},
    themes,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    Run(Command),
    /// Select the tab with this id.
    GotoTab(u32),
    /// Open a tab with the profile of this id.
    NewTab(String),
    /// Switch to the color scheme with this name.
    SetColorScheme(String),
    /// Open the settings drawer at the row with this id (`settings_ui::Descriptor::id`).
    OpenSetting(String),
    /// Type the snippet at this index of the settings into the focused pane.
    Snippet(usize),
    /// Do this to the saved layout of this name.
    Layout(LayoutAction, String),
}

/// What an entry of a saved layout does to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutAction {
    /// Opens its tabs in this window, after the ones it has; in a window of their own.
    Open,
    OpenInNewWindow,
    Rename,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// Command id, `tabs.<id>`, `profile.<id>`, `theme.<name>` or `setting.<id>`.
    pub id: String,
    pub label: String,
    /// The command's first shortcut; empty for the other entries and for unbound commands.
    pub shortcut: String,
    pub action: PaletteAction,
}

/// An open tab as the palette lists it, in the order of the tab strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteTab {
    pub id: u32,
    pub title: String,
}

/// All entries in display order: commands, then the open tabs, then profiles, then color schemes, then settings.
pub fn items(
    lang: Lang,
    keymap: &Keymap,
    profiles: &[Profile],
    tabs: &[PaletteTab],
    schemes: &[themes::Scheme],
) -> Vec<PaletteItem> {
    let commands = Command::ALL
        .iter()
        .filter(|command| command.in_palette())
        .map(|command| PaletteItem {
            id: command.id().to_owned(),
            label: command.label(lang),
            shortcut: keymap.shortcut_text(*command),
            action: PaletteAction::Run(*command),
        });
    let count = tabs.len();
    let open_tabs = tabs.iter().enumerate().map(|(index, tab)| PaletteItem {
        id: format!("tabs.{}", tab.id),
        label: Text::PaletteGotoTab.format(lang, &[&tab.title]),
        shortcut: Command::tab_goto_number(index, count)
            .map(|number| keymap.shortcut_text(Command::TabGoto(number)))
            .unwrap_or_default(),
        action: PaletteAction::GotoTab(tab.id),
    });
    let new_tabs = profiles.iter().map(|profile| PaletteItem {
        id: format!("profile.{}", profile.id),
        label: Text::PaletteNewTabWith.format(lang, &[&profile.name]),
        shortcut: String::new(),
        action: PaletteAction::NewTab(profile.id.clone()),
    });
    let schemes = themes::theme_names(schemes).map(|name| PaletteItem {
        id: format!("theme.{name}"),
        label: Text::PaletteColorScheme.format(lang, &[&name]),
        shortcut: String::new(),
        action: PaletteAction::SetColorScheme(name.to_owned()),
    });
    let settings = settings_ui::TABLE.iter().map(|row| PaletteItem {
        id: format!("setting.{}", row.id),
        label: Text::PaletteSetting.format(lang, &[&row.label(lang)]),
        shortcut: String::new(),
        action: PaletteAction::OpenSetting(row.id.to_owned()),
    });
    commands
        .chain(open_tabs)
        .chain(new_tabs)
        .chain(schemes)
        .chain(settings)
        .collect()
}

/// An entry per snippet of the settings, by its name: choosing it types the snippet.
pub fn snippet_items(lang: Lang, snippets: &[crate::settings::Snippet]) -> Vec<PaletteItem> {
    snippets
        .iter()
        .enumerate()
        .map(|(index, snippet)| PaletteItem {
            id: format!("snippet.{index}"),
            label: Text::PaletteSnippet.format(lang, &[&snippet.name]),
            shortcut: String::new(),
            action: PaletteAction::Snippet(index),
        })
        .collect()
}

/// Four entries per saved layout, by its name: open it here, open it in a new window, rename it, delete it.
pub fn layout_items(lang: Lang, names: &[&str]) -> Vec<PaletteItem> {
    let entries = [
        (LayoutAction::Open, "open", Text::PaletteOpenLayout),
        (LayoutAction::OpenInNewWindow, "openWindow", Text::PaletteOpenLayoutWindow),
        (LayoutAction::Rename, "rename", Text::PaletteRenameLayout),
        (LayoutAction::Delete, "delete", Text::PaletteDeleteLayout),
    ];
    names
        .iter()
        .flat_map(|name| {
            entries.map(|(action, id, text)| PaletteItem {
                id: format!("layout.{id}.{name}"),
                label: text.format(lang, &[name]),
                shortcut: String::new(),
                action: PaletteAction::Layout(action, (*name).to_owned()),
            })
        })
        .collect()
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
            options: Default::default(),
        };
        merge(&[found], &[mine])
    }

    #[test]
    fn a_snippet_is_an_entry_by_its_name_and_found_by_it() {
        let snippets = [crate::settings::Snippet { name: "git status".into(), input: "git status\r".into() }];
        let entries = snippet_items(Lang::EnUs, &snippets);
        assert_eq!(entries[0].label, "Snippet: git status");
        assert_eq!(entries[0].action, PaletteAction::Snippet(0));
        assert_eq!(search(entries, "gst").len(), 1);
    }

    #[test]
    fn a_saved_layout_has_entries_to_open_it_here_or_in_a_window_rename_it_and_delete_it() {
        let entries = layout_items(Lang::EnUs, &["agents", "{0}"]);
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(&labels[..4], ["Open layout: agents", "Open layout in a new window: agents", "Rename layout: agents", "Delete layout: agents"]);
        assert_eq!(entries[1].action, PaletteAction::Layout(LayoutAction::OpenInNewWindow, "agents".into()));
        assert_eq!(entries[7].label, "Delete layout: {0}", "a name with braces is shown literally");
        let ids: BTreeSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len());
        let found = search(entries, "open layout agents");
        assert_eq!(found[0].action, PaletteAction::Layout(LayoutAction::Open, "agents".into()));
        assert_eq!(layout_items(Lang::ZhTw, &["工作"])[0].label, "開啟配置：工作");
    }

    fn default_items(lang: Lang) -> Vec<PaletteItem> {
        items(
            lang,
            &build_keymap(default_bindings(), &BTreeMap::new()).keymap,
            &profiles(),
            &[],
            &[],
        )
    }

    #[test]
    fn lists_commands_then_profiles_then_color_schemes_then_settings() {
        let entries = default_items(Lang::ZhTw);
        let commands = Command::ALL.iter().filter(|c| c.in_palette()).count();
        assert_eq!(entries.len(), commands + 2 + themes::SCHEMES.len() + settings_ui::TABLE.len());
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
        let entries = items(Lang::EnUs, &keymap, &[], &[], &[]);
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
        let entries = items(Lang::EnUs, &Keymap::default(), &merge(&[weird], &[]), &[], &[]);
        assert!(entries.iter().any(|e| e.label == "New tab: {0} {1}"));
    }

    fn open_tabs(titles: &[&str]) -> Vec<PaletteTab> {
        titles
            .iter()
            .enumerate()
            .map(|(at, title)| PaletteTab {
                id: 10 + at as u32,
                title: (*title).to_owned(),
            })
            .collect()
    }

    #[test]
    fn every_open_tab_gets_a_go_to_entry_between_the_commands_and_the_profiles() {
        let keymap = build_keymap(default_bindings(), &BTreeMap::new()).keymap;
        let tabs = open_tabs(&["build", "logs", "claude"]);
        let entries = items(Lang::ZhTw, &keymap, &profiles(), &tabs, &[]);
        let commands = Command::ALL.iter().filter(|c| c.in_palette()).count();
        assert_eq!(entries[commands].id, "tabs.10");
        assert_eq!(entries[commands].label, "切換到分頁：build");
        assert_eq!(entries[commands].action, PaletteAction::GotoTab(10));
        assert_eq!(entries[commands].shortcut, "Ctrl+Shift+1");
        assert_eq!(entries[commands + 2].shortcut, "Ctrl+Shift+3");
        assert_eq!(entries[commands + 3].id, "profile.pwsh");
        assert_eq!(entries.len(), commands + 3 + 2 + themes::SCHEMES.len() + settings_ui::TABLE.len());
        let english = items(Lang::EnUs, &keymap, &[], &tabs, &[]);
        assert_eq!(english[commands + 1].label, "Go to tab: logs");
    }

    #[test]
    fn only_the_first_eight_tabs_and_the_last_show_a_number_shortcut() {
        let keymap = build_keymap(default_bindings(), &BTreeMap::new()).keymap;
        let titles: Vec<String> = (1..=12).map(|n| format!("t{n}")).collect();
        let tabs = open_tabs(&titles.iter().map(String::as_str).collect::<Vec<_>>());
        let commands = Command::ALL.iter().filter(|c| c.in_palette()).count();
        let entries = items(Lang::EnUs, &keymap, &[], &tabs, &[]);
        let shortcuts: Vec<&str> = entries[commands..commands + 12]
            .iter()
            .map(|e| e.shortcut.as_str())
            .collect();
        assert_eq!(
            &shortcuts[..8],
            [
                "Ctrl+Shift+1",
                "Ctrl+Shift+2",
                "Ctrl+Shift+3",
                "Ctrl+Shift+4",
                "Ctrl+Shift+5",
                "Ctrl+Shift+6",
                "Ctrl+Shift+7",
                "Ctrl+Shift+8"
            ]
        );
        assert_eq!(&shortcuts[8..11], ["", "", ""]);
        assert_eq!(shortcuts[11], "Ctrl+Shift+9");
    }

    #[test]
    fn a_tab_titled_with_braces_is_shown_literally_and_titles_can_be_searched() {
        let keymap = Keymap::default();
        let tabs = open_tabs(&["{0} {1}", "build"]);
        let entries = items(Lang::EnUs, &keymap, &[], &tabs, &[]);
        assert!(entries.iter().any(|e| e.label == "Go to tab: {0} {1}"));
        let found = search(entries, "to tab: bui");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, PaletteAction::GotoTab(11));
    }

    #[test]
    fn every_row_of_the_settings_drawer_has_an_entry_that_opens_it_there() {
        let zh = default_items(Lang::ZhTw);
        let opacity = zh.iter().find(|e| e.id == "setting.appearance.opacity").unwrap();
        assert_eq!(opacity.label, "設定：背景不透明度");
        assert_eq!(opacity.action, PaletteAction::OpenSetting("appearance.opacity".into()));
        assert_eq!(opacity.shortcut, "");
        let found = search(zh.clone(), "不透明");
        assert!(found.iter().any(|e| e.id == "setting.appearance.opacity"));
        assert_eq!(settings_ui::TABLE.iter().filter(|row| zh.iter().any(|e| e.id == format!("setting.{}", row.id))).count(), settings_ui::TABLE.len());
        let en = default_items(Lang::EnUs);
        assert_eq!(en.iter().find(|e| e.id == "setting.language").unwrap().label, "Settings: Language");
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
