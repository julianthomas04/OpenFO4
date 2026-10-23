<#
.SYNOPSIS
    Prints the version of the naga crate locked in src/engine/Cargo.lock.
.DESCRIPTION
    naga-cli is released in lockstep with naga, so this is the version of naga-cli to install for validating the WGSL the installer
    emits against the compiler the engine uses. Used by ci.ps1 and .github/workflows/ci.yml. Throws unless the lock file lists exactly
    one naga package.
.PARAMETER LockFile
    The Cargo.lock to read. Defaults to the engine's.
#>
[CmdletBinding()]
param(
    [string]$LockFile = (Join-Path (Split-Path $PSScriptRoot) 'src/engine/Cargo.lock')
)

$ErrorActionPreference = 'Stop'
$entries = [regex]::Matches((Get-Content -LiteralPath $LockFile -Raw), '(?m)^name = "naga"\s+version = "([^"]+)"')
if ($entries.Count -ne 1) {
    throw "Expected exactly one naga package in $LockFile, found $($entries.Count)."
}

$entries[0].Groups[1].Value
