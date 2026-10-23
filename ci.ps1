<#
.SYNOPSIS
    Runs the same checks as .github/workflows/ci.yml, in the same order, fail-fast.
    Never runs the game or the installer.
.DESCRIPTION
    The C# tests validate the emitted WGSL with naga-cli. Unless -SkipNaga is given, ci.ps1 requires the naga version that
    src/engine/Cargo.lock locks (tools/naga_version.ps1) and, when OPENFO4_NAGA is unset, exports it from the naga on PATH.
    OPENFO4_NAGA must be a file path; a bare command name is not supported. CI itself never skips these tests.
.PARAMETER SkipRust
    Skip all Rust checks.
.PARAMETER SkipDotnet
    Skip all C# checks.
.PARAMETER SkipPython
    Skip the Python (installer collision worker) tests.
.PARAMETER SkipGuard
    Skip the repo guard (no game-derived material in the tree) and its tests.
.PARAMETER SkipDeps
    Skip cargo-deny and cargo-machete.
.PARAMETER SkipNaga
    Do not require naga-cli. The WGSL validation tests then skip, unless OPENFO4_NAGA is already set: a preset OPENFO4_NAGA still runs
    them, and fails them when it does not point to an existing file.
.PARAMETER Coverage
    Run tests with coverage and write reports to coverage/ (reported only, there is no target).
.PARAMETER IncludeIgnored
    Also run the #[ignore]d Rust tests (GPU adapter required; the real-install tests need OPENFO4_INSTALL_DIR).
#>
[CmdletBinding()]
param(
    [switch]$SkipRust,
    [switch]$SkipDotnet,
    [switch]$SkipPython,
    [switch]$SkipGuard,
    [switch]$SkipDeps,
    [switch]$SkipNaga,
    [switch]$Coverage,
    [switch]$IncludeIgnored
)

$ErrorActionPreference = 'Stop'
$repoRoot = $PSScriptRoot
$engineDir = Join-Path $repoRoot 'src/engine'
$coverageDir = Join-Path $repoRoot 'coverage'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TERM_COLOR = 'always'

function Invoke-Step {
    param([string]$Title, [scriptblock]$Command, [string]$WorkingDirectory = $repoRoot)
    Write-Host ""
    Write-Host "=== $Title ===" -ForegroundColor Cyan
    Push-Location $WorkingDirectory
    try {
        & $Command
        if ($LASTEXITCODE -ne 0) {
            throw "Step failed with exit code ${LASTEXITCODE}: $Title"
        }
    }
    finally {
        Pop-Location
    }
}

function Assert-Tool {
    param([string]$Name, [string]$InstallHint)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required tool '$Name' not found. Install with: $InstallHint"
    }
}

function Use-LockedNaga {
    $locked = & (Join-Path $repoRoot 'tools/naga_version.ps1')
    if (-not $env:OPENFO4_NAGA) {
        Assert-Tool 'naga' "cargo install naga-cli --version $locked --locked"
        $env:OPENFO4_NAGA = (Get-Command naga).Source
    }
    if (-not (Test-Path -LiteralPath $env:OPENFO4_NAGA -PathType Leaf)) {
        throw "OPENFO4_NAGA points to '$env:OPENFO4_NAGA', which is not a file. Point it at the naga executable."
    }

    $installed = ((& $env:OPENFO4_NAGA --version | Out-String).Trim() -split '\s+')[-1]
    if ($installed -ne $locked) {
        throw "'$env:OPENFO4_NAGA' is naga $installed but src/engine/Cargo.lock locks naga $locked. Install the matching version with: cargo install naga-cli --version $locked --locked"
    }
}

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

if (-not $SkipGuard) {
    Assert-Tool 'python' 'https://www.python.org/downloads/'

    Invoke-Step 'repo-guard: no game-derived material in the tree' { python tools/repo_guard }
}

if (-not $SkipRust) {
    Assert-Tool 'cargo' 'https://rustup.rs'

    Invoke-Step 'rust-fmt: cargo fmt --check' { cargo fmt --all -- --check } $engineDir
    Invoke-Step 'rust-lint: cargo clippy' { cargo clippy --all-targets --locked -- -D warnings } $engineDir

    if ($Coverage) {
        Assert-Tool 'cargo-llvm-cov' 'cargo install cargo-llvm-cov --locked (and: rustup component add llvm-tools-preview)'
        $rustCoverage = Join-Path $coverageDir 'rust'
        New-Item -ItemType Directory -Force $rustCoverage | Out-Null
        Invoke-Step 'rust-test: cargo llvm-cov (tests + coverage)' {
            cargo llvm-cov --locked --lcov --output-path (Join-Path $rustCoverage 'lcov.info')
        } $engineDir
        Invoke-Step 'rust-test: coverage summary' {
            cargo llvm-cov report --summary-only | Tee-Object -FilePath (Join-Path $rustCoverage 'summary.txt')
        } $engineDir
    }
    else {
        Invoke-Step 'rust-test: cargo test' { cargo test --locked } $engineDir
    }

    if ($IncludeIgnored) {
        Invoke-Step 'rust-test: ignored tests (GPU adapter required)' { cargo test --locked -- --ignored } $engineDir
    }

    if (-not $SkipDeps) {
        Assert-Tool 'cargo-deny' 'cargo install cargo-deny --locked'
        Assert-Tool 'cargo-machete' 'cargo install cargo-machete --locked'
        Invoke-Step 'rust-deps: cargo deny check' { cargo deny --locked check } $engineDir
        Invoke-Step 'rust-deps: cargo machete' { cargo machete } $engineDir
    }
}

if (-not $SkipPython) {
    Assert-Tool 'python' 'https://www.python.org/downloads/'

    Invoke-Step 'python: unittest (installer collision worker)' { python -m unittest discover -s tests/installer_python -v }
    if (-not $SkipGuard) {
        Invoke-Step 'python: unittest (repo guard)' { python -m unittest discover -s tests/repo_guard -v }
    }
}

if (-not $SkipDotnet) {
    Assert-Tool 'dotnet' 'https://dotnet.microsoft.com/download'

    if (-not $SkipNaga) {
        Use-LockedNaga
    }

    Invoke-Step 'dotnet: restore tools' { dotnet tool restore }
    Invoke-Step 'dotnet: restore' { dotnet restore OpenFO4.sln --locked-mode }
    Invoke-Step 'dotnet: restore decoder' { dotnet restore tools/bink2-decoder/Bink2Decoder.csproj --locked-mode }
    Invoke-Step 'dotnet: restore installer release graph' { dotnet restore src/installer/installer.csproj --locked-mode -r win-x64 -p:OpenFO4Package=true }
    Invoke-Step 'dotnet: restore decoder release graph' { dotnet restore tools/bink2-decoder/Bink2Decoder.csproj --locked-mode -r win-x64 -p:SelfContained=true -p:PublishSingleFile=true }
    Invoke-Step 'dotnet: format --verify-no-changes' { dotnet format OpenFO4.sln --no-restore --verify-no-changes }
    Invoke-Step 'dotnet: build (Release, warnings are errors)' {
        dotnet build OpenFO4.sln -c Release --no-restore -warnaserror
    }

    if ($Coverage) {
        $dotnetCoverage = Join-Path $coverageDir 'dotnet'
        if (Test-Path $dotnetCoverage) { Remove-Item -Recurse -Force $dotnetCoverage }
        Invoke-Step 'dotnet: test with coverage' {
            dotnet test OpenFO4.sln -c Release --no-build --collect:"XPlat Code Coverage" --results-directory (Join-Path $dotnetCoverage 'raw')
        }
        Invoke-Step 'dotnet: coverage report' {
            dotnet tool run reportgenerator "-reports:$dotnetCoverage/raw/**/coverage.cobertura.xml" "-targetdir:$dotnetCoverage/report" '-reporttypes:MarkdownSummaryGithub;TextSummary;Cobertura'
            if ($LASTEXITCODE -eq 0) { Get-Content (Join-Path $dotnetCoverage 'report/Summary.txt') }
        }
    }
    else {
        Invoke-Step 'dotnet: test' { dotnet test OpenFO4.sln -c Release --no-build }
    }
}

$stopwatch.Stop()
Write-Host ""
Write-Host ("All checks passed in {0:mm\:ss}." -f $stopwatch.Elapsed) -ForegroundColor Green
