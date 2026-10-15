# Nekominal

A terminal for Windows, written in Rust with no web technology. It is made for coding agents: Claude Code, including its
full-screen mode, runs in it.

The window is a plain Win32 window. Direct3D 11 draws the terminal and the interface, DirectComposition puts the result over a
blurred backdrop, and ConPTY runs the shells.

## Install

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal
```

Update with `scoop update nekominal`. A release is also a zip file: unpack it anywhere and run `Nekominal.exe`. The zip holds
the ConPTY pair (`conpty.dll`, `OpenConsole.exe`) the sessions run on; keep the three files together.

## Features

- Tabs and split panes. Drag a tab out of the title bar for a window of its own, or onto the title bar of another window to join it.
- A status mark on each tab that asks for you: a bell, a notification, a finished or failed command, an agent that waits.
- Menu, command palette, find bar, tab list, and a settings page, all over the blurred backdrop. Mica, Mica Alt, acrylic or none;
  ten color schemes; the opacity of the background.
- Copy and paste with confirmations for large or multi-line text, OSC 52, links, IME composition drawn in place (Zhuyin tested).
- Screen readers see the tabs, the text of the terminal, and the open menu, through UI Automation.
- Traditional Chinese and English.

## Shortcuts

| Action | Shortcut |
| --- | --- |
| New tab | Ctrl+Shift+T |
| Choose a profile for a new tab | Ctrl+Shift+N |
| Split right, split down | Alt+Shift+D, Alt+Shift+S |
| Move between panes | Alt+Arrow |
| Zoom the pane | Ctrl+Shift+Z |
| Command palette | Ctrl+Shift+P |
| Find | Ctrl+Shift+F |
| Settings | Ctrl+Shift+O |
| Full screen | F11 |

The settings page lists every shortcut with its binding. `settings.json` in `%APPDATA%\Nekominal` holds what the page changes
and can be edited by hand; the page has a button that opens it.

## Build

Rust 1.95 or later, the Visual Studio Build Tools, Windows 11 (the backdrops need build 22621 or later).

```powershell
scripts\fetch-conpty.ps1                 # the ConPTY pair, checked against a pinned hash and signature
cargo build --release -p nekominal-shell # target\release\nekominal-next.exe
scripts\package.ps1                      # dist\Nekominal-<version>-windows-x64.zip
```

Tests:

```powershell
cargo test -p nekominal-model -p nekominal-input -p nekominal-term -p nekominal-render -p nekominal-pane -p nekominal-ui -p nekominal-shell
```

`tools\slice-drive\next-scenarios.ps1` drives the built window with real keyboard and mouse input and reads the results back
(`Scenario-NextTabs`, `Scenario-NextSettings`, `Scenario-NextClaude`, ...). The scenarios take the foreground, so leave the
machine alone while they run.

## Layout

| Crate | What it holds |
| --- | --- |
| `nekominal-pty` | ConPTY host and sessions |
| `nekominal-term` | the terminal emulator (a wrapper around `alacritty_terminal`), search, OSC handling |
| `nekominal-input` | encoding of keys and the mouse, including the kitty keyboard protocol |
| `nekominal-model` | settings, commands, key map, palette, themes, strings |
| `nekominal-render` | the Direct3D 11 grid renderer |
| `nekominal-ui` | the drawn interface: rounded rectangles, text, icons |
| `nekominal-pane` | a pane: a session, an emulator, and a renderer |
| `nekominal-shell` | the application: window, tabs, pop-ups, settings page |

## Releases

The version is `version` under `[workspace.package]` in `Cargo.toml`. Change it, commit, and push a tag with the same number:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

The release workflow builds and tests the archive, attaches it to a release here and to a release of the bucket, and writes
`bucket\nekominal.json` in `KoukeNeko/scoop-bucket`. The repository secret `SCOOP_BUCKET_TOKEN` has to be able to write to the bucket.
