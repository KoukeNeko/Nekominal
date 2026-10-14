# Scenarios for the application: each one starts nekominal.exe with its own settings, drives it with real input and
# leaves screenshots (app window only) in docs/screenshots. Dot-source this file from the repository root, then call a
# scenario or `Run-All`.
#
#   . .\tools\slice-drive\scenarios.ps1; Scenario-Glass; Scenario-Blur
#
# Every scenario goes through run-app.ps1: private APPDATA (settings.json is written from -Settings), NO_COLOR removed,
# guarded screenshots (Shot-Safe / Shot-WithPattern). Scenario results are printed as PASS/FAIL lines taken from the
# application log, so a run can be read without opening the images.

$global:NkRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$global:NkShots = Join-Path $global:NkRoot 'docs\screenshots'
$global:NkWork = Join-Path $env:TEMP 'nk-scenarios'
New-Item -ItemType Directory -Force $global:NkShots, $global:NkWork | Out-Null

function Invoke-App([string]$Name, [hashtable]$Settings = @{}, [scriptblock]$Script, [switch]$Pattern, [switch]$NoShot, [switch]$Maximize) {
    $params = @{ Name = $Name; OutDir = $global:NkWork; WorkDir = $global:NkWork; Settings = $Settings; Script = $Script; SettleMs = 3500 }
    if ($Pattern) { $params.Pattern = $true }
    if ($NoShot) { $params.NoShot = $true }
    if ($Maximize) { $params.Maximize = $true }
    & (Join-Path $PSScriptRoot 'run-app.ps1') @params | Out-Null
    $shot = Join-Path $global:NkWork "$Name.png"
    if (Test-Path $shot) { Copy-Item $shot (Join-Path $global:NkShots "$Name.png") -Force }
    Join-Path $global:NkWork "$Name.log"
}

function Check([string]$Log, [string]$What, [string]$Pattern) {
    $ok = (Test-Path $Log) -and (Select-String -Path $Log -Pattern $Pattern -Quiet)
    '{0}  {1}' -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $What
}

# Sample content for the screenshots: colour, CJK, emoji, a directory listing.
function Fill-Sample {
    Wait-Prompt
    # The keyboard layout of this machine is the Microsoft Zhuyin IME: full-width punctuation and surrogate pairs
    # injected as Unicode input start an IME composition and arrive late, so the sample is typed from ideographs and
    # ASCII only and the emoji is built by the shell.
    Type-Line 'Write-Host ("Nekominal  貓咪終端機  你好世界  " + [char]::ConvertFromUtf32(0x1F431)) -ForegroundColor Cyan' 40
    # The command echo and its output both hold the text: wait for the second occurrence.
    Wait-Text '(?s)你好世界.*你好世界' 8000 | Out-Null; Start-Sleep -Milliseconds 700
    Type-Line 'ls C:\Windows | select -first 7' 40
    Wait-Text 'Branding' 8000 | Out-Null; Start-Sleep -Milliseconds 600
}

function Scenario-Glass {
    $log = Invoke-App 'glass-dark' @{ theme = 'Tokyo Night' } { Fill-Sample }
    Check $log 'dark glass: acrylic, opacity 0.72' 'backdrop=Acrylic opacity=0.72'
    $log = Invoke-App 'glass-light' @{ theme = 'One Half Light' } { Fill-Sample }
    Check $log 'light glass: scheme classified light' "scheme 'One Half Light' light=true"
}

function Scenario-Blur {
    $log = Invoke-App 'blur-proof' @{} -Pattern { Fill-Sample }
    Check $log 'blur: a premultiplied swap chain over the acrylic backdrop' 'PreMultiplied'
}

function Scenario-Backdrops {
    foreach ($b in @(@('mica', 'backdrop-mica'), @('mica-alt', 'backdrop-mica-alt'), @('none', 'backdrop-none'))) {
        $log = Invoke-App $b[1] @{ appearance = @{ backdrop = $b[0] } } { Fill-Sample }
        Check $log "backdrop $($b[0])" ('backdrop=' + @{ 'mica' = 'Mica'; 'mica-alt' = 'MicaAlt'; 'none' = 'None' }[$b[0]])
    }
    foreach ($o in @(1.0, 0.5)) {
        $name = 'opacity-' + [int]($o * 100)
        $log = Invoke-App $name @{ appearance = @{ opacity = $o } } -Pattern { Fill-Sample }
        Check $log "opacity $o" ('opacity=' + $o.ToString('0.00', [Globalization.CultureInfo]::InvariantCulture))
    }
}

function Scenario-Inactive {
    $log = Invoke-App 'inactive' @{} -Pattern -NoShot {
        Fill-Sample
        Shot-Inactive (Join-Path $global:NkWork 'inactive.png')
    }
    Copy-Item (Join-Path $global:NkWork 'inactive.png') (Join-Path $global:NkShots 'inactive.png') -Force -ErrorAction SilentlyContinue
    Check $log 'deactivation reaches the app' 'window active = false'
}

function Scenario-TabsSplits {
    $log = Invoke-App 'tabs-splits' @{} {
        # Every new tab or pane starts a shell: type only once its prompt is on screen (the focused pane's UIA text).
        Wait-Prompt
        Type-Line '$Host.UI.RawUI.WindowTitle = "build"' 40; Start-Sleep -Milliseconds 700
        Type-Line 'Write-Host "tab one" -ForegroundColor Green' 40; Start-Sleep -Milliseconds 600
        Chord @(0x11, 0x10) 0x54; Start-Sleep -Milliseconds 1200; Wait-Prompt
        Type-Line '$Host.UI.RawUI.WindowTitle = "claude"' 40; Start-Sleep -Milliseconds 700
        Type-Line 'Write-Host "tab two" -ForegroundColor Yellow' 40; Start-Sleep -Milliseconds 600
        Chord @(0x11, 0x10) 0x54; Start-Sleep -Milliseconds 1200; Wait-Prompt
        Type-Line '$Host.UI.RawUI.WindowTitle = "logs"' 40; Start-Sleep -Milliseconds 700
        Type-Line 'Write-Host "left pane" -ForegroundColor Magenta' 40; Start-Sleep -Milliseconds 600
        Chord @(0x12, 0x10) 0x44; Start-Sleep -Milliseconds 1200; Wait-Prompt
        Type-Line 'Write-Host "right pane" -ForegroundColor Cyan' 40; Start-Sleep -Milliseconds 600
        Chord @(0x12, 0x10) 0x53; Start-Sleep -Milliseconds 1200; Wait-Prompt
        Type-Line 'Write-Host "bottom right pane" -ForegroundColor Red' 40; Start-Sleep -Milliseconds 900
    }
    Check $log 'three tabs' 'selected tab 3'
    Check $log 'split right and down' 'command pane.splitDown'
}

function Scenario-Overlays {
    $log = Invoke-App 'overlays' @{} -NoShot {
        Fill-Sample
        Chord @(0x11, 0x10) 0x50; Start-Sleep -Milliseconds 900
        Type-Unicode '色彩' 80; Start-Sleep -Milliseconds 700
        Shot-Safe (Join-Path $global:NkWork 'palette.png')
        Press 0x1B; Start-Sleep -Milliseconds 500
        Click-Name '外觀'; Start-Sleep -Milliseconds 900
        Shot-Safe (Join-Path $global:NkWork 'appearance-flyout.png')
        Press 0x1B; Start-Sleep -Milliseconds 500
        Chord @(0x11, 0x10) 0x4F; Start-Sleep -Milliseconds 1200
        Shot-Safe (Join-Path $global:NkWork 'settings-general.png')
        Press 0x28; Start-Sleep -Milliseconds 400   # Down: the Appearance page
        Press 0x0D; Start-Sleep -Milliseconds 900
        Shot-Safe (Join-Path $global:NkWork 'settings-appearance.png')
        Press 0x1B
    }
    foreach ($n in 'palette', 'appearance-flyout', 'settings-general', 'settings-appearance') {
        Copy-Item (Join-Path $global:NkWork "$n.png") (Join-Path $global:NkShots "$n.png") -Force -ErrorAction SilentlyContinue
    }
    Check $log 'palette opened' 'overlay None -> Palette'
    Check $log 'settings opened' 'overlay None -> Settings'
}

function Run-All {
    Scenario-Glass; Scenario-Blur; Scenario-Backdrops; Scenario-Inactive; Scenario-TabsSplits; Scenario-Overlays
}
