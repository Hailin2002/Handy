<#
.SYNOPSIS
    Deterministic diff of two run-gate summary.json bundles for pre/post
    verification comparisons.

.DESCRIPTION
    Compares two summary.json files produced by scripts/verify/run-gate.ps1
    and reports every material difference while ignoring volatile noise, so
    two runs of the same gate on the same tree diff clean even though their
    timestamps, durations and artifact locations differ.

    Normalized away (volatile, never compared):
        generatedAt, startedAt, totalDurationMs          top-level timestamps/durations
        repositoryRoot, artifactsDir                     absolute paths
        steps[].durationMs                               per-step timing
        steps[].log, steps[].logSha256Sidecar            absolute paths

    Compared per step (matched by the "name" field, case-sensitively):
        exitCode     the child's exit code
        timedOut     wrapper TIMED OUT verdict
        sha256       SHA256 over the step's log CONTENT (recorded by the gate)

    Plus set differences: steps present only in one bundle are reported as
    addedSteps / removedSteps.

    The "command" field is deliberately NOT compared: it embeds the package
    manager's absolute path ('auto' may resolve bun vs npm per machine), so
    comparing it would produce machine-dependent false positives.

    Output:
      stdout   exactly one JSON document (machine-readable):
               { schemaVersion, generator, generatedAt, baseline, current,
                 identical, addedSteps, removedSteps, normalizedFields,
                 differences[], summary }
               where each differences[] entry is
               { step, field, baseline, current } ("<missing>" marks a field
               absent from one side).
      stderr   one human-readable summary line.

    Exit codes:
      0   bundles are materially identical after normalization
      1   at least one material difference (or added/removed step)
      2   infrastructure error: missing/unreadable input, invalid JSON,
          malformed bundle, duplicate step names

    Arguments may point at a summary.json file directly or at a directory
    containing one (the run-gate artifacts directory layout).

    ConstrainedLanguage compatibility follows run-gate.ps1's conventions:
    [Console]::Error.WriteLine is wrapped in a cmdlet fallback; no generic
    types, Stopwatch or [IO.File] calls are used.
#>

[CmdletBinding()]
param(
    # Baseline bundle: a summary.json file or a directory containing one.
    [Parameter(Mandatory = $true)]
    [string]$Baseline,

    # Current bundle: a summary.json file or a directory containing one.
    [Parameter(Mandatory = $true)]
    [string]$Current
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$exitIdentical = 0
$exitDifferent = 1
$exitInfrastructure = 2

# ---------------------------------------------------------------------------
# CLM-tolerant helpers (same pattern as run-gate.ps1).
# ---------------------------------------------------------------------------

# Writes one line to stderr; [Console]::Error.WriteLine is CLM-blocked.
function Write-ConsoleError {
    param([string]$Message)

    try {
        [Console]::Error.WriteLine($Message)
    }
    catch {
        Write-Host $Message -ForegroundColor Yellow
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

try {
    # ---------------------------------------------------------------------
    # Locate and load both bundles.
    # ---------------------------------------------------------------------
    function Resolve-SummaryPath {
        param([string]$Argument, [string]$Label)

        $path = Resolve-AbsolutePath $Argument
        if (Test-Path -LiteralPath $path -PathType Container) {
            $path = Join-Path $path 'summary.json'
        }
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "$Label bundle: no summary.json found at '$path'."
        }
        return $path
    }

    function Read-SummaryBundle {
        param([string]$Path, [string]$Label)

        $raw = Get-Content -LiteralPath $Path -Raw
        $bundle = $raw | ConvertFrom-Json
        if ($null -eq $bundle -or
            $bundle.GetType().Name -notin @('PSCustomObject', 'PSObject')) {
            throw "$Label bundle '$Path' is not a JSON object."
        }
        if (-not $bundle.PSObject.Properties['steps']) {
            throw "$Label bundle '$Path' has no 'steps' field; not a run-gate summary."
        }
        return $bundle
    }

    $baselinePath = Resolve-SummaryPath -Argument $Baseline -Label 'baseline'
    $currentPath = Resolve-SummaryPath -Argument $Current -Label 'current'

    $baselineBundle = Read-SummaryBundle -Path $baselinePath -Label 'baseline'
    $currentBundle = Read-SummaryBundle -Path $currentPath -Label 'current'

    # ---------------------------------------------------------------------
    # Project a bundle onto its comparable form: steps keyed by name, each
    # reduced to exitCode / timedOut / sha256 (log-content hash).
    # ---------------------------------------------------------------------
    function Get-ComparableStep {
        param([object]$Step, [string]$BundleLabel)

        $nameProp = $Step.PSObject.Properties['name']
        if (-not $nameProp -or [string]::IsNullOrEmpty([string]$nameProp.Value)) {
            throw "$BundleLabel bundle contains a step without a 'name'."
        }

        $comparable = @{}
        foreach ($field in @('exitCode', 'timedOut', 'sha256')) {
            $prop = $Step.PSObject.Properties[$field]
            $comparable[$field] = if ($prop) {
                [string]$prop.Value
            }
            else {
                $null   # absent on this side -> reported, never silently equal
            }
        }
        return @{ name = [string]$nameProp.Value; fields = $comparable }
    }

    # Returns an array of comparable steps. Matching is done later with
    # case-SENSITIVE (-ceq) comparisons: PowerShell hashtables are
    # case-insensitive by default, which would both merge distinct steps
    # whose names differ only in case into a false "duplicate" and hide a
    # case-only step rename as "no change".
    function Get-ComparableSteps {
        param([object]$Bundle, [string]$BundleLabel)

        $steps = @()
        foreach ($step in @($Bundle.steps)) {
            $comparable = Get-ComparableStep -Step $step -BundleLabel $BundleLabel
            foreach ($existing in @($steps)) {
                if ($existing.name -ceq $comparable.name) {
                    throw "$BundleLabel bundle contains duplicate step name '$($comparable.name)'."
                }
            }
            $steps += $comparable
        }
        return @($steps)
    }

    $baselineSteps = Get-ComparableSteps -Bundle $baselineBundle -BundleLabel 'baseline'
    $currentSteps = Get-ComparableSteps -Bundle $currentBundle -BundleLabel 'current'

    # ---------------------------------------------------------------------
    # Compare: removed/added steps first, then field-level diffs.
    # Matching is case-sensitive; see Get-ComparableSteps.
    # ---------------------------------------------------------------------
    function Find-Step {
        param([object[]]$Steps, [string]$Name)

        foreach ($step in $Steps) {
            if ($step.name -ceq $Name) { return $step }
        }
        return $null
    }

    $removedNames = @()
    foreach ($baseStep in $baselineSteps) {
        if ($null -eq (Find-Step -Steps $currentSteps -Name $baseStep.name)) {
            $removedNames += $baseStep.name
        }
    }
    $addedNames = @()
    foreach ($currStep in $currentSteps) {
        if ($null -eq (Find-Step -Steps $baselineSteps -Name $currStep.name)) {
            $addedNames += $currStep.name
        }
    }

    $differences = @()
    foreach ($baseStep in ($baselineSteps | Sort-Object -Property name)) {
        # Names are unique within each bundle (enforced above), so at most
        # one match exists and it is a step present on both sides.
        $currStep = Find-Step -Steps $currentSteps -Name $baseStep.name
        if ($null -eq $currStep) { continue }

        foreach ($field in @('exitCode', 'timedOut', 'sha256')) {
            $baseValue = $baseStep.fields[$field]
            $currValue = $currStep.fields[$field]
            # [string]$null is '' , which would equate an absent field with an
            # empty one; keep absence distinguishable.
            $baseShown = if ($null -ne $baseValue) { $baseValue } else { '<missing>' }
            $currShown = if ($null -ne $currValue) { $currValue } else { '<missing>' }
            if ($baseValue -cne $currValue) {
                $differences += [ordered]@{
                    step     = $baseStep.name
                    field    = $field
                    baseline = $baseShown
                    current  = $currShown
                }
            }
        }
    }

    $removedSteps = @($removedNames | Sort-Object)
    $addedSteps = @($addedNames | Sort-Object)

    $identical = (($differences.Count -eq 0) -and
        ($addedSteps.Count -eq 0) -and ($removedSteps.Count -eq 0))

    # ---------------------------------------------------------------------
    # Emit the machine-readable JSON document on stdout.
    # ---------------------------------------------------------------------
    $verdict = if ($identical) { 'IDENTICAL' } else { 'DIFFERENT' }
    $result = [ordered]@{
        schemaVersion    = 1
        generator        = 'scripts/verify/diff-gate-summaries.ps1'
        generatedAt      = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        baseline         = $baselinePath
        current          = $currentPath
        identical        = $identical
        addedSteps       = @($addedSteps)
        removedSteps     = @($removedSteps)
        normalizedFields = @(
            'generatedAt', 'startedAt', 'totalDurationMs',
            'repositoryRoot', 'artifactsDir',
            'steps[].durationMs', 'steps[].log', 'steps[].logSha256Sidecar'
        )
        comparedFields   = @('steps[].exitCode', 'steps[].timedOut', 'steps[].sha256')
        differences      = @($differences)
        summary          = (
            "gate-summary diff: $verdict - {0} difference(s), {1} added step(s), {2} removed step(s)" -f
            $differences.Count, $addedSteps.Count, $removedSteps.Count)
    }

    $json = $result | ConvertTo-Json -Depth 6
    $null = $json | ConvertFrom-Json   # must round-trip or we do not ship it

    Write-Output $json

    # Human-readable line goes to stderr so stdout stays pure JSON.
    Write-ConsoleError ("diff-gate-summaries: {0}" -f $result.summary)

    exit $(if ($identical) { $exitIdentical } else { $exitDifferent })
}
catch {
    Write-ConsoleError "diff-gate-summaries: infrastructure error: $($_.Exception.Message)"
    exit $exitInfrastructure
}
