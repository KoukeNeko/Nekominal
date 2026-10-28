# Builds the release executables (Nekominal.exe and updater.exe) and packs them, with the ConPTY pair they need, into dist/Nekominal-<version>-windows-<arch>.zip
# and a checksum file beside it. Run from any directory; cargo has to be on PATH.
#
#   scripts\package.ps1               # x64, version of the workspace
#   scripts\package.ps1 -Arch arm64   # ARM64, built for aarch64-pc-windows-msvc (needs the MSVC ARM64 build tools)
#   scripts\package.ps1 -NoBuild      # pack the executable that is already built
param(
    [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($manifest -notmatch '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw 'No workspace version in Cargo.toml.' }
$version = $Matches[1]

& (Join-Path $PSScriptRoot 'fetch-conpty.ps1') -Arch $Arch

# x64 stays in target\release, where a plain cargo build puts it.
$target, $release = if ($Arch -eq 'arm64') {
    @('--target', 'aarch64-pc-windows-msvc'), (Join-Path $root 'target\aarch64-pc-windows-msvc\release')
} else { @(), (Join-Path $root 'target\release') }

if (-not $NoBuild) {
    Push-Location $root
    try {
        cargo build --release --locked -p nekominal-shell -p nekominal-updater @target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
}

$files = @{
    'Nekominal.exe'   = Join-Path $release 'nekominal-next.exe'
    'conpty.dll'      = Join-Path $release 'conpty.dll'
    'OpenConsole.exe' = Join-Path $release 'OpenConsole.exe'
    # Updates the app while the programs of its tabs keep running (crates/nekominal-updater). Not named Nekominal: the
    # Scoop manifest stops every process of that name before it installs.
    'updater.exe'     = Join-Path $release 'updater.exe'
}
foreach ($file in $files.Values) {
    if (-not (Test-Path $file)) { throw "Missing $file. Build first, without -NoBuild." }
}

$name = "Nekominal-$version-windows-$Arch"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($entry in $files.GetEnumerator()) {
    Copy-Item $entry.Value (Join-Path $stage $entry.Key)
}
foreach ($doc in 'README.md', 'LICENSE', 'THIRD-PARTY-NOTICES.md') {
    Copy-Item (Join-Path $root $doc) (Join-Path $stage $doc)
}

# The files at the top of the archive, so that Scoop and a plain unzip both find the executable at once.
$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -Path "$zip.sha256" -Value "$hash  $name.zip" -Encoding ascii
Write-Host "$zip"
Write-Host "sha256 $hash"
