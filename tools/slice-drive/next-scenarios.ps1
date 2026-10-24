# The self-drawn shell (crates/nekominal-shell, nekominal-next.exe) driven with real input. It has no UI Automation tree
# yet, so what it did is read back through the clipboard (select all, copy) and the process counters.
#
#   . .\tools\slice-drive\next-scenarios.ps1
#   Scenario-NextTabs        # a new tab from the button and from the keyboard, switching, closing: each tab keeps its own text
#   Scenario-NextClipboard   # select by dragging, copy, paste, and a program copying through OSC 52
#   Scenario-NextIme         # Zhuyin: composition and commit
#   Scenario-NextKeys        # Ctrl+U, Ctrl+C, Backspace and CJK text reach the shell
#   Scenario-NextClaude [fullscreen|classic]   # Claude Code starts, takes text and exits, no model call
#   Scenario-NextResize [fullscreen|classic]   # 18 window sizes in a drag-like storm while Claude Code runs, against a fresh run
#   Scenario-NextIdle        # memory and CPU of an idle window
#   Scenario-NextExit        # closing the last window ends the process without forced cleanup
#   Scenario-NextWindows     # two windows in one process; only the last close ends it
#   Scenario-NextTabMove     # pull a chip out into a window of its own, then drop it on the title bar of the other
#   Scenario-NextAcrylic     # the menu, the palette and a confirmation show the blurred window below
#   Scenario-NextAll
#
# A scenario fails (throws) when a check fails. The clipboard of the user is saved and restored.

. (Join-Path $PSScriptRoot 'drive-lib.ps1')
. (Join-Path $PSScriptRoot 'app-lib.ps1')

$global:NextExe = Join-Path $PSScriptRoot '..\..\target\release\nekominal-next.exe'
$global:NextFailures = @()

function Next-Check([string]$Name, [bool]$Ok, [string]$Detail) {
    Write-Host ("{0,-4} {1} | {2}" -f $(if ($Ok) { 'ok' } else { 'FAIL' }), $Name, $Detail)
    if (-not $Ok) { $global:NextFailures += $Name }
}

# A settings folder of its own, so that what the app saves never reaches the user's settings, with `$Settings` (JSON) in it.
# The choice of the first start is made already: the scenarios drive the window, not the welcome screen.
function New-NextAppData([string]$Settings = '{}') {
    $appData = Join-Path $env:TEMP ('nk-appdata-' + [guid]::NewGuid())
    $folder = Join-Path $appData 'Nekominal'; [void](New-Item -ItemType Directory -Force $folder)
    if ($Settings -notmatch '"welcomed"') { $Settings = $Settings -replace '^\s*\{', '{ "welcomed": true,' -replace ',\s*\}\s*$', ' }' }
    [IO.File]::WriteAllText((Join-Path $folder 'settings.json'), $Settings)
    $appData
}

# Runs `$Body` with the shell started, in a clean environment, and puts the user's clipboard back afterwards.
function Run-Next([scriptblock]$Body, [string]$Settings = '{}') {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        $global:NextAppData = New-NextAppData $Settings
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500 -Env @{ APPDATA = $global:NextAppData }
        & $Body
    } finally {
        Stop-Pattern
        Stop-App
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
    if ($global:NextFailures.Count -gt 0) { throw "failed: $($global:NextFailures -join ', ')" }
}

# A point of the window in DIP, as screen pixels.
function Next-At([double]$X, [double]$Y) {
    $r = Win-Rect; $s = Get-Scale
    @([int]($r.Left + $X * $s), [int]($r.Top + $Y * $s))
}

# The middle of the menu button ("...") in DIP for a window `$Width` DIP wide: it is 32 wide and lies against the three
# caption buttons (120 DIP together).
function Next-MenuX([double]$Width) { $Width - 120 - 16 }

# The height of the title bar, in DIP, and the middle of it.
$global:NextBar = 32
function Next-BarY { $global:NextBar / 2 }

# Where a pop-up that hangs from a button of the bar starts, in DIP from the top of the window: the buttons are as tall as
# the bar (32), and the pop-up hangs 4 below them. Its rows start 4 further down and are 32 tall.
function Next-PopupTop { $global:NextBar + 4 }
function Next-PopupRowY([int]$Row) { (Next-PopupTop) + 4 + 32 * $Row + 16 }

# The whole text of the selected tab, through select all and copy.
function Next-Text {
    Set-Clipboard -Value ''
    Press 0x41 @(0x11, 0x10)    # Ctrl+Shift+A
    # Ctrl+Insert, the other copy key: a launcher (Raycast) may take Ctrl+Shift+C for itself before the app sees it.
    Press 0x2D @(0x11)          # Ctrl+Insert
    Start-Sleep -Milliseconds 300
    [string](Get-Clipboard -Raw)
}

function Next-Line([string]$Text) { Type-Unicode $Text 30; Press 0x0D; Start-Sleep -Milliseconds 900 }

function Scenario-NextTabs {
    Run-Next {
        Next-Line 'echo TAB-ONE'
        $plus = Next-At ((Next-TabListX 1) - 32) (Next-BarY); Click-At $plus[0] $plus[1]; Start-Sleep -Milliseconds 1500
        Next-Line 'echo TAB-TWO'
        $text = Next-Text
        Next-Check 'tabs.second-has-its-own-text' ($text -match 'TAB-TWO' -and $text -notmatch 'TAB-ONE') 'the new tab shows what was typed in it only'
        $first = Next-At 60 (Next-BarY); Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 600
        $text = Next-Text
        Next-Check 'tabs.first-keeps-its-text' ($text -match 'TAB-ONE' -and $text -notmatch 'TAB-TWO') 'selecting the first tab shows its own text'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500    # Ctrl+Shift+T
        $text = Next-Text
        Next-Check 'tabs.keyboard-new-tab' ($text -notmatch 'TAB-ONE' -and $text -notmatch 'TAB-TWO') 'Ctrl+Shift+T opens a third, empty tab'
        Press 0x57 @(0x11, 0x10); Start-Sleep -Milliseconds 800    # Ctrl+Shift+W
        Next-Line 'echo AFTER-CLOSE'
        $text = Next-Text
        Next-Check 'tabs.close' ($text -match 'AFTER-CLOSE') 'after closing the third tab the window still takes input'
    }
}

function Scenario-NextSplit {
    Run-Next {
        Next-Line 'echo LEFT-PANE'
        Press 0x44 @(0x12, 0x10); Start-Sleep -Milliseconds 1500    # Alt+Shift+D: split right
        Next-Line 'echo RIGHT-PANE'
        $text = Next-Text
        Next-Check 'split.new-pane-is-focused-and-separate' ($text -match 'RIGHT-PANE' -and $text -notmatch 'LEFT-PANE') 'the new pane has the keyboard and its own text'
        Press 0x25 @(0x12); Start-Sleep -Milliseconds 800           # Alt+Left
        $text = Next-Text
        Next-Check 'split.focus-left' ($text -match 'LEFT-PANE' -and $text -notmatch 'RIGHT-PANE') 'Alt+Left moves the keyboard to the left pane'
        Press 0x53 @(0x12, 0x10); Start-Sleep -Milliseconds 1500    # Alt+Shift+S: split down
        Next-Line 'echo BOTTOM-PANE'
        $text = Next-Text
        Next-Check 'split.down' ($text -match 'BOTTOM-PANE' -and $text -notmatch 'LEFT-PANE') 'a pane split below has its own text'
        Press 0x5A @(0x11, 0x10); Start-Sleep -Milliseconds 500     # Ctrl+Shift+Z: zoom
        Next-Line 'echo ZOOMED'
        Press 0x5A @(0x11, 0x10); Start-Sleep -Milliseconds 500
        $text = Next-Text
        Next-Check 'split.zoom-round-trip' ($text -match 'ZOOMED' -and $text -match 'BOTTOM-PANE') 'zooming and back keeps the pane and its text'
        Press 0x57 @(0x11, 0x10); Start-Sleep -Milliseconds 800     # Ctrl+Shift+W: close pane
        Press 0x57 @(0x11, 0x10); Start-Sleep -Milliseconds 800
        Next-Line 'echo LAST-PANE'
        $text = Next-Text
        Next-Check 'split.close-leaves-one' ($text -match 'LAST-PANE') 'after closing two panes the remaining one takes input'
    }
}

# How many pixels of a screen region are light (text on a dark surface): a pop-up that drew nothing has none.
function Next-LightPixels([int]$X, [int]$Y, [int]$W, [int]$H) {
    Add-Type -AssemblyName System.Drawing
    $path = Join-Path $env:TEMP 'next-light.png'
    [void](ShotRegion $path $X $Y $W $H)
    $bitmap = [System.Drawing.Bitmap]::FromFile($path)
    try {
        $count = 0
        for ($py = 0; $py -lt $bitmap.Height; $py += 2) { for ($px = 0; $px -lt $bitmap.Width; $px += 2) { if ($bitmap.GetPixel($px, $py).GetBrightness() -gt 0.6) { $count++ } } }
        $count
    } finally { $bitmap.Dispose() }
}

# How many sampled pixels differ between two screen regions saved as pictures.
function Next-DiffPixels([string]$A, [string]$B) {
    Add-Type -AssemblyName System.Drawing
    $first = [System.Drawing.Bitmap]::FromFile($A); $second = [System.Drawing.Bitmap]::FromFile($B)
    try {
        $count = 0
        for ($py = 0; $py -lt [Math]::Min($first.Height, $second.Height); $py += 2) {
            for ($px = 0; $px -lt [Math]::Min($first.Width, $second.Width); $px += 2) {
                $p = $first.GetPixel($px, $py); $q = $second.GetPixel($px, $py)
                if ([Math]::Abs($p.R - $q.R) + [Math]::Abs($p.G - $q.G) + [Math]::Abs($p.B - $q.B) -gt 60) { $count++ }
            }
        }
        $count
    } finally { $first.Dispose(); $second.Dispose() }
}

function Scenario-NextFind {
    Run-Next {
        Next-Line 'echo needle-A'; Next-Line 'echo hay'; Next-Line 'echo needle-B'
        $r = Win-Rect
        $region = @(($r.Left + 4), ($r.Top + 80), 700, 300)
        $before = Join-Path $env:TEMP 'find-before.png'; $during = Join-Path $env:TEMP 'find-during.png'; $after = Join-Path $env:TEMP 'find-after.png'
        ShotRegion $before $region[0] $region[1] $region[2] $region[3] | Out-Null
        Press 0x46 @(0x11, 0x10); Start-Sleep -Milliseconds 900       # Ctrl+Shift+F
        Type-Unicode 'needle' 40; Start-Sleep -Milliseconds 700
        ShotRegion $during $region[0] $region[1] $region[2] $region[3] | Out-Null
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($r.Left + 4) ($r.Top + 40) 1620 340 | Out-Null }
        $marked = Next-DiffPixels $before $during
        Next-Check 'find.highlights-the-matches' ($marked -gt 300) "$marked pixels changed once the matches were marked"
        Press 0x1B; Start-Sleep -Milliseconds 700                      # Esc
        ShotRegion $after $region[0] $region[1] $region[2] $region[3] | Out-Null
        $left = Next-DiffPixels $before $after
        Next-Check 'find.escape-clears-them' ($left -lt 80) "$left pixels differ from the start after Esc"
        # Output that arrives while the bar is open is counted too.
        Next-Line 'Start-Sleep 5; echo needle-LATE'
        Press 0x46 @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Type-Unicode 'needle' 40; Start-Sleep -Milliseconds 700
        $popup = Next-PopupWindow
        $bar = New-Object D+RECT; [void][D]::GetWindowRect($popup, [ref]$bar)
        $count1 = Join-Path $env:TEMP 'find-count-1.png'; $count2 = Join-Path $env:TEMP 'find-count-2.png'
        [void](ShotRegion $count1 ($bar.Right - 100) $bar.Top 90 ($bar.Bottom - $bar.Top))
        Start-Sleep -Seconds 6
        [void](ShotRegion $count2 ($bar.Right - 100) $bar.Top 90 ($bar.Bottom - $bar.Top))
        Next-Check 'find.count-follows-new-output' ((Next-DiffPixels $count1 $count2) -gt 10) 'the count in the bar changed when a line with the text came in'
        Press 0x1B; Start-Sleep -Milliseconds 700
        Next-Line 'echo AFTER-FIND'
        Next-Check 'find.pane-takes-typing-again' ((Next-Text) -match 'AFTER-FIND') 'after Esc the pane takes the typing again'
    }
}

function Scenario-NextFullscreen {
    Run-Next {
        Add-Type -AssemblyName System.Windows.Forms
        $before = Win-Rect
        Press 0x7A; Start-Sleep -Milliseconds 1200                     # F11
        $screen = [System.Windows.Forms.Screen]::FromHandle($script:Hwnd).Bounds
        $full = Win-Rect
        Next-Check 'fullscreen.covers-the-monitor' ($full.Left -le $screen.Left -and $full.Top -le $screen.Top -and ($full.Right - $full.Left) -ge $screen.Width -and ($full.Bottom - $full.Top) -ge $screen.Height) "window $($full.Left),$($full.Top)-$($full.Right),$($full.Bottom) on monitor $($screen.Width)x$($screen.Height)"
        Next-Line 'echo IN-FULL'
        Next-Check 'fullscreen.takes-typing' ((Next-Text) -match 'IN-FULL') 'the pane takes the typing in full screen'
        Press 0x7A; Start-Sleep -Milliseconds 1200
        $back = Win-Rect
        Next-Check 'fullscreen.restores-the-window' ($back.Left -eq $before.Left -and $back.Top -eq $before.Top -and $back.Right -eq $before.Right -and $back.Bottom -eq $before.Bottom) "back at $($back.Left),$($back.Top)-$($back.Right),$($back.Bottom)"
    }
}

function Scenario-NextAppearance {
    Run-Next {
        $r = Win-Rect; $s = Get-Scale
        $menuX = Next-MenuX (($r.Right - $r.Left) / $s)
        $button = Next-At $menuX (Next-BarY)
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        $row = Next-At ($menuX - 60) ((Next-PopupTop) + 294)                  # the Mica row of the backdrop group
        Click-At $row[0] $row[1]; Start-Sleep -Milliseconds 1200
        $file = Join-Path $global:NextAppData 'Nekominal\settings.json'
        $saved = if (Test-Path $file) { Get-Content $file -Raw } else { '' }
        Next-Check 'appearance.backdrop-is-saved' ($saved -match '"backdrop":\s*"mica"') "settings.json: $($saved -replace '\s+',' ')"
        Next-Line 'echo STILLWORKS'
        Next-Check 'appearance.window-still-takes-typing' ((Next-Text) -match 'STILLWORKS') 'the pane takes the typing after the backdrop changed'
    }
}

function Scenario-NextTabMarks {
    Run-Next {
        $r = Win-Rect; $scale = Get-Scale
        $region = @([int]($r.Left + 4 * $scale), [int]($r.Top + 4 * $scale), [int](40 * $scale), [int](24 * $scale))
        $shots = 1..4 | ForEach-Object { Join-Path $env:TEMP "tabmark-$_.png" }
        Next-Line 'Start-Sleep 5; [Console]::Out.Write("`e]9;ping`a")'
        Start-Sleep -Milliseconds 300
        ShotRegion $shots[0] $region[0] $region[1] $region[2] $region[3] | Out-Null          # the first tab, selected, quiet
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500                              # a second tab takes the selection
        ShotRegion $shots[1] $region[0] $region[1] $region[2] $region[3] | Out-Null          # the first tab, not selected, before the ping
        Start-Sleep -Seconds 6
        ShotRegion $shots[2] $region[0] $region[1] $region[2] $region[3] | Out-Null          # after the ping rang out of sight
        $marked = Next-DiffPixels $shots[1] $shots[2]
        Next-Check 'tabmarks.notification-marks-the-other-tab' ($marked -gt 8) "$marked pixels of the first chip changed when the ping rang out of sight"
        $first = Next-At 60 (Next-BarY); Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 800
        ShotRegion $shots[3] $region[0] $region[1] $region[2] $region[3] | Out-Null          # the first tab again, selected
        $left = Next-DiffPixels $shots[0] $shots[3]
        Next-Check 'tabmarks.selecting-the-tab-clears-the-mark' ($left -lt 8) "$left pixels differ from the quiet chip after selecting it"
    }
}

function Scenario-NextMenu {
    Run-Next {
        Next-Line 'echo ONE'
        $r = Win-Rect; $s = Get-Scale
        $menuX = Next-MenuX (($r.Right - $r.Left) / $s)    # the middle of the "..." button
        $button = Next-At $menuX (Next-BarY)
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($button[0] - 300) $button[1] 340 300 }
        Next-Check 'menu.is-drawn' ((Next-LightPixels ($button[0] - 300) ($button[1] + 10) 300 250) -gt 100) 'the pop-up shows text, not just its backdrop'
        $row = Next-At ($menuX - 60) (Next-PopupRowY 1)        # second row: split right
        Click-At $row[0] $row[1]; Start-Sleep -Milliseconds 1500
        Next-Line 'echo TWO'
        $text = Next-Text
        Next-Check 'menu.split-right' ($text -match 'TWO' -and $text -notmatch 'ONE') 'the menu row split the pane and the new pane has the keyboard'
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        Press 0x1B; Start-Sleep -Milliseconds 600               # Esc
        Next-Line 'echo AFTER-ESC'
        Next-Check 'menu.escape-closes' ((Next-Text) -match 'AFTER-ESC') 'after Esc the pane takes the typing again'
    }
}

# The middle of the shell button (the one right of +) in DIP, from the same arithmetic as crates/nekominal-shell/src/layout.rs:
# the tabs start 6 from the left edge with 4 between them, as wide as the strip allows, and the buttons are 32 wide and as
# tall as the bar. The right of the bar is taken as empty: a shell without an agent in a folder without git.
function Next-TabListX([int]$Tabs) {
    $r = Win-Rect; $scale = Get-Scale
    $width = ($r.Right - $r.Left) / $scale
    $menuLeft = $width - 120 - 32                       # the menu button, against the three caption buttons
    $stripEnd = $menuLeft - 48                          # what stays free for dragging the window
    $available = [Math]::Max(0.0, $stripEnd - 6 - 64)   # the + and the shell button take 32 each
    $chip = [Math]::Min(216.0, [Math]::Max(48.0, ($available - 4 * ($Tabs - 1)) / $Tabs))
    $shown = [Math]::Min($Tabs, [Math]::Max(1.0, [Math]::Floor(($available + 4) / ($chip + 4) + 0.001)))
    $newTab = [Math]::Max(6.0, [Math]::Min(6 + $shown * ($chip + 4), $stripEnd - 64))
    $newTab + 32 + 16
}

function Scenario-NextTabList {
    Run-Next {
        # The position plate shows only when the strip cannot show every tab: a narrow window and ten tabs.
        Set-WindowWidth 700
        Next-Line 'echo MARK-1'
        foreach ($n in 2..10) { Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500; Next-Line "echo MARK-$n" }
        $button = Next-At ((Next-TabListX 10) + 30) (Next-BarY)    # the position plate, right of the shell button
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($button[0] - 120) ($button[1] - 20) 420 340 }
        $row = Next-At ((Next-TabListX 10) - 40) (Next-PopupRowY 1)    # the second row: tab 2
        Click-At $row[0] $row[1]; Start-Sleep -Milliseconds 800
        $text = Next-Text
        Next-Check 'tablist.select-second' ($text -match 'MARK-2' -and $text -notmatch 'MARK-6') 'choosing the second row of the list selects the second tab'
    }
}

function Scenario-NextPalette {
    Run-Next {
        Next-Line 'echo ONE'
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 900    # Ctrl+Shift+P
        Type-Unicode 'split pane right' 40; Start-Sleep -Milliseconds 500
        $r = Win-Rect
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($r.Left + 200) ($r.Top + 40) 1000 420 }
        Next-Check 'palette.is-drawn' ((Next-LightPixels ($r.Left + 430) ($r.Top + 110) 700 200) -gt 100) 'the palette shows its query and rows, not just its backdrop'
        Press 0x0D; Start-Sleep -Milliseconds 1500                  # Enter: the first match
        Next-Line 'echo TWO'
        $text = Next-Text
        Next-Check 'palette.runs-the-first-match' ($text -match 'TWO' -and $text -notmatch 'ONE') 'typing a name and Enter ran the command: the pane was split'
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 700
        Press 0x1B; Start-Sleep -Milliseconds 500                   # Esc
        Next-Line 'echo AFTER-ESC'
        Next-Check 'palette.escape-closes' ((Next-Text) -match 'AFTER-ESC') 'after Esc the pane takes the typing again'
    }
}

# The one visible window of a class that belongs to the test app: its handle, or zero when there is none.
function Next-WindowOfClass([string]$Class) {
    $found = [Collections.Generic.List[IntPtr]]::new()
    [void][D]::EnumWindows([D+EnumProc]{ param($h, $l)
        [uint32]$windowPid = 0
        [void][D]::GetWindowThreadProcessId($h, [ref]$windowPid)
        if ($windowPid -eq $script:App.Id -and [D]::IsWindowVisible($h)) {
            $name = [Text.StringBuilder]::new(256)
            [void][D]::GetClassName($h, $name, $name.Capacity)
            if ($name.ToString() -eq $Class) { $found.Add($h) }
        }
        return $true
    }, [IntPtr]::Zero)
    if ($found.Count -gt 1) { throw "more than one $Class window in the test app" }
    if ($found.Count -eq 1) { return $found[0] }
    return [IntPtr]::Zero
}

function Next-PopupWindow { Next-WindowOfClass 'NekominalPopup' }

# The window of the settings: a window of its own, owned by the one of the shell.
function Next-SettingsWindow { Next-WindowOfClass 'NekominalSettings' }

# The window of the settings as a UI Automation client sees it; $null while it is not open.
function Next-SettingsRoot {
    $window = Next-SettingsWindow
    if ($window -eq [IntPtr]::Zero) { return $null }
    [System.Windows.Automation.AutomationElement]::FromHandle($window)
}

function Next-ByType($Root, [System.Windows.Automation.ControlType]$Type) {
    $is = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $Type)
    @($Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $is))
}

# The names of the rows the page lists; none while the window is closed.
function Next-SettingsRows {
    $root = Next-SettingsRoot
    if (-not $root) { return @() }
    @(Next-ByType $root ([System.Windows.Automation.ControlType]::ListItem) | ForEach-Object { $_.Current.Name })
}

# What the search line of the page says: '(closed)' without the window.
function Next-SettingsSearch {
    $root = Next-SettingsRoot
    if (-not $root) { return '(closed)' }
    $edit = Next-ByType $root ([System.Windows.Automation.ControlType]::Edit) | Select-Object -First 1
    if (-not $edit) { return '(none)' }
    $pattern = $null
    if ($edit.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { return $pattern.Current.Value }
    ''
}

# Presses a tab of the page (0 is the first section): the pointer goes where UI Automation says the tab is.
function Next-ClickSettingsTab([int]$Index) {
    $tabs = @(Next-ByType (Next-SettingsRoot) ([System.Windows.Automation.ControlType]::TabItem))
    if ($Index -ge $tabs.Count) { throw "the page has $($tabs.Count) tabs, not $($Index + 1)" }
    $at = $tabs[$Index].Current.BoundingRectangle
    Click-At ([int]($at.Left + $at.Width / 2)) ([int]($at.Top + $at.Height / 2)); Start-Sleep -Milliseconds 500
}

function Scenario-NextConfirm {
    Run-Next {
        $payload = 'PASTE-GUARD-' + ('x' * 6000)
        Set-Clipboard -Value $payload
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 700
        $popup = Next-PopupWindow
        Next-Check 'confirm.large-paste-asks' ($popup -ne [IntPtr]::Zero) 'large clipboard text opens a confirmation'
        if ($popup -eq [IntPtr]::Zero) { throw 'confirmation missing; no acceptance keys sent' }
        $rect = [D+RECT]::new(); [void][D]::GetWindowRect($popup, [ref]$rect)
        Next-Check 'confirm.drawn' ((Next-LightPixels ($rect.Left + 15) ($rect.Top + 10) ($rect.Right - $rect.Left - 30) ($rect.Bottom - $rect.Top - 20)) -gt 50) 'question and buttons contain rendered text'    # about 90 with the text alone; a pop-up that drew nothing has none
        Next-Check 'confirm.owner-stays-active' ([D]::GetForegroundWindow() -eq $script:Hwnd) 'the confirmation does not activate a different window'
        Press 0x0D; Start-Sleep -Milliseconds 400
        Next-Check 'confirm.enter-cancels' ((Next-PopupWindow) -eq [IntPtr]::Zero -and (Next-Text) -notmatch 'PASTE-GUARD-') 'Enter initially cancels without inserting clipboard text'
        Set-Clipboard -Value $payload
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 500
        Press 0x73 @(0x12); Start-Sleep -Milliseconds 400  # Alt+F4 dismisses the popup only.
        Next-Check 'confirm.alt-f4-cancels' (-not $script:App.HasExited -and (Next-PopupWindow) -eq [IntPtr]::Zero) 'Alt+F4 cancels the popup and keeps the terminal'
        Press 0x1B; Start-Sleep -Milliseconds 300
        Set-Clipboard -Value $payload
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 500
        Press 0x27; Press 0x0D; Start-Sleep -Milliseconds 600
        Next-Check 'confirm.accept-pastes' ((Next-Text) -match 'PASTE-GUARD-') 'explicit acceptance inserts the held clipboard text'
        Press 0x1B; Start-Sleep -Milliseconds 300
        Next-Line 'Start-Sleep -Seconds 60'
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 600
        Next-Check 'confirm.busy-window-asks' ((Next-PopupWindow) -ne [IntPtr]::Zero) 'a running session requires confirmation'
        Press 0x1B; Start-Sleep -Milliseconds 300
        Next-Check 'confirm.cancel-keeps-session' (-not $script:App.HasExited -and (Next-PopupWindow) -eq [IntPtr]::Zero) 'Escape leaves the running window open'
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 500
        Press 0x27; Press 0x0D
        $ended = $script:App.WaitForExit(5000)
        Next-Check 'confirm.accept-closes-window' $ended 'explicit acceptance closes the running window'
        if ($ended) { Next-Check 'confirm.exit-success' ($script:App.ExitCode -eq 0) "exit code $($script:App.ExitCode)" }
    }
    Run-Next {
        Press 0x73 @(0x12)
        $ended = $script:App.WaitForExit(5000)
        Next-Check 'confirm.alt-f4-idle-window' $ended 'without a popup Alt+F4 closes an idle window normally'
        if ($ended) { Next-Check 'confirm.alt-f4-exit-success' ($script:App.ExitCode -eq 0) "exit code $($script:App.ExitCode)" }
    }
}

function Scenario-NextSplitter {
    Run-Next {
        Press 0x44 @(0x12, 0x10); Start-Sleep -Milliseconds 1500    # Alt+Shift+D: split right
        $width = { Next-Line '$Host.UI.RawUI.WindowSize.Width'; $t = Next-Text; [int](($t -split "`n" | Where-Object { $_ -match '^\s*\d+\s*$' } | Select-Object -Last 1).Trim()) }
        $before = & $width
        $r = Win-Rect; $s = Get-Scale
        $mid = [int](($r.Right - $r.Left) / 2); $y = [int](($r.Top + ($r.Bottom - $r.Top) * 0.6))
        Drag-To ($r.Left + $mid) $y ($r.Left + [int](($r.Right - $r.Left) * 0.25)) $y
        Start-Sleep -Milliseconds 800
        $after = & $width
        Next-Check 'splitter.drag-widens-the-right-pane' ($after -gt $before + 10) "right pane $before -> $after columns"
    }
}

function Scenario-NextClipboard {
    Run-Next {
        Set-Clipboard -Value 'before'
        Next-Line 'echo SELECTME-12345 done'
        # The second row of the terminal holds the output "SELECTME-12345"; drag across its first columns.
        $from = Next-At 2 ($global:NextBar + 27); $to = Next-At 160 ($global:NextBar + 27)
        Drag-To $from[0] $from[1] $to[0] $to[1] 15 20
        Press 0x2D @(0x11); Start-Sleep -Milliseconds 300    # Ctrl+Insert, as in Next-Text
        $copied = [string](Get-Clipboard -Raw)
        Next-Check 'clipboard.drag-copy' ($copied.Trim() -eq 'SELECTME-12345') "copied '$copied'"
        Set-Clipboard -Value 'PASTED-TEXT'
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 600      # Ctrl+Shift+V
        $text = Next-Text
        Next-Check 'clipboard.paste' ($text -match 'PASTED-TEXT') 'pasted text reached the prompt'
        Press 0x1B            # Escape clears the input line of PSReadLine
        $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('from-osc52'))
        Set-Clipboard -Value 'before'
        Next-Line ('[Console]::Out.Write("$([char]27)]52;c;' + $b64 + '$([char]7)")')
        Next-Check 'clipboard.osc52-write' ([string](Get-Clipboard -Raw) -eq 'from-osc52') 'a program copied through OSC 52'
    }
}

function Scenario-NextKeys {
    Run-Next {
        Type-Unicode 'discard this' 30
        Press 0x55 @(0x11)    # Ctrl+U: the editing key reaches the shell, which clears the line
        Next-Line 'echo AFTER-CTRL-U'
        $text = Next-Text
        Next-Check 'keys.ctrl-u' ($text -match 'AFTER-CTRL-U' -and $text -notmatch 'discard thisecho') 'Ctrl+U cleared the typed text'
        Next-Line 'Start-Sleep 30'
        Press 0x43 @(0x11)    # Ctrl+C with no selection is the program's
        Start-Sleep -Milliseconds 800
        Next-Line 'echo AFTER-INTERRUPT'
        $text = Next-Text
        Next-Check 'keys.ctrl-c-interrupts' ($text -match 'AFTER-INTERRUPT') 'Ctrl+C ended the sleeping command'
        Type-Unicode 'abc' 30
        Press 0x08            # Backspace
        Press 0x0D; Start-Sleep -Milliseconds 700
        $text = Next-Text
        Next-Check 'keys.backspace' ($text -match "ab\s*$|'abc'" -or $text -match 'ab: The term') 'Backspace removed one character'
        Next-Line 'echo 你好世界'
        $text = Next-Text
        Next-Check 'keys.cjk' ($text -match '你好世界') 'CJK text arrives intact'
    }
}

# Claude Code in the shell, under these rules: the
# environment of this session never reaches the app, auto-update and the user's customizations are off, nothing is ever
# sent to the model, and Enter goes to the CLI only when the input box holds exactly /exit. The screen text is not
# printed: it shows account and folder names.
function Scenario-NextClaude([ValidateSet('fullscreen', 'classic')][string]$Mode = 'fullscreen') {
    $claude = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
    $cwd = Join-Path $PSScriptRoot '..\..\target\agent-probe\cwd'
    New-Item -ItemType Directory -Force $cwd | Out-Null
    $cwd = (Resolve-Path $cwd).Path
    # A settings folder of its own, as in Run-Next: closing the window saves a session, which must not replace the user's.
    $envTable = @{ DISABLE_AUTOUPDATER = '1'; CLAUDE_CODE_SAFE_MODE = '1'; APPDATA = (New-NextAppData) }
    if ($Mode -eq 'fullscreen') { $envTable['CLAUDE_CODE_NO_FLICKER'] = '1' } else { $envTable['CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN'] = '1' }
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500 -Env $envTable
        Next-Line "Set-Location '$cwd'"
        Next-Line "& '$claude'"
        Start-Sleep -Seconds 9
        $text = Next-Text
        Next-Check "claude.$Mode.ui" ($text -match 'Claude Code' -and $text -match '[─]{10}') 'the banner and the rules of the input box are on the screen'
        $middle = Next-At 500 300
        Move-To $middle[0] $middle[1]
        [D]::Mouse(0x0800, 120); Start-Sleep -Milliseconds 300
        [D]::Mouse(0x0800, [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]-120), 0)); Start-Sleep -Milliseconds 300
        Type-Unicode 'ab你好' 60; Start-Sleep -Milliseconds 800
        Next-Check "claude.$Mode.typing" ((Next-Text) -match 'ab你好') 'typed text, CJK included, reaches the input box after the wheel moved over it'
        Press 0x55 @(0x11)
        Type-Unicode '/exit' 60; Start-Sleep -Milliseconds 500
        $guard = Next-Text
        if ($guard -match 'ab你好' -or $guard -notmatch '/exit') { throw 'the input box does not hold exactly /exit: Enter not sent' }
        Press 0x0D; Start-Sleep -Seconds 4
        Next-Check "claude.$Mode.exit" ((Next-Text) -match 'PS .*>\s*$') 'after /exit the prompt of the shell is back'
    } finally {
        Stop-App
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
    if ($global:NextFailures.Count -gt 0) { throw "failed: $($global:NextFailures -join ', ')" }
}
# Microsoft Bopomofo through real key presses: s u 3 is the syllable for ni3 and the first candidate is 你, c l 3 is hao3
# and 好. The composition is drawn inline at the cursor (its picture is kept for looking at), Enter commits it, and the
# committed text reaches the shell once.
function Scenario-NextIme {
    Run-Next {
        Press 0x53; Press 0x55; Press 0x33; Start-Sleep -Milliseconds 700
        # Not Next-Text here: its chords would commit the composition.
        Shot (Join-Path ([IO.Path]::GetTempPath()) 'nekominal-drive\next-ime-preedit.png') | Out-Null
        Press 0x0D; Start-Sleep -Milliseconds 900
        Press 0x43; Press 0x4C; Press 0x33; Start-Sleep -Milliseconds 500
        Press 0x0D; Start-Sleep -Milliseconds 900
        $text = Next-Text
        Next-Check 'ime.commit' ($text -match '你好') "the composed text 你好 reached the prompt"
        Next-Check 'ime.once' (([regex]::Matches($text, '你')).Count -eq 1) 'each committed character appears once'
    }
}
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class NextW {
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int h2, uint flags);
}
'@

# Sets the size of the window in DIP (SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE).
function Next-Size([double]$Width, [double]$Height) {
    $s = Get-Scale
    [void][NextW]::SetWindowPos($script:Hwnd, [IntPtr]::Zero, 0, 0, [int]($Width * $s), [int]($Height * $s), 0x0016)
}

# Ends the CLI the way a person does, and only when the input box holds exactly /exit: nothing else may reach the model.
function Next-ExitClaude {
    Press 0x55 @(0x11)
    Type-Unicode '/exit' 60; Start-Sleep -Milliseconds 500
    if ((Next-Text) -notmatch '/exit') { throw 'the input box does not hold /exit: Enter not sent' }
    Press 0x0D; Start-Sleep -Seconds 4
}

# The text of a CLI screen with what changes from run to run (the suggestion in the empty box) made equal.
function Next-Normalized([string]$Text) {
    # The suggestion, the usage notice of the account, the effort indicator and the hint about agents in the footer come and
    # go with time and account state.
    (($Text -split "`r?`n") | ForEach-Object { if ($_ -match 'Try "') { '<suggestion>' } elseif ($_ -match 'of your weekly limit|\S \w+ · /effort') { '<status>' } else { ($_ -replace ' · ← for agents', '').TrimEnd() } }) -join "`n"
}

# A storm of window sizes while the CLI runs, then the screen against a fresh run of the CLI at the final size (full-screen
# mode), or one banner in the whole buffer and nothing pushed into the scrollback (classic mode). The sizes change every
# 40 ms, 18 times, the way a drag does; no model call is made.
function Scenario-NextResize([ValidateSet('fullscreen', 'classic')][string]$Mode = 'fullscreen') {
    $claude = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
    $cwd = Join-Path $PSScriptRoot '..\..\target\agent-probe\cwd'
    New-Item -ItemType Directory -Force $cwd | Out-Null
    $cwd = (Resolve-Path $cwd).Path
    # A settings folder of its own, as in Run-Next: closing the window saves a session, which must not replace the user's.
    $envTable = @{ DISABLE_AUTOUPDATER = '1'; CLAUDE_CODE_SAFE_MODE = '1'; APPDATA = (New-NextAppData) }
    if ($Mode -eq 'fullscreen') { $envTable['CLAUDE_CODE_NO_FLICKER'] = '1' } else { $envTable['CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN'] = '1' }
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500 -Env $envTable
        Next-Line "Set-Location '$cwd'"
        Next-Line "& '$claude'"
        Start-Sleep -Seconds 9
        $sizes = @(@(900, 600), @(1300, 800), @(700, 500), @(1100, 700), @(560, 420), @(1200, 760), @(800, 540), @(1000, 900), @(640, 480),
                   @(1180, 640), @(760, 700), @(980, 560), @(600, 800), @(1240, 720), @(720, 520), @(1060, 780), @(840, 600), @(1000, 640))
        foreach ($size in $sizes) { Next-Size $size[0] $size[1]; Start-Sleep -Milliseconds 40 }
        Start-Sleep -Seconds 2
        $storm = Next-Text
        $banners = ([regex]::Matches($storm, 'Claude Code v\d')).Count
        if ($Mode -eq 'classic') {
            Next-Check 'resize.classic.one-banner' ($banners -eq 1) "the whole buffer holds $banners copies of the banner after $($sizes.Count) resizes"
        }
        Next-Check "resize.$Mode.alive" ($storm -match 'Claude Code' -and $storm -match '[─]{10}') 'the CLI redrew to the last size: banner and input box rules are there'
        if ($Mode -eq 'fullscreen') {
            Next-ExitClaude
            Next-Line "& '$claude'"
            Start-Sleep -Seconds 9
            $fresh = Next-Text
            $a = Next-Normalized $storm; $b = Next-Normalized $fresh
            $first = ''
            if ($a -cne $b) {
                $left = $a -split "`n"; $right = $b -split "`n"
                for ($i = 0; $i -lt [Math]::Max($left.Count, $right.Count); $i++) { if ($left[$i] -cne $right[$i]) { $first = "; first difference at line $($i + 1): [$($left[$i])] against [$($right[$i])]"; break } }
            }
            Next-Check 'resize.fullscreen.same-as-fresh' ($a -ceq $b) ("the screen after the storm and a fresh run at the same size have the same text ({0} and {1} lines){2}" -f ($a -split "`n").Count, ($b -split "`n").Count, $first)
        }
        Next-ExitClaude
    } finally {
        Stop-App
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
    if ($global:NextFailures.Count -gt 0) { throw "failed: $($global:NextFailures -join ', ')" }
}

function Scenario-NextIdle {
    Run-Next {
        Start-Sleep -Seconds 3
        $app = $script:App; $app.Refresh()
        $private = $app.PrivateMemorySize64 / 1MB
        Next-Check 'idle.memory' ($private -lt 200) ("{0:N0} MB private, {1:N0} MB working set" -f $private, ($app.WorkingSet64 / 1MB))
        $before = $app.TotalProcessorTime.TotalMilliseconds; Start-Sleep -Seconds 5; $app.Refresh()
        $spent = $app.TotalProcessorTime.TotalMilliseconds - $before
        Next-Check 'idle.cpu' ($spent -lt 100) ("{0:N0} ms of CPU in 5 s while nothing happens" -f $spent)
    }
}

# Close through WM_CLOSE, not the harness's forced cleanup. This needs no keyboard or foreground ownership.
function Scenario-NextExit {
    Run-Next {
        $app = $script:App
        Next-Check 'exit.close-request' ($app.CloseMainWindow()) 'the app received WM_CLOSE at its idle prompt'
        $exited = $app.WaitForExit(5000)
        Next-Check 'exit.process-ends' $exited 'closing the last window ends the process within 5 seconds'
        if ($exited) { Next-Check 'exit.success' ($app.ExitCode -eq 0) "exit code $($app.ExitCode)" }
    }
}

function Scenario-NextTabMove {
    Run-Next {
        $app = $script:App; $first = $script:Hwnd
        Next-Line 'echo TAB-ONE'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Line 'echo TAB-TWO'
        $scale = Get-Scale
        # Tear off: the second chip (216 DIP wide, 4 apart) is let go of in the middle of the window.
        $r = Win-Rect
        $chip = @([int]($r.Left + 322 * $scale), [int]($r.Top + 24 * $scale))
        Move-To $chip[0] $chip[1]; Start-Sleep -Milliseconds 200
        [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 200
        $goal = @([int]($r.Left + 500 * $scale), [int]($r.Top + 400 * $scale))
        for ($k = 1; $k -le 30; $k++) { Move-To ([int]($chip[0] + ($goal[0] - $chip[0]) * $k / 30)) ([int]($chip[1] + ($goal[1] - $chip[1]) * $k / 30)); Start-Sleep -Milliseconds 20 }
        Start-Sleep -Milliseconds 800
        # The button is still down: the tab is already a window, and the pointer holds it by its chip.
        $windows = @([D]::AllWindows([uint32]$app.Id))
        Next-Check 'tabmove.window-exists-while-held' ($windows.Count -eq 2) "$($windows.Count) windows while the chip is still held"
        $held = $windows | Where-Object { $_ -ne $first } | Select-Object -First 1
        if ($held) {
            $h1 = New-Object D+RECT; [void][D]::GetWindowRect($held, [ref]$h1)
            $onBar = $goal[0] -ge $h1.Left -and $goal[0] -le $h1.Right -and $goal[1] -ge $h1.Top -and $goal[1] -le ($h1.Top + $global:NextBar * $scale)
            Next-Check 'tabmove.window-follows-the-pointer' $onBar "the pointer ($($goal[0]),$($goal[1])) is on the title bar of the new window ($($h1.Left),$($h1.Top))-($($h1.Right),$($h1.Bottom))"
            Move-To ($goal[0] + 150) $goal[1]; Start-Sleep -Milliseconds 300
            $h2 = New-Object D+RECT; [void][D]::GetWindowRect($held, [ref]$h2)
            Next-Check 'tabmove.window-keeps-following' ([Math]::Abs(($h2.Left - $h1.Left) - 150) -le 3) "the window moved $($h2.Left - $h1.Left) px for 150 px of the pointer"
        }
        [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 1200
        $windows = @([D]::AllWindows([uint32]$app.Id))
        Next-Check 'tabmove.tear-off-makes-a-window' ($windows.Count -eq 2) "$($windows.Count) windows after pulling a chip out"
        $second = $windows | Where-Object { $_ -ne $first } | Select-Object -First 1
        if (-not $second) { return }
        [D]::ForceForeground($second); Start-Sleep -Milliseconds 500
        $text = Next-Text
        Next-Check 'tabmove.new-window-has-the-tab' ($text -match 'TAB-TWO' -and $text -notmatch 'TAB-ONE') 'the new window shows the tab that was pulled out, with its history'
        [D]::ForceForeground($first); Start-Sleep -Milliseconds 500
        $text = Next-Text
        Next-Check 'tabmove.old-window-keeps-the-other' ($text -match 'TAB-ONE' -and $text -notmatch 'TAB-TWO') 'the first window shows only its own tab'
        # Merge: the chip of the new window is let go of on the title bar of the first one. The new window starts lower
        # than that bar, so raising it hides none of it.
        [D]::ForceForeground($second); Start-Sleep -Milliseconds 500
        $script:Hwnd = $second
        $n = Win-Rect
        $script:Hwnd = $first
        $r = Win-Rect
        $from = @([int]($n.Left + 102 * $scale), [int]($n.Top + 24 * $scale)); $to = @([int]($r.Left + 700 * $scale), [int]($r.Top + 24 * $scale))
        Move-To $from[0] $from[1]; Start-Sleep -Milliseconds 200
        [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 200
        for ($k = 1; $k -le 30; $k++) { Move-To ([int]($from[0] + ($to[0] - $from[0]) * $k / 30)) ([int]($from[1] + ($to[1] - $from[1]) * $k / 30)); Start-Sleep -Milliseconds 20 }
        Start-Sleep -Milliseconds 900
        # Held over the strip of the first window, the tab is in it already and the pointer goes on holding it.
        $windows = @([D]::AllWindows([uint32]$app.Id))
        Next-Check 'tabmove.joins-while-held' ($windows.Count -eq 1) "$($windows.Count) windows while the chip is held over the other strip"
        [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 1200
        $windows = @([D]::AllWindows([uint32]$app.Id))
        Next-Check 'tabmove.merge-closes-the-emptied-window' ($windows.Count -eq 1 -and $windows[0] -eq $first) "$($windows.Count) window left"
        [D]::ForceForeground($first); Start-Sleep -Milliseconds 500
        $r = Win-Rect
        $region = @(($r.Left + 4), ($r.Top + 80), 700, 300)
        $before = Join-Path $env:TEMP 'tabmove-before.png'; $after = Join-Path $env:TEMP 'tabmove-after.png'
        ShotRegion $before $region[0] $region[1] $region[2] $region[3] | Out-Null
        Next-Line 'echo AFTER-MOVE'
        ShotRegion $after $region[0] $region[1] $region[2] $region[3] | Out-Null
        Next-Check 'tabmove.output-after-the-move-is-drawn' ((Next-DiffPixels $before $after) -gt 200) 'what the moved tab prints reaches the screen of its new window'
        $text = Next-Text
        Next-Check 'tabmove.merged-tab-is-selected-and-alive' ($text -match 'TAB-TWO' -and $text -match 'AFTER-MOVE') 'the merged tab is the selected one, its session still takes input'
        Click-At ([int]($r.Left + 90 * $scale)) ([int]($r.Top + 24 * $scale)); Start-Sleep -Milliseconds 700
        Next-Check 'tabmove.first-tab-still-there' ((Next-Text) -match 'TAB-ONE') 'the first tab is still in the window'
    }
}

function Scenario-NextWindows([switch]$Inspect) {
    Run-Next {
        $app = $script:App
        $first = $script:Hwnd
        $app.Refresh(); $oneWindowMB = $app.PrivateMemorySize64 / 1MB
        # Read the application message id from its source instead of keeping a second definition.
        $source = Get-Content (Join-Path $PSScriptRoot '../../crates/nekominal-shell/src/win.rs') -Raw
        if ($source -notmatch 'const WM_NEW_WINDOW: u32 = WM_APP \+ (\d+);') { throw 'new-window message not found' }
        $message = [uint32](0x8000 + [int]$Matches[1])
        Next-Check 'windows.new-request' ([D]::PostMessage($first, $message, [UIntPtr]::Zero, [IntPtr]::Zero)) 'new-window request queued'
        $deadline = [DateTime]::UtcNow.AddSeconds(8)
        do {
            Start-Sleep -Milliseconds 100
            $windows = @([D]::AllWindows([uint32]$app.Id))
        } while ($windows.Count -lt 2 -and [DateTime]::UtcNow -lt $deadline)
        Next-Check 'windows.two-in-one-process' ($windows.Count -eq 2) "$($windows.Count) visible windows belong to the same process"
        if ($Inspect) {
            Write-Host ('info windows.inspect | handles ' + (($windows | ForEach-Object { $_.ToInt64() }) -join ', '))
            Start-Sleep -Seconds 15
        }
        Start-Sleep -Seconds 2
        $app.Refresh()
        Write-Host ('info windows.memory | one {0:N1} MB, two {1:N1} MB private' -f $oneWindowMB, ($app.PrivateMemorySize64 / 1MB))
        Next-Check 'windows.first-close-request' ([D]::PostMessage($first, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)) 'close the first idle window'
        Start-Sleep -Milliseconds 800
        $remaining = @([D]::AllWindows([uint32]$app.Id))
        Next-Check 'windows.other-survives' (-not $app.HasExited -and $remaining.Count -eq 1 -and $remaining[0] -ne $first) 'the other window stays alive'
        if ($remaining.Count -eq 1) {
            [void][D]::PostMessage($remaining[0], 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)
            $ended = $app.WaitForExit(5000)
            Next-Check 'windows.last-close-exits' $ended 'the last close ends the process within 5 seconds'
            if ($ended) { Next-Check 'windows.exit-success' ($app.ExitCode -eq 0) "exit code $($app.ExitCode)" }
        }
    }
}

function Scenario-NextAll { Scenario-NextTabs; Scenario-NextClipboard; Scenario-NextKeys; Scenario-NextIme; Scenario-NextResize fullscreen; Scenario-NextResize classic; Scenario-NextClaude fullscreen; Scenario-NextClaude classic; Scenario-NextIdle; Scenario-NextExit }

# How much of the picture behind a pop-up shows through it: the mean colour spread (largest channel minus smallest) of
# the pixels that are not text. A flat surface has none (0.0 measured with the system backdrop, 33 with the blur).
function Next-BackdropSpread([string]$Path, [int]$X, [int]$Y, [int]$W, [int]$H) {
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $spread = 0.0; $count = 0
        for ($py = $Y; $py -lt $Y + $H; $py += 3) {
            for ($px = $X; $px -lt $X + $W; $px++) {
                $c = $bitmap.GetPixel($px, $py)
                if ($c.GetBrightness() -gt 0.6) { continue }
                $spread += [Math]::Max($c.R, [Math]::Max($c.G, $c.B)) - [Math]::Min($c.R, [Math]::Min($c.G, $c.B)); $count++
            }
        }
        if ($count) { $spread / $count } else { 0 }
    } finally { $bitmap.Dispose() }
}
# The stripes below the window (very transparent for this) must show through the pop-up that is open. A flat surface has no
# spread at all; the stripes behind a pop-up may be grey for a good part of it (18 measured for the menu, 26 to 29 for the
# palette and the confirmation), so the line is 10.
function Next-CheckAcrylic([string]$Name, [string]$Shot) {
    $popup = Next-PopupWindow
    if ($popup -eq [IntPtr]::Zero) { Next-Check "acrylic.$Name-opens" $false 'no pop-up window'; return }
    $rect = New-Object D+RECT; [void][D]::GetWindowRect($popup, [ref]$rect)
    [void](Shot-WithPattern $Shot)
    $window = Win-Rect
    $spread = Next-BackdropSpread $Shot ($rect.Left - $window.Left + 40 + 4) ($rect.Top - $window.Top + 40 + 4) ($rect.Right - $rect.Left - 8) ($rect.Bottom - $rect.Top - 8)
    Next-Check "acrylic.$Name-shows-the-stripes" ($spread -gt 10) ("colour spread {0:N1}" -f $spread)
}

# The menu, the command palette and a confirmation over a window of strong colours.
function Scenario-NextAcrylic {
    Run-Next {
        Start-Pattern
        Place-PatternUnderApp 40
        $r = Win-Rect; $s = Get-Scale
        $menuX = Next-MenuX (($r.Right - $r.Left) / $s)
        $button = Next-At $menuX (Next-BarY)
        $dir = if ($env:NEXT_SHOT) { Split-Path $env:NEXT_SHOT } else { $env:TEMP }
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        Next-CheckAcrylic 'menu' (Join-Path $dir 'acrylic-menu.png')
        Press 0x1B; Start-Sleep -Milliseconds 500
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 900    # Ctrl+Shift+P
        Next-CheckAcrylic 'palette' (Join-Path $dir 'acrylic-palette.png')
        Press 0x1B; Start-Sleep -Milliseconds 500
        Set-Clipboard -Value ('x' * 6000)
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Next-CheckAcrylic 'confirm' (Join-Path $dir 'acrylic-confirm.png')
        Press 0x1B; Start-Sleep -Milliseconds 300
    } '{ "appearance": { "opacity": 0.2 } }'
}

# What the app has kept in the settings file of this run.
function Next-SavedSettings {
    $path = Join-Path $global:NextAppData 'Nekominal\settings.json'
    if (Test-Path $path) { Get-Content $path -Raw | ConvertFrom-Json } else { $null }
}

# The settings: a window of their own that opens from the keyboard, its rows are driven by the keys and the pointer, what
# they change is kept in the settings file, and a screen reader is told of its tabs, its search line and its rows. The
# palette opens it at a row and sets a colour scheme.
function Scenario-NextSettings {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        Next-Line 'echo ONE'
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200         # Ctrl+Shift+O (Ctrl+, is taken by the Zhuyin IME)
        $listed = Next-SettingsRows
        Next-Check 'settings.opens-in-a-window-of-its-own' ((Next-SettingsWindow) -ne [IntPtr]::Zero -and $listed -contains 'Background opacity') "the window lists its rows: $($listed.Count) rows"
        Next-Check 'settings.adds-no-tab-to-the-shell' (@(Next-TabNames).Count -eq 1) "tabs of the shell: $(@(Next-TabNames) -join ' | ')"
        $window = Next-SettingsWindow; $s = Get-Scale
        $shot = if ($env:NEXT_SHOT) { Join-Path (Split-Path $env:NEXT_SHOT) 'settings-window.png' } else { Join-Path $env:TEMP 'settings-window.png' }
        [void](Shot-Window $window $shot)
        $bounds = New-Object D+RECT; [void][D]::GetWindowRect($window, [ref]$bounds)
        Next-Check 'settings.is-drawn' ((Next-LightPixels ($bounds.Left + 240) ($bounds.Top + [int](150 * $s)) 600 300) -gt 100) 'the rows show text, not just a backdrop'
        1..6 | ForEach-Object { Press 0x28 }; Start-Sleep -Milliseconds 300   # Down x6: the opacity row, after the layout of the tabs, the sidebar, the groups, the scheme and the backdrop
        Press 0x27; Press 0x27; Press 0x27; Start-Sleep -Milliseconds 500     # Right x3
        $saved = Next-SavedSettings
        Next-Check 'settings.right-steps-the-value-and-keeps-it' ($saved -and $saved.appearance.opacity -eq 0.75) "opacity in the file: $($saved.appearance.opacity)"
        Press 0x0D; Start-Sleep -Milliseconds 500                             # Enter: one more
        Next-Check 'settings.enter-steps-it' ((Next-SavedSettings).appearance.opacity -eq 0.76) "opacity $((Next-SavedSettings).appearance.opacity)"
        Type-Unicode 'opacity' 30; Start-Sleep -Milliseconds 500              # the search line
        $listed = Next-SettingsRows
        Next-Check 'settings.search-narrows-the-rows' ($listed.Count -ge 1 -and $listed.Count -lt 5 -and $listed -contains 'Background opacity') "$($listed.Count) rows match: $($listed -join ', ')"
        Next-Check 'settings.search-line-says-what-was-typed' ((Next-SettingsSearch) -eq 'opacity') "the search says: $(Next-SettingsSearch)"
        Press 0x28; Press 0x25; Start-Sleep -Milliseconds 500                 # Down to the first match, Left: one back
        Next-Check 'settings.left-steps-back' ((Next-SavedSettings).appearance.opacity -eq 0.75) "opacity $((Next-SavedSettings).appearance.opacity)"
        Press 0x1B; Start-Sleep -Milliseconds 400                             # Esc: clears the search, the window stays
        Next-Check 'settings.escape-clears-the-search' ((Next-SettingsRows).Count -gt 5 -and (Next-SettingsWindow) -ne [IntPtr]::Zero) 'all the rows of the section are back'
        Next-ClickSettingsTab 1                                               # the second tab with the pointer
        $listed = Next-SettingsRows
        Next-Check 'settings.tab-with-the-pointer-switches-the-rows' ($listed -contains 'Copy on select' -and $listed -notcontains 'Background opacity') "rows now: $($listed.Count)"
        Press 0x09; Start-Sleep -Milliseconds 400                             # Tab: the next section
        Next-Check 'settings.tab-key-switches-sections' ((Next-SettingsRows) -notcontains 'Copy on select') 'the third section has other rows'
        Press 0x1B; Start-Sleep -Milliseconds 900                             # Esc: closes the window
        Next-Line 'echo AFTER'
        Next-Check 'settings.escape-closes-the-window' ((Next-Text) -match 'AFTER' -and (Next-SettingsWindow) -eq [IntPtr]::Zero) 'the window is gone and the terminal takes the typing again'
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 900              # palette
        Type-Unicode 'color scheme: nord' 30; Start-Sleep -Milliseconds 500
        Press 0x0D; Start-Sleep -Milliseconds 800
        Next-Check 'settings.palette-sets-the-colour-scheme' ((Next-SavedSettings).theme -eq 'Nord') "theme in the file: $((Next-SavedSettings).theme)"
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Type-Unicode 'settings: language' 30; Start-Sleep -Milliseconds 500
        Press 0x0D; Start-Sleep -Milliseconds 1200
        Next-Check 'settings.palette-opens-the-row' ((Next-SettingsRows) -contains 'Language') 'a settings entry of the palette opens the window at its section'
        Press 0x1B; Start-Sleep -Milliseconds 900
        Press 0x4E @(0x11, 0x10); Start-Sleep -Milliseconds 900              # Ctrl+Shift+N: the profiles
        Next-Check 'profiles.list-opens' ((Next-PopupWindow) -ne [IntPtr]::Zero) 'Ctrl+Shift+N lists the profiles'
        Press 0x28; Press 0x0D; Start-Sleep -Milliseconds 1800
        Next-Line 'echo FROM-PROFILE'
        $text = Next-Text
        Next-Check 'profiles.choice-opens-a-tab' ($text -match 'FROM-PROFILE' -and $text -notmatch 'AFTER') 'choosing a profile opens a new tab of its own'
    }
}

# The window as a UI Automation client (a screen reader) sees it: its tabs, the text of its terminal, an open menu.
function Scenario-NextAccess {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        Next-Line 'echo ACCESS-MARK'
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $all = {
            $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
                ForEach-Object {
                    $pattern = $null; $value = ''
                    if ($_.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { $value = $pattern.Current.Value }
                    [pscustomobject]@{ Type = $_.Current.ControlType.ProgrammaticName; Name = $_.Current.Name; Value = $value }
                }
        }
        $found = @(& $all)
        Next-Check 'access.window-has-a-name' ($root.Current.Name -eq 'Nekominal') "window name: $($root.Current.Name)"
        Next-Check 'access.tab-is-listed' (@($found | Where-Object { $_.Type -eq 'ControlType.TabItem' }).Count -eq 1) 'one tab item for the one tab'
        Next-Check 'access.terminal-text-is-readable' (@($found | Where-Object { $_.Type -eq 'ControlType.Document' -and $_.Value -match 'ACCESS-MARK' }).Count -eq 1) 'the terminal element holds the text on the screen'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        $found = @(& $all)
        Next-Check 'access.new-tab-is-listed' (@($found | Where-Object { $_.Type -eq 'ControlType.TabItem' }).Count -eq 2) 'a second tab item after a new tab'
        Next-Check 'access.terminal-follows-the-selected-tab' (@($found | Where-Object { $_.Type -eq 'ControlType.Document' -and $_.Value -match 'ACCESS-MARK' }).Count -eq 0) 'the text of the new tab replaces the first one'
        $r = Win-Rect; $s = Get-Scale
        $button = Next-At (Next-MenuX (($r.Right - $r.Left) / $s)) (Next-BarY)
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        $found = @(& $all)
        Next-Check 'access.menu-rows-are-listed' (@($found | Where-Object { $_.Type -eq 'ControlType.MenuItem' }).Count -gt 5) "$(@($found | Where-Object { $_.Type -eq 'ControlType.MenuItem' }).Count) menu items while the menu is open"
        Press 0x1B; Start-Sleep -Milliseconds 400
    }
}

# The rows of the title bar in which a cell of the top right corner has light pixels (the glyph of a caption button).
function Next-GlyphRows([string]$Path, [int]$X, [int]$W, [int]$H) {
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $rows = @()
        for ($py = 0; $py -lt $H; $py++) {
            for ($px = $X; $px -lt $X + $W; $px++) { if ($bitmap.GetPixel($px, $py).GetBrightness() -gt 0.7) { $rows += $py; break } }
        }
        $rows
    } finally { $bitmap.Dispose() }
}

# The minimize, maximize and close buttons: their glyphs in the middle of the title bar, and what each does.
function Scenario-NextCaption {
    Run-Next {
        $r = Win-Rect; $s = Get-Scale
        $bar = [int]($global:NextBar * $s); $cell = [int](40 * $s)
        $shot = Join-Path $env:TEMP 'next-caption.png'
        [void](ShotRegion $shot ($r.Right - 3 * $cell - 8) $r.Top (3 * $cell + 8) $bar)
        $names = 'minimize', 'maximize', 'close'
        for ($i = 0; $i -lt 3; $i++) {
            $rows = @(Next-GlyphRows $shot (8 + $i * $cell + 6) ($cell - 12) $bar)
            $middle = if ($rows.Count) { ($rows[0] + $rows[-1]) / 2 } else { -1 }
            Next-Check "caption.$($names[$i])-glyph-is-centred" ($rows.Count -gt 0 -and [Math]::Abs($middle - $bar / 2) -le 2) "glyph rows $($rows[0])..$($rows[-1]), middle $middle of a bar $bar px tall"
        }
        # Hovering a button lays the plate of the bar's buttons under its glyph, in the colours of the scheme.
        Add-Type -AssemblyName System.Drawing
        $plate = {
            param($path, $i)
            $bitmap = [System.Drawing.Bitmap]::FromFile($path)
            try { $c = $bitmap.GetPixel([int](8 + $i * $cell + 8 * $s), [int]($bar / 2)); $c.R + $c.G + $c.B } finally { $bitmap.Dispose() }
        }
        Move-To ([int]($r.Left + 300)) ([int]($r.Top + 300)); Start-Sleep -Milliseconds 700
        [void](ShotRegion $shot ($r.Right - 3 * $cell - 8) $r.Top (3 * $cell + 8) $bar)
        $idle = 0..2 | ForEach-Object { & $plate $shot $_ }
        for ($i = 0; $i -lt 3; $i++) {
            Move-To ([int]($r.Right - (3 - $i) * $cell + $cell / 2)) ([int]($r.Top + $bar / 2)); Start-Sleep -Milliseconds 700
            [void](ShotRegion $shot ($r.Right - 3 * $cell - 8) $r.Top (3 * $cell + 8) $bar)
            $lit = & $plate $shot $i
            Next-Check "caption.$($names[$i])-hover-plate" ($lit - $idle[$i] -gt 12) "the plate lifts the sum of the colour channels by $($lit - $idle[$i])"
        }
        $button = { param($i) @([int]($r.Right - (3 - $i) * $cell + $cell / 2), [int]($r.Top + $bar / 2)) }
        $max = & $button 1
        Click-At $max[0] $max[1]; Start-Sleep -Milliseconds 900
        Next-Check 'caption.maximize' ([D]::IsZoomed($script:Hwnd)) 'the middle button maximizes the window'
        $r = Win-Rect
        $max = @([int]($r.Right - 2 * $cell + $cell / 2), [int]($r.Top + $bar / 2))
        Click-At $max[0] $max[1]; Start-Sleep -Milliseconds 900
        Next-Check 'caption.restore' (-not [D]::IsZoomed($script:Hwnd)) 'the same button restores it'
        $r = Win-Rect
        $min = @([int]($r.Right - 3 * $cell + $cell / 2), [int]($r.Top + $bar / 2))
        Click-At $min[0] $min[1]; Start-Sleep -Milliseconds 900
        Next-Check 'caption.minimize' ([D]::IsIconic($script:Hwnd)) 'the left button minimizes the window'
        [void][D]::ShowWindow($script:Hwnd, 9); Start-Sleep -Milliseconds 600
        $r = Win-Rect
        $close = @([int]($r.Right - $cell / 2), [int]($r.Top + $bar / 2))
        Click-At $close[0] $close[1]; Start-Sleep -Milliseconds 1500
        $script:App.Refresh()
        Next-Check 'caption.close' ($script:App.HasExited) 'the right button closes the window and the process ends'
    }
}

function Next-WindowTitle {
    $text = [Text.StringBuilder]::new(256)
    [void][D]::GetWindowText($script:Hwnd, $text, 256)
    $text.ToString()
}

# The tabs of the shell window. The window of the settings is owned by it, so UI Automation shows that window under this
# one, with the tabs of its sections: those are not tabs of the shell and are left out.
function Next-TabNames {
    $tabType = [System.Windows.Automation.ControlType]::TabItem
    $ofSettings = @{}
    $settings = Next-SettingsRoot
    if ($settings) { foreach ($tab in Next-ByType $settings $tabType) { $ofSettings[($tab.GetRuntimeId() -join '.')] = $true } }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    @(Next-ByType $root $tabType | Where-Object { -not $ofSettings.ContainsKey(($_.GetRuntimeId() -join '.')) } | ForEach-Object { $_.Current.Name })
}

# What the settings change in the window: the title of a tab, the mark of a bell, the menu of the right button, the title
# of the window while it waits for the user.
function Scenario-NextBehavior {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        Next-Line 'Set-Location C:\Windows'
        Start-Sleep -Milliseconds 800
        $names = Next-TabNames
        Next-Check 'behavior.tab-title-names-the-folder' ($names -contains 'Windows') "tab titles: $($names -join ' | ')"
    }
    foreach ($visual in $true, $false) {
        Run-Next {
            $r = Win-Rect; $scale = Get-Scale
            $region = @([int]($r.Left + 4 * $scale), [int]($r.Top + 4 * $scale), [int](40 * $scale), [int](24 * $scale))
            $quiet = Join-Path $env:TEMP 'bell-quiet.png'; $rung = Join-Path $env:TEMP 'bell-rung.png'
            Next-Line 'Start-Sleep 5; [Console]::Out.Write([char]7)'
            Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
            [void](ShotRegion $quiet $region[0] $region[1] $region[2] $region[3])
            Start-Sleep -Seconds 6
            [void](ShotRegion $rung $region[0] $region[1] $region[2] $region[3])
            $marked = Next-DiffPixels $quiet $rung
            if ($visual) { Next-Check 'behavior.bell-marks-the-tab' ($marked -gt 8) "$marked pixels of the chip changed" }
            else { Next-Check 'behavior.bell-mark-can-be-turned-off' ($marked -lt 8) "$marked pixels of the chip changed with bell.visual off" }
        } ('{ "bell": { "visual": ' + $visual.ToString().ToLower() + ' } }')
    }
    Run-Next {
        $r = Win-Rect; $scale = Get-Scale
        Click-At ([int]($r.Left + 400 * $scale)) ([int]($r.Top + 300 * $scale)) 'right'; Start-Sleep -Milliseconds 900
        $popup = Next-PopupWindow
        Next-Check 'behavior.right-click-opens-the-menu' ($popup -ne [IntPtr]::Zero) 'rightClick menu opens a menu'
        if ($popup -ne [IntPtr]::Zero) {
            $rect = New-Object D+RECT; [void][D]::GetWindowRect($popup, [ref]$rect)
            Next-Check 'behavior.right-click-menu-is-drawn' ((Next-LightPixels ($rect.Left + 10) ($rect.Top + 10) ($rect.Right - $rect.Left - 20) ($rect.Bottom - $rect.Top - 20)) -gt 20) 'the menu of two rows (Paste, Select all) shows them: 47 sampled light pixels, none when nothing is drawn'
        }
        Press 0x1B; Start-Sleep -Milliseconds 400
    } '{ "rightClick": "menu" }'
    Run-Next {
        Start-Pattern
        Place-PatternUnderApp 0
        Next-Line 'exit 3'
        Start-Sleep -Milliseconds 800
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Check 'behavior.title-is-plain-while-active' ((Next-WindowTitle) -eq 'Nekominal') "title: $(Next-WindowTitle)"
        if (Move-PatternAway 320 220) { [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 1200 }
        Next-Check 'behavior.title-counts-the-tabs-that-wait' ((Next-WindowTitle) -eq '!1 Nekominal') "title: $(Next-WindowTitle)"
        Stop-Pattern
    }
}

# The tab strip: the shell button lists the shells and does not light the menu button, a chip dragged over its neighbour changes the order and
# follows the pointer on the way, a closed tab makes the others slide over several frames instead of jumping, and the
# strip comes to rest. The places of the chips are read from the tree a screen reader gets, which carries what is drawn.
function Scenario-NextBar {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
    Run-Next {
        $r = Win-Rect; $s = Get-Scale
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $isTab = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)
        $chips = { @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $isTab) | ForEach-Object { [pscustomobject]@{ Name = $_.Current.Name; Rect = $_.Current.BoundingRectangle } }) }
        $menuX = Next-MenuX (($r.Right - $r.Left) / $s)
        $plate = { param($path) $b = [System.Drawing.Bitmap]::FromFile($path); try { $at = Next-At ($menuX - 12) 24; $c = $b.GetPixel($at[0] - $r.Left, $at[1] - $r.Top); $c.R + $c.G + $c.B } finally { $b.Dispose() } }
        $bar = [int](60 * $s)
        Move-To ([int]($r.Left + 300)) ([int]($r.Top + 400)); Start-Sleep -Milliseconds 600
        $idle = Join-Path $env:TEMP 'bar-idle.png'; $open = Join-Path $env:TEMP 'bar-open.png'
        [void](ShotRegion $idle $r.Left $r.Top ($r.Right - $r.Left) $bar)
        $list = Next-At (Next-TabListX 1) (Next-BarY)
        Click-At $list[0] $list[1]; Start-Sleep -Milliseconds 900
        [void](ShotRegion $open $r.Left $r.Top ($r.Right - $r.Left) $bar)
        Next-Check 'bar.shell-button-opens-the-list-of-shells' ((Next-PopupWindow) -ne [IntPtr]::Zero) 'the button right of + lists the shells a new tab can start'
        Next-Check 'bar.shell-button-does-not-light-the-menu-button' ([Math]::Abs((& $plate $open) - (& $plate $idle)) -le 3) "menu plate sum idle $(& $plate $idle), list open $(& $plate $open)"
        Press 0x1B; Start-Sleep -Milliseconds 500

        Next-Line 'echo AAA'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Line 'echo BBB'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Line 'echo CCC'
        Start-Sleep -Milliseconds 600
        $centre = { param($c) @([int](($c.Rect.Left + $c.Rect.Right) / 2), [int](($c.Rect.Top + $c.Rect.Bottom) / 2)) }
        # What each chip, from the left, shows when it is chosen: the tabs by what was typed in them.
        $order = {
            $letters = foreach ($c in (& $chips)) {
                $at = & $centre $c
                Click-At $at[0] $at[1]; Start-Sleep -Milliseconds 500
                $text = Next-Text
                foreach ($name in 'AAA', 'BBB', 'CCC') { if ($text -match "echo $name") { $name } }
            }
            ($letters -join ',')
        }
        $now = & $chips
        $got = & $order; Next-Check "bar.tabs-in-order" ($got -eq "AAA,BBB,CCC") "the chips from the left show: $got"

        # Hold the third chip, move it over the first, and look at the chips on the way, then let go.
        $now = & $chips
        $third = & $centre $now[2]; $first = & $centre $now[0]
        Move-To $third[0] $third[1]; Start-Sleep -Milliseconds 200
        [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 200
        $steps = 20
        for ($k = 1; $k -le $steps; $k++) { Move-To ([int]($third[0] + ($first[0] - 10 * $s - $third[0]) * $k / $steps)) $third[1]; Start-Sleep -Milliseconds 25 }
        Start-Sleep -Milliseconds 700
        $during = & $chips
        $held = $during[2]
        Next-Check 'bar.held-chip-follows-the-pointer' ([Math]::Abs((($held.Rect.Left + $held.Rect.Right) / 2) - ($first[0] - 10 * $s)) -lt 12 * $s) "the chip is at $([int](($held.Rect.Left + $held.Rect.Right) / 2)), the pointer at $([int]($first[0] - 10 * $s)) (the chip stops at the start of the strip)"
        Next-Check 'bar.other-chips-make-way' ($during[0].Rect.Left -gt $now[0].Rect.Left + 20 * $s) "the first chip moved from $([int]$now[0].Rect.Left) to $([int]$during[0].Rect.Left)"
        [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 900
        $got = & $order; Next-Check "bar.dragging-a-chip-reorders-the-tabs" ($got -eq "CCC,AAA,BBB") "the chips from the left show: $got"
        $after = & $chips
        Next-Check 'bar.dropped-chip-settles-in-its-slot' ([Math]::Abs($after[0].Rect.Left - $now[0].Rect.Left) -lt 2 * $s) "first chip at $([int]$after[0].Rect.Left), the first slot at $([int]$now[0].Rect.Left)"

        # Close the first tab: the others slide into its place over several frames.
        $first = & $centre $after[0]
        Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 600
        Press 0x57 @(0x11, 0x10) 40 0    # Ctrl+Shift+W closes the selected tab, the first one; no wait: the slide has begun
        $seen = [Collections.Generic.List[double]]::new()
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while ($clock.ElapsedMilliseconds -lt 700) {
            $now2 = & $chips
            if ($now2.Count -eq 2) { $seen.Add([Math]::Round($now2[1].Rect.Left)) }
        }
        $distinct = @($seen | Select-Object -Unique)
        Next-Check 'bar.closing-a-tab-slides-the-others' ($distinct.Count -ge 4) "the second chip was seen at $($distinct.Count) places on its way: $(($distinct | Select-Object -First 8) -join ' ')"
        Next-Check 'bar.strip-comes-to-rest' (@($seen | Select-Object -Last 5 | Select-Object -Unique).Count -eq 1) 'the chip stands still at the end'
    }
}

# Ctrl+Shift+V goes into the line of text that has the keyboard: the search of the settings window and the query of the
# palette, not the terminal of the shell.
function Scenario-NextPaste {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        Next-Line 'echo ONE'
        Set-Clipboard -Value 'opacity'
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        $all = (Next-SettingsRows).Count
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 700
        $found = Next-SettingsRows
        Next-Check 'paste.into-the-search-of-the-page' ($found.Count -lt $all -and $found -contains 'Background opacity') "$all rows, $($found.Count) after the paste"
        Press 0x1B; Start-Sleep -Milliseconds 300; Press 0x1B; Start-Sleep -Milliseconds 900
        Next-Check 'paste.did-not-reach-the-terminal' ((Next-Text) -notmatch 'opacity') 'the terminal of the shell did not get the text'
        Set-Clipboard -Value 'split pane right'
        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 700
        Press 0x0D; Start-Sleep -Milliseconds 1500
        Next-Line 'echo TWO'
        $text = Next-Text
        Next-Check 'paste.into-the-palette-query' ($text -match 'TWO' -and $text -notmatch 'ONE') 'the pasted name ran the command: the pane was split'
    }
}

# restoreSession: what the window had when it was closed (the tabs and the folders they were in) is there again at the next
# start, in new shells.
function Scenario-NextRestore {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        Next-Line 'Set-Location C:\Windows'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Line 'Set-Location C:\Windows\System32'
        $first = $script:App
        Press 0x73 @(0x12); Start-Sleep -Milliseconds 1500      # Alt+F4: it asks only when a shell is busy
        if (-not $first.HasExited) { Press 0x27; Press 0x0D }
        [void]$first.WaitForExit(10000)
        Next-Check 'restore.window-closed' $first.HasExited 'the window closed after the confirmation'
        $file = Join-Path $global:NextAppData 'Nekominal\session.json'
        Next-Check 'restore.session-is-written' ((Test-Path $file) -and ((Get-Content $file -Raw) -match 'System32')) 'the session file names the folders'
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500 -Env @{ APPDATA = $global:NextAppData }
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $isTab = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)
        $count = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $isTab).Count
        Next-Check 'restore.tabs-come-back' ($count -eq 2) "$count tabs after the start"
        Next-Check 'restore.selected-tab-keeps-its-folder' ((Next-Text) -match 'System32>') 'the second tab, which was selected, is in the folder it was in'
        $chip = Next-At 60 24; Click-At $chip[0] $chip[1]; Start-Sleep -Milliseconds 700
        Next-Check 'restore.first-tab-keeps-its-folder' ((Next-Text) -match 'Windows>' -and (Next-Text) -notmatch 'System32') 'the first tab is in its folder too'
    } '{ "restoreSession": true }'
}

# The shortcuts page: Enter on a command waits for a chord, the chord becomes its shortcut at once and is kept in the settings,
# a chord that programs in the terminal need is refused, Delete puts the default back.
function Scenario-NextKeybind {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        $tabs = { @(Next-TabNames).Count }
        $bindings = { $s = Next-SavedSettings; if ($s -and $s.keybindings) { ($s.keybindings | ConvertTo-Json -Compress) } else { '' } }
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200         # the settings window
        1..4 | ForEach-Object { Press 0x09 }; Start-Sleep -Milliseconds 400    # Tab x4: the shortcuts section
        Press 0x28; Start-Sleep -Milliseconds 300                         # Down: the first command
        Press 0x0D; Start-Sleep -Milliseconds 400                         # Enter: wait for a chord
        Press 0x43 @(0x11); Start-Sleep -Milliseconds 400                 # Ctrl+C: a terminal program needs it
        Next-Check 'keybind.a-chord-programs-need-is-refused' ((& $bindings) -eq '') "bindings: $(& $bindings)"
        Press 0x59 @(0x11, 0x10); Start-Sleep -Milliseconds 700           # Ctrl+Shift+Y (Ctrl+Alt is AltGr: programs need it)
        $kept = & $bindings
        Next-Check 'keybind.chord-is-kept-in-the-settings' ($kept -match 'Ctrl\+Shift\+Y') "bindings: $kept"
        Press 0x1B; Start-Sleep -Milliseconds 900                         # closes the window
        $before = & $tabs
        Press 0x59 @(0x11, 0x10); Start-Sleep -Milliseconds 1500
        Next-Check 'keybind.new-chord-runs-the-command' ((& $tabs) -eq $before + 1) "tabs $before -> $(& $tabs)"
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        1..4 | ForEach-Object { Press 0x09 }; Start-Sleep -Milliseconds 300
        Press 0x28; Start-Sleep -Milliseconds 300
        Press 0x2E; Start-Sleep -Milliseconds 600                         # Delete: the default again
        Next-Check 'keybind.delete-puts-the-default-back' ((& $bindings) -notmatch 'Ctrl\+Shift\+Y') "bindings: $(& $bindings)"
        Press 0x1B; Start-Sleep -Milliseconds 900
    }
}

# kitty keyboard protocol: a program that asks for key release events (flag 2) gets them, in the shell's own window.
function Scenario-NextKittyKeys {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        $script = Join-Path $env:TEMP 'nk-kitty-keys.js'
        Set-Content -Path $script -Encoding ascii -Value @'
process.stdout.write('\x1b[>11u');
process.stdin.setRawMode(true);
process.stdin.on('data', d => { const s = JSON.stringify(d.toString('latin1')); console.log('KEYS' + s.replace(/\u001b/g, 'ESC')); if (s.includes('q')) { process.stdout.write('\x1b[<u'); process.exit(0); } });
'@
        Next-Line "node '$script'"
        Start-Sleep -Milliseconds 1200
        Press 0x74; Start-Sleep -Milliseconds 600       # F5: the input method takes the letters
        # Read from the tree a screen reader gets: the clipboard shortcuts of the shell are not what is looked at here.
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $document = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document))
        $text = $document.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
        Next-Check 'kitty.release-is-reported' ($text -match 'KEYS' -and $text -match '15;1:3~') "what the program read: $((($text -split "`n") | Where-Object { $_ -match 'KEYS' }) -join ' | ')"
        Press 0x51; Start-Sleep -Milliseconds 600
    } '{ "keyboard": { "kittyProtocol": true } }'
}

# The lines of text: the caret and the selection in the palette, the search of the settings page and the fields of its rows, a
# font family that is put in effect at once, and a find bar that stays open when the terminal is clicked.
function Scenario-NextTextFields {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $isList = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::List)
        # What the line says: the tree of the window names the list that has it after the text.
        $line = { $e = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $isList); if ($e) { $e.Current.Name } else { '(none)' } }
        Next-Line 'echo ONE'

        Press 0x50 @(0x11, 0x10); Start-Sleep -Milliseconds 800            # the palette
        Type-Unicode 'plit pane' 30; Press 0x24; Start-Sleep -Milliseconds 200; Type-Unicode 's' 30; Start-Sleep -Milliseconds 400
        Next-Check 'fields.palette-home-and-typing' ((& $line) -eq 'split pane') "the query: $(& $line)"
        Press 0x41 @(0x11); Press 0x43 @(0x11); Start-Sleep -Milliseconds 300
        Next-Check 'fields.palette-select-all-and-copy' ((Get-Clipboard -Raw).Trim() -eq 'split pane') "clipboard: $((Get-Clipboard -Raw).Trim())"
        Type-Unicode 'zoom' 30; Start-Sleep -Milliseconds 300
        Next-Check 'fields.typing-replaces-the-selection' ((& $line) -eq 'zoom') "the query: $(& $line)"
        Press 0x25 @(0x10); Press 0x25 @(0x10); Start-Sleep -Milliseconds 200; Press 0x2E; Start-Sleep -Milliseconds 300
        Next-Check 'fields.shift-arrows-select-and-delete-removes' ((& $line) -eq 'zo') "the query: $(& $line)"
        Press 0x1B; Start-Sleep -Milliseconds 500

        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200           # the settings window
        Type-Unicode 'opcity' 30; 1..4 | ForEach-Object { Press 0x25 }; Type-Unicode 'a' 30; Start-Sleep -Milliseconds 500
        Next-Check 'fields.page-search-caret' ((Next-SettingsSearch) -eq 'opacity') "the search: $(Next-SettingsSearch)"
        Press 0x1B; Start-Sleep -Milliseconds 300                           # clears the search
        1..7 | ForEach-Object { Press 0x28 }; Start-Sleep -Milliseconds 300    # Down x7: the font row, under the rows of the layout of the tabs
        $r = Win-Rect; $s = Get-Scale
        $before = Join-Path $env:TEMP 'font-before.png'; $after = Join-Path $env:TEMP 'font-after.png'
        Press 0x1B; Start-Sleep -Milliseconds 900                           # closes the window: the shell shows what is under it
        [void](ShotRegion $before ($r.Left + 4) ($r.Top + [int](60 * $s)) 700 200)
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        1..7 | ForEach-Object { Press 0x28 }; Start-Sleep -Milliseconds 300
        Press 0x0D; Start-Sleep -Milliseconds 300                           # Enter: edit, the text selected
        Type-Unicode 'Courier New' 30; Press 0x0D; Start-Sleep -Milliseconds 700
        $saved = Next-SavedSettings
        Next-Check 'fields.field-edit-is-kept' ((ConvertTo-Json $saved.font -Compress) -match 'Courier New') "font in the file: $(ConvertTo-Json $saved.font -Compress)"
        Press 0x1B; Start-Sleep -Milliseconds 900
        [void](ShotRegion $after ($r.Left + 4) ($r.Top + [int](60 * $s)) 700 200)
        Next-Check 'fields.font-family-reaches-the-open-terminal' ((Next-DiffPixels $before $after) -gt 200) "$(Next-DiffPixels $before $after) pixels of the terminal changed with the font"

        # The find bar stays when the terminal is clicked, and the keyboard is the terminal's then.
        Next-Line 'echo needle-A'
        Press 0x46 @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Type-Unicode 'needle' 30; Start-Sleep -Milliseconds 500
        $pane = Next-At 400 300; Click-At $pane[0] $pane[1]; Start-Sleep -Milliseconds 500
        Next-Check 'fields.find-bar-stays-after-a-click-outside' ((Next-PopupWindow) -ne [IntPtr]::Zero) 'the bar is still open'
        Next-Line 'echo TYPED-IN-TERMINAL'
        Next-Check 'fields.keyboard-is-the-terminals-again' ((Next-Text) -match 'TYPED-IN-TERMINAL' -and (& $line) -eq 'needle') "the bar still says: $(& $line)"
        $bar = New-Object D+RECT; [void][D]::GetWindowRect((Next-PopupWindow), [ref]$bar)
        Click-At ([int](($bar.Left + $bar.Right) / 2)) ([int](($bar.Top + $bar.Bottom) / 2)); Start-Sleep -Milliseconds 400
        Type-Unicode 'zz' 30; Start-Sleep -Milliseconds 400
        Next-Check 'fields.click-on-the-bar-gives-it-the-keyboard' ((& $line) -eq 'needlezz') "the bar says: $(& $line)"
        Press 0x1B; Start-Sleep -Milliseconds 500
    }
}

# The glyphs of the title bar sit on its middle line: the + and the shell button, the menu button, and the mark that
# stands before the title of a tab that asks for something.
function Scenario-NextCentre {
    Add-Type -AssemblyName System.Drawing
    Run-Next {
        $r = Win-Rect; $s = Get-Scale
        Next-Line 'exit 3'
        Start-Sleep -Milliseconds 1200
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500          # a second tab: the first one shows its failure mark
        Move-To ([int]($r.Left + 300)) ([int]($r.Top + 500)); Start-Sleep -Milliseconds 600
        $path = Join-Path $env:TEMP 'centre.png'
        [void](ShotRegion $path $r.Left $r.Top ($r.Right - $r.Left) ([int](60 * $s)))
        $bitmap = [System.Drawing.Bitmap]::FromFile($path)
        try {
            $menuX = Next-MenuX (($r.Right - $r.Left) / $s)
            $shellX = Next-TabListX 2
            $spots = [ordered]@{ 'the mark before a title' = 19; 'the + button' = $shellX - 32; 'the shell button' = $shellX; 'the menu button' = $menuX }
            foreach ($name in $spots.Keys) {
                $rows = @()
                # The glyphs of the buttons are quiet and the mark of a failure red: both lighter than the bar. The rows stop
                # above the line under the selected tab.
                $light = 0.4
                for ($py = [int](2 * $s); $py -lt [int](29 * $s); $py++) {
                    for ($px = [int](($spots[$name] - 10) * $s); $px -le [int](($spots[$name] + 10) * $s); $px++) { if ($bitmap.GetPixel($px, $py).GetBrightness() -gt $light) { $rows += $py; break } }
                }
                $middle = if ($rows.Count) { ($rows[0] + $rows[-1]) / 2 } else { -100 }
                Next-Check "centre.$($name -replace ' ', '-')" ([Math]::Abs($middle - (Next-BarY) * $s) -le 1.2) ("{0}: the glyph is at {1} px, the middle of the bar at {2} px" -f $name, $middle, ((Next-BarY) * $s))
            }
        } finally { $bitmap.Dispose() }
    }
}

# The settings are a window of their own: opening makes a window and no tab, the shell takes the typing while the window is
# open, opening again brings back the same window with what was typed in it, and the shortcut and Escape close it.
function Scenario-NextSettingsWindow {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    Run-Next {
        $windows = { @([D]::AllWindows([uint32]$script:App.Id)).Count }
        Next-Line 'echo FIRST-TAB'
        Next-Check 'settingswindow.none-at-the-start' ((Next-SettingsWindow) -eq [IntPtr]::Zero -and (& $windows) -eq 1 -and @(Next-TabNames).Count -eq 1) "windows: $(& $windows), tabs: $(@(Next-TabNames) -join ' | ')"
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        $window = Next-SettingsWindow
        Next-Check 'settingswindow.opening-makes-a-window-and-no-tab' ($window -ne [IntPtr]::Zero -and (& $windows) -eq 2 -and @(Next-TabNames).Count -eq 1) "windows: $(& $windows), tabs: $(@(Next-TabNames) -join ' | ')"
        Next-Check 'settingswindow.rows-are-listed' ((Next-SettingsRows).Count -gt 5) 'the rows of the first section are listed'
        Next-Check 'settingswindow.has-the-keyboard-when-it-opens' ([D]::ForegroundPid() -eq [uint32]$script:App.Id) 'the window that came up is the one the keyboard is in'
        Type-Unicode 'opacity' 30; Start-Sleep -Milliseconds 400
        Next-Check 'settingswindow.takes-the-typing' ((Next-SettingsSearch) -eq 'opacity') "the search says: $(Next-SettingsSearch)"
        # The title row is drawn by the window, as in the shell: its buttons act and what they leave of it drags the window.
        $s = Get-Scale
        $frame = { $f = New-Object D+RECT; if ([D]::DwmGetWindowAttribute($window, 9, [ref]$f, 16) -ne 0) { [void][D]::GetWindowRect($window, [ref]$f) }; $f }
        $button = { param($i) $f = & $frame; @([int]($f.Right - (3 - $i) * 46 * $s + 23 * $s), [int]($f.Top + 20 * $s)) }
        $at = & $button 1; Click-At $at[0] $at[1]; Start-Sleep -Milliseconds 1100
        Next-Check 'settingswindow.maximize-button-maximizes' ([D]::IsZoomed($window)) 'the middle button of the title row maximizes the window'
        $at = & $button 1; Click-At $at[0] $at[1]; Start-Sleep -Milliseconds 1100
        Next-Check 'settingswindow.maximize-button-restores' (-not [D]::IsZoomed($window)) 'the same button restores it'
        $before = (& $frame).Left; $top = & $frame
        Drag-To ([int]($top.Left + 300 * $s)) ([int]($top.Top + 20 * $s)) ([int]($top.Left + 380 * $s)) ([int]($top.Top + 60 * $s)) 12 20
        $moved = (& $frame).Left - $before
        Next-Check 'settingswindow.title-row-drags-the-window' ([Math]::Abs($moved - 80 * $s) -le 4) "the window moved $moved px for a drag of $([int](80 * $s)) px"
        Next-Check 'settingswindow.keeps-its-search-through-all-that' ((Next-SettingsSearch) -eq 'opacity') "the search says: $(Next-SettingsSearch)"
        # Back to the shell: the window stays open and the typing goes to the terminal.
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 700
        Next-Line 'echo STILLWORKS'
        Next-Check 'settingswindow.shell-takes-typing-while-it-is-open' ((Next-Text) -match 'STILLWORKS' -and (Next-SettingsSearch) -eq 'opacity') "the search says: $(Next-SettingsSearch)"
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 900
        Next-Check 'settingswindow.opening-again-brings-back-the-same-window' ((Next-SettingsWindow) -eq $window -and (Next-SettingsSearch) -eq 'opacity' -and (& $windows) -eq 2) "windows: $(& $windows), the search says: $(Next-SettingsSearch)"
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 900            # the shortcut of the settings, in the window: closes it
        Next-Check 'settingswindow.shortcut-closes-it' ((Next-SettingsWindow) -eq [IntPtr]::Zero -and (& $windows) -eq 1) "windows: $(& $windows)"
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        Next-Check 'settingswindow.opens-afresh-after-closing' ((Next-SettingsWindow) -ne [IntPtr]::Zero -and (Next-SettingsSearch) -eq '') "the search says: $(Next-SettingsSearch)"
        Press 0x1B; Start-Sleep -Milliseconds 900                           # Esc, with nothing in the search: closes it
        Next-Check 'settingswindow.escape-closes-it' ((Next-SettingsWindow) -eq [IntPtr]::Zero -and (& $windows) -eq 1) "windows: $(& $windows)"
        Next-Line 'echo AFTER-CLOSE'
        Next-Check 'settingswindow.terminal-after-closing' ((Next-Text) -match 'AFTER-CLOSE') 'the terminal takes the typing again'
        Press 0x4F @(0x11, 0x10); Start-Sleep -Milliseconds 1200
        $window = Next-SettingsWindow
        $close = & $button 2
        Click-At $close[0] $close[1]; Start-Sleep -Milliseconds 1200
        Next-Check 'settingswindow.close-button-closes-it' ((Next-SettingsWindow) -eq [IntPtr]::Zero -and (& $windows) -eq 1) "windows: $(& $windows)"
    }
}
