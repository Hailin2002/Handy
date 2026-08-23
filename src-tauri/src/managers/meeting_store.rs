//! MeetingStore repository seam — compile-only scaffold.
//!
//! Transcribed from `docs/meeting-mode-architecture/repository-boundary-contract.md`:
//! §2 (trait seam sketches), §2.1 (trait form of the seam), §4 (units of work
//! U1–U9) and §5 (change-envelope semantics). Entity fields follow
//! `docs/meeting-mode-plan.md` §9, lines 1164–1217; the planned home is plan
//! §5 lines 238–243. This file owns ownership-matrix rows 1–5
//! (`meeting_sessions`, `meeting_audio_chunks`, `meeting_stt_jobs`,
//! `meeting_transcript_segments`, `meeting_summaries`; contract §3).
//!
//! Scaffold status (task-0134):
//!
//! - Traits and entity types only. There is **no SQL**, no concrete
//!   `MeetingStore` struct, no constructor and no manager wiring; the concrete
//!   store (contract §2, rule R2) arrives with its implementation task.
//! - Every place whose exact shape depends on the pending schema authority
//!   (task-0020 / `db-schema.md`) carries a *parity note* instead of guessing
//!   DDL. Contract §0.1 item 1 makes task-0020 the sole schema authority.
//! - The shared seam vocabulary ([`Uuid`], [`ChangeEnvelope`], [`StoreError`],
//!   [`Page`]/[`PageOf`], [`WorkerId`], [`Clock`], [`UuidSource`]) lives here
//!   for now; `knowledge_store.rs` consumes it via `super::meeting_store`. If
//!   a third consumer appears, extract it to a dedicated shared module.
//!
//! Enforcement: the `const` assertions near the bottom of this file fail the
//! build if any type crossing the seam stops being `Send + Sync`.
//! `rusqlite::Connection` is `!Sync` (it wraps a `std::cell::RefCell`;
//! verified against rusqlite 0.37 `src/lib.rs` lines 345–346, no
//! `unsafe impl Sync`), therefore no seam type can embed a raw database
//! connection (see also contract §7.2 point 1 and §7.3).
//!
//! # Compile-time connection ban (gate G1)
//!
//! The doc-tests below pin **both directions** of that gate, so a future edit
//! that smuggles a connection across the seam fails CI instead of review:
//!
//! A type naming a `rusqlite::Connection` cannot satisfy the exact predicate
//! every seam-crossing type must satisfy — this must NOT compile:
//!
//! ```compile_fail
//! const fn assert_send_sync<T: Send + Sync + ?Sized>() {}
//! const _: () = assert_send_sync::<rusqlite::Connection>();
//! ```
//!
//! Control: the predicate itself is satisfiable over the very same dependency
//! (`Mutex` restores `Sync` for the `Send` connection — the shape a concrete
//! store implementation may legally hold *below* the seam), proving the
//! failure above comes from the connection escaping above the seam and not
//! from a broken example. Embeddings behind `&`, `Rc`, or `RefCell` fail the
//! gate identically to the direct field shown above:
//!
//! ```
//! const fn assert_send_sync<T: Send + Sync + ?Sized>() {}
//! const _: () = assert_send_sync::<std::sync::Mutex<rusqlite::Connection>>();
//! ```

// Compile-only scaffold: consumers (MeetingManager, MeetingTranscriptionQueue,
// MeetingSummaryService) arrive with the implementation task, so every entity
// field and trait method is intentionally unused for now. Remove this allow
// when the concrete store is wired in lib.rs.
#![allow(dead_code)]

use std::fmt;

use chrono::{DateTime, Utc};

// ---------------------------------------------------------------------------
// Shared seam vocabulary
// ---------------------------------------------------------------------------

/// Stable entity identifier (contract §5.1; plan line 1151).
///
/// Compile-only skeleton: currently a lowercase-hyphenated text wrapper.
/// Implementation tasks back this with `uuid::Uuid` (v4) generation inside the
/// stores — the contract explicitly leaves the crate choice open as long as
/// §5 semantics hold. Never derived from SQLite autoincrement, never recycled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Uuid(pub String);

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Envelope carried by (or projected onto) every syncable core record.
/// Semantics: contract §5 (id §5.1, revision §5.2, updated_at §5.3,
/// tombstone §5.4); content_hash stays on the entities themselves (§5.5).
#[derive(Debug, Clone)]
pub struct ChangeEnvelope {
    /// UUIDv4, text-stable, never autoincrement-derived (§5.1).
    pub id: Uuid,
    /// Per-entity monotone counter, starts at 1, +1 per committed mutation (§5.2, W4).
    pub revision: i64,
    /// RFC3339 UTC, millisecond precision, from the injected [`Clock`] (§5.3).
    pub updated_at: DateTime<Utc>,
    /// Tombstone marker; `None` == alive (§5.4). Hard DELETE does not exist
    /// for tier-A entities in v1 (W3).
    pub deleted_at: Option<DateTime<Utc>>,
}

impl ChangeEnvelope {
    /// Boolean deletion projection consumed above the seam; consumers never
    /// infer deletion from absence (§5.4).
    pub fn deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// Typed error crossing the seam. No panics across the boundary (contract §2
/// sketch-level notes).
///
/// Parity note vs. contract §2: the sketch shows `Sql(rusqlite::Error)`, but
/// this scaffold maps SQL failures to their display text at the store
/// boundary so that **no rusqlite type appears above the seam** (leaf gate
/// G1). Concrete stores convert `rusqlite::Error` into this variant.
#[derive(Debug)]
pub enum StoreError {
    /// Requested entity does not exist (or only as a tombstone, per read rules).
    NotFound {
        /// Owning-table name, stable for diagnostics.
        entity: &'static str,
        /// Addressed id.
        id: Uuid,
    },
    /// Uniqueness/state-machine/version violations.
    Conflict {
        /// Owning-table name.
        entity: &'static str,
        /// Addressed id.
        id: Uuid,
        /// Human-readable reason; never contains secrets (plan line 1334).
        reason: String,
    },
    /// SQL failure, message mapped at the store boundary (see type doc).
    Sql {
        /// Short operation tag for diagnostics (e.g. `"U1 create_session"`).
        operation: &'static str,
        /// `rusqlite::Error` display text.
        message: String,
    },
    /// Failure surfaced from the AssetStore byte path (rule R4; N3).
    Asset {
        /// Diagnostic message; bytes never cross the seam.
        message: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound { entity, id } => write!(f, "{entity} {id} not found"),
            StoreError::Conflict { entity, id, reason } => {
                write!(f, "conflict on {entity} {id}: {reason}")
            }
            StoreError::Sql { operation, message } => {
                write!(f, "sql failure in {operation}: {message}")
            }
            StoreError::Asset { message } => write!(f, "asset store failure: {message}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Offset/limit page request for list reads (e.g. [`MeetingRepository::list_sessions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Number of leading rows to skip.
    pub offset: u64,
    /// Maximum number of rows to return.
    pub limit: u32,
}

/// Owned page result; row handles never cross the seam (contract §2, read side).
#[derive(Debug, Clone)]
pub struct PageOf<T> {
    /// One page of owned, typed values.
    pub items: Vec<T>,
    /// Total alive-row count for pagination UIs (parity note: exact counting
    /// semantics over tombstones is fixed by task-0020/db-schema.md).
    pub total: u64,
    /// The request that produced this page.
    pub page: Page,
}

/// Identifies the worker claiming an operational job (U4 / K10; tier B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerId(
    /// Stable worker label (process-scoped); format fixed by the scheduler contract.
    pub String,
);

/// Injected clock port; enables fixed clocks in unit tests (contract §5.3).
pub trait Clock: Send + Sync {
    /// Current UTC instant, truncated to millisecond precision by the store.
    fn now(&self) -> DateTime<Utc>;
}

/// Injected UUID-source port; enables deterministic ids in unit tests (§5.1).
pub trait UuidSource: Send + Sync {
    /// Fresh UUIDv4 rendered as lowercase hyphenated text.
    fn new_uuid(&self) -> Uuid;
}

// ---------------------------------------------------------------------------
// Entity types — ownership-matrix rows 1–5 (contract §3)
// ---------------------------------------------------------------------------

/// Chunk lifecycle states; plan line 1187 (`writing`, `ready`, `damaged`,
/// `deleted`). Mutations map to units U5–U7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkState {
    /// Bytes still being appended (U5).
    Writing,
    /// Finalized with checksum and sizes (U6).
    Ready,
    /// Marked damaged (U7).
    Damaged,
    /// Soft-deleted tombstone state (U7, W3).
    Deleted,
}

/// STT job states; plan line 1197. State churn is tier B: no envelope, no
/// `change_log` writes (W6, U4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttJobState {
    /// Waiting for a worker.
    Queued,
    /// Claimed and running.
    Running,
    /// Backing off before the next attempt.
    RetryWait,
    /// Completed successfully.
    Succeeded,
    /// Terminally failed.
    Failed,
    /// Explicitly cancelled.
    Cancelled,
}

/// Row 1 — `meeting_sessions` (plan lines 1164–1178; tier A, U1–U3).
///
/// Parity notes marked below are resolved by task-0020 / `db-schema.md`,
/// the sole schema authority (contract §0.1 item 1).
#[derive(Debug, Clone)]
pub struct MeetingSession {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// User-facing title.
    pub title: String,
    /// Capture start; end is optional while the session is live.
    pub started_at: DateTime<Utc>,
    /// Capture end; `None` until finalized (parity note: nullability pending DDL).
    pub ended_at: Option<DateTime<Utc>>,
    /// Session lifecycle state (parity note: value set pending task-0020 /
    /// session-state-contract alignment; kept textual in the skeleton).
    pub state: String,
    /// STT provider id chosen for the session.
    pub stt_provider: String,
    /// Snapshot of the *non-secret* provider configuration (plan line 1170;
    /// secrets stay in the secret store, plan line 1334).
    pub provider_config_snapshot: serde_json::Value,
    /// Local model used for on-device processing.
    pub meeting_model_id: Option<String>,
    /// Language hint, if any.
    pub language: Option<String>,
    /// Microphone device identifier used (parity note: device identity format pending).
    pub microphone_device: Option<String>,
    /// Output/output-device identifier used (parity note: device identity format pending).
    pub output_device: Option<String>,
    /// Whether microphone audio was captured (plan line 1174).
    pub has_microphone_audio: bool,
    /// Whether system audio was captured (plan line 1174).
    pub has_system_audio: bool,
    /// Retention policy applied to recordings (parity note: policy value set pending).
    pub recording_retention_policy: String,
    /// Summary provider id, if summaries are enabled.
    pub summary_provider: Option<String>,
    /// Summary pipeline state (parity note: value set pending; mirrored by
    /// [`MeetingSummary.state`]).
    pub summary_state: String,
    /// Last operator-visible error, never secrets.
    pub last_error: Option<String>,
    /// Set once at creation, never modified afterwards (§5.3).
    pub created_at: DateTime<Utc>,
}

/// Row 2 — `meeting_audio_chunks` (plan lines 1180–1188; tier A, U5–U7).
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Owning session, referenced by stable UUID only (plan line 1151).
    pub meeting_id: Uuid,
    /// Ordering within the session timeline.
    pub sequence: i64,
    /// Capture source label (microphone/system; parity note: value set pending).
    pub source: String,
    /// Timeline position, milliseconds from session start.
    pub timeline_start_ms: i64,
    /// Timeline end, milliseconds from session start.
    pub timeline_end_ms: i64,
    /// Relative storage key into the AssetStore — never an absolute path
    /// (plan lines 1158–1159, N3; plan's `path` column resolves to this).
    pub storage_key: String,
    /// Sample rate of the stored bytes.
    pub sample_rate: u32,
    /// Number of samples; final value written by [`MeetingRepository::finalize_chunk`].
    pub sample_count: i64,
    /// Stored byte size (plan line 1186).
    pub file_size: i64,
    /// SHA-256 over the exact stored byte sequence, 64-char lowercase hex (§5.5).
    pub checksum: Option<String>,
    /// Lifecycle state (plan line 1187).
    pub state: ChunkState,
    /// Device identifier for gap/drift analysis (plan line 1188; parity note:
    /// identity format pending).
    pub device_id: Option<String>,
    /// Gap-/drift-metadata blob (plan line 1188; structure pending).
    pub gap_drift_metadata: Option<serde_json::Value>,
}

/// Row 3 — `meeting_stt_jobs` (plan lines 1190–1199; tier B, U4 — no
/// envelope, no `change_log`, W6).
#[derive(Debug, Clone)]
pub struct SttJob {
    /// Stable job id (operational record; hard deletes permitted once finished, §5.4).
    pub id: Uuid,
    /// Owning session.
    pub meeting_id: Uuid,
    /// Referenced chunk ids (plan line 1193).
    pub chunk_ids: Vec<Uuid>,
    /// Capture source or Azure channel mapping (plan line 1194; parity note:
    /// channel-mapping shape pending).
    pub source: String,
    /// Timeline coverage start, milliseconds.
    pub timeline_start_ms: i64,
    /// Timeline coverage end, milliseconds.
    pub timeline_end_ms: i64,
    /// Provider id executing the job.
    pub provider: String,
    /// Configuration snapshot without secrets (plan line 1196).
    pub config_snapshot: serde_json::Value,
    /// Job state-machine position (plan line 1197).
    pub state: SttJobState,
    /// Attempts made so far.
    pub attempt_count: u32,
    /// Earliest next retry instant (`retry_wait` backoff).
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Last error text, never secrets.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Row 4 — `meeting_transcript_segments` (plan lines 1201–1208; tier A per
/// ownership-matrix row 4).
///
/// Parity note: plan line 1208 lists only `created_at`, while the ownership
/// matrix assigns tier A (full envelope). Matrix wins for ownership/tier
/// questions; the envelope projection below follows it. Exact column layout
/// is task-0020's call. Their `source_item` projection is maintained by
/// KnowledgeStore (N2) — this store never writes `source_items`.
#[derive(Debug, Clone)]
pub struct TranscriptSegment {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Owning session.
    pub meeting_id: Uuid,
    /// Producing STT job.
    pub job_id: Uuid,
    /// Capture source label (parity note: value set mirrors chunks).
    pub source: String,
    /// Segment start on the audio timeline, milliseconds.
    pub start_ms: i64,
    /// Segment end on the audio timeline, milliseconds.
    pub end_ms: i64,
    /// Recognized text.
    pub text: String,
    /// Optional word-level timestamps (plan line 1206; shape pending).
    pub word_timestamps: Option<serde_json::Value>,
    /// Optional recognition confidence (plan line 1206).
    pub confidence: Option<f32>,
    /// Provider that produced the segment.
    pub provider: String,
    /// Model id used.
    pub model_id: Option<String>,
    /// Ordering within the producing job/meeting.
    pub sequence: i64,
    /// Creation timestamp (plan line 1208).
    pub created_at: DateTime<Utc>,
}

/// Row 5 — `meeting_summaries` (plan lines 1210–1217; tier A, U9).
#[derive(Debug, Clone)]
pub struct MeetingSummary {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Summarized session.
    pub meeting_id: Uuid,
    /// Summary provider id.
    pub provider: String,
    /// Model id used.
    pub model: Option<String>,
    /// Prompt version for reproducibility (plan line 1154).
    pub prompt_version: String,
    /// Pipeline state (parity note: value set pending task-0020).
    pub state: String,
    /// Rendered markdown summary.
    pub summary_markdown: String,
    /// Structured decisions JSON (plan line 1216).
    pub decisions: serde_json::Value,
    /// Structured action items JSON (plan line 1216).
    pub action_items: serde_json::Value,
    /// Structured open questions JSON (plan line 1216).
    pub open_questions: serde_json::Value,
    /// Last pipeline error, never secrets.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Drafts, patches and claims (parameters of the units of work U1–U9)
// ---------------------------------------------------------------------------

/// Draft for [`MeetingRepository::create_session`] (U1). Id, revision,
/// timestamps and envelope are assigned by the store (§5.1–§5.3).
#[derive(Debug, Clone)]
pub struct NewMeetingSession {
    /// Initial title.
    pub title: String,
    /// Capture start.
    pub started_at: DateTime<Utc>,
    /// Chosen STT provider.
    pub stt_provider: String,
    /// Non-secret provider configuration snapshot.
    pub provider_config_snapshot: serde_json::Value,
    /// Local model id.
    pub meeting_model_id: Option<String>,
    /// Language hint.
    pub language: Option<String>,
    /// Microphone device identifier.
    pub microphone_device: Option<String>,
    /// Output device identifier.
    pub output_device: Option<String>,
    /// Microphone audio presence.
    pub has_microphone_audio: bool,
    /// System audio presence.
    pub has_system_audio: bool,
    /// Retention policy for the recording.
    pub recording_retention_policy: String,
    /// Summary provider, if enabled.
    pub summary_provider: Option<String>,
}

/// Partial update for [`MeetingRepository::update_session`] (U2).
/// `None` fields leave the column unchanged; the store bumps revision and
/// `updated_at` itself (W4, §5.3).
#[derive(Debug, Default, Clone)]
pub struct SessionPatch {
    /// New title.
    pub title: Option<String>,
    /// New session state.
    pub state: Option<String>,
    /// Finalized end timestamp.
    pub ended_at: Option<DateTime<Utc>>,
    /// Updated summary state mirror.
    pub summary_state: Option<String>,
    /// Updated error field.
    pub last_error: Option<String>,
    /// Updated retention policy.
    pub recording_retention_policy: Option<String>,
}

/// Draft for [`MeetingRepository::append_chunk`] (U5); starts in
/// [`ChunkState::Writing`] with revision 1.
#[derive(Debug, Clone)]
pub struct NewAudioChunk {
    /// Owning session.
    pub meeting_id: Uuid,
    /// Timeline order.
    pub sequence: i64,
    /// Capture source label.
    pub source: String,
    /// Timeline start, milliseconds.
    pub timeline_start_ms: i64,
    /// Timeline end, milliseconds.
    pub timeline_end_ms: i64,
    /// Relative AssetStore key (never absolute paths, plan lines 1158–1159).
    pub storage_key: String,
    /// Sample rate.
    pub sample_rate: u32,
    /// Samples written so far.
    pub sample_count: i64,
    /// Device identifier.
    pub device_id: Option<String>,
}

/// Facts supplied by [`MeetingRepository::finalize_chunk`] (U6); moves the
/// chunk to [`ChunkState::Ready`] with checksum and sizes (§5.5 damage basis).
#[derive(Debug, Clone)]
pub struct ChunkFacts {
    /// Final stored byte size.
    pub file_size: i64,
    /// Final sample count.
    pub sample_count: i64,
    /// SHA-256 over the exact stored bytes, 64-char lowercase hex.
    pub checksum: String,
    /// Gap/drift metadata recorded at finalize time.
    pub gap_drift_metadata: Option<serde_json::Value>,
}

/// Segment draft for [`MeetingRepository::insert_segments`] (U8); batched
/// inserts share one transaction with one log row per segment.
#[derive(Debug, Clone)]
pub struct NewTranscriptSegment {
    /// Capture source label.
    pub source: String,
    /// Segment start, milliseconds.
    pub start_ms: i64,
    /// Segment end, milliseconds.
    pub end_ms: i64,
    /// Recognized text.
    pub text: String,
    /// Word-level timestamps, if present.
    pub word_timestamps: Option<serde_json::Value>,
    /// Confidence, if known.
    pub confidence: Option<f32>,
    /// Producing provider.
    pub provider: String,
    /// Model id.
    pub model_id: Option<String>,
    /// Ordering within the job.
    pub sequence: i64,
}

/// Summary payload for [`MeetingRepository::upsert_summary`] (U9); the store
/// decides INSERT vs UPDATE from the meeting id (K/U9 semantics, W4).
#[derive(Debug, Clone)]
pub struct NewMeetingSummary {
    /// Summarized session.
    pub meeting_id: Uuid,
    /// Summary provider.
    pub provider: String,
    /// Model id.
    pub model: Option<String>,
    /// Prompt version.
    pub prompt_version: String,
    /// Pipeline state.
    pub state: String,
    /// Markdown summary body.
    pub summary_markdown: String,
    /// Structured decisions.
    pub decisions: serde_json::Value,
    /// Structured action items.
    pub action_items: serde_json::Value,
    /// Structured open questions.
    pub open_questions: serde_json::Value,
    /// Pipeline error, if any.
    pub last_error: Option<String>,
}

/// Lease handed out by [`MeetingRepository::claim_due_stt_job`] (U4, tier B —
/// no change_log, W6; recovery follows STT-§3.4, not the log).
#[derive(Debug, Clone)]
pub struct SttJobClaim {
    /// The claimed job with its post-claim state.
    pub job: SttJob,
    /// Claiming worker.
    pub worker: WorkerId,
    /// Claim instant (lease expiry semantics: STT-§3).
    pub claimed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Trait seam — contract §2 read/write blocks mirrored 1:1 in §2.1 trait form
// ---------------------------------------------------------------------------

/// Repository seam for the meeting capture pipeline.
///
/// Consumed by `MeetingManager`, `MeetingTranscriptionQueue` and
/// `MeetingSummaryService` (contract §6) as `Arc<dyn MeetingRepository>`.
///
/// Contract sources: §2 (sketch, read/write blocks), §2.1 (trait form),
/// §4.2 (units of work U1–U9). Every write member is one unit of work: the
/// implementation opens, commits or rolls back its transaction internally
/// (W1); callers cannot inject a connection and cannot compose partial writes.
/// Reads return owned, typed values — never row handles, connections or SQL
/// strings. Blocking signatures are intentional: callers offload to async
/// runtimes behind Tauri commands (mirrors `HistoryManager`).
///
/// Enforcement: this trait is object-safe by construction (no generic
/// methods, no borrowed-lifetime returns) and — see the `const` assertions
/// below — none of its parameter or return types can embed a
/// `rusqlite::Connection`, because every seam type is `Send + Sync` while
/// `Connection` is `!Sync`.
pub trait MeetingRepository: Send + Sync {
    /// Read: fetch one alive session (read-only list in §4.2).
    fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError>;

    /// Read: paginated session list.
    fn list_sessions(&self, page: Page) -> Result<PageOf<MeetingSession>, StoreError>;

    /// Read: transcript segments of a meeting, timeline-ordered.
    fn segments_for_meeting(&self, meeting_id: Uuid) -> Result<Vec<TranscriptSegment>, StoreError>;

    /// Read: current summary of a meeting, if any.
    fn summary_for_meeting(&self, meeting_id: Uuid) -> Result<Option<MeetingSummary>, StoreError>;

    /// Read (passive inspection): due STT jobs for the queue feed (STT-§3);
    /// claiming happens in U4.
    fn due_stt_jobs(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<SttJob>, StoreError>;

    /// U1 — INSERT `meeting_sessions` (rev = 1) + `change_log`(create).
    fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError>;

    /// U2 — UPDATE + rev++ + `change_log`(update).
    fn update_session(&self, id: Uuid, patch: SessionPatch) -> Result<MeetingSession, StoreError>;

    /// U3 — soft delete: `deleted_at` + rev++ + tombstone log row (W3).
    fn soft_delete_session(&self, id: Uuid) -> Result<(), StoreError>;

    /// U5 — INSERT chunk in state [`ChunkState::Writing`] (rev = 1) + log row.
    fn append_chunk(&self, chunk: NewAudioChunk) -> Result<AudioChunk, StoreError>;

    /// U6 — finalize chunk: checksum/sizes + state [`ChunkState::Ready`]
    /// + rev++ + log row.
    fn finalize_chunk(&self, id: Uuid, facts: ChunkFacts) -> Result<AudioChunk, StoreError>;

    /// U7 — mark chunk damaged/deleted (tombstone log row on delete, W3).
    fn mark_chunk(
        &self,
        id: Uuid,
        state: ChunkState,
        reason: Option<String>,
    ) -> Result<(), StoreError>;

    /// U4 — atomically claim the next due STT job for `worker` (tier B, no log).
    fn claim_due_stt_job(
        &self,
        worker: WorkerId,
        now: DateTime<Utc>,
    ) -> Result<Option<SttJobClaim>, StoreError>;

    /// U4 — advance a job through the plan-line-1197 state machine (tier B, no log).
    fn transition_stt_job(
        &self,
        id: Uuid,
        to: SttJobState,
        err: Option<String>,
    ) -> Result<(), StoreError>;

    /// U8 — batched segment INSERT + one log row per inserted segment, single
    /// transaction.
    fn insert_segments(
        &self,
        job_id: Uuid,
        segments: Vec<NewTranscriptSegment>,
    ) -> Result<Vec<TranscriptSegment>, StoreError>;

    /// U9 — INSERT or UPDATE `meeting_summaries` + rev++ + log row.
    fn upsert_summary(&self, summary: NewMeetingSummary) -> Result<MeetingSummary, StoreError>;
}

// ---------------------------------------------------------------------------
// Compile-time seam enforcement
// ---------------------------------------------------------------------------

/// No-op `const fn` usable in `const` contexts to prove `T: Send + Sync`.
///
/// `rusqlite::Connection` is `!Sync`; any seam type that embedded one (directly
/// or behind `Rc`/`RefCell`) would fail these assertions at compile time.
/// This is the mechanical half of contract §7.2 point 1 ("accidental SQL in a
/// command fails to compile for lack of types, not merely review") applied to
/// the seam's own types.
const fn assert_send_sync<T: Send + Sync + ?Sized>() {}

// Checked on every build of this module — not just under `cargo test`.
const _: () = {
    // The trait itself is dyn-compatible (object-safe) and `Send + Sync`.
    let _ = assert_send_sync::<dyn MeetingRepository>;
    // Every type crossing the seam in a signature:
    let _ = assert_send_sync::<Uuid>;
    let _ = assert_send_sync::<ChangeEnvelope>;
    let _ = assert_send_sync::<StoreError>;
    let _ = assert_send_sync::<Page>;
    let _ = assert_send_sync::<PageOf<MeetingSession>>;
    let _ = assert_send_sync::<WorkerId>;
    let _ = assert_send_sync::<ChunkState>;
    let _ = assert_send_sync::<SttJobState>;
    let _ = assert_send_sync::<MeetingSession>;
    let _ = assert_send_sync::<AudioChunk>;
    let _ = assert_send_sync::<SttJob>;
    let _ = assert_send_sync::<TranscriptSegment>;
    let _ = assert_send_sync::<MeetingSummary>;
    let _ = assert_send_sync::<NewMeetingSession>;
    let _ = assert_send_sync::<SessionPatch>;
    let _ = assert_send_sync::<NewAudioChunk>;
    let _ = assert_send_sync::<ChunkFacts>;
    let _ = assert_send_sync::<NewTranscriptSegment>;
    let _ = assert_send_sync::<NewMeetingSummary>;
    let _ = assert_send_sync::<SttJobClaim>;
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Domain-service stand-in proving the seam is implementable **without
    /// any database**: a fake needs no connection, no path, no SQL (contract
    /// §2.1 motivation).
    struct FakeMeetingRepository;

    #[allow(unused_variables)]
    impl MeetingRepository for FakeMeetingRepository {
        fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError> {
            unimplemented!("fake")
        }
        fn list_sessions(&self, page: Page) -> Result<PageOf<MeetingSession>, StoreError> {
            unimplemented!("fake")
        }
        fn segments_for_meeting(
            &self,
            meeting_id: Uuid,
        ) -> Result<Vec<TranscriptSegment>, StoreError> {
            unimplemented!("fake")
        }
        fn summary_for_meeting(
            &self,
            meeting_id: Uuid,
        ) -> Result<Option<MeetingSummary>, StoreError> {
            unimplemented!("fake")
        }
        fn due_stt_jobs(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<SttJob>, StoreError> {
            unimplemented!("fake")
        }
        fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError> {
            unimplemented!("fake")
        }
        fn update_session(
            &self,
            id: Uuid,
            patch: SessionPatch,
        ) -> Result<MeetingSession, StoreError> {
            unimplemented!("fake")
        }
        fn soft_delete_session(&self, id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn append_chunk(&self, chunk: NewAudioChunk) -> Result<AudioChunk, StoreError> {
            unimplemented!("fake")
        }
        fn finalize_chunk(&self, id: Uuid, facts: ChunkFacts) -> Result<AudioChunk, StoreError> {
            unimplemented!("fake")
        }
        fn mark_chunk(
            &self,
            id: Uuid,
            state: ChunkState,
            reason: Option<String>,
        ) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn claim_due_stt_job(
            &self,
            worker: WorkerId,
            now: DateTime<Utc>,
        ) -> Result<Option<SttJobClaim>, StoreError> {
            unimplemented!("fake")
        }
        fn transition_stt_job(
            &self,
            id: Uuid,
            to: SttJobState,
            err: Option<String>,
        ) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn insert_segments(
            &self,
            job_id: Uuid,
            segments: Vec<NewTranscriptSegment>,
        ) -> Result<Vec<TranscriptSegment>, StoreError> {
            unimplemented!("fake")
        }
        fn upsert_summary(&self, summary: NewMeetingSummary) -> Result<MeetingSummary, StoreError> {
            unimplemented!("fake")
        }
    }

    /// The trait is object-safe: it can be used as `Arc<dyn MeetingRepository>`,
    /// exactly the shape command handlers and services will take (contract
    /// §2.1 rule).
    #[test]
    fn seam_is_object_safe_and_fake_substitutable_without_a_database() {
        let repo: Arc<dyn MeetingRepository> = Arc::new(FakeMeetingRepository);
        // Same handle flows through a clone, as `State<Arc<dyn …>>` would.
        let moved = Arc::clone(&repo);
        let _ = (repo, moved);
    }

    /// Boolean tombstone projection per §5.4.
    #[test]
    fn envelope_deleted_projection_follows_tombstone_marker() {
        let alive = ChangeEnvelope {
            id: Uuid("00000000-0000-4000-8000-000000000001".to_string()),
            revision: 1,
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let mut gone = alive.clone();
        gone.deleted_at = Some(Utc::now());
        gone.revision = 2;
        assert!(!alive.deleted());
        assert!(gone.deleted());
    }
}
