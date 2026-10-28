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
  Or <a href="https://github.com/KoukeNeko/Nekominal/releases/latest">download the zip</a> and run <code>Nekominal.exe</code>. Windows 11, x64 or ARM64.
</p>

## Light by design

Nekominal is one native program of about 9 MB: Rust, Direct3D 11 and ConPTY. There is no Electron, no web view and
no runtime to install. A window draws only when something on it changes, so a terminal you are not looking at costs
next to nothing.

|  | Nekominal | Windows Terminal |
| --- | ---: | ---: |
| Memory, one tab | **81 MB** | 114 MB |
| Memory, five tabs | **138 MB** | 193 MB |
| Launch to a running shell | **0.70 s** | 0.77 s |
| CPU at rest, one tab | 0.06 s per 20 s | 0.05 s per 20 s |
| Printing a 20 MB text file | 11.1 s | 11.1 s |

<sub>Measured on Windows 11 against Windows Terminal 1.24, both with their default settings and `pwsh -NoProfile`;
the median of three runs. Memory is the working set of the terminal and its OpenConsole, the shells left out.</sub>

## Knows what your agents are doing

Run Claude Code, Codex, Gemini CLI or Aider in a tab and the tab says whether it is **working**, **waiting for you**,
**done** or **failed**. The sidebar groups the tabs by state, the title bar counts the ones that wait, and
**Ctrl+Shift+U** takes you to the next of them. Nothing to set up.

![Nekominal's sidebar with five agent tabs grouped by state: waiting, running and done](assets/screenshots/agents.png)

For Claude Code, the title bar also shows the model, the share of the context used, the cost and the time. Hover the
meter for the details, and for the session's Remote Control link when it has one.

| Agent cards | By repository |
| :---: | :---: |
| ![Each tab as a card with its state, folder and branch](assets/screenshots/cards.png) | ![The tabs under the repository they work in, with their state](assets/screenshots/repos.png) |

## Made for parallel work

- **One worktree per agent.** Open a tab in a new git worktree from the command palette; removing it shows what would
  be lost first.
- **Updates keep your tabs running.** Update and restart, and every shell and agent carries on in the new version.
- **Pull requests and ports on the tabs**, and phone notifications through ntfy or a webhook when an agent needs you.
- **Scriptable.** `nekominal cli` lists, opens, splits and reads panes, so your own tools can drive it.

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
