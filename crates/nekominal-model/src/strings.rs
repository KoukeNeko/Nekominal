//! Every user-visible string of the shell, in Traditional Chinese (Taiwan) and English.
//!
//! One row per string holds both languages, so a language can never miss an id. A command's label is the
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
    ($($variant:ident $id:literal $zh:literal $en:literal;)*) => {
        /// A string of the interface; see the module documentation.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Text {
            $($variant,)*
        }

        /// (string, id, zh-TW, en-US), in declaration order: the one source of truth of this module.
        const TABLE: &[(Text, &str, &str, &str)] = &[
            $((Text::$variant, $id, $zh, $en),)*
        ];
    };
}

texts! {
    // Commands: the id is the command id.
    TabNew "tab.new" "新增分頁" "New tab";
    TabClose "tab.close" "關閉分頁" "Close tab";
    TabNext "tab.next" "下一個分頁" "Next tab";
    TabPrevious "tab.previous" "上一個分頁" "Previous tab";
    TabGoto "tab.goto" "切換到分頁 {0}" "Go to tab {0}";
    TabMoveLeft "tab.moveLeft" "向左移動分頁" "Move tab left";
    TabMoveRight "tab.moveRight" "向右移動分頁" "Move tab right";
    TabProfiles "tab.profiles" "設定檔" "Profiles";
    PaneSplitRight "pane.splitRight" "向右分割窗格" "Split pane right";
    PaneSplitDown "pane.splitDown" "向下分割窗格" "Split pane down";
    PaneClose "pane.close" "關閉窗格" "Close pane";
    PaneZoom "pane.zoom" "縮放窗格" "Zoom pane";
    PaneRestart "pane.restart" "重新啟動工作階段" "Restart session";
    PaneFocusLeft "pane.focusLeft" "移到左側窗格" "Focus pane on the left";
    PaneFocusRight "pane.focusRight" "移到右側窗格" "Focus pane on the right";
    PaneFocusUp "pane.focusUp" "移到上方窗格" "Focus pane above";
    PaneFocusDown "pane.focusDown" "移到下方窗格" "Focus pane below";
    PaneResizeLeft "pane.resizeLeft" "向左調整窗格大小" "Resize pane left";
    PaneResizeRight "pane.resizeRight" "向右調整窗格大小" "Resize pane right";
    PaneResizeUp "pane.resizeUp" "向上調整窗格大小" "Resize pane up";
    PaneResizeDown "pane.resizeDown" "向下調整窗格大小" "Resize pane down";
    EditCopy "edit.copy" "複製" "Copy";
    EditPaste "edit.paste" "貼上" "Paste";
    EditSelectAll "edit.selectAll" "全選" "Select all";
    EditFind "edit.find" "尋找" "Find";
    EditClear "edit.clear" "清除緩衝區" "Clear buffer";
    ScrollPageUp "scroll.pageUp" "向上捲動一頁" "Scroll up one page";
    ScrollPageDown "scroll.pageDown" "向下捲動一頁" "Scroll down one page";
    ScrollTop "scroll.top" "捲動到頂端" "Scroll to top";
    ScrollBottom "scroll.bottom" "捲動到底端" "Scroll to bottom";
    PromptPrevious "prompt.previous" "上一個提示字元" "Previous prompt";
    PromptNext "prompt.next" "下一個提示字元" "Next prompt";
    ViewFullscreen "view.fullscreen" "全螢幕" "Full screen";
    ViewZoomIn "view.zoomIn" "放大字型" "Increase font size";
    ViewZoomOut "view.zoomOut" "縮小字型" "Decrease font size";
    ViewZoomReset "view.zoomReset" "重設字型大小" "Reset font size";
    AppPalette "app.palette" "命令選擇區" "Command palette";
    AppSettings "app.settings" "設定" "Settings";
    // "設定檔" already names a profile, so the settings file is spelled out by its format.
    AppOpenSettingsFile "app.openSettingsFile" "開啟設定 JSON" "Open settings file";
    AppReloadSettings "app.reloadSettings" "重新載入設定" "Reload settings";
    AppearanceBackdropNext "appearance.backdrop.next" "切換背景材質" "Switch backdrop";
    AppearanceOpacityUp "appearance.opacity.up" "提高背景不透明度" "Increase background opacity";
    AppearanceOpacityDown "appearance.opacity.down" "降低背景不透明度" "Decrease background opacity";

    // Tab strip and menus that have no command of their own.
    TabUntitled "tab.untitled" "終端機" "Terminal";
    TabRename "tab.rename" "重新命名分頁" "Rename tab";
    TabDuplicate "tab.duplicate" "複製分頁" "Duplicate tab";
    TabCloseOthers "tab.closeOthers" "關閉其他分頁" "Close other tabs";
    TabCloseRight "tab.closeRight" "關閉右側分頁" "Close tabs to the right";
    TabMoveToNewWindow "tab.moveToNewWindow" "移至新視窗" "Move to new window";
    MenuTerminal "menu.terminal" "終端機功能表" "Terminal menu";
    DropPastePath "drop.pastePath" "貼上路徑" "Paste path";

    // Tab and pane indicators.
    AgentWorking "agent.working" "執行中" "Running";
    AgentBlocked "agent.blocked" "等待輸入" "Waiting for input";
    AgentDone "agent.done" "已完成" "Done";
    AgentError "agent.error" "錯誤" "Error";
    AgentNotification "agent.notification" "通知" "Notification";
    AgentBell "agent.bell" "鈴聲" "Bell";

    // Find bar. Its text box is labelled `EditFind`.
    FindNone "find.none" "沒有結果" "No results";
    FindCount "find.count" "{0} / {1}" "{0} / {1}";
    FindMatchCase "find.matchCase" "區分大小寫" "Match case";
    FindRegex "find.regex" "規則運算式" "Regular expression";
    FindWholeWord "find.wholeWord" "完整單字" "Whole word";
    FindPrevious "find.previous" "上一個" "Previous";
    FindNext "find.next" "下一個" "Next";

    // Command palette. A profile entry starts with the label of `TabNew`.
    PalettePlaceholder "palette.placeholder" "輸入命令" "Type a command";
    PaletteEmpty "palette.empty" "沒有符合的命令" "No matching commands";
    PaletteNewTabWith "palette.newTabWith" "新增分頁：{0}" "New tab: {0}";
    PaletteColorScheme "palette.colorScheme" "色彩配置：{0}" "Color scheme: {0}";

    // Buttons shared by dialogs. A confirmation's button names the action; Paste uses `EditPaste`.
    CommonClose "common.close" "關閉" "Close";
    CommonCancel "common.cancel" "取消" "Cancel";
    CommonRetry "common.retry" "重試" "Retry";
    CommonAuto "common.auto" "自動" "Automatic";

    // Confirmations: titles that name the action, answered by the buttons above.
    DialogPasteLines "dialog.pasteLines" "貼上 {0} 行？" "Paste {0} lines?";
    DialogPasteSize "dialog.pasteSize" "貼上 {0}？" "Paste {0}?";
    DialogCloseSessions "dialog.closeSessions" "關閉 {0} 個執行中的工作階段？" "Close running sessions ({0})?";

    // Messages: what happened, and the recovery when there is one.
    StatusExited "status.exited" "[已結束，代碼 {0}]" "[Exited with code {0}]";
    StatusSpawnFailed "status.spawnFailed" "無法啟動 {0}：{1}" "Cannot start {0}: {1}";
    StatusNoProfile "status.noProfile" "找不到可用的設定檔" "No usable profile found";
    StatusSettingsProblems "status.settingsProblems" "設定 JSON 有 {0} 個問題" "Settings file has {0} problems";
    StatusSettingsSaveFailed "status.settingsSaveFailed" "無法儲存設定：{0}" "Cannot save settings: {0}";
    StatusRestoreFailed "status.restoreFailed" "無法還原工作階段：{0}" "Cannot restore sessions: {0}";
    InfobarWidthModel "infobar.widthModel" "未使用隨附的 ConPTY，字元寬度可能不一致" "Bundled ConPTY is not in use, so character widths may not match";
    InfobarNoColor "infobar.noColor" "已設定 NO_COLOR，PowerShell 輸出不含色彩" "NO_COLOR is set, so PowerShell output has no color";

    // Settings page: sections, then one row label per setting followed by its choices.
    SettingsSearch "settings.search" "搜尋設定" "Search settings";
    SettingsEmpty "settings.empty" "沒有符合的設定" "No matching settings";
    SettingsGeneral "settings.general" "一般" "General";
    SettingsAppearance "settings.appearance" "外觀" "Appearance";
    SettingsTerminal "settings.terminal" "終端機" "Terminal";
    SettingsProfiles "settings.profiles" "設定檔" "Profiles";
    SettingsAgents "settings.agents" "代理程式" "Agents";
    SettingsKeys "settings.keys" "快速鍵" "Shortcuts";
    SettingsAbout "settings.about" "關於" "About";

    SettingsLanguage "settings.language" "語言" "Language";
    LanguageZhTw "language.zhTw" "繁體中文" "繁體中文";
    LanguageEnUs "language.enUs" "English" "English";
    SettingsRestoreSession "settings.restoreSession" "啟動時還原工作階段" "Restore sessions on start";
    SettingsConfirmClose "settings.confirmClose" "關閉前確認" "Confirm before closing";
    SettingsConfirmCloseThreshold "settings.confirmCloseThreshold" "觸發確認的工作階段數" "Sessions that trigger confirmation";

    SettingsTabBarDensity "settings.tabBarDensity" "分頁列密度" "Tab bar density";
    DensityComfortable "density.comfortable" "寬鬆" "Comfortable";
    DensityCompact "density.compact" "精簡" "Compact";
    SettingsBackdrop "settings.backdrop" "背景材質" "Backdrop";
    BackdropAcrylic "backdrop.acrylic" "壓克力" "Acrylic";
    BackdropMica "backdrop.mica" "Mica" "Mica";
    BackdropMicaAlt "backdrop.micaAlt" "Mica Alt" "Mica Alt";
    BackdropNone "backdrop.none" "無" "None";
    SettingsOpacity "settings.opacity" "背景不透明度" "Background opacity";
    SettingsTheme "settings.theme" "色彩配置" "Color scheme";
    SettingsFontFamily "settings.fontFamily" "字型" "Font";
    SettingsFontSize "settings.fontSize" "字型大小" "Font size";
    SettingsLineHeight "settings.lineHeight" "行高" "Line height";
    SettingsCursorStyle "settings.cursorStyle" "游標形狀" "Cursor shape";
    CursorBlock "cursor.block" "區塊" "Block";
    CursorBar "cursor.bar" "直線" "Bar";
    CursorUnderline "cursor.underline" "底線" "Underline";
    SettingsCursorBlink "settings.cursorBlink" "游標閃爍" "Cursor blink";

    SettingsScrollback "settings.scrollback" "捲動緩衝區行數" "Scrollback lines";
    SettingsDefaultProfile "settings.defaultProfile" "預設設定檔" "Default profile";
    SettingsCopyOnSelect "settings.copyOnSelect" "選取時複製" "Copy on select";
    SettingsRightClick "settings.rightClick" "右鍵" "Right click";
    RightClickSmart "rightClick.smart" "複製或貼上" "Copy or paste";
    RightClickMenu "rightClick.menu" "顯示功能表" "Show menu";
    SettingsWarnMultiline "settings.warnMultiline" "貼上多行前確認" "Confirm before pasting multiple lines";
    SettingsWarnLarge "settings.warnLarge" "貼上大量文字前確認（KiB）" "Confirm before pasting large text (KiB)";
    SettingsBellVisual "settings.bellVisual" "鈴聲視覺提示" "Visual bell";
    SettingsBellTaskbar "settings.bellTaskbar" "工作列提示" "Taskbar alert";
    SettingsKitty "settings.kitty" "kitty 鍵盤協定" "kitty keyboard protocol";
    SettingsAgentEnvironment "settings.agentEnvironment" "代理程式環境變數" "Agent environment variables";
    SettingsShellIntegration "settings.shellIntegration" "PowerShell 整合" "PowerShell integration";

    SettingsFile "settings.file" "設定 JSON" "Settings file";
    SettingsVersion "settings.version" "版本" "Version";
    SettingsConpty "settings.conpty" "ConPTY" "ConPTY";
    ConptyBundled "conpty.bundled" "隨附 {0}" "Bundled {0}";
    ConptySystem "conpty.system" "系統內建" "Built in";
}

impl Text {
    /// Every string, in declaration order.
    pub fn all() -> impl Iterator<Item = Text> {
        TABLE.iter().map(|row| row.0)
    }

    fn row(self) -> &'static (Text, &'static str, &'static str, &'static str) {
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

    /// The string with its `{0}` placeholders still in place.
    pub fn get(self, lang: Lang) -> &'static str {
        let row = self.row();
        match lang {
            Lang::ZhTw => row.2,
            Lang::EnUs => row.3,
        }
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
        let homonyms: [&[Text]; 2] = [
            &[Text::TabUntitled, Text::SettingsTerminal],
            &[Text::TabProfiles, Text::SettingsProfiles],
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
                // Numbered tabs share one template; Ctrl+C shares the Copy label.
                Command::TabGoto(_) => assert_eq!(by_id, None),
                Command::EditSmartCopy => assert_eq!(command.text(), Text::EditCopy),
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
