# Plays a soft sine tone over the default output device for N seconds so the
# WASAPI loopback capture sees real (non-silent) audio.
# Usage: pwsh -File play-tone.ps1 -Seconds 30 [-Freq 440] [-Rate 44100]
param(
    [int]$Seconds = 30,
    [int]$Freq = 440,
    [int]$Rate = 44100,
    [double]$Amplitude = 0.15
)

$ErrorActionPreference = 'Stop'
$wavPath = Join-Path $PSScriptRoot 'tone.wav'

if (-not (Test-Path $wavPath)) {
    # 2 seconds of sine, looped by the player below.
    $durSec = 2
    $samples = $Rate * $durSec
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter($ms)
    $dataBytes = $samples * 2
    # RIFF header (PCM16 mono)
    $bw.Write([Text.Encoding]::ASCII.GetBytes('RIFF'))
    $bw.Write([UInt32](36 + $dataBytes))
    $bw.Write([Text.Encoding]::ASCII.GetBytes('WAVE'))
    $bw.Write([Text.Encoding]::ASCII.GetBytes('fmt '))
    $bw.Write([UInt32]16); $bw.Write([UInt16]1); $bw.Write([UInt16]1)
    $bw.Write([UInt32]$Rate); $bw.Write([UInt32]($Rate * 2))
    $bw.Write([UInt16]2); $bw.Write([UInt16]16)
    $bw.Write([Text.Encoding]::ASCII.GetBytes('data'))
    $bw.Write([UInt32]$dataBytes)
    for ($i = 0; $i -lt $samples; $i++) {
        $v = [Math]::Sin(2 * [Math]::PI * $Freq * $i / $Rate) * $Amplitude
        $bw.Write([Int16][Math]::Round($v * 32767))
    }
    $bw.Flush()
    [System.IO.File]::WriteAllBytes($wavPath, $ms.ToArray())
    $bw.Close(); $ms.Close()
}

$player = New-Object System.Media.SoundPlayer($wavPath)
$player.PlayLooping()
try {
    Start-Sleep -Seconds $Seconds
} finally {
    $player.Stop()
}
Write-Output "played ${Seconds}s of ${Freq}Hz tone"
