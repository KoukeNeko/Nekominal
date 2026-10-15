# Helpers for driving nekominal.exe on top of drive-lib.ps1 (dot-sourced by run-app.ps1).
#
#   Shot-Safe          guarded screenshot of the app window (skipped when another window is on top)
#   Wait-Log           waits for a regex to appear in the app log
#   Get-Element/Click-Name   UI Automation lookup by name (chips, tool buttons carry AutomationProperties.Name)
#   Start-Pattern / Place-PatternUnderApp / Stop-Pattern   a striped window of this script under the app, to show the blur
#   Type-Line          types a line into the focused terminal (Unicode input) and presses Enter

Add-Type -AssemblyName UIAutomationClient -ErrorAction SilentlyContinue
Add-Type -AssemblyName UIAutomationTypes -ErrorAction SilentlyContinue

$script:PatternProc = $null

function Shot-Safe([string]$Path) {
    for ($k = 0; $k -lt 8; $k++) {
        $r = Win-Rect
        $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
        if ([D]::ForegroundPid() -eq [uint32]$script:App.Id -and [D]::PidAt($cx, $cy) -eq [uint32]$script:App.Id -and [D]::PidAt($r.Left + 200, $r.Top + 300) -eq [uint32]$script:App.Id) { return (Shot $Path) }
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd)
        Start-Sleep -Milliseconds 500
    }
    'shot skipped: another window is on top of the test window'
}

function Wait-Log([string]$Pattern, [int]$TimeoutMs = 8000) {
    $log = $script:LogFile
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Test-Path $log) -and (Select-String -Path $log -Pattern $Pattern -Quiet)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    $false
}

# Finds a UI element by its automation name; -Class narrows to controls whose class name ends with it
# (the toolbar button "外觀" and the settings nav item "外觀" share a name).
function Get-Element([string]$Name, [string]$Class = '') {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
    foreach ($e in $all) { if (-not $Class -or $e.Current.ClassName.EndsWith($Class)) { return $e } }
    $null
}

function Get-ElementRect([string]$Name, [string]$Class = '') {
    $e = Get-Element $Name $Class
    if (-not $e) { throw "no UI element named '$Name' $Class" }
    $b = $e.Current.BoundingRectangle
    @{ X = [int]$b.X; Y = [int]$b.Y; W = [int]$b.Width; H = [int]$b.Height }
}

function Click-Name([string]$Name, [string]$Button = 'left', [int]$Gap = 500, [double]$Fx = 0.5, [string]$Class = '') {
    $r = Get-ElementRect $Name $Class
    Click-At ([int]($r.X + $r.W * $Fx)) ([int]($r.Y + $r.H / 2)) $Button $Gap
}

# The visible text of the terminal that has the keyboard focus, read through the UI Automation Text pattern of the
# pane's own peer (the same route a screen reader takes).
function Get-TerminalText {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        if ($e.Current.HasKeyboardFocus) {
            $p = $null
            if ($e.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref]$p)) { return $p.DocumentRange.GetText(-1) }
        }
    }
    $null
}

# Waits until the focused terminal ends in a shell prompt (text ends with '>') and has stopped changing, so the next
# line is typed into a ready PSReadLine instead of racing the shell start-up or the previous command.
function Wait-Prompt([int]$TimeoutMs = 20000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs); $last = $null; $stable = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        $t = Get-TerminalText
        if ($t -and $t.TrimEnd().EndsWith('>')) {
            if ($t -eq $last) { $stable++ } else { $stable = 0 }
            if ($stable -ge 2) { return $true }
        } else { $stable = 0 }
        $last = $t
        Start-Sleep -Milliseconds 200
    }
    $false
}

# Sets a Slider (found by its UIA control type) through the RangeValue pattern: the same ValueChanged a drag raises.
function Set-Slider([double]$Value, [int]$Index = 0) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Slider)
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if ($all.Count -le $Index) { throw "no slider #$Index (found $($all.Count))" }
    $p = $all[$Index].GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
    $p.SetValue($Value)
}

# Mean colour of a rectangle of a saved screenshot, as 'R,G,B'.
function Region-Average([string]$Png, [int]$X, [int]$Y, [int]$W, [int]$H) {
    $bmp = [System.Drawing.Bitmap]::FromFile($Png)
    try {
        $r = 0; $g = 0; $b = 0; $n = 0
        for ($yy = $Y; $yy -lt $Y + $H; $yy += 3) { for ($xx = $X; $xx -lt $X + $W; $xx += 3) { $c = $bmp.GetPixel($xx, $yy); $r += $c.R; $g += $c.G; $b += $c.B; $n++ } }
        '{0:N0},{1:N0},{2:N0}' -f ($r / $n), ($g / $n), ($b / $n)
    } finally { $bmp.Dispose() }
}
function Type-Line([string]$Text, [int]$GapMs = 25) {
    Type-Unicode $Text $GapMs
    Press 0x0D
}

# The visible text of the focused-or-first terminal pane, read through UI Automation (the pane's TextPattern, which
# is what a screen reader sees). Empty when the control is not there yet.
function Get-TerminalText([int]$Index = -1) {
    $element = $null
    if ($Index -lt 0) {
        # The pane that has the keyboard focus, when it is a terminal.
        try { $focused = [System.Windows.Automation.AutomationElement]::FocusedElement; if ($focused.Current.ClassName -eq 'NekominalTerminal') { $element = $focused } } catch { }
        $Index = 0
    }
    if (-not $element) {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
        $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
        if ($all.Count -le $Index) { return '' }
        $element = $all[$Index]
    }
    try {
        $tp = $element.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
        $tp.DocumentRange.GetText(-1)
    } catch { '' }
}

# Waits until the terminal text matches the regex (UIA text, refreshed by the app at 10 Hz).
function Wait-Text([string]$Pattern, [int]$TimeoutMs = 15000, [int]$Index = -1) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Get-TerminalText $Index) -match $Pattern) { return $true }
        Start-Sleep -Milliseconds 150
    }
    $false
}

# Waits for the shell's prompt, so typing never races the shell's start-up (PSReadLine treats input that piled up
# before the prompt as one paste).
function Wait-Prompt([int]$TimeoutMs = 20000, [int]$Index = -1) {
    if (-not (Wait-Text 'PS [^\r\n]*>' $TimeoutMs $Index)) { throw 'the shell prompt never appeared' }
    Start-Sleep -Milliseconds 400
}

function Chord([int[]]$Mods, [int]$Vk) { Press $Vk $Mods }

# Holds a chord the way a keyboard does: modifiers down, then the key's down event repeated every $RepeatMs for
# $HoldMs (the second and later events carry the auto-repeat flag), then everything released.
function Hold-Chord([int[]]$Mods, [int]$Vk, [int]$HoldMs = 2000, [int]$RepeatMs = 33) {
    Assert-Foreground
    foreach ($m in $Mods) { [D]::Key([uint16]$m, $false); Start-Sleep -Milliseconds 15 }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $HoldMs) { [D]::Key([uint16]$Vk, $false); Start-Sleep -Milliseconds $RepeatMs }
    [D]::Key([uint16]$Vk, $true)
    $r = $Mods.Clone(); [array]::Reverse($r)
    foreach ($m in $r) { Start-Sleep -Milliseconds 15; [D]::Key([uint16]$m, $true) }
    Start-Sleep -Milliseconds 400
}

# Number of terminal panes of the visible tab (hidden tabs are not in the UI Automation tree).
function Count-Panes {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond).Count
}

# Sets the width of the window in DIP (DIP * the scale of the window's monitor pixels), keeping its position and
# height. The outer rectangle is wider than the visible one by the invisible frame, which is kept.
function Set-WindowWidth([int]$Dip, [double]$Scale = 0) {
    if ($Scale -le 0) { $Scale = Get-Scale }
    $visible = Win-Rect
    $outer = New-Object D+RECT
    [void][D]::GetWindowRect($script:Hwnd, [ref]$outer)
    $frame = ($outer.Right - $outer.Left) - ($visible.Right - $visible.Left)
    [void][P]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, $outer.Left, $outer.Top, [int]($Dip * $Scale) + $frame, $outer.Bottom - $outer.Top, 0x0014)
    Start-Sleep -Milliseconds 900
}
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class Menus {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  // The window class of every popup menu, including the system menu.
  public static bool MenuIsOpen() { return FindWindow("#32768", null) != IntPtr.Zero; }
}
'@

$script:PatternScript = @'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object System.Windows.Forms.Form
$f.Text = 'nk-blur-pattern'; $f.FormBorderStyle = 'None'; $f.StartPosition = 'Manual'; $f.ShowInTaskbar = $false
$f.Left = 0; $f.Top = 0; $f.Width = 3000; $f.Height = 2000
$f.BackColor = [System.Drawing.Color]::White
$f.Add_Paint({
    param($s, $e)
    $g = $e.Graphics
    $colors = 'Red','Lime','Blue','Yellow','Magenta','Cyan','Orange','Black'
    for ($i = 0; $i -lt 20; $i++) {
        $b = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromName($colors[$i % 8]))
        $g.FillRectangle($b, $i * 150, 0, 75, $s.Height)
    }
    $font = New-Object System.Drawing.Font('Consolas', 28, [System.Drawing.FontStyle]::Bold)
    $black = [System.Drawing.Brushes]::Black
    for ($y = 0; $y -lt $s.Height; $y += 70) { $g.DrawString('BLUR PATTERN 0123456789 ABCDEFGH', $font, $black, 10, $y) }
})
[System.Windows.Forms.Application]::Run($f)
'@

function Start-Pattern {
    $file = Join-Path ([IO.Path]::GetTempPath()) 'nk-pattern.ps1'
    Set-Content -Path $file -Value $script:PatternScript -Encoding UTF8
    # CreateNoWindow, not -WindowStyle Hidden: the latter would also hide the form (the first window inherits SW_HIDE).
    $psi = New-Object System.Diagnostics.ProcessStartInfo 'pwsh', "-NoProfile -File ""$file"""
    $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
    $script:PatternProc = [Diagnostics.Process]::Start($psi)
    Start-Sleep -Milliseconds 1500
}

# Puts the pattern window directly below the app window in z-order, covering its rectangle plus `$Margin` pixels on
# every side (so a capture with a margin shows the crisp pattern around the blurred window).
function Place-PatternUnderApp([int]$Margin = 0) {
    if (-not $script:PatternProc) { return }
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40 -and $h -eq [IntPtr]::Zero; $i++) {
        $h = [D]::FindWindow([uint32]$script:PatternProc.Id)
        if ($h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($h -eq [IntPtr]::Zero) { return }
    $script:PatternHwnd = $h
    $script:PatternMargin = $Margin
    $r = Win-Rect
    [void][P]::SetWindowPos($h, $script:Hwnd, $r.Left - $Margin, $r.Top - $Margin, ($r.Right - $r.Left) + 2 * $Margin, ($r.Bottom - $r.Top) + 2 * $Margin, 0x0010 -bor 0x0040)
    Start-Sleep -Milliseconds 300
    [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 400
}

# Screenshot of the app window plus the margin around it, taken only while the app owns the middle and the pattern
# window (this script's own) owns every corner: nothing else of the desktop is captured.
function Shot-WithPattern([string]$Path) {
    $m = $script:PatternMargin
    for ($k = 0; $k -lt 8; $k++) {
        $r = Win-Rect
        $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
        $pp = [uint32]$script:PatternProc.Id; $ap = [uint32]$script:App.Id
        $corners = @(@(($r.Left - $m + 4), ($r.Top - $m + 4)), @(($r.Right + $m - 4), ($r.Top - $m + 4)), @(($r.Left - $m + 4), ($r.Bottom + $m - 4)), @(($r.Right + $m - 4), ($r.Bottom + $m - 4)))
        $cornersOk = $true
        foreach ($p in $corners) { if ([D]::PidAt($p[0], $p[1]) -ne $pp) { $cornersOk = $false } }
        if ([D]::ForegroundPid() -eq $ap -and [D]::PidAt($cx, $cy) -eq $ap -and $cornersOk) {
            $w = ($r.Right - $r.Left) + 2 * $m; $hh = ($r.Bottom - $r.Top) + 2 * $m
            return (ShotRegion $Path ($r.Left - $m) ($r.Top - $m) $w $hh)
        }
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd)
        Start-Sleep -Milliseconds 500
    }
    'pattern shot skipped: another window is on top'
}
# Screenshot of the app while another window of this script is the active one (the acrylic backdrop then shows its
# inactive look). The pattern window moves to a screen corner; the shot is skipped unless the app owns its centre and
# two corners.
function Shot-Inactive([string]$Path) {
    $ph = $script:PatternHwnd
    if (-not (Move-PatternAway 320 220)) { return 'inactive shot skipped: no free corner of the screen' }
    [D]::ForceForeground($ph); Start-Sleep -Milliseconds 1200
    $r = Win-Rect; $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2); $app = [uint32]$script:App.Id
    if ([D]::PidAt($cx, $cy) -eq $app -and [D]::PidAt($r.Left + 20, $r.Top + 20) -eq $app -and [D]::PidAt($r.Right - 20, $r.Bottom - 20) -eq $app) { Shot $Path } else { 'inactive shot skipped' }
}

function Stop-Pattern { if ($script:PatternProc -and -not $script:PatternProc.HasExited) { $script:PatternProc.Kill() } }

# Moves the pattern window (this script's own) to a corner of the screen that the app window does not cover, so that
# it can take the focus from the app without hiding any of it. False when every corner is covered.
function Move-PatternAway([int]$W = 300, [int]$H = 200) {
    $left = [D]::GetSystemMetrics(76); $top = [D]::GetSystemMetrics(77)
    $right = $left + [D]::GetSystemMetrics(78); $bottom = $top + [D]::GetSystemMetrics(79)
    $app = Win-Rect
    foreach ($spot in @(@($left, ($bottom - $H)), @(($right - $W), ($bottom - $H)), @(($right - $W), $top), @($left, $top))) {
        $x = $spot[0]; $y = $spot[1]
        if ($x + $W -le $app.Left -or $x -ge $app.Right -or $y + $H -le $app.Top -or $y -ge $app.Bottom) {
            [void][P]::SetWindowPos($script:PatternHwnd, [IntPtr]::Zero, $x, $y, $W, $H, 0x0040)
            return $true
        }
    }
    $false
}

Add-Type @'
using System; using System.Runtime.InteropServices;
public static class P {
  public static readonly IntPtr HWND_BOTTOM = new IntPtr(1);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
'@

# --- terminal pointer helpers ---------------------------------------------------------------------------------------------

# One line per check: ok, or FAIL with what was expected and what came.
function Check([string]$Name, $Expected, $Actual) {
    $ok = ($Expected -ceq $Actual)
    '{0} {1}{2}' -f $(if ($ok) { 'ok  ' } else { 'FAIL' }), $Name, $(if ($ok) { '' } else { " | expected [$Expected] got [$Actual]" })
}

# The grid of the focused terminal in screen pixels: the pane writes "pane N grid CxR cells of WxH px" to the log, and the
# UI Automation rectangle of its control is where the cells start.
function Get-Grid {
    $line = Select-String -Path $script:LogFile -Pattern 'pane \d+ grid (\d+)x(\d+) cells of (\d+)x(\d+) px' | Select-Object -Last 1
    if (-not $line) { throw 'the pane wrote no grid line' }
    $g = $line.Matches[0].Groups
    $el = [System.Windows.Automation.AutomationElement]::FocusedElement
    if ($el.Current.ClassName -ne 'NekominalTerminal') { throw "the focus is on '$($el.Current.ClassName)', not on a terminal" }
    $r = $el.Current.BoundingRectangle
    @{ X = $r.X; Y = $r.Y; W = $r.Width; H = $r.Height; Cols = [int]$g[1].Value; Rows = [int]$g[2].Value; CW = [int]$g[3].Value; CH = [int]$g[4].Value }
}

# The screen point of a position on the grid. Fractions pick a part of the cell: column + 0.25 is its left half and
# column + 0.75 its right half; the row is the middle of the cell's row.
function Cell-Pt($Grid, [double]$Col, [double]$Row) { @{ X = [int]($Grid.X + $Col * $Grid.CW); Y = [int]($Grid.Y + ($Row + 0.5) * $Grid.CH) } }

# Row and column of the last place a text is on the visible screen, which is its output when the command that printed it
# is on screen too (the lines of the UI Automation text are the rows of the grid).
function Find-Cell([string]$Text) {
    $lines = (Get-TerminalText) -split "`n"
    for ($row = $lines.Count - 1; $row -ge 0; $row--) {
        $col = $lines[$row].IndexOf($Text, [StringComparison]::Ordinal)
        if ($col -ge 0) { return @{ Row = $row; Col = $col } }
    }
    throw "'$Text' is not on the screen"
}

# Holds keys (virtual-key codes) down around a block, the way Shift+drag and Alt+drag are made.
function With-Keys([int[]]$Vk, [scriptblock]$Do) {
    Assert-Foreground
    foreach ($k in $Vk) { [D]::Key([uint16]$k, $false); Start-Sleep -Milliseconds 40 }
    try { & $Do } finally { foreach ($k in $Vk) { [D]::Key([uint16]$k, $true); Start-Sleep -Milliseconds 40 } }
}

# Drags between two grid positions (see Cell-Pt).
function Drag-Cells($Grid, [double]$FromCol, [double]$FromRow, [double]$ToCol, [double]$ToRow) {
    $a = Cell-Pt $Grid $FromCol $FromRow; $b = Cell-Pt $Grid $ToCol $ToRow
    Drag-To $a.X $a.Y $b.X $b.Y 12 20
}

# Clicks a grid position $Times times, quickly enough to be a double or triple click.
function Click-Cell($Grid, [double]$Col, [double]$Row, [int]$Times = 1, [string]$Button = 'left') {
    $p = Cell-Pt $Grid $Col $Row
    for ($i = 1; $i -le $Times; $i++) { Click-At $p.X $p.Y $Button $(if ($i -lt $Times) { 0 } else { 400 }) }
}

# Wheel notches at the pointer (positive: away from the user, which scrolls the history up).
function Mouse-Wheel([int]$Notches) {
    Assert-Foreground
    $delta = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]($Notches * 120)), 0)
    [D]::Mouse(0x0800, $delta)
    Start-Sleep -Milliseconds 300
}

# Ctrl+Shift+C, then what the clipboard holds. A sentinel first, so a copy that did nothing cannot pass.
function Copy-Selection {
    Set-Clipboard -Value 'SENTINEL'
    Chord @(0x11, 0x10) 0x43
    Start-Sleep -Milliseconds 500
    (Get-Clipboard -Raw) -replace "`r`n", "`n"
}

# Number of pixels that differ between two screenshots of the same size inside a rectangle (screenshot coordinates).
function Region-Diff([string]$PathA, [string]$PathB, [int]$X, [int]$Y, [int]$W, [int]$H) {
    $one = [System.Drawing.Bitmap]::FromFile($PathA); $two = [System.Drawing.Bitmap]::FromFile($PathB)
    try {
        $n = 0
        for ($yy = $Y; $yy -lt [Math]::Min($Y + $H, $one.Height); $yy++) { for ($xx = $X; $xx -lt [Math]::Min($X + $W, $one.Width); $xx++) { if ($one.GetPixel($xx, $yy).ToArgb() -ne $two.GetPixel($xx, $yy).ToArgb()) { $n++ } } }
        $n
    } finally { $one.Dispose(); $two.Dispose() }
}

# True while the mouse cursor over the app is the hand (the arrow otherwise).
function Cursor-IsHand { [D]::CursorHandle() -eq [D]::LoadCursor([IntPtr]::Zero, [IntPtr]32649) }

# --- checks that fail a scenario -----------------------------------------------------------------------------------------------

# One line per check for a condition: ok, or FAIL with the detail that shows why.
function Check-True([string]$Name, [bool]$Ok, [string]$Detail = '') {
    '{0} {1}{2}' -f $(if ($Ok) { 'ok  ' } else { 'FAIL' }), $Name, $(if ($Ok -or -not $Detail) { '' } else { " | $Detail" })
}

# Throws when any line of `$Results` is a FAIL, after the lines themselves have been written, so a scenario that
# found a mismatch fails (and says what) instead of printing numbers for somebody to read.
function Assert-Results($Results, [string]$Scenario) {
    $Results
    $failed = @($Results | Where-Object { $_ -like 'FAIL*' })
    if ($failed.Count -gt 0) { throw "$Scenario`: $($failed.Count) of $(@($Results).Count) checks failed" }
}

# --- the strings of the app ------------------------------------------------------------------------------------------------------

# A string of the interface in Traditional Chinese (the scenarios run with language zh-TW), read from the one table
# the app is built from (crates/nekominal-model/src/strings.rs), so a scenario names an element by the words the app
# shows instead of by a copy of them. `{0}`, `{1}` are filled from $With; -Lang en gives the English text, for a run
# whose settings say language en-US.
function Nk-Text([string]$Id, [object[]]$With = @(), [string]$Lang = 'zh') {
    if (-not $script:NkStrings) {
        $table = Join-Path $PSScriptRoot '..\..\crates\nekominal-model\src\strings.rs'
        $script:NkStrings = @{}; $script:NkStringsEn = @{}
        foreach ($m in [regex]::Matches([IO.File]::ReadAllText($table), '(?m)^\s*\w+\s+"([^"]+)"\s+"((?:[^"\\]|\\.)*)"\s+"((?:[^"\\]|\\.)*)";')) {
            $script:NkStrings[$m.Groups[1].Value] = $m.Groups[2].Value.Replace('\"', '"')
            $script:NkStringsEn[$m.Groups[1].Value] = $m.Groups[3].Value.Replace('\"', '"')
        }
        if ($script:NkStrings.Count -lt 100) { throw "read only $($script:NkStrings.Count) strings from $table" }
    }
    $text = if ($Lang -eq 'en') { $script:NkStringsEn[$Id] } else { $script:NkStrings[$Id] }
    if ($null -eq $text) { throw "the table of strings has no '$Id'" }
    for ($i = 0; $i -lt $With.Count; $i++) { $text = $text.Replace("{$i}", [string]$With[$i]) }
    $text
}

# The tooltip (and accessible name) of a button that has a command: its label and the shortcut in brackets.
function Nk-Tip([string]$Id, [string]$Shortcut) { "$(Nk-Text $Id) ($Shortcut)" }

# The title of a chip, without the state its accessible name adds (title and state joined as tab.nameStatus says).
function Chip-Title($Chip) {
    if (-not $Chip.Status) { return $Chip.Name }
    $pattern = '^' + [regex]::Escape((Nk-Text 'tab.nameStatus' @('<0>', $Chip.Status))).Replace('<0>', '(.*)') + '$'
    if ($Chip.Name -match $pattern) { $Matches[1] } else { $Chip.Name }
}

# A rectangle of the first terminal, as a part of its width and height, in the pixels of a screenshot that starts at
# ($OriginX, $OriginY) on the screen: where to sample a region that the shell does not draw into.
function Terminal-Region([double]$Fx, [double]$Fy, [int]$W, [int]$H, [int]$OriginX, [int]$OriginY) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
    $pane = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if (-not $pane) { throw 'no terminal pane' }
    $r = $pane.Current.BoundingRectangle
    @{ X = [int]($r.X + $r.Width * $Fx - $OriginX); Y = [int]($r.Y + $r.Height * $Fy - $OriginY); W = $W; H = $H }
}
