# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
function Prepare-DependencySources([string]$Repo, [string]$Upstream, [string]$Work, $Lock) {
    $cache = Join-Path $Repo 'build/cache/ffdec/dependencies'
    New-Item -ItemType Directory -Force $cache | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    foreach ($dependency in $Lock.dependencies) {
        $origin = $Upstream
        if ($dependency.PSObject.Properties.Name -contains 'sourceArchive') {
            $input = $dependency.sourceArchive
            $archive = Join-Path $cache $input.file
            if (-not (Test-Path $archive)) {
                Invoke-WebRequest $input.url -OutFile "$archive.partial"
                if ((Get-FileHash "$archive.partial").Hash.ToLowerInvariant() -ne $input.sha256) { throw "Source hash mismatch: $($input.file)" }
                Move-Item "$archive.partial" $archive
            }
            if ((Get-FileHash $archive).Hash.ToLowerInvariant() -ne $input.sha256) { throw "Source hash mismatch: $($input.file)" }
            $origin = Join-Path $cache ($input.file + '.unpacked')
            if (-not (Test-Path $origin)) {
                New-Item -ItemType Directory $origin | Out-Null
                if ($input.file.EndsWith('.jar')) { [IO.Compression.ZipFile]::ExtractToDirectory($archive, $origin) }
                else {
                    & tar.exe -xf $archive -C $origin
                    if ($LASTEXITCODE -ne 0) { throw "Source extraction failed: $($input.file)" }
                }
            }
        }
        $name = [IO.Path]::GetFileNameWithoutExtension($dependency.file)
        $destination = Join-Path $Work "dependency-sources/$name/src"
        New-Item -ItemType Directory -Force $destination | Out-Null
        $source = [IO.Path]::GetFullPath((Join-Path $origin $dependency.sourceRoot))
        $componentRoot = if ($dependency.PSObject.Properties.Name -contains 'sourceArchive') { $origin } else { Join-Path $Upstream $dependency.sourcePath }
        foreach ($grammar in Get-ChildItem $componentRoot -Recurse -File | Where-Object { $_.Extension -in '.g4','.flex' }) {
            $target = Join-Path $Work "dependency-sources/$name/generator-inputs/$([IO.Path]::GetRelativePath($componentRoot,$grammar.FullName))"
            New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
            Copy-Item -LiteralPath $grammar.FullName -Destination $target
        }
        foreach ($java in Get-ChildItem $source -Recurse -File -Filter *.java) {
            $relative = [IO.Path]::GetRelativePath($source, $java.FullName)
            if ($relative -match '(^|[/\\])(test|tests)([/\\]|$)' -or $java.Name -match '^(StrokeTest|LzmaAlone|LzmaBench)\.java$') { continue }
            $target = Join-Path $destination $relative
            New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
            Copy-Item $java.FullName $target
        }
        if ($dependency.file -eq 'tomlj-1.1.1.jar') {
            foreach ($java in Get-ChildItem $destination -Recurse -Filter *.java) {
                $text = [IO.File]::ReadAllText($java.FullName)
                $text = $text -replace '(?m)^import org\.checkerframework\.[^;]+;\r?\n', ''
                $text = $text -replace '(?s)@DefaultQualifier\(.*?\}\)', ''
                $text = $text -replace '@(Nullable|NonNull)\b\s*', ''
                [IO.File]::WriteAllText($java.FullName, $text)
            }
        }
        if ($dependency.file -eq 'avi.jar') {
            $reader = Join-Path $destination 'org/monte/media/jpeg/MJPGImageReader.java'
            $text = [IO.File]::ReadAllText($reader).Replace('import com.sun.imageio.plugins.jpeg.JPEGImageReader;', '')
            $text = $text.Replace('new JPEGImageReader(getOriginatingProvider())', 'javax.imageio.ImageIO.getImageReadersByFormatName("JPEG").next()')
            [IO.File]::WriteAllText($reader, $text)
        }
        if ($dependency.file -eq 'gnujpdf.jar') {
            $graphics = Join-Path $destination 'gnu/jpdf/PDFGraphics.java'
            $text = [IO.File]::ReadAllText($graphics).Replace('import jdk.internal.reflect.Reflection;', '')
            [IO.File]::WriteAllText($graphics, $text)
        }
        if ($dependency.file -eq 'jpacker.jar') {
            $cli = Join-Path $destination 'com/jpacker/JPacker.java'
            $text = [IO.File]::ReadAllText($cli) -replace '(?m)^import [^;]+;\r?\n', ''
            $at = $text.IndexOf('public class JPacker {')
            if ($at -lt 0) { throw 'JPacker CLI source anchor changed' }
            $text = $text.Substring(0,$at) + @'
public class JPacker {
    public static void main(String[] args) throws java.io.IOException {
        throw new java.io.IOException("Canvas/JavaScript packing is outside the minimal SVG/font exporter");
    }
}
'@
            [IO.File]::WriteAllText($cli, $text)
        }
        $notice = if ($dependency.notice.StartsWith('licenses/')) { Join-Path $PSScriptRoot $dependency.notice } else { Join-Path $origin $dependency.notice }
        $licenseDirectory = Join-Path $Work "licenses/$name"
        New-Item -ItemType Directory -Force $licenseDirectory | Out-Null
        Copy-Item -LiteralPath $notice -Destination (Join-Path $licenseDirectory 'LICENSE.txt')
        if ($dependency.file -eq 'ttf.jar') {
            Copy-Item (Join-Path $Upstream 'libsrc/ffdec_lib/lib/ttf.fontastic.license.txt') $licenseDirectory
        }
        # JLayer uses serialized decoder lookup tables as runtime resources.
        if ($dependency.file -eq 'jlayer-1.0.2.jar') {
            $resources = Join-Path $Upstream 'libsrc/jlayer-1.0.2/src/main/resources'
            if (Test-Path $resources) { Copy-Item "$resources/*" $destination -Recurse -Force }
        }
        $copyrights = @(foreach ($java in Get-ChildItem $destination -Recurse -Filter *.java) {
            Get-Content $java.FullName -TotalCount 70 | Where-Object { $_ -match '(?i)copyright\s*(\(c\)|[0-9:$])' }
        }) | Sort-Object -Unique
        @("Retained source notices for $name", '', $copyrights) | Set-Content (Join-Path $licenseDirectory 'ATTRIBUTIONS.txt') -Encoding utf8NoBOM
        Write-Output "Prepared dependency sources: $($dependency.file)"
    }
}

function Build-DependencySources([string]$Work, [string]$Jdk, $Lock) {
    $libraryDirectory = Join-Path $Work 'lib'
    New-Item -ItemType Directory -Force $libraryDirectory | Out-Null
    foreach ($dependency in $Lock.dependencies) {
        $name = [IO.Path]::GetFileNameWithoutExtension($dependency.file)
        $root = Join-Path $Work "dependency-sources/$name"
        $source = Join-Path $root 'src'
        $classes = Join-Path $root 'classes'
        New-Item -ItemType Directory -Force $classes | Out-Null
        $javaFiles = @(Get-ChildItem $source -Recurse -Filter *.java)
        if ($javaFiles.Count -eq 0) { throw "No preferred-form sources: $name" }
        $javaFiles | ForEach-Object { '"' + $_.FullName.Replace('\', '/') + '"' } | Set-Content (Join-Path $root 'sources.args') -Encoding utf8NoBOM
        $output = @(& (Join-Path $Jdk 'bin/javac.exe') --release 21 -encoding UTF-8 -classpath "$libraryDirectory/*" -d $classes ('@' + (Join-Path $root 'sources.args')) 2>&1)
        $code = $LASTEXITCODE
        $output | Set-Content (Join-Path $root 'javac.log') -Encoding utf8NoBOM
        if ($code -ne 0 -or ($output -match 'An exception has occurred in the compiler|error:')) { $output | Write-Output; throw "Dependency compile failed: $name (exit $code)" }
        foreach ($resource in Get-ChildItem $source -Recurse -File | Where-Object { $_.Extension -ne '.java' }) {
            $target = Join-Path $classes ([IO.Path]::GetRelativePath($source, $resource.FullName))
            New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
            Copy-Item $resource.FullName $target
        }
        & (Join-Path $Jdk 'bin/jar.exe') --create --file (Join-Path $libraryDirectory $dependency.file) -C $classes .
        if ($LASTEXITCODE -ne 0) { throw "Dependency jar failed: $name" }
        Write-Output "Rebuilt dependency: $($dependency.file)"
    }
}
