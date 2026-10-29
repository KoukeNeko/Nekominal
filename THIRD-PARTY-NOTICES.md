# Third-party notices

Nekominal itself is under the [MIT License](LICENSE). The release archives also carry work of others, under their own
licences.

## ConPTY (`conpty.dll`, `OpenConsole.exe`)

The pseudoconsole the shells run on comes unchanged from Microsoft's `Microsoft.Windows.Console.ConPTY` NuGet package, built
from [microsoft/terminal](https://github.com/microsoft/terminal).

```text
MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Program marks (`assets/brands`)

A tab whose program's file has no icon, and no installed program beside it that has one (Git Bash for `git`, `node.exe`
for `npm`), is shown by that program's mark, unchanged from [Simple Icons](https://simpleicons.org) 16.36.0. Simple
Icons is under [CC0 1.0](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md); each mark is used only to
name the program it belongs to and stays the trademark of its owner, which neither endorses nor is affiliated with
Nekominal. Two marks carry their own licence:

- `git.svg`: the Git logo by [Jason Long](https://git-scm.com/community/logos), under
  [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/).
- `yarn.svg`: the Yarn logo from [yarnpkg/assets](https://github.com/yarnpkg/assets), under
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

The others (`github`, `npm`, `pnpm`, `uv`, `esbuild`, `vite`, `curl`, `7zip`) come under Simple Icons' CC0. Marks under
a share-alike licence (Rust's, Neovim's) are left out. Which program takes which mark is the table `PROGRAMS` in
`crates/nekominal-shell/src/icons.rs`; a test there keeps it, this folder and this list in step.

## Rust crates

`Nekominal.exe` is built from Rust crates published on crates.io (among them `alacritty_terminal`, `vte`, `windows`,
`swash`, `serde` and `accesskit`), each under its own licence, mostly MIT or Apache-2.0. `Cargo.lock` lists every crate and
version that goes into a build; `cargo tree` shows where each comes from.
