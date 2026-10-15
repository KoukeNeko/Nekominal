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
#   Scenario-TabDrag          # dragging an unselected chip of a compressed strip moves it by the widths of the layout the press made
#   Scenario-Titles           # default tab titles: folders, collisions, program titles, bidi cleaning
#   Scenario-Panes            # pane count and zoom marks on a chip, the corner label of a zoomed pane, warning line of an unfocused pane
#   Scenario-Status           # tab status marks for OSC 7501 / 9 / 777 and BEL, bell decisions with the window active and not
#   Scenario-Unread           # unread responses: the !N of the plate, the grouped list, Ctrl+Shift+U, alerts of an inactive window
#   Scenario-StatusExit       # a program that ends with an error in the middle of a progress report shows an error
#   Scenario-Toolbar          # bar tiers at 1440/1000/700/500 DIP, the menu of the "..." button, the split rows of the tab menu
#   Scenario-Drawer           # the settings drawer: open and close, no resize, focus, Esc steps, search in both languages
#   Scenario-DrawerRows       # keyboard-only colour scheme and opacity tasks, preview and restore, reset, toggles, backdrop None
#   Scenario-DrawerPages      # settings file problems, profiles, agents, advanced
#   Scenario-DrawerKeys       # the shortcuts page: recording, conflicts, Replace, reset, restore all
#   Scenario-FirstWrite       # the first rewrite of settings.json keeps the original as settings.json.orig
#   Scenario-DrawerGlass      # the text of the drawer measured over the striped window at 20 % and 100 % background opacity
#   Scenario-DrawerWidths     # the drawer at 1440/1000/700/500 DIP of window and in English: width rule, margins, no resize
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
# browser or editor starts. The interface language is zh-TW unless the settings say otherwise, so that the names of
# elements (Nk-Text) are the same on every machine.
function Run-App([string]$Name, [hashtable]$Settings = @{}, [scriptblock]$Script, [switch]$Pattern, [switch]$NoShot, [hashtable]$Env = @{}) {
    if (-not $Settings.ContainsKey('language')) { $Settings = $Settings + @{ language = 'zh-TW' } }
    $params = @{ Name = $Name; OutDir = $global:NkShots; Settings = $Settings; Script = $Script; SettleMs = 3500; Env = (@{ NEKOMINAL_NO_LAUNCH = '1' } + $Env) }
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
        Click-Name 'logs'; Type-Line 'Start-Sleep 60'; Start-Sleep -Milliseconds 800                        # a running command: closing asks
        $l = Get-ElementRect 'logs'; Click-At ($l.X + 40) ($l.Y + 16) 'middle' 1200                            # middle click closes
        Shot-Safe (Shot-Path 'tab-middle-click-confirm')                                                       # a running shell asks first
        Click-Name (Nk-Text 'common.close') -Class 'Button'; Start-Sleep -Milliseconds 1200; Shot-Safe (Shot-Path 'tab-middle-closed')
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
        Open-AppearanceFlyout; Start-Sleep -Milliseconds 900; Shot-Safe (Shot-Path 'overlay-appearance-flyout'); Press 0x1B; Start-Sleep -Milliseconds 500
        $t = Get-ElementRect (Get-Chips | Select-Object -First 1).Name; Click-At ($t.X + 40) ($t.Y + 16) 'right' 900; Shot-Safe (Shot-Path 'overlay-tab-menu'); Press 0x1B; Start-Sleep -Milliseconds 500
        # Closing asks only when work would end: a command runs.
        Type-Line 'Start-Sleep 60'; Start-Sleep -Milliseconds 800
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200; Shot-Safe (Shot-Path 'overlay-close-dialog'); Press 0x1B; Start-Sleep -Milliseconds 500
        # The settings drawer floats over the terminal; the pointer is moved off the caption buttons so that no snap layout
        # flyout is in the picture.
        Open-Drawer; $window = Win-Rect; Move-To ($window.Left + 200) ($window.Top + 500); Start-Sleep -Milliseconds 500
        Shot-Safe (Shot-Path 'settings-appearance')
        Click-Name (Nk-Text 'settings.keys') -Class 'ToggleButton'; Start-Sleep -Milliseconds 900; Move-To ($window.Left + 200) ($window.Top + 500); Shot-Safe (Shot-Path 'settings-keys')
    } -NoShot
}

function Scenario-Inactive {
    Run-App 'inactive' @{} {
        Fill-Terminal
        Shot-WithPattern (Shot-Path 'inactive-1-active')
        # Our own pattern window becomes the active one, outside the app rectangle: the app stays fully visible.
        if (-not (Move-PatternAway)) { throw 'no corner of the screen is free of the app window' }
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
# a row passes when the peak stays below $global:WitnessThreshold. The witness first has to hear one MessageBeep (the
# positive control, audible once), or no row proves anything and the scenario fails. The log must show a WM_MENUCHAR
# (answered with MNC_CLOSE) for every Alt+character row and none for the others. Then Alt+Space must still open the
# system menu and holding Alt+Shift+D for two seconds must add exactly one pane.
function Scenario-Silence {
    . (Join-Path $PSScriptRoot 'sound-witness.ps1')
    Run-App 'silence' @{} {
        First-Prompt
        Start-SoundWitness | Out-Null
        $results = @()
        $control = Test-WitnessPositive
        $results += Check-True 'positive control: the witness hears one MessageBeep' $control.Seen $control.Text
        $audible = $control.Seen
        # The split buttons are on the bar from 1200 DIP.
        Set-WindowWidth 1300
        # The beep of the control has to fade out before the baseline of the first row is taken.
        for ($i = 0; $i -lt 20; $i++) { Reset-SoundWitness; Start-Sleep -Milliseconds 600; if ((Get-SoundPeak) -lt $global:WitnessThreshold) { break } }
        # A row: the trigger and whether it sends Alt+character to Windows (WM_SYSCHAR, then WM_MENUCHAR).
        $rows = [ordered]@{
            'toolbar split right button' = @{ Do = { Click-Name (Nk-Tip 'pane.splitRight' 'Alt+Shift+D') -Class 'Button' }; AltChar = $false }
            'menu button, split right row' = @{ Do = { Click-Name (Nk-Text 'menu.app') -Class 'Button'; Start-Sleep -Milliseconds 500; Click-Name (Nk-Tip 'pane.splitRight' 'Alt+Shift+D') }; AltChar = $false }
            'palette: split down'        = @{ Do = { Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 500; Type-Unicode (Nk-Text 'pane.splitDown') 40; Press 0x0D }; AltChar = $false }
            'Alt+Shift+D'                = @{ Do = { Chord @(0x12, 0x10) 0x44 }; AltChar = $true }
            'Alt+Shift+S'                = @{ Do = { Chord @(0x12, 0x10) 0x53 }; AltChar = $true }
            'Alt+Shift+='                = @{ Do = { Chord @(0x12, 0x10) 0xBB }; AltChar = $true }
            'Alt+Shift+-'                = @{ Do = { Chord @(0x12, 0x10) 0xBD }; AltChar = $true }
            'Alt+Left'                   = @{ Do = { Chord @(0x12) 0x25 }; AltChar = $false }
            'Alt+Shift+Left'             = @{ Do = { Chord @(0x12, 0x10) 0x25 }; AltChar = $false }
            'Alt tap, then x'            = @{ Do = { Press 0x12; Press 0x58; Press 0x1B }; AltChar = $false }
            'Alt+x'                      = @{ Do = { Chord @(0x12) 0x58 }; AltChar = $true }
            'Ctrl+Shift+Z'               = @{ Do = { Chord @(0x11, 0x10) 0x5A; Chord @(0x11, 0x10) 0x5A }; AltChar = $false }
            'Ctrl+Shift+T'               = @{ Do = { Chord @(0x11, 0x10) 0x54 }; AltChar = $false }
        }
        foreach ($name in $rows.Keys) {
            $mark = Log-Mark
            $peak = Measure-Peak $rows[$name].Do
            $answered = @(Log-Since $mark | Where-Object { $_ -match 'WM_MENUCHAR #\d+ answered with MNC_CLOSE' }).Count
            $quiet = $null -ne $peak -and $peak -lt $global:WitnessThreshold
            $detail = if ($null -eq $peak) { 'inconclusive: the endpoint was not quiet before the row' } elseif (-not $audible) { 'inconclusive: the witness did not hear its positive control' } else { 'peak {0:E1}' -f $peak }
            $results += Check-True "$name is quiet" ($quiet -and $audible) $detail
            $results += Check-True "$name raises WM_MENUCHAR $(if ($rows[$name].AltChar) { 'and it is answered' } else { 'never' })" $(if ($rows[$name].AltChar) { $answered -ge 1 } else { $answered -eq 0 }) "$answered answered in the log"
        }
        $script:menu = $false
        $menuPeak = Measure-Peak { Chord @(0x12) 0x20; Start-Sleep -Milliseconds 500; $script:menu = [Menus]::MenuIsOpen(); Press 0x1B }
        $results += Check-True 'Alt+Space still opens the system menu' $script:menu
        $results += Check-True 'and is quiet' ($audible -and $null -ne $menuPeak -and $menuPeak -lt $global:WitnessThreshold) ('peak {0:E1}' -f $menuPeak)
        Chord @(0x11, 0x10) 0x54; Wait-Prompt | Out-Null
        $before = Count-Panes
        Hold-Chord @(0x12, 0x10) 0x44 2000
        Start-Sleep -Milliseconds 800
        $results += Check 'holding Alt+Shift+D for 2 s adds exactly one pane' ($before + 1) (Count-Panes)
        Assert-Results $results 'silence'
    } -NoShot
}

function Log-Mark { @(Get-Content -LiteralPath $script:LogFile).Count }
function Log-Since([int]$Mark) { $all = @(Get-Content -LiteralPath $script:LogFile); if ($all.Count -gt $Mark) { $all[$Mark..($all.Count - 1)] } else { @() } }
function Log-Ms([string]$Line) { [double]$Line.Substring(0, 9).Trim() }
# How many elements have this class name, or a class name that ends with it after a dot (XAML controls report their
# full name: 'Microsoft.UI.Xaml.Controls.InfoBar').
function Count-Class([string]$Class) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $count = 0
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
        $name = $e.Current.ClassName
        if ($name -eq $Class -or $name.EndsWith(".$Class")) { $count++ }
    }
    $count
}
# Whether a confirmation dialog is open. A dialog lives in a Popup of its own, which UI Automation names by its title;
# what every dialog of the app has is a Cancel button.
function Dialog-Open { [bool](Get-Element (Nk-Text 'common.cancel') 'Button') }
function Get-Slider([int]$Index = 0) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Slider)
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)[$Index]
}
function Select-Radio([string]$Name) { (Get-Element $Name 'RadioButton').GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select() }

# Opens the quick appearance flyout (background material and opacity).
function Open-AppearanceFlyout {
    Click-Name (Nk-Text 'menu.app') -Class 'Button'; Start-Sleep -Milliseconds 600
    # The row closes the menu, and the flyout opens under the same button once it is gone.
    Click-Name (Nk-Text 'menu.opacity') 'left' 900
}

function Rgb([string]$Text) { [int[]]($Text -split ',') }
# How far apart two mean colours are: the sum of the differences of the channels.
function Rgb-Distance([string]$A, [string]$B) { $x = Rgb $A; $y = Rgb $B; [Math]::Abs($x[0] - $y[0]) + [Math]::Abs($x[1] - $y[1]) + [Math]::Abs($x[2] - $y[2]) }
function Slider-Value($Slider) { $Slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value }

# --- the settings drawer ---------------------------------------------------------------------------------------------------
# Polls until the condition holds (UI Automation sees a change a little after it is made).
function Wait-Until([scriptblock]$Condition, [int]$TimeoutMs = 5000) {
    $end = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $end) { if (& $Condition) { return $true }; Start-Sleep -Milliseconds 80 }
    $false
}
function Focused-Name { try { [System.Windows.Automation.AutomationElement]::FocusedElement.Current.Name } catch { '' } }
function Focused-Class { try { [System.Windows.Automation.AutomationElement]::FocusedElement.Current.ClassName } catch { '' } }
# The search box is the one element of the drawer that is always there while it is open (a hidden drawer is not in the tree).
function Drawer-Search { Get-Element (Nk-Text 'settings.search') 'TextBox' }
function Drawer-Open { [bool](Drawer-Search) }
# Ctrl+Shift+O, not Ctrl+,: the Bopomofo input method of this machine takes the comma chord as punctuation.
function Open-Drawer { Chord @(0x11, 0x10) 0x4F; [void](Wait-Until { Drawer-Open }); Start-Sleep -Milliseconds 400 }
function Close-Drawer { Press 0x1B; [void](Wait-Until { -not (Drawer-Open) }); Start-Sleep -Milliseconds 400 }
# The state of a toggle button found by its name: On or Off.
function Toggle-State([string]$Name) {
    $e = Get-Element $Name 'ToggleButton'
    if ($e) { [string]$e.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState }
}
# The lines of the log since $Mark that say the first pane was given another grid (a resize of its ConPTY follows each).
function Grid-Lines([int]$Mark) { @(Log-Since $Mark | Where-Object { $_ -match 'pane 1 grid ' }) }
# The rectangle of the first terminal pane on the screen, and a click on it (at its left, where the drawer is not).
function Terminal-Rect {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
    $b = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond).Current.BoundingRectangle
    @{ X = [int]$b.X; Y = [int]$b.Y; W = [int]$b.Width; H = [int]$b.Height }
}
function Click-Terminal { $r = Terminal-Rect; Click-At ($r.X + 120) ($r.Y + [int]($r.H / 2)) 'left' 500 }

# Background opacity: a live change on an idle terminal changes its background (pixels over the striped window), a
# replay of 200 slider events makes no SetSystemBackdrop call, at most one apply per frame and one save and shows no
# InfoBar or dialog, backdrop None turns the slider off without touching the saved value, and 20 backdrop
# switches log no error. (A change made through UI Automation activates the window first, so a change while the
# window is inactive cannot be driven from here.)
function Scenario-Opacity {
    Run-App 'opacity' @{ appearance = @{ opacity = 0.55 } } {
        First-Prompt
        Type-Line 'Clear-Host'; Wait-Prompt | Out-Null
        $results = @()
        Open-AppearanceFlyout; Start-Sleep -Milliseconds 1200
        # 1. idle terminal: no key, no output between the pictures. The sampled region is a part of the first terminal
        # (blank after Clear-Host), found from its UI Automation rectangle, in the pixels of the picture.
        $shot = {
            param($name)
            Shot-WithPattern (Shot-Path $name) | Out-Null
            $window = Win-Rect; $margin = $script:PatternMargin
            $region = Terminal-Region 0.35 0.6 300 150 ($window.Left - $margin) ($window.Top - $margin)
            Region-Average (Shot-Path $name) $region.X $region.Y $region.W $region.H
        }
        $a = & $shot 'opacity-live-55'
        Set-Slider 25 0; Start-Sleep -Milliseconds 1500
        $b = & $shot 'opacity-live-25'
        Set-Slider 100 0; Start-Sleep -Milliseconds 1500
        $c = & $shot 'opacity-live-100'
        'idle terminal, mean RGB of its background: 55% {0} | 25% {1} | 100% {2}' -f $a, $b, $c
        $apart = (Rgb-Distance $a $b), (Rgb-Distance $b $c), (Rgb-Distance $a $c)
        $results += Check-True 'the background of an idle terminal follows the slider (55, 25 and 100 % all differ)' (($apart | Measure-Object -Minimum).Minimum -ge 6) "distances $($apart -join ', ')"
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
        $gaps = @(for ($i = 1; $i -lt $applies.Count; $i++) { (Log-Ms $applies[$i]) - (Log-Ms $applies[$i - 1]) })
        $shortest = if ($gaps.Count) { ($gaps | Measure-Object -Minimum).Minimum } else { [double]::PositiveInfinity }
        'replay: 200 value changes in {0} ms -> {1} applies (shortest gap {2:N1} ms)' -f $replayMs, $applies.Count, $shortest
        $results += Check-True 'a replay of 200 value changes applies at least the last one and far fewer than 200' ($applies.Count -ge 1 -and $applies.Count -lt 100) "$($applies.Count) applies"
        $results += Check-True 'and applies at most once per frame (no two applies closer than 5 ms)' ($shortest -ge 5) "shortest gap $shortest ms"
        $results += Check-True 'the last value is the one applied' ($applies.Count -ge 1 -and $applies[-1] -match 'opacity applied 0\.96$') "$($applies[-1])"
        $results += Check 'it never sets the system backdrop' 0 (@($lines | Where-Object { $_ -match 'system backdrop set' }).Count)
        $results += Check-True 'it saves at most once' (@($lines | Where-Object { $_ -match 'settings saved' }).Count -le 1) "$(@($lines | Where-Object { $_ -match 'settings saved' }).Count) saves"
        $results += Check 'it logs no apply error' 0 (@($lines | Where-Object { $_ -match 'apply_glass:|apply_opacity:' }).Count)
        $results += Check 'it shows no InfoBar' 0 (Count-Class 'InfoBar')
        $results += Check 'and no dialog' $false (Dialog-Open)
        $file = Join-Path (Join-Path (Join-Path (Split-Path $script:LogFile) 'opacity-appdata') 'Nekominal') 'settings.json'
        $results += Check 'the last value is what was saved' 0.96 ((Get-Content $file -Raw | ConvertFrom-Json).appearance.opacity)
        # 3. twenty backdrop switches: the system backdrop is replaced once per change of kind, nothing fails
        $mark = Log-Mark
        $names = (Nk-Text 'backdrop.acrylic'), (Nk-Text 'backdrop.mica'), (Nk-Text 'backdrop.micaAlt'), (Nk-Text 'backdrop.none')
        foreach ($i in 0..19) { Select-Radio $names[$i % 4]; Start-Sleep -Milliseconds 150 }
        Start-Sleep -Milliseconds 800
        $lines = @(Log-Since $mark)
        # The window starts on Acrylic, which is the first choice: 19 of the 20 choices change the kind.
        $results += Check '20 backdrop choices replace the system backdrop once per change of kind' 19 (@($lines | Where-Object { $_ -match 'system backdrop set' }).Count)
        $results += Check 'and log no error' 0 (@($lines | Where-Object { $_ -match 'apply_glass:|apply_opacity:|rror' }).Count)
        $results += Check 'and show no InfoBar' 0 (Count-Class 'InfoBar')
        # 4. backdrop None: the slider is off and reads 100%, the saved value (the last one of the replay) stays
        Select-Radio (Nk-Text 'backdrop.none'); Start-Sleep -Milliseconds 1500
        $s = Get-Slider 0
        $saved = (Get-Content $file -Raw | ConvertFrom-Json).appearance
        'backdrop None: slider enabled={0} value={1}, saved appearance: {2}' -f $s.Current.IsEnabled, (Slider-Value $s), ($saved | ConvertTo-Json -Compress)
        $results += Check 'backdrop None turns the slider off' $false $s.Current.IsEnabled
        $results += Check 'and shows 100 %' 100 (Slider-Value $s)
        $results += Check 'and keeps the saved opacity' 0.96 $saved.opacity
        Select-Radio (Nk-Text 'backdrop.acrylic'); Start-Sleep -Milliseconds 800
        $results += Check 'back to Acrylic the slider is on' $true (Get-Slider 0).Current.IsEnabled
        $results += Check 'and shows the saved opacity' 96 (Slider-Value (Get-Slider 0))
        Press 0x1B; Start-Sleep -Milliseconds 500
        Assert-Results $results 'opacity'
    } -Pattern -NoShot
}

function Plate-Text {
    $plate = Get-Element (Nk-Text 'tab.all') 'Button'
    if (-not $plate) { return $null }
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    ($plate.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name }) -join ''
}

# The tab shown now (one-based), from the last "selected tab" line of the log.
function Selected-Tab {
    $line = Select-String -Path $script:LogFile -Pattern 'selected tab (\d+)' | Select-Object -Last 1
    if ($line) { [int]$line.Matches[0].Groups[1].Value } else { 0 }
}

# Presses the position plate, which opens the list of every tab.
function Open-TabList {
    $r = (Get-Element (Nk-Text 'tab.all') 'Button').Current.BoundingRectangle
    Click-At ([int]($r.X + $r.Width / 2)) ([int]($r.Y + $r.Height / 2)) 'left' 800
}

# The chips of the strip as UI Automation shows them (one per tab: a named container with a position in its set).
function Get-Chips {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $pos = [System.Windows.Automation.AutomationProperty]::LookupById(30152)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NamedContainerAutomationPeer')
    foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        $p = $e.GetCurrentPropertyValue($pos)
        # Any other named container (the drawer) reports the default position, -1.
        if ($p -is [int] -and $p -ge 1) { [pscustomobject]@{ Index = $p; Name = $e.Current.Name; Status = $e.Current.ItemStatus; Rect = $e.Current.BoundingRectangle; Offscreen = $e.Current.IsOffscreen } }
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

# The horizontal scroll offset of the tab strip in DIP, from the scroll pattern of its ScrollViewer.
function Strip-Offset {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $scroll = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ScrollViewer')))
    $pattern = $scroll.GetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern)
    $visible = $pattern.Current.HorizontalViewSize
    if ($visible -ge 100) { return 0.0 }
    $viewport = $scroll.Current.BoundingRectangle.Width / (Get-Scale)
    $pattern.Current.HorizontalScrollPercent / 100 * ($viewport * 100 / $visible - $viewport)
}
# The strip as it is now, in DIP, with the checks the width algorithm and the position plate promise: the plate says
# `current / total` (or the total alone on a narrow strip), shown plus hidden chips are all of them, the selected chip
# is in view with 12 DIP to a hidden neighbour, an unselected chip is 48 to 216 DIP wide and the selected one at least
# 100. Returns the checks; the line describing the strip is written out too.
function Measure-Strip([int]$N, [int]$W) {
    $scale = Get-Scale
    $chips = @(Get-Chips); $plate = Plate-Text; $edges = Get-EdgeCounts; $selected = Selected-Tab
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $scroll = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ScrollViewer')))
    $v = $scroll.Current.BoundingRectangle
    # A chip is visible when any part of it lies inside the strip's viewport; the UIA rectangle of a chip that is
    # scrolled out is empty (infinite).
    $shown = @($chips | Where-Object { -not [double]::IsInfinity($_.Rect.Width) -and $_.Rect.X + $_.Rect.Width -gt $v.X + 0.5 -and $_.Rect.X -lt $v.X + $v.Width - 0.5 })
    $sel = $chips | Where-Object { $_.Index -eq $selected } | Select-Object -First 1
    $left = $null; $right = $null
    if ($sel -and -not [double]::IsInfinity($sel.Rect.Width)) { $left = ($sel.Rect.X - $v.X) / $scale; $right = ($v.X + $v.Width - $sel.Rect.X - $sel.Rect.Width) / $scale }
    $widths = $chips | Sort-Object Index | ForEach-Object { if ([double]::IsInfinity($_.Rect.Width)) { $null } else { $_.Rect.Width / $scale } }
    'N={0,2} W={1,4}: plate "{2}" | selected {3}, chips {4}, visible {5} + hidden {6}+{7} | selected margins {8:N0}/{9:N0} DIP | widths {10}' -f $N, $W, $plate, $selected, $chips.Count, $shown.Count, $edges[0], $edges[1], $left, $right, (($widths | ForEach-Object { if ($null -eq $_) { '-' } else { [int]$_ } }) -join ',')
    $label = "N=$N W=$W"
    if ($N -lt 2) { Check-True "$label a single tab has no plate" ($null -eq $plate) "plate '$plate'" }
    else { Check-True "$label the plate says the position or the total" ($plate -in "$selected / $N", "$N") "plate '$plate', selected tab $selected" }
    Check "$label there is a chip for every tab" $N $chips.Count
    Check "$label visible and hidden chips are all of them" $N ($shown.Count + $edges[0] + $edges[1])
    Check-True "$label the selected chip is in view" ($null -ne $left -and $left -ge -0.7 -and $right -ge -0.7) "margins $left / $right"
    Check-True "$label with 12 DIP to a hidden neighbour" ((($edges[0] -eq 0) -or ($left -ge 11.3)) -and (($edges[1] -eq 0) -or ($right -ge 11.3))) "margins $left / $right, hidden $($edges -join '+')"
    # Only a chip clear of both edges of the viewport has its own width: UI Automation clips the rectangle of one cut by an edge to the part that shows, so it ends at the edge.
    $whole = @($chips | Where-Object { -not [double]::IsInfinity($_.Rect.Width) -and $_.Rect.X -gt $v.X + 0.7 -and $_.Rect.X + $_.Rect.Width -lt $v.X + $v.Width - 0.7 })
    $narrow = @($whole | Where-Object { $_.Rect.Width / $scale -lt $(if ($_.Index -eq $selected) { 99 } else { 47.3 }) })
    $wide = @($whole | Where-Object { $_.Rect.Width / $scale -gt 216.7 })
    Check-True "$label chips are at least 48 DIP wide (the selected one 100) and at most 216" ($narrow.Count -eq 0 -and $wide.Count -eq 0) "narrow: $(($narrow | ForEach-Object { $_.Index }) -join ','), wide: $(($wide | ForEach-Object { $_.Index }) -join ',')"
}

# Tab awareness: the position plate, the width algorithm, the edge counts, the selected chip in view, the list of every
# tab, most-recently-used Ctrl+Tab and closing back to the tab used before. One run grows the tab count to 30 and
# measures every window width at 1, 2, 5, 12 and 30 tabs. A tab in a folder named like a markup extension must be
# in the list and picked by it.
function Scenario-Tabs {
    $global:NkGuidDir = Join-Path ([IO.Path]::GetTempPath()) '{nk-3F2504E0-4F89-41D3-9A0C-0305E82C3301}'
    [void][IO.Directory]::CreateDirectory($global:NkGuidDir)
    try {
        Run-App 'tabs' @{ confirmClose = $false } {
            $results = [System.Collections.Generic.List[string]]::new()
            First-Prompt
            Name-Tab 'aaa'
            # the single tab has no plate
            foreach ($w in 800, 988, 1920) { Set-WindowWidth $w; Measure-Strip 1 $w | ForEach-Object { if ($_ -like 'ok *' -or $_ -like 'FAIL*') { $results.Add($_) } else { $_ } } }
            Set-WindowWidth 988
            New-Tab; Name-Tab 'bbb'; New-Tab; Name-Tab 'ccc'
            # most recently used: select aaa, bbb, ccc in turn, then Ctrl+Tab once and with Ctrl held for two presses
            foreach ($name in 'aaa', 'bbb', 'ccc') { Click-Name $name; Start-Sleep -Milliseconds 300 }
            Chord @(0x11) 0x09; Start-Sleep -Milliseconds 500
            $results.Add((Check 'Ctrl+Tab, tapped, goes to the tab used before (bbb)' 2 (Selected-Tab)))
            [D]::Key(0x11, $false); Start-Sleep -Milliseconds 100
            Press 0x09; Press 0x09; Start-Sleep -Milliseconds 200
            [D]::Key(0x11, $true); Start-Sleep -Milliseconds 500
            $results.Add((Check 'Ctrl held, Tab twice, goes two back in the order of use (aaa)' 1 (Selected-Tab)))
            # closing the selected tab goes back to the one used before it, not to a neighbour
            Click-Name 'ccc'; Start-Sleep -Milliseconds 300; Click-Name 'aaa'; Start-Sleep -Milliseconds 300; Click-Name 'bbb'; Start-Sleep -Milliseconds 300
            $b = Get-ElementRect 'bbb'; Click-At ($b.X + 40) ($b.Y + 16) 'middle' 1000
            $results.Add((Check 'closing bbb (selected, aaa before it) shows aaa, and leaves two tabs' '1 2' ('{0} {1}' -f (Selected-Tab), @(Get-Chips).Count)))
            Name-Tab 'claude one'
            # a folder named like a markup extension: its tab is in the list, and Enter on its row picks that tab
            $folder = Split-Path $global:NkGuidDir -Leaf
            New-Tab; Type-Line "Set-Location -LiteralPath '$global:NkGuidDir'"; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 700
            $chips = @(Get-Chips | Sort-Object Index)
            $results.Add((Check 'a tab in a folder named like a markup extension is called by the folder' $folder (Chip-Title $chips[-1])))
            Open-TabList
            $results.Add((Check 'the list has a row for every tab, none dropped' $chips.Count (@(Get-Items).Count)))
            Type-Unicode 'nk-3F25' 40; Start-Sleep -Milliseconds 600
            $results.Add((Check 'the folder name narrows the list to its tab' 1 (@(Get-Items).Count)))
            Press 0x0D; Start-Sleep -Milliseconds 700
            $results.Add((Check 'Enter on that row selects that tab' $chips.Count (Selected-Tab)))
            Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 800
            $results.Add((Check 'closing it leaves the two tabs' 2 (@(Get-Chips).Count)))
            $targets = 2, 5, 12, 30
            $widths = 800, 988, 1200, 1920
            $count = 2
            foreach ($n in $targets) {
                while ($count -lt $n) { New-Tab; $count++; if ($count -in 8, 20) { Name-Tab "claude $count" } }
                foreach ($w in $widths) {
                    Set-WindowWidth $w
                    Measure-Strip $n $w | ForEach-Object { if ($_ -like 'ok *' -or $_ -like 'FAIL*') { $results.Add($_) } else { $_ } }
                }
            }
            # the selected chip scrolls into view within 200 ms: first tab, then the last, then a middle one
            Set-WindowWidth 988
            foreach ($target in 1, 30, 14) {
                $mark = Log-Mark
                Open-TabList
                # Up and Down move the selection of the list (clamped at the ends): row 1, the last row, or row 14.
                $ups = if ($target -eq 30) { 0 } else { 40 }
                $downs = switch ($target) { 1 { 0 } 30 { 40 } default { $target - 1 } }
                foreach ($i in 1..$ups) { if ($ups) { Press 0x26 @() 20 20 } }
                foreach ($i in 1..$downs) { if ($downs) { Press 0x28 @() 20 20 } }
                Start-Sleep -Milliseconds 300
                Press 0x0D; Start-Sleep -Milliseconds 700
                $log = @(Log-Since $mark | Where-Object { $_ -match 'selected tab|strip scrolled|tab list' } | ForEach-Object { $_.Trim() -replace '\s+ms\s+', 'ms ' })
                'list -> tab {0}: {1}' -f $target, ($log -join ' | ')
                $results.Add((Check "list: row $target selects tab $target" $target (Selected-Tab)))
                $picked = @(Log-Since $mark | Where-Object { $_ -match 'selected tab' })[-1]
                $scrolled = @(Log-Since $mark | Where-Object { $_ -match 'strip scrolled' })
                if ($scrolled.Count -gt 0) {
                    $delay = (Log-Ms $scrolled[-1]) - (Log-Ms $picked)
                    $results.Add((Check-True "list: the strip scrolls to tab $target within 200 ms of the choice" ($delay -le 200) "$delay ms"))
                }
                Measure-Strip 30 988 | ForEach-Object { if ($_ -like 'ok *' -or $_ -like 'FAIL*') { $results.Add($_) } else { $_ } }
            }
            # a strip the user scrolled away stays where it is when a program reports a folder (and so renames the tab)
            $scrollEl = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd).FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'ScrollViewer')))
            $box = $scrollEl.Current.BoundingRectangle
            $placed = Strip-Offset
            Move-To ([int]($box.X + $box.Width / 2)) ([int]($box.Y + $box.Height / 2)); Start-Sleep -Milliseconds 300
            Mouse-Wheel 8; Start-Sleep -Milliseconds 800
            $scrolled = Strip-Offset
            Type-Line 'Set-Location C:\Windows'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 1200
            $kept = Strip-Offset
            'strip offset: {0:N0} after the list, {1:N0} after the wheel, {2:N0} after a folder report' -f $placed, $scrolled, $kept
            $results.Add((Check-True 'the wheel scrolled the strip away from the selected tab' ($scrolled -lt $placed - 100) "$placed -> $scrolled"))
            $results.Add((Check-True 'and a folder report that renames the tab does not snap it back' ([Math]::Abs($kept - $scrolled) -lt 2) "$scrolled -> $kept"))
            # filtering the list of 30 tabs, the time from the first key to the rows            $mark = Log-Mark
            Open-TabList
            $titles = @(Get-Chips | Sort-Object Index | ForEach-Object { Chip-Title $_ })
            $results.Add((Check 'the list of 30 has 30 rows' 30 (@(Get-Items).Count)))
            Type-Unicode 'cla' 60; Start-Sleep -Milliseconds 700
            $rows = @(Get-Items)
            $expected = @($titles | Where-Object { $_ -match '(?i)c.*l.*a' })
            'list of 30 after typing cla: {0} rows ({1})' -f $rows.Count, (($rows | ForEach-Object { $_.Current.Name }) -join ' / ')
            $results.Add((Check 'typing cla keeps the rows whose tab matches' $expected.Count $rows.Count))
            $times = @(Log-Since $mark | Where-Object { $_ -match 'tab list: \d+ rows in [\d.]+ ms' } | ForEach-Object { [double]([regex]::Match($_, 'rows in ([\d.]+) ms').Groups[1].Value) })
            'tab list timings: ' + ($times -join ', ') + ' ms'
            $results.Add((Check-True 'the list builds in under 100 ms (30 rows, then the filtered ones)' ($times.Count -ge 2 -and ($times | Measure-Object -Maximum).Maximum -lt 100) "$($times -join ', ') ms"))
            Press 0x1B; Start-Sleep -Milliseconds 400
            # the palette gets one go-to entry per tab
            Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 700
            Type-Unicode (Nk-Text 'palette.gotoTab' @('claude')) 40; Start-Sleep -Milliseconds 600
            $goto = @($titles | Where-Object { $_ -match '(?i)c.*l.*a.*u.*d.*e' })
            $results.Add((Check "the palette has an entry for each of the tabs called claude" $goto.Count (@(Get-Items).Count)))
            Press 0x1B; Start-Sleep -Milliseconds 400
            Shot-Safe (Shot-Path 'tabs-30') | Out-Null
            Assert-Results $results 'tabs'
        } -NoShot
    } finally {
        [IO.Directory]::Delete($global:NkGuidDir, $true)
    }
}

# Status marks: what a tab shows for OSC 7501, OSC 9 / 777 and BEL, read from the chips' accessible status (the
# state in words, so it is not colour alone), then the bell in three places: the pane in view (nothing), a hidden tab
# (a mark), an inactive window (a mark and a taskbar flash, at most one a second). Nothing here is audible.
function Scenario-Status {
    Run-App 'status' @{ confirmClose = $false } {
        First-Prompt
        Name-Tab 'aaa'
        Set-WindowWidth 1400
        $results = [System.Collections.Generic.List[string]]::new()
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
        # The state of every tab in words ('' for a tab at rest), by title.
        $states = { $s = [ordered]@{}; foreach ($c in Get-Chips | Sort-Object Index) { $s[(Chip-Title $c)] = $c.Status }; $s }
        $show = { param($label) $s = & $states; foreach ($title in $s.Keys) { '{0} {1,-14} status "{2}"' -f $label, $title, $s[$title] } }
        $expect = {
            param($label, $wanted)
            $s = & $states
            foreach ($title in $wanted.Keys) { $results.Add((Check "$label`: $title" $wanted[$title] $s[$title])) }
        }
        $blocked = Nk-Text 'agent.blocked'; $working = Nk-Text 'agent.working'; $done = Nk-Text 'agent.done'; $failed = Nk-Text 'agent.error'
        $bell = Nk-Text 'agent.bell'; $note = Nk-Text 'agent.notification'
        & $show 'hidden tabs:'
        $results.Add((Check 'there are eight tabs' 8 @(Get-Chips).Count))
        & $expect 'hidden tabs' ([ordered]@{ 'aaa' = ''; 'blocked-tab' = $blocked; 'working-tab' = $working; 'done-tab' = $done; 'error-tab' = $failed; 'bell-tab' = $bell; 'note-tab' = $note; 'last' = '' })
        Shot-Safe (Shot-Path 'status-marks') | Out-Null
        # a blocked agent is still blocked when the tab is looked at; a finished one, a failed one, a bell and a note are seen at once
        foreach ($look in @(@('blocked-tab', $blocked), @('done-tab', $done), @('error-tab', $failed), @('note-tab', $note))) {
            Click-Name (Nk-Text 'tab.nameStatus' @($look[0], $look[1])); Start-Sleep -Milliseconds 500
        }
        & $show 'after looking at blocked, done, error, note:'
        & $expect 'after looking' ([ordered]@{ 'blocked-tab' = $blocked; 'working-tab' = $working; 'done-tab' = ''; 'error-tab' = ''; 'bell-tab' = $bell; 'note-tab' = '' })
        # working ends at the next prompt: the report is gone once the command returns
        Click-Name 'last' | Out-Null; Start-Sleep -Milliseconds 500
        Type-Line ($esc + (& $report 'state=working:id=x') + 'Start-Sleep 4'); Start-Sleep -Milliseconds 1500
        $during = (Get-Chips | Where-Object { $_.Index -eq 8 }).Status
        Start-Sleep -Milliseconds 5000
        $after = (Get-Chips | Where-Object { $_.Index -eq 8 }).Status
        'working in the pane in view: while the command runs "{0}", after the next prompt "{1}"' -f $during, $after
        $results.Add((Check 'working shows while the command runs' $working $during))
        $results.Add((Check 'and is gone at the next prompt' '' $after))
        # a program that reported progress and was interrupted (Ctrl+C) leaves no spinner behind: the prompt clears it
        Type-Line ($esc + 'Write-Host -NoNewline "$e]9;4;1;40$a"; Start-Sleep 60'); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'a progress report shows the tab as working' $working ((Get-Chips | Where-Object { $_.Index -eq 8 }).Status)))
        Chord @(0x11) 0x43; Start-Sleep -Milliseconds 2500
        $results.Add((Check 'and Ctrl+C, with the prompt back, takes it away' '' ((Get-Chips | Where-Object { $_.Index -eq 8 }).Status)))
        # the bell in the pane in view, then in an inactive window twice within a second
        $mark = Log-Mark
        Click-Name 'last' | Out-Null; Start-Sleep -Milliseconds 500
        Type-Line ($esc + 'Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 1200
        Type-Line ($esc + 'Start-Sleep 3; Write-Host -NoNewline $a; Start-Sleep -Milliseconds 300; Write-Host -NoNewline $a; Start-Sleep 3; Write-Host -NoNewline $a')
        if (-not (Move-PatternAway)) { throw 'no corner of the screen is free of the app window' }
        [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 9000
        $lines = @(Log-Since $mark)
        $decisions = @($lines | Where-Object { $_ -match 'bell in pane' } | ForEach-Object {
            $m = [regex]::Match($_, 'chip: (true|false), taskbar_flash: (true|false), sound: (true|false)')
            [pscustomobject]@{ Chip = $m.Groups[1].Value; Flash = $m.Groups[2].Value; Sound = $m.Groups[3].Value }
        })
        'bell decisions (watching, then inactive: first, second within a second, a later one):'
        $lines | Where-Object { $_ -match 'bell in pane|taskbar flash|window active' } | ForEach-Object { 'log: ' + $_.Trim() }
        $results.Add((Check 'four bells were decided' 4 $decisions.Count))
        if ($decisions.Count -eq 4) {
            $got = ($decisions | ForEach-Object { "chip=$($_.Chip) flash=$($_.Flash) sound=$($_.Sound)" })
            $results.Add((Check 'a bell in the pane in view of the active window does nothing' 'chip=false flash=false sound=false' $got[0]))
            $results.Add((Check 'in an inactive window the first marks the tab and flashes the taskbar' 'chip=true flash=true sound=false' $got[1]))
            $results.Add((Check 'the second, within a second, flashes nothing' 'chip=true flash=false sound=false' $got[2]))
            $results.Add((Check 'a later one flashes again' 'chip=true flash=true sound=false' $got[3]))
        }
        $results.Add((Check-True 'the window was reported inactive' (@($lines | Where-Object { $_ -match 'window active = false' }).Count -ge 1)))
        $results.Add((Check 'no taskbar flash failed' 0 (@($lines | Where-Object { $_ -match 'taskbar flash:' }).Count)))
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 600
        # the title an agent reported names the tab for as long as the agent runs, also after its finished report was seen
        $titleOf = { Chip-Title (Get-Chips | Where-Object { $_.Index -eq 8 }) }
        $plain = & $titleOf
        $agentTitle = 'Fix the build'
        $encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($agentTitle))
        Type-Line ($esc + (& $report "state=done:id=x:app=claude:title=$encoded") + 'Start-Sleep 60'); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'an agent title names the tab' $agentTitle (& $titleOf)))
        $results.Add((Check 'and the finished report, seen at once, leaves the name as it is' '' ((Get-Chips | Where-Object { $_.Index -eq 8 }).Status)))
        Chord @(0x11) 0x43; Start-Sleep -Milliseconds 2500
        $results.Add((Check 'at the prompt, with the agent gone, the tab is named by the rules again' $plain (& $titleOf)))
        'ended in {0:N0} s' -f ((Get-Date) - $start).TotalSeconds
        Assert-Results $results 'status'
    } -Pattern -NoShot
}

# The texts of the position plate one by one ('3 / 12', then '!2' while some tab waits), or nothing when it is hidden.
function Plate-Parts {
    $plate = Get-Element (Nk-Text 'tab.all') 'Button'
    if (-not $plate) { return @() }
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    @($plate.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name })
}

# The title of the window of the app (what the taskbar button and Alt+Tab show).
function Window-Title { $text = New-Object System.Text.StringBuilder 256; [void][D]::GetWindowText($script:Hwnd, $text, 256); $text.ToString() }

# Chords with Ctrl+Shift: the jump to the next tab that needs a response.
function Jump-To-Response { Chord @(0x11, 0x10) 0x55; Start-Sleep -Milliseconds 500 }

# Unread responses, the jump and the alerts of an inactive window: twelve tabs, three of which report `blocked` six seconds
# after they were left. The position plate says `!3`, the list puts the three first, viewing one takes it off the count,
# Ctrl+Shift+U goes round the tabs that wait in the order of the strip and wraps, a response that begins in the pane being
# looked at is not unread, and a response that begins while the window is not active is announced, flashes the taskbar
# button and puts the count in the title of the window. Only two alerts are outstanding at a time.
function Scenario-Unread {
    Run-App 'unread' @{ confirmClose = $false } {
        First-Prompt
        Set-WindowWidth 1400
        $results = [System.Collections.Generic.List[string]]::new()
        $esc = '$e=[char]27;$a=[char]7;'
        $report = { param($fields) $esc + 'Write-Host -NoNewline "$e]7501;' + $fields + '$a";' }
        $blocked = Nk-Text 'agent.blocked'
        $waits = { param($n) Type-Line ($esc + 'Start-Sleep 6;' + (& $report "state=blocked:kind=permission:id=t${n}:app=claude:msg=QWxsb3cgZWRpdD8=") + 'Start-Sleep 300') }
        Name-Tab 't1'
        foreach ($n in 2..12) {
            New-Tab; Name-Tab "t$n"
            if ($n -in 3, 6, 9) { & $waits $n; Start-Sleep -Milliseconds 300 }
        }
        Start-Sleep -Milliseconds 8000
        $titles = { @(Get-Chips | Sort-Object Index | ForEach-Object { Chip-Title $_ }) }
        $alerts = { @(Get-Content -LiteralPath $script:LogFile | Where-Object { $_ -match 'alert tab' }) }
        $results.Add((Check 'twelve tabs, the twelfth shown' '12 / 12' (Plate-Parts)[0]))
        $results.Add((Check 'three of them wait in hidden tabs: the plate says !3 after the position' '12 / 12|!3' ((Plate-Parts) -join '|')))
        $results.Add((Check 'two alerts were raised and the third tab waits without one' 2 (& $alerts).Count))
        $results.Add((Check 'the active window keeps its title' 'Nekominal' (Window-Title)))
        Shot-Safe (Shot-Path 'unread-plate') | Out-Null
        $results.Add((Check-True 'the alerts were announced and the taskbar did not flash (the window is active)' (@(& $alerts | Where-Object { $_ -match 'announced true, taskbar flash false' }).Count -eq 2) ((& $alerts) -join ' / ')))
        # the list: the three that wait come first, in the order of the strip, and the list opens on the first
        Open-TabList
        Shot-Safe (Shot-Path 'unread-list') | Out-Null
        $rows = @(Get-Items | ForEach-Object { $_.Current.Name })
        $wanted = 't3', 't6', 't9' | ForEach-Object { Nk-Text 'tab.nameStatus' @($_, $blocked) }
        $results.Add((Check 'the list has a row for each tab' 12 $rows.Count))
        $results.Add((Check 'the first rows are the tabs that wait, in the order of the strip' ($wanted -join '|') (($rows | Select-Object -First 3) -join '|')))
        $results.Add((Check 'the others follow in the order of the strip' 't1 t2 t4' (($rows | Select-Object -Skip 3 -First 3) -join ' ')))
        Press 0x0D; Start-Sleep -Milliseconds 700
        $results.Add((Check 'Enter on the list opened on the first row selects the first tab that waits' 3 (Selected-Tab)))
        $results.Add((Check 'viewing it takes it off the count' '3 / 12|!2' ((Plate-Parts) -join '|')))
        # the jump goes to the next tab that waits, round the strip
        Jump-To-Response
        $results.Add((Check 'Ctrl+Shift+U goes to the next tab that waits' 6 (Selected-Tab)))
        $results.Add((Check 'and that one leaves the count' '6 / 12|!1' ((Plate-Parts) -join '|')))
        Jump-To-Response
        $results.Add((Check 'then the next' 9 (Selected-Tab)))
        $results.Add((Check 'with all three seen there is no count' '9 / 12' ((Plate-Parts) -join '|')))
        Jump-To-Response
        $results.Add((Check 'after the last it wraps to the first, which still waits' 3 (Selected-Tab)))
        $results.Add((Check 'what was seen does not become unread when it is left' '3 / 12' ((Plate-Parts) -join '|')))
        # a response that begins in the pane being looked at is not unread and raises no alert
        $before = (& $alerts).Count
        Click-Name 't1'; Start-Sleep -Milliseconds 400
        Type-Line ($esc + (& $report 'state=blocked:id=here') + 'Start-Sleep 300'); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'a tab that waits in front of the user adds nothing to the count' '1 / 12' ((Plate-Parts) -join '|')))
        $results.Add((Check 'and raises no alert' $before (& $alerts).Count))
        $results.Add((Check 'the chip says it waits all the same' $blocked ((Get-Chips | Where-Object { $_.Index -eq 1 }).Status)))
        # a response that begins while the window is not active: announced, flashed, counted in the title
        Click-Name 't12'; Start-Sleep -Milliseconds 400
        Type-Line ($esc + 'Start-Sleep 6;' + (& $report 'state=blocked:kind=question:id=late:app=claude') + 'Start-Sleep 300'); Start-Sleep -Milliseconds 400
        Click-Name 't2'; Start-Sleep -Milliseconds 400
        if (-not (Move-PatternAway)) { throw 'no corner of the screen is free of the app window' }
        [D]::ForceForeground($script:PatternHwnd); Start-Sleep -Milliseconds 8500
        $inactive = @(& $alerts | Select-Object -Skip $before)
        $results.Add((Check 'one more alert, announced, and the taskbar button flashed' 1 $inactive.Count))
        $results.Add((Check-True 'it flashed the taskbar button' ($inactive.Count -eq 1 -and $inactive[0] -match 'announced true, taskbar flash true') ($inactive -join ' / ')))
        $results.Add((Check 'the title of the inactive window counts the tabs that wait (t1, t3, t6, t9 and t12)' '!5 Nekominal' (Window-Title)))
        [void][D]::ShowWindow($script:Hwnd, 9); [D]::ForceForeground($script:Hwnd); Start-Sleep -Milliseconds 900
        $results.Add((Check 'back in the foreground the title is plain again' 'Nekominal' (Window-Title)))
        $results.Add((Check 'and the response that came meanwhile is on the plate' '2 / 12|!1' ((Plate-Parts) -join '|')))
        # a narrow window keeps the count and drops the position
        Set-WindowWidth 700
        $results.Add((Check 'in the narrow bar the plate has the total and the count' '12|!1' ((Plate-Parts) -join '|')))
        Set-WindowWidth 1400
        Assert-Results $results 'unread'
    } -Pattern -NoShot
}

# The controls on the right of the bar that are there now, by name (the split pair, the palette and the menu button).
function Bar-Right {
    $names = (Nk-Tip 'pane.splitRight' 'Alt+Shift+D'), (Nk-Tip 'pane.splitDown' 'Alt+Shift+S'), (Nk-Tip 'app.palette' 'Ctrl+Shift+P'), (Nk-Text 'menu.app')
    @($names | Where-Object { $e = Get-Element $_ 'Button'; $e -and -not $e.Current.IsOffscreen })
}

# The rows of the menu in order as UI Automation shows them: name, and whether it is enabled and checked (toggle rows).
function Menu-Rows {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $rows = foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
        if ($e.Current.ClassName -in 'MenuFlyoutItem', 'ToggleMenuFlyoutItem', 'MenuFlyoutSubItem') {
            $on = $null; $p = $null
            if ($e.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) { $on = ($p.Current.ToggleState -eq 'On') }
            [pscustomobject]@{ Name = $e.Current.Name; Enabled = $e.Current.IsEnabled; Checked = $on; Y = $e.Current.BoundingRectangle.Y; X = $e.Current.BoundingRectangle.X }
        }
    }
    @($rows | Sort-Object Y)
}

# Opens the menu of the "..." button and returns its rows.
function Open-Menu { Click-Name (Nk-Text 'menu.app') -Class 'Button'; Start-Sleep -Milliseconds 700; Menu-Rows }

# Toolbar tiers and the menu: the right-hand group shows 4, 2, 1 and 1 controls at 1440, 1000, 700 and 500 DIP, holds its
# tier for 24 DIP while the window shrinks, and nothing clips at 500 DIP while the drag area keeps its 48 DIP. The menu
# holds the same rows in the same order at every width, with the shortcuts of the live keymap, and each row works. The
# tab menu splits the pane of its own tab.
function Scenario-Toolbar {
    Run-App 'toolbar' @{ confirmClose = $false; keybindings = @{ 'edit.find' = 'Ctrl+Shift+Y' } } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        New-Tab; New-Tab
        $scale = Get-Scale
        $split = Nk-Tip 'pane.splitRight' 'Alt+Shift+D'; $down = Nk-Tip 'pane.splitDown' 'Alt+Shift+S'; $palette = Nk-Tip 'app.palette' 'Ctrl+Shift+P'; $menu = Nk-Text 'menu.app'
        $shown = [ordered]@{ '1440' = @($split, $down, $palette, $menu); '1000' = @($palette, $menu); '700' = @($menu); '500' = @($menu) }
        $expected = @(
            (Nk-Tip 'tab.new' 'Ctrl+Shift+T'), (Nk-Tip 'pane.splitRight' 'Alt+Shift+D'), (Nk-Tip 'pane.splitDown' 'Alt+Shift+S'),
            (Nk-Tip 'pane.zoom' 'Ctrl+Shift+Z'), (Nk-Tip 'pane.close' 'Ctrl+Shift+W'), (Nk-Tip 'edit.find' 'Ctrl+Shift+Y'),
            (Nk-Tip 'app.palette' 'Ctrl+Shift+P'), (Nk-Text 'settings.backdrop'), (Nk-Text 'menu.opacity'),
            (Nk-Tip 'view.fullscreen' 'F11'), (Nk-Tip 'app.settings' 'Ctrl+,')
        )
        # the xaml numbers of the cluster: margin 8 and padding 2 on its left
        $xaml = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\..\crates\nekominal-app\src\xaml\shell.xaml'))
        $clusterMargin = [double]([regex]::Match($xaml, 'x:Name="ToolCluster"[^>]*Padding="(\d+)" Margin="(\d+),').Groups[2].Value)
        $clusterPadding = [double]([regex]::Match($xaml, 'x:Name="ToolCluster"[^>]*Padding="(\d+)" Margin="(\d+),').Groups[1].Value)
        foreach ($key in $shown.Keys) {
            $w = [int]$key
            Set-WindowWidth $w
            $win = Win-Rect
            $bar = @(Bar-Right)
            '{0} DIP: right group {1}' -f $w, ($bar -join ' | ')
            $results.Add((Check "$w DIP shows $($shown[$key].Count) controls on the right" ($shown[$key] -join '|') ($bar -join '|')))
            $rows = @(Open-Menu)
            $results.Add((Check "$w DIP: the menu holds the same rows in the same order" ($expected -join '|') (($rows | ForEach-Object { $_.Name }) -join '|')))
            Press 0x1B; Start-Sleep -Milliseconds 400
            # nothing sticks out of the window, the controls do not overlap the caption buttons, the drag area keeps 48 DIP
            $edge = $win.Right
            $captions = @(([System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)).FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, 'Minimize'))) | Where-Object { -not $_.Current.IsOffscreen })
            $captionLeft = if ($captions.Count) { ($captions | ForEach-Object { $_.Current.BoundingRectangle.X } | Measure-Object -Minimum).Minimum } else { $edge }
            $menuButton = (Get-Element $menu 'Button').Current.BoundingRectangle
            $results.Add((Check-True "$w DIP: the menu button ends left of the caption buttons" ($menuButton.X + $menuButton.Width -le $captionLeft) "button ends at $($menuButton.X + $menuButton.Width), captions start at $captionLeft"))
            $chips = @(Get-Chips | Where-Object { -not [double]::IsInfinity($_.Rect.Width) })
            $plate = (Get-Element (Nk-Text 'tab.all') 'Button').Current.BoundingRectangle
            $clusterLeft = (@($bar | ForEach-Object { (Get-Element $_ 'Button').Current.BoundingRectangle.X } | Measure-Object -Minimum).Minimum) - $clusterPadding * $scale
            $dragDip = ($clusterLeft - $clusterMargin * $scale - ($plate.X + $plate.Width)) / $scale
            '   drag area {0:N1} DIP, menu button ends {1:N1} DIP left of the caption buttons' -f $dragDip, (($captionLeft - $menuButton.X - $menuButton.Width) / $scale)
            $results.Add((Check-True "$w DIP: the drag area keeps 48 DIP" ($dragDip -ge 47.5) ('{0:N1} DIP' -f $dragDip)))
            $results.Add((Check-True "$w DIP: the plate is whole and left of the controls" ($plate.X + $plate.Width -le $clusterLeft -and $plate.X -ge $win.Left) "plate $($plate.X)..$($plate.X + $plate.Width), group from $clusterLeft"))
            Shot-Safe (Shot-Path "toolbar-$w") | Out-Null
        }
        # a window that shrinks keeps its tier for 24 DIP, one that grows does not wait: the same width has the tier of its direction
        Set-WindowWidth 1440; Set-WindowWidth 1190
        $results.Add((Check '1190 DIP, shrunk from 1440, still has the wide tier' 4 @(Bar-Right).Count))
        Set-WindowWidth 1100; Set-WindowWidth 1190
        $results.Add((Check '1190 DIP, grown from 1100, has the medium tier' 2 @(Bar-Right).Count))
        Set-WindowWidth 1000; Set-WindowWidth 790
        $results.Add((Check '790 DIP, shrunk from 1000, still has the medium tier' 2 @(Bar-Right).Count))
        Set-WindowWidth 700; Set-WindowWidth 790
        $results.Add((Check '790 DIP, grown from 700, has the narrow tier' 1 @(Bar-Right).Count))
        # what each row does, at the wide tier
        Set-WindowWidth 1440
        $rows = Open-Menu
        $zoom = $rows | Where-Object { $_.Name -eq (Nk-Tip 'pane.zoom' 'Ctrl+Shift+Z') }
        $results.Add((Check 'with one pane the zoom row is off' $false $zoom.Enabled))
        $opacityRow = $rows | Where-Object { $_.Name -eq (Nk-Text 'menu.opacity') }
        Press 0x1B; Start-Sleep -Milliseconds 400
        Split-Right
        $zoomName = Nk-Tip 'pane.zoom' 'Ctrl+Shift+Z'
        $rows = Open-Menu; $zoom = $rows | Where-Object { $_.Name -eq $zoomName }
        $results.Add((Check 'with two panes it is on and unchecked' 'True False' ('{0} {1}' -f $zoom.Enabled, $zoom.Checked)))
        Click-Name $zoomName 'left' 900
        $rows = Open-Menu; $zoom = $rows | Where-Object { $_.Name -eq $zoomName }
        $results.Add((Check 'after zooming it is checked and one pane shows' 'True 1' ('{0} {1}' -f $zoom.Checked, (Count-Panes))))
        Click-Name $zoomName 'left' 900
        $results.Add((Check 'choosing it again brings the panes back' 2 (Count-Panes)))
        # background material: a radio group, the current one marked; a choice changes the backdrop
        Open-Menu | Out-Null
        Click-Name (Nk-Text 'settings.backdrop'); Start-Sleep -Milliseconds 700
        $current = @(Menu-Rows | Where-Object { $_.Name -in (Nk-Text 'backdrop.acrylic'), (Nk-Text 'backdrop.mica'), (Nk-Text 'backdrop.micaAlt'), (Nk-Text 'backdrop.none') })
        $results.Add((Check 'the submenu lists the four backdrops and only Acrylic is marked' (((Nk-Text 'backdrop.acrylic') + '|True|' + (Nk-Text 'backdrop.mica') + '|False|' + (Nk-Text 'backdrop.micaAlt') + '|False|' + (Nk-Text 'backdrop.none') + '|False')) (($current | Sort-Object Y | ForEach-Object { "$($_.Name)|$($_.Checked)" }) -join '|')))
        $mark = Log-Mark
        Click-Name (Nk-Text 'backdrop.mica') 'left' 1000
        $results.Add((Check 'choosing Mica sets the system backdrop once' 1 (@(Log-Since $mark | Where-Object { $_ -match 'system backdrop set: Mica$' }).Count)))
        Open-Menu | Out-Null
        Click-Name (Nk-Text 'settings.backdrop'); Start-Sleep -Milliseconds 700
        $current = @(Menu-Rows | Where-Object { $_.Name -in (Nk-Text 'backdrop.acrylic'), (Nk-Text 'backdrop.mica') })
        $results.Add((Check 'the menu then marks Mica' 'False True' (($current | Sort-Object Y | ForEach-Object { $_.Checked }) -join ' ')))
        Press 0x1B; Press 0x1B; Start-Sleep -Milliseconds 500
        # the opacity row opens the appearance flyout under the menu button
        Open-AppearanceFlyout
        $slider = Get-Slider 0
        $button = (Get-Element $menu 'Button').Current.BoundingRectangle
        $box = $slider.Current.BoundingRectangle
        $results.Add((Check-True 'the opacity row opens the flyout under the menu button, its right edge on the button' ($box.Y -gt $button.Y + $button.Height -and [Math]::Abs(($box.X + $box.Width) - ($button.X + $button.Width)) -lt 40 * $scale) "slider $($box.X)..$($box.X + $box.Width) at $($box.Y), button $($button.X)..$($button.X + $button.Width)"))
        Press 0x1B; Start-Sleep -Milliseconds 500
        # find and settings, which have no button any more, are two steps away
        Open-Menu | Out-Null; Click-Name (Nk-Tip 'edit.find' 'Ctrl+Shift+Y') 'left' 900
        $results.Add((Check 'find, from the menu, opens the find bar' $true ([bool](Get-Element (Nk-Text 'edit.find') 'TextBox'))))
        Press 0x1B; Start-Sleep -Milliseconds 500
        Open-Menu | Out-Null; Click-Name (Nk-Tip 'app.settings' 'Ctrl+,') 'left' 1200
        $results.Add((Check 'settings, from the menu, opens the settings drawer' $true (Drawer-Open)))
        Press 0x1B; Start-Sleep -Milliseconds 800
        # the tab menu splits the pane of its own tab, shown or not
        $chips = @(Get-Chips | Sort-Object Index)
        $first = Get-ElementRect $chips[0].Name
        Click-At ($first.X + 40) ($first.Y + 16) 'right' 900
        $results.Add((Check 'the menu of a tab has both split rows' $true ([bool]((Get-Element (Nk-Text 'pane.splitRight')) -and (Get-Element (Nk-Text 'pane.splitDown'))))))
        $before = Count-Panes
        Click-Name (Nk-Text 'pane.splitDown') 'left' 1200; Wait-Prompt | Out-Null
        $results.Add((Check 'splitting from the menu of the first tab shows that tab with one more pane' '1 2' ('{0} {1}' -f (Selected-Tab), (Count-Panes))))
        Assert-Results $results 'toolbar'
    } -NoShot
}
# Dragging an unselected chip of a compressed strip: the press that starts the drag selects the chip, which makes it wider,
# and the drag has to count with the widths of that layout. Twelve tabs at 988 DIP, the chip of tab 7 dragged 110 DIP to
# the right passes the middle of its right neighbour (78 DIP away once it is selected) and not that of the next (130).
function Scenario-TabDrag {
    Run-App 'tab-drag' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        Name-Tab 't1'
        foreach ($n in 2..12) { New-Tab; Name-Tab "t$n" }
        Set-WindowWidth 988
        $order = { @(Get-Chips | Sort-Object Index | ForEach-Object { Chip-Title $_ }) }
        $before = & $order
        'before: ' + ($before -join ' ')
        $chip = (Get-Chips | Where-Object { (Chip-Title $_) -eq 't7' })
        $r = $chip.Rect
        $scale = Get-Scale
        $x = [int]($r.X + 8 * $scale); $y = [int]($r.Y + $r.Height / 2)
        Drag-To $x $y ([int]($x + 110 * $scale)) $y 20 25
        Start-Sleep -Milliseconds 800
        $after = & $order
        'after:  ' + ($after -join ' ')
        $expected = @($before | Where-Object { $_ -ne 't7' }); $at = $expected.IndexOf('t8') + 1
        $expected = @($expected[0..($at - 1)]) + 't7' + @($expected[$at..($expected.Count - 1)])
        $results.Add((Check 'dragging the chip of tab 7 110 DIP to the right moves it past tab 8 only' ($expected -join ' ') ($after -join ' ')))
        $results.Add((Check 'and it stays the tab shown, now eighth' '8 / 12' (Plate-Text)))
        Assert-Results $results 'tab drag'
    } -NoShot
}
# A program that reported progress and then ended with an error shows an error, not the work it was doing: the exit
# clears the progress and a failed exit outranks it. The program is a profile of its own (it keeps its last screen).
function Scenario-StatusExit {
    $command = '[Console]::Out.Write([string][char]27 + "]9;4;1;40" + [char]7); Start-Sleep 9; exit 3'
    $broken = @{ id = 'broken'; name = 'broken'; program = 'pwsh.exe'; args = @('-NoProfile', '-Command', $command) }
    Run-App 'status-exit' @{ defaultProfile = 'broken'; profiles = @($broken) } {
        $status = { (Get-Chips | Select-Object -First 1).Status }
        $during = $null
        for ($i = 0; $i -lt 20 -and $during -ne (Nk-Text 'agent.working'); $i++) { Start-Sleep -Milliseconds 250; $during = & $status }
        # The program ends nine seconds after it started.
        for ($i = 0; $i -lt 60 -and (& $status) -eq (Nk-Text 'agent.working'); $i++) { Start-Sleep -Milliseconds 250 }
        Start-Sleep -Milliseconds 500
        Assert-Results @(
            (Check 'while the program reports progress the tab is working' (Nk-Text 'agent.working') $during),
            (Check 'once it ended with code 3 the tab shows an error' (Nk-Text 'agent.error') (& $status)),
            (Check-True 'and the pane says how it ended' ((Get-TerminalText) -like ('*' + (Nk-Text 'status.exited' @(3)) + '*')) (Get-TerminalText))
        ) 'status (exit)'
    } -NoShot
}

# Default tab titles: five tabs in different folders get five names, two folders called src get their parents, a title
# a program sets wins until it is generic again and a bidirectional override is cleaned out of titles and folder names.
function Scenario-Titles {
    $global:NkCjk = Join-Path ([IO.Path]::GetTempPath()) 'nk-title-測試資料夾'
    # A folder whose name holds U+202E (right-to-left override): the folder name must reach the chip without it.
    $global:NkRlo = Join-Path ([IO.Path]::GetTempPath()) "nk-title-evil$([char]0x202E)gnp.exe"
    [void][IO.Directory]::CreateDirectory($global:NkCjk)
    [void][IO.Directory]::CreateDirectory($global:NkRlo)
    try {
        Run-App 'titles' @{ confirmClose = $false } {
            First-Prompt
            # Ten tabs: wide enough that every chip is on screen to be clicked.
            Set-WindowWidth 1900
            $results = [System.Collections.Generic.List[string]]::new()
            $titles = { @(Get-Chips | Sort-Object Index | ForEach-Object { Chip-Title $_ }) }
            $names = { (& $titles) -join ' | ' }
            $go = { param($dir) New-Tab; Type-Line "Set-Location -LiteralPath '$dir'"; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 400 }
            'first tab, nothing typed yet: ' + (& $names)
            foreach ($dir in 'C:\Windows', 'C:\Windows\System32', 'C:\', 'C:\Users\Public') { & $go $dir }
            'five folders: ' + (& $names)
            $five = & $titles
            $results.Add((Check 'tabs in four folders are called by their last folder' 'Windows|System32|C:|Public' ($five[1..4] -join '|')))
            $results.Add((Check 'and the five tabs have five different names' 5 @($five | Select-Object -Unique).Count))
            & $go (Join-Path $global:NkRoot 'crates\nekominal-app\src')
            & $go (Join-Path $global:NkRoot 'crates\nekominal-model\src')
            'two src folders: ' + (& $names)
            $src = & $titles
            $results.Add((Check 'two folders called src are told apart by the folder above' 'nekominal-app/src|nekominal-model/src' ($src[5..6] -join '|')))
            & $go $global:NkCjk
            'a folder with a CJK name: ' + (& $names)
            $results.Add((Check 'a folder with a CJK name is shown whole' 'nk-title-測試資料夾' ((& $titles)[-1])))
            & $go $global:NkRlo
            'a folder with U+202E in its name: ' + (& $names)
            $results.Add((Check 'a folder name with U+202E reaches the chip without it' "nk-title-evilgnp.exe" ((& $titles)[-1])))
            # a title the program set wins over the folder, a generic one does not
            Type-Line '$Host.UI.RawUI.WindowTitle = "build"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title "build": ' + (& $names)
            $results.Add((Check 'a title the program set wins over the folder' 'build' ((& $titles)[-1])))
            Type-Line '$Host.UI.RawUI.WindowTitle = "pwsh.exe"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title "pwsh.exe" (generic): ' + (& $names)
            $results.Add((Check 'the name of the shell itself is no title: the folder shows again' 'nk-title-evilgnp.exe' ((& $titles)[-1])))
            Type-Line '$Host.UI.RawUI.WindowTitle = "pay$([char]0x202E)load"'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'program title with U+202E: ' + (& $names)
            $results.Add((Check 'a program title with U+202E is cleaned' 'payload' ((& $titles)[-1])))
            Type-Line '$Host.UI.RawUI.WindowTitle = "x" * 1000'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            $results.Add((Check 'a title of 1000 characters is cut to 120' ('x' * 120) ((& $titles)[-1])))
            # a name the user gives is kept, an empty name hands the title back to the rules
            $rename = { param($chipName, $text)
                $r = Get-ElementRect $chipName; Click-At ($r.X + 30) ($r.Y + 16) 'right' 800
                Click-Name (Nk-Text 'tab.rename') | Out-Null; Start-Sleep -Milliseconds 800
                if ($text) { Type-Unicode $text 40 } else { Press 0x08 }
                Press 0x0D; Start-Sleep -Milliseconds 800 }
            & $rename 'Windows' 'fixed'
            'renamed the tab of C:\Windows "fixed": ' + (& $names)
            $results.Add((Check 'a name the user gave is shown' 'fixed' ((& $titles)[1])))
            Click-Name 'fixed' | Out-Null; Start-Sleep -Milliseconds 400
            Type-Line 'Set-Location C:\Users'; Wait-Prompt | Out-Null; Start-Sleep -Milliseconds 500
            'after cd C:\Users in it: ' + (& $names)
            $results.Add((Check 'and kept when the folder changes' 'fixed' ((& $titles)[1])))
            & $rename 'fixed' ''
            'name cleared: ' + (& $names)
            $results.Add((Check 'an emptied name hands the title back to the folder' 'Users' ((& $titles)[1])))
            Assert-Results $results 'titles'
        }
    } finally {
        [IO.Directory]::Delete($global:NkCjk, $true)
        [IO.Directory]::Delete($global:NkRlo, $true)
    }
}

# Closing: a tab at its prompt closes without asking, a running command asks and the dialog names the tab, the
# middle button closes when it comes up (not when it goes down), a shell that exits cleanly closes its pane (also one a
# custom profile starts) and a program profile that exits cleanly keeps its last screen.
function Scenario-Close {
    # hostname.exe stands for an agent: a program that prints something and ends with code 0.
    $agent = @{ id = 'agent'; name = 'agent'; program = 'hostname.exe'; args = @() }
    Run-App 'close' @{ defaultProfile = 'agent'; profiles = @($agent) } {
        Start-Sleep -Milliseconds 3000
        $text = Get-TerminalText
        Assert-Results @(
            (Check 'a program profile that exited with 0 leaves the window open' $false $script:App.HasExited),
            (Check 'and its tab' 1 @(Get-Chips).Count),
            (Check-True 'and its last screen' ($text -match [regex]::Escape([Environment]::MachineName)) "text: $text"),
            (Check-True 'and the line that says it ended' ($text -like ('*' + (Nk-Text 'status.exited' @(0)) + '*')) "text: $text")
        ) 'close (program profile)'
    } -NoShot
    # A custom profile for a shell is a shell: typing exit closes its pane like it does for a built-in one.
    Run-App 'close-custom-shell' @{ profiles = @(@{ id = 'custom-pwsh'; name = 'custom-pwsh'; program = 'pwsh.exe'; args = @('-NoLogo') }) } {
        First-Prompt
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 700
        Type-Unicode (Nk-Text 'palette.newTabWith' @('custom-pwsh')) 40; Start-Sleep -Milliseconds 500; Press 0x0D
        Wait-Prompt | Out-Null
        $before = @(Get-Chips).Count
        Type-Line 'exit'; Start-Sleep -Milliseconds 1800
        Assert-Results @(
            (Check 'a second tab was opened with the custom shell profile' 2 $before),
            (Check 'typing exit in it closes its tab' 1 @(Get-Chips).Count),
            (Check 'and the window stays' $false $script:App.HasExited)
        ) 'close (custom shell)'
    } -NoShot
    Run-App 'close-shell' @{} {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        New-Tab
        Type-Line 'exit'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'typing exit in a shell tab closes it' 1 @(Get-Chips).Count))
        New-Tab
        $dialogs = { Dialog-Open }
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        $results.Add((Check 'closing a tab at its prompt asks nothing' 'False 1' ('{0} {1}' -f (& $dialogs), @(Get-Chips).Count)))
        New-Tab
        Type-Line '$Host.UI.RawUI.WindowTitle = "deploy"; Start-Sleep 60'; Start-Sleep -Milliseconds 1200
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1200
        $named = [bool](Get-Element (Nk-Text 'dialog.closeTab' @('deploy')))
        $results.Add((Check 'closing a tab with a running command asks, naming the tab' 'True True 2' ('{0} {1} {2}' -f (& $dialogs), $named, @(Get-Chips).Count)))
        Press 0x1B; Start-Sleep -Milliseconds 600
        $results.Add((Check 'cancelling keeps the tab' 2 @(Get-Chips).Count))
        # the middle button: down does nothing, up over the chip closes
        New-Tab
        $chips = @(Get-Chips); $r = Get-ElementRect $chips[-1].Name
        Move-To ($r.X + 30) ($r.Y + 16); Start-Sleep -Milliseconds 300
        [D]::Mouse(0x0020, 0); Start-Sleep -Milliseconds 800
        $afterDown = @(Get-Chips).Count
        [D]::Mouse(0x0040, 0); Start-Sleep -Milliseconds 1200
        $results.Add((Check 'the middle button closes when it comes up, not when it goes down' '3 2' ('{0} {1}' -f $afterDown, @(Get-Chips).Count)))
        Assert-Results $results 'close (shell)'
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
        $results += Check 'and asks about the running session' $true ([bool](Get-Element (Nk-Text 'dialog.closeSessions' @(1))))
        Press 0x1B; Start-Sleep -Milliseconds 800
        $results += Check 'Cancel keeps the window and the command' $false $script:App.HasExited
        [void]$script:App.CloseMainWindow(); Start-Sleep -Milliseconds 1200
        Click-Name (Nk-Text 'common.close') -Class 'Button'
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
    Run-App 'panes' @{ theme = 'Tokyo Night' } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $textCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
        $texts = {
            $e = Get-Element (Get-Chips | Select-Object -First 1).Name
            @($e.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCond) | ForEach-Object { $_.Current.Name })
        }
        '1 pane: chip texts "{0}", panes {1}' -f ((& $texts) -join '|'), (Count-Panes)
        $results.Add((Check 'a tab with one pane shows no count' 0 @((& $texts) | Where-Object { $_ -match '^\d+$' }).Count))
        Split-Right; Split-Down
        '3 panes: chip texts "{0}", panes {1}' -f ((& $texts) -join '|'), (Count-Panes)
        $results.Add((Check 'three panes show their count' 1 @((& $texts) | Where-Object { $_ -eq '3' }).Count))
        $results.Add((Check 'and are three terminals' 3 (Count-Panes)))
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 900
        $badge = Get-Element (Nk-Text 'tab.zoomed')
        $badgeText = if ($badge) { (@($badge.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCond) | ForEach-Object { $_.Current.Name })) -join '' } else { '' }
        'zoomed: corner label present = {0}, text "{1}", chip texts "{2}"' -f [bool]$badge, $badgeText, ((& $texts) -join '|')
        $results.Add((Check 'zooming shows the corner label' $true ([bool]$badge)))
        $results.Add((Check-True 'which says which pane of three' ($badgeText -match '^[123] / 3$') "text '$badgeText'"))
        $results.Add((Check 'and the chip keeps its count' 1 @((& $texts) | Where-Object { $_ -eq '3' }).Count))
        $results.Add((Check 'only the zoomed pane is a terminal on screen' 1 (Count-Panes)))
        Shot-Safe (Shot-Path 'panes-zoomed') | Out-Null
        $b = $badge.Current.BoundingRectangle
        Click-At ([int]($b.X + $b.Width / 2)) ([int]($b.Y + $b.Height / 2)) 'left' 900
        'pressed the label: corner label present = {0}, panes shown {1}' -f [bool](Get-Element (Nk-Text 'tab.zoomed')), (Count-Panes)
        $results.Add((Check 'pressing the label brings the other panes back' 'False 3' ('{0} {1}' -f [bool](Get-Element (Nk-Text 'tab.zoomed')), (Count-Panes))))
        # a bell in the pane that is not focused
        $esc = '$e=[char]27;$a=[char]7;'
        Type-Line ($esc + 'Start-Sleep 3; Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 300
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 5000
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
        $termCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'NekominalTerminal')
        $rects = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $termCond) | ForEach-Object { $_.Current.BoundingRectangle })
        $ringing = $rects | Sort-Object { $_.Y * 10000 + $_.X } | Select-Object -Last 1
        $win = Win-Rect
        # The first rows of the pane, left to right at 40 px from its edge: the warning line is the first 2 DIP.
        $sample = {
            param($rect, $path)
            Shot-Safe $path | Out-Null
            $bmp = [System.Drawing.Bitmap]::FromFile($path)
            try {
                $x = [int]($rect.X - $win.Left + 40); $y0 = [int]($rect.Y - $win.Top)
                @(0..4 | ForEach-Object { $c = $bmp.GetPixel($x, $y0 + $_); ,@([int]$c.R, [int]$c.G, [int]$c.B) })
            } finally { $bmp.Dispose() }
        }
        $show = { param($rows) ($rows | ForEach-Object { '({0},{1},{2})' -f $_[0], $_[1], $_[2] }) -join ' ' }
        $rang = & $sample $ringing (Shot-Path 'panes-warning')
        'top rows of the pane that rang (unfocused): ' + (& $show $rang)
        $results.Add((Check-True 'the pane that rang has a warning (amber) line on its top edge' ($rang[0][0] - $rang[0][2] -gt 40) (& $show $rang)))
        $results.Add((Check-True 'and the line is thin: the rows below are not amber' ($rang[4][0] - $rang[4][2] -lt 40) (& $show $rang)))
        Click-At ([int]($ringing.X + $ringing.Width / 2)) ([int]($ringing.Y + $ringing.Height / 2)) 'left' 900
        $focused = & $sample $ringing (Shot-Path 'panes-focused')
        'top rows of it once focused: ' + (& $show $focused)
        $results.Add((Check-True 'focusing it takes the warning line away and puts the focus (accent) line there' ($focused[0][2] - $focused[0][0] -gt 40) (& $show $focused)))
        # the same bell while a zoom hides the pane that rang: the corner label of the zoomed pane warns, with the glyph
        # and the word of the state, and goes back to its plain look when the pane has been seen
        Type-Line ($esc + 'Start-Sleep 4; Write-Host -NoNewline $a'); Start-Sleep -Milliseconds 300
        Chord @(0x12) 0x25; Start-Sleep -Milliseconds 400
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 6500
        $warned = Get-Element (Nk-Text 'tab.nameStatus' @((Nk-Text 'tab.zoomed'), (Nk-Text 'agent.bell')))
        $glyphs = if ($warned) { @($warned.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCond) | ForEach-Object { $_.Current.Name }) } else { @() }
        'zoomed, the hidden pane rang: label "{0}", texts "{1}"' -f [bool]$warned, ($glyphs -join '|')
        $results.Add((Check 'a pane hidden by the zoom that rang is named by the corner label with the word of its state' $true ([bool]$warned)))
        $results.Add((Check 'and the label shows the glyph of that state' 1 @($glyphs | Where-Object { $_ -eq [string][char]0xEA8F }).Count))
        Shot-Safe (Shot-Path 'panes-zoomed-warning') | Out-Null
        if ($warned) { $b = $warned.Current.BoundingRectangle; Click-At ([int]($b.X + $b.Width / 2)) ([int]($b.Y + $b.Height / 2)) 'left' 900 }
        $rects = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $termCond) | ForEach-Object { $_.Current.BoundingRectangle })
        $ringing = $rects | Sort-Object { $_.Y * 10000 + $_.X } | Select-Object -Last 1
        Click-At ([int]($ringing.X + $ringing.Width / 2)) ([int]($ringing.Y + $ringing.Height / 2)) 'left' 900
        Chord @(0x11, 0x10) 0x5A; Start-Sleep -Milliseconds 900
        $plain = [bool](Get-Element (Nk-Text 'tab.zoomed')); $still = [bool](Get-Element (Nk-Text 'tab.nameStatus' @((Nk-Text 'tab.zoomed'), (Nk-Text 'agent.bell'))))
        $results.Add((Check 'once the pane that rang has been looked at, the corner label is plain again' 'True False' ('{0} {1}' -f $plain, $still)))
        Assert-Results $results 'panes'
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
# The settings drawer, first half: it opens by the shortcut, the menu and the palette and closes by Esc, the close button and
# the shortcut; it floats over the terminal without resizing it (no new grid of the first pane in the log), leaves the tab
# strip and the terminals usable while it is open, gives the focus back to the pane it came from; Esc steps back one thing
# at a time (the search first); the search finds a row by its label in both languages; the sections are there.
function Scenario-Drawer {
    Run-App 'drawer' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $search = Nk-Text 'settings.search'
        $mark = Log-Mark
        Open-Drawer
        $results.Add((Check 'Ctrl+Shift+O opens the drawer' $true (Drawer-Open)))
        $results.Add((Check 'the focus is in its search box' $search (Focused-Name)))
        $sections = 'settings.appearance', 'settings.terminal', 'settings.profiles', 'settings.agents', 'settings.keys', 'settings.advanced'
        $results.Add((Check 'the six sections are there, in order' (($sections | ForEach-Object { Nk-Text $_ }) -join '|') ((@($sections | ForEach-Object { Get-Element (Nk-Text $_) 'ToggleButton' } | Where-Object { $_ } | Sort-Object { $_.Current.BoundingRectangle.X } | ForEach-Object { $_.Current.Name })) -join '|')))
        $results.Add((Check 'the first section is chosen' 'On' (Toggle-State (Nk-Text 'settings.appearance'))))
        Close-Drawer
        $results.Add((Check 'Esc closes it' $false (Drawer-Open)))
        $results.Add((Check 'and the focus is back in the terminal' 'NekominalTerminal' (Focused-Class)))
        Type-Line 'echo typed-after-esc'
        $results.Add((Check 'what is typed then reaches the terminal' $true (Wait-Text 'typed-after-esc' 6000)))
        $results.Add((Check 'opening and closing it did not give the pane another grid' 0 (Grid-Lines $mark).Count))

        # the menu and the close button
        Open-Menu | Out-Null; Click-Name (Nk-Tip 'app.settings' 'Ctrl+,') 'left' 800
        $results.Add((Check 'the menu row opens it' $true (Wait-Until { Drawer-Open })))
        Click-Name (Nk-Text 'common.close') -Class 'Button'; Start-Sleep -Milliseconds 500
        $results.Add((Check 'the close button closes it' $false (Drawer-Open)))
        $results.Add((Check 'and the focus is back in the terminal' 'NekominalTerminal' (Focused-Class)))

        # the palette lists a row of the table and opens the drawer there
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 700
        Type-Unicode (Nk-Text 'palette.setting' @((Nk-Text 'settings.opacity'))) 40; Start-Sleep -Milliseconds 500; Press 0x0D; Start-Sleep -Milliseconds 900
        $results.Add((Check 'the palette entry of a setting opens the drawer on that row' (Nk-Text 'settings.opacity') (Focused-Name)))
        Close-Drawer

        # the shortcut when it is open: in the drawer it closes, from the terminal it brings the focus to the drawer
        Open-Drawer; Chord @(0x11, 0x10) 0x4F; Start-Sleep -Milliseconds 700
        $results.Add((Check 'the shortcut again, with the focus in the drawer, closes it' $false (Drawer-Open)))
        Open-Drawer; Click-Terminal
        $results.Add((Check 'it stays open while the terminal has the focus' $true (Drawer-Open)))
        $results.Add((Check 'and the focus is in the terminal' 'NekominalTerminal' (Focused-Class)))
        Type-Line 'echo typed-while-open'
        $results.Add((Check 'typing reaches the terminal while the drawer is open' $true (Wait-Text 'typed-while-open' 6000)))
        Chord @(0x11, 0x10) 0x4F; Start-Sleep -Milliseconds 700
        $results.Add((Check 'the shortcut from the terminal puts the focus in the search box' $search (Focused-Name)))
        $results.Add((Check 'and the drawer stays' $true (Drawer-Open)))
        # the tab strip stays usable
        New-Tab
        $results.Add((Check 'a new tab opens while the drawer is open' 2 @(Get-Chips).Count))
        $first = (Get-Chips | Sort-Object Index | Select-Object -First 1)
        $r = Get-ElementRect $first.Name; Click-At ($r.X + 40) ($r.Y + 16) 'left' 700
        $results.Add((Check 'a chip can be clicked and selects its tab' 1 (Selected-Tab)))
        $results.Add((Check 'and the drawer is still open' $true (Drawer-Open)))
        Close-Drawer

        # Esc steps back one thing at a time
        Open-Drawer
        Type-Unicode 'zzqq' 40; Start-Sleep -Milliseconds 500
        $results.Add((Check 'a search without a match says so' $true ([bool](Get-Element (Nk-Text 'settings.empty')))))
        Press 0x1B; Start-Sleep -Milliseconds 500
        $value = (Drawer-Search).GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
        $results.Add((Check 'the first Esc clears the search and keeps the drawer' 'True|' ((Drawer-Open).ToString() + '|' + $value)))
        Close-Drawer
        $results.Add((Check 'the second Esc closes it' $false (Drawer-Open)))

        # the search finds a row by its label
        Open-Drawer
        Type-Unicode (Nk-Text 'settings.opacity').Substring(2) 40; Start-Sleep -Milliseconds 600
        $results.Add((Check 'the search finds the opacity slider by part of its label (zh-TW)' $true ([bool](Get-Slider 0))))
        Close-Drawer
        Assert-Results $results 'drawer'
    } -NoShot
    Run-App 'drawer-en' @{ language = 'en-US' } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        Open-Drawer
        $en = { param($id) Nk-Text $id -Lang en }
        $results.Add((Check 'the interface is English' $true ([bool](Get-Element (& $en 'settings.search') 'TextBox'))))
        $results.Add((Check 'the sections are in English' (@('settings.appearance', 'settings.terminal', 'settings.profiles', 'settings.agents', 'settings.keys', 'settings.advanced' | ForEach-Object { & $en $_ }) -join '|') ((@('settings.appearance', 'settings.terminal', 'settings.profiles', 'settings.agents', 'settings.keys', 'settings.advanced' | ForEach-Object { Get-Element (& $en $_) 'ToggleButton' } | Where-Object { $_ } | Sort-Object { $_.Current.BoundingRectangle.X } | ForEach-Object { $_.Current.Name })) -join '|')))
        Type-Unicode 'opacity' 40; Start-Sleep -Milliseconds 600
        $results.Add((Check 'the search finds the opacity slider by its English label' (& $en 'settings.opacity') ((Get-Slider 0).Current.Name)))
        Close-Drawer
        Assert-Results $results 'drawer-en'
    } -NoShot
}
# One key press of a keyboard-only task, counted in $script:Taps (a chord is one press).
function Tap([int]$Vk, [int[]]$Mods = @()) { $script:Taps++; Press $Vk $Mods }
# The element of a row's choice: "row: option", the name a reader says.
function Choice-Name([string]$Row, [string]$Option) { Nk-Text 'common.labelValue' @($Row, $Option) }
# The reset button of a row, by the name it has.
function Reset-Button([string]$Row) { Get-Element (Nk-Text 'common.labelValue' @((Nk-Text 'common.restoreDefault'), $Row)) 'Button' }
function Field-Value([string]$Name) { (Get-Element $Name 'TextBox').GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value }
function Saved-Settings { Get-Content (Join-Path (Join-Path (Join-Path (Split-Path $script:LogFile) ((Split-Path $script:LogFile -Leaf).Replace('.log', '') + '-appdata')) 'Nekominal') 'settings.json') -Raw | ConvertFrom-Json }

# The settings drawer, second half: the colour scheme is chosen with the keyboard alone (5 presses; the study allows 8) and
# previewed while a tile is pointed at or has the focus, and Esc or leaving the grid puts the kept scheme back; a modified
# row has a reset button that restores the default (and the file then holds nothing for it); a font size typed into a
# field reaches the terminal at once; a toggle of the terminal page is saved. Backdrop and opacity: the slider is reached
# with 4 Tab presses, 60 % takes 9 key presses in all, a replay of 200 value changes never sets the backdrop and applies
# at most once per frame, and the slider is off and reads 100 % under backdrop None while the saved value stays.
function Scenario-DrawerRows {
    Run-App 'drawer-rows' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        # 1. keyboard only: Ctrl+Shift+O, Tab to the strip, Tab to the tiles, Down to Nord, Enter
        $script:Taps = 0; $watch = [Diagnostics.Stopwatch]::StartNew()
        $mark = Log-Mark
        Tap 0x4F @(0x11, 0x10); [void](Wait-Until { Drawer-Open })
        Tap 0x09; Tap 0x09
        $results.Add((Check 'two Tab presses from the search box reach the colour scheme tiles (on the kept scheme)' 'Tokyo Night' (Focused-Name)))
        Tap 0x28; Start-Sleep -Milliseconds 500
        $results.Add((Check 'Down moves to Nord, one row down in the grid' 'Nord' (Focused-Name)))
        $seen = @(Log-Since $mark | Where-Object { $_ -match "glass: scheme 'Nord'" }).Count
        $results.Add((Check-True 'the terminals show Nord as soon as its tile has the focus' ($seen -ge 1) "$seen glass lines"))
        $results.Add((Check 'which is only a preview: the kept scheme is still Tokyo Night' 'On|Off' ((Toggle-State 'Tokyo Night') + '|' + (Toggle-State 'Nord'))))
        Tap 0x0D; Start-Sleep -Milliseconds 400; $watch.Stop()
        $results.Add((Check 'Enter keeps it' 'On|Off' ((Toggle-State 'Nord') + '|' + (Toggle-State 'Tokyo Night'))))
        $results.Add((Check-True 'the whole task took at most 8 key presses' ($script:Taps -le 8) "$($script:Taps) presses"))
        $results.Add((Check-True 'and at most 10 s' ($watch.Elapsed.TotalSeconds -le 10) ('{0:N1} s' -f $watch.Elapsed.TotalSeconds)))
        'colour scheme task: {0} key presses, {1:N1} s' -f $script:Taps, $watch.Elapsed.TotalSeconds
        Start-Sleep -Milliseconds 1200
        $results.Add((Check 'and the file holds it' 'Nord' ((Saved-Settings).theme)))
        # 2. preview and restore: the keyboard (Esc), the pointer (leaving the grid)
        $mark = Log-Mark
        Tap 0x27; Start-Sleep -Milliseconds 500
        $results.Add((Check 'Right moves the focus to the next tile and previews it' "Gruvbox Dark" (Focused-Name)))
        $results.Add((Check-True 'the terminals show it' (@(Log-Since $mark | Where-Object { $_ -match "glass: scheme 'Gruvbox Dark'" }).Count -ge 1)))
        Press 0x1B; Start-Sleep -Milliseconds 600
        $results.Add((Check-True 'Esc puts the kept scheme back on the terminals' (@(Log-Since $mark | Where-Object { $_ -match "glass: scheme 'Nord'" }).Count -ge 1)))
        $results.Add((Check 'and keeps the drawer open with the focus on the kept tile' 'True|Nord' ((Drawer-Open).ToString() + '|' + (Focused-Name))))
        $mark = Log-Mark
        $tile = Get-ElementRect 'Dracula' 'ToggleButton'
        Move-To ($tile.X + [int]($tile.W / 2)) ($tile.Y + [int]($tile.H / 2)); Start-Sleep -Milliseconds 600
        $results.Add((Check-True 'pointing at a tile previews its scheme' (@(Log-Since $mark | Where-Object { $_ -match "glass: scheme 'Dracula'" }).Count -ge 1)))
        $results.Add((Check 'without keeping it' 'Off' (Toggle-State 'Dracula')))
        $window = Win-Rect; Move-To ($window.Left + 200) ($window.Top + 500); Start-Sleep -Milliseconds 700
        $results.Add((Check-True 'leaving the grid puts the kept scheme back' (@(Log-Since $mark | Where-Object { $_ -match "glass: scheme 'Nord'" }).Count -ge 1)))
        $results.Add((Check 'the kept scheme is still marked' 'On' (Toggle-State 'Nord')))

        # 3. a modified row: its reset button, the file, the terminal
        $size = Nk-Text 'settings.fontSize'
        $results.Add((Check 'an unchanged row has its reset button off' $false (Reset-Button $size).Current.IsEnabled))
        $mark = Log-Mark
        Click-Name $size -Class 'TextBox'; Chord @(0x11) 0x41; Type-Unicode '17' 60; Press 0x0D; Start-Sleep -Milliseconds 1200
        $results.Add((Check 'a font size typed into its field and entered is in effect' 17 ([int](Field-Value $size))))
        $results.Add((Check-True 'the terminal took the new size (its grid changed)' ((Grid-Lines $mark).Count -ge 1)))
        $results.Add((Check 'the row now has its reset button on' $true (Reset-Button $size).Current.IsEnabled))
        $results.Add((Check 'and the file holds the size' 17 ((Saved-Settings).font.size)))
        Click-Name (Nk-Text 'common.labelValue' @((Nk-Text 'common.restoreDefault'), $size)) -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'reset restores the default' '14' (Field-Value $size)))
        $results.Add((Check 'and turns the reset button off' $false (Reset-Button $size).Current.IsEnabled))
        $results.Add((Check 'and the file holds nothing for the size any more' $null ((Saved-Settings).font)))
        # 4. the terminal page: a toggle is saved
        Click-Name (Nk-Text 'settings.terminal') -Class 'ToggleButton'; Start-Sleep -Milliseconds 600
        $row = Nk-Text 'settings.copyOnSelect'
        $results.Add((Check 'copy on select starts off' 'On|Off' ((Toggle-State (Choice-Name $row (Nk-Text 'toggle.off'))) + '|' + (Toggle-State (Choice-Name $row (Nk-Text 'toggle.on'))))))
        Click-Name (Choice-Name $row (Nk-Text 'toggle.on')); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'choosing On switches it' 'Off|On' ((Toggle-State (Choice-Name $row (Nk-Text 'toggle.off'))) + '|' + (Toggle-State (Choice-Name $row (Nk-Text 'toggle.on'))))))
        $results.Add((Check 'and the file holds it' $true ((Saved-Settings).copyOnSelect)))
        Press 0x1B; Start-Sleep -Milliseconds 400
        Assert-Results $results 'drawer-rows'
    } -NoShot
    Run-App 'drawer-opacity' @{ confirmClose = $false; appearance = @{ opacity = 0.72 } } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $script:Taps = 0; $mark = Log-Mark
        Tap 0x4F @(0x11, 0x10); [void](Wait-Until { Drawer-Open })
        foreach ($i in 1..4) { Tap 0x09 }
        $results.Add((Check 'four Tab presses from the search box reach the opacity slider' (Nk-Text 'settings.opacity') (Focused-Name)))
        Tap 0x22; Tap 0x22; Tap 0x25; Tap 0x25; Start-Sleep -Milliseconds 800
        $results.Add((Check 'two Page Down and two Left arrows make 72 % into 60 %' 60 (Slider-Value (Get-Slider 0))))
        $results.Add((Check-True 'the task took at most 15 key presses' ($script:Taps -le 15) "$($script:Taps) presses"))
        'opacity task: {0} key presses' -f $script:Taps
        Start-Sleep -Milliseconds 1200
        $results.Add((Check 'the file holds 0.6' 0.6 ((Saved-Settings).appearance.opacity)))
        $results.Add((Check 'the value label reads 60%' '60%' (Get-Element '60%' 'TextBlock').Current.Name))
        $results.Add((Check 'none of it set the system backdrop' 0 (@(Log-Since $mark | Where-Object { $_ -match 'system backdrop set' }).Count)))
        # a replay of 200 value changes through UI Automation, the same events a drag raises
        $mark = Log-Mark
        $range = (Get-Slider 0).GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
        for ($i = 0; $i -lt 200; $i++) { $range.SetValue(20 + [Math]::Abs((($i * 4) % 160) - 80)) }
        Start-Sleep -Milliseconds 1800
        $lines = @(Log-Since $mark)
        $applies = @($lines | Where-Object { $_ -match 'opacity applied' })
        $gaps = @(for ($i = 1; $i -lt $applies.Count; $i++) { (Log-Ms $applies[$i]) - (Log-Ms $applies[$i - 1]) })
        $shortest = if ($gaps.Count) { ($gaps | Measure-Object -Minimum).Minimum } else { [double]::PositiveInfinity }
        $results.Add((Check-True 'a replay of 200 value changes applies far fewer than 200 times' ($applies.Count -ge 1 -and $applies.Count -lt 100) "$($applies.Count) applies"))
        $results.Add((Check-True 'at most once per frame' ($shortest -ge 5) "shortest gap $shortest ms"))
        $results.Add((Check 'without setting the system backdrop' 0 (@($lines | Where-Object { $_ -match 'system backdrop set' }).Count)))
        $results.Add((Check-True 'and saving at most once' (@($lines | Where-Object { $_ -match 'settings saved' }).Count -le 1)))
        $results.Add((Check 'with no apply error' 0 (@($lines | Where-Object { $_ -match 'apply_glass:|apply_opacity:' }).Count)))
        $results.Add((Check 'and no InfoBar' 0 (Count-Class 'InfoBar')))
        # backdrop None: the slider is off and reads 100 %, the saved value stays
        $saved = (Saved-Settings).appearance.opacity
        Click-Name (Choice-Name (Nk-Text 'settings.backdrop') (Nk-Text 'backdrop.none')); Start-Sleep -Milliseconds 1500
        $slider = Get-Slider 0
        $results.Add((Check 'backdrop None turns the slider off' $false $slider.Current.IsEnabled))
        $results.Add((Check 'and it reads 100' 100 (Slider-Value $slider)))
        $results.Add((Check 'and its label reads 100%' '100%' (Get-Element '100%' 'TextBlock').Current.Name))
        $results.Add((Check 'while the file keeps the opacity' $saved ((Saved-Settings).appearance.opacity)))
        Click-Name (Choice-Name (Nk-Text 'settings.backdrop') (Nk-Text 'backdrop.acrylic')); Start-Sleep -Milliseconds 1200
        $results.Add((Check 'back on Acrylic the slider is on again' $true (Get-Slider 0).Current.IsEnabled))
        $results.Add((Check 'and shows the kept opacity' ([int]($saved * 100)) (Slider-Value (Get-Slider 0))))
        Press 0x1B; Start-Sleep -Milliseconds 400
        Assert-Results $results 'drawer-opacity'
    } -NoShot
}
# The settings drawer, third part: a settings file with wrong values opens with a warning that has the action "open the
# settings file" (logged, not opened), the advanced page lists each problem in the language of the interface with the
# file, and each affected row says so; fixing the file and reloading takes it all away. The profile list chooses the
# default profile, the agents page holds its four rows (the OSC 52 choice is saved and reset), the advanced page the
# language, the version, ConPTY and the file.
function Scenario-DrawerPages {
    $bad = @{ theme = 'Nope'; font = @{ size = 400 }; cursor = @{ style = 'star' }; keybindings = @{ 'tab.new' = 'nonsense+'; 'paste' = 'Ctrl+V' } }
    Run-App 'drawer-pages' (@{ confirmClose = $false } + $bad) {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $results.Add((Check 'a file with 5 problems opens with the warning that says so' $true ([bool](Get-Element (Nk-Text 'status.settingsProblems' @(5))))))
        $action = Nk-Text 'app.openSettingsFile'
        $results.Add((Check 'and offers to open the settings file' $true ([bool](Get-Element $action 'Button'))))
        $mark = Log-Mark
        Click-Name $action -Class 'Button'; Start-Sleep -Milliseconds 800
        $results.Add((Check-True 'the action asks for the settings file (and starts nothing)' (@(Log-Since $mark | Where-Object { $_ -match 'not opened \(NEKOMINAL_NO_LAUNCH is set\): .*settings\.json' }).Count -eq 1)))
        Open-Drawer
        $row = { param($key) Get-Element $key 'TextBlock' }
        $results.Add((Check 'the appearance page tags the three rows whose value was invalid' 3 @(Get-ElementsByName (Nk-Text 'setting.invalid')).Count))
        Click-Name (Nk-Text 'settings.advanced') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $results.Add((Check 'the advanced page has the heading of the problems' $true ([bool](& $row (Nk-Text 'settings.problems')))))
        $results.Add((Check 'and the action to open the file' $true ([bool](Get-Element $action 'Button'))))
        foreach ($key in 'theme', 'font.size', 'cursor.style', 'keybindings.tab.new', 'keybindings.paste') { $results.Add((Check "the problem of $key is listed" $true ([bool](& $row $key)))) }
        $results.Add((Check 'in the words of the interface' $true ([bool](& $row '需要 6 到 72 的數字'))))
        $results.Add((Check 'also for a shortcut that is not valid' $true ([bool](& $row (Nk-Text 'problem.badShortcut' @('nonsense+'))))))
        $results.Add((Check 'and a command that does not exist' $true ([bool](& $row (Nk-Text 'problem.unknownCommand')))))
        $results.Add((Check 'the file itself is on the page' $true ([bool](& $row (Nk-Text 'settings.file')))))
        Close-Drawer
        # fixing the file and reloading takes the warning away
        $file = Join-Path (Join-Path (Join-Path (Split-Path $script:LogFile) 'drawer-pages-appdata') 'Nekominal') 'settings.json'
        Set-Content -Path $file -Value '{ "language": "zh-TW", "confirmClose": false }' -Encoding UTF8
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 700; Type-Unicode (Nk-Text 'app.reloadSettings') 40; Press 0x0D; Start-Sleep -Milliseconds 1200
        $results.Add((Check 'a fixed file and a reload take the warning away' $false ([bool](Get-Element (Nk-Text 'status.settingsProblems' @(5))))))
        Open-Drawer
        $results.Add((Check 'and the tags' 0 @(Get-ElementsByName (Nk-Text 'setting.invalid')).Count))
        Click-Name (Nk-Text 'settings.advanced') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $results.Add((Check 'and the list of problems' $false ([bool](& $row (Nk-Text 'settings.problems')))))
        $results.Add((Check 'while the version, ConPTY and the file stay' 3 @((Nk-Text 'settings.version'), (Nk-Text 'settings.conpty'), (Nk-Text 'settings.file') | Where-Object { & $row $_ }).Count))
        Close-Drawer
        Assert-Results $results 'drawer-pages'
    } -NoShot
    Run-App 'drawer-pages-en' (@{ language = 'en-US' } + $bad) {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        Open-Drawer
        Click-Name (Nk-Text 'settings.advanced' -Lang en) -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $texts = @(Get-ElementsByName '*' | ForEach-Object { $_.Current.Name } | Where-Object { $_ -match 'Needs|No such|valid shortcut|No color scheme' })
        $texts | ForEach-Object { 'problem text: ' + $_ }
        $results.Add((Check 'the problems are listed in English' 5 $texts.Count))
        $results.Add((Check 'with no Chinese in them' 0 @($texts | Where-Object { $_ -match '[⺀-鿿]' }).Count))
        $results.Add((Check 'the number problem reads like the table says' $true ($texts -contains (Nk-Text 'problem.number' @(6, 72) -Lang en))))
        Close-Drawer
        Assert-Results $results 'drawer-pages-en'
    } -NoShot
    Run-App 'drawer-pages-rows' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        Open-Drawer
        # profiles: the default profile is chosen from the list; reset puts the first one back
        Click-Name (Nk-Text 'settings.profiles') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $sections = 'settings.appearance', 'settings.terminal', 'settings.profiles', 'settings.agents', 'settings.keys', 'settings.advanced' | ForEach-Object { Nk-Text $_ }
        $profiles = @(Get-ElementsByName '*' 'ToggleButton' | Where-Object { $sections -notcontains $_.Current.Name } | Sort-Object { $_.Current.BoundingRectangle.Y })
        'profiles: ' + (($profiles | ForEach-Object { $_.Current.Name }) -join ' | ')
        $results.Add((Check-True 'the profile list has the shells of this machine' ($profiles.Count -ge 2)))
        $states = @($profiles | ForEach-Object { [string]$_.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState })
        $results.Add((Check 'the first is the default' 'On' $states[0]))
        $second = $profiles[1].Current.Name
        Click-Name $second -Class 'ToggleButton'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'choosing the second makes it the default' 'On' (Toggle-State $second)))
        $results.Add((Check 'and the file holds its id' $true (-not [string]::IsNullOrEmpty((Saved-Settings).defaultProfile))))
        $results.Add((Check 'the row has its reset button on' $true (Reset-Button (Nk-Text 'settings.defaultProfile')).Current.IsEnabled))
        Click-Name (Nk-Text 'common.labelValue' @((Nk-Text 'common.restoreDefault'), (Nk-Text 'settings.defaultProfile'))) -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'reset makes the first the default again' 'Off' (Toggle-State $second)))
        $results.Add((Check 'and the file holds none' $null ((Saved-Settings).defaultProfile)))
        # agents: four rows; the OSC 52 choice
        Click-Name (Nk-Text 'settings.agents') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $rows = 'settings.agentEnvironment', 'settings.kitty', 'settings.shellIntegration', 'settings.osc52' | ForEach-Object { Nk-Text $_ }
        $results.Add((Check 'the agents page has its four rows' 4 @($rows | Where-Object { Get-Element $_ 'TextBlock' }).Count))
        $osc = Nk-Text 'settings.osc52'
        $results.Add((Check 'OSC 52 starts on write only' 'On' (Toggle-State (Choice-Name $osc (Nk-Text 'osc52.write')))))
        Click-Name (Choice-Name $osc (Nk-Text 'osc52.readWrite')); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'choosing read and write is saved' 'read-write' ((Saved-Settings).osc52)))
        Click-Name (Nk-Text 'common.labelValue' @((Nk-Text 'common.restoreDefault'), $osc)) -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'and reset takes it back out of the file' $null ((Saved-Settings).osc52)))
        # the kitty protocol row is a toggle like the others
        $kitty = Nk-Text 'settings.kitty'
        Click-Name (Choice-Name $kitty (Nk-Text 'toggle.off')); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'turning the kitty keyboard protocol off is saved' $false ((Saved-Settings).keyboard.kittyProtocol)))
        # advanced: the language is a setting of the file (read at start-up)
        Click-Name (Nk-Text 'settings.advanced') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        Click-Name (Choice-Name (Nk-Text 'settings.language') (Nk-Text 'language.enUs')); Start-Sleep -Milliseconds 1500
        $results.Add((Check 'choosing English is saved for the next start' 'en-US' ((Saved-Settings).language)))
        $results.Add((Check 'the interface stays in its language until then' $true ([bool](Get-Element (Nk-Text 'settings.search') 'TextBox'))))
        Close-Drawer
        Assert-Results $results 'drawer-pages-rows'
    } -NoShot
}
# The settings drawer, fourth part: the shortcuts page. A chord is recorded for a command (the key that is pressed does not
# run the command meanwhile); a chord that another command has says so and is taken only by Replace, Cancel leaves
# everything as it was; the chords programs need (Ctrl+C, Ctrl+D) ask the same; Backspace removes a binding; Esc lets go of
# a recording and keeps the drawer; a row has a reset button, everything has the confirmation "restore all"; the file
# holds only the difference and the menus and tooltips show the new chord.
function Key-Chip([string]$Command, [string]$Chord) { Nk-Text 'common.labelValue' @((Nk-Text $Command), $Chord) }
function Key-Button([string]$Command, [string]$Action) { Nk-Text 'common.labelValue' @((Nk-Text $Command), $Action) }
function Scenario-DrawerKeys {
    Run-App 'drawer-keys' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        Open-Drawer
        Click-Name (Nk-Text 'settings.keys') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $newTab = Key-Chip 'tab.new' 'Ctrl+Shift+T'
        $results.Add((Check 'the page lists the shortcuts of the commands as buttons' $true ([bool](Get-Element $newTab 'Button'))))
        $results.Add((Check 'a command with two shortcuts has both' $true ([bool]((Get-Element (Key-Chip 'pane.splitRight' 'Alt+Shift+D') 'Button') -and (Get-Element (Key-Chip 'pane.splitRight' 'Alt+Shift+=') 'Button')))))
        $results.Add((Check 'a command without one has the button that adds one' $true ([bool](Get-Element (Key-Button 'tab.close' (Nk-Text 'keys.add')) 'Button'))))
        $results.Add((Check 'no row is modified: the restore button is off' $false (Get-Element (Nk-Text 'common.restoreDefault') 'Button').Current.IsEnabled))
        # a chord that is another command's: Cancel leaves everything as it was
        $chips0 = @(Get-Chips).Count
        Click-Name $newTab -Class 'Button'; Start-Sleep -Milliseconds 500
        $results.Add((Check 'a click on a shortcut waits for a key' $true ([bool](Get-Element (Key-Button 'tab.new' (Nk-Text 'keys.record')) 'Button'))))
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 600
        $results.Add((Check 'Ctrl+Shift+W, which closes a pane, says so' $true ([bool](Get-Element (Nk-Text 'keys.conflict' @((Nk-Text 'pane.close'))) 'TextBlock'))))
        $results.Add((Check 'and offers Replace and Cancel' $true ([bool]((Get-Element (Key-Button 'tab.new' (Nk-Text 'keys.replace')) 'Button') -and (Get-Element (Key-Button 'tab.new' (Nk-Text 'common.cancel')) 'Button')))))
        $results.Add((Check 'the focus is on Cancel' (Key-Button 'tab.new' (Nk-Text 'common.cancel')) (Focused-Name)))
        $results.Add((Check 'the chord did not close the pane' 1 (Count-Panes)))
        Click-Name (Key-Button 'tab.new' (Nk-Text 'common.cancel')) -Class 'Button'; Start-Sleep -Milliseconds 600
        $results.Add((Check 'Cancel brings the old shortcut back' $true ([bool](Get-Element $newTab 'Button'))))
        $results.Add((Check 'and the close shortcut is still the pane command' $true ([bool](Get-Element (Key-Chip 'pane.close' 'Ctrl+Shift+W') 'Button'))))
        # the key that is pressed while a chord is waited for is not a command
        Click-Name $newTab -Class 'Button'; Start-Sleep -Milliseconds 400
        Chord @(0x11, 0x10) 0x54; Start-Sleep -Milliseconds 700
        $results.Add((Check 'Ctrl+Shift+T, pressed while waiting, opens no tab (it is the chord of the row itself, so no conflict)' $chips0 @(Get-Chips).Count))
        $results.Add((Check 'and the row has it again' $true ([bool](Get-Element $newTab 'Button'))))
        # Replace takes it
        Click-Name $newTab -Class 'Button'; Start-Sleep -Milliseconds 400
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 600
        Click-Name (Key-Button 'tab.new' (Nk-Text 'keys.replace')) -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'Replace gives the new tab command the chord' $true ([bool](Get-Element (Key-Chip 'tab.new' 'Ctrl+Shift+W') 'Button'))))
        $results.Add((Check 'and leaves the close command without it' $false ([bool](Get-Element (Key-Chip 'pane.close' 'Ctrl+Shift+W') 'Button'))))
        $keys = (Saved-Settings).keybindings
        $results.Add((Check 'the file holds the new chord of the new tab command' 'Ctrl+Shift+W' ([string]($keys.'tab.new'))))
        $results.Add((Check 'and that the close command has none' 0 @($keys.'pane.close').Count))
        $results.Add((Check 'the tooltip of the new tab button shows the chord' $true ([bool](Get-Element (Nk-Tip 'tab.new' 'Ctrl+Shift+W') 'Button'))))
        $results.Add((Check 'and the rows that changed have their reset buttons on' $true ([bool]((Reset-Button (Nk-Text 'tab.new')).Current.IsEnabled -and (Reset-Button (Nk-Text 'pane.close')).Current.IsEnabled))))
        Close-Drawer
        Chord @(0x11, 0x10) 0x57; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'the chord now opens a tab' ($chips0 + 1) @(Get-Chips).Count))
        # reset of one row and the confirmation of "restore all"
        Open-Drawer; Click-Name (Nk-Text 'settings.keys') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        Click-Name (Nk-Text 'common.labelValue' @((Nk-Text 'common.restoreDefault'), (Nk-Text 'tab.new'))) -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'reset gives the new tab command its default back' $true ([bool](Get-Element $newTab 'Button'))))
        $results.Add((Check 'and takes it out of the file' $null ((Saved-Settings).keybindings.'tab.new')))
        $results.Add((Check 'the other row is still modified, so the restore button is on' $true (Get-Element (Nk-Text 'common.restoreDefault') 'Button').Current.IsEnabled))
        Click-Name (Nk-Text 'common.restoreDefault') -Class 'Button'; Start-Sleep -Milliseconds 800
        $results.Add((Check 'restore all asks first' $true ([bool](Get-Element (Nk-Text 'dialog.restoreKeys')))))
        Click-Name (Nk-Text 'common.cancel') -Class 'Button'; Start-Sleep -Milliseconds 700
        $results.Add((Check 'Cancel changes nothing' $false ([bool](Get-Element (Key-Chip 'pane.close' 'Ctrl+Shift+W') 'Button'))))
        Click-Name (Nk-Text 'common.restoreDefault') -Class 'Button'; Start-Sleep -Milliseconds 800
        Click-Name (Nk-Text 'common.restore') -Class 'Button'; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'Restore brings back every default' $true ([bool](Get-Element (Key-Chip 'pane.close' 'Ctrl+Shift+W') 'Button'))))
        $results.Add((Check 'and the file holds no keybindings' $null ((Saved-Settings).keybindings)))
        # what programs need: Ctrl+D and Ctrl+C ask like a command does
        $add = Key-Button 'tab.close' (Nk-Text 'keys.add')
        Click-Name $add -Class 'Button'; Start-Sleep -Milliseconds 400
        Chord @(0x11) 0x44; Start-Sleep -Milliseconds 600
        $results.Add((Check 'Ctrl+D, which a shell needs, says so' $true ([bool](Get-Element (Nk-Text 'keys.reserved') 'TextBlock'))))
        Click-Name (Key-Button 'tab.close' (Nk-Text 'common.cancel')) -Class 'Button'; Start-Sleep -Milliseconds 500
        Click-Name $add -Class 'Button'; Start-Sleep -Milliseconds 400
        Chord @(0x11) 0x43; Start-Sleep -Milliseconds 600
        $results.Add((Check 'Ctrl+C is the copy command and says so' $true ([bool](Get-Element (Nk-Text 'keys.conflict' @((Nk-Text 'edit.copy'))) 'TextBlock'))))
        Click-Name (Key-Button 'tab.close' (Nk-Text 'common.cancel')) -Class 'Button'; Start-Sleep -Milliseconds 500
        $results.Add((Check 'the file holds nothing yet' $null ((Saved-Settings).keybindings)))
        # Esc lets go of a recording and keeps the drawer
        Click-Name $add -Class 'Button'; Start-Sleep -Milliseconds 400
        Press 0x1B; Start-Sleep -Milliseconds 600
        $results.Add((Check 'Esc lets go of the recording and the drawer stays' 'True|True' ((Drawer-Open).ToString() + '|' + [bool](Get-Element $add 'Button'))))
        # a chord is added with the plus, taken away with Backspace
        Click-Name $add -Class 'Button'; Start-Sleep -Milliseconds 400
        Chord @(0x11, 0x10) 0x51; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'a free chord is taken at once' $true ([bool](Get-Element (Key-Chip 'tab.close' 'Ctrl+Shift+Q') 'Button'))))
        $results.Add((Check 'and written to the file' 'Ctrl+Shift+Q' ([string](Saved-Settings).keybindings.'tab.close')))
        Click-Name (Key-Chip 'tab.close' 'Ctrl+Shift+Q') -Class 'Button'; Start-Sleep -Milliseconds 400
        Press 0x08; Start-Sleep -Milliseconds 1500
        $results.Add((Check 'Backspace on a waiting shortcut removes it' $false ([bool](Get-Element (Key-Chip 'tab.close' 'Ctrl+Shift+Q') 'Button'))))
        $results.Add((Check 'and the file holds nothing again' $null ((Saved-Settings).keybindings)))
        Close-Drawer
        Assert-Results $results 'drawer-keys'
    } -NoShot
}
# The first rewrite of a settings file by the application keeps the file as it was (comments and keys of a newer version
# included) as settings.json.orig, and no later write touches that copy.
function Scenario-FirstWrite {
    Run-App 'drawer-orig' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $dir = Join-Path (Join-Path (Split-Path $script:LogFile) 'drawer-orig-appdata') 'Nekominal'
        $original = "// my own notes`r`n{ `"language`": `"zh-TW`", `"confirmClose`": false, `"futureKey`": { `"x`": 1 } } // 設定`r`n"
        [IO.File]::WriteAllText((Join-Path $dir 'settings.json'), $original, (New-Object Text.UTF8Encoding($false)))
        Open-Drawer
        Click-Name (Nk-Text 'settings.terminal') -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
        $row = Nk-Text 'settings.copyOnSelect'
        Click-Name (Choice-Name $row (Nk-Text 'toggle.on')); Start-Sleep -Milliseconds 1800
        $orig = Join-Path $dir 'settings.json.orig'
        $results.Add((Check 'the first change made in the window keeps the file that was there' $true (Test-Path $orig)))
        $results.Add((Check 'byte for byte, notes and unknown key included' $original ([IO.File]::ReadAllText($orig))))
        $written = [IO.File]::ReadAllText((Join-Path $dir 'settings.json'))
        $results.Add((Check 'while the file itself is now written from the settings (no notes)' $false ($written -match 'my own notes')))
        $results.Add((Check 'and holds the change' $true ((Saved-Settings).copyOnSelect)))
        Click-Name (Choice-Name $row (Nk-Text 'toggle.off')); Start-Sleep -Milliseconds 1800
        $results.Add((Check 'a later change leaves the original alone' $original ([IO.File]::ReadAllText($orig))))
        $results.Add((Check 'and the backup is the version before the last write' $true ((Get-Content (Join-Path $dir 'settings.json.bak') -Raw) -match 'copyOnSelect')))
        Close-Drawer
        Assert-Results $results 'drawer-orig'
    } -NoShot
}
# The text of the drawer must not depend on the opacity of the background: over the striped window (the worst thing that can
# show through a panel) the contrast of the text of a row, measured in the picture as the brightest ink pixel against the
# darkest pixel of the region, is taken with the opacity at 20 % and at 100 %, for a dark and a light scheme.
function Region-Contrast([string]$Png, [int]$X, [int]$Y, [int]$W, [int]$H) {
    $bmp = [System.Drawing.Bitmap]::FromFile($Png)
    try {
        $lin = { param($c) $v = $c / 255.0; if ($v -le 0.04045) { $v / 12.92 } else { [Math]::Pow(($v + 0.055) / 1.055, 2.4) } }
        $min = 2.0; $max = -1.0
        for ($yy = $Y; $yy -lt [Math]::Min($Y + $H, $bmp.Height); $yy++) {
            for ($xx = $X; $xx -lt [Math]::Min($X + $W, $bmp.Width); $xx++) {
                $c = $bmp.GetPixel($xx, $yy)
                $l = 0.2126 * (& $lin $c.R) + 0.7152 * (& $lin $c.G) + 0.0722 * (& $lin $c.B)
                if ($l -lt $min) { $min = $l }; if ($l -gt $max) { $max = $l }
            }
        }
        [pscustomobject]@{ Min = $min; Max = $max; Contrast = ($max + 0.05) / ($min + 0.05) }
    } finally { $bmp.Dispose() }
}
function Scenario-DrawerGlass {
    foreach ($theme in 'Tokyo Night', 'One Half Light') {
        $name = 'drawer-glass-' + ($theme -replace ' ', '-').ToLower()
        Run-App $name @{ theme = $theme; confirmClose = $false } {
            First-Prompt
            Open-Drawer
            $results = [System.Collections.Generic.List[string]]::new()
            $slider = Get-Slider 0
            $label = Get-ElementRect (Nk-Text 'settings.opacity') 'TextBlock'
            $window = Win-Rect; $margin = $script:PatternMargin
            Move-To ($window.Left + 120) ($window.Top + 600); Start-Sleep -Milliseconds 400
            $measure = {
                param($percent, $shot)
                Set-Slider $percent 0; Start-Sleep -Milliseconds 1600
                Shot-WithPattern (Shot-Path $shot) | Out-Null
                Region-Contrast (Shot-Path $shot) ($label.X - ($window.Left - $margin)) ($label.Y - ($window.Top - $margin)) $label.W $label.H
            }
            $low = & $measure 20 "$name-20"
            $high = & $measure 100 "$name-100"
            'theme {0}: text contrast over the striped window: opacity 20% {1:N2} (ink {2:N3}, darkest {3:N3}) | 100% {4:N2} (ink {5:N3}, darkest {6:N3})' -f $theme, $low.Contrast, $low.Max, $low.Min, $high.Contrast, $high.Max, $high.Min
            $delta = [Math]::Abs($low.Contrast - $high.Contrast)
            $results.Add((Check-True "the text contrast of a row changes by at most 0.1 between 20 % and 100 % opacity ($theme)" ($delta -le 0.1) ('{0:N3}' -f $delta)))
            $results.Add((Check-True "and the text reads at both (at least 4.5:1) ($theme)" (($low.Contrast -ge 4.5) -and ($high.Contrast -ge 4.5)) ('{0:N2} / {1:N2}' -f $low.Contrast, $high.Contrast)))
            Close-Drawer
            Assert-Results $results $name
        } -Pattern -NoShot
    }
}
# The width of the drawer at 1440, 1000, 700 and 500 DIP of window: 480 DIP while the window leaves room for it and its 8
# DIP margins, never wider than the window less the margins; always inside the window, its right edge 8 DIP from the edge
# of the window and the terminal under it not resized. Pictures of each width in docs/screenshots.
function Scenario-DrawerWidths {
    Run-App 'drawer-widths' @{ confirmClose = $false } {
        First-Prompt
        $results = [System.Collections.Generic.List[string]]::new()
        $scale = Get-Scale
        foreach ($width in 1440, 1000, 700, 500) {
            Set-WindowWidth $width
            $mark = Log-Mark
            Open-Drawer
            $window = Win-Rect
            $search = (Drawer-Search).Current.BoundingRectangle
            # The terminal area is as wide as the body the drawer floats in. The search box has 16 DIP of the sheet on each
            # side, and the sheet a 1 DIP border.
            $body = Terminal-Rect
            $bodyDip = $body.W / $scale
            $drawerDip = $search.Width / $scale + 34
            $right = (($body.X + $body.W) - ($search.X + $search.Width)) / $scale - 17
            $expected = [Math]::Max(320, [Math]::Min(480, $bodyDip - 16))
            'at {0} DIP of window (body {1:N1}): drawer {2:N1} DIP wide, {3:N1} DIP from the right edge of the body' -f $width, $bodyDip, $drawerDip, $right
            $results.Add((Check-True "$width DIP: the drawer is the width the rule gives (480, or the window less 16, at least 320)" ([Math]::Abs($drawerDip - $expected) -le 2) ('{0:N1} vs {1:N1}' -f $drawerDip, $expected)))
            $results.Add((Check-True "$width DIP: its right edge is 8 DIP inside the window" ([Math]::Abs($right - 8) -le 1.5) ('{0:N1} DIP' -f $right)))
            $results.Add((Check-True "$width DIP: it fits inside the window" ($search.X -ge $window.Left)))
            $move = Win-Rect; Move-To ($move.Left + 150) ($move.Top + 600); Start-Sleep -Milliseconds 400
            Shot-Safe (Shot-Path "drawer-width-$width") | Out-Null
            Close-Drawer
            $results.Add((Check "$width DIP: opening and closing it gave the pane no other grid" 0 (Grid-Lines $mark).Count))
        }
        Assert-Results $results 'drawer-widths'
    } -NoShot
    # English: the strip of sections is longer than the sheet, and the rows have longer labels.
    Run-App 'drawer-english' @{ language = 'en-US'; confirmClose = $false } {
        First-Prompt
        Open-Drawer
        # The strip fits at the default width: the last chip ends inside the viewport (a chip cut by it is clipped to it).
        $last = (Get-Element (Nk-Text 'settings.advanced' -Lang en) 'ToggleButton').Current.BoundingRectangle
        $viewport = (Get-Element (Nk-Text 'settings.search' -Lang en) 'TextBox').Current.BoundingRectangle
        Assert-Results @((Check-True 'English: the last section chip is whole inside the strip' ($last.X + $last.Width -le $viewport.X + $viewport.Width - 1) ('chip ends {0:N1}, strip ends {1:N1}' -f ($last.X + $last.Width), ($viewport.X + $viewport.Width)))) 'drawer-english'
        $window = Win-Rect; Move-To ($window.Left + 150) ($window.Top + 600); Start-Sleep -Milliseconds 400
        Shot-Safe (Shot-Path 'drawer-en-appearance') | Out-Null
        foreach ($s in 'settings.terminal', 'settings.profiles', 'settings.agents', 'settings.keys', 'settings.advanced') {
            Click-Name (Nk-Text $s -Lang en) -Class 'ToggleButton'; Start-Sleep -Milliseconds 700
            Move-To ($window.Left + 150) ($window.Top + 600); Start-Sleep -Milliseconds 300
            Shot-Safe (Shot-Path ('drawer-en-' + ($s -replace 'settings\.', ''))) | Out-Null
        }
        Close-Drawer
    } -NoShot
}
function Get-ElementsByName([string]$Name, [string]$Class = '') {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:Hwnd)
    $all = if ($Name -eq '*') { $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) } else { $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name))) }
    foreach ($e in $all) { if (-not $Class -or $e.Current.ClassName.EndsWith($Class)) { $e } }
}
function Scenario-All { Scenario-Glass; Scenario-BlurProof; Scenario-TabsSplits; Scenario-Overlays; Scenario-Inactive; Scenario-Ime; Scenario-Perf; Scenario-Silence; Scenario-Opacity; Scenario-Tabs; Scenario-TabDrag; Scenario-Status; Scenario-Unread; Scenario-StatusExit; Scenario-Toolbar; Scenario-Titles; Scenario-Close; Scenario-Panes; Scenario-Drawer; Scenario-DrawerRows; Scenario-DrawerPages; Scenario-DrawerKeys; Scenario-FirstWrite; Scenario-DrawerGlass; Scenario-DrawerWidths }
