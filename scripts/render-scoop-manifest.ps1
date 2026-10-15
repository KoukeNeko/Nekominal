# Writes the Scoop manifest of a release. The release workflow runs it and commits the file to the bucket
# (KoukeNeko/scoop-bucket, bucket\nekominal.json).
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$Sha256,
    [Parameter(Mandatory)][string]$Output
)

$ErrorActionPreference = 'Stop'

if ($Sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "Not a SHA-256: $Sha256" }

# The running window is stopped first: Scoop replaces the files of the version in place.
$stop = 'Get-Process Nekominal -ErrorAction SilentlyContinue | Stop-Process -Force -PassThru | Wait-Process'

$manifest = [ordered]@{
    version      = $Version
    description  = 'A Windows terminal in Rust for coding agents: Claude Code, including its full-screen mode, runs in it'
    homepage     = 'https://github.com/KoukeNeko/scoop-bucket#nekominal'
    license      = 'Proprietary'
    architecture = [ordered]@{
        '64bit' = [ordered]@{ url = $Url; hash = $Sha256.ToLowerInvariant() }
    }
    bin           = 'Nekominal.exe'
    shortcuts     = @(, @('Nekominal.exe', 'Nekominal'))
    pre_install   = $stop
    pre_uninstall = $stop
}

$json = $manifest | ConvertTo-Json -Depth 6
# The other manifests of the bucket are indented by four spaces.
$json = [regex]::Replace($json, '(?m)^( +)', { param($m) ' ' * ($m.Groups[1].Length * 2) })
[IO.File]::WriteAllText($Output, $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "Wrote $Output"
