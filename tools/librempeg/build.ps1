param(
    [string]$WorkRoot = (Join-Path $env:TEMP 'openfo4-librempeg'),
    [string]$LockPath = (Join-Path $PSScriptRoot 'toolchain.lock.json'),
    [int]$Jobs = 4,
    [switch]$ResumeBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Jobs -lt 1 -or $Jobs -gt 64) { throw 'Jobs must be between 1 and 64.' }
$WorkRoot = [IO.Path]::GetFullPath($WorkRoot)
if ($WorkRoot -match '\s') { throw 'The isolated build directory must not contain whitespace.' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$cache = Join-Path $WorkRoot 'cache'
$lock = Get-Content -LiteralPath $LockPath -Raw | ConvertFrom-Json
if ($lock.schema -ne 1) { throw 'Unsupported toolchain lock schema.' }
function Get-Sha256([string] $Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

# Fetches a locked file into the cache: downloaded beside its place, verified against the lock's SHA-256 and only then moved in, so a
# failed or tampered download never becomes a cache file.
function Save-LockedFile($Entry, [string] $Path) {
    if ($Entry.url -notmatch '^https://') { throw "Locked file $($Entry.file) has no https URL to fetch it from." }
    New-Item -ItemType Directory -Force -Path (Split-Path $Path) | Out-Null
    $part = "$Path.part"
    if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part -Force }
    Write-Host "Fetching $($Entry.url)"
    try {
        Invoke-WebRequest -Uri $Entry.url -OutFile $part -UseBasicParsing
        if ((Get-Sha256 $part) -ne $Entry.sha256) { throw "SHA256 mismatch of the download of $($Entry.url)." }
        Move-Item -LiteralPath $part -Destination $Path -Force
    } finally {
        if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part -Force }
    }
}

foreach ($entry in @($lock.base) + @($lock.packages) + @($lock.sources)) {
    if ($entry.file -ne [IO.Path]::GetFileName($entry.file) -or $entry.sha256 -notmatch '^[0-9a-f]{64}$') { throw 'Invalid lock entry.' }
    $file = Join-Path $cache $entry.file
    if (-not (Test-Path -LiteralPath $file)) { Save-LockedFile $entry $file }
    elseif ((Get-Sha256 $file) -ne $entry.sha256) {
        Write-Warning "SHA256 mismatch of the cached $file; fetching it again."
        Save-LockedFile $entry $file
    }
}

$buildRoot = Join-Path $WorkRoot 'build'
if (-not $ResumeBuild) {
    if (Test-Path -LiteralPath $buildRoot) { throw "Use -ResumeBuild for the existing isolated toolchain: $buildRoot" }
    New-Item -ItemType Directory -Path $buildRoot | Out-Null
    & tar.exe -xf (Join-Path $cache $lock.base.file) -C $buildRoot
    if ($LASTEXITCODE -ne 0) { throw 'MSYS2 extraction failed.' }
}
$msys = Join-Path $buildRoot 'msys64'
$bash = Join-Path $msys 'usr/bin/bash.exe'
if ($ResumeBuild) {
    foreach ($required in @('msys64/ucrt64/bin/gcc.exe', 'sources/librempeg/configure', 'sources/x264/configure', 'prefix/lib/libx264.a')) {
        if (-not (Test-Path -LiteralPath (Join-Path $buildRoot $required))) { throw "Cannot resume incomplete build state (missing $required): delete $buildRoot or use a fresh -WorkRoot and build without -ResumeBuild." }
    }
} else {
    foreach ($source in $lock.sources) {
        if ($source.name -notin @('librempeg', 'x264')) { throw 'Unexpected source name.' }
        $target = Join-Path $buildRoot "sources/$($source.name)"
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        & tar.exe -xf (Join-Path $cache $source.file) -C $target --strip-components=1
        if ($LASTEXITCODE -ne 0) { throw "Source extraction failed: $($source.name)" }
    }
}
$shellScript = (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'build.sh') -Raw).Replace("`r`n", "`n")
[IO.File]::WriteAllText((Join-Path $buildRoot 'build.sh'), $shellScript, [Text.UTF8Encoding]::new($false))
$oldSystem = $env:MSYSTEM
$oldPath = $env:PATH
$oldWork = $env:OPENFO4_LIBREMPEG_WORK
$oldJobs = $env:OPENFO4_LIBREMPEG_JOBS
try {
    $env:MSYSTEM = 'UCRT64'
    $env:PATH = "$(Join-Path $msys 'ucrt64/bin');$(Join-Path $msys 'usr/bin');$oldPath"
    $env:OPENFO4_LIBREMPEG_WORK = $buildRoot
    $env:OPENFO4_LIBREMPEG_JOBS = [string]$Jobs
    & $bash -lc 'true'
    if ($LASTEXITCODE -ne 0) { throw 'MSYS2 initialization failed.' }
    if (-not $ResumeBuild) {
        foreach ($package in $lock.packages) {
            Copy-Item -LiteralPath (Join-Path $cache $package.file) -Destination (Join-Path $msys "var/cache/pacman/pkg/$($package.file)")
        }
        & $bash -lc 'pacman -U --noconfirm /var/cache/pacman/pkg/*.pkg.tar.*'
        if ($LASTEXITCODE -ne 0) { throw 'Offline locked toolchain installation failed.' }
    }
    & $bash -lc 'bash "$(cygpath -u "$OPENFO4_LIBREMPEG_WORK")/build.sh"' *> (Join-Path $buildRoot 'build.log')
    if ($LASTEXITCODE -ne 0) { throw "Librempeg build failed. See $buildRoot/build.log" }
} finally {
    $env:MSYSTEM = $oldSystem
    $env:PATH = $oldPath
    $env:OPENFO4_LIBREMPEG_WORK = $oldWork
    $env:OPENFO4_LIBREMPEG_JOBS = $oldJobs
}

$allowedDlls = @('ADVAPI32.dll','bcrypt.dll','CRYPT32.dll','GDI32.dll','IMM32.dll','KERNEL32.dll','msvcrt.dll','ntdll.dll','ole32.dll','OLEAUT32.dll','PSAPI.DLL','SHELL32.dll','SHLWAPI.dll','USER32.dll','USERENV.dll','VERSION.dll','WINMM.dll','WS2_32.dll','ucrtbase.dll')
foreach ($tool in @('ffmpeg', 'ffprobe')) {
    $imports = Get-Content -LiteralPath (Join-Path $buildRoot "$tool-imports.txt")
    foreach ($line in $imports) {
        if ($line -match 'DLL Name:\s*(\S+)') {
            $dll = $Matches[1]
            if ($dll -notin $allowedDlls -and $dll -notlike 'api-ms-win-*.dll') { throw "Unexpected runtime DLL $dll in $tool. Review and package its license/source before distribution." }
        }
    }
}
$bundleRoot = [IO.Path]::GetFullPath((Join-Path $repo 'build'))
if ((Test-Path -LiteralPath $bundleRoot) -and ((Get-Item -LiteralPath $bundleRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing to package beneath a linked build directory: $bundleRoot" }
$finalOutput = Join-Path $bundleRoot 'librempeg'
$output = Join-Path $bundleRoot ('librempeg.candidate-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $output, (Join-Path $output 'source'), (Join-Path $output 'build-info'), (Join-Path $output 'licenses') | Out-Null
foreach ($tool in @('ffmpeg.exe', 'ffprobe.exe')) { Copy-Item -LiteralPath (Join-Path $buildRoot "prefix/bin/$tool") -Destination $output }
foreach ($source in $lock.sources) { Copy-Item -LiteralPath (Join-Path $cache $source.file) -Destination (Join-Path $output 'source') }
Copy-Item -LiteralPath $LockPath -Destination (Join-Path $output 'build-info/toolchain.lock.json')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build.ps1'), (Join-Path $PSScriptRoot 'build.sh'), (Join-Path $PSScriptRoot 'resolve.ps1'), (Join-Path $PSScriptRoot 'README.md'), (Join-Path $PSScriptRoot 'THIRD_PARTY_NOTICES.txt') -Destination (Join-Path $output 'build-info')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'THIRD_PARTY_NOTICES.txt') -Destination $output
Copy-Item -LiteralPath (Join-Path $buildRoot 'sources/librempeg/COPYING.GPLv3') -Destination (Join-Path $output 'licenses/Librempeg-GPLv3.txt')
Copy-Item -LiteralPath (Join-Path $buildRoot 'sources/librempeg/COPYING.AGPLv3') -Destination (Join-Path $output 'licenses/Librempeg-AGPLv3.txt')
Copy-Item -LiteralPath (Join-Path $buildRoot 'sources/x264/COPYING') -Destination (Join-Path $output 'licenses/x264-COPYING.txt')
Copy-Item -Path (Join-Path $buildRoot '*-imports.txt'), (Join-Path $buildRoot 'ffmpeg-*.txt') -Destination (Join-Path $output 'build-info')
$licenseRoot = Join-Path $msys 'ucrt64/share/licenses'
if (Test-Path -LiteralPath $licenseRoot) { Copy-Item -LiteralPath $licenseRoot -Destination (Join-Path $output 'licenses/toolchain-runtime') -Recurse }
Get-ChildItem -LiteralPath $output -File -Recurse | ForEach-Object {
    [ordered]@{ file = [IO.Path]::GetRelativePath($output, $_.FullName); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'manifest.json') -Encoding utf8
$backup = Join-Path $bundleRoot ('librempeg.previous-' + [Guid]::NewGuid().ToString('N'))
foreach ($movePath in @($output, $finalOutput, $backup)) {
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($movePath)) -ne $bundleRoot) { throw 'Bundle promotion path escaped the build directory.' }
    if ((Test-Path -LiteralPath $movePath) -and ((Get-Item -LiteralPath $movePath).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing to move a linked bundle directory: $movePath" }
}
if (Test-Path -LiteralPath $finalOutput) { Move-Item -LiteralPath $finalOutput -Destination $backup }
try {
    Move-Item -LiteralPath $output -Destination $finalOutput
} catch {
    if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $finalOutput)) { Move-Item -LiteralPath $backup -Destination $finalOutput }
    throw
}
Write-Output "Built isolated bundle: $finalOutput"
if (Test-Path -LiteralPath $backup) { Write-Output "Previous bundle preserved: $backup" }
