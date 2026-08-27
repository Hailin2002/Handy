//! Dual-source (microphone + Windows system audio) crash-safe meeting capture.
//!
//! Implements the contract in
//! `docs/meeting-mode-architecture/recorder-contract.md` and plan §7
//! ("Windows-Audioaufnahme"). Tolerance constants and their measured
//! justification live in [`super::meeting_tolerances`].
//!
//! # Why this is not the dictation recorder
//!
//! [`super::AudioRecorder`] accumulates the whole take in a `Vec<f32>` and
//! returns it at `stop()`. That is correct for a 10-second dictation and fatal
//! for a two-hour meeting. This module never holds more than a bounded working
//! set: capture blocks are handed to a per-source writer thread through a
//! queue capped at [`tolerances::MAX_QUEUED_CAPTURE_BLOCKS`], converted to PCM16
//! through a fixed-size scratch buffer, and streamed straight into a rotating
//! chunk file. Recorder memory is therefore a function of the queue bound and
//! the chunk buffer, not of meeting duration.
//!
//! # Structure
//!
//! - **Pure timeline math** ([`plan_block`], [`DriftTracker`], [`DriftController`])
//!   — no I/O, no threads, directly replayable from spike CSV columns.
//! - **Chunk I/O** ([`ChunkWriter`], [`ChunkFs`]) — streaming WAV writer with a
//!   `.wav.part` staging name and an atomic rename as the single commit point.
//! - **Per-source pipeline** ([`SourceWriter`]) — gap fill, resampling, drift
//!   correction, rollover, event emission. Runs on its own thread per source.
//! - **Capture seam** ([`CaptureBackend`]) — [`CpalCaptureBackend`] in
//!   production, fakes in tests, so all logic above is testable without audio
//!   hardware.
//! - **Session orchestration** ([`MeetingRecorderImpl`]).

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

use super::meeting_tolerances as tolerances;
use super::FrameResampler;
use tolerances::{TIMELINE_CHANNELS, TIMELINE_SAMPLES_PER_MS, TIMELINE_SAMPLE_RATE_HZ};

// ===========================================================================
// 1. Core types (contract §2)
// ===========================================================================

/// Physical capture source. Persisted verbatim as `meeting_audio_chunks.source`.
/// This is technical provenance information, never speaker identification.
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

    /// All sources, in a stable order for deterministic iteration.
    pub fn all() -> [SourceKind; 2] {
        [SourceKind::Microphone, SourceKind::System]
    }
}

/// A half-open range on the shared meeting timeline, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRange {
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One pause/resume boundary recorded on the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub wall_clock: SystemTime,
}

/// Chunk states, one-to-one with `meeting_audio_chunks.state` (plan §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkState {
    /// File open as `<seq>-<tag>.wav.part`, header sizes provisional.
    Writing,
    /// Header finalized, fsynced, atomically renamed, checksum recorded.
    Ready,
    /// Unusable (truncated beyond recovery, unparseable, checksum mismatch).
    Damaged,
    /// Content removed by retention/user deletion; row retained.
    Deleted,
}

pub type ChunkId = i64;

#[derive(Clone, Debug)]
pub struct ChunkDescriptor {
    pub chunk_id: ChunkId,
    pub source: SourceKind,
    /// Monotonic per-session sequence number, formatted `%06d` in filenames.
    /// Independent per source.
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

/// Filesystem layout owned by the recorder for one session (plan §7).
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

#[derive(Clone, Debug)]
pub struct SourceOpenRequest {
    /// cpal device name, as returned by the existing device enumeration.
    pub device_name: String,
    /// For [`SourceKind::System`]: bind to the OS default output at open time
    /// and re-evaluate on default-device change events. Always `false` for the
    /// microphone.
    pub follow_default_device: bool,
    /// Rollover target; defaults to [`tolerances::CHUNK_ROLLOVER_TARGET_MS`].
    pub rollover_target: Duration,
}

impl SourceOpenRequest {
    /// Request with the contract's default rollover target.
    pub fn new(device_name: impl Into<String>) -> Self {
        Self {
            device_name: device_name.into(),
            follow_default_device: false,
            rollover_target: Duration::from_millis(tolerances::CHUNK_ROLLOVER_TARGET_MS),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SourceDeviceSet {
    /// `None` = record without this source (e.g. user chose "Nur Mikrofon").
    pub microphone: Option<SourceOpenRequest>,
    pub system: Option<SourceOpenRequest>,
}

impl SourceDeviceSet {
    fn get(&self, source: SourceKind) -> Option<&SourceOpenRequest> {
        match source {
            SourceKind::Microphone => self.microphone.as_ref(),
            SourceKind::System => self.system.as_ref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightOutcome {
    Available,
    Missing,
    PermissionDenied,
    /// Render device cannot be opened as loopback input in shared mode.
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

/// Gap classification, identical to the spike harness `gap_class` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapClass {
    /// <= [`tolerances::GAP_CLASS_NORMAL_MAX_MS`]
    Normal,
    /// <= [`tolerances::GAP_CLASS_SPARSE_MAX_MS`]
    Sparse,
    /// > [`tolerances::GAP_CLASS_SPARSE_MAX_MS`]
    Long,
}

impl GapClass {
    /// Spike-harness spelling, so recorder telemetry can be diffed against
    /// `analyze-spike-csv.ps1` output directly.
    pub fn as_str(self) -> &'static str {
        match self {
            GapClass::Normal => "normal",
            GapClass::Sparse => "sparse",
            GapClass::Long => "gap",
        }
    }
}

/// Classify a gap exactly like the spike harness does.
pub fn classify_gap(gap_ms: f64) -> GapClass {
    if gap_ms <= tolerances::GAP_CLASS_NORMAL_MAX_MS {
        GapClass::Normal
    } else if gap_ms <= tolerances::GAP_CLASS_SPARSE_MAX_MS {
        GapClass::Sparse
    } else {
        GapClass::Long
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    Active,
    Degraded,
    Lost,
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
/// Emitted from recorder threads; sinks must be non-blocking and must not panic.
#[derive(Clone, Debug)]
pub enum RecorderEvent {
    Started {
        at_timeline_ms: f64,
    },
    Paused {
        marker: TimelineMarker,
    },
    Resumed {
        marker: TimelineMarker,
    },
    /// Throttled to [`tolerances::LEVEL_EVENT_INTERVAL_MS`] per source.
    LevelUpdate {
        source: SourceKind,
        peak: f32,
        rms: f32,
    },
    SourceStateChanged {
        source: SourceKind,
        state: SourceState,
    },
    SourceSegmentStarted {
        source: SourceKind,
        segment_id: i64,
        device_name: String,
        at_timeline_ms: f64,
    },
    ChunkFinalized {
        info: FinalizedChunkInfo,
    },
    /// A detected input gap was filled with silence. `range` is the filled span.
    TimelineGap {
        source: SourceKind,
        range: TimelineRange,
        class: GapClass,
    },
    /// Slow clock drift was compensated within tolerance.
    DriftCorrectionApplied {
        source: SourceKind,
        ratio_ppm: f64,
    },
    /// Drift left the correction envelope; audio continues uncorrected.
    DriftToleranceExceeded {
        source: SourceKind,
        measured_drift_frames: i64,
        expected_frames: i64,
        delivered_frames: i64,
    },
    /// Leading offset of the first microphone callback exceeded
    /// [`tolerances::INITIAL_OFFSET_FILL_TOLERANCE_MS`]; capture proceeds.
    StartupOffsetWarning {
        source: SourceKind,
        offset_ms: f64,
    },
    MicDeviceLost {
        segment_id: i64,
        at_timeline_ms: f64,
        detail: String,
    },
    SystemDeviceLost {
        segment_id: i64,
        at_timeline_ms: f64,
        detail: String,
    },
    DefaultDeviceChanged {
        observed_default: String,
    },
    /// Stream resumed after system sleep/wake; lists silence-filled spans.
    SleepWake {
        gap_ranges: Vec<(SourceKind, TimelineRange)>,
    },
    DiskSpaceLow {
        free_bytes: u64,
    },
    ControlledStopInitiated {
        reason: StopReason,
    },
    Stopped {
        finalized_chunks: usize,
    },
}

/// Consumer-side sink. MeetingManager installs one; it fans out to Tauri
/// events, UI state and logging.
pub trait RecorderEventSink: Send + Sync {
    fn on_event(&self, event: &RecorderEvent);
}

/// Sink that discards everything — the default until `subscribe` is called.
pub struct NullEventSink;

impl RecorderEventSink for NullEventSink {
    fn on_event(&self, _event: &RecorderEvent) {}
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecorderError {
    MicDeviceUnavailable {
        device_name: String,
        detail: String,
    },
    SystemOutputUnavailable {
        device_name: String,
        detail: String,
    },
    /// Exclusive-mode WASAPI conflict: loopback needs WASAPI shared mode.
    LoopbackSharedModeRequired {
        device_name: String,
        detail: String,
    },
    CapturePermissionDenied {
        source: SourceKind,
        detail: String,
    },
    /// Loopback cannot be provided at all on this machine/configuration.
    LoopbackUnavailable {
        detail: String,
    },
    InsufficientDiskSpace {
        required_bytes: u64,
        free_bytes: u64,
    },
    ChunkCreateFailed {
        source: SourceKind,
        detail: String,
    },
    ChunkStatePersistFailed {
        chunk_id: ChunkId,
        detail: String,
    },
    StreamError {
        source: SourceKind,
        detail: String,
    },
    SessionAlreadyActive,
    NoActiveSession,
    RecoveryFailed {
        chunk_id: ChunkId,
        detail: String,
    },
}

impl fmt::Display for RecorderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// Patch applied together with a chunk state transition. `None` fields are not
/// modified.
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

/// Minimal read/write seam over `meeting_audio_chunks`, implemented by
/// `MeetingStore`. Keeps the recorder storage-agnostic.
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
    /// Marker pairs of the session, oldest first.
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

/// Dual-source meeting capture. One instance per active session.
pub trait MeetingRecorder: Send {
    fn preflight(&self, devices: &SourceDeviceSet) -> Result<Vec<PreflightCheck>, RecorderError>;
    fn start_session(
        &mut self,
        layout: SessionLayout,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<(), RecorderError>;
    fn open_source(
        &mut self,
        source: SourceKind,
        request: SourceOpenRequest,
    ) -> Result<i64, RecorderError>;
    fn close_source(&mut self, source: SourceKind, reason: StopReason)
        -> Result<(), RecorderError>;
    fn pause(&mut self) -> Result<TimelineMarker, RecorderError>;
    fn resume(&mut self, devices: SourceDeviceSet) -> Result<TimelineMarker, RecorderError>;
    fn stop(&mut self) -> Result<SessionSummary, RecorderError>;
    fn subscribe(&mut self, sink: Arc<dyn RecorderEventSink>);
    fn snapshot(&self) -> RecorderSnapshot;
    fn recover_open_chunks(
        &self,
        session_id: &str,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<RecoveryReport, RecorderError>;
}

// ===========================================================================
// 2. Small utilities
// ===========================================================================

/// Lock a mutex, recovering the guard from poisoning instead of panicking.
///
/// The recorder must never abort a running meeting because an unrelated thread
/// panicked while holding a lock; the protected state is always either
/// append-only bookkeeping or a status snapshot, both of which stay usable.
///
/// Thin wrapper over [`crate::sync::MutexExt::lock_recover`], kept as a free
/// function because this module's call sites read `lock_recover(&self.x)`
/// throughout.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    crate::sync::MutexExt::lock_recover(m)
}

/// Timeline milliseconds → whole timeline samples (16 samples per ms, exact).
fn ms_to_samples(ms: f64) -> u64 {
    if ms <= 0.0 {
        0
    } else {
        (ms * TIMELINE_SAMPLES_PER_MS).round() as u64
    }
}

/// Timeline samples → milliseconds.
fn samples_to_ms(samples: u64) -> f64 {
    samples as f64 / TIMELINE_SAMPLES_PER_MS
}

// ===========================================================================
// 3. Pure timeline math (contract §5.2, §5.3, §9)
// ===========================================================================

/// One capture callback's worth of normalized mono audio, stamped at callback
/// entry from the session's single monotonic origin.
#[derive(Clone, Debug)]
pub struct CaptureBlock {
    /// `Instant::now()` at callback entry, minus the origin, minus accumulated
    /// pause time — i.e. already on the shared timeline.
    pub timeline_ms: f64,
    /// Downmixed mono samples at the device-native rate.
    pub mono: Vec<f32>,
    pub native_rate: u32,
}

/// Where a capture block lands on the timeline and how much silence precedes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockPlacement {
    /// Silence samples (16 kHz) to insert before the block's audio.
    pub fill_samples: u64,
    /// Detected gap in ms; `0.0` when inside the jitter tolerance.
    pub gap_ms: f64,
    /// Classification of `gap_ms`, spike-harness compatible.
    pub class: GapClass,
    /// True when the fill was capped by [`tolerances::MAX_SINGLE_GAP_FILL_MS`].
    pub fill_capped: bool,
}

/// Pure gap-detection/placement function (contract §9: "Timeline/gap/drift math
/// must be pure functions over callback records").
///
/// * `written_samples` — timeline samples already committed for this source.
/// * `started` — whether any block has been placed yet.
/// * `arrival_ms` — timeline stamp taken at callback entry.
/// * `frames` — native frames carried by this block.
/// * `native_rate` — device-native sample rate.
///
/// The block is assumed to cover `[arrival_ms - block_duration, arrival_ms)`:
/// a capture callback delivers audio that was captured *before* it fired.
pub fn plan_block(
    written_samples: u64,
    started: bool,
    arrival_ms: f64,
    frames: u64,
    native_rate: u32,
) -> BlockPlacement {
    if native_rate == 0 {
        return BlockPlacement {
            fill_samples: 0,
            gap_ms: 0.0,
            class: GapClass::Normal,
            fill_capped: false,
        };
    }
    let block_dur_ms = frames as f64 / native_rate as f64 * 1000.0;
    let block_start_ms = (arrival_ms - block_dur_ms).max(0.0);

    // Leading offset: the span between session origin and this source's first
    // frame is pre-filled with silence so both tracks share sample 0.
    if !started {
        let capped = block_start_ms.min(tolerances::MAX_SINGLE_GAP_FILL_MS);
        return BlockPlacement {
            fill_samples: ms_to_samples(capped),
            gap_ms: block_start_ms,
            class: classify_gap(block_start_ms),
            fill_capped: block_start_ms > capped,
        };
    }

    let written_end_ms = samples_to_ms(written_samples);
    let gap_ms = block_start_ms - written_end_ms;

    // Ordinary scheduling jitter is absorbed by the drift controller, not by
    // silence insertion — otherwise every callback would nudge the track.
    if gap_ms <= tolerances::CALLBACK_JITTER_NORMAL_MAX_MS {
        return BlockPlacement {
            fill_samples: 0,
            gap_ms: 0.0,
            class: GapClass::Normal,
            fill_capped: false,
        };
    }

    let capped = gap_ms.min(tolerances::MAX_SINGLE_GAP_FILL_MS);
    BlockPlacement {
        fill_samples: ms_to_samples(capped),
        gap_ms,
        class: classify_gap(gap_ms),
        fill_capped: gap_ms > capped,
    }
}

/// Per-source clock-deviation tracker.
///
/// Measures exactly what the spike harness `drift_frames` column measures —
/// delivered device frames minus frames a gap-free stream would have delivered —
/// but accumulates it **only across contiguous callback intervals** (those with
/// no detected gap). That exclusion is what separates genuine clock drift from
/// starvation: WASAPI loopback simply stops delivering during silence, which
/// would otherwise read as an enormous negative drift (see
/// [`super::meeting_tolerances`] for the measured example).
#[derive(Clone, Debug)]
pub struct DriftTracker {
    native_rate: u32,
    last_arrival_ms: Option<f64>,
    contiguous_ms: f64,
    contiguous_frames: u64,
    delivered_frames: u64,
}

impl DriftTracker {
    pub fn new(native_rate: u32) -> Self {
        Self {
            native_rate,
            last_arrival_ms: None,
            contiguous_ms: 0.0,
            contiguous_frames: 0,
            delivered_frames: 0,
        }
    }

    /// Record one callback. `had_gap` suppresses accumulation for the interval
    /// that contained the gap.
    pub fn observe(&mut self, arrival_ms: f64, frames: u64, had_gap: bool) {
        self.delivered_frames = self.delivered_frames.saturating_add(frames);
        if let Some(prev) = self.last_arrival_ms {
            let dt = arrival_ms - prev;
            if !had_gap && dt > 0.0 {
                self.contiguous_ms += dt;
                self.contiguous_frames = self.contiguous_frames.saturating_add(frames);
            }
        }
        self.last_arrival_ms = Some(arrival_ms);
    }

    /// Frames a gap-free stream would have delivered over the contiguous span.
    pub fn expected_frames(&self) -> f64 {
        self.contiguous_ms / 1000.0 * self.native_rate as f64
    }

    /// `delivered − expected`, in **native** frames (spike `drift_frames`).
    pub fn drift_frames_native(&self) -> i64 {
        self.contiguous_frames as i64 - self.expected_frames().round() as i64
    }

    /// Same quantity expressed in 16 kHz timeline frames, which is the unit the
    /// [`tolerances::DRIFT_WARN_FRAMES`] / [`tolerances::DRIFT_EXCEEDED_FRAMES`]
    /// bounds are documented in.
    pub fn drift_frames_timeline(&self) -> i64 {
        if self.native_rate == 0 {
            return 0;
        }
        (self.drift_frames_native() as f64 * TIMELINE_SAMPLE_RATE_HZ as f64
            / self.native_rate as f64)
            .round() as i64
    }

    /// Measured rate error in parts per million over the contiguous span.
    pub fn measured_ppm(&self) -> f64 {
        let expected = self.expected_frames();
        if expected <= 0.0 {
            return 0.0;
        }
        (self.contiguous_frames as f64 - expected) / expected * 1.0e6
    }

    pub fn contiguous_ms(&self) -> f64 {
        self.contiguous_ms
    }

    pub fn delivered_frames(&self) -> u64 {
        self.delivered_frames
    }

    /// Reset accumulated statistics (used after sleep/wake, per contract §7.1:
    /// "drift counters reset at wake").
    pub fn reset(&mut self) {
        self.last_arrival_ms = None;
        self.contiguous_ms = 0.0;
        self.contiguous_frames = 0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriftDecision {
    /// New resampler ppm offset to apply, if any.
    pub retune_ppm: Option<f64>,
    /// Drift left the correction envelope for the first time.
    pub exceeded: bool,
    /// Drift crossed the warning bound for the first time.
    pub degraded: bool,
}

/// Rate-limited, deadbanded drift correction with an escape hatch
/// (contract §5.3).
#[derive(Clone, Debug, Default)]
pub struct DriftController {
    last_retune_ms: Option<f64>,
    current_ppm: f64,
    exceeded: bool,
    warned: bool,
}

impl DriftController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_ppm(&self) -> f64 {
        self.current_ppm
    }

    pub fn has_exceeded(&self) -> bool {
        self.exceeded
    }

    /// Decide what to do about the currently measured drift.
    ///
    /// * `now_ms` — timeline position of the observation.
    /// * `drift_timeline_frames` — accumulated drift in 16 kHz frames.
    /// * `measured_ppm` — instantaneous rate error over the contiguous span.
    pub fn evaluate(
        &mut self,
        now_ms: f64,
        drift_timeline_frames: i64,
        measured_ppm: f64,
    ) -> DriftDecision {
        let mut decision = DriftDecision::default();

        // Device warm-up window: the spike analyzer discards the first 10 s for
        // exactly the same reason.
        if now_ms < tolerances::DRIFT_STABLE_WINDOW_MS {
            return decision;
        }

        let magnitude = drift_timeline_frames.abs();

        if magnitude >= tolerances::DRIFT_EXCEEDED_FRAMES {
            if !self.exceeded {
                self.exceeded = true;
                decision.exceeded = true;
                decision.degraded = true;
                // Stop correcting; keep capturing uncorrected. Never drop or
                // truncate samples to "fix" drift.
                if self.current_ppm != 0.0 {
                    self.current_ppm = 0.0;
                    decision.retune_ppm = Some(0.0);
                }
            }
            return decision;
        }

        if self.exceeded {
            return decision;
        }

        if magnitude >= tolerances::DRIFT_WARN_FRAMES && !self.warned {
            self.warned = true;
            decision.degraded = true;
        }

        if let Some(last) = self.last_retune_ms {
            if now_ms - last < tolerances::DRIFT_RETUNE_MIN_INTERVAL_MS {
                return decision;
            }
        }

        // Cancel the ongoing rate error and retire the accumulated offset over
        // the correction horizon.
        let catchup_ppm = -(drift_timeline_frames as f64)
            / (TIMELINE_SAMPLE_RATE_HZ as f64 * tolerances::DRIFT_CORRECTION_HORIZON_MS / 1000.0)
            * 1.0e6;
        let desired = (-measured_ppm + catchup_ppm).clamp(
            -tolerances::DRIFT_CORRECTION_MAX_PPM,
            tolerances::DRIFT_CORRECTION_MAX_PPM,
        );

        if (desired - self.current_ppm).abs() < tolerances::DRIFT_DEADBAND_PPM {
            return decision;
        }

        self.current_ppm = desired;
        self.last_retune_ms = Some(now_ms);
        decision.retune_ppm = Some(desired);
        decision
    }
}

/// Peak and RMS of a mono block, in the same scale as the dictation level meter.
pub fn block_levels(mono: &[f32]) -> (f32, f32) {
    if mono.is_empty() {
        return (0.0, 0.0);
    }
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    for &s in mono {
        peak = peak.max(s.abs());
        sum_sq += f64::from(s) * f64::from(s);
    }
    let rms = (sum_sq / mono.len() as f64).sqrt() as f32;
    (peak, rms)
}

// ===========================================================================
// 4. WAV helpers and chunk I/O
// ===========================================================================

/// Canonical header length of the recorder's WAV files. The recorder writes a
/// minimal, fixed 44-byte RIFF header and nothing else, which is what makes
/// truncation recovery (§8 2a) a pure arithmetic operation.
pub const WAV_HEADER_LEN: usize = 44;

const BYTES_PER_SAMPLE: u64 = 2;

/// Payload size written into the *provisional* header of an open `.wav.part`.
///
/// It must be the largest representable value, not `0`: crash recovery resolves
/// a truncated chunk with `min(declared_data_size, actual_bytes_after_header)`
/// (contract §8.2a), so a provisional `0` would make every interrupted chunk
/// look empty and be discarded. `u32::MAX` lets the actual on-disk length win,
/// which is exactly the information a crashed writer leaves behind.
const PROVISIONAL_DATA_BYTES: u32 = u32::MAX;

/// The header an open chunk carries until [`ChunkWriter::finalize`] patches it.
fn provisional_header() -> [u8; WAV_HEADER_LEN] {
    wav_header(PROVISIONAL_DATA_BYTES)
}

/// Build the 44-byte PCM16 / 16 kHz / mono header for `data_bytes` of payload.
fn wav_header(data_bytes: u32) -> [u8; WAV_HEADER_LEN] {
    let mut h = [0u8; WAV_HEADER_LEN];
    let channels = TIMELINE_CHANNELS;
    let rate = TIMELINE_SAMPLE_RATE_HZ;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = rate * u32::from(block_align);

    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36u32.saturating_add(data_bytes)).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&bits.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// Declared payload size of a recorder-written WAV, or `None` if the file is
/// not one (recovery case 2b).
pub fn parse_wav_header(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < WAV_HEADER_LEN {
        return None;
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" || &bytes[12..16] != b"fmt " {
        return None;
    }
    if &bytes[36..40] != b"data" {
        return None;
    }
    let mut le = [0u8; 4];
    le.copy_from_slice(&bytes[40..44]);
    Some(u32::from_le_bytes(le))
}

/// Filesystem operations used by chunk finalisation, behind a trait so tests can
/// inject faults at each step of the commit sequence (contract §9).
pub trait ChunkFs: Send + Sync {
    fn create_new(&self, path: &Path) -> io::Result<File>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// fsync the directory containing `dir` so the rename itself is durable.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
}

/// Production filesystem.
#[derive(Debug, Default)]
pub struct RealChunkFs;

impl ChunkFs for RealChunkFs {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        // NTFS has no fsyncable directory handle through `std::fs`; the rename
        // is journaled by the filesystem, so this is a documented no-op on
        // Windows and a real fsync elsewhere.
        #[cfg(target_os = "windows")]
        {
            let _ = dir;
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            File::open(dir)?.sync_all()
        }
    }
}

/// Streaming PCM16 chunk writer. Holds a fixed-size buffer; never accumulates
/// the recording.
struct ChunkWriter {
    chunk_id: ChunkId,
    sequence: u32,
    source: SourceKind,
    part_path: PathBuf,
    final_path: PathBuf,
    storage_key: String,
    file: Option<BufWriter<File>>,
    /// Timeline sample index of this chunk's first sample.
    start_sample: u64,
    samples: u64,
    /// Reused PCM16 conversion scratch — this is the whole per-chunk RAM cost.
    scratch: Vec<u8>,
    gap_count: u32,
    gap_samples: u64,
}

/// PCM16 conversion scratch size (samples). 4 096 samples = 8 KiB of bytes.
const SCRATCH_SAMPLES: usize = 4096;

/// Pre-zeroed silence block, reused for every gap fill so a multi-minute gap
/// costs no additional allocation.
const SILENCE_BLOCK: [f32; SCRATCH_SAMPLES] = [0.0; SCRATCH_SAMPLES];

impl ChunkWriter {
    fn open(
        fs_impl: &dyn ChunkFs,
        audio_dir: &Path,
        source: SourceKind,
        sequence: u32,
        chunk_id: ChunkId,
        start_sample: u64,
    ) -> Result<Self, RecorderError> {
        let name = format!("{:06}-{}.wav", sequence, source.file_tag());
        let final_path = audio_dir.join(&name);
        let part_path = audio_dir.join(format!("{name}.part"));
        let mut file =
            fs_impl
                .create_new(&part_path)
                .map_err(|e| RecorderError::ChunkCreateFailed {
                    source,
                    detail: format!("{}: {e}", part_path.display()),
                })?;
        file.write_all(&provisional_header())
            .map_err(|e| RecorderError::ChunkCreateFailed {
                source,
                detail: format!("header write: {e}"),
            })?;
        Ok(Self {
            chunk_id,
            sequence,
            source,
            part_path,
            final_path,
            storage_key: format!("audio/{name}"),
            file: Some(BufWriter::with_capacity(64 * 1024, file)),
            start_sample,
            samples: 0,
            scratch: Vec::with_capacity(SCRATCH_SAMPLES * 2),
            gap_count: 0,
            gap_samples: 0,
        })
    }

    fn write(&mut self, data: &[f32]) -> Result<(), RecorderError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| RecorderError::ChunkCreateFailed {
                source: self.source,
                detail: "chunk already finalized".to_string(),
            })?;
        for block in data.chunks(SCRATCH_SAMPLES) {
            self.scratch.clear();
            for &s in block {
                let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                self.scratch.extend_from_slice(&v.to_le_bytes());
            }
            file.write_all(&self.scratch)
                .map_err(|e| RecorderError::ChunkCreateFailed {
                    source: self.source,
                    detail: format!("{}: {e}", self.part_path.display()),
                })?;
        }
        self.samples = self.samples.saturating_add(data.len() as u64);
        Ok(())
    }

    fn range(&self) -> TimelineRange {
        TimelineRange {
            start_ms: samples_to_ms(self.start_sample),
            end_ms: samples_to_ms(self.start_sample + self.samples),
        }
    }

    /// Commit sequence of contract §3.1.2: patch header → flush/fsync → atomic
    /// rename → fsync directory → checksum.
    fn finalize(mut self, fs_impl: &dyn ChunkFs) -> Result<FinalizedChunk, RecorderError> {
        let mut buf = self
            .file
            .take()
            .ok_or_else(|| RecorderError::ChunkCreateFailed {
                source: self.source,
                detail: "chunk already finalized".to_string(),
            })?;
        buf.flush().map_err(|e| self.io_err("flush", e))?;
        let mut file = buf
            .into_inner()
            .map_err(|e| self.io_err_owned("into_inner", e.into_error().to_string()))?;

        let data_bytes = u32::try_from(self.samples.saturating_mul(BYTES_PER_SAMPLE))
            .map_err(|_| self.io_err_owned("size", "chunk exceeds 4 GiB".to_string()))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| self.io_err("seek", e))?;
        file.write_all(&wav_header(data_bytes))
            .map_err(|e| self.io_err("header patch", e))?;
        file.flush().map_err(|e| self.io_err("flush", e))?;
        file.sync_all().map_err(|e| self.io_err("fsync", e))?;
        drop(file);

        fs_impl
            .rename(&self.part_path, &self.final_path)
            .map_err(|e| self.io_err("rename", e))?;
        if let Some(dir) = self.final_path.parent() {
            // A failed directory fsync does not invalidate the rename itself;
            // recovery reconciles anything the OS lost.
            let _ = fs_impl.sync_dir(dir);
        }

        let checksum = sha256_file(&self.final_path).map_err(|e| self.io_err("checksum", e))?;

        Ok(FinalizedChunk {
            info: FinalizedChunkInfo {
                chunk_id: self.chunk_id,
                source: self.source,
                sequence: self.sequence,
                range: self.range(),
                sample_count: self.samples,
            },
            storage_key: self.storage_key.clone(),
            checksum,
            gap_stats: (self.gap_count, self.gap_samples),
        })
    }

    fn io_err(&self, stage: &str, e: io::Error) -> RecorderError {
        self.io_err_owned(stage, e.to_string())
    }

    fn io_err_owned(&self, stage: &str, e: String) -> RecorderError {
        RecorderError::ChunkCreateFailed {
            source: self.source,
            detail: format!("{stage} failed for {}: {e}", self.part_path.display()),
        }
    }
}

struct FinalizedChunk {
    info: FinalizedChunkInfo,
    storage_key: String,
    checksum: String,
    gap_stats: (u32, u64),
}

/// Streaming SHA-256 with a fixed 64 KiB window.
fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ===========================================================================
// 5. Per-source writer pipeline
// ===========================================================================

/// Everything a source writer needs to start; passed to the writer thread.
struct SourceWriterConfig {
    source: SourceKind,
    session_id: String,
    audio_dir: PathBuf,
    segment_id: i64,
    native_rate: u32,
    rollover_samples: u64,
    /// Timeline sample the source continues at (non-zero after resume).
    initial_written: u64,
    /// First sequence number to use (continues across pause/resume).
    initial_sequence: u32,
}

/// Owns one source's normalization → chunking pipeline. Single-threaded by
/// construction: exactly one writer thread drives it.
struct SourceWriter {
    cfg: SourceWriterConfig,
    store: Arc<dyn ChunkStateStore>,
    fs_impl: Arc<dyn ChunkFs>,
    sink: Arc<dyn RecorderEventSink>,
    shared: Arc<Mutex<SharedState>>,
    resampler: FrameResampler,
    effective_in_rate: u32,
    drift: DriftTracker,
    drift_ctl: DriftController,
    chunk: Option<ChunkWriter>,
    next_sequence: u32,
    written: u64,
    started: bool,
    last_level_ms: f64,
    state: SourceState,
    finalized: Vec<FinalizedChunkInfo>,
    degraded_ranges: Vec<(SourceKind, TimelineRange, GapClass)>,
    /// Reused resampler output buffer; capacity stays at one frame.
    out_buf: Vec<f32>,
    warned_startup_offset: bool,
    /// Session clock, shared with the capture backend so idle silence lands on
    /// the same timeline the blocks are stamped against.
    origin: Instant,
    paused_ms: Arc<Mutex<f64>>,
}

impl SourceWriter {
    fn new(
        cfg: SourceWriterConfig,
        store: Arc<dyn ChunkStateStore>,
        fs_impl: Arc<dyn ChunkFs>,
        sink: Arc<dyn RecorderEventSink>,
        shared: Arc<Mutex<SharedState>>,
        origin: Instant,
        paused_ms: Arc<Mutex<f64>>,
    ) -> Self {
        let native_rate = cfg.native_rate.max(1);
        let resampler = FrameResampler::new(
            native_rate as usize,
            TIMELINE_SAMPLE_RATE_HZ as usize,
            Duration::from_millis(tolerances::RESAMPLER_FRAME_MS),
        );
        let written = cfg.initial_written;
        let next_sequence = cfg.initial_sequence;
        Self {
            store,
            fs_impl,
            sink,
            shared,
            resampler,
            effective_in_rate: native_rate,
            drift: DriftTracker::new(native_rate),
            drift_ctl: DriftController::new(),
            chunk: None,
            next_sequence,
            written,
            // After a resume the timeline continues where it froze, so the
            // source is "started": no leading-silence pre-fill is re-applied.
            started: cfg.initial_written > 0,
            last_level_ms: f64::NEG_INFINITY,
            state: SourceState::Active,
            finalized: Vec::new(),
            degraded_ranges: Vec::new(),
            out_buf: Vec::new(),
            warned_startup_offset: false,
            origin,
            paused_ms,
            cfg,
        }
    }

    /// Current session timeline position.
    ///
    /// A writer thread only exists while the recorder is actually recording —
    /// `pause` tears every writer down before freezing the clock — so the
    /// paused branch of the recorder's own clock cannot apply here.
    fn timeline_now_ms(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0 - *lock_recover(&self.paused_ms)
    }

    /// Extends a silent source's track up to (nearly) now.
    ///
    /// WASAPI loopback stops delivering callbacks entirely while nothing plays,
    /// so without this a meeting where no system audio was played produced *no
    /// system chunk at all*: the track was missing rather than silent, the
    /// session reported no system audio, and the user could not tell "nothing
    /// was playing" from "loopback never worked". Silence is a legitimate
    /// recording, and the plan requires missing spans to be represented as
    /// silence instead of shortening the track.
    ///
    /// The fill deliberately trails `now` by [`tolerances::IDLE_SILENCE_LAG_MS`]
    /// so a block that is merely late is still placed at its own timestamp
    /// rather than appended behind an already-advanced write head.
    fn fill_idle_silence(&mut self) -> Result<(), RecorderError> {
        if matches!(self.state, SourceState::Lost | SourceState::Closed) {
            return Ok(());
        }
        let target_ms = self.timeline_now_ms() - tolerances::IDLE_SILENCE_LAG_MS;
        if target_ms <= 0.0 {
            return Ok(());
        }
        let missing = ms_to_samples(target_ms).saturating_sub(self.written);
        if missing == 0 {
            return Ok(());
        }
        // A source that never delivered a block starts at timeline zero: its
        // silence *is* the leading offset, so no separate pre-fill applies.
        self.started = true;
        self.write_silence(missing)?;
        // Publish an honest zero level so a silent meter reads "nothing is
        // playing" rather than keeping the last value from minutes ago.
        self.emit(RecorderEvent::LevelUpdate {
            source: self.cfg.source,
            peak: 0.0,
            rms: 0.0,
        });
        Ok(())
    }

    fn emit(&self, event: RecorderEvent) {
        self.sink.on_event(&event);
    }

    fn set_state(&mut self, state: SourceState) {
        if self.state == state {
            return;
        }
        self.state = state;
        {
            let mut shared = lock_recover(&self.shared);
            if let Some(status) = shared.statuses.get_mut(&self.cfg.source) {
                status.state = state;
            }
        }
        self.emit(RecorderEvent::SourceStateChanged {
            source: self.cfg.source,
            state,
        });
    }

    /// Process one capture block. Never blocks on anything but its own file.
    fn push_block(&mut self, block: CaptureBlock) -> Result<(), RecorderError> {
        if block.mono.is_empty() {
            return Ok(());
        }
        let (peak, rms) = block_levels(&block.mono);
        if block.timeline_ms - self.last_level_ms >= tolerances::LEVEL_EVENT_INTERVAL_MS as f64 {
            self.last_level_ms = block.timeline_ms;
            self.emit(RecorderEvent::LevelUpdate {
                source: self.cfg.source,
                peak,
                rms,
            });
        }

        let frames = block.mono.len() as u64;
        let probe = plan_block(
            self.written,
            self.started,
            block.timeline_ms,
            frames,
            self.cfg.native_rate,
        );
        let had_gap = probe.fill_samples > 0;

        // Audio still buffered inside the resampler was captured *before* the
        // hole, so it must reach the file before the silence — otherwise a 20 ms
        // tail would be re-timestamped to the far side of a multi-second gap.
        // Resetting afterwards also stops the FFT overlap from smearing across
        // the discontinuity. The fill is then re-planned against the advanced
        // write position, so the gap still ends exactly on the timeline.
        let placement = if had_gap && self.started {
            self.flush_resampler()?;
            self.resampler.reset();
            plan_block(
                self.written,
                self.started,
                block.timeline_ms,
                frames,
                self.cfg.native_rate,
            )
        } else {
            probe
        };

        if !self.started {
            self.started = true;
            // Contract §5.3.5: only the microphone warns about a large leading
            // offset. A silent loopback legitimately delivers its first callback
            // minutes late (measured: 5 709 ms in the smoke run).
            if self.cfg.source == SourceKind::Microphone
                && probe.gap_ms > tolerances::INITIAL_OFFSET_FILL_TOLERANCE_MS
                && !self.warned_startup_offset
            {
                self.warned_startup_offset = true;
                self.emit(RecorderEvent::StartupOffsetWarning {
                    source: self.cfg.source,
                    offset_ms: probe.gap_ms,
                });
            }
        }

        if had_gap {
            let range = TimelineRange {
                start_ms: samples_to_ms(self.written),
                end_ms: samples_to_ms(self.written + placement.fill_samples),
            };
            self.write_silence(placement.fill_samples)?;
            if let Some(chunk) = self.chunk.as_mut() {
                chunk.gap_count = chunk.gap_count.saturating_add(1);
                chunk.gap_samples = chunk.gap_samples.saturating_add(placement.fill_samples);
            }
            // Classification and reporting use the *detected* gap, so recorder
            // telemetry stays directly comparable to the spike harness even
            // though the written fill is adjusted for the flushed tail.
            if probe.gap_ms >= tolerances::GAP_EVENT_MIN_MS {
                self.emit(RecorderEvent::TimelineGap {
                    source: self.cfg.source,
                    range,
                    class: probe.class,
                });
            }
            // Only the microphone degrades on gaps: for the system source a long
            // silence means "nothing is playing", which the contract explicitly
            // classifies as not an error.
            if self.cfg.source == SourceKind::Microphone
                && probe.gap_ms >= tolerances::SEGMENT_DEGRADED_GAP_MS
            {
                self.degraded_ranges
                    .push((self.cfg.source, range, probe.class));
                self.set_state(SourceState::Degraded);
            }
        }

        self.drift.observe(block.timeline_ms, frames, had_gap);
        self.apply_drift(block.timeline_ms)?;

        let mut out = std::mem::take(&mut self.out_buf);
        out.clear();
        self.resampler
            .push(&block.mono, |frame| out.extend_from_slice(frame));
        let result = self.write_samples(&out);
        out.clear();
        self.out_buf = out;
        result
    }

    /// Drain the resampler's internal buffers into the current chunk.
    ///
    /// `FrameResampler::finish` zero-pads its trailing partial frame, so the
    /// track can end up to one resampler frame (20 ms) longer than the exact
    /// timeline at each flush point. Mid-stream that is absorbed by re-planning
    /// the following gap fill; only the last flush of a chunk keeps the padding.
    fn flush_resampler(&mut self) -> Result<(), RecorderError> {
        let mut out = std::mem::take(&mut self.out_buf);
        out.clear();
        self.resampler.finish(|frame| out.extend_from_slice(frame));
        let result = self.write_samples(&out);
        out.clear();
        self.out_buf = out;
        result
    }

    fn apply_drift(&mut self, now_ms: f64) -> Result<(), RecorderError> {
        let drift_frames = self.drift.drift_frames_timeline();
        let decision = self
            .drift_ctl
            .evaluate(now_ms, drift_frames, self.drift.measured_ppm());

        if decision.exceeded {
            self.emit(RecorderEvent::DriftToleranceExceeded {
                source: self.cfg.source,
                measured_drift_frames: drift_frames,
                expected_frames: self.drift.expected_frames().round() as i64,
                delivered_frames: self.drift.delivered_frames() as i64,
            });
        }
        if decision.degraded {
            self.set_state(SourceState::Degraded);
        }
        if let Some(ppm) = decision.retune_ppm {
            self.retune_resampler(ppm)?;
            self.emit(RecorderEvent::DriftCorrectionApplied {
                source: self.cfg.source,
                ratio_ppm: ppm,
            });
        }
        Ok(())
    }

    /// Modulate the resampler ratio by `ppm`.
    ///
    /// [`FrameResampler`] wraps rubato's fixed-ratio FFT resampler, so the ratio
    /// is modulated through the *effective input rate*: emitting more output per
    /// unit time means pretending the device runs slower. Because the effective
    /// rate is an integer, the smallest representable step at 48 kHz is ~20.8 ppm
    /// — which is why [`tolerances::DRIFT_DEADBAND_PPM`] is 25 ppm. On healthy
    /// hardware (measured ±7 ppm) this path never runs at all.
    fn retune_resampler(&mut self, ppm: f64) -> Result<(), RecorderError> {
        let target = (self.cfg.native_rate as f64 / (1.0 + ppm / 1.0e6)).round();
        if !target.is_finite() || target < 1.0 {
            return Ok(());
        }
        let target = target as u32;
        if target == self.effective_in_rate {
            return Ok(());
        }
        // Flush the outgoing resampler so its buffered tail is not lost.
        self.flush_resampler()?;

        self.effective_in_rate = target;
        self.resampler = FrameResampler::new(
            target as usize,
            TIMELINE_SAMPLE_RATE_HZ as usize,
            Duration::from_millis(tolerances::RESAMPLER_FRAME_MS),
        );
        Ok(())
    }

    fn write_silence(&mut self, mut samples: u64) -> Result<(), RecorderError> {
        while samples > 0 {
            let n = samples.min(SCRATCH_SAMPLES as u64) as usize;
            self.write_samples(&SILENCE_BLOCK[..n])?;
            samples -= n as u64;
        }
        Ok(())
    }

    fn write_samples(&mut self, mut data: &[f32]) -> Result<(), RecorderError> {
        while !data.is_empty() {
            self.ensure_chunk()?;
            let remaining = match self.chunk.as_ref() {
                Some(chunk) => self.cfg.rollover_samples.saturating_sub(chunk.samples),
                None => return Ok(()),
            };
            if remaining == 0 {
                self.finalize_chunk()?;
                continue;
            }
            let take = (remaining as usize).min(data.len());
            if let Some(chunk) = self.chunk.as_mut() {
                chunk.write(&data[..take])?;
            }
            self.written = self.written.saturating_add(take as u64);
            data = &data[take..];
        }
        Ok(())
    }

    fn ensure_chunk(&mut self) -> Result<(), RecorderError> {
        if self.chunk.is_some() {
            return Ok(());
        }
        let sequence = self.next_sequence;
        let name = format!("{:06}-{}.wav", sequence, self.cfg.source.file_tag());
        let descriptor = ChunkDescriptor {
            chunk_id: 0,
            source: self.cfg.source,
            sequence,
            source_segment_id: self.cfg.segment_id,
            state: ChunkState::Writing,
            range: TimelineRange {
                start_ms: samples_to_ms(self.written),
                end_ms: samples_to_ms(self.written),
            },
            sample_count: 0,
            storage_key: format!("audio/{name}"),
            checksum_sha256: None,
            recovered: false,
        };
        let chunk_id = self
            .store
            .insert_writing_chunk(&self.cfg.session_id, &descriptor)
            .map_err(|e| RecorderError::ChunkCreateFailed {
                source: self.cfg.source,
                detail: format!("store insert: {e}"),
            })?;
        let writer = ChunkWriter::open(
            self.fs_impl.as_ref(),
            &self.cfg.audio_dir,
            self.cfg.source,
            sequence,
            chunk_id,
            self.written,
        )?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        {
            let mut shared = lock_recover(&self.shared);
            if let Some(status) = shared.statuses.get_mut(&self.cfg.source) {
                status.current_chunk_sequence = Some(sequence);
            }
        }
        self.chunk = Some(writer);
        Ok(())
    }

    fn finalize_chunk(&mut self) -> Result<(), RecorderError> {
        let Some(chunk) = self.chunk.take() else {
            return Ok(());
        };
        if chunk.samples == 0 {
            // Never publish an empty chunk; drop the staging file instead.
            let part = chunk.part_path.clone();
            let chunk_id = chunk.chunk_id;
            drop(chunk);
            let _ = fs::remove_file(&part);
            let _ = self.store.transition(
                chunk_id,
                ChunkState::Writing,
                ChunkState::Damaged,
                &ChunkPatch::default(),
            );
            return Ok(());
        }
        let chunk_id = chunk.chunk_id;
        let finalized = chunk.finalize(self.fs_impl.as_ref())?;
        let patch = ChunkPatch {
            storage_key: Some(finalized.storage_key.clone()),
            range: Some(finalized.info.range),
            sample_count: Some(finalized.info.sample_count),
            checksum_sha256: Some(finalized.checksum.clone()),
            recovered: Some(false),
            gap_stats: Some(finalized.gap_stats),
        };
        // Contract §3.1.2: retry the DB step once; on repeated failure keep the
        // file — it is already durably `ready` on disk and recovery reconciles.
        let mut persisted =
            self.store
                .transition(chunk_id, ChunkState::Writing, ChunkState::Ready, &patch);
        if persisted.is_err() {
            persisted =
                self.store
                    .transition(chunk_id, ChunkState::Writing, ChunkState::Ready, &patch);
        }
        if let Err(detail) = persisted {
            self.emit(RecorderEvent::ChunkFinalized {
                info: finalized.info.clone(),
            });
            self.finalized.push(finalized.info);
            self.bump_backlog();
            return Err(RecorderError::ChunkStatePersistFailed { chunk_id, detail });
        }

        self.emit(RecorderEvent::ChunkFinalized {
            info: finalized.info.clone(),
        });
        self.finalized.push(finalized.info);
        self.bump_backlog();
        Ok(())
    }

    fn bump_backlog(&self) {
        let mut shared = lock_recover(&self.shared);
        if let Some(status) = shared.statuses.get_mut(&self.cfg.source) {
            status.backlog_chunks_ready = status.backlog_chunks_ready.saturating_add(1);
            status.current_chunk_sequence = None;
        }
    }

    /// Flush the resampler tail and finalize the open chunk (rollover, pause,
    /// close_source, stop).
    fn finish(&mut self) -> Result<(), RecorderError> {
        let result = self.flush_resampler();
        let finalize = self.finalize_chunk();
        result.and(finalize)
    }

    fn outcome(&mut self, error: Option<RecorderError>) -> SourceOutcome {
        SourceOutcome {
            finalized: std::mem::take(&mut self.finalized),
            degraded_ranges: std::mem::take(&mut self.degraded_ranges),
            written: self.written,
            next_sequence: self.next_sequence,
            error,
        }
    }
}

#[derive(Debug, Default)]
struct SourceOutcome {
    finalized: Vec<FinalizedChunkInfo>,
    degraded_ranges: Vec<(SourceKind, TimelineRange, GapClass)>,
    written: u64,
    next_sequence: u32,
    error: Option<RecorderError>,
}

/// Commands accepted by a source's writer thread.
enum WriterCmd {
    Block(CaptureBlock),
    /// Finalize the open chunk but keep the thread alive (device-loss path:
    /// contract §7.1 requires the open chunk to reach `ready` first).
    FinalizeOpen,
    /// Finalize and terminate, returning the outcome.
    Finish(mpsc::Sender<SourceOutcome>),
}

fn run_source_writer(
    mut writer: SourceWriter,
    rx: mpsc::Receiver<WriterCmd>,
    queued: Arc<AtomicUsize>,
) {
    let mut error: Option<RecorderError> = None;
    let idle_tick = Duration::from_millis(tolerances::IDLE_SILENCE_TICK_MS);
    loop {
        let cmd = match rx.recv_timeout(idle_tick) {
            Ok(cmd) => cmd,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No block arrived this tick. For a healthy but silent source
                // that is the normal loopback behaviour, not a fault.
                if error.is_none() {
                    if let Err(e) = writer.fill_idle_silence() {
                        log::error!(
                            "meeting recorder: {:?} idle fill error: {e}",
                            writer.cfg.source
                        );
                        error = Some(e);
                        let _ = writer.finish();
                        writer.set_state(SourceState::Lost);
                    }
                }
                continue;
            }
            // Sender dropped without an explicit Finish (recorder was dropped):
            // still finalize so no chunk is lost.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match cmd {
            WriterCmd::Block(block) => {
                queued.fetch_sub(1, Ordering::AcqRel);
                if let Err(e) = writer.push_block(block) {
                    log::error!(
                        "meeting recorder: {:?} writer error: {e}",
                        writer.cfg.source
                    );
                    if error.is_none() {
                        error = Some(e);
                    }
                    // Stop consuming into a broken chunk; keep draining the
                    // queue so the capture callback never blocks.
                    let _ = writer.finish();
                    writer.set_state(SourceState::Lost);
                }
            }
            WriterCmd::FinalizeOpen => {
                if let Err(e) = writer.finish() {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
            }
            WriterCmd::Finish(reply) => {
                // Close the timeline gap first: a source that fell silent
                // shortly before the stop still owns that span.
                if error.is_none() {
                    if let Err(e) = writer.fill_idle_silence() {
                        error = Some(e);
                    }
                }
                if let Err(e) = writer.finish() {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
                let outcome = writer.outcome(error);
                let _ = reply.send(outcome);
                return;
            }
        }
    }
    let _ = writer.finish();
}

// ===========================================================================
// 6. Capture backend seam
// ===========================================================================

/// A live capture stream handle. Dropping or calling [`CaptureStream::stop`]
/// ends capture.
pub trait CaptureStream: Send {
    fn native_sample_rate(&self) -> u32;
    fn stop(&mut self);
}

/// Opens capture streams. Production implementation is [`CpalCaptureBackend`];
/// tests inject fakes so every layer above is exercised without audio hardware.
pub trait CaptureBackend: Send + Sync {
    fn preflight(&self, source: SourceKind, request: &SourceOpenRequest) -> PreflightCheck;

    /// Open `source`. `on_block` is called from the capture callback and must be
    /// cheap; `on_error` is called from the cpal error callback.
    fn open(
        &self,
        source: SourceKind,
        request: &SourceOpenRequest,
        on_block: Box<dyn FnMut(CaptureBlock) + Send>,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn CaptureStream>, RecorderError>;
}

/// Map a device-open failure to the contract's error taxonomy.
fn classify_open_error(source: SourceKind, device_name: &str, detail: String) -> RecorderError {
    let lower = detail.to_lowercase();
    if super::is_microphone_access_denied(&detail) {
        return RecorderError::CapturePermissionDenied { source, detail };
    }
    if lower.contains("exclusive") || lower.contains("0x8889000a") {
        return RecorderError::LoopbackSharedModeRequired {
            device_name: device_name.to_string(),
            detail,
        };
    }
    match source {
        SourceKind::Microphone => RecorderError::MicDeviceUnavailable {
            device_name: device_name.to_string(),
            detail,
        },
        SourceKind::System => RecorderError::SystemOutputUnavailable {
            device_name: device_name.to_string(),
            detail,
        },
    }
}

// ---------------------------------------------------------------------------
// cpal backend
// ---------------------------------------------------------------------------

/// Production capture backend.
///
/// The microphone is an ordinary cpal input stream. System audio is a **render**
/// device opened as an input stream: cpal 0.16 sets
/// `AUDCLNT_STREAMFLAGS_LOOPBACK` automatically when the device's data flow is
/// `eRender`, which is exactly what the Phase-1 spike harness proved on target
/// hardware (`src-tauri/src/bin/meeting_loopback_spike.rs`). The stream config
/// must come from `default_output_config` for loopback and
/// `default_input_config` for capture.
pub struct CpalCaptureBackend {
    origin: Instant,
    /// Accumulated paused time, subtracted from every stamp so the timeline
    /// freezes across pause (contract §2.2 rule 3).
    paused_ms: Arc<Mutex<f64>>,
}

impl CpalCaptureBackend {
    pub fn new(origin: Instant, paused_ms: Arc<Mutex<f64>>) -> Self {
        Self { origin, paused_ms }
    }
}

impl CaptureBackend for CpalCaptureBackend {
    fn preflight(&self, source: SourceKind, request: &SourceOpenRequest) -> PreflightCheck {
        #[cfg(not(target_os = "windows"))]
        if source == SourceKind::System {
            return PreflightCheck {
                source,
                device_name: request.device_name.clone(),
                outcome: PreflightOutcome::Missing,
                native_sample_rate_hint: None,
                detail: "system-audio loopback is only supported on Windows (plan §2)".to_string(),
            };
        }

        match find_device(source, &request.device_name, request.follow_default_device) {
            Ok((device, name)) => {
                let config = device_config(&device, source);
                match config {
                    Ok(cfg) => PreflightCheck {
                        source,
                        device_name: name,
                        outcome: PreflightOutcome::Available,
                        native_sample_rate_hint: Some(cfg.sample_rate().0),
                        detail: String::new(),
                    },
                    Err(detail) => {
                        let lower = detail.to_lowercase();
                        let outcome = if super::is_microphone_access_denied(&detail) {
                            PreflightOutcome::PermissionDenied
                        } else if lower.contains("exclusive") || lower.contains("0x8889000a") {
                            PreflightOutcome::SharedModeRequired
                        } else {
                            PreflightOutcome::Missing
                        };
                        PreflightCheck {
                            source,
                            device_name: name,
                            outcome,
                            native_sample_rate_hint: None,
                            detail,
                        }
                    }
                }
            }
            Err(detail) => PreflightCheck {
                source,
                device_name: request.device_name.clone(),
                outcome: PreflightOutcome::Missing,
                native_sample_rate_hint: None,
                detail,
            },
        }
    }

    fn open(
        &self,
        source: SourceKind,
        request: &SourceOpenRequest,
        on_block: Box<dyn FnMut(CaptureBlock) + Send>,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn CaptureStream>, RecorderError> {
        #[cfg(not(target_os = "windows"))]
        if source == SourceKind::System {
            return Err(RecorderError::LoopbackUnavailable {
                detail: "system-audio loopback is only supported on Windows (plan §2)".to_string(),
            });
        }

        let origin = self.origin;
        let paused = Arc::clone(&self.paused_ms);
        let device_name = request.device_name.clone();
        let follow_default = request.follow_default_device;

        // cpal's `Stream` is not `Send` on every backend, so the stream is built
        // and owned by a dedicated thread — the same pattern the dictation
        // recorder uses.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, RecorderError>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let name_for_thread = device_name.clone();
        let join = std::thread::Builder::new()
            .name(format!("meeting-capture-{}", source.file_tag()))
            .spawn(move || {
                let built = build_cpal_stream(
                    source,
                    &name_for_thread,
                    follow_default,
                    origin,
                    paused,
                    on_block,
                    on_error,
                );
                match built {
                    Ok((stream, rate)) => {
                        let _ = ready_tx.send(Ok(rate));
                        // Park until told to stop; the stream must stay alive
                        // and owned by this thread.
                        let _ = stop_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| RecorderError::StreamError {
                source,
                detail: format!("failed to spawn capture thread: {e}"),
            })?;

        match ready_rx.recv() {
            Ok(Ok(rate)) => Ok(Box::new(CpalStreamHandle {
                rate,
                stop_tx: Some(stop_tx),
                join: Some(join),
            })),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(e) => {
                let _ = join.join();
                Err(RecorderError::StreamError {
                    source,
                    detail: format!("capture thread died before reporting readiness: {e}"),
                })
            }
        }
    }
}

struct CpalStreamHandle {
    rate: u32,
    stop_tx: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl CaptureStream for CpalStreamHandle {
    fn native_sample_rate(&self) -> u32 {
        self.rate
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for CpalStreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn find_device(
    source: SourceKind,
    device_name: &str,
    follow_default: bool,
) -> Result<(cpal::Device, String), String> {
    let devices = match source {
        SourceKind::Microphone => super::list_input_devices(),
        SourceKind::System => super::list_output_devices(),
    }
    .map_err(|e| format!("device enumeration failed: {e}"))?;

    if follow_default {
        if let Some(info) = devices.iter().find(|d| d.is_default) {
            return Ok((info.device.clone(), info.name.clone()));
        }
    }
    if let Some(info) = devices.iter().find(|d| d.name == device_name) {
        return Ok((info.device.clone(), info.name.clone()));
    }
    if let Some(info) = devices.iter().find(|d| d.name.contains(device_name)) {
        return Ok((info.device.clone(), info.name.clone()));
    }
    Err(format!("device '{device_name}' not found"))
}

fn device_config(
    device: &cpal::Device,
    source: SourceKind,
) -> Result<cpal::SupportedStreamConfig, String> {
    use cpal::traits::DeviceTrait;
    // Render devices expose their mix format only via `default_output_config`;
    // real capture devices via `default_input_config`.
    let result = match source {
        SourceKind::Microphone => device.default_input_config(),
        SourceKind::System => device.default_output_config(),
    };
    result.map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_cpal_stream(
    source: SourceKind,
    device_name: &str,
    follow_default: bool,
    origin: Instant,
    paused_ms: Arc<Mutex<f64>>,
    on_block: Box<dyn FnMut(CaptureBlock) + Send>,
    on_error: Box<dyn Fn(String) + Send + Sync>,
) -> Result<(cpal::Stream, u32), RecorderError> {
    use cpal::traits::{DeviceTrait, StreamTrait};
    use cpal::Sample;

    let (device, resolved_name) = find_device(source, device_name, follow_default)
        .map_err(|detail| classify_open_error(source, device_name, detail))?;
    let config = device_config(&device, source)
        .map_err(|detail| classify_open_error(source, &resolved_name, detail))?;
    let native_rate = config.sample_rate().0;
    let channels = usize::from(config.channels().max(1));

    let on_block = Arc::new(Mutex::new(on_block));

    macro_rules! build {
        ($ty:ty) => {{
            let on_block = Arc::clone(&on_block);
            let paused = Arc::clone(&paused_ms);
            let data_cb = move |data: &[$ty], _: &cpal::InputCallbackInfo| {
                let elapsed_ms = origin.elapsed().as_secs_f64() * 1000.0;
                let timeline_ms = elapsed_ms - *lock_recover(&paused);
                let mut mono = Vec::with_capacity(data.len() / channels + 1);
                for frame in data.chunks_exact(channels) {
                    let sum: f32 = frame.iter().map(|&s| s.to_sample::<f32>()).sum();
                    mono.push(sum / channels as f32);
                }
                let block = CaptureBlock {
                    timeline_ms,
                    mono,
                    native_rate,
                };
                (lock_recover(&on_block))(block);
            };
            let err_cb = move |err: cpal::StreamError| {
                on_error(err.to_string());
            };
            device.build_input_stream(&config.clone().into(), data_cb, err_cb, None)
        }};
    }

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::I8 => build!(i8),
        cpal::SampleFormat::U8 => build!(u8),
        other => {
            return Err(classify_open_error(
                source,
                &resolved_name,
                format!("unsupported sample format {other:?}"),
            ))
        }
    }
    .map_err(|e| classify_open_error(source, &resolved_name, e.to_string()))?;

    stream
        .play()
        .map_err(|e| classify_open_error(source, &resolved_name, e.to_string()))?;
    Ok((stream, native_rate))
}

// ===========================================================================
// 7. Session orchestration
// ===========================================================================

#[derive(Default)]
struct SharedState {
    statuses: HashMap<SourceKind, SourceStatus>,
}

/// State that survives a source being closed and reopened (pause/resume, device
/// loss) so the timeline and the sequence counter continue rather than restart.
#[derive(Clone, Copy, Debug, Default)]
struct SourceContinuation {
    written: u64,
    next_sequence: u32,
}

struct OpenSource {
    device_name: String,
    stream: Box<dyn CaptureStream>,
    tx: mpsc::Sender<WriterCmd>,
    join: JoinHandle<()>,
    queued: Arc<AtomicUsize>,
    segment_id: i64,
    lost: Arc<AtomicBool>,
}

/// The dual-source meeting recorder.
pub struct MeetingRecorderImpl {
    backend: Arc<dyn CaptureBackend>,
    fs_impl: Arc<dyn ChunkFs>,
    sink: Arc<dyn RecorderEventSink>,
    layout: Option<SessionLayout>,
    store: Option<Arc<dyn ChunkStateStore>>,
    origin: Instant,
    paused_ms: Arc<Mutex<f64>>,
    pause_started: Option<Instant>,
    state: RecorderState,
    markers: Vec<TimelineMarker>,
    next_segment_id: i64,
    open: HashMap<SourceKind, OpenSource>,
    continuation: HashMap<SourceKind, SourceContinuation>,
    requests: HashMap<SourceKind, SourceOpenRequest>,
    shared: Arc<Mutex<SharedState>>,
    summary: SessionSummary,
}

impl MeetingRecorderImpl {
    /// Recorder wired to real audio hardware and the real filesystem.
    pub fn new() -> Self {
        let origin = Instant::now();
        let paused_ms = Arc::new(Mutex::new(0.0));
        let backend: Arc<dyn CaptureBackend> =
            Arc::new(CpalCaptureBackend::new(origin, Arc::clone(&paused_ms)));
        Self::with_parts(backend, Arc::new(RealChunkFs), origin, paused_ms)
    }

    /// Recorder with injected capture backend and filesystem — the seam the
    /// headless tests use.
    pub fn with_parts(
        backend: Arc<dyn CaptureBackend>,
        fs_impl: Arc<dyn ChunkFs>,
        origin: Instant,
        paused_ms: Arc<Mutex<f64>>,
    ) -> Self {
        Self {
            backend,
            fs_impl,
            sink: Arc::new(NullEventSink),
            layout: None,
            store: None,
            origin,
            paused_ms,
            pause_started: None,
            state: RecorderState::Idle,
            markers: Vec::new(),
            next_segment_id: 1,
            open: HashMap::new(),
            continuation: HashMap::new(),
            requests: HashMap::new(),
            shared: Arc::new(Mutex::new(SharedState::default())),
            summary: SessionSummary::default(),
        }
    }

    fn timeline_now_ms(&self) -> f64 {
        if let Some(started) = self.pause_started {
            // Frozen while paused.
            let elapsed = self.origin.elapsed().as_secs_f64() * 1000.0;
            let paused_now = started.elapsed().as_secs_f64() * 1000.0;
            elapsed - *lock_recover(&self.paused_ms) - paused_now
        } else {
            self.origin.elapsed().as_secs_f64() * 1000.0 - *lock_recover(&self.paused_ms)
        }
    }

    fn require_session(
        &self,
    ) -> Result<(&SessionLayout, &Arc<dyn ChunkStateStore>), RecorderError> {
        match (self.layout.as_ref(), self.store.as_ref()) {
            (Some(layout), Some(store)) => Ok((layout, store)),
            _ => Err(RecorderError::NoActiveSession),
        }
    }

    fn emit(&self, event: RecorderEvent) {
        self.sink.on_event(&event);
    }

    /// Shut a source's stream and writer thread down, collecting its outcome.
    fn take_down(&mut self, source: SourceKind) -> Option<SourceOutcome> {
        let mut open = self.open.remove(&source)?;
        open.stream.stop();
        let (tx, rx) = mpsc::channel();
        let outcome = if open.tx.send(WriterCmd::Finish(tx)).is_ok() {
            rx.recv().ok()
        } else {
            None
        };
        drop(open.tx);
        let _ = open.join.join();
        let outcome = outcome.unwrap_or_default();
        let entry = self.continuation.entry(source).or_default();
        entry.written = outcome.written.max(entry.written);
        entry.next_sequence = outcome.next_sequence.max(entry.next_sequence);
        Some(outcome)
    }

    fn absorb(&mut self, outcome: SourceOutcome) {
        if let Some(error) = &outcome.error {
            log::warn!("meeting recorder: source writer finished with error: {error}");
        }
        self.summary.finalized_chunks.extend(outcome.finalized);
        self.summary.degraded_ranges.extend(outcome.degraded_ranges);
    }

    /// Device currently bound to `source`, if it is open.
    ///
    /// MeetingManager needs this to decide whether a `resume` re-validation
    /// found the *same* device (continue the segment) or a different one (start
    /// a new source segment, contract §6).
    pub fn source_device_name(&self, source: SourceKind) -> Option<&str> {
        self.open.get(&source).map(|o| o.device_name.as_str())
    }

    /// Segment id currently open for `source`.
    pub fn current_segment_id(&self, source: SourceKind) -> Option<i64> {
        self.open.get(&source).map(|o| o.segment_id)
    }

    /// Whether the capture stream for `source` reported a fatal device loss.
    ///
    /// Set by the cpal error callback. MeetingManager polls this to drive the
    /// "Wiederverbinden / Gerät wechseln" repair flow of contract §7.1 without
    /// having to correlate events.
    pub fn is_source_lost(&self, source: SourceKind) -> bool {
        self.open
            .get(&source)
            .map(|o| o.lost.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    /// Capture blocks currently waiting for `source`'s writer thread.
    ///
    /// Hard-bounded by [`tolerances::MAX_QUEUED_CAPTURE_BLOCKS`]; exposed so the
    /// bounded-memory property is observable (and asserted in tests) rather than
    /// merely claimed.
    pub fn queued_capture_blocks(&self, source: SourceKind) -> usize {
        self.open
            .get(&source)
            .map(|o| o.queued.load(Ordering::Acquire))
            .unwrap_or(0)
    }

    fn open_source_inner(
        &mut self,
        source: SourceKind,
        request: SourceOpenRequest,
    ) -> Result<i64, RecorderError> {
        let (layout, store) = self.require_session()?;
        let layout = layout.clone();
        let store = Arc::clone(store);

        if let Some(outcome) = self.take_down(source) {
            self.absorb(outcome);
        }

        let segment_id = self.next_segment_id;
        self.next_segment_id += 1;

        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<WriterCmd>();

        // The capture callback pushes here. It never blocks: when the writer is
        // more than MAX_QUEUED_CAPTURE_BLOCKS behind, blocks are dropped and the
        // resulting hole is silence-filled by ordinary gap bookkeeping. This is
        // the bound that keeps RAM independent of meeting duration.
        let cb_tx = tx.clone();
        let cb_queued = Arc::clone(&queued);
        let on_block = Box::new(move |block: CaptureBlock| {
            if cb_queued.load(Ordering::Acquire) >= tolerances::MAX_QUEUED_CAPTURE_BLOCKS {
                return;
            }
            cb_queued.fetch_add(1, Ordering::AcqRel);
            if cb_tx.send(WriterCmd::Block(block)).is_err() {
                cb_queued.fetch_sub(1, Ordering::AcqRel);
            }
        });

        let lost = Arc::new(AtomicBool::new(false));
        let err_sink = Arc::clone(&self.sink);
        let err_shared = Arc::clone(&self.shared);
        let err_tx = tx.clone();
        let err_lost = Arc::clone(&lost);
        let err_origin = self.origin;
        let err_paused = Arc::clone(&self.paused_ms);
        let on_error = Box::new(move |detail: String| {
            if err_lost.swap(true, Ordering::AcqRel) {
                return;
            }
            let at = err_origin.elapsed().as_secs_f64() * 1000.0 - *lock_recover(&err_paused);
            // Contract §7.1: the open chunk of the lost source is finalized to
            // `ready` *before* the loss is reported as terminal.
            let _ = err_tx.send(WriterCmd::FinalizeOpen);
            let event = match source {
                SourceKind::Microphone => RecorderEvent::MicDeviceLost {
                    segment_id,
                    at_timeline_ms: at,
                    detail: detail.clone(),
                },
                SourceKind::System => RecorderEvent::SystemDeviceLost {
                    segment_id,
                    at_timeline_ms: at,
                    detail: detail.clone(),
                },
            };
            err_sink.on_event(&event);
            {
                let mut shared = lock_recover(&err_shared);
                if let Some(status) = shared.statuses.get_mut(&source) {
                    status.state = SourceState::Lost;
                }
            }
            err_sink.on_event(&RecorderEvent::SourceStateChanged {
                source,
                state: SourceState::Lost,
            });
        });

        let stream = self.backend.open(source, &request, on_block, on_error)?;
        let native_rate = stream.native_sample_rate().max(1);

        fs::create_dir_all(layout.audio_dir()).map_err(|e| RecorderError::ChunkCreateFailed {
            source,
            detail: format!("create audio dir: {e}"),
        })?;

        let cont = self.continuation.get(&source).copied().unwrap_or_default();
        let rollover_ms = request.rollover_target.as_millis().max(1) as u64;
        let cfg = SourceWriterConfig {
            source,
            session_id: layout.session_id.clone(),
            audio_dir: layout.audio_dir(),
            segment_id,
            native_rate,
            rollover_samples: ms_to_samples(rollover_ms as f64).max(1),
            initial_written: cont.written,
            initial_sequence: cont.next_sequence.max(1),
        };

        {
            let mut shared = lock_recover(&self.shared);
            let backlog = shared
                .statuses
                .get(&source)
                .map(|s| s.backlog_chunks_ready)
                .unwrap_or(0);
            shared.statuses.insert(
                source,
                SourceStatus {
                    source,
                    state: SourceState::Active,
                    device_name: Some(request.device_name.clone()),
                    current_chunk_sequence: None,
                    backlog_chunks_ready: backlog,
                },
            );
        }

        let writer = SourceWriter::new(
            cfg,
            store,
            Arc::clone(&self.fs_impl),
            Arc::clone(&self.sink),
            Arc::clone(&self.shared),
            self.origin,
            Arc::clone(&self.paused_ms),
        );
        let thread_queued = Arc::clone(&queued);
        let join = std::thread::Builder::new()
            .name(format!("meeting-writer-{}", source.file_tag()))
            .spawn(move || run_source_writer(writer, rx, thread_queued))
            .map_err(|e| RecorderError::ChunkCreateFailed {
                source,
                detail: format!("failed to spawn writer thread: {e}"),
            })?;

        let at = self.timeline_now_ms();
        self.requests.insert(source, request.clone());
        self.open.insert(
            source,
            OpenSource {
                device_name: request.device_name.clone(),
                stream,
                tx,
                join,
                queued,
                segment_id,
                lost,
            },
        );

        self.emit(RecorderEvent::SourceSegmentStarted {
            source,
            segment_id,
            device_name: request.device_name,
            at_timeline_ms: at,
        });
        self.emit(RecorderEvent::SourceStateChanged {
            source,
            state: SourceState::Active,
        });
        Ok(segment_id)
    }

    /// Recovery step §8.2 for a single open chunk row.
    fn recover_one(
        &self,
        layout: &SessionLayout,
        store: &Arc<dyn ChunkStateStore>,
        descriptor: &ChunkDescriptor,
        report: &mut RecoveryReport,
    ) -> Result<(), RecorderError> {
        let name = format!(
            "{:06}-{}.wav",
            descriptor.sequence,
            descriptor.source.file_tag()
        );
        let audio_dir = layout.audio_dir();
        let part = audio_dir.join(format!("{name}.part"));
        let final_path = audio_dir.join(&name);

        // Crash after rename, before DB commit: the file is already valid.
        if final_path.exists() && !part.exists() {
            return finish_recovered(store, descriptor, &final_path, &name, report);
        }

        if !part.exists() {
            // §8.2c: row exists, file missing.
            store
                .transition(
                    descriptor.chunk_id,
                    ChunkState::Writing,
                    ChunkState::Damaged,
                    &ChunkPatch {
                        recovered: Some(true),
                        ..Default::default()
                    },
                )
                .map_err(|detail| RecorderError::RecoveryFailed {
                    chunk_id: descriptor.chunk_id,
                    detail,
                })?;
            report.damaged.push(descriptor.chunk_id);
            return Ok(());
        }

        let file_len = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let mut header = [0u8; WAV_HEADER_LEN];
        let parsed = File::open(&part)
            .and_then(|mut f| f.read_exact(&mut header).map(|_| ()))
            .ok()
            .and_then(|_| parse_wav_header(&header));

        let Some(declared) = parsed else {
            // §8.2b: unparseable/headerless/too short.
            let quarantine = layout.recovery_dir();
            let target = quarantine.join(format!("{name}.part"));
            // Only report a file as quarantined if it actually moved. Both
            // results were previously discarded while `target` was pushed into
            // the report regardless, so a failed move left the caller told a
            // file sits in recovery/ that is still in place.
            let quarantined = fs::create_dir_all(&quarantine)
                .and_then(|()| fs::rename(&part, &target))
                .is_ok();
            store
                .transition(
                    descriptor.chunk_id,
                    ChunkState::Writing,
                    ChunkState::Damaged,
                    &ChunkPatch {
                        recovered: Some(true),
                        ..Default::default()
                    },
                )
                .map_err(|detail| RecorderError::RecoveryFailed {
                    chunk_id: descriptor.chunk_id,
                    detail,
                })?;
            report.damaged.push(descriptor.chunk_id);
            if quarantined {
                report.quarantined_files.push(target);
            }
            return Ok(());
        };

        // §8.2a: truncate to min(declared, actual) rounded down to whole PCM16
        // frames, patch the header, fsync, atomic rename.
        let actual_payload = file_len.saturating_sub(WAV_HEADER_LEN as u64);
        let usable = actual_payload.min(u64::from(declared)) / BYTES_PER_SAMPLE * BYTES_PER_SAMPLE;
        if usable == 0 {
            let quarantine = layout.recovery_dir();
            let target = quarantine.join(format!("{name}.part"));
            // Only report a file as quarantined if it actually moved. Both
            // results were previously discarded while `target` was pushed into
            // the report regardless, so a failed move left the caller told a
            // file sits in recovery/ that is still in place.
            let quarantined = fs::create_dir_all(&quarantine)
                .and_then(|()| fs::rename(&part, &target))
                .is_ok();
            store
                .transition(
                    descriptor.chunk_id,
                    ChunkState::Writing,
                    ChunkState::Damaged,
                    &ChunkPatch {
                        recovered: Some(true),
                        ..Default::default()
                    },
                )
                .map_err(|detail| RecorderError::RecoveryFailed {
                    chunk_id: descriptor.chunk_id,
                    detail,
                })?;
            report.damaged.push(descriptor.chunk_id);
            if quarantined {
                report.quarantined_files.push(target);
            }
            return Ok(());
        }

        let repair = || -> io::Result<()> {
            let mut f = OpenOptions::new().read(true).write(true).open(&part)?;
            let data_bytes = u32::try_from(usable).unwrap_or(PROVISIONAL_DATA_BYTES);
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&wav_header(data_bytes))?;
            f.set_len(WAV_HEADER_LEN as u64 + usable)?;
            f.flush()?;
            f.sync_all()
        };
        repair().map_err(|e| RecorderError::RecoveryFailed {
            chunk_id: descriptor.chunk_id,
            detail: format!("header repair: {e}"),
        })?;
        self.fs_impl
            .rename(&part, &final_path)
            .map_err(|e| RecorderError::RecoveryFailed {
                chunk_id: descriptor.chunk_id,
                detail: format!("rename: {e}"),
            })?;
        let _ = self.fs_impl.sync_dir(&audio_dir);

        finish_recovered(store, descriptor, &final_path, &name, report)
    }
}

fn finish_recovered(
    store: &Arc<dyn ChunkStateStore>,
    descriptor: &ChunkDescriptor,
    final_path: &Path,
    name: &str,
    report: &mut RecoveryReport,
) -> Result<(), RecorderError> {
    let len = fs::metadata(final_path).map(|m| m.len()).unwrap_or(0);
    let sample_count = len.saturating_sub(WAV_HEADER_LEN as u64) / BYTES_PER_SAMPLE;
    let checksum = sha256_file(final_path).ok();
    let start_ms = descriptor.range.start_ms;
    let patch = ChunkPatch {
        storage_key: Some(format!("audio/{name}")),
        range: Some(TimelineRange {
            start_ms,
            end_ms: start_ms + samples_to_ms(sample_count),
        }),
        sample_count: Some(sample_count),
        checksum_sha256: checksum,
        recovered: Some(true),
        gap_stats: None,
    };
    store
        .transition(
            descriptor.chunk_id,
            ChunkState::Writing,
            ChunkState::Ready,
            &patch,
        )
        .map_err(|detail| RecorderError::RecoveryFailed {
            chunk_id: descriptor.chunk_id,
            detail,
        })?;
    report.finalized.push(descriptor.chunk_id);
    Ok(())
}

impl Default for MeetingRecorderImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl MeetingRecorder for MeetingRecorderImpl {
    fn preflight(&self, devices: &SourceDeviceSet) -> Result<Vec<PreflightCheck>, RecorderError> {
        let mut checks = Vec::new();
        for source in SourceKind::all() {
            if let Some(request) = devices.get(source) {
                checks.push(self.backend.preflight(source, request));
            }
        }
        Ok(checks)
    }

    fn start_session(
        &mut self,
        layout: SessionLayout,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<(), RecorderError> {
        if self.state != RecorderState::Idle {
            return Err(RecorderError::SessionAlreadyActive);
        }
        fs::create_dir_all(layout.audio_dir()).map_err(|e| RecorderError::ChunkCreateFailed {
            source: SourceKind::Microphone,
            detail: format!("create audio dir: {e}"),
        })?;

        self.state = RecorderState::Recovering;
        self.layout = Some(layout.clone());
        self.store = Some(Arc::clone(&store));
        // Stale `writing` chunks from a crashed process are reconciled before a
        // single new sample is written (contract §8).
        let _ = self.recover_open_chunks(&layout.session_id, Arc::clone(&store));

        self.origin = Instant::now();
        *lock_recover(&self.paused_ms) = 0.0;
        self.pause_started = None;
        self.markers.clear();
        self.continuation.clear();
        self.requests.clear();
        self.summary = SessionSummary::default();
        lock_recover(&self.shared).statuses.clear();
        self.state = RecorderState::Recording;
        self.emit(RecorderEvent::Started {
            at_timeline_ms: 0.0,
        });
        Ok(())
    }

    fn open_source(
        &mut self,
        source: SourceKind,
        request: SourceOpenRequest,
    ) -> Result<i64, RecorderError> {
        if self.state != RecorderState::Recording {
            return Err(RecorderError::NoActiveSession);
        }
        self.open_source_inner(source, request)
    }

    fn close_source(
        &mut self,
        source: SourceKind,
        reason: StopReason,
    ) -> Result<(), RecorderError> {
        let Some(outcome) = self.take_down(source) else {
            return Ok(());
        };
        self.absorb(outcome);
        self.requests.remove(&source);
        {
            let mut shared = lock_recover(&self.shared);
            if let Some(status) = shared.statuses.get_mut(&source) {
                status.state = SourceState::Closed;
            }
        }
        self.emit(RecorderEvent::SourceStateChanged {
            source,
            state: SourceState::Closed,
        });
        if reason != StopReason::Requested {
            self.emit(RecorderEvent::ControlledStopInitiated { reason });
        }
        Ok(())
    }

    fn pause(&mut self) -> Result<TimelineMarker, RecorderError> {
        if self.state != RecorderState::Recording {
            return Err(RecorderError::NoActiveSession);
        }
        // Freeze the clock *before* tearing the sources down: joining the writer
        // threads takes tens of milliseconds, and that time belongs to the pause,
        // not to the timeline. Otherwise the resume marker would land after the
        // pause marker even though no audio was captured in between.
        self.pause_started = Some(Instant::now());
        let timeline_ms = self.timeline_now_ms();
        for source in SourceKind::all() {
            if let Some(outcome) = self.take_down(source) {
                self.absorb(outcome);
            }
        }
        self.state = RecorderState::Paused;
        let marker = TimelineMarker {
            kind: MarkerKind::Pause,
            timeline_ms,
            wall_clock: SystemTime::now(),
        };
        self.markers.push(marker.clone());
        self.emit(RecorderEvent::Paused {
            marker: marker.clone(),
        });
        Ok(marker)
    }

    fn resume(&mut self, devices: SourceDeviceSet) -> Result<TimelineMarker, RecorderError> {
        if self.state != RecorderState::Paused {
            return Err(RecorderError::NoActiveSession);
        }
        // The paused span is removed from the timeline, so the resume marker
        // lands at exactly the pause position (contract §6 pause semantics 3).
        if let Some(started) = self.pause_started.take() {
            *lock_recover(&self.paused_ms) += started.elapsed().as_secs_f64() * 1000.0;
        }
        self.state = RecorderState::Recording;
        let timeline_ms = self.timeline_now_ms();

        let mut first_error = None;
        for source in SourceKind::all() {
            if let Some(request) = devices.get(source).cloned() {
                if let Err(e) = self.open_source_inner(source, request) {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        if let Some(e) = first_error {
            // Re-validation failed: stay paused, per contract §6.
            for source in SourceKind::all() {
                if let Some(outcome) = self.take_down(source) {
                    self.absorb(outcome);
                }
            }
            self.pause_started = Some(Instant::now());
            self.state = RecorderState::Paused;
            return Err(e);
        }

        let marker = TimelineMarker {
            kind: MarkerKind::Resume,
            timeline_ms,
            wall_clock: SystemTime::now(),
        };
        self.markers.push(marker.clone());
        self.emit(RecorderEvent::Resumed {
            marker: marker.clone(),
        });
        Ok(marker)
    }

    fn stop(&mut self) -> Result<SessionSummary, RecorderError> {
        if self.state == RecorderState::Idle {
            return Err(RecorderError::NoActiveSession);
        }
        self.state = RecorderState::Stopping;
        for source in SourceKind::all() {
            if let Some(outcome) = self.take_down(source) {
                self.absorb(outcome);
            }
        }
        if self.pause_started.take().is_some() {
            // nothing further: the timeline stays frozen at its last position
        }
        self.summary.stopped_reason = Some(StopReason::Requested);
        self.state = RecorderState::Idle;
        let summary = self.summary.clone();
        self.emit(RecorderEvent::Stopped {
            finalized_chunks: summary.finalized_chunks.len(),
        });
        self.layout = None;
        self.store = None;
        Ok(summary)
    }

    fn subscribe(&mut self, sink: Arc<dyn RecorderEventSink>) {
        self.sink = sink;
    }

    fn snapshot(&self) -> RecorderSnapshot {
        let shared = lock_recover(&self.shared);
        let mut sources: Vec<SourceStatus> = SourceKind::all()
            .iter()
            .filter_map(|s| shared.statuses.get(s).cloned())
            .collect();
        sources.sort_by_key(|s| match s.source {
            SourceKind::Microphone => 0,
            SourceKind::System => 1,
        });
        RecorderSnapshot {
            state: self.state,
            timeline_now_ms: self.timeline_now_ms(),
            sources,
            markers: self.markers.clone(),
        }
    }

    fn recover_open_chunks(
        &self,
        session_id: &str,
        store: Arc<dyn ChunkStateStore>,
    ) -> Result<RecoveryReport, RecorderError> {
        let layout = match self.layout.as_ref() {
            Some(layout) => layout.clone(),
            None => return Err(RecorderError::NoActiveSession),
        };
        let mut report = RecoveryReport::default();
        let open =
            store
                .open_chunks(session_id)
                .map_err(|detail| RecorderError::RecoveryFailed {
                    chunk_id: 0,
                    detail,
                })?;

        for descriptor in &open {
            self.recover_one(&layout, &store, descriptor, &mut report)?;
        }

        // §8.3 reverse reconciliation: quarantine `.part` files that no row
        // claims. Orphan `.wav` files are left in place — they are valid audio;
        // adopting them into rows is MeetingStore's job because only it can mint
        // ids for rows that never existed.
        let audio_dir = layout.audio_dir();
        if let Ok(entries) = fs::read_dir(&audio_dir) {
            let claimed: Vec<String> = open
                .iter()
                .map(|d| format!("{:06}-{}.wav.part", d.sequence, d.source.file_tag()))
                .collect();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".wav.part") || claimed.contains(&name) {
                    continue;
                }
                let quarantine = layout.recovery_dir();
                let _ = fs::create_dir_all(&quarantine);
                let target = quarantine.join(&name);
                if fs::rename(entry.path(), &target).is_ok() {
                    report.quarantined_files.push(target);
                }
            }
        }
        Ok(report)
    }
}

impl Drop for MeetingRecorderImpl {
    fn drop(&mut self) {
        for source in SourceKind::all() {
            let _ = self.take_down(source);
        }
    }
}

// ===========================================================================
// 8. Tests
// ===========================================================================
//
// Every test below runs headless: audio hardware is replaced by `FakeBackend`,
// which hands the recorder synthetic `CaptureBlock`s with explicit timeline
// stamps. That is what makes the timeline/gap/drift/rollover/recovery logic
// assertable on CI (ubuntu-24.04) as well as on the Windows target.

#[cfg(test)]
mod tests;
