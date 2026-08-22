# Analyzes a meeting_loopback_spike CSV log and writes a SUMMARY analysis
# artifact (markdown) with measured max gap, drift, gap-class histogram,
# silence statistics and stream-error/event correlation.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File analyze-spike-csv.ps1 -CsvPath <log.csv> [-EventsPath <events.log>] -OutPath <SUMMARY.md>
param(
    [Parameter(Mandatory=$true)][string]$CsvPath,
    [string]$EventsPath,
    [Parameter(Mandatory=$true)][string]$OutPath
)

$ErrorActionPreference = 'Stop'

$meta = [ordered]@{}
$rows = New-Object System.Collections.Generic.List[object]

function ToD([string]$v) { $d = 0.0; if ([double]::TryParse($v, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$d)) { $d } else { 0.0 } }
function ToI([string]$v) { $i = [int64]0; if ([int64]::TryParse($v, [Globalization.NumberStyles]::Integer, [Globalization.CultureInfo]::InvariantCulture, [ref]$i)) { $i } else { [int64]0 } }

foreach ($line in [System.IO.File]::ReadLines((Resolve-Path $CsvPath))) {
    if ($line.StartsWith('# ')) {
        $body = $line.Substring(2)
        if ($body -match '^([A-Za-z_]+)[ =](.*)$') { $meta[$Matches[1]] = $Matches[2] }
        elseif ($body -match '^([A-Za-z_]+)$') { $meta[$Matches[1]] = '' }
        continue
    }
    if ($line -match '^\s*$') { continue }
    # CSV parse honoring quotes
    $fields = New-Object System.Collections.Generic.List[string]
    $cur = New-Object System.Text.StringBuilder
    $inQ = $false
    for ($i = 0; $i -lt $line.Length; $i++) {
        $c = $line[$i]
        if ($inQ) {
            if ($c -eq '"') {
                if ($i + 1 -lt $line.Length -and $line[$i+1] -eq '"') { [void]$cur.Append('"'); $i++ }
                else { $inQ = $false }
            } else { [void]$cur.Append($c) }
        } else {
            if ($c -eq '"') { $inQ = $true }
            elseif ($c -eq ',') { $fields.Add($cur.ToString()); [void]$cur.Clear() }
            else { [void]$cur.Append($c) }
        }
    }
    $fields.Add($cur.ToString())
    if ($fields[0] -eq 'source') { continue } # header
    $rows.Add([pscustomobject]@{
        source        = $fields[0]
        cbIndex       = ToI $fields[1]
        tRelMs        = ToD $fields[2]
        gapMs         = ToD $fields[3]
        frames        = ToI $fields[4]
        totalFrames   = ToI $fields[5]
        sampleRate    = ToI $fields[6]
        channels      = ToI $fields[7]
        sampleFormat  = $fields[8]
        expected      = ToD $fields[9]
        drift         = ToD $fields[10]
        gapClass      = $fields[11]
        silent        = $fields[12]
        peak          = ToD $fields[13]
        rms           = ToD $fields[14]
        error         = $fields[15]
    }) | Out-Null
}

$md = New-Object System.Text.StringBuilder
function Emit([string]$s) { [void]$md.AppendLine($s) }

Emit "# Meeting Loopback Spike - CSV Analysis"
Emit ""
Emit "Log: $CsvPath"
Emit ""
Emit "## Run metadata"
Emit ""
foreach ($k in $meta.Keys) { Emit "- ${k}: $($meta[$k])" }
Emit ""

foreach ($src in @('microphone', 'loopback')) {
    $r = @($rows | Where-Object { $_.source -eq $src -and $_.error -eq '' })
    $errs = @($rows | Where-Object { $_.source -eq $src -and $_.error -ne '' })
    if ($r.Count -eq 0 -and $errs.Count -eq 0) { continue }
    $rate = 0; $ch = 0; $fmt = '?'
    if ($r.Count) { $rate = $r[0].sampleRate; $ch = $r[0].channels; $fmt = $r[0].sampleFormat }
    $spanMs = 0; if ($r.Count) { $spanMs = $r[$r.Count-1].tRelMs }
    $maxGap = 0; if ($r.Count) { $maxGap = ($r | Measure-Object gapMs -Maximum).Maximum }
    $driftEnd = 0; if ($r.Count) { $driftEnd = $r[$r.Count-1].drift }
    $driftMin = 0; $driftMax = 0
    if ($r.Count) {
        $driftMin = ($r | Measure-Object drift -Minimum).Minimum
        $driftMax = ($r | Measure-Object drift -Maximum).Maximum
    }
    # drift trend: stable window after the first 10 s
    $stable = @($r | Where-Object { $_.tRelMs -gt 10000 })
    $driftFirstStable = 0; if ($stable.Count) { $driftFirstStable = $stable[0].drift }
    $driftRatePerHour = 0.0
    if ($stable.Count -gt 1) {
        $dtH = ($stable[$stable.Count-1].tRelMs - $stable[0].tRelMs) / 3600000.0
        if ($dtH -gt 0) { $driftRatePerHour = ($stable[$stable.Count-1].drift - $stable[0].drift) / $dtH }
    }
    $normal = @($r | Where-Object { $_.gapClass -eq 'normal' }).Count
    $sparse = @($r | Where-Object { $_.gapClass -eq 'sparse' }).Count
    $long   = @($r | Where-Object { $_.gapClass -eq 'gap' }).Count
    $silentN = @($r | Where-Object { $_.silent -eq 'true' }).Count
    $silentFrac = 0; if ($r.Count) { $silentFrac = $silentN / $r.Count }
    $gapTimeSparseS = 0.0
    $s1 = ($r | Where-Object { $_.gapClass -eq 'sparse' } | Measure-Object gapMs -Sum).Sum
    if ($s1) { $gapTimeSparseS = $s1 / 1000.0 }
    $gapTimeLongS = 0.0
    $s2 = ($r | Where-Object { $_.gapClass -eq 'gap' } | Measure-Object gapMs -Sum).Sum
    if ($s2) { $gapTimeLongS = $s2 / 1000.0 }
    $topGaps = @($r | Sort-Object gapMs -Descending | Select-Object -First 10)
    $totalFrames = 0; if ($r.Count) { $totalFrames = $r[$r.Count-1].totalFrames }
    $driftEndMs = 0.0; if ($rate) { $driftEndMs = $driftEnd / $rate * 1000.0 }

    Emit "## Source: $src"
    Emit ""
    Emit "| Metric | Value |"
    Emit "| --- | --- |"
    Emit "| Callbacks | $($r.Count) |"
    Emit "| Stream errors | $($errs.Count) |"
    Emit "| Config | $rate Hz, $ch ch, $fmt |"
    Emit "| Measurement span (last callback) | $([math]::Round($spanMs/1000.0,1)) s |"
    Emit "| Total frames | $totalFrames |"
    Emit "| Max gap | $([math]::Round($maxGap,3)) ms |"
    Emit "| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | $normal / $sparse / $long |"
    Emit "| Time in sparse gaps | $([math]::Round($gapTimeSparseS,3)) s |"
    Emit "| Time in long gaps (>1 s) | $([math]::Round($gapTimeLongS,3)) s |"
    Emit "| Drift at end | $([math]::Round($driftEnd,1)) frames ($([math]::Round($driftEndMs,3)) ms) |"
    Emit "| Drift range | $([math]::Round($driftMin,1)) .. $([math]::Round($driftMax,1)) frames |"
    Emit "| Drift trend (stable window >10 s) | $([math]::Round($driftFirstStable,1)) to $([math]::Round($driftEnd,1)) frames; rate $([math]::Round($driftRatePerHour,1)) frames/h |"
    Emit "| Silent fraction (callbacks) | $([math]::Round($silentFrac*100.0,2)) % ($silentN/$($r.Count)) |"
    Emit ""
    if ($errs.Count -gt 0) {
        Emit "### Stream errors"
        Emit ""
        Emit "| t_rel_ms | Error |"
        Emit "| --- | --- |"
        foreach ($e in $errs) { Emit "| $($e.tRelMs) | $($e.error) |" }
        Emit ""
    }
    if ($long -gt 0 -or ($topGaps.Count -gt 0 -and $topGaps[0].gapMs -gt 250)) {
        Emit "### Top 10 largest gaps"
        Emit ""
        Emit "| t_rel_ms | gap_ms | class | silent |"
        Emit "| --- | --- | --- | --- |"
        foreach ($g in $topGaps) { Emit "| $([math]::Round($g.tRelMs,1)) | $([math]::Round($g.gapMs,3)) | $($g.gapClass) | $($g.silent) |" }
        Emit ""
    }
}

# Cross-source comparison
$mic  = @($rows | Where-Object { $_.source -eq 'microphone' -and $_.error -eq '' })
$loop = @($rows | Where-Object { $_.source -eq 'loopback' -and $_.error -eq '' })
if ($mic.Count -and $loop.Count) {
    Emit "## Cross-source comparison"
    Emit ""
    $driftDiffEnd = $loop[$loop.Count-1].drift - $mic[$mic.Count-1].drift
    $spanS = [math]::Min($mic[$mic.Count-1].tRelMs, $loop[$loop.Count-1].tRelMs) / 1000.0
    $driftDiffMs = 0.0; if ($loop[0].sampleRate) { $driftDiffMs = $driftDiffEnd / $loop[0].sampleRate * 1000.0 }
    Emit "- Relative drift loopback vs microphone at end: $([math]::Round($driftDiffEnd,1)) frames ($([math]::Round($driftDiffMs,3)) ms) after $([math]::Round($spanS,1)) s"
    if ($spanS -gt 0) {
        Emit "- Relative drift rate: $([math]::Round($driftDiffEnd / ($spanS/3600.0),1)) frames/h"
    }
    $micMax  = ($mic  | Measure-Object gapMs -Maximum).Maximum
    $loopMax = ($loop | Measure-Object gapMs -Maximum).Maximum
    Emit "- Max gap microphone: $([math]::Round($micMax,3)) ms; loopback: $([math]::Round($loopMax,3)) ms"
    Emit ""
}

# Events correlation
if ($EventsPath -and (Test-Path $EventsPath)) {
    Emit '## Event log (scenario markers)'
    Emit ''
    Emit '```text'
    [void]$md.Append([System.IO.File]::ReadAllText((Resolve-Path $EventsPath)))
    Emit '```'
    Emit ""
}

$outFull = if ([System.IO.Path]::IsPathRooted($OutPath)) { $OutPath } else { Join-Path (Get-Location).Path $OutPath }
[System.IO.File]::WriteAllText($outFull, $md.ToString(), (New-Object System.Text.UTF8Encoding($true)))
Write-Output "wrote $outFull"
