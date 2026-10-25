# Writes the Scoop manifest of a release. The release workflow runs it and commits the file to the bucket
# (KoukeNeko/scoop-bucket, bucket\nekominal.json).
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$Sha256,
    [Parameter(Mandatory)][string]$Arm64Url,
    [Parameter(Mandatory)][string]$Arm64Sha256,
    [Parameter(Mandatory)][string]$Output
)

$ErrorActionPreference = 'Stop'

foreach ($hash in $Sha256, $Arm64Sha256) {
    if ($hash -notmatch '^[0-9a-fA-F]{64}$') { throw "Not a SHA-256: $hash" }
}

# The running window is stopped first: Scoop replaces the files of the version in place.
$stop = 'Get-Process Nekominal -ErrorAction SilentlyContinue | Stop-Process -Force -PassThru | Wait-Process'

# Uninstalling (not updating, which runs the same hook) takes out the entries of Explorer's menu the app wrote.
$unregister = "if (`$cmd -eq 'uninstall') { Remove-Item 'HKCU:\Software\Classes\Directory\shell\Nekominal', 'HKCU:\Software\Classes\Directory\Background\shell\Nekominal' -Recurse -ErrorAction SilentlyContinue }"

$manifest = [ordered]@{
    version      = $Version
    description  = 'A Windows terminal in Rust for coding agents: Claude Code, including its full-screen mode, runs in it'
    homepage     = 'https://github.com/KoukeNeko/scoop-bucket#nekominal'
    license      = 'MIT'
    architecture = [ordered]@{
        '64bit' = [ordered]@{ url = $Url; hash = $Sha256.ToLowerInvariant() }
        'arm64' = [ordered]@{ url = $Arm64Url; hash = $Arm64Sha256.ToLowerInvariant() }
    }
    bin           = 'Nekominal.exe'
    shortcuts     = @(, @('Nekominal.exe', 'Nekominal'))
    pre_install   = $stop
    pre_uninstall = @($stop, $unregister)
}

$json = $manifest | ConvertTo-Json -Depth 6
# The other manifests of the bucket are indented by four spaces.
$json = [regex]::Replace($json, '(?m)^( +)', { param($m) ' ' * ($m.Groups[1].Length * 2) })
[IO.File]::WriteAllText($Output, $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "Wrote $Output"
