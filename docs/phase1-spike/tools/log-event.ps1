# Appends a timestamped scenario marker to a spike events.log so CSV rows can be
# correlated with manually injected scenarios (see analyze-spike-csv.ps1).
# Usage: pwsh -File log-event.ps1 -EventsPath <events.log> -Message "DEVICE_LOSS_START ..."
param(
    [Parameter(Mandatory=$true)][string]$EventsPath,
    [Parameter(Mandatory=$true)][string]$Message
)

$line = '{0:yyyy-MM-dd HH:mm:ss.fffzzz} {1}' -f (Get-Date), $Message
Add-Content -Path $EventsPath -Value $line
Write-Output $line
