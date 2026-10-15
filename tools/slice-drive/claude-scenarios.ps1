# Claude Code CLI inside the real window: what a person sees and feels once claude.exe runs in a pwsh tab of nekominal.exe.
# The headless half (queries, modes, key bytes, a resize storm) is tools/agent-probe; see docs/agent-cli-compat.md.
#
#   . .\tools\slice-drive\claude-scenarios.ps1
#   Scenario-ClaudeScreen       # full-screen mode: the tab's environment, the main UI pixel by pixel, typing, the help screen
#   Scenario-ClaudeKeys         # Backspace, word moves, Shift+Enter, Esc, Tab, Ctrl+C, Ctrl+L, bracketed and large pastes
#   Scenario-ClaudeMouse        # wheel, click to place the caret, the CLI's selection, Shift+drag, a link, the right button
#   Scenario-ClaudeIme          # Zhuyin: the preedit, the rectangle the input method places its window from, the commit
#   Scenario-ClaudeFrames       # every capture while typing is a whole frame; typing under a resize storm
#   Scenario-ClaudeResize       # a storm, a drag, maximize, half-screen snaps, F11 and the font size against a fresh run
#   Scenario-ClaudeClassic      # classic mode: the same, and one banner in the history after narrowing
#   Scenario-ClaudePanes        # a split, a zoom, another tab and back, twelve tabs, closing a pane and a tab
#   Scenario-ClaudeWindow       # focus reports, the transcript hand-off, closing the window
#   Scenario-ClaudeExit         # idle CPU and GPU, then /exit and Ctrl+C twice: modes, cursor, prompt marks
#   Scenario-ClaudeAll
#
# A scenario fails (throws) when a check fails; the lines it prints are `ok`, `FAIL` and `info`.
#
# Safety rules (they hold in every function here):
#   * The app starts with a scrubbed environment: every CLAUDE*, ANTHROPIC*, MCP_*, OTEL_* variable, NO_COLOR and anything
#     named like a secret is removed from the child, then DISABLE_AUTOUPDATER=1, CLAUDE_CODE_SAFE_MODE=1 and the renderer
#     switch of the mode are set. Per process only; the tab is asked for the names it received and the scenario stops if
#     a variable it did not ask for is there.
#   * Nothing is ever sent to the model. Enter reaches the CLI only through Submit-Local, which checks that the input box
#     holds exactly /help or /exit (a suggestion menu must select the same command); Shift+Enter and Alt+Enter are tried
#     with /help in the box, where a submit would be harmless.
#   * One claude.exe at a time. The processes below the app are recorded and any survivor is killed when a run ends;
#     claude processes that do not descend from the app are never touched.
#   * Screenshots and text dumps of the CLI show account and folder names: they go to $global:NkClaudeShots, a folder
#     outside the repository, and are never committed.

. (Join-Path $PSScriptRoot 'app-scenarios.ps1')

$global:NkClaudeShots = if ($env:NK_CLAUDE_SHOTS) { $env:NK_CLAUDE_SHOTS } else { Join-Path ([IO.Path]::GetTempPath()) 'nekominal-drive\claude-shots' }
New-Item -ItemType Directory -Force $global:NkClaudeShots | Out-Null
$global:NkShots = $global:NkClaudeShots
$global:ClaudeExe = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
# An empty folder inside the workspace the user already trusts: the CLI shows no trust dialog there, so nothing is written.
$global:ClaudeCwd = if ($env:NEKOMINAL_PROBE_CWD) { $env:NEKOMINAL_PROBE_CWD } else { Join-Path $global:NkRoot 'target\agent-probe\cwd' }
New-Item -ItemType Directory -Force $global:ClaudeCwd | Out-Null

$global:ClaudeFailures = @()
$global:ClaudeKids = @{}

$global:ClaudeRule = [string][char]0x2500
$global:ClaudeMarker = [string][char]0x276F

# --- environment -----------------------------------------------------------------------------------------------------------

# Names the child must not inherit.
function Get-ScrubbedNames {
    Get-ChildItem Env: | Where-Object { $_.Name -match '^(CLAUDE|ANTHROPIC|MCP_|OTEL_)' -or $_.Name -eq 'NO_COLOR' -or $_.Name -match '(_TOKEN|_SECRET|_API_KEY)$|PASSWORD' } | ForEach-Object { $_.Name }
}

# The Env table for Run-App: a $null value removes the variable (see Start-App).
function Get-ClaudeEnv([ValidateSet('fullscreen', 'classic')][string]$Mode) {
    $table = @{}
    foreach ($name in Get-ScrubbedNames) { $table[$name] = $null }
    $table['DISABLE_AUTOUPDATER'] = '1'
    $table['CLAUDE_CODE_SAFE_MODE'] = '1'
    if ($Mode -eq 'fullscreen') { $table['CLAUDE_CODE_NO_FLICKER'] = '1' } else { $table['CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN'] = '1' }
    # claude.exe is not on PATH; the folder is added for this process only.
    $table['PATH'] = $env:PATH + ';' + (Split-Path $global:ClaudeExe)
    $table
}

# --- checks ----------------------------------------------------------------------------------------------------------------

# One line per check; a failed one is remembered and fails the scenario when the run is over (Finish-Claude).
function Claude-Check([string]$Id, [bool]$Ok, [string]$Evidence) {
    if (-not $Ok) { $global:ClaudeFailures += $Id }
    '{0} {1} | {2}' -f $(if ($Ok) { 'ok  ' } else { 'FAIL' }), $Id, $Evidence
}
function Claude-Info([string]$Id, [string]$Evidence) { 'info {0} | {1}' -f $Id, $Evidence }
function Finish-Claude {
    if ($global:ClaudeFailures.Count -gt 0) { throw ('{0} check(s) failed: {1}' -f $global:ClaudeFailures.Count, ($global:ClaudeFailures -join ', ')) }
}

# --- pixels ----------------------------------------------------------------------------------------------------------------

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class Ink {
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  // The contrast inside every cell of a grid: the largest spread of one colour channel over the cell's pixels. Text, a
  // box-drawing line or a cursor has a high one; a flat or softly blurred background has a low one, whatever its colour.
  // `px` are the BGRA bytes of a capture; (ox, oy) is where the grid starts in it. The cell is measured `inset` pixels in
  // from its edges: a block character may overdraw its neighbour by a pixel so that no seam shows. The first row skips
  // `topSkip` more pixels: the line that marks the focused pane (or one that rang) is drawn over the top of the pane.
  public static int[] Contrast(byte[] px, int stride, int width, int height, int ox, int oy, int cw, int ch, int cols, int rows, int inset, int topSkip) {
    int[] result = new int[cols * rows];
    for (int row = 0; row < rows; row++) {
      for (int col = 0; col < cols; col++) {
        int top = row == 0 ? topSkip : 0;
        int x0 = ox + col * cw + inset, y0 = oy + row * ch + inset + top, w = cw - 2 * inset, h = ch - 2 * inset - top;
        int minB = 255, minG = 255, minR = 255, maxB = 0, maxG = 0, maxR = 0;
        for (int y = Math.Max(y0, 0); y < Math.Min(y0 + h, height); y++) {
          int i = y * stride + Math.Max(x0, 0) * 4;
          for (int x = Math.Max(x0, 0); x < Math.Min(x0 + w, width); x++, i += 4) {
            int b = px[i], g = px[i + 1], r = px[i + 2];
            if (b < minB) minB = b; if (b > maxB) maxB = b;
            if (g < minG) minG = g; if (g > maxG) maxG = g;
            if (r < minR) minR = r; if (r > maxR) maxR = r;
          }
        }
        result[row * cols + col] = Math.Max(maxB - minB, Math.Max(maxG - minG, maxR - minR));
      }
    }
    return result;
  }
  // The number of pixels of two captures whose colour differs by more than `tolerance` in some channel, over the rectangle
  // (x, y, w, h). The alpha channel of a capture of a window with a blurred backdrop is not stable, so it is not compared.
  public static int DiffRegion(byte[] a, int strideA, byte[] b, int strideB, int x0, int y0, int w, int h, int tolerance) {
    int n = 0;
    for (int y = y0; y < y0 + h; y++) {
      int i = y * strideA + x0 * 4, j = y * strideB + x0 * 4;
      for (int x = x0; x < x0 + w; x++, i += 4, j += 4) {
        if (Math.Abs(a[i] - b[j]) > tolerance || Math.Abs(a[i + 1] - b[j + 1]) > tolerance || Math.Abs(a[i + 2] - b[j + 2]) > tolerance) n++;
      }
    }
    return n;
  }
  // The mean colour of a rectangle, as R,G,B.
  public static int[] Mean(byte[] px, int stride, int width, int height, int x0, int y0, int w, int h) {
    long b = 0, g = 0, r = 0, n = 0;
    for (int y = Math.Max(y0, 0); y < Math.Min(y0 + h, height); y++) {
      int i = y * stride + Math.Max(x0, 0) * 4;
      for (int x = Math.Max(x0, 0); x < Math.Min(x0 + w, width); x++, i += 4) { b += px[i]; g += px[i + 1]; r += px[i + 2]; n++; }
    }
    if (n == 0) return new int[] { 0, 0, 0 };
    return new int[] { (int)(r / n), (int)(g / n), (int)(b / n) };
  }
}
'@

# The pixels of a saved capture: BGRA bytes and the geometry.
function Read-Pixels([string]$Png) {
    $bmp = [System.Drawing.Bitmap]::FromFile($Png)
    try {
        $rect = New-Object System.Drawing.Rectangle 0, 0, $bmp.Width, $bmp.Height
        $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $bytes = New-Object byte[] ($data.Stride * $bmp.Height)
            [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
            @{ Bytes = $bytes; Stride = $data.Stride; W = $bmp.Width; H = $bmp.Height }
        } finally { $bmp.UnlockBits($data) }
    } finally { $bmp.Dispose() }
}

# How many cells of two captures of the same pane differ in how much ink they hold (the contrast of the cell by more than 30),
# in the rows whose text did not change between them. A notice of the CLI that expires by itself changes its row; the
# blurred backdrop moves a little behind the glass and is not ink.
function Compare-ShotCells([string]$PathA, [string]$PathB, $Grid, $WinRect, [string[]]$LinesA, [string[]]$LinesB) {
    $a = Get-Contrast $PathA $Grid $WinRect; $b = Get-Contrast $PathB $Grid $WinRect
    $differing = @(); $changedRows = @()
    for ($row = 0; $row -lt $Grid.Rows; $row++) {
        $textA = if ($row -lt $LinesA.Count) { $LinesA[$row] } else { '' }; $textB = if ($row -lt $LinesB.Count) { $LinesB[$row] } else { '' }
        if ($textA -cne $textB) { $changedRows += $row; continue }
        for ($col = 0; $col -lt $Grid.Cols; $col++) { if ([Math]::Abs($a.Flat[$row * $Grid.Cols + $col] - $b.Flat[$row * $Grid.Cols + $col]) -gt 30) { $differing += "r$row c$col" } }
    }
    @{ Differing = $differing; ChangedRows = $changedRows }
}

# Cells that take two columns: CJK, full-width forms and emoji; combining marks and joiners take none.
function Get-CellWidth([int]$CodePoint) {
    if ($CodePoint -ge 0x300 -and $CodePoint -le 0x36F -or $CodePoint -ge 0x200B -and $CodePoint -le 0x200F -or $CodePoint -eq 0xFE0F) { return 0 }
    if ($CodePoint -ge 0x1100 -and $CodePoint -le 0x115F -or $CodePoint -ge 0x2E80 -and $CodePoint -le 0xA4CF -or $CodePoint -ge 0xAC00 -and $CodePoint -le 0xD7A3 `
        -or $CodePoint -ge 0xF900 -and $CodePoint -le 0xFAFF -or $CodePoint -ge 0xFE30 -and $CodePoint -le 0xFE6F -or $CodePoint -ge 0xFF00 -and $CodePoint -le 0xFF60 `
        -or $CodePoint -ge 0xFFE0 -and $CodePoint -le 0xFFE6 -or $CodePoint -ge 0x1F300 -and $CodePoint -le 0x1F64F -or $CodePoint -ge 0x1F900 -and $CodePoint -le 0x1F9FF `
        -or $CodePoint -ge 0x20000 -and $CodePoint -le 0x3FFFD) { return 2 }
    1
}

# The columns each character of a row of text starts at, and the width of the row. Rows are read the way the grid holds
# them: a wide character takes two columns.
function Get-RowColumns([string]$Row) {
    $col = 0; $starts = @()
    for ($i = 0; $i -lt $Row.Length; $i++) {
        $cp = [char]::ConvertToUtf32($Row, $i)
        if ([char]::IsHighSurrogate($Row[$i])) { $i++ }
        $starts += $col
        $col += Get-CellWidth $cp
    }
    @{ Starts = $starts; Width = $col }
}

# The contrast of every cell of the focused terminal in a capture of the window ($Png, taken with Shot-Safe), as rows of
# arrays, with the grid it was measured on.
function Get-Contrast([string]$Png, $Grid, $WinRect) {
    $pix = Read-Pixels $Png
    $ox = [int]($Grid.X - $WinRect.Left); $oy = [int]($Grid.Y - $WinRect.Top)
    $flat = [Ink]::Contrast($pix.Bytes, $pix.Stride, $pix.W, $pix.H, $ox, $oy, $Grid.CW, $Grid.CH, $Grid.Cols, $Grid.Rows, 2, 4)
    @{ Flat = $flat; Cols = $Grid.Cols; Rows = $Grid.Rows; Pixels = $pix; Ox = $ox; Oy = $oy; CW = $Grid.CW; CH = $Grid.CH }
}

# What a capture holds that the text of the screen does not account for, and what the text has that the capture lacks.
# Stray: a cell past the end of its row's text (or a blank cell of a row without wide characters) with ink in it, which is
# what a fragment left over from an earlier frame looks like. Missing: a visible ASCII character of a row without wide
# characters whose cell has no ink. $CaretRows are the rows the cursor may be on; $Ignore lists rectangles (row, column, row,
# column) of the app's own overlays, such as the corner label of a zoomed pane.
function Compare-Ink($Contrast, [string[]]$Lines, [int]$Threshold = 40, [int[]]$CaretRows = @(), $Ignore = @()) {
    $stray = @(); $missing = @(); $rowsChecked = 0
    for ($row = 0; $row -lt $Contrast.Rows; $row++) {
        $text = if ($row -lt $Lines.Count) { $Lines[$row] } else { '' }
        $cols = Get-RowColumns $text
        $plain = $text -match '^[\x20-\x7E─-╿▀-▟ ]*$'
        $rowsChecked++
        for ($col = 0; $col -lt $Contrast.Cols; $col++) {
            $ink = $Contrast.Flat[$row * $Contrast.Cols + $col]
            if (@($Ignore | Where-Object { $row -ge $_[0] -and $row -le $_[2] -and $col -ge $_[1] -and $col -le $_[3] }).Count -gt 0) { continue }
            if ($col -ge $cols.Width) {
                # The cursor sits just past the end of the text of its row (and one cell further after a trailing space).
                if ($ink -gt $Threshold -and -not ($CaretRows -contains $row -and $col -le $cols.Width + 1)) { $stray += "r$row c$col ink $ink" }
            } elseif ($plain) {
                $ch = $text[$col]
                $blank = ($ch -eq ' ' -or $ch -eq [char]0xA0)
                if ($blank -and $ink -gt $Threshold) { $stray += "r$row c$col ink $ink (blank)" }
                # Block elements are flat colour: only glyphs with strokes are held to have ink.
                $solid = ($ch -ge [char]0x2580 -and $ch -le [char]0x259F)
                if (-not $blank -and -not $solid -and $ink -le 12) { $missing += "r$row c$col '$ch' ink $ink" }
            }
        }
    }
    @{ Stray = $stray; Missing = $missing; Rows = $rowsChecked }
}

# --- processes -------------------------------------------------------------------------------------------------------------

# Every process below a pid, from one snapshot of the process table.
function Get-Descendants([int]$RootPid) {
    $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CreationDate)
    $found = @{}
    $queue = [System.Collections.Generic.Queue[int]]::new(); $queue.Enqueue($RootPid)
    while ($queue.Count -gt 0) {
        $parent = $queue.Dequeue()
        foreach ($p in $all | Where-Object { $_.ParentProcessId -eq $parent -and -not $found.ContainsKey([int]$_.ProcessId) }) { $found[[int]$p.ProcessId] = $p; $queue.Enqueue([int]$p.ProcessId) }
    }
    $found.Values
}

# Remembers the processes below the app (with their start time, so a reused pid is never mistaken for one of them).
function Remember-Kids {
    foreach ($p in Get-Descendants $script:App.Id | Where-Object { $_.Name -match '^(claude|pwsh|powershell|OpenConsole|conhost|cmd|node)\.exe$' }) { $global:ClaudeKids[[int]$p.ProcessId] = $p.CreationDate }
}

# Kills what the run left behind, only from the remembered set.
function Stop-Kids {
    foreach ($id in @($global:ClaudeKids.Keys)) {
        $proc = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($proc -and [Math]::Abs(($proc.StartTime - $global:ClaudeKids[$id]).TotalSeconds) -lt 2) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue; "cleanup: stopped leftover $($proc.ProcessName) $id" }
    }
    $global:ClaudeKids = @{}
}

# --- the screen as UI Automation shows it ------------------------------------------------------------------------------------

function Get-ScreenLines([int]$Index = -1) { @((Get-TerminalText $Index) -split "`n" | ForEach-Object { $_.TrimEnd("`r") }) }

# The input box of the CLI: the marker row directly below a horizontal rule, up to the next rule. $null when the screen has none.
function Get-ClaudeBox($Lines = $null) {
    if ($null -eq $Lines) { $Lines = Get-ScreenLines }
    for ($row = $Lines.Count - 1; $row -ge 1; $row--) {
        if ($Lines[$row - 1].StartsWith($global:ClaudeRule) -and $Lines[$row].StartsWith($global:ClaudeMarker)) {
            $end = $null
            for ($next = $row + 1; $next -lt $Lines.Count; $next++) { if ($Lines[$next].StartsWith($global:ClaudeRule)) { $end = $next; break } }
            if ($null -eq $end) { return $null }
            $strip = [char[]]@(' ', [char]0xA0)
            $text = @($Lines[$row].Substring(1).Trim($strip))
            for ($next = $row + 1; $next -lt $end; $next++) { $text += $Lines[$next].Trim($strip) }
            return @{ Row = $row; End = $end; Lines = $text; Text = ($text -join "`n") }
        }
    }
    $null
}

# The text a submit would send, or $null when it cannot be told safely: no box, or a suggestion menu whose selected entry is
# not the command that was typed (Enter would run the entry). The menu is drawn above the box in full-screen mode and below
# it in classic mode; its selected row starts with the marker after an indent.
function Get-SubmitText {
    $lines = Get-ScreenLines
    $box = Get-ClaudeBox $lines
    if (-not $box) { return $null }
    for ($row = 0; $row -lt $lines.Count; $row++) {
        if ($row -ge $box.Row - 1 -and $row -le $box.End) { continue }
        $trim = $lines[$row].TrimStart()
        if ($lines[$row].StartsWith(' ') -and $trim.StartsWith($global:ClaudeMarker + ' ')) {
            $selected = ($trim.Substring(1).Trim() -split '\s+')[0]
            $typed = ($box.Text -split '\s+')[0]
            if ($selected -ne $typed) { return $null }
        }
    }
    $box.Text
}

# Waits for the input box to read exactly $Text.
function Wait-Box([string]$Text, [int]$TimeoutMs = 6000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        $box = Get-ClaudeBox
        if ($box -and $box.Text -ceq $Text) { return $true }
        Start-Sleep -Milliseconds 120
    }
    $false
}

# Enter is only ever pressed here: /help and /exit are handled by the CLI itself and never reach the model.
function Submit-Local([ValidateSet('/help', '/exit')][string]$Command) {
    Type-Unicode $Command 40
    if (-not (Wait-Box $Command)) { throw "the input box does not read '$Command'; Enter not sent" }
    if ((Get-SubmitText) -cne $Command) { throw "a suggestion menu selects another entry than '$Command'; Enter not sent" }
    Press 0x0D
}

# Clears the input box (Ctrl+U) and waits until nothing but the placeholder is left.
function Clear-Box {
    for ($round = 0; $round -lt 12; $round++) {
        $box = Get-ClaudeBox
        if ($box -and ($box.Text -eq '' -or $box.Text -match '^Try "')) { return }
        # Ctrl+E and Ctrl+U empty the line the cursor is on; Backspace on an empty line joins it to the one above.
        Press 0x45 @(0x11) 40 60; Press 0x55 @(0x11) 40 60; Press 0x08 @() 40 60
        Start-Sleep -Milliseconds 150
    }
    throw 'the input box did not empty'
}

# A confirmation dialog of the app is open: its Cancel button is on screen.
function Dialog-Open { [bool](Get-Element '取消' 'Button') }

# Types `claude` in the pwsh tab, in the empty folder, and waits for the main UI. A login, onboarding or trust screen is
# reported and stops the run: nothing here signs in or accepts a dialog.
function Start-Claude {
    Type-Line "Set-Location '$($global:ClaudeCwd)'; Clear-Host"; Wait-Prompt | Out-Null
    Type-Line 'claude'
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $deadline) {
        $text = Get-TerminalText
        foreach ($blocked in 'Select login method', 'Sign in', 'Choose the text style', 'Do you trust the files', 'Quick safety check') {
            if ($text -match [regex]::Escape($blocked)) { Remember-Kids; throw "BLOCKED: the CLI shows '$blocked' instead of the main UI; nothing was accepted" }
        }
        if ((Get-ClaudeBox) -and $text -match 'Claude Code v\d') { Start-Sleep -Milliseconds 1500; Remember-Kids; return }
        Start-Sleep -Milliseconds 250
    }
    # What the screen showed, for whoever reads the failure (the folder is outside the repository).
    [IO.File]::WriteAllText((Join-Path $global:NkClaudeShots 'start-failure.txt'), (Get-TerminalText))
    Shot-Safe (Join-Path $global:NkClaudeShots 'start-failure.png') | Out-Null
    throw 'the main UI of the CLI never appeared'
}

# Quits with Ctrl+C twice (never Enter) and waits for the shell prompt.
function Stop-Claude {
    # The second press has to come within the CLI's double-press window (about 800 ms).
    Press 0x43 @(0x11) 40 0; Start-Sleep -Milliseconds 300
    Press 0x43 @(0x11)
    Wait-Prompt | Out-Null
}

# What the tab holds of the variables that must not leak, by name only (a value could be a secret), and the values of the
# terminal contract and of the agent environment. Stops the run if anything other than what this script set is there.
function Get-TabEnvironment([string[]]$Allowed) {
    Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
    Type-Line '"HELD{" + ((Get-ChildItem Env: | Where-Object { $_.Name -match ''^(CLAUDE|ANTHROPIC|MCP_|OTEL_)'' -or $_.Name -eq ''NO_COLOR'' -or $_.Name -match ''(_TOKEN|_SECRET|_API_KEY)$|PASSWORD'' } | Sort-Object Name | ForEach-Object Name) -join '','') + "}"'
    Start-Sleep -Milliseconds 700
    $held = [regex]::Match((Get-ScreenLines) -join '', 'HELD\{([A-Z_0-9,]*)\}').Groups[1].Value -split ',' | Where-Object { $_ }
    $leaked = @($held | Where-Object { $Allowed -notcontains $_ })
    if ($leaked.Count -gt 0) { throw "the tab inherited variables it must not have: $($leaked -join ', ')" }
    Type-Line '"VALUES{FORCE_HYPERLINK=$env:FORCE_HYPERLINK;CLAUDE_CODE_NATIVE_CURSOR=$env:CLAUDE_CODE_NATIVE_CURSOR;TERM=$env:TERM;COLORTERM=$env:COLORTERM;TERM_PROGRAM=$env:TERM_PROGRAM;}"'
    Start-Sleep -Milliseconds 700
    $values = @{}
    # The last match: the echo of the command has the same shape, with the variable names where the values are.
    foreach ($pair in [regex]::Matches((Get-ScreenLines) -join '', 'VALUES\{([^}]*);\}')[-1].Groups[1].Value -split ';') {
        $name, $value = $pair -split '=', 2
        if ($name) { $values[$name] = $value }
    }
    @{ Held = @($held); Values = $values }
}

# A capture of the window, checked against the text of the screen (Compare-Ink). The screen has to be still: the text read
# before and after the capture is the same.
function Capture-Ink([string]$Name, [int]$Threshold = 40, [int]$Pane = -1, [int[]]$CaretRows = $null, $Ignore = @()) {
    $path = Join-Path $global:NkClaudeShots "$Name.png"
    for ($try = 0; $try -lt 6; $try++) {
        Start-Sleep -Milliseconds 600
        $before = @(Get-PaneInfos)
        $shot = Shot-Safe $path
        if (-not ($shot -like 'shot *')) { throw "the capture was skipped: $shot" }
        $win = Win-Rect
        $after = @(Get-PaneInfos)
        $textBefore = ($before | ForEach-Object { $_.Text }) -join "`0"; $textAfter = ($after | ForEach-Object { $_.Text }) -join "`0"
        if ($textBefore -ceq $textAfter) { break }
    }
    $info = if ($Pane -ge 0) { $after[$Pane] } else { $claudePane = Get-ClaudePane $after; if ($claudePane) { $claudePane } else { $after | Where-Object { $_.Focused } | Select-Object -First 1 } }
    $grid = $info.Grid
    $lines = $info.Lines
    $contrast = Get-Contrast $path $grid $win
    $box = Get-ClaudeBox $lines
    $caretRows = if ($null -ne $CaretRows) { $CaretRows } elseif ($box) { @($box.Row..($box.End - 1)) } else { @() }
    @{ Path = $path; Lines = $lines; Grid = $grid; Contrast = $contrast; Box = $box; Ink = (Compare-Ink $contrast $lines $Threshold $caretRows $Ignore) }
}

function Check-Ink([string]$Id, $Capture) {
    $ink = $Capture.Ink
    Claude-Check $Id ($ink.Stray.Count -eq 0 -and $ink.Missing.Count -eq 0) ('{0} rows compared with the text: {1} cells with ink where the text is blank {2}, {3} characters without ink {4}' -f $ink.Rows, $ink.Stray.Count, (($ink.Stray | Select-Object -First 4) -join '; '), $ink.Missing.Count, (($ink.Missing | Select-Object -First 4) -join '; '))
}

# The mean colour (R,G,B) of a cell of a capture.
function Get-CellColor($Capture, [int]$Row, [int]$Col) {
    $c = $Capture.Contrast; $px = $c.Pixels
    [Ink]::Mean($px.Bytes, $px.Stride, $px.W, $px.H, $c.Ox + $Col * $c.CW + 2, $c.Oy + $Row * $c.CH + 2, $c.CW - 4, $c.CH - 4)
}

# The main UI of the CLI on a settled screen.
function Check-MainUi($Cap, [string]$Id, [string]$Mode) {
    $g = $Cap.Grid; $lines = $Cap.Lines; $box = $Cap.Box
    Claude-Check "$Id.banner" ([bool]($lines -match 'Claude Code v\d+\.\d+\.\d+')) 'the start banner is on the screen'
    Claude-Check "$Id.input-box" ($null -ne $box) 'the input box (marker row between two rules) is on the screen'
    if ($box) {
        $top = Get-RowColumns $lines[$box.Row - 1]; $bottom = Get-RowColumns $lines[$box.End]
        Claude-Check "$Id.rules-full-width" ($top.Width -eq $g.Cols -and $bottom.Width -eq $g.Cols) "both rules are $($top.Width) and $($bottom.Width) cells wide on a $($g.Cols)-column grid"
        if ($Mode -eq 'fullscreen') {
            Claude-Check "$Id.pinned" ($box.Row -eq $g.Rows - 3 -and $box.End -eq $g.Rows - 2 -and $lines[$g.Rows - 1].Trim().Length -gt 0) "input box on rows $($box.Row)-$($box.End) of $($g.Rows), status line on the last row"
        }
    }
    Check-Ink "$Id.ink-matches-text" $Cap
}

# Types text into the empty input box and checks that the box reads it and that every cell of it is drawn where the
# grid puts it: ink in the cells of the characters (both halves of a wide one), the cursor in the next cell and nothing
# after it. The box row starts its text at column 2 (the marker and a no-break space).
function Check-Typed([string]$Id, [string]$Text) {
    Type-Unicode $Text 30
    $reads = Wait-Box $Text
    Claude-Check "$Id.arrives" $reads "the input box reads the typed text ($($Text.Length) UTF-16 units)"
    $cap = Capture-Ink "typed-$Id"
    if ($cap.Box) {
        $row = $cap.Box.Row; $flat = $cap.Contrast.Flat; $cols = $cap.Grid.Cols
        $layout = Get-RowColumns $Text
        $bad = @()
        $index = 0
        foreach ($rune in $Text.EnumerateRunes()) {
            $width = Get-CellWidth $rune.Value; $start = 2 + $layout.Starts[$index]; $index++
            if ($width -eq 0 -or $rune.Value -eq 0x20) { continue }
            foreach ($cell in $start..($start + $width - 1)) { if ($flat[$row * $cols + $cell] -le 12) { $bad += "'$rune' cell $cell has no ink" } }
        }
        $after = 2 + $layout.Width
        Claude-Check "$Id.cells" ($bad.Count -eq 0) "ink in every cell of the text, columns 2-$($after - 1)$(if ($bad.Count) { ': ' + ($bad -join '; ') })"
        # The cursor is a solid block: flat, so it is told from the background by its colour, against a blank cell of the same row.
        $blank = Get-CellColor $cap $row ($cols - 4); $cursor = Get-CellColor $cap $row $after; $next = Get-CellColor $cap $row ($after + 1)
        $far = { param($a, $b) [Math]::Max([Math]::Abs($a[0] - $b[0]), [Math]::Max([Math]::Abs($a[1] - $b[1]), [Math]::Abs($a[2] - $b[2]))) }
        Claude-Check "$Id.cursor-after-text" ((& $far $cursor $blank) -gt 80 -and (& $far $next $blank) -lt 25) "the cursor block is in column $after, the cell after the text (colour distance from a blank cell $(& $far $cursor $blank)), the next cell is blank (distance $(& $far $next $blank))"
    }
    Check-Ink "$Id.ink-matches-text" $cap
    Clear-Box
}

# Runs a scenario script block with the scrubbed environment. The block runs inside Run-App (see run-app.ps1).
function Run-Claude([string]$Name, [ValidateSet('fullscreen', 'classic')][string]$Mode, [scriptblock]$Script, [hashtable]$Settings = @{}, [switch]$Pattern) {
    $global:ClaudeFailures = @(); $global:ClaudeKids = @{}
    try {
        Run-App $Name $Settings $Script -NoShot -Env (Get-ClaudeEnv $Mode) -Pattern:$Pattern
    } finally {
        Stop-Kids
    }
    Finish-Claude
}

# Full-screen mode (CLAUDE_CODE_NO_FLICKER=1): the environment of the tab, the main UI pixel by pixel, typing.
function Scenario-ClaudeScreen {
    Run-Claude 'claude-screen' 'fullscreen' {
        First-Prompt
        $allowed = (Get-ClaudeEnv 'fullscreen').GetEnumerator() | Where-Object { $null -ne $_.Value -and $_.Key -match '^CLAUDE' } | ForEach-Object { $_.Key }
        $tab = Get-TabEnvironment ($allowed + 'CLAUDE_CODE_NATIVE_CURSOR')
        Claude-Check 'env.scrubbed' $true ('the tab holds only these of the scrubbed names: ' + ($tab.Held -join ', '))
        Claude-Check 'env.agent-defaults' ($tab.Values['FORCE_HYPERLINK'] -eq '1' -and $tab.Values['CLAUDE_CODE_NATIVE_CURSOR'] -eq '1') ('plain pwsh tab, default settings: FORCE_HYPERLINK=' + $tab.Values['FORCE_HYPERLINK'] + ', CLAUDE_CODE_NATIVE_CURSOR=' + $tab.Values['CLAUDE_CODE_NATIVE_CURSOR'] + ', TERM=' + $tab.Values['TERM'] + ', COLORTERM=' + $tab.Values['COLORTERM'] + ', TERM_PROGRAM=' + $tab.Values['TERM_PROGRAM'])
        Start-Claude
        $cap = Capture-Ink 'main-ui'
        Check-MainUi $cap 'ui' 'fullscreen'
        # Colours: the mascot's solid blocks are the CLI's orange, the safe-mode warning is yellow.
        $solid = $null
        for ($row = 0; $row -lt $cap.Lines.Count -and -not $solid; $row++) { $at = $cap.Lines[$row].IndexOf([char]0x2588); if ($at -ge 0) { $solid = @{ Row = $row; Col = (Get-RowColumns $cap.Lines[$row].Substring(0, $at)).Width } } }
        if ($solid) {
            $rgb = Get-CellColor $cap $solid.Row $solid.Col
            Claude-Check 'ui.mascot-orange' ($rgb[0] -ge 180 -and $rgb[0] -le 240 -and $rgb[1] -ge 90 -and $rgb[1] -le 150 -and $rgb[2] -ge 50 -and $rgb[2] -le 120) "the mascot's solid block at row $($solid.Row), column $($solid.Col) is rgb($($rgb -join ','))"
        } else { Claude-Info 'ui.mascot-orange' 'no solid block on the screen' }
        foreach ($case in @(@('ascii', 'Hello, world 123 ~!@#'), @('cjk', '你好世界ㄅㄆㄇ'), @('mixed', 'ab你好cd'), @('emoji', 'x😀y'), @('combining', "cafe$([char]0x301) ab"))) { Check-Typed $case[0] $case[1] }
        # the help screen: tabs, a list and a link, drawn with box and symbol characters
        Submit-Local '/help'
        Claude-Check 'help.opens' (Wait-Text 'Shortcuts' 8000) 'the /help screen opened'
        Start-Sleep -Milliseconds 800
        $help = Capture-Ink 'help' 40 -1 @()
        Check-Ink 'help.ink-matches-text' $help
        Press 0x1B; Start-Sleep -Milliseconds 1000
        Wait-Settled 'fullscreen' | Out-Null
        $after = Capture-Ink 'after-help'; Check-MainUi $after 'after-help' 'fullscreen'
        Stop-Claude
    }
}
# Keys in full-screen mode. Shift+Enter and Alt+Enter are tried with /help in the box (a submit would only open the help),
# the clipboard of the user is put back, and nothing here presses Enter on anything else.
function Scenario-ClaudeKeys {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Run-Claude 'claude-keys' 'fullscreen' {
            First-Prompt
            Start-Claude
            $lines = { (Get-ClaudeBox).Lines }
            $type = { param($text) Type-Unicode $text 30; if (-not (Wait-Box $text)) { throw "box does not read '$text'" } }

            Type-Unicode 'abc' 30; Wait-Box 'abc' | Out-Null; Press 0x08; Claude-Check 'keys.backspace' (Wait-Box 'ab') 'Backspace removes the last character'
            Clear-Box
            & $type 'one two three'; Press 0x08 @(0x11); Claude-Check 'keys.ctrl-backspace' (Wait-Box 'one two') 'Ctrl+Backspace removes the last word'
            Clear-Box
            & $type 'one two'; Press 0x25 @(0x11); Type-Unicode 'X' 30; Claude-Check 'keys.ctrl-left' (Wait-Box 'one Xtwo') 'Ctrl+Left moves a word, the next character lands before "two"'
            Clear-Box
            & $type 'one two'; Press 0x42 @(0x12); Type-Unicode 'X' 30; Claude-Check 'keys.alt-b' (Wait-Box 'one Xtwo') 'Alt+B moves back a word'
            Press 0x46 @(0x12); Type-Unicode 'Y' 30; Claude-Check 'keys.alt-f' (Wait-Box 'one XtwoY') 'Alt+F moves forward a word'
            Clear-Box
            & $type 'one two'; Press 0x08 @(0x12); Claude-Check 'keys.alt-backspace' (Wait-Box 'one') 'Alt+Backspace removes the last word'
            Clear-Box
            & $type 'hello'; Press 0x24; Type-Unicode 'X' 30; $atHome = Wait-Box 'Xhello'; Press 0x23; Type-Unicode 'Y' 30
            Claude-Check 'keys.home-end' ($atHome -and (Wait-Box 'XhelloY')) 'Home then X gives Xhello, End then Y gives XhelloY'
            Clear-Box
            & $type 'abcd'; Press 0x25; Press 0x25; Press 0x2E; Claude-Check 'keys.delete' (Wait-Box 'abd') 'Left twice then Delete removes the character at the cursor'
            Clear-Box

            # Shift+Enter and Alt+Enter: a newline, not a submit
            foreach ($case in @(@('shift-enter', @(0x10)), @('alt-enter', @(0x12)))) {
                & $type '/help'
                Press 0x0D $case[1]; Start-Sleep -Milliseconds 700
                $l = & $lines
                $screen = (Get-TerminalText)
                Claude-Check "keys.$($case[0])" ($l.Count -ge 2 -and $l[0] -eq '/help' -and $screen -notmatch 'Shortcuts') "the box holds $($l.Count) lines (first '$($l[0])') and the help is not open"
                Clear-Box
            }

            # Esc closes the suggestion menu and keeps the text; Tab completes
            & $type '/hel'
            $menuBefore = @(Get-ScreenLines | Where-Object { $_ -match '/help' }).Count
            Press 0x1B; Start-Sleep -Milliseconds 500
            $menuAfter = @(Get-ScreenLines | Where-Object { $_ -match '/help' }).Count
            Claude-Check 'keys.esc-closes-menu' ((Wait-Box '/hel') -and $menuBefore -ge 1 -and $menuAfter -le 0) "the menu listed /help ($menuBefore lines mention it), after Esc $menuAfter lines do and the box still reads /hel"
            Clear-Box
            & $type '/hel'; Press 0x09; Start-Sleep -Milliseconds 500
            $box = Get-ClaudeBox
            Claude-Check 'keys.tab-completes' ($box.Text -match '^/help') "Tab turned /hel into '$($box.Text)'"
            Clear-Box

            # Ctrl+C: with text it clears the box and arms the exit; the exit message goes away by itself
            & $type 'something typed'
            Press 0x43 @(0x11) 40 0; Start-Sleep -Milliseconds 500
            $armed = (Get-TerminalText) -match 'Press Ctrl-C again to exit'
            $box = Get-ClaudeBox
            Claude-Check 'keys.ctrl-c-clears' ($armed -and ($box.Text -eq '' -or $box.Text -match '^Try "')) "first Ctrl+C: box '$($box.Text)', exit message shown = $armed"
            Start-Sleep -Milliseconds 2500
            Claude-Check 'keys.ctrl-c-disarms' (-not ((Get-TerminalText) -match 'Press Ctrl-C again to exit')) 'the exit message is gone after 2.5 s and the CLI still runs'

            # Ctrl+L redraws the same screen
            & $type 'redraw test'
            # The hint row above the box changes by itself (a tip that ages out), so it is left out.
            $stable = { param($rows) ($rows | Where-Object { $_ -notmatch 'Ctrl\+Y to paste|xhigh' }) -join "`n" }
            $before = & $stable (Get-ScreenLines)
            Press 0x4C @(0x11); Start-Sleep -Milliseconds 1200
            $after = & $stable (Get-ScreenLines)
            Claude-Check 'keys.ctrl-l' ($before -ceq $after) 'Ctrl+L leaves the same screen'
            Clear-Box

            # Paste: bracketed, so a multi-line text stays in the box
            Set-Clipboard -Value "line one`nline two`nline three"
            Press 0x56 @(0x11); Start-Sleep -Milliseconds 900
            $l = & $lines
            $dialog = [int](Dialog-Open)
            Claude-Check 'paste.ctrl-v' ($dialog -eq 0 -and $l.Count -eq 3 -and $l[0] -eq 'line one' -and $l[2] -eq 'line three') "Ctrl+V: $($l.Count) lines in the box, no dialog ($dialog), nothing submitted"
            Clear-Box
            Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 900
            $l = & $lines
            Claude-Check 'paste.ctrl-shift-v' (-not (Dialog-Open) -and $l.Count -eq 3 -and $l[1] -eq 'line two') "Ctrl+Shift+V: $($l.Count) lines in the box"
            Clear-Box
            # A big paste asks first even to a program with bracketed paste
            $big = ((1..60 | ForEach-Object { 'a line of pasted text with some length to it ' + $_ + ('.' * 40) }) -join "`n")
            Set-Clipboard -Value $big
            Press 0x56 @(0x11); Start-Sleep -Milliseconds 1200
            Shot-Safe (Join-Path $global:NkClaudeShots 'paste-big-dialog.png') | Out-Null
            Claude-Check 'paste.big-asks' (Dialog-Open) "a paste of $($big.Length) bytes in 60 lines asks first"
            Press 0x1B; Start-Sleep -Milliseconds 800
            $box = Get-ClaudeBox
            Claude-Check 'paste.big-cancel' ($box.Text -eq '' -or $box.Text -match '^Try "') "after Esc the box is empty ('$($box.Text)')"
            Press 0x56 @(0x11); Start-Sleep -Milliseconds 1200
            Click-Name '貼上' -Class 'Button'; Start-Sleep -Milliseconds 1200
            $box = Get-ClaudeBox
            Claude-Check 'paste.big-confirm' ($box.Text -match 'Pasted text #1 \+\d+ lines') "after Paste the box reads '$($box.Text)'"
            Shot-Safe (Join-Path $global:NkClaudeShots 'paste-big-done.png') | Out-Null
            Clear-Box
            Stop-Claude
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}
# --- the window ------------------------------------------------------------------------------------------------------------

function Get-OuterRect { $r = New-Object D+RECT; [void][D]::GetWindowRect($script:Hwnd, [ref]$r); $r }

# Waits until the pane's grid has these dimensions (the pane logs one line per change).
function Wait-Grid([int]$Cols, [int]$Rows, [int]$TimeoutMs = 4000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        $line = Select-String -Path $script:LogFile -Pattern 'pane \d+ grid (\d+)x(\d+) cells' | Select-Object -Last 1
        if ($line -and [int]$line.Matches[0].Groups[1].Value -eq $Cols -and [int]$line.Matches[0].Groups[2].Value -eq $Rows) { return $true }
        Start-Sleep -Milliseconds 100
    }
    $false
}

# Sets the outer size of the window in pixels, keeping its position.
function Set-WindowPixels([int]$Width, [int]$Height) {
    $o = Get-OuterRect
    [void][P]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, $o.Left, $o.Top, $Width, $Height, 0x0014)
}

# Makes the window as many rows taller or shorter as it takes for the focused pane to hold $Rows rows.
function Set-WindowRows([int]$Rows) {
    $g = Get-Grid; $o = Get-OuterRect
    Set-WindowPixels ($o.Right - $o.Left) (($o.Bottom - $o.Top) + ($Rows - $g.Rows) * $g.CH)
    if (-not (Wait-Grid $g.Cols $Rows)) { throw "the pane never reached $Rows rows" }
    Start-Sleep -Milliseconds 600
}

# Mouse in full-screen mode: the wheel, a click in the box, the CLI's own selection and Shift+drag (the terminal's), a
# link with Ctrl+click, the right button, and nothing echoed into the box.
function Scenario-ClaudeMouse {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Run-Claude 'claude-mouse' 'fullscreen' {
            First-Prompt
            Check-WheelBytes
            Start-Claude
            Check-ClaudeWheel
            $g = Get-Grid

            # Click: the caret goes where the cell was clicked
            Type-Unicode 'hello world' 30; Wait-Box 'hello world' | Out-Null
            $row = (Get-ClaudeBox).Row
            Click-Cell $g 8.25 $row
            Type-Unicode 'Z' 30
            Claude-Check 'mouse.click-places-caret' (Wait-Box 'hello Zworld') "a click on the left half of column 8 of the box row put Z before 'world' ('$((Get-ClaudeBox).Text)')"
            Clear-Box

            # The CLI's own selection: a drag over the banner copies it
            $banner = Find-Cell 'Claude Code v'
            Set-Clipboard -Value 'SENTINEL'
            Drag-Cells $g ($banner.Col + 0.25) $banner.Row ($banner.Col + 12.75) $banner.Row
            Start-Sleep -Milliseconds 900
            $copied = (Get-Clipboard -Raw) -replace "`r`n", "`n"
            Shot-Safe (Join-Path $global:NkClaudeShots 'drag-select.png') | Out-Null
            Claude-Check 'mouse.cli-selection-copies' ($copied -ceq 'Claude Code v') "a plain drag over 'Claude Code v' left '$copied' on the clipboard"
            # Shift+drag is the terminal's own selection and a copy command takes it
            Click-Cell $g 40 ($g.Rows - 8)
            With-Keys @(0x10) { Drag-Cells $g ($banner.Col + 0.25) ($banner.Row + 1) ($banner.Col + 9.75) ($banner.Row + 1) }
            $native = Copy-Selection
            Claude-Check 'mouse.shift-drag-selects-natively' ($native -ceq 'Sonnet 5.5') "Shift+drag over the model name and Ctrl+Shift+C put '$native' on the clipboard"
            # Ctrl+C copies while the terminal has a selection and is the CLI's interrupt otherwise
            With-Keys @(0x10) { Drag-Cells $g ($banner.Col + 0.25) ($banner.Row + 1) ($banner.Col + 9.75) ($banner.Row + 1) }
            Set-Clipboard -Value 'SENTINEL'
            # the CLI's exit message lasts about 800 ms: it is read 400 ms after the key
            Press 0x43 @(0x11) 40 0; Start-Sleep -Milliseconds 400
            $smart = (Get-Clipboard -Raw) -replace "`r`n", "`n"
            $armed = (Get-TerminalText) -match 'Press Ctrl-C again'
            Claude-Check 'mouse.ctrl-c-copies-a-selection' ($smart -ceq 'Sonnet 5.5' -and -not $armed) "Ctrl+C with a Shift+drag selection put '$smart' on the clipboard; the CLI's exit armed = $armed"
            Click-Cell $g 40 ($g.Rows - 8)
            Press 0x43 @(0x11) 40 0; Start-Sleep -Milliseconds 400
            Claude-Check 'mouse.ctrl-c-without-selection-is-the-clis' ((Get-TerminalText) -match 'Press Ctrl-C again') 'Ctrl+C without a selection reaches the CLI (it arms its exit)'
            Start-Sleep -Milliseconds 2500

            # Ctrl+click on a link of the help (FORCE_HYPERLINK=1 from the default agent environment makes the URL an OSC 8 link)
            Submit-Local '/help'
            Claude-Check 'help.opens' (Wait-Text 'Shortcuts' 8000) 'the /help screen opened'
            Start-Sleep -Milliseconds 600
            $url = $null
            foreach ($l in Get-ScreenLines) { if ($l -match '(https://code\.claude\.com\S*)') { $url = $Matches[1] } }
            if ($url) {
                $at = Find-Cell $url
                $p = Cell-Pt $g ($at.Col + 6) $at.Row
                Move-To $p.X $p.Y; Start-Sleep -Milliseconds 150; Move-To ($p.X + 2) $p.Y; Start-Sleep -Milliseconds 700
                Claude-Check 'link.hand-cursor' (Cursor-IsHand) "the pointer over $url in the help is the hand"
                # A plain Ctrl+click is the CLI's: it tracks the mouse, gets the click and opens the link itself (it started the
                # browser in an earlier run), which nothing here can stop. Ctrl+Shift+click is the terminal's own.
                $mark = Log-Mark
                $known = @(Get-CimInstance Win32_Process | ForEach-Object { $_.ProcessId })
                With-Keys @(0x11, 0x10) { Click-At $p.X $p.Y 'left' 700 }
                Start-Sleep -Milliseconds 1500
                $started = @(Get-CimInstance Win32_Process | Where-Object { $known -notcontains $_.ProcessId } | ForEach-Object { $_.Name })
                Claude-Info 'link.processes-started' ("processes that appeared within 2 s of the click: " + $(if ($started) { $started -join ', ' } else { 'none' }))
                $lines = @(Log-Since $mark | Where-Object { $_ -match 'opening|not opened|link refused' })
                Claude-Check 'link.ctrl-shift-click' ([bool]($lines -match [regex]::Escape($url))) ("Ctrl+Shift+click: " + (($lines | ForEach-Object { $_.Trim() }) -join ' / '))
            } else { Claude-Check 'link.in-help' $false 'no https://code.claude.com address on the help screen' }
            Press 0x1B; Start-Sleep -Milliseconds 700

            # The right button goes to the CLI, not to Nekominal's menu. The CLI pastes the clipboard by itself on a right
            # click, a moment later (it reads the clipboard through a helper process).
            Set-Clipboard -Value 'RIGHT-CLICK-CLIPBOARD'
            Click-Cell $g 40 ($g.Rows - 8) 1 'right'
            Start-Sleep -Milliseconds 700
            $menu = [bool](Get-Element '貼上 (Ctrl+V)')
            Wait-Box 'RIGHT-CLICK-CLIPBOARD' 4000 | Out-Null
            $box = Get-ClaudeBox
            Claude-Check 'mouse.right-click' (-not $menu) "right click: no app menu; the CLI answered with a paste of its own (box '$($box.Text)')"
            Clear-Box

            # Nothing the mouse did arrived as text
            $stray = [regex]::Matches((Get-TerminalText), '\[<\d+;\d+;\d+[Mm]').Count
            $box = Get-ClaudeBox
            Claude-Check 'mouse.no-stray-bytes' ($stray -eq 0 -and ($box.Text -eq '' -or $box.Text -match '^Try "')) "$stray mouse reports on the screen, the box reads '$($box.Text)'"
            Stop-Claude
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}
# --- idle and exit ---------------------------------------------------------------------------------------------------------

# CPU time of the app and of what runs below it, and the GPU use of the app, over a time in which nothing touches the
# window (no UI Automation call, no input).
function Measure-Idle([int]$Seconds = 10) {
    $app = $script:App; $app.Refresh()
    $cpu = @{}
    $watched = @($app.Id) + @(Get-Descendants $app.Id | ForEach-Object { [int]$_.ProcessId })
    foreach ($id in $watched) { $proc = Get-Process -Id $id -ErrorAction SilentlyContinue; if ($proc) { $cpu[$id] = @{ Name = $proc.ProcessName; T0 = $proc.TotalProcessorTime.TotalMilliseconds } } }
    $gpu = $null
    try {
        $samples = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples $Seconds -ErrorAction Stop)
        $gpu = ($samples | ForEach-Object { ($_.CounterSamples | Where-Object { $_.InstanceName -match "pid_$($app.Id)_" } | Measure-Object -Property CookedValue -Sum).Sum } | Measure-Object -Maximum -Average)
    } catch { Start-Sleep -Seconds $Seconds }
    $report = @()
    foreach ($id in $cpu.Keys) {
        $proc = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($proc) { $report += [pscustomobject]@{ Name = $cpu[$id].Name; Pid = $id; CpuPercent = [Math]::Round(($proc.TotalProcessorTime.TotalMilliseconds - $cpu[$id].T0) / ($Seconds * 10), 2) } }
    }
    @{ Cpu = $report; Gpu = $gpu }
}

# The terminal after the CLI is gone, checked with a stand-in program that reads stdin raw and turns no mode on (see
# input-probe.js): any report of the pointer, any escape sequence for a key that has a plain byte, means a mode the CLI
# set is still on. Then the shell integration: the previous-prompt jump needs the OSC 133 marks the shell writes.
function Check-AfterExit([string]$Id) {
    $g = Get-Grid
    $lines = Get-ScreenLines
    $last = ($lines | Where-Object { $_.Trim().Length -gt 0 } | Select-Object -Last 1)
    Claude-Check "$Id.prompt-back" ($last -match '^PS [^\r\n]*>' -and (Get-TerminalText) -notmatch [regex]::Escape($global:ClaudeMarker) -and (Get-TerminalText) -notmatch 'Claude Code v') "the screen ends in the shell prompt ('$last') and nothing of the CLI is left"
    # The cursor is back: a cell next to the end of the prompt has the colour of a cursor block.
    $cap = Capture-Ink "after-$Id"
    $row = ($lines.Count - 1); while ($row -gt 0 -and $lines[$row].Trim().Length -eq 0) { $row-- }
    $at = (Get-RowColumns $lines[$row]).Width
    $blank = Get-CellColor $cap $row ($g.Cols - 4); $near = @($at, ($at + 1) | ForEach-Object { Get-CellColor $cap $row $_ })
    $dist = ($near | ForEach-Object { [Math]::Max([Math]::Abs($_[0] - $blank[0]), [Math]::Max([Math]::Abs($_[1] - $blank[1]), [Math]::Abs($_[2] - $blank[2]))) } | Measure-Object -Maximum).Maximum
    Claude-Check "$Id.cursor-visible" ($dist -gt 60) "a cursor block is drawn after the prompt (colour distance $dist from a blank cell)"
    Claude-Check "$Id.ink-matches-text" ($cap.Ink.Stray.Count -eq 0) "$($cap.Ink.Stray.Count) cells with ink where the text is blank $(($cap.Ink.Stray | Select-Object -First 3) -join '; ')"

    if (Get-Command node -ErrorAction SilentlyContinue) {
        Type-Line "node `"$(Join-Path $PSScriptRoot 'input-probe.js')`" `"`" `"`" 6500"
        Start-Sleep -Milliseconds 1500
        $p = Cell-Pt $g 20 5
        Move-To $p.X $p.Y; Start-Sleep -Milliseconds 150; Move-To ($p.X + 30) ($p.Y + 20); Start-Sleep -Milliseconds 150
        [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 80; [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 150
        Mouse-Wheel 1; Mouse-Wheel -1
        Press 0x1B; Press 0x0D @(0x10)
        $text = ''
        $deadline = [DateTime]::UtcNow.AddSeconds(12)
        while ([DateTime]::UtcNow -lt $deadline -and $text -notmatch 'got:\[.*\]') { Start-Sleep -Milliseconds 300; $text = (Get-ScreenLines) -join '' }
        $got = [regex]::Match($text, 'got:\[(.*?)\]').Groups[1].Value
        Claude-Check "$Id.mouse-reporting-off" ($got -notmatch '<ESC>\[<') "move, click and wheel over the pane reached a plain program as '$($got -replace '[\r\n]', '')'"
        Claude-Check "$Id.keyboard-flags-popped" ($got -notmatch '<ESC>\[\d+(;\d+)?u') "Esc and Shift+Enter arrive as plain bytes (kitty keyboard flags are off again)"
        Wait-Prompt | Out-Null
        Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
    } else { Claude-Info "$Id.mouse-reporting-off" 'node is not installed: not tested' }

    # bracketed paste is off again: a paste of several lines asks first
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Set-Clipboard -Value "one`ntwo"
        Press 0x56 @(0x11); Start-Sleep -Milliseconds 900
        $asked = Dialog-Open
        if ($asked) { Press 0x1B; Start-Sleep -Milliseconds 500 }
        Claude-Check "$Id.bracketed-paste-off" $asked 'a paste of two lines into the shell asks first (the CLI turned bracketed paste off)'
    } finally { if ($null -ne $saved) { Set-Clipboard -Value $saved } }

    # main screen with a scrollback, and prompt marks
    Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
    foreach ($n in 1..3) { Type-Line ('"cmd-{0}"; 1..30 | ForEach-Object {{ "pad-$_" }}' -f $n); Wait-Prompt | Out-Null }
    Chord @(0x11, 0x10) 0x21; Start-Sleep -Milliseconds 500
    $scrolled = (Get-ScreenLines)[0] -notmatch '^PS [^\r\n]*>$'
    Chord @(0x11, 0x10) 0x23; Start-Sleep -Milliseconds 500
    Claude-Check "$Id.scrollback-works" $scrolled 'Ctrl+Shift+PageUp scrolls the main screen history (the CLI left the alternate screen)'
    Chord @(0x11, 0x12) 0x26; Start-Sleep -Milliseconds 600
    Claude-Check "$Id.prompt-marks" ((Get-ScreenLines)[0] -match '^PS [^\r\n]*> "cmd-\d"') "Ctrl+Alt+Up puts an earlier prompt on the top row ('$((Get-ScreenLines)[0])')"
    Chord @(0x11, 0x10) 0x23; Start-Sleep -Milliseconds 400
}

# Idle, then /exit, then Ctrl+C twice: the CLI is started twice in the same tab.
function Scenario-ClaudeExit {
    Run-Claude 'claude-exit' 'fullscreen' {
        First-Prompt
        Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
        Start-Claude
        Start-Sleep -Seconds 3
        $idle = Measure-Idle 10
        $app = ($idle.Cpu | Where-Object { $_.Pid -eq $script:App.Id }).CpuPercent
        $others = ($idle.Cpu | Where-Object { $_.Pid -ne $script:App.Id } | ForEach-Object { "$($_.Name) $($_.CpuPercent) %" }) -join ', '
        Claude-Check 'idle.cpu' ($app -lt 3) "10 s idle with the CLI open: nekominal.exe $app % of one core; below it: $others"
        if ($idle.Gpu) { Claude-Info 'idle.gpu' ("GPU engine use of nekominal.exe over 10 s: average {0:N2} %, peak {1:N2} %" -f $idle.Gpu.Average, $idle.Gpu.Maximum) } else { Claude-Info 'idle.gpu' 'no GPU counter for the process' }
        # The screen does not change when nothing happens: two captures 3 s apart are identical.
        $a = Join-Path $global:NkClaudeShots 'idle-a.png'; $b = Join-Path $global:NkClaudeShots 'idle-b.png'
        $paneBefore = @(Get-PaneInfos)[0]
        Shot-Safe $a | Out-Null; Start-Sleep -Seconds 3; Shot-Safe $b | Out-Null
        $paneAfter = @(Get-PaneInfos)[0]
        $still = Compare-ShotCells $a $b $paneAfter.Grid (Win-Rect) $paneBefore.Lines $paneAfter.Lines
        Claude-Check 'idle.screen-still' ($still.Differing.Count -eq 0) "two captures of the window 3 s apart: $($still.Differing.Count) cells hold different ink in the rows whose text did not change $(($still.Differing | Select-Object -First 6) -join ' '); rows that changed text: $($still.ChangedRows -join ',')"

        Submit-Local '/exit'
        Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 800
        Check-AfterExit 'exit-slash'

        Start-Claude
        Stop-Claude
        Start-Sleep -Milliseconds 800
        Check-AfterExit 'exit-ctrl-c'
    }
}
# --- panes -----------------------------------------------------------------------------------------------------------------

# Every terminal pane of the visible tab, left to right and top to bottom: its grid (the pixel rectangle of its control, and
# the cell size every pane shares), its text and whether it has the keyboard focus. The grid is the one the pane logs
# (columns and rows are the whole cells that fit), worked out here because the log does not say which pane wrote a line.
function Get-PaneInfos {
    $cell = Select-String -Path $script:LogFile -Pattern 'pane \d+ grid \d+x\d+ cells of (\d+)x(\d+) px' | Select-Object -Last 1
    if (-not $cell) { throw 'the panes wrote no grid line' }
    $cw = [int]$cell.Matches[0].Groups[1].Value; $ch = [int]$cell.Matches[0].Groups[2].Value
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
    $infos = @()
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        $r = $e.Current.BoundingRectangle
        if ($r.Width -le 0) { continue }
        $text = ''
        try { $text = $e.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern).DocumentRange.GetText(-1) } catch { }
        $grid = @{ X = $r.X; Y = $r.Y; W = $r.Width; H = $r.Height; Cols = [int][Math]::Floor($r.Width / $cw); Rows = [int][Math]::Floor($r.Height / $ch); CW = $cw; CH = $ch }
        $infos += [pscustomobject]@{ Grid = $grid; Text = $text; Lines = @($text -split "`n" | ForEach-Object { $_.TrimEnd("`r") }); Focused = $e.Current.HasKeyboardFocus; X = $r.X; Y = $r.Y }
    }
    @($infos | Sort-Object { [int]$_.X }, { [int]$_.Y })
}

# Waits for a shell prompt in the focused pane when the prompt may wrap over two rows (a pane of half the width): the text
# ends in '>' and has stopped changing.
function Wait-ShellPrompt([int]$TimeoutMs = 20000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs); $last = $null; $stable = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        $t = Get-TerminalText
        if ($t -and $t.TrimEnd().EndsWith('>')) { if ($t -ceq $last) { $stable++ } else { $stable = 0 }; if ($stable -ge 2) { return } } else { $stable = 0 }
        $last = $t
        Start-Sleep -Milliseconds 200
    }
    throw 'the shell prompt never appeared'
}

# The pane that shows the CLI's input box.
function Get-ClaudePane($Infos) { $Infos | Where-Object { Get-ClaudeBox $_.Lines } | Select-Object -First 1 }

# --- resizing --------------------------------------------------------------------------------------------------------------

Add-Type -AssemblyName System.Windows.Forms

# The work area of the screen the window is on, in pixels, and the DPI scale of the window (it cannot be changed from here).
function Get-WorkArea { [System.Windows.Forms.Screen]::FromHandle($script:Hwnd).WorkingArea }
function Get-WindowScale { [Ink]::GetDpiForWindow($script:Hwnd) / 96.0 }

# Sizes the window takes in a storm, as fractions of the work area: narrow to wide, small to large, tall and short.
$global:StormSizes = @(@(0.35, 0.5), @(0.9, 0.9), @(0.5, 0.4), @(0.7, 0.8), @(0.3, 0.6), @(0.95, 0.5), @(0.4, 0.9), @(0.6, 0.35), @(0.85, 0.85), @(0.45, 0.55),
    @(0.32, 0.4), @(0.8, 0.95), @(0.55, 0.6), @(0.25, 0.45), @(0.9, 0.6), @(0.38, 0.8), @(0.7, 0.45), @(0.5, 0.7))

# Resizes the window through $Sizes (fractions of the work area) with a pause of 30 to 60 ms between two sizes, keeping the
# top left corner on the screen. Returns how long the storm took.
function Invoke-ResizeStorm($Sizes = $global:StormSizes, [int]$Seed = 7) {
    $rng = [System.Random]::new($Seed)
    $work = Get-WorkArea
    $o = Get-OuterRect
    $left = [Math]::Max($o.Left, $work.Left); $top = [Math]::Max($o.Top, $work.Top)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    foreach ($size in $Sizes) {
        $width = [Math]::Min([int]($work.Width * $size[0]), $work.Right - $left)
        $height = [Math]::Min([int]($work.Height * $size[1]), $work.Bottom - $top)
        [void][P]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, $left, $top, [Math]::Max($width, 640), [Math]::Max($height, 400), 0x0014)
        Start-Sleep -Milliseconds $rng.Next(30, 61)
    }
    $watch.Elapsed.TotalMilliseconds
}

# Resizes the window the way a drag of its edge does: many small steps, down to a small size and back to the one it had.
function Invoke-DragStorm([int]$Steps = 50, [int]$StepMs = 16) {
    $work = Get-WorkArea; $o = Get-OuterRect
    $w0 = $o.Right - $o.Left; $h0 = $o.Bottom - $o.Top
    $w1 = [int]($work.Width * 0.3); $h1 = [int]($work.Height * 0.4)
    $watch = [Diagnostics.Stopwatch]::StartNew(); $count = 0
    foreach ($phase in @(@($w0, $h0, $w1, $h1), @($w1, $h1, $w0, $h0))) {
        for ($i = 1; $i -le $Steps; $i++) {
            $width = [int]($phase[0] + ($phase[2] - $phase[0]) * $i / $Steps); $height = [int]($phase[1] + ($phase[3] - $phase[1]) * $i / $Steps)
            [void][P]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, $o.Left, $o.Top, [Math]::Max($width, 640), [Math]::Max($height, 400), 0x0014)
            $count++; Start-Sleep -Milliseconds $StepMs
        }
    }
    @{ Count = $count; Ms = $watch.Elapsed.TotalMilliseconds }
}

# Waits until the screen is the CLI's main UI at the size of the grid (the box and its rules are where the grid says) and the
# text has not changed for 600 ms. Returns the milliseconds from the call, or -1.
function Wait-Settled([string]$Mode = 'fullscreen', [int]$TimeoutMs = 15000) {
    $watch = [Diagnostics.Stopwatch]::StartNew(); $last = $null; $stableSince = $null
    while ($watch.ElapsedMilliseconds -lt $TimeoutMs) {
        $pane = Get-ClaudePane (Get-PaneInfos)
        if (-not $pane) { Start-Sleep -Milliseconds 100; continue }
        $text = $pane.Text
        $lines = $pane.Lines
        $box = Get-ClaudeBox $lines
        $g = $pane.Grid
        $fits = $box -and (Get-RowColumns $lines[$box.Row - 1]).Width -eq $g.Cols -and ($Mode -ne 'fullscreen' -or ($box.End -eq $g.Rows - 2))
        if ($fits -and $text -ceq $last) { if (-not $stableSince) { $stableSince = $watch.ElapsedMilliseconds } elseif ($watch.ElapsedMilliseconds - $stableSince -ge 600) { return [int]$stableSince } }
        else { $stableSince = $null }
        $last = $text
        Start-Sleep -Milliseconds 100
    }
    -1
}

# The text of a screen with what changes by itself left out (the placeholder of the box, the hint above it).
function Get-StableLines($Lines) { @($Lines | ForEach-Object { if ($_ -match '^(\u276F\s*)Try ".*"$') { $Matches[1] + 'Try "..."' } elseif ($_ -match 'Ctrl\+Y to paste|xhigh|You''ve used') { '' } else { $_ } }) }

# Compares two captures of the same CLI at the same size: the same text, and the same cells inked, apart from the box row.
function Compare-Screens($A, $B, [string]$Id, [string]$AlignOn = '') {
    # Classic mode prints below whatever the shell had on screen, so both screens are compared from the line that matches $AlignOn.
    $offA = 0; $offB = 0
    if ($AlignOn) {
        $offA = [Math]::Max(0, @($A.Lines | ForEach-Object { $_ -match $AlignOn }).IndexOf($true)); $offB = [Math]::Max(0, @($B.Lines | ForEach-Object { $_ -match $AlignOn }).IndexOf($true))
    }
    $textA = @(Get-StableLines $A.Lines | Select-Object -Skip $offA); $textB = @(Get-StableLines $B.Lines | Select-Object -Skip $offB)
    $changed = @(0..([Math]::Min($textA.Count, $textB.Count) - 1) | Where-Object { $textA[$_] -cne $textB[$_] })
    Claude-Check "$Id.same-text" ($changed.Count -eq 0) $(if ($changed.Count) { "rows differ: " + (($changed | Select-Object -First 4 | ForEach-Object { "r${_} [$($textA[$_])] vs [$($textB[$_])]" }) -join '; ') } else { 'the text of both screens is the same' })
    $cols = $A.Grid.Cols; $rows = $A.Grid.Rows - [Math]::Max($offA, $offB); $diff = @()
    if ($A.Grid.Cols -eq $B.Grid.Cols -and $A.Grid.Rows -eq $B.Grid.Rows) {
        for ($row = 0; $row -lt $rows; $row++) {
            # the box row has a placeholder that changes, the hint beside the status line a tip that ages out
            if ($A.Box -and $row + $offA -eq $A.Box.Row) { continue }
            if ($A.Lines[$row + $offA] -match 'Ctrl\+Y to paste|xhigh' -or $B.Lines[$row + $offB] -match 'Ctrl\+Y to paste|xhigh') { continue }
            for ($col = 0; $col -lt $cols; $col++) {
                $inkA = $A.Contrast.Flat[($row + $offA) * $cols + $col] -gt 40; $inkB = $B.Contrast.Flat[($row + $offB) * $cols + $col] -gt 40
                if ($inkA -ne $inkB) { $diff += "r$row c$col" }
            }
        }
        Claude-Check "$Id.same-ink" ($diff.Count -eq 0) "$($diff.Count) cells inked in one screen and not in the other (box row left out) $(($diff | Select-Object -First 6) -join ' ')"
    } else { Claude-Check "$Id.same-ink" $false "the grids differ: $($A.Grid.Cols)x$($A.Grid.Rows) and $($B.Grid.Cols)x$($B.Grid.Rows)" }
}

# Full-screen mode through resizes in the real window: a storm of sizes, maximize and restore, half-screen snaps. After each
# the screen is checked against its own text (no fragments), and after the storm against a fresh run of the CLI at the
# same size.
function Scenario-ClaudeResize {
    Run-Claude 'claude-resize' 'fullscreen' {
        First-Prompt
        Start-Claude
        $scale = Get-WindowScale
        Claude-Info 'resize.dpi' ("the window is at {0:N0} % DPI scale (cannot be changed from here); work area {1}x{2} px" -f ($scale * 100), (Get-WorkArea).Width, (Get-WorkArea).Height)
        $grid0 = Get-Grid
        $logMark = Log-Mark

        $took = Invoke-ResizeStorm
        $settled = Wait-Settled 'fullscreen'
        Claude-Check 'storm.settles' ($settled -ge 0) ("{0} resizes in {1:N0} ms; after the last one the CLI's screen fit the grid and was still after {2} ms" -f $global:StormSizes.Count, $took, $settled)
        $grids = @(Log-Since $logMark | Where-Object { $_ -match 'pane \d+ grid' }).Count
        $configured = @(Log-Since $logMark | Where-Object { $_ -match 'configured ' }).Count
        Claude-Info 'storm.work' "$grids grid changes and $configured swap chain reconfigurations were logged during and after the storm"
        $a = Capture-Ink 'storm-final'
        Check-MainUi $a 'storm' 'fullscreen'
        Claude-Info 'storm.grid' "final grid $($a.Grid.Cols)x$($a.Grid.Rows) (started at $($grid0.Cols)x$($grid0.Rows))"

        # a fresh run of the CLI at this size
        Stop-Claude
        Start-Claude
        Wait-Settled 'fullscreen' | Out-Null
        $b = Capture-Ink 'storm-fresh'
        Check-MainUi $b 'fresh' 'fullscreen'
        Compare-Screens $a $b 'storm.vs-fresh'

        # maximize, restore, half-screen snaps (what the snap layouts do: the work area's left and right halves)
        [void][D]::ShowWindow($script:Hwnd, 3); Wait-Settled 'fullscreen' | Out-Null
        $max = Capture-Ink 'maximized'; Check-MainUi $max 'maximized' 'fullscreen'
        Claude-Info 'resize.maximized' "maximized grid $($max.Grid.Cols)x$($max.Grid.Rows)"
        [void][D]::ShowWindow($script:Hwnd, 9); Wait-Settled 'fullscreen' | Out-Null
        $restored = Capture-Ink 'restored'; Check-MainUi $restored 'restored' 'fullscreen'
        # a drag of the edge: 100 small steps down and back to the size it had
        $drag = Invoke-DragStorm
        $settled = Wait-Settled 'fullscreen'
        Claude-Check 'drag.settles' ($settled -ge 0) ("{0} resizes in {1:N0} ms ({2:N0} ms each), the screen was settled {3} ms after the last one" -f $drag.Count, $drag.Ms, ($drag.Ms / $drag.Count), $settled)
        $afterDrag = Capture-Ink 'after-drag'; Check-MainUi $afterDrag 'drag' 'fullscreen'
        Compare-Screens $restored $afterDrag 'drag.vs-before'
        # the font size: every step changes the cell and so the grid; back to the default is the screen it was
        foreach ($i in 1..2) { Press 0xBB @(0x11) }
        Wait-Settled 'fullscreen' | Out-Null
        $bigger = Capture-Ink 'font-bigger'; Check-MainUi $bigger 'font-bigger' 'fullscreen'
        foreach ($i in 1..4) { Press 0xBD @(0x11) }
        Wait-Settled 'fullscreen' | Out-Null
        $smaller = Capture-Ink 'font-smaller'; Check-MainUi $smaller 'font-smaller' 'fullscreen'
        Press 0x30 @(0x11)
        Wait-Settled 'fullscreen' | Out-Null
        $reset = Capture-Ink 'font-reset'; Check-MainUi $reset 'font-reset' 'fullscreen'
        Claude-Info 'resize.font' "grid $($bigger.Grid.Cols)x$($bigger.Grid.Rows) two steps bigger, $($smaller.Grid.Cols)x$($smaller.Grid.Rows) two steps smaller, $($reset.Grid.Cols)x$($reset.Grid.Rows) after Ctrl+0"
        Claude-Check 'font.ordering' ($bigger.Grid.Cols -lt $restored.Grid.Cols -and $smaller.Grid.Cols -gt $restored.Grid.Cols) 'a bigger font gives fewer columns and a smaller one more'
        Compare-Screens $restored $reset 'font-reset.vs-before'
        $work = Get-WorkArea
        foreach ($half in @(@('left-half', $work.Left), @('right-half', ($work.Left + [int]($work.Width / 2))))) {
            [void][P]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, $half[1], $work.Top, [int]($work.Width / 2), $work.Height, 0x0014)
            Wait-Settled 'fullscreen' | Out-Null
            $snap = Capture-Ink "snap-$($half[0])"; Check-MainUi $snap "snap-$($half[0])" 'fullscreen'
            Claude-Info "resize.$($half[0])" "grid $($snap.Grid.Cols)x$($snap.Grid.Rows)"
        }
        # F11: the window fills the screen and comes back
        Press 0x7A; Start-Sleep -Milliseconds 1200
        Wait-Settled 'fullscreen' | Out-Null
        $full = Capture-Ink 'window-fullscreen'; Check-MainUi $full 'window-fullscreen' 'fullscreen'
        Claude-Info 'resize.f11' "window full screen: grid $($full.Grid.Cols)x$($full.Grid.Rows)"
        Press 0x7A; Start-Sleep -Milliseconds 1200
        Wait-Settled 'fullscreen' | Out-Null
        $leftF11 = Capture-Ink 'window-fullscreen-left'; Check-MainUi $leftF11 'window-fullscreen-left' 'fullscreen'
        Stop-Claude
    }
}
# The whole buffer of the focused pane (scrollback and screen) as text: Select all, then copy (the sentinel first, so a copy
# that did nothing cannot pass). The click afterwards ends the selection.
function Get-BufferText {
    Chord @(0x11, 0x10) 0x41; Start-Sleep -Milliseconds 400
    $text = Copy-Selection
    $g = Get-Grid
    Click-Cell $g 3 ($g.Rows - 1)
    $text
}

# Classic mode (CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1): the CLI prints into the main screen, the history is the terminal's,
# the mouse is the terminal's. The storm narrows and widens the window; afterwards the buffer holds one copy of the banner
# (commit 38f4082) and the screen is what a fresh run shows at that size.
function Scenario-ClaudeClassic {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Run-Claude 'claude-classic' 'classic' {
            First-Prompt
            $allowed = (Get-ClaudeEnv 'classic').GetEnumerator() | Where-Object { $null -ne $_.Value -and $_.Key -match '^CLAUDE' } | ForEach-Object { $_.Key }
            $tab = Get-TabEnvironment ($allowed + 'CLAUDE_CODE_NATIVE_CURSOR')
            Claude-Check 'env.scrubbed' $true ('the tab holds only these of the scrubbed names: ' + ($tab.Held -join ', '))
            Start-Claude
            $cap = Capture-Ink 'classic-main-ui'
            Check-MainUi $cap 'classic-ui' 'classic'
            # the CLI does not track the mouse in classic mode: a plain drag is the terminal's own selection
            $g = Get-Grid
            $banner = Find-Cell 'Claude Code v'
            Click-Cell $g 40 ($g.Rows - 3)
            Drag-Cells $g ($banner.Col + 0.25) $banner.Row ($banner.Col + 12.75) $banner.Row
            $native = Copy-Selection
            Claude-Check 'classic.plain-drag-selects-natively' ($native -ceq 'Claude Code v') "a plain drag over 'Claude Code v' and Ctrl+Shift+C put '$native' on the clipboard"
            Click-Cell $g 40 ($g.Rows - 3)

            Check-Typed 'classic-typed' 'ab你好😀'
            # the wheel is the terminal's: with no history yet it moves nothing and nothing reaches the CLI
            $g = Get-Grid
            Move-To ([int]($g.X + $g.W / 2)) ([int]($g.Y + $g.H / 2)); Start-Sleep -Milliseconds 300
            $before = (Get-ScreenLines) -join "`n"
            Mouse-Wheel 3; Mouse-Wheel -3
            $box = Get-ClaudeBox
            Claude-Check 'classic.wheel-not-reported' (((Get-ScreenLines) -join "`n") -ceq $before -and ($box.Text -eq '' -or $box.Text -match '^Try "')) "the wheel over the pane changed nothing and no report reached the box ('$($box.Text)')"

            $took = Invoke-ResizeStorm
            $settled = Wait-Settled 'classic'
            Claude-Check 'storm.settles' ($settled -ge 0) ("{0} resizes in {1:N0} ms; the box and its rules fit the grid and the screen was still after {2} ms" -f $global:StormSizes.Count, $took, $settled)
            $a = Capture-Ink 'classic-storm-final'
            Check-MainUi $a 'classic-storm' 'classic'
            Claude-Info 'storm.grid' "final grid $($a.Grid.Cols)x$($a.Grid.Rows)"
            # one banner in the whole buffer, not one per width the window had
            $buffer = Get-BufferText
            [IO.File]::WriteAllText((Join-Path $global:NkClaudeShots 'classic-buffer.txt'), $buffer)
            $banners = [regex]::Matches($buffer, 'Claude Code v\d').Count
            $warnings = [regex]::Matches($buffer, 'Safe mode: all').Count
            Claude-Check 'storm.one-banner-in-the-history' ($banners -eq 1) "the buffer (scrollback and screen, $(($buffer -split "`n").Count) lines) holds $banners copies of the banner and $warnings of the safe-mode line after $($global:StormSizes.Count) resizes"
            $bufferLines = @($buffer -split "`n")
            $lead = [Math]::Max(0, @($bufferLines | ForEach-Object { $_ -match 'Claude Code v' }).IndexOf($true))
            # nothing was pushed into the scrollback either: the top of the history is the screen
            $live = (Get-ScreenLines) -join "`n"
            Chord @(0x11, 0x10) 0x24; Start-Sleep -Milliseconds 500
            $top = (Get-ScreenLines) -join "`n"
            Chord @(0x11, 0x10) 0x23; Start-Sleep -Milliseconds 500
            Claude-Check 'storm.no-scrollback' ($live -ceq $top) 'Ctrl+Shift+Home shows the same screen: no line of the old frames was pushed into the scrollback'
            Claude-Info 'storm.history' "the buffer has $lead lines before the banner: the shell's own prompt line above it, which a fresh run has, is not in the buffer after the storm"

            # a fresh run at this size
            Stop-Claude
            Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
            Start-Claude
            Wait-Settled 'classic' | Out-Null
            $b = Capture-Ink 'classic-fresh'
            Check-MainUi $b 'classic-fresh' 'classic'
            # in classic mode the box is not pinned: compare where the CLI put things relative to the banner
            $rowA = ($a.Lines | Select-String 'Claude Code v' | Select-Object -First 1).LineNumber
            $rowB = ($b.Lines | Select-String 'Claude Code v' | Select-Object -First 1).LineNumber
            Claude-Info 'storm.vs-fresh.banner-row' "banner on screen row $rowA after the storm and row $rowB in the fresh run"
            Compare-Screens $a $b 'classic-storm.vs-fresh' 'Claude Code v'
            Stop-Claude
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}
# A pane of a split, a zoom, another tab and back: the CLI's pane resizes and redraws, the sibling pane is not touched and
# every screen is checked against its own text. The tab chip names what the CLI set, closing asks and names it.
function Scenario-ClaudePanes {
    Run-Claude 'claude-panes' 'fullscreen' {
        First-Prompt
        Start-Claude
        $single = Capture-Ink 'panes-1-single'
        Check-MainUi $single 'single' 'fullscreen'
        $chip = @(Get-Chips)[0]
        Claude-Check 'tab.title' ($chip.Name -eq 'Claude Code') "the tab chip is named '$($chip.Name)' while the CLI runs (status '$($chip.Status)')"

        # Split right: the CLI's pane gets half the width, the new shell takes the focus.
        Chord @(0x12, 0x10) 0x44; Wait-ShellPrompt
        $settled = Wait-Settled 'fullscreen'
        $infos = @(Get-PaneInfos)
        Claude-Check 'split.two-panes' ($infos.Count -eq 2 -and $settled -ge 0) "$($infos.Count) panes, the CLI's screen fit its half after $settled ms"
        $cli = Capture-Ink 'panes-2-split-cli'
        Check-MainUi $cli 'split-cli' 'fullscreen'
        Claude-Check 'split.cli-narrower' ($cli.Grid.Cols -lt $single.Grid.Cols) "the CLI's grid went from $($single.Grid.Cols)x$($single.Grid.Rows) to $($cli.Grid.Cols)x$($cli.Grid.Rows)"
        $shellIndex = @(0..($infos.Count - 1) | Where-Object { -not (Get-ClaudeBox $infos[$_].Lines) })[0]
        $lastRow = { param($i) $rows = $infos[$i].Lines; $r = $rows.Count - 1; while ($r -gt 0 -and $rows[$r].Trim().Length -eq 0) { $r-- }; $r }
        $sibling = Capture-Ink 'panes-2-split-shell' 40 $shellIndex @(& $lastRow $shellIndex)
        Check-Ink 'split-shell.ink-matches-text' $sibling
        # output in the sibling leaves the CLI's pane as it was
        Type-Line 'Get-ChildItem $env:windir | Select-Object -First 6 | Out-String'; Start-Sleep -Milliseconds 600; Wait-ShellPrompt
        $after = @(Get-PaneInfos)
        $cliIndex = @(0..($after.Count - 1) | Where-Object { Get-ClaudeBox $after[$_].Lines })[0]
        Claude-Check 'split.sibling-output-leaves-cli' (((Get-StableLines $after[$cliIndex].Lines) -join "`n") -ceq ((Get-StableLines $cli.Lines) -join "`n")) 'six lines printed in the shell pane did not change the CLI pane'

        # focus the CLI's pane (the left one) and zoom it
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 700
        Type-Unicode 'zoom' 30; Claude-Check 'split.keys-reach-cli' (Wait-Box 'zoom') 'after Alt+Left the keys go to the CLI pane'; Clear-Box
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 900
        $zoomSettled = Wait-Settled 'fullscreen'
        $zoom = Capture-Ink 'panes-3-zoomed' 40 -1 $null @(,@(0, ($single.Grid.Cols - 12), 1, ($single.Grid.Cols - 1))); Check-MainUi $zoom 'zoomed' 'fullscreen'
        Claude-Check 'zoom.fills-the-tab' ($zoom.Grid.Cols -eq $single.Grid.Cols -and $zoom.Grid.Rows -eq $single.Grid.Rows -and $zoomSettled -ge 0) "zoomed grid $($zoom.Grid.Cols)x$($zoom.Grid.Rows) (single pane: $($single.Grid.Cols)x$($single.Grid.Rows)), settled after $zoomSettled ms"
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 900
        Wait-Settled 'fullscreen' | Out-Null
        $unzoomed = Capture-Ink 'panes-4-unzoomed'; Check-MainUi $unzoomed 'unzoomed' 'fullscreen'
        Compare-Screens $cli $unzoomed 'unzoom.vs-split'

        # another tab and back
        New-Tab; Type-Line 'echo the other tab'; Wait-Prompt | Out-Null
        Chord @(0x11) 0x09; Start-Sleep -Milliseconds 900
        Wait-Settled 'fullscreen' | Out-Null
        $back = Capture-Ink 'panes-5-back'; Check-MainUi $back 'back' 'fullscreen'
        Compare-Screens $cli $back 'back.vs-split'
        # twelve tabs: eleven shells and the CLI's; walking back to the CLI's tab
        foreach ($i in 1..10) { New-Tab }
        Claude-Check 'tabs.twelve' (@(Get-Chips).Count -eq 12) "$(@(Get-Chips).Count) tabs, the CLI runs in the first"
        Claude-Info 'tabs.plate' ("the position plate reads '" + (Plate-Text) + "'")
        $plate = Get-Element '所有分頁' 'Button'; $pr = $plate.Current.BoundingRectangle
        Click-At ([int]($pr.X + $pr.Width / 2)) ([int]($pr.Y + $pr.Height / 2)) 'left' 900
        foreach ($i in 1..15) { Press 0x26 @() 20 20 }
        Press 0x0D; Start-Sleep -Milliseconds 900
        Wait-Settled 'fullscreen' | Out-Null
        $walked = Capture-Ink 'panes-6-twelve'; Check-MainUi $walked 'twelve' 'fullscreen'
        Compare-Screens $cli $walked 'twelve.vs-split'

        # close the shell's pane of the split: the CLI gets the whole tab back
        Chord @(0x12) 0x27; Start-Sleep -Milliseconds 600
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        $closed = Wait-Settled 'fullscreen'
        $one = Capture-Ink 'panes-7-closed'; Check-MainUi $one 'closed' 'fullscreen'
        Claude-Check 'close.cli-has-the-tab' ($one.Grid.Cols -eq $single.Grid.Cols -and $one.Grid.Rows -eq $single.Grid.Rows -and $closed -ge 0 -and @(Get-PaneInfos).Count -eq 1) "after closing the shell pane: $(@(Get-PaneInfos).Count) pane, grid $($one.Grid.Cols)x$($one.Grid.Rows), settled after $closed ms"
        Compare-Screens $single $one 'closed.vs-single'

        # closing the CLI's own tab asks and names it
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        Claude-Check 'close.asks-and-names-the-tab' ((Dialog-Open) -and [bool](Get-Element '關閉「Claude Code」？')) 'closing the pane that runs the CLI asks first: 關閉「Claude Code」？'
        Press 0x1B; Start-Sleep -Milliseconds 800
        Claude-Check 'close.cancel-keeps-it' ((Get-ClaudeBox) -and @(Get-PaneInfos).Count -ge 1) 'cancelling leaves the CLI running'
        Stop-Claude
        $chip = @(Get-Chips | Where-Object { $_.Index -eq 1 })[0]
        Claude-Info 'tab.title-after-exit' "the chip reads '$($chip.Name)' after the CLI exited"
    }
}
# --- input method ----------------------------------------------------------------------------------------------------------

# The log of the app says what the input method asked for: the text of the composition (TextUpdating), the rectangle it
# places its window from (LayoutRequested, the cursor cell in screen pixels) and the commit.
function Get-ImeEvents([int]$Mark) {
    $lines = @(Log-Since $Mark)
    $preedits = @($lines | ForEach-Object { if ($_ -match "IME TextUpdating -> preedit '(.*)'") { $Matches[1] } })
    $layouts = @($lines | ForEach-Object { if ($_ -match 'IME LayoutRequested -> cursor cell \(([\d.]+), ([\d.]+), ([\d.]+), ([\d.]+)\)') { , @([double]$Matches[1], [double]$Matches[2], [double]$Matches[3], [double]$Matches[4]) } })
    $commits = @($lines | ForEach-Object { if ($_ -match "IME COMMIT '(.*)'") { $Matches[1] } })
    # a composition is pending when the last preedit came after the last commit
    $lastPreedit = -1; $lastCommit = -1
    for ($i = 0; $i -lt $lines.Count; $i++) { if ($lines[$i] -match 'IME TextUpdating -> preedit ''.+''') { $lastPreedit = $i } elseif ($lines[$i] -match 'IME COMMIT') { $lastCommit = $i } }
    @{ Preedits = $preedits; Layouts = $layouts; Commits = $commits; Pending = ($lastPreedit -gt $lastCommit) }
}

# Enter commits a composition the input method holds (it takes the key, the CLI does not get it). It is only pressed when
# the log shows a pending composition and the box still reads what it read before: if the keys had gone to the CLI as
# letters, Enter would submit them to the model.
function Commit-Ime([int]$Mark, [string]$BoxBefore) {
    $events = Get-ImeEvents $Mark
    if (-not $events.Pending) { throw 'no composition is pending in the log: Enter not sent' }
    $box = Get-ClaudeBox
    $now = if ($box.Text -match '^Try "') { '' } else { $box.Text }
    if ($now -cne $BoxBefore) { throw "the box reads '$now' while a composition is pending, not '$BoxBefore': Enter not sent" }
    Press 0x0D
}

# Zhuyin through the real input method (Microsoft Bopomofo, s u 3 is nǐ, c l 3 is hǎo): the preedit and the rectangle the
# input method places its window from are at the CLI's caret, the commit arrives intact and is not submitted.
function Check-ClaudeIme([bool]$Native) {
    $id = if ($Native) { 'ime' } else { 'ime-no-native-cursor' }
    $baseline = Capture-Ink "$id-0-baseline"
    $box = $baseline.Box
    $cell = @(($baseline.Grid.X + 2 * $baseline.Grid.CW), ($baseline.Grid.Y + $box.Row * $baseline.Grid.CH))
    $mark = Log-Mark
    Press 0x53; Press 0x55; Press 0x33; Start-Sleep -Milliseconds 900
    $events = Get-ImeEvents $mark
    Claude-Check "$id.composition-starts" ($events.Pending -and $events.Preedits.Count -ge 3 -and $events.Preedits[0] -eq 'ㄋ') "the input method composed '$($events.Preedits -join ''' ''')' for s u 3"
    $layout = if ($events.Layouts.Count) { $events.Layouts[-1] } else { @(0, 0, 0, 0) }
    $atCaret = [Math]::Abs($layout[0] - $cell[0]) -le 1 -and [Math]::Abs($layout[1] - $cell[1]) -le 1
    if ($Native) {
        Claude-Check "$id.window-at-caret" $atCaret "the rectangle given to the input method is ($($layout[0]), $($layout[1]), $($layout[2]) x $($layout[3])); the caret cell (column 2 of the box row) is at ($($cell[0]), $($cell[1]))"
    } else {
        Claude-Info "$id.window-at-caret" "without CLAUDE_CODE_NATIVE_CURSOR the rectangle is ($($layout[0]), $($layout[1])); the caret cell is at ($($cell[0]), $($cell[1])): at the caret = $atCaret"
    }
    $text = (Get-ClaudeBox).Text
    Claude-Check "$id.box-untouched" ($text -eq '' -or $text -match '^Try "') "while composing the CLI's box holds '$text'"
    $composing = Capture-Ink "$id-1-composing"
    # the preedit is drawn in the cells of the caret: the box row changes there and nowhere beyond the placeholder
    $px0 = Read-Pixels $baseline.Path; $px1 = Read-Pixels $composing.Path
    $cw = $baseline.Grid.CW; $ch = $baseline.Grid.CH
    $ox = $baseline.Contrast.Ox; $oy = $baseline.Contrast.Oy
    $changed = [Ink]::DiffRegion($px0.Bytes, $px0.Stride, $px1.Bytes, $px1.Stride, $ox + 2 * $cw, $oy + $box.Row * $ch, 4 * $cw, $ch, 24)
    Claude-Check "$id.preedit-drawn-at-caret" ($changed -gt 200) "$changed pixels of the four cells at the caret differ from the box without the composition"
    if (-not $Native) { Press 0x1B; Start-Sleep -Milliseconds 600; return }

    $before = ''
    $first = ($events.Preedits | Where-Object { $_ })[-1]
    Commit-Ime $mark $before; Start-Sleep -Milliseconds 900
    $events = Get-ImeEvents $mark
    $committed = $events.Commits[-1]
    Claude-Check "$id.commit-arrives" ($committed -and (Wait-Box $committed)) "the commit '$committed' is what the box reads ('$((Get-ClaudeBox).Text)')"
    Start-Sleep -Milliseconds 800
    $afterEnter = Get-ClaudeBox
    Claude-Check "$id.commit-not-submitted" ($afterEnter -and $afterEnter.Text -ceq $committed) "a second later the box still reads '$($afterEnter.Text)': Enter went to the input method, not to the CLI"
    # a second syllable: the window follows the caret, which moved two cells
    $mark2 = Log-Mark
    Press 0x43; Press 0x4C; Press 0x33; Start-Sleep -Milliseconds 900
    $events2 = Get-ImeEvents $mark2
    $width = (Get-RowColumns $committed).Width
    $cell2 = @(($baseline.Grid.X + (2 + $width) * $baseline.Grid.CW), $cell[1])
    $layout2 = if ($events2.Layouts.Count) { $events2.Layouts[-1] } else { @(0, 0, 0, 0) }
    Claude-Check "$id.window-follows-caret" ($events2.Pending -and [Math]::Abs($layout2[0] - $cell2[0]) -le 1 -and [Math]::Abs($layout2[1] - $cell2[1]) -le 1) "after '$committed' the rectangle is ($($layout2[0]), $($layout2[1])) and the caret cell ($($cell2[0]), $($cell2[1]))"
    Commit-Ime $mark2 $committed; Start-Sleep -Milliseconds 900
    $both = $committed + $events2.Commits[-1]
    $events2 = Get-ImeEvents $mark2
    $both = $committed + $events2.Commits[-1]
    Claude-Check "$id.second-commit" (Wait-Box $both) "the box reads '$((Get-ClaudeBox).Text)' (want '$both')"
    # Backspace removes whole characters, and the cells are where the text is
    Press 0x08; Claude-Check "$id.backspace" (Wait-Box $committed) 'Backspace removes the last character of the commit'
    $cap = Capture-Ink "$id-2-committed"; Check-Ink "$id.ink-matches-text" $cap
    Clear-Box
}

# Zhuyin in the CLI with the default environment (the real cursor at the caret) and without it.
function Scenario-ClaudeIme {
    Run-Claude 'claude-ime' 'fullscreen' {
        First-Prompt
        Start-Claude
        Check-ClaudeIme $true
        Stop-Claude
    }
    Run-Claude 'claude-ime-plain' 'fullscreen' {
        First-Prompt
        $allowed = (Get-ClaudeEnv 'fullscreen').GetEnumerator() | Where-Object { $null -ne $_.Value -and $_.Key -match '^CLAUDE' } | ForEach-Object { $_.Key }
        $tab = Get-TabEnvironment $allowed
        Claude-Check 'ime-no-native-cursor.env' ($tab.Values['CLAUDE_CODE_NATIVE_CURSOR'] -eq '' -and $tab.Values['FORCE_HYPERLINK'] -eq '') "agentEnvironment is off: FORCE_HYPERLINK='$($tab.Values['FORCE_HYPERLINK'])', CLAUDE_CODE_NATIVE_CURSOR='$($tab.Values['CLAUDE_CODE_NATIVE_CURSOR'])'"
        Start-Claude
        Check-ClaudeIme $false
        Stop-Claude
    } @{ agentEnvironment = $false }
}
# --- frames, typing under load, focus, transcript, closing ----------------------------------------------------------------

# A capture of the window in memory (no file), taken only while the app owns the middle of its window.
function Grab-Window {
    $r = Win-Rect; $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
    if ([D]::PidAt([int](($r.Left + $r.Right) / 2), [int](($r.Top + $r.Bottom) / 2)) -ne [uint32]$script:App.Id) { return $null }
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    try {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size $w, $h)); $g.Dispose()
        $data = $bmp.LockBits((New-Object System.Drawing.Rectangle 0, 0, $w, $h), [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try { $bytes = New-Object byte[] ($data.Stride * $h); [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length); @{ Bytes = $bytes; Stride = $data.Stride; W = $w; H = $h } } finally { $bmp.UnlockBits($data) }
    } finally { $bmp.Dispose() }
}

# Types text on a thread of its own (so the window can be captured while it arrives), one character every $GapMs.
function Start-Typing([string]$Text, [int]$GapMs = 20) {
    Assert-Foreground
    Start-ThreadJob -ArgumentList $Text, $GapMs -ScriptBlock { param($text, $gap) foreach ($ch in $text.ToCharArray()) { [D]::Unicode($ch); Start-Sleep -Milliseconds $gap } }
}

# Frames while text is typed into the box: every capture of the window is a whole frame. The rules above and below the box
# are inked in every column, the conversation above the box is what it was before the first key, and the status line is.
function Check-Frames([string]$Id, [string]$Text, [int]$GapMs = 15) {
    $info = Get-PaneInfos | Where-Object { Get-ClaudeBox $_.Lines } | Select-Object -First 1
    $grid = $info.Grid; $box = Get-ClaudeBox $info.Lines; $win = Win-Rect
    $ox = [int]($grid.X - $win.Left); $oy = [int]($grid.Y - $win.Top)
    $inked = { param($shot) $flat = [Ink]::Contrast($shot.Bytes, $shot.Stride, $shot.W, $shot.H, $ox, $oy, $grid.CW, $grid.CH, $grid.Cols, $grid.Rows, 2, 4); ,@($flat | ForEach-Object { $_ -gt 40 }) }
    $first = Grab-Window; $baseline = & $inked $first
    $job = Start-Typing $Text $GapMs
    $frames = 0; $bad = @(); $cols = $grid.Cols
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($job.State -eq 'Running' -or $watch.ElapsedMilliseconds -lt 300) {
        $shot = Grab-Window
        if ($shot) {
            $frames++
            $map = & $inked $shot
            $problems = @()
            foreach ($rule in ($box.Row - 1), $box.End) { $n = 0; for ($col = 0; $col -lt $cols; $col++) { if ($map[$rule * $cols + $col]) { $n++ } }; if ($n -ne $cols) { $problems += "rule row $rule has ink in $n of $cols cells" } }
            # the conversation (rows above the hint row) and the status line
            $diff = 0
            for ($row = 0; $row -lt $box.Row - 2; $row++) { for ($col = 0; $col -lt $cols; $col++) { if ($map[$row * $cols + $col] -ne $baseline[$row * $cols + $col]) { $diff++ } } }
            if ($diff -gt 0) { $problems += "$diff cells of the conversation changed" }
            if (-not ($map[($grid.Rows - 1) * $cols + 4] -and $map[($grid.Rows - 1) * $cols + 6])) { $problems += 'the status line is not drawn' }
            if ($problems.Count) { $bad += "frame ${frames}: " + ($problems -join ', ') }
        }
    }
    Receive-Job $job -Wait -AutoRemoveJob | Out-Null
    Claude-Check "$Id.whole-frames" ($frames -ge 10 -and $bad.Count -eq 0) "$frames captures of the window while $($Text.Length) characters were typed, $($bad.Count) not a whole frame $(($bad | Select-Object -First 3) -join '; ')"
    Wait-Box $Text | Out-Null
}

# Typed text arrives whole and in order while the window is resized under it, and nothing is garbled afterwards.
function Check-TypingDuringStorm {
    $text = 'storm typing 你好 abc 123 😀 end'
    $job = Start-Typing $text 25
    $took = Invoke-ResizeStorm
    Receive-Job $job -Wait -AutoRemoveJob | Out-Null
    $settled = Wait-Settled 'fullscreen'
    $box = Get-ClaudeBox
    Claude-Check 'storm-typing.arrives' ($box.Text -ceq $text) "typed $($text.Length) characters during a $([int]$took) ms storm; the box reads '$($box.Text)'"
    $cap = Capture-Ink 'storm-typing'; Check-MainUi $cap 'storm-typing' 'fullscreen'
    Clear-Box
}

# The CLI's frames and typing while the window is resized.
function Scenario-ClaudeFrames {
    Run-Claude 'claude-frames' 'fullscreen' {
        First-Prompt
        Start-Claude
        Check-Frames 'frames-ascii' 'frame check typed at speed 0123456789 abcdefghij' 12
        Clear-Box
        Check-Frames 'frames-cjk' '你好世界ㄅㄆㄇ😀 mixed text 日本語' 15
        Clear-Box
        Check-TypingDuringStorm
        Stop-Claude
    }
}
# Focus reports (DECSET 1004, which the CLI turns on): a stand-in program that asks for them gets ESC[O when its pane loses
# the focus and ESC[I when it gets it back, for a switch between panes and for another window taking the focus.
function Check-FocusReports {
    Type-Line "node `"$(Join-Path $PSScriptRoot 'input-probe.js')`" 1004 `"`" 14000"
    Start-Sleep -Milliseconds 1500
    Chord @(0x12, 0x10) 0x44; Wait-ShellPrompt          # a split: the new pane takes the focus
    Chord @(0x12) 0x25; Start-Sleep -Milliseconds 800     # Alt+Left: back
    [void][P]::SetWindowPos($script:PatternHwnd, [IntPtr]::Zero, 40, 1300, 300, 200, 0x0040)
    [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 1200       # another window of this script takes the focus
    [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 1200
    $text = ''
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline -and $text -notmatch 'got:\[.*\]') { Start-Sleep -Milliseconds 400; $text = (@(Get-PaneInfos) | ForEach-Object { $_.Lines }) -join '' }
    $got = [regex]::Match($text, 'got:\[(.*?)\]').Groups[1].Value
    Claude-Check 'focus.reports' ($got -ceq '<ESC>[O<ESC>[I<ESC>[O<ESC>[I') "a pane switch away and back, then another window active and this one again: '$got' (want out, in, out, in)"
}

# Ctrl+O shows the whole transcript and `[` hands it to the terminal: the CLI leaves the alternate screen, the conversation is
# in the buffer and the mouse is the terminal's; Esc brings the full-screen UI back.
function Check-Transcript {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Press 0x4F @(0x11); Start-Sleep -Milliseconds 900
        Claude-Check 'transcript.opens' ((Get-TerminalText) -match 'Showing detailed transcript') 'Ctrl+O shows the detailed transcript'
        Type-Unicode '[' 60; Start-Sleep -Milliseconds 1500
        $buffer = Get-BufferText
        Claude-Check 'transcript.handed-to-the-buffer' ($buffer -match 'Claude Code v\d' -and $buffer -match 'Showing detailed transcript') "after '[' the buffer ($(($buffer -split "`n").Count) lines) holds the banner and the transcript footer"
        Press 0x1B; Start-Sleep -Milliseconds 1500
        $settled = Wait-Settled 'fullscreen'
        $cap = Capture-Ink 'transcript-back'; Check-MainUi $cap 'transcript-back' 'fullscreen'
        Claude-Check 'transcript.back' ($settled -ge 0) "Esc returned to the full-screen UI, settled after $settled ms"
    } finally { if ($null -ne $saved) { Set-Clipboard -Value $saved } }
}

# The window and the focus while the CLI runs: focus reports, the transcript, closing the window.
function Scenario-ClaudeWindow {
    Run-Claude 'claude-window' 'fullscreen' {
        First-Prompt
        Check-FocusReports
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 800       # close the split's pane
        Start-Claude
        Check-Transcript
        # closing the window with the CLI running asks first
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 1500
        $script:App.Refresh()
        $asked = (-not $script:App.HasExited) -and (Dialog-Open)
        Claude-Check 'close.window-asks' $asked 'closing the window while the CLI runs shows a confirmation and the window stays'
        if (-not $asked) { return }
        Press 0x1B; Start-Sleep -Milliseconds 800
        Claude-Check 'close.window-cancel-keeps-it' ((-not $script:App.HasExited) -and (Get-ClaudeBox)) 'cancelling leaves the window and the CLI as they were'
        Stop-Claude
    } -Pattern
}
# Every scenario of this file in turn (one claude.exe at a time). A scenario that fails does not stop the others; the run
# fails at the end when any did.
function Scenario-ClaudeAll {
    $failed = @()
    foreach ($name in 'Scenario-ClaudeScreen', 'Scenario-ClaudeKeys', 'Scenario-ClaudeMouse', 'Scenario-ClaudeIme', 'Scenario-ClaudeFrames', 'Scenario-ClaudeResize', 'Scenario-ClaudeClassic', 'Scenario-ClaudePanes', 'Scenario-ClaudeWindow', 'Scenario-ClaudeExit') {
        "=== $name"
        try { & $name } catch { $failed += $name; "SCENARIO FAILED: $name | $($_.Exception.Message)" }
    }
    if ($failed.Count -gt 0) { throw "$($failed.Count) scenario(s) failed: $($failed -join ', ')" }
}