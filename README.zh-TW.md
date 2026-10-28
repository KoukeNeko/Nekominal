<p align="center">
  <img src="assets/banner.png" alt="Nekominal：一個輕量的 Windows 終端機">
</p>

<p align="center">
  <strong>一個輕量的 Windows 終端機。</strong><br>
  原生 Rust 與 Direct3D，內部沒有網頁執行環境；每個分頁都會告訴你哪個 agent 在等你。
</p>

<p align="center">
  <a href="https://github.com/KoukeNeko/Nekominal/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/KoukeNeko/Nekominal?style=for-the-badge&logo=github&label=RELEASE&color=2196F3"></a>
  <a href="https://github.com/KoukeNeko/Nekominal/actions/workflows/ci.yml"><img alt="CI 狀態" src="https://img.shields.io/github/actions/workflow/status/KoukeNeko/Nekominal/ci.yml?branch=master&style=for-the-badge&logo=githubactions&logoColor=white&label=CI"></a>
  <img alt="Windows 11，x64 與 ARM64" src="https://img.shields.io/badge/WINDOWS_11-x64_%7C_ARM64-0078D4?style=for-the-badge&logo=windows11&logoColor=white">
  <a href="LICENSE"><img alt="MIT licence" src="https://img.shields.io/badge/LICENSE-MIT-4CAF50?style=for-the-badge&logo=github"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <strong>繁體中文</strong>
</p>

## 安裝

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal
```

也可以從 [Releases](https://github.com/KoukeNeko/Nekominal/releases) 下載 zip，解壓後執行 `Nekominal.exe`。
支援 Windows 11，x64 或 ARM64。

## 功能

- **不佔資源。** 單一原生程式：Rust、Direct3D 11 與 ConPTY，沒有 Electron 或 web view。視窗只在畫面有變化時才重繪。
- **每個分頁都有 agent 狀態。** 工作中、等你回應、已完成或失敗。Ctrl+Shift+U 跳到下一個等你的分頁。
- **標題列顯示 Claude Code 狀態。** 模型、context 用量、費用與時間；滑鼠移到計量條上可看詳細資訊。
- **也支援 Codex、Gemini CLI 與 Aider**，不用另外設定。
- **分頁側邊欄**，可依狀態或 repository 分組，並顯示每個分頁的 pull request 與開啟的 port。
- 從命令選擇區**在新的 git worktree 開分頁**，讓每個 agent 有自己的檔案。
- **更新時分頁不中斷。** 按「更新並重新啟動」，每個 shell 與 agent 都會在新版本中繼續執行。
- 分割窗格、拖出分頁、下拉視窗（Win+`）、儲存版面與還原工作階段。
- 網址、路徑與 hash 的提示模式；在編輯器開啟 `file:line`；搜尋所有分頁。
- 內嵌圖片（Sixel、kitty）、字型連字、Mica 與壓克力背景，可匯入 Windows Terminal 與 iTerm2 的配色。
- 透過 ntfy 或 webhook 傳送手機通知，並提供 `nekominal cli` 給腳本使用。

![Nekominal 側邊欄依狀態分組的五個 agent 分頁：等待中、執行中、已完成](assets/screenshots/agents.png)

| Agent 卡片 | 依 repository 分組 |
| :---: | :---: |
| ![每個分頁一張卡片，顯示狀態、資料夾與分支](assets/screenshots/cards.png) | ![分頁依所在的 repository 分組並顯示狀態](assets/screenshots/repos.png) |

## 快速鍵

| 動作 | 快速鍵 |
| --- | --- |
| 新分頁 · 選擇設定檔 | Ctrl+Shift+T · Ctrl+Shift+N |
| 下一個等你的分頁 · agent 面板 | Ctrl+Shift+U · Ctrl+Shift+J |
| 向右分割 · 向下分割 | Alt+Shift+D · Alt+Shift+S |
| 命令選擇區 · 設定 | Ctrl+Shift+P · Ctrl+Shift+O |
| 尋找 · 搜尋所有分頁 | Ctrl+Shift+F · Ctrl+Shift+G |
| 提示模式 | Ctrl+Shift+E |

所有命令都能在命令選擇區找到，所有快速鍵都可以在設定中修改。
設定檔位於 `%APPDATA%\Nekominal\settings.json`。

## 建置

```powershell
scripts\fetch-conpty.ps1
cargo build --release -p nekominal-shell
cargo test --workspace
```

需要 Rust 1.95+ 與 Visual Studio Build Tools。[版本說明](distribution/release-notes)。

## 支持

<a href="https://buymeacoffee.com/doershing"><img alt="Buy Me a Coffee" src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-doershing-FFDD00?style=for-the-badge&logo=buymeacoffee&logoColor=black"></a>

## 授權

[MIT](LICENSE) © KoukeNeko。發行檔中的 ConPTY 檔案屬於 Microsoft，同樣採用 MIT 授權；請見
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
