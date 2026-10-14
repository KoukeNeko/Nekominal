# Regenerates crates/nekominal-winui-sys/src/bindings.rs in one generator run (about 2 s, bit-identical output).
#
#   pwsh -File tools/winui-gen/regen.ps1 -Winmd <dir with the 30 .winmd files>
#
# * `filter-gen.txt` is the committed, already pruned filter (1,059 lines). It is the only input besides the
#   metadata. When a new WinUI API is needed, add `Namespace::Type` lines to filter-extra.txt in the blueprint's
#   chrome-winui-src/gen and run its mkbindings.ps1 once (about 170 s: it drops unresolvable entries one by one),
#   then commit the new filter-gen.txt.
# * windows-bindgen pipes its output through `rustfmt` found on PATH; without it the generator panics with
#   "failed to format output with `rustfmt`".
# * The .winmd files (Windows App SDK 2.5.1: Foundation, WinUI, InteractiveExperiences, plus extras.winmd with
#   IWindowNative / ISwapChainPanelNative) are vendored under tools/winui-gen/winmd, see winmd.sha256.
param(
    [string]$Winmd = (Join-Path $PSScriptRoot 'winmd'),
    [string]$Out = (Join-Path $PSScriptRoot '..\..\crates\nekominal-winui-sys\src\bindings.rs')
)
$ErrorActionPreference = 'Stop'
if (-not (Get-Command rustfmt -ErrorAction SilentlyContinue)) { throw 'rustfmt is not on PATH' }
Push-Location $PSScriptRoot
try {
    cargo build --release --manifest-path Cargo.toml
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot 'target' }
    # the generator reads implements.txt and compose.txt from the current directory
    & (Join-Path $targetDir 'release\winui-gen.exe') (Resolve-Path $Winmd) $Out 'filter-gen.txt'
    if ($LASTEXITCODE -ne 0) { throw "generator failed with $LASTEXITCODE" }
    '{0}  {1:N0} bytes' -f $Out, (Get-Item $Out).Length
} finally { Pop-Location }
