//! Every user-visible string of the shell. The rows below name each string (its variant and its id) and say what it is
//! for; the words are in `lang/<tag>.json`, a file a language, built into the program. A string a language does not
//! have shows in English, and the tests hold English and Traditional Chinese to having all of them. A command's label is the
//! row named by the command id (`Command::text`), and everything that offers the same action (tab strip
//! tooltip, tool button, menu, palette) shows that row: the same action keeps one label. Strings with
//! `{0}`, `{1}` placeholders are filled by [`Text::format`].

use std::fmt::{Display, Write as _};

/// The product name, the same in every language.
pub const APP_NAME: &str = "Nekominal";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    ZhTw,
    EnUs,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::ZhTw, Lang::EnUs];

    /// BCP 47 tag, as `settings.json` and the system report it.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::ZhTw => "zh-TW",
            Lang::EnUs => "en-US",
        }
    }

    /// The language whose [`tag`](Lang::tag) is `tag`, in any case.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|lang| lang.tag().eq_ignore_ascii_case(tag))
    }

    /// The language for a system locale name such as `zh-Hant-TW`, `zh-HK` or `en-GB`: any Chinese locale
    /// gets Traditional Chinese (Taiwan), everything else English.
    pub fn from_system_tag(tag: &str) -> Lang {
        let primary = tag.split(['-', '_']).next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("zh") {
            Lang::ZhTw
        } else {
            Lang::EnUs
        }
    }
}

macro_rules! texts {
    ($($variant:ident $id:literal;)*) => {
        /// A string of the interface; see the module documentation.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Text {
            $($variant,)*
        }

        /// (string, id), in declaration order.
        const TABLE: &[(Text, &str)] = &[
            $((Text::$variant, $id),)*
        ];
    };
}

/// The words of each language, a file of `lang/` (`<tag>.json`, an object from id to text) built into the program.
fn file(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhTw => include_str!("../lang/zh-TW.json"),
        Lang::EnUs => include_str!("../lang/en-US.json"),
    }
}

/// The text of every string in `lang`, in the order of [`TABLE`]; None where its file does not have it. Read once.
fn words(lang: Lang) -> &'static [Option<String>] {
    static READ: std::sync::OnceLock<Vec<Vec<Option<String>>>> = std::sync::OnceLock::new();
    let all = READ.get_or_init(|| {
        Lang::ALL
            .iter()
            .map(|lang| {
                // A file that does not read is a language with no words of its own: every string falls back to English.
                let mut map: std::collections::HashMap<String, String> = serde_json::from_str(file(*lang)).unwrap_or_default();
                TABLE.iter().map(|(_, id)| map.remove(*id)).collect()
            })
            .collect()
    });
    &all[Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0)]
}

texts! {
    // Commands: the id is the command id.
    TabNew "tab.new";
    TabClose "tab.close";
    TabNext "tab.next";
    TabPrevious "tab.previous";
    TabGoto "tab.goto";
    TabSelectLeft "tab.selectLeft";
    TabSelectRight "tab.selectRight";
    TabNextNeedsResponse "tab.nextNeedsResponse";
    TabMoveLeft "tab.moveLeft";
    TabMoveRight "tab.moveRight";
    TabProfiles "tab.profiles";
    PaneSplitRight "pane.splitRight";
    PaneSplitDown "pane.splitDown";
    PaneClose "pane.close";
    PaneZoom "pane.zoom";
    PaneRestart "pane.restart";
    PaneFocusLeft "pane.focusLeft";
    PaneFocusRight "pane.focusRight";
    PaneFocusUp "pane.focusUp";
    PaneFocusDown "pane.focusDown";
    PaneResizeLeft "pane.resizeLeft";
    PaneResizeRight "pane.resizeRight";
    PaneResizeUp "pane.resizeUp";
    PaneResizeDown "pane.resizeDown";
    EditCopy "edit.copy";
    EditSmartCopy "edit.smartCopy";
    EditPaste "edit.paste";
    EditSelectAll "edit.selectAll";
    EditFind "edit.find";
    EditClear "edit.clear";
    ScrollPageUp "scroll.pageUp";
    ScrollPageDown "scroll.pageDown";
    ScrollTop "scroll.top";
    ScrollBottom "scroll.bottom";
    PromptPrevious "prompt.previous";
    PromptNext "prompt.next";
    ViewFullscreen "view.fullscreen";
    ViewSidebar "view.sidebar";
    ViewAgents "view.agents";
    AppUpdate "app.update";
    TabColor "tab.color";
    PaneSwapLeft "pane.swapLeft";
    PaneSwapRight "pane.swapRight";
    PaneSwapUp "pane.swapUp";
    PaneSwapDown "pane.swapDown";
    PaneToTab "pane.toTab";
    PaneBroadcast "pane.broadcast";
    EditExport "edit.export";
    EditMarkMode "edit.markMode";
    EditCopyFormatted "edit.copyFormatted";
    EditSelectOutput "edit.selectOutput";
    EditCopyOutput "edit.copyOutput";
    EditHints "edit.hints";
    ViewAlwaysOnTop "view.alwaysOnTop";
    SettingsExplorerMenu "settings.explorerMenu";
    SettingsInheritEnvironment "settings.inheritEnvironment";
    SettingsEditor "settings.editor";
    EditSearchTabs "edit.searchTabs";
    ExplorerOpenHere "explorer.openHere";
    ExplorerOpen "explorer.open";
    ExplorerOpenAdmin "explorer.openAdmin";
    ViewFocusMode "view.focusMode";
    ViewQuake "view.quake";
    QuakeKeyTaken "quake.keyTaken";
    QuakeKeyMissing "quake.keyMissing";
    TabRenameTo "tab.renameTo";
    TabRenameAuto "tab.renameAuto";
    DialogCloseTabs "dialog.closeTabs";
    ColorRed "color.red";
    ColorOrange "color.orange";
    ColorYellow "color.yellow";
    ColorGreen "color.green";
    ColorTeal "color.teal";
    ColorBlue "color.blue";
    ColorPurple "color.purple";
    ColorPink "color.pink";
    ViewZoomIn "view.zoomIn";
    ViewZoomOut "view.zoomOut";
    ViewZoomReset "view.zoomReset";
    AppPalette "app.palette";
    AppNewWindow "app.newWindow";
    AppSettings "app.settings";
    // "設定檔" already names a profile, so the settings file is spelled out by its format.
    AppOpenSettingsFile "app.openSettingsFile";
    AppReloadSettings "app.reloadSettings";
    AppImportSchemes "app.importSchemes";
    AppImportItermColors "app.importItermColors";
    SchemesImported "schemes.imported";
    SchemesNoneNew "schemes.noneNew";
    SchemesNotImported "schemes.notImported";
    TabNewWorktree "tab.newWorktree";
    TabRemoveWorktree "tab.removeWorktree";
    AppSaveLayout "app.saveLayout";
    AppearanceBackdropNext "appearance.backdrop.next";
    AppearanceOpacityUp "appearance.opacity.up";
    AppearanceOpacityDown "appearance.opacity.down";

    // Tab strip and menus that have no command of their own.
    TabUntitled "tab.untitled";
    TabRename "tab.rename";
    TabDuplicate "tab.duplicate";
    TabCloseOthers "tab.closeOthers";
    TabCloseRight "tab.closeRight";
    TabMoveToNewWindow "tab.moveToNewWindow";
    MenuTerminal "menu.terminal";
    // The button of the top bar that opens the menu of everything the bar folds away, and the row of that menu that
    // opens the appearance flyout (its backdrop row is `SettingsBackdrop`).
    MenuApp "menu.app";
    MenuOpacity "menu.opacity";
    DropPastePath "drop.pastePath";

    // The position plate of the tab strip and the list of every tab (the palette in its tab mode).
    TabAll "tab.all";
    TabListPlaceholder "tabs.placeholder";
    TabListEmpty "tabs.empty";
    TabPaneCount "tab.paneCount";
    // Which pane of a tab waits for the user: the list of every tab says it in place of the pane count.
    TabPanePosition "tab.panePosition";
    TabZoomed "tab.zoomed";
    // The accessible name of a tab that is not at rest: its title, then its state.
    TabNameStatus "tab.nameStatus";

    // Tab and pane indicators.
    AgentWorking "agent.working";
    AgentBlocked "agent.blocked";
    AgentDone "agent.done";
    AgentError "agent.error";
    AgentNotification "agent.notification";
    AgentBell "agent.bell";
    // The agents panel: its last row, and its only row when no pane runs an agent.
    AgentsToday "agents.today";
    AgentsNone "agents.none";

    // Find bar. Its text box is labelled `EditFind`.
    FindNone "find.none";
    FindCount "find.count";
    FindMatchCase "find.matchCase";
    FindRegex "find.regex";
    FindWholeWord "find.wholeWord";
    FindPrevious "find.previous";
    FindNext "find.next";

    // Command palette. A profile entry starts with the label of `TabNew`.
    PalettePlaceholder "palette.placeholder";
    PaletteEmpty "palette.empty";
    PaletteNewTabWith "palette.newTabWith";
    PaletteColorScheme "palette.colorScheme";
    PaletteGotoTab "palette.gotoTab";
    // An entry that opens the settings drawer at a row.
    PaletteSetting "palette.setting";
    PaletteSnippet "palette.snippet";
    // The entries of a saved layout, by its name.
    PaletteOpenLayout "palette.openLayout";
    PaletteOpenLayoutWindow "palette.openLayoutWindow";
    PaletteRenameLayout "palette.renameLayout";
    PaletteDeleteLayout "palette.deleteLayout";
    // The palette as a line to type a name in: the row Enter chooses, or what is wrong with the line.
    LayoutTypeName "layout.typeName";
    LayoutSaveAs "layout.saveAs";
    LayoutRenameTo "layout.renameTo";
    LayoutFailed "layout.failed";
    WorktreeTypeBranch "worktree.typeBranch";
    WorktreeCreate "worktree.create";
    WorktreeBadBranch "worktree.badBranch";
    WorktreeReservedBranch "worktree.reservedBranch";
    WorktreeCopyFailed "worktree.copyFailed";
    WorktreeNotRepo "worktree.notRepo";
    WorktreeFailed "worktree.failed";
    WorktreeIsMain "worktree.isMain";
    WorktreeDirty "worktree.dirty";
    WorktreeDetached "worktree.detached";
    // The dialog of Remove this worktree: its title, the lines of what was checked, the two choices and the main button.
    DialogRemoveWorktree "dialog.removeWorktree";
    WorktreeClean "worktree.clean";
    WorktreeChanged "worktree.changed";
    WorktreePushed "worktree.pushed";
    WorktreeUnpushed "worktree.unpushed";
    WorktreeNoRemote "worktree.noRemote";
    WorktreeMerged "worktree.merged";
    WorktreeMergedPr "worktree.mergedPr";
    WorktreeNotMerged "worktree.notMerged";
    WorktreeTabsClose "worktree.tabsClose";
    WorktreeAgentRuns "worktree.agentRuns";
    WorktreeIgnored "worktree.ignored";
    WorktreeMergedIgnoredLost "worktree.mergedIgnoredLost";
    WorktreeDeleteBranch "worktree.deleteBranch";
    WorktreeNothingLost "worktree.nothingLost";
    WorktreeOnlyMerged "worktree.onlyMerged";
    WorktreeArchive "worktree.archive";
    WorktreeKeepsBranch "worktree.keepsBranch";
    WorktreeCommitPushFirst "worktree.commitPushFirst";
    WorktreeCommitFirst "worktree.commitFirst";
    WorktreePushFirst "worktree.pushFirst";
    WorktreeChangesLost "worktree.changesLost";
    WorktreeRemove "worktree.remove";
    WorktreeShowTab "worktree.showTab";
    WorktreeBranchKept "worktree.branchKept";
    WorktreeRemoveFailed "worktree.removeFailed";
    SuggestionHistory "suggestion.history";
    SuggestionSnippet "suggestion.snippet";
    EditSuggestions "edit.suggestions";

    // Buttons shared by dialogs. A confirmation's button names the action; Paste uses `EditPaste`.
    CommonClose "common.close";
    CommonCancel "common.cancel";
    CommonOk "common.ok";
    CommonRetry "common.retry";
    CommonAuto "common.auto";
    // A row of the settings drawer that differs from its default: the button that puts it back, and the two buttons that
    // step a number.
    CommonRestore "common.restore";
    CommonRestoreDefault "common.restoreDefault";
    CommonDecrease "common.decrease";
    CommonIncrease "common.increase";

    // Confirmations: titles that name the action, answered by the buttons above.
    DialogPasteLines "dialog.pasteLines";
    DialogPasteSize "dialog.pasteSize";
    DialogCloseSessions "dialog.closeSessions";
    DialogCloseTab "dialog.closeTab";
    DialogRestartSession "dialog.restartSession";
    DialogRestoreKeys "dialog.restoreKeys";

    // A setting that was changed and could not be put in effect: the InfoBar names it, the row of the opacity says it.
    SettingNotApplied "setting.notApplied";
    // The shortcuts page: the button of a row that adds a binding, what a row says while a key is waited for and when
    // the key is taken (by a command or by a program in the terminal), and the buttons that answer that.
    KeyAdd "keys.add";
    KeyRecord "keys.record";
    KeyConflict "keys.conflict";
    KeysPress "keys.press";
    KeyReserved "keys.reserved";
    SettingsSnippetsHeading "settings.snippetsHeading";
    SettingsSnippets "settings.snippets";
    SettingsNotificationsHeading "settings.notificationsHeading";
    SettingsLayouts "settings.layouts";
    SnippetsAdd "snippets.add";
    SnippetInput "snippets.input";
    KeysResetAll "keys.resetAll";
    KeyReplace "keys.replace";
    // The tag of a row whose value in the settings file was not valid, and the heading of the list of those problems.
    SettingInvalid "setting.invalid";
    SettingsProblems "settings.problems";
    LabelValue "common.labelValue";

    // Messages: what happened, and the recovery when there is one.
    StatusExited "status.exited";
    StatusSpawnFailed "status.spawnFailed";
    StatusNoProfile "status.noProfile";
    StatusSettingsProblems "status.settingsProblems";
    // What a value of the settings file should have been (the key comes before it, see `LabelValue`).
    ProblemNumber "problem.number";
    ProblemBool "problem.bool";
    ProblemText "problem.text";
    ProblemObject "problem.object";
    ProblemArray "problem.array";
    ProblemTextOrList "problem.textOrList";
    ProblemFontFamily "problem.fontFamily";
    ProblemOneOf "problem.oneOf";
    ProblemRequired "problem.required";
    ProblemColor "problem.color";
    ProblemUnknownTheme "problem.unknownTheme";
    ProblemUnknownCommand "problem.unknownCommand";
    ProblemBadShortcut "problem.badShortcut";
    ProblemRegex "problem.regex";
    ProblemMostEntries "problem.mostEntries";
    ProblemMostChars "problem.mostChars";
    ProblemUnparsable "problem.unparsable";
    ProblemJsonObject "problem.jsonObject";
    StatusSettingsSaveFailed "status.settingsSaveFailed";
    StatusSettingsLoadFailed "status.settingsLoadFailed";
    StatusOpenFailed "status.openFailed";
    StatusRestoreFailed "status.restoreFailed";
    InfobarWidthModel "infobar.widthModel";
    InfobarNoColor "infobar.noColor";

    // Settings drawer: sections, then one row label per setting followed by its choices.
    SettingsSearch "settings.search";
    SettingsEmpty "settings.empty";
    SettingsAppearance "settings.appearance";
    SettingsTerminal "settings.terminal";
    SettingsProfiles "settings.profiles";
    SettingsAgents "settings.agents";
    SettingsKeys "settings.keys";
    SettingsAdvanced "settings.advanced";
    // The Previews page: the section, and its two rows (the preview of an image under the pointer).
    SettingsPreviews "settings.previews";
    SettingsPreviewImages "settings.previewImages";
    SettingsPreviewSize "settings.previewSize";
    // The two choices of a row that is on or off (and the "off" of the clipboard row).
    ToggleOn "toggle.on";
    ToggleOff "toggle.off";

    SettingsLanguage "settings.language";
    LanguageZhTw "language.zhTw";
    LanguageEnUs "language.enUs";
    SettingsRestoreSession "settings.restoreSession";
    // The line under the text a restored pane shows again from the last session; {0} is when that session ended.
    PaneRestored "pane.restored";
    // The line under the text of a pane whose program could not be kept through an update: a new shell runs below it.
    PaneNotKept "pane.notKept";
    // The update could not start: the tabs stay as they are. {0} is what went wrong.
    UpdateHandOffFailed "update.handOffFailed";
    SettingsConfirmClose "settings.confirmClose";
    SettingsConfirmCloseThreshold "settings.confirmCloseThreshold";

    SettingsTabBarDensity "settings.tabBarDensity";
    DensityComfortable "density.comfortable";
    DensityCompact "density.compact";
    SettingsBackdrop "settings.backdrop";
    BackdropAcrylic "backdrop.acrylic";
    BackdropMica "backdrop.mica";
    BackdropMicaAlt "backdrop.micaAlt";
    BackdropNone "backdrop.none";
    SettingsOpacity "settings.opacity";
    SettingsTheme "settings.theme";
    SettingsTabTitle "settings.tabTitle";
    SettingsFontFamily "settings.fontFamily";
    SettingsFontSize "settings.fontSize";
    SettingsLineHeight "settings.lineHeight";
    SettingsLigatures "settings.ligatures";
    NoteLigatures "settings.ligaturesNote";
    SettingsCursorStyle "settings.cursorStyle";
    CursorBlock "cursor.block";
    CursorBar "cursor.bar";
    CursorUnderline "cursor.underline";
    SettingsCursorBlink "settings.cursorBlink";
    SettingsCursorBlinkRate "settings.cursorBlinkRate";

    SettingsScrollback "settings.scrollback";
    SettingsDefaultProfile "settings.defaultProfile";
    ProfileName "profile.name";
    ProfileProgram "profile.program";
    ProfileArgs "profile.args";
    ProfileCwd "profile.cwd";
    ProfileEnv "profile.env";
    ProfileElevate "profile.elevate";
    ProfileSameAsWindow "profile.sameAsWindow";
    ProfileNew "profile.new";
    ProfileDelete "profile.delete";
    ProfileRestore "profile.restore";
    ProfileBack "profile.back";
    ProfileNeedsValue "profile.needsValue";
    ProfileBadVariable "profile.badVariable";
    ProfileDuplicateVariable "profile.duplicateVariable";
    ProfileAddVariable "profile.addVariable";
    ProfileValue "profile.value";
    ProfileBrowse "profile.browse";
    SettingsCopyOnSelect "settings.copyOnSelect";
    SettingsStickyHeader "settings.stickyHeader";
    SettingsRightClick "settings.rightClick";
    RightClickSmart "rightClick.smart";
    RightClickMenu "rightClick.menu";
    SettingsWarnMultiline "settings.warnMultiline";
    SettingsWarnLarge "settings.warnLarge";
    SettingsBellVisual "settings.bellVisual";
    SettingsBellTaskbar "settings.bellTaskbar";
    SettingsBellSound "settings.bellSound";
    BellSoundBackground "bellSound.background";
    BellSoundAlways "bellSound.always";
    SettingsToast "settings.toast";
    SettingsLongCommand "settings.longCommand";
    // The phone notifications of the Terminal page: the rows, the notes under their labels, and the test row.
    SettingsPushHeading "settings.pushHeading";
    SettingsPushUrl "settings.pushUrl";
    NotePushUrl "settings.pushUrlNote";
    SettingsPushKind "settings.pushKind";
    PushKindNtfy "pushKind.ntfy";
    PushKindWebhook "pushKind.webhook";
    SettingsPushWhen "settings.pushWhen";
    PushWhenAway "pushWhen.away";
    SettingsPushToken "settings.pushToken";
    NotePushToken "settings.pushTokenNote";
    SettingsPushEvents "settings.pushEvents";
    PushEventFailed "pushEvent.failed";
    PushEventDone "pushEvent.done";
    PushEventLongCommand "pushEvent.longCommand";
    PushEventMessage "pushEvent.message";
    PushBadUrl "push.badUrl";
    PushTest "push.test";
    PushTestSend "push.testSend";
    PushTestBody "push.testBody";
    PushSending "push.sending";
    PushDelivered "push.delivered";
    PushLastSent "push.lastSent";
    PushLastTried "push.lastTried";
    PushJustNow "push.justNow";
    PushMinutesAgo "push.minutesAgo";
    // The Triggers page: its title and intro, the form of a rule, its actions and the ready-made rules.
    SettingsTriggers "settings.triggers";
    TriggersIntro "triggers.intro";
    TriggerAdd "triggers.add";
    TriggerPattern "triggers.pattern";
    TriggerInstant "triggers.instant";
    TriggerProgram "triggers.program";
    TriggerProgramNote "triggers.programNote";
    TriggerAction "triggers.action";
    TriggerHighlight "triggers.highlight";
    TriggerBadge "triggers.badge";
    TriggerSend "triggers.send";
    TriggerBell "triggers.bell";
    TriggerNotifyText "triggers.notifyText";
    TriggerSendText "triggers.sendText";
    TriggerColor "triggers.color";
    TriggerBadPattern "triggers.badPattern";
    TriggerPresetErrors "triggers.presetErrors";
    TriggerPresetTests "triggers.presetTests";
    TriggerPresetDeploy "triggers.presetDeploy";
    TriggerPresetPassword "triggers.presetPassword";
    ToastCommandDone "toast.commandDone";
    ToastCommandFailed "toast.commandFailed";
    SettingsKitty "settings.kitty";
    SettingsAgentEnvironment "settings.agentEnvironment";
    SettingsShellIntegration "settings.shellIntegration";
    SettingsOsc52 "settings.osc52";
    Osc52Write "osc52.write";
    Osc52ReadWrite "osc52.readWrite";
    SettingsTabLayout "settings.tabLayout";
    LayoutTop "layout.top";
    LayoutList "layout.list";
    LayoutIcons "layout.icons";
    SettingsTabStyle "settings.tabStyle";
    NoteTabStyle "settings.tabStyleNote";
    StyleActivity "style.activity";
    StyleCards "style.cards";
    StyleRepo "style.repo";
    RepoTabs "group.repoTabs";
    SettingsSidebarOpen "settings.sidebarOpen";
    SettingsGroupByState "settings.groupByState";
    TabsWaiting "tabs.waiting";
    GroupWaiting "group.waiting";
    GroupOther "group.other";
    SettingsResults "settings.results";
    WelcomeTitle "welcome.title";
    WelcomeLater "welcome.later";
    WelcomeTopNote "welcome.topNote";
    WelcomeSide "welcome.side";
    WelcomeSideNote "welcome.sideNote";
    WelcomeStyle "welcome.style";
    StyleList "style.list";
    StyleIcons "style.icons";
    WelcomeStart "welcome.start";
    SettingsStatusBar "settings.statusBar";
    SettingsStatusBarNoAgent "settings.statusBarNoAgent";
    StatusBarPlace "statusBar.place";
    StatusBarHide "statusBar.hide";
    SettingsStatusState "settings.statusState";
    SettingsStatusModel "settings.statusModel";
    SettingsStatusContext "settings.statusContext";
    SettingsStatusCost "settings.statusCost";
    SettingsStatusTime "settings.statusTime";
    SettingsStatusBranch "settings.statusBranch";
    SettingsStatusFolder "settings.statusFolder";
    WindowMove "window.move";
    WindowSize "window.size";
    WindowMinimize "window.minimize";
    WindowMaximize "window.maximize";
    ClaudeAuto "claude.auto";
    ClaudeBarOff "claude.barOff";
    CardContext "card.context";
    CardUsed "card.used";
    CardWindow "card.window";
    // Under the bar of the context card, when Claude Code's settings name a window it compacts at. {0} is that window, {1} the tokens until it.
    CardCompactAt "card.compactAt";
    CardCompactLeft "card.compactLeft";
    // The card of a pull request, under its badge while the pointer is on it: its state, checks, review and merge, and
    // its buttons. {0} is a login or a count.
    PrOpenOnGitHub "pr.openOnGitHub";
    PrDismiss "pr.dismiss";
    PrStateOpen "pr.stateOpen";
    PrStateDraft "pr.stateDraft";
    PrStateMerged "pr.stateMerged";
    PrStateClosed "pr.stateClosed";
    PrBy "pr.by";
    PrChecks "pr.checks";
    PrChecksPassed "pr.checksPassed";
    PrChecksFailed "pr.checksFailed";
    PrChecksRunning "pr.checksRunning";
    PrApproved "pr.approved";
    PrChangesRequested "pr.changesRequested";
    PrReviewRequired "pr.reviewRequired";
    PrBehind "pr.behind";
    PrConflicts "pr.conflicts";
    PrBlocked "pr.blocked";
    CardCacheRead "card.cacheRead";
    CardCacheWrite "card.cacheWrite";
    CardNewInput "card.newInput";
    CardFree "card.free";
    CardSession "card.session";
    CardRemote "card.remote";
    CardRemoteOn "card.remoteOn";
    CardCopyLink "card.copyLink";
    CardCost "card.cost";
    CardTime "card.time";
    CardTokens "card.tokens";
    CardLastReply "card.lastReply";
    CardLines "card.lines";
    CardIn "card.in";
    CardOut "card.out";
    // The menu of the right button on a path or an address of a pane: open it (`CommonOpen`, `CardCopyLink` copy an address).
    LinkCopyPath "link.copyPath";
    LinkReveal "link.reveal";
    // The card of the branch, under it in the title bar: what the folder is, and while git is asked, or when it cannot say.
    BranchMain "branch.main";
    BranchWorktree "branch.worktree";
    BranchDetached "branch.detached";
    BranchReading "branch.reading";
    BranchFailed "branch.failed";
    // {0} is the branch it pushes to (origin/master), {1} a count; the hours and days of the last commit's age.
    BranchSynced "branch.synced";
    BranchNoUpstream "branch.noUpstream";
    BranchBehind "branch.behind";
    BranchStashes "branch.stashes";
    BranchHoursAgo "branch.hoursAgo";
    BranchDaysAgo "branch.daysAgo";
    SubagentsTitle "subagents.title";
    SubagentsWorkflow "subagents.workflow";
    SubagentsRunning "subagents.running";
    SubagentsDone "subagents.done";
    SubagentsMore "subagents.more";
    SubagentsRunningFor "subagents.runningFor";
    SubagentsFinishedAfter "subagents.finishedAfter";
    SubagentsFailedCount "subagents.failedCount";
    SubagentsReply "subagents.reply";
    SubagentsResult "subagents.result";
    SubagentsPrompt "subagents.prompt";
    SubagentsToolCalls "subagents.toolCalls";
    SubagentsEarlier "subagents.earlier";
    SubagentsCopyReply "subagents.copyReply";
    SubagentsOpenTranscript "subagents.openTranscript";
    SubagentsOpenJournal "subagents.openJournal";
    SubagentsEscBack "subagents.escBack";
    CardNoStatusLine "card.noStatusLine";
    ClaudeForeign "claude.foreign";
    ClaudeMissing "claude.missing";

    SettingsFile "settings.file";
    SettingsVersion "settings.version";
    SettingsAbout "settings.about";
    SettingsRepo "settings.repo";
    SettingsContributors "settings.contributors";
    CommonOpen "common.open";
    UpdateCheck "update.check";
    UpdateChecking "update.checking";
    UpdateLatest "update.latest";
    UpdateNewer "update.newer";
    UpdateFailed "update.failed";
    UpdateNotScoop "update.notScoop";
    UpdateDownload "update.download";
    SettingsCheckUpdates "settings.checkUpdates";
    // The notes of the releases on the About page: of the versions after this one, or of this one.
    SettingsReleaseNotes "settings.releaseNotes";
    // The window of the updater (the hold mode), which holds the tabs while the app is replaced. {0} is a version or a count.
    UpdateHoldTitle "update.holdTitle";
    UpdateHoldTaking "update.holdTaking";
    UpdateHoldNotGiven "update.holdNotGiven";
    UpdateHoldAsking "update.holdAsking";
    UpdateHoldWaiting "update.holdWaiting";
    UpdateHoldScoop "update.holdScoop";
    UpdateHoldStillRunning "update.holdStillRunning";
    UpdateHoldDownloading "update.holdDownloading";
    UpdateHoldInstalling "update.holdInstalling";
    UpdateHoldStarting "update.holdStarting";
    UpdateHoldScoopFailed "update.holdScoopFailed";
    UpdateHoldInstallerFailed "update.holdInstallerFailed";
    UpdateHoldDone "update.holdDone";
    UpdateHoldNotStarted "update.holdNotStarted";
    // The toast of a newer version the app found by itself. {0} is the version.
    UpdateToastTitle "update.toastTitle";
    UpdateToastBody "update.toastBody";
    SettingsConpty "settings.conpty";
    ConptyBundled "conpty.bundled";
    ConptySystem "conpty.system";
}

impl Text {
    /// Every string, in declaration order.
    pub fn all() -> impl Iterator<Item = Text> {
        TABLE.iter().map(|row| row.0)
    }

    fn row(self) -> &'static (Text, &'static str) {
        // Every variant has exactly one row, in declaration order.
        &TABLE[self as usize]
    }

    /// The stable id: the command id for a command's label, `area.name` otherwise.
    pub fn id(self) -> &'static str {
        self.row().1
    }

    pub fn from_id(id: &str) -> Option<Text> {
        TABLE.iter().find(|row| row.1 == id).map(|row| row.0)
    }

    /// The string with its `{0}` placeholders still in place: in `lang`, else in English, else its id.
    pub fn get(self, lang: Lang) -> &'static str {
        let at = self as usize;
        words(lang)[at].as_deref().or_else(|| words(Lang::EnUs)[at].as_deref()).unwrap_or(self.id())
    }

    /// The string with `{0}`, `{1}`, ... replaced by `args`. A placeholder without an argument stays as
    /// written, so a mistake shows up in the interface instead of vanishing.
    pub fn format(self, lang: Lang, args: &[&dyn Display]) -> String {
        let template = self.get(lang);
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            let placeholder = tail
                .find('}')
                .and_then(|end| Some((tail[1..end].parse::<usize>().ok()?, end)));
            match placeholder {
                Some((index, end)) if index < args.len() => {
                    // Writing into a String cannot fail.
                    let _ = write!(out, "{}", args[index]);
                    rest = &tail[end + 1..];
                }
                _ => {
                    out.push('{');
                    rest = &tail[1..];
                }
            }
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::commands::Command;

    /// Indexes `n` of every `{n}` in the template.
    fn placeholders(template: &str) -> BTreeSet<usize> {
        let mut found = BTreeSet::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            let tail = &rest[start + 1..];
            if let Some(end) = tail.find('}')
                && let Ok(index) = tail[..end].parse()
            {
                found.insert(index);
            }
            rest = tail;
        }
        found
    }

    fn all_strings() -> Vec<(Text, Lang, &'static str)> {
        Text::all()
            .flat_map(|text| Lang::ALL.map(|lang| (text, lang, text.get(lang))))
            .collect()
    }

    #[test]
    fn every_variant_sits_at_its_own_row_and_ids_are_unique() {
        let mut ids = BTreeSet::new();
        for (index, row) in TABLE.iter().enumerate() {
            assert_eq!(row.0 as usize, index, "{}", row.1);
            assert!(ids.insert(row.1), "duplicate id {}", row.1);
            assert_eq!(Text::from_id(row.1), Some(row.0));
        }
        assert_eq!(Text::from_id("no.such.id"), None);
    }

    #[test]
    fn every_language_file_reads_and_names_only_strings_there_are_and_english_and_chinese_have_them_all() {
        for lang in Lang::ALL {
            let map: BTreeMap<String, String> = serde_json::from_str(file(lang)).unwrap_or_else(|e| panic!("lang/{}.json: {e}", lang.tag()));
            for id in map.keys() {
                assert!(Text::from_id(id).is_some(), "lang/{}.json names {id}, which is no string", lang.tag());
            }
        }
        for lang in [Lang::EnUs, Lang::ZhTw] {
            let missing: Vec<&str> = TABLE.iter().filter(|(text, _)| words(lang)[*text as usize].is_none()).map(|(_, id)| *id).collect();
            assert!(missing.is_empty(), "lang/{}.json has no {missing:?}", lang.tag());
        }
    }

    #[test]
    fn no_string_is_empty_or_padded_in_either_language() {
        for (text, lang, value) in all_strings() {
            assert!(!value.is_empty(), "{} in {lang:?}", text.id());
            assert_eq!(value, value.trim(), "{} in {lang:?}", text.id());
        }
    }

    #[test]
    fn both_languages_take_the_same_arguments_and_none_is_left_unfilled() {
        for text in Text::all() {
            let zh = placeholders(text.get(Lang::ZhTw));
            let en = placeholders(text.get(Lang::EnUs));
            assert_eq!(zh, en, "{}", text.id());
            assert_eq!(
                zh.iter().copied().collect::<Vec<_>>(),
                (0..zh.len()).collect::<Vec<_>>(),
                "{} must number its arguments from 0",
                text.id()
            );
            let dummies: Vec<String> = (0..zh.len()).map(|n| format!("<{n}>")).collect();
            let args: Vec<&dyn Display> = dummies.iter().map(|d| d as &dyn Display).collect();
            for lang in Lang::ALL {
                let filled = text.format(lang, &args);
                assert!(
                    !filled.contains('{') && !filled.contains('}'),
                    "{} in {lang:?}: {filled}",
                    text.id()
                );
                for dummy in &dummies {
                    assert!(
                        filled.contains(dummy.as_str()),
                        "{} in {lang:?} drops {dummy}",
                        text.id()
                    );
                }
            }
        }
    }

    #[test]
    fn formatting_fills_placeholders_and_keeps_a_missing_one_visible() {
        assert_eq!(
            Text::StatusSpawnFailed.format(Lang::ZhTw, &[&"PowerShell", &"拒絕存取"]),
            "無法啟動 PowerShell：拒絕存取"
        );
        assert_eq!(Text::FindCount.format(Lang::EnUs, &[&2, &17]), "2 / 17");
        assert_eq!(
            Text::StatusExited.format(Lang::EnUs, &[&-1]),
            "[Exited with code -1]"
        );
        assert_eq!(
            Text::StatusSpawnFailed.format(Lang::EnUs, &[&"cmd"]),
            "Cannot start cmd: {1}"
        );
        assert_eq!(Text::TabNew.format(Lang::EnUs, &[&"unused"]), "New tab");
        // An argument that itself contains braces is inserted as is, never re-read as a placeholder.
        assert_eq!(
            Text::PaletteNewTabWith.format(Lang::EnUs, &[&"{0}"]),
            "New tab: {0}"
        );
    }

    #[test]
    fn text_is_unique_within_a_language_except_for_documented_homonyms() {
        // The same words in two places are the same action (one id, reused) or a different role that
        // happens to share a noun. These are the second kind.
        let homonyms: [&[Text]; 3] = [
            &[Text::TabUntitled, Text::SettingsTerminal],
            &[Text::TabProfiles, Text::SettingsProfiles],
            // English calls a pull request that is not merged or closed open, as the button that opens something.
            &[Text::PrStateOpen, Text::CommonOpen],
        ];
        for lang in Lang::ALL {
            let mut by_text: BTreeMap<&str, Vec<Text>> = BTreeMap::new();
            for text in Text::all() {
                by_text.entry(text.get(lang)).or_default().push(text);
            }
            for (value, owners) in by_text.into_iter().filter(|(_, owners)| owners.len() > 1) {
                assert!(
                    homonyms.contains(&owners.as_slice()),
                    "{lang:?}: {value:?} is the text of {:?}",
                    owners.iter().map(|t| t.id()).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn a_command_label_is_the_row_named_by_the_command_id() {
        for command in Command::ALL {
            let by_id = Text::from_id(command.id());
            match command {
                // Numbered tabs share one template.
                Command::TabGoto(_) => assert_eq!(by_id, None),
                _ => assert_eq!(by_id, Some(command.text()), "{}", command.id()),
            }
        }
        assert_eq!(Text::TabGoto.id(), "tab.goto");
        // No other row borrows a command id.
        for text in Text::all() {
            if let Some(command) = Command::from_id(text.id()) {
                assert_eq!(command.text(), text, "{}", text.id());
            }
        }
    }

    #[test]
    fn the_same_action_keeps_the_same_label_everywhere() {
        for lang in Lang::ALL {
            let new_tab = Text::TabNew.get(lang);
            assert!(
                Text::PaletteNewTabWith.get(lang).starts_with(new_tab),
                "{lang:?}: the profile entry must start with the New tab label"
            );
            assert!(
                Text::DropPastePath
                    .get(lang)
                    .starts_with(Text::EditPaste.get(lang)),
                "{lang:?}: dropping a path pastes it"
            );
            // "Go to tab 3" and "Go to tab: build" are one action named by a number or by a title.
            let stem = Text::TabGoto.get(lang).trim_end_matches("{0}").trim_end();
            assert!(
                Text::PaletteGotoTab.get(lang).starts_with(stem),
                "{lang:?}: the entry of a tab must start with the Go to tab label"
            );
        }
    }

    #[test]
    fn language_tags_round_trip_and_system_locales_map_to_a_language() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_system_tag(lang.tag()), lang);
        }
        for tag in [
            "zh-Hant-TW",
            "zh-TW",
            "zh_TW",
            "zh-HK",
            "zh-CN",
            "ZH-tw",
            "zh",
        ] {
            assert_eq!(Lang::from_system_tag(tag), Lang::ZhTw, "{tag}");
        }
        for tag in ["en-US", "en-GB", "ja-JP", "de-DE", "", "zhx"] {
            assert_eq!(Lang::from_system_tag(tag), Lang::EnUs, "{tag}");
        }
    }

    /// Characters that exist only in Simplified Chinese (their Traditional forms differ).
    const SIMPLIFIED_ONLY: &str = "设选终标签认删复贴关开启缓冲滚动颜编码显键运进话连网数据应对确错误历记帐录载导击销缩项属许语简这为无与时间内区页机级线处务种环变发现图当档阅读写输释译验证测试状态还实联络绑绝绘统续维紧约红绿蓝个们单户择";

    /// Mainland wording that Taiwan does not use (with the Taiwan form in the comment).
    const MAINLAND_TERMS: &[&str] = &[
        "匹配",   // 符合
        "當前",   // 目前
        "組件",   // 元件
        "緩存",   // 快取
        "默認",   // 預設
        "文件夾", // 資料夾
        "軟件",   // 軟體
        "信息",   // 資訊
        "設置",   // 設定
        "界面",   // 介面
        "鼠標",   // 滑鼠
        "視頻",   // 影片
        "網絡",   // 網路
        "服務器", // 伺服器
        "數據",   // 資料
        "打印",   // 列印
        "文檔",   // 文件
        "內存",   // 記憶體
        "磁盤",   // 磁碟
        "支持",   // 支援
        "激活",   // 啟用
        "優化",   // 最佳化
        "運行",   // 執行
        "窗口",   // 視窗
        "菜單",   // 功能表
        "快捷鍵", // 快速鍵
        "字體",   // 字型
        "文本",   // 文字
        "保存",   // 儲存
        "刷新",   // 重新整理
        "退出",   // 結束
        "重啟",   // 重新啟動
        "標籤頁", // 分頁
        "插件",   // 外掛程式
        "用戶",   // 使用者
    ];

    #[test]
    fn chinese_text_is_traditional_and_uses_taiwan_wording() {
        for text in Text::all() {
            let zh = text.get(Lang::ZhTw);
            if let Some(found) = zh.chars().find(|c| SIMPLIFIED_ONLY.contains(*c)) {
                panic!(
                    "{}: {zh:?} contains the Simplified-only character {found}",
                    text.id()
                );
            }
            for term in MAINLAND_TERMS {
                assert!(!zh.contains(term), "{}: {zh:?} uses {term}", text.id());
            }
        }
    }

    #[test]
    fn the_simplified_character_list_names_no_traditional_character() {
        // A Traditional string full of the Taiwan wording the shell uses must pass the check above.
        let sample = "設定 選項 終端機 標籤 視窗 預設 刪除 複製 貼上 搜尋 關閉 開啟 緩衝區 捲動 顏色 編碼 字型 顯示 鍵盤 啟動 執行 工作階段 連線 資料 應用 對話 確認 錯誤 歷史 記錄 帳戶 載入 儲存 導入 擊 縮放 項目 屬性 許可 語言 簡體 這 為 無 與 時間 內容 區 頁 機 級 處 務 種 環境 變數 發現 圖 當 檔案 閱讀 寫入 輸出 釋放 驗證 測試 狀態 還原 實際 聯絡 統一 繼續 維持 緊急 約定 紅綠藍 個 們 單 戶 擇";
        assert_eq!(sample.chars().find(|c| SIMPLIFIED_ONLY.contains(*c)), None);
        assert!(MAINLAND_TERMS.iter().all(|term| !sample.contains(term)));
    }

    #[test]
    fn english_text_is_english_apart_from_the_language_names() {
        for text in Text::all() {
            let en = text.get(Lang::EnUs);
            if text == Text::LanguageZhTw {
                continue;
            }
            assert!(
                en.chars().all(|c| !('\u{2E80}'..='\u{9FFF}').contains(&c)
                    && !('\u{FF00}'..='\u{FFEF}').contains(&c)),
                "{}: {en:?}",
                text.id()
            );
        }
    }

    #[test]
    fn language_names_are_written_in_their_own_language() {
        for lang in Lang::ALL {
            assert_eq!(Text::LanguageZhTw.get(lang), "繁體中文");
            assert_eq!(Text::LanguageEnUs.get(lang), "English");
        }
    }

    #[test]
    fn routine_text_has_no_exclamations_emoji_or_voice() {
        for (text, lang, value) in all_strings() {
            let id = text.id();
            assert!(
                !value.contains('!') && !value.contains('！'),
                "{id} in {lang:?}: {value:?}"
            );
            assert!(
                !value
                    .chars()
                    .any(|c| ('\u{1F000}'..='\u{1FFFF}').contains(&c)
                        || ('\u{2600}'..='\u{27BF}').contains(&c)),
                "{id} in {lang:?} has an emoji"
            );
            assert!(
                !value.contains("我們")
                    && !value.contains("你的")
                    && !value.to_lowercase().starts_with("we ")
                    && !value.to_lowercase().contains(" your "),
                "{id} in {lang:?}: {value:?}"
            );
            assert!(
                !value.ends_with('。') && !value.ends_with('.'),
                "{id} in {lang:?}: labels carry no full stop"
            );
        }
    }

    #[test]
    fn only_confirmations_are_questions() {
        for (text, lang, value) in all_strings() {
            let question = value.contains('？') || value.contains('?');
            assert_eq!(
                question,
                text.id().starts_with("dialog."),
                "{} in {lang:?}: {value:?}",
                text.id()
            );
        }
    }
}
