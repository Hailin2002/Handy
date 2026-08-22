# Verifies that every fenced ```rust code block in recorder-contract.md
# compiles as a standalone library crate (no external dependencies).
# Usage: pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1
param(
    [string]$ContractPath = (Join-Path $PSScriptRoot 'recorder-contract.md')
)

$ErrorActionPreference = 'Stop'

$lines = [System.IO.File]::ReadAllLines((Resolve-Path $ContractPath))
$blocks = New-Object System.Collections.Generic.List[string]
$inBlock = $false
$cur = New-Object System.Text.StringBuilder

foreach ($line in $lines) {
    if (-not $inBlock) {
        if ($line -match '^```rust\s*$') { $inBlock = $true; [void]$cur.Clear() }
    } else {
        if ($line -match '^```\s*$') {
            $inBlock = $false
            $blocks.Add($cur.ToString())
        } else {
            [void]$cur.AppendLine($line)
        }
    }
}

if ($inBlock) { throw 'unterminated rust code block in contract file' }
if ($blocks.Count -eq 0) { throw 'no rust code blocks found' }

$sketch = "#![allow(dead_code)]`n#![allow(unused_imports)]`n`n" + (($blocks -join "`n"))
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("recorder-contract-sketch-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempDir | Out-Null
try {
    $src = Join-Path $tempDir 'sketch.rs'
    [System.IO.File]::WriteAllText($src, $sketch, (New-Object System.Text.UTF8Encoding($false)))
    $out = & rustc --edition 2021 --crate-type lib --emit=metadata -o (Join-Path $tempDir 'libsketch.rmeta') $src 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Output $out
        throw "rustc failed with exit code $LASTEXITCODE"
    }
    Write-Output "OK: $($blocks.Count) rust block(s) from '$(Split-Path -Leaf $ContractPath)' compiled as a library crate ($(rustc --version))."
} finally {
    Remove-Item -Recurse -Force $tempDir
}
