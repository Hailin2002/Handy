<#
.SYNOPSIS
    Generates the spoken-audio fixtures the assistant live tests consume.

.DESCRIPTION
    Produces 16 kHz mono 16-bit WAVs with the Windows speech synthesiser:

      question_en.wav   a real question ("How many grams are in four ounces?")
      short_thanks.wav  "Thank you." - the most common recogniser-on-silence
                        hallucination, spoken for real so the blank guard is
                        exercised end to end
      silence.wav       2.5 s of nothing
      question_de.wav   the German question, only if a de-* voice is installed

    The files land in logs/tts-fixtures (git-ignored) and are picked up by

      cargo test --lib assistant::tests::live_ -- --ignored --nocapture

    which also needs the Azure Speech key in the keychain and OPENROUTER_API_KEY
    in the environment. Override the directory with SPEAK_TTS_FIXTURES.

    Windows only: System.Speech is not available elsewhere. The tests skip
    cleanly when the fixtures are absent.
#>
param(
    [string]$OutDir = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "logs\tts-fixtures")
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $OutDir | Out-Null

Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
    16000,
    [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
    [System.Speech.AudioFormat.AudioChannel]::Mono
)
$voices = $synth.GetInstalledVoices() | ForEach-Object { $_.VoiceInfo }
$english = $voices | Where-Object { $_.Culture.Name -like "en-*" } | Select-Object -First 1
$german = $voices | Where-Object { $_.Culture.Name -like "de-*" } | Select-Object -First 1
if (-not $english) { throw "No English voice installed; the fixtures need one." }

function Write-Spoken([string]$Name, [System.Speech.Synthesis.VoiceInfo]$Voice, [string]$Text) {
    $synth.SelectVoice($Voice.Name)
    $synth.SetOutputToWaveFile((Join-Path $OutDir $Name), $format)
    $synth.Speak($Text)
    $synth.SetOutputToNull()
    Write-Host "wrote $Name ($($Voice.Name))"
}

Write-Spoken "question_en.wav" $english "How many grams are in four ounces?"
Write-Spoken "short_thanks.wav" $english "Thank you."
if ($german) { Write-Spoken "question_de.wav" $german "Wie viele Gramm sind vier Unzen?" }
else { Write-Host "no German voice installed; skipping question_de.wav" }

# Silence via SSML so it comes out in exactly the same format as the speech.
$synth.SelectVoice($english.Name)
$synth.SetOutputToWaveFile((Join-Path $OutDir "silence.wav"), $format)
$synth.SpeakSsml('<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US"><break time="2500ms"/></speak>')
$synth.SetOutputToNull()
Write-Host "wrote silence.wav"

$synth.Dispose()
Get-ChildItem $OutDir -Filter *.wav | ForEach-Object { "{0,-18} {1,8} bytes" -f $_.Name, $_.Length }
