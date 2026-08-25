<#
.SYNOPSIS
    Bundle integrity verifier for gate artifact directories produced by
    scripts/verify/run-gate.ps1: validates logs, SHA256 sidecars and
    summary.json against each other.

.DESCRIPTION
    Given a gate artifacts directory (see run-gate.ps1), asserts:

      1. Every *.log file has a *.log.sha256 sidecar whose recorded digest
         matches the actual SHA256 of the log (Get-FileHash; certutil.exe
         fallback under ConstrainedLanguage, compared case-insensitively).
      2. summary.json parses as JSON and round-trips through
         ConvertTo-Json -> ConvertFrom-Json without loss of parseability.
      3. The summary lists EXACTLY the set of *.log files present in the
         directory (no listed-but-missing logs, no unlisted-but-present
         logs), and each step's recorded sha256 matches the actual log
         digest.
      4. summary.json has its own sidecar whose digest matches the file.

    Exit code contract:
      0   bundle is intact (all assertions hold)
      1   bundle is INVALID (at least one assertion failed; every violation
          is printed so callers see the complete defect list, not just the
          first)
      2   INFRASTRUCTURE error (bad arguments, artifacts directory missing
          or unreadable, unexpected engine failure) - distinct from "the
          bundle proved guilty"

    -SelfTest fabricates synthetic bundles (a known-good one plus one per
    defect class) in a scratch directory, verifies each verdict by invoking
    this very script recursively on them, and exits 0 only when every case
    produces its expected exit code. Synthetic bundles mirror the exact
    artifact shape run-gate.ps1 emits ("<64-hex>  <leafname>" sidecars,
    schemaVersion 1 summaries). A transcript of the last self-test run is
    written to logs/verify-bundle-selftest/ for evidence.

    ConstrainedLanguage compatibility (verification-leaf conventions, see
    run-gate.ps1): all output, hashing, path resolution and file writing go
    through the CLM-tolerant helpers below (cmdlet-first with guarded .NET /
    native fallbacks); no raw [Console]::Error, [IO.File], Process spawn,
    Add-Type or generic-type instantiation is attempted. The script completes
    and produces correct verdicts under ConstrainedLanguage; Get-FileHash
    degrades to certutil.exe there, whose lowercase hex is handled by the
    case-insensitive digest comparison.

.EXAMPLE
    ./scripts/verify/assert-bundle-integrity.ps1 logs/verify-gate
    ./scripts/verify/assert-bundle-integrity.ps1 -SelfTest
#>

[CmdletBinding()]
param(
    # Gate artifacts directory to verify (position 0). Required unless
    # -SelfTest is given.
    [Parameter(Position = 0)]
    [string]$ArtifactsDir,

    # Fabricate good and defective synthetic bundles in a scratch area and
    # verify this script's verdict on each. Ignores -ArtifactsDir.
    [switch]$SelfTest,

    # Where the self-test appends its transcript. Default:
    # <repo>/logs/verify-bundle-selftest/selftest-last-run.log
    [string]$ReportPath,

    # Root directory for self-test scratch bundles. Default:
    # <repo>/logs/verify-bundle-selftest/scratch (inside the repository
    # rather than the system temp area: the declared logs scope covers it,
    # .gitignore ignores logs/, and isolated-lane runs stay self-contained).
    [string]$ScratchRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$exitCodeValid = 0
$exitCodeInvalid = 1
$exitCodeInfrastructure = 2

# ---------------------------------------------------------------------------
# ConstrainedLanguage-tolerant helpers (verification-leaf conventions,
# mirrored from scripts/verify/run-gate.ps1).
# ---------------------------------------------------------------------------

# Writes one line to stderr; [Console]::Error.WriteLine is CLM-blocked.
function Write-ConsoleError {
    param([string]$Message)

    try {
        [Console]::Error.WriteLine($Message)
    }
    catch {
        Write-Host $Message -ForegroundColor Red
    }
}

# Absolute-path normalization; [IO.Path]::GetFullPath is CLM-blocked.
function Resolve-FullPath {
    param([string]$Path)

    try {
        return [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        if (Split-Path -Path $Path -IsAbsolute) { return $Path }
        return Join-Path (Get-Location -PSProvider FileSystem).ProviderPath $Path
    }
}

# Resolves relative paths against the caller's location even when the target
# does not exist yet; $ExecutionContext.SessionState.Path.* is CLM-blocked.
function Resolve-AbsolutePath {
    param([string]$Path)

    try {
        return $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath(
            $Path)
    }
    catch {
        if (Split-Path -Path $Path -IsAbsolute) { return $Path }
        return Join-Path (Get-Location -PSProvider FileSystem).ProviderPath $Path
    }
}

# UTF-8 without BOM writer; direct .NET write on FullLanguage hosts,
# Set-Content keeps CLM alive (Windows PowerShell 5.1 prepends a BOM there -
# documented residual limit inherited from run-gate.ps1).
function Write-Utf8NoBomFile {
    param([string]$Path, [string]$Text)

    try {
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
    }
    catch {
        Set-Content -LiteralPath $Path -Value $Text -Encoding UTF8
    }
}

# SHA256 of a file as hex. Get-FileHash cannot instantiate its hash provider
# under CLM; certutil.exe is a native tool and prints "<64 hex>" on its own
# line. Callers MUST compare digests case-insensitively because the fallback
# emits lowercase where Get-FileHash emits uppercase.
function Get-Sha256Hex {
    param([string]$Path)

    try {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    }
    catch {
        $hexLines = @(& certutil.exe -hashfile $Path SHA256 |
            Where-Object { $_ -match '^[0-9a-fA-F]{64}\s*$' } |
            ForEach-Object { $_.Trim() })
        if (-not $hexLines) {
            throw "could not SHA256-hash '$Path' (Get-FileHash unavailable and certutil fallback produced no digest)."
        }
        return (@($hexLines) -join '')
    }
}

# StrictMode-safe property probe: returns $null instead of throwing when a
# parsed JSON object lacks the property.
function Get-JsonPropertySafe {
    param($Object, [string]$Name)

    try {
        return $Object.$Name
    }
    catch {
        return $null
    }
}

# ---------------------------------------------------------------------------
# Core validation. Returns an array of human-readable violation strings;
# an empty array means the bundle is intact. Throws only on infrastructure
# failures (unreadable directory, unhashable file), which the caller maps to
# exit 2.
# ---------------------------------------------------------------------------
function Get-BundleViolations {
    param([string]$BundleDir)

    # Results travel through a script-scoped variable, not the function
    # output pipeline: Windows PowerShell 5.1 turns a returned (empty)
    # array into a phantom empty-string item when the caller wraps the
    # call in @(), which flagged intact bundles as INVALID.
    $violations = @()
    $script:bundleViolations = @()

    # --- Collect the actual *.log population -------------------------------
    $logFiles = @(Get-ChildItem -LiteralPath $BundleDir -Filter '*.log' -File |
        Sort-Object -Property Name)
    $actualLogNames = @($logFiles | ForEach-Object { $_.Name })

    # --- Assertion 1: every *.log has a matching, truthful sidecar ---------
    foreach ($log in $logFiles) {
        $sidecarPath = "$($log.FullName).sha256"
        if (-not (Test-Path -LiteralPath $sidecarPath)) {
            $violations += "missing sidecar for '$($log.Name)': expected '$($log.Name).sha256'"
            continue
        }
        # Index-guarded read: under StrictMode 3.0 (pwsh 7 'Latest'),
        # [0] on the empty array a zero-byte file yields would throw and
        # get misreported as infrastructure (exit 2) instead of invalid.
        $firstLines = @(Get-Content -LiteralPath $sidecarPath -TotalCount 1)
        $firstLine = ''
        if ($firstLines.Count -ge 1) { $firstLine = [string]$firstLines[0] }
        $tokens = @($firstLine -split '\s+' | Where-Object { $_.Length -gt 0 })
        $recorded = ''
        if ($tokens.Count -ge 1 -and $tokens[0] -match '^[0-9a-fA-F]{64}$') {
            $recorded = $tokens[0]
        }
        else {
            $violations += "malformed sidecar '$($log.Name).sha256': first line is not '<64-hex-sha256>  <file>'"
            continue
        }
        $actual = Get-Sha256Hex -Path $log.FullName
        if (-not $recorded.Equals($actual, [StringComparison]::OrdinalIgnoreCase)) {
            $violations += (
                "digest mismatch for '$($log.Name)': sidecar records " +
                "$($recorded.Substring(0, 12))... but file hashes to " +
                "$($actual.Substring(0, 12))...")
        }
    }

    # --- Assertion 2: summary.json parses and round-trips -------------------
    $summaryPath = Join-Path $BundleDir 'summary.json'
    if (-not (Test-Path -LiteralPath $summaryPath)) {
        $violations += "missing summary.json: directory contains $($actualLogNames.Count) log file(s) but no machine-readable summary"
        $script:bundleViolations = $violations
        return
    }
    $summary = $null
    try {
        $summary = Get-Content -LiteralPath $summaryPath -Raw |
            ConvertFrom-Json
    }
    catch {
        $violations += "summary.json does not parse as JSON: $($_.Exception.Message)"
        $script:bundleViolations = $violations
        return
    }
    try {
        # Round-trip: re-serialize and re-parse. Any structural surprise
        # (lossy depth, broken nesting) surfaces here.
        $null = ($summary | ConvertTo-Json -Depth 6) | ConvertFrom-Json
    }
    catch {
        $violations += "summary.json does not round-trip through ConvertTo-Json/ConvertFrom-Json: $($_.Exception.Message)"
    }

    # --- Assertion 3: summary lists exactly the logs present ---------------
    $rawSteps = Get-JsonPropertySafe -Object $summary -Name 'steps'
    $expectedLogNames = @()
    if ($null -ne $rawSteps) {
        foreach ($step in @($rawSteps)) {
            $logRef = Get-JsonPropertySafe -Object $step -Name 'log'
            if ($null -eq $logRef -or [string]::IsNullOrWhiteSpace([string]$logRef)) {
                $violations += "summary step entry is missing its 'log' reference"
                continue
            }
            $expectedLogNames += Split-Path -Leaf ([string]$logRef)

            # Cross-check the digest the summary itself records against the
            # actual log, tying summary and logs together.
            $recordedInSummary = [string](Get-JsonPropertySafe -Object $step -Name 'sha256')
            $logFullPath = Join-Path $BundleDir (Split-Path -Leaf ([string]$logRef))
            if ($recordedInSummary -and (Test-Path -LiteralPath $logFullPath)) {
                $actualDigest = Get-Sha256Hex -Path $logFullPath
                if (-not $recordedInSummary.Equals($actualDigest, [StringComparison]::OrdinalIgnoreCase)) {
                    # Manual length guard instead of [Math]::Min: static .NET
                    # helper invocation is not reliably available under CLM.
                    $prefix = $recordedInSummary
                    if ($prefix.Length -gt 12) { $prefix = $prefix.Substring(0, 12) }
                    $violations += (
                        "summary-recorded digest mismatch for '$(Split-Path -Leaf ([string]$logRef))': " +
                        "summary says ${prefix}..., file hashes to " +
                        "$($actualDigest.Substring(0, 12))...")
                }
            }
        }
    }
    else {
        $violations += "summary.json has no 'steps' collection; cannot enumerate the logs it claims"
    }

    # Exact-set comparison, both directions, order-independent. Duplicates
    # in the summary are their own defect: -Unique would otherwise hide a
    # doubly listed log behind a matching set key.
    $duplicateLogNames = @($expectedLogNames | Group-Object |
        Where-Object { $_.Count -gt 1 } | ForEach-Object { $_.Name })
    foreach ($name in $duplicateLogNames) {
        $violations += "summary lists log '$name' more than once"
    }
    $expectedSorted = @($expectedLogNames | Sort-Object -Unique)
    $actualSorted = @($actualLogNames | Sort-Object -Unique)
    $expectedKey = $expectedSorted -join '|'
    $actualKey = $actualSorted -join '|'
    if ($expectedKey -ne $actualKey) {
        foreach ($name in $expectedSorted) {
            if ($actualSorted -notcontains $name) {
                $violations += "summary lists log '$name' but the file is not present in the bundle"
            }
        }
        foreach ($name in $actualSorted) {
            if ($expectedSorted -notcontains $name) {
                $violations += "log '$name' is present in the bundle but not listed in summary.json"
            }
        }
    }

    # --- Assertion 4: summary.json has a truthful sidecar of its own -------
    $summarySidecarPath = "$summaryPath.sha256"
    if (-not (Test-Path -LiteralPath $summarySidecarPath)) {
        $violations += "missing sidecar for summary.json: expected 'summary.json.sha256'"
    }
    else {
        # Same index guard as above: a zero-byte summary sidecar must be
        # reported as malformed, never crash into exit 2.
        $firstLines = @(Get-Content -LiteralPath $summarySidecarPath -TotalCount 1)
        $firstLine = ''
        if ($firstLines.Count -ge 1) { $firstLine = [string]$firstLines[0] }
        $tokens = @($firstLine -split '\s+' | Where-Object { $_.Length -gt 0 })
        $recorded = ''
        if ($tokens.Count -ge 1 -and $tokens[0] -match '^[0-9a-fA-F]{64}$') {
            $recorded = $tokens[0]
        }
        else {
            $violations += "malformed sidecar 'summary.json.sha256': first line is not '<64-hex-sha256>  <file>'"
        }
        if ($recorded) {
            $actual = Get-Sha256Hex -Path $summaryPath
            if (-not $recorded.Equals($actual, [StringComparison]::OrdinalIgnoreCase)) {
                $violations += (
                    "digest mismatch for 'summary.json': sidecar records " +
                    "$($recorded.Substring(0, 12))... but file hashes to " +
                    "$($actual.Substring(0, 12))...")
            }
        }
    }

    $script:bundleViolations = $violations
}

# ---------------------------------------------------------------------------
# Synthetic bundle fabrication for -SelfTest. Mirrors the exact artifact
# shape run-gate.ps1 emits, then applies one defect per non-good mode.
#   Mode: good | tampered-log | missing-sidecar | corrupt-summary |
#         summary-extra-log | tampered-summary
# ---------------------------------------------------------------------------
function New-SyntheticBundle {
    param([string]$Dir, [ValidateSet(
        'good', 'tampered-log', 'missing-sidecar',
        'corrupt-summary', 'summary-extra-log', 'tampered-summary')]
        [string]$Mode)

    New-Item -ItemType Directory -Path $Dir -Force | Out-Null

    # Two honest logs with truthful sidecars.
    $logTexts = @{ 'alpha.log' = "alpha step output - all green`nline two`n";
                   'beta.log'  = "beta step output - all green`n" }
    $digests = @{}
    foreach ($name in @('alpha.log', 'beta.log')) {
        $path = Join-Path $Dir $name
        Write-Utf8NoBomFile -Path $path -Text $logTexts[$name]
        $digests[$name] = Get-Sha256Hex -Path $path
        Write-Utf8NoBomFile -Path "$path.sha256" -Text (
            "$($digests[$name])  $name`n")
    }

    # Summary over the real logs (+ one phantom entry for the extra-log mode,
    # so the summary/list divergence exists before the sidecar is written -
    # the sidecar stays internally consistent with the tampered summary).
    $stepNames = @('alpha.log', 'beta.log')
    if ($Mode -eq 'summary-extra-log') {
        $stepNames += 'gamma.log'
    }
    $steps = @()
    foreach ($name in $stepNames) {
        $digest = $digests[$name]
        if (-not $digest) { $digest = ('f' * 64) }
        $steps += [ordered]@{
            name             = $name.Replace('.log', '')
            command          = "synthetic-$($name.Replace('.log', ''))"
            exitCode         = 0
            timedOut         = $false
            durationMs       = 1
            log              = (Join-Path $Dir $name)
            logSha256Sidecar = "$(Join-Path $Dir $name).sha256"
            sha256           = $digest
        }
    }
    $summary = [ordered]@{
        schemaVersion   = 1
        generator       = 'scripts/verify/assert-bundle-integrity.ps1 (synthetic self-test bundle)'
        generatedAt     = '2026-01-01T00:00:00.000Z'
        overallPass     = $true
        overallExitCode = 0
        failedSteps     = @()
        steps           = @($steps)
    }
    $json = $summary | ConvertTo-Json -Depth 6
    $null = $json | ConvertFrom-Json   # fabricated summaries must round-trip too
    $summaryPath = Join-Path $Dir 'summary.json'
    Write-Utf8NoBomFile -Path $summaryPath -Text ($json + "`n")
    $summaryDigest = Get-Sha256Hex -Path $summaryPath
    Write-Utf8NoBomFile -Path "$summaryPath.sha256" -Text (
        "$summaryDigest  summary.json`n")

    # Post-mutations, applied AFTER the sidecars exist so the defect is real.
    switch ($Mode) {
        'tampered-log' {
            # Flip bytes in alpha.log behind its sidecar's back.
            Add-Content -LiteralPath (Join-Path $Dir 'alpha.log') `
                -Value 'TAMPERED BYTE RANGE' -Encoding UTF8
        }
        'missing-sidecar' {
            Remove-Item -LiteralPath (Join-Path $Dir 'beta.log.sha256') -Force
        }
        'corrupt-summary' {
            Write-Utf8NoBomFile -Path $summaryPath -Text '{ this is ] not json'
        }
        'tampered-summary' {
            # Still-valid JSON, different bytes than what the sidecar covers.
            Write-Utf8NoBomFile -Path $summaryPath -Text ($json + "`n`n")
        }
    }
}

# ---------------------------------------------------------------------------
# Self-test driver.
# ---------------------------------------------------------------------------
function Invoke-SelfTest {
    param([string]$ScriptPath, [string]$ScratchRoot, [string]$TranscriptPath)

    # Unique scratch dir without [guid]::NewGuid() (CLM-blocked): timestamp
    # plus random component.
    $scratch = Join-Path $ScratchRoot (
        "bundle-selftest-{0}-{1}" -f
        (Get-Date).ToString('yyyyMMdd-HHmmss'), (Get-Random -Maximum 99999))
    New-Item -ItemType Directory -Path $scratch -Force | Out-Null

    $cases = @(
        @{ Mode = 'good';              Expect = $exitCodeValid;        Label = 'intact bundle verifies clean' }
        @{ Mode = 'tampered-log';      Expect = $exitCodeInvalid;      Label = 'log tampered behind sidecar' }
        @{ Mode = 'missing-sidecar';   Expect = $exitCodeInvalid;      Label = 'log sidecar removed' }
        @{ Mode = 'corrupt-summary';   Expect = $exitCodeInvalid;      Label = 'summary.json is not JSON' }
        @{ Mode = 'summary-extra-log'; Expect = $exitCodeInvalid;      Label = 'summary lists a phantom log' }
        @{ Mode = 'tampered-summary';  Expect = $exitCodeInvalid;      Label = 'summary.json edited after sidecar' }
        # Infrastructure class: a nonexistent bundle directory must yield the
        # dedicated exit 2, never a false "valid" or plain "invalid".
        @{ Mode = $null;               Expect = $exitCodeInfrastructure; Label = 'nonexistent artifacts directory' }
    )

    $timestamp = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    $lines = @(
        "assert-bundle-integrity self-test $timestamp"
        "language mode: $($ExecutionContext.SessionState.LanguageMode)"
    )
    $failures = 0
    try {
        foreach ($case in $cases) {
            if ($null -eq $case.Mode) {
                $target = Join-Path $scratch 'does-not-exist'
            }
            else {
                $target = Join-Path $scratch $case.Mode
                New-SyntheticBundle -Dir $target -Mode $case.Mode
            }
            # Recursive invocation proves the real exit-code contract
            # end-to-end, in whatever language mode this session runs.
            # Two invocation shapes are impossible here:
            #   * in-process `& $ScriptPath`: the target script ends every
            #     path in `exit`, which would tear down THIS session instead
            #     of yielding a per-case code;
            #   * `& $psExe ... > $null 2>&1`: redirecting a native child's
            #     output through PowerShell pipe-captures its stdio and
            #     deadlocks on hosts where piped stdio capture is denied
            #     (observed live in the isolated verification lane).
            # Start-Process with inherited stdio avoids both problems,
            # needs no persistent machine setting, and works under
            # ConstrainedLanguage.
            $psExe = (Get-Process -Id $PID).Path
            if (-not $psExe) {
                $psExe = (Get-Command -Name 'powershell.exe' -ErrorAction SilentlyContinue).Source
            }
            if (-not $psExe) {
                throw "cannot re-run '$ScriptPath': no host PowerShell executable found."
            }
            $child = Start-Process -FilePath $psExe `
                -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass',
                    '-File', "`"$ScriptPath`"",
                    '-ArtifactsDir', "`"$target`"") `
                -Wait -PassThru -WindowStyle Hidden
            $code = $child.ExitCode
            if ($null -eq $code) {
                # A stale/missing exit code would silently validate garbage;
                # treat "no code" as failure.
                $code = -1
            }
            $ok = ($code -eq $case.Expect)
            if (-not $ok) { $failures++ }
            $lines += (
                "{0} case={1,-18} expected={2} actual={3}  # {4}" -f
                $(if ($ok) { 'PASS' } else { 'FAIL' }),
                $(if ($null -eq $case.Mode) { '<missing-dir>' } else { $case.Mode }),
                $case.Expect, $code, $case.Label)
            Write-Host ("assert-bundle-integrity self-test: [{0}] {1}: exit {2} (expected {3})" -f
                $(if ($ok) { 'PASS' } else { 'FAIL' }), $case.Label, $code, $case.Expect)
        }
    }
    finally {
        $lines += (
            "verdict: {0} ({1}/{2} cases passed)" -f
            $(if ($failures -eq 0) { 'SELFTEST-PASS' } else { 'SELFTEST-FAIL' }),
            ($cases.Count - $failures), $cases.Count)
        # Evidence trail in the declared logs scope; best-effort only - the
        # verdict is decided by the cases, not by the transcript write.
        try {
            $reportDir = Split-Path -Parent $TranscriptPath
            if ($reportDir -and -not (Test-Path -LiteralPath $reportDir)) {
                New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
            }
            Write-Utf8NoBomFile -Path $TranscriptPath -Text (($lines -join "`n") + "`n")
        }
        catch {
            Write-Host "assert-bundle-integrity self-test: could not write transcript '$TranscriptPath': $($_.Exception.Message)" -ForegroundColor Yellow
        }
        try {
            Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
        }
        catch {
        }
    }

    if ($failures -gt 0) {
        Write-ConsoleError "assert-bundle-integrity self-test: $failures of $($cases.Count) cases FAILED."
        exit $exitCodeInvalid
    }
    Write-Host ("assert-bundle-integrity self-test: all {0} cases passed." -f $cases.Count)
}

# ---------------------------------------------------------------------------
# Entry point.
# ---------------------------------------------------------------------------
try {
    if ($SelfTest) {
        $repoRoot = Resolve-FullPath (Join-Path $PSScriptRoot '..\..')
        if (-not $ReportPath) {
            $ReportPath = Join-Path $repoRoot (
                'logs\verify-bundle-selftest\selftest-last-run.log')
        }
        if (-not $ScratchRoot) {
            $ScratchRoot = Join-Path $repoRoot (
                'logs\verify-bundle-selftest\scratch')
        }
        Invoke-SelfTest -ScriptPath $PSCommandPath `
            -ScratchRoot (Resolve-AbsolutePath $ScratchRoot) `
            -TranscriptPath (Resolve-AbsolutePath $ReportPath)
        # Invoke-SelfTest only exits the process itself when cases FAILED
        # (exit 1); reaching this line means every case matched its expected
        # verdict, so the self-test contract demands a clean exit 0.
        exit $exitCodeValid
    }

    if ([string]::IsNullOrWhiteSpace($ArtifactsDir)) {
        Write-ConsoleError "assert-bundle-integrity: pass a gate artifacts directory (or -SelfTest)."
        exit $exitCodeInfrastructure
    }

    $resolved = Resolve-AbsolutePath $ArtifactsDir
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        # Caller error, not a verdict about any bundle: infrastructure.
        Write-ConsoleError "assert-bundle-integrity: artifacts directory '$resolved' does not exist or is not a directory."
        exit $exitCodeInfrastructure
    }

    # Result comes back via $script:bundleViolations (PS 5.1 pipeline
    # quirk workaround, see Get-BundleViolations); $null = swallows any
    # stray non-violation output.
    $null = Get-BundleViolations -BundleDir $resolved
    $violations = @($script:bundleViolations)
    if ($violations.Count -eq 0) {
        Write-Host "assert-bundle-integrity: VALID - '$resolved' (logs, sidecars and summary.json are mutually consistent)."
        exit $exitCodeValid
    }
    Write-Host "assert-bundle-integrity: INVALID - '$resolved' has $($violations.Count) violation(s):"
    foreach ($v in $violations) {
        Write-Host "  - $v"
    }
    exit $exitCodeInvalid
}
catch {
    Write-ConsoleError "assert-bundle-integrity: infrastructure error: $($_.Exception.Message)"
    exit $exitCodeInfrastructure
}
