//! Real Silero VAD inference (plan §8): feeds genuinely captured audio (the
//! loopback e2e's system track, or freshly generated TTS) and genuine silence
//! through the ONNX model and asserts speech is separated from noise.

use speak_app_lib::audio_toolkit::constants::WHISPER_SAMPLE_RATE;
use speak_app_lib::audio_toolkit::vad::{SileroVad, VadFrame, VoiceActivityDetector};

const FRAME: usize = 480; // 30 ms at 16 kHz

fn model_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models/silero_vad_v4.onnx")
}

/// Reads a wav as mono f32 at any rate/channel layout via the headless
/// loader (same normalization the CLI uses).
fn load_mono_16k(path: &std::path::Path) -> Vec<f32> {
    let reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .expect("float samples"),
        hound::SampleFormat::Int => reader
            .into_samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .expect("int samples"),
    };
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    if spec.sample_rate == WHISPER_SAMPLE_RATE {
        return mono;
    }
    let mut resampler = speak_app_lib::audio_toolkit::audio::FrameResampler::new(
        spec.sample_rate as usize,
        WHISPER_SAMPLE_RATE as usize,
        std::time::Duration::from_millis(30),
    );
    let mut out = Vec::new();
    let mut push = |frame: &[f32]| out.extend_from_slice(frame);
    resampler.push(&mono, &mut push);
    resampler.finish(&mut push);
    out
}

/// Fraction of 30 ms frames the model classifies as speech.
fn speech_ratio(samples: &[f32]) -> f64 {
    let mut vad = SileroVad::new(model_path(), 0.5).expect("silero loads");
    let mut speech = 0usize;
    let mut total = 0usize;
    for chunk in samples.chunks(FRAME) {
        if chunk.len() != FRAME {
            break;
        }
        total += 1;
        if matches!(
            vad.push_frame(chunk).expect("vad frame"),
            VadFrame::Speech(_)
        ) {
            speech += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        speech as f64 / total as f64
    }
}

fn ensure_speech_wav() -> std::path::PathBuf {
    // Prefer the genuinely captured loopback file from the e2e test.
    let keep = std::env::temp_dir().join("loopback-capture");
    if let Ok(entries) = std::fs::read_dir(&keep) {
        let mut best: Option<(std::path::PathBuf, u64)> = None;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("system") && name.ends_with(".wav") {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if best.as_ref().map(|(_, s)| size > *s).unwrap_or(true) {
                    best = Some((entry.path(), size));
                }
            }
        }
        if let Some((path, _)) = best {
            return path;
        }
    }
    // Fallback: generate TTS speech.
    let path = std::env::temp_dir().join("tts-german.wav");
    if !path.exists() {
        let script = format!(
            "Add-Type -AssemblyName System.Speech; \
             $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000,[System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,[System.Speech.AudioFormat.AudioChannel]::Mono); \
             $s.SetOutputToWaveFile('{}', $f); \
             $s.Speak('Dies ist ein Spracherkennungstest fuer das VAD.'); $s.Dispose()",
            path.to_string_lossy().replace('\\', "\\\\")
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .expect("tts");
        assert!(out.status.success());
    }
    path
}

#[test]
fn silero_separates_real_speech_from_real_silence() {
    let speech_path = ensure_speech_wav();
    let speech = load_mono_16k(&speech_path);
    assert!(
        speech.len() > WHISPER_SAMPLE_RATE as usize,
        "speech clip too short"
    );

    let spoken = speech_ratio(&speech);
    eprintln!("speech ratio on {}: {spoken:.2}", speech_path.display());
    assert!(
        spoken > 0.3,
        "Silero found almost no speech in a spoken clip (ratio {spoken:.2})"
    );

    // Genuine digital silence plus a tiny dither so the resampler/VAD see a
    // realistic "room dead" signal rather than exact zeros.
    let silence: Vec<f32> = (0..WHISPER_SAMPLE_RATE * 2)
        .map(|i| ((i as f32 * 0.01).sin()) * 1e-5)
        .collect();
    let quiet = speech_ratio(&silence);
    eprintln!("speech ratio on silence: {quiet:.2}");
    assert!(
        quiet < 0.05,
        "Silero hallucinated speech in silence (ratio {quiet:.2})"
    );
}
