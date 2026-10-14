# Fetches the version-matched conpty.dll + OpenConsole.exe pair (x64) from the
# Microsoft.Windows.Console.ConPTY NuGet package into crates/nekominal-pty/resources.
# The pair must come from the same package; a stale or mismatched pair crashes shells.
param(
    [string]$Version = '1.25.260930003',
    [string]$Sha256 = '02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root 'crates\nekominal-pty\resources'

$current = Join-Path $dest 'VERSION'
if ((Test-Path $current) -and ((Get-Content $current -Raw).Trim() -eq $Version) -and
    (Test-Path (Join-Path $dest 'conpty.dll')) -and (Test-Path (Join-Path $dest 'OpenConsole.exe'))) {
    Write-Host "ConPTY $Version already present."
    return
}

$work =Join-Path ([IO.Path]::GetTempPath()) ("nekominal-conpty-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null

try {
    $nupkg = Join-Path $work 'conpty.zip'
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$Version/microsoft.windows.console.conpty.$Version.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $nupkg -UseBasicParsing

    $actual = (Get-FileHash $nupkg -Algorithm SHA256).Hash
    if ($actual -ne $Sha256) {
        throw "SHA256 mismatch for ConPTY package $Version. Expected $Sha256, got $actual."
    }

    $extracted = Join-Path $work 'pkg'
    Expand-Archive $nupkg $extracted -Force

    $files = @{
        'conpty.dll'      = Join-Path $extracted 'runtimes\win-x64\native\conpty.dll'
        'OpenConsole.exe' = Join-Path $extracted 'build\native\runtimes\x64\OpenConsole.exe'
    }

    foreach ($file in $files.Values) {
        $signature = Get-AuthenticodeSignature $file
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notlike 'CN=Microsoft Corporation,*') {
            throw "Unexpected signature on ${file}: $($signature.Status) $($signature.SignerCertificate.Subject)"
        }
    }

    New-Item -ItemType Directory -Force $dest | Out-Null
    foreach ($name in $files.Keys) {
        Copy-Item $files[$name] (Join-Path $dest $name) -Force
    }
    Set-Content -Path (Join-Path $dest 'VERSION') -Value $Version -NoNewline
    Write-Host "ConPTY $Version installed to $dest"
}
finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
