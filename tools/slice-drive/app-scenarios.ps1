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
#   Scenario-Panes            # pane count and zoom marks on a chip, the corner label of a zoomed pane, warning line of an unfocused pane
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

# Every run is started with NEKOMINAL_NO_LAUNCH: a link or the settings file is logged where it would be opened, and no
# browser or editor starts.
function Run-App([string]$Name, [hashtable]$Settings = @{}, [scriptblock]$Script, [switch]$Pattern, [switch]$NoShot) {
    $params = @{ Name = $Name; OutDir = $global:NkShots; Settings = $Settings; Script = $Script; SettleMs = 3500; Env = @{ NEKOMINAL_NO_LAUNCH = '1' } }
    if ($Pattern) { $params['Pattern'] = $true }
    if ($NoShot) { $params['NoShot'] = $true }
    if ($global:NkExe) { $params['Exe'] = $global:NkExe }
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

# Closing: a tab at its prompt closes without asking, a running command asks and the dialog names the tab, the
# middle button closes when it comes up (not when it goes down), a shell that exits cleanly closes its pane and a
# program profile that exits cleanly keeps its last screen.
function Scenario-Close {
    $agent = @{ id = 'agent'; name = 'agent'; program = 'cmd.exe'; args = @('/c', 'echo final-screen-text') }
    Run-App 'close' @{ defaultProfile = 'agent'; profiles = @($agent) } {
        Start-Sleep -Milliseconds 3000
        'program profile exited with 0: window still open = {0}, tabs {1}, text has output = {2}, text has status = {3}' -f (-not $script:App.HasExited), @(Get-Chips).Count, ((Get-TerminalText) -match 'final-screen-text'), ((Get-TerminalText) -match '已結束，代碼 0')
    } -NoShot
    Run-App 'close-shell' @{} {
        First-Prompt
        New-Tab
        Type-Line 'exit'; Start-Sleep -Milliseconds 1500
        'typed exit in a shell tab: tabs {0} (want 1)' -f @(Get-Chips).Count
        New-Tab
        $dialogs = { Count-Class 'ContentDialog' }
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        'closed a tab at its prompt: dialog {0}, tabs {1} (want 0 and 1)' -f (& $dialogs), @(Get-Chips).Count
        New-Tab
        Type-Line '$Host.UI.RawUI.WindowTitle = "deploy"; Start-Sleep 60'; Start-Sleep -Milliseconds 1200
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        'closed a tab with a running command: dialog {0}, title found = {1} (want 1 and True), tabs {2}' -f (& $dialogs), [bool](Get-Element '關閉「deploy」？'), @(Get-Chips).Count
        Press 0x1B; Start-Sleep -Milliseconds 600
        'cancelled: tabs {0} (want 2)' -f @(Get-Chips).Count
        # the middle button: down does nothing, up over the chip closes
        New-Tab
        $chips = @(Get-Chips); $r = Get-ElementRect $chips[-1].Name
        Move-To ($r.X + 30) ($r.Y + 16); Start-Sleep -Milliseconds 300
        [D]::Mouse(0x0020, 0); Start-Sleep -Milliseconds 800
        $afterDown = @(Get-Chips).Count
        [D]::Mouse(0x0040, 0); Start-Sleep -Milliseconds 1200
        'middle button: tabs after down {0}, after up {1} (want 3 then 2)' -f $afterDown, @(Get-Chips).Count
    } -NoShot
}

# Closing the window ends every session, so it asks like closing a tab does: with a command running the window stays and the
# question counts the sessions, Cancel keeps everything, Close ends the window; an idle shell closes without a question.
function Scenario-CloseWindow {
    $failed = $false
    Run-App 'close-window-running' @{} {
        First-Prompt
        Type-Line 'Start-Sleep 120'; Start-Sleep -Milliseconds 1200
        $results = @()
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 1500
        $script:App.Refresh()
        $results += Check 'closing the window with a command running keeps it open' $false $script:App.HasExited
        $results += Check 'and asks about the running session' $true ([bool](Get-Element '關閉 1 個執行中的工作階段？'))
        Press 0x1B; Start-Sleep -Milliseconds 800
        $results += Check 'Cancel keeps the window and the command' $false $script:App.HasExited
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 1200
        Click-Name '關閉' -Class 'Button'
        $results += Check 'Close ends the window' $true ($script:App.WaitForExit(5000))
        $results
        if ($results -like 'FAIL*') { throw 'close-window: a check failed' }
    } -NoShot
    Run-App 'close-window-idle' @{} {
        First-Prompt
        [void]$script:App.CloseMainWindow()
        $result = Check 'an idle shell closes without a question' $true ($script:App.WaitForExit(5000))
        $result
        if ($result -like 'FAIL*') { throw 'close-window: a check failed' }
    } -NoShot
}

# Pane markers: a tab with one pane shows no count, three panes show the count (and "Zoomed" in a corner label while one
# is zoomed, a press on it brings the others back), and an unfocused pane that rang has a warning line over its top
# edge until it is focused (the focus line and the warning line are never on the same pane).
function Scenario-Panes {
    Run-App 'panes' @{} {
        First-Prompt
        $texts = {
            $chip = Get-Chips | Select-Object -First 1
            $e = Get-Element $chip.Name
            $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
            ($e.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name }) -join '|'
        }
        '1 pane: chip texts "{0}", panes {1}' -f (& $texts), (Count-Panes)
        Split-Right; Split-Down
        '3 panes: chip texts "{0}", panes {1}' -f (& $texts), (Count-Panes)
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 900
        $badge = Get-Element '已縮放'
        $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
        'zoomed: corner label present = {0}, text "{1}", chip texts "{2}"' -f [bool]$badge, (($badge.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name }) -join ''), (& $texts)
        Shot-Safe (Shot-Path 'panes-zoomed') | Out-Null
        $b = $badge.Current.BoundingRectangle
        Click-At ([int]($b.X + $b.Width / 2)) ([int]($b.Y + $b.Height / 2)) 'left' 900
        'pressed the label: corner label present = {0}, panes shown {1}' -f [bool](Get-Element '已縮放'), (Count-Panes)
        # a bell in the pane that is not focused
        $esc = '$e=[char]27;$a=[char]7;'
        Type-Line ($esc + 'Start-Sleep 3; Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 300
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 5000
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $termCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
        $rects = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $termCond) | ForEach-Object { $_.Current.BoundingRectangle })
        $ringing = $rects | Sort-Object { $_.Y * 10000 + $_.X } | Select-Object -Last 1
        $win = Win-Rect
        $sample = {
            param($rect, $path)
            Shot-Safe $path | Out-Null
            $bmp = [System.Drawing.Bitmap]::FromFile($path)
            try {
                $x = [int]($rect.X - $win.Left + 40); $y0 = [int]($rect.Y - $win.Top)
                (0..4 | ForEach-Object { $c = $bmp.GetPixel($x, $y0 + $_); '({0},{1},{2})' -f $c.R, $c.G, $c.B }) -join ' '
            } finally { $bmp.Dispose() }
        }
        'top rows of the pane that rang (unfocused): ' + (& $sample $ringing (Shot-Path 'panes-warning'))
        Click-At ([int]($ringing.X + $ringing.Width / 2)) ([int]($ringing.Y + $ringing.Height / 2)) 'left' 900
        'top rows of it once focused: ' + (& $sample $ringing (Shot-Path 'panes-focused'))
    } -NoShot
}

function Get-Items {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ListViewItem')
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
}

# Selection and copy: a drag, a double click, a triple click and a click on a wrapped line, a wide-character line, Alt+drag
# block, Shift+click extend, smart copy (Ctrl+C copies with a selection and interrupts without one), typing dismisses
# the selection, the right button (copy, else paste; the pasted text is not run) and the paste confirmation. Each
# check copies with Ctrl+Shift+C and reads the clipboard of this session; the clipboard of the user is put back at the
# end (text only).
function Scenario-Selection {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    try {
        Run-App 'selection' @{} {
            First-Prompt
            Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
            Type-Line '"alpha beta gamma delta"; "second line here"; ("0123456789" * 20); "你好世界😀 ok"'; Wait-Prompt | Out-Null
            Start-Sleep -Milliseconds 600
            $g = Get-Grid
            'grid {0}x{1}, cells {2}x{3} px, origin {4},{5}' -f $g.Cols, $g.Rows, $g.CW, $g.CH, $g.X, $g.Y
            $alpha = Find-Cell 'alpha beta gamma delta'
            $second = Find-Cell 'second line here'
            $wrapped = Find-Cell '0123456789012345'
            $wide = Find-Cell '你好世界'
            $beta = $alpha.Col + 6; $gamma = $alpha.Col + 11
            $below = $g.Rows - 3

            Drag-Cells $g ($beta + 0.25) $alpha.Row ($beta + 3.75) $alpha.Row
            Shot-Safe (Shot-Path 'selection-drag') | Out-Null
            Check 'drag over "beta" copies beta' 'beta' (Copy-Selection)
            Check 'copying dismissed the selection (a second copy finds none)' 'SENTINEL' (Copy-Selection)

            Click-Cell $g ($gamma + 1) $alpha.Row 2
            Check 'double click copies the word' 'gamma' (Copy-Selection)
            Click-Cell $g 3 $second.Row 3
            Check 'triple click copies the line without a line break' 'second line here' (Copy-Selection)
            Click-Cell $g 3 $wrapped.Row 3
            Check 'triple click on a wrapped row copies the whole line, joined' ('0123456789' * 20) (Copy-Selection)
            Drag-Cells $g ($wrapped.Col + 0.25) $wrapped.Row ($g.Cols - 0.25) ($wrapped.Row + 1)
            Check 'a drag across the wrap has no line break inside' $false ((Copy-Selection) -match "`n")
            Drag-Cells $g ($wide.Col + 0.25) $wide.Row ($wide.Col + 9.75) $wide.Row
            Check 'wide characters and the emoji are copied once' '你好世界😀' (Copy-Selection)

            With-Keys @(0x12) { Drag-Cells $g ($alpha.Col + 0.25) $alpha.Row ($alpha.Col + 2.75) $second.Row }
            Check 'Alt+drag selects a block' "alp`nsec" (Copy-Selection)

            Drag-Cells $g ($alpha.Col + 0.25) $alpha.Row ($alpha.Col + 4.75) $alpha.Row
            $p = Cell-Pt $g ($beta + 3.75) $alpha.Row
            With-Keys @(0x10) { Click-At $p.X $p.Y 'left' 400 }
            Check 'Shift+click extends the selection' 'alpha beta' (Copy-Selection)

            Click-Cell $g ($gamma + 1) $alpha.Row 2
            Set-Clipboard -Value 'SENTINEL'
            Chord @(0x11) 0x43; Start-Sleep -Milliseconds 500
            Check 'Ctrl+C with a selection copies' 'gamma' ((Get-Clipboard -Raw) -replace "`r`n", "`n")
            Check 'and sends no interrupt to the shell' $true ((Get-TerminalText) -notmatch 'KeyboardInterrupt|\^C')
            Type-Line 'Start-Sleep 25'; Start-Sleep -Milliseconds 1200
            Chord @(0x11) 0x43
            Check 'Ctrl+C without a selection interrupts the program' $true (Wait-Text 'PS [^\r\n]*>\s*$' 6000)

            Click-Cell $g ($gamma + 1) $alpha.Row 2
            Type-Unicode 'x' 30; Start-Sleep -Milliseconds 300
            Check 'typing dismisses the selection' 'SENTINEL' (Copy-Selection)
            Press 0x08

            # The right button: with a selection it copies and spends it, without one it pastes.
            Click-Cell $g ($gamma + 1) $alpha.Row 2
            Set-Clipboard -Value 'SENTINEL'
            Click-Cell $g 10 $below 1 'right'
            Check 'right click with a selection copies it' 'gamma' ((Get-Clipboard -Raw) -replace "`r`n", "`n")
            Check 'and spends it' 'SENTINEL' (Copy-Selection)
            Set-Clipboard -Value 'pasted-by-right-click'
            Click-Cell $g 10 $below 1 'right'; Start-Sleep -Milliseconds 600
            Check 'right click without a selection pastes' $true ((Get-TerminalText) -match 'PS [^\r\n]*> pasted-by-right-click\s*$')
            Press 0x1B

            # A big paste asks first, and Cancel pastes nothing.
            Set-Clipboard -Value ('x' * 6000)
            Chord @(0x11, 0x10) 0x56; Start-Sleep -Milliseconds 900
            Check 'a 6000-byte paste asks first (the dialog names the size)' $true ([bool](Get-Element '貼上 5.9 KiB？'))
            Shot-Safe (Shot-Path 'paste-confirm') | Out-Null
            Click-Name '取消' 'left' 700 -Class 'Button'
            Check 'Cancel pasted nothing' $false ((Get-TerminalText) -match 'xxxxxxxx')
            Chord @(0x11, 0x10) 0x56; Start-Sleep -Milliseconds 900
            Click-Name '貼上' 'left' 700 -Class 'Button'
            Check 'the paste button pastes it' $true ((Get-TerminalText) -match 'xxxxxxxx')
            Press 0x1B

            # Several lines ask unless the program asked for bracketed paste: cmd.exe does not.
            Type-Line 'cmd'; Wait-Text '\w:\\[^\r\n]*>\s*$' 8000 | Out-Null
            Set-Clipboard -Value "echo one`r`necho two"
            Chord @(0x11, 0x10) 0x56; Start-Sleep -Milliseconds 900
            Check 'two lines ask first under cmd' $true ([bool](Get-Element '貼上 2 行？'))
            Click-Name '取消' 'left' 700 -Class 'Button'
            Type-Line 'exit'
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}
# What the pointer does around a program: copy on select, what a program may do with the clipboard through OSC 52 (write
# by default, read only when the setting allows it), a program that tracks the mouse (it gets a plain drag, Shift+drag
# selects natively), the wheel and the history, typing returning to the live screen and a selection drag held above the
# pane (it scrolls and keeps selecting). input-probe.js (node, raw stdin) plays the full-screen program.
function Scenario-Pointer {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $probe = Join-Path $PSScriptRoot 'input-probe.js'
    $first = { param($text) ($text -split "`n")[0] -replace '\D', '' }
    try {
        Run-App 'pointer' @{ copyOnSelect = $true; osc52 = 'read-write' } {
            First-Prompt
            Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
            Type-Line '"alpha beta gamma delta"'; Wait-Prompt | Out-Null
            Start-Sleep -Milliseconds 600
            $g = Get-Grid
            $alpha = Find-Cell 'alpha beta gamma delta'
            $beta = $alpha.Col + 6; $gamma = $alpha.Col + 11

            Set-Clipboard -Value 'SENTINEL'
            Drag-Cells $g ($beta + 0.25) $alpha.Row ($beta + 3.75) $alpha.Row; Start-Sleep -Milliseconds 500
            Check 'copy on select: a drag copies without a key' 'beta' ((Get-Clipboard -Raw) -replace "`r`n", "`n")
            Click-Cell $g ($gamma + 1) $alpha.Row 2; Start-Sleep -Milliseconds 500
            Check 'copy on select: a double click copies the word' 'gamma' ((Get-Clipboard -Raw) -replace "`r`n", "`n")
            Click-Cell $g 3 ($g.Rows - 3)

            # OSC 52: the program copies, and (read-write) reads.
            $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('from-osc52 你好'))
            Set-Clipboard -Value 'SENTINEL'
            Type-Line ('$e=[char]27;$b=[char]7;Write-Host -NoNewline "$e]52;c;' + $b64 + '$b"'); Wait-Prompt | Out-Null
            Check 'OSC 52 write reaches the clipboard' 'from-osc52 你好' ((Get-Clipboard -Raw) -replace "`r`n", "`n")
            Set-Clipboard -Value 'read-me-123'
            Type-Line "node `"$probe`" `"`" osc52 2500"
            Check 'OSC 52 read (read-write) answers the program' $true (Wait-Text '<ESC>\]52;c;cmVhZC1tZS0xMjM=<BEL>' 9000)
            Wait-Prompt | Out-Null

            # A program that tracks the mouse gets a plain drag; Shift takes it back.
            if (Get-Command node -ErrorAction SilentlyContinue) {
                Type-Line 'Clear-Host; "alpha beta gamma delta"'; Wait-Prompt | Out-Null
                $alpha = Find-Cell 'alpha beta gamma delta'
                Type-Line "node `"$probe`" 1000,1002,1006 `"`" 9000"
                Start-Sleep -Milliseconds 1500
                $row = $alpha.Row
                Drag-Cells $g 2.25 $row 7.75 $row
                Check 'a plain drag is the program''s: nothing selected' 'SENTINEL' (Copy-Selection)
                With-Keys @(0x10) { Drag-Cells $g ($alpha.Col + 0.25) $row ($alpha.Col + 9.75) $row }
                Check 'Shift+drag selects natively' 'alpha beta' (Copy-Selection)
                Wait-Text 'got:\[.*\]' 14000 | Out-Null
                $seen = ((Get-TerminalText) -split "`n") -join ''
                $report = [regex]::Match($seen, 'got:\[.*?\]')
                'the probe received: ' + $report.Value
                Check 'the probe saw the plain drag (press, motion, release) and not the Shift one' $true ($seen -match 'got:\[<ESC>\[<0;3;\d+M(<ESC>\[<32;\d+;\d+M)+<ESC>\[<0;8;\d+m\]')
            } else { 'skipped: node is not installed' }

            # The wheel moves the history, a key brings the live screen back.
            Type-Line 'Clear-Host; 1..150 | ForEach-Object { "wheel line $_" }'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            $live = & $first (Get-TerminalText)
            Move-To ([int]($g.X + $g.W / 2)) ([int]($g.Y + $g.H / 2))
            Mouse-Wheel 3; Start-Sleep -Milliseconds 400
            $up = & $first (Get-TerminalText)
            Check 'three notches of the wheel scroll nine lines up' 9 ([int]$live - [int]$up)
            Mouse-Wheel -3; Start-Sleep -Milliseconds 400
            Check 'and three back come down again' $live (& $first (Get-TerminalText))
            Mouse-Wheel 5; Start-Sleep -Milliseconds 300
            Type-Unicode 'x' 30; Start-Sleep -Milliseconds 500
            Check 'a key returns to the live screen' $live (& $first (Get-TerminalText))
            Press 0x08

            # A drag held above the pane scrolls and keeps selecting.
            $start = Cell-Pt $g 4 ($g.Rows - 2)
            Assert-Foreground
            Move-To $start.X $start.Y; Start-Sleep -Milliseconds 200
            [D]::Mouse(0x0002, 0); Start-Sleep -Milliseconds 200
            for ($i = 1; $i -le 8; $i++) { Move-To $start.X ([int]($start.Y - ($start.Y - $g.Y + 40) * $i / 8)); Start-Sleep -Milliseconds 30 }
            Start-Sleep -Milliseconds 1500
            [D]::Mouse(0x0004, 0); Start-Sleep -Milliseconds 400
            $lines = @((Copy-Selection) -split "`n").Count
            "a drag held 40 px above the pane for 1.5 s selected $lines lines (the pane shows $($g.Rows))"
            Check 'the selection grew well past one screen' $true ($lines -gt $g.Rows + 15)
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}

# The defaults for a program: it may copy and may not read the clipboard (OSC 52), and the right button (rightClick:
# menu) opens the menu whose Copy is off without a selection.
function Scenario-Menu {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $probe = Join-Path $PSScriptRoot 'input-probe.js'
    try {
        Run-App 'menu' @{ rightClick = 'menu' } {
            First-Prompt
            Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
            Type-Line '"alpha beta gamma delta"'; Wait-Prompt | Out-Null
            Start-Sleep -Milliseconds 600
            $g = Get-Grid
            $alpha = Find-Cell 'alpha beta gamma delta'
            Set-Clipboard -Value 'read-me-123'
            Type-Line "node `"$probe`" `"`" osc52 2500"
            Start-Sleep -Milliseconds 4500; Wait-Prompt | Out-Null
            Check 'OSC 52 read is denied by default: the program got nothing' $true ((Get-TerminalText) -match 'got:\[\]')

            Click-Cell $g 10 ($g.Rows - 3) 1 'right'; Start-Sleep -Milliseconds 700
            Check 'the right button opens a menu with Copy, Paste and Select all' $true ((Get-Element '貼上 (Ctrl+V)') -and (Get-Element '全選 (Ctrl+Shift+A)'))
            Check 'Copy is off without a selection' $false ((Get-Element '複製 (Ctrl+Shift+C)').Current.IsEnabled)
            Shot-Safe (Shot-Path 'pane-menu') | Out-Null
            Click-Name '全選 (Ctrl+Shift+A)' 'left' 700
            Check 'Select all from the menu selects the text' $true ((Copy-Selection) -match 'alpha beta gamma delta')
            Click-Cell $g ($alpha.Col + 12) $alpha.Row 2
            Click-Cell $g 10 ($g.Rows - 3) 1 'right'; Start-Sleep -Milliseconds 700
            Check 'Copy is on with a selection' $true ((Get-Element '複製 (Ctrl+Shift+C)').Current.IsEnabled)
            Press 0x1B
        }
    } finally {
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
}
# Scrolling, prompts and Clear buffer: a page up and down, the top and the bottom of the history, the previous and next
# prompt (the PowerShell integration marks every prompt with OSC 133), and Clear buffer from the palette (screen and
# scrollback go, the shell draws its prompt again).
function Scenario-Scroll {
    Run-App 'scroll' @{} {
        First-Prompt
        Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
        $first = { param($text) $m = [regex]::Match((($text -split "`n")[0]), 'line-(\d+)'); if ($m.Success) { [int]$m.Groups[1].Value } else { -1 } }
        Type-Line '1..200 | ForEach-Object { "line-$_" }'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 600
        $g = Get-Grid
        $live = & $first (Get-TerminalText)
        Chord @(0x11, 0x10) 0x21; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Shift+PageUp scrolls up one page' $g.Rows ($live - (& $first (Get-TerminalText)))
        Chord @(0x11, 0x10) 0x22; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Shift+PageDown scrolls back down' $live (& $first (Get-TerminalText))
        Chord @(0x11, 0x10) 0x24; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Shift+Home shows the oldest line: the prompt that started the output' $true ((((Get-TerminalText) -split "`n")[0]) -match '^PS [^\r\n]*> 1\.\.200')
        Chord @(0x11, 0x10) 0x23; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Shift+End returns to the live screen' $live (& $first (Get-TerminalText))

        # prompts: eight commands with thirty lines of output each
        Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
        foreach ($n in 1..8) { Type-Line ('"cmd-{0}"; 1..30 | ForEach-Object {{ "pad-$_" }}' -f $n); Wait-Prompt | Out-Null }
        Start-Sleep -Milliseconds 500
        $isPrompt = { ((Get-TerminalText) -split "`n")[0] -match '^PS [^\r\n]*> "cmd-(\d)"' }
        $number = { if (((Get-TerminalText) -split "`n")[0] -match '"cmd-(\d)"') { [int]$Matches[1] } else { -1 } }
        Chord @(0x11, 0x12) 0x26; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Alt+Up puts a prompt on the top row' $true (& $isPrompt)
        $a = & $number
        Chord @(0x11, 0x12) 0x26; Start-Sleep -Milliseconds 500
        Check 'and again the one before it' ($a - 1) (& $number)
        Shot-Safe (Shot-Path 'scroll-prompt') | Out-Null
        Chord @(0x11, 0x12) 0x28; Start-Sleep -Milliseconds 500
        Check 'Ctrl+Alt+Down comes back to it' $a (& $number)
        foreach ($i in 1..12) { Chord @(0x11, 0x12) 0x28; Start-Sleep -Milliseconds 200 }
        Check 'past the last prompt it lands on the live screen' $true ((Get-TerminalText) -match 'PS [^\r\n]*>\s*$')

        # Clear buffer from the palette
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 600; Type-Unicode '清除緩衝區' 40; Start-Sleep -Milliseconds 400; Press 0x0D
        Start-Sleep -Milliseconds 1200
        $after = Get-TerminalText
        Check 'Clear buffer leaves only the prompt the shell drew again' $true ($after -match '^PS [^\r\n]*>\s*$')
        Chord @(0x11, 0x10) 0x24; Start-Sleep -Milliseconds 400
        # The shell's own redraw (Ctrl+L) clears its screen once more and leaves one blank line behind.
        Check 'and the history is gone (Ctrl+Shift+Home shows at most one blank line more)' $after ((Get-TerminalText) -replace '^\s*\n', '')
        Shot-Safe (Shot-Path 'scroll-cleared') | Out-Null
    }
}
# Links: an OSC 8 hyperlink, a URL in the text (also one that wraps) and a link that may not be opened. Hovering one
# that may be opened underlines it on every row it covers, shows the hand cursor and a tooltip with where it goes;
# text that is not a link changes nothing. Ctrl+click goes through the same gate as everything else and the target is
# logged before the shell is asked: nothing here starts a browser (NEKOMINAL_NO_LAUNCH), so a web link only reaches the
# log line where the shell would be asked; a link to a file that does not exist is refused, and one with a scheme that
# is not allowed is not a link at all.
function Scenario-Links {
    Run-App 'links' @{ appearance = @{ backdrop = 'none' } } {
        First-Prompt
        Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
        $esc = '$e=[char]27;$b=[char]7;'
        Type-Line ($esc + '"plain https://example.com/plain/path and more"; "${e}]8;;https://example.org/osc8${b}hyperlink text${e}]8;;${b} after"; "bad ${e}]8;;javascript:alert(1)${b}script-link${e}]8;;${b}"; "gone ${e}]8;;file:///C:/nekominal-no-such-folder/nothing.txt${b}missing-file${e}]8;;${b}"; ("x" * 120) + " https://example.net/" + ("long" * 30)')
        Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 700
        $g = Get-Grid
        $plain = Find-Cell 'example.com/plain/path'
        $osc = Find-Cell 'hyperlink text'
        $bad = Find-Cell 'script-link'
        $gone = Find-Cell 'missing-file'
        $wrapped = Find-Cell 'longlonglong'
        $win = Win-Rect
        $cellArea = { param($cell, $cols) @(([int]($g.X - $win.Left + $cell.Col * $g.CW)), ([int]($g.Y - $win.Top + $cell.Row * $g.CH)), ($cols * $g.CW), $g.CH) }
        $diff = { param($a, $b, $cell, $cols) $r = & $cellArea $cell $cols; Region-Diff $a $b $r[0] $r[1] $r[2] $r[3] }

        # A hover is more than one move: the pointer arrives, is nudged and rests (the arrow comes back on every move).
        $hoverAt = { param($pt) Move-To $pt.X $pt.Y; Start-Sleep -Milliseconds 150; Move-To ($pt.X + 2) $pt.Y; Start-Sleep -Milliseconds 600 }
        # nothing hovered: the baseline
        $away = Cell-Pt $g ($g.Cols - 2) ($g.Rows - 1); Move-To $away.X $away.Y; Start-Sleep -Milliseconds 600
        $base = Shot-Path 'links-base'; Shot-Safe $base | Out-Null
        $p = Cell-Pt $g ($plain.Col + 4) $plain.Row; & $hoverAt $p
        Check 'the hand cursor shows over a URL in the text' $true (Cursor-IsHand)
        $hover = Shot-Path 'links-hover-url'; Shot-Safe $hover | Out-Null
        Check 'a URL in the text is underlined' $true ((& $diff $base $hover $plain 22) -gt 40)
        $tipShot = Shot-Path 'links-tooltip'; Shot-Safe $tipShot | Out-Null
        # The tooltip is a box of its own above the link: it changes a good many pixels over the row of text above.
        $above = @{ Row = $plain.Row - 1; Col = $plain.Col }
        Check 'a tooltip with the address hangs above the link' $true ((& $diff $base $tipShot $above 40) -gt 1500)

        $p = Cell-Pt $g ($osc.Col + 3) $osc.Row; & $hoverAt $p
        $hover = Shot-Path 'links-hover-osc8'; Shot-Safe $hover | Out-Null
        Check 'an OSC 8 hyperlink is underlined and shows the hand' $true ((& $diff $base $hover $osc 14) -gt 40 -and (Cursor-IsHand))

        $p = Cell-Pt $g ($bad.Col + 3) $bad.Row; & $hoverAt $p
        $hover = Shot-Path 'links-hover-refused'; Shot-Safe $hover | Out-Null
        Check 'a script address is not a link: no underline, no hand' $true ((& $diff $base $hover $bad 11) -eq 0 -and -not (Cursor-IsHand))

        $p = Cell-Pt $g ($wrapped.Col + 2) $wrapped.Row; & $hoverAt $p
        $hover = Shot-Path 'links-hover-wrapped'; Shot-Safe $hover | Out-Null
        $rowsMarked = @(($wrapped.Row - 1)..($wrapped.Row + 1) | Where-Object { (& $diff $base $hover @{ Row = $_; Col = 0 } $g.Cols) -gt 40 }).Count
        Check 'a URL that wraps is underlined on every row it covers (2 or more rows)' $true ($rowsMarked -ge 2)

        $p = Cell-Pt $g 2 $plain.Row; & $hoverAt $p
        Check 'plain text beside a link shows no hand' $false (Cursor-IsHand)

        # Ctrl+click
        $mark = Log-Mark
        $p = Cell-Pt $g ($gone.Col + 3) $gone.Row
        With-Keys @(0x11) { Click-At $p.X $p.Y 'left' 600 }
        $lines = Log-Since $mark | Where-Object { $_ -match 'link refused|opening' }
        $lines | ForEach-Object { 'log: ' + $_.Trim() }
        Check 'Ctrl+click on a link to a file that does not exist is refused, naming the path' $true ([bool]($lines -match 'link refused \(no such file\): C:\\nekominal-no-such-folder\\nothing\.txt'))
        $mark = Log-Mark
        $p = Cell-Pt $g ($bad.Col + 3) $bad.Row
        With-Keys @(0x11) { Click-At $p.X $p.Y 'left' 600 }
        Check 'Ctrl+click on a script address is only a click: nothing is logged' 0 @(Log-Since $mark | Where-Object { $_ -match 'link|opening' }).Count
        # A web link: a click alone opens nothing, Ctrl+click names the target and hands it to the shell (not started here).
        $mark = Log-Mark
        $p = Cell-Pt $g ($plain.Col + 4) $plain.Row
        Click-At $p.X $p.Y 'left' 600
        Check 'a click on a web link without Ctrl opens nothing' 0 @(Log-Since $mark | Where-Object { $_ -match 'link|opening' }).Count
        With-Keys @(0x11) { Click-At $p.X $p.Y 'left' 600 }
        $lines = @(Log-Since $mark | Where-Object { $_ -match 'link refused|opening|not opened' })
        $lines | ForEach-Object { 'log: ' + $_.Trim() }
        Check 'Ctrl+click on a web link names it, then the shell is asked' $true ([bool]($lines -match 'opening Web link: https://example\.com/plain/path$') -and [bool]($lines -match 'not opened \(NEKOMINAL_NO_LAUNCH is set\): https://example\.com/plain/path$'))
    }
}
# Restart, reload and the settings file. A shell that exits with an error leaves its last screen and the pane can be
# started again without a question; one that still runs asks first. Reload reads settings.json again (a new colour
# scheme and font size show at once, a wrong value is reported like at start-up, a file that is fine takes the report
# away). Opening the settings file is logged where the shell would be asked: the run sets NEKOMINAL_NO_LAUNCH.
function Scenario-Restart {
    $file = Join-Path ([IO.Path]::GetTempPath()) 'nekominal-drive\restart-appdata\Nekominal\settings.json'
    Run-App 'restart' @{ confirmClose = $true; appearance = @{ backdrop = 'none' }; theme = 'Tokyo Night' } {
        First-Prompt
        Type-Line 'Set-PSReadLineOption -PredictionSource None; Clear-Host'; Wait-Prompt | Out-Null
        $palette = { param($name) Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 600; Type-Unicode $name 40; Start-Sleep -Milliseconds 500; Press 0x0D; Start-Sleep -Milliseconds 800 }

        # an ended session: restarting needs no question
        Type-Line 'exit 3'; Start-Sleep -Milliseconds 1500
        Check 'a shell that exits with code 3 leaves its last screen' $true ((Get-TerminalText) -match '\[已結束，代碼 3\]')
        & $palette '重新啟動工作階段'
        Check 'restarting an ended session asks nothing and starts on a clear screen with a new prompt' $true (Wait-Text '^PS [^\r\n]*>\s*$' 15000)
        Chord @(0x11, 0x10) 0x24; Start-Sleep -Milliseconds 500
        Check 'and the old screen is in the scrollback' $true ((Get-TerminalText) -match '\[已結束，代碼 3\]')
        Chord @(0x11, 0x10) 0x23; Start-Sleep -Milliseconds 300
        Type-Line 'echo again'
        Check 'and the pane works again' $true (Wait-Text 'again\s+PS ' 8000)

        # a session that still works: it asks, Cancel keeps it
        Type-Line 'Start-Sleep 40'; Start-Sleep -Milliseconds 800
        & $palette '重新啟動工作階段'
        Check 'restarting a running command asks, naming the tab' $true ([bool](Get-Element -Name '重新啟動工作階段' -Class 'Button'))
        Shot-Safe (Shot-Path 'restart-confirm') | Out-Null
        Click-Name '取消' 'left' 700 -Class 'Button'
        Start-Sleep -Milliseconds 800
        Check 'Cancel leaves the command running' $false ((Get-TerminalText) -match 'PS [^\r\n]*>\s*$')
        & $palette '重新啟動工作階段'
        Click-Name '重新啟動工作階段' 'left' 700 -Class 'Button'
        Check 'confirming starts the shell again' $true (Wait-Text 'PS [^\r\n]*>\s*$' 15000)

        # reload: another scheme and a bigger font, then a wrong value, then a fixed file
        $g = Get-Grid
        Shot-Safe (Shot-Path 'restart-before') | Out-Null
        $colourBefore = Region-Average (Shot-Path 'restart-before') 600 500 40 40
        Set-Content -LiteralPath $file -Encoding UTF8 -Value '{ "theme": "Dracula", "font": { "size": 22 }, "appearance": { "backdrop": "none" } }'
        & $palette '重新載入設定'
        Start-Sleep -Milliseconds 1500
        Shot-Safe (Shot-Path 'restart-reloaded') | Out-Null
        $colourAfter = Region-Average (Shot-Path 'restart-reloaded') 600 500 40 40
        Check 'reload applies the colour scheme' $true ($colourBefore -ne $colourAfter)
        $grid = Select-String -Path $script:LogFile -Pattern 'pane \d+ grid (\d+)x(\d+) cells of (\d+)x(\d+) px' | Select-Object -Last 1
        Check 'reload applies the font size (a new cell size)' $true ($grid.Matches[0].Groups[3].Value -ne [string]$g.CW)
        Check 'a file without problems reports none' $false ([bool](Get-Element '設定 JSON 有 1 個問題'))
        Set-Content -LiteralPath $file -Encoding UTF8 -Value '{ "font": { "size": 999 }, "theme": "Dracula" }'
        & $palette '重新載入設定'
        Start-Sleep -Milliseconds 800
        Check 'a wrong value is reported like at start-up' $true ([bool](Get-Element '設定 JSON 有 1 個問題'))
        Shot-Safe (Shot-Path 'restart-problem') | Out-Null
        Set-Content -LiteralPath $file -Encoding UTF8 -Value '{ "theme": "Dracula" }'
        & $palette '重新載入設定'
        Start-Sleep -Milliseconds 800
        Check 'a fixed file takes the report away' $false ([bool](Get-Element '設定 JSON 有 1 個問題'))
        Set-Content -LiteralPath $file -Encoding UTF8 -Value '{ "theme": "Dracula", "keybindings": { "tab.new": "Ctrl+Shift+Y" } }'
        & $palette '重新載入設定'
        Start-Sleep -Milliseconds 600
        Check 'a shortcut from the file shows in the tooltip of the button' $true ([bool](Get-Element '新增分頁 (Ctrl+Shift+Y)'))

        # the settings file: logged, not opened
        $mark = Log-Mark
        & $palette '開啟設定 JSON'
        Start-Sleep -Milliseconds 800
        $lines = @(Log-Since $mark | Where-Object { $_ -match 'settings file|not opened' })
        $lines | ForEach-Object { 'log: ' + $_.Trim() }
        Check 'opening the settings file names the file and starts nothing' $true ([bool]($lines -match 'not opened \(NEKOMINAL_NO_LAUNCH is set\): .*restart-appdata\\Nekominal\\settings\.json'))
    }
}
function Scenario-All { Scenario-Glass; Scenario-BlurProof; Scenario-TabsSplits; Scenario-Overlays; Scenario-Inactive; Scenario-Ime; Scenario-Perf; Scenario-Silence; Scenario-Opacity; Scenario-Tabs; Scenario-Status; Scenario-Titles; Scenario-Close; Scenario-Panes }
