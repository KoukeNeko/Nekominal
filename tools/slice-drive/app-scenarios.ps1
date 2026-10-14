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
#   Scenario-Tabs             # position plate, chip widths, edge counts, reveal, list of every tab, MRU Ctrl+Tab, closing back
#   Scenario-Titles           # default tab titles: folders, collisions, program titles, bidi cleaning
#   Scenario-Status           # tab status marks for OSC 7501 / 9 / 777 and BEL, bell decisions with the window active and not
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

function Plate-Text {
    $plate = Get-Element '所有分頁' 'Button'
    if (-not $plate) { return $null }
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    ($plate.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name }) -join ''
}

# The chips of the strip as UI Automation shows them (one per tab: a named container with a position in its set).
function Get-Chips {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $pos = [System.Windows.Automation.AutomationProperty]::LookupById(30152)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NamedContainerAutomationPeer')
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        $p = $e.GetCurrentPropertyValue($pos)
        if ($p -is [int]) { [pscustomobject]@{ Index = $p; Name = $e.Current.Name; Status = $e.Current.ItemStatus; Rect = $e.Current.BoundingRectangle; Offscreen = $e.Current.IsOffscreen } }
    }
}

# The edge counts ("‹ 3", "5 ›") of the strip.
function Get-EdgeCounts {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $left = 0; $right = 0
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        $name = $e.Current.Name
        if ($name -match '^‹ (\d+)$' -and -not $e.Current.IsOffscreen) { $left = [int]$Matches[1] }
        elseif ($name -match '^(\d+) ›$' -and -not $e.Current.IsOffscreen) { $right = [int]$Matches[1] }
    }
    @($left, $right)
}

# Tab awareness: the position plate, the width algorithm, the edge counts, the selected chip in view, the list of every
# tab, most-recently-used Ctrl+Tab and closing back to the tab used before. One run grows the tab count to 30 and
# measures every window width at 1, 2, 5, 12 and 30 tabs.
function Scenario-Tabs {
    Run-App 'tabs' @{ confirmClose = $false } {
        First-Prompt
        Name-Tab 'aaa'
        # the single tab has no plate
        foreach ($w in 800, 988, 1920) { Set-WindowWidth $w; 'N= 1 W={0,4}: plate present = {1}, chips {2}' -f $w, ($null -ne (Get-Element '所有分頁' 'Button')), @(Get-Chips).Count }
        Set-WindowWidth 988
        New-Tab; Name-Tab 'bbb'; New-Tab; Name-Tab 'ccc'
        # most recently used: select aaa, bbb, ccc in turn, then Ctrl+Tab once and with Ctrl held for two presses
        foreach ($name in 'aaa', 'bbb', 'ccc') { Click-Name $name; Start-Sleep -Milliseconds 300 }
        Chord @(0x11) 0x09; Start-Sleep -Milliseconds 500
        $tap = Plate-Text
        [D]::Key(0x11, $false); Start-Sleep -Milliseconds 100
        Press 0x09; Press 0x09; Start-Sleep -Milliseconds 200
        [D]::Key(0x11, $true); Start-Sleep -Milliseconds 500
        'MRU after aaa, bbb, ccc: Ctrl+Tab tap -> {0} (want 2 / 3, bbb); Ctrl held, Tab twice -> {1} (want 1 / 3, aaa)' -f $tap, (Plate-Text)
        # closing the selected tab goes back to the one used before it, not to a neighbour
        Click-Name 'ccc'; Start-Sleep -Milliseconds 300; Click-Name 'aaa'; Start-Sleep -Milliseconds 300; Click-Name 'bbb'; Start-Sleep -Milliseconds 300
        $b = Get-ElementRect 'bbb'; Click-At ($b.X + 40) ($b.Y + 16) 'middle' 1000
        'closed bbb (selected, after aaa): plate {0} (want 1 / 2, aaa)' -f (Plate-Text)
        Name-Tab 'claude one'
        $measure = {
            param($n, $w)
            $chips = @(Get-Chips); $plate = Plate-Text; $edges = Get-EdgeCounts
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
            $scroll = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ScrollViewer')))
            $v = $scroll.Current.BoundingRectangle
            # a chip is visible when any part of it lies inside the strip's viewport; the UIA rectangle of a chip that is
            # scrolled out is empty (infinite)
            $shown = @($chips | Where-Object { -not [double]::IsInfinity($_.Rect.Width) -and $_.Rect.X + $_.Rect.Width -gt $v.X + 0.5 -and $_.Rect.X -lt $v.X + $v.Width - 0.5 })
            $sel = $chips | Where-Object { ($plate -match '^(\d+) / ' -and $_.Index -eq [int]$Matches[1]) -or ($plate -eq "$n" -and $_.Index -eq $n) -or ($null -eq $plate -and $_.Index -eq 1) } | Select-Object -First 1
            $margin = if ($sel -and -not [double]::IsInfinity($sel.Rect.Width)) { '{0:N0}/{1:N0}' -f (($sel.Rect.X - $v.X) / 1.5), (($v.X + $v.Width - $sel.Rect.X - $sel.Rect.Width) / 1.5) } else { '?' }
            $widthsDip = ($chips | Sort-Object Index | ForEach-Object { if ([double]::IsInfinity($_.Rect.Width)) { '-' } else { [int]($_.Rect.Width / 1.5) } }) -join ','
            'N={0,2} W={1,4}: plate "{2}" | chips {3}, visible {4} + hidden {5}+{6} = {7} | selected margins {8} DIP | widths {9}' -f $n, $w, $plate, $chips.Count, $shown.Count, $edges[0], $edges[1], ($shown.Count + $edges[0] + $edges[1]), $margin, $widthsDip
        }
        $targets = 2, 5, 12, 30
        $widths = 800, 988, 1200, 1920
        $count = 2
        foreach ($n in $targets) {
            while ($count -lt $n) { New-Tab; $count++; if ($count -in 8, 20) { Name-Tab "claude $count" } }
            foreach ($w in $widths) {
                Set-WindowWidth $w
                & $measure $n $w
            }
        }
        # the selected chip scrolls into view: first tab, then the last, then a middle one
        Set-WindowWidth 988
        foreach ($target in 1, 30, 14) {
            $mark = Log-Mark
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $plateButton = Get-Element '所有分頁' 'Button'
            $pr = $plateButton.Current.BoundingRectangle
            Click-At ([int]($pr.X + $pr.Width / 2)) ([int]($pr.Y + $pr.Height / 2)) 'left' 900
            # Up and Down move the selection of the list (clamped at the ends): row 1, the last row, or row 14.
            $ups = if ($target -eq 30) { 0 } else { 40 }
            $downs = switch ($target) { 1 { 0 } 30 { 40 } default { $target - 1 } }
            foreach ($i in 1..$ups) { if ($ups) { Press 0x26 @() 20 20 } }
            foreach ($i in 1..$downs) { if ($downs) { Press 0x28 @() 20 20 } }
            Start-Sleep -Milliseconds 300
            Press 0x0D; Start-Sleep -Milliseconds 700
            $log = (Log-Since $mark | Where-Object { $_ -match 'selected tab|strip scrolled|tab list' } | ForEach-Object { $_.Trim() -replace '\s+ms\s+', 'ms ' }) -join ' | '
            'list -> tab {0}: {1} | {2}' -f $target, (& $measure 30 988), $log
        }
        # filtering the list of 30 tabs
        $mark = Log-Mark
        $plateButton = Get-Element '所有分頁' 'Button'; $pr = $plateButton.Current.BoundingRectangle
        Click-At ([int]($pr.X + $pr.Width / 2)) ([int]($pr.Y + $pr.Height / 2)) 'left' 700
        $all = @(Get-Items).Count
        Type-Unicode 'cla' 60; Start-Sleep -Milliseconds 700
        $rows = @(Get-Items)
        'list of 30: {0} rows; after typing cla: {1} rows ({2}); {3}' -f $all, $rows.Count, (($rows | ForEach-Object { $_.Current.Name }) -join ' / '), ((Log-Since $mark | Where-Object { $_ -match 'tab list' }) -join ' | ')
        Press 0x1B; Start-Sleep -Milliseconds 400
        # the palette gets one go-to entry per tab
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 700
        Type-Unicode '切換到分頁：claude' 40; Start-Sleep -Milliseconds 600
        'palette rows for "切換到分頁：claude": {0}' -f (@(Get-Items).Count)
        Press 0x1B; Start-Sleep -Milliseconds 400
        Shot-Safe (Shot-Path 'tabs-30') | Out-Null
    } -NoShot
}

# Status marks: what a tab shows for OSC 7501, OSC 9 / 777 and BEL, read from the chips' accessible status (the
# state in words, so it is not colour alone), then the bell in three places: the pane in view (nothing), a hidden tab
# (a mark), an inactive window (a mark and a taskbar flash, at most one a second). Nothing here is audible.
function Scenario-Status {
    Run-App 'status' @{ confirmClose = $false } {
        First-Prompt
        Name-Tab 'aaa'
        Set-WindowWidth 1400
        $esc = '$e=[char]27;$a=[char]7;'
        $report = { param($fields) $esc + 'Write-Host -NoNewline "$e]7501;' + $fields + '$a";' }
        $start = Get-Date
        New-Tab; Name-Tab 'blocked-tab'; Type-Line ((& $report 'state=blocked:kind=permission:id=t:app=claude:msg=QWxsb3cgZWRpdD8=') + 'Start-Sleep 90'); Start-Sleep -Milliseconds 400
        New-Tab; Name-Tab 'working-tab'; Type-Line ((& $report 'state=working:progress=40:id=t:app=claude') + 'Start-Sleep 90'); Start-Sleep -Milliseconds 400
        New-Tab; Name-Tab 'done-tab'; Type-Line ($esc + 'Start-Sleep 5;' + (& $report 'state=done:id=t:app=claude')); Start-Sleep -Milliseconds 400
        New-Tab; Name-Tab 'error-tab'; Type-Line ($esc + 'Start-Sleep 5;' + (& $report 'state=error:id=t:app=claude:msg=Qm9vbQ==')); Start-Sleep -Milliseconds 400
        New-Tab; Name-Tab 'bell-tab'; Type-Line ($esc + 'Start-Sleep 4; Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 300
        New-Tab; Name-Tab 'note-tab'; Type-Line ($esc + 'Start-Sleep 4; Write-Host -NoNewline "$e]777;notify;Claude;needs$([char]0x202E) input$a"'); Start-Sleep -Milliseconds 300
        New-Tab; Name-Tab 'last'
        Start-Sleep -Milliseconds 6000
        $show = { param($label) foreach ($c in Get-Chips | Sort-Object Index) { '{0} {1,2} {2,-14} status "{3}"' -f $label, $c.Index, $c.Name.Split('，')[0], $c.Status } }
        & $show 'hidden tabs:'
        Shot-Safe (Shot-Path 'status-marks') | Out-Null
        # a blocked agent is still blocked when the tab is looked at; a finished one is seen at once
        Click-Name 'blocked-tab，等待輸入'; Start-Sleep -Milliseconds 500
        Click-Name 'done-tab，已完成'; Start-Sleep -Milliseconds 500
        Click-Name 'error-tab，錯誤'; Start-Sleep -Milliseconds 500
        Click-Name 'note-tab，通知'; Start-Sleep -Milliseconds 500
        & $show 'after looking at blocked, done, error, note:'
        # working ends at the next prompt: the report is gone once the command returns
        Click-Name 'last' | Out-Null; Start-Sleep -Milliseconds 500
        Type-Line ($esc + (& $report 'state=working:id=x') + 'Start-Sleep 4'); Start-Sleep -Milliseconds 1500
        $during = (Get-Chips | Where-Object { $_.Index -eq 8 }).Status
        Start-Sleep -Milliseconds 5000
        'working in the pane in view: while the command runs "{0}", after the next prompt "{1}"' -f $during, (Get-Chips | Where-Object { $_.Index -eq 8 }).Status
        # the bell in the pane in view, then in an inactive window twice within a second
        $mark = Log-Mark
        Click-Name 'last' | Out-Null; Start-Sleep -Milliseconds 500
        Type-Line ($esc + 'Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 1200
        Type-Line ($esc + 'Start-Sleep 3; Write-Host -NoNewline $a; Start-Sleep -Milliseconds 300; Write-Host -NoNewline $a; Start-Sleep 3; Write-Host -NoNewline $a')
        [void][P]::SetWindowPos($script:PatternHwnd, [IntPtr]::Zero, 40, 1300, 300, 200, 0x0040)
        [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 9000
        'bell decisions (watching, then inactive: first, second within a second, a later one):'
        Log-Since $mark | Where-Object { $_ -match 'bell in pane|taskbar flash|window active' } | ForEach-Object { 'log: ' + $_.Trim() }
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 600
        'ended in {0:N0} s' -f ((Get-Date) - $start).TotalSeconds
    } -Pattern -NoShot
}

# Default tab titles: five tabs in different folders get five names, two folders called src get their parents, a title
# a program sets wins until it is generic again and a bidirectional override is cleaned out.
function Scenario-Titles {
    $global:NkCjk = Join-Path ([IO.Path]::GetTempPath()) 'nk-title-測試資料夾'
    [void][IO.Directory]::CreateDirectory($global:NkCjk)
    try {
        Run-App 'titles' @{ confirmClose = $false } {
            First-Prompt
            $names = { (Get-Chips | Sort-Object Index | ForEach-Object { $_.Name }) -join ' | ' }
            $go = { param($dir) New-Tab; Type-Line "Set-Location '$dir'"; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 400 }
            'first tab, nothing typed yet: ' + (& $names)
            foreach ($dir in 'C:\Windows', 'C:\Windows\System32', 'C:\', 'C:\Users\Public') { & $go $dir }
            'five folders: ' + (& $names)
            & $go (Join-Path $env:USERPROFILE 'Desktop\Nekominal\crates\nekominal-app\src')
            & $go (Join-Path $env:USERPROFILE 'Desktop\Nekominal\crates\nekominal-model\src')
            'two src folders: ' + (& $names)
            & $go $global:NkCjk
            'a folder with a CJK name: ' + (& $names)
            # a title the program set wins over the folder, a generic one does not
            Type-Line '$Host.UI.RawUI.WindowTitle = "build"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title "build": ' + (& $names)
            Type-Line '$Host.UI.RawUI.WindowTitle = "pwsh.exe"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title "pwsh.exe" (generic): ' + (& $names)
            Type-Line '$Host.UI.RawUI.WindowTitle = "pay$([char]0x202E)load"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title with U+202E: ' + (& $names)
            # a name the user gives is kept, an empty name hands the title back to the rules
            $rename = { param($chipName, $text)
                $r = Get-ElementRect $chipName; Click-At ($r.X + 30) ($r.Y + 16) 'right' 800
                Click-Name '重新命名分頁' | Out-Null; Start-Sleep -Milliseconds 800
                if ($text) { Type-Unicode $text 40 } else { Press 0x08 }
                Press 0x0D; Start-Sleep -Milliseconds 800 }
            & $rename 'Windows' 'fixed'
            'renamed the tab of C:\Windows "fixed": ' + (& $names)
            Click-Name 'fixed' | Out-Null; Start-Sleep -Milliseconds 400
            Type-Line 'Set-Location C:\Users'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'after cd C:\Users in it: ' + (& $names)
            & $rename 'fixed' ''
            'name cleared: ' + (& $names)
        }
    } finally {
        [IO.Directory]::Delete($global:NkCjk, $true)
    }
}

function Get-Items {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ListViewItem')
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
}

function Scenario-All { Scenario-Glass; Scenario-BlurProof; Scenario-TabsSplits; Scenario-Overlays; Scenario-Inactive; Scenario-Ime; Scenario-Perf; Scenario-Silence; Scenario-Opacity; Scenario-Tabs; Scenario-Status; Scenario-Titles }
