<#
.SYNOPSIS
  Builds Nekominal when needed and opens it, so it can be looked at.

.DESCRIPTION
  Checks the ConPTY pair, runs `cargo build --release -p nekominal-shell`
  when the executable is missing or older than the sources, and starts target\release\nekominal-next.exe.

  Without options the app opens with your own settings and returns at once. Any of -Theme, -Backdrop, -Opacity and
  -Language (or -Isolated) opens it with a private settings file and session in a temporary folder instead, waits
  until the window is closed and deletes the folder: your settings are neither read nor changed.

.EXAMPLE
  .\scripts\preview.ps1
  Opens Nekominal with your settings.

.EXAMPLE
  .\scripts\preview.ps1 -Theme Nord -Backdrop mica -Opacity 80
  Opens it as if those were the settings, without touching the real ones.

.EXAMPLE
  .\scripts\preview.ps1 -Isolated -Language en-US -Log
  Opens it with default settings in English and keeps the app log at %TEMP%\nekominal-preview.log.

.EXAMPLE
  .\scripts\preview.ps1 -DryRun
  Prints what would be built and started, and does neither.
#>
[CmdletBinding()]
param(
    # Run cargo even when the executable looks current.
    [switch]$Rebuild,
    # Never run cargo; stop when there is no executable.
    [switch]$NoBuild,
    # Private settings and session in a temporary folder (implied by -Theme, -Backdrop, -Opacity and -Language).
    [switch]$Isolated,
    # A colour scheme by the name the settings drawer shows, for example "Tokyo Night" or "Nord".
    [string]$Theme,
    [ValidateSet('acrylic', 'mica', 'mica-alt', 'none')]
    [string]$Backdrop,
    # Background opacity in percent; the app keeps it between 20 and 100.
    [ValidateRange(20, 100)]
    [int]$Opacity,
    [ValidateSet('auto', 'zh-TW', 'en-US')]
    [string]$Language,
    # Write the app log to %TEMP%\nekominal-preview.log and print where it is.
    [switch]$Log,
    # Remove CLAUDE*, ANTHROPIC*, MCP_* and NO_COLOR from the environment the app (and the shells in it) get.
    [switch]$CleanEnv,
    # Print what would happen and do nothing.
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\nekominal-next.exe'
$exeDir = Split-Path -Parent $exe
$resources = Join-Path $root 'crates\nekominal-pty\resources'
$logPath = Join-Path ([IO.Path]::GetTempPath()) 'nekominal-preview.log'

function Say([string]$text) { Write-Host $text }
function Fail([string]$text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }

# --- the ConPTY pair beside the executable ----------------------------------------------------------------------------
$pairMissing = -not ((Test-Path (Join-Path $resources 'conpty.dll')) -and (Test-Path (Join-Path $resources 'OpenConsole.exe')))
if ($pairMissing) {
    Say 'The ConPTY pair is not in crates\nekominal-pty\resources.'
    if ($DryRun) { Say '  (would run scripts\fetch-conpty.ps1)' }
    else { & (Join-Path $PSScriptRoot 'fetch-conpty.ps1') }
}

# --- the build ---------------------------------------------------------------------------------------------------------
$running = @(Get-Process -Name nekominal-next -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })
$reason = $null
if ($Rebuild) { $reason = '-Rebuild' }
elseif (-not (Test-Path $exe)) { $reason = 'there is no executable yet' }
elseif (-not $NoBuild) {
    $sources = @(Get-ChildItem -Path (Join-Path $root 'crates') -Recurse -File -Include '*.rs', '*.hlsl', '*.toml' |
            Where-Object { $_.FullName -notmatch '\\target\\' }) + @(Get-Item (Join-Path $root 'Cargo.toml'), (Join-Path $root 'Cargo.lock'))
    $newest = $sources | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($newest.LastWriteTime -gt (Get-Item $exe).LastWriteTime) {
        $reason = "$($newest.FullName.Substring($root.Length + 1)) is newer than the executable"
    }
}
if ($NoBuild -and -not (Test-Path $exe)) { Fail "there is no $exe and -NoBuild was given." }

if ($reason -and -not $NoBuild) {
    Say "Build needed: $reason."
    if ($running.Count -gt 0) { Fail "Nekominal is running (pid $($running[0].Id)) and its executable cannot be replaced: close it first." }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        $candidates = @()
        if ($env:CARGO_HOME) { $candidates += (Join-Path $env:CARGO_HOME 'bin\cargo.exe') }
        $candidates += (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe')
        $candidates += (Join-Path $env:USERPROFILE 'scoop\persist\rustup-msvc\.cargo\bin\cargo.exe')
        $found = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
        if (-not $found) { Fail 'cargo was not found. Install Rust (rustup) with the MSVC toolchain and run this again.' }
        $bin = Split-Path -Parent $found
        if (-not $DryRun) {
            $env:PATH = "$bin;$env:PATH"
            if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Split-Path -Parent $bin }
            $rustup = Join-Path (Split-Path -Parent $env:CARGO_HOME) '.rustup'
            if (-not $env:RUSTUP_HOME -and (Test-Path $rustup)) { $env:RUSTUP_HOME = $rustup }
        }
        $cargo = $found
        Say "cargo: $found (not on PATH)"
    }
    if ($DryRun) { Say '  (would run: cargo build --release -p nekominal-shell)' }
    else {
        Push-Location $root
        try {
            & cargo build --release -p nekominal-shell
            if ($LASTEXITCODE -ne 0) { Fail "cargo build failed (exit code $LASTEXITCODE)." }
        } finally { Pop-Location }
    }
} elseif (Test-Path $exe) {
    Say "Executable is current: $exe"
}

# The build copies the pair next to the executable; a copied folder may not have it.
if (-not $DryRun) {
    foreach ($name in 'conpty.dll', 'OpenConsole.exe') {
        $target = Join-Path $exeDir $name
        if (-not (Test-Path $target)) { Copy-Item (Join-Path $resources $name) $target }
    }
}

# --- settings of the run -----------------------------------------------------------------------------------------------
$overrides = $PSBoundParameters.ContainsKey('Theme') -or $PSBoundParameters.ContainsKey('Backdrop') -or
    $PSBoundParameters.ContainsKey('Opacity') -or $PSBoundParameters.ContainsKey('Language')
if ($overrides) { $Isolated = [switch]::new($true) }

$settings = [ordered]@{}
if ($PSBoundParameters.ContainsKey('Theme')) { $settings['theme'] = $Theme }
if ($PSBoundParameters.ContainsKey('Language')) { $settings['language'] = $Language }
if ($PSBoundParameters.ContainsKey('Backdrop') -or $PSBoundParameters.ContainsKey('Opacity')) {
    $appearance = [ordered]@{}
    if ($PSBoundParameters.ContainsKey('Backdrop')) { $appearance['backdrop'] = $Backdrop }
    if ($PSBoundParameters.ContainsKey('Opacity')) { $appearance['opacity'] = [math]::Round($Opacity / 100.0, 2) }
    $settings['appearance'] = $appearance
}

# Environment changes of the run: name -> value, $null removes the variable.
$changes = [ordered]@{}
$profileDir = $null
if ($Isolated) {
    $profileDir = Join-Path ([IO.Path]::GetTempPath()) ('nekominal-preview-' + [Guid]::NewGuid().ToString('N'))
    $changes['APPDATA'] = Join-Path $profileDir 'appdata'
}
if ($Log) { $changes['NEKOMINAL_LOG'] = $logPath }
if ($CleanEnv) {
    foreach ($item in Get-ChildItem Env:) {
        if ($item.Name -match '^(CLAUDE|ANTHROPIC|MCP_)' -or $item.Name -eq 'NO_COLOR') { $changes[$item.Name] = $null }
    }
}

if ($DryRun) {
    Say ''
    Say 'Dry run: nothing was built or started.'
    Say "  executable      $exe ($(if (Test-Path $exe) { 'exists' } else { 'missing' }))"
    Say "  settings        $(if ($Isolated) { 'private, in a temporary folder' } else { 'yours (%APPDATA%\Nekominal)' })"
    if ($settings.Count -gt 0) { Say ('  settings.json   ' + ($settings | ConvertTo-Json -Depth 4 -Compress)) }
    foreach ($name in $changes.Keys) { if ($null -ne $changes[$name]) { Say "  environment     $name = $($changes[$name])" } }
    $removed = @($changes.Keys | Where-Object { $null -eq $changes[$_] })
    if ($removed.Count -gt 0) { Say "  environment     $($removed.Count) variables removed (CLAUDE*, ANTHROPIC*, MCP_*, NO_COLOR)" }
    Say "  after the start $(if ($Isolated -or $Log) { 'waits until the window is closed' } else { 'returns at once' })"
    exit 0
}

# --- start -------------------------------------------------------------------------------------------------------------
$saved = @{}
foreach ($name in $changes.Keys) { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }
try {
    if ($Isolated) {
        $config = Join-Path $changes['APPDATA'] 'Nekominal'
        New-Item -ItemType Directory -Force $config | Out-Null
        if ($settings.Count -gt 0) {
            [IO.File]::WriteAllText((Join-Path $config 'settings.json'), ($settings | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding $false))
        }
        Say "Private settings in $profileDir (deleted when the window is closed)."
    }
    if ($Log -and (Test-Path $logPath)) { Remove-Item -LiteralPath $logPath -Force }
    foreach ($name in $changes.Keys) { [Environment]::SetEnvironmentVariable($name, $changes[$name]) }
    $app = Start-Process -FilePath $exe -WorkingDirectory $exeDir -PassThru
} finally {
    foreach ($name in $changes.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
}

if ($Isolated -or $Log) {
    Say "Nekominal is open (pid $($app.Id)). Close its window to finish."
    $app.WaitForExit()
    if ($Isolated -and (Test-Path -LiteralPath $profileDir)) { Remove-Item -LiteralPath $profileDir -Recurse -Force }
    if ($Log) { Say "App log: $logPath" }
    Say "Nekominal exited with code $($app.ExitCode)."
} else {
    Say "Nekominal is open (pid $($app.Id))."
}
