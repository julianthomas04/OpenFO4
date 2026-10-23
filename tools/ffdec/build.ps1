# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
[CmdletBinding()]
param([string]$Jdk = 'C:/Program Files/Java/jdk-23', [string]$SourceBundle)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$work = Join-Path $repo 'build/ffdec-work'
& (Join-Path $PSScriptRoot 'prepare.ps1') -Refresh -SourceBundle $SourceBundle
. (Join-Path $PSScriptRoot 'dependencies.ps1')
$lock = Get-Content (Join-Path $PSScriptRoot 'toolchain.lock.json') -Raw | ConvertFrom-Json
Build-DependencySources $work $Jdk $lock
$classes = Join-Path $work 'classes'
New-Item -ItemType Directory -Force $classes | Out-Null
$diagnostics = @(& (Join-Path $Jdk 'bin/javac.exe') --release 21 -encoding UTF-8 -d $classes -classpath (Join-Path $work 'lib/*') ('@' + (Join-Path $work 'sources.args')) 2>&1)
$compilerExit = $LASTEXITCODE
$diagnostics | Set-Content (Join-Path $work 'javac.log') -Encoding utf8NoBOM
$diagnostics | Write-Output
if ($compilerExit -ne 0 -or ($diagnostics -match 'An exception has occurred in the compiler|error:')) {
    throw "javac failed or reported an internal compiler exception (exit $compilerExit); see $work/javac.log"
}
foreach ($required in 'openfo4/MinimalFfdec.class','com/jpexs/decompiler/flash/SWF.class','com/jpexs/decompiler/flash/exporters/FontExporter.class','com/jpexs/decompiler/flash/exporters/FrameExporter.class') {
    if (-not (Test-Path (Join-Path $classes $required))) { throw "Missing compiled class $required" }
}
$source = Join-Path $work 'src'
foreach ($resource in Get-ChildItem $source -Recurse -File | Where-Object { $_.Extension -ne '.java' }) {
    $relative = [IO.Path]::GetRelativePath($source, $resource.FullName)
    if ($resource.Extension -in '.ttf','.otf','.ico','.png','.gif','.jpg','.jpeg') { continue }
    $target = Join-Path $classes $relative
    New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
    Copy-Item $resource.FullName $target
}
# Preserve upstream exporter metadata. The adapter's -help version separately identifies the fork for cache invalidation.
@'
version=26.2.1
version.major=26
version.minor=2
version.release=1
version.build=0
version.debug=false
nightly=false
build=OpenFO4 minimal export fork
builder=OpenFO4
'@ | Set-Content (Join-Path $classes 'project.properties') -Encoding ascii
$runtime = Join-Path $repo 'build/ffdec-candidate'
if (Test-Path $runtime) {
    if ([IO.Path]::GetFullPath($runtime) -ne (Join-Path $repo 'build/ffdec-candidate')) { throw 'Unsafe candidate path' }
    if ((Get-Item -LiteralPath $runtime).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing linked candidate' }
    Remove-Item -LiteralPath $runtime -Recurse -Force
}
New-Item -ItemType Directory -Force $runtime | Out-Null
Copy-Item (Join-Path $work 'lib') $runtime -Recurse -Force
Copy-Item (Join-Path $work 'licenses') $runtime -Recurse
$jars = @(Get-ChildItem (Join-Path $work 'lib') -Filter *.jar | Sort-Object Name | ForEach-Object { 'lib/' + $_.Name })
$manifest = "Manifest-Version: 1.0`nMain-Class: openfo4.MinimalFfdec`nClass-Path: " + ($jars -join ' ') + "`n`n"
Set-Content (Join-Path $work 'MANIFEST.MF') $manifest -Encoding utf8NoBOM
& (Join-Path $Jdk 'bin/jar.exe') --create --file (Join-Path $runtime 'ffdec-cli.jar') --manifest (Join-Path $work 'MANIFEST.MF') -C $classes .
$jarExit = $LASTEXITCODE
if ($jarExit -ne 0 -or -not (Test-Path (Join-Path $runtime 'ffdec-cli.jar'))) { throw "jar creation failed (exit $jarExit)" }
$destination = Join-Path $repo 'build/ffdec'
if (Test-Path $destination) {
    if ([IO.Path]::GetFullPath($destination) -ne (Join-Path $repo 'build/ffdec')) { throw 'Unsafe runtime path' }
    if ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing linked runtime' }
    Remove-Item -LiteralPath $destination -Recurse -Force
}
Move-Item -LiteralPath $runtime -Destination $destination
. (Join-Path $PSScriptRoot 'materialize.ps1')
$compilerVersion = @(& (Join-Path $Jdk 'bin/javac.exe') -version 2>&1) -join ' '
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Java compiler' }
Write-FFDecSourceBundle $repo $work $destination $compilerVersion $lock
Write-Output "Built $runtime/ffdec-cli.jar; matching preferred-form sources and license notices are in build/ffdec-sources."
