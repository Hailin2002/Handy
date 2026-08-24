#Requires -Version 5.1
<#
.SYNOPSIS
    F3-immune blob-level prettier format check (operationalizes
    docs/meeting-mode-verification-environment.md, finding F3).

.DESCRIPTION
    `core.autocrlf=true` checks the working tree out with CRLF while
    `.prettierrc` pins `"endOfLine": "lf"`, so a plain
    `prettier --check` fails on committed-clean files with checkout noise.
    This script implements the documented manual triage protocol:

      1. read the working-tree content of each input file,
      2. normalize CRLF -> LF,
      3. write it to a UTF-8-no-BOM temp copy that keeps the original
         extension,
      4. run `node_modules\.bin\prettier.cmd --check --config <repo>/.prettierrc`
         on that copy.

    The verdict therefore reflects the *blob* formatting, not the checkout's
    line endings:

      BLOB-CLEAN          LF-normalized content passes prettier --check
                          (CRLF-only working-tree noise cannot cause this
                          verdict to flip - autocrlf immunity)
      BLOB-DIRTY          LF-normalized content has real style issues
      BLOB-SKIP-MISSING   path does not exist in the working tree
                          (e.g. deleted in git diff --name-only output)
      BLOB-ERROR          prettier failed without a "Code style issues"
                      report (e.g. parse error, no parser) - detail included

.INPUTS
    Either pass paths as arguments/positional parameters, or pipe the output
    of `git diff --name-only` / `git diff --name-only --cached` in:

        .\scripts\verify\check-format-blob.ps1 docs/meeting-mode-plan.md src/App.tsx
        git diff --name-only HEAD | .\scripts\verify\check-format-blob.ps1

.PARAMETER Path
    One or more repo-relative or absolute file paths.

.PARAMETER Json
    Emit machine-readable JSON (one object per file: file, verdict,
    prettierExitCode, prettierDetail) instead of text lines.

.PARAMETER SelfTest
    Run the known-answer self-test instead of checking inputs. In one run it
    creates two synthetic fixtures under logs/format-blob-selftest/:
    a deliberately misformatted JSON file (must be BLOB-DIRTY) and a
    CRLF-but-clean JSON file (must be BLOB-CLEAN, proving autocrlf
    immunity). Exits non-zero if either expectation fails.

.EXAMPLE
    .\scripts\verify\check-format-blob.ps1 -SelfTest

.NOTES
    Exit codes (check mode):
        0   every file BLOB-CLEAN or BLOB-SKIP-MISSING
        1   at least one file BLOB-DIRTY or BLOB-ERROR
        2   no input files given
    Self-test mode exits 0 on pass, 1 on failure.
#>
[CmdletBinding()]
param(
    # ValueFromRemainingArguments lets `script a b c` bind all paths, while
    # ValueFromPipeline keeps `git diff --name-only | script` working; on its
    # own, ValueFromPipeline would only ever bind ONE positional argument.
    [Parameter(Position = 0, ValueFromPipeline = $true, ValueFromRemainingArguments = $true)]
    [string[]]$Path,

    [switch]$Json,
    [switch]$SelfTest,

    [string]$ConfigPath,
    [string]$RepoRoot
)

begin {
    $script:_files = [System.Collections.Generic.List[string]]::new()
    $script:_sawPipelineInput = $false
}

process {
    # Runs once per PIPELINE input object only; direct arguments never
    # reach this block, so they are collected in end{} instead.
    $script:_sawPipelineInput = $true
    if ($null -ne $Path) {
        foreach ($p in $Path) { if ($p) { $script:_files.Add($p) } }
    }
}

end {
    $ErrorActionPreference = 'Stop'

    # Direct-argument invocation: process{} never ran, collect here.
    if (-not $script:_sawPipelineInput -and $null -ne $Path) {
        foreach ($p in $Path) { if ($p) { $script:_files.Add($p) } }
    }

    if (-not $RepoRoot) {
        $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
    }
    $RepoRoot = (Resolve-Path $RepoRoot).Path

    $prettierCmd = Join-Path $RepoRoot 'node_modules\.bin\prettier.cmd'
    if (-not (Test-Path $prettierCmd)) {
        throw "prettier shim not found at '$prettierCmd' - run 'npm.cmd install' in the repo root first."
    }
    if (-not $ConfigPath) {
        $ConfigPath = Join-Path $RepoRoot '.prettierrc'
    }
    if (-not (Test-Path $ConfigPath)) {
        throw "prettier config not found at '$ConfigPath'."
    }

    # --- core check ---------------------------------------------------------

    # Reads a file that may still be handle-locked by a just-exited process,
    # retrying briefly; returns empty list if it never unlocks.
    function Read-UnlockedFile {
        param([string]$LiteralPath)
        for ($attempt = 0; $attempt -lt 10; $attempt++) {
            try {
                return @(Get-Content -LiteralPath $LiteralPath -ErrorAction Stop)
            }
            catch [IO.IOException] {
                Start-Sleep -Milliseconds 200
            }
            catch {
                return @()
            }
        }
        return @()
    }

    function Invoke-BlobFormatCheck {
        param([System.Collections.Generic.List[string]]$Files)

        $results = New-Object System.Collections.Generic.List[object]
        if ($Files.Count -eq 0) { return , $results }

        # one shared temp directory inside the OS temp area; per-file copies
        # keep the original extension so prettier picks the right parser.
        $tmpDir = Join-Path ([IO.Path]::GetTempPath()) ("fmt-blob-" + [Guid]::NewGuid().ToString('N'))
        [void][IO.Directory]::CreateDirectory($tmpDir)
        try {
            $utf8NoBom = [Text.UTF8Encoding]::new($false)

            foreach ($entry in $Files) {
                $full = $entry -replace '/', '\'
                if (-not [IO.Path]::IsPathRooted($full)) {
                    $full = Join-Path $RepoRoot $full
                }

                if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
                    $results.Add([pscustomobject]@{
                        file             = $entry
                        verdict          = 'BLOB-SKIP-MISSING'
                        prettierExitCode = $null
                        prettierDetail   = 'file not present in working tree (deleted/untracked-in-diff?)'
                    })
                    continue
                }
                $full = (Resolve-Path -LiteralPath $full).Path

                $text = [IO.File]::ReadAllText($full)
                $normalized = $text.Replace("`r`n", "`n")

                $tmpFile = Join-Path $tmpDir (
                    [Guid]::NewGuid().ToString('N') + [IO.Path]::GetExtension($full)
                )
                [IO.File]::WriteAllText($tmpFile, $normalized, $utf8NoBom)

                # Run prettier via `cmd /d /c` with file-level redirection.
                # stderr is sent straight to a file instead of through the
                # PowerShell pipeline, so progress lines cannot become a
                # terminating NativeCommandError on Windows PowerShell 5.1,
                # and no separate process-spawn API (e.g. Start-Process,
                # which restricted environments may deny) is required.
                $stdOutTmp = "$tmpFile.out"
                $stdErrTmp = "$tmpFile.err"
                $cmdLine = '"{0}" --check --config "{1}" --no-color "{2}" 1>"{3}" 2>"{4}"' -f `
                    $prettierCmd, $ConfigPath, $tmpFile, $stdOutTmp, $stdErrTmp
                & cmd.exe /d /c $cmdLine
                $exitCode = $LASTEXITCODE
                # cmd /c waits for the full batch->node chain, but a handle
                # can briefly outlive it; retry reads defensively anyway.
                $stdOut = Read-UnlockedFile -LiteralPath $stdOutTmp
                $stdErr = Read-UnlockedFile -LiteralPath $stdErrTmp
                $detail = (($stdOut + $stdErr) | ForEach-Object { "$_" }) -join "`n"
                Remove-Item -LiteralPath $stdOutTmp, $stdErrTmp -Force -ErrorAction SilentlyContinue
                $detail = $detail.Trim()
                # replace the random temp name with the real path in the detail
                $detail = $detail.Replace($tmpFile, $entry)

                if ($exitCode -eq 0) {
                    $verdict = 'BLOB-CLEAN'
                    if (-not $detail) { $detail = 'All matched files use Prettier code style!' }
                }
                elseif ($detail -match 'Code style issues') {
                    $verdict = 'BLOB-DIRTY'
                }
                else {
                    $verdict = 'BLOB-ERROR'
                }

                $results.Add([pscustomobject]@{
                    file             = $entry
                    verdict          = $verdict
                    prettierExitCode = $exitCode
                    prettierDetail   = $detail
                })
            }
        }
        finally {
            for ($attempt = 0; $attempt -lt 10; $attempt++) {
                try {
                    [void][IO.Directory]::Delete($tmpDir, $true)
                    break
                }
                catch [IO.IOException] {
                    Start-Sleep -Milliseconds 200
                }
            }
        }

        return , $results
    }

    # --- self-test ----------------------------------------------------------

    function Invoke-BlobFormatSelfTest {
        $selfTestDir = Join-Path $RepoRoot 'logs\format-blob-selftest'
        [void][IO.Directory]::CreateDirectory($selfTestDir)

        # Fixture 1: deliberately misformatted (compact JSON, no final newline)
        # -> must be BLOB-DIRTY.
        $dirtyFixture = Join-Path $selfTestDir 'selftest-dirty.json'
        [IO.File]::WriteAllText(
            $dirtyFixture,
            '{"misformatted":true,"reason":"compact json, missing indent and final newline"}',
            [Text.UTF8Encoding]::new($false))

        # Fixture 2: correctly formatted content but stored with CRLF endings -
        # exactly what autocrlf=true produces on checkout -> must be BLOB-CLEAN.
        $cleanCrlfFixture = Join-Path $selfTestDir 'selftest-crlf-clean.json'
        [IO.File]::WriteAllText(
            $cleanCrlfFixture,
            "{`r`n  `"wellformed`": true,`r`n  `"endings`": `"crlf`"`r`n}`r`n",
            [Text.UTF8Encoding]::new($false))

        $fixtures = [System.Collections.Generic.List[string]]::new()
        $fixtures.Add($dirtyFixture)
        $fixtures.Add($cleanCrlfFixture)

        $checks = Invoke-BlobFormatCheck -Files $fixtures
        $byVerdict = @{}
        foreach ($c in $checks) { $byVerdict[$c.file] = $c }

        $expectations = @(
            @{ fixture = $dirtyFixture;      expected = 'BLOB-DIRTY'; label = 'misformatted file is detected as DIRTY' },
            @{ fixture = $cleanCrlfFixture;  expected = 'BLOB-CLEAN'; label = 'CRLF-but-clean file is immune to autocrlf (CLEAN)' }
        )

        $passed = $true
        $lines = New-Object System.Collections.Generic.List[string]
        foreach ($e in $expectations) {
            $actual = $byVerdict[$e.fixture].verdict
            $ok = ($actual -eq $e.expected)
            if (-not $ok) { $passed = $false }
            # NOTE: use .Add(), not += - "+=" re-boxes the generic List into
            # an untyped Object[], losing ToArray()/count semantics.
            [void]$lines.Add(('{0}  expected={1}  actual={2}  exit={3}  {4}' -f `
                $(if ($ok) { 'PASS' } else { 'FAIL' }), $e.expected, $actual,
                  $byVerdict[$e.fixture].prettierExitCode, $e.label))
        }

        # NOTE: materialize collections through the pipeline, NOT via
        # @( $list ) or .ToArray(): on Windows PowerShell 5.1, @( $list )
        # yields Object[]{List} which breaks pscustomobject/Add-Member
        # ("Argument types do not match"), and .ToArray() on an already
        # unboxed Object[] is forwarded to its elements (e.g.
        # "[System.String] does not contain a method named 'ToArray'").
        $summary = [pscustomobject]@{
            selfTest   = 'check-format-blob'
            passed     = $passed
            assertions = @($lines | ForEach-Object { $_ })
            results    = @($checks | ForEach-Object { $_ })
            timestamp  = (Get-Date).ToString('o')
        }

        $resultJson = Join-Path $selfTestDir 'selftest-result.json'
        $summary | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resultJson -Encoding UTF8

        if ($Json) {
            $summary | ConvertTo-Json -Depth 5
        }
        else {
            Write-Host '=== check-format-blob self-test ==='
            foreach ($l in $lines) { Write-Host $l }
            Write-Host ("Result file: {0}" -f $resultJson)
            Write-Host ("SELFTEST " + $(if ($passed) { 'PASSED' } else { 'FAILED' }))
        }

        return $passed
    }

    # --- dispatch -----------------------------------------------------------

    if ($SelfTest) {
        $ok = Invoke-BlobFormatSelfTest
        exit $(if ($ok) { 0 } else { 1 })
    }

    if ($script:_files.Count -eq 0) {
        Write-Host ('No input files. Pass paths as arguments or pipe git diff --name-only, e.g.')
        Write-Host ('  {0} docs/plan.md src/App.tsx' -f $MyInvocation.ScriptName)
        exit 2
    }

    $results = Invoke-BlobFormatCheck -Files $script:_files

    if ($Json) {
        $results | ConvertTo-Json -Depth 4
    }
    else {
        foreach ($r in $results) {
            Write-Host ('{0}  {1}' -f $r.verdict, $r.file)
        }
    }

    $blocking = @($results | Where-Object { $_.verdict -eq 'BLOB-DIRTY' -or $_.verdict -eq 'BLOB-ERROR' })
    exit $(if ($blocking.Count -gt 0) { 1 } else { 0 })
}
