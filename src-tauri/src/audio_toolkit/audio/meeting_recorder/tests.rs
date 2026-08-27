//! Tests for [`super`] — dual-source capture, timeline math and chunk I/O.
//!
//! Extracted so the implementation file stays implementation. Same
//! arrangement the project already uses for settings/tests.rs,
//! model/download/tests.rs and meeting_store/.

use super::*;
use std::sync::atomic::AtomicI64;

// -- test doubles -------------------------------------------------------

#[derive(Default)]
struct InMemoryStore {
    next_id: AtomicI64,
    rows: Mutex<Vec<ChunkDescriptor>>,
    fail_transitions: AtomicBool,
}

impl InMemoryStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicI64::new(1),
            ..Default::default()
        })
    }

    fn row(&self, chunk_id: ChunkId) -> Option<ChunkDescriptor> {
        lock_recover(&self.rows)
            .iter()
            .find(|r| r.chunk_id == chunk_id)
            .cloned()
    }

    fn all(&self) -> Vec<ChunkDescriptor> {
        lock_recover(&self.rows).clone()
    }

    fn seed_writing(&self, descriptor: ChunkDescriptor) -> ChunkId {
        let mut rows = lock_recover(&self.rows);
        let id = descriptor.chunk_id;
        rows.push(descriptor);
        id
    }
}

impl ChunkStateStore for InMemoryStore {
    fn open_chunks(&self, _session_id: &str) -> Result<Vec<ChunkDescriptor>, String> {
        Ok(lock_recover(&self.rows)
            .iter()
            .filter(|r| r.state == ChunkState::Writing)
            .cloned()
            .collect())
    }

    fn transition(
        &self,
        chunk_id: ChunkId,
        from: ChunkState,
        to: ChunkState,
        patch: &ChunkPatch,
    ) -> Result<(), String> {
        if self.fail_transitions.load(Ordering::Acquire) {
            return Err("injected store failure".to_string());
        }
        let mut rows = lock_recover(&self.rows);
        let row = rows
            .iter_mut()
            .find(|r| r.chunk_id == chunk_id)
            .ok_or_else(|| format!("no such chunk {chunk_id}"))?;
        if row.state != from {
            return Err(format!("expected {:?}, found {:?}", from, row.state));
        }
        row.state = to;
        if let Some(v) = patch.storage_key.clone() {
            row.storage_key = v;
        }
        if let Some(v) = patch.range {
            row.range = v;
        }
        if let Some(v) = patch.sample_count {
            row.sample_count = v;
        }
        if let Some(v) = patch.checksum_sha256.clone() {
            row.checksum_sha256 = Some(v);
        }
        if let Some(v) = patch.recovered {
            row.recovered = v;
        }
        Ok(())
    }

    fn insert_writing_chunk(
        &self,
        _session_id: &str,
        descriptor: &ChunkDescriptor,
    ) -> Result<ChunkId, String> {
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        let mut row = descriptor.clone();
        row.chunk_id = id;
        lock_recover(&self.rows).push(row);
        Ok(id)
    }
}

#[derive(Default)]
struct CollectingSink {
    events: Mutex<Vec<RecorderEvent>>,
    /// Delete each finalized chunk immediately — used by the bounded-memory
    /// test so a two-hour run does not need 230 MB of free disk.
    delete_finalized: Mutex<Option<PathBuf>>,
}

impl RecorderEventSink for CollectingSink {
    fn on_event(&self, event: &RecorderEvent) {
        if let RecorderEvent::ChunkFinalized { info } = event {
            if let Some(dir) = lock_recover(&self.delete_finalized).as_ref() {
                let name = format!("{:06}-{}.wav", info.sequence, info.source.file_tag());
                let _ = fs::remove_file(dir.join(name));
            }
        }
        lock_recover(&self.events).push(event.clone());
    }
}

impl CollectingSink {
    fn snapshot(&self) -> Vec<RecorderEvent> {
        lock_recover(&self.events).clone()
    }

    fn count_gaps(&self) -> usize {
        self.snapshot()
            .iter()
            .filter(|e| matches!(e, RecorderEvent::TimelineGap { .. }))
            .count()
    }
}

struct FakeCtl {
    on_block: Box<dyn FnMut(CaptureBlock) + Send>,
    on_error: Box<dyn Fn(String) + Send + Sync>,
}

#[derive(Default)]
struct FakeBackend {
    rate: u32,
    ctls: Mutex<HashMap<SourceKind, FakeCtl>>,
    fail_open: Mutex<HashMap<SourceKind, RecorderError>>,
}

impl FakeBackend {
    fn new(rate: u32) -> Arc<Self> {
        Arc::new(Self {
            rate,
            ..Default::default()
        })
    }

    /// Deliver one capture block on the shared timeline.
    fn feed(&self, source: SourceKind, timeline_ms: f64, samples: &[f32]) {
        let mut ctls = lock_recover(&self.ctls);
        if let Some(ctl) = ctls.get_mut(&source) {
            (ctl.on_block)(CaptureBlock {
                timeline_ms,
                mono: samples.to_vec(),
                native_rate: self.rate,
            });
        }
    }

    /// Feed `total_ms` of constant-amplitude audio in `block_ms` slices.
    fn feed_span(
        &self,
        source: SourceKind,
        from_ms: f64,
        total_ms: f64,
        block_ms: f64,
        amplitude: f32,
    ) -> f64 {
        let per_block = (self.rate as f64 * block_ms / 1000.0).round() as usize;
        let buf = vec![amplitude; per_block];
        let mut t = from_ms;
        let end = from_ms + total_ms;
        while t + block_ms <= end + 1e-9 {
            t += block_ms;
            self.feed(source, t, &buf);
        }
        t
    }

    fn fail(&self, source: SourceKind, detail: &str) {
        let ctls = lock_recover(&self.ctls);
        if let Some(ctl) = ctls.get(&source) {
            (ctl.on_error)(detail.to_string());
        }
    }
}

impl CaptureBackend for FakeBackend {
    fn preflight(&self, source: SourceKind, request: &SourceOpenRequest) -> PreflightCheck {
        PreflightCheck {
            source,
            device_name: request.device_name.clone(),
            outcome: PreflightOutcome::Available,
            native_sample_rate_hint: Some(self.rate),
            detail: String::new(),
        }
    }

    fn open(
        &self,
        source: SourceKind,
        _request: &SourceOpenRequest,
        on_block: Box<dyn FnMut(CaptureBlock) + Send>,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn CaptureStream>, RecorderError> {
        if let Some(err) = lock_recover(&self.fail_open).get(&source) {
            return Err(err.clone());
        }
        lock_recover(&self.ctls).insert(source, FakeCtl { on_block, on_error });
        Ok(Box::new(FakeStream { rate: self.rate }))
    }
}

struct FakeStream {
    rate: u32,
}

impl CaptureStream for FakeStream {
    fn native_sample_rate(&self) -> u32 {
        self.rate
    }
    fn stop(&mut self) {}
}

/// Filesystem that can fail a chosen step of the commit sequence.
struct FaultyFs {
    inner: RealChunkFs,
    fail_rename: AtomicBool,
}

impl FaultyFs {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RealChunkFs,
            fail_rename: AtomicBool::new(false),
        })
    }
}

impl ChunkFs for FaultyFs {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        self.inner.create_new(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.fail_rename.load(Ordering::Acquire) {
            return Err(io::Error::other("injected rename failure"));
        }
        self.inner.rename(from, to)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.inner.sync_dir(dir)
    }
}

// -- helpers ------------------------------------------------------------

fn layout_in(dir: &Path) -> SessionLayout {
    SessionLayout {
        session_id: "session-under-test".to_string(),
        root: dir.to_path_buf(),
    }
}

fn request(name: &str, rollover_ms: u64) -> SourceOpenRequest {
    SourceOpenRequest {
        device_name: name.to_string(),
        follow_default_device: false,
        rollover_target: Duration::from_millis(rollover_ms),
    }
}

/// Read a finalized chunk and assert its header is internally consistent.
/// Returns the sample count.
fn assert_valid_wav(path: &Path) -> u64 {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        bytes.len() >= WAV_HEADER_LEN,
        "{} shorter than a WAV header",
        path.display()
    );
    let declared = parse_wav_header(&bytes)
        .unwrap_or_else(|| panic!("{} is not a valid RIFF/WAVE file", path.display()));
    assert_eq!(
        declared as usize,
        bytes.len() - WAV_HEADER_LEN,
        "{}: declared data size does not match file length",
        path.display()
    );
    let riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(
        riff as usize,
        bytes.len() - 8,
        "{}: bad RIFF size",
        path.display()
    );
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        TIMELINE_SAMPLE_RATE_HZ,
        "{}: chunks must be 16 kHz",
        path.display()
    );
    assert_eq!(
        u16::from_le_bytes([bytes[22], bytes[23]]),
        TIMELINE_CHANNELS,
        "{}: chunks must be mono",
        path.display()
    );
    assert_eq!(
        u16::from_le_bytes([bytes[34], bytes[35]]),
        16,
        "{}: chunks must be PCM16",
        path.display()
    );
    u64::from(declared) / BYTES_PER_SAMPLE
}

fn wav_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("wav"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn part_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("part"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn writer_for(
    dir: &Path,
    source: SourceKind,
    store: Arc<dyn ChunkStateStore>,
    fs_impl: Arc<dyn ChunkFs>,
    sink: Arc<dyn RecorderEventSink>,
    rollover_ms: u64,
    native_rate: u32,
) -> SourceWriter {
    let audio_dir = dir.join("audio");
    fs::create_dir_all(&audio_dir).expect("create audio dir");
    SourceWriter::new(
        SourceWriterConfig {
            source,
            session_id: "session-under-test".to_string(),
            audio_dir,
            segment_id: 1,
            native_rate,
            rollover_samples: ms_to_samples(rollover_ms as f64),
            initial_written: 0,
            initial_sequence: 1,
        },
        store,
        fs_impl,
        sink,
        Arc::new(Mutex::new(SharedState::default())),
        // Tests drive `push_block` with explicit timestamps; the idle clock
        // is only consulted by `fill_idle_silence`, which the idle tests
        // exercise through their own writer.
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    )
}

/// Builds a writer whose session clock already started `age_ms` ago, so
/// `fill_idle_silence` has a definite span to close without any waiting.
fn idle_writer_for(
    dir: &Path,
    source: SourceKind,
    store: Arc<dyn ChunkStateStore>,
    sink: Arc<dyn RecorderEventSink>,
    rollover_ms: u64,
    age_ms: u64,
) -> SourceWriter {
    let audio_dir = dir.join("audio");
    fs::create_dir_all(&audio_dir).expect("create audio dir");
    SourceWriter::new(
        SourceWriterConfig {
            source,
            session_id: "session-under-test".to_string(),
            audio_dir,
            segment_id: 1,
            native_rate: 16_000,
            rollover_samples: ms_to_samples(rollover_ms as f64),
            initial_written: 0,
            initial_sequence: 1,
        },
        store,
        Arc::new(RealChunkFs),
        sink,
        Arc::new(Mutex::new(SharedState::default())),
        Instant::now() - Duration::from_millis(age_ms),
        Arc::new(Mutex::new(0.0)),
    )
}

/// Regression: WASAPI loopback delivers no callbacks at all while nothing
/// plays, so a meeting where no system audio was played used to produce no
/// system chunk whatsoever — the track was missing rather than silent.
/// Every field recording in the wild showed zero `*-system.wav` files.
#[test]
fn a_silent_source_still_records_a_track() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    // 5 s of session, of which the last second is held back by the lag.
    let mut writer = idle_writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        sink.clone(),
        10_000,
        5_000,
    );

    writer.fill_idle_silence().expect("idle fill");
    writer.finish().expect("finish");

    let files = wav_files(&dir.path().join("audio"));
    assert_eq!(
        files.len(),
        1,
        "a silent source owns exactly one chunk here"
    );
    // 5 000 ms − 1 000 ms lag = 4 s at 16 kHz. The clock advances slightly
    // between construction and the fill, so allow a small upward slack.
    let samples = assert_valid_wav(&files[0]);
    assert!(
        (64_000..64_800).contains(&samples),
        "expected ~4 s of silence, got {samples} samples"
    );
}

/// The fill trails `now`, so a block that is merely late still lands at its
/// own timestamp instead of behind an already-advanced write head.
#[test]
fn idle_silence_stops_short_of_now() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = idle_writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        sink.clone(),
        60_000,
        3_000,
    );

    writer.fill_idle_silence().expect("idle fill");
    let written_ms = samples_to_ms(writer.written);
    assert!(
        written_ms <= 3_000.0 - tolerances::IDLE_SILENCE_LAG_MS + 50.0,
        "idle fill must trail now by the lag, wrote {written_ms} ms"
    );
}

/// A session younger than the lag has nothing to fill yet, so the first
/// ticks must not open a chunk at all.
#[test]
fn idle_silence_does_nothing_before_the_lag_elapsed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = idle_writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        sink.clone(),
        60_000,
        200,
    );

    writer.fill_idle_silence().expect("idle fill");
    assert_eq!(writer.written, 0);
    assert!(
        wav_files(&dir.path().join("audio")).is_empty(),
        "no chunk may be opened before there is anything to write"
    );
}

/// A lost source must not keep growing a track it can no longer capture.
#[test]
fn idle_silence_stops_once_the_source_is_lost() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = idle_writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        sink.clone(),
        60_000,
        5_000,
    );

    writer.set_state(SourceState::Lost);
    writer.fill_idle_silence().expect("idle fill");
    assert_eq!(writer.written, 0, "a lost source records nothing further");
}

// -- 1. timeline: gap insertion ----------------------------------------

#[test]
fn plan_block_prefills_leading_offset_with_exact_silence() {
    // First microphone callback of spike run r3 arrived at t_rel_ms = 11.48
    // carrying 480 frames @ 48 kHz (= 10 ms of audio), so its audio starts
    // at 1.48 ms and 1.48 ms of leading silence aligns it to sample 0.
    let p = plan_block(0, false, 11.48, 480, 48_000);
    assert_eq!(p.fill_samples, ms_to_samples(1.48));
    assert_eq!(p.fill_samples, 24);
    assert_eq!(p.class, GapClass::Normal);
    assert!(!p.fill_capped);
}

#[test]
fn plan_block_fills_gap_with_exactly_the_missing_sample_count() {
    // 1 000 ms already written; next block's audio starts at 2 000 ms.
    let p = plan_block(16_000, true, 2_010.0, 480, 48_000);
    assert_eq!(p.gap_ms, 1_000.0);
    // 1 000 ms of 16 kHz mono silence is exactly 16 000 samples.
    assert_eq!(p.fill_samples, 16_000);
    assert_eq!(p.class, GapClass::Sparse);
}

#[test]
fn plan_block_absorbs_measured_callback_jitter_without_filling() {
    // The largest `normal`-class gap ever measured is 78.54 ms (smoke run,
    // microphone). It must not produce silence insertion, otherwise every
    // scheduling hiccup would nudge the track.
    let written = 16_000; // 1 000 ms
    let p = plan_block(written, true, 1_000.0 + 78.54 + 10.0, 480, 48_000);
    assert_eq!(
        p.fill_samples, 0,
        "measured jitter must not be silence-filled"
    );
    assert_eq!(p.gap_ms, 0.0);
}

#[test]
fn plan_block_caps_pathological_gap() {
    let p = plan_block(0, true, 10.0 * 3_600_000.0, 480, 48_000);
    assert!(p.fill_capped);
    assert_eq!(
        p.fill_samples,
        ms_to_samples(tolerances::MAX_SINGLE_GAP_FILL_MS)
    );
}

#[test]
fn gap_classification_matches_spike_harness_boundaries() {
    assert_eq!(classify_gap(0.0), GapClass::Normal);
    assert_eq!(classify_gap(78.538), GapClass::Normal); // measured max jitter
    assert_eq!(classify_gap(250.0), GapClass::Normal);
    assert_eq!(classify_gap(250.001), GapClass::Sparse);
    assert_eq!(classify_gap(1000.0), GapClass::Sparse);
    assert_eq!(classify_gap(1000.001), GapClass::Long);
    assert_eq!(classify_gap(5708.989), GapClass::Long); // measured starve
                                                        // Spelling must match `analyze-spike-csv.ps1` so telemetry is diffable.
    assert_eq!(GapClass::Normal.as_str(), "normal");
    assert_eq!(GapClass::Sparse.as_str(), "sparse");
    assert_eq!(GapClass::Long.as_str(), "gap");
}

#[test]
fn gap_is_written_as_exactly_the_right_number_of_silent_samples() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        sink.clone(),
        600_000,
        16_000,
    );

    // 500 ms of audio, a 2 s hole, then 500 ms more.
    let block = vec![0.5f32; 160]; // 10 ms @ 16 kHz
    for i in 1..=50 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    for i in 1..=50 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: 2_500.0 + i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    writer.finish().expect("finish");

    // 500 ms audio + 2 000 ms silence + 500 ms audio = 3 000 ms = 48 000
    // samples. The track is never shortened.
    let files = wav_files(&dir.path().join("audio"));
    assert_eq!(files.len(), 1);
    assert_eq!(assert_valid_wav(&files[0]), 48_000);
    assert_eq!(writer.written, 48_000);
    assert_eq!(sink.count_gaps(), 1);
}

// -- 2. timeline: drift -------------------------------------------------

#[test]
fn drift_tracker_reproduces_measured_ppm() {
    // A nominal stream must measure ~0 ppm.
    let mut t = DriftTracker::new(48_000);
    for i in 1..=6_000 {
        t.observe(i as f64 * 10.0, 480, false);
    }
    assert!(
        t.measured_ppm().abs() < 1.0,
        "nominal stream must measure ~0 ppm, got {}",
        t.measured_ppm()
    );

    // A genuinely fast clock: same wall time, a surplus of frames. Four
    // extra frames every 100 blocks over 48 000 frames is ~83 ppm, the same
    // order as the worst measured device (+70.3 ppm, run r3 loopback).
    let mut fast = DriftTracker::new(48_000);
    for i in 1..=6_000 {
        let frames = if i % 100 == 0 { 484 } else { 480 };
        fast.observe(i as f64 * 10.0, frames, false);
    }
    let measured = fast.measured_ppm();
    assert!(
        (measured - 83.3).abs() < 5.0,
        "expected ~83 ppm, got {measured}"
    );
    assert!(fast.drift_frames_native() > 0);
    // 83 ppm at 48 kHz maps to a proportionally smaller 16 kHz frame count.
    assert!(fast.drift_frames_timeline() < fast.drift_frames_native());
}

#[test]
fn drift_tracker_excludes_starvation_intervals() {
    // WASAPI loopback delivers nothing while nothing is playing (measured:
    // 5 709 ms of zero callbacks in smoke-final-verify). That must not be
    // read as clock drift.
    let mut t = DriftTracker::new(48_000);
    for i in 1..=100 {
        t.observe(i as f64 * 10.0, 480, false);
    }
    let before = t.drift_frames_native();
    // 5.7 s hole, flagged as a gap.
    t.observe(1_000.0 + 5_709.0, 480, true);
    for i in 1..=100 {
        t.observe(6_709.0 + i as f64 * 10.0, 480, false);
    }
    assert!(
        t.drift_frames_native().abs() <= before.abs() + 1,
        "starvation must not pollute the drift estimate"
    );
    assert!(
        t.measured_ppm().abs() < 100.0,
        "measured ppm after starvation was {}",
        t.measured_ppm()
    );
    // Delivered-frame bookkeeping still counts everything.
    assert_eq!(t.delivered_frames(), 201 * 480);
}

#[test]
fn drift_controller_respects_deadband_rate_limit_and_bound() {
    let mut ctl = DriftController::new();

    // Warm-up window: nothing happens before the stable window.
    assert_eq!(
        ctl.evaluate(5_000.0, 5_000, 500.0),
        DriftDecision::default()
    );

    // Measured healthy-device noise (±7 ppm) is inside the deadband.
    let d = ctl.evaluate(20_000.0, 10, 7.0);
    assert_eq!(d.retune_ppm, None, "healthy hardware must never be retuned");

    // A real deviation is corrected, and the applied ppm honours the bound.
    let d = ctl.evaluate(30_000.0, 4_000, 70.3);
    let ppm = d.retune_ppm.expect("correction expected");
    assert!(
        ppm.abs() <= tolerances::DRIFT_CORRECTION_MAX_PPM,
        "applied {ppm} ppm exceeds the documented bound"
    );
    assert!(ppm < 0.0, "positive drift must be corrected downwards");
    assert!(d.degraded, "4 000 frames is past DRIFT_WARN_FRAMES");

    // Rate limit: no second retune inside one second.
    let d = ctl.evaluate(30_500.0, 8_000, 200.0);
    assert_eq!(d.retune_ppm, None, "retune must be rate-limited to 1/s");

    // Escape hatch: beyond the envelope correction stops for good.
    let d = ctl.evaluate(60_000.0, tolerances::DRIFT_EXCEEDED_FRAMES + 1, 900.0);
    assert!(d.exceeded);
    assert!(ctl.has_exceeded());
    let d = ctl.evaluate(120_000.0, 100_000, 900.0);
    assert_eq!(d.retune_ppm, None);
    assert!(!d.exceeded, "exceeded is reported exactly once");
}

#[test]
fn drift_correction_never_exceeds_the_documented_ppm_envelope() {
    let mut ctl = DriftController::new();
    let mut now = tolerances::DRIFT_STABLE_WINDOW_MS;
    // Sweep drift magnitudes right up to the escape hatch.
    for step in 0..200 {
        now += tolerances::DRIFT_RETUNE_MIN_INTERVAL_MS;
        let drift = (step * 79) % (tolerances::DRIFT_EXCEEDED_FRAMES - 1);
        let d = ctl.evaluate(now, drift, (step as f64) * 13.0 - 1_000.0);
        if let Some(ppm) = d.retune_ppm {
            assert!(
                ppm.abs() <= tolerances::DRIFT_CORRECTION_MAX_PPM,
                "applied {ppm} ppm at step {step}"
            );
        }
    }
}

// -- 3. chunk rollover --------------------------------------------------

#[test]
fn rollover_produces_individually_valid_equally_sized_chunks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        sink.clone(),
        1_000, // 1 s rollover
        16_000,
    );

    let block = vec![0.25f32; 160]; // 10 ms
    for i in 1..=250 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    writer.finish().expect("finish");

    let audio = dir.path().join("audio");
    let files = wav_files(&audio);
    assert_eq!(files.len(), 3, "2.5 s at 1 s rollover = 3 chunks");
    assert_eq!(assert_valid_wav(&files[0]), 16_000);
    assert_eq!(assert_valid_wav(&files[1]), 16_000);
    assert_eq!(assert_valid_wav(&files[2]), 8_000);

    // Filenames follow the persistence layout exactly.
    for (i, f) in files.iter().enumerate() {
        assert_eq!(
            f.file_name().and_then(|n| n.to_str()),
            Some(format!("{:06}-microphone.wav", i + 1).as_str())
        );
    }
    // No staging file survives a clean finish.
    assert!(part_files(&audio).is_empty());

    // Every chunk is `ready` in the store with a checksum and a contiguous
    // timeline range.
    let rows = store.all();
    assert_eq!(rows.len(), 3);
    let mut expected_start = 0.0;
    for row in &rows {
        assert_eq!(row.state, ChunkState::Ready);
        assert!(row.checksum_sha256.is_some());
        assert_eq!(row.range.start_ms, expected_start);
        expected_start = row.range.end_ms;
    }
    assert_eq!(expected_start, 2_500.0);
}

#[test]
fn sequence_numbers_are_never_reused_across_rollover() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        Arc::new(NullEventSink),
        100,
        16_000,
    );
    let block = vec![0.1f32; 160];
    for i in 1..=100 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    writer.finish().expect("finish");
    let mut seqs: Vec<u32> = store.all().iter().map(|r| r.sequence).collect();
    let before = seqs.len();
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len(), before, "sequence numbers must be unique");
    assert_eq!(seqs.first().copied(), Some(1));
}

// -- 4. atomic finalisation --------------------------------------------

#[test]
fn final_path_never_holds_a_partially_written_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        Arc::new(NullEventSink),
        1_000,
        16_000,
    );

    let block = vec![0.4f32; 160];
    for i in 1..=250 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
        // Invariant checked after *every* block: whatever sits at a final
        // `.wav` path is a complete, self-consistent file. Only the `.part`
        // name is ever incomplete.
        for f in wav_files(&audio) {
            assert_valid_wav(&f);
        }
        assert!(part_files(&audio).len() <= 1);
    }
    writer.finish().expect("finish");
}

#[test]
fn failed_rename_leaves_no_file_at_the_final_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let faulty = FaultyFs::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        faulty.clone(),
        Arc::new(NullEventSink),
        10_000,
        16_000,
    );

    let block = vec![0.4f32; 160];
    for i in 1..=50 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    faulty.fail_rename.store(true, Ordering::Release);
    let err = writer.finish().expect_err("rename failure must surface");
    assert!(matches!(err, RecorderError::ChunkCreateFailed { .. }));

    assert!(
        wav_files(&audio).is_empty(),
        "a failed commit must not publish anything at the final path"
    );
    assert_eq!(part_files(&audio).len(), 1, "staging file is preserved");
    // The row is still `writing`, so recovery will pick it up.
    assert_eq!(store.all()[0].state, ChunkState::Writing);
}

#[test]
fn simulated_crash_mid_chunk_leaves_all_previous_chunks_usable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        Arc::new(NullEventSink),
        1_000,
        16_000,
    );

    let block = vec![0.6f32; 160];
    for i in 1..=250 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    // Hard kill: drop the writer without ever calling finish(). The open
    // chunk's buffered data is lost; everything already renamed is not.
    drop(writer);

    let files = wav_files(&audio);
    assert_eq!(files.len(), 2, "two completed chunks survived the crash");
    assert_eq!(assert_valid_wav(&files[0]), 16_000);
    assert_eq!(assert_valid_wav(&files[1]), 16_000);
    assert_eq!(
        part_files(&audio).len(),
        1,
        "one open chunk was interrupted"
    );

    let states: Vec<ChunkState> = store.all().iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        vec![ChunkState::Ready, ChunkState::Ready, ChunkState::Writing]
    );
}

#[test]
fn repeated_store_failure_keeps_the_file_and_reports_persist_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        Arc::new(NullEventSink),
        10_000,
        16_000,
    );
    let block = vec![0.2f32; 160];
    for i in 1..=50 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    store.fail_transitions.store(true, Ordering::Release);
    let err = writer.finish().expect_err("persist failure must surface");
    assert!(matches!(err, RecorderError::ChunkStatePersistFailed { .. }));
    // Contract §3.1.2: the file is kept — it is already durably `ready`.
    assert_eq!(wav_files(&audio).len(), 1);
    assert_valid_wav(&wav_files(&audio)[0]);
}

// -- 5. independent sources --------------------------------------------

#[test]
fn one_source_failing_leaves_the_other_recording_and_rolling_over() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let backend = FakeBackend::new(16_000);
    let sink = Arc::new(CollectingSink::default());
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.subscribe(sink.clone());
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    rec.open_source(SourceKind::Microphone, request("mic", 1_000))
        .expect("open mic");
    rec.open_source(SourceKind::System, request("speakers", 1_000))
        .expect("open system");

    // Both sources record 1.5 s.
    backend.feed_span(SourceKind::Microphone, 0.0, 1_500.0, 10.0, 0.3);
    backend.feed_span(SourceKind::System, 0.0, 1_500.0, 10.0, 0.3);

    // The microphone device disappears.
    backend.fail(SourceKind::Microphone, "device removed");
    assert!(rec.is_source_lost(SourceKind::Microphone));

    // The system source keeps recording for another 2.5 s and keeps rolling.
    backend.feed_span(SourceKind::System, 1_500.0, 2_500.0, 10.0, 0.3);

    let summary = rec.stop().expect("stop");

    let mic: Vec<PathBuf> = wav_files(&audio)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("microphone"))
        .collect();
    let sys: Vec<PathBuf> = wav_files(&audio)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("-system"))
        .collect();

    // Mic: the chunk open at loss time was finalized to `ready` first.
    assert_eq!(mic.len(), 2, "mic keeps its completed + finalized chunk");
    let mic_samples: u64 = mic.iter().map(|p| assert_valid_wav(p)).sum();
    assert_eq!(mic_samples, 24_000, "1.5 s of microphone audio survived");

    // System: unaffected, rolled over four times.
    assert_eq!(
        sys.len(),
        4,
        "system source kept rolling over after mic loss"
    );
    let sys_samples: u64 = sys.iter().map(|p| assert_valid_wav(p)).sum();
    assert_eq!(sys_samples, 64_000, "4 s of system audio");

    assert_eq!(summary.finalized_chunks.len(), 6);
    assert!(part_files(&audio).is_empty());

    let events = sink.snapshot();
    assert!(events
        .iter()
        .any(|e| matches!(e, RecorderEvent::MicDeviceLost { .. })));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RecorderEvent::SystemDeviceLost { .. })),
        "system source must be untouched by a microphone failure"
    );
}

#[test]
fn sources_keep_independent_rollover_counters() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let backend = FakeBackend::new(16_000);
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    rec.open_source(SourceKind::Microphone, request("mic", 500))
        .expect("open mic");
    rec.open_source(SourceKind::System, request("speakers", 2_000))
        .expect("open system");

    backend.feed_span(SourceKind::Microphone, 0.0, 2_000.0, 10.0, 0.2);
    backend.feed_span(SourceKind::System, 0.0, 2_000.0, 10.0, 0.2);
    rec.stop().expect("stop");

    let audio = dir.path().join("audio");
    let mic = wav_files(&audio)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("microphone"))
        .count();
    let sys = wav_files(&audio)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("-system"))
        .count();
    assert_eq!(mic, 4, "500 ms rollover over 2 s");
    assert_eq!(sys, 1, "2 000 ms rollover over 2 s");
}

// -- 6. pause / resume --------------------------------------------------

#[test]
fn pause_and_resume_markers_land_at_the_same_frozen_timeline_position() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let backend = FakeBackend::new(16_000);
    let sink = Arc::new(CollectingSink::default());
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.subscribe(sink.clone());
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    rec.open_source(SourceKind::Microphone, request("mic", 60_000))
        .expect("open mic");

    backend.feed_span(SourceKind::Microphone, 0.0, 1_000.0, 10.0, 0.5);
    let pause = rec.pause().expect("pause");
    assert_eq!(rec.snapshot().state, RecorderState::Paused);
    // Pause is a durability checkpoint: the open chunk is already `ready`.
    assert_eq!(wav_files(&audio).len(), 1);
    assert_eq!(assert_valid_wav(&wav_files(&audio)[0]), 16_000);
    assert!(part_files(&audio).is_empty());

    std::thread::sleep(Duration::from_millis(120));

    let devices = SourceDeviceSet {
        microphone: Some(request("mic", 60_000)),
        system: None,
    };
    let resume = rec.resume(devices).expect("resume");

    // The paused span is removed from the timeline, so both markers sit at
    // the same position even though 120 ms of wall time elapsed.
    assert_eq!(pause.kind, MarkerKind::Pause);
    assert_eq!(resume.kind, MarkerKind::Resume);
    assert!(
        (resume.timeline_ms - pause.timeline_ms).abs() < 30.0,
        "timeline must freeze across pause: pause {} vs resume {}",
        pause.timeline_ms,
        resume.timeline_ms
    );
    assert!(resume.wall_clock > pause.wall_clock);

    // Audio after resume continues at the frozen position: no 120 ms hole.
    backend.feed_span(SourceKind::Microphone, 1_000.0, 1_000.0, 10.0, 0.5);
    rec.stop().expect("stop");

    let total: u64 = wav_files(&audio).iter().map(|p| assert_valid_wav(p)).sum();
    assert_eq!(total, 32_000, "2 s of capture, pause excluded");
    assert_eq!(
        sink.count_gaps(),
        0,
        "a pause must never be reported as a timeline gap"
    );

    // Sequence numbers continue across the pause; none is reused.
    let seqs: Vec<u32> = store.all().iter().map(|r| r.sequence).collect();
    assert_eq!(seqs, vec![1, 2]);

    let snapshot = rec.snapshot();
    assert_eq!(snapshot.markers.len(), 2);
    assert_eq!(snapshot.markers[0].kind, MarkerKind::Pause);
    assert_eq!(snapshot.markers[1].kind, MarkerKind::Resume);
}

#[test]
fn resume_with_an_unavailable_device_keeps_the_session_paused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let backend = FakeBackend::new(16_000);
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    rec.open_source(SourceKind::Microphone, request("mic", 60_000))
        .expect("open mic");
    backend.feed_span(SourceKind::Microphone, 0.0, 200.0, 10.0, 0.5);
    rec.pause().expect("pause");

    lock_recover(&backend.fail_open).insert(
        SourceKind::Microphone,
        RecorderError::MicDeviceUnavailable {
            device_name: "mic".to_string(),
            detail: "unplugged".to_string(),
        },
    );
    let devices = SourceDeviceSet {
        microphone: Some(request("mic", 60_000)),
        system: None,
    };
    let err = rec.resume(devices).expect_err("resume must fail");
    assert!(matches!(err, RecorderError::MicDeviceUnavailable { .. }));
    assert_eq!(rec.snapshot().state, RecorderState::Paused);
}

// -- 7. recovery --------------------------------------------------------

fn seed_open_chunk(store: &InMemoryStore, sequence: u32, start_ms: f64) -> ChunkId {
    let id = 900 + i64::from(sequence);
    store.seed_writing(ChunkDescriptor {
        chunk_id: id,
        source: SourceKind::Microphone,
        sequence,
        source_segment_id: 1,
        state: ChunkState::Writing,
        range: TimelineRange {
            start_ms,
            end_ms: start_ms,
        },
        sample_count: 0,
        storage_key: format!("audio/{sequence:06}-microphone.wav"),
        checksum_sha256: None,
        recovered: false,
    })
}

fn recorder_for_recovery(dir: &Path) -> MeetingRecorderImpl {
    let mut rec = MeetingRecorderImpl::with_parts(
        FakeBackend::new(16_000),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.layout = Some(layout_in(dir));
    rec
}

#[test]
fn recovery_repairs_a_truncated_open_chunk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 7, 1_000.0);

    // A crashed writer: header still claims 0 bytes (never patched), 1 001
    // payload bytes on disk — deliberately an odd number, i.e. the last
    // PCM16 frame is half-written.
    let path = audio.join("000007-microphone.wav.part");
    let mut f = File::create(&path).expect("create");
    f.write_all(&provisional_header()).expect("header");
    f.write_all(&vec![7u8; 1_001]).expect("payload");
    drop(f);

    let rec = recorder_for_recovery(dir.path());
    let report = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover");

    assert_eq!(report.finalized, vec![id]);
    assert!(report.damaged.is_empty());
    let final_path = audio.join("000007-microphone.wav");
    // Truncated down to a whole frame: 1 000 bytes = 500 samples.
    assert_eq!(assert_valid_wav(&final_path), 500);
    assert!(!path.exists(), "the staging file is gone after the rename");

    let row = store.row(id).expect("row");
    assert_eq!(row.state, ChunkState::Ready);
    assert!(row.recovered);
    assert_eq!(row.sample_count, 500);
    assert_eq!(row.range.start_ms, 1_000.0);
    assert!((row.range.end_ms - 1_031.25).abs() < 1e-9);
    assert!(row.checksum_sha256.is_some());
}

#[test]
fn recovery_uses_the_smaller_of_declared_and_actual_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 3, 0.0);

    // Header claims 4 000 bytes, only 800 were actually flushed.
    let path = audio.join("000003-microphone.wav.part");
    let mut f = File::create(&path).expect("create");
    f.write_all(&wav_header(4_000)).expect("header");
    f.write_all(&vec![1u8; 800]).expect("payload");
    drop(f);

    let rec = recorder_for_recovery(dir.path());
    rec.recover_open_chunks("session-under-test", store.clone())
        .expect("recover");
    assert_eq!(assert_valid_wav(&audio.join("000003-microphone.wav")), 400);
    assert_eq!(store.row(id).expect("row").sample_count, 400);
}

#[test]
fn recovery_quarantines_an_unparseable_open_chunk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 4, 0.0);

    let path = audio.join("000004-microphone.wav.part");
    fs::write(&path, b"not a wav file at all").expect("write");

    let rec = recorder_for_recovery(dir.path());
    let report = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover");

    assert_eq!(report.damaged, vec![id]);
    assert_eq!(report.quarantined_files.len(), 1);
    assert!(report.quarantined_files[0].exists());
    assert!(!path.exists());
    assert!(wav_files(&audio).is_empty());
    assert_eq!(store.row(id).expect("row").state, ChunkState::Damaged);
}

#[test]
fn recovery_marks_a_row_without_a_file_damaged() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("audio")).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 9, 0.0);

    let rec = recorder_for_recovery(dir.path());
    let report = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover");
    assert_eq!(report.damaged, vec![id]);
    assert_eq!(store.row(id).expect("row").state, ChunkState::Damaged);
}

#[test]
fn recovery_adopts_a_chunk_renamed_before_the_db_commit() {
    // Crash between the atomic rename and the DB transaction: the `.wav`
    // exists and is valid, the row still says `writing`.
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 2, 500.0);

    let mut f = File::create(audio.join("000002-microphone.wav")).expect("create");
    f.write_all(&wav_header(320)).expect("header");
    f.write_all(&vec![0u8; 320]).expect("payload");
    drop(f);

    let rec = recorder_for_recovery(dir.path());
    let report = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover");
    assert_eq!(report.finalized, vec![id]);
    let row = store.row(id).expect("row");
    assert_eq!(row.state, ChunkState::Ready);
    assert_eq!(row.sample_count, 160);
    assert!(row.recovered);
}

#[test]
fn recovery_quarantines_orphan_part_files_and_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 1, 0.0);

    let mut f = File::create(audio.join("000001-microphone.wav.part")).expect("create");
    f.write_all(&provisional_header()).expect("header");
    f.write_all(&vec![3u8; 640]).expect("payload");
    drop(f);
    // An orphan staging file that no row claims.
    fs::write(audio.join("000042-system.wav.part"), b"orphan").expect("write orphan");

    let rec = recorder_for_recovery(dir.path());
    let first = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover");
    assert_eq!(first.finalized, vec![id]);
    assert_eq!(first.quarantined_files.len(), 1);
    assert!(first.quarantined_files[0]
        .to_string_lossy()
        .contains("000042-system"));

    let before: Vec<ChunkState> = store.all().iter().map(|r| r.state).collect();
    let files_before = wav_files(&audio);

    // Idempotence: a second run changes nothing.
    let second = rec
        .recover_open_chunks("session-under-test", store.clone())
        .expect("recover twice");
    assert!(second.finalized.is_empty());
    assert!(second.damaged.is_empty());
    assert!(second.quarantined_files.is_empty());
    assert_eq!(
        before,
        store
            .all()
            .iter()
            .map(|r| r.state)
            .collect::<Vec<ChunkState>>()
    );
    assert_eq!(files_before, wav_files(&audio));
}

#[test]
fn start_session_recovers_before_writing_new_audio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let store = InMemoryStore::new();
    let id = seed_open_chunk(&store, 5, 0.0);
    let mut f = File::create(audio.join("000005-microphone.wav.part")).expect("create");
    f.write_all(&provisional_header()).expect("header");
    f.write_all(&vec![2u8; 320]).expect("payload");
    drop(f);

    let backend = FakeBackend::new(16_000);
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    assert_eq!(store.row(id).expect("row").state, ChunkState::Ready);
    assert_eq!(assert_valid_wav(&audio.join("000005-microphone.wav")), 160);
    rec.stop().expect("stop");
}

// -- 8. bounded memory --------------------------------------------------

#[test]
fn two_hour_run_does_not_grow_recorder_memory() {
    // Explicit acceptance criterion: RAM must not grow with meeting
    // duration. Two hours of 16 kHz mono are pushed through the real chunk
    // pipeline; the sink deletes each finalized chunk so the test needs
    // ~2 MB of disk rather than 230 MB, but every byte is still written,
    // header-patched, renamed and checksummed.
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    fs::create_dir_all(&audio).expect("mkdir");
    *lock_recover(&sink.delete_finalized) = Some(audio.clone());

    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        sink.clone(),
        tolerances::CHUNK_ROLLOVER_TARGET_MS,
        16_000,
    );

    // 100 ms blocks → 72 000 callbacks over two hours.
    let block = vec![0.35f32; 1_600];
    let total_blocks = 72_000u64;
    let mut capacity_after_first_minute = None;

    for i in 1..=total_blocks {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 100.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
        if i == 600 {
            capacity_after_first_minute = Some((
                writer.out_buf.capacity(),
                writer
                    .chunk
                    .as_ref()
                    .map(|c| c.scratch.capacity())
                    .unwrap_or(0),
            ));
        }
    }

    let (out_cap_1min, scratch_cap_1min) =
        capacity_after_first_minute.expect("captured after one minute");
    let out_cap_2h = writer.out_buf.capacity();
    let scratch_cap_2h = writer
        .chunk
        .as_ref()
        .map(|c| c.scratch.capacity())
        .unwrap_or(0);

    // The two working buffers are the recorder's entire per-sample memory.
    // Their capacity after two hours must equal their capacity after one
    // minute — that is the bounded-memory property, asserted rather than
    // claimed in a comment.
    assert_eq!(
        out_cap_1min, out_cap_2h,
        "resampler output buffer grew from {out_cap_1min} to {out_cap_2h}"
    );
    assert_eq!(
        scratch_cap_1min, scratch_cap_2h,
        "PCM16 scratch grew from {scratch_cap_1min} to {scratch_cap_2h}"
    );
    assert!(
        out_cap_2h <= SCRATCH_SAMPLES,
        "output buffer capacity {out_cap_2h} is not bounded"
    );
    assert!(
        scratch_cap_2h <= SCRATCH_SAMPLES * 2,
        "scratch capacity {scratch_cap_2h} is not bounded"
    );

    writer.finish().expect("finish");

    // Two hours at a 60 s rollover is exactly 120 chunks, and the full
    // timeline was written: nothing was buffered in RAM instead.
    assert_eq!(writer.finalized.len(), 120);
    assert_eq!(writer.written, 2 * 3_600 * 16_000);
    // No timeline gaps: a perfectly regular stream must not silence-fill.
    assert_eq!(sink.count_gaps(), 0);
    assert!(part_files(&audio).is_empty());
}

#[test]
fn capture_queue_is_hard_bounded_when_the_writer_stalls() {
    // The queue bound is what decouples RAM from duration when the disk is
    // slow: past MAX_QUEUED_CAPTURE_BLOCKS the capture callback drops blocks
    // instead of growing the queue, and the resulting hole is silence-filled
    // by ordinary gap bookkeeping.
    let queued = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel::<WriterCmd>();
    let cb_queued = Arc::clone(&queued);
    let mut on_block: Box<dyn FnMut(CaptureBlock) + Send> = Box::new(move |block| {
        if cb_queued.load(Ordering::Acquire) >= tolerances::MAX_QUEUED_CAPTURE_BLOCKS {
            return;
        }
        cb_queued.fetch_add(1, Ordering::AcqRel);
        if tx.send(WriterCmd::Block(block)).is_err() {
            cb_queued.fetch_sub(1, Ordering::AcqRel);
        }
    });

    for i in 0..10_000 {
        on_block(CaptureBlock {
            timeline_ms: i as f64 * 10.0,
            mono: vec![0.1; 160],
            native_rate: 16_000,
        });
    }
    assert_eq!(
        queued.load(Ordering::Acquire),
        tolerances::MAX_QUEUED_CAPTURE_BLOCKS
    );
    let delivered = rx.try_iter().count();
    assert_eq!(delivered, tolerances::MAX_QUEUED_CAPTURE_BLOCKS);
}

// -- 9. misc contract surface ------------------------------------------

#[test]
fn source_file_tags_match_the_persistence_layout() {
    assert_eq!(SourceKind::Microphone.file_tag(), "microphone");
    assert_eq!(SourceKind::System.file_tag(), "system");
    let layout = SessionLayout {
        session_id: "m1".into(),
        root: PathBuf::from("meetings").join("m1"),
    };
    assert!(layout.audio_dir().ends_with("audio"));
    assert!(layout.recovery_dir().ends_with("recovery"));
}

#[test]
fn wav_header_round_trips() {
    let h = wav_header(1_234);
    assert_eq!(parse_wav_header(&h), Some(1_234));
    assert_eq!(parse_wav_header(&h[..40]), None);
    assert_eq!(
        parse_wav_header(b"garbage garbage garbage garbage garbage ...."),
        None
    );
}

#[test]
fn pcm16_conversion_clamps_instead_of_wrapping() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audio = dir.path().join("audio");
    fs::create_dir_all(&audio).expect("mkdir");
    let mut chunk = ChunkWriter::open(&RealChunkFs, &audio, SourceKind::Microphone, 1, 1, 0)
        .expect("open chunk");
    chunk.write(&[2.0, -2.0, 0.0, 1.0, -1.0]).expect("write");
    let finalized = chunk.finalize(&RealChunkFs).expect("finalize");
    assert_eq!(finalized.info.sample_count, 5);

    let bytes = fs::read(audio.join("000001-microphone.wav")).expect("read");
    let samples: Vec<i16> = bytes[WAV_HEADER_LEN..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect();
    assert_eq!(samples, vec![32_767, -32_767, 0, 32_767, -32_767]);
}

#[test]
fn levels_match_the_dictation_meter_scale() {
    let (peak, rms) = block_levels(&[0.0, 0.5, -1.0, 0.5]);
    assert_eq!(peak, 1.0);
    assert!((rms - 0.6123724).abs() < 1e-5);
    let (peak, rms) = block_levels(&[]);
    assert_eq!((peak, rms), (0.0, 0.0));
    // Silence classification uses the spike run's threshold verbatim.
    let (quiet, _) = block_levels(&[0.00005, -0.00002]);
    assert!(quiet < tolerances::SILENCE_PEAK_THRESHOLD);
}

#[test]
fn resampling_from_48k_lands_on_the_timeline_rate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let mut writer = writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        Arc::new(RealChunkFs),
        Arc::new(NullEventSink),
        600_000,
        48_000,
    );
    // 2 s of 48 kHz audio in the measured 480-frame (10 ms) blocks.
    let block = vec![0.2f32; 480];
    for i in 1..=200 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 48_000,
            })
            .expect("push");
    }
    writer.finish().expect("finish");
    let files = wav_files(&dir.path().join("audio"));
    assert_eq!(files.len(), 1);
    let samples = assert_valid_wav(&files[0]);
    // 2 s at 16 kHz = 32 000 samples, within one resampler frame.
    assert!(
        (samples as i64 - 32_000).abs() <= 320,
        "expected ~32 000 timeline samples, got {samples}"
    );
}

#[test]
fn system_source_silence_is_never_treated_as_degradation() {
    // Measured behaviour: silent WASAPI loopback delivers zero callbacks for
    // seconds at a time. Contract §7.1 says that is not an error.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = writer_for(
        dir.path(),
        SourceKind::System,
        store.clone(),
        Arc::new(RealChunkFs),
        sink.clone(),
        600_000,
        16_000,
    );
    let block = vec![0.4f32; 160];
    writer
        .push_block(CaptureBlock {
            timeline_ms: 5_709.0,
            mono: block.clone(),
            native_rate: 16_000,
        })
        .expect("push");
    writer
        .push_block(CaptureBlock {
            timeline_ms: 30_000.0,
            mono: block,
            native_rate: 16_000,
        })
        .expect("push");
    writer.finish().expect("finish");

    assert_eq!(
        writer.state,
        SourceState::Active,
        "silence is not degradation"
    );
    assert!(writer.degraded_ranges.is_empty());
    // The hole is still represented as silence — tracks are never shortened.
    // The final flush zero-pads to a whole resampler frame, so the track may
    // exceed the exact timeline by at most one frame (20 ms).
    let frame = ms_to_samples(tolerances::RESAMPLER_FRAME_MS as f64);
    let written = assert_valid_wav(&wav_files(&dir.path().join("audio"))[0]);
    let exact = ms_to_samples(30_000.0);
    assert!(
        written >= exact && written <= exact + frame,
        "expected {exact}..={} timeline samples, got {written}",
        exact + frame
    );
    // ...and it is still reported, so the UI can show it.
    assert_eq!(sink.count_gaps(), 2);
}

#[test]
fn microphone_gap_flags_the_source_degraded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let sink = Arc::new(CollectingSink::default());
    let mut writer = writer_for(
        dir.path(),
        SourceKind::Microphone,
        store.clone(),
        Arc::new(RealChunkFs),
        sink.clone(),
        600_000,
        16_000,
    );
    let block = vec![0.4f32; 160];
    for i in 1..=10 {
        writer
            .push_block(CaptureBlock {
                timeline_ms: i as f64 * 10.0,
                mono: block.clone(),
                native_rate: 16_000,
            })
            .expect("push");
    }
    writer
        .push_block(CaptureBlock {
            timeline_ms: 3_000.0,
            mono: block,
            native_rate: 16_000,
        })
        .expect("push");
    writer.finish().expect("finish");

    assert_eq!(writer.state, SourceState::Degraded);
    assert_eq!(writer.degraded_ranges.len(), 1);
    let (source, range, class) = writer.degraded_ranges[0];
    assert_eq!(source, SourceKind::Microphone);
    assert_eq!(class, GapClass::Long);
    assert!((range.start_ms - 100.0).abs() < 1.0);
}

#[test]
fn preflight_reports_one_check_per_requested_source() {
    let rec = MeetingRecorderImpl::with_parts(
        FakeBackend::new(48_000),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    let mut devices = SourceDeviceSet {
        microphone: Some(request("mic", 60_000)),
        system: None,
    };
    let checks = rec.preflight(&devices).expect("preflight");
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].source, SourceKind::Microphone);
    assert_eq!(checks[0].outcome, PreflightOutcome::Available);
    assert_eq!(checks[0].native_sample_rate_hint, Some(48_000));

    devices.system = Some(request("speakers", 60_000));
    assert_eq!(rec.preflight(&devices).expect("preflight").len(), 2);
}

#[test]
fn open_error_classification_matches_the_contract_taxonomy() {
    assert!(matches!(
        classify_open_error(SourceKind::Microphone, "mic", "Access is denied.".into()),
        RecorderError::CapturePermissionDenied { .. }
    ));
    assert!(matches!(
        classify_open_error(
            SourceKind::System,
            "spk",
            "AUDCLNT_E_EXCLUSIVE_MODE_ONLY".into()
        ),
        RecorderError::LoopbackSharedModeRequired { .. }
    ));
    assert!(matches!(
        classify_open_error(SourceKind::System, "spk", "not found".into()),
        RecorderError::SystemOutputUnavailable { .. }
    ));
    assert!(matches!(
        classify_open_error(SourceKind::Microphone, "mic", "not found".into()),
        RecorderError::MicDeviceUnavailable { .. }
    ));
}

#[cfg(not(target_os = "windows"))]
#[test]
fn system_audio_is_reported_unsupported_off_windows() {
    // Plan §2 puts macOS/Linux system audio out of scope; the file still
    // compiles there and the unsupported path is explicit, not silent.
    let backend = CpalCaptureBackend::new(Instant::now(), Arc::new(Mutex::new(0.0)));
    let check = backend.preflight(SourceKind::System, &request("any", 60_000));
    assert_eq!(check.outcome, PreflightOutcome::Missing);
    assert!(check.detail.contains("Windows"));

    // `matches!` rather than `expect_err`: the Ok type is a trait object and
    // deliberately carries no `Debug` bound.
    let opened = backend.open(
        SourceKind::System,
        &request("any", 60_000),
        Box::new(|_| {}),
        Box::new(|_| {}),
    );
    assert!(
        matches!(opened, Err(RecorderError::LoopbackUnavailable { .. })),
        "loopback must be unavailable off Windows"
    );
}

#[test]
fn snapshot_reflects_backend_state_for_all_three_ui_surfaces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let backend = FakeBackend::new(16_000);
    let mut rec = MeetingRecorderImpl::with_parts(
        backend.clone(),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    assert_eq!(rec.snapshot().state, RecorderState::Idle);
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    rec.open_source(SourceKind::Microphone, request("mic", 1_000))
        .expect("open");
    rec.open_source(SourceKind::System, request("spk", 1_000))
        .expect("open");

    let snap = rec.snapshot();
    assert_eq!(snap.state, RecorderState::Recording);
    assert_eq!(snap.sources.len(), 2);
    assert_eq!(snap.sources[0].source, SourceKind::Microphone);
    assert_eq!(snap.sources[1].source, SourceKind::System);
    assert!(snap.sources.iter().all(|s| s.state == SourceState::Active));
    assert_eq!(rec.source_device_name(SourceKind::Microphone), Some("mic"));
    assert_eq!(rec.current_segment_id(SourceKind::Microphone), Some(1));
    assert_eq!(rec.current_segment_id(SourceKind::System), Some(2));
    assert_eq!(rec.queued_capture_blocks(SourceKind::Microphone), 0);

    backend.feed_span(SourceKind::Microphone, 0.0, 2_000.0, 10.0, 0.3);
    rec.stop().expect("stop");
    assert_eq!(rec.snapshot().state, RecorderState::Idle);
}

#[test]
fn second_start_session_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = InMemoryStore::new();
    let mut rec = MeetingRecorderImpl::with_parts(
        FakeBackend::new(16_000),
        Arc::new(RealChunkFs),
        Instant::now(),
        Arc::new(Mutex::new(0.0)),
    );
    rec.start_session(layout_in(dir.path()), store.clone())
        .expect("start");
    let err = rec
        .start_session(layout_in(dir.path()), store.clone())
        .expect_err("second start must fail");
    assert_eq!(err, RecorderError::SessionAlreadyActive);
    rec.stop().expect("stop");
    assert_eq!(
        rec.stop().expect_err("no session"),
        RecorderError::NoActiveSession
    );
}

#[test]
fn recorder_error_display_covers_every_variant() {
    let variants = vec![
        RecorderError::MicDeviceUnavailable {
            device_name: "m".into(),
            detail: String::new(),
        },
        RecorderError::SystemOutputUnavailable {
            device_name: "s".into(),
            detail: String::new(),
        },
        RecorderError::LoopbackSharedModeRequired {
            device_name: "s".into(),
            detail: String::new(),
        },
        RecorderError::CapturePermissionDenied {
            source: SourceKind::Microphone,
            detail: String::new(),
        },
        RecorderError::LoopbackUnavailable { detail: "x".into() },
        RecorderError::InsufficientDiskSpace {
            required_bytes: 1,
            free_bytes: 0,
        },
        RecorderError::ChunkCreateFailed {
            source: SourceKind::System,
            detail: String::new(),
        },
        RecorderError::ChunkStatePersistFailed {
            chunk_id: 1,
            detail: String::new(),
        },
        RecorderError::StreamError {
            source: SourceKind::System,
            detail: String::new(),
        },
        RecorderError::SessionAlreadyActive,
        RecorderError::NoActiveSession,
        RecorderError::RecoveryFailed {
            chunk_id: 2,
            detail: String::new(),
        },
    ];
    for v in variants {
        assert!(!v.to_string().is_empty(), "{v:?} has no Display text");
    }
}
