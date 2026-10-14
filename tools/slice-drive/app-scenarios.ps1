# The scenarios that drive nekominal.exe with real SendInput and guarded screenshots. Dot-source, then:
#
#   . .\tools\slice-drive\app-scenarios.ps1
#   Scenario-Glass            # glass-dark.png, glass-light.png
#   Scenario-BlurProof        # blur-proof.png, backdrop-*.png, opacity-*.png, blur-light.png over the striped test window
#   Scenario-TabsSplits       # tabs-splits.png plus tabs-focus-left, pane-zoom, tab-reorder, tab-middle-click-confirm, tab-middle-closed
#   Scenario-Overlays         # palette, find bar, appearance flyout, tab menu, close dialog, settings pages
#   Scenario-Inactive         # the window while another window is active`n#   Scenario-Ime              # Bopomofo composition in two panes (ime-preedit, ime-two-panes)
#   Scenario-Silence          # sound witness over the trigger matrix (needs an audio endpoint), Alt+Space, held Alt+Shift+D
#   Scenario-Opacity          # live opacity pixels on an idle terminal, 200-event replay, 20 backdrop switches, backdrop None
#   Scenario-All
#
# Every scenario goes through run-app.ps1, so the safety rules there apply: input only while the foreground window
# is the started process, screenshots only of that window (plus the margin of the pattern window this script owns),
# a private APPDATA so the user's settings.json is never touched. Screenshots land in docs/screenshots; logs and the
# private APPDATA in %TEMP%\nekominal-drive. Dot-source this file from a fresh pwsh; do not dot-source run-app.ps1.
#
# Chords used (Ctrl+Shift+digit is left out on purpose: the Bopomofo input method owns Ctrl+Shift+1):
#   Ctrl+Shift+T new tab, Ctrl+Tab next tab, Alt+Shift+D / Alt+Shift+S split right / down, Alt+Left focus left,
#   Ctrl+Shift+Z zoom pane, Ctrl+Shift+P palette, Ctrl+Shift+F find, Ctrl+Shift+W close pane, Ctrl+Shift+O settings.

$global:NkRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$global:NkShots = Join-Path $global:NkRoot 'docs\screenshots'
New-Item -ItemType Directory -Force $global:NkShots | Out-Null
$global:NkRun = Join-Path $PSScriptRoot 'run-app.ps1'

function Shot-Path([string]$Name) { Join-Path $global:NkShots "$Name.png" }

function First-Prompt { Wait-Log 'pane 1 first frame' | Out-Null; Wait-Prompt | Out-Null }
function New-Tab { Chord @(0x11, 0x10) 0x54; Wait-Prompt | Out-Null }
function Split-Right { Chord @(0x12, 0x10) 0x44; Wait-Prompt | Out-Null }
function Split-Down { Chord @(0x12, 0x10) 0x53; Wait-Prompt | Out-Null }

# Coloured text, CJK, emoji and a table: what the terminal area shows in most screenshots. PSReadLine's grey inline
# predictions are switched off first so the picture shows only what the shell printed.
function Fill-Terminal {
    First-Prompt
    Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
    Type-Line 'Write-Host "`e[31mred `e[32mgreen `e[33myellow `e[34mblue `e[35mmagenta `e[36mcyan`e[0m  你好 世界  日本語  😀 ✔"'; Wait-Prompt | Out-Null
    Type-Line 'Get-ChildItem $env:windir | Select-Object -First 4 Mode, Length, Name'; Wait-Prompt | Out-Null
}

# Names the current tab through the title the shell reports (what a program does with OSC 0/2).
function Name-Tab([string]$Title) {
    Type-Line "`$Host.UI.RawUI.WindowTitle = `"$Title`""; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 300
}

function Run-App([string]$Name, [hashtable]$Settings = @{}, [scriptblock]$Script, [switch]$Pattern, [switch]$NoShot) {
    $params = @{ Name = $Name; OutDir = $global:NkShots; Settings = $Settings; Script = $Script; SettleMs = 3500 }
    if ($Pattern) { $params['Pattern'] = $true }
    if ($NoShot) { $params['NoShot'] = $true }
    & $global:NkRun @params
}

function Scenario-Glass {
    Run-App 'glass-dark' @{ theme = 'Tokyo Night' } {
        Fill-Terminal; Name-Tab 'build'
        New-Tab; Name-Tab 'logs'
        New-Tab; Name-Tab 'claude'
        Chord @(0x11) 0x09; Start-Sleep -Milliseconds 700                  # Ctrl+Tab wraps to the first tab
        $r = Get-ElementRect 'logs'; Move-To ($r.X + 40) ($r.Y + 16); Start-Sleep -Milliseconds 600   # hover state of a chip
    }
    Run-App 'glass-light' @{ theme = 'One Half Light' } { Fill-Terminal; Name-Tab 'build' }
}

function Scenario-BlurProof {
    Run-App 'blur-proof' @{} { Fill-Terminal } -Pattern
    foreach ($b in 'mica', 'mica-alt', 'none') {
        Run-App "backdrop-$b" @{ appearance = @{ backdrop = $b } } { Fill-Terminal } -Pattern
    }
    foreach ($o in 1.0, 0.5) {
        Run-App "opacity-$o" @{ appearance = @{ opacity = $o } } { Fill-Terminal } -Pattern
    }
    Run-App 'blur-light' @{ theme = 'One Half Light' } { Fill-Terminal } -Pattern
}

function Scenario-TabsSplits {
    # Behaviour run: splits, focus movement, divider drag, zoom, tab reorder and middle-click close.
    Run-App 'tabs-behaviour' @{} {
        First-Prompt; Name-Tab 'build'
        New-Tab; Name-Tab 'logs'
        New-Tab; Name-Tab 'claude'
        Split-Right; Type-Line 'echo right pane'; Wait-Prompt | Out-Null
        Split-Down; Type-Line 'echo bottom right'; Wait-Prompt | Out-Null
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 600                  # Alt+Left: focus the left pane
        Shot-Safe (Shot-Path 'tabs-focus-left')
        $r = Win-Rect; $x = [int](($r.Left + $r.Right) / 2); $y = $r.Top + 400
        Drag-To $x $y ($x + 200) $y 20 20; Start-Sleep -Milliseconds 600     # drag the vertical divider
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 800; Shot-Safe (Shot-Path 'pane-zoom')
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 800            # Ctrl+Shift+Z toggles zoom
        $a = Get-ElementRect 'build'; $c = Get-ElementRect 'claude'
        Drag-To ($a.X + 30) ($a.Y + 16) ($c.X + $c.W - 10) ($c.Y + 16) 25 25; Start-Sleep -Milliseconds 700   # reorder
        Shot-Safe (Shot-Path 'tab-reorder')
        $l = Get-ElementRect 'logs'; Click-At ($l.X + 40) ($l.Y + 16) 'middle' 1200                            # middle click closes
        Shot-Safe (Shot-Path 'tab-middle-click-confirm')                                                       # a running shell asks first
        Click-Name '關閉' -Class 'Button'; Start-Sleep -Milliseconds 1200; Shot-Safe (Shot-Path 'tab-middle-closed')
    } -NoShot
    # The picture that stands for tabs and splits.
    Run-App 'tabs-splits' @{} {
        First-Prompt; Name-Tab 'build'
        New-Tab; Name-Tab 'logs'
        New-Tab; Name-Tab 'claude'
        Split-Right; Type-Line 'echo right pane'; Wait-Prompt | Out-Null
        Split-Down; Type-Line 'echo bottom right'; Wait-Prompt | Out-Null
        $a = Get-ElementRect 'build'; Move-To ($a.X + 40) ($a.Y + 16); Start-Sleep -Milliseconds 600
    }
}

function Scenario-Overlays {
    Run-App 'overlays' @{} {
        Fill-Terminal
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 900; Shot-Safe (Shot-Path 'overlay-palette')
        Type-Unicode '分割' 60; Start-Sleep -Milliseconds 500; Shot-Safe (Shot-Path 'overlay-palette-filter'); Press 0x1B; Start-Sleep -Milliseconds 500
        Chord @(0x11, 0x10) 0x46; Start-Sleep -Milliseconds 700; Type-Unicode 'red' 60; Start-Sleep -Milliseconds 600; Shot-Safe (Shot-Path 'overlay-find'); Press 0x1B; Start-Sleep -Milliseconds 500
        Click-Name '外觀' -Class 'Button'; Start-Sleep -Milliseconds 900; Shot-Safe (Shot-Path 'overlay-appearance-flyout'); Press 0x1B; Start-Sleep -Milliseconds 500
        $t = Get-ElementRect 'pwsh'; Click-At ($t.X + 40) ($t.Y + 16) 'right' 900; Shot-Safe (Shot-Path 'overlay-tab-menu'); Press 0x1B; Start-Sleep -Milliseconds 500
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200; Shot-Safe (Shot-Path 'overlay-close-dialog'); Press 0x1B; Start-Sleep -Milliseconds 500
        Chord @(0x11, 0x10) 0x4F; Start-Sleep -Milliseconds 1200
        Click-Name '外觀' -Class 'NavigationViewItem'; Start-Sleep -Milliseconds 900; Shot-Safe (Shot-Path 'settings-appearance')
        Click-Name '快速鍵' -Class 'NavigationViewItem'; Start-Sleep -Milliseconds 900; Shot-Safe (Shot-Path 'settings-keys')
    } -NoShot
}

function Scenario-Inactive {
    Run-App 'inactive' @{} {
        Fill-Terminal
        Shot-WithPattern (Shot-Path 'inactive-1-active')
        # Our own pattern window becomes the active one, outside the app rectangle: the app stays fully visible.
        [void][P]::SetWindowPos($script:PatternHwnd, [IntPtr]::Zero, 40, 1300, 300, 200, 0x0040)
        [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 1500
        $r = Win-Rect; $ap = [uint32]$script:App.Id
        $visible = ([D]::PidAt([int](($r.Left + $r.Right) / 2), [int](($r.Top + $r.Bottom) / 2)) -eq $ap) -and ([D]::PidAt($r.Left + 20, $r.Top + 20) -eq $ap) -and ([D]::PidAt($r.Right - 20, $r.Bottom - 20) -eq $ap)
        if ($visible -and [D]::ForegroundPid() -eq [uint32]$script:PatternProc.Id) { Shot (Shot-Path 'inactive-2-inactive') } else { 'inactive shot skipped (another window interfered)' }
    } -Pattern -NoShot
}

# Microsoft Bopomofo through real key presses: s u 3 is ㄋㄧˇ -> 你, c l 3 is ㄏㄠˇ -> 好. Commits must land in the pane that
# owned the composition, also after the focus moved to another pane.
function Scenario-Ime {
    Run-App 'ime' @{} {
        First-Prompt
        Press 0x53; Press 0x55; Press 0x33; Start-Sleep -Milliseconds 600                    # s u 3: preedit in pane 1
        Shot-Safe (Shot-Path 'ime-preedit')
        Press 0x0D; Start-Sleep -Milliseconds 800                                            # Enter commits 你
        "pane 1 after commit: " + ((Get-TerminalText) -split "`n" | Select-Object -Last 1)
        Split-Right
        Press 0x43; Press 0x4C; Press 0x33; Start-Sleep -Milliseconds 600                    # c l 3 in the new pane
        Press 0x0D; Start-Sleep -Milliseconds 800                                            # commits 好
        "pane 2 after commit: " + ((Get-TerminalText) -split "`n" | Select-Object -Last 1)
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 700                                    # Alt+Left: back to pane 1
        "pane 1 again:        " + ((Get-TerminalText) -split "`n" | Select-Object -Last 1)
        Shot-Safe (Shot-Path 'ime-two-panes')
    } -NoShot
}

# Numbers: start-up, idle CPU and memory, a 30,000-line flood, six maximize/restore cycles while that output is on
# screen, and whether any child process outlives the window. Prints one line per measurement.
function Scenario-Perf {
    $global:NkKids = @()
    Run-App 'perf' @{} {
        First-Prompt
        $p = $script:App
        $first = Select-String -Path $script:LogFile -Pattern 'pane 1 first frame' | Select-Object -First 1
        "startup: window ready, first pane frame logged at " + ($first.Line.Trim() -replace '\s+pane 1 first frame', '')
        Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
        $p.Refresh(); $cpu0 = $p.TotalProcessorTime.TotalMilliseconds; Start-Sleep -Seconds 10; $p.Refresh()
        'idle 10 s: cpu {0:N2} % of one core, working set {1:N0} MB, private {2:N0} MB, {3} threads, {4} handles' -f ((($p.TotalProcessorTime.TotalMilliseconds - $cpu0) / 10000) * 100), ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), $p.Threads.Count, $p.HandleCount
        Type-Line '1..30000 | ForEach-Object { "line $_ the quick brown fox 你好 世界" }'
        $watch = [Diagnostics.Stopwatch]::StartNew(); Wait-Prompt -TimeoutMs 120000 | Out-Null
        $p.Refresh()
        'flood 30,000 lines (about 1.5 MB): prompt back after {0:N1} s, working set {1:N0} MB, private {2:N0} MB, peak working set {3:N0} MB' -f $watch.Elapsed.TotalSeconds, ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), ($p.PeakWorkingSet64 / 1MB)
        $before = (Select-String -Path $script:LogFile -Pattern 'configured ').Count
        for ($i = 0; $i -lt 6; $i++) { [void][D]::ShowWindow($script:Hwnd, 3); Start-Sleep -Milliseconds 900; [void][D]::ShowWindow($script:Hwnd, 9); Start-Sleep -Milliseconds 900 }
        $after = (Select-String -Path $script:LogFile -Pattern 'configured ').Count
        "resize stress: 6 maximize/restore cycles, $($after - $before) swap chain reconfigurations, app still running: $(-not $p.HasExited)"
        [void][D]::ShowWindow($script:Hwnd, 3); Start-Sleep -Milliseconds 1500; Shot-Safe (Shot-Path 'maximized')
        $global:NkKids = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $p.Id } | ForEach-Object { $_.ProcessId })
        $global:NkKids += @(Get-CimInstance Win32_Process | Where-Object { $global:NkKids -contains $_.ParentProcessId } | ForEach-Object { $_.ProcessId })
        "children before close: " + ($global:NkKids -join ', ')
    } -NoShot
    Start-Sleep -Seconds 3
    $alive = @($global:NkKids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
    "children still alive 3 s after close: $($alive.Count)"
    Select-String -Path (Join-Path ([IO.Path]::GetTempPath()) 'nekominal-drive\perf.log') -Pattern 'PANIC|FATAL' | ForEach-Object { 'LOG: ' + $_.Line }
}

# Silence: the witness (sound-witness.ps1) watches the default render endpoint while each trigger of the matrix runs;
# a row passes when the peak stays below $global:WitnessThreshold. Then Alt+Space must still open the system menu and
# holding Alt+Shift+D for two seconds must add exactly one pane. Audible only when a row fails.
function Scenario-Silence {
    . (Join-Path $PSScriptRoot 'sound-witness.ps1')
    Run-App 'silence' @{} {
        First-Prompt
        Start-SoundWitness
        $rows = [ordered]@{
            'toolbar split right button' = { Click-Name '向右分割窗格 (Alt+Shift+D)' -Class 'Button' }
            'palette: split down'        = { Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 500; Type-Unicode '向下分割' 40; Press 0x0D }
            'Alt+Shift+D'                = { Chord @(0x12, 0x10) 0x44 }
            'Alt+Shift+S'                = { Chord @(0x12, 0x10) 0x53 }
            'Alt+Shift+='                = { Chord @(0x12, 0x10) 0xBB }
            'Alt+Shift+-'                = { Chord @(0x12, 0x10) 0xBD }
            'Alt+Left'                   = { Chord @(0x12) 0x25 }
            'Alt+Shift+Left'             = { Chord @(0x12, 0x10) 0x25 }
            'Alt tap, then x'            = { Press 0x12; Press 0x58; Press 0x1B }
            'Alt+x'                      = { Chord @(0x12) 0x58 }
            'Ctrl+Shift+Z'               = { Chord @(0x11, 0x10) 0x5A; Chord @(0x11, 0x10) 0x5A }
            'Ctrl+Shift+T'               = { Chord @(0x11, 0x10) 0x54 }
        }
        foreach ($name in $rows.Keys) {
            $peak = Measure-Peak $rows[$name]
            $verdict = if ($null -eq $peak) { 'INCONCLUSIVE' } elseif ($peak -lt $global:WitnessThreshold) { 'quiet' } else { 'SOUND' }
            '{0,-28} {1,-12} peak {2:E1}' -f $name, $verdict, $peak
        }
        $menuPeak = Measure-Peak { Chord @(0x12) 0x20; Start-Sleep -Milliseconds 500; $script:menu = [Menus]::MenuIsOpen(); Press 0x1B }
        'Alt+Space: system menu open = {0}, peak {1:E1}' -f $script:menu, $menuPeak
        Chord @(0x11, 0x10) 0x54; Wait-Prompt | Out-Null
        $before = Count-Panes
        Hold-Chord @(0x12, 0x10) 0x44 2000
        Start-Sleep -Milliseconds 800
        'hold Alt+Shift+D for 2 s: panes {0} -> {1}' -f $before, (Count-Panes)
        $menuchar = @(Select-String -Path $script:LogFile -Pattern 'WM_MENUCHAR').Count
        'WM_MENUCHAR lines in the log: {0}' -f $menuchar
    } -NoShot
}

function Log-Mark { @(Get-Content -LiteralPath $script:LogFile).Count }
function Log-Since([int]$Mark) { $all = @(Get-Content -LiteralPath $script:LogFile); if ($all.Count -gt $Mark) { $all[$Mark..($all.Count - 1)] } else { @() } }
function Log-Ms([string]$Line) { [double]$Line.Substring(0, 9).Trim() }
function Count-Class([string]$Class) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, $Class)
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond).Count
}
function Get-Slider([int]$Index = 0) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Slider)
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)[$Index]
}
function Select-Radio([string]$Name) { (Get-Element $Name 'RadioButton').GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select() }

# Background opacity: a live change on an idle terminal changes its background (pixels over the striped window), a
# replay of 200 slider events makes no SetSystemBackdrop call, at most one apply per frame and one save and shows no
# InfoBar or dialog, backdrop None turns the slider off without touching the saved value, and 20 backdrop
# switches log no error. (A change made through UI Automation activates the window first, so a change while the
# window is inactive cannot be driven from here.)
function Scenario-Opacity {
    Run-App 'opacity' @{ appearance = @{ opacity = 0.55 } } {
        First-Prompt
        Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
        $r0 = Win-Rect
        Click-Name '外觀' -Class 'Button'; Start-Sleep -Milliseconds 1200
        # 1. idle terminal: no key, no output between the two pictures
        $shot = { param($name) Shot-WithPattern (Shot-Path $name) | Out-Null; Region-Average (Shot-Path $name) 190 600 300 150 }
        $a = & $shot 'opacity-live-55'
        Set-Slider 25 0; Start-Sleep -Milliseconds 1500
        $b = & $shot 'opacity-live-25'
        Set-Slider 100 0; Start-Sleep -Milliseconds 1500
        $c = & $shot 'opacity-live-100'
        'idle terminal, mean RGB of its background: 55% {0} | 25% {1} | 100% {2}' -f $a, $b, $c
        Set-Slider 55 0; Start-Sleep -Milliseconds 1500
        # 2. replay of 200 slider events
        $mark = Log-Mark
        $range = (Get-Slider 0).GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
        $watch = [Diagnostics.Stopwatch]::StartNew()
        for ($i = 0; $i -lt 200; $i++) { $range.SetValue(20 + [Math]::Abs((($i * 4) % 160) - 80)) }
        $replayMs = $watch.ElapsedMilliseconds
        Start-Sleep -Milliseconds 1500
        $lines = @(Log-Since $mark)
        $applies = @($lines | Where-Object { $_ -match 'opacity applied' })
        $gaps = for ($i = 1; $i -lt $applies.Count; $i++) { (Log-Ms $applies[$i]) - (Log-Ms $applies[$i - 1]) }
        'replay: 200 value changes in {0} ms -> {1} applies (shortest gap {2:N1} ms), {3} SetSystemBackdrop, {4} saves, {5} apply errors, InfoBar {6}, dialog {7}' -f $replayMs, $applies.Count, ($gaps | Measure-Object -Minimum).Minimum, @($lines | Where-Object { $_ -match 'system backdrop set' }).Count, @($lines | Where-Object { $_ -match 'settings saved' }).Count, @($lines | Where-Object { $_ -match 'apply_glass:|apply_opacity:' }).Count, (Count-Class 'InfoBar'), (Count-Class 'ContentDialog')
        # 3. twenty backdrop switches
        $mark = Log-Mark
        $names = '壓克力', 'Mica', 'Mica Alt', '無'
        foreach ($i in 0..19) { Select-Radio $names[$i % 4]; Start-Sleep -Milliseconds 150 }
        Start-Sleep -Milliseconds 800
        $lines = @(Log-Since $mark)
        '20 backdrop switches: {0} SetSystemBackdrop, {1} full applies, {2} errors' -f @($lines | Where-Object { $_ -match 'system backdrop set' }).Count, @($lines | Where-Object { $_ -match 'glass: scheme' }).Count, @($lines | Where-Object { $_ -match 'apply_glass:|apply_opacity:|rror' }).Count
        # 4. backdrop None: the slider is off and reads 100%, the saved value (the last one of the replay) stays
        Select-Radio '無'; Start-Sleep -Milliseconds 1500
        $s = Get-Slider 0
        $file = Join-Path (Join-Path (Join-Path (Split-Path $script:LogFile) 'opacity-appdata') 'Nekominal') 'settings.json'
        'backdrop None: slider enabled={0} value={1}, saved settings: {2}' -f $s.Current.IsEnabled, $s.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value, ((Get-Content $file -Raw) -replace '\s+', ' ')
        Select-Radio '壓克力'; Start-Sleep -Milliseconds 800
        'back to Acrylic: slider enabled={0} value={1}' -f (Get-Slider 0).Current.IsEnabled, (Get-Slider 0).GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value
        Press 0x1B; Start-Sleep -Milliseconds 500
    } -Pattern -NoShot
}

function Scenario-All { Scenario-Glass; Scenario-BlurProof; Scenario-TabsSplits; Scenario-Overlays; Scenario-Inactive; Scenario-Ime; Scenario-Perf; Scenario-Silence; Scenario-Opacity }
