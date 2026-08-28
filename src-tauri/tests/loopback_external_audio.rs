//! Measures the system-audio loopback against whatever is playing right now.
//!
//! `meeting_loopback_e2e` proves loopback works for a WAV played through
//! MediaPlayer. That is a cooperative, local, uncompressed source. Streaming
//! audio from a browser goes through a different path — a separate process,
//! its own audio session, a decoder, possibly a different sample rate and
//! channel layout — and "works with MediaPlayer" does not settle whether it
//! works with that.
//!
//! This test captures the default output as loopback and reports the level per
//! second, so an external source (a browser tab, a media player, anything) can
//! be verified by ear and by number at the same time.
//!
//! `#[ignore]` on purpose: it asserts nothing about silence, because whether
//! anything is playing is not something the test controls. Run it deliberately:
//!
//!     cargo test --test loopback_external_audio -- --ignored --nocapture
//!
//! Set `SPEAK_LOOPBACK_SECONDS` to change the 12-second default.

use speak_app_lib::audio_toolkit::audio::meeting_recorder::{self as rec, MeetingRecorder};
use speak_app_lib::managers::meeting::SqliteChunkStateStore;
use speak_app_lib::managers::meeting_store::SqliteMeetingStore;
use std::sync::Arc;

/// Root-mean-square of a PCM16 payload. Silence sits near zero; normal
/// programme material is in the hundreds to low thousands.
fn rms_i16(bytes: &[u8]) -> f64 {
    if bytes.len() < 2 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut n = 0u64;
    for pair in bytes.chunks_exact(2) {
        let s = i16::from_le_bytes([pair[0], pair[1]]) as f64;
        sum += s * s;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64).sqrt()
    }
}

#[test]
#[ignore = "measures whatever is currently playing; run deliberately"]
fn report_system_loopback_level() {
    let seconds: u64 = std::env::var("SPEAK_LOOPBACK_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);

    let dir = tempfile::tempdir().expect("tempdir");
    let meeting_id = uuid::Uuid::new_v4().to_string();
    let db_path = dir.path().join("meetings.db");
    let store = Arc::new(SqliteMeetingStore::open(db_path.clone()).expect("store"));

    {
        let conn = speak_app_lib::managers::meeting_store::open_meeting_database(&db_path)
            .expect("session db");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO meeting_sessions (id, title, state, started_at, created_at, updated_at)
             VALUES (?1, 'loopback-external', 'recording', ?2, ?2, ?2)",
            rusqlite::params![meeting_id, now],
        )
        .expect("session row");
    }

    let chunk_store =
        Arc::new(SqliteChunkStateStore::open(store.clone(), &db_path).expect("chunk store"));
    let mut recorder = rec::MeetingRecorderImpl::new();

    let layout = rec::SessionLayout {
        session_id: meeting_id.clone(),
        root: dir.path().join("meetings").join(&meeting_id),
    };
    recorder
        .start_session(layout, chunk_store)
        .expect("session");

    // System audio only — the microphone would pick the same sound up through
    // the speakers and make the result meaningless.
    let mut request = rec::SourceOpenRequest::new("");
    request.follow_default_device = true;
    request.rollover_target = std::time::Duration::from_secs(1);
    recorder
        .open_source(rec::SourceKind::System, request)
        .expect("open system loopback");

    eprintln!("capturing system loopback for {seconds}s — start playback now");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let summary = recorder.stop().expect("stop");
    eprintln!("stopped: {summary:?}");

    // Walk the finalised chunks in order and report each one's level.
    let root = dir.path().join("meetings").join(&meeting_id);
    let mut files: Vec<_> = walk_wavs(&root);
    files.sort();

    let mut peak_rms = 0.0f64;
    let mut audible = 0usize;
    for (i, path) in files.iter().enumerate() {
        let bytes = std::fs::read(path).unwrap_or_default();
        if bytes.len() <= 44 {
            continue;
        }
        let rms = rms_i16(&bytes[44..]);
        peak_rms = peak_rms.max(rms);
        if rms > 100.0 {
            audible += 1;
        }
        eprintln!(
            "  chunk {i:2}  {:>8} bytes  rms {rms:8.1}  {}",
            bytes.len() - 44,
            if rms > 100.0 { "AUDIBLE" } else { "silence" }
        );
    }

    // Optionally keep the captured audio so it can be fed to the transcription
    // pipeline — the point being that loopback output is real programme audio,
    // not merely non-silent bytes.
    if let Ok(dest) = std::env::var("SPEAK_LOOPBACK_KEEP") {
        let dest = std::path::PathBuf::from(dest);
        let _ = std::fs::create_dir_all(&dest);
        for (i, path) in files.iter().enumerate() {
            let target = dest.join(format!("chunk_{i:02}.wav"));
            if let Err(e) = std::fs::copy(path, &target) {
                eprintln!("  could not keep {}: {e}", target.display());
            }
        }
        eprintln!("kept {} chunk(s) in {}", files.len(), dest.display());
    }
    eprintln!(
        "\nRESULT files={} audible={} peak_rms={peak_rms:.1}",
        files.len(),
        audible
    );
    assert!(!files.is_empty(), "loopback produced no chunks at all");
}

fn walk_wavs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_wavs(&path));
        } else if path.extension().is_some_and(|e| e == "wav") {
            out.push(path);
        }
    }
    out
}

/// Transcribes chunks kept by `report_system_loopback_level` through the same
/// transcribe-cpp engine the app uses.
///
/// Levels alone do not settle the question. A capture can be loud and still be
/// garbage — wrong sample rate, interleaved channels read as mono, a bit depth
/// mismatch — and every one of those produces confident-looking non-zero rms.
/// Feeding the bytes to the recogniser and reading words back out is what
/// distinguishes "captured audio" from "captured *this* audio".
///
///     SPEAK_LOOPBACK_KEEP=<dir> SPEAK_LOOPBACK_MODEL=<path to .gguf> \
///       cargo test --test loopback_external_audio -- --ignored --nocapture
#[test]
#[ignore = "needs kept chunks and a local model; run deliberately"]
fn transcribe_kept_loopback_chunks() {
    let Ok(dir) = std::env::var("SPEAK_LOOPBACK_KEEP") else {
        eprintln!("SPEAK_LOOPBACK_KEEP unset — nothing to transcribe");
        return;
    };
    let Ok(model_path) = std::env::var("SPEAK_LOOPBACK_MODEL") else {
        eprintln!("SPEAK_LOOPBACK_MODEL unset — no model to transcribe with");
        return;
    };

    // Only recorder output — a stray source WAV in the same directory would
    // make this test transcribe the thing it is supposed to be verifying.
    let mut files: Vec<_> = walk_wavs(std::path::Path::new(&dir))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chunk_"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no kept chunks in {dir}");

    // Chunks are 16 kHz mono PCM16 with a 44-byte header; concatenating the
    // payloads rebuilds the timeline the recorder split up.
    let mut samples: Vec<f32> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_default();
        if bytes.len() <= 44 {
            continue;
        }
        for pair in bytes[44..].chunks_exact(2) {
            samples.push(i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0);
        }
    }
    eprintln!(
        "joined {} chunk(s) into {:.1}s of audio",
        files.len(),
        samples.len() as f64 / 16_000.0
    );

    // Dynamic-backend builds need their modules registered before any load.
    speak_app_lib::managers::transcription::init_transcribe_backend();
    let model = transcribe_cpp::Model::load_with(
        std::path::Path::new(&model_path),
        &transcribe_cpp::ModelOptions::default(),
    )
    .expect("load model");
    let mut session = model.session().expect("session");
    let text = session
        .run(
            &samples,
            &transcribe_cpp::RunOptions {
                task: transcribe_cpp::Task::Transcribe,
                language: Some("en".to_string()),
                ..Default::default()
            },
        )
        .expect("transcribe")
        .text;

    eprintln!("\nTRANSCRIPT: {}\n", text.trim());
    assert!(
        !text.trim().is_empty(),
        "the loopback capture transcribed to nothing"
    );
}
