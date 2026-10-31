#Requires -Version 7
<#
.SYNOPSIS
    Builds the engine (release) and the installer into a self-contained installer exe that carries the game.

.DESCRIPTION
    1. Downloads the pinned Python (embeddable package) and Temurin JRE into build/cache, verifying their SHA-256.
    2. Builds the engine with `cargo build --release --locked` (skipped with -DownloadOnly). Builds FFDec (tools/ffdec/build.ps1) with
       its matching sources and Librempeg (tools/librempeg/build.ps1, resuming its toolchain in the temp directory when present).
    3. Stages the payload (game, Python, Java, ffdec, the installer's data files, licence notices) in build/payload and zips it.
    4. Publishes the installer as a self-contained single-file win-x64 exe with the payload embedded.
    The result is build/release/OpenFO4-installer-<Version>.exe. Nothing downloaded or built is committed (build/ is ignored).

.PARAMETER Version
    The release version: letters, digits, '.', '_' and '-', starting with a letter or digit (for example 0.1.0-alpha).
    It is passed to the engine build as OPENFO4_VERSION, stamps the installer and names the exe.

.PARAMETER DownloadOnly
    Only download and verify the runtimes and stage them; builds nothing.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $Version,

    [switch] $DownloadOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-ValidVersion([string] $Value) {
    # Same rule as InstallerVersion.Pattern in src/installer/resources/installer_version.cs.
    if ($Value -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]*\z') {
        throw "-Version must hold only letters, digits, '.', '_' and '-' and start with a letter or digit (for example 0.1.0-alpha), got '$Value'."
    }
}

Assert-ValidVersion $Version
if (-not $DownloadOnly) {
    $changes = git -C $PSScriptRoot status --porcelain --untracked-files=no
    if ($LASTEXITCODE -ne 0 -or $changes) { throw 'Build releases from a clean tracked checkout so packaged sources match the build.' }
}

$RepoRoot = $PSScriptRoot
$BuildRoot = Join-Path $RepoRoot 'build'
$CacheDir = Join-Path $BuildRoot 'cache'
$PayloadDir = Join-Path $BuildRoot 'payload'
$PayloadZip = Join-Path $BuildRoot 'payload.zip'
$PayloadIdFile = Join-Path $BuildRoot 'payload.id'
$PublishDir = Join-Path $BuildRoot 'publish'
$ReleaseDir = Join-Path $BuildRoot 'release'
$EngineDir = Join-Path $RepoRoot 'src/engine'
$InstallerProject = Join-Path $RepoRoot 'src/installer/installer.csproj'
$SourceStage = Join-Path $BuildRoot 'release-sources'
# Librempeg's MSYS2 toolchain cannot build below a path with whitespace, so it keeps its toolchain and objects outside the checkout.
$LibrempegWorkRoot = Join-Path ([IO.Path]::GetTempPath()) 'openfo4-librempeg'
$LicensingScript = Join-Path $RepoRoot 'tools/licensing/release.py'
$PythonCommand = Get-Command python -ErrorAction SilentlyContinue
if (-not $PythonCommand) { throw 'Python 3.11 or later is required for source packaging. Supply it on PATH.' }
$ReleasePython = $PythonCommand.Source

function Invoke-Licensing([string[]] $Arguments) {
    & $ReleasePython $LicensingScript @Arguments
    if ($LASTEXITCODE -ne 0) { throw 'Release source/notices packaging failed.' }
}

$Runtimes = @((Get-Content -LiteralPath (Join-Path $RepoRoot 'tools/licensing/runtime-inputs.json') -Raw | ConvertFrom-Json).runtimes)

function Write-Step([string] $Message) { Write-Host "==> $Message" -ForegroundColor Cyan }

function Invoke-Native([string] $Description, [scriptblock] $Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$Description failed with exit code $LASTEXITCODE." }
}

function Get-VerifiedDownload([pscustomobject] $Runtime) {
    $file = Join-Path $CacheDir ([System.IO.Path]::GetFileName(([uri] $Runtime.Url).LocalPath))
    if ((Test-Path $file) -and ((Get-FileHash $file -Algorithm SHA256).Hash -ne $Runtime.Sha256.ToUpperInvariant())) {
        Write-Warning "Cached $file does not match its pinned SHA-256; downloading it again."
        Remove-Item $file -Force
    }
    if (-not (Test-Path $file)) {
        Write-Step "Downloading $($Runtime.Name) $($Runtime.Label)"
        $partial = "$file.download"
        Invoke-WebRequest -Uri $Runtime.Url -OutFile $partial
        $actual = (Get-FileHash $partial -Algorithm SHA256).Hash
        if ($actual -ne $Runtime.Sha256.ToUpperInvariant()) {
            Remove-Item $partial -Force
            throw "SHA-256 of the downloaded $($Runtime.Name) is $actual, expected $($Runtime.Sha256). Refusing to use it."
        }
        Move-Item $partial $file
    }
    Write-Host "    $($Runtime.Name) $($Runtime.Label): SHA-256 verified"
    return $file
}

function Expand-Runtime([string] $Archive, [string] $Destination, [switch] $FlattenSingleFolder) {
    $scratch = "$Destination.unpacking"
    if (Test-Path $scratch) { Remove-Item $scratch -Recurse -Force }
    Expand-Archive -Path $Archive -DestinationPath $scratch
    $source = $scratch
    if ($FlattenSingleFolder) {
        $children = @(Get-ChildItem $scratch)
        if ($children.Count -ne 1 -or -not $children[0].PSIsContainer) { throw "Expected one top-level folder in $Archive." }
        $source = $children[0].FullName
    }
    Move-Item $source $Destination
    if (Test-Path $scratch) { Remove-Item $scratch -Recurse -Force }
}

function Get-PeImportedDlls([string] $Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x00004550) { throw "$Path is not a PE file." }
    $sectionCount = [BitConverter]::ToUInt16($bytes, $pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $pe + 20)
    $optional = $pe + 24
    $directories = $optional + $(if ([BitConverter]::ToUInt16($bytes, $optional) -eq 0x10b) { 96 } else { 112 })
    $sections = foreach ($i in 0..($sectionCount - 1)) {
        $at = $optional + $optionalSize + 40 * $i
        [pscustomobject]@{
            Rva  = [BitConverter]::ToUInt32($bytes, $at + 12)
            Size = [Math]::Max([BitConverter]::ToUInt32($bytes, $at + 8), [BitConverter]::ToUInt32($bytes, $at + 16))
            Raw  = [BitConverter]::ToUInt32($bytes, $at + 20)
        }
    }
    $toOffset = {
        param([uint32] $rva)
        $section = $sections | Where-Object { $rva -ge $_.Rva -and $rva -lt ($_.Rva + $_.Size) } | Select-Object -First 1
        if ($null -eq $section) { return -1 }
        return [int]($rva - $section.Rva + $section.Raw)
    }
    $readName = {
        param([int] $offset)
        $end = $offset
        while ($bytes[$end] -ne 0) { $end++ }
        return [System.Text.Encoding]::ASCII.GetString($bytes, $offset, $end - $offset).ToLowerInvariant()
    }
    $names = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($table in @(@{ Index = 1; Stride = 20; NameField = 12 }, @{ Index = 13; Stride = 32; NameField = 4 })) {
        $rva = [BitConverter]::ToUInt32($bytes, $directories + 8 * $table.Index)
        if ($rva -eq 0) { continue }
        $offset = & $toOffset $rva
        while ($offset -ge 0) {
            $nameRva = [BitConverter]::ToUInt32($bytes, $offset + $table.NameField)
            if ($nameRva -eq 0) { break }
            $nameOffset = & $toOffset $nameRva
            if ($nameOffset -lt 0) { break }
            [void] $names.Add((& $readName $nameOffset))
            $offset += $table.Stride
        }
    }
    return $names
}

function Assert-SelfContainedRuntime([string] $ExePath) {
    $crtPattern = '^(vcruntime\d+(_\d+)?|msvcp\d+(_\d+)?|concrt\d+|vccorlib\d+|vcomp\d+|ucrtbase(d)?|api-ms-win-crt-.+)\.dll$'
    $found = @(Get-PeImportedDlls $ExePath | Where-Object { $_ -match $crtPattern })
    if ($found.Count -gt 0) {
        throw "$ExePath imports the C runtime ($($found -join ', ')). The engine must link it statically: src/engine/.cargo/config.toml sets '-C target-feature=+crt-static', so check that RUSTFLAGS is not overriding it."
    }
    Write-Host '    openfo4.exe links the C runtime statically'
}

# The Windows DLLs every process can load; anything else the engine imports must travel beside it.
function Test-WindowsSystemDll([string] $Name) {
    $Name -match '^(api|ext)-ms-' -or (Test-Path -LiteralPath (Join-Path ([Environment]::SystemDirectory) $Name) -PathType Leaf)
}

function Assert-RuntimeDllsStaged([string] $ExePath, [string] $GameDir) {
    $missing = @(Get-PeImportedDlls $ExePath |
        Where-Object { -not (Test-WindowsSystemDll $_) -and -not (Test-Path -LiteralPath (Join-Path $GameDir $_) -PathType Leaf) })
    if ($missing.Count -gt 0) {
        throw "$ExePath imports DLLs that are not staged beside it: $($missing -join ', '). The engine may only depend on DLLs that ship with Windows; stage any other DLL it needs in Add-GameFiles."
    }
    Write-Host '    every DLL openfo4.exe imports is staged beside it or ships with Windows'
}

function Add-GameFiles([string] $GameDir) {
    $engineExe = $EngineExecutable
    if (-not (Test-Path $engineExe)) { throw "The engine build did not produce $engineExe." }
    New-Item -ItemType Directory -Force -Path $GameDir | Out-Null
    Copy-Item $engineExe $GameDir

    Assert-SelfContainedRuntime $engineExe
    Assert-RuntimeDllsStaged $engineExe $GameDir
}

function Copy-InstallerResources([string] $Destination) {
    $files = @(
        'src/installer/python/fo4_collision_worker.py',
        'src/installer/file_converters/esm_schema.sql',
        'src/installer/shader_translator/shader_manifest.json',
        'third_party/pynifly/bhk_autounpack.py',
        'third_party/pynifly/LICENSE',
        'third_party/pynifly/SOURCE.md'
    )
    foreach ($relative in $files) {
        $target = Join-Path $Destination $relative
        New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
        Copy-Item (Join-Path $RepoRoot $relative) $target
    }
    $ffdecSources = Join-Path $BuildRoot 'ffdec-package-sources'
    if (Test-Path -LiteralPath $ffdecSources) { Remove-Item -LiteralPath $ffdecSources -Recurse -Force }
    & (Join-Path $RepoRoot 'tools/ffdec/package.ps1') -Destination (Join-Path $Destination 'third_party/ffdec') `
        -SourceDestination $ffdecSources | Out-Host
    Copy-Librempeg (Join-Path $Destination 'third_party/librempeg')
    Copy-Bink2Decoder (Join-Path $Destination 'third_party/bink2-decoder')
}

# The decoder is a separate GPL-3.0-or-later program, never linked into the installer. It is published self-contained and as a single
# file for the same reason the installer is: the target machine need not have a .NET runtime.
function Copy-Bink2Decoder([string] $Destination) {
    $project = Join-Path $RepoRoot 'tools/bink2-decoder'
    $published = Join-Path $BuildRoot 'bink2-decoder'
    if (Test-Path $published) { Remove-Item $published -Recurse -Force }
    Invoke-Native 'dotnet publish (Bink2 decoder)' {
        dotnet publish (Join-Path $project 'Bink2Decoder.csproj') -c Release -r win-x64 --self-contained true -o $published `
            -p:PublishSingleFile=true -p:EnableCompressionInSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:DebugType=none `
            -p:RuntimeFrameworkVersion=8.0.32 -p:RestoreLockedMode=true `
            "-p:Version=$Version"
    }
    $decoder = Join-Path $published 'Bink2Decoder.exe'
    if (-not (Test-Path $decoder)) { throw "dotnet publish did not produce $decoder." }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item $decoder $Destination
    # The licence texts and the corresponding-source notes that travel with the executable, as with Librempeg's.
    foreach ($notice in 'LICENSE', 'COPYING.LGPLv2.1', 'THIRD_PARTY_NOTICES.md', 'DECODER_SOURCES.md') {
        $file = Join-Path $project $notice
        if (-not (Test-Path $file)) { throw "$file is missing." }
        Copy-Item $file $Destination
    }
}

function Copy-Librempeg([string] $Destination) {
    $built = Join-Path $BuildRoot 'librempeg'
    $ffmpeg = Join-Path $built 'ffmpeg.exe'
    $licenses = Join-Path $built 'licenses'
    foreach ($required in $ffmpeg, $licenses, (Join-Path $built 'source'), (Join-Path $built 'build-info'), (Join-Path $built 'manifest.json')) {
        if (-not (Test-Path $required)) { throw "$required is missing from the Librempeg build." }
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item $ffmpeg $Destination
    Copy-Item $licenses $Destination -Recurse
    $sourceTarget = Join-Path $SourceStage 'librempeg'
    New-Item -ItemType Directory -Force -Path $sourceTarget | Out-Null
    foreach ($relative in 'source', 'build-info', 'manifest.json', 'THIRD_PARTY_NOTICES.txt', 'licenses') {
        Copy-Item -LiteralPath (Join-Path $built $relative) -Destination $sourceTarget -Recurse
    }
    $notices = Join-Path $built 'THIRD_PARTY_NOTICES.txt'
    if (Test-Path $notices) { Copy-Item $notices $Destination }
}

function Add-ThirdPartyNotices([string] $Destination) {
    $notices = Get-Content (Join-Path $RepoRoot 'src/installer/packaging/THIRD_PARTY_NOTICES.txt') -Raw
    foreach ($runtime in $Runtimes) { $notices = $notices.Replace("{{$($runtime.Name.ToUpperInvariant())}}", $runtime.Label) }
    Set-Content -Path (Join-Path $Destination 'THIRD_PARTY_NOTICES.txt') -Value $notices -Encoding utf8NoBOM -NoNewline
}

# ---------------------------------------------------------------------------------------------------------------------

Write-Host 'Building release installer. This can take a while.' -ForegroundColor Yellow
New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
Write-Step 'Fetching the pinned runtimes'
$python = Get-VerifiedDownload $Runtimes[0]
$java = Get-VerifiedDownload $Runtimes[1]

if (-not $DownloadOnly) {
    Write-Step 'Building the engine release'
    Push-Location $EngineDir
    $previousVersion = $env:OPENFO4_VERSION
    $env:OPENFO4_VERSION = $Version
    try {
        Invoke-Native 'cargo build' { cargo build --release --locked --target x86_64-pc-windows-msvc }
        $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Cargo build output.' }
        $EngineExecutable = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release/openfo4.exe'
        if (-not (Test-Path -LiteralPath $EngineExecutable -PathType Leaf)) { throw "The engine build did not produce $EngineExecutable." }
    } finally {
        $env:OPENFO4_VERSION = $previousVersion
        Pop-Location
    }

    foreach ($project in $InstallerProject, (Join-Path $RepoRoot 'tools/bink2-decoder/Bink2Decoder.csproj')) {
        $packageMode = if ($project -eq $InstallerProject) { '-p:OpenFO4Package=true' } else { '-p:PublishSingleFile=true' }
        Invoke-Native 'Build the Windows .NET dependency graph' {
            dotnet build $project -c Release -r win-x64 --self-contained true -p:RestoreLockedMode=true $packageMode `
                -p:RuntimeFrameworkVersion=8.0.32 "-p:Version=$Version"
        }
    }
    Write-Step 'Building FFDec and its matching sources'
    & (Join-Path $RepoRoot 'tools/ffdec/build.ps1') | Out-Host
    Write-Step 'Building Librempeg'
    $librempegResume = Test-Path -LiteralPath (Join-Path $LibrempegWorkRoot 'build')
    & (Join-Path $RepoRoot 'tools/librempeg/build.ps1') -WorkRoot $LibrempegWorkRoot -ResumeBuild:$librempegResume | Out-Host
    Invoke-Licensing -Arguments @('prepare', '--repo', $RepoRoot)
}

Write-Step 'Staging the payload'
if (Test-Path $PayloadDir) { Remove-Item $PayloadDir -Recurse -Force }
New-Item -ItemType Directory -Path $PayloadDir | Out-Null
Expand-Runtime $python (Join-Path $PayloadDir 'runtime/python')
Expand-Runtime $java (Join-Path $PayloadDir 'runtime/jre') -FlattenSingleFolder
if (-not $DownloadOnly) {
    foreach ($runtime in $Runtimes) {
        Invoke-Native "Apply the exact $($runtime.Name) runtime exclusions" {
            & $ReleasePython (Join-Path $RepoRoot 'tools/licensing/runtime_curation.py') --repo $RepoRoot --runtime $runtime.Name `
                --destination (Join-Path $PayloadDir $runtime.Destination)
        }
    }
}
Add-ThirdPartyNotices $PayloadDir
if (-not $DownloadOnly) {
    if (Test-Path -LiteralPath $SourceStage) { Remove-Item -LiteralPath $SourceStage -Recurse -Force }
    Invoke-Licensing -Arguments @('stage', '--repo', $RepoRoot, '--payload', $PayloadDir, '--sources', $SourceStage, '--version', $Version)
    Copy-InstallerResources $PayloadDir
    Add-GameFiles (Join-Path $PayloadDir 'game')
    Invoke-Native 'Copy scoped Microsoft runtime notices' {
        & $ReleasePython (Join-Path $RepoRoot 'tools/licensing/runtime_legal.py') --repo $RepoRoot --payload $PayloadDir
    }
}

if ($DownloadOnly) {
    Write-Step "Done (-DownloadOnly): runtimes verified and staged in $PayloadDir"
    return
}

Write-Step 'Zipping the payload'
if (Test-Path $PayloadZip) { Remove-Item $PayloadZip -Force }
[System.IO.Compression.ZipFile]::CreateFromDirectory($PayloadDir, $PayloadZip, [System.IO.Compression.CompressionLevel]::Optimal, $false)
$payloadHash = (Get-FileHash $PayloadZip -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -Path $PayloadIdFile -Value $payloadHash -NoNewline -Encoding ascii
Write-Host ('    payload {0:N1} MB, SHA-256 {1}' -f ((Get-Item $PayloadZip).Length / 1MB), $payloadHash)

Write-Step "Publishing the installer $Version"
if (Test-Path $PublishDir) { Remove-Item $PublishDir -Recurse -Force }
Invoke-Native 'dotnet publish' {
    dotnet publish $InstallerProject -c Release -o $PublishDir `
        -p:OpenFO4Package=true -p:Version=$Version -p:PayloadZip=$PayloadZip -p:PayloadIdFile=$PayloadIdFile `
        -p:RuntimeFrameworkVersion=8.0.32 -p:RestoreLockedMode=true
}

$published = Join-Path $PublishDir 'installer.exe'
if (-not (Test-Path $published)) { throw "dotnet publish did not produce $published." }
$extras = @(Get-ChildItem $PublishDir -File | Where-Object { $_.Name -ne 'installer.exe' })
if ($extras.Count -gt 0) { throw "The publish folder holds more than the installer exe: $($extras.Name -join ', ')." }

New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
$final = Join-Path $ReleaseDir "OpenFO4-installer-$Version.exe"
if (Test-Path -LiteralPath $final) { throw "Refusing to overwrite release asset: $final" }
Copy-Item -LiteralPath $published -Destination $final
$finalHash = (Get-FileHash $final -Algorithm SHA256).Hash.ToLowerInvariant()
Invoke-Licensing -Arguments @('finalize', '--repo', $RepoRoot, '--sources', $SourceStage, '--installer', $final, '--output', $ReleaseDir, '--version', $Version)

Write-Step 'Done'
Write-Host ('    {0}' -f $final)
Write-Host ('    {0:N1} MB, SHA-256 {1}' -f ((Get-Item $final).Length / 1MB), $finalHash)
