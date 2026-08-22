# ELEVATED. Performs a real S3 sleep / RTC-wake cycle during the spike run:
# enables all AC wake timers, registers a wake task N minutes ahead, logs the
# SLEEP_ENTER marker, suspends, and after resume logs RESUMED and restores the
# original wake-timer setting. The scheduled task re-invokes this script with
# -MarkWake right after resume to log the wake marker.
# Usage (from an elevated shell):
#   pwsh -File sleep-wake-test.ps1 -EventsPath <events.log> [-WakeAfterMin 4]
param(
    [Parameter(Mandatory=$true)][string]$EventsPath,
    [switch]$MarkWake,
    [int]$WakeAfterMin = 4
)

$ErrorActionPreference = 'Continue'
$taskName = 'SpeakSpikeSleepWakeTest'

function Log-Event([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $msg
    Add-Content -Path $EventsPath -Value $line
    Write-Output $line
}

if ($MarkWake) {
    Log-Event "RESUMED_FROM_SLEEP wake_task_fired=true"
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    exit 0
}

function Get-RtcWakeAc {
    $out = powercfg /q SCHEME_CURRENT SUB_SLEEP RTCWAKE 2>$null
    foreach ($line in $out) {
        if ($line -match 'Current AC Power Setting Index:\s*0x([0-9a-fA-F]+)') {
            return [int]$Matches[1]
        }
    }
    return $null
}

$prev = Get-RtcWakeAc
Log-Event "SLEEP_TEST_PREP prev_rtcwake_ac=$prev"

# 2 = enable all wake timers (previous value restored after resume).
powercfg /setacvalueindex SCHEME_CURRENT SUB_SLEEP RTCWAKE 2 | Out-Null
powercfg /setactive SCHEME_CURRENT | Out-Null

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File '$PSCommandPath' -MarkWake -EventsPath `"$EventsPath`""
$trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes($WakeAfterMin))
$settings = New-ScheduledTaskSettingsSet -WakeToRun -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
Log-Event ("SLEEP_ARMED wake_task={0} at={1:yyyy-MM-dd HH:mm:ss}" -f $taskName, (Get-Date).AddMinutes($WakeAfterMin))

Log-Event "SLEEP_ENTER"
rundll32.exe powrprof.dll,SetSuspendState 0,1,0
# Execution resumes here after wake.
Log-Event "SLEEP_SCRIPT_RESUMED"

# Restore the original wake-timer policy.
if ($null -ne $prev) {
    powercfg /setacvalueindex SCHEME_CURRENT SUB_SLEEP RTCWAKE $prev | Out-Null
    powercfg /setactive SCHEME_CURRENT | Out-Null
    Log-Event "SLEEP_TEST_DONE rtcwake_restored=$prev"
} else {
    Log-Event "SLEEP_TEST_DONE rtcwake_restore_skipped"
}

# Fallback cleanup if the wake task never fired (manual wake).
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
