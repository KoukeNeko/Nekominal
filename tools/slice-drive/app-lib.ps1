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
    [void][P]::SetWindowPos($ph, [IntPtr]::Zero, 3300, 1700, 320, 220, 0x0040)
    [D]::ForceForeground($ph); Start-Sleep -Milliseconds 1200
    $r = Win-Rect; $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2); $app = [uint32]$script:App.Id
    if ([D]::PidAt($cx, $cy) -eq $app -and [D]::PidAt($r.Left + 20, $r.Top + 20) -eq $app -and [D]::PidAt($r.Right - 20, $r.Bottom - 20) -eq $app) { Shot $Path } else { 'inactive shot skipped' }
}

function Stop-Pattern { if ($script:PatternProc -and -not $script:PatternProc.HasExited) { $script:PatternProc.Kill() } }

Add-Type @'
using System; using System.Runtime.InteropServices;
public static class P {
  public static readonly IntPtr HWND_BOTTOM = new IntPtr(1);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
'@
