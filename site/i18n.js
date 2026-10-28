// The page is written in Traditional Chinese; this file holds the English, keyed by each element's data-i18n / data-i18n-alt.
// shortcut: a language is a dictionary plus a stored choice, no plural or format rules; move to a library when a third language comes.
const EN = {
  title: 'Nekominal — a light Windows terminal',
  desc: 'Native Rust and Direct3D, no web runtime inside. Every tab tells you which agent is waiting for you.',
  copy: 'Copy', copied: 'Copied', copyFailed: 'Copy by hand',
  'nav.light': 'Light', 'nav.agents': 'Agents', 'nav.parallel': 'Parallel work', 'nav.lang': '中文', 'nav.download': 'Download',
  'hero.title': 'A light<br><span class="grad">Windows terminal</span>',
  'hero.sub': 'Native Rust and Direct3D, no web runtime inside. Every tab tells you which agent is waiting for you.',
  'cap.0': `<h3>The journey of one keystroke</h3><ol><li>You type in the window: Rust handles the keys, the tabs and the panes.</li><li>The keys go through ConPTY to your shell.</li><li>The output of the shell comes back through ConPTY.</li><li>Direct3D redraws when the picture changes.</li></ol><p class="hint">Point at a layer to see what it does.</p>`,
  'cap.g': `<h3>Chromium · Node · WebView <span>not there</span></h3><p>Many desktop apps ship a web engine inside. Nekominal has no such layer: its interface is not a web page but drawn by the program itself, so there is no runtime to install and no memory spent on one.</p><ul><li>No Electron, no WebView.</li><li>The whole folder is about 10 MB.</li></ul>`,
  'cap.1': `<h3>Window and tabs <span>Rust</span></h3><p>Rust says of itself that it has no runtime and no garbage collector, and that its type system and ownership model guarantee memory safety and thread safety.</p><ul><li>Tabs, split panes, the command palette, search and settings live in this layer.</li><li>Each session has three threads: one reads the output, one waits for the process to end, one puts both in order for the window.</li></ul><a href="https://www.rust-lang.org/">rust-lang.org</a>`,
  'cap.2': `<h3>Drawing <span>Direct3D 11</span></h3><p>Direct3D 11 is the GPU graphics API of Windows. Microsoft describes it for games, scientific and desktop applications, and it has been there since Windows 7.</p><ul><li>The interface is drawn by Nekominal itself: rounded rectangles, shadows, text and icons all go through the GPU.</li><li>Glyphs are put in an atlas texture and reused.</li><li>It redraws only when something changes, so a terminal you are not looking at costs next to nothing.</li></ul><a href="https://learn.microsoft.com/windows/win32/direct3d11/atoc-dx-graphics-direct3d-11">Direct3D 11 on Microsoft Learn</a>`,
  'cap.3': `<h3>Terminal backend <span>ConPTY</span></h3><p>ConPTY, the Windows pseudo console, lets a program be the terminal of a command-line program: the window writes the input into one pipe, reads text and virtual terminal sequences from another, and draws them itself.</p><ul><li>Nekominal brings Microsoft's conpty.dll and OpenConsole.exe, a matching pair, and falls back to the one in Windows when it cannot load them.</li><li>Every pane runs in a Job Object, so closing a pane ends its whole process tree.</li><li>About 2 ms of output is merged into one block, and reading pauses when 256 KB are waiting to be taken.</li></ul><a href="https://learn.microsoft.com/windows/console/creating-a-pseudoconsole-session">Pseudoconsole sessions on Microsoft Learn</a>`,
  'cap.4': `<h3>Your own shell <span>pwsh, WSL …</span></h3><p>Nekominal has no shell of its own. It starts the one you already use and connects it to the window through ConPTY. To the shell it looks like any terminal.</p><ul><li>PowerShell (pwsh), WSL and the like all work.</li><li>The shell is not part of the size of Nekominal.</li></ul>`,
  'hero.total': 'Nekominal.exe is 9 MB; with ConPTY about 10 MB',
  'hero.aria': 'The four layers of Nekominal, and a fifth above them that is not there',
  's.ghost': 'This layer does not exist', 's.1': 'Window and tabs', 's.2': 'Drawing', 's.2s': 'Direct3D 11, redraws on change', 's.3': 'Terminal backend', 's.4': 'Your own shell',
  'agents.main': 'The sidebar groups the tabs by state: waiting, failed, running, done',
  'hero.alt': 'The Nekominal window, its sidebar listing five agent tabs grouped by state',
  'light.h': 'Light by design',
  'light.lead': 'Nekominal.exe is about 9 MB, a native program in Rust and Direct3D 11; the two ConPTY files sit beside it, about 10 MB for the whole folder. There is no Electron, no WebView and no runtime to install. A window draws only when something on it changes, so a terminal you are not looking at costs next to nothing.',
  'light.t3': 'Launch until the shell is ready', 'light.n3': '1.34 s', 'light.v3': '1.23 s', 'light.w3': '2.48 s',
  'light.t4': 'CPU at rest, per 20 s', 'light.n4': '0.031 s', 'light.v4': '0.047 s', 'light.w4': '0.25 s',
  'light.note': 'All three terminals were measured on the same Windows 11 machine, in the same way: default settings, a cold start, running <code>pwsh</code>; Windows Terminal and Warp the median of five runs, Nekominal the mean of two. Memory is the working set of the terminal and its OpenConsole, the shells left out. Ready means the shell has finished starting and waits for input; CPU at rest is the processor time over 20 s with one tab open and nobody touching it, three windows a run, the median of them. Versions: Nekominal (a build after 0.2.14), Windows Terminal 1.24, Warp 0.2026.10.14 (default settings except that restoring windows at start is off).',
  'light.t1': 'Memory, one tab',
  'light.t2': 'Memory, five tabs',
  'agents.h': 'Knows what your agents are doing',
  'agents.lead': 'Run Claude Code, Codex, Gemini CLI or Aider in a tab and the tab says whether it is <b>working</b>, <b>waiting for you</b>, <b>done</b> or <b>failed</b>. The sidebar groups the tabs by state, the title bar counts the ones that wait, and <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>U</kbd> takes you to the next of them. Nothing to set up.',
  'agents.cards': 'Agent cards', 'agents.cards.alt': 'Each tab as a card with its state, folder and branch',
  'agents.repos': 'By repository', 'agents.repos.alt': 'The tabs under the repository they work in, with their state',
  'ctx.txt': '<h3>The context, at a glance</h3><p>For Claude Code, the title bar shows the model, the share of the context used, the cost and the time. Hover the meter to see how much is read from cache, written to cache and new input, and the tokens and lines of the session.</p><p>When the session has a Remote Control link, you can open or copy it there.</p>',
  'ctx.alt': 'Hovering the context meter shows a card with the context used, the cache, the cost and the time',
  'sub.txt': '<h3>The subagents card</h3><p>The title bar counts how many subagents run and how many have ended, and each row of the sidebar shows how many its tab\'s agent runs.</p><p>Open the card to see what every subagent is doing, and the phases of every workflow.</p>',
  'sub.alt': 'The title bar says 12 running, 95 done; the open card lists the phases of a workflow and its subagents',
  'drawer.txt': '<h3>Open one to see what it did</h3><p>A row of the card opens a drawer: what the subagent was asked, the tools it called and what it said. For a workflow, its phases and agents.</p>',
  'drawer.alt': 'A drawer on the right showing the phase of a workflow and four running agents',
  'par.h': 'Made for parallel work',
  'par.1h': 'One worktree per agent', 'par.1p': 'Open a tab in a new git worktree from the command palette; removing it shows what would be lost first.',
  'par.2h': 'Updates keep your tabs running', 'par.2p': 'Update and restart, and every shell and agent carries on in the new version.',
  'par.3h': 'Pull requests and ports on the tabs', 'par.3p': 'And phone notifications through ntfy or a webhook when an agent needs you.',
  'par.4h': 'Scriptable', 'par.4p': '<code>nekominal cli</code> lists, opens, splits and reads panes, so your own tools can drive it.',
  'term.h': 'A terminal first',
  'term.lead': 'Split panes, tear-off tabs, a drop-down window on Win+`, saved layouts and session restore. Hints for URLs, paths and hashes, <code>file:line</code> opened in your editor, search across all tabs. Sixel and kitty images, font ligatures, Mica and acrylic, and colour schemes imported from Windows Terminal and iTerm2.',
  'term.split': 'Split panes: right, down, as you like', 'term.split.alt': 'One window split into three panes, each running a git command',
  'term.palette': 'The command palette: every command, with its shortcut beside it', 'term.palette.alt': 'The command palette, with split typed, listing two split commands and their shortcuts',
  'term.search': 'Search across all tabs: find which one printed it', 'term.search.alt': 'Search across all tabs, listing the lines with finished in three tabs',
  'term.hints': 'Hints: a URL, a path, a hash, one letter to open it', 'term.hints.alt': 'Hint mode puts a single letter before a URL, a path and a hash',
  'term.settings': 'Settings is a tab too: layout, colours, profiles, agents, shortcuts', 'term.settings.alt': 'The settings tab, its appearance page with the tab layout and colour schemes',
  'term.lig': 'Font ligatures, Sixel and kitty images', 'term.lig.alt': 'Font ligatures, and a Sixel and a kitty image shown in the terminal',
  'term.yazi': 'Image previews in yazi', 'term.yazi.alt': 'An image previewed in yazi',
  'term.chafa': 'Images with chafa', 'term.chafa.alt': 'An image shown in the terminal with chafa',
  'key.1': 'Next tab that waits', 'key.2': 'Command palette', 'key.3': 'Split right', 'key.4': 'Search all tabs', 'key.5': 'Hints', 'key.6': 'Settings',
  'key.note': 'Every command is in the palette, and any shortcut can be changed in Settings.',
  'inst.h': 'Install',
  'inst.lead': 'Or download the zip and run <code>Nekominal.exe</code>. Windows 11, x64 or ARM64.',
  'inst.btn': 'Download the latest release',
  'foot.notes': 'Release notes', 'foot.support': 'Support',
};

const KEY = 'nekominal-lang';
const store = { get() { try { return localStorage.getItem(KEY); } catch { return null; } }, set(v) { try { localStorage.setItem(KEY, v); } catch {} } };
const first = new URLSearchParams(location.search).get('lang') ?? store.get() ?? (navigator.language.startsWith('zh') ? 'zh' : 'en');
let lang = first === 'en' ? 'en' : 'zh';

const meta = document.querySelector('meta[name="description"]');
const zh = { title: document.title, desc: meta.content };
const text = (el, key) => (lang === 'en' ? EN[key] ?? el.dataset.zh : el.dataset.zh);

function apply() {
  document.documentElement.lang = lang === 'en' ? 'en' : 'zh-Hant';
  document.title = lang === 'en' ? EN.title : zh.title;
  meta.content = lang === 'en' ? EN.desc : zh.desc;
  document.querySelectorAll('[data-i18n]').forEach(el => { el.dataset.zh ??= el.innerHTML; el.innerHTML = text(el, el.dataset.i18n); });
  document.querySelectorAll('[data-i18n-aria]').forEach(el => { el.dataset.zhAria ??= el.getAttribute('aria-label'); el.setAttribute('aria-label', lang === 'en' ? EN[el.dataset.i18nAria] ?? el.dataset.zhAria : el.dataset.zhAria); });
  document.querySelectorAll('[data-i18n-alt]').forEach(el => { el.dataset.zh ??= el.alt; el.alt = text(el, el.dataset.i18nAlt); });
}

// What the copy buttons say after a click, in the language of the page.
const ZH = {
  copied: '已複製', copyFailed: '請手動複製', copy: '複製',
  'cap.0': `<h3>一次按鍵的旅程</h3><ol><li>你在視窗打字：Rust 處理按鍵、分頁與窗格。</li><li>按鍵經 ConPTY 交給你的 shell。</li><li>shell 的輸出經 ConPTY 回到視窗。</li><li>Direct3D 在畫面有變化時重繪。</li></ol><p class="hint">把游標移到某一層，看它做什麼。</p>`,
  'cap.g': `<h3>Chromium · Node · WebView <span>不存在</span></h3><p>常見的桌面應用會把網頁引擎一起裝進來。Nekominal 沒有這一層：介面不是網頁，而是程式自己畫出來的，所以不用另外安裝執行環境，也沒有它佔用的記憶體。</p><ul><li>沒有 Electron、沒有 WebView。</li><li>整個資料夾約 10 MB。</li></ul>`,
  'cap.1': `<h3>視窗與分頁 <span>Rust</span></h3><p>Rust 官方說明它沒有執行階段（runtime）也沒有垃圾回收，並由型別系統與所有權模型保證記憶體安全與執行緒安全。</p><ul><li>分頁、分割窗格、命令選擇區、搜尋與設定，都在這一層。</li><li>每個工作階段有三條執行緒：讀輸出、等行程結束、把兩者整理成有順序的事件交給視窗。</li></ul><a href="https://www.rust-lang.org/">Rust 官方網站</a>`,
  'cap.2': `<h3>繪製 <span>Direct3D 11</span></h3><p>Direct3D 11 是 Windows 的 GPU 繪圖介面。微軟說明它可用於遊戲、科學與桌面應用程式，從 Windows 7 起就有。</p><ul><li>介面由 Nekominal 自己畫：圓角矩形、陰影、文字與圖示，都經過 GPU。</li><li>字形先放進圖集貼圖，之後重複使用。</li><li>畫面有變化才重繪，你沒在看的終端機幾乎不花資源。</li></ul><a href="https://learn.microsoft.com/windows/win32/direct3d11/atoc-dx-graphics-direct3d-11">微軟官方文件：Direct3D 11</a>`,
  'cap.3': `<h3>終端機後端 <span>ConPTY</span></h3><p>ConPTY 是 Windows 的偽終端機，讓程式可以替命令列程式當「終端機」：視窗把輸入寫進一條管線，從另一條管線讀回文字與虛擬終端序列，再自己畫出來。</p><ul><li>Nekominal 隨附微軟的 conpty.dll 與 OpenConsole.exe（版本相配的一組），載入不了時退回 Windows 內建的。</li><li>每個窗格都放在 Job Object 裡，關掉窗格就連同整個行程樹一起結束。</li><li>約 2 毫秒內的輸出合成一塊；積壓超過 256 KB 還沒被消化就暫停讀取。</li></ul><a href="https://learn.microsoft.com/windows/console/creating-a-pseudoconsole-session">微軟官方文件：偽終端機工作階段</a>`,
  'cap.4': `<h3>你原本的 shell <span>pwsh、WSL …</span></h3><p>Nekominal 沒有自己的 shell。它啟動你原本在用的那一個，透過 ConPTY 接上視窗；對 shell 來說，這就是一個普通的終端機。</p><ul><li>PowerShell（pwsh）、WSL 等都可以。</li><li>shell 不算在 Nekominal 的大小裡。</li></ul>`,
};
window.i18n = { say: key => (lang === 'en' ? EN[key] : ZH[key]) };

document.getElementById('lang').addEventListener('click', () => { lang = lang === 'en' ? 'zh' : 'en'; store.set(lang); apply(); });
apply();
