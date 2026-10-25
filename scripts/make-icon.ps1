# Builds assets\nekominal.ico from the two drawings of the icon beside it: nekominal-small.svg (a thicker line, larger in its
# square) up to 32 px, where the title bar and the taskbar show it, and nekominal.svg above that. Each size is rendered by
# headless Chrome or Edge, then the PNGs are packed into one .ico (PNG entries, which Windows Vista and later read at every
# size).
param([string]$Out = (Join-Path $PSScriptRoot '..\assets\nekominal.ico'))
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$browser = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { throw 'Chrome or Edge is needed to render the icon.' }

$work = Join-Path ([IO.Path]::GetTempPath()) ('nekominal-icon-' + [guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
try {
    $sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
    $pngs = foreach ($size in $sizes) {
        $svg = if ($size -le 32) { 'nekominal-small.svg' } else { 'nekominal.svg' }
        $source = (Join-Path $root "assets\$svg") -replace '\\', '/'
        $page = Join-Path $work "$size.html"
        Set-Content $page "<!doctype html><html><body style='margin:0;background:transparent'><img src='file:///$source' width='$size' height='$size' style='display:block'></body></html>" -Encoding utf8
        $png = Join-Path $work "$size.png"
        & $browser --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 `
            --allow-file-access-from-files "--window-size=$size,$size" "--screenshot=$png" "file:///$($page -replace '\\', '/')" 2>$null | Out-Null
        for ($i = 0; $i -lt 50 -and -not (Test-Path $png); $i++) { Start-Sleep -Milliseconds 100 }
        if (-not (Test-Path $png)) { throw "no PNG for $size px" }
        [pscustomobject]@{ Size = $size; Bytes = [IO.File]::ReadAllBytes($png) }
    }

    # ICONDIR, then an ICONDIRENTRY per image, then the PNGs.
    $stream = New-Object IO.MemoryStream
    $w = New-Object IO.BinaryWriter $stream
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$pngs.Count)
    $offset = 6 + 16 * $pngs.Count
    foreach ($p in $pngs) {
        $edge = if ($p.Size -ge 256) { 0 } else { $p.Size }   # 0 means 256
        $w.Write([byte]$edge); $w.Write([byte]$edge); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$p.Bytes.Length); $w.Write([uint32]$offset)
        $offset += $p.Bytes.Length
    }
    foreach ($p in $pngs) { $w.Write($p.Bytes) }
    $w.Flush()
    New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
    [IO.File]::WriteAllBytes($Out, $stream.ToArray())
    "wrote $Out ($($pngs.Count) sizes, $($stream.Length) bytes)"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
