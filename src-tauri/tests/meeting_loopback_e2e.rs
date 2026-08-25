//! Real-hardware loopback test (plan §7): opens the actual microphone and
//! the actual system-audio loopback, plays a spoken file through the default
//! output, and asserts the system track captured non-silent audio.
//!
//! This test touches real devices — run it on the target machine with
//! `cargo test --test meeting_loopback_e2e`.

use speak_app_lib::audio_toolkit::audio::meeting_recorder::{self as rec, MeetingRecorder};
use speak_app_lib::managers::meeting::SqliteChunkStateStore;
use speak_app_lib::managers::meeting_store::SqliteMeetingStore;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct EventLog {
    events: Mutex<Vec<String>>,
}

impl rec::RecorderEventSink for EventLog {
    fn on_event(&self, event: &rec::RecorderEvent) {
        let line = match event {
            rec::RecorderEvent::ChunkFinalized { info } => format!(
                "finalized seq={} source={:?} samples={}",
                info.sequence, info.source, info.sample_count
            ),
            other => format!("{other:?}"),
        };
        self.events.lock().unwrap().push(line);
    }
}

/// Root-mean-square of the PCM16 payload — near zero for silence.
fn rms_i16(bytes: &[u8]) -> f64 {
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}

fn ensure_speech_file() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("tts-german.wav");
    if path.exists() {
        return path;
    }
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000,[System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,[System.Speech.AudioFormat.AudioChannel]::Mono); \
         $s.SetOutputToWaveFile('{}', $f); \
         $s.Speak('Dies ist ein Loopback Test des Meeting Systems.'); $s.Dispose()",
        path.to_string_lossy().replace('\\', "\\\\")
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .expect("tts process");
    assert!(output.status.success(), "tts generation failed");
    path
}

fn play_to_default_output(path: &std::path::Path) {
    let script = format!(
        "Add-Type -AssemblyName presentationCore; \
         $p = New-Object System.Windows.Media.MediaPlayer; \
         $p.Open([Uri]::new('{}')); Start-Sleep -Milliseconds 400; $p.Play(); \
         Start-Sleep -Seconds 9",
        path.to_string_lossy().replace('\\', "/")
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .stdout(std::process::Stdio::null())
        .status();
}

#[test]
fn loopback_captures_what_the_default_output_plays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let meeting_id = uuid::Uuid::new_v4().to_string();
    let db_path = dir.path().join("meetings.db");
    let store = Arc::new(SqliteMeetingStore::open(db_path.clone()).expect("store"));

    // The chunk store enforces the sessions FK: the recorder writes chunks
    // for a session row that the manager normally creates before capture.
    {
        let conn = speak_app_lib::managers::meeting_store::open_meeting_database(&db_path)
            .expect("session db");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO meeting_sessions (id, title, state, started_at, created_at, updated_at)
             VALUES (?1, 'loopback-e2e', 'recording', ?2, ?2, ?2)",
            rusqlite::params![meeting_id, now],
        )
        .expect("session row");
    }

    let chunk_store =
        Arc::new(SqliteChunkStateStore::open(store.clone(), &db_path).expect("chunk store"));

    let log = Arc::new(EventLog::default());
    let mut recorder = rec::MeetingRecorderImpl::new();
    recorder.subscribe(log.clone());

    let layout = rec::SessionLayout {
        session_id: meeting_id.clone(),
        root: dir.path().join("meetings").join(&meeting_id),
    };
    recorder
        .start_session(layout, chunk_store)
        .expect("session");

    // Microphone: empty name substring-matches the first input device.
    recorder
        .open_source(rec::SourceKind::Microphone, rec::SourceOpenRequest::new(""))
        .expect("open microphone");
    // System: follow the OS default output — the behaviour under test.
    let mut system_request = rec::SourceOpenRequest::new("");
    system_request.follow_default_device = true;
    recorder
        .open_source(rec::SourceKind::System, system_request)
        .expect("open system loopback");

    // Play spoken audio through the default output while capturing.
    let wav = ensure_speech_file();
    let player = std::thread::spawn(move || play_to_default_output(&wav));

    // Capture while the file plays (9 s playback + margin).
    std::thread::sleep(std::time::Duration::from_secs(10));
    let _ = player.join();

    eprintln!("snapshot before close: {:?}", recorder.snapshot());
    for e in log.events.lock().unwrap().iter() {
        eprintln!("event: {e}");
    }

    recorder
        .close_source(rec::SourceKind::System, rec::StopReason::Requested)
        .expect("close system");
    recorder
        .close_source(rec::SourceKind::Microphone, rec::StopReason::Requested)
        .expect("close microphone");
    let summary = recorder.stop().expect("stop");

    // At least one finalized system chunk with real samples.
    let finalized_system: Vec<&rec::FinalizedChunkInfo> = summary
        .finalized_chunks
        .iter()
        .filter(|c| matches!(c.source, rec::SourceKind::System))
        .collect();
    assert!(
        !finalized_system.is_empty(),
        "no system chunk was finalized: {:?}",
        summary.finalized_chunks
    );
    assert!(
        finalized_system.iter().any(|c| c.sample_count > 16_000),
        "system chunks hold less than a second of audio: {finalized_system:?}"
    );

    // The captured audio must be non-silent: scan the system wavs for a
    // meaningful RMS (the played speech is loud; silence sits near zero).
    let audio_dir = dir.path().join("meetings").join(&meeting_id).join("audio");
    let mut system_rms_max = 0.0f64;
    let mut checked = 0;
    for entry in std::fs::read_dir(&audio_dir).expect("audio dir") {
        let path = entry.expect("entry").path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.contains("system") || !name.ends_with(".wav") {
            continue;
        }
        checked += 1;
        let mut file = std::fs::File::open(&path).expect("open wav");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes).expect("read wav");
        if bytes.len() > 44 {
            system_rms_max = system_rms_max.max(rms_i16(&bytes[44..]));
        }
    }
    assert!(checked > 0, "no system wav files were written");
    assert!(
        system_rms_max > 100.0,
        "system track is silent (max rms {system_rms_max:.1} over {checked} files) — loopback captured nothing audible"
    );
}
