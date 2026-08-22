# ELEVATED. Emulates physical unplug/reconnect of the meeting audio device by
# disabling and re-enabling its PnP device node(s) (MEDIA class). This kills the
# open WASAPI streams exactly like a cable pull, letting the spike log real
# stream-error rows. Markers are appended to the run's events.log.
# Usage (from an elevated shell):
#   pwsh -File device-loss-test.ps1 -EventsPath <events.log> [-OffSeconds 90] [-DevicePattern 'Focusrite']
param(
    [Parameter(Mandatory=$true)][string]$EventsPath,
    [int]$OffSeconds = 90,
    [string]$DevicePattern = 'Focusrite'
)

$ErrorActionPreference = 'Continue'

function Log-Event([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $msg
    Add-Content -Path $EventsPath -Value $line
    Write-Output $line
}

$targets = Get-PnpDevice -PresentOnly |
    Where-Object { $_.Class -eq 'MEDIA' -and $_.FriendlyName -match $DevicePattern }

if (-not $targets) { Log-Event "DEVICE_LOSS_ABORT no_media_device_matching=$DevicePattern"; exit 1 }
Log-Event ("DEVICE_LOSS_START targets={0} off_s={1}" -f (($targets | ForEach-Object InstanceId) -join '|'), $OffSeconds)

foreach ($t in $targets) {
    try { Disable-PnpDevice -InstanceId $t.InstanceId -Confirm:$false -ErrorAction Stop } catch { Log-Event "DEVICE_DISABLE_FAILED $($_.Exception.Message)" }
}
Log-Event "DEVICE_DISABLED"

Start-Sleep -Seconds $OffSeconds

foreach ($t in $targets) {
    try { Enable-PnpDevice -InstanceId $t.InstanceId -Confirm:$false -ErrorAction Stop } catch { Log-Event "DEVICE_ENABLE_FAILED $($_.Exception.Message)" }
}
Log-Event "DEVICE_REENABLED"

# Wait until present again so the reconnect is complete before continuing.
$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Seconds 3
    $back = Get-PnpDevice -PresentOnly |
        Where-Object { $_.Class -eq 'MEDIA' -and $_.FriendlyName -match $DevicePattern }
} while (-not $back -and (Get-Date) -lt $deadline)

if ($back) { Log-Event "DEVICE_RECONNECT_CONFIRMED" } else { Log-Event "DEVICE_RECONNECT_TIMEOUT" }
