# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
function Write-FFDecSourceBundle([string]$Repo, [string]$Work, [string]$Runtime, [string]$CompilerVersion, $Lock) {
    $notices = @(
        ('OpenFO4 minimal FFDec ' + $Lock.version),
        'Original adapter/build code: Copyright 2026 Julian Thomas and OpenFO4 contributors; Apache-2.0.',
        'FFDec library: Copyright 2010-2026 JPEXS and other retained source authors; LGPL-3.0-or-later.',
        'The combined export tool is conveyed under GPL-3.0-or-later with separately licensed components.',
        'Full license texts and retained component attributions are below licenses/.',
        'Complete matching preferred-form source and build scripts are the OpenFO4 sources ZIP beside the installer on the same release page.',
        'The release-level THIRD_PARTY_NOTICES.txt and legal/COMPONENT_INDEX.txt provide its exact download link.',
        'Replaceable standalone Java programs are executed through the JVM; no proprietary runtime or original game asset is included.',
        '',
        'Rebuilt dependencies (historical prebuilt versions are not asserted):'
    )
    foreach ($dependency in $Lock.dependencies) { $notices += "$($dependency.file): $($dependency.licenseExpression); licenses/$([IO.Path]::GetFileNameWithoutExtension($dependency.file))/" }
    $notices | Set-Content (Join-Path $Runtime 'THIRD_PARTY_NOTICES.txt') -Encoding utf8NoBOM
    $destination = [IO.Path]::GetFullPath((Join-Path $Repo 'build/ffdec-sources'))
    if (Test-Path $destination) {
        if ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing linked source bundle' }
        Remove-Item -LiteralPath $destination -Recurse -Force
    }
    $preferred = Join-Path $destination 'preferred-form'
    New-Item -ItemType Directory -Force $preferred | Out-Null
    Copy-Item (Join-Path $Work 'src'), (Join-Path $Work 'licenses') $preferred -Recurse
    Copy-Item (Join-Path $Work 'generator-inputs') $preferred -Recurse
    foreach ($dependency in $Lock.dependencies) {
        $name = [IO.Path]::GetFileNameWithoutExtension($dependency.file)
        $target = Join-Path $preferred "dependency-sources/$name"
        New-Item -ItemType Directory -Force $target | Out-Null
        Copy-Item (Join-Path $Work "dependency-sources/$name/src") $target -Recurse
        $grammar = Join-Path $Work "dependency-sources/$name/generator-inputs"
        if (Test-Path $grammar) { Copy-Item $grammar $target -Recurse }
    }
    $tools = Join-Path $destination 'build-tools'
    New-Item -ItemType Directory $tools | Out-Null
    Get-ChildItem $PSScriptRoot -File | Where-Object { $_.Extension -in '.ps1','.json','.md' } | Copy-Item -Destination $tools
    Copy-Item (Join-Path $PSScriptRoot 'licenses') $tools -Recurse
    Copy-Item (Join-Path $PSScriptRoot 'tests') $tools -Recurse
    Get-ChildItem (Join-Path $tools 'tests') -Recurse -File -Filter *.pyc | Remove-Item
    $sourceFiles = @(Get-ChildItem $destination -Recurse -File | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path = [IO.Path]::GetRelativePath($destination, $_.FullName).Replace('\','/'); sha256 = (Get-FileHash $_.FullName).Hash.ToLowerInvariant() }
    })
    $sourceManifest = [ordered]@{
        schema = 1; component = 'OpenFO4 minimal FFDec'; version = $Lock.version
        compiler = $CompilerVersion; targetJava = 21; sourceInputs = $Lock; files = $sourceFiles
    }
    $sourceManifest | ConvertTo-Json -Depth 14 | Set-Content (Join-Path $destination 'source-files.json') -Encoding utf8NoBOM
    @'
# Minimal FFDec complete preferred-form sources

The preferred-form tree is exactly the patched Java source and resources used to build the accompanying tool and every retained dependency. No dependency JAR is accepted as a substitute for its source. `source-files.json` records every source/resource hash, compiler identity, exact acquisition source pins, original reference JAR hashes and modifications. Upstream copyright and license headers are retained. Original OpenFO4 code is Apache-2.0; upstream library/component licenses remain effective and the combined tool is GPL-3.0-or-later compatible.

No GUI distribution, Adobe SWC, restricted CMYK decoder, native decoder or Bethesda asset is included. The complete upstream FFDec acquisition archive is deliberately excluded. Its hash is acquisition provenance only, not a license grant for excluded material. Library sources are LGPL-3.0-or-later; font output uses GPL-2.0-or-later DoubleType plus LGPL-2.1-or-later Fontastic. Miterstroke retains GPL-2.0-only with its Classpath exception. Each dependency's source identity is explicit; unknown historical binary versions are not asserted.

Build offline on Windows with PowerShell7, the Windows tar utility and an installed JDK21 or newer (tested compiler is recorded in source-files.json):

`pwsh -NoProfile -File build-tools/build.ps1 -Jdk C:/path/to/jdk -SourceBundle C:/absolute/path/to/this/ffdec`

All preferred-form sources are already prepared, so this command does not download anything or need Ant, JFlex, CheckerFramework or package-manager restores. Outputs are placed under the source bundle parent's build directory. Build logs record compiler failures. Run the original synthetic fixture harness in build-tools/tests against an independently retained reference only under appropriate test scheduling. The synthetic fixtures contain no game data.

Changes are recorded in build-tools/toolchain.lock.json and embodied in the preferred-form tree. The upstream source patch instructions are in build-tools/prepare.ps1 and build-tools/dependencies.ps1. Complete upstream sources can additionally be acquired through those pinned URLs for comparison, but are unnecessary for rebuilding this archive.
'@ | Set-Content (Join-Path $destination 'SOURCES.md') -Encoding utf8NoBOM
    $manifest = [ordered]@{
        schema = 1; component = 'OpenFO4 minimal FFDec'; version = $Lock.version
        licenseExpression = 'GPL-3.0-or-later'
        compiler = $CompilerVersion; sourceManifestSha256 = (Get-FileHash (Join-Path $destination 'source-files.json')).Hash.ToLowerInvariant()
        origin = $Lock.upstream; modifications = $Lock.modifications
        files = @(Get-ChildItem $Runtime -Recurse -File | Where-Object { $_.Name -ne 'runtime-files.json' } | Sort-Object FullName | ForEach-Object {
            [ordered]@{path=[IO.Path]::GetRelativePath($Runtime,$_.FullName).Replace('\','/');sha256=(Get-FileHash $_.FullName).Hash.ToLowerInvariant()}
        })
        sourceInputs = @($Lock.dependencies | ForEach-Object { [ordered]@{runtimePath='lib/'+$_.file; sourcePath='preferred-form/dependency-sources/'+[IO.Path]::GetFileNameWithoutExtension($_.file)+'/src'; licenseExpression=$_.licenseExpression} })
    }
    $manifest | ConvertTo-Json -Depth 14 | Set-Content (Join-Path $Runtime 'runtime-files.json') -Encoding utf8NoBOM
}
