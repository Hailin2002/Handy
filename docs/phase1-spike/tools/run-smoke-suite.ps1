# Unattended smoke-suite driver for the Phase 1 meeting-loopback spike.
#
# Composes the existing phase1-spike tools into ONE repeatable command:
#   1. plays the scenario tone via play-tone.ps1 (dual-source smoke: real audio
#      on the default output device while the spike captures it via WASAPI
#      loopback plus the microphone),
#   2. drives a SHORT bounded capture by the meeting_loopback_spike binary
#      (same launch convention as run-120min.ps1, but with a smoke-sized
#      schedule instead of its fixed 120-min one, whose first tone only starts
#      at t=300 s),
#   3. analyzes the resulting CSV via analyze-spike-csv.ps1 into SUMMARY.md,
#   4. emits a machine-readable verdict: JSON on stdout AND written to
#      <run-dir>/verdict.json.
#
# Usage:
#   pwsh -NoProfile -ExecutionPolicy Bypass -File run-smoke-suite.ps1 `
#        [-RunId <id>] [-DurationS 90] [-ToneSeconds 30] [-ToneStartDelayS 5]
#        [-OutputDevice <name>] [-InputDevice <name>] [-ExePath <spike.exe>]
#        [-DryRun]
#
# Idempotency (per run-id):
#   - Each run gets its own directory <OutRoot>\<RunId>; an existing directory
#     is NEVER mutated.
#   - Re-invoking with a run-id whose directory already contains a valid
#     verdict.json simply re-emits that stored verdict without touching
#     anything (idempotent replay).
#   - Re-invoking with a run-id whose directory exists WITHOUT a verdict.json
#     refuses (exit 3) and leaves the prior directory untouched; pick a new
#     run-id.
#
# Exit codes: 0 = verdict PASS; 1 = verdict FAIL;
#             2 = dry-run found unavailable tools / invalid settings;
#             3 = refused: run-id already used without a completed verdict;
#             4 = invalid arguments.

param(
    [string]$RunId,
    [string]$OutRoot,
    [int]$DurationS = 90,         # capture length; acceptance requires >= 60 s
    [int]$ToneSeconds = 30,       # tone playback window inside the capture
    [int]$ToneStartDelayS = 5,    # silence lead-in before the tone starts
    [string]$ExePath,             # meeting_loopback_spike binary; auto-detected if omitted
    [string]$OutputDevice,        # optional --output-device passthrough
    [string]$InputDevice,         # optional --input-device passthrough
    [switch]$DryRun               # validate tool availability only; no audio, no dirs
)

$ErrorActionPreference = 'Stop'

# --- constants ---------------------------------------------------------------
$MinCaptureS = 60              # acceptance gate G1: capture must span >= 60 s
$SpikeName   = 'meeting_loopback_spike.exe'
$RepoRoot    = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$PlayTool    = Join-Path $PSScriptRoot 'play-tone.ps1'
$AnalyzeTool = Join-Path $PSScriptRoot 'analyze-spike-csv.ps1'
$EventsLog   = 'events.log'    # marker vocabulary matches tools/README.md section 4

# --- helpers -----------------------------------------------------------------
function Emit([object]$obj) {
    $obj | ConvertTo-Json -Depth 8
}

function Fail-Args([string]$msg) {
    Emit @{ mode = $(if ($DryRun) { 'dryrun' } else { 'live' }); ok = $false; error = $msg } | Out-Host
    exit 4
}

# Quote-aware single-line CSV splitter (mirrors analyze-spike-csv.ps1 lines 28-44).
function Split-CsvLine([string]$line) {
    $fields = New-Object System.Collections.Generic.List[string]
    $cur = New-Object System.Text.StringBuilder
    $inQ = $false
    for ($i = 0; $i -lt $line.Length; $i++) {
        $c = $line[$i]
        if ($inQ) {
            if ($c -eq '"') {
                if ($i + 1 -lt $line.Length -and $line[$i + 1] -eq '"') { [void]$cur.Append('"'); $i++ }
                else { $inQ = $false }
            } else { [void]$cur.Append($c) }
        } else {
            if ($c -eq '"') { $inQ = $true }
            elseif ($c -eq ',') { $fields.Add($cur.ToString()); [void]$cur.Clear() }
            else { [void]$cur.Append($c) }
        }
    }
    $fields.Add($cur.ToString())
    return ,$fields.ToArray()
}

# Extracts the smoke-relevant statistics from a spike CSV (non-error rows only,
# matching the analyzer's convention).
function Get-CsvStats([string]$csvPath) {
    $stats = @{
        micRows = 0; loopRows = 0; micErrors = 0; loopErrors = 0
        micSpanMs = 0.0; loopSpanMs = 0.0; loopNonSilentRows = 0
    }
    if (-not (Test-Path $csvPath)) { return $stats }
    foreach ($line in [System.IO.File]::ReadLines((Resolve-Path $csvPath))) {
        if ($line.StartsWith('#')) { continue }
        if ($line -match '^\s*$') { continue }
        $f = Split-CsvLine $line
        if ($f[0] -eq 'source') { continue } # header
        $src = $f[0]; $err = $f[15]; $tRelMs = 0.0; $silent = $f[12]
        [void][double]::TryParse($f[2], [Globalization.NumberStyles]::Float,
            [Globalization.CultureInfo]::InvariantCulture, [ref]$tRelMs)
        if ($src -eq 'microphone') {
            if ($err -ne '') { $stats.micErrors++ } else { $stats.micRows++; if ($tRelMs -gt $stats.micSpanMs) { $stats.micSpanMs = $tRelMs } }
        } elseif ($src -eq 'loopback') {
            if ($err -ne '') { $stats.loopErrors++ } else {
                $stats.loopRows++
                if ($tRelMs -gt $stats.loopSpanMs) { $stats.loopSpanMs = $tRelMs }
                if ($silent -eq 'false') { $stats.loopNonSilentRows++ }
            }
        }
    }
    return $stats
}

# --- argument validation -----------------------------------------------------
if ($DurationS -lt $MinCaptureS) {
    Fail-Args "-DurationS must be >= $MinCaptureS (acceptance gate: capture span >= ${MinCaptureS}s); got $DurationS"
}
if ($ToneSeconds -lt 1 -or ($ToneStartDelayS + $ToneSeconds) -ge ($DurationS - 5)) {
    Fail-Args "tone window [$ToneStartDelayS..$($ToneStartDelayS + $ToneSeconds)s] must fit inside the capture with >= 5 s margin (DurationS=$DurationS)"
}
if (-not $RunId) { $RunId = 'smoke-' + (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss') }
if ($RunId -match '[\\/ ]') { Fail-Args "-RunId must not contain path separators or spaces: '$RunId'" }
if (-not $OutRoot) { $OutRoot = Join-Path $PSScriptRoot 'runs' }

# --- locate the spike binary -------------------------------------------------
$exeCandidates = @()
if ($ExePath) {
    $exeCandidates += $ExePath
} else {
    $exeCandidates += @(
        (Join-Path $RepoRoot "src-tauri\target\release\$SpikeName"),
        (Join-Path $RepoRoot "src-tauri\target\debug\$SpikeName")
    )
}
$exeFound = $null
foreach ($cand in $exeCandidates) {
    if ($cand -and (Test-Path $cand)) { $exeFound = (Resolve-Path $cand).Path; break }
}

# --- G3: dry-run validation (no audio capture, no directory creation) --------
if ($DryRun) {
    $checks = @(
        @{ name = 'spike_binary_available'; ok = [bool]$exeFound; detail = $(if ($exeFound) { $exeFound } else { "not found; tried: $($exeCandidates -join '; ')" }) },
        @{ name = 'play_tone_tool_available'; ok = (Test-Path $PlayTool); detail = $PlayTool },
        @{ name = 'analyzer_tool_available'; ok = (Test-Path $AnalyzeTool); detail = $AnalyzeTool },
        @{ name = 'capture_duration_valid'; ok = ($DurationS -ge $MinCaptureS); detail = "${DurationS}s >= ${MinCaptureS}s" },
        @{ name = 'tone_window_fits_capture'; ok = ($ToneStartDelayS + $ToneSeconds) -lt ($DurationS - 5); detail = "tone ends at $($ToneStartDelayS + $ToneSeconds)s of ${DurationS}s" }
    )
    $ok = ($checks | Where-Object { -not $_.ok }).Count -eq 0
    Emit @{
        mode = 'dryrun'; ok = $ok; run_id = $RunId
        would_use_exe = $exeFound
        checks = $checks
        note = 'dry-run validated tool availability only; no audio was captured and no directories were created'
    } | Out-Host
    if ($ok) { exit 0 } else { exit 2 }
}

# --- G2: idempotent per run-id, never mutate prior run directories ------------
$runDir = Join-Path $OutRoot $RunId
if (Test-Path $runDir) {
    $priorVerdict = Join-Path $runDir 'verdict.json'
    if (Test-Path $priorVerdict) {
        # Idempotent replay: return the stored verdict unchanged.
        $stored = Get-Content $priorVerdict -Raw
        $storedObj = $stored | ConvertFrom-Json
        Emit @{ replayed = $true; verdict_path = $priorVerdict; verdict = $storedObj } | Out-Host
        if ($storedObj.status -eq 'pass') { exit 0 } else { exit 1 }
    }
    Emit @{
        mode = 'live'; ok = $false; error = "run directory already exists without a verdict.json: $runDir"
        hint = 'the prior run directory was NOT mutated; choose a new -RunId'
    } | Out-Host
    exit 3
}
New-Item -ItemType Directory -Path $runDir -Force | Out-Null

$csv        = Join-Path $runDir 'meeting_loopback_spike_smoke.csv'
$eventsPath = Join-Path $runDir $EventsLog
$summaryMd  = Join-Path $runDir 'SUMMARY.md'
$startedUtc = (Get-Date).ToUniversalTime()

function Log-Event([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $msg
    Add-Content -Path $eventsPath -Value $line
    Write-Output $line
}

# --- G1: one command performs tone -> capture -> analyze ----------------------
$psHost = (Get-Process -Id $PID).Path
if (-not $psHost) { $psHost = 'powershell' }
if (-not $exeFound) {
    Log-Event "SMOKE_ABORT reason=spike_binary_not_found tried=$($exeCandidates -join '; ')"
    # Record an honest fail verdict so this run-id is not a dead-end: a retry
    # with the same id will replay this verdict (exit 1) instead of refusing.
    $abortChecks = @(
        @{ name = 'spike_binary_available'; expected = 'existing meeting_loopback_spike.exe'; actual = "not found; tried: $($exeCandidates -join '; ')"; pass = $false }
    )
    $verdict = [ordered]@{
        schema = 'smoke-suite-verdict/1'; mode = 'live'; status = 'fail'
        run_id = $RunId; error = 'spike binary not found; build it with: cargo build --release --bin meeting_loopback_spike'
        started_utc = $startedUtc.ToString('o'); finished_utc = (Get-Date).ToUniversalTime().ToString('o')
        checks = $abortChecks
    }
    [System.IO.File]::WriteAllText((Join-Path $runDir 'verdict.json'),
        ($verdict | ConvertTo-Json -Depth 8) + [Environment]::NewLine,
        (New-Object System.Text.UTF8Encoding($false)))
    Emit $verdict | Out-Host
    exit 1
}

Log-Event "RUN_LAUNCH exe=$exeFound duration_s=$DurationS"
$spikeArgs = @('--duration', "$DurationS", '--output-log', "`"$csv`"")
if ($OutputDevice) { $spikeArgs += @('--output-device', "`"$OutputDevice`"") }
if ($InputDevice)  { $spikeArgs += @('--input-device',  "`"$InputDevice`"") }

# Launch via raw ProcessStartInfo: on Windows PowerShell 5.1 this build,
# Start-Process -PassThru objects never report .ExitCode even with the handle
# cached, while [Diagnostics.Process]::Start does.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $exeFound
$psi.Arguments = ($spikeArgs -join ' ')
$psi.WorkingDirectory = $runDir
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$proc = [System.Diagnostics.Process]::Start($psi)
$stdoutTask = $proc.StandardOutput.ReadToEndAsync()
$stderrTask = $proc.StandardError.ReadToEndAsync()
Log-Event "RUN_STARTED pid=$($proc.Id)"

$toneProc = $null
try {
    # Short smoke schedule: silence lead-in, then continuous tone playback.
    Start-Sleep -Seconds $ToneStartDelayS
    Log-Event ("TONE_START t_rel_s={0}" -f [math]::Round(((Get-Date)-$startedUtc.ToLocalTime()).TotalSeconds,1))
    $toneArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$PlayTool`"",'-Seconds',"$ToneSeconds")
    $toneProc = Start-Process -FilePath $psHost -ArgumentList $toneArgs -PassThru -WindowStyle Hidden
    if (-not $toneProc.WaitForExit(($ToneSeconds + 30) * 1000)) {
        try { $toneProc.Kill() } catch {}
        Log-Event "TONE_KILLED_AFTER_TIMEOUT"
    } else {
        Log-Event ("TONE_END t_rel_s={0}" -f [math]::Round(((Get-Date)-$startedUtc.ToLocalTime()).TotalSeconds,1))
    }

    # Wait for the bounded capture to end on its own.
    $waitMs = ($DurationS + 180) * 1000
    if (-not $proc.WaitForExit($waitMs)) {
        try { $proc.Kill() } catch {}
        Log-Event "RUN_DID_NOT_EXIT_AFTER_TIMEOUT"
        $spikeExitCode = $null
    } else {
        $spikeExitCode = $proc.ExitCode
    }
    [System.IO.File]::WriteAllText((Join-Path $runDir 'spike-stdout.log'), $stdoutTask.Result)
    [System.IO.File]::WriteAllText((Join-Path $runDir 'spike-stderr.log'), $stderrTask.Result)
    Log-Event "RUN_FINISHED exitcode=$spikeExitCode"
} finally {
    if ($toneProc -and -not $toneProc.HasExited) { try { $toneProc.Kill() } catch {} }
}

# Analysis step (existing tool, verbatim).
Log-Event "ANALYZE_START"
& $psHost -NoProfile -ExecutionPolicy Bypass -File $AnalyzeTool `
    -CsvPath $csv -EventsPath $eventsPath -OutPath $summaryMd *>&1 |
    ForEach-Object { "$_" } | Set-Content -Path (Join-Path $runDir 'analysis-output.log') -Encoding UTF8
Log-Event "ANALYZE_DONE"

# --- verdict ------------------------------------------------------------------
$finishedUtc = (Get-Date).ToUniversalTime()
$stats = Get-CsvStats $csv
# Capture span = the LONGEST source span. WASAPI loopback delivers no packets
# during genuine silence, so the loopback stream's last callback sits at the
# end of the tone window; the microphone runs for the whole bounded capture.
$captureSpanS = [math]::Max($stats.micSpanMs, $stats.loopSpanMs) / 1000.0

$checks = @(
    @{ name = 'capture_span_at_least_60s'; expected = ">= $MinCaptureS s"; actual = "$([math]::Round($captureSpanS,1)) s"; pass = ($captureSpanS -ge $MinCaptureS) },
    @{ name = 'microphone_source_present'; expected = '> 0 non-error rows'; actual = "$($stats.micRows) rows"; pass = ($stats.micRows -gt 0) },
    @{ name = 'loopback_source_present';   expected = '> 0 non-error rows'; actual = "$($stats.loopRows) rows"; pass = ($stats.loopRows -gt 0) },
    @{ name = 'no_stream_errors';          expected = '0 error rows';      actual = "mic=$($stats.micErrors) loopback=$($stats.loopErrors)"; pass = (($stats.micErrors + $stats.loopErrors) -eq 0) },
    @{ name = 'loopback_heard_tone';       expected = '> 0 non-silent loopback callbacks'; actual = "$($stats.loopNonSilentRows) non-silent rows"; pass = ($stats.loopNonSilentRows -gt 0) },
    @{ name = 'analyzer_summary_written';  expected = 'SUMMARY.md exists'; actual = "$(Test-Path $summaryMd)"; pass = ((Test-Path $summaryMd) -and ((Get-Item $summaryMd).Length -gt 100)) }
)
$overallPass = (($checks | Where-Object { -not $_.pass }).Count -eq 0) -and ($spikeExitCode -eq 0)

$verdict = [ordered]@{
    schema        = 'smoke-suite-verdict/1'
    mode          = 'live'
    status        = $(if ($overallPass) { 'pass' } else { 'fail' })
    run_id        = $RunId
    started_utc   = $startedUtc.ToString('o')
    finished_utc  = $finishedUtc.ToString('o')
    duration_s    = $DurationS
    tone_seconds  = $ToneSeconds
    spike_exe     = $exeFound
    spike_exit_code = $spikeExitCode
    artifacts     = @{ csv = $csv; summary_md = $summaryMd; events_log = $eventsPath }
    checks        = $checks
}
$verdictJson = $verdict | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText((Join-Path $runDir 'verdict.json'), $verdictJson + [Environment]::NewLine,
    (New-Object System.Text.UTF8Encoding($false)))
Emit $verdict | Out-Host

if ($overallPass) { exit 0 } else { exit 1 }
