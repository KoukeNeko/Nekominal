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

# Runs `$Body` with the shell started, in a clean environment, and puts the user's clipboard back afterwards.
function Run-Next([scriptblock]$Body, [string]$Settings = '') {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        # A settings folder of its own: what the app saves must not reach the user's settings.
        $global:NextAppData = Join-Path $env:TEMP ('nk-appdata-' + [guid]::NewGuid())
        if ($Settings) { $folder = Join-Path $global:NextAppData 'Nekominal'; [void](New-Item -ItemType Directory -Force $folder); [IO.File]::WriteAllText((Join-Path $folder 'settings.json'), $Settings) }
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

# The whole text of the selected tab, through select all and copy.
function Next-Text {
    Set-Clipboard -Value ''
    Press 0x41 @(0x11, 0x10)    # Ctrl+Shift+A
    Press 0x43 @(0x11, 0x10)    # Ctrl+Shift+C
    Start-Sleep -Milliseconds 300
    [string](Get-Clipboard -Raw)
}

function Next-Line([string]$Text) { Type-Unicode $Text 30; Press 0x0D; Start-Sleep -Milliseconds 900 }

function Scenario-NextTabs {
    Run-Next {
        Next-Line 'echo TAB-ONE'
        $plus = Next-At 248 24; Click-At $plus[0] $plus[1]; Start-Sleep -Milliseconds 1500
        Next-Line 'echo TAB-TWO'
        $text = Next-Text
        Next-Check 'tabs.second-has-its-own-text' ($text -match 'TAB-TWO' -and $text -notmatch 'TAB-ONE') 'the new tab shows what was typed in it only'
        $first = Next-At 60 24; Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 600
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
        $menuX = ($r.Right - $r.Left) / $s - 138 - 12 - 16
        $button = Next-At $menuX 24
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        $row = Next-At ($menuX - 60) (44 + 296)                  # the Mica row of the backdrop group
        Click-At $row[0] $row[1]; Start-Sleep -Milliseconds 1200
        $file = Join-Path $global:NextAppData 'Nekominal\settings.json'
        $saved = if (Test-Path $file) { Get-Content $file -Raw } else { '' }
        Next-Check 'appearance.backdrop-is-saved' ($saved -match '"backdrop":\s*"mica"') "settings.json: $($saved -replace '\s+',' ')"
        Next-Line 'echo STILL-WORKS'
        Next-Check 'appearance.window-still-takes-typing' ((Next-Text) -match 'STILL-WORKS') 'the pane takes the typing after the backdrop changed'
    }
}

function Scenario-NextTabMarks {
    Run-Next {
        $r = Win-Rect; $scale = Get-Scale
        $region = @([int]($r.Left + 12 * $scale), [int]($r.Top + 10 * $scale), [int](40 * $scale), [int](28 * $scale))
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
        $first = Next-At 60 24; Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 800
        ShotRegion $shots[3] $region[0] $region[1] $region[2] $region[3] | Out-Null          # the first tab again, selected
        $left = Next-DiffPixels $shots[0] $shots[3]
        Next-Check 'tabmarks.selecting-the-tab-clears-the-mark' ($left -lt 8) "$left pixels differ from the quiet chip after selecting it"
    }
}

function Scenario-NextMenu {
    Run-Next {
        Next-Line 'echo ONE'
        $r = Win-Rect; $s = Get-Scale
        $menuX = ($r.Right - $r.Left) / $s - 138 - 12 - 16    # the middle of the "..." button, with 138 DIP of caption buttons
        $button = Next-At $menuX 24
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($button[0] - 300) $button[1] 340 300 }
        Next-Check 'menu.is-drawn' ((Next-LightPixels ($button[0] - 300) ($button[1] + 10) 300 250) -gt 100) 'the pop-up shows text, not just its backdrop'
        $row = Next-At ($menuX - 60) 98                        # second row: split right
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

# The middle of the tab-list button in DIP, from the same arithmetic as crates/nekominal-shell/src/layout.rs.
function Next-TabListX([int]$Tabs) {
    $r = Win-Rect; $scale = Get-Scale
    $width = ($r.Right - $r.Left) / $scale
    $menuX = $width - 138 - 12 - 32
    $stripEnd = $menuX - 48
    $available = [Math]::Max(0.0, $stripEnd - 12 - 72)
    $chip = [Math]::Min(216.0, [Math]::Max(48.0, ($available - 4 * ($Tabs - 1)) / $Tabs))
    $shown = [Math]::Min($Tabs, [Math]::Max(1.0, [Math]::Floor(($available + 4) / ($chip + 4) + 0.001)))
    $newTab = [Math]::Max(12.0, [Math]::Min(12 + $shown * ($chip + 4), $stripEnd - 72 - 4))
    $newTab + 36 + 16
}

function Scenario-NextTabList {
    Run-Next {
        Next-Line 'echo MARK-1'
        foreach ($n in 2..6) { Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500; Next-Line "echo MARK-$n" }
        $button = Next-At (Next-TabListX 6) 24
        Click-At $button[0] $button[1]; Start-Sleep -Milliseconds 900
        if ($env:NEXT_SHOT) { ShotRegion $env:NEXT_SHOT ($button[0] - 120) ($button[1] - 20) 420 340 }
        $row = Next-At ((Next-TabListX 6) - 40) (44 + 6 + 32 * 1 + 16)    # the second row: tab 2
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

function Next-PopupWindow {
    $found = [Collections.Generic.List[IntPtr]]::new()
    [void][D]::EnumWindows([D+EnumProc]{ param($h, $l)
        [uint32]$windowPid = 0
        [void][D]::GetWindowThreadProcessId($h, [ref]$windowPid)
        if ($windowPid -eq $script:App.Id -and [D]::IsWindowVisible($h)) {
            $class = [Text.StringBuilder]::new(256)
            [void][D]::GetClassName($h, $class, $class.Capacity)
            if ($class.ToString() -eq 'NekominalPopup') { $found.Add($h) }
        }
        return $true
    }, [IntPtr]::Zero)
    if ($found.Count -gt 1) { throw 'more than one popup in the test app' }
    if ($found.Count -eq 1) { return $found[0] }
    return [IntPtr]::Zero
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
        Next-Check 'confirm.drawn' ((Next-LightPixels ($rect.Left + 15) ($rect.Top + 10) ($rect.Right - $rect.Left - 30) ($rect.Bottom - $rect.Top - 20)) -gt 100) 'question and buttons contain rendered text'
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
        $from = Next-At 2 (48 + 27); $to = Next-At 160 (48 + 27)
        Drag-To $from[0] $from[1] $to[0] $to[1] 15 20
        Press 0x43 @(0x11, 0x10); Start-Sleep -Milliseconds 300
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

# Claude Code in the shell, started the way the WinUI scenarios start it (see claude-scenarios.ps1 for the rules): the
# environment of this session never reaches the app, auto-update and the user's customizations are off, nothing is ever
# sent to the model, and Enter goes to the CLI only when the input box holds exactly /exit. The screen text is not
# printed: it shows account and folder names.
function Scenario-NextClaude([ValidateSet('fullscreen', 'classic')][string]$Mode = 'fullscreen') {
    $claude = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
    $cwd = Join-Path $PSScriptRoot '..\..\target\agent-probe\cwd'
    New-Item -ItemType Directory -Force $cwd | Out-Null
    $cwd = (Resolve-Path $cwd).Path
    $envTable = @{ DISABLE_AUTOUPDATER = '1'; CLAUDE_CODE_SAFE_MODE = '1' }
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
    # The suggestion, the usage notice of the account and the effort indicator come and go with time and account state.
    (($Text -split "`r?`n") | ForEach-Object { if ($_ -match 'Try "') { '<suggestion>' } elseif ($_ -match 'of your weekly limit|◉ \w+ · /effort') { '<status>' } else { $_.TrimEnd() } }) -join "`n"
}

# A storm of window sizes while the CLI runs, then the screen against a fresh run of the CLI at the final size (full-screen
# mode), or one banner in the whole buffer and nothing pushed into the scrollback (classic mode). The sizes change every
# 40 ms, 18 times, the way a drag does; no model call is made.
function Scenario-NextResize([ValidateSet('fullscreen', 'classic')][string]$Mode = 'fullscreen') {
    $claude = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
    $cwd = Join-Path $PSScriptRoot '..\..\target\agent-probe\cwd'
    New-Item -ItemType Directory -Force $cwd | Out-Null
    $cwd = (Resolve-Path $cwd).Path
    $envTable = @{ DISABLE_AUTOUPDATER = '1'; CLAUDE_CODE_SAFE_MODE = '1' }
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
            Next-Check 'resize.fullscreen.same-as-fresh' ($a -ceq $b) ("the screen after the storm and a fresh run at the same size have the same text ({0} and {1} lines)" -f ($a -split "`n").Count, ($b -split "`n").Count)
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
        Drag-To $chip[0] $chip[1] ([int]($r.Left + 500 * $scale)) ([int]($r.Top + 400 * $scale)); Start-Sleep -Milliseconds 1500
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
        Drag-To ([int]($n.Left + 102 * $scale)) ([int]($n.Top + 24 * $scale)) ([int]($r.Left + 700 * $scale)) ([int]($r.Top + 24 * $scale)); Start-Sleep -Milliseconds 2000
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
# The stripes below the window (very transparent for this) must show through the pop-up that is open.
function Next-CheckAcrylic([string]$Name, [string]$Shot) {
    $popup = Next-PopupWindow
    if ($popup -eq [IntPtr]::Zero) { Next-Check "acrylic.$Name-opens" $false 'no pop-up window'; return }
    $rect = New-Object D+RECT; [void][D]::GetWindowRect($popup, [ref]$rect)
    [void](Shot-WithPattern $Shot)
    $window = Win-Rect
    $spread = Next-BackdropSpread $Shot ($rect.Left - $window.Left + 40 + 4) ($rect.Top - $window.Top + 40 + 4) ($rect.Right - $rect.Left - 8) ($rect.Bottom - $rect.Top - 8)
    Next-Check "acrylic.$Name-shows-the-stripes" ($spread -gt 20) ("colour spread {0:N1}" -f $spread)
}

# The menu, the command palette and a confirmation over a window of strong colours.
function Scenario-NextAcrylic {
    Run-Next {
        Start-Pattern
        Place-PatternUnderApp 40
        $r = Win-Rect; $s = Get-Scale
        $menuX = ($r.Right - $r.Left) / $s - 138 - 12 - 16
        $button = Next-At $menuX 24
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
