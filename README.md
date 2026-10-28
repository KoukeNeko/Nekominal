<p align="center">
  <img src="assets/nekominal.svg" width="128" alt="Nekominal">
</p>

<h1 align="center">Nekominal</h1>

<p align="center">
  <strong>A Windows terminal made for coding agents.</strong><br>
  See at a glance which tabs are working and which wait for you.
</p>

<p align="center">
  <a href="https://github.com/KoukeNeko/Nekominal/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/KoukeNeko/Nekominal?style=for-the-badge&logo=github&label=RELEASE&color=2196F3"></a>
  <a href="https://github.com/KoukeNeko/Nekominal/actions/workflows/ci.yml"><img alt="CI status" src="https://img.shields.io/github/actions/workflow/status/KoukeNeko/Nekominal/ci.yml?branch=master&style=for-the-badge&logo=githubactions&logoColor=white&label=CI"></a>
  <img alt="Windows 11, x64 and ARM64" src="https://img.shields.io/badge/WINDOWS_11-x64_%7C_ARM64-0078D4?style=for-the-badge&logo=windows11&logoColor=white">
  <a href="LICENSE"><img alt="MIT licence" src="https://img.shields.io/badge/LICENSE-MIT-4CAF50?style=for-the-badge&logo=github"></a>
</p>

<p align="center">
  <strong>English</strong> · <a href="README.zh-TW.md">繁體中文</a>
</p>

## Install

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal
```

Or download a zip from [Releases](https://github.com/KoukeNeko/Nekominal/releases), unpack it and run `Nekominal.exe`.
Windows 11, x64 or ARM64.

## What it does

- **Agent state on every tab.** Working, waiting for you, done or failed. Ctrl+Shift+U jumps to the next tab that waits.
- **Claude Code in the title bar.** The model, the context used, the cost and the time. Hover the meter for the details.
- **Works with Codex, Gemini CLI and Aider too**, with nothing to set up.
- **Sidebar of tabs**, grouped by state or by repository, with each tab's pull request and open ports.
- **A tab in a new git worktree** from the command palette, so each agent has its own files.
- **Updates keep your tabs running.** Update and restart, and every shell and agent carries on in the new version.
- Split panes, tear-off tabs, a drop-down window (Win+`), saved layouts and session restore.
- Hints for URLs, paths and hashes; open `file:line` in your editor; search across all tabs.
- Inline images (Sixel, kitty), font ligatures, Mica and acrylic, colour schemes from Windows Terminal and iTerm2.
- Phone notifications through ntfy or a webhook, and a `nekominal cli` for scripts.

| Inline images and ligatures | `chafa` | `yazi` |
| :---: | :---: | :---: |
| ![Sixel and kitty pictures under a line of ligatures](assets/screenshots/images-and-ligatures.png) | ![chafa showing one picture three ways](assets/screenshots/chafa.png) | ![yazi with a picture in its preview pane](assets/screenshots/yazi-preview.png) |

## Shortcuts

| Action | Shortcut |
| --- | --- |
| New tab · choose a profile | Ctrl+Shift+T · Ctrl+Shift+N |
| Next tab that waits · agents panel | Ctrl+Shift+U · Ctrl+Shift+J |
| Split right · split down | Alt+Shift+D · Alt+Shift+S |
| Command palette · settings | Ctrl+Shift+P · Ctrl+Shift+O |
| Find · search all tabs | Ctrl+Shift+F · Ctrl+Shift+G |
| Hints | Ctrl+Shift+E |

Every command is in the palette, and any shortcut can be changed in Settings.
Settings live in `%APPDATA%\Nekominal\settings.json`.

## Build

```powershell
scripts\fetch-conpty.ps1
cargo build --release -p nekominal-shell
cargo test --workspace
```

Requires Rust 1.95+ and the Visual Studio Build Tools. [Release notes](distribution/release-notes).

## Support

<a href="https://buymeacoffee.com/doershing"><img alt="Buy Me a Coffee" src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-doershing-FFDD00?style=for-the-badge&logo=buymeacoffee&logoColor=black"></a>

## License

[MIT](LICENSE) © KoukeNeko. The ConPTY files in the releases are Microsoft's, also under MIT; see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
