# MeetingRecorder & Shared-Timeline Interface Contract

Status: **contract draft — no implementation yet** (Phase 2 prerequisite)
Source of truth: `docs/meeting-mode-plan.md`, sections 5 ("Zielarchitektur" →
`MeetingRecorder`), 6.x (Session-Zustände, Startvalidierung, Pause, Stoppen,
Fehlerverhalten), 7 ("Windows-Audioaufnahme": Loopback, Gemeinsame Zeitachse,
Persistenzformat) and 14 (Testmatrix rows "Timeline-, Gap- und Drift-Berechnungen",
"Chunk-Rollover und atomare Finalisierung", "Recovery beschädigter/offener Chunks").
Planned implementation home: `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`.
This document defines the interface contract only; it changes no source code.

Reuse obligations (from plan §5): the meeting recorder reuses device enumeration
(`audio_toolkit::audio::device::{list_input_devices, list_output_devices}`),
sample conversion utilities and the existing
[`FrameResampler`](../../src-tauri/src/audio_toolkit/audio/resampler.rs). The
dictation path (`AudioRecorder`) is not modified and carries no meeting-specific
cases.

---

## 1. Scope and module boundaries

`MeetingRecorder` owns exactly one concern: **turning two live capture streams into
a pair of crash-safe, timeline-consistent PCM16 WAV chunk files plus a stream of
level/state/error events.** It does _not_ own:

| Concern                                               | Owner                                         | Interaction                                                                                                        |
| ----------------------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Session state machine, Tauri commands/events          | `MeetingManager`                              | consumes recorder events, drives `open_source`/`pause`/`resume`/`stop`                                             |
| Chunk/job/segment persistence (SQL)                   | `MeetingStore`                                | recorder reports chunk transitions through a `ChunkStateStore` trait so the recorder itself stays storage-agnostic |
| STT segmentation, queues, providers                   | `MeetingTranscriptionQueue`, provider clients | consume only chunks in state `ready`                                                                               |
| Free-disk policy, provider/model/retention validation | `MeetingManager` (Startvalidierung)           | recorder surfaces device- and disk-related failures only                                                           |

The recorder is the **single writer** of the session's `audio/` directory. No other
component creates, renames or deletes files under
`meetings/<meeting-id>/audio/` while a session is active.

## 2. Core types

### 2.1 Source identity

```rust
/// Physical capture source. Persisted verbatim as `meeting_audio_chunks.source`.
/// This is technical provenance information, never speaker identification
/// (plan §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Microphone,
    System,
}

impl SourceKind {
    /// Filename component per the persistence layout:
    /// `000001-microphone.wav` / `000001-system.wav`.
    pub fn file_tag(self) -> &'static str {
        match self {
            SourceKind::Microphone => "microphone",
            SourceKind::System => "system",
        }
    }
}
```

The two sources are technically independent streams with independent failure,
independent rollover counters and independent gap/drift bookkeeping. Nothing in
the recorder assumes that chunk _N_ of microphone covers the same wall time as
chunk _N_ of system.

### 2.2 Timeline model

```rust
/// Target sample format of the shared timeline: 16 kHz, mono, per source.
pub const TIMELINE_SAMPLE_RATE_HZ: u32 = 16_000;
pub const TIMELINE_CHANNELS: u16 = 1;

/// A half-open range on the shared meeting timeline, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRange {
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One pause/resume boundary pair recorded on the timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MarkerKind {
    Pause,
    Resume,
}

#[derive(Clone, Debug)]
pub struct TimelineMarker {
    pub kind: MarkerKind,
    /// Timeline position of the marker.
    pub timeline_ms: f64,
    /// Wall-clock instant, for display only; never used for audio math.
    pub wall_clock: std::time::SystemTime,
}
```

Timeline rules (normative):

1. **One monotonic origin per session.** At `start_session`, the recorder captures
   a single `std::time::Instant` as `timeline_origin`. Every capture callback from
   both sources is stamped with `Instant::now()` at callback entry;
   `timeline_ms = stamp.duration_since(timeline_origin).as_secs_f64() * 1000.0`.
   Wall-clock timestamps are metadata only and never drive audio alignment.
2. **Per-source sample counting.** Each source additionally maintains a monotonically
   increasing counter of _delivered device frames_ (`total_frames` in spike terms)
   and its native `sample_rate`. Expected frames at time _t_ are
   `expected(t) = (t - t_first_callback_ms) / 1000.0 * native_sample_rate`,
   mirroring the spike harness columns `frames`, `total_frames`, `expected_frames`.
3. **Capture-time timeline, not wall-time.** Paused intervals are removed from the
   timeline: after a resume, new audio continues at the timeline position where the
   pause marker was placed. Consequence: transcript timestamps and audio playback
   positions stay aligned, and gap detection must never treat a paused interval as
   a gap.
4. **Tracks are never shortened.** Any missing time on a source is represented as
   digital silence at its correct timeline position (plan §7, "Gemeinsame
   Zeitachse", task 2). Audio already written is never shifted.

### 2.3 Normalization to 16 kHz mono

Per source, before any byte reaches a chunk file:

1. **Downmix to mono** using the same channel-selection semantics as the dictation
   recorder (selected input channel if configured for the microphone; average of
   channels otherwise; loopback stereo is averaged to mono).
2. **Resample** from the device-native rate to 16 kHz through `FrameResampler`
   (rubato, fixed chunking). Drift correction (§5) modulates this resampler's
   effective ratio; it never blocks or back-pressures the capture callback thread.
3. **Convert to PCM16**: `f32` samples clamped to `[-1.0, 1.0]`, scaled by 32767,
   rounded to nearest, stored little-endian. All persisted capture audio is
   PCM16 @ 16 kHz mono (plan §7 Persistenzformat, §12 storage budget).

## 3. Chunk lifecycle state machine

Chunk states correspond one-to-one to `meeting_audio_chunks.state`
(plan §9): `writing`, `ready`, `damaged`, `deleted`.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkState {
    /// File open as `<seq>-<tag>.wav.part`, header sizes provisional.
    Writing,
    /// Header finalized, fsynced, atomically renamed, checksum recorded.
    Ready,
    /// Unusable (truncated beyond recovery, unparseable, checksum mismatch).
    /// Kept for forensics; excluded from STT job creation.
    Damaged,
    /// Content removed by retention/user deletion; row retained with state set.
    Deleted,
}
```

```rust
pub type ChunkId = i64;

#[derive(Clone, Debug)]
pub struct ChunkDescriptor {
    pub chunk_id: ChunkId,
    pub source: SourceKind,
    /// Monotonic per-session sequence number, formatted `%06d` in filenames.
    /// Independent per source: mic chunk 000007 may outlive sys chunk 000006.
    pub sequence: u32,
    /// Source segment (§6) this chunk belongs to.
    pub source_segment_id: i64,
    pub state: ChunkState,
    pub range: TimelineRange,
    /// Timeline samples (16 kHz mono frames) contained in the finalized file.
    pub sample_count: u64,
    /// Storage key relative to the meeting directory; never an absolute path.
    pub storage_key: String,
    /// SHA-256 over the finalized file bytes; `None` while `Writing`/`Damaged`.
    pub checksum_sha256: Option<String>,
    /// Set when the chunk was produced by crash recovery (§8).
    pub recovered: bool,
}
```

### 3.1 Transitions

```text
                 create temp file + DB row (tx A)
   (none) ─────────────────────────────────────────► writing
                                                      │
                     rollover | pause | stop |        │  finalize (tx B):
                     source close                     │  patch WAV header sizes,
                                                      │  fsync file,
                                                      │  atomic rename .part → .wav,
                                                      │  fsync directory,
                                                      │  checksum + DB update (tx C)
                                                      ├───────────────► ready
                                                      │
                    unparseable/truncated beyond       │
                    repair (write error, recovery)     │
                                                      ├───────────────► damaged
                                                      │
                    retention expiry / user delete      │
                    (content unlink + DB tx)           └───────────────► deleted

    ready ──► deleted          (allowed)
    damaged ──► deleted        (allowed, forensics expired)
    ready ──► damaged          (allowed only on read-time verification failure)
    anything ──► writing       (never; sequences are never reused)
```

Normative properties:

1. **Atomicity boundary.** The rename `.wav.part → .wav` is the single atomic
   commit point of a chunk. A process that crashes at any earlier point leaves at
   most one `writing` chunk per source; a process that crashes after the rename
   leaves a fully valid `ready` chunk even if the DB transaction had not yet run
   (§8 reconciles filesystem vs. database).
2. **Order inside finalize:** patch header → flush/fsync file → rename → fsync
   containing directory → compute checksum → DB transition `writing→ready` in one
   transaction. If the DB step fails, the recorder retries once; on repeated
   failure it raises `RecorderError::ChunkStatePersistFailed` and keeps the file
   (the recovery scan will reconcile it).
3. **Rollover.** Target duration `CHUNK_ROLLOVER_TARGET_MS` (§5.1), measured in
   timeline samples, not wall time, so gaps do not produce short junk chunks.
   Rollover finalizes the current chunk (state `ready`) and immediately opens the
   next sequence number as `writing`. Rollover is per source and asynchronous
   between sources; nothing requires cross-source alignment. If opening the next
   temp file fails (e.g. disk full), the recorder does **not** lose the current
   chunk: it first finalizes it, then enters the controlled-stop path (§7.4).
4. **STT consumption rule.** Only chunks in state `ready` may be referenced by
   `meeting_stt_jobs`. Consumers observe readiness exclusively via the
   `RecorderEvent::ChunkFinalized` event or the store — never by watching files.
5. **Deletion** unlinks content first, then commits the DB row to `deleted`; a
   crash between the two leaves an orphan file that the recovery scan removes.

## 4. Recorder interface sketch (Rust)

The following is the full public surface, in compilable shape (verified in
Appendix B against rustc 1.98). Implementation notes that are not part of the
signature contract are doc comments.

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Session layout
// ---------------------------------------------------------------------------

/// Filesystem layout owned by the recorder for one session
/// (plan §7 "Persistenzformat"). Created idempotently at session start.
#[derive(Clone, Debug)]
pub struct SessionLayout {
    pub session_id: String,
    /// `meetings/<meeting-id>/` resolved via the portable app-data logic.
    pub root: PathBuf,
}

impl SessionLayout {
    pub fn audio_dir(&self) -> PathBuf {
        self.root.join("audio")
    }
    pub fn recovery_dir(&self) -> PathBuf {
        self.root.join("recovery")
    }
}

// ---------------------------------------------------------------------------
// Device requests and preflight
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SourceOpenRequest {
    /// cpal device name, as returned by the existing device enumeration.
    pub device_name: String,
    /// For `SourceKind::System`: bind to the OS default output at open time and
    /// re-evaluate on default-device change events (§7.2). For the microphone
    /// this flag is always `false`: the selected microphone stays binding.
    pub follow_default_device: bool,
    /// Rollover target; defaults to `CHUNK_ROLLOVER_TARGET_MS`.
    pub rollover_target: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct SourceDeviceSet {
    /// `None` = record without this source (e.g. user chose "Nur Mikrofon").
    pub microphone: Option<SourceOpenRequest>,
    pub system: Option<SourceOpenRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightOutcome {
    Available,
    Missing,
    PermissionDenied,
    /// Render device cannot be opened as loopback input in shared mode
    /// (exclusive-mode WASAPI conflict surfaced by cpal).
    SharedModeRequired,
}

#[derive(Clone, Debug)]
pub struct PreflightCheck {
    pub source: SourceKind,
    pub device_name: String,
    pub outcome: PreflightOutcome,
    pub native_sample_rate_hint: Option<u32>,
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapClass {
    /// <= GAP_CLASS_NORMAL_MAX_MS (spike harness class `normal`)
    Normal,
    /// <= GAP_CLASS_SPARSE_MAX_MS (spike harness class `sparse`)
    Sparse,
    /// >  GAP_CLASS_SPARSE_MAX_MS (spike harness class `gap`)
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    /// Capturing and persisting.
    Active,
    /// Capturing with degraded quality (gaps/drift above warning thresholds).
    Degraded,
    /// Device lost; no capture. Other source unaffected.
    Lost,
    /// Deliberately closed (mic-only fallback, controlled stop).
    Closed,
}

#[derive(Clone, Debug)]
pub struct FinalizedChunkInfo {
    pub chunk_id: ChunkId,
    pub source: SourceKind,
    pub sequence: u32,
    pub range: TimelineRange,
    pub sample_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Requested,
    DiskFull,
    FatalStreamError,
}

/// Event channel from recorder to MeetingManager (and onward to Tauri).
/// Emitted from recorder threads; implementations must be non-blocking and must
/// not panic. Ordering guarantees: events from one source are ordered; events
/// across sources are not globally ordered.
#[derive(Clone, Debug)]
pub enum RecorderEvent {
    Started { at_timeline_ms: f64 },
    Paused { marker: TimelineMarker },
    Resumed { marker: TimelineMarker },
    /// Throttled to LEVEL_EVENT_INTERVAL_MS per source; peak/rms of the
    /// normalized mono signal, same scale as the dictation level meter.
    LevelUpdate { source: SourceKind, peak: f32, rms: f32 },
    SourceStateChanged { source: SourceKind, state: SourceState },
    SourceSegmentStarted {
        source: SourceKind,
        segment_id: i64,
        device_name: String,
        at_timeline_ms: f64,
    },
    ChunkFinalized { info: FinalizedChunkInfo },
    /// A detected input gap was filled with silence. `range` is the filled span.
    TimelineGap {
        source: SourceKind,
        range: TimelineRange,
        class: GapClass,
    },
    /// Slow clock drift was compensated within tolerance.
    DriftCorrectionApplied { source: SourceKind, ratio_ppm: f64 },
    /// Drift left the correction envelope; audio continues uncorrected.
    DriftToleranceExceeded {
        source: SourceKind,
        measured_drift_frames: i64,
        expected_frames: i64,
        delivered_frames: i64,
    },
    // -- Fehlerverhalten events (§7 of this document) --
    MicDeviceLost { segment_id: i64, at_timeline_ms: f64, detail: String },
    SystemDeviceLost { segment_id: i64, at_timeline_ms: f64, detail: String },
    DefaultDeviceChanged { observed_default: String },
    /// Stream resumed after system sleep/wake; lists silence-filled spans
    /// caused by the suspension.
    SleepWake {
        gap_ranges: Vec<(SourceKind, TimelineRange)>,
    },
    DiskSpaceLow { free_bytes: u64 },
    ControlledStopInitiated { reason: StopReason },
    Stopped { finalized_chunks: usize },
}

/// Consumer-side sink. MeetingManager installs one sink; it fans out to Tauri
/// events, UI state and logging.
pub trait RecorderEventSink: Send + Sync {
    fn on_event(&self, event: &RecorderEvent);
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum RecorderError {
    /// Startvalidierung / open failure, microphone side.
    MicDeviceUnavailable { device_name: String, detail: String },
    /// Startvalidierung / open failure, output/loopback side.
    SystemOutputUnavailable { device_name: String, detail: String },
    /// Exclusive-mode WASAPI conflict: loopback needs WASAPI shared mode
    /// (plan §7 "Loopback"). Distinct variant so the UI can explain the fix.
    LoopbackSharedModeRequired { device_name: String, detail: String },
    /// OS-level capture permission missing (microphone privacy setting).
    CapturePermissionDenied { source: SourceKind, detail: String },
    /// Loopback cannot be provided at all on this machine/configuration;
    /// enables the explicit "Nur Mikrofon aufnehmen" fallback.
    LoopbackUnavailable { detail: String },
    /// Startvalidierung: free space below the required minimum.
    InsufficientDiskSpace { required_bytes: u64, free_bytes: u64 },
    /// A new chunk file could not be created during a running session.
    ChunkCreateFailed { source: SourceKind, detail: String },
    /// Chunk state could not be persisted after finalize (retried once).
    ChunkStatePersistFailed { chunk_id: ChunkId, detail: String },
    /// Live stream error surfaced mid-session (device removal, driver error).
    StreamError { source: SourceKind, detail: String },
    SessionAlreadyActive,
    NoActiveSession,
    RecoveryFailed { chunk_id: ChunkId, detail: String },
}

impl std::fmt::Display for RecorderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecorderError::MicDeviceUnavailable { device_name, .. } => {
                write!(f, "microphone device unavailable: {device_name}")
            }
            RecorderError::SystemOutputUnavailable { device_name, .. } => {
                write!(f, "system output device unavailable: {device_name}")
            }
            RecorderError::LoopbackSharedModeRequired { device_name, .. } => {
                write!(
                    f,
                    "loopback requires WASAPI shared mode on device: {device_name}"
                )
            }
            RecorderError::CapturePermissionDenied { source, .. } => {
                write!(f, "capture permission denied for {source:?}")
            }
            RecorderError::LoopbackUnavailable { detail } => {
                write!(f, "loopback unavailable: {detail}")
            }
            RecorderError::InsufficientDiskSpace {
                required_bytes,
                free_bytes,
            } => write!(
                f,
                "insufficient disk space: need {required_bytes}, have {free_bytes}"
            ),
            RecorderError::ChunkCreateFailed { source, .. } => {
                write!(f, "failed to create chunk for {source:?}")
            }
            RecorderError::ChunkStatePersistFailed { chunk_id, .. } => {
                write!(f, "failed to persist state for chunk {chunk_id}")
            }
            RecorderError::StreamError { source, .. } => {
                write!(f, "stream error on {source:?}")
            }
            RecorderError::SessionAlreadyActive => write!(f, "session already active"),
            RecorderError::NoActiveSession => write!(f, "no active session"),
            RecorderError::RecoveryFailed { chunk_id, .. } => {
                write!(f, "recovery failed for chunk {chunk_id}")
            }
        }
    }
}

impl std::error::Error for RecorderError {}

// ---------------------------------------------------------------------------
// Persistence seam (implemented by MeetingStore)
// ---------------------------------------------------------------------------

/// Patch applied together with a chunk state transition. Fields left as `None`
/// are not modified.
#[derive(Clone, Debug, Default)]
pub struct ChunkPatch {
    pub storage_key: Option<String>,
    pub range: Option<TimelineRange>,
    pub sample_count: Option<u64>,
    pub checksum_sha256: Option<String>,
    pub recovered: Option<bool>,
    /// Gap metadata: (count, total_filled_samples) accumulated in this chunk.
    pub gap_stats: Option<(u32, u64)>,
}

/// Minimal read/write seam over `meeting_audio_chunks`. Implemented by
/// `MeetingStore`; all methods are transactional on the implementor side.
pub trait ChunkStateStore: Send + Sync {
    fn open_chunks(&self, session_id: &str) -> Result<Vec<ChunkDescriptor>, String>;
    fn transition(
        &self,
        chunk_id: ChunkId,
        from: ChunkState,
        to: ChunkState,
        patch: &ChunkPatch,
    ) -> Result<(), String>;
    fn insert_writing_chunk(
        &self,
        session_id: &str,
        descriptor: &ChunkDescriptor,
    ) -> Result<ChunkId, String>;
}

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderState {
    Idle,
    Recording,
    Paused,
    Stopping,
    Recovering,
}

#[derive(Clone, Debug)]
pub struct SourceStatus {
    pub source: SourceKind,
    pub state: SourceState,
    pub device_name: Option<String>,
    pub current_chunk_sequence: Option<u32>,
    pub backlog_chunks_ready: usize,
}

#[derive(Clone, Debug)]
pub struct RecorderSnapshot {
    pub state: RecorderState,
    pub timeline_now_ms: f64,
    pub sources: Vec<SourceStatus>,
    /// Marker pairs of the session, oldest first (for the running-view UI).
    pub markers: Vec<TimelineMarker>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionSummary {
    pub finalized_chunks: Vec<FinalizedChunkInfo>,
    pub degraded_ranges: Vec<(SourceKind, TimelineRange, GapClass)>,
    pub stopped_reason: Option<StopReason>,
}

#[derive(Clone, Debug, Default)]
pub struct RecoveryReport {
    /// Open chunks repaired to state `ready`.
    pub finalized: Vec<ChunkId>,
    /// Open chunks moved to state `damaged`.
    pub damaged: Vec<ChunkId>,
    /// Orphan `.part` files quarantined into the session `recovery/` dir.
    pub quarantined_files: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// Constants (values and evidence: section 5.1)
// ---------------------------------------------------------------------------

/// Rollover target per chunk, plan §7 "Persistenzformat". Testable constant;
/// Phase 1/2 may revise it based on measurement. Not spike-derived.
pub const CHUNK_ROLLOVER_TARGET_MS: u64 = 60_000;

/// Level-event throttle. UI constant, not spike-derived.
pub const LEVEL_EVENT_INTERVAL_MS: u64 = 100;

/// Spike-harness gap-class boundaries, identical to
/// `docs/phase1-spike/tools/analyze-spike-csv.ps1` (column `gap_class`).
pub const GAP_CLASS_NORMAL_MAX_MS: f64 = 250.0;
pub const GAP_CLASS_SPARSE_MAX_MS: f64 = 1000.0;

/// Silence classification threshold carried over from the spike runs
/// (`# silence_threshold=0.0001` run metadata in
/// `docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv`). Cross-device
/// confirmation pending task-0008 measurement (columns `peak`, `rms`, `silent`).
pub const SILENCE_PEAK_THRESHOLD: f32 = 0.0001;

// ---------------------------------------------------------------------------
// The MeetingRecorder trait
// ---------------------------------------------------------------------------

/// Dual-source meeting capture. One instance per active session.
///
/// Threading model: `open_source` spawns the capture callback plus a writer
/// task per source. Capture callbacks never block on disk I/O, on inference, or
/// on each other (plan §8: "Aufnahme-Threads warten niemals auf Inferenz").
pub trait MeetingRecorder: Send {
    /// Validate devices without opening them (Startvalidierung subset owned by
    /// the recorder). Provider/model/retention validation happens in
    /// MeetingManager and is deliberately out of scope here.
    fn preflight(&self, devices: &SourceDeviceSet) -> Result<Vec<PreflightCheck>, RecorderError>;

    /// Create the session layout, fix the monotonic timeline origin, and run
    /// crash recovery for this session if it has stale `writing` chunks (§8).
    fn start_session(
        &mut self,
        layout: SessionLayout,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<(), RecorderError>;

    /// Open one capture source and begin the first (or next) source segment.
    /// May be called for a second source while the first is live, for a
    /// replacement device after loss, or to add system audio to a
    /// microphone-only session (which starts a new source segment).
    fn open_source(
        &mut self,
        source: SourceKind,
        request: SourceOpenRequest,
    ) -> Result<i64, RecorderError>; // Ok(segment_id)

    /// Close exactly one source while the other keeps running
    /// (Fehlerverhalten: "Mikrofon fällt aus: Systemaudio läuft weiter").
    /// Finalizes the open chunk of that source.
    fn close_source(
        &mut self,
        source: SourceKind,
        reason: StopReason,
    ) -> Result<(), RecorderError>;

    /// Finalize both open chunks, emit a `Pause` marker, stop capturing.
    /// No audio is captured or persisted while paused.
    fn pause(&mut self) -> Result<TimelineMarker, RecorderError>;

    /// Re-validate both devices (Startvalidierung rules apply again), then
    /// resume at the frozen timeline position with a `Resume` marker. A device
    /// that changed identity starts a new source segment (§6).
    fn resume(&mut self, devices: SourceDeviceSet) -> Result<TimelineMarker, RecorderError>;

    /// Finalize all open chunks and stop both streams. Returns only after the
    /// last chunk is durably `ready` (plan §6.x "Stoppen": UI may close only
    /// after safe finalization).
    fn stop(&mut self) -> Result<SessionSummary, RecorderError>;

    /// Install the event fan-out sink. Replaces any previous sink.
    fn subscribe(&mut self, sink: Arc<dyn RecorderEventSink>);

    /// Consistent view for Hauptfenster, Overlay and Tray (plan §6.11: all
    /// three read the same backend state; there is no second frontend-local
    /// recording state).
    fn snapshot(&self) -> RecorderSnapshot;

    /// Crash-recovery protocol of §8. Idempotent; safe to call at startup for
    /// every session whose persisted state claims recording/paused.
    fn recover_open_chunks(
        &self,
        session_id: &str,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<RecoveryReport, RecorderError>;
}
```

## 5. Gap-filling and drift-correction boundaries

### 5.1 Tolerance constants

Values are **not guessed**: each constant either cites the spike harness or is
explicitly marked _pending task-0008 measurement_, naming the CSV column(s) of
`docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv` that decide it. The
spike CSV schema (header row) is:
`source, cb_index, t_rel_ms, gap_ms, frames, total_frames, sample_rate, channels,
sample_format, expected_frames, drift_frames, gap_class, silent, peak, rms, error`.

| Constant                           | Provisional value | Status                                                                                                                                              | Decided by (spike column / source)                       |
| ---------------------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| `TIMELINE_SAMPLE_RATE_HZ`          | 16 000            | fixed by plan §7                                                                                                                                    | —                                                        |
| `TIMELINE_CHANNELS`                | 1                 | fixed by plan §7                                                                                                                                    | —                                                        |
| `CHUNK_ROLLOVER_TARGET_MS`         | 60 000            | initial value fixed by plan §7 ("Phase 1 prüft, ob ein anderer Wert messbar besser ist"); revisitable after Phase 1                                 | —                                                        |
| `LEVEL_EVENT_INTERVAL_MS`          | 100               | UI constant                                                                                                                                         | —                                                        |
| `GAP_CLASS_NORMAL_MAX_MS`          | 250.0             | **fixed by spike harness** (class boundary implemented in `analyze-spike-csv.ps1`, column `gap_class`)                                              | `gap_class`                                              |
| `GAP_CLASS_SPARSE_MAX_MS`          | 1000.0            | **fixed by spike harness** (same)                                                                                                                   | `gap_class`                                              |
| `SILENCE_PEAK_THRESHOLD`           | 0.0001            | taken from spike run metadata `# silence_threshold`; cross-device confirmation pending task-0008 measurement                                        | `peak`, `rms`, `silent`, meta line `# silence_threshold` |
| `GAP_EVENT_MIN_MS`                 | —                 | **pending task-0008 measurement**: smallest gap worth surfacing as `TimelineGap` to the UI                                                          | `gap_ms`, `gap_class`                                    |
| `SEGMENT_DEGRADED_GAP_MS`          | —                 | **pending task-0008 measurement**: gap above which a source segment is flagged `Degraded`                                                           | `gap_ms`, `gap_class` (`gap` rows)                       |
| `CALLBACK_JITTER_NORMAL_MAX_MS`    | —                 | **pending task-0008 measurement**: upper bound of `gap_ms` among `normal`-class callbacks; below this no gap bookkeeping is emitted                 | `gap_ms` filtered by `gap_class='normal'`                |
| `INITIAL_OFFSET_FILL_TOLERANCE_MS` | —                 | **pending task-0008 measurement**: max leading offset between the two first callbacks that is silently filled; above it a startup warning is raised | `t_rel_ms` of `cb_index=1` rows per `source`             |
| `DRIFT_CORRECTION_MAX_PPM`         | —                 | **pending task-0008 measurement**: maximum resampler-ratio offset applied for slow-clock compensation                                               | `drift_frames`, `t_rel_ms` (trend slope)                 |
| `DRIFT_WARN_FRAMES`                | —                 | **pending task-0008 measurement**: drift magnitude that flips a source to `Degraded`                                                                | `drift_frames`                                           |
| `DRIFT_EXCEEDED_FRAMES`            | —                 | **pending task-0008 measurement**: drift magnitude that emits `DriftToleranceExceeded`                                                              | `drift_frames`, `expected_frames`, `total_frames`        |
| `DISK_LOW_WATERMARK_BYTES`         | —                 | product constant, not spike-derived; proposal: ≥ 2 × hourly two-track budget (≈ 230 MB/h, plan §12); pending product decision                       | —                                                        |

Until task-0008 lands, the recorder implementation must compile these constants
from a single `tolerances.rs` module carrying the same table so that replacing
pending values later is a one-file change plus test updates.

### 5.2 Gap-filling algorithm boundaries

1. **Detection.** On each callback of a source, compare the callback timestamp
   against the delivery schedule implied by previously delivered frames
   (`expected_frames` vs. `total_frames`, exactly the spike computation).
   A gap exists when the inter-callback delta exceeds
   `CALLBACK_JITTER_NORMAL_MAX_MS` _(pending task-0008)_.
2. **Classification.** Gaps are classified with the spike harness classes
   (`Normal`/`Sparse`/`Long` via `GAP_CLASS_*_MAX_MS`), so recorder telemetry and
   spike analysis remain directly comparable.
3. **Representation.** Every gap — regardless of size — is inserted into the
   16 kHz track as digital silence covering exactly the missing timeline range.
   The track length always equals elapsed timeline time; the invariant
   `next_write_position == last_written_position + silence_len` holds at every
   chunk write. Tracks are never shortened and audio is never re-timestamped.
4. **Reporting.** Gaps ≥ `GAP_EVENT_MIN_MS` _(pending task-0008)_ emit
   `RecorderEvent::TimelineGap`. Gaps ≥ `SEGMENT_DEGRADED_GAP_MS` _(pending
   task-0008)_ flip the source to `SourceState::Degraded` and are recorded in the
   chunk's `gap_stats` and in `SessionSummary::degraded_ranges`.
5. **Exclusions.** Paused intervals are excluded by construction (timeline is
   frozen across pause). A device-loss interval is _not_ gap-filled indefinitely:
   `close_source` ends the segment, and the hole between segments remains as
   timeline silence with segment-boundary metadata instead of a growing gap.
6. **Cross-source independence.** A gap in one source never inserts silence into
   the other source's track and never delays its writer.

### 5.3 Drift-correction algorithm boundaries

1. **Measurement.** Per source, `drift = delivered_frames − expected_frames`,
   computed identically to the spike `drift_frames` column, evaluated over the
   stable window (>10 s runtime) used by `analyze-spike-csv.ps1`.
2. **Correction mechanism.** The per-source resampler ratio is modulated by
   `ratio = 1 + ppm/1e6`, with `|ppm| ≤ DRIFT_CORRECTION_MAX_PPM` _(pending
   task-0008)_, steering cumulative drift toward zero slowly. Correction acts on
   the writer path only; capture callbacks are never blocked, resized, or
   dropped for drift reasons.
3. **Deadband and hysteresis.** Correction applies only outside a deadband
   proportional to measurement noise observed in the spike data _(deadband width
   pending task-0008, derived from the `drift_frames` scatter of `normal`
   callbacks)_, and ratio changes are rate-limited so the resampler is retuned at
   most once per second per source.
4. **Escape hatch.** When |drift| exceeds `DRIFT_EXCEEDED_FRAMES` _(pending
   task-0008)_ the recorder stops correcting, continues capturing uncorrected,
   emits `DriftToleranceExceeded`, and marks the source `Degraded`. It never
   drops samples or truncates the track to "fix" drift.
5. **Startup offset.** Whichever source opens first defines timeline position of
   the other's first frame; the difference up to
   `INITIAL_OFFSET_FILL_TOLERANCE_MS` _(pending task-0008)_ is pre-filled with
   leading silence (plan task 1 of "Gemeinsame Zeitachse"). Beyond that, a
   startup warning event is raised but capture proceeds.

## 6. Pause markers and source segments

### Source segment

A **source segment** is a maximal span `(source, device_identity, [from_ms, to_ms))`
in which one unchanged physical device fed one source. Segments begin at:

- first `open_source` after session start or resume;
- re-open after device loss with the same or a different device;
- resume where re-validation found a different device than the previous segment
  (plan §6.x "Pause": "ein Gerätewechsel wird als neuer Quellenabschnitt
  dokumentiert");
- `DefaultDeviceChanged` handled under an enabled `follow_default_device` option
  (system source only): the old segment closes cleanly and a new one begins on
  the new default device — never silently (plan Fehlerverhalten).

Segments end at pause, `close_source`, or stop. Every segment start emits
`RecorderEvent::SourceSegmentStarted`; `MeetingStore` records segment identity
(device id/name, opened-at timeline ms) and every chunk references its segment.
Transcript rendering can therefore show "ab hier: anderes Gerät" without any
speaker-diary claim.

### Pause semantics

1. `pause()` finalizes the currently open chunk of **each** active source to
   `ready` (so pause is also a durability checkpoint) and writes a single
   `TimelineMarker { kind: Pause }`.
2. During pause: no capture callbacks are processed into chunks, no levels are
   reported, the timeline position is frozen, and the recorder state is `Paused`.
3. `resume()` re-validates devices (same checks as Startvalidierung; failures
   surface as the corresponding `RecorderError` variants and keep the session in
   `Paused`), optionally starts new source segments, and writes
   `TimelineMarker { kind: Resume }` at the same timeline position as the pause
   marker. Gap-fill logic skips the closed `[pause, resume)` span entirely.
4. Markers are part of `RecorderSnapshot` so the running view can render pauses
   in the transcript gutter; they are persisted by MeetingManager.

## 7. Error behavior: named events, errors, and consumers

Every case from plan §6.x "Fehlerverhalten" and "Startvalidierung" maps to a
named variant plus a responsible consumer. "Consumer" names the component that
must react visibly.

### 7.1 Laufende Aufnahme (Fehlerverhalten)

| Plan case                     | Recorder surface                                                                                                                                                                                                        | Consumer & required reaction                                                                                                                                                                     |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Mikrofon fällt aus            | `RecorderEvent::MicDeviceLost`, then `SourceStateChanged { Microphone, Lost }`; open mic chunk finalized to `ready` first                                                                                               | `MeetingManager` keeps system source running; `MeetingRecorderView` shows orange warning with actions _Wiederverbinden_ / _Gerät wechseln_; Tray/Overlay reflect degraded state via `snapshot()` |
| Systemgerät fällt aus         | `RecorderEvent::SystemDeviceLost`, `SourceStateChanged { System, Lost }`                                                                                                                                                | Same as above mirrored; UI offers _Loopback neu öffnen_ or _Ohne Systemaudio fortführen_; choosing the latter calls `close_source(System, …)` and sets `has_system_audio=false` from that point  |
| Standardgerät wechselt        | `RecorderEvent::DefaultDeviceChanged` always; plus `SourceSegmentStarted` **only if** follow-default option enabled                                                                                                     | `MeetingSettings` owns the option; `MeetingRecorderView` shows a non-blocking notice either way; binding mode (option off) changes nothing about capture                                         |
| Keine Systemlautstärke        | **No error.** Silence is legitimate: `LevelUpdate { peak ≈ 0 }` only; `SILENCE_PEAK_THRESHOLD` classifies callbacks as silent for telemetry                                                                             | Setup test flow (`MeetingPreflight` "Systemaudio testen") distinguishes wrong-device vs. truly-silent, per plan                                                                                  |
| Datenträger fast voll         | `RecorderEvent::DiskSpaceLow` at watermark; if the next chunk temp file cannot be created safely: finalize current chunk(s), emit `ControlledStopInitiated { reason: DiskFull }`, then `stop()` semantics automatically | `MeetingRecorderView` warns early; on controlled stop shows green "Audio gesichert" plus explanation; no data loss (all finalized chunks stay usable)                                            |
| Azure/Custom nicht erreichbar | **Outside recorder scope** — capture continues untouched. Boundary: recorder never blocks on network; queue sets jobs to `retry_wait`                                                                                   | `MeetingProcessingView` checklist + retry controls; `MeetingRecorderView` shows growing backlog count from `snapshot().sources[*].backlog_chunks_ready`                                          |
| Lokales Modell zu langsam     | **Outside recorder scope** — backlog grows on disk only, RAM bounded by chunk rotation                                                                                                                                  | `MeetingRecorderView` "Verarbeitung liegt zurück" state from backlog count                                                                                                                       |
| App-Absturz                   | Recovery protocol §8; results consumed at next startup                                                                                                                                                                  | `MeetingRecoveryBanner` offers processing of recovered chunks; session never stays stuck in `recording`                                                                                          |
| Sleep/wake (spike scenario)   | `RecorderEvent::SleepWake { gap_ranges }` with silence-filled spans; sources re-validated, drift counters reset at wake                                                                                                 | `MeetingRecorderView` brief notice; gaps visible in transcript as filled ranges                                                                                                                  |

### 7.2 Startvalidierung

| Plan case                                                                                       | Named variant                                                                                                                     | Consumer                                                                                                        |
| ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Mikrofon nicht verfügbar/öffnbar                                                                | `preflight` check `Missing` or `RecorderError::MicDeviceUnavailable`                                                              | Start-dialog status row "Mikrofon" + focused repair dialog (device picker)                                      |
| Systemausgabegerät nicht verfügbar                                                              | `preflight` check `Missing` or `RecorderError::SystemOutputUnavailable`                                                           | Status row "Systemaudio"; repair dialog                                                                         |
| Loopback exclusive-mode conflict                                                                | `RecorderError::LoopbackSharedModeRequired`                                                                                       | Repair dialog explains closing exclusive-mode apps / switching device                                           |
| Loopback grundsätzlich nicht möglich                                                            | `RecorderError::LoopbackUnavailable`                                                                                              | Start dialog offers explicit fallback _"Nur Mikrofon aufnehmen"_; choice stored visibly in the session          |
| Berechtigungen fehlen                                                                           | `preflight` check `PermissionDenied` / `RecorderError::CapturePermissionDenied`                                                   | OS-permission repair instructions in dialog                                                                     |
| Freier Speicherplatz zu gering                                                                  | `RecorderError::InsufficientDiskSpace` (checked by MeetingManager using the §12 budget; recorder enforces nothing extra at start) | Start button disabled + storage hint with per-hour estimate                                                     |
| STT-Provider/Sprache, Modell, Azure-/Custom-Konfiguration, Summary-Provider, Aufbewahrungsregel | **Not recorder variants** (validated by `MeetingManager` against settings/providers)                                              | Focused repair dialogs per plan §6.x Startvalidierung; recorder receives an already-validated `SourceDeviceSet` |

Rule (G2): every recorder-visible failure has exactly one named variant, and
every variant appears in exactly one consumer row above. Variants without a
consumer are forbidden; consumer rows without a variant must reference their
non-recorder owner explicitly (as done for provider/model/retention).

## 8. Open-chunk crash-recovery protocol

Runs at `start_session` (and standalone via `recover_open_chunks`) for any session
whose persisted state is `starting`/`recording`/`paused` while no live process
owns it (process-liveness is established by MeetingManager, e.g. pid/lock file;
out of recorder scope).

Protocol, in order:

1. **Enumerate from the database.** Read all chunk rows in state `writing` for
   the session (`ChunkStateStore::open_chunks`). The DB row list is authoritative
   for _what was attempted_; the filesystem is authoritative for _what survived_.
2. **For each `writing` row**, locate `<seq>-<tag>.wav.part` in `audio/`:
   a. **File present and parseable:** read the RIFF/WAV structure; take
   `min(declared_data_size, actual_file_bytes_after_header)` truncated down to
   a whole PCM16 frame (even byte count); patch the header sizes to the
   truncated length; fsync; **atomic rename** to the final `.wav` name; fsync
   directory; compute checksum; transition `writing → ready` with
   `recovered=true`, `sample_count = truncated_bytes / 2`,
   `timeline_end_ms = timeline_start_ms + sample_count / 16000 * 1000`. The
   resulting silence-to-next-chunk continuation is handled by ordinary gap
   bookkeeping, because the crash simply ends the captured range early.
   b. **File present but unparseable/headerless/too short (<44 bytes or no valid
   RIFF):** transition `writing → damaged`, move the file to `recovery/`
   keeping its name for forensics.
   c. **Row exists, file missing:** transition `writing → damaged` (empty
   `storage_key` note); nothing else existed to lose.
3. **Reverse reconciliation.** Scan `audio/` for `.wav` files without a `ready`
   row (crash between rename and DB commit): verify parseability + checksum-less
   integrity (frame alignment), then insert/update the row to `ready` with
   `recovered=true`. Orphan `.part` files with no row at all are quarantined into
   `recovery/`.
4. **Session disposition.** After recovery the recorder reports
   `RecoveryReport`; MeetingManager moves the session out of `recording` into the
   post-stop flow (`stopped_unprocessed` / `recoverable_error` per plan §6.x
   Session-Zustände), so recovered chunks flow into normal STT jobs and the
   `MeetingRecoveryBanner` explains what happened. A session is never left in
   `recording` after a crashed process (hard requirement, plan §6.x).
5. **Idempotence.** Steps 2–3 are safe to repeat: `ready` chunks are skipped,
   renames of already-renamed files are no-ops, and `damaged` rows are stable.

Guarantee (matches plan §7 "Persistenzformat"): a crash can damage at most the
currently open chunk per source; all previously renamed chunks remain playable
and transcribable.

## 9. Verification hooks (maps to plan §14 automated tests)

- Timeline/gap/drift math must be pure functions over callback records so tests
  replay spike CSVs directly (columns `t_rel_ms`, `gap_ms`, `frames`,
  `total_frames`, `drift_frames`) and assert identical classification to
  `analyze-spike-csv.ps1`.
- Rollover/finalize tests assert: no `.part` remains after clean stop, header
  fields match actual sizes, rename is atomic under a fault-injection seam
  (filesystem operations behind a small internal trait for testing).
- Recovery tests cover §8 cases 2a/2b/2c, orphan `.wav`, orphan `.part`,
  double-run idempotence.
- Failure-injection tests: kill writer between header-patch/rename/db-commit and
  confirm each intermediate state recovers per §8.

---

## Appendix A — Gate traceability

- **G1** (trait signatures): §4 contains the complete `MeetingRecorder` trait and
  all supporting event/error/state types; compilability verified in Appendix B.
- **G2** (every plan-§7-Fehlerverhalten and Startvalidierung case named +
  consumer): tables §7.1 and §7.2 cover all eight Fehlerverhalten bullets and all
  seven Startvalidierung bullets.
- **G3** (state machine, atomic rename, interrupted-open recovery): §3.1
  transitions + normative properties; §8 recovery protocol steps 2a–c.
- **G4** (tolerance constants cite spike values or carry pending-task-0008 marker
  with CSV source column): table §5.1, including exact CSV column names.
- **G5** (git status only under `docs/meeting-mode-architecture/`): verified after
  authoring; see mission result evidence.

## Appendix B — Compilability verification of the Rust sketch

All Rust code blocks of §2–§4 concatenated compile as a library crate with
`rustc 1.98.0` and no external dependencies. The verification command and result
are recorded in the mission report; re-run with:

```powershell
pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1
```

_(helper script extracts fenced ```rust blocks from this file, wraps them in
`#![allow(dead_code)]`and runs`rustc --edition 2021 --crate-type lib`.)_
