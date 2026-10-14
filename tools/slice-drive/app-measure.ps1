# Numbers for the application: start-up, idle cost, memory, a 30,000 line flood, maximize/restore stress and many tabs.
# Everything runs in one nekominal.exe started through run-app.ps1 (GUI lock, private APPDATA, guarded input).
#
#   . .\tools\slice-drive\app-measure.ps1; Measure-App
#
# The numbers are printed as `name = value` lines. The app process is found by name: run-app.ps1 holds a machine-wide
# lock, so there is only one nekominal.exe of a driver run at a time.

function Get-AppStats {
    $p = Get-Process nekominal -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $p) { return $null }
    $p.Refresh()
    [pscustomobject]@{ Cpu = $p.TotalProcessorTime.TotalSeconds; Ws = [math]::Round($p.WorkingSet64 / 1MB); Private = [math]::Round($p.PrivateMemorySize64 / 1MB); Threads = $p.Threads.Count; Handles = $p.HandleCount }
}

function Measure-App {
    $global:NkMeasure = [ordered]@{}
    $run = Join-Path $PSScriptRoot 'run-app.ps1'
    $work = Join-Path $env:TEMP 'nk-measure'
    New-Item -ItemType Directory -Force $work | Out-Null
    & $run -Name measure -OutDir $work -WorkDir $work -SettleMs 3000 -NoShot -Script {
        Wait-Prompt
        $a = Get-AppStats
        "one tab, prompt up:        working set $($a.Ws) MB, private $($a.Private) MB, threads $($a.Threads), handles $($a.Handles)"
        # idle CPU over 10 s
        $c0 = (Get-AppStats).Cpu; Start-Sleep -Seconds 10; $c1 = (Get-AppStats).Cpu
        "idle CPU over 10 s:        $([math]::Round(($c1 - $c0) / 10 * 100, 2)) % of one core"
        # flood: 30,000 lines
        $t0 = [DateTime]::UtcNow
        Type-Line '1..30000 | ForEach-Object { "flood line $_" }' 30
        $done = Wait-Text 'flood line 30000' 90000
        "30,000 line flood:         finished=$done in $([math]::Round(([DateTime]::UtcNow - $t0).TotalSeconds, 1)) s"
        $a = Get-AppStats
        "after the flood:           working set $($a.Ws) MB, private $($a.Private) MB"
        # maximize / restore stress while idle after the flood
        $h = (Get-Process nekominal | Select-Object -First 1).MainWindowHandle
        for ($i = 0; $i -lt 6; $i++) { [void][D]::ShowWindow($h, 3); Start-Sleep -Milliseconds 900; [void][D]::ShowWindow($h, 9); Start-Sleep -Milliseconds 900 }
        $a = Get-AppStats
        "after 6 maximize/restore:  working set $($a.Ws) MB, private $($a.Private) MB, alive=$(-not (Get-Process nekominal -ErrorAction SilentlyContinue).HasExited)"
        # eight more tabs
        for ($i = 0; $i -lt 8; $i++) { Chord @(0x11, 0x10) 0x54; Start-Sleep -Milliseconds 1500 }
        Start-Sleep -Milliseconds 2000
        $a = Get-AppStats
        "nine tabs:                 working set $($a.Ws) MB, private $($a.Private) MB, threads $($a.Threads), handles $($a.Handles)"
        $c0 = (Get-AppStats).Cpu; Start-Sleep -Seconds 10; $c1 = (Get-AppStats).Cpu
        "idle CPU, nine tabs:       $([math]::Round(($c1 - $c0) / 10 * 100, 2)) % of one core"
    }
    $log = Join-Path $work 'measure.log'
    'log facts:'
    $lines = Get-Content $log
    "  swap chains created:       $(($lines | Select-String 'configured ' ).Count)"
    "  surfaces suspended:        $(($lines | Select-String 'surface suspended').Count)"
    "  first frame:               $(($lines | Select-String 'pane 1 first frame' | Select-Object -First 1).Line.Trim())"
    "  shell built:               $(($lines | Select-String 'shell built' | Select-Object -First 1).Line.Trim())"
    "  PANIC lines:               $(($lines | Select-String 'PANIC').Count)"
    # Only the OpenConsole.exe next to our exe counts: other terminals on this machine run their own.
    $left = Get-Process OpenConsole, nekominal -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '*Nekominal*' }
    "  processes left after exit: $(@($left).Count)"
}
