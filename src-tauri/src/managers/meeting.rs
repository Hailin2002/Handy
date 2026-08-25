//! MeetingManager — the meeting session state machine and its persistence.
//!
//! Implements `docs/meeting-mode-architecture/session-state-contract.md` (SSC):
//! the manager is the **single authoritative owner** of the session state
//! machine (SSC §1). It is the only component that decides transitions, the
//! only one that writes `meeting_sessions.state`, and the only one that
//! publishes `meeting://*` events toward the Tauri layer.
//!
//! Layering:
//!
//! - **Persistence** is delegated to [`super::meeting_store`]
//!   (`SqliteMeetingStore`, the one migration chain for `meetings.db`). The
//!   wave-1 CRUD journey (create → open → save → list) keeps its own small SQL
//!   surface here, unchanged, so the shipped Meetings UI does not regress.
//! - **Capture** is consumed strictly behind the recorder contract
//!   (`docs/meeting-mode-architecture/recorder-contract.md`): this module only
//!   ever talks to the [`rec::MeetingRecorder`] trait, never to a concrete
//!   capture implementation. The single production binding lives in
//!   [`default_recorder_factory`] — see the INTEGRATION POINT marker there.
//! - **Events** go through the [`MeetingEventSink`] seam so the state machine
//!   is testable without a Tauri runtime, and so rule O1 ("persist before
//!   publish") is enforced in exactly one place ([`MeetingManager::transition`]).
//!
//! Deviations from the contract that are forced by the current schema (the
//! meeting migration chain is append-only and owned elsewhere) are marked
//! `SCHEMA NOTE` in the code:
//!
//! - `state_revision` is the tier-A envelope `revision` of `meeting_sessions`
//!   (there is no separate `state_revision` column). It is persisted, strictly
//!   monotone per session and survives restarts, which is what O2/O3/O5 need.
//! - `previous_active_state` (SSC T17's proposed column) is kept in memory
//!   only; after a restart the reconciliation protocol of §6 decides the
//!   disposition instead, which is the same outcome the column would produce.

use super::meeting_store::{
    meeting_db_path, open_meeting_database, ChunkFacts, ChunkState as StoreChunkState,
    MeetingRepository, NewAudioChunk, NewSttJob, SessionPatch, SqliteMeetingStore, StoreError,
    Uuid as StoreUuid,
};
use super::meeting_transcription::{
    MeetingSttEngine, MeetingTranscriptionQueue, QueueConfig, QueueDelegate,
};
use crate::audio_toolkit::audio::meeting_recorder as rec;
use crate::managers::meeting_export as export;
use crate::managers::meeting_summary as summarize;
use crate::storage::asset_store::AssetStore as _;

/// Controlled-deletion scope (plan §11): keep the transcript but drop the raw
/// recording, or remove everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DeleteScope {
    AudioOnly,
    Everything,
}

fn string_list_of(value: &serde_json::Value) -> Vec<String> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}
use anyhow::Result;
use log::{debug, error, info, warn};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tauri::AppHandle;

/// Session lifecycle state assigned at creation (kept for the wave-1 CRUD
/// callers that predate the state machine).
pub const SESSION_STATE_DRAFT: &str = "draft";

/// Timeline sample rate of the meeting capture format (recorder contract §2:
/// 16 kHz mono per source). Mirrored as a local constant so this module does
/// not depend on the tolerance table's private layout.
const TIMELINE_SAMPLE_RATE_HZ: u32 = 16_000;

/// Free-space watermarks from `disk-guard-contract.md` §3.1. Local STT needs
/// the warn watermark; cloud STT additionally reserves the upload budget.
const DISK_STOP_WATERMARK_BYTES: u64 = 4_888_576;
const DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL: u64 = 235_288_576;
const CLOUD_UPLOAD_TRANSIENT_RESERVE_BYTES: u64 = 500_000_000;

// ---------------------------------------------------------------------------
// Event topics (live-view-streaming-contract §2, frontend `src/lib/meetingIpc.ts`)
// ---------------------------------------------------------------------------

/// `meeting://session-created` — a draft row now exists (SSC T01).
pub const TOPIC_SESSION_CREATED: &str = "meeting://session-created";
/// `meeting://session-state` — every published transition (SSC §2.2).
pub const TOPIC_SESSION_STATE: &str = "meeting://session-state";
/// `meeting://session-error` — redacted, explainable failure (SSC T04/T13/T17).
pub const TOPIC_SESSION_ERROR: &str = "meeting://session-error";
/// `meeting://level-update` — capture levels per source (LV §2.3).
pub const TOPIC_LEVEL_UPDATE: &str = "meeting://level-update";
/// `meeting://transcript-delta` — append-only segment batches (LV §2.1).
/// Published by the transcription queue leaf; declared here because this
/// module owns the topic registry.
#[allow(dead_code)]
pub const TOPIC_TRANSCRIPT_DELTA: &str = "meeting://transcript-delta";
/// `meeting://jobs-updated` — aggregate job counters (LV §2.2).
pub const TOPIC_JOBS_UPDATED: &str = "meeting://jobs-updated";
/// `meeting://summary-available` — summary persisted (SSC T12).
/// Published by the summary leaf; see the note on the transcript topic.
#[allow(dead_code)]
pub const TOPIC_SUMMARY_AVAILABLE: &str = "meeting://summary-available";
/// `meeting://session-recovered` — one recovered session (SSC §6.2 R6).
pub const TOPIC_SESSION_RECOVERED: &str = "meeting://session-recovered";
/// `meeting://recovery-report` — the consolidated boot report.
pub const TOPIC_RECOVERY_REPORT: &str = "meeting://recovery-report";

// ---------------------------------------------------------------------------
// State machine (SSC §2)
// ---------------------------------------------------------------------------

/// The SSC §2.1 state set, verbatim. Serialization matches
/// `MeetingSessionState` in `src/lib/types/meeting.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingState {
    /// Start dialog open, nothing captured yet.
    Draft,
    /// Validation/opening of devices in progress.
    Starting,
    /// Configured sources capturing; chunks rolling.
    Recording,
    /// Capture frozen at a marker; nothing stored or transmitted.
    Paused,
    /// Controlled finalization of all open chunks.
    Stopping,
    /// Capture done; STT jobs outstanding.
    Transcribing,
    /// Optional; summary provider running over the complete transcript.
    Summarizing,
    /// All jobs succeeded; terminal success.
    Completed,
    /// User aborted after audio was secured; processing not started.
    StoppedUnprocessed,
    /// User aborted before/at start; terminal.
    Cancelled,
    /// Data-preserving failure that blocks progress.
    RecoverableError,
}

impl MeetingState {
    /// Persisted/wire token. Must stay identical to the TypeScript union.
    pub const fn as_str(self) -> &'static str {
        match self {
            MeetingState::Draft => "draft",
            MeetingState::Starting => "starting",
            MeetingState::Recording => "recording",
            MeetingState::Paused => "paused",
            MeetingState::Stopping => "stopping",
            MeetingState::Transcribing => "transcribing",
            MeetingState::Summarizing => "summarizing",
            MeetingState::Completed => "completed",
            MeetingState::StoppedUnprocessed => "stopped_unprocessed",
            MeetingState::Cancelled => "cancelled",
            MeetingState::RecoverableError => "recoverable_error",
        }
    }

    /// Parses a persisted token. Unknown tokens are rejected rather than
    /// silently coerced, so a corrupt row cannot masquerade as `draft`.
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "draft" => MeetingState::Draft,
            "starting" => MeetingState::Starting,
            "recording" => MeetingState::Recording,
            "paused" => MeetingState::Paused,
            "stopping" => MeetingState::Stopping,
            "transcribing" => MeetingState::Transcribing,
            "summarizing" => MeetingState::Summarizing,
            "completed" => MeetingState::Completed,
            "stopped_unprocessed" => MeetingState::StoppedUnprocessed,
            "cancelled" => MeetingState::Cancelled,
            "recoverable_error" => MeetingState::RecoverableError,
            _ => return None,
        })
    }

    /// Active = a capture session exists and is owned by a live process
    /// (SSC §6.2 R1 candidate set).
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            MeetingState::Starting
                | MeetingState::Recording
                | MeetingState::Paused
                | MeetingState::Stopping
        )
    }

    /// True only while audio is actually being written (drives the recording
    /// indicator, the tray state and the elapsed clock).
    pub const fn is_capturing(self) -> bool {
        matches!(self, MeetingState::Recording)
    }

    /// Terminal states never leave except through an explicitly separate flow.
    #[allow(dead_code)]
    pub const fn is_terminal(self) -> bool {
        matches!(self, MeetingState::Completed | MeetingState::Cancelled)
    }
}

/// The SSC §2.2 transition table. Everything not listed here is forbidden;
/// [`MeetingManager::transition`] is the only caller.
pub fn transition_allowed(from: MeetingState, to: MeetingState) -> bool {
    use MeetingState::*;
    matches!(
        (from, to),
        // T02 / T16
        (Draft, Starting)
            | (Draft, Cancelled)
            // T03 / T04 / T16 / T17 / recovery R5(a)
            | (Starting, Recording)
            | (Starting, Draft)
            | (Starting, Cancelled)
            | (Starting, RecoverableError)
            | (Starting, StoppedUnprocessed)
            // T05 / T07 / T17 / recovery R5(a)
            | (Recording, Paused)
            | (Recording, Stopping)
            | (Recording, RecoverableError)
            | (Recording, StoppedUnprocessed)
            // T06 / T07 / T17 / recovery R5(a)
            | (Paused, Recording)
            | (Paused, Stopping)
            | (Paused, RecoverableError)
            | (Paused, StoppedUnprocessed)
            // T08 / T09 / T15 / T17
            | (Stopping, Transcribing)
            | (Stopping, Completed)
            | (Stopping, StoppedUnprocessed)
            | (Stopping, RecoverableError)
            // T10 / T11 / T13
            | (Transcribing, Summarizing)
            | (Transcribing, Completed)
            | (Transcribing, RecoverableError)
            // T12 / T13
            | (Summarizing, Completed)
            | (Summarizing, RecoverableError)
            // T14
            | (RecoverableError, Transcribing)
            | (RecoverableError, Summarizing)
            | (RecoverableError, Completed)
            // T18
            | (StoppedUnprocessed, Transcribing)
            | (StoppedUnprocessed, Completed)
    )
}

// ---------------------------------------------------------------------------
// Wire payloads — field-for-field mirrors of `src/lib/types/meeting.ts`
// ---------------------------------------------------------------------------

/// Capture sources, kept apart everywhere (plan §8 two-track timeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingAudioSource {
    /// Local microphone track.
    Microphone,
    /// System/loopback track.
    System,
}

impl MeetingAudioSource {
    /// Wire token, also used as the `meeting_audio_chunks.source` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            MeetingAudioSource::Microphone => "microphone",
            MeetingAudioSource::System => "system",
        }
    }

    fn from_recorder(kind: rec::SourceKind) -> Self {
        match kind {
            rec::SourceKind::Microphone => MeetingAudioSource::Microphone,
            rec::SourceKind::System => MeetingAudioSource::System,
        }
    }
}

/// `meeting://session-created`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSessionCreatedEvent {
    /// Session id.
    pub meeting_id: String,
    /// Title as persisted.
    pub title: String,
    /// Always `draft` (T01).
    pub state: MeetingState,
    /// Revision of the created row.
    pub state_revision: i64,
}

/// `meeting://session-state` (SSC §2.2, carries `state_revision` per O2).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSessionStateEvent {
    /// Session id.
    pub meeting_id: String,
    /// Previous state; absent for the very first publication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<MeetingState>,
    /// New state.
    pub to: MeetingState,
    /// Strictly increasing per session (O2).
    pub state_revision: i64,
    /// Set when the session captures the microphone only (plan E5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microphone_only: Option<bool>,
}

/// `meeting://session-error`. Messages are pre-redacted (O7).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSessionErrorEvent {
    /// Session id.
    pub meeting_id: String,
    /// Taxonomy code, e.g. `micLost`, `diskFull`, `consentRequired`.
    pub code: String,
    /// Already-redacted detail; never a raw backend error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Revision at the time of the failure, when a session row exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<i64>,
    /// False ⇒ the session cannot continue without user action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recoverable: Option<bool>,
}

/// `meeting://level-update` (LV §2.3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingLevelUpdateEvent {
    /// Session id.
    pub meeting_id: String,
    /// Which track the level belongs to.
    pub source: MeetingAudioSource,
    /// Peak amplitude, 0..1.
    pub peak: f32,
    /// RMS amplitude, 0..1.
    pub rms: f32,
    /// Timeline position of the measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_timeline_ms: Option<f64>,
}

/// One persisted, finished transcript segment (LV §2.1). Filled by the
/// transcription leaf; declared here so the wire shape has one owner.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingTranscriptSegmentDto {
    /// Segment id.
    pub id: String,
    /// Producing STT job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Capture source.
    pub source: MeetingAudioSource,
    /// Start on the audio timeline.
    pub start_ms: i64,
    /// End on the audio timeline.
    pub end_ms: i64,
    /// Ordering within the meeting.
    pub sequence: i64,
    /// Recognized text.
    pub text: String,
    /// Recognition confidence, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// `meeting://transcript-delta` (LV §2.1) — append-only.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingTranscriptDeltaEvent {
    /// Session id.
    pub meeting_id: String,
    /// Newly available segments, ascending by `sequence`.
    pub segments: Vec<MeetingTranscriptSegmentDto>,
    /// Highest sequence contained in this delta.
    pub last_sequence: i64,
    /// Revision at emission time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<i64>,
    /// Set when the backend dropped segments to bound the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// Per-source job counters (LV §2.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct MeetingJobCounts {
    /// Waiting for a worker.
    pub queued: i64,
    /// Claimed and running.
    pub running: i64,
    /// Backing off before the next attempt.
    pub retry_wait: i64,
    /// Terminally failed.
    pub failed: i64,
    /// Completed successfully.
    pub succeeded: i64,
}

/// Oldest failure block of `meeting://jobs-updated`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingOldestFailed {
    /// How many jobs are in `failed`.
    pub count: i64,
    /// RFC3339 instant of the first failure, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_failed_at: Option<String>,
}

/// `meeting://jobs-updated` (LV §2.2) — aggregates only, never per-job churn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct MeetingJobsUpdatedEvent {
    /// Session id.
    pub meeting_id: String,
    /// Counters keyed by `MeetingAudioSource` token.
    pub per_source: HashMap<String, MeetingJobCounts>,
    /// Jobs that are neither succeeded nor failed.
    pub total_backlog: i64,
    /// Failure summary, when at least one job failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_failed: Option<MeetingOldestFailed>,
    /// Revision at emission time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<i64>,
    /// Processing-checklist totals (plan §6.12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible_total: Option<i64>,
    /// Successfully transcribed units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub succeeded_total: Option<i64>,
    /// Chunks prepared for processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepared_total: Option<i64>,
    /// Chunks already prepared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepared_done: Option<i64>,
}

/// `meeting://summary-available` (SSC T12).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSummaryAvailableEvent {
    /// Session id.
    pub meeting_id: String,
    /// Revision at emission time.
    pub state_revision: i64,
}

/// Why a session was interrupted (startup-recovery-dataflow §3 S7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingRecoveryCause {
    /// The process died while the session was active.
    Crash,
    /// The machine slept and the capture never resumed.
    SleepWake,
    /// Cause could not be determined.
    Unknown,
}

/// One pending recovery offer (`meeting_list_recoveries`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingRecoveryOffer {
    /// Session id.
    pub meeting_id: String,
    /// Session title, for the banner.
    pub title: String,
    /// Interruption cause.
    pub cause: MeetingRecoveryCause,
    /// Chunks that survived and are offered for processing.
    pub recovered_chunks: i64,
    /// Chunks that could not be repaired.
    pub damaged_chunks: i64,
    /// Files quarantined into `recovery/`.
    pub quarantined_files: i64,
    /// RFC3339 instant of the last observed update before the interruption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted_at: Option<String>,
}

/// `meeting://recovery-report` and `meeting://session-recovered`.
///
/// Both topics share this shape because the frontend applies them with the
/// same reducer (`meetingStore.applyRecoveryReport`); the per-session topic
/// simply carries a single-element `offers` array plus the SSC §6.2 R6 fields.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingRecoveryReportEvent {
    /// All offers the user may act on.
    pub offers: Vec<MeetingRecoveryOffer>,
    /// Set on `meeting://session-recovered` (R6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<String>,
    /// Revision after the disposition commit (R6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<i64>,
}

/// How the user resolves a recovery offer (startup-recovery §5 U2–U4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingRecoveryResolution {
    /// Keep the audio and start processing.
    Recover,
    /// Decide later; the offer stays.
    Later,
    /// Delete the session and its assets.
    Delete,
}

/// One row of the compact start dialog (plan §6.7 / "Startvalidierung").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MeetingPreflightCheckId {
    /// Microphone device row.
    Microphone,
    /// System/loopback device row.
    SystemAudio,
    /// STT provider/model row.
    Transcription,
    /// Recognition language row.
    Language,
    /// Free disk space row.
    Disk,
    /// OS capture permission row.
    Permission,
}

/// Traffic light of one preflight row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingPreflightStatus {
    /// Ready to record.
    Ok,
    /// Usable, but degraded (e.g. mic-only fallback).
    Warning,
    /// Blocks the start button.
    Error,
}

/// One preflight row.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingPreflightCheck {
    /// Row identity.
    pub id: MeetingPreflightCheckId,
    /// Traffic light.
    pub status: MeetingPreflightStatus,
    /// Human-readable current value ("Default microphone", "Deutsch", …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Error taxonomy code so the UI can pick the repair copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Pre-redacted detail line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Result of `preflight_meeting`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingPreflightReport {
    /// One row per [`MeetingPreflightCheckId`].
    pub checks: Vec<MeetingPreflightCheck>,
    /// The start button is only enabled once this is true.
    pub can_start: bool,
    /// Loopback unavailable ⇒ the user may explicitly fall back (plan E5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microphone_only_available: Option<bool>,
}

/// Compact error projection inside [`MeetingSnapshot`].
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSnapshotError {
    /// Taxonomy code.
    pub code: String,
    /// Redacted detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `get_meeting_snapshot` (SSC §5 / LV §6.2). The UI is a pure function of
/// this snapshot plus subsequent events.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingSnapshot {
    /// Session id.
    pub meeting_id: String,
    /// Current title.
    pub title: String,
    /// Current state.
    pub state: MeetingState,
    /// Current revision (O5 handshake value).
    pub state_revision: i64,
    /// Banked capture milliseconds (pause-corrected, excludes the running leg).
    pub elapsed_ms: i64,
    /// Epoch ms at which the current capture leg started; null while frozen.
    pub running_since: Option<i64>,
    /// Whether the session captures the microphone only.
    pub microphone_only: Option<bool>,
    /// Where processing happens.
    pub provider_kind: String,
    /// Provider display name.
    pub provider_name: Option<String>,
    /// Whether a summary is requested.
    pub summary_enabled: Option<bool>,
    /// Highest persisted transcript sequence.
    pub transcript_head_sequence: Option<i64>,
    /// Current job aggregates.
    pub jobs: Option<MeetingJobsUpdatedEvent>,
    /// Last observed levels per source token.
    pub levels: Option<HashMap<String, f32>>,
    /// Last operator-visible error.
    pub last_error: Option<MeetingSnapshotError>,
}

// ---------------------------------------------------------------------------
// Event sink seam (rule O1 lives in exactly one place)
// ---------------------------------------------------------------------------

/// Publication seam toward the Tauri layer.
///
/// Exists so the state machine can be unit-tested without a Tauri runtime and
/// so every publication funnels through one call site that runs *after* the
/// persistence commit (SSC O1).
pub trait MeetingEventSink: Send + Sync {
    /// Publishes one already-serialized payload on `topic`.
    fn publish(&self, topic: &'static str, payload: serde_json::Value);
}

/// Production sink: emits on the Tauri event bus.
///
/// The `meeting://*` topics carry a scheme separator, which tauri-specta's
/// `#[derive(Event)]` cannot produce (it kebab-cases the struct ident), so the
/// payloads are emitted by name instead of through `collect_events!`. The
/// frontend subscribes with `listen()` on exactly these constants.
pub struct TauriEventSink {
    app: AppHandle,
}

impl TauriEventSink {
    /// Wraps an app handle.
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl MeetingEventSink for TauriEventSink {
    fn publish(&self, topic: &'static str, payload: serde_json::Value) {
        use tauri::Emitter;
        if let Err(e) = self.app.emit(topic, payload) {
            warn!("Failed to emit {}: {}", topic, e);
        }
    }
}

/// Sink used when no Tauri runtime is available (tests, headless).
#[allow(dead_code)]
pub struct NullEventSink;

impl MeetingEventSink for NullEventSink {
    fn publish(&self, _topic: &'static str, _payload: serde_json::Value) {}
}

// ---------------------------------------------------------------------------
// Environment seam (Startvalidierung inputs the recorder does not own)
// ---------------------------------------------------------------------------

/// Non-device start validation inputs (recorder-contract §7.2 assigns these to
/// the manager).
#[derive(Debug, Clone)]
pub struct ProviderSnapshot {
    /// `local` or `cloud`.
    pub kind: String,
    /// Display name of the STT provider.
    pub name: String,
    /// Recognition language label, if configured.
    pub language: Option<String>,
    /// False ⇒ keys/model missing; blocks the start button.
    pub configured: bool,
    /// Summary provider id, when summaries are enabled. Consumed by the
    /// summary leaf's own preflight row.
    #[allow(dead_code)]
    pub summary_provider: Option<String>,
    /// Retention policy token persisted with the session, consumed by the
    /// retention leaf.
    #[allow(dead_code)]
    pub retention_policy: String,
}

impl Default for ProviderSnapshot {
    fn default() -> Self {
        Self {
            kind: "local".to_string(),
            name: "Local".to_string(),
            language: None,
            configured: true,
            summary_provider: None,
            retention_policy: "default".to_string(),
        }
    }
}

/// Injected summary LLM handle: `(provider_id, model, executor)`.
pub type SummaryLlmConfig = (String, String, Box<dyn summarize::SummaryLlm>);

/// Everything the manager needs to validate a start that is not a device fact.
pub trait MeetingEnvironment: Send + Sync {
    /// The configured capture devices.
    fn device_set(&self) -> rec::SourceDeviceSet;
    /// Device availability, delegated to the recorder's own preflight.
    fn device_checks(&self, devices: &rec::SourceDeviceSet) -> Vec<rec::PreflightCheck>;
    /// Free bytes on the volume that holds the meeting assets.
    fn free_disk_bytes(&self) -> Option<u64>;
    /// STT/summary configuration snapshot.
    fn provider(&self) -> ProviderSnapshot;
    /// Effective STT provider for a session being started. The default
    /// ignores the per-meeting override; the real environment honors it
    /// (plan §13 Phase 6: Local/Azure/Custom chosen per meeting).
    fn provider_for(
        &self,
        override_provider: Option<crate::settings::SpeechInputProvider>,
    ) -> ProviderSnapshot {
        let _ = override_provider;
        self.provider()
    }
    /// OS-level capture permission.
    fn microphone_permission_granted(&self) -> bool;
    /// Persisted cloud-summarization consent (plan summary-provider-contract).
    /// Default false keeps the cloud path unreachable for fakes.
    fn summary_cloud_consent(&self) -> bool {
        false
    }
    /// Configured LLM for summaries as `(provider_id, model, executor)`.
    /// `None` means "not configured" — the summary then fails with a visible
    /// code instead of silently skipping. Tests inject scripted executors.
    fn summary_llm(&self) -> Option<SummaryLlmConfig> {
        None
    }
}

/// Pen-test hardening: a pathological title (megabytes pasted, zero-width
/// spam) must be rejected at the boundary instead of flowing into the
/// database, every event payload and every render path.
const MAX_TITLE_CHARS: usize = 500;

fn ensure_title_reasonable(title: &str) -> Result<()> {
    if title.chars().count() > MAX_TITLE_CHARS {
        anyhow::bail!("meetingTitleTooLong");
    }
    Ok(())
}

/// Free bytes available to the calling user on the volume holding `path`.
///
/// Windows uses `GetDiskFreeSpaceExW` directly (no extra crate feature),
/// Linux uses `statvfs`. Other targets report `None`, which the preflight
/// renders as a warning rather than a false "not enough space" error.
// Each `cfg` arm is a self-contained block, so the Windows arm returns early
// instead of falling through to the unreachable tail of the other arms.
#[allow(clippy::needless_return)]
pub fn free_disk_bytes_at(path: &Path) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                lp_directory_name: *const u16,
                lp_free_bytes_available_to_caller: *mut u64,
                lp_total_number_of_bytes: *mut u64,
                lp_total_number_of_free_bytes: *mut u64,
            ) -> i32;
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        // SAFETY: `wide` is a NUL-terminated UTF-16 path and the three out
        // parameters are valid, uniquely borrowed stack slots.
        let ok = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) != 0
        };
        return ok.then_some(available);
    }
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: `stat` is zeroed and `c_path` is a valid NUL-terminated path.
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
                return None;
            }
            Some(stat.f_bavail as u64 * stat.f_frsize as u64)
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        None
    }
}

/// Production environment: reads Speak settings and asks a recorder for the
/// device facts, so device truth has exactly one owner.
pub struct AppEnvironment {
    app: AppHandle,
    assets_root: PathBuf,
    recorder_factory: RecorderFactory,
}

impl MeetingEnvironment for AppEnvironment {
    fn device_set(&self) -> rec::SourceDeviceSet {
        let settings = crate::settings::get_settings(&self.app);
        let microphone = settings
            .selected_microphone
            .clone()
            .filter(|name| !name.is_empty())
            .map(rec::SourceOpenRequest::new);
        let system = match settings
            .selected_output_device
            .clone()
            .filter(|name| !name.is_empty())
        {
            // Explicit choice: bind to that render device.
            Some(name) => {
                let mut request = rec::SourceOpenRequest::new(name);
                request.follow_default_device = false;
                Some(request)
            }
            // No choice yet: follow the OS default output instead of silently
            // recording nothing (real-world finding: YouTube played, system
            // track empty because the setting was never touched).
            None => {
                let mut request = rec::SourceOpenRequest::new("");
                request.follow_default_device = true;
                Some(request)
            }
        };
        rec::SourceDeviceSet { microphone, system }
    }

    fn device_checks(&self, devices: &rec::SourceDeviceSet) -> Vec<rec::PreflightCheck> {
        let recorder = (self.recorder_factory)();
        recorder.preflight(devices).unwrap_or_else(|e| {
            warn!("Meeting device preflight failed: {}", e);
            Vec::new()
        })
    }

    fn free_disk_bytes(&self) -> Option<u64> {
        free_disk_bytes_at(&self.assets_root)
    }

    fn provider(&self) -> ProviderSnapshot {
        self.provider_for(None)
    }

    fn provider_for(
        &self,
        override_provider: Option<crate::settings::SpeechInputProvider>,
    ) -> ProviderSnapshot {
        let settings = crate::settings::get_settings(&self.app);
        // Meetings pick their STT provider independently of dictation
        // (plan §13 Phase 6 acceptance).
        let effective = override_provider.unwrap_or(settings.meeting_stt_provider);
        // `configured` must mirror the exact condition under which lib.rs
        // registers the cloud engine: otherwise a missing secret would pass
        // preflight, pin "Azure Speech" on the session and then silently
        // transcribe locally while the UI keeps claiming cloud.
        let azure_configured = !settings.azure_speech_endpoint.trim().is_empty()
            && crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
                .is_some();
        let (kind, name, configured) = match effective {
            crate::settings::SpeechInputProvider::Local => (
                "local",
                "Local".to_string(),
                !settings.selected_model.is_empty(),
            ),
            crate::settings::SpeechInputProvider::Azure => {
                ("cloud", "Azure Speech".to_string(), azure_configured)
            }
            crate::settings::SpeechInputProvider::Custom => (
                "cloud",
                "Custom endpoint".to_string(),
                !settings.custom_stt_endpoint.trim().is_empty(),
            ),
        };
        ProviderSnapshot {
            kind: kind.to_string(),
            name,
            language: Some(settings.selected_language.clone()),
            configured,
            summary_provider: None,
            retention_policy: "default".to_string(),
        }
    }

    fn microphone_permission_granted(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            let status = crate::commands::audio::get_windows_microphone_permission_status();
            !status.supported
                || status.overall_access != crate::commands::audio::PermissionAccess::Denied
        }
        #[cfg(not(target_os = "windows"))]
        {
            true
        }
    }

    fn summary_cloud_consent(&self) -> bool {
        crate::settings::get_settings(&self.app).meeting_summary_cloud_consent
    }

    fn summary_llm(&self) -> Option<SummaryLlmConfig> {
        let settings = crate::settings::get_settings(&self.app);
        let provider = settings.active_post_process_provider().cloned()?;
        let model = settings.post_process_models.get(&provider.id)?.clone();
        if model.trim().is_empty() {
            return None;
        }
        let api_key =
            crate::secret_store::get_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider.id)
                .unwrap_or_default();
        let executor = summarize::CloudSummarizer::new(provider.clone(), api_key, model.clone());
        Some((provider.id, model, Box::new(executor)))
    }
}

// ---------------------------------------------------------------------------
// Recorder binding (INTEGRATION POINT)
// ---------------------------------------------------------------------------

/// Factory for capture backends. The manager never names a concrete recorder
/// type anywhere else, so swapping the implementation (or injecting a fake in
/// tests) touches exactly one line.
pub type RecorderFactory = Arc<dyn Fn() -> Box<dyn rec::MeetingRecorder> + Send + Sync>;

/// **INTEGRATION POINT** — the single place where the session state machine
/// binds to a concrete capture implementation. Everything above this line
/// speaks only `recorder-contract.md` types.
pub fn default_recorder_factory() -> RecorderFactory {
    Arc::new(|| Box::new(rec::MeetingRecorderImpl::new()))
}

/// Forwards recorder events into the manager's serialization queue.
///
/// Recorder callbacks may fire from capture threads while the manager holds
/// its own lock, so the sink must never block: it only enqueues (SSC O4 —
/// unordered inbound, ordered outbound).
struct ChannelRecorderSink {
    tx: mpsc::Sender<(String, rec::RecorderEvent)>,
    meeting_id: String,
}

impl rec::RecorderEventSink for ChannelRecorderSink {
    fn on_event(&self, event: &rec::RecorderEvent) {
        let _ = self.tx.send((self.meeting_id.clone(), event.clone()));
    }
}

// ---------------------------------------------------------------------------
// ChunkStateStore — recorder-contract §4 seam onto `meeting_audio_chunks`
// ---------------------------------------------------------------------------

/// Maps the recorder's numeric [`rec::ChunkId`] onto the store's UUID rows.
///
/// The recorder contract fixes `ChunkId = i64` while the meeting schema uses
/// stable UUID primary keys (plan §9 line 1151). Chunk ids are only meaningful
/// inside one recorder session (or one recovery run), so a process-local
/// bijection is sufficient and keeps both contracts intact.
#[derive(Default)]
struct ChunkIdMap {
    next: i64,
    to_uuid: HashMap<i64, StoreUuid>,
}

impl ChunkIdMap {
    fn insert(&mut self, uuid: StoreUuid) -> i64 {
        self.next += 1;
        self.to_uuid.insert(self.next, uuid);
        self.next
    }

    fn get(&self, id: i64) -> Option<StoreUuid> {
        self.to_uuid.get(&id).cloned()
    }
}

/// SQLite-backed [`rec::ChunkStateStore`] handed to the recorder.
pub struct SqliteChunkStateStore {
    store: Arc<SqliteMeetingStore>,
    conn: Mutex<Connection>,
    ids: Mutex<ChunkIdMap>,
}

impl SqliteChunkStateStore {
    /// Opens a dedicated connection onto the shared meeting database.
    pub fn open(store: Arc<SqliteMeetingStore>, db_path: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            store,
            conn: Mutex::new(open_meeting_database(db_path)?),
            ids: Mutex::new(ChunkIdMap::default()),
        })
    }

    fn source_from_token(token: &str) -> rec::SourceKind {
        match token {
            "system" => rec::SourceKind::System,
            _ => rec::SourceKind::Microphone,
        }
    }
}

impl rec::ChunkStateStore for SqliteChunkStateStore {
    fn open_chunks(&self, session_id: &str) -> Result<Vec<rec::ChunkDescriptor>, String> {
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, sequence, source, timeline_start_ms, timeline_end_ms,
                        storage_key, sample_count, checksum
                 FROM meeting_audio_chunks
                 WHERE meeting_id = ?1 AND state = 'writing' AND deleted_at IS NULL
                 ORDER BY sequence ASC",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let rows = mapped
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        drop(conn);

        let mut ids = lock(&self.ids);
        Ok(rows
            .into_iter()
            .map(
                |(id, sequence, source, start, end, key, samples, checksum)| {
                    let chunk_id = ids.insert(StoreUuid(id));
                    rec::ChunkDescriptor {
                        chunk_id,
                        source: Self::source_from_token(&source),
                        sequence: sequence.max(0) as u32,
                        source_segment_id: 0,
                        state: rec::ChunkState::Writing,
                        range: rec::TimelineRange {
                            start_ms: start as f64,
                            end_ms: end as f64,
                        },
                        sample_count: samples.max(0) as u64,
                        storage_key: key,
                        checksum_sha256: checksum,
                        recovered: false,
                    }
                },
            )
            .collect())
    }

    fn transition(
        &self,
        chunk_id: rec::ChunkId,
        from: rec::ChunkState,
        to: rec::ChunkState,
        patch: &rec::ChunkPatch,
    ) -> Result<(), String> {
        let uuid = lock(&self.ids)
            .get(chunk_id)
            .ok_or_else(|| format!("unknown chunk id {chunk_id}"))?;
        if from == to {
            return Ok(());
        }
        match to {
            rec::ChunkState::Ready => {
                let sample_count = patch.sample_count.unwrap_or(0) as i64;
                self.store
                    .finalize_chunk(
                        uuid,
                        ChunkFacts {
                            // PCM16 mono plus the 44-byte RIFF header.
                            file_size: sample_count * 2 + 44,
                            sample_count,
                            checksum: patch.checksum_sha256.clone().unwrap_or_default(),
                            gap_drift_metadata: patch.gap_stats.map(|(count, filled)| {
                                serde_json::json!({ "gap_count": count, "filled_samples": filled })
                            }),
                        },
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            rec::ChunkState::Damaged => self
                .store
                .mark_chunk(uuid, StoreChunkState::Damaged, Some("recovery".to_string()))
                .map_err(|e| e.to_string()),
            rec::ChunkState::Deleted => self
                .store
                .mark_chunk(
                    uuid,
                    StoreChunkState::Deleted,
                    Some("retention".to_string()),
                )
                .map_err(|e| e.to_string()),
            rec::ChunkState::Writing => {
                Err("chunks cannot transition back into 'writing'".to_string())
            }
        }
    }

    fn insert_writing_chunk(
        &self,
        session_id: &str,
        descriptor: &rec::ChunkDescriptor,
    ) -> Result<rec::ChunkId, String> {
        let chunk = self
            .store
            .append_chunk(NewAudioChunk {
                meeting_id: StoreUuid(session_id.to_string()),
                sequence: descriptor.sequence as i64,
                source: MeetingAudioSource::from_recorder(descriptor.source)
                    .as_str()
                    .to_string(),
                timeline_start_ms: descriptor.range.start_ms as i64,
                timeline_end_ms: descriptor.range.end_ms as i64,
                storage_key: descriptor.storage_key.clone(),
                sample_rate: TIMELINE_SAMPLE_RATE_HZ,
                sample_count: descriptor.sample_count as i64,
                device_id: None,
            })
            .map_err(|e| e.to_string())?;
        Ok(lock(&self.ids).insert(chunk.envelope.id))
    }
}

// ---------------------------------------------------------------------------
// Wave-1 CRUD projection (unchanged shape — the shipped Meetings UI uses it)
// ---------------------------------------------------------------------------

/// Frontend-facing projection of one persisted meeting session.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingSession {
    /// Stable UUIDv4 (plan §9 line 1151).
    pub id: String,
    /// User-editable title.
    pub title: String,
    /// Lifecycle state token.
    pub state: String,
    /// Capture start, Unix epoch milliseconds.
    pub started_at: i64,
    /// Capture end, if finalized.
    pub ended_at: Option<i64>,
    /// Creation instant, Unix epoch milliseconds.
    pub created_at: i64,
    /// Last mutation instant, Unix epoch milliseconds.
    pub updated_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Locks a mutex, recovering from poisoning instead of panicking: a poisoned
/// lock must not take the recording path down (no `unwrap` in production code).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Neutral fallback title when the caller supplies none.
fn fallback_title(started_at_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(started_at_ms)
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// In-memory projection of one session the manager currently tracks.
#[derive(Debug, Clone)]
struct SessionRuntime {
    title: String,
    state: MeetingState,
    revision: i64,
    elapsed_base_ms: i64,
    running_since: Option<i64>,
    microphone_only: bool,
    summary_enabled: bool,
    provider_kind: String,
    provider_name: Option<String>,
    /// SSC T17's `previous_active_state`, kept in memory (see module docs).
    previous_active: Option<MeetingState>,
    last_error: Option<MeetingSnapshotError>,
    levels: HashMap<String, f32>,
}

/// Everything that must be mutated under one lock so transitions are totally
/// ordered per session (SSC O2: "publishers serialize transitions").
struct ManagerInner {
    sessions: HashMap<String, SessionRuntime>,
    recorder: Option<Box<dyn rec::MeetingRecorder>>,
    /// The one session that currently owns the capture devices, if any.
    capture_owner: Option<String>,
}

/// The meeting session state machine plus the wave-1 CRUD surface.
pub struct MeetingManager {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    store: Arc<SqliteMeetingStore>,
    events: Arc<dyn MeetingEventSink>,
    env: Arc<dyn MeetingEnvironment>,
    assets_root: PathBuf,
    recorder_factory: RecorderFactory,
    inner: Mutex<ManagerInner>,
    recoveries: Mutex<Vec<MeetingRecoveryOffer>>,
    recorder_events: Mutex<Option<mpsc::Sender<(String, rec::RecorderEvent)>>>,
    /// Local STT queue (plan §13 Phase 5); attached after construction.
    queue: Mutex<Option<Arc<MeetingTranscriptionQueue>>>,
}

impl MeetingManager {
    /// Opens (and migrates) `meetings.db` in the portable-aware app data dir
    /// and wires the production event sink, environment and recorder binding.
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let db_path = meeting_db_path(app_handle)?;
        let assets_root = crate::portable::app_data_dir(app_handle)?.join("meetings");
        let factory = default_recorder_factory();
        let env = Arc::new(AppEnvironment {
            app: app_handle.clone(),
            assets_root: assets_root.clone(),
            recorder_factory: factory.clone(),
        });
        Self::open_with(
            db_path,
            assets_root,
            Arc::new(TauriEventSink::new(app_handle.clone())),
            env,
            factory,
        )
    }

    /// The shared repository behind this manager, so sibling managers (e.g.
    /// [`crate::managers::project::ProjectManager`]) read and write the same
    /// SQLite file without a second connection.
    pub fn store_handle(&self) -> Arc<SqliteMeetingStore> {
        self.store.clone()
    }

    /// Opens (and migrates) the database at an explicit path with default
    /// (inert) event/environment seams. Public so tests can prove persistence
    /// across a full close/reopen cycle.
    #[allow(dead_code)]
    pub fn open(db_path: PathBuf) -> Result<Self> {
        let assets_root = db_path
            .parent()
            .map(|p| p.join("meetings"))
            .unwrap_or_else(|| PathBuf::from("meetings"));
        Self::open_with(
            db_path,
            assets_root,
            Arc::new(NullEventSink),
            Arc::new(StaticEnvironment::default()),
            default_recorder_factory(),
        )
    }

    /// Full constructor; every collaborator is injected.
    pub fn open_with(
        db_path: PathBuf,
        assets_root: PathBuf,
        events: Arc<dyn MeetingEventSink>,
        env: Arc<dyn MeetingEnvironment>,
        recorder_factory: RecorderFactory,
    ) -> Result<Self> {
        info!("Initializing meeting database at {:?}", db_path);
        let conn = open_meeting_database(&db_path)?;
        let store = Arc::new(SqliteMeetingStore::open(db_path.clone())?);
        debug!("Meeting database ready at {:?}", db_path);

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            store,
            events,
            env,
            assets_root,
            recorder_factory,
            inner: Mutex::new(ManagerInner {
                sessions: HashMap::new(),
                recorder: None,
                capture_owner: None,
            }),
            recoveries: Mutex::new(Vec::new()),
            recorder_events: Mutex::new(None),
            queue: Mutex::new(None),
        })
    }

    /// Starts the recorder-event pump. Must be called once, after the manager
    /// is behind an `Arc`, because the pump holds a `Weak` back-reference.
    pub fn attach_recorder_events(self: &Arc<Self>) {
        let (tx, rx) = mpsc::channel::<(String, rec::RecorderEvent)>();
        *lock(&self.recorder_events) = Some(tx);
        let weak: Weak<MeetingManager> = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("meeting-recorder-events".into())
            .spawn(move || {
                while let Ok((meeting_id, event)) = rx.recv() {
                    let Some(manager) = weak.upgrade() else {
                        break;
                    };
                    manager.handle_recorder_event(&meeting_id, &event);
                }
            })
            .map(|_| ())
            .unwrap_or_else(|e| error!("Failed to spawn meeting event pump: {}", e));
    }

    /// Attaches and spawns the local STT queue (plan §13 Phase 5). Must be
    /// called once, after the manager is behind an `Arc`, because the delegate
    /// holds a `Weak` back-reference.
    pub fn attach_transcription_queue(
        self: &Arc<Self>,
        engine: Arc<dyn MeetingSttEngine>,
        dictation_active: Arc<dyn Fn() -> bool + Send + Sync>,
        vad_model_path: Option<PathBuf>,
    ) {
        let queue = MeetingTranscriptionQueue::new(
            Arc::clone(&self.store),
            self.assets_root.clone(),
            engine,
            dictation_active,
            vad_model_path,
            Arc::new(ManagerDelegate {
                manager: Arc::downgrade(self),
            }),
            QueueConfig::default(),
        );
        *lock(&self.queue) = Some(Arc::clone(&queue));
        queue.spawn();
    }

    /// Nudges the STT queue when new work may have become claimable.
    fn wake_queue(&self) {
        if let Some(queue) = lock(&self.queue).as_ref() {
            queue.wake();
        }
    }

    // -- wave-1 CRUD --------------------------------------------------------

    /// Creates a session row in state [`SESSION_STATE_DRAFT`].
    pub fn create_session(&self, title: Option<String>) -> Result<MeetingSession> {
        let started_at = now_ms();
        let title = match title.map(|t| t.trim().to_string()) {
            Some(t) if !t.is_empty() => t,
            _ => fallback_title(started_at),
        };
        ensure_title_reasonable(&title)?;
        let id = uuid::Uuid::new_v4().to_string();
        {
            let conn = lock(&self.conn);
            conn.execute(
                "INSERT INTO meeting_sessions (id, title, state, started_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                params![id, title, SESSION_STATE_DRAFT, started_at],
            )?;
        }
        debug!("Created meeting session {} ({})", id, title);

        // T01: the draft exists, so the state machine may track it and the UI
        // may subscribe (rule O1 — the row is committed before publication).
        let provider = self.env.provider();
        lock(&self.inner).sessions.insert(
            id.clone(),
            SessionRuntime {
                title: title.clone(),
                state: MeetingState::Draft,
                revision: 1,
                elapsed_base_ms: 0,
                running_since: None,
                microphone_only: false,
                summary_enabled: false,
                provider_kind: provider.kind.clone(),
                provider_name: Some(provider.name.clone()),
                previous_active: None,
                last_error: None,
                levels: HashMap::new(),
            },
        );
        self.publish(
            TOPIC_SESSION_CREATED,
            &MeetingSessionCreatedEvent {
                meeting_id: id.clone(),
                title: title.clone(),
                state: MeetingState::Draft,
                state_revision: 1,
            },
        );

        Ok(MeetingSession {
            id,
            title,
            state: SESSION_STATE_DRAFT.to_string(),
            started_at,
            ended_at: None,
            created_at: started_at,
            updated_at: started_at,
        })
    }

    /// Current summary of a meeting, if any (Phase 7 read side).
    pub fn get_summary(
        &self,
        meeting_id: &str,
    ) -> Result<Option<super::meeting_store::MeetingSummary>> {
        self.store
            .summary_for_meeting(StoreUuid(meeting_id.to_string()))
            .map_err(Into::into)
    }

    /// Chronological transcript of a meeting (Phase 7 detail view).
    pub fn get_transcript(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<super::meeting_store::TranscriptSegment>> {
        self.store
            .segments_for_meeting(StoreUuid(meeting_id.to_string()))
            .map_err(Into::into)
    }

    /// Controlled deletion (plan §11). `AudioOnly` removes the raw recording
    /// (asset files + chunk rows) while keeping transcript and summary;
    /// `Everything` additionally tombstones transcript and summary rows and
    /// soft-deletes the session. Refused while the meeting owns the capture.
    pub fn delete_meeting_data(&self, meeting_id: &str, scope: DeleteScope) -> Result<()> {
        {
            let inner = lock(&self.inner);
            if inner.capture_owner.as_deref() == Some(meeting_id)
                || inner.sessions.get(meeting_id).is_some_and(|r| {
                    r.state.is_active()
                        || matches!(
                            r.state,
                            MeetingState::Transcribing | MeetingState::Summarizing
                        )
                })
            {
                // Deleting while jobs are mid-flight would orphan their output
                // into a tombstoned session (zombie transcripts/summaries).
                anyhow::bail!("meetingIsActive");
            }
        }

        let uuid = StoreUuid(meeting_id.to_string());
        // Files first: a row tombstone without its bytes is recoverable; the
        // reverse would claim deletion that did not happen.
        crate::storage::asset_store::LocalAssetStore::with_root(self.assets_root.clone())
            .delete_meeting_tree(meeting_id)
            .map_err(|e| anyhow::anyhow!("asset delete failed: {e}"))?;

        match scope {
            DeleteScope::AudioOnly => self.store.delete_meeting_audio_rows(uuid)?,
            DeleteScope::Everything => {
                self.store.purge_meeting_content(uuid.clone())?;
                // No dangling project memberships for a deleted meeting.
                self.store.delete_project_links_for_meeting(uuid)?;
                if !self.soft_delete_session(meeting_id)? {
                    anyhow::bail!("meeting session {meeting_id} not found");
                }
            }
        }
        Ok(())
    }

    /// Renders one export payload from persisted rows (Phase 7 read side).
    pub fn build_export(
        &self,
        meeting_id: &str,
        format: export::ExportFormat,
    ) -> Result<Option<export::MeetingExport>> {
        let Some(session) = self.get_session(meeting_id)? else {
            return Ok(None);
        };
        let segments = self
            .store
            .segments_for_meeting(StoreUuid(meeting_id.to_string()))?;
        let export_segments: Vec<export::ExportSegment> = segments
            .iter()
            .map(|s| export::ExportSegment {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                source: s.source.clone(),
                text: s.text.clone(),
            })
            .collect();
        let summary = self
            .store
            .summary_for_meeting(StoreUuid(meeting_id.to_string()))?
            .filter(|s| s.state == "completed")
            .map(|s| export::ExportSummary {
                provider: s.provider,
                model: s.model,
                prompt_version: s.prompt_version,
                summary_markdown: s.summary_markdown,
                decisions: string_list_of(&s.decisions),
                action_items: string_list_of(&s.action_items),
                open_questions: string_list_of(&s.open_questions),
            });
        Ok(Some(export::render(
            format,
            meeting_id,
            &session.title,
            session.started_at,
            &export_segments,
            summary.as_ref(),
        )))
    }

    /// Fetches one alive (non-tombstoned) session by id.
    pub fn get_session(&self, id: &str) -> Result<Option<MeetingSession>> {
        let conn = lock(&self.conn);
        let session = conn
            .query_row(
                "SELECT id, title, state, started_at, ended_at, created_at, updated_at
                 FROM meeting_sessions WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                row_to_session,
            )
            .optional()?;
        Ok(session)
    }

    /// Lists alive sessions, newest first, optionally filtered by title.
    pub fn list_sessions(&self, query: Option<&str>) -> Result<Vec<MeetingSession>> {
        let conn = lock(&self.conn);
        let pattern = query
            // `%`/`_` are LIKE wildcards and `\` is the ESCAPE character of
            // the statement below; strip them so user input stays literal.
            .map(|q| format!("%{}%", q.replace(['%', '_', '\\'], "")))
            .filter(|p| p != "%%");
        let mut stmt = conn.prepare(
            "SELECT id, title, state, started_at, ended_at, created_at, updated_at
             FROM meeting_sessions
             WHERE deleted_at IS NULL AND (?1 IS NULL OR title LIKE ?1 ESCAPE '\\')
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let sessions = stmt
            .query_map(params![pattern], row_to_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    /// Saves a new title (revision bump + `updated_at`, plan §9 line 1153).
    pub fn rename_session(&self, id: &str, title: String) -> Result<MeetingSession> {
        let title = title.trim().to_string();
        if title.is_empty() {
            anyhow::bail!("Meeting title must not be empty");
        }
        ensure_title_reasonable(&title)?;
        {
            let conn = lock(&self.conn);
            let updated = conn.execute(
                "UPDATE meeting_sessions
                 SET title = ?2, revision = revision + 1, updated_at = ?3
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id, title, now_ms()],
            )?;
            if updated == 0 {
                anyhow::bail!("Meeting session {} not found", id);
            }
        }
        if let Some(runtime) = lock(&self.inner).sessions.get_mut(id) {
            runtime.title = title.clone();
            runtime.revision += 1;
        }
        self.get_session(id)?
            .ok_or_else(|| anyhow::anyhow!("Meeting session {} not found after update", id))
    }

    /// Soft-deletes a session (tombstone, plan §9 line 1152).
    /// Soft-deletes a session (tombstone, plan §9 line 1152).
    ///
    /// Pen-tester's rule: a session that owns the capture must never be
    /// tombstoned underneath its recorder — that would orphan a live writer,
    /// land new chunks on a deleted row and manufacture a bogus recovery
    /// offer on the next start. Stop or cancel first.
    pub fn soft_delete_session(&self, id: &str) -> Result<bool> {
        {
            let inner = lock(&self.inner);
            if inner.capture_owner.as_deref() == Some(id) {
                anyhow::bail!("meetingIsActive");
            }
            if let Some(runtime) = inner.sessions.get(id) {
                if runtime.state.is_active()
                    || matches!(
                        runtime.state,
                        MeetingState::Transcribing | MeetingState::Summarizing
                    )
                {
                    anyhow::bail!("meetingIsActive");
                }
            }
        }
        let conn = lock(&self.conn);
        let updated = conn.execute(
            "UPDATE meeting_sessions SET deleted_at = ?2, revision = revision + 1, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id, now_ms()],
        )?;
        Ok(updated > 0)
    }

    // -- state machine ------------------------------------------------------

    /// The sole decision point (SSC §1.2). Guards the transition against the
    /// §2.2 table, commits the persisted effect, then publishes (rule O1).
    ///
    /// Returns the revision the transition committed at.
    fn transition(
        &self,
        inner: &mut ManagerInner,
        meeting_id: &str,
        to: MeetingState,
        patch: SessionPatch,
    ) -> Result<i64, String> {
        let runtime = inner
            .sessions
            .get(meeting_id)
            .cloned()
            .ok_or_else(|| format!("unknown meeting session {meeting_id}"))?;
        let from = runtime.state;
        if !transition_allowed(from, to) {
            return Err(format!(
                "illegal meeting transition {} -> {}",
                from.as_str(),
                to.as_str()
            ));
        }

        let mut patch = patch;
        patch.state = Some(to.as_str().to_string());
        let stored = self
            .store
            .update_session(StoreUuid(meeting_id.to_string()), patch)
            .map_err(|e| e.to_string())?;
        let revision = stored.envelope.revision;

        let now = now_ms();
        if let Some(runtime) = inner.sessions.get_mut(meeting_id) {
            // Pause-corrected clock: leaving capture banks the running leg,
            // entering capture opens a new one.
            if from.is_capturing() && !to.is_capturing() {
                if let Some(started) = runtime.running_since.take() {
                    runtime.elapsed_base_ms += (now - started).max(0);
                }
            } else if !from.is_capturing() && to.is_capturing() {
                runtime.running_since = Some(now);
            }
            if to == MeetingState::RecoverableError && from.is_active() {
                runtime.previous_active = Some(from);
            }
            runtime.state = to;
            runtime.revision = revision;
        }

        let microphone_only = runtime.microphone_only.then_some(true);
        self.publish(
            TOPIC_SESSION_STATE,
            &MeetingSessionStateEvent {
                meeting_id: meeting_id.to_string(),
                from: Some(from),
                to,
                state_revision: revision,
                microphone_only,
            },
        );
        info!(
            "meeting {} transitioned {} -> {} (revision {})",
            meeting_id,
            from.as_str(),
            to.as_str(),
            revision
        );
        Ok(revision)
    }

    fn publish<T: Serialize>(&self, topic: &'static str, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(value) => self.events.publish(topic, value),
            Err(e) => warn!("Failed to serialize payload for {}: {}", topic, e),
        }
    }

    fn emit_error(&self, meeting_id: &str, code: &str, detail: Option<String>, recoverable: bool) {
        let revision = lock(&self.inner)
            .sessions
            .get(meeting_id)
            .map(|r| r.revision);
        if let Some(runtime) = lock(&self.inner).sessions.get_mut(meeting_id) {
            runtime.last_error = Some(MeetingSnapshotError {
                code: code.to_string(),
                detail: detail.clone(),
            });
        }
        self.publish(
            TOPIC_SESSION_ERROR,
            &MeetingSessionErrorEvent {
                meeting_id: meeting_id.to_string(),
                code: code.to_string(),
                detail,
                state_revision: revision,
                recoverable: Some(recoverable),
            },
        );
    }

    /// Loads a session into the in-memory projection if it is not tracked yet.
    fn ensure_tracked(&self, inner: &mut ManagerInner, meeting_id: &str) -> Result<(), String> {
        if inner.sessions.contains_key(meeting_id) {
            return Ok(());
        }
        let row = self
            .load_row(meeting_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown meeting session {meeting_id}"))?;
        inner.sessions.insert(meeting_id.to_string(), row);
        Ok(())
    }

    /// Reads the persisted projection of one session (post-restart rebuild).
    fn load_row(&self, meeting_id: &str) -> Result<Option<SessionRuntime>> {
        let conn = lock(&self.conn);
        let row = conn
            .query_row(
                "SELECT title, state, revision, stt_provider, summary_provider,
                        has_microphone_audio, has_system_audio, last_error, started_at, ended_at
                 FROM meeting_sessions WHERE id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            title,
            state,
            revision,
            provider,
            summary_provider,
            has_mic,
            has_system,
            last_error,
            started_at,
            ended_at,
        )) = row
        else {
            return Ok(None);
        };
        let state = MeetingState::parse(&state).unwrap_or(MeetingState::Draft);
        // Elapsed time is only known durably for finished sessions; a live
        // session re-adopts its clock when the state machine resumes control.
        let elapsed = ended_at.map(|end| (end - started_at).max(0)).unwrap_or(0);
        Ok(Some(SessionRuntime {
            title,
            state,
            revision,
            elapsed_base_ms: elapsed,
            running_since: None,
            microphone_only: has_mic != 0 && has_system == 0,
            summary_enabled: summary_provider.is_some(),
            provider_kind: if provider == "local" {
                "local"
            } else {
                "cloud"
            }
            .to_string(),
            provider_name: Some(provider),
            previous_active: None,
            last_error: last_error.map(|code| MeetingSnapshotError { code, detail: None }),
            levels: HashMap::new(),
        }))
    }

    // -- preflight (plan "Startvalidierung") --------------------------------

    /// Runs the full start validation and renders the compact start-dialog
    /// rows (plan §6.7). Never mutates session state.
    pub fn preflight(&self) -> MeetingPreflightReport {
        let devices = self.env.device_set();
        let device_checks = self.env.device_checks(&devices);
        let provider = self.env.provider();
        let mut checks = Vec::new();

        let mic = device_checks
            .iter()
            .find(|c| c.source == rec::SourceKind::Microphone);
        checks.push(device_row(
            MeetingPreflightCheckId::Microphone,
            mic,
            devices.microphone.as_ref().map(|r| r.device_name.clone()),
            true,
        ));
        let system = device_checks
            .iter()
            .find(|c| c.source == rec::SourceKind::System);
        checks.push(device_row(
            MeetingPreflightCheckId::SystemAudio,
            system,
            devices.system.as_ref().map(|r| r.device_name.clone()),
            false,
        ));

        checks.push(MeetingPreflightCheck {
            id: MeetingPreflightCheckId::Transcription,
            status: if provider.configured {
                MeetingPreflightStatus::Ok
            } else {
                MeetingPreflightStatus::Error
            },
            value: Some(provider.name.clone()),
            code: (!provider.configured).then(|| "providerNotConfigured".to_string()),
            detail: None,
        });

        checks.push(MeetingPreflightCheck {
            id: MeetingPreflightCheckId::Language,
            status: MeetingPreflightStatus::Ok,
            value: provider.language.clone(),
            code: None,
            detail: None,
        });

        let required = if provider.kind == "cloud" {
            DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL + CLOUD_UPLOAD_TRANSIENT_RESERVE_BYTES
        } else {
            DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL
        };
        checks.push(match self.env.free_disk_bytes() {
            Some(free) if free >= required => MeetingPreflightCheck {
                id: MeetingPreflightCheckId::Disk,
                status: MeetingPreflightStatus::Ok,
                value: Some(format!("{} MB free", free / 1_000_000)),
                code: None,
                detail: None,
            },
            Some(free) => MeetingPreflightCheck {
                id: MeetingPreflightCheckId::Disk,
                status: MeetingPreflightStatus::Error,
                value: Some(format!("{} MB free", free / 1_000_000)),
                code: Some("diskFull".to_string()),
                detail: Some(format!("{} MB required", required / 1_000_000)),
            },
            None => MeetingPreflightCheck {
                id: MeetingPreflightCheckId::Disk,
                status: MeetingPreflightStatus::Warning,
                value: None,
                code: Some("diskUnknown".to_string()),
                detail: None,
            },
        });

        let permission_ok = self.env.microphone_permission_granted();
        checks.push(MeetingPreflightCheck {
            id: MeetingPreflightCheckId::Permission,
            status: if permission_ok {
                MeetingPreflightStatus::Ok
            } else {
                MeetingPreflightStatus::Error
            },
            value: None,
            code: (!permission_ok).then(|| "permissionDenied".to_string()),
            detail: None,
        });

        let can_start = checks
            .iter()
            .all(|c| c.status != MeetingPreflightStatus::Error);
        let mic_ok = checks.iter().any(|c| {
            c.id == MeetingPreflightCheckId::Microphone && c.status == MeetingPreflightStatus::Ok
        });
        let system_ok = checks.iter().any(|c| {
            c.id == MeetingPreflightCheckId::SystemAudio && c.status == MeetingPreflightStatus::Ok
        });

        MeetingPreflightReport {
            checks,
            can_start,
            microphone_only_available: Some(mic_ok && !system_ok),
        }
    }

    // -- lifecycle commands -------------------------------------------------

    /// T01 — creates the draft the start dialog works on.
    pub fn create_draft(&self, title: Option<String>) -> Result<MeetingSnapshot, String> {
        let session = self.create_session(title).map_err(|e| e.to_string())?;
        self.snapshot(&session.id)
    }

    /// T02/T03 — consent gate, start validation, device opening. Kept for
    /// callers/tests that need the plain default-provider start; the command
    /// layer uses [`Self::start_with_provider`].
    #[allow(dead_code)]
    pub fn start(
        &self,
        meeting_id: &str,
        title: Option<String>,
        microphone_only: bool,
        summary_enabled: bool,
        consent_acknowledged: bool,
    ) -> Result<MeetingSnapshot, String> {
        self.start_with_provider(
            meeting_id,
            title,
            microphone_only,
            summary_enabled,
            consent_acknowledged,
            None,
        )
    }

    /// Like [`Self::start`], but pins this session's STT provider regardless
    /// of the global default (plan §13 Phase 6: Local/Azure/Custom are chosen
    /// per meeting and must not touch the dictation provider).
    pub fn start_with_provider(
        &self,
        meeting_id: &str,
        title: Option<String>,
        microphone_only: bool,
        summary_enabled: bool,
        consent_acknowledged: bool,
        stt_provider: Option<crate::settings::SpeechInputProvider>,
    ) -> Result<MeetingSnapshot, String> {
        // Gate G4 (SSC §7) is fail-closed and evaluated *before* T02, so no
        // capture resource can be opened without an acknowledged consent.
        if !consent_acknowledged {
            self.emit_error(meeting_id, "consentRequired", None, true);
            return Err("consentRequired".to_string());
        }

        if let Some(title) = title {
            if !title.trim().is_empty() {
                let _ = self.rename_session(meeting_id, title);
            }
        }

        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        if let Some(owner) = inner.capture_owner.clone() {
            if owner != meeting_id {
                return Err("anotherMeetingIsRecording".to_string());
            }
        }
        if let Some(runtime) = inner.sessions.get_mut(meeting_id) {
            runtime.microphone_only = microphone_only;
            runtime.summary_enabled = summary_enabled;
        }

        self.transition(&mut inner, meeting_id, MeetingState::Starting, patch_none())?;

        // Step 1 of SSC §3.1: manager-owned validation before any stream opens.
        let report = self.preflight();
        if !report.can_start {
            let code = report
                .checks
                .iter()
                .find(|c| c.status == MeetingPreflightStatus::Error)
                .and_then(|c| c.code.clone())
                .unwrap_or_else(|| "preflightFailed".to_string());
            let _ = self.transition(&mut inner, meeting_id, MeetingState::Draft, patch_none());
            drop(inner);
            self.emit_error(meeting_id, &code, None, true);
            return Err(code);
        }

        let mut devices = self.env.device_set();
        if microphone_only {
            devices.system = None;
        }

        if let Err(code) = self.open_capture(&mut inner, meeting_id, &devices) {
            let _ = self.transition(&mut inner, meeting_id, MeetingState::Draft, patch_none());
            drop(inner);
            self.emit_error(meeting_id, &code, None, true);
            return Err(code);
        }

        inner.capture_owner = Some(meeting_id.to_string());
        // Gate G-indicator (SSC §7.2): T03 is the sole capture-boundary entry.
        let capturing = inner
            .recorder
            .as_ref()
            .map(|r| r.snapshot().state == rec::RecorderState::Recording)
            .unwrap_or(false);
        if capturing {
            self.transition(
                &mut inner,
                meeting_id,
                MeetingState::Recording,
                patch_none(),
            )?;
        }
        drop(inner);
        // Pin the effective provider choice for this session *after* capture
        // opened successfully: every later STT job routes through the stored
        // snapshot, so a mid-meeting settings change cannot split a transcript
        // across backends.
        if let Err(e) = self.persist_provider_choice(meeting_id, stt_provider) {
            warn!("Failed to persist meeting provider choice: {}", e);
        }
        self.snapshot(meeting_id)
    }

    /// Writes the effective STT provider for one session into its row so the
    /// queue's `session_provider` lookup resolves it (plan §13 Phase 6).
    fn persist_provider_choice(
        &self,
        meeting_id: &str,
        override_provider: Option<crate::settings::SpeechInputProvider>,
    ) -> Result<()> {
        let snapshot = self.env.provider_for(override_provider);
        let kind = snapshot.kind.as_str();
        let name = snapshot.name.as_str();
        let language = snapshot
            .language
            .clone()
            .filter(|l| !l.is_empty() && l != "auto");
        {
            let conn = lock(&self.conn);
            let updated = conn.execute(
                // Metadata-only write (like the queue's own lookups): no
                // revision bump, because no state event accompanies it.
                "UPDATE meeting_sessions
                 SET stt_provider = ?2, provider_config_snapshot = ?3, language = ?4,
                     updated_at = ?5
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    meeting_id,
                    name,
                    serde_json::json!({ "kind": kind }).to_string(),
                    language,
                    now_ms()
                ],
            )?;
            if updated == 0 {
                anyhow::bail!("Meeting session {} not found", meeting_id);
            }
        }
        if let Some(runtime) = lock(&self.inner).sessions.get_mut(meeting_id) {
            runtime.provider_kind = kind.to_string();
            runtime.provider_name = Some(name.to_string());
        }
        Ok(())
    }

    fn open_capture(
        &self,
        inner: &mut ManagerInner,
        meeting_id: &str,
        devices: &rec::SourceDeviceSet,
    ) -> Result<(), String> {
        let mut recorder = (self.recorder_factory)();
        if let Some(tx) = lock(&self.recorder_events).clone() {
            recorder.subscribe(Arc::new(ChannelRecorderSink {
                tx,
                meeting_id: meeting_id.to_string(),
            }));
        }
        let layout = rec::SessionLayout {
            session_id: meeting_id.to_string(),
            root: self.assets_root.join(meeting_id),
        };
        let chunk_store = Arc::new(
            SqliteChunkStateStore::open(self.store.clone(), &self.db_path)
                .map_err(|e| e.to_string())?,
        );
        recorder
            .start_session(layout, chunk_store)
            .map_err(recorder_error_code)?;
        if let Some(request) = devices.microphone.clone() {
            recorder
                .open_source(rec::SourceKind::Microphone, request)
                .map_err(recorder_error_code)?;
        }
        if let Some(request) = devices.system.clone() {
            if let Err(e) = recorder.open_source(rec::SourceKind::System, request) {
                // Plan Fehlerverhalten: a missing loopback must not lose the
                // microphone track; the session continues microphone-only.
                warn!(
                    "System audio unavailable, continuing microphone-only: {}",
                    e
                );
                if let Some(runtime) = inner.sessions.get_mut(meeting_id) {
                    runtime.microphone_only = true;
                }
            }
        }
        inner.recorder = Some(recorder);
        Ok(())
    }

    /// T05 — pause.
    pub fn pause(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        if let Some(recorder) = inner.recorder.as_mut() {
            recorder.pause().map_err(recorder_error_code)?;
        }
        self.transition(&mut inner, meeting_id, MeetingState::Paused, patch_none())?;
        drop(inner);
        self.snapshot(meeting_id)
    }

    /// T06 — resume, re-running the device and disk subset of the validation
    /// (disk-guard-contract §3.2 rule 5).
    pub fn resume(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        let microphone_only = inner
            .sessions
            .get(meeting_id)
            .map(|r| r.microphone_only)
            .unwrap_or(false);
        if let Some(free) = self.env.free_disk_bytes() {
            if free < DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL {
                drop(inner);
                self.emit_error(meeting_id, "diskFull", None, true);
                return Err("diskFull".to_string());
            }
        }
        let mut devices = self.env.device_set();
        if microphone_only {
            devices.system = None;
        }
        if let Some(recorder) = inner.recorder.as_mut() {
            // A resume-validation failure leaves the session paused (RC §6.3).
            recorder.resume(devices).map_err(recorder_error_code)?;
        }
        self.transition(
            &mut inner,
            meeting_id,
            MeetingState::Recording,
            patch_none(),
        )?;
        drop(inner);
        self.snapshot(meeting_id)
    }

    /// T07 → T08/T09/T15 — the controlled stop.
    ///
    /// `abort` only changes the *destination*; the durability work is
    /// identical (SSC T15).
    pub fn stop(&self, meeting_id: &str, abort: bool) -> Result<MeetingSnapshot, String> {
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        self.transition(&mut inner, meeting_id, MeetingState::Stopping, patch_none())?;

        // `stop()` returns only after the last chunk is durably `ready`
        // (RC §4); that return is the sole permission to run T08/T09/T15.
        if let Some(mut recorder) = inner.recorder.take() {
            if let Err(e) = recorder.stop() {
                warn!("Meeting recorder stop reported {}", e);
            }
        }
        inner.capture_owner = None;

        let ended = SessionPatch {
            ended_at: Some(chrono::Utc::now()),
            ..SessionPatch::default()
        };
        if abort {
            self.transition(
                &mut inner,
                meeting_id,
                MeetingState::StoppedUnprocessed,
                ended,
            )?;
            drop(inner);
            return self.snapshot(meeting_id);
        }

        let enqueued = self.enqueue_ready_chunks(meeting_id).unwrap_or_else(|e| {
            warn!("Could not enqueue STT jobs for {}: {}", meeting_id, e);
            0
        });
        let destination = if enqueued > 0 {
            MeetingState::Transcribing
        } else {
            MeetingState::Completed
        };
        self.transition(&mut inner, meeting_id, destination, ended)?;
        drop(inner);
        self.publish_jobs(meeting_id);
        self.wake_queue();
        self.snapshot(meeting_id)
    }

    /// T16 — user abort before capture started.
    pub fn cancel(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        if let Some(mut recorder) = inner.recorder.take() {
            let _ = recorder.stop();
        }
        if inner.capture_owner.as_deref() == Some(meeting_id) {
            inner.capture_owner = None;
        }
        let ended = SessionPatch {
            ended_at: Some(chrono::Utc::now()),
            ..SessionPatch::default()
        };
        self.transition(&mut inner, meeting_id, MeetingState::Cancelled, ended)?;
        drop(inner);
        self.snapshot(meeting_id)
    }

    /// T14 — user retries failed jobs.
    pub fn retry_jobs(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        {
            let conn = lock(&self.conn);
            conn.execute(
                // STT §3.2: an explicit user action moves `failed` back to
                // `queued` without touching the attempt history.
                "UPDATE meeting_stt_jobs
                 SET state = 'queued', next_attempt_at = NULL, last_error = NULL, updated_at = ?2
                 WHERE meeting_id = ?1 AND state = 'failed'",
                params![meeting_id, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        }
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        let state = inner
            .sessions
            .get(meeting_id)
            .map(|r| r.state)
            .unwrap_or(MeetingState::Draft);
        if state == MeetingState::RecoverableError {
            let previous = inner
                .sessions
                .get(meeting_id)
                .and_then(|r| r.previous_active)
                .filter(|s| *s == MeetingState::Summarizing)
                .unwrap_or(MeetingState::Transcribing);
            let clear = SessionPatch {
                last_error: Some(String::new()),
                ..SessionPatch::default()
            };
            self.transition(&mut inner, meeting_id, previous, clear)?;
            if let Some(runtime) = inner.sessions.get_mut(meeting_id) {
                runtime.last_error = None;
            }
        }
        drop(inner);
        self.publish_jobs(meeting_id);
        self.wake_queue();
        self.snapshot(meeting_id)
    }

    /// The provider snapshot persisted at session creation, so a meeting keeps
    /// its STT backend even if global settings change mid-recording. Falls
    /// back to the live environment when the row predates the column.
    fn session_provider(&self, meeting_id: &str) -> (String, serde_json::Value) {
        let stored: Option<(String, String, Option<String>)> = {
            let conn = lock(&self.conn);
            conn.query_row(
                "SELECT stt_provider, provider_config_snapshot, language
                 FROM meeting_sessions WHERE id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, Option<String>>(2)?)),
            )
            .optional()
            .unwrap_or(None)
        };
        match stored {
            Some((name, snapshot_raw, language)) => {
                let mut config: serde_json::Value =
                    serde_json::from_str(&snapshot_raw).unwrap_or(serde_json::json!({}));
                if let Some(language) = language.filter(|l| !l.is_empty()) {
                    config["language"] = serde_json::Value::String(language);
                }
                (name, config)
            }
            None => {
                let provider = self.env.provider();
                (
                    provider.name.clone(),
                    serde_json::json!({ "kind": provider.kind }),
                )
            }
        }
    }

    /// Enqueues one STT job per not-yet-covered `ready` chunk (plan line 1193:
    /// segment-wise processing; STT §3 idempotent enqueue).
    ///
    /// A chunk is "covered" once any `meeting_stt_job_chunks` row references
    /// it, regardless of that job's state — retries own their chunks.
    fn enqueue_ready_chunks(&self, meeting_id: &str) -> Result<usize> {
        let rows: Vec<(String, String, i64, i64)> = {
            let conn = lock(&self.conn);
            let mut stmt = conn.prepare(
                "SELECT id, source, timeline_start_ms, timeline_end_ms
                 FROM meeting_audio_chunks
                 WHERE meeting_id = ?1 AND state = 'ready' AND deleted_at IS NULL
                   AND id NOT IN (SELECT chunk_id FROM meeting_stt_job_chunks)
                 ORDER BY sequence",
            )?;
            let mapped = stmt.query_map(params![meeting_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if rows.is_empty() {
            return Ok(0);
        }
        let (provider_name, provider_config) = self.session_provider(meeting_id);
        let mut enqueued = 0usize;
        for (id, source, start, end) in rows {
            self.store.enqueue_stt_job(NewSttJob {
                meeting_id: StoreUuid(meeting_id.to_string()),
                chunk_ids: vec![StoreUuid(id)],
                source,
                timeline_start_ms: start,
                timeline_end_ms: end,
                provider: provider_name.clone(),
                config_snapshot: provider_config.clone(),
                next_attempt_at: None,
            })?;
            enqueued += 1;
        }
        Ok(enqueued)
    }

    /// Incremental counterpart of [`Self::enqueue_ready_chunks`] for one chunk
    /// the recorder just finalized. Returns `false` when the chunk is already
    /// covered by a job or not in `ready` state yet.
    fn enqueue_chunk_job(&self, meeting_id: &str, info: &rec::FinalizedChunkInfo) -> Result<bool> {
        let source = MeetingAudioSource::from_recorder(info.source).as_str();
        let row: Option<(String, i64, i64)> = {
            let conn = lock(&self.conn);
            conn.query_row(
                "SELECT id, timeline_start_ms, timeline_end_ms
                 FROM meeting_audio_chunks
                 WHERE meeting_id = ?1 AND sequence = ?2 AND source = ?3
                   AND state = 'ready' AND deleted_at IS NULL",
                params![meeting_id, info.sequence as i64, source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
        };
        let Some((chunk_id, start, end)) = row else {
            return Ok(false);
        };
        let covered: bool = {
            let conn = lock(&self.conn);
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_stt_job_chunks WHERE chunk_id = ?1)",
                params![chunk_id],
                |row| row.get(0),
            )?
        };
        if covered {
            return Ok(false);
        }
        let (provider_name, provider_config) = self.session_provider(meeting_id);
        self.store.enqueue_stt_job(NewSttJob {
            meeting_id: StoreUuid(meeting_id.to_string()),
            chunk_ids: vec![StoreUuid(chunk_id)],
            source: source.to_string(),
            timeline_start_ms: start,
            timeline_end_ms: end,
            provider: provider_name,
            config_snapshot: provider_config,
            next_attempt_at: None,
        })?;
        Ok(true)
    }

    // -- snapshots and aggregates ------------------------------------------

    /// SSC §5 / O5 — the authoritative rebuild source for every window.
    pub fn snapshot(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        let mut inner = lock(&self.inner);
        self.ensure_tracked(&mut inner, meeting_id)?;
        let runtime = inner
            .sessions
            .get(meeting_id)
            .cloned()
            .ok_or_else(|| format!("unknown meeting session {meeting_id}"))?;
        drop(inner);

        Ok(MeetingSnapshot {
            meeting_id: meeting_id.to_string(),
            title: runtime.title,
            state: runtime.state,
            state_revision: runtime.revision,
            elapsed_ms: runtime.elapsed_base_ms,
            running_since: runtime.running_since,
            microphone_only: Some(runtime.microphone_only),
            provider_kind: runtime.provider_kind,
            provider_name: runtime.provider_name,
            summary_enabled: Some(runtime.summary_enabled),
            transcript_head_sequence: self.transcript_head(meeting_id).ok(),
            jobs: self.job_counts(meeting_id).ok(),
            levels: Some(runtime.levels),
            last_error: runtime.last_error,
        })
    }

    fn transcript_head(&self, meeting_id: &str) -> Result<i64> {
        let conn = lock(&self.conn);
        let head: Option<i64> = conn.query_row(
            "SELECT MAX(sequence) FROM meeting_transcript_segments
             WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |row| row.get(0),
        )?;
        Ok(head.unwrap_or(0))
    }

    fn job_counts(&self, meeting_id: &str) -> Result<MeetingJobsUpdatedEvent> {
        let conn = lock(&self.conn);
        let mut stmt = conn.prepare(
            "SELECT source, state, COUNT(*) FROM meeting_stt_jobs
             WHERE meeting_id = ?1 GROUP BY source, state",
        )?;
        let mapped = stmt.query_map(params![meeting_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let rows = mapped.collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);

        let mut per_source: HashMap<String, MeetingJobCounts> = HashMap::new();
        let mut backlog = 0i64;
        let mut failed = 0i64;
        let mut succeeded = 0i64;
        let mut eligible = 0i64;
        for (source, state, count) in rows {
            let entry = per_source.entry(source).or_default();
            eligible += count;
            match state.as_str() {
                "queued" => {
                    entry.queued += count;
                    backlog += count;
                }
                "running" => {
                    entry.running += count;
                    backlog += count;
                }
                "retry_wait" => {
                    entry.retry_wait += count;
                    backlog += count;
                }
                "failed" => {
                    entry.failed += count;
                    failed += count;
                }
                "succeeded" => {
                    entry.succeeded += count;
                    succeeded += count;
                }
                _ => {}
            }
        }
        let revision = lock(&self.inner)
            .sessions
            .get(meeting_id)
            .map(|r| r.revision);
        Ok(MeetingJobsUpdatedEvent {
            meeting_id: meeting_id.to_string(),
            per_source,
            total_backlog: backlog,
            oldest_failed: (failed > 0).then_some(MeetingOldestFailed {
                count: failed,
                first_failed_at: None,
            }),
            state_revision: revision,
            eligible_total: Some(eligible),
            succeeded_total: Some(succeeded),
            prepared_total: Some(eligible),
            prepared_done: Some(succeeded),
        })
    }

    fn publish_jobs(&self, meeting_id: &str) {
        match self.job_counts(meeting_id) {
            Ok(event) => self.publish(TOPIC_JOBS_UPDATED, &event),
            Err(e) => warn!("Could not compute job counts for {}: {}", meeting_id, e),
        }
    }

    /// Queue-drain settlement: when a session in `transcribing` has no
    /// backlog left, it advances to `completed` — or `recoverable_error` when
    /// jobs failed terminally (SSC T10/T11/T13; STT §3.2).
    fn settle_after_queue(&self, meeting_id: &str) {
        let Ok(counts) = self.job_counts(meeting_id) else {
            return;
        };
        if counts.total_backlog > 0 {
            return;
        }
        let mut inner = lock(&self.inner);
        let Some(runtime) = inner.sessions.get(meeting_id) else {
            return;
        };
        if runtime.state != MeetingState::Transcribing {
            return;
        }
        let failed = counts.oldest_failed.is_some();
        // Phase 7: a finished transcription flows into summarization only when
        // the user enabled it AND the persisted cloud consent is set. Anything
        // else completes without ever touching an LLM.
        let wants_summary = !failed && runtime.summary_enabled && self.env.summary_cloud_consent();
        let (destination, patch) = if failed {
            (
                MeetingState::RecoverableError,
                SessionPatch {
                    last_error: Some("sttFailed".to_string()),
                    ..SessionPatch::default()
                },
            )
        } else if wants_summary {
            (MeetingState::Summarizing, patch_none())
        } else {
            (MeetingState::Completed, patch_none())
        };
        if let Err(e) = self.transition(&mut inner, meeting_id, destination, patch) {
            warn!("Queue settlement for {} failed: {}", meeting_id, e);
            return;
        }
        let run_summary = destination == MeetingState::Summarizing;
        drop(inner);
        if run_summary {
            // Runs without holding manager locks: the LLM round-trips dominate
            // this phase and no capture is live anymore.
            self.run_summary_job(meeting_id, true);
        }
    }

    /// Executes the map-reduce summary for one meeting and persists exactly
    /// one `meeting_summaries` row (U9 upsert — retries cannot duplicate).
    fn run_summary_job(&self, meeting_id: &str, entered_summarizing: bool) {
        // Consent is re-checked here: flipping the flag mid-run must not be
        // able to sneak a cloud call in through a stale decision.
        let Some((provider_id, model, llm)) = self.env.summary_llm() else {
            self.finish_summary_job(meeting_id, entered_summarizing, Err("summaryNotConfigured"));
            return;
        };
        if !self.env.summary_cloud_consent() {
            self.finish_summary_job(
                meeting_id,
                entered_summarizing,
                Err("summaryConsentRequired"),
            );
            return;
        }

        let segments = match self
            .store
            .segments_for_meeting(StoreUuid(meeting_id.to_string()))
        {
            Ok(segments) => segments,
            Err(e) => {
                self.finish_summary_job(meeting_id, entered_summarizing, Err("summaryFailed"));
                warn!("Summary could not read segments: {}", e);
                return;
            }
        };
        let spans: Vec<summarize::SummarySpan> = segments
            .iter()
            .map(|s| summarize::SummarySpan {
                start_ms: s.start_ms,
                text: s.text.clone(),
            })
            .collect();
        if spans.is_empty() {
            self.finish_summary_job(meeting_id, entered_summarizing, Err("summaryFailed"));
            return;
        }

        let fingerprint = summarize::input_fingerprint(&spans);
        let summarizer = summarize::MeetingSummarizer::new(llm);
        let outcome = match summarizer.summarize(spans) {
            Ok(summary) => {
                let persisted =
                    self.store
                        .upsert_summary(super::meeting_store::NewMeetingSummary {
                            meeting_id: StoreUuid(meeting_id.to_string()),
                            provider: provider_id.clone(),
                            model: Some(model),
                            prompt_version: format!("{}:{fingerprint}", summarize::PROMPT_VERSION),
                            state: "completed".to_string(),
                            summary_markdown: summary.summary_markdown,
                            decisions: serde_json::json!(summary.decisions),
                            action_items: serde_json::json!(summary.action_items),
                            open_questions: serde_json::json!(summary.open_questions),
                            last_error: None,
                        });
                match persisted {
                    Ok(_) => {
                        info!("Meeting {} summarized", meeting_id);
                        Ok(())
                    }
                    Err(e) => {
                        warn!("Summary persist failed: {}", e);
                        Err("summaryFailed")
                    }
                }
            }
            Err(e) => {
                let message = match e {
                    summarize::SummaryError::Llm(m) | summarize::SummaryError::Unparsable(m) => m,
                };
                let _ = self
                    .store
                    .upsert_summary(super::meeting_store::NewMeetingSummary {
                        meeting_id: StoreUuid(meeting_id.to_string()),
                        provider: provider_id,
                        model: Some(model),
                        prompt_version: summarize::PROMPT_VERSION.to_string(),
                        state: "failed".to_string(),
                        summary_markdown: String::new(),
                        decisions: serde_json::json!([]),
                        action_items: serde_json::json!([]),
                        open_questions: serde_json::json!([]),
                        last_error: Some(message),
                    });
                Err("summaryFailed")
            }
        };
        self.finish_summary_job(meeting_id, entered_summarizing, outcome);
    }

    /// Terminal bookkeeping for [`Self::run_summary_job`]: walks Summarizing ?
    /// Completed / RecoverableError when the hook entered via Summarizing; a
    /// re-summarization started from `completed` refreshes silently.
    fn finish_summary_job(
        &self,
        meeting_id: &str,
        entered_summarizing: bool,
        outcome: Result<(), &'static str>,
    ) {
        if !entered_summarizing {
            return;
        }
        let mut inner = lock(&self.inner);
        let (destination, patch) = match outcome {
            Ok(()) => (MeetingState::Completed, patch_none()),
            Err(code) => (
                MeetingState::RecoverableError,
                SessionPatch {
                    last_error: Some(code.to_string()),
                    ..SessionPatch::default()
                },
            ),
        };
        if let Err(e) = self.transition(&mut inner, meeting_id, destination, patch) {
            warn!("Summary settlement for {} failed: {}", meeting_id, e);
        }
    }

    /// Public re-summarization entry ("Neu-Zusammenfassung"). Refuses while
    /// capturing or without consent; from `recoverable_error` it walks the
    /// legal RecoverableError?Summarizing edge, from `completed` it refreshes
    /// the row in place without faking a state transition.
    pub fn request_summary(&self, meeting_id: &str) -> Result<MeetingSnapshot, String> {
        let was_recoverable = {
            let inner = lock(&self.inner);
            let runtime = inner
                .sessions
                .get(meeting_id)
                .ok_or_else(|| "meetingNotFound".to_string())?;
            if runtime.state.is_active() {
                return Err("meetingIsActive".to_string());
            }
            if !runtime.summary_enabled {
                return Err("summaryNotEnabled".to_string());
            }
            runtime.state == MeetingState::RecoverableError
        };

        if !self.env.summary_cloud_consent() {
            return Err("summaryConsentRequired".to_string());
        }

        if was_recoverable {
            let mut inner = lock(&self.inner);
            self.transition(
                &mut inner,
                meeting_id,
                MeetingState::Summarizing,
                patch_none(),
            )?;
        }
        self.run_summary_job(meeting_id, was_recoverable);
        self.snapshot(meeting_id)
    }

    // -- recorder events (SSC O4: serialization point) ----------------------

    fn handle_recorder_event(&self, meeting_id: &str, event: &rec::RecorderEvent) {
        match event {
            rec::RecorderEvent::Started { .. } => {
                let mut inner = lock(&self.inner);
                if inner.sessions.get(meeting_id).map(|r| r.state) == Some(MeetingState::Starting) {
                    let _ = self.transition(
                        &mut inner,
                        meeting_id,
                        MeetingState::Recording,
                        patch_none(),
                    );
                }
            }
            rec::RecorderEvent::LevelUpdate { source, peak, rms } => {
                let source = MeetingAudioSource::from_recorder(*source);
                if let Some(runtime) = lock(&self.inner).sessions.get_mut(meeting_id) {
                    runtime.levels.insert(source.as_str().to_string(), *peak);
                }
                self.publish(
                    TOPIC_LEVEL_UPDATE,
                    &MeetingLevelUpdateEvent {
                        meeting_id: meeting_id.to_string(),
                        source,
                        peak: *peak,
                        rms: *rms,
                        at_timeline_ms: None,
                    },
                );
            }
            rec::RecorderEvent::MicDeviceLost { detail, .. } => {
                self.emit_error(meeting_id, "micLost", Some(detail.clone()), true);
            }
            rec::RecorderEvent::SystemDeviceLost { detail, .. } => {
                self.emit_error(meeting_id, "systemAudioLost", Some(detail.clone()), true);
            }
            rec::RecorderEvent::DiskSpaceLow { free_bytes } => {
                let recoverable = *free_bytes > DISK_STOP_WATERMARK_BYTES;
                self.emit_error(meeting_id, "diskLow", None, recoverable);
            }
            rec::RecorderEvent::ControlledStopInitiated { reason } => {
                // Routes through the same T07→T08 path so "Audio gesichert"
                // stays truthful (SSC §3.3 rule 4).
                warn!("Controlled stop for {} ({:?})", meeting_id, reason);
                if let Err(e) = self.stop(meeting_id, false) {
                    error!("Controlled stop failed for {}: {}", meeting_id, e);
                }
            }
            rec::RecorderEvent::ChunkFinalized { info } => {
                // Incremental STT (plan line 1332): one job per finalized
                // chunk while recording; enqueue is idempotent per chunk.
                match self.enqueue_chunk_job(meeting_id, info) {
                    Ok(true) => {
                        self.publish_jobs(meeting_id);
                        self.wake_queue();
                    }
                    Ok(false) => {}
                    Err(e) => {
                        // The stop-path enqueue is the safety net for this.
                        warn!("Could not enqueue chunk job for {}: {}", meeting_id, e);
                    }
                }
            }
            _ => {}
        }
    }

    // -- startup reconciliation (SSC §6) ------------------------------------

    /// Runs the restart reconciliation protocol (SSC §6.2 R1–R6) exactly once
    /// per app start and returns the offers the UI renders.
    ///
    /// Hard requirement (plan line 894): no session may remain in an active
    /// state after this returns.
    pub fn run_startup_recovery(&self) -> Result<Vec<MeetingRecoveryOffer>> {
        // R1: every session persisted as active describes a dead process,
        // because a live recorder cannot exist before this runs.
        let candidates: Vec<(String, String, i64)> = {
            let conn = lock(&self.conn);
            let mut stmt = conn.prepare(
                "SELECT id, title, updated_at FROM meeting_sessions
                 WHERE deleted_at IS NULL
                   AND state IN ('starting','recording','paused','stopping')
                 ORDER BY updated_at ASC",
            )?;
            let mapped = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };

        // R3: job reconciliation for ALL meetings (STT §3.4 verbatim) —
        // `running` becomes `queued` with no attempt-count penalty, overdue
        // `retry_wait` rows simply become due.
        {
            let conn = lock(&self.conn);
            conn.execute(
                "UPDATE meeting_stt_jobs
                 SET state = 'queued', claimed_by = NULL, claimed_at = NULL, updated_at = ?1
                 WHERE state = 'running'",
                params![now_ms()],
            )?;
        }

        let mut offers = Vec::new();
        for (meeting_id, title, updated_at) in candidates {
            // R2: the recorder owns the open-chunk repair protocol (RC §8);
            // this manager never re-implements it.
            let report = self.recover_open_chunks(&meeting_id);
            let (ready, damaged) = self.chunk_counts(&meeting_id).unwrap_or((0, 0));

            let mut inner = lock(&self.inner);
            if let Err(e) = self.ensure_tracked(&mut inner, &meeting_id) {
                warn!("Skipping recovery of {}: {}", meeting_id, e);
                continue;
            }
            let state = inner
                .sessions
                .get(&meeting_id)
                .map(|r| r.state)
                .unwrap_or(MeetingState::Draft);
            // R5: disposition. (a) mid-capture sessions offer their audio,
            // (b) sessions past capture advance, (c) failures preserve data.
            let destination = match state {
                MeetingState::Stopping => {
                    if ready > 0 {
                        MeetingState::StoppedUnprocessed
                    } else {
                        MeetingState::Completed
                    }
                }
                _ => MeetingState::StoppedUnprocessed,
            };
            let revision = match self.transition(
                &mut inner,
                &meeting_id,
                destination,
                SessionPatch {
                    ended_at: Some(chrono::Utc::now()),
                    ..SessionPatch::default()
                },
            ) {
                Ok(revision) => revision,
                Err(e) => {
                    error!("Recovery disposition failed for {}: {}", meeting_id, e);
                    continue;
                }
            };
            drop(inner);

            let offer = MeetingRecoveryOffer {
                meeting_id: meeting_id.clone(),
                title,
                cause: MeetingRecoveryCause::Crash,
                recovered_chunks: ready,
                damaged_chunks: damaged,
                quarantined_files: report.quarantined_files.len() as i64,
                interrupted_at: chrono::DateTime::from_timestamp_millis(updated_at)
                    .map(|dt| dt.to_rfc3339()),
            };
            // R6: one consolidated event per affected session, after commit.
            self.publish(
                TOPIC_SESSION_RECOVERED,
                &MeetingRecoveryReportEvent {
                    offers: vec![offer.clone()],
                    meeting_id: Some(meeting_id),
                    state_revision: Some(revision),
                },
            );
            offers.push(offer);
        }

        *lock(&self.recoveries) = offers.clone();
        self.publish(
            TOPIC_RECOVERY_REPORT,
            &MeetingRecoveryReportEvent {
                offers: offers.clone(),
                meeting_id: None,
                state_revision: None,
            },
        );
        Ok(offers)
    }

    fn recover_open_chunks(&self, meeting_id: &str) -> rec::RecoveryReport {
        let chunk_store = match SqliteChunkStateStore::open(self.store.clone(), &self.db_path) {
            Ok(store) => Arc::new(store),
            Err(e) => {
                warn!("Recovery store unavailable for {}: {}", meeting_id, e);
                return rec::RecoveryReport::default();
            }
        };
        let recorder = (self.recorder_factory)();
        match recorder.recover_open_chunks(meeting_id, chunk_store) {
            Ok(report) => report,
            Err(e) => {
                // A recorder that cannot run standalone recovery must not
                // block the hard requirement "never stay in `recording`"; the
                // disposition below still runs and the data is preserved.
                warn!("Open-chunk recovery skipped for {}: {}", meeting_id, e);
                rec::RecoveryReport::default()
            }
        }
    }

    fn chunk_counts(&self, meeting_id: &str) -> Result<(i64, i64)> {
        let conn = lock(&self.conn);
        let ready: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_audio_chunks
             WHERE meeting_id = ?1 AND state = 'ready' AND deleted_at IS NULL",
            params![meeting_id],
            |row| row.get(0),
        )?;
        let damaged: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_audio_chunks
             WHERE meeting_id = ?1 AND state = 'damaged' AND deleted_at IS NULL",
            params![meeting_id],
            |row| row.get(0),
        )?;
        Ok((ready, damaged))
    }

    /// Pending recovery offers (`meeting_list_recoveries`).
    pub fn list_recoveries(&self) -> Vec<MeetingRecoveryOffer> {
        lock(&self.recoveries).clone()
    }

    /// Applies the user's decision to one offer (`meeting_resolve_recovery`).
    pub fn resolve_recovery(
        &self,
        meeting_id: &str,
        resolution: MeetingRecoveryResolution,
    ) -> Result<bool, String> {
        match resolution {
            MeetingRecoveryResolution::Later => {}
            MeetingRecoveryResolution::Recover => {
                // T18: the user chose "process now".
                let enqueued = self.enqueue_ready_chunks(meeting_id).unwrap_or(0);
                let destination = if enqueued > 0 {
                    MeetingState::Transcribing
                } else {
                    MeetingState::Completed
                };
                let mut inner = lock(&self.inner);
                self.ensure_tracked(&mut inner, meeting_id)?;
                self.transition(&mut inner, meeting_id, destination, patch_none())?;
                drop(inner);
                self.publish_jobs(meeting_id);
            }
            MeetingRecoveryResolution::Delete => {
                self.soft_delete_session(meeting_id)
                    .map_err(|e| e.to_string())?;
                lock(&self.inner).sessions.remove(meeting_id);
            }
        }
        if !matches!(resolution, MeetingRecoveryResolution::Later) {
            lock(&self.recoveries).retain(|o| o.meeting_id != meeting_id);
        }
        Ok(true)
    }

    /// True while any tracked session holds the capture devices. Used by the
    /// tray/shortcut surfaces so all of them read the same state (plan §6.2
    /// principle 9); never used to gate the dictation path (plan §6.10).
    pub fn active_capture_session(&self) -> Option<String> {
        let inner = lock(&self.inner);
        inner.capture_owner.clone().or_else(|| {
            inner
                .sessions
                .iter()
                .find(|(_, r)| r.state.is_active())
                .map(|(id, _)| id.clone())
        })
    }

    /// State of the session that currently owns capture, if any.
    pub fn active_capture_state(&self) -> Option<MeetingState> {
        let inner = lock(&self.inner);
        inner
            .sessions
            .values()
            .map(|r| r.state)
            .find(|s| s.is_active())
    }
}

/// Empty patch; [`MeetingManager::transition`] fills in the state itself.
fn patch_none() -> SessionPatch {
    SessionPatch::default()
}

/// STT-queue callbacks routed back into the manager. Holds a `Weak` so the
/// worker thread never keeps the manager (and its DB handles) alive.
struct ManagerDelegate {
    manager: Weak<MeetingManager>,
}

impl QueueDelegate for ManagerDelegate {
    fn on_segments_persisted(&self, meeting_id: &str, segments: Vec<MeetingTranscriptSegmentDto>) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let last_sequence = segments.last().map(|s| s.sequence).unwrap_or(0);
        let revision = lock(&manager.inner)
            .sessions
            .get(meeting_id)
            .map(|r| r.revision);
        manager.publish(
            TOPIC_TRANSCRIPT_DELTA,
            &MeetingTranscriptDeltaEvent {
                meeting_id: meeting_id.to_string(),
                last_sequence,
                state_revision: revision,
                truncated: None,
                segments,
            },
        );
    }

    fn on_queue_progress(&self, meeting_id: &str) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        manager.publish_jobs(meeting_id);
        manager.settle_after_queue(meeting_id);
    }
}

/// Maps a recorder failure onto the error taxonomy code the UI renders.
fn recorder_error_code(error: rec::RecorderError) -> String {
    match error {
        rec::RecorderError::MicDeviceUnavailable { .. } => "micUnavailable",
        rec::RecorderError::SystemOutputUnavailable { .. } => "systemAudioUnavailable",
        rec::RecorderError::LoopbackSharedModeRequired { .. } => "loopbackSharedModeRequired",
        rec::RecorderError::LoopbackUnavailable { .. } => "loopbackUnavailable",
        rec::RecorderError::CapturePermissionDenied { .. } => "permissionDenied",
        rec::RecorderError::InsufficientDiskSpace { .. } => "diskFull",
        rec::RecorderError::ChunkCreateFailed { .. }
        | rec::RecorderError::ChunkStatePersistFailed { .. } => "storageFailed",
        rec::RecorderError::StreamError { .. } => "captureFailed",
        rec::RecorderError::SessionAlreadyActive => "anotherMeetingIsRecording",
        rec::RecorderError::NoActiveSession => "noActiveSession",
        rec::RecorderError::RecoveryFailed { .. } => "recoveryFailed",
    }
    .to_string()
}

/// Renders one device preflight row. A missing microphone blocks the start;
/// a missing system source only warns, because plan E5 lets the user fall
/// back to microphone-only capture explicitly.
fn device_row(
    id: MeetingPreflightCheckId,
    check: Option<&rec::PreflightCheck>,
    configured_name: Option<String>,
    required: bool,
) -> MeetingPreflightCheck {
    let missing_status = if required {
        MeetingPreflightStatus::Error
    } else {
        MeetingPreflightStatus::Warning
    };
    match check {
        Some(check) => {
            let (status, code) = match check.outcome {
                rec::PreflightOutcome::Available => (MeetingPreflightStatus::Ok, None),
                rec::PreflightOutcome::Missing => (missing_status, Some("deviceUnavailable")),
                rec::PreflightOutcome::PermissionDenied => {
                    (MeetingPreflightStatus::Error, Some("permissionDenied"))
                }
                rec::PreflightOutcome::SharedModeRequired => (
                    MeetingPreflightStatus::Warning,
                    Some("loopbackSharedModeRequired"),
                ),
            };
            MeetingPreflightCheck {
                id,
                status,
                value: Some(check.device_name.clone()),
                code: code.map(str::to_string),
                detail: (!check.detail.is_empty()).then(|| check.detail.clone()),
            }
        }
        None => MeetingPreflightCheck {
            id,
            status: missing_status,
            value: configured_name,
            code: Some("deviceUnavailable".to_string()),
            detail: None,
        },
    }
}

/// Inert environment used by [`MeetingManager::open`] and by tests that do not
/// care about validation inputs.
pub struct StaticEnvironment {
    /// Device set handed to the recorder.
    pub devices: rec::SourceDeviceSet,
    /// Device availability answers.
    pub checks: Vec<rec::PreflightCheck>,
    /// Free bytes reported by the volume probe.
    pub free_bytes: Option<u64>,
    /// Provider snapshot.
    pub provider: ProviderSnapshot,
    /// OS capture permission.
    pub permission_denied: bool,
    /// Summary cloud consent (Phase 7 tests).
    pub summary_consent: std::sync::atomic::AtomicBool,
    /// Injected summary LLM (Phase 7 tests).
    pub llm: std::sync::Mutex<Option<SummaryLlmConfig>>,
}

impl Default for StaticEnvironment {
    fn default() -> Self {
        Self {
            devices: rec::SourceDeviceSet {
                microphone: None,
                system: None,
            },
            checks: Vec::new(),
            free_bytes: None,
            provider: ProviderSnapshot::default(),
            permission_denied: false,
            summary_consent: std::sync::atomic::AtomicBool::new(false),
            llm: std::sync::Mutex::new(None),
        }
    }
}

impl MeetingEnvironment for StaticEnvironment {
    fn device_set(&self) -> rec::SourceDeviceSet {
        self.devices.clone()
    }

    fn device_checks(&self, _devices: &rec::SourceDeviceSet) -> Vec<rec::PreflightCheck> {
        self.checks.clone()
    }

    fn free_disk_bytes(&self) -> Option<u64> {
        self.free_bytes
    }

    fn provider(&self) -> ProviderSnapshot {
        self.provider.clone()
    }

    /// Mirrors the real environment: an explicit per-meeting override wins
    /// over whatever default this static fixture carries.
    fn provider_for(
        &self,
        override_provider: Option<crate::settings::SpeechInputProvider>,
    ) -> ProviderSnapshot {
        let (kind, name) = match override_provider {
            Some(crate::settings::SpeechInputProvider::Azure) => ("cloud", "Azure Speech"),
            Some(crate::settings::SpeechInputProvider::Custom) => ("cloud", "Custom endpoint"),
            _ => return self.provider.clone(),
        };
        ProviderSnapshot {
            kind: kind.to_string(),
            name: name.to_string(),
            language: self.provider.language.clone(),
            configured: true,
            summary_provider: None,
            retention_policy: self.provider.retention_policy.clone(),
        }
    }

    fn microphone_permission_granted(&self) -> bool {
        !self.permission_denied
    }

    fn summary_cloud_consent(&self) -> bool {
        self.summary_consent
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn summary_llm(&self) -> Option<SummaryLlmConfig> {
        self.llm.lock().expect("llm").take()
    }
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSession> {
    Ok(MeetingSession {
        id: row.get(0)?,
        title: row.get(1)?,
        state: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meeting_store::{
        NewProjectTask, NewTranscriptSegment, SttJobState, Uuid, WorkerId,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    // ---------------------------------------------------------------- doubles

    /// Records every publication so tests can assert ordering and revisions.
    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl RecordingSink {
        fn topics(&self, topic: &str) -> Vec<serde_json::Value> {
            lock(&self.events)
                .iter()
                .filter(|(t, _)| t == topic)
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    impl MeetingEventSink for RecordingSink {
        fn publish(&self, topic: &'static str, payload: serde_json::Value) {
            lock(&self.events).push((topic.to_string(), payload));
        }
    }

    /// Recorder double implementing the contract surface only.
    struct FakeRecorder {
        state: rec::RecorderState,
        fail_open: bool,
        opened: Arc<AtomicBool>,
    }

    impl rec::MeetingRecorder for FakeRecorder {
        fn preflight(
            &self,
            _devices: &rec::SourceDeviceSet,
        ) -> Result<Vec<rec::PreflightCheck>, rec::RecorderError> {
            Ok(Vec::new())
        }

        fn start_session(
            &mut self,
            _layout: rec::SessionLayout,
            _store: Arc<dyn rec::ChunkStateStore>,
        ) -> Result<(), rec::RecorderError> {
            Ok(())
        }

        fn open_source(
            &mut self,
            _source: rec::SourceKind,
            _request: rec::SourceOpenRequest,
        ) -> Result<i64, rec::RecorderError> {
            if self.fail_open {
                return Err(rec::RecorderError::MicDeviceUnavailable {
                    device_name: "test".into(),
                    detail: "unplugged".into(),
                });
            }
            self.state = rec::RecorderState::Recording;
            self.opened.store(true, Ordering::SeqCst);
            Ok(1)
        }

        fn close_source(
            &mut self,
            _source: rec::SourceKind,
            _reason: rec::StopReason,
        ) -> Result<(), rec::RecorderError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<rec::TimelineMarker, rec::RecorderError> {
            self.state = rec::RecorderState::Paused;
            Ok(marker(rec::MarkerKind::Pause))
        }

        fn resume(
            &mut self,
            _devices: rec::SourceDeviceSet,
        ) -> Result<rec::TimelineMarker, rec::RecorderError> {
            self.state = rec::RecorderState::Recording;
            Ok(marker(rec::MarkerKind::Resume))
        }

        fn stop(&mut self) -> Result<rec::SessionSummary, rec::RecorderError> {
            self.state = rec::RecorderState::Idle;
            Ok(rec::SessionSummary::default())
        }

        fn subscribe(&mut self, _sink: Arc<dyn rec::RecorderEventSink>) {}

        fn snapshot(&self) -> rec::RecorderSnapshot {
            rec::RecorderSnapshot {
                state: self.state,
                timeline_now_ms: 0.0,
                sources: Vec::new(),
                markers: Vec::new(),
            }
        }

        fn recover_open_chunks(
            &self,
            _session_id: &str,
            _store: Arc<dyn rec::ChunkStateStore>,
        ) -> Result<rec::RecoveryReport, rec::RecorderError> {
            Ok(rec::RecoveryReport::default())
        }
    }

    fn marker(kind: rec::MarkerKind) -> rec::TimelineMarker {
        rec::TimelineMarker {
            kind,
            timeline_ms: 0.0,
            wall_clock: std::time::SystemTime::now(),
        }
    }

    fn fake_factory(fail_open: bool) -> RecorderFactory {
        Arc::new(move || {
            Box::new(FakeRecorder {
                state: rec::RecorderState::Idle,
                fail_open,
                opened: Arc::new(AtomicBool::new(false)),
            })
        })
    }

    fn healthy_env() -> StaticEnvironment {
        StaticEnvironment {
            devices: rec::SourceDeviceSet {
                microphone: Some(rec::SourceOpenRequest::new("Mic")),
                system: Some(rec::SourceOpenRequest::new("Speakers")),
            },
            checks: vec![
                rec::PreflightCheck {
                    source: rec::SourceKind::Microphone,
                    device_name: "Mic".into(),
                    outcome: rec::PreflightOutcome::Available,
                    native_sample_rate_hint: Some(48_000),
                    detail: String::new(),
                },
                rec::PreflightCheck {
                    source: rec::SourceKind::System,
                    device_name: "Speakers".into(),
                    outcome: rec::PreflightOutcome::Available,
                    native_sample_rate_hint: Some(48_000),
                    detail: String::new(),
                },
            ],
            free_bytes: Some(50_000_000_000),
            provider: ProviderSnapshot::default(),
            permission_denied: false,
            summary_consent: std::sync::atomic::AtomicBool::new(false),
            llm: std::sync::Mutex::new(None),
        }
    }

    struct Harness {
        dir: tempfile::TempDir,
        manager: MeetingManager,
        sink: Arc<RecordingSink>,
        db_path: PathBuf,
    }

    fn harness_with(env: StaticEnvironment, fail_open: bool) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("meetings.db");
        let sink = Arc::new(RecordingSink::default());
        let manager = MeetingManager::open_with(
            db_path.clone(),
            dir.path().join("assets"),
            sink.clone(),
            Arc::new(env),
            fake_factory(fail_open),
        )
        .expect("open manager");
        Harness {
            dir,
            manager,
            sink,
            db_path,
        }
    }

    fn harness() -> Harness {
        harness_with(healthy_env(), false)
    }

    fn temp_manager() -> (tempfile::TempDir, MeetingManager) {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            MeetingManager::open(dir.path().join("meetings.db")).expect("open meeting db");
        (dir, manager)
    }

    // ------------------------------------------------------- wave-1 CRUD

    /// Journey step 1+2: a created session can be opened again by id.
    #[test]
    fn create_then_open_returns_draft_session() {
        let (_dir, manager) = temp_manager();
        let before = chrono::Utc::now().timestamp_millis();
        let created = manager.create_session(Some("Kickoff".into())).unwrap();

        assert_eq!(created.title, "Kickoff");
        assert_eq!(created.state, SESSION_STATE_DRAFT);
        assert!(created.started_at >= before);

        let opened = manager.get_session(&created.id).unwrap().expect("found");
        assert_eq!(opened.id, created.id);
        assert_eq!(opened.title, "Kickoff");
    }

    /// Empty/absent titles fall back to a non-empty neutral title.
    #[test]
    fn create_with_blank_title_falls_back_to_neutral_title() {
        let (_dir, manager) = temp_manager();
        assert!(!manager
            .create_session(Some("   ".into()))
            .unwrap()
            .title
            .is_empty());
        assert!(!manager.create_session(None).unwrap().title.is_empty());
    }

    /// Journey step 3 (save): renaming persists across a close/reopen.
    #[test]
    fn rename_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("meetings.db");

        let created = {
            let manager = MeetingManager::open(db_path.clone()).unwrap();
            manager.create_session(Some("Before".into())).unwrap()
        };

        let reopened = MeetingManager::open(db_path.clone()).unwrap();
        let saved = reopened
            .rename_session(&created.id, "Real title".into())
            .unwrap();
        assert_eq!(saved.title, "Real title");

        let third = MeetingManager::open(db_path).unwrap();
        assert_eq!(
            third.get_session(&created.id).unwrap().unwrap().title,
            "Real title"
        );
    }

    /// Renaming rejects blank titles and unknown ids.
    #[test]
    fn rename_rejects_blank_title_and_unknown_id() {
        let (_dir, manager) = temp_manager();
        let created = manager.create_session(None).unwrap();
        assert!(manager.rename_session(&created.id, "  ".into()).is_err());
        assert!(manager
            .rename_session("00000000-0000-4000-8000-00000000dead", "x".into())
            .is_err());
    }

    /// Journey step 4 (list): newest first, query filters on the title.
    #[test]
    fn lists_newest_first_and_filters_by_query() {
        let (_dir, manager) = temp_manager();
        let alpha = manager.create_session(Some("Alpha Sync".into())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let beta = manager
            .create_session(Some("Budget Review".into()))
            .unwrap();

        let all = manager.list_sessions(None).unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec![beta.id.as_str(), alpha.id.as_str()]);

        let filtered = manager
            .list_sessions(Some("budget".to_string()).as_deref())
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, beta.id);
    }

    /// Pen-test: hostile titles (empty, oversized) are rejected at the
    /// boundary instead of flowing into the database and every event.
    #[test]
    fn titles_are_length_capped_and_non_empty() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();

        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(h.manager.rename_session(&draft.meeting_id, long).is_err());
        assert!(h
            .manager
            .create_session(Some("y".repeat(MAX_TITLE_CHARS + 1)))
            .is_err());

        let ok = "x".repeat(MAX_TITLE_CHARS);
        h.manager
            .rename_session(&draft.meeting_id, ok)
            .expect("exactly-at-limit title accepted");
    }

    // ------------------------------------------------------- summary (Phase 7)

    /// Scripted LLM shared with the summary unit tests' contract.
    struct FakeSummaryLlm {
        answer: String,
    }
    impl summarize::SummaryLlm for FakeSummaryLlm {
        fn complete(&self, _prompt: String) -> Result<String, String> {
            Ok(self.answer.clone())
        }
    }

    const SUMMARY_JSON: &str = r##"{"summary_markdown": "# Besprechung", "decisions": ["OK"],
        "action_items": ["Protokoll senden"], "open_questions": ["Budget?"]}"##;

    fn env_with_summary(answer: &'static str) -> StaticEnvironment {
        let mut env = healthy_env();
        env.summary_consent
            .store(true, std::sync::atomic::Ordering::SeqCst);
        env.llm = std::sync::Mutex::new(Some((
            "openrouter".to_string(),
            "test-model".to_string(),
            Box::new(FakeSummaryLlm {
                answer: answer.to_string(),
            }),
        )));
        env
    }

    #[test]
    fn settle_with_summary_enabled_walks_transcribing_summarizing_completed() {
        let h = harness_with(env_with_summary(SUMMARY_JSON), false);
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, true, true)
            .unwrap();
        seed_ready_chunk(&h.manager, &draft.meeting_id);
        h.manager.stop(&draft.meeting_id, false).unwrap();
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Transcribing
        );
        seed_segment_for_first_job(&h.manager, &draft.meeting_id);
        h.manager.settle_after_queue(&draft.meeting_id);

        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Completed
        );

        let summary = h
            .manager
            .store
            .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
            .unwrap()
            .expect("summary persisted");
        assert_eq!(summary.state, "completed");
        assert_eq!(summary.decisions, serde_json::json!(["OK"]));
        assert_eq!(
            summary.action_items,
            serde_json::json!(["Protokoll senden"])
        );
        assert_eq!(summary.open_questions, serde_json::json!(["Budget?"]));
        assert!(summary.prompt_version.contains("meeting-summary-v1"));
    }

    #[test]
    fn without_consent_the_summary_never_runs() {
        let env = healthy_env();
        // The LLM slot is left empty on purpose: if the pipeline ever tried to
        // reach an LLM without consent it would fail loudly here instead of
        // silently skipping — and the state assertions below pin the skip.
        let h = harness_with(env, false);
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, true, true)
            .unwrap();
        h.manager.stop(&draft.meeting_id, false).unwrap();
        // Straight to Completed: no cloud call, no Summarizing state.
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Completed
        );
        assert!(h
            .manager
            .store
            .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn llm_failure_lands_in_recoverable_error_and_persists_a_failed_row() {
        // Consent on, LLM answers empty -> parse fails after the map call.
        let mut env = healthy_env();
        env.summary_consent
            .store(true, std::sync::atomic::Ordering::SeqCst);
        env.llm = std::sync::Mutex::new(Some((
            "openrouter".to_string(),
            "m".to_string(),
            Box::new(FakeSummaryLlm {
                answer: String::new(),
            }),
        )));
        let h = harness_with(env, false);
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, true, true)
            .unwrap();
        seed_ready_chunk(&h.manager, &draft.meeting_id);
        h.manager.stop(&draft.meeting_id, false).unwrap();
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Transcribing
        );
        seed_segment_for_first_job(&h.manager, &draft.meeting_id);
        h.manager.settle_after_queue(&draft.meeting_id);

        // With transcript content and consent present, settlement enters
        // Summarizing; the broken LLM answer must land the meeting in
        // recoverable_error and persist the failed row.
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::RecoverableError
        );
        let summary = h
            .manager
            .store
            .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
            .unwrap()
            .expect("failed row is persisted");
        assert_eq!(summary.state, "failed");
    }

    /// Seeds one ready chunk + one persisted segment through the store API so
    /// summary flows have real content to work on.
    fn seed_ready_chunk(manager: &MeetingManager, meeting_id: &str) {
        let chunk = manager
            .store
            .append_chunk(NewAudioChunk {
                meeting_id: StoreUuid(meeting_id.to_string()),
                sequence: 0,
                source: "microphone".to_string(),
                timeline_start_ms: 0,
                timeline_end_ms: 2_000,
                storage_key: format!("audio/{meeting_id}/000000-microphone.wav"),
                sample_rate: 16_000,
                sample_count: 32_000,
                device_id: Some("test-mic".to_string()),
            })
            .expect("chunk");
        manager
            .store
            .finalize_chunk(
                chunk.envelope.id.clone(),
                ChunkFacts {
                    file_size: 64_064,
                    sample_count: 32_000,
                    checksum: "a".repeat(64),
                    gap_drift_metadata: None,
                },
            )
            .expect("finalize");
    }

    /// Persists one segment for the job created by the stop path and marks the
    /// job succeeded, mirroring what the real queue worker would do before
    /// settlement runs.
    fn seed_segment_for_first_job(manager: &MeetingManager, meeting_id: &str) {
        let _ = meeting_id;
        let now = chrono::Utc::now();
        let claim = manager
            .store
            .claim_due_stt_job(WorkerId("test-worker".to_string()), now)
            .expect("claim")
            .expect("a queued job");
        let job = claim.job;
        manager
            .store
            .insert_segments(
                job.id.clone(),
                vec![NewTranscriptSegment {
                    source: "microphone".to_string(),
                    start_ms: 0,
                    end_ms: 1_500,
                    text: "Wir einigen uns auf Montag.".to_string(),
                    word_timestamps: None,
                    confidence: Some(0.9),
                    provider: "Local".to_string(),
                    model_id: Some("local-whisper".to_string()),
                    sequence: 0,
                }],
            )
            .expect("segments");
        manager
            .store
            .transition_stt_job(job.id, SttJobState::Succeeded, None)
            .expect("succeed");
    }

    /// Full-journey smoke (plan §13 end-to-end): record → STT → summary →
    /// export → project assignment → WhatsApp dedupe → tasks → controlled
    /// deletion. One chain, so a regression anywhere breaks loudly here.
    #[test]
    fn the_whole_plan_journey_works_end_to_end() {
        let h = harness_with(env_with_summary(SUMMARY_JSON), false);

        // -- 1. Meeting lifecycle with pinned provider ----------------------
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start_with_provider(
                &draft.meeting_id,
                None,
                false,
                true,
                true,
                Some(crate::settings::SpeechInputProvider::Azure),
            )
            .unwrap();
        assert_eq!(
            h.manager.session_provider(&draft.meeting_id).0,
            "Azure Speech"
        );

        // -- 2. Capture + local STT queue path ------------------------------
        seed_ready_chunk(&h.manager, &draft.meeting_id);
        h.manager.stop(&draft.meeting_id, false).unwrap();
        seed_segment_for_first_job(&h.manager, &draft.meeting_id);
        h.manager.settle_after_queue(&draft.meeting_id);

        // -- 3. Summary pipeline --------------------------------------------
        let meeting = StoreUuid(draft.meeting_id.clone());
        let summary = h
            .manager
            .store
            .summary_for_meeting(meeting.clone())
            .unwrap()
            .expect("summary persisted");
        assert_eq!(summary.state, "completed");

        // -- 4. Export -------------------------------------------------------
        for format in [
            export::ExportFormat::Markdown,
            export::ExportFormat::Txt,
            export::ExportFormat::Json,
        ] {
            let payload = h
                .manager
                .build_export(&draft.meeting_id, format)
                .unwrap()
                .unwrap();
            assert!(!payload.content.is_empty(), "{format:?} rendered empty");
        }

        // -- 5. Projects: create + assign + inbox -----------------------------
        let store = h.manager.store_handle();
        let project = store
            .create_project(crate::managers::meeting_store::NewProject {
                name: "Journey".to_string(),
                description: String::new(),
            })
            .unwrap();
        store
            .assign_meeting_to_project(StoreUuid(project.id.clone()), meeting.clone())
            .unwrap();
        assert_eq!(
            store
                .list_meeting_ids_for_project(StoreUuid(project.id.clone()))
                .unwrap(),
            vec![draft.meeting_id.clone()]
        );

        // -- 6. WhatsApp import is idempotent ---------------------------------
        let ingestion =
            crate::managers::source_ingestion::SourceIngestionService::new(store.clone());
        let chat = "31.12.24, 23:59 - Alice: Journeys end";
        let first = ingestion.ingest_whatsapp_text("Chat", chat, None).unwrap();
        assert!(first.created);
        let second = ingestion.ingest_whatsapp_text("Chat", chat, None).unwrap();
        assert!(!second.created, "same content must not duplicate");
        assert_eq!(first.source.id, second.source.id);

        // -- 7. Task workflow protects manual edits ---------------------------
        let pid = StoreUuid(project.id.clone());
        let (task, created) = store
            .suggest_task(NewProjectTask {
                project_id: pid.clone(),
                title: "Follow up".to_string(),
                description: String::new(),
                origin: "ai",
            })
            .unwrap();
        assert!(created);
        store
            .decide_task(
                Uuid(task.id.clone()),
                "accepted",
                Some("Follow up Friday".into()),
                None,
            )
            .unwrap();
        let (_same, not_created) = store
            .suggest_task(NewProjectTask {
                project_id: pid,
                title: "Follow up".to_string(),
                description: String::new(),
                origin: "ai",
            })
            .unwrap();
        assert!(!not_created);
        let tasks = store.list_tasks(Uuid(project.id.clone())).unwrap();
        assert_eq!(tasks[0].title, "Follow up Friday", "manual edit survives");

        // -- 8. Controlled deletion keeps transcript on AudioOnly --------------
        h.manager
            .delete_meeting_data(&draft.meeting_id, DeleteScope::AudioOnly)
            .unwrap();
        assert!(h.manager.get_transcript(&draft.meeting_id).unwrap().len() == 1);
        assert!(h.manager.get_summary(&draft.meeting_id).unwrap().is_some());

        // Everything removes session + content.
        h.manager
            .delete_meeting_data(&draft.meeting_id, DeleteScope::Everything)
            .unwrap();
        assert!(h.manager.get_session(&draft.meeting_id).unwrap().is_none());
        // The project survives and simply has no members anymore.
        assert_eq!(
            store
                .list_meeting_ids_for_project(StoreUuid(project.id))
                .unwrap(),
            Vec::<String>::new()
        );
    }

    /// Dumb-user combination matrix: every wild click sequence must end in a
    /// sane state without corrupting the store.
    #[test]
    fn hostile_click_matrix_stays_consistent() {
        let h = harness();

        // Double-start on the same meeting.
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        assert!(h
            .manager
            .start(&draft.meeting_id, None, false, false, true)
            .is_err());

        // Start a second meeting while the first records.
        let second = h.manager.create_draft(None).unwrap();
        assert!(h
            .manager
            .start(&second.meeting_id, None, false, false, true)
            .is_err());

        // Pause/pause/resume/resume spam.
        h.manager.pause(&draft.meeting_id).unwrap();
        assert!(h.manager.pause(&draft.meeting_id).is_err());
        h.manager.resume(&draft.meeting_id).unwrap();
        assert!(h.manager.resume(&draft.meeting_id).is_err());

        // Cancel is illegal mid-recording (must stop first) — refused.
        assert!(h.manager.cancel(&draft.meeting_id).is_err());

        // Stop, then stop again: terminal states reject further stops.
        h.manager.stop(&draft.meeting_id, false).unwrap();
        assert!(h.manager.stop(&draft.meeting_id, false).is_err());

        // Cancel AFTER completion is refused (terminal is terminal).
        assert!(h.manager.cancel(&draft.meeting_id).is_err());

        // The second draft is unaffected and can start after the first ended.
        h.manager
            .start(&second.meeting_id, None, false, false, true)
            .unwrap();
        h.manager.stop(&second.meeting_id, false).unwrap();

        // Cancel from a terminal state is refused; from a fresh draft it works.
        assert!(h.manager.cancel(&second.meeting_id).is_err());
        let third = h.manager.create_draft(None).unwrap();
        h.manager.cancel(&third.meeting_id).unwrap();

        // Pause→stop and resume→stop are legal exits from both states.
        let fourth = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&fourth.meeting_id, None, false, false, true)
            .unwrap();
        h.manager.pause(&fourth.meeting_id).unwrap();
        h.manager.stop(&fourth.meeting_id, false).unwrap();
        let fifth = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&fifth.meeting_id, None, false, false, true)
            .unwrap();
        h.manager.pause(&fifth.meeting_id).unwrap();
        h.manager.resume(&fifth.meeting_id).unwrap();
        h.manager.stop(&fifth.meeting_id, false).unwrap();

        // Deleting while transcription is in flight must be refused: the STT
        // jobs would write into a tombstoned session. After settlement the
        // meeting becomes deletable again.
        let sixth = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&sixth.meeting_id, None, false, false, true)
            .unwrap();
        seed_ready_chunk(&h.manager, &sixth.meeting_id);
        h.manager.stop(&sixth.meeting_id, false).unwrap();
        assert_eq!(
            h.manager.snapshot(&sixth.meeting_id).unwrap().state,
            MeetingState::Transcribing
        );
        assert!(h.manager.soft_delete_session(&sixth.meeting_id).is_err());
        assert!(h
            .manager
            .delete_meeting_data(&sixth.meeting_id, DeleteScope::Everything)
            .is_err());
        seed_segment_for_first_job(&h.manager, &sixth.meeting_id);
        h.manager.settle_after_queue(&sixth.meeting_id);
        assert!(h.manager.soft_delete_session(&sixth.meeting_id).unwrap());

        // Deleting a cancelled meeting works; deleting again is a no-op false.
        assert!(h.manager.soft_delete_session(&second.meeting_id).unwrap());

        // Rename on a deleted session is refused.
        assert!(h
            .manager
            .rename_session(&second.meeting_id, "Ghost".to_string())
            .is_err());
    }

    /// Everything removes all content rows and the session itself.
    #[test]
    fn controlled_deletion_respects_the_two_scopes() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        seed_ready_chunk(&h.manager, &draft.meeting_id);
        h.manager.stop(&draft.meeting_id, false).unwrap();
        seed_segment_for_first_job(&h.manager, &draft.meeting_id);
        // Settle like the real queue would — deletion during `transcribing`
        // is refused (see hostile click matrix).
        h.manager.settle_after_queue(&draft.meeting_id);

        // A raw audio directory exists for this meeting before deletion.
        let meeting_tree = h
            .dir
            .path()
            .join("assets")
            .join("meetings")
            .join(&draft.meeting_id);
        std::fs::create_dir_all(&meeting_tree).expect("create tree");

        // AudioOnly keeps transcript rows and the session.
        h.manager
            .delete_meeting_data(&draft.meeting_id, DeleteScope::AudioOnly)
            .unwrap();
        assert!(!meeting_tree.exists(), "audio files are removed");
        {
            let conn = lock(&h.manager.conn);
            let alive_segments: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM meeting_transcript_segments
                     WHERE meeting_id = ?1 AND deleted_at IS NULL",
                    params![draft.meeting_id],
                    |r| r.get(0),
                )
                .unwrap_or(-1);
            assert_eq!(alive_segments, 1, "transcript survives AudioOnly");
        }

        // Everything additionally tombstones content and the session.
        h.manager
            .delete_meeting_data(&draft.meeting_id, DeleteScope::Everything)
            .unwrap();
        assert!(h.manager.get_session(&draft.meeting_id).unwrap().is_none());

        let conn = lock(&h.manager.conn);
        let alive_segments: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meeting_transcript_segments WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![draft.meeting_id],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        assert_eq!(alive_segments, 0, "segments are gone after full purge");
    }

    /// Pen-test: deleting the session that currently owns the capture must be
    /// refused — the recorder would keep writing into a tombstoned row.
    #[test]
    fn delete_refuses_an_actively_capturing_session() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();

        let err = h
            .manager
            .soft_delete_session(&draft.meeting_id)
            .expect_err("active capture must block deletion");
        assert!(err.to_string().contains("meetingIsActive"));

        // After stopping, deletion works as usual.
        h.manager.stop(&draft.meeting_id, false).unwrap();
        assert!(h.manager.soft_delete_session(&draft.meeting_id).unwrap());
    }

    /// Delete hides the session from get/list but keeps the tombstoned row.
    #[test]
    fn delete_is_soft_and_hides_session() {
        let (_dir, manager) = temp_manager();
        let created = manager.create_session(Some("Doomed".into())).unwrap();

        assert!(manager.soft_delete_session(&created.id).unwrap());
        assert!(!manager.soft_delete_session(&created.id).unwrap());
        assert!(manager.get_session(&created.id).unwrap().is_none());
        assert!(manager.list_sessions(None).unwrap().is_empty());

        let conn = lock(&manager.conn);
        let tombstones: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meeting_sessions WHERE id = ?1 AND deleted_at IS NOT NULL",
                params![created.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tombstones, 1);
    }

    /// Migrations are idempotent: reopening an up-to-date database is a no-op.
    #[test]
    fn reopen_does_not_duplicate_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("meetings.db");
        drop(MeetingManager::open(db_path.clone()).unwrap());
        drop(MeetingManager::open(db_path.clone()).unwrap());

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meeting_sessions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // -------------------------------------------------- state machine table

    /// The SSC §2.2 table, asserted row by row, plus the closed complement:
    /// every pair not in the table is rejected.
    #[test]
    fn transition_table_matches_the_contract_and_rejects_everything_else() {
        use MeetingState::*;
        let legal = [
            (Draft, Starting),
            (Draft, Cancelled),
            (Starting, Recording),
            (Starting, Draft),
            (Starting, Cancelled),
            (Starting, RecoverableError),
            (Starting, StoppedUnprocessed),
            (Recording, Paused),
            (Recording, Stopping),
            (Recording, RecoverableError),
            (Recording, StoppedUnprocessed),
            (Paused, Recording),
            (Paused, Stopping),
            (Paused, RecoverableError),
            (Paused, StoppedUnprocessed),
            (Stopping, Transcribing),
            (Stopping, Completed),
            (Stopping, StoppedUnprocessed),
            (Stopping, RecoverableError),
            (Transcribing, Summarizing),
            (Transcribing, Completed),
            (Transcribing, RecoverableError),
            (Summarizing, Completed),
            (Summarizing, RecoverableError),
            (RecoverableError, Transcribing),
            (RecoverableError, Summarizing),
            (RecoverableError, Completed),
            (StoppedUnprocessed, Transcribing),
            (StoppedUnprocessed, Completed),
        ];
        for (from, to) in legal {
            assert!(
                transition_allowed(from, to),
                "{} -> {} must be legal",
                from.as_str(),
                to.as_str()
            );
        }

        let all = [
            Draft,
            Starting,
            Recording,
            Paused,
            Stopping,
            Transcribing,
            Summarizing,
            Completed,
            StoppedUnprocessed,
            Cancelled,
            RecoverableError,
        ];
        for from in all {
            for to in all {
                let expected = legal.contains(&(from, to));
                assert_eq!(
                    transition_allowed(from, to),
                    expected,
                    "{} -> {} classified wrongly",
                    from.as_str(),
                    to.as_str()
                );
            }
        }

        // The forbidden shortcuts the contract calls out by name.
        assert!(!transition_allowed(Recording, Completed));
        assert!(!transition_allowed(Completed, Recording));
        assert!(!transition_allowed(Cancelled, Draft));
    }

    /// Every state token round-trips through the persisted representation and
    /// matches the TypeScript union exactly.
    #[test]
    fn state_tokens_match_the_frontend_union() {
        for (state, token) in [
            (MeetingState::Draft, "draft"),
            (MeetingState::Starting, "starting"),
            (MeetingState::Recording, "recording"),
            (MeetingState::Paused, "paused"),
            (MeetingState::Stopping, "stopping"),
            (MeetingState::Transcribing, "transcribing"),
            (MeetingState::Summarizing, "summarizing"),
            (MeetingState::Completed, "completed"),
            (MeetingState::StoppedUnprocessed, "stopped_unprocessed"),
            (MeetingState::Cancelled, "cancelled"),
            (MeetingState::RecoverableError, "recoverable_error"),
        ] {
            assert_eq!(state.as_str(), token);
            assert_eq!(MeetingState::parse(token), Some(state));
            assert_eq!(serde_json::to_value(state).unwrap(), token);
        }
        assert_eq!(MeetingState::parse("processing"), None);
    }

    // ---------------------------------------------------------- lifecycle

    #[test]
    fn start_pause_resume_stop_walks_the_contract_path() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        assert_eq!(draft.state, MeetingState::Draft);

        let started = h
            .manager
            .start(&draft.meeting_id, Some("Weekly".into()), false, false, true)
            .unwrap();
        assert_eq!(started.state, MeetingState::Recording);
        assert!(started.running_since.is_some());

        let paused = h.manager.pause(&draft.meeting_id).unwrap();
        assert_eq!(paused.state, MeetingState::Paused);
        assert!(paused.running_since.is_none());

        let resumed = h.manager.resume(&draft.meeting_id).unwrap();
        assert_eq!(resumed.state, MeetingState::Recording);

        // No chunks were captured by the fake recorder, so T09 applies.
        let stopped = h.manager.stop(&draft.meeting_id, false).unwrap();
        assert_eq!(stopped.state, MeetingState::Completed);

        let states: Vec<String> = h
            .sink
            .topics(TOPIC_SESSION_STATE)
            .iter()
            .filter_map(|v| v["to"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            states,
            vec![
                "starting",
                "recording",
                "paused",
                "recording",
                "stopping",
                "completed"
            ]
        );
    }

    /// Phase 6: the provider chosen for one meeting is persisted on start and
    /// drives later STT job routing — without touching dictation settings.
    #[test]
    fn start_pins_the_chosen_provider_for_the_session() {
        let h = harness();
        let pinned = h.manager.create_draft(None).unwrap();
        let started = h
            .manager
            .start_with_provider(
                &pinned.meeting_id,
                None,
                false,
                false,
                true,
                Some(crate::settings::SpeechInputProvider::Azure),
            )
            .unwrap();
        assert_eq!(started.provider_kind, "cloud");
        assert_eq!(started.provider_name.as_deref(), Some("Azure Speech"));

        // The queue resolves jobs through this stored snapshot.
        let (name, config) = h.manager.session_provider(&pinned.meeting_id);
        assert_eq!(name, "Azure Speech");
        assert_eq!(config["kind"], "cloud");

        // A sibling meeting keeps the global default.
        let _ = h.manager.stop(&pinned.meeting_id, false);
        let plain = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&plain.meeting_id, None, false, false, true)
            .unwrap();
        let (default_name, _) = h.manager.session_provider(&plain.meeting_id);
        assert_eq!(default_name, "Local");
    }

    /// Fail-testing: a corrupted or hostile provider snapshot in the database
    /// must degrade to an empty config, never panic the queue thread.
    #[test]
    fn session_provider_survives_a_corrupted_snapshot() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        {
            let conn = lock(&h.manager.conn);
            conn.execute(
                "UPDATE meeting_sessions
                 SET stt_provider = 'Azure Speech',
                     provider_config_snapshot = '{not json at all',
                     language = NULL
                 WHERE id = ?1",
                params![draft.meeting_id],
            )
            .expect("corrupt snapshot");
        }
        let (name, config) = h.manager.session_provider(&draft.meeting_id);
        assert_eq!(name, "Azure Speech");
        assert_eq!(
            config,
            serde_json::json!({}),
            "unparsable config degrades to empty"
        );
    }

    /// O2: revisions are strictly increasing, and exactly one event is
    /// published per transition (no gaps caused by concurrent emissions).
    #[test]
    fn revisions_are_strictly_monotone_with_one_event_per_transition() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        h.manager.pause(&draft.meeting_id).unwrap();
        h.manager.resume(&draft.meeting_id).unwrap();
        h.manager.stop(&draft.meeting_id, true).unwrap();

        let events = h.sink.topics(TOPIC_SESSION_STATE);
        let revisions: Vec<i64> = events
            .iter()
            .filter_map(|v| v["state_revision"].as_i64())
            .collect();
        assert_eq!(revisions.len(), events.len());
        for pair in revisions.windows(2) {
            assert!(
                pair[1] > pair[0],
                "revisions must strictly increase, saw {:?}",
                revisions
            );
        }
        // Each published transition consumed exactly one revision.
        for pair in revisions.windows(2) {
            assert_eq!(pair[1] - pair[0], 1, "revision gap in {:?}", revisions);
        }
        assert_eq!(
            h.manager
                .snapshot(&draft.meeting_id)
                .unwrap()
                .state_revision,
            *revisions.last().unwrap()
        );
    }

    /// Illegal transitions are refused and leave the persisted state alone.
    #[test]
    fn illegal_transition_is_refused_and_does_not_persist() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        // pause is only legal from `recording`.
        let err = h.manager.pause(&draft.meeting_id).unwrap_err();
        assert!(err.contains("illegal meeting transition"), "{err}");
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Draft
        );

        // stop from draft is equally illegal.
        assert!(h.manager.stop(&draft.meeting_id, false).is_err());
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Draft
        );
    }

    /// Gate G4 is fail-closed: without consent no capture resource is opened.
    #[test]
    fn start_without_consent_is_refused_before_any_device_opens() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        let err = h
            .manager
            .start(&draft.meeting_id, None, false, false, false)
            .unwrap_err();
        assert_eq!(err, "consentRequired");
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Draft
        );
        assert!(h.sink.topics(TOPIC_SESSION_STATE).is_empty());
        let errors = h.sink.topics(TOPIC_SESSION_ERROR);
        assert_eq!(errors[0]["code"], "consentRequired");
    }

    /// T04: a failing device open returns the session to `draft` with a code.
    #[test]
    fn device_failure_returns_to_draft_with_error() {
        let h = harness_with(healthy_env(), true);
        let draft = h.manager.create_draft(None).unwrap();
        let err = h
            .manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap_err();
        assert_eq!(err, "micUnavailable");
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Draft
        );
        let states: Vec<String> = h
            .sink
            .topics(TOPIC_SESSION_STATE)
            .iter()
            .filter_map(|v| v["to"].as_str().map(str::to_string))
            .collect();
        assert_eq!(states, vec!["starting", "draft"]);
    }

    /// A stop that the user aborts still finalizes, but lands in
    /// `stopped_unprocessed` (T15).
    #[test]
    fn aborted_stop_lands_in_stopped_unprocessed() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        let stopped = h.manager.stop(&draft.meeting_id, true).unwrap();
        assert_eq!(stopped.state, MeetingState::StoppedUnprocessed);
    }

    /// A second session cannot seize the capture devices while one records.
    #[test]
    fn a_second_meeting_cannot_start_while_one_records() {
        let h = harness();
        let first = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&first.meeting_id, None, false, false, true)
            .unwrap();
        let second = h.manager.create_draft(None).unwrap();
        let err = h
            .manager
            .start(&second.meeting_id, None, false, false, true)
            .unwrap_err();
        assert_eq!(err, "anotherMeetingIsRecording");
    }

    /// Plan §6.10: a dictation stays possible while a meeting records. The
    /// meeting path owns no dictation resource, so the dictation binding is
    /// unaffected by an active capture session.
    #[test]
    fn an_active_meeting_leaves_the_dictation_path_untouched() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        assert_eq!(
            h.manager.active_capture_state(),
            Some(MeetingState::Recording)
        );
        // The dictation shortcut ids are still routed to the dictation
        // coordinator, and the meeting ids are disjoint from them.
        assert!(crate::transcription_coordinator::is_transcribe_binding(
            "transcribe"
        ));
        assert!(!crate::shortcut::is_meeting_binding("transcribe"));
        assert!(!crate::transcription_coordinator::is_transcribe_binding(
            crate::settings::MEETING_BINDING_START_STOP
        ));
    }

    // ---------------------------------------------------------- persistence

    /// Phase 4 acceptance (plan line 1577): after a simulated restart the
    /// persisted state and the snapshot agree.
    #[test]
    fn state_survives_a_simulated_restart() {
        let h = harness();
        let draft = h.manager.create_draft(Some("Board".into())).unwrap();
        h.manager
            .start(&draft.meeting_id, None, true, false, true)
            .unwrap();
        h.manager.stop(&draft.meeting_id, true).unwrap();
        let before = h.manager.snapshot(&draft.meeting_id).unwrap();

        let reopened = MeetingManager::open(h.db_path.clone()).unwrap();
        let after = reopened.snapshot(&draft.meeting_id).unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.state, MeetingState::StoppedUnprocessed);
        assert_eq!(after.state_revision, before.state_revision);
        assert_eq!(after.title, "Board");
    }

    // ------------------------------------------------------------- preflight

    #[test]
    fn preflight_passes_with_a_healthy_environment() {
        let h = harness();
        let report = h.manager.preflight();
        assert!(report.can_start);
        assert_eq!(report.checks.len(), 6);
        assert_eq!(report.microphone_only_available, Some(false));
    }

    #[test]
    fn preflight_blocks_on_a_missing_microphone() {
        let mut env = healthy_env();
        env.checks[0].outcome = rec::PreflightOutcome::Missing;
        let h = harness_with(env, false);
        let report = h.manager.preflight();
        assert!(!report.can_start);
        let mic = report
            .checks
            .iter()
            .find(|c| c.id == MeetingPreflightCheckId::Microphone)
            .unwrap();
        assert_eq!(mic.status, MeetingPreflightStatus::Error);
        assert_eq!(mic.code.as_deref(), Some("deviceUnavailable"));
    }

    /// Plan E5: a missing loopback only warns and offers the mic-only fallback.
    #[test]
    fn preflight_offers_microphone_only_when_loopback_is_missing() {
        let mut env = healthy_env();
        env.checks[1].outcome = rec::PreflightOutcome::Missing;
        let h = harness_with(env, false);
        let report = h.manager.preflight();
        assert!(report.can_start);
        assert_eq!(report.microphone_only_available, Some(true));
    }

    #[test]
    fn preflight_blocks_on_low_disk_and_on_unconfigured_provider() {
        let mut env = healthy_env();
        env.free_bytes = Some(1_000_000);
        let h = harness_with(env, false);
        let report = h.manager.preflight();
        assert!(!report.can_start);
        assert!(report
            .checks
            .iter()
            .any(|c| c.code.as_deref() == Some("diskFull")));

        let mut env = healthy_env();
        env.provider.configured = false;
        let h = harness_with(env, false);
        assert!(!h.manager.preflight().can_start);

        let mut env = healthy_env();
        env.permission_denied = true;
        let h = harness_with(env, false);
        assert!(!h.manager.preflight().can_start);
    }

    /// Cloud providers must reserve the upload budget on top of the local one.
    #[test]
    fn preflight_requires_the_cloud_reserve_for_cloud_providers() {
        let mut env = healthy_env();
        env.free_bytes = Some(DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL + 1);
        env.provider.kind = "cloud".to_string();
        let h = harness_with(env, false);
        assert!(!h.manager.preflight().can_start);
    }

    /// Start refuses when the preflight fails, and rewinds `starting → draft`.
    #[test]
    fn start_refuses_when_the_preflight_fails() {
        let mut env = healthy_env();
        env.free_bytes = Some(1);
        let h = harness_with(env, false);
        let draft = h.manager.create_draft(None).unwrap();
        let err = h
            .manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap_err();
        assert_eq!(err, "diskFull");
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Draft
        );
    }

    // -------------------------------------------------------------- recovery

    /// R5(a): a session left in `recording` by a crash is offered for
    /// processing and never stays active.
    #[test]
    fn startup_recovery_disposes_active_sessions_and_produces_offers() {
        let h = harness();
        let draft = h.manager.create_draft(Some("Crashed".into())).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();

        // Simulate the crash: the process dies, the row stays `recording`.
        drop(h.manager);
        let sink = Arc::new(RecordingSink::default());
        let restarted = MeetingManager::open_with(
            h.db_path.clone(),
            PathBuf::from("assets"),
            sink.clone(),
            Arc::new(healthy_env()),
            fake_factory(false),
        )
        .unwrap();

        let offers = restarted.run_startup_recovery().unwrap();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].meeting_id, draft.meeting_id);
        assert_eq!(offers[0].title, "Crashed");
        assert_eq!(offers[0].cause, MeetingRecoveryCause::Crash);
        assert_eq!(
            restarted.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::StoppedUnprocessed
        );
        assert_eq!(restarted.list_recoveries().len(), 1);
        // R6 publishes per session and once consolidated.
        assert_eq!(sink.topics(TOPIC_SESSION_RECOVERED).len(), 1);
        assert_eq!(sink.topics(TOPIC_RECOVERY_REPORT).len(), 1);

        // Idempotence: a second run finds no candidates.
        assert!(restarted.run_startup_recovery().unwrap().is_empty());
    }

    /// R3: crashed `running` jobs return to `queued` without a retry penalty.
    #[test]
    fn startup_recovery_requeues_running_jobs_without_penalty() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        {
            let conn = lock(&h.manager.conn);
            conn.execute(
                "INSERT INTO meeting_stt_jobs
                   (id, meeting_id, source, timeline_start_ms, timeline_end_ms, provider,
                    state, attempt_count, created_at, updated_at)
                 VALUES ('job-1', ?1, 'microphone', 0, 1000, 'local', 'running', 2, 0, 0)",
                params![draft.meeting_id],
            )
            .unwrap();
        }
        h.manager.run_startup_recovery().unwrap();
        let conn = lock(&h.manager.conn);
        let (state, attempts): (String, i64) = conn
            .query_row(
                "SELECT state, attempt_count FROM meeting_stt_jobs WHERE id = 'job-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "queued");
        assert_eq!(attempts, 2, "crash recovery must not count as an attempt");
    }

    /// Resolutions: "later" keeps the offer, "delete" removes the session.
    #[test]
    fn recovery_resolutions_behave_as_the_dataflow_specifies() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        drop(h.manager);
        let restarted = MeetingManager::open_with(
            h.db_path.clone(),
            PathBuf::from("assets"),
            Arc::new(RecordingSink::default()),
            Arc::new(healthy_env()),
            fake_factory(false),
        )
        .unwrap();
        restarted.run_startup_recovery().unwrap();

        assert!(restarted
            .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Later)
            .unwrap());
        assert_eq!(restarted.list_recoveries().len(), 1);

        assert!(restarted
            .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Delete)
            .unwrap());
        assert!(restarted.list_recoveries().is_empty());
        assert!(restarted.get_session(&draft.meeting_id).unwrap().is_none());
    }

    /// T18: "recover" with no eligible audio completes instead of hanging in
    /// `transcribing` forever.
    #[test]
    fn recovering_without_eligible_audio_completes() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, false, false, true)
            .unwrap();
        h.manager.stop(&draft.meeting_id, true).unwrap();
        assert!(h
            .manager
            .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Recover)
            .unwrap());
        assert_eq!(
            h.manager.snapshot(&draft.meeting_id).unwrap().state,
            MeetingState::Completed
        );
    }

    // ---------------------------------------------------------- wire shapes

    /// The snapshot serializes exactly the field names the frontend reads.
    #[test]
    fn snapshot_payload_matches_the_typescript_contract() {
        let h = harness();
        let draft = h.manager.create_draft(Some("Sync".into())).unwrap();
        let value = serde_json::to_value(h.manager.snapshot(&draft.meeting_id).unwrap()).unwrap();
        for field in [
            "meeting_id",
            "title",
            "state",
            "state_revision",
            "elapsed_ms",
            "running_since",
            "microphone_only",
            "provider_kind",
            "provider_name",
            "summary_enabled",
            "transcript_head_sequence",
            "jobs",
            "levels",
            "last_error",
        ] {
            assert!(value.get(field).is_some(), "snapshot lacks {field}");
        }
        assert_eq!(value["state"], "draft");
    }

    /// The state event carries `meeting_id`/`from`/`to`/`state_revision`.
    #[test]
    fn session_state_event_matches_the_typescript_contract() {
        let h = harness();
        let draft = h.manager.create_draft(None).unwrap();
        h.manager
            .start(&draft.meeting_id, None, true, false, true)
            .unwrap();
        let event = &h.sink.topics(TOPIC_SESSION_STATE)[0];
        assert_eq!(event["meeting_id"], draft.meeting_id);
        assert_eq!(event["from"], "draft");
        assert_eq!(event["to"], "starting");
        assert!(event["state_revision"].as_i64().is_some());
        // A microphone-only session says so on the wire (plan E5).
        let recording = &h.sink.topics(TOPIC_SESSION_STATE)[1];
        assert_eq!(recording["microphone_only"], true);
    }

    /// Preflight rows use the camelCase ids and lowercase statuses of the
    /// TypeScript union.
    #[test]
    fn preflight_payload_matches_the_typescript_contract() {
        let h = harness();
        let value = serde_json::to_value(h.manager.preflight()).unwrap();
        let ids: Vec<&str> = value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                "microphone",
                "systemAudio",
                "transcription",
                "language",
                "disk",
                "permission"
            ]
        );
        assert_eq!(value["checks"][0]["status"], "ok");
        assert_eq!(value["can_start"], true);
    }

    /// Recovery offers use the field names `MeetingRecoveryOffer` declares.
    #[test]
    fn recovery_offer_payload_matches_the_typescript_contract() {
        let offer = MeetingRecoveryOffer {
            meeting_id: "m".into(),
            title: "t".into(),
            cause: MeetingRecoveryCause::SleepWake,
            recovered_chunks: 1,
            damaged_chunks: 2,
            quarantined_files: 3,
            interrupted_at: Some("2026-01-01T00:00:00Z".into()),
        };
        let value = serde_json::to_value(&offer).unwrap();
        assert_eq!(value["meeting_id"], "m");
        assert_eq!(value["cause"], "sleep_wake");
        assert_eq!(value["recovered_chunks"], 1);
        assert_eq!(value["damaged_chunks"], 2);
        assert_eq!(value["quarantined_files"], 3);
    }

    /// The chunk-state seam maps recorder ids onto store UUID rows and back.
    #[test]
    fn chunk_state_store_round_trips_through_the_repository() {
        use rec::ChunkStateStore;
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("meetings.db");
        let manager = MeetingManager::open(db_path.clone()).unwrap();
        let session = manager.create_session(Some("Chunks".into())).unwrap();
        let store = SqliteChunkStateStore::open(manager.store.clone(), &db_path).unwrap();

        let descriptor = rec::ChunkDescriptor {
            chunk_id: 0,
            source: rec::SourceKind::Microphone,
            sequence: 0,
            source_segment_id: 1,
            state: rec::ChunkState::Writing,
            range: rec::TimelineRange {
                start_ms: 0.0,
                end_ms: 1000.0,
            },
            sample_count: 16_000,
            storage_key: "audio/000000-microphone.wav".into(),
            checksum_sha256: None,
            recovered: false,
        };
        let id = store
            .insert_writing_chunk(&session.id, &descriptor)
            .unwrap();
        let open = store.open_chunks(&session.id).unwrap();
        assert_eq!(open.len(), 1);

        store
            .transition(
                id,
                rec::ChunkState::Writing,
                rec::ChunkState::Ready,
                &rec::ChunkPatch {
                    sample_count: Some(16_000),
                    checksum_sha256: Some("a".repeat(64)),
                    ..rec::ChunkPatch::default()
                },
            )
            .unwrap();
        assert!(store.open_chunks(&session.id).unwrap().is_empty());
        assert_eq!(manager.chunk_counts(&session.id).unwrap(), (1, 0));

        // A finalized chunk makes the stop path enqueue an STT job (T08).
        assert_eq!(manager.enqueue_ready_chunks(&session.id).unwrap(), 1);
    }
}
