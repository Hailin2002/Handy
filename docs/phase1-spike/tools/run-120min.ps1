# Drives the full 120-minute meeting_loopback_spike run:
# - starts the spike binary (mic + loopback) with a fixed CSV path
# - alternates silence / tone-playback periods per the scenario schedule
# - appends timestamped events to events.log for later CSV correlation
# Usage: pwsh -File run-120min.ps1 -ExePath <spike.exe> -WorkDir <out dir> [-OutputDevice <name>]
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$WorkDir,
    [string]$OutputDevice,
    [int]$DurationS = 7200
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
$csv = Join-Path $WorkDir 'meeting_loopback_spike_120min.csv'
$events = Join-Path $WorkDir 'events.log'
Remove-Item $events -ErrorAction SilentlyContinue

function Log-Event([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $msg
    Add-Content -Path $events -Value $line
    Write-Output $line
}

$args_ = @('--duration', "$DurationS", '--output-log', "`"$csv`"")
if ($OutputDevice) { $args_ += @('--output-device', "`"$OutputDevice`"") }

Log-Event "RUN_LAUNCH exe=$ExePath args=$($args_ -join ' ')"
$proc = Start-Process -FilePath $ExePath -ArgumentList $args_ -WorkingDirectory $WorkDir `
        -RedirectStandardOutput (Join-Path $WorkDir 'spike-stdout.log') `
        -RedirectStandardError (Join-Path $WorkDir 'spike-stderr.log') `
        -PassThru -NoNewWindow
Log-Event "RUN_STARTED pid=$($proc.Id)"

# Scenario schedule (seconds from launch): alternating silence and playback.
# Long continuous silence windows exercise WASAPI loopback silence behavior.
$schedule = @(
    @{ start = 0;    tone = $false },
    @{ start = 300;  tone = $true  },   # tone periods last ~55 s each
    @{ start = 360;  tone = $false },   # long continuous silence (~19 min)
    @{ start = 1500; tone = $true  },
    @{ start = 1560; tone = $false },
    @{ start = 2700; tone = $true  },
    @{ start = 2760; tone = $false },
    @{ start = 3900; tone = $true  },
    @{ start = 3960; tone = $false },
    @{ start = 5100; tone = $true  },
    @{ start = 5160; tone = $false },
    @{ start = 6300; tone = $true  },
    @{ start = 6360; tone = $false }
)

$toneJob = $null
$currentIdx = -1
$t0 = Get-Date

try {
    while (-not $proc.HasExited) {
        $elapsed = ((Get-Date) - $t0).TotalSeconds
        if ($elapsed -ge ($DurationS - 2)) { break }

        # determine which schedule entry is active now
        $idx = 0
        for ($i = 0; $i -lt $schedule.Count; $i++) {
            if ($schedule[$i].start -le $elapsed) { $idx = $i }
        }

        if ($idx -ne $currentIdx) {
            $currentIdx = $idx
            if ($schedule[$idx].tone) {
                Log-Event ("TONE_START t_rel_s={0}" -f [math]::Round(((Get-Date)-$t0).TotalSeconds,1))
                $toneJob = Start-Process -FilePath 'powershell' `
                    -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'play-tone.ps1'),'-Seconds','55' `
                    -PassThru -WindowStyle Hidden
            } else {
                Log-Event ("SILENCE_START t_rel_s={0}" -f [math]::Round(((Get-Date)-$t0).TotalSeconds,1))
            }
        }

        if ($toneJob -and -not $toneJob.HasExited -and
            ((Get-Date) - $toneJob.StartTime).TotalSeconds -gt 60) {
            try { $toneJob.Kill() } catch {}
            Log-Event "TONE_KILLED_AFTER_TIMEOUT"
        }
        if ($toneJob -and $toneJob.HasExited -and -not $script:toneEndLogged) {
            Log-Event ("TONE_END t_rel_s={0}" -f [math]::Round(((Get-Date)-$t0).TotalSeconds,1))
            $script:toneEndLogged = $true
            $toneJob = $null
        }
        if (-not ($toneJob)) { $script:toneEndLogged = $false }

        Start-Sleep -Seconds 5
    }
} finally {
    if ($toneJob -and -not $toneJob.HasExited) { try { $toneJob.Kill() } catch {} }
}

Log-Event ("RUN_WAIT_EXIT elapsed_s={0}" -f [math]::Round(((Get-Date)-$t0).TotalSeconds,1))
if (-not $proc.WaitForExit(120000)) { Log-Event "RUN_DID_NOT_EXIT_AFTER_TIMEOUT" }
else { Log-Event "RUN_FINISHED exitcode=$($proc.ExitCode)" }
Write-Output "CSV: $csv"
