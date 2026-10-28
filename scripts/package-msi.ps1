# Builds dist/Nekominal-<version>-windows-<arch>.msi, and a checksum file beside it, from the folder that scripts\package.ps1
# staged in dist/. Run package.ps1 first, with the same -Arch. Needs the WiX 5 tool (`dotnet tool install --global wix --version 5.0.2`).
#
#   scripts\package-msi.ps1               # x64
#   scripts\package-msi.ps1 -Arch arm64   # ARM64
#   scripts\package-msi.ps1 -Wix C:\tools\wix.exe -Stage D:\somewhere\files -Version 0.2.4   # another place, another version
param(
    [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
    [string]$Wix = 'wix',
    [string]$Stage,
    [string]$Version
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
if (-not $Version) {
    $manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    if ($manifest -notmatch '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw 'No workspace version in Cargo.toml.' }
    $Version = $Matches[1]
}
# An MSI version is three numbers: a pre-release or build suffix of the semver would be refused.
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "The version $Version is not three numbers; an MSI cannot carry it." }

$name = "Nekominal-$Version-windows-$Arch"
$dist = Join-Path $root 'dist'
if (-not $Stage) { $Stage = Join-Path $dist $name }
if (-not (Test-Path (Join-Path $Stage 'Nekominal.exe'))) { throw "No Nekominal.exe in $Stage. Run scripts\package.ps1 first." }

$msi = Join-Path $dist "$name.msi"
New-Item -ItemType Directory -Force $dist | Out-Null
if (Test-Path $msi) { Remove-Item $msi -Force }

& $Wix build (Join-Path $root 'distribution\nekominal.wxs') -arch $Arch `
    -d "Version=$Version" -d "Source=$Stage" -d "Icon=$(Join-Path $root 'assets\nekominal.ico')" -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed with exit code $LASTEXITCODE." }

$hash = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -Path "$msi.sha256" -Value "$hash  $name.msi" -Encoding ascii
Write-Host "$msi"
Write-Host "sha256 $hash"
