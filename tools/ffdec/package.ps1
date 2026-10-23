# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Destination,
    [Parameter(Mandatory)][string]$SourceDestination
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$runtime = Join-Path $repo 'build/ffdec'
$sources = Join-Path $repo 'build/ffdec-sources'
foreach ($required in 'ffdec-cli.jar', 'lib', 'licenses', 'THIRD_PARTY_NOTICES.txt') {
    if (-not (Test-Path -LiteralPath (Join-Path $runtime $required))) { throw "Build FFDec first; missing $required." }
}
foreach ($required in 'preferred-form', 'SOURCES.md') {
    if (-not (Test-Path -LiteralPath (Join-Path $sources $required))) { throw "FFDec matching source bundle is missing $required." }
}
if (Test-Path -LiteralPath $Destination) { throw "Use a fresh FFDec runtime destination: $Destination" }
if (Test-Path -LiteralPath $SourceDestination) { throw "Use a fresh FFDec source destination: $SourceDestination" }
New-Item -ItemType Directory -Force -Path $Destination, $SourceDestination | Out-Null
foreach ($relative in 'ffdec-cli.jar', 'lib', 'licenses', 'THIRD_PARTY_NOTICES.txt') {
    Copy-Item -LiteralPath (Join-Path $runtime $relative) -Destination $Destination -Recurse
}
Get-ChildItem -LiteralPath $sources | Where-Object { $_.Name -ne 'COMPATIBILITY.json' } | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $SourceDestination -Recurse
}