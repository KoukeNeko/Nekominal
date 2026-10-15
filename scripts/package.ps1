# Builds the release executable and packs it, with the ConPTY pair it needs, into dist/Nekominal-<version>-windows-x64.zip
# and a checksum file beside it. Run from any directory; cargo has to be on PATH.
#
#   scripts\package.ps1             # version of the workspace
#   scripts\package.ps1 -NoBuild    # pack the executable that is already built
param(
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($manifest -notmatch '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw 'No workspace version in Cargo.toml.' }
$version = $Matches[1]

& (Join-Path $PSScriptRoot 'fetch-conpty.ps1')

if (-not $NoBuild) {
    Push-Location $root
    try {
        cargo build --release --locked -p nekominal-shell
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
}

$release = Join-Path $root 'target\release'
$files = @{
    'Nekominal.exe'   = Join-Path $release 'nekominal-next.exe'
    'conpty.dll'      = Join-Path $release 'conpty.dll'
    'OpenConsole.exe' = Join-Path $release 'OpenConsole.exe'
}
foreach ($file in $files.Values) {
    if (-not (Test-Path $file)) { throw "Missing $file. Build first, without -NoBuild." }
}

$name = "Nekominal-$version-windows-x64"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($entry in $files.GetEnumerator()) {
    Copy-Item $entry.Value (Join-Path $stage $entry.Key)
}
Copy-Item (Join-Path $root 'README.md') (Join-Path $stage 'README.md')

# The files at the top of the archive, so that Scoop and a plain unzip both find the executable at once.
$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -Path "$zip.sha256" -Value "$hash  $name.zip" -Encoding ascii
Write-Host "$zip"
Write-Host "sha256 $hash"
