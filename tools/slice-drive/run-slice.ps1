# Drives the vertical slice (crates/nekominal-slice) with real SendInput and guarded screenshots.
#
#   & .\tools\slice-drive\run-slice.ps1 -Exe target\release\slice.exe -Name base `
#       -Env @{ SLICE_STDIN = 'echo hi\r'; SLICE_STDIN_MS = '2500'; SLICE_DUMP_MS = '6000' }
#
# Call it with & in the same pwsh process (a hashtable argument does not survive `pwsh -File`).
#
# Safety rules (learned the hard way, see critique.md section 6):
#   * input is sent only while the foreground window belongs to the started process (drive-lib Assert-Foreground);
#   * a screenshot is taken only when the foreground window AND the pixels under the capture area belong to the
#     test process (Shot-Safe); otherwise it is skipped. A plain CopyFromScreen captured another application's
#     window once and that image had to be deleted;
#   * NO_COLOR is removed from the child environment: the Claude Code host exports NO_COLOR=1 and pwsh then
#     strips every ANSI sequence from Write-Host output, which looks exactly like a renderer colour bug.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\slice.exe'),
    [string]$OutDir = (Join-Path $PSScriptRoot 'out'),
    [string]$Name = 'base',
    [hashtable]$Env = @{},
    [scriptblock]$Script = $null,
    [int]$SettleMs = 3500,
    [switch]$Maximize,
    [switch]$ShotBefore,
    [int]$StressCycles = 0
)
$ErrorActionPreference = 'Stop'
[Environment]::SetEnvironmentVariable('NO_COLOR', $null)
. (Join-Path $PSScriptRoot 'drive-lib.ps1')
New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = (Resolve-Path $OutDir).Path
$log = Join-Path $out "$Name.log"
if (Test-Path -LiteralPath $log) { [IO.File]::Delete($log) }
$Env['NEKOMINAL_LOG'] = $log

function Shot-Safe([string]$Path) {
    for ($k = 0; $k -lt 6; $k++) {
        $r = Win-Rect
        $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
        if ([D]::ForegroundPid() -eq [uint32]$script:App.Id -and [D]::PidAt($cx, $cy) -eq [uint32]$script:App.Id -and [D]::PidAt($r.Left + 200, $r.Top + 300) -eq [uint32]$script:App.Id) { return (Shot $Path) }
        Start-Sleep -Milliseconds 500
    }
    'shot skipped: another window is on top of the test window'
}

Start-App -Exe (Resolve-Path $Exe).Path -Log '' -Layout '' -Env $Env -SettleMs $SettleMs
try {
    if ($ShotBefore) { Shot-Safe (Join-Path $out "$Name-before.png") }
    if ($Maximize) { [void][D]::ShowWindow($script:Hwnd, 3); Start-Sleep -Milliseconds 2500 }
    for ($i = 0; $i -lt $StressCycles; $i++) { [void][D]::ShowWindow($script:Hwnd, 9); Start-Sleep -Milliseconds 700; [void][D]::ShowWindow($script:Hwnd, 3); Start-Sleep -Milliseconds 700 }
    if ($Script) { & $Script }
    Start-Sleep -Milliseconds 800
    Shot-Safe (Join-Path $out "$Name.png")
} finally {
    Stop-App
}
Get-Content $log -Tail 60
