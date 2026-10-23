# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
[CmdletBinding()]
param([switch]$Refresh, [string]$SourceBundle)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$lock = Get-Content (Join-Path $PSScriptRoot 'toolchain.lock.json') -Raw | ConvertFrom-Json
. (Join-Path $PSScriptRoot 'dependencies.ps1')
function Remove-JavaBlock([string]$Text, [string]$Anchor, [string]$Body) {
    $at = $Text.IndexOf($Anchor)
    if ($at -lt 0) { throw "Source patch anchor changed: $Anchor" }
    $open = $Text.IndexOf('{', $at)
    $depth = 1
    $close = $open + 1
    while ($close -lt $Text.Length -and $depth -gt 0) {
        if ($Text[$close] -eq '{') { $depth++ }
        if ($Text[$close] -eq '}') { $depth-- }
        $close++
    }
    if ($depth -ne 0) { throw "Unclosed source block: $Anchor" }
    return $Text.Substring(0, $open + 1) + "`n" + $Body + "`n    " + $Text.Substring($close - 1)
}
if ($SourceBundle) {
    $preferred = [IO.Path]::GetFullPath((Join-Path $SourceBundle 'preferred-form'))
    foreach ($required in 'src','dependency-sources','licenses') {
        if (-not (Test-Path (Join-Path $preferred $required) -PathType Container)) { throw "Incomplete preferred-form bundle: $required" }
    }
    $work = [IO.Path]::GetFullPath((Join-Path $repo 'build/ffdec-work'))
    if (Test-Path $work) {
        if (-not $Refresh -or ((Get-Item -LiteralPath $work).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Use -Refresh with an unlinked work directory' }
        Remove-Item -LiteralPath $work -Recurse -Force
    }
    New-Item -ItemType Directory -Force $work | Out-Null
    Copy-Item (Join-Path $preferred '*') $work -Recurse
    Get-ChildItem (Join-Path $work 'src') -Recurse -Filter *.java | ForEach-Object { '"' + $_.FullName.Replace('\', '/') + '"' } |
        Set-Content (Join-Path $work 'sources.args') -Encoding utf8NoBOM
    Write-Output 'Prepared offline preferred-form source bundle; no downloads or source patches applied.'
    return
}
$cache = Join-Path $repo 'build/cache/ffdec'
$archive = Join-Path $cache 'jpexs-version26.2.1.tar.gz'
$pin = $lock.upstream.sha256
New-Item -ItemType Directory -Force $cache | Out-Null
if (-not (Test-Path $archive)) {
    Invoke-WebRequest $lock.upstream.url -OutFile "$archive.partial"
    if ((Get-FileHash "$archive.partial").Hash.ToLowerInvariant() -ne $pin) { throw 'FFDec source hash mismatch' }
    Move-Item "$archive.partial" $archive
}
if ((Get-FileHash $archive).Hash.ToLowerInvariant() -ne $pin) { throw 'FFDec source hash mismatch' }
$upstream = Join-Path $cache 'jpexs-decompiler-version26.2.1'
if (-not (Test-Path $upstream)) {
    & tar.exe -xf $archive -C $cache
    if ($LASTEXITCODE) { throw 'Source extraction failed' }
}
$work = Join-Path $repo 'build/ffdec-work'
if (Test-Path $work) {
    if (-not $Refresh) { throw 'Use -Refresh to replace the prior FFDec working tree.' }
    if ([IO.Path]::GetFullPath($work) -ne (Join-Path $repo 'build/ffdec-work')) { throw 'Unsafe working directory' }
    if ((Get-Item -LiteralPath $work).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing linked work directory' }
    Remove-Item -LiteralPath $work -Recurse -Force
}
$source = Join-Path $work 'src'
New-Item -ItemType Directory -Force $source | Out-Null
Copy-Item (Join-Path $upstream 'libsrc/ffdec_lib/src/*') $source -Recurse
# The native registry/player sources are optional library functionality, not SVG/font code.
Remove-Item -LiteralPath (Join-Path $source 'com/jpexs/decompiler/flash/jna') -Recurse
foreach ($unused in 'com/jpexs/helpers/resource/Swf2Exe.bin','com/jpexs/helpers/resource/fd_air','com/jpexs/decompiler/flash/exporters/XamlExporterStub.zip') {
    Remove-Item -LiteralPath (Join-Path $source $unused) -Recurse
}
$helper = Join-Path $source 'com/jpexs/decompiler/flash/helpers/ImageHelper.java'
$text = [IO.File]::ReadAllText($helper)
$text = $text -replace '(?m)^import org\.monte\.media\.jpeg\.[^;]+;\r?\n', ''
$start = $text.IndexOf('        BufferedImage in;', $text.IndexOf('public static BufferedImage read(InputStream'))
$end = $text.IndexOf('        int type = in.getType();', $start)
if ($start -lt 0 -or $end -lt 0) { throw 'ImageHelper patch anchor changed' }
$replacement = @'
        BufferedImage in;
        try (ImageInputStream stream = ImageIO.createImageInputStream(input)) {
            in = ImageIO.read(stream);
        } catch (IOException failure) {
            openfo4.ExportDiagnostics.unsupported(failure);
            throw failure;
        }
        if (in == null) {
            IOException failure = new IOException("Image format unsupported by the OpenFO4 minimal exporter");
            openfo4.ExportDiagnostics.unsupported(failure);
            throw failure;
        }

'@
[IO.File]::WriteAllText($helper, $text.Substring(0, $start) + $replacement + $text.Substring($end))
$font = Join-Path $source 'com/jpexs/decompiler/flash/exporters/FontExporter.java'
$text = [IO.File]::ReadAllText($font) -replace '(?m)^import com\.google\.typography\.[^;]+;\r?\n', ''
# WOFF is outside the installer's required TTF contract; remove its unrelated sfntly dependency.
$needle = 'if (mode == FontExportMode.WOFF)'
for ($i = 0; $i -lt 2; $i++) {
    $at = $text.LastIndexOf($needle)
    if ($at -lt 0) { throw 'WOFF source patch anchor changed' }
    $tail = Remove-JavaBlock $text.Substring($at) $needle '            throw new IOException("WOFF is unsupported by the minimal TTF exporter");'
    $text = $text.Substring(0, $at) + $tail.Replace($needle, 'if (mode != FontExportMode.TTF)')
}
[IO.File]::WriteAllText($font, $text)
$frame = Join-Path $source 'com/jpexs/decompiler/flash/exporters/FrameExporter.java'
$text = [IO.File]::ReadAllText($frame) -replace '(?m)^import com\.jpexs\.images\.apng\.[^;]+;\r?\n', ''
$text = Remove-JavaBlock $text 'public static void makeAnimatedPng(' '        throw new IOException("Animated PNG is outside the minimal SVG exporter");'
[IO.File]::WriteAllText($frame, $text)
$swf = Join-Path $source 'com/jpexs/decompiler/flash/SWF.java'
$text = [IO.File]::ReadAllText($swf).Replace('import com.jpexs.debugger.flash.SWD;', '')
foreach ($method in 'public boolean generatePCodeSwdFile(File file, Map<String, Set<Integer>> breakpoints, String swfHash)', 'public boolean generateSwdFile(File file, Map<String, Set<Integer>> breakpoints, String swfHash)') {
    $text = Remove-JavaBlock $text $method '        throw new IOException("Debugger source export is outside the minimal SVG/font exporter");'
}
[IO.File]::WriteAllText($swf, $text)
$video = Join-Path $source 'com/jpexs/decompiler/flash/tags/DefineVideoStreamTag.java'
$text = [IO.File]::ReadAllText($video)
foreach ($method in 'public synchronized void toImage(', 'public void toSVG(') {
    $text = Remove-JavaBlock $text $method '        UnsupportedOperationException failure = new UnsupportedOperationException("Rendering embedded SWF video is unsupported by the minimal exporter"); openfo4.ExportDiagnostics.unsupported(failure); throw failure;'
}
[IO.File]::WriteAllText($video, $text)
foreach ($java in Get-ChildItem $source -Recurse -Filter *.java) {
    $text = [IO.File]::ReadAllText($java.FullName)
    if ($text.Contains('dev.matrixlab.webp4j')) {
        $text = $text.Replace('import dev.matrixlab.webp4j.WebPCodec;', 'import com.jpexs.decompiler.flash.helpers.UnsupportedWebP;')
        $text = $text.Replace('import dev.matrixlab.webp4j.gif.GifToWebPConfig;', 'import com.jpexs.decompiler.flash.helpers.UnsupportedWebP.Options;')
        $text = $text.Replace('WebPCodec.', 'UnsupportedWebP.').Replace('GifToWebPConfig', 'Options')
        [IO.File]::WriteAllText($java.FullName, $text)
    }
}
Copy-Item (Join-Path $PSScriptRoot 'src/*') $source -Recurse -Force
$lib = Join-Path $work 'lib'
New-Item -ItemType Directory -Force $lib | Out-Null
Prepare-DependencySources $repo $upstream $work $lock
$generators = Join-Path $work 'generator-inputs/ffdec/lexers'
New-Item -ItemType Directory -Force $generators | Out-Null
Copy-Item (Join-Path $upstream 'libsrc/ffdec_lib/lexers/*') $generators -Recurse
$legal = Join-Path $work 'licenses/ffdec'
New-Item -ItemType Directory -Force $legal | Out-Null
Copy-Item (Join-Path $upstream 'libsrc/ffdec_lib/license.txt') (Join-Path $legal 'LGPL-3.0.txt')
Copy-Item (Join-Path $upstream 'license.txt') (Join-Path $legal 'GPL-3.0.txt')
Copy-Item (Join-Path $repo 'LICENSE') (Join-Path $legal 'OpenFO4-Apache-2.0.txt')
# Only source and data from the library are compiled. No GUI, SDK SWCs or application assets are copied.
Get-ChildItem $source -Recurse -Filter *.java | ForEach-Object { '"' + $_.FullName.Replace('\', '/') + '"' } |
    Set-Content (Join-Path $work 'sources.args') -Encoding utf8NoBOM
Write-Output "Prepared preferred-form sources: $source"
