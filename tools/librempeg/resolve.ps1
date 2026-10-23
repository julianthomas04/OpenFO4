param(
    [string]$WorkRoot = (Join-Path $env:TEMP 'openfo4-librempeg'),
    [string]$LockPath = (Join-Path $PSScriptRoot 'toolchain.lock.json')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$WorkRoot = [IO.Path]::GetFullPath($WorkRoot)
if ($WorkRoot -match '\s') { throw 'The isolated build directory must not contain whitespace.' }
if (Test-Path -LiteralPath $LockPath) { throw "A lock already exists: $LockPath. Preserve it; do not resolve dependencies again." }
$cache = Join-Path $WorkRoot 'cache'
$msys = Join-Path $WorkRoot 'msys64'
New-Item -ItemType Directory -Force -Path $cache | Out-Null
$base = [ordered]@{
    url = 'https://github.com/msys2/msys2-installer/releases/download/2026-09-27/msys2-base-x86_64-20260927.tar.xz'
    file = 'msys2-base-x86_64-20260927.tar.xz'
    sha256 = 'ea2f31a0b6ade63914ce441ffb022f0f6aa96982bfefa2326460a26d5fb01322'
}

function Get-Download([string]$Url, [string]$File, [string]$Expected = '') {
    if (-not (Test-Path -LiteralPath $File)) {
        Invoke-WebRequest -Uri $Url -OutFile "$File.download"
        Move-Item -LiteralPath "$File.download" -Destination $File
    }
    $hash = (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Expected -and $hash -ne $Expected) { throw "SHA256 mismatch: $File" }
    return $hash
}

Get-Download $base.url (Join-Path $cache $base.file) $base.sha256 | Out-Null
if (Test-Path -LiteralPath $msys) { throw "Use a fresh WorkRoot when resolving: $msys already exists." }
& tar.exe -xf (Join-Path $cache $base.file) -C $WorkRoot
if ($LASTEXITCODE -ne 0) { throw 'MSYS2 base extraction failed.' }
$bash = Join-Path $msys 'usr/bin/bash.exe'
$oldSystem = $env:MSYSTEM
$oldPath = $env:PATH
try {
    $env:MSYSTEM = 'UCRT64'
    $env:PATH = "$(Join-Path $msys 'ucrt64/bin');$(Join-Path $msys 'usr/bin');$oldPath"
    & $bash -lc 'true'
    if ($LASTEXITCODE -ne 0) { throw 'MSYS2 initialization failed.' }
    & $bash -lc 'pacman -Sy --noconfirm'
    if ($LASTEXITCODE -ne 0) { throw 'Package database download failed.' }
    $requested = @('make', 'diffutils', 'perl', 'mingw-w64-ucrt-x86_64-gcc', 'mingw-w64-ucrt-x86_64-nasm', 'mingw-w64-ucrt-x86_64-pkgconf')
    $packageNames = $requested -join ' '
    $urls = @(& $bash -lc "pacman -Sp --noconfirm --print-format '%l' $packageNames")
    if ($LASTEXITCODE -ne 0) { throw 'Package closure resolution failed.' }
    $urls = @($urls | Where-Object { $_ -match '^https?://' })
    if ($urls.Count -eq 0) { throw 'Package closure is empty.' }
    & $bash -lc "pacman -Sw --noconfirm $packageNames"
    if ($LASTEXITCODE -ne 0) { throw 'Signed package acquisition failed.' }
    $packages = @(
        foreach ($url in $urls) {
            $file = [IO.Path]::GetFileName(([uri]$url).AbsolutePath)
            $downloaded = Join-Path $msys "var/cache/pacman/pkg/$file"
            if (-not (Test-Path -LiteralPath $downloaded)) { throw "Resolved package was not cached: $file" }
            Copy-Item -LiteralPath $downloaded -Destination (Join-Path $cache $file)
            [ordered]@{ url = $url; file = $file; sha256 = (Get-FileHash -LiteralPath $downloaded -Algorithm SHA256).Hash.ToLowerInvariant() }
        }
    )
} finally {
    $env:MSYSTEM = $oldSystem
    $env:PATH = $oldPath
}

$sources = @(
    [ordered]@{ name = 'librempeg'; commit = '395e8d99bed806c77b188f77599f25ee82f8f3bd'; url = 'https://codeload.github.com/librempeg/librempeg/tar.gz/395e8d99bed806c77b188f77599f25ee82f8f3bd'; file = 'librempeg-395e8d99bed806c77b188f77599f25ee82f8f3bd.tar.gz' },
    [ordered]@{ name = 'x264'; commit = '0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee'; url = 'https://codeload.github.com/mirror/x264/tar.gz/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee'; file = 'x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz' }
)
foreach ($source in $sources) {
    $source.sha256 = Get-Download $source.url (Join-Path $cache $source.file)
    & tar.exe -tf (Join-Path $cache $source.file) | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Source download is not an archive: $($source.name)" }
}
$lock = [ordered]@{ schema = 1; base = $base; packages = $packages; sources = $sources }
$lock | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $LockPath -Encoding utf8
Write-Output "Frozen acquisition lock: $LockPath"
Write-Output "Cache: $cache"
Write-Output 'Review and commit the lock before building. No converter or game asset was executed.'
