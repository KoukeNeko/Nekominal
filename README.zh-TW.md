<p align="center">
  <img src="assets/nekominal.svg" width="128" alt="Nekominal">
</p>

<h1 align="center">Nekominal</h1>

<p align="center">
  <strong>為 coding agent 打造的 Windows 終端機。</strong><br>
  原生 Rust、Direct3D 與 ConPTY，一眼看出哪些分頁正在工作、哪些在等你。
</p>

<p align="center">
  <a href="https://github.com/KoukeNeko/Nekominal/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/KoukeNeko/Nekominal?style=for-the-badge&logo=github&label=RELEASE&color=2196F3"></a>
  <a href="https://github.com/KoukeNeko/Nekominal/actions/workflows/ci.yml"><img alt="CI 狀態" src="https://img.shields.io/github/actions/workflow/status/KoukeNeko/Nekominal/ci.yml?branch=master&style=for-the-badge&logo=githubactions&logoColor=white&label=CI"></a>
  <img alt="Windows 11，x64 與 ARM64" src="https://img.shields.io/badge/WINDOWS_11-x64_%7C_ARM64-0078D4?style=for-the-badge&logo=windows11&logoColor=white">
  <img alt="以 Rust 撰寫" src="https://img.shields.io/badge/RUST-1.95+-DEA584?style=for-the-badge&logo=rust&logoColor=black">
  <a href="LICENSE"><img alt="MIT licence" src="https://img.shields.io/badge/LICENSE-MIT-4CAF50?style=for-the-badge&logo=github"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <strong>繁體中文</strong>
</p>

<p align="center">
  <a href="#開始使用">開始使用</a>
  · <a href="#功能">功能</a>
  · <a href="#快速鍵">快速鍵</a>
  · <a href="distribution/release-notes">版本說明</a>
  · <a href="#技術參考">技術參考</a>
</p>

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal

nekominal -d D:\src\app                            # 在該資料夾開一個分頁
nekominal -d D:\src\app `; sp -V -p "Ubuntu"         # 旁邊再開一個 WSL 窗格
```

Nekominal 是以 Rust 撰寫的 Windows 終端機，完全不用網頁技術：一般的 Win32 視窗、以 Direct3D 11 繪製文字與介面、
DirectComposition 疊在 Mica 或壓克力背景上，再以 ConPTY 執行 shell。Claude Code 可以在裡面執行，包含它的全螢幕模式。

**它知道你的 agent 在做什麼。** 每個分頁都會顯示它的 agent 是在工作、等你回答、已完成還是出錯。側邊欄可以依這些狀態分組，
標題列會計算有幾個分頁在等你，一個鍵就能跳到下一個。對 Claude Code，標題列還會顯示模型、已使用的 context、費用與時間，
旁邊是分支與資料夾。

**它不信任程式印出的內容。** 程式印出的連結無法透過 Ctrl+點選執行程式；大量的控制碼或組合字元不會讓視窗凍結；程式只能依設定寫入剪貼簿。
貼上大量或多行文字前會先詢問。

## 功能

### 為 agent 而做

- **看得見的狀態。** 工作中的分頁會轉圈、等你的分頁會閃動、失敗的會變紅。把分頁放在側邊欄並依狀態分組：
  **等待回應**、**工作中**、**已完成**、**其他**。Ctrl+Shift+U 跳到下一個等你的分頁。
- **標題列上的 Claude Code。** 在你沒有自己的 status line 時，Nekominal 會交給 Claude Code 一個自己的 status line，
  並顯示模型、context 用量、費用與時間，還有分支與資料夾。
- 任何程式都能用 **agent 狀態協定（OSC 7501）**、進度（OSC 9;4）與通知（OSC 9、OSC 777）回報，顯示在它的分頁上，
  視窗在背景時也會提醒工作列。
- **在新的 git 工作樹開啟分頁。** 從命令選擇區輸入分支名稱，Nekominal 會在儲存庫旁邊執行 `git worktree add`
  （`app` 的分支 `fix/login` 放在 `app-fix-login`），並在那裡開一個以分支命名的分頁，讓每個 agent 各有自己的檔案。

### 指令與它們的輸出

- **PowerShell、cmd、Git Bash 與 WSL bash 的 shell integration** 會標記每個提示字元、指令與結束代碼（OSC 133）以及目前資料夾（OSC 9;9），
  不必修改你的 profile。
- 從命令選擇區 **選取或複製指令的輸出**；再按一次會換成前一個指令。失敗的指令在捲軸上有紅色標記，
  Ctrl+Alt+Up／Down 在提示字元之間跳動。
- **提示選取（Ctrl+Shift+E）。** 畫面上的每個網址、檔案路徑（含 `:行:欄`）與 git 雜湊都會有一個字母標籤，
  輸入它就複製，Shift+字母則開啟。
- **鍵盤選取（Ctrl+Shift+M）**、Alt+拖曳區塊選取、含格式複製（HTML）、把整個窗格匯出成檔案、支援大小寫與規則運算式的搜尋。
- **建議（Ctrl+Shift+H）**，來自你的片段與 PowerShell 的歷史紀錄。

### 分頁、窗格與視窗

- 分頁可放在標題列或側邊欄，第一次啟動時選擇。可重新命名、上色、複製、移動、關閉其他分頁。
- 分割窗格、放大其中一個、互換、把一個移到新分頁，或同時輸入到所有窗格。
- 把分頁拖出視窗就成為新視窗，也可以拖到另一個視窗。
- **Win+`** 的下拉視窗、專注模式、視窗置頂、全螢幕。
- 需要的話，下次啟動時分頁、窗格與它們的資料夾都會回來。
- 把視窗的分頁存成有名字的配置，之後從命令選擇區在這個視窗或新視窗開啟，或用 `nekominal --layout <名稱>` 開啟。

### 設定檔

- 自動找到：PowerShell 7 與 Windows PowerShell、cmd、Git Bash、每個 WSL 發行版、Visual Studio 開發人員 shell，
  以及 `~/.ssh/config` 裡的主機。
- 每個設定檔都可以有自己的環境變數、色彩配置、字型大小、起始資料夾與 **以系統管理員身分執行**。設定裡有一頁可以編輯，
  程式與資料夾都能用檔案選擇器挑選。
- 新分頁使用 Windows 為新程式準備的環境，與 Windows Terminal 相同，所以一分鐘前安裝的工具已經在 `PATH` 上。

### 外觀

- Mica、Mica Alt、壓克力或無，透明度可自訂。
- 內建色彩配置、Windows Terminal 格式的自訂配色、從 Windows Terminal 匯入配色，以及匯入 iTerm2 色彩配置檔（`.itermcolors`，
  例如 iTerm2-Color-Schemes 收錄的配色）。
- 自選字型、行高、游標樣式與閃爍速度。文字在每種配色上都保有對比。

### 融入 Windows

- 檔案總管右鍵選單的 **在 Nekominal 開啟**，可選「在這裡開啟」或「以系統管理員身分開啟」。
- 工作列按鈕的捷徑清單列出設定檔。
- 與 `wt` 相似的命令列：`-d`、`-p`、`nt`、`sp -H/-V`、`--` 之後的指令，以及用 `;` 分隔的多個動作。
- 從命令選擇區或「關於」頁 **更新並重新啟動**，透過 Scoop 完成。
- 螢幕閱讀器可以讀到分頁、文字、設定與選單（UI Automation）。輸入法組字直接畫在游標處。繁體中文與英文介面。

### 比較

|                                   | Nekominal | Windows Terminal | WezTerm |
| --------------------------------- | :-------: | :--------------: | :-----: |
| 每個分頁的 agent 狀態              |    ✅     |        —         |    —    |
| 視窗中的 Claude Code 狀態          |    ✅     |        —         |    —    |
| 依狀態分組分頁                     |    ✅     |        —         |    —    |
| 選取指令輸出                       |    ✅     |        ✅        |   ✅    |
| 提示選取／quick select             |    ✅     |        —         |   ✅    |
| 新分頁使用全新的環境變數           |    ✅     |        ✅        |    —    |
| 檔案總管選單、捷徑清單             |    ✅     |        ✅        |   ⚠️    |
| 自訂色彩配置                       |    ✅     |        ✅        |   ✅    |
| 內嵌圖片（Sixel、kitty）           |    —      |        ⚠️        |   ✅    |
| 字型連字                           |    —      |        ✅        |   ✅    |
| Windows 預設終端機                 |    —      |        ✅        |    —    |
| macOS 與 Linux                     |    —      |        —         |   ✅    |

⚠️ 部分支援：Windows Terminal 支援 Sixel 但不支援 kitty 協定；WezTerm 的安裝程式會加入檔案總管選單，但沒有捷徑清單。

## 開始使用

1. **以 [Scoop](https://scoop.sh) 安裝：**

   ```powershell
   scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
   scoop install koukeneko/nekominal
   ```

   Scoop 會自動選擇 x64 或 ARM64 版本。每個[版本](https://github.com/KoukeNeko/Nekominal/releases)也有 zip 檔：
   解壓到任何地方後執行 `Nekominal.exe`，並讓 `conpty.dll` 與 `OpenConsole.exe` 留在旁邊。

2. **選擇分頁的位置。** 第一次啟動時會詢問：放在標題列、側邊欄清單，或側邊欄圖示。之後可在「設定 › 外觀」更改。

3. **執行 agent。** 在 PowerShell 分頁輸入 `claude`。它的狀態會顯示在分頁上，模型、context 與費用則顯示在標題列。

4. **調成你要的樣子。** Ctrl+, 開啟設定：設定檔、配色、字型、快速鍵。所有設定也都在
   `%APPDATA%\Nekominal\settings.json`，設定視窗可以直接幫你開啟它。

5. **從任何地方啟動。** 在檔案總管對資料夾按右鍵、從工作列按鈕選設定檔，或寫成指令：

   ```powershell
   nekominal -p cmd -d D:\src\app
   ```

## 快速鍵

| 動作 | 快速鍵 |
| --- | --- |
| 新增分頁 · 選擇設定檔 | Ctrl+Shift+T · Ctrl+Shift+N |
| 下一個分頁 · 第 1–9 個分頁 · 左／右側分頁 | Ctrl+Tab · Ctrl+Shift+1–9 · Ctrl+PageUp／PageDown |
| 下一個等你的分頁 | Ctrl+Shift+U |
| 向右分割 · 向下分割 | Alt+Shift+D · Alt+Shift+S |
| 在窗格間移動 · 調整大小 | Alt+方向鍵 · Alt+Shift+方向鍵 |
| 放大窗格 · 關閉窗格 | Ctrl+Shift+Z · Ctrl+Shift+W |
| 提示選取 · 鍵盤選取 · 建議 | Ctrl+Shift+E · Ctrl+Shift+M · Ctrl+Shift+H |
| 上一個／下一個提示字元 | Ctrl+Alt+Up／Down |
| 搜尋 · 命令選擇區 · 設定 | Ctrl+Shift+F · Ctrl+Shift+P · Ctrl+, |
| 字型大小 | Ctrl+= · Ctrl+- · Ctrl+0 |
| 側邊欄 · 全螢幕 · 下拉視窗 | Ctrl+Shift+B · F11 · Win+` |

所有命令都在命令選擇區裡，設定的「快速鍵」頁可以直接按下新的組合鍵來重新指定。

## 相容性

- Windows 11（背景效果需要 build 22621 以上），x64 與 ARM64。
- Shell 與 agent 透過 ConPTY 執行，使用每個版本隨附的 ConPTY。
- PowerShell 7、Windows PowerShell 5.1、cmd、Git Bash 與 WSL 的 bash 會套用 shell integration（cmd 沒有結束代碼）；
  其他 shell 照原樣執行。

---

## 技術參考

### 設定

`%APPDATA%\Nekominal\settings.json` 只保存與預設值不同的部分。檔案裡可以有註解；讀不懂的值會被回報，不會被刪除，
你最早寫的檔案會保留為 `settings.json.orig`。

```jsonc
{
  "theme": "Tokyo Night",
  "profiles": [
    { "id": "pwsh", "cwd": "D:\\src", "env": { "LANG": "zh_TW.UTF-8" } },
    { "id": "admin", "name": "Admin PowerShell", "program": "pwsh.exe", "elevate": true, "colorScheme": "Nord" }
  ],
  "schemes": [
    { "name": "Mine", "background": "#1E1E2E", "foreground": "#CDD6F4", "black": "#45475A", "red": "#F38BA8", "...": "..." }
  ]
}
```

`id` 與自動找到的 shell 相同的項目會調整該 shell；有 `program` 的項目會新增設定檔。`schemes` 使用 Windows Terminal 的鍵名，
從那裡複製的配色可以直接使用。

### 命令列

```text
nekominal [-d <資料夾>] [-p <設定檔>] [-- <指令> ...]
          [; nt <相同選項>] [; sp -H|-V <相同選項>]
```

`-p` 可用設定檔的 id 或名稱。在 PowerShell 裡，`;` 要寫成 `` `; ``，避免被 PowerShell 吃掉。`--` 之後的指令會直接執行，
不經過 shell integration；要讓 Claude Code 取得 status line，請在 PowerShell、cmd、Git Bash 或 WSL bash 分頁輸入 `claude`。

### 建置

Rust 1.95 以上、Visual Studio Build Tools（建置 ARM64 需要 ARM64 工具）、Windows 11。

```powershell
scripts\fetch-conpty.ps1                 # ConPTY 檔案，會比對固定的雜湊與簽章
cargo build --release -p nekominal-shell # target\release\nekominal-next.exe
scripts\package.ps1                      # dist\Nekominal-<版本>-windows-x64.zip
scripts\package.ps1 -Arch arm64          # dist\Nekominal-<版本>-windows-arm64.zip
```

### 測試

```powershell
cargo test --workspace                   # 快速：不開視窗、不連網路
cargo test --workspace -- --ignored      # 另外執行真正的 ConPTY 工作階段、PowerShell 與 Node
```

`tools\slice-drive\next-scenarios.ps1` 會以真正的鍵盤與滑鼠輸入操作建置好的視窗。這些情境會搶走前景，執行時請不要操作電腦。
執行真正 Claude Code CLI 的測試在 `tools\agent-probe`，需要已登入的 `claude.exe`。

### 結構

| Crate | 內容 |
| --- | --- |
| `nekominal-pty` | ConPTY 主機與工作階段、尋找 shell、新分頁的環境變數 |
| `nekominal-term` | 終端機模擬器（包裝 `alacritty_terminal`）、連結、搜尋、OSC 處理 |
| `nekominal-input` | 鍵盤與滑鼠編碼，包含 kitty 鍵盤協定 |
| `nekominal-model` | 設定、設定檔、命令、按鍵對應、命令選擇區、配色、字串 |
| `nekominal-render` | Direct3D 11 格狀繪製器 |
| `nekominal-ui` | 自繪介面：圓角矩形、文字、圖示 |
| `nekominal-pane` | 窗格：一個工作階段、一個模擬器、一個繪製器、提示選取 |
| `nekominal-shell` | 應用程式本身：視窗、分頁、側邊欄、彈出視窗、設定、Windows 整合、更新 |

### 發布

版本號是 `Cargo.toml` 裡 `[workspace.package]` 的 `version`。在 `distribution/release-notes/vX.Y.Z.md` 寫好版本說明
（先英文、再繁體中文），commit 後推送 annotated tag：

```powershell
git tag -a v0.2.0 -m v0.2.0
git push origin v0.2.0
```

發布流程會建置並測試兩個壓縮檔、附上那份說明發布，再把 Scoop manifest 寫進
[KoukeNeko/scoop-bucket](https://github.com/KoukeNeko/scoop-bucket)。手動執行的流程會在發布前停下。

## 支持

如果 Nekominal 對你有幫助，歡迎支持開發：

<a href="https://buymeacoffee.com/doershing"><img alt="Buy Me a Coffee" src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-doershing-FFDD00?style=for-the-badge&logo=buymeacoffee&logoColor=black"></a>

## 授權

[MIT](LICENSE) © KoukeNeko。發布檔裡的 ConPTY 檔案來自 Microsoft，同樣採用 MIT，詳見
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
