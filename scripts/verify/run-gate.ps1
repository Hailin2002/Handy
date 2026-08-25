<#
.SYNOPSIS
    One-command verification gate: sequentially runs every repository check,
    writing a per-step log, a SHA256 sidecar per log, and a machine-readable
    summary.json.

.DESCRIPTION
    Runs the full verification sequence for this repository, one step at a
    time, in a fixed order:

        lint            -> <pm> run lint
        format:check    -> <pm> run format:check
        frontend-build  -> <pm> run build
        cargo-fmt       -> cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
        cargo-clippy    -> cargo clippy --manifest-path src-tauri/Cargo.toml <clippy args>
        cargo-test      -> cargo test  --manifest-path src-tauri/Cargo.toml

    Every step is executed through scripts/verify/run-with-timeout.ps1 (the
    verification wrapper leaf), which streams combined stdout/stderr into a
    dedicated per-step log file, enforces a hard wall-clock timeout, and
    returns the child exit code (124 on timeout, 125 on start failure).

    Artifacts written under -ArtifactsDir (default: <repo>/logs/verify-gate,
    which .gitignore already covers):

        <step>.log            combined output of the step
        <step>.log.sha256     "<sha256>  <logfile>" sidecar, verifiable with
                              Get-FileHash against the log
        summary.json          machine-readable result of the whole run
        fingerprint.json      machine-readable environment preflight snapshot
                              captured BEFORE the first step (see below)
        fingerprint.json.sha256
                              "<sha256>  fingerprint.json" sidecar, mirroring
                              the summary.json sidecar contract

    Environment preflight (task-0308): before the first step executes, the
    runner captures a zero-cost fingerprint of the verification environment
    into fingerprint.json, reusing the capture patterns of
    scripts/verify/capture-env-fingerprint.ps1 where compatible:

        package manager resolution (F1/F2)   bun/npm probe result + sources
        git core.autocrlf         (F3)       checkout line-ending setting
        CMAKE_BUILD_PARALLEL_LEVEL(F4)       shell env vs. repo serial pin
        VULKAN_SDK presence       (F5)       SDK path / glslc on PATH
        node/npm/cargo/bun versions          first line of `<tool> --version`
        OS build                             Windows build number + UBR

    Every fact carries a stable key name and a `classification` tag
    (machine-local | harness-local | repo-enforced) citing
    docs/meeting-mode-verification-environment.md, so later evidence readers
    can attribute a failure to the exact environment class deterministically.
    Individual lookups fail SOFT (null + recorded) - a missing tool is data,
    not an error. Only a structural preflight failure (e.g. fingerprint.json
    cannot be written) aborts the gate with exit 2 and a console message;
    a successful preflight never changes step results or the overall exit
    code.

    Idempotency contract (gate G3): the artifacts directory is emptied at the
    start of every run, so a second invocation leaves exactly the artifacts of
    that invocation and nothing from any earlier run.

    Exit codes:
      0   every executed step returned 0
      1   at least one step failed (see summary.json for per-step codes)
      2   the gate runner itself hit an infrastructure error (bad arguments,
          unwritable artifacts directory, preflight failure, ...) before or
          during bookkeeping

    ConstrainedLanguage compatibility (see also run-with-timeout.ps1): the
    runner itself completes under ConstrainedLanguage in a degraded mode.
    Replaced .NET constructs: Stopwatch -> Get-Date/New-TimeSpan deltas;
    Generic.List[object] -> plain array; [IO.File]::WriteAllText +
    UTF8Encoding::new -> Write-Utf8NoBomFile helper (Set-Content fallback,
    which on Windows PowerShell 5.1 prepends a BOM); Get-FileHash ->
    Get-Sha256Hex helper with certutil.exe fallback (residual CLM limit:
    certutil emits lowercase hex where Get-FileHash emits uppercase);
    [Console]::Error and
    [IO.Path]::GetFullPath calls are wrapped with cmdlet equivalents; the
    $PSBoundParameters.ContainsKey bound-parameter probe falls back to a
    value comparison that is exact because -StepTimeoutMs has
    ValidateRange(1000, ...).
    Residual CLM limits (verified live 2026-08-22): certutil emits lowercase
    hex where Get-FileHash emits uppercase, and Set-Content prepends a BOM;
    additionally ConstrainedLanguage does not recognize named parameters in
    array-splat form (@('-X','y') binds as positional values), which the step
    invocation below avoids by hash-splatting.
    Each step still runs through run-with-timeout.ps1, whose core is NOT
    CLM-reducible, so every step exits 2 ("unsupported language mode") and
    the gate reports FAIL - explicitly and with artifacts, not with cryptic
    "Cannot create type" engine errors.

.NOTES
    Steps run strictly sequentially ("serial"): no two toolchains ever overlap,
    which is what keeps cargo invocations from contending for the target
    directory lock. The runner never aborts the sequence on a failing step;
    later steps still run so one bad check cannot hide the state of the rest.
#>

[CmdletBinding()]
param(
    # Directory for logs, sidecars and summary.json. Created if missing;
    # EMPTIED at the start of every run (idempotency).
    [string]$ArtifactsDir,

    # Subset of step names to run (default: all). Names must match the
    # built-in step table. Mainly for targeted re-verification; the default
    # single invocation always covers the whole gate.
    [ValidateSet(
        'lint', 'format-check', 'frontend-build',
        'cargo-fmt', 'cargo-clippy', 'cargo-test'
    )]
    [string[]]$Steps,

    # Optional uniform wall-clock budget in milliseconds overriding the
    # built-in per-step defaults.
    [ValidateRange(1000, [int]::MaxValue)]
    [int]$StepTimeoutMs,

    # JavaScript package manager used for the frontend steps. 'auto'
    # (default) prefers bun and falls back to npm when bun is not on PATH;
    # 'bun' / 'npm' force one explicitly and fail with exit 2 when missing.
    [ValidateSet('auto', 'bun', 'npm')]
    [string]$PackageManager = 'auto'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$exitCodeInfrastructure = 2

# ---------------------------------------------------------------------------
# ConstrainedLanguage-tolerant helpers. Each one uses the same .NET call as
# before on FullLanguage hosts (byte-identical artifacts) and falls back to
# cmdlet/core-type equivalents when the language mode blocks the method.
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

# UTF-8 without BOM writer. Primary path is byte-identical to the previous
# direct [IO.File]::WriteAllText + UTF8Encoding($false) calls; under CLM the
# .NET write is blocked and Set-Content keeps the run alive (Windows
# PowerShell 5.1 Set-Content prepends a BOM - documented residual limit).
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

# SHA256 of a file as uppercase hex. Get-FileHash cannot instantiate its
# hash provider under CLM ("Cannot create type"); certutil.exe is a native
# tool and needs no .NET, printing "<64 hex>" on its own line.
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
            throw "run-gate: could not SHA256-hash '$Path' (Get-FileHash unavailable and certutil fallback produced no digest)."
        }
        return (@($hexLines) -join '')
    }
}

try {
    # ---------------------------------------------------------------------
    # Locate the repository root and the timeout wrapper.
    # ---------------------------------------------------------------------
    $repoRoot = Resolve-FullPath (Join-Path $PSScriptRoot '..\..')
    $wrapper = Join-Path $PSScriptRoot 'run-with-timeout.ps1'
    if (-not (Test-Path -LiteralPath $wrapper)) {
        Write-ConsoleError "run-gate: timeout wrapper not found at '$wrapper'."
        exit $exitCodeInfrastructure
    }

    # The gate must behave identically regardless of the caller's working
    # directory: all step commands below are expressed relative to the root.
    # Relative -ArtifactsDir values are resolved against the caller's
    # location BEFORE we move, and Push-/Pop-Location restore it on exit.
    if (-not $ArtifactsDir) {
        # 'logs/' is already covered by .gitignore.
        $ArtifactsDir = Join-Path $repoRoot 'logs\verify-gate'
    }
    $resolvedArtifacts = Resolve-AbsolutePath $ArtifactsDir
    # Refuse artifact directories that CONTAIN the repository root: emptying
    # those at startup (below) would delete the repository itself. Directories
    # inside the repo (the default) or anywhere else are fine.
    $artifactsPrefix = $resolvedArtifacts.TrimEnd('\') + '\'
    $repoPrefix = $repoRoot.TrimEnd('\') + '\'
    if ($repoPrefix.StartsWith($artifactsPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Write-ConsoleError "run-gate: refusing -ArtifactsDir '$resolvedArtifacts': it contains the repository root."
        exit $exitCodeInfrastructure
    }
    Push-Location -LiteralPath $repoRoot

    # ---------------------------------------------------------------------
    # Prepare the artifacts directory: idempotent reset (gate G3).
    # Moved AHEAD of the preflight and package-manager resolution so that
    # every gate run - including one that exits 2 before the first step -
    # leaves THIS run's fingerprint.json evidence in a freshly reset
    # directory instead of stale artifacts from an earlier run.
    # ---------------------------------------------------------------------
    if (-not (Test-Path -LiteralPath $resolvedArtifacts)) {
        New-Item -ItemType Directory -Path $resolvedArtifacts -Force |
            Out-Null
    }
    else {
        # Remove everything from previous runs so no stale log, sidecar,
        # summary or fingerprint can survive into this run's results.
        Get-ChildItem -LiteralPath $resolvedArtifacts -Force |
            Remove-Item -Recurse -Force
    }

    # ---------------------------------------------------------------------
    # Environment preflight (task-0308): zero-cost fact capture BEFORE the
    # first step. Reuses the capture patterns of capture-env-fingerprint.ps1
    # (tool-version probing, registry build readout, soft-fail lookups) in a
    # reduced, gate-focused form; every fact is tagged with its environment
    # class per docs/meeting-mode-verification-environment.md.
    # ---------------------------------------------------------------------

    # Resolver for the frontend steps' package manager. Definition only at
    # this point; invoked once inside the preflight so its outcome is part of
    # the fingerprint evidence.
    function Resolve-PackageManagerCommand {
        param([string]$Mode)

        $preference = switch ($Mode) {
            'bun' { @('bun') }
            'npm' { @('npm.cmd', 'npm') }
            default { @('bun', 'npm.cmd', 'npm') }
        }
        foreach ($candidate in $preference) {
            $found = Get-Command -Name $candidate -ErrorAction SilentlyContinue
            # Only real executables are usable: the timeout wrapper starts
            # children through System.Diagnostics.ProcessStartInfo, which
            # cannot launch .ps1 shims directly.
            if ($found -and $found.Source -and
                ($found.Source -like '*.exe' -or $found.Source -like '*.cmd' -or
                    $found.Source -like '*.bat')) {
                return [ordered]@{
                    name   = if ($candidate -like 'npm*') { 'npm' } else { 'bun' }
                    source = $found.Source
                }
            }
        }
        return $null
    }

    # Same probe shape as Get-ToolVersion in capture-env-fingerprint.ps1:
    # first stdout line of `<exe> --version`, $null when absent. Never
    # throws; PowerShell's own pipeline captures the output, so this is not
    # affected by the sandbox's named-pipe restriction (F6).
    function Get-PreflightToolVersion {
        param([string]$Exe)

        $resolved = Get-Command $Exe -ErrorAction SilentlyContinue
        if (-not $resolved) { return $null }
        try {
            $output = @(& $Exe --version 2>$null)
            if (-not $output) { return $null }
            return ('{0}' -f @($output)[0]).Trim()
        } catch {
            return $null
        }
    }

    try {
        $preflightCapturedAt = (Get-Date).ToUniversalTime().ToString(
            'yyyy-MM-ddTHH:mm:ss.fffZ')

        # --- F1/F2: bun/npm resolution, probed once here so the outcome
        # lands in the fingerprint and the step table reuses exactly this
        # result.
        $bunCommand     = Get-Command 'bun' -ErrorAction SilentlyContinue
        $npmPs1Command  = Get-Command 'npm' -ErrorAction SilentlyContinue
        $pmResolution   = Resolve-PackageManagerCommand -Mode $PackageManager

        # --- F3: git checkout line-ending setting. Unset -> null (git then
        # applies its built-in default); the repo side (.prettierrc pins LF)
        # is repo-enforced and recorded as the static counterpart.
        $autocrlfValue = $null
        try {
            $autocrlfOut = @(& git config --get core.autocrlf 2>$null)
            if ($LASTEXITCODE -eq 0 -and @($autocrlfOut).Count -gt 0) {
                $autocrlfValue = ('{0}' -f @($autocrlfOut)[0]).Trim()
            }
        } catch { $autocrlfValue = $null }

        # --- F4: serial-build sanity. The pin lives in .cargo/config.toml
        # ([env]) and is therefore repo-enforced; it applies to cargo build
        # scripts, so an unset shell variable is normal and recorded as-is.
        $cmakeParallelEnv = if ($env:CMAKE_BUILD_PARALLEL_LEVEL) {
            $env:CMAKE_BUILD_PARALLEL_LEVEL
        } else { $null }

        # --- F5: Vulkan SDK visibility of THIS shell (machine-local;
        # restricted harness shells do not always inherit the machine env).
        $vulkanSdkPath = if ($env:VULKAN_SDK) { $env:VULKAN_SDK } else { $null }
        $glslcCommand  = Get-Command 'glslc' -ErrorAction SilentlyContinue

        # --- OS build (registry readout, same source as
        # capture-env-fingerprint.ps1). Individual lookups fail SOFT per the
        # preflight contract: -ErrorAction SilentlyContinue plus a $cv null
        # guard turn a restricted/missing key into fallback values instead
        # of aborting the gate; only structural failures exit 2.
        $osVersionInfo = [Environment]::OSVersion.Version
        $cv = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction SilentlyContinue
        $buildNumber = if ($cv -and $cv.PSObject.Properties['CurrentBuildNumber']) {
            [string]$cv.CurrentBuildNumber
        } else { '{0}' -f $osVersionInfo.Build }
        $ubr = if ($cv -and $cv.PSObject.Properties['UBR']) {
            [string]$cv.UBR
        } else { '{0}' -f $osVersionInfo.Revision }

        $verificationDoc = 'docs/meeting-mode-verification-environment.md'

        $fingerprint = [ordered]@{
            schemaVersion  = 1
            generator      = 'scripts/verify/run-gate.ps1'
            capturedAt     = $preflightCapturedAt
            repositoryRoot = $repoRoot
            classificationReference = $verificationDoc
            facts          = [ordered]@{

                packageManagerResolution = [ordered]@{
                    factIds        = @('F1', 'F2')
                    classification = 'machine-local'
                    sourceDoc      = "$verificationDoc (F1, F2)"
                    requestedMode  = $PackageManager
                    resolvedName   = $(if ($pmResolution) { $pmResolution.name } else { $null })
                    resolvedSource = $(if ($pmResolution) { $pmResolution.source } else { $null })
                    bunOnPath      = [bool]$bunCommand
                    bunSource      = $(if ($bunCommand) { $bunCommand.Source } else { $null })
                    npmPowerShellShimPath = $(if ($npmPs1Command) { $npmPs1Command.Source } else { $null })
                    executionPolicy = ('{0}' -f (Get-ExecutionPolicy))
                }

                gitCoreAutocrlf = [ordered]@{
                    factIds        = @('F3')
                    classification = 'machine-local'
                    sourceDoc      = "$verificationDoc (F3)"
                    value          = $autocrlfValue
                    repoEnforcedCounterpart = '.prettierrc: endOfLine "lf"'
                }

                cmakeBuildParallelLevel = [ordered]@{
                    factIds        = @('F4')
                    classification = 'repo-enforced'
                    sourceDoc      = "$verificationDoc (F4)"
                    shellEnvironmentValue   = $cmakeParallelEnv
                    repositoryPinValue      = '1'
                    matchesRepositoryPin    = ($cmakeParallelEnv -eq '1')
                    pinSource      = '.cargo/config.toml [env] CMAKE_BUILD_PARALLEL_LEVEL'
                }

                vulkanSdk = [ordered]@{
                    factIds        = @('F5')
                    classification = 'machine-local'
                    sourceDoc      = "$verificationDoc (F5)"
                    present        = [bool]$vulkanSdkPath
                    path           = $vulkanSdkPath
                    glslcOnPath    = [bool]$glslcCommand
                }

                toolchainVersions = [ordered]@{
                    factIds        = @()
                    classification = 'machine-local'
                    sourceDoc      = $verificationDoc
                    node           = Get-PreflightToolVersion 'node'
                    npm            = Get-PreflightToolVersion 'npm.cmd'
                    cargo          = Get-PreflightToolVersion 'cargo'
                    bun            = Get-PreflightToolVersion 'bun'
                }

                operatingSystem = [ordered]@{
                    factIds        = @()
                    classification = 'machine-local'
                    sourceDoc      = $verificationDoc
                    platform       = 'windows'
                    osVersion      = '{0}.{1}.{2}' -f $osVersionInfo.Major,
                                        $osVersionInfo.Minor, $osVersionInfo.Build
                    buildNumber    = $buildNumber
                    updateBuildRevision = $ubr
                    fullBuild      = '{0}.{1}' -f $buildNumber, $ubr
                    architecture   = if ([Environment]::Is64BitOperatingSystem) {
                        '64-bit'
                    } else {
                        '32-bit'
                    }
                }
            }
        }

        $fingerprintJson = $fingerprint | ConvertTo-Json -Depth 6
        $null = $fingerprintJson | ConvertFrom-Json   # must round-trip

        $fingerprintPath = Join-Path $resolvedArtifacts 'fingerprint.json'
        Write-Utf8NoBomFile -Path $fingerprintPath -Text (
            $fingerprintJson + [Environment]::NewLine)

        # Sidecar mirroring the per-log/summary contract.
        $fingerprintHash = Get-Sha256Hex -Path $fingerprintPath
        Write-Utf8NoBomFile -Path "$fingerprintPath.sha256" -Text (
            "$fingerprintHash  fingerprint.json`n")

        Write-Host ("run-gate: preflight: fingerprint written to {0}" -f
            $fingerprintPath)
    }
    catch {
        Write-ConsoleError ("run-gate: environment preflight failed: {0}" -f
            $_.Exception.Message)
        Pop-Location
        exit $exitCodeInfrastructure
    }

    # ---------------------------------------------------------------------
    # Frontend steps run on the package manager the preflight resolved; the
    # fingerprint under facts.packageManagerResolution records exactly this
    # result, so evidence and execution can never disagree.
    # ---------------------------------------------------------------------
    $resolvedPm = $pmResolution
    if (-not $resolvedPm) {
        # The fingerprint written above already carries the unresolved
        # outcome as evidence; exit 2 with a message (infrastructure error).
        Write-ConsoleError (
            "run-gate: neither bun nor npm was found on PATH; cannot run frontend steps.")
        Pop-Location
        exit $exitCodeInfrastructure
    }
    Write-Host ("run-gate: package manager: {0} ({1})" -f
        $resolvedPm.name, $resolvedPm.source)

    # ---------------------------------------------------------------------
    # Step table. Order defines execution order.
    # ---------------------------------------------------------------------
    $srcTauriManifest = 'src-tauri/Cargo.toml'
    $pmCommand = [string]$resolvedPm.source
    $allSteps = [ordered]@{
        'lint' = @{
            Command  = $pmCommand
            Args     = @('run', 'lint')
            TimeoutMs = 300000
        }
        'format-check' = @{
            Command  = $pmCommand
            Args     = @('run', 'format:check')
            TimeoutMs = 600000
        }
        'frontend-build' = @{
            Command  = $pmCommand
            Args     = @('run', 'build')
            TimeoutMs = 900000
        }
        'cargo-fmt' = @{
            Command  = 'cargo'
            Args     = @('fmt', '--manifest-path', $srcTauriManifest, '--', '--check')
            TimeoutMs = 180000
        }
        'cargo-clippy' = @{
            Command  = 'cargo'
            Args     = @('clippy', '--manifest-path', $srcTauriManifest, '--all-targets')
            TimeoutMs = 3600000
        }
        'cargo-test' = @{
            Command  = 'cargo'
            Args     = @('test', '--manifest-path', $srcTauriManifest)
            TimeoutMs = 5400000
        }
    }

    $selectedNames = if ($Steps) { @($Steps) } else { @($allSteps.Keys) }

    # The artifacts directory was already created and reset ahead of the
    # environment preflight above (so the fingerprint lands beside
    # summary.json even on early exits); nothing left to prepare here.

    # ---------------------------------------------------------------------
    # Execute the selected steps sequentially through the wrapper.
    # ---------------------------------------------------------------------
    # Plain array, not Generic.List[object]: New-Object on generic types is
    # blocked under ConstrainedLanguage; six steps do not need amortized
    # append.
    $results = @()
    $runStartedAt = (Get-Date).ToUniversalTime()

    foreach ($name in $selectedNames) {
        $step = $allSteps[$name]
        # $PSBoundParameters.ContainsKey is CLM-blocked ("Cannot invoke
        # method": the bound-parameters dictionary is not a core type), so a
        # fallback decides bound-ness from the value itself. Exact, because
        # -StepTimeoutMs has ValidateRange(1000, ...): an unbound parameter
        # is 0, any bound one is >= 1000.
        $stepTimeoutBound = $false
        try {
            $stepTimeoutBound = $PSBoundParameters.ContainsKey('StepTimeoutMs')
        }
        catch {
            $stepTimeoutBound = ($StepTimeoutMs -gt 0)
        }
        $timeoutMs = if ($stepTimeoutBound) {
            $StepTimeoutMs
        }
        else {
            $step.TimeoutMs
        }

        $logPath = Join-Path $resolvedArtifacts "$name.log"
        Write-Host "run-gate: [$name] starting: $($step.Command) $($step.Args -join ' ')"

        # Get-Date delta instead of System.Diagnostics.Stopwatch: identical
        # millisecond precision for step durations, and constructible in CLM.
        $stepStartedAt = Get-Date
        # Named-parameter splatting, NOT positional passing: on Windows
        # PowerShell 5.1 a positionally passed array collapses into a single
        # $ArgumentList element when the target uses ValueFromRemainingArguments,
        # which would hand cargo one giant quoted argument. Splatting binds
        # each element individually.
        $wrapperCall = @{
            FilePath    = $step.Command
            ArgumentList = @($step.Args)
            TimeoutMs   = $timeoutMs
            LogPath     = $logPath
        }
        & $wrapper @wrapperCall
        $stepExit = $LASTEXITCODE
        $durationMs = [int]((New-TimeSpan -Start $stepStartedAt -End (Get-Date)).TotalMilliseconds)

        $sidecarPath = "$logPath.sha256"
        if (-not (Test-Path -LiteralPath $logPath)) {
            # The wrapper exits 125/2 WITHOUT creating a log when it cannot
            # even start the child (e.g. tool missing from PATH, or CLM
            # guard). Emit a stub log here so every step unconditionally
            # produces a log + SHA256 sidecar pair.
            Write-ConsoleError "run-gate: [$name] wrapper produced no log (start failure?); writing stub"
            Write-Utf8NoBomFile -Path $logPath -Text (
                "[run-gate] no log was produced by the wrapper for this step " +
                "(exit $stepExit); the child process most likely failed to start.")
        }
        # Every log gets its sidecar, wrapper-produced or stub (gate G2).
        $hashHex = Get-Sha256Hex -Path $logPath
        # Same "<hash>  <file>" shape GNU coreutils sha256sum emits, so
        # both Get-FileHash comparisons and sha256sum -c accept it.
        Write-Utf8NoBomFile -Path $sidecarPath -Text (
            "$hashHex  $(Split-Path -Leaf $logPath)`n")

        # 124 is the wrapper's timeout code, but a child could also exit 124
        # on its own; only trust it when the log carries the wrapper's
        # explicit TIMED OUT marker.
        $timedOut = ($stepExit -eq 124) -and
            (Test-Path -LiteralPath $logPath) -and
            (Select-String -LiteralPath $logPath -SimpleMatch '[run-with-timeout] TIMED OUT' -Quiet)

        $results += [ordered]@{
            name         = $name
            command      = (@($step.Command) + @($step.Args)) -join ' '
            exitCode     = $stepExit
            timedOut     = $timedOut
            durationMs   = $durationMs
            log          = Resolve-FullPath $logPath
            logSha256Sidecar = Resolve-FullPath $sidecarPath
            sha256       = $hashHex
        }

        Write-Host ("run-gate: [{0}] finished: exit {1} ({2:N0} ms)" -f
            $name, $stepExit, $durationMs)
    }

    # ---------------------------------------------------------------------
    # Emit summary.json (validated before it lands on disk).
    # ---------------------------------------------------------------------
    $failed = @($results | Where-Object { $_.exitCode -ne 0 })
    $summary = [ordered]@{
        schemaVersion   = 1
        generator       = 'scripts/verify/run-gate.ps1'
        generatedAt     = (Get-Date).ToUniversalTime().ToString(
            'yyyy-MM-ddTHH:mm:ss.fffZ')
        startedAt       = $runStartedAt.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
        repositoryRoot  = $repoRoot
        artifactsDir    = $resolvedArtifacts
        packageManager  = [string]$resolvedPm.name
        overallPass     = ($failed.Count -eq 0)
        overallExitCode = $(if ($failed.Count -eq 0) { 0 } else { 1 })
        failedSteps     = @($failed | ForEach-Object { $_.name })
        totalDurationMs = [int]((New-TimeSpan -Start $runStartedAt -End (
                    Get-Date)).TotalMilliseconds)
        steps           = @(@($results))
    }

    $json = $summary | ConvertTo-Json -Depth 6
    $null = $json | ConvertFrom-Json   # must round-trip or we do not ship it

    $summaryPath = Join-Path $resolvedArtifacts 'summary.json'
    Write-Utf8NoBomFile -Path $summaryPath -Text ($json + [Environment]::NewLine)

    # Sidecar for the summary itself, mirroring the per-log contract.
    $summaryHash = Get-Sha256Hex -Path $summaryPath
    Write-Utf8NoBomFile -Path "$summaryPath.sha256" -Text (
        "$summaryHash  summary.json`n")

    Write-Host ("run-gate: overall {0}; summary: {1}" -f
        $(if ($summary.overallPass) { 'PASS' } else { 'FAIL' }),
        $summaryPath)

    Pop-Location
    exit $summary.overallExitCode
}
catch {
    # The push may not have happened yet (e.g. a bad -ArtifactsDir failed
    # during resolution); popping an empty stack must not replace the
    # infrastructure error - and its documented exit code 2 - with a new
    # unhandled exception.
    try { Pop-Location } catch { }
    Write-ConsoleError "run-gate: infrastructure error: $($_.Exception.Message)"
    exit $exitCodeInfrastructure
}
