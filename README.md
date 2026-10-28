<p align="center">
  <img src="assets/banner.png" alt="Nekominal: a Windows terminal that stays light">
</p>

<p align="center">
  <strong>A Windows terminal that stays light.</strong><br>
  Native Rust and Direct3D, no web runtime inside, and every tab tells you when an agent needs you.
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

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal
```

<p align="center">
  Or take the <a href="https://github.com/KoukeNeko/Nekominal/releases/latest">installer (MSI) or the zip</a> of the latest release: the installer needs no administrator. Windows 11, x64 or ARM64.<br>
  <a href="https://koukeneko.github.io/Nekominal/">koukeneko.github.io/Nekominal</a>
</p>

## Light by design

Nekominal.exe is about 9 MB, a native program in Rust and Direct3D 11; the two ConPTY files beside it bring the folder
to about 10 MB. There is no Electron, no WebView and no runtime to install. A window draws only when something on it
changes, and the cursor stops blinking a few seconds after the last key, so a terminal you are not looking at costs
next to nothing.

|  | Nekominal | Windows Terminal | Warp |
| --- | ---: | ---: | ---: |
| Memory, one tab | **78 MB** | 114 MB | 554 MB |
| Memory, five tabs | **135 MB** | 192 MB | 640 MB |
| Launch until the shell is ready | **1.38 s** | 1.50 s | 2.48 s |
| CPU at rest, one tab, per 20 s | **0.031 s** | 0.078 s | 0.25 s |
| Installed size | **10 MB** | | 398 MB |

<sub>Measured on one Windows 11 machine, in the same way for the three: default settings, a cold start, running `pwsh`,
the median of five runs. Memory is the working set of the terminal and its OpenConsole, the shells left out. Ready means
the shell has finished starting and waits for input; CPU at rest is the processor time over 20 s with one tab open and
nobody touching it. Nekominal 0.2.15 and Windows Terminal 1.24 were measured in one session, Warp 0.2026.10.14 in
another, with its default settings except that restoring windows at start is off.</sub>

## Knows what your agents are doing

Run Claude Code, Codex, Gemini CLI or Aider in a tab and the tab says whether it is working, waiting for you, done
or failed. The sidebar groups the tabs by state, the title bar counts the ones that wait, and Ctrl+Shift+U takes you to
the next of them. There is nothing to set up.

![Nekominal's sidebar with five agent tabs grouped by state: waiting, running and done](assets/screenshots/agents.png)

For Claude Code, the title bar also shows the model, the share of the context used, the cost and the time. Hover the
meter for the details, and for the session's Remote Control link when it has one.

| Agent cards | By repository |
| :---: | :---: |
| ![Each tab as a card with its state, folder and branch](assets/screenshots/cards.png) | ![The tabs under the repository they work in, with their state](assets/screenshots/repos.png) |

## Made for parallel work

- Give each agent its own git worktree: the command palette opens a tab in a new one, and removing it first shows what
  would be lost.
- Update and restart, and every shell and agent carries on in the new version.
- Tabs show their pull request and the ports they listen on, and an agent that needs you can reach your phone through
  ntfy or a webhook.
- `nekominal cli` lists, opens, splits and reads panes, so your own tools can drive it.

## A terminal first

Split panes, tear-off tabs, a drop-down window on Win+\`, saved layouts and session restore. Hints for URLs, paths and
hashes, `file:line` opened in your editor, search across all tabs. Sixel and kitty images, font ligatures, Mica and
acrylic, and colour schemes imported from Windows Terminal and iTerm2.

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
