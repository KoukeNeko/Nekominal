# Drives the real application (crates/nekominal-app, nekominal.exe) with real SendInput and guarded screenshots.
#
#   . .\tools\slice-drive\run-app.ps1 -Name glass-dark -Settings @{ theme = 'Tokyo Night' } -Script { ... }
#
# Call it with & / dot-sourcing from the same pwsh process (a hashtable or scriptblock does not survive
# `pwsh -File`). The script block runs with drive-lib.ps1 loaded: Press, Type-Unicode, Click-At, Drag-To, Shot-Safe,
# Win-Rect, Wait-Log, Chip-Rect, Add-Settings...
#
# Safety rules (critique.md section 6, trap 38):
#   * input is sent only while the foreground window belongs to the started process (Assert-Foreground);
#   * a screenshot is taken only when the foreground window AND the pixels under the capture area belong to the
#     test process (Shot-Safe); otherwise it is skipped;
#   * NO_COLOR is removed from the child environment (pwsh strips ANSI when it is set);
#   * the app reads and writes its settings under APPDATA: the run gets a private APPDATA directory, so the real
#     settings file of the user is never read or changed. -Settings becomes that directory's settings.json;
#   * a backdrop-pattern window (see Start-Pattern) belongs to this script; nothing else is captured.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\nekominal.exe'),
    [string]$OutDir = (Join-Path $PSScriptRoot 'out'),
    [string]$WorkDir = (Join-Path ([IO.Path]::GetTempPath()) 'nekominal-drive'),
    [string]$Name = 'app',
    [hashtable]$Settings = @{},
    [hashtable]$Env = @{},
    [scriptblock]$Script = $null,
    [int]$SettleMs = 3500,
    [switch]$Maximize,
    [switch]$Pattern,
    [switch]$NoShot
)
$ErrorActionPreference = 'Stop'
[Environment]::SetEnvironmentVariable('NO_COLOR', $null)
. (Join-Path $PSScriptRoot 'drive-lib.ps1')
. (Join-Path $PSScriptRoot 'app-lib.ps1')
New-Item -ItemType Directory -Force $OutDir, $WorkDir | Out-Null
$out = (Resolve-Path $OutDir).Path
$work = (Resolve-Path $WorkDir).Path
$log = Join-Path $work "$Name.log"
if (Test-Path -LiteralPath $log) { [IO.File]::Delete($log) }
$appData = Join-Path $work "$Name-appdata"
if (Test-Path -LiteralPath $appData) { Remove-Item -Recurse -Force $appData }
$cfg = Join-Path $appData 'Nekominal'
New-Item -ItemType Directory -Force $cfg | Out-Null
if ($Settings.Count -gt 0) { ($Settings | ConvertTo-Json -Depth 6) | Set-Content -Encoding UTF8 (Join-Path $cfg 'settings.json') }
$script:LogFile = $log
$Env['NEKOMINAL_LOG'] = $log
$Env['APPDATA'] = $appData

# One GUI run at a time on this desktop: two runs would type into each other's window. The mutex is machine-wide, so
# a second run (another shell, another agent) waits for the first to finish instead of interleaving its input.
$gui = New-Object System.Threading.Mutex($false, 'Global\Nekominal.GuiDrive')
try { $locked = $gui.WaitOne([TimeSpan]::FromMinutes(15)) } catch [System.Threading.AbandonedMutexException] { $locked = $true }
if (-not $locked) { throw 'another GUI run has held the lock for 15 minutes' }
try {
    if ($Pattern) { Start-Pattern }
    Start-App -Exe (Resolve-Path $Exe).Path -Log '' -Layout '' -Env $Env -SettleMs $SettleMs
    if ($Pattern) { Place-PatternUnderApp -Margin 90 }
    if ($Maximize) { [void][D]::ShowWindow($script:Hwnd, 3); Start-Sleep -Milliseconds 2500 }
    if ($Script) { & $Script }
    Start-Sleep -Milliseconds 600
    if (-not $NoShot) { if ($Pattern) { Shot-WithPattern (Join-Path $out "$Name.png") } else { Shot-Safe (Join-Path $out "$Name.png") } }
} finally {
    Stop-App
    if ($Pattern) { Stop-Pattern }
    $gui.ReleaseMutex()
}
if (Test-Path $log) { Get-Content $log -Tail 12 }
