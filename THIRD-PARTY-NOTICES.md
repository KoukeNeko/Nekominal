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

## Rust crates

`Nekominal.exe` is built from Rust crates published on crates.io (among them `alacritty_terminal`, `vte`, `windows`,
`swash`, `serde` and `accesskit`), each under its own licence, mostly MIT or Apache-2.0. `Cargo.lock` lists every crate and
version that goes into a build; `cargo tree` shows where each comes from.
