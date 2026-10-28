// The page is written in Traditional Chinese; this file holds the English, keyed by each element's data-i18n / data-i18n-alt.
// shortcut: a language is a dictionary plus a stored choice, no plural or format rules; move to a library when a third language comes.
const EN = {
  title: 'Nekominal — a light Windows terminal',
  desc: 'Native Rust and Direct3D, no web runtime inside. Every tab tells you which agent is waiting for you.',
  copy: 'Copy', copied: 'Copied', copyFailed: 'Copy by hand',
  'nav.light': 'Light', 'nav.agents': 'Agents', 'nav.parallel': 'Parallel work', 'nav.lang': '中文', 'nav.download': 'Download',
  'hero.title': 'A light<br><span class="grad">Windows terminal</span>',
  'hero.sub': 'Native Rust and Direct3D, no web runtime inside. Every tab tells you which agent is waiting for you.',
  'cap.0': 'Point at any layer.',
  'cap.g': 'This layer does not exist: no Chromium, no Node, no web view.',
  'cap.1': 'Tabs, split panes, the command palette, search and settings all live in this layer.',
  'cap.2': 'Drawn with Direct3D 11, and redrawn only when something changes, so a terminal you are not looking at costs next to nothing.',
  'cap.3': 'ConPTY is the pseudo console of Windows. It connects your shell to the window.',
  'cap.4': 'pwsh, WSL and the rest: the shell you already use, running as it always did.',
  'hero.total': 'Nekominal.exe itself is only 9 MB; with the two ConPTY files, the whole folder is about 10 MB',
  'hero.aria': 'The four layers of Nekominal, and a fifth above them that is not there',
  's.ghost': 'This layer does not exist', 's.1': 'Window and tabs', 's.2': 'Drawing', 's.2s': 'Direct3D 11, redraws on change', 's.3': 'Terminal backend', 's.4': 'Your own shell',
  'agents.main': 'The sidebar groups the tabs by state: waiting, failed, running, done',
  'hero.alt': 'The Nekominal window, its sidebar listing five agent tabs grouped by state',
  'light.h': 'Light by design',
  'light.lead': 'Nekominal.exe is about 9 MB, a native program in Rust and Direct3D 11; the two ConPTY files sit beside it, about 10 MB for the whole folder. There is no Electron, no web view and no runtime to install. A window draws only when something on it changes, so a terminal you are not looking at costs next to nothing.',
  'light.t1': 'Memory, one tab',
  'light.t2': 'Memory, five tabs',
  'light.n3': '0.70 s', 'light.t3': 'Launch to a running shell', 'light.v3': '0.76 s',
  'light.n4': '0.02 s', 'light.t4': 'CPU at rest, per 20 s', 'light.v4': '0.09 s',
  'light.note': 'Measured on Windows 11 against Windows Terminal 1.24, both with their default settings and <code>pwsh -NoProfile</code>; the median of three runs. Memory is the working set of the terminal and its OpenConsole, the shells left out.',
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
  'cap.0': '把游標移到任何一層上看看。',
  'cap.g': '這一層不存在：沒有 Chromium、沒有 Node、沒有 web view。',
  'cap.1': '分頁、分割窗格、命令選擇區、搜尋與設定，都在這一層。',
  'cap.2': '用 Direct3D 11 繪製，畫面有變化才重繪，你沒在看的終端機幾乎不花資源。',
  'cap.3': 'ConPTY 是 Windows 的偽終端機，把你的 shell 接到視窗上。',
  'cap.4': 'pwsh、WSL 等等：你原本在用的 shell，照常執行。',
};
window.i18n = { say: key => (lang === 'en' ? EN[key] : ZH[key]) };

document.getElementById('lang').addEventListener('click', () => { lang = lang === 'en' ? 'zh' : 'en'; store.set(lang); apply(); });
apply();
