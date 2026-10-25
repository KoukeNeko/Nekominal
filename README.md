<p align="center"><img src="assets/nekominal.svg" width="112" alt="Nekominal"></p>

# Nekominal

A terminal for Windows, written in Rust with no web technology. It is made for coding agents: Claude Code, including its
full-screen mode, runs in it, and the window shows which tabs are working and which wait for you.

The window is a plain Win32 window. Direct3D 11 draws the terminal and the interface, DirectComposition puts the result over a
blurred backdrop, and ConPTY runs the shells.

## Install

```powershell
scoop bucket add koukeneko https://github.com/KoukeNeko/scoop-bucket
scoop install koukeneko/nekominal
```

To update, use **Update and restart** in the command palette or on the About page of the settings: Scoop cannot replace a
program that is running, so Nekominal closes its windows, updates, and starts again. From another terminal,
`scoop update nekominal` does the same while Nekominal is closed.

A release is also a zip file, one for x64 and one for ARM64 (Scoop picks the one of the machine): unpack it anywhere and run `Nekominal.exe`. The zip holds the ConPTY pair (`conpty.dll`,
`OpenConsole.exe`) the sessions run on; keep the three files together.

## Features

- **Tabs on top or in a sidebar.** The first start asks which; the settings change it later. On top, the tabs are words in a
  40-DIP title bar with a line under the selected one. In the sidebar, a tab is a row (its program, its state, its folder
  and branch) or an icon with a ring of the colour of its state. The sidebar keeps your order, or groups the tabs by state;
  Ctrl+Shift+B hides and shows it.
- **What the agents are doing.** A tab at work has a spinner, a tab that waits for you pulses, and the title bar counts the tabs
  that wait ("2 waiting"; a click goes to the next). For Claude Code the title bar also shows the model, the context used and
  the cost.
- Split panes. The tabs slide to their places; drag a tab out of the window and it becomes a window of its own, or drop it on
  another window. The middle button closes a tab.
- Menu, command palette, find bar and tab list, all over the blurred backdrop, and following the window when it moves. Mica,
  Mica Alt, acrylic or none; ten color schemes; the opacity of the background.
- The settings are a window of their own: a column of sections with a search line, a control for every setting, and a
  shortcuts page that rebinds a command by pressing the new chord. The About page shows the version, looks for a newer one,
  and lists the contributors.
- A blinking cursor at a pace you choose, or a steady one.
- Optional: the tabs, the panes, their folders and the place of the window come back at the next start (`restoreSession`).
- Copy and paste with confirmations for large or multi-line text, OSC 52, links, IME composition drawn in place (Zhuyin tested).
- The kitty keyboard protocol, key releases included, for the programs that ask for it.
- Screen readers see the tabs, the text of the terminal, the settings and the open menu, through UI Automation.
- Traditional Chinese and English.

## Shortcuts

| Action | Shortcut |
| --- | --- |
| New tab | Ctrl+Shift+T |
| Choose a profile for a new tab | Ctrl+Shift+N |
| Next tab that waits for you | Ctrl+Shift+U |
| Hide or show the sidebar | Ctrl+Shift+B |
| Split right, split down | Alt+Shift+D, Alt+Shift+S |
| Move between panes | Alt+Arrow |
| Zoom the pane | Ctrl+Shift+Z |
| Command palette | Ctrl+Shift+P |
| Find | Ctrl+Shift+F |
| Settings | Ctrl+Shift+O |
| Full screen | F11 |

The settings list every shortcut with its binding. `settings.json` in `%APPDATA%\Nekominal` holds what the settings change and
can be edited by hand; the settings have a button that opens it.

## Build

Rust 1.95 or later, the Visual Studio Build Tools, Windows 11 (the backdrops need build 22621 or later).

```powershell
scripts\fetch-conpty.ps1                 # the ConPTY pair, checked against a pinned hash and signature
cargo build --release -p nekominal-shell # target\release\nekominal-next.exe
scripts\package.ps1                      # dist\Nekominal-<version>-windows-x64.zip
scripts\package.ps1 -Arch arm64          # dist\Nekominal-<version>-windows-arm64.zip (MSVC ARM64 build tools)
```

The icon is drawn in `assets\nekominal.svg` (and `nekominal-small.svg`, with a thicker line, for 32 px and less).
`scripts\make-icon.ps1` renders them with Chrome or Edge into `assets\nekominal.ico`, which the build puts into the program.

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
| `nekominal-shell` | the application: window, tabs, sidebar, pop-ups, settings, updates |

## Releases

The version is `version` under `[workspace.package]` in `Cargo.toml`. Change it, commit, and push an annotated tag with the
same number:

```powershell
git tag -a v0.2.0 -m v0.2.0
git push origin v0.2.0
```

The release workflow writes the contributors of this repository into the build (`crates\nekominal-shell\contributors.json`),
builds and tests the archive, attaches it to a release here and to a release of the bucket, and writes
`bucket\nekominal.json` in `KoukeNeko/scoop-bucket`. The repository secret `SCOOP_BUCKET_TOKEN` has to be able to write to the
bucket.
