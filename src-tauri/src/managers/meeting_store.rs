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

    /// Read: highest segment `sequence` of a meeting (`-1` when none), so the
    /// queue can allocate the next slot without materializing every row.
    fn max_segment_sequence(&self, meeting_id: Uuid) -> Result<i64, StoreError>;

    /// Controlled deletion (plan §11): tombstones the chunk rows of a meeting
    /// (raw recording metadata) while keeping transcript and summary.
    fn delete_meeting_audio_rows(&self, meeting_id: Uuid) -> Result<(), StoreError>;

    /// Controlled deletion: tombstones chunk, transcript and summary rows of
    /// a meeting. The session row itself is soft-deleted separately.
    fn purge_meeting_content(&self, meeting_id: Uuid) -> Result<(), StoreError>;

    // -- projects (Phase 8) -------------------------------------------------

    fn create_project(&self, draft: NewProject) -> Result<Project, StoreError>;
    fn list_projects(&self) -> Result<Vec<Project>, StoreError>;
    fn rename_project(&self, id: Uuid, name: String) -> Result<Project, StoreError>;
    fn soft_delete_project(&self, id: Uuid) -> Result<(), StoreError>;
    /// Idempotent: re-assigning an already-linked pair succeeds silently.
    fn assign_meeting_to_project(
        &self,
        project_id: Uuid,
        meeting_id: Uuid,
    ) -> Result<(), StoreError>;
    fn unassign_meeting_from_project(
        &self,
        project_id: Uuid,
        meeting_id: Uuid,
    ) -> Result<(), StoreError>;
    fn list_meeting_ids_for_project(&self, project_id: Uuid) -> Result<Vec<String>, StoreError>;
    /// The "Eingang": alive sessions with no project link.
    fn list_unassigned_meeting_ids(&self) -> Result<Vec<String>, StoreError>;

    /// Removes all project links of one meeting — used when the meeting
    /// itself is fully deleted, so projects keep no dangling members.
    fn delete_project_links_for_meeting(&self, meeting_id: Uuid) -> Result<(), StoreError>;

    // -- sources (Phase 9) ---------------------------------------------------

    /// Idempotent import: a repeated `content_hash` returns the existing
    /// source without duplicating anything (plan §13 Phase 9 acceptance).
    fn create_chat_source(&self, draft: NewChatSource) -> Result<(Source, bool), StoreError>;
    fn list_sources(&self) -> Result<Vec<Source>, StoreError>;
    fn list_source_items(&self, source_id: Uuid) -> Result<Vec<SourceItem>, StoreError>;
    /// Records extracted media. Insert-only and idempotent per content hash,
    /// so a repeated import of the same archive adds nothing.
    fn attach_source_assets(
        &self,
        source_id: Uuid,
        assets: Vec<NewSourceAsset>,
    ) -> Result<Vec<SourceAsset>, StoreError>;
    fn list_source_assets(&self, source_id: Uuid) -> Result<Vec<SourceAsset>, StoreError>;

    // -- project tasks & snapshots (Phase 10) --------------------------------

    /// Manual task creation (origin `manual`, state `accepted`).
    fn create_manual_task(&self, draft: NewProjectTask) -> Result<ProjectTask, StoreError>;
    /// AI suggestion: insert-only and idempotent on the content hash, so a
    /// human-edited or rejected twin is never overwritten or duplicated.
    fn suggest_task(&self, draft: NewProjectTask) -> Result<(ProjectTask, bool), StoreError>;
    fn list_tasks(&self, project_id: Uuid) -> Result<Vec<ProjectTask>, StoreError>;
    /// Applies a user decision (`accepted | edited | rejected`) plus optional
    /// edited title/description; stamps human_edited_at.
    fn decide_task(
        &self,
        id: Uuid,
        state: &str,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<ProjectTask, StoreError>;
    fn create_snapshot(
        &self,
        project_id: Uuid,
        summary_markdown: String,
        prompt_version: String,
        model: Option<String>,
        source_marker: String,
    ) -> Result<ProjectSnapshot, StoreError>;
    fn list_snapshots(&self, project_id: Uuid) -> Result<Vec<ProjectSnapshot>, StoreError>;

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

    /// U4 policy hook — move the *currently claimed* job back to `retry_wait`
    /// with an explicit backoff deadline. The queue owns retry scheduling
    /// (stt-provider-contract §3); the store only persists its decision.
    fn reschedule_stt_job(
        &self,
        id: Uuid,
        next_attempt_at: DateTime<Utc>,
        err: Option<String>,
    ) -> Result<(), StoreError>;

    /// Read — chunks referenced by one job, in consumption order
    /// (`meeting_stt_job_chunks.position`). Input side of the STT queue.
    fn chunks_for_job(&self, job_id: Uuid) -> Result<Vec<AudioChunk>, StoreError>;

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

// ---------------------------------------------------------------------------
// Concrete SQLite store — schema, migrations and shared database access
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use super::change_log::{self, ChangeLogEntry, ChangeOperation};

/// File name of the meeting database inside the portable-aware app-data
/// directory. Meetings deliberately do not share `history.db` with the
/// dictation path (plan §9 lines 1145–1147: "Meetings erhalten eigene
/// Migrationen und Tabellen"; contract §0.1 item 3).
pub const MEETING_DB_FILE: &str = "meetings.db";

/// Table names, kept as constants because they double as `change_log.entity`
/// values addressed by later sync clients (contract §5.1 last bullet).
const ENTITY_SESSIONS: &str = "meeting_sessions";
const ENTITY_CHUNKS: &str = "meeting_audio_chunks";
const ENTITY_SEGMENTS: &str = "meeting_transcript_segments";
const ENTITY_SUMMARIES: &str = "meeting_summaries";

/// The one migration chain for `meetings.db`.
///
/// **Append-only.** `rusqlite_migration` tracks progress in the SQLite
/// `user_version` pragma (schema-versioning-policy §7 row DB, §5 "SQLite
/// note"), so every existing installation has already applied a prefix of this
/// slice. Editing or reordering an existing entry would silently skip or
/// re-apply DDL on those installations; new schema always arrives as a new
/// trailing `M::up`.
///
/// Step 1 is the wave-1 `meeting_sessions` table verbatim as it originally
/// shipped from `MeetingManager` — bit-identical so that databases already at
/// `user_version = 1` are recognized as up to date for that step and their
/// rows are never touched. Steps 2–6 are additive `CREATE TABLE`/`CREATE INDEX`
/// statements only: no `ALTER`, no `DROP`, no `UPDATE`, therefore existing
/// session rows survive the upgrade intact (test
/// `legacy_session_row_survives_new_migrations`).
///
/// Timestamps are Unix epoch milliseconds (`INTEGER`) throughout, matching the
/// wave-1 table and `transcription_history`; the seam converts to
/// `DateTime<Utc>` at the boundary (contract §5.3).
static MIGRATIONS: &[M] = &[
    // -- 1 -----------------------------------------------------------------
    // Row 1, `meeting_sessions` (plan §9 lines 1164–1178). Unchanged.
    M::up(
        "CREATE TABLE IF NOT EXISTS meeting_sessions (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'draft',
        stt_provider TEXT NOT NULL DEFAULT 'local',
        provider_config_snapshot TEXT NOT NULL DEFAULT '{}',
        meeting_model_id TEXT,
        language TEXT,
        microphone_device TEXT,
        output_device TEXT,
        has_microphone_audio INTEGER NOT NULL DEFAULT 0,
        has_system_audio INTEGER NOT NULL DEFAULT 0,
        recording_retention_policy TEXT NOT NULL DEFAULT 'default',
        summary_provider TEXT,
        summary_state TEXT NOT NULL DEFAULT 'none',
        last_error TEXT,
        started_at INTEGER NOT NULL,
        ended_at INTEGER,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );",
    ),
    // -- 2 -----------------------------------------------------------------
    // Row 2, `meeting_audio_chunks` (plan §9 lines 1180–1188; tier A).
    //
    // `path` from the plan is persisted as `storage_key`: a *relative*
    // AssetStore key, never an absolute path (plan lines 1158–1159, contract
    // note N3). `state_reason` is a diagnostics-only addition holding the
    // `reason` argument of U7 (`mark_chunk`); the plan's bullet list fixes the
    // required columns, not a maximal set, and the reason has nowhere else to
    // live. It never carries secrets (plan line 1334).
    M::up(
        "CREATE TABLE IF NOT EXISTS meeting_audio_chunks (
        id TEXT PRIMARY KEY,
        meeting_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
        sequence INTEGER NOT NULL,
        source TEXT NOT NULL,
        timeline_start_ms INTEGER NOT NULL,
        timeline_end_ms INTEGER NOT NULL,
        storage_key TEXT NOT NULL,
        sample_rate INTEGER NOT NULL,
        sample_count INTEGER NOT NULL DEFAULT 0,
        file_size INTEGER NOT NULL DEFAULT 0,
        checksum TEXT,
        state TEXT NOT NULL DEFAULT 'writing',
        state_reason TEXT,
        device_id TEXT,
        gap_drift_metadata TEXT,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER,
        UNIQUE (meeting_id, source, sequence)
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_audio_chunks_timeline
        ON meeting_audio_chunks (meeting_id, timeline_start_ms, sequence);",
    ),
    // -- 3 -----------------------------------------------------------------
    // Row 3, `meeting_stt_jobs` (plan §9 lines 1190–1199; tier B — no
    // envelope, no change_log, W6). The plan's "referenzierte Chunk-IDs" is a
    // set, so it becomes a link table rather than a JSON blob (plan line 1157:
    // relationships live in foreign keys, "statt nur in JSON versteckter
    // Beziehungen"). `claimed_by`/`claimed_at` carry the U4 worker lease.
    M::up(
        "CREATE TABLE IF NOT EXISTS meeting_stt_jobs (
        id TEXT PRIMARY KEY,
        meeting_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
        source TEXT NOT NULL,
        timeline_start_ms INTEGER NOT NULL,
        timeline_end_ms INTEGER NOT NULL,
        provider TEXT NOT NULL,
        config_snapshot TEXT NOT NULL DEFAULT '{}',
        state TEXT NOT NULL DEFAULT 'queued',
        attempt_count INTEGER NOT NULL DEFAULT 0,
        next_attempt_at INTEGER,
        claimed_by TEXT,
        claimed_at INTEGER,
        last_error TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS meeting_stt_job_chunks (
        job_id TEXT NOT NULL REFERENCES meeting_stt_jobs(id) ON DELETE CASCADE,
        chunk_id TEXT NOT NULL REFERENCES meeting_audio_chunks(id) ON DELETE RESTRICT,
        position INTEGER NOT NULL,
        PRIMARY KEY (job_id, chunk_id)
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_stt_jobs_due
        ON meeting_stt_jobs (state, next_attempt_at, created_at);",
    ),
    // -- 4 -----------------------------------------------------------------
    // Row 4, `meeting_transcript_segments` (plan §9 lines 1201–1208; tier A
    // per ownership matrix row 4, hence the full envelope).
    //
    // `job_id` uses ON DELETE RESTRICT, not CASCADE: STT jobs are tier B and
    // may legitimately be hard-deleted once finished (contract §5.4 last
    // bullet), while segments are tier A and must never disappear without a
    // tombstone (W3). RESTRICT turns "delete a job that still has segments"
    // into a loud constraint error instead of silent tier-A data loss.
    M::up(
        "CREATE TABLE IF NOT EXISTS meeting_transcript_segments (
        id TEXT PRIMARY KEY,
        meeting_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
        job_id TEXT NOT NULL REFERENCES meeting_stt_jobs(id) ON DELETE RESTRICT,
        source TEXT NOT NULL,
        start_ms INTEGER NOT NULL,
        end_ms INTEGER NOT NULL,
        text TEXT NOT NULL,
        word_timestamps TEXT,
        confidence REAL,
        provider TEXT NOT NULL,
        model_id TEXT,
        sequence INTEGER NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_timeline
        ON meeting_transcript_segments (meeting_id, start_ms, sequence);",
    ),
    // -- 5 -----------------------------------------------------------------
    // Row 5, `meeting_summaries` (plan §9 lines 1210–1217; tier A, U9).
    // One current summary per meeting — `upsert_summary` (U9) decides
    // INSERT vs. UPDATE from the meeting id, so the uniqueness is a schema
    // constraint rather than a convention.
    M::up(
        "CREATE TABLE IF NOT EXISTS meeting_summaries (
        id TEXT PRIMARY KEY,
        meeting_id TEXT NOT NULL UNIQUE REFERENCES meeting_sessions(id) ON DELETE CASCADE,
        provider TEXT NOT NULL,
        model TEXT,
        prompt_version TEXT NOT NULL,
        state TEXT NOT NULL,
        summary_markdown TEXT NOT NULL DEFAULT '',
        decisions TEXT NOT NULL DEFAULT '[]',
        action_items TEXT NOT NULL DEFAULT '[]',
        open_questions TEXT NOT NULL DEFAULT '[]',
        last_error TEXT,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );",
    ),
    // -- 6 -----------------------------------------------------------------
    // Row 16, `change_log` (plan §9 lines 1320–1325; tier C, append-only).
    // Written exclusively by `managers/change_log.rs` inside the same
    // transaction as its tier-A mutation (W2/W3). Local ordering is the
    // autoincrement `seq`, not the timestamp (contract note N5).
    M::up(
        "CREATE TABLE IF NOT EXISTS change_log (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        entity TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        revision INTEGER NOT NULL,
        operation TEXT NOT NULL,
        occurred_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_change_log_entity
        ON change_log (entity, entity_id, seq);",
    ),
    // -- 7 -----------------------------------------------------------------
    // Phase 8: `projects` plus the many-to-many `project_meeting_links`
    // (plan §6.16–§6.18). The UNIQUE on meeting_id would make it one-of-many;
    // deliberately omitted so a meeting may live in several projects, while
    // the composite PK keeps duplicate links impossible.
    M::up(
        "CREATE TABLE IF NOT EXISTS projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS project_meeting_links (
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        meeting_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (project_id, meeting_id)
    );
    CREATE INDEX IF NOT EXISTS idx_project_meeting_links_meeting
        ON project_meeting_links (meeting_id);",
    ),
    // -- 8 -----------------------------------------------------------------
    // Phase 9: generic `sources` plus normalised `source_items` (plan §9).
    // `sources.content_hash` UNIQUE makes a repeated/overlapping export an
    // idempotent no-op (acceptance: "derselbe Export erzeugt keine Dubletten");
    // items dedupe per source on their own content hash.
    M::up(
        "CREATE TABLE IF NOT EXISTS sources (
        id TEXT PRIMARY KEY,
        source_type TEXT NOT NULL,
        title TEXT NOT NULL,
        ingestion_state TEXT NOT NULL DEFAULT 'imported',
        parser_version TEXT NOT NULL DEFAULT '',
        content_hash TEXT NOT NULL UNIQUE,
        raw_asset_id TEXT,
        occurred_at INTEGER,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS source_items (
        id TEXT PRIMARY KEY,
        source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
        item_type TEXT NOT NULL DEFAULT 'chat_message',
        external_key TEXT,
        author_label TEXT,
        occurred_at INTEGER,
        sequence INTEGER NOT NULL,
        text TEXT NOT NULL,
        is_media INTEGER NOT NULL DEFAULT 0,
        content_hash TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_source_items_dedupe
        ON source_items (source_id, content_hash);
    CREATE INDEX IF NOT EXISTS idx_source_items_source
        ON source_items (source_id, sequence);",
    ),
    // -- 9 -----------------------------------------------------------------
    // Phase 10: `project_snapshots` (versioned analysis states) and
    // `project_tasks` with the suggestion workflow. The UNIQUE
    // (project_id, content_hash) on tasks makes repeated AI runs idempotent;
    // suggestions are insert-only so a human edit can never be overwritten.
    M::up(
        "CREATE TABLE IF NOT EXISTS project_snapshots (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        summary_markdown TEXT NOT NULL DEFAULT '',
        prompt_version TEXT NOT NULL DEFAULT '',
        model TEXT,
        source_marker TEXT NOT NULL DEFAULT '',
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS project_tasks (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        state TEXT NOT NULL DEFAULT 'suggested',
        origin TEXT NOT NULL DEFAULT 'ai',
        content_hash TEXT NOT NULL,
        human_edited_at INTEGER,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_project_tasks_dedupe
        ON project_tasks (project_id, content_hash);",
    ),
    // -- 10 ----------------------------------------------------------------
    // One audio chunk is transcribed by exactly one STT job: retries reuse the
    // existing row (`retry_jobs` only flips `failed` back to `queued`), so a
    // second job for the same chunk can only come from the check-then-act race
    // between the recorder's per-chunk enqueue and the stop path's sweep. That
    // race was observed in the field — three chunks each carried two jobs
    // created in the same millisecond, and every one of their segments landed
    // in the transcript twice. The unique index turns the race into a
    // constraint violation that `enqueue_stt_job` absorbs.
    //
    // Existing databases may already contain such duplicates, so the older
    // job of each pair (and its segments) is dropped before the index is
    // built; the surviving job keeps the identical text.
    M::up(
        "DELETE FROM meeting_transcript_segments
         WHERE job_id IN (
            SELECT jc.job_id FROM meeting_stt_job_chunks jc
            WHERE jc.job_id NOT IN (
                SELECT MIN(inner_jc.job_id) FROM meeting_stt_job_chunks inner_jc
                GROUP BY inner_jc.chunk_id
            )
         );
    DELETE FROM meeting_stt_job_chunks
     WHERE job_id NOT IN (
        SELECT MIN(inner_jc.job_id) FROM meeting_stt_job_chunks inner_jc
        GROUP BY inner_jc.chunk_id
     );
    DELETE FROM meeting_stt_jobs
     WHERE id NOT IN (SELECT job_id FROM meeting_stt_job_chunks);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_meeting_stt_job_chunks_unique
        ON meeting_stt_job_chunks (chunk_id);",
    ),
    // -- 11 ----------------------------------------------------------------
    // Phase 9: media extracted from a WhatsApp archive. Bytes live in the
    // AssetStore, this table only holds metadata and the *relative* storage
    // key (plan §9: "keine Binärdaten als große SQLite-BLOBs", "keine
    // absoluten Dateipfade als dauerhafte Identität").
    //
    // The unique (source_id, content_hash) makes re-importing an overlapping
    // export idempotent for media exactly as the item hash does for messages,
    // so the same photo cannot pile up on repeated imports.
    M::up(
        "CREATE TABLE IF NOT EXISTS source_assets (
        id TEXT PRIMARY KEY,
        source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
        storage_key TEXT NOT NULL,
        original_name TEXT NOT NULL,
        mime TEXT NOT NULL DEFAULT '',
        size_bytes INTEGER NOT NULL DEFAULT 0,
        content_hash TEXT NOT NULL,
        analysis_state TEXT NOT NULL DEFAULT 'not_analysed',
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_source_assets_dedupe
        ON source_assets (source_id, content_hash);
    CREATE INDEX IF NOT EXISTS idx_source_assets_source
        ON source_assets (source_id, original_name);",
    ),
];

/// Resolves the portable-aware path of `meetings.db`.
///
/// Single source of truth for both [`SqliteMeetingStore`] and
/// `MeetingManager`: the two must open the *same* file, or they would run the
/// one migration chain against two databases.
pub fn meeting_db_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, tauri::Error> {
    Ok(crate::portable::app_data_dir(app_handle)?.join(MEETING_DB_FILE))
}

/// Opens `meetings.db` at `db_path`, applies [`MIGRATIONS`] and returns the
/// ready connection.
///
/// Shared by every meeting-domain component so that connection pragmas and the
/// migration chain cannot drift apart. WAL keeps readers non-blocking during
/// capture writes; `foreign_keys = ON` is per-connection in SQLite and must be
/// set on *every* connection, which is exactly why this lives here.
pub fn open_meeting_database(db_path: &Path) -> Result<Connection, StoreError> {
    let op = "open_meeting_database";
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StoreError::Sql {
            operation: op,
            message: format!("cannot create {}: {e}", parent.display()),
        })?;
    }

    let mut conn = Connection::open(db_path).map_err(sql_err(op))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sql_err(op))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(sql_err(op))?;

    let migrations = Migrations::new(MIGRATIONS.to_vec());
    #[cfg(debug_assertions)]
    migrations.validate().map_err(|e| StoreError::Sql {
        operation: op,
        message: format!("invalid meeting migrations: {e}"),
    })?;
    migrations
        .to_latest(&mut conn)
        .map_err(|e| StoreError::Sql {
            operation: op,
            message: format!("migration failed: {e}"),
        })?;

    Ok(conn)
}

/// Maps a `rusqlite::Error` onto [`StoreError::Sql`] for one named operation.
///
/// The rusqlite type is flattened to its display text on purpose: no SQL type
/// may appear above the seam (contract §7.2 point 1, gate G1).
fn sql_err(operation: &'static str) -> impl Fn(rusqlite::Error) -> StoreError {
    move |e| StoreError::Sql {
        operation,
        message: e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Default ports (Clock / UuidSource)
// ---------------------------------------------------------------------------

/// Wall-clock implementation of [`Clock`], truncated to millisecond precision
/// by the store's persistence format (contract §5.3).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Production [`UuidSource`]: random UUIDv4, lowercase hyphenated, never
/// derived from a SQLite rowid (contract §5.1, plan §9 line 1151).
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomUuidSource;

impl UuidSource for RandomUuidSource {
    fn new_uuid(&self) -> Uuid {
        Uuid(uuid::Uuid::new_v4().to_string())
    }
}

// ---------------------------------------------------------------------------
// Value conversions between the seam types and the SQLite column formats
// ---------------------------------------------------------------------------

/// Unix epoch milliseconds, the timestamp format of every column in
/// `meetings.db` (matches the wave-1 tables and `transcription_history`).
fn to_ms(value: DateTime<Utc>) -> i64 {
    value.timestamp_millis()
}

/// Inverse of [`to_ms`]. A value outside chrono's representable range cannot
/// be produced by [`to_ms`], so it can only come from foreign writes; falling
/// back to "now" keeps the read total instead of panicking below the seam.
fn from_ms(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
}

fn json_text(value: &serde_json::Value) -> String {
    value.to_string()
}

/// Parses a JSON column. A malformed value is a read error rather than a
/// silent `null`: guessing would hide provider-snapshot corruption.
fn json_column(index: usize, raw: &str) -> rusqlite::Result<serde_json::Value> {
    serde_json::from_str(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Persisted token for a chunk state (plan §9 line 1187; snake_case, resolving
/// identifier-registry finding F-03 for this store's rows).
fn chunk_state_token(state: ChunkState) -> &'static str {
    match state {
        ChunkState::Writing => "writing",
        ChunkState::Ready => "ready",
        ChunkState::Damaged => "damaged",
        ChunkState::Deleted => "deleted",
    }
}

/// Inverse of [`chunk_state_token`]. An unknown value is classified as
/// [`ChunkState::Damaged`] — the fail-safe mandated by
/// schema-versioning-policy §4 rule R7 / UFR-2, so a future-major state can
/// never be silently consumed as audio.
fn chunk_state_from_token(raw: &str) -> ChunkState {
    match raw {
        "writing" => ChunkState::Writing,
        "ready" => ChunkState::Ready,
        "deleted" => ChunkState::Deleted,
        _ => ChunkState::Damaged,
    }
}

/// Persisted token for an STT job state (plan §9 line 1197).
fn job_state_token(state: SttJobState) -> &'static str {
    match state {
        SttJobState::Queued => "queued",
        SttJobState::Running => "running",
        SttJobState::RetryWait => "retry_wait",
        SttJobState::Succeeded => "succeeded",
        SttJobState::Failed => "failed",
        SttJobState::Cancelled => "cancelled",
    }
}

/// Inverse of [`job_state_token`]. Unlike chunk states there is no safe
/// fallback for a work item, so an unknown token is rejected (UFR-2).
fn job_state_from_token(index: usize, raw: &str) -> rusqlite::Result<SttJobState> {
    match raw {
        "queued" => Ok(SttJobState::Queued),
        "running" => Ok(SttJobState::Running),
        "retry_wait" => Ok(SttJobState::RetryWait),
        "succeeded" => Ok(SttJobState::Succeeded),
        "failed" => Ok(SttJobState::Failed),
        "cancelled" => Ok(SttJobState::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("unknown meeting_stt_jobs.state value {other:?}").into(),
        )),
    }
}

/// Terminal job states: they accept no further transition (plan §9 line 1197).
fn job_state_is_terminal(state: SttJobState) -> bool {
    matches!(
        state,
        SttJobState::Succeeded | SttJobState::Failed | SttJobState::Cancelled
    )
}

// ---------------------------------------------------------------------------
// Column lists and row mappers
// ---------------------------------------------------------------------------

const SESSION_COLUMNS: &str = "id, revision, updated_at, deleted_at, title, started_at, ended_at, \
     state, stt_provider, provider_config_snapshot, meeting_model_id, language, \
     microphone_device, output_device, has_microphone_audio, has_system_audio, \
     recording_retention_policy, summary_provider, summary_state, last_error, created_at";

const CHUNK_COLUMNS: &str = "id, revision, updated_at, deleted_at, meeting_id, sequence, source, \
     timeline_start_ms, timeline_end_ms, storage_key, sample_rate, sample_count, file_size, \
     checksum, state, device_id, gap_drift_metadata";

const JOB_COLUMNS: &str = "id, meeting_id, source, timeline_start_ms, timeline_end_ms, provider, \
     config_snapshot, state, attempt_count, next_attempt_at, last_error, created_at, updated_at";

const SEGMENT_COLUMNS: &str = "id, revision, updated_at, deleted_at, meeting_id, job_id, source, \
     start_ms, end_ms, text, word_timestamps, confidence, provider, model_id, sequence, created_at";

const SUMMARY_COLUMNS: &str = "id, revision, updated_at, deleted_at, meeting_id, provider, model, \
     prompt_version, state, summary_markdown, decisions, action_items, open_questions, last_error, \
     created_at";

/// Reads the four envelope columns, which every tier-A mapper selects first
/// and in the same order (contract §5).
fn envelope_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeEnvelope> {
    Ok(ChangeEnvelope {
        id: Uuid(row.get(0)?),
        revision: row.get(1)?,
        updated_at: from_ms(row.get(2)?),
        deleted_at: row.get::<_, Option<i64>>(3)?.map(from_ms),
    })
}

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSession> {
    let snapshot_raw: String = row.get(9)?;
    Ok(MeetingSession {
        envelope: envelope_from_row(row)?,
        title: row.get(4)?,
        started_at: from_ms(row.get(5)?),
        ended_at: row.get::<_, Option<i64>>(6)?.map(from_ms),
        state: row.get(7)?,
        stt_provider: row.get(8)?,
        provider_config_snapshot: json_column(9, &snapshot_raw)?,
        meeting_model_id: row.get(10)?,
        language: row.get(11)?,
        microphone_device: row.get(12)?,
        output_device: row.get(13)?,
        has_microphone_audio: row.get(14)?,
        has_system_audio: row.get(15)?,
        recording_retention_policy: row.get(16)?,
        summary_provider: row.get(17)?,
        summary_state: row.get(18)?,
        last_error: row.get(19)?,
        created_at: from_ms(row.get(20)?),
    })
}

fn chunk_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudioChunk> {
    let state_raw: String = row.get(14)?;
    let gap_raw: Option<String> = row.get(16)?;
    Ok(AudioChunk {
        envelope: envelope_from_row(row)?,
        meeting_id: Uuid(row.get(4)?),
        sequence: row.get(5)?,
        source: row.get(6)?,
        timeline_start_ms: row.get(7)?,
        timeline_end_ms: row.get(8)?,
        storage_key: row.get(9)?,
        sample_rate: row.get::<_, i64>(10)? as u32,
        sample_count: row.get(11)?,
        file_size: row.get(12)?,
        checksum: row.get(13)?,
        state: chunk_state_from_token(&state_raw),
        device_id: row.get(15)?,
        gap_drift_metadata: gap_raw.map(|raw| json_column(16, &raw)).transpose()?,
    })
}

/// Maps a job row **without** its chunk ids; those come from the
/// `meeting_stt_job_chunks` link table via [`load_job_chunk_ids`].
fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SttJob> {
    let config_raw: String = row.get(6)?;
    let state_raw: String = row.get(7)?;
    Ok(SttJob {
        id: Uuid(row.get(0)?),
        meeting_id: Uuid(row.get(1)?),
        chunk_ids: Vec::new(),
        source: row.get(2)?,
        timeline_start_ms: row.get(3)?,
        timeline_end_ms: row.get(4)?,
        provider: row.get(5)?,
        config_snapshot: json_column(6, &config_raw)?,
        state: job_state_from_token(7, &state_raw)?,
        attempt_count: row.get::<_, i64>(8)?.max(0) as u32,
        next_attempt_at: row.get::<_, Option<i64>>(9)?.map(from_ms),
        last_error: row.get(10)?,
        created_at: from_ms(row.get(11)?),
        updated_at: from_ms(row.get(12)?),
    })
}

fn segment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TranscriptSegment> {
    let words_raw: Option<String> = row.get(10)?;
    Ok(TranscriptSegment {
        envelope: envelope_from_row(row)?,
        meeting_id: Uuid(row.get(4)?),
        job_id: Uuid(row.get(5)?),
        source: row.get(6)?,
        start_ms: row.get(7)?,
        end_ms: row.get(8)?,
        text: row.get(9)?,
        word_timestamps: words_raw.map(|raw| json_column(10, &raw)).transpose()?,
        confidence: row.get::<_, Option<f64>>(11)?.map(|v| v as f32),
        provider: row.get(12)?,
        model_id: row.get(13)?,
        sequence: row.get(14)?,
        created_at: from_ms(row.get(15)?),
    })
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSummary> {
    let decisions_raw: String = row.get(10)?;
    let actions_raw: String = row.get(11)?;
    let questions_raw: String = row.get(12)?;
    Ok(MeetingSummary {
        envelope: envelope_from_row(row)?,
        meeting_id: Uuid(row.get(4)?),
        provider: row.get(5)?,
        model: row.get(6)?,
        prompt_version: row.get(7)?,
        state: row.get(8)?,
        summary_markdown: row.get(9)?,
        decisions: json_column(10, &decisions_raw)?,
        action_items: json_column(11, &actions_raw)?,
        open_questions: json_column(12, &questions_raw)?,
        last_error: row.get(13)?,
        created_at: from_ms(row.get(14)?),
        // The envelope's `updated_at` (column 2) is the row's single
        // last-write timestamp; the entity mirrors it for consumers that read
        // the summary without unpacking the envelope.
        updated_at: from_ms(row.get(2)?),
    })
}

/// Loads the transcript segments produced by one job, in sequence order.
fn load_job_segments(conn: &Connection, job_id: &str) -> rusqlite::Result<Vec<TranscriptSegment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SEGMENT_COLUMNS} FROM meeting_transcript_segments
         WHERE job_id = ?1 AND deleted_at IS NULL ORDER BY sequence ASC"
    ))?;
    let rows = stmt.query_map(params![job_id], segment_from_row)?;
    rows.collect()
}

/// Loads the ordered chunk ids referenced by one job (plan §9 line 1193).
fn load_job_chunk_ids(conn: &Connection, job_id: &str) -> rusqlite::Result<Vec<Uuid>> {
    let mut stmt = conn.prepare(
        "SELECT chunk_id FROM meeting_stt_job_chunks
         WHERE job_id = ?1 ORDER BY position ASC, chunk_id ASC",
    )?;
    let ids = stmt
        .query_map(params![job_id], |row| row.get::<_, String>(0).map(Uuid))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Single-row reads shared by the read side and by the write units that must
/// return the post-mutation state. Taking `&Connection` means a
/// `&Transaction` works too (it derefs), so writes read their own uncommitted
/// rows without a second code path.
fn read_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<MeetingSession>> {
    conn.query_row(
        &format!(
            "SELECT {SESSION_COLUMNS} FROM meeting_sessions WHERE id = ?1 AND deleted_at IS NULL"
        ),
        params![id],
        session_from_row,
    )
    .optional()
}

fn read_chunk(conn: &Connection, id: &str) -> rusqlite::Result<Option<AudioChunk>> {
    conn.query_row(
        &format!("SELECT {CHUNK_COLUMNS} FROM meeting_audio_chunks WHERE id = ?1"),
        params![id],
        chunk_from_row,
    )
    .optional()
}

fn read_job(conn: &Connection, id: &str) -> rusqlite::Result<Option<SttJob>> {
    let job = conn
        .query_row(
            &format!("SELECT {JOB_COLUMNS} FROM meeting_stt_jobs WHERE id = ?1"),
            params![id],
            job_from_row,
        )
        .optional()?;
    match job {
        Some(mut job) => {
            job.chunk_ids = load_job_chunk_ids(conn, id)?;
            Ok(Some(job))
        }
        None => Ok(None),
    }
}

fn read_summary_by_meeting(
    conn: &Connection,
    meeting_id: &str,
) -> rusqlite::Result<Option<MeetingSummary>> {
    conn.query_row(
        &format!(
            "SELECT {SUMMARY_COLUMNS} FROM meeting_summaries
             WHERE meeting_id = ?1 AND deleted_at IS NULL"
        ),
        params![meeting_id],
        summary_from_row,
    )
    .optional()
}

// ---------------------------------------------------------------------------
// SqliteMeetingStore — the sole production implementation of the seam
// ---------------------------------------------------------------------------

/// SQLite-backed [`MeetingRepository`] over `meetings.db`
/// (ownership-matrix rows 1–5, contract §3).
///
/// Concurrency: the connection sits behind a `Mutex` because
/// `rusqlite::Connection` is `!Sync`. That mutex is also what makes U4
/// (`claim_due_stt_job`) safe against two in-process workers — the select and
/// the claiming UPDATE run inside one `IMMEDIATE` transaction, so a second
/// caller either waits or observes the already-claimed state and moves on.
///
/// Every mutating method is one unit of work (contract §4 rule W1): it opens
/// exactly one transaction, appends its `change_log` row(s) inside that same
/// transaction for tier-A rows (W2/W3), bumps `revision` by one (W4) and
/// commits or rolls back before returning. Callers cannot inject a connection
/// and cannot compose partial writes.
///
/// Not wired into `lib.rs` yet — `MeetingManager` still serves the shipped
/// session journey through its own methods. Both open the same file through
/// [`open_meeting_database`], so the schema cannot diverge.
pub struct SqliteMeetingStore {
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
    uuids: Arc<dyn UuidSource>,
    db_path: PathBuf,
}

impl SqliteMeetingStore {
    /// Opens the store on the portable-aware `meetings.db` with production
    /// ports (system clock, random UUIDv4).
    pub fn new(app_handle: &tauri::AppHandle) -> Result<Self, StoreError> {
        let path = meeting_db_path(app_handle).map_err(|e| StoreError::Sql {
            operation: "SqliteMeetingStore::new",
            message: format!("cannot resolve app data dir: {e}"),
        })?;
        Self::open(path)
    }

    /// Opens the store at an explicit path with production ports.
    pub fn open(db_path: PathBuf) -> Result<Self, StoreError> {
        Self::open_with_ports(db_path, Arc::new(SystemClock), Arc::new(RandomUuidSource))
    }

    /// Opens the store with injected [`Clock`] and [`UuidSource`] ports, the
    /// shape tests use for deterministic timestamps and ids (contract §5.1,
    /// §5.3).
    pub fn open_with_ports(
        db_path: PathBuf,
        clock: Arc<dyn Clock>,
        uuids: Arc<dyn UuidSource>,
    ) -> Result<Self, StoreError> {
        let conn = open_meeting_database(&db_path)?;
        Ok(Self {
            conn: Mutex::new(conn),
            clock,
            uuids,
            db_path,
        })
    }

    /// Path of the database file this store is bound to (diagnostics only —
    /// no caller may open it themselves, contract §7.1 rule R1).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Locks the connection, mapping a poisoned mutex onto a typed error
    /// instead of propagating a panic across the seam.
    fn locked(&self, operation: &'static str) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Sql {
            operation,
            message: "meeting database mutex poisoned by an earlier panic".to_string(),
        })
    }

    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

/// Classifies a `rusqlite` failure that a caller can actually act on.
///
/// Constraint violations (unknown `meeting_id`, duplicate `(meeting, source,
/// sequence)`, a second summary for one meeting) are caller errors, so they
/// surface as [`StoreError::Conflict`] rather than as an opaque SQL string.
fn classify<'a>(
    entity: &'static str,
    id: &'a Uuid,
    operation: &'static str,
) -> impl Fn(rusqlite::Error) -> StoreError + 'a {
    move |e| match e {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Conflict {
                entity,
                id: id.clone(),
                reason: e.to_string(),
            }
        }
        other => StoreError::Sql {
            operation,
            message: other.to_string(),
        },
    }
}

/// Shared tombstone helper for the controlled-deletion methods (plan §11).
fn tombstone_rows_impl(
    store: &SqliteMeetingStore,
    op: &'static str,
    meeting_id: &str,
    entity: &'static str,
) -> Result<(), StoreError> {
    let conn = store.locked(op)?;
    conn.execute(
        &format!(
            "UPDATE {entity} SET deleted_at = ?2, revision = revision + 1,
             updated_at = ?2 WHERE meeting_id = ?1 AND deleted_at IS NULL"
        ),
        params![meeting_id, chrono::Utc::now().timestamp_millis()],
    )
    .map_err(classify(entity, &Uuid(meeting_id.to_string()), op))?;
    Ok(())
}

impl SqliteMeetingStore {
    fn tombstone_rows(
        &self,
        op: &'static str,
        meeting_id: &str,
        entity: &'static str,
    ) -> Result<(), StoreError> {
        tombstone_rows_impl(self, op, meeting_id, entity)
    }
}

/// Wire-facing project row (Phase 8). Kept flat so it serializes straight to
/// the frontend without an envelope projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct NewProject {
    pub name: String,
    pub description: String,
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

const PROJECT_COLUMNS: &str = "id, name, description, created_at, updated_at";

/// Wire-facing source row (Phase 9).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Source {
    pub id: String,
    pub source_type: String,
    pub title: String,
    pub ingestion_state: String,
    pub parser_version: String,
    pub content_hash: String,
    pub created_at: i64,
}

/// One media file extracted from an imported archive.
///
/// `analysis_state` is `not_analysed` until a model actually looked at it; the
/// plan requires unsupported media to stay available and be marked plainly as
/// "nicht analysiert" rather than being presented as understood.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct SourceAsset {
    pub id: String,
    pub source_id: String,
    pub storage_key: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub content_hash: String,
    pub analysis_state: String,
}

/// Draft row for [`MeetingRepository::attach_source_assets`].
#[derive(Debug, Clone)]
pub struct NewSourceAsset {
    pub storage_key: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub content_hash: String,
}

/// Wire-facing normalized chat/message item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct SourceItem {
    pub id: String,
    pub source_id: String,
    pub author_label: Option<String>,
    pub occurred_at: Option<i64>,
    pub sequence: i64,
    pub text: String,
    pub is_media: bool,
}

pub struct NewChatSource {
    /// Value for `sources.source_type`, supplied by the
    /// [`ChatImporter`](crate::managers::chat_import::ChatImporter) that read
    /// the export. Was a SQL literal until a second export format became
    /// possible.
    pub source_type: String,
    pub title: String,
    pub parser_version: String,
    pub content_hash: String,
    /// `(author_label, text, is_media)` triples in chat order.
    pub messages: Vec<(Option<String>, String, bool)>,
}

const SOURCE_COLUMNS: &str =
    "id, source_type, title, ingestion_state, parser_version, content_hash, created_at";

fn source_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        id: row.get(0)?,
        source_type: row.get(1)?,
        title: row.get(2)?,
        ingestion_state: row.get(3)?,
        parser_version: row.get(4)?,
        content_hash: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const TASK_COLUMNS: &str = "id, project_id, title, description, state, origin, human_edited_at";

/// Stable dedupe key for AI suggestions: same title+description text is the
/// same task regardless of which run produced it.
fn content_hash_of(title: &str, description: &str) -> String {
    let mut hasher = sha2::Sha256::default();
    hasher.update(title.trim().as_bytes());
    hasher.update([0x1f]);
    hasher.update(description.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Shared insert path for manual tasks and AI suggestions (plan §10).
fn insert_task_impl(
    store: &SqliteMeetingStore,
    draft: NewProjectTask,
    state: &str,
    content_hash: Option<String>,
) -> Result<ProjectTask, StoreError> {
    let op = "insert_task";
    let now = chrono::Utc::now().timestamp_millis();
    let id = uuid::Uuid::new_v4().to_string();
    let hash = content_hash.unwrap_or_else(|| content_hash_of(&draft.title, &draft.description));
    let conn = store.locked(op)?;
    conn.execute(
        "INSERT INTO project_tasks (id, project_id, title, description, state, origin,
         content_hash, human_edited_at, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
        params![
            id,
            draft.project_id.0,
            draft.title.trim(),
            draft.description.trim(),
            state,
            draft.origin,
            hash,
            if draft.origin == "manual" {
                Some(now)
            } else {
                None
            },
            now
        ],
    )
    .map_err(classify("project_tasks", &Uuid(id.clone()), op))?;
    conn.query_row(
        &format!("SELECT {TASK_COLUMNS} FROM project_tasks WHERE id = ?1"),
        params![id],
        task_from_row,
    )
    .map_err(sql_err(op))
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectTask> {
    Ok(ProjectTask {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        state: row.get(4)?,
        origin: row.get(5)?,
        human_edited_at: row.get(6)?,
    })
}

const SNAPSHOT_COLUMNS: &str =
    "id, project_id, summary_markdown, prompt_version, model, source_marker, created_at";

fn snapshot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectSnapshot> {
    Ok(ProjectSnapshot {
        id: row.get(0)?,
        project_id: row.get(1)?,
        summary_markdown: row.get(2)?,
        prompt_version: row.get(3)?,
        model: row.get(4)?,
        source_marker: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn read_source_by_hash(conn: &Connection, content_hash: &str) -> rusqlite::Result<Option<Source>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SOURCE_COLUMNS} FROM sources WHERE content_hash = ?1 AND deleted_at IS NULL"
    ))?;
    let mut rows = stmt.query_map(params![content_hash], source_from_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Wire-facing project task (Phase 10).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ProjectTask {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    /// `suggested | accepted | edited | rejected`
    pub state: String,
    /// `ai | manual` — who created it first.
    pub origin: String,
    pub human_edited_at: Option<i64>,
}

pub struct NewProjectTask {
    pub project_id: Uuid,
    pub title: String,
    pub description: String,
    pub origin: &'static str,
}

/// Wire-facing versioned snapshot (Phase 10).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ProjectSnapshot {
    pub id: String,
    pub project_id: String,
    pub summary_markdown: String,
    pub prompt_version: String,
    pub model: Option<String>,
    pub source_marker: String,
    pub created_at: i64,
}

impl MeetingRepository for SqliteMeetingStore {
    // -- Read side ---------------------------------------------------------

    fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError> {
        let op = "get_session";
        let conn = self.locked(op)?;
        read_session(&conn, &id.0).map_err(sql_err(op))
    }

    fn list_sessions(&self, page: Page) -> Result<PageOf<MeetingSession>, StoreError> {
        let op = "list_sessions";
        let conn = self.locked(op)?;

        // `total` counts alive rows only: tombstones are invisible to list
        // reads, and consumers learn about deletion through the envelope of a
        // row they already hold (contract §5.4), never through a count.
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meeting_sessions WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(sql_err(op))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM meeting_sessions
                 WHERE deleted_at IS NULL
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?1 OFFSET ?2"
            ))
            .map_err(sql_err(op))?;
        let items = stmt
            .query_map(
                params![page.limit as i64, page.offset as i64],
                session_from_row,
            )
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;

        Ok(PageOf {
            items,
            total: total.max(0) as u64,
            page,
        })
    }

    fn segments_for_meeting(&self, meeting_id: Uuid) -> Result<Vec<TranscriptSegment>, StoreError> {
        let op = "segments_for_meeting";
        let conn = self.locked(op)?;
        // Chronological along the audio timeline, not by insertion order:
        // jobs finish out of order, so `start_ms` is the ordering authority
        // and `sequence` only breaks ties inside one start offset.
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SEGMENT_COLUMNS} FROM meeting_transcript_segments
                 WHERE meeting_id = ?1 AND deleted_at IS NULL
                 ORDER BY start_ms ASC, sequence ASC, rowid ASC"
            ))
            .map_err(sql_err(op))?;
        let segments = stmt
            .query_map(params![meeting_id.0], segment_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(segments)
    }

    fn max_segment_sequence(&self, meeting_id: Uuid) -> Result<i64, StoreError> {
        let op = "max_segment_sequence";
        let conn = self.locked(op)?;
        conn.query_row(
            "SELECT COALESCE(MAX(sequence), -1) FROM meeting_transcript_segments
             WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![meeting_id.0],
            |row| row.get(0),
        )
        .map_err(sql_err(op))
    }

    fn delete_meeting_audio_rows(&self, meeting_id: Uuid) -> Result<(), StoreError> {
        self.tombstone_rows(
            "delete_meeting_audio_rows",
            &meeting_id.0,
            "meeting_audio_chunks",
        )
    }

    fn create_project(&self, draft: NewProject) -> Result<Project, StoreError> {
        let op = "create_project";
        let now = chrono::Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.locked(op)?;
        conn.execute(
            "INSERT INTO projects (id, name, description, created_at, updated_at) VALUES (?1,?2,?3,?4,?4)",
            params![id, draft.name.trim(), draft.description.trim(), now],
        )
        .map_err(classify("projects", &Uuid(id.clone()), op))?;
        Ok(Project {
            id,
            name: draft.name.trim().to_string(),
            description: draft.description.trim().to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let op = "list_projects";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM projects WHERE deleted_at IS NULL ORDER BY name ASC"
            ))
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map([], project_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn rename_project(&self, id: Uuid, name: String) -> Result<Project, StoreError> {
        let op = "rename_project";
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(StoreError::Conflict {
                entity: "projects",
                id: id.clone(),
                reason: "project name must not be empty".to_string(),
            });
        }
        let conn = self.locked(op)?;
        let updated = conn
            .execute(
                "UPDATE projects SET name = ?2, revision = revision + 1, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
                params![id.0, name, chrono::Utc::now().timestamp_millis()],
            )
            .map_err(classify("projects", &id, op))?;
        if updated == 0 {
            return Err(StoreError::NotFound {
                entity: "projects",
                id,
            });
        }
        conn.query_row(
            &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
            params![id.0],
            project_from_row,
        )
        .map_err(sql_err(op))
    }

    fn soft_delete_project(&self, id: Uuid) -> Result<(), StoreError> {
        let op = "soft_delete_project";
        let conn = self.locked(op)?;
        conn.execute(
            "UPDATE projects SET deleted_at = ?2, revision = revision + 1, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id.0, chrono::Utc::now().timestamp_millis()],
        )
        .map_err(classify("projects", &id, op))?;
        // Tombstoned projects release their meetings to the inbox immediately
        // (plan §11: unlinking never deletes the meeting).
        conn.execute(
            "DELETE FROM project_meeting_links WHERE project_id = ?1",
            params![id.0],
        )
        .map_err(sql_err(op))?;
        Ok(())
    }

    fn assign_meeting_to_project(
        &self,
        project_id: Uuid,
        meeting_id: Uuid,
    ) -> Result<(), StoreError> {
        let op = "assign_meeting_to_project";
        let conn = self.locked(op)?;
        conn.execute(
            "INSERT OR IGNORE INTO project_meeting_links (project_id, meeting_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                project_id.0,
                meeting_id.0,
                chrono::Utc::now().timestamp_millis()
            ],
        )
        .map_err(sql_err(op))?;
        Ok(())
    }

    fn unassign_meeting_from_project(
        &self,
        project_id: Uuid,
        meeting_id: Uuid,
    ) -> Result<(), StoreError> {
        let op = "unassign_meeting_from_project";
        let conn = self.locked(op)?;
        conn.execute(
            "DELETE FROM project_meeting_links WHERE project_id = ?1 AND meeting_id = ?2",
            params![project_id.0, meeting_id.0],
        )
        .map_err(sql_err(op))?;
        Ok(())
    }

    fn list_meeting_ids_for_project(&self, project_id: Uuid) -> Result<Vec<String>, StoreError> {
        let op = "list_meeting_ids_for_project";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare("SELECT meeting_id FROM project_meeting_links WHERE project_id = ?1")
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map(params![project_id.0], |row| row.get(0))
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn list_unassigned_meeting_ids(&self) -> Result<Vec<String>, StoreError> {
        let op = "list_unassigned_meeting_ids";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id FROM meeting_sessions s
                 WHERE s.deleted_at IS NULL AND s.state != 'draft'
                   AND NOT EXISTS (
                       SELECT 1 FROM project_meeting_links l WHERE l.meeting_id = s.id
                   )
                 ORDER BY s.updated_at DESC",
            )
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn delete_project_links_for_meeting(&self, meeting_id: Uuid) -> Result<(), StoreError> {
        let op = "delete_project_links_for_meeting";
        let conn = self.locked(op)?;
        conn.execute(
            "DELETE FROM project_meeting_links WHERE meeting_id = ?1",
            params![meeting_id.0],
        )
        .map_err(sql_err(op))?;
        Ok(())
    }

    fn create_chat_source(&self, draft: NewChatSource) -> Result<(Source, bool), StoreError> {
        let op = "create_chat_source";
        let now = chrono::Utc::now().timestamp_millis();

        // Dedupe first: an identical export must be a visible no-op, not a
        // second source row (plan §13 Phase 9 acceptance).
        {
            let conn = self.locked(op)?;
            if let Some(existing) =
                read_source_by_hash(&conn, &draft.content_hash).map_err(sql_err(op))?
            {
                return Ok((existing, false));
            }
        }

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO sources (id, source_type, title, ingestion_state, parser_version,
             content_hash, occurred_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'imported', ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                draft.source_type,
                draft.title.trim(),
                draft.parser_version,
                draft.content_hash,
                now,
                now
            ],
        )
        .map_err(classify("sources", &Uuid(id.clone()), op))?;

        for (sequence, (author, text, is_media)) in draft.messages.iter().enumerate() {
            let mut hasher = sha2::Sha256::default();
            hasher.update(text.as_bytes());
            let item_hash = format!("{:x}", hasher.finalize());
            tx.execute(
                "INSERT OR IGNORE INTO source_items (id, source_id, author_label, sequence,
                 text, is_media, content_hash, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    id,
                    author,
                    sequence as i64,
                    text,
                    *is_media,
                    item_hash,
                    now
                ],
            )
            .map_err(classify("source_items", &Uuid(id.clone()), op))?;
        }
        tx.commit().map_err(sql_err(op))?;
        drop(conn);

        {
            let conn = self.locked(op)?;
            let source = conn
                .query_row(
                    &format!("SELECT {SOURCE_COLUMNS} FROM sources WHERE id = ?1"),
                    params![id],
                    source_from_row,
                )
                .map_err(sql_err(op))?;
            Ok((source, true))
        }
    }

    fn create_manual_task(&self, draft: NewProjectTask) -> Result<ProjectTask, StoreError> {
        insert_task_impl(self, draft, "accepted", None)
    }

    fn suggest_task(&self, draft: NewProjectTask) -> Result<(ProjectTask, bool), StoreError> {
        let op = "suggest_task";
        let content_hash = content_hash_of(&draft.title, &draft.description);
        {
            let conn = self.locked(op)?;
            if let Some(existing) = conn
                .query_row(
                    &format!(
                        "SELECT {TASK_COLUMNS} FROM project_tasks
                         WHERE project_id = ?1 AND content_hash = ?2 AND deleted_at IS NULL"
                    ),
                    params![draft.project_id.0, content_hash],
                    task_from_row,
                )
                .optional()
                .map_err(sql_err(op))?
            {
                // Insert-only doctrine (WR-1..WR-4): a human-edited or
                // rejected twin is never touched by later AI runs.
                return Ok((existing, false));
            }
        }
        let task = insert_task_impl(self, draft, "suggested", Some(content_hash))?;
        Ok((task, true))
    }

    fn list_tasks(&self, project_id: Uuid) -> Result<Vec<ProjectTask>, StoreError> {
        let op = "list_tasks";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM project_tasks
                 WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY created_at ASC"
            ))
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map(params![project_id.0], task_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn decide_task(
        &self,
        id: Uuid,
        state: &str,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<ProjectTask, StoreError> {
        let op = "decide_task";
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.locked(op)?;
        let updated = conn
            .execute(
                "UPDATE project_tasks SET
                state = ?2,
                title = COALESCE(?3, title),
                description = COALESCE(?4, description),
                human_edited_at = ?5,
                revision = revision + 1,
                updated_at = ?5
             WHERE id = ?1 AND deleted_at IS NULL",
                params![id.0, state, title, description, now],
            )
            .map_err(classify("project_tasks", &id, op))?;
        if updated == 0 {
            return Err(StoreError::NotFound {
                entity: "project_tasks",
                id,
            });
        }
        conn.query_row(
            &format!("SELECT {TASK_COLUMNS} FROM project_tasks WHERE id = ?1"),
            params![id.0],
            task_from_row,
        )
        .map_err(sql_err(op))
    }

    fn create_snapshot(
        &self,
        project_id: Uuid,
        summary_markdown: String,
        prompt_version: String,
        model: Option<String>,
        source_marker: String,
    ) -> Result<ProjectSnapshot, StoreError> {
        let op = "create_snapshot";
        let now = chrono::Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.locked(op)?;
        conn.execute(
            "INSERT INTO project_snapshots (id, project_id, summary_markdown, prompt_version,
             model, source_marker, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
            params![
                id,
                project_id.0,
                summary_markdown,
                prompt_version,
                model,
                source_marker,
                now
            ],
        )
        .map_err(classify("project_snapshots", &Uuid(id.clone()), op))?;
        conn.query_row(
            &format!("SELECT {SNAPSHOT_COLUMNS} FROM project_snapshots WHERE id = ?1"),
            params![id],
            snapshot_from_row,
        )
        .map_err(sql_err(op))
    }

    fn list_snapshots(&self, project_id: Uuid) -> Result<Vec<ProjectSnapshot>, StoreError> {
        let op = "list_snapshots";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SNAPSHOT_COLUMNS} FROM project_snapshots
                 WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY created_at DESC"
            ))
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map(params![project_id.0], snapshot_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn list_sources(&self) -> Result<Vec<Source>, StoreError> {
        let op = "list_sources";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SOURCE_COLUMNS} FROM sources
                 WHERE deleted_at IS NULL ORDER BY created_at DESC"
            ))
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map([], source_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn list_source_items(&self, source_id: Uuid) -> Result<Vec<SourceItem>, StoreError> {
        let op = "list_source_items";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source_id, author_label, occurred_at, sequence, text, is_media
                 FROM source_items WHERE source_id = ?1 AND deleted_at IS NULL ORDER BY sequence ASC",
            )
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map(params![source_id.0], |row| {
                Ok(SourceItem {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    author_label: row.get(2)?,
                    occurred_at: row.get(3)?,
                    sequence: row.get(4)?,
                    text: row.get(5)?,
                    is_media: row.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn attach_source_assets(
        &self,
        source_id: Uuid,
        assets: Vec<NewSourceAsset>,
    ) -> Result<Vec<SourceAsset>, StoreError> {
        let op = "attach_source_assets";
        let now = to_ms(self.now());
        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;
        for asset in &assets {
            // `OR IGNORE` against the (source_id, content_hash) index: a
            // repeated import of the same archive must add nothing rather
            // than fail (plan §6.19 idempotent re-import).
            tx.execute(
                "INSERT OR IGNORE INTO source_assets
                   (id, source_id, storage_key, original_name, mime, size_bytes,
                    content_hash, analysis_state, revision, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'not_analysed', 1, ?8, ?8)",
                params![
                    self.uuids.new_uuid().0,
                    source_id.0,
                    asset.storage_key,
                    asset.original_name,
                    asset.mime,
                    asset.size_bytes,
                    asset.content_hash,
                    now,
                ],
            )
            .map_err(classify("source_assets", &source_id, op))?;
        }
        tx.commit().map_err(sql_err(op))?;
        drop(conn);
        self.list_source_assets(source_id)
    }

    fn list_source_assets(&self, source_id: Uuid) -> Result<Vec<SourceAsset>, StoreError> {
        let op = "list_source_assets";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source_id, storage_key, original_name, mime, size_bytes,
                        content_hash, analysis_state
                 FROM source_assets
                 WHERE source_id = ?1 AND deleted_at IS NULL
                 ORDER BY original_name ASC",
            )
            .map_err(sql_err(op))?;
        let rows = stmt
            .query_map(params![source_id.0], |row| {
                Ok(SourceAsset {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    storage_key: row.get(2)?,
                    original_name: row.get(3)?,
                    mime: row.get(4)?,
                    size_bytes: row.get(5)?,
                    content_hash: row.get(6)?,
                    analysis_state: row.get(7)?,
                })
            })
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        Ok(rows)
    }

    fn purge_meeting_content(&self, meeting_id: Uuid) -> Result<(), StoreError> {
        let op = "purge_meeting_content";
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.locked(op)?;
        for entity in [
            "meeting_audio_chunks",
            "meeting_transcript_segments",
            "meeting_summaries",
        ] {
            conn.execute(
                &format!(
                    "UPDATE {entity} SET deleted_at = ?2, revision = revision + 1,
                     updated_at = ?2 WHERE meeting_id = ?1 AND deleted_at IS NULL"
                ),
                params![meeting_id.0, now],
            )
            .map_err(classify(entity, &meeting_id, op))?;
        }
        Ok(())
    }

    fn summary_for_meeting(&self, meeting_id: Uuid) -> Result<Option<MeetingSummary>, StoreError> {
        let op = "summary_for_meeting";
        let conn = self.locked(op)?;
        read_summary_by_meeting(&conn, &meeting_id.0).map_err(sql_err(op))
    }

    fn due_stt_jobs(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<SttJob>, StoreError> {
        let op = "due_stt_jobs";
        let conn = self.locked(op)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM meeting_stt_jobs
                 WHERE state IN ('queued', 'retry_wait')
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
                 ORDER BY COALESCE(next_attempt_at, created_at) ASC, created_at ASC, rowid ASC
                 LIMIT ?2"
            ))
            .map_err(sql_err(op))?;
        let mut jobs = stmt
            .query_map(params![to_ms(now), limit as i64], job_from_row)
            .map_err(sql_err(op))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_err(op))?;
        for job in &mut jobs {
            job.chunk_ids = load_job_chunk_ids(&conn, &job.id.0).map_err(sql_err(op))?;
        }
        Ok(jobs)
    }

    // -- U1: create_session ------------------------------------------------

    fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError> {
        let op = "U1 create_session";
        let id = self.uuids.new_uuid();
        let now = self.now();
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;
        tx.execute(
            "INSERT INTO meeting_sessions (
                id, title, state, stt_provider, provider_config_snapshot, meeting_model_id,
                language, microphone_device, output_device, has_microphone_audio,
                has_system_audio, recording_retention_policy, summary_provider, summary_state,
                last_error, started_at, ended_at, revision, created_at, updated_at, deleted_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, 'none', NULL, ?14, NULL, 1, ?15, ?15, NULL
             )",
            params![
                id.0,
                draft.title,
                crate::managers::meeting::SESSION_STATE_DRAFT,
                draft.stt_provider,
                json_text(&draft.provider_config_snapshot),
                draft.meeting_model_id,
                draft.language,
                draft.microphone_device,
                draft.output_device,
                draft.has_microphone_audio,
                draft.has_system_audio,
                draft.recording_retention_policy,
                draft.summary_provider,
                to_ms(draft.started_at),
                now_ms,
            ],
        )
        .map_err(classify(ENTITY_SESSIONS, &id, op))?;

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_SESSIONS,
                entity_id: id.clone(),
                revision: 1,
                occurred_at: now,
                operation: ChangeOperation::Create,
            },
        )
        .map_err(sql_err(op))?;

        let created = read_session(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_SESSIONS,
                id: id.clone(),
            })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(created)
    }

    // -- U2: update_session ------------------------------------------------

    fn update_session(&self, id: Uuid, patch: SessionPatch) -> Result<MeetingSession, StoreError> {
        let op = "U2 update_session";
        let now = self.now();

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        // Read-modify-write inside the transaction rather than assembling a
        // dynamic SET clause: `None` means "leave unchanged" for every patch
        // field, and this keeps that rule in one place instead of in six
        // SQL fragments. `ended_at`/`last_error` therefore cannot be *cleared*
        // through the patch — clearing is not a modelled operation.
        let current = read_session(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_SESSIONS,
                id: id.clone(),
            })?;

        let title = patch.title.unwrap_or(current.title);
        let state = patch.state.unwrap_or(current.state);
        let ended_at = patch.ended_at.or(current.ended_at);
        let summary_state = patch.summary_state.unwrap_or(current.summary_state);
        let last_error = patch.last_error.or(current.last_error);
        let retention = patch
            .recording_retention_policy
            .unwrap_or(current.recording_retention_policy);
        let revision = current.envelope.revision + 1;

        tx.execute(
            "UPDATE meeting_sessions
             SET title = ?2, state = ?3, ended_at = ?4, summary_state = ?5, last_error = ?6,
                 recording_retention_policy = ?7, revision = ?8, updated_at = ?9
             WHERE id = ?1 AND deleted_at IS NULL",
            params![
                id.0,
                title,
                state,
                ended_at.map(to_ms),
                summary_state,
                last_error,
                retention,
                revision,
                to_ms(now),
            ],
        )
        .map_err(classify(ENTITY_SESSIONS, &id, op))?;

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_SESSIONS,
                entity_id: id.clone(),
                revision,
                occurred_at: now,
                operation: ChangeOperation::Update,
            },
        )
        .map_err(sql_err(op))?;

        let updated = read_session(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_SESSIONS,
                id: id.clone(),
            })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(updated)
    }

    // -- U3: soft_delete_session -------------------------------------------

    fn soft_delete_session(&self, id: Uuid) -> Result<(), StoreError> {
        let op = "U3 soft_delete_session";
        let now = self.now();

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        let current = read_session(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_SESSIONS,
                id: id.clone(),
            })?;
        let revision = current.envelope.revision + 1;

        // Tombstone, never DELETE: hard deletion of tier-A rows does not exist
        // in v1 (contract §5.4). Chunks, segments and summaries of the meeting
        // keep their rows too — the FK cascades exist for integrity, but no
        // code path in this store ever triggers them.
        tx.execute(
            "UPDATE meeting_sessions
             SET deleted_at = ?2, revision = ?3, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id.0, to_ms(now), revision],
        )
        .map_err(classify(ENTITY_SESSIONS, &id, op))?;

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_SESSIONS,
                entity_id: id.clone(),
                revision,
                occurred_at: now,
                operation: ChangeOperation::Tombstone,
            },
        )
        .map_err(sql_err(op))?;

        tx.commit().map_err(sql_err(op))
    }

    // -- U5: append_chunk --------------------------------------------------

    fn append_chunk(&self, chunk: NewAudioChunk) -> Result<AudioChunk, StoreError> {
        let op = "U5 append_chunk";
        let id = self.uuids.new_uuid();
        let now = self.now();
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        tx.execute(
            "INSERT INTO meeting_audio_chunks (
                id, meeting_id, sequence, source, timeline_start_ms, timeline_end_ms,
                storage_key, sample_rate, sample_count, file_size, checksum, state,
                state_reason, device_id, gap_drift_metadata, revision, created_at,
                updated_at, deleted_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL, 'writing',
                NULL, ?10, NULL, 1, ?11, ?11, NULL
             )",
            params![
                id.0,
                chunk.meeting_id.0,
                chunk.sequence,
                chunk.source,
                chunk.timeline_start_ms,
                chunk.timeline_end_ms,
                chunk.storage_key,
                chunk.sample_rate as i64,
                chunk.sample_count,
                chunk.device_id,
                now_ms,
            ],
        )
        .map_err(classify(ENTITY_CHUNKS, &id, op))?;

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_CHUNKS,
                entity_id: id.clone(),
                revision: 1,
                occurred_at: now,
                operation: ChangeOperation::Create,
            },
        )
        .map_err(sql_err(op))?;

        let created = read_chunk(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_CHUNKS,
                id: id.clone(),
            })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(created)
    }

    // -- U6: finalize_chunk ------------------------------------------------

    fn finalize_chunk(&self, id: Uuid, facts: ChunkFacts) -> Result<AudioChunk, StoreError> {
        let op = "U6 finalize_chunk";
        let now = self.now();

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        let current = read_chunk(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_CHUNKS,
                id: id.clone(),
            })?;
        // Only a chunk still being written can be finalized. Re-finalizing a
        // `ready` chunk would rewrite its checksum, which is exactly the fact
        // damage detection relies on (contract §5.5).
        if current.state != ChunkState::Writing {
            return Err(StoreError::Conflict {
                entity: ENTITY_CHUNKS,
                id,
                reason: format!(
                    "chunk is {}, only 'writing' chunks can be finalized",
                    chunk_state_token(current.state)
                ),
            });
        }
        let revision = current.envelope.revision + 1;

        tx.execute(
            // `timeline_end_ms` is derived here, not at insert time: a chunk is
            // opened empty (start == end) and only its finalized sample count
            // tells how far along the timeline it actually reaches. Leaving it
            // at the insert value made every chunk zero-length, which in turn
            // collapsed every STT job range and every transcript timestamp to
            // the chunk start (regression: transcript segments all reported
            // start_ms == end_ms).
            "UPDATE meeting_audio_chunks
             SET file_size = ?2, sample_count = ?3, checksum = ?4,
                 gap_drift_metadata = COALESCE(?5, gap_drift_metadata),
                 timeline_end_ms = timeline_start_ms
                     + (?3 * 1000) / MAX(sample_rate, 1),
                 state = 'ready', revision = ?6, updated_at = ?7
             WHERE id = ?1",
            params![
                id.0,
                facts.file_size,
                facts.sample_count,
                facts.checksum,
                facts.gap_drift_metadata.as_ref().map(json_text),
                revision,
                to_ms(now),
            ],
        )
        .map_err(classify(ENTITY_CHUNKS, &id, op))?;

        // A durably finalized chunk is the only honest proof that a source
        // actually produced audio, so the session's capability flags are
        // raised here rather than optimistically at start. Nothing wrote them
        // before, which left every session claiming "no microphone audio, no
        // system audio" regardless of what had been captured — so the mic-only
        // badge could never render correctly.
        let source_column = match current.source.as_str() {
            "microphone" => Some("has_microphone_audio"),
            "system" => Some("has_system_audio"),
            _ => None,
        };
        if let Some(column) = source_column {
            tx.execute(
                &format!(
                    "UPDATE meeting_sessions SET {column} = 1, updated_at = ?2
                     WHERE id = ?1 AND {column} = 0"
                ),
                params![current.meeting_id.0, to_ms(now)],
            )
            .map_err(classify(ENTITY_SESSIONS, &id, op))?;
        }

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_CHUNKS,
                entity_id: id.clone(),
                revision,
                occurred_at: now,
                operation: ChangeOperation::Update,
            },
        )
        .map_err(sql_err(op))?;

        let finalized = read_chunk(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_CHUNKS,
                id: id.clone(),
            })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(finalized)
    }

    // -- U7: mark_chunk ----------------------------------------------------

    fn mark_chunk(
        &self,
        id: Uuid,
        state: ChunkState,
        reason: Option<String>,
    ) -> Result<(), StoreError> {
        let op = "U7 mark_chunk";
        // U7 is defined as `damaged`/`deleted` only; `writing` and `ready` are
        // reached through U5/U6, so accepting them here would create a second,
        // unlogged path into the chunk lifecycle.
        if !matches!(state, ChunkState::Damaged | ChunkState::Deleted) {
            return Err(StoreError::Conflict {
                entity: ENTITY_CHUNKS,
                id,
                reason: format!(
                    "mark_chunk accepts only 'damaged' or 'deleted', got '{}'",
                    chunk_state_token(state)
                ),
            });
        }
        let now = self.now();
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        let current = read_chunk(&tx, &id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_CHUNKS,
                id: id.clone(),
            })?;
        let revision = current.envelope.revision + 1;
        let deleting = state == ChunkState::Deleted;

        // `deleted` sets the tombstone marker as well, so the chunk surfaces
        // as `deleted == true` across the seam (contract §5.4); `damaged`
        // keeps the row alive for recovery/forensics.
        tx.execute(
            "UPDATE meeting_audio_chunks
             SET state = ?2, state_reason = ?3, revision = ?4, updated_at = ?5,
                 deleted_at = CASE WHEN ?6 THEN COALESCE(deleted_at, ?5) ELSE deleted_at END
             WHERE id = ?1",
            params![
                id.0,
                chunk_state_token(state),
                reason,
                revision,
                now_ms,
                deleting
            ],
        )
        .map_err(classify(ENTITY_CHUNKS, &id, op))?;

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_CHUNKS,
                entity_id: id.clone(),
                revision,
                occurred_at: now,
                operation: if deleting {
                    ChangeOperation::Tombstone
                } else {
                    ChangeOperation::Update
                },
            },
        )
        .map_err(sql_err(op))?;

        tx.commit().map_err(sql_err(op))
    }

    // -- U4: claim_due_stt_job ---------------------------------------------

    fn claim_due_stt_job(
        &self,
        worker: WorkerId,
        now: DateTime<Utc>,
    ) -> Result<Option<SttJobClaim>, StoreError> {
        let op = "U4 claim_due_stt_job";
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        // IMMEDIATE takes the write lock up front, so the candidate SELECT and
        // the claiming UPDATE cannot interleave with another process doing the
        // same. Tier B: no envelope, no change_log (rule W6).
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_err(op))?;

        let candidate: Option<String> = tx
            .query_row(
                "SELECT id FROM meeting_stt_jobs
                 WHERE state IN ('queued', 'retry_wait')
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
                 ORDER BY COALESCE(next_attempt_at, created_at) ASC, created_at ASC, rowid ASC
                 LIMIT 1",
                params![now_ms],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_err(op))?;

        let Some(job_id) = candidate else {
            tx.commit().map_err(sql_err(op))?;
            return Ok(None);
        };

        // The state predicate is repeated in the UPDATE so the claim is a
        // compare-and-set: if anything moved the job since the SELECT, zero
        // rows change and nobody hands out a duplicate lease.
        let claimed = tx
            .execute(
                "UPDATE meeting_stt_jobs
                 SET state = 'running', attempt_count = attempt_count + 1,
                     claimed_by = ?2, claimed_at = ?3, updated_at = ?3
                 WHERE id = ?1 AND state IN ('queued', 'retry_wait')",
                params![job_id, worker.0, now_ms],
            )
            .map_err(sql_err(op))?;
        if claimed == 0 {
            tx.commit().map_err(sql_err(op))?;
            return Ok(None);
        }

        let job = read_job(&tx, &job_id)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: "meeting_stt_jobs",
                id: Uuid(job_id.clone()),
            })?;
        tx.commit().map_err(sql_err(op))?;

        Ok(Some(SttJobClaim {
            job,
            worker,
            claimed_at: now,
        }))
    }

    // -- U4: transition_stt_job --------------------------------------------

    fn transition_stt_job(
        &self,
        id: Uuid,
        to: SttJobState,
        err: Option<String>,
    ) -> Result<(), StoreError> {
        let op = "U4 transition_stt_job";
        let now = self.now();

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        let current =
            read_job(&tx, &id.0)
                .map_err(sql_err(op))?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "meeting_stt_jobs",
                    id: id.clone(),
                })?;

        // Terminal states are terminal (plan §9 line 1197): a succeeded job
        // that could be moved back to `queued` would re-emit its segments.
        if job_state_is_terminal(current.state) {
            return Err(StoreError::Conflict {
                entity: "meeting_stt_jobs",
                id,
                reason: format!(
                    "job is in terminal state '{}' and cannot move to '{}'",
                    job_state_token(current.state),
                    job_state_token(to)
                ),
            });
        }

        // Leaving `running` releases the worker lease. `next_attempt_at` is
        // deliberately NOT computed here: retry backoff is the queue's policy
        // (stt-provider-contract §3), and inventing a schedule in the store
        // would make two components disagree about when a job is due.
        let releasing = to != SttJobState::Running;
        tx.execute(
            "UPDATE meeting_stt_jobs
             SET state = ?2,
                 last_error = COALESCE(?3, last_error),
                 claimed_by = CASE WHEN ?4 THEN NULL ELSE claimed_by END,
                 claimed_at = CASE WHEN ?4 THEN NULL ELSE claimed_at END,
                 updated_at = ?5
             WHERE id = ?1",
            params![id.0, job_state_token(to), err, releasing, to_ms(now)],
        )
        .map_err(classify("meeting_stt_jobs", &id, op))?;

        tx.commit().map_err(sql_err(op))
    }

    // -- U4: reschedule_stt_job --------------------------------------------

    fn reschedule_stt_job(
        &self,
        id: Uuid,
        next_attempt_at: DateTime<Utc>,
        err: Option<String>,
    ) -> Result<(), StoreError> {
        let op = "U4 reschedule_stt_job";
        let now = self.now();

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        let current =
            read_job(&tx, &id.0)
                .map_err(sql_err(op))?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "meeting_stt_jobs",
                    id: id.clone(),
                })?;
        if job_state_is_terminal(current.state) {
            return Err(StoreError::Conflict {
                entity: "meeting_stt_jobs",
                id,
                reason: format!(
                    "job is in terminal state '{}' and cannot be rescheduled",
                    job_state_token(current.state)
                ),
            });
        }
        // Only a held lease can be pushed back; anything else means two
        // workers disagree about who owns the job.
        if current.state != SttJobState::Running {
            return Err(StoreError::Conflict {
                entity: "meeting_stt_jobs",
                id,
                reason: format!(
                    "job is '{}', only a 'running' lease can move to 'retry_wait'",
                    job_state_token(current.state)
                ),
            });
        }

        tx.execute(
            "UPDATE meeting_stt_jobs
             SET state = 'retry_wait', next_attempt_at = ?2,
                 last_error = COALESCE(?3, last_error),
                 claimed_by = NULL, claimed_at = NULL, updated_at = ?4
             WHERE id = ?1",
            params![id.0, to_ms(next_attempt_at), err, to_ms(now)],
        )
        .map_err(classify("meeting_stt_jobs", &id, op))?;

        tx.commit().map_err(sql_err(op))
    }

    // -- read: chunks_for_job ----------------------------------------------

    fn chunks_for_job(&self, job_id: Uuid) -> Result<Vec<AudioChunk>, StoreError> {
        let op = "read chunks_for_job";
        let conn = self.locked(op)?;

        let ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT chunk_id FROM meeting_stt_job_chunks
                     WHERE job_id = ?1 ORDER BY position ASC, chunk_id ASC",
                )
                .map_err(sql_err(op))?;
            let mapped = stmt
                .query_map(params![job_id.0], |row| row.get(0))
                .map_err(sql_err(op))?;
            mapped
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_err(op))?
        };

        let mut chunks = Vec::with_capacity(ids.len());
        for chunk_id in ids {
            match read_chunk(&conn, &chunk_id).map_err(sql_err(op))? {
                Some(chunk) => chunks.push(chunk),
                None => {
                    return Err(StoreError::NotFound {
                        entity: ENTITY_CHUNKS,
                        id: Uuid(chunk_id),
                    })
                }
            }
        }
        Ok(chunks)
    }

    // -- U8: insert_segments -----------------------------------------------

    fn insert_segments(
        &self,
        job_id: Uuid,
        segments: Vec<NewTranscriptSegment>,
    ) -> Result<Vec<TranscriptSegment>, StoreError> {
        let op = "U8 insert_segments";
        let now = self.now();
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        // The meeting is taken from the job rather than from the caller: the
        // job already fixes which session its output belongs to, and letting a
        // caller supply it again would allow the two to disagree.
        let job = read_job(&tx, &job_id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: "meeting_stt_jobs",
                id: job_id.clone(),
            })?;

        // Idempotent resume (plan §13 Phase 6: saved responses are never
        // duplicated): if a previous attempt of this job already committed its
        // segments (crash between insert and the Succeeded transition), return
        // them instead of writing a second copy.
        let existing = load_job_segments(&tx, &job_id.0).map_err(sql_err(op))?;
        if !existing.is_empty() {
            log::debug!(
                "Job {} already has {} persisted segment(s); skipping re-insert",
                job_id.0,
                existing.len()
            );
            tx.commit().map_err(sql_err(op))?;
            return Ok(existing);
        }

        let mut inserted = Vec::with_capacity(segments.len());
        for segment in segments {
            let id = self.uuids.new_uuid();
            tx.execute(
                "INSERT INTO meeting_transcript_segments (
                    id, meeting_id, job_id, source, start_ms, end_ms, text, word_timestamps,
                    confidence, provider, model_id, sequence, revision, created_at,
                    updated_at, deleted_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?13, NULL
                 )",
                params![
                    id.0,
                    job.meeting_id.0,
                    job_id.0,
                    segment.source,
                    segment.start_ms,
                    segment.end_ms,
                    segment.text,
                    segment.word_timestamps.as_ref().map(json_text),
                    segment.confidence.map(f64::from),
                    segment.provider,
                    segment.model_id,
                    segment.sequence,
                    now_ms,
                ],
            )
            .map_err(classify(ENTITY_SEGMENTS, &id, op))?;

            // One log row per segment (U8), all in this single transaction.
            change_log::append(
                &tx,
                ChangeLogEntry {
                    entity: ENTITY_SEGMENTS,
                    entity_id: id.clone(),
                    revision: 1,
                    occurred_at: now,
                    operation: ChangeOperation::Create,
                },
            )
            .map_err(sql_err(op))?;

            let stored = tx
                .query_row(
                    &format!(
                        "SELECT {SEGMENT_COLUMNS} FROM meeting_transcript_segments WHERE id = ?1"
                    ),
                    params![id.0],
                    segment_from_row,
                )
                .map_err(sql_err(op))?;
            inserted.push(stored);
        }

        tx.commit().map_err(sql_err(op))?;
        Ok(inserted)
    }

    // -- U9: upsert_summary ------------------------------------------------

    fn upsert_summary(&self, summary: NewMeetingSummary) -> Result<MeetingSummary, StoreError> {
        let op = "U9 upsert_summary";
        let now = self.now();
        let now_ms = to_ms(now);

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        // INSERT-vs-UPDATE is decided from the meeting id (U9 semantics); the
        // UNIQUE constraint on `meeting_id` makes that the only possible
        // outcome even under a race.
        let existing = read_summary_by_meeting(&tx, &summary.meeting_id.0).map_err(sql_err(op))?;

        let (id, revision, operation) = match &existing {
            Some(row) => (
                row.envelope.id.clone(),
                row.envelope.revision + 1,
                ChangeOperation::Update,
            ),
            None => (self.uuids.new_uuid(), 1, ChangeOperation::Create),
        };

        if existing.is_some() {
            tx.execute(
                "UPDATE meeting_summaries
                 SET provider = ?2, model = ?3, prompt_version = ?4, state = ?5,
                     summary_markdown = ?6, decisions = ?7, action_items = ?8,
                     open_questions = ?9, last_error = ?10, revision = ?11, updated_at = ?12
                 WHERE id = ?1",
                params![
                    id.0,
                    summary.provider,
                    summary.model,
                    summary.prompt_version,
                    summary.state,
                    summary.summary_markdown,
                    json_text(&summary.decisions),
                    json_text(&summary.action_items),
                    json_text(&summary.open_questions),
                    summary.last_error,
                    revision,
                    now_ms,
                ],
            )
            .map_err(classify(ENTITY_SUMMARIES, &id, op))?;
        } else {
            tx.execute(
                "INSERT INTO meeting_summaries (
                    id, meeting_id, provider, model, prompt_version, state, summary_markdown,
                    decisions, action_items, open_questions, last_error, revision,
                    created_at, updated_at, deleted_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12, NULL
                 )",
                params![
                    id.0,
                    summary.meeting_id.0,
                    summary.provider,
                    summary.model,
                    summary.prompt_version,
                    summary.state,
                    summary.summary_markdown,
                    json_text(&summary.decisions),
                    json_text(&summary.action_items),
                    json_text(&summary.open_questions),
                    summary.last_error,
                    now_ms,
                ],
            )
            .map_err(classify(ENTITY_SUMMARIES, &id, op))?;
        }

        change_log::append(
            &tx,
            ChangeLogEntry {
                entity: ENTITY_SUMMARIES,
                entity_id: id.clone(),
                revision,
                occurred_at: now,
                operation,
            },
        )
        .map_err(sql_err(op))?;

        let stored = read_summary_by_meeting(&tx, &summary.meeting_id.0)
            .map_err(sql_err(op))?
            .ok_or_else(|| StoreError::NotFound {
                entity: ENTITY_SUMMARIES,
                id: id.clone(),
            })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(stored)
    }
}

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
        fn max_segment_sequence(&self, _meeting_id: Uuid) -> Result<i64, StoreError> {
            unimplemented!("fake")
        }
        fn delete_meeting_audio_rows(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn purge_meeting_content(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn create_project(&self, _draft: NewProject) -> Result<Project, StoreError> {
            unimplemented!("fake")
        }
        fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
            unimplemented!("fake")
        }
        fn rename_project(&self, _id: Uuid, _name: String) -> Result<Project, StoreError> {
            unimplemented!("fake")
        }
        fn soft_delete_project(&self, _id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn assign_meeting_to_project(
            &self,
            _project_id: Uuid,
            _meeting_id: Uuid,
        ) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn unassign_meeting_from_project(
            &self,
            _project_id: Uuid,
            _meeting_id: Uuid,
        ) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn list_meeting_ids_for_project(
            &self,
            _project_id: Uuid,
        ) -> Result<Vec<String>, StoreError> {
            unimplemented!("fake")
        }
        fn list_unassigned_meeting_ids(&self) -> Result<Vec<String>, StoreError> {
            unimplemented!("fake")
        }
        fn delete_project_links_for_meeting(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn create_chat_source(&self, _draft: NewChatSource) -> Result<(Source, bool), StoreError> {
            unimplemented!("fake")
        }
        fn list_sources(&self) -> Result<Vec<Source>, StoreError> {
            unimplemented!("fake")
        }
        fn list_source_items(&self, _source_id: Uuid) -> Result<Vec<SourceItem>, StoreError> {
            unimplemented!("fake")
        }
        fn attach_source_assets(
            &self,
            _source_id: Uuid,
            _assets: Vec<NewSourceAsset>,
        ) -> Result<Vec<SourceAsset>, StoreError> {
            unimplemented!("fake")
        }
        fn list_source_assets(&self, _source_id: Uuid) -> Result<Vec<SourceAsset>, StoreError> {
            unimplemented!("fake")
        }
        fn create_manual_task(&self, _draft: NewProjectTask) -> Result<ProjectTask, StoreError> {
            unimplemented!("fake")
        }
        fn suggest_task(&self, _draft: NewProjectTask) -> Result<(ProjectTask, bool), StoreError> {
            unimplemented!("fake")
        }
        fn list_tasks(&self, _project_id: Uuid) -> Result<Vec<ProjectTask>, StoreError> {
            unimplemented!("fake")
        }
        fn decide_task(
            &self,
            _id: Uuid,
            _state: &str,
            _title: Option<String>,
            _description: Option<String>,
        ) -> Result<ProjectTask, StoreError> {
            unimplemented!("fake")
        }
        fn create_snapshot(
            &self,
            _project_id: Uuid,
            _summary_markdown: String,
            _prompt_version: String,
            _model: Option<String>,
            _source_marker: String,
        ) -> Result<ProjectSnapshot, StoreError> {
            unimplemented!("fake")
        }
        fn list_snapshots(&self, _project_id: Uuid) -> Result<Vec<ProjectSnapshot>, StoreError> {
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
            let _ = (id, to, err);
            unimplemented!("fake")
        }
        fn reschedule_stt_job(
            &self,
            id: Uuid,
            next_attempt_at: DateTime<Utc>,
            err: Option<String>,
        ) -> Result<(), StoreError> {
            let _ = (id, next_attempt_at, err);
            unimplemented!("fake")
        }
        fn chunks_for_job(&self, job_id: Uuid) -> Result<Vec<AudioChunk>, StoreError> {
            let _ = job_id;
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

/// Draft for [`SqliteMeetingStore::enqueue_stt_job`].
///
/// Tier B (plan §9 lines 1190–1199): no envelope, no `change_log`, so there is
/// no `revision` here. `next_attempt_at` is `None` for "due immediately".
#[derive(Debug, Clone)]
pub struct NewSttJob {
    /// Owning session.
    pub meeting_id: Uuid,
    /// Chunks this job transcribes, in consumption order (plan line 1193).
    pub chunk_ids: Vec<Uuid>,
    /// Capture source or Azure channel mapping.
    pub source: String,
    /// Timeline coverage start, milliseconds.
    pub timeline_start_ms: i64,
    /// Timeline coverage end, milliseconds.
    pub timeline_end_ms: i64,
    /// Provider id that will execute the job.
    pub provider: String,
    /// Non-secret configuration snapshot (plan line 1196, line 1334).
    pub config_snapshot: serde_json::Value,
    /// Earliest attempt instant; `None` makes the job due right away.
    pub next_attempt_at: Option<DateTime<Utc>>,
}

impl SqliteMeetingStore {
    /// Enqueues one STT job together with its chunk references.
    ///
    /// **Inherent, not part of [`MeetingRepository`], on purpose.** The
    /// repository-boundary contract's unit list covers job *state churn* (U4)
    /// but names no unit that creates a job, so adding an `enqueue` member to
    /// the trait would extend the seam ahead of its contract. Jobs must
    /// nevertheless be creatable for the queue (and for this module's tests)
    /// to have anything to claim, so the operation lives on the concrete store
    /// until `MeetingTranscriptionQueue` lands and the contract's write block
    /// gains the corresponding unit.
    ///
    /// The job row and its `meeting_stt_job_chunks` links are one transaction:
    /// a job that references no chunks would be claimed and then transcribe
    /// nothing. Tier B, so no `change_log` rows (rule W6).
    pub fn enqueue_stt_job(&self, draft: NewSttJob) -> Result<SttJob, StoreError> {
        let op = "enqueue_stt_job";
        let id = self.uuids.new_uuid();
        let now_ms = to_ms(self.now());

        let mut conn = self.locked(op)?;
        let tx = conn.transaction().map_err(sql_err(op))?;

        // Idempotence guard, inside the write transaction. The two callers
        // (`enqueue_chunk_job` on recorder finalize and `enqueue_ready_chunks`
        // on stop) both pre-check coverage on a *different* connection, so
        // their checks can interleave; only this transaction — backed by the
        // `idx_meeting_stt_job_chunks_unique` index — is authoritative.
        // Returning the existing job makes the losing caller a no-op instead
        // of duplicating the chunk's transcript.
        for chunk_id in &draft.chunk_ids {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT job_id FROM meeting_stt_job_chunks WHERE chunk_id = ?1",
                    params![chunk_id.0],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sql_err(op))?;
            if let Some(job_id) = existing {
                let job =
                    read_job(&tx, &job_id)
                        .map_err(sql_err(op))?
                        .ok_or(StoreError::NotFound {
                            entity: "meeting_stt_jobs",
                            id: Uuid(job_id),
                        })?;
                return Ok(job);
            }
        }

        tx.execute(
            "INSERT INTO meeting_stt_jobs (
                id, meeting_id, source, timeline_start_ms, timeline_end_ms, provider,
                config_snapshot, state, attempt_count, next_attempt_at, claimed_by,
                claimed_at, last_error, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', 0, ?8, NULL, NULL, NULL, ?9, ?9
             )",
            params![
                id.0,
                draft.meeting_id.0,
                draft.source,
                draft.timeline_start_ms,
                draft.timeline_end_ms,
                draft.provider,
                json_text(&draft.config_snapshot),
                draft.next_attempt_at.map(to_ms),
                now_ms,
            ],
        )
        .map_err(classify("meeting_stt_jobs", &id, op))?;

        for (position, chunk_id) in draft.chunk_ids.iter().enumerate() {
            tx.execute(
                "INSERT INTO meeting_stt_job_chunks (job_id, chunk_id, position)
                 VALUES (?1, ?2, ?3)",
                params![id.0, chunk_id.0, position as i64],
            )
            .map_err(classify("meeting_stt_job_chunks", &id, op))?;
        }

        let job =
            read_job(&tx, &id.0)
                .map_err(sql_err(op))?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "meeting_stt_jobs",
                    id: id.clone(),
                })?;
        tx.commit().map_err(sql_err(op))?;
        Ok(job)
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use rusqlite_migration::Migrations;
    use std::path::PathBuf;

    // -- fixtures ----------------------------------------------------------

    fn temp_store() -> (tempfile::TempDir, SqliteMeetingStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            SqliteMeetingStore::open(dir.path().join(MEETING_DB_FILE)).expect("open meeting store");
        (dir, store)
    }

    fn draft_session(title: &str) -> NewMeetingSession {
        NewMeetingSession {
            title: title.to_string(),
            started_at: Utc::now(),
            stt_provider: "local".to_string(),
            provider_config_snapshot: serde_json::json!({ "model": "parakeet" }),
            meeting_model_id: Some("parakeet-v3".to_string()),
            language: Some("de".to_string()),
            microphone_device: Some("mic-0".to_string()),
            output_device: None,
            has_microphone_audio: true,
            has_system_audio: false,
            recording_retention_policy: "keep".to_string(),
            summary_provider: Some("local-llm".to_string()),
        }
    }

    fn draft_chunk(meeting: &Uuid, sequence: i64) -> NewAudioChunk {
        NewAudioChunk {
            meeting_id: meeting.clone(),
            sequence,
            source: "microphone".to_string(),
            timeline_start_ms: sequence * 60_000,
            timeline_end_ms: (sequence + 1) * 60_000,
            storage_key: format!("meetings/{}/chunks/{sequence:05}.wav", meeting.0),
            sample_rate: 16_000,
            sample_count: 0,
            device_id: Some("mic-0".to_string()),
        }
    }

    fn draft_job(meeting: &Uuid, chunks: Vec<Uuid>) -> NewSttJob {
        NewSttJob {
            meeting_id: meeting.clone(),
            chunk_ids: chunks,
            source: "microphone".to_string(),
            timeline_start_ms: 0,
            timeline_end_ms: 60_000,
            provider: "local".to_string(),
            config_snapshot: serde_json::json!({ "beam": 5 }),
            next_attempt_at: None,
        }
    }

    /// Regression: a chunk is inserted empty (`timeline_end_ms` ==
    /// `timeline_start_ms`) and only its finalized sample count says how far
    /// along the timeline it reaches. Finalisation used to leave the end at
    /// the insert value, so every chunk stayed zero-length and every STT job
    /// range and transcript timestamp derived from it collapsed onto the chunk
    /// start (observed in the field: all segments reported start == end == 0).
    #[test]
    fn finalize_chunk_derives_the_timeline_end_from_the_sample_count() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Timeline"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        let mut draft = draft_chunk(&meeting_id, 1);
        draft.timeline_start_ms = 60_000;
        draft.timeline_end_ms = 60_000;
        let chunk = store.append_chunk(draft).expect("chunk");

        // 16 kHz mono, 48 000 samples = exactly 3 s of audio.
        let finalized = store
            .finalize_chunk(
                chunk.envelope.id.clone(),
                ChunkFacts {
                    file_size: 48_000 * 2 + 44,
                    sample_count: 48_000,
                    checksum: "a".repeat(64),
                    gap_drift_metadata: None,
                },
            )
            .expect("finalize");

        assert_eq!(finalized.timeline_start_ms, 60_000);
        assert_eq!(
            finalized.timeline_end_ms, 63_000,
            "a 3 s chunk starting at 60 s must end at 63 s, not at its start"
        );
    }

    /// Regression: `has_microphone_audio` / `has_system_audio` were written
    /// once at session creation and never again, so a session always claimed
    /// it had captured nothing and the mic-only badge could not render.
    #[test]
    fn finalizing_a_chunk_records_which_sources_produced_audio() {
        let (_dir, store) = temp_store();
        let mut draft = draft_session("Sources");
        draft.has_microphone_audio = false;
        draft.has_system_audio = false;
        let session = store.create_session(draft).expect("session");
        let meeting_id = session.envelope.id.clone();

        let mut system_chunk = draft_chunk(&meeting_id, 0);
        system_chunk.source = "system".to_string();
        let chunk = store.append_chunk(system_chunk).expect("chunk");
        store
            .finalize_chunk(
                chunk.envelope.id,
                ChunkFacts {
                    file_size: 1_644,
                    sample_count: 800,
                    checksum: "b".repeat(64),
                    gap_drift_metadata: None,
                },
            )
            .expect("finalize");

        let read = store.get_session(meeting_id).expect("read").expect("row");
        assert!(
            read.has_system_audio,
            "a finalized system chunk proves system audio was captured"
        );
        assert!(
            !read.has_microphone_audio,
            "no microphone chunk was finalized, so the flag must stay false"
        );
    }

    /// Regression: the recorder's per-chunk enqueue and the stop path's sweep
    /// pre-check coverage on a different connection, so their checks can
    /// interleave. Three chunks in a real recording ended up with two jobs
    /// created in the same millisecond, and every one of their segments landed
    /// in the transcript twice.
    #[test]
    fn enqueueing_a_covered_chunk_returns_the_existing_job() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Race"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        let chunk = store
            .append_chunk(draft_chunk(&meeting_id, 0))
            .expect("chunk");

        let first = store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id.clone()]))
            .expect("first enqueue");
        let second = store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id]))
            .expect("second enqueue");

        assert_eq!(first.id, second.id, "the losing caller must be a no-op");
        let jobs = store.due_stt_jobs(Utc::now(), 10).expect("due jobs");
        assert_eq!(jobs.len(), 1, "exactly one job may own a chunk");
    }

    /// A session plus one finalized chunk plus one claimed-ready job — the
    /// minimum context most write units need.
    fn seeded(store: &SqliteMeetingStore) -> (Uuid, Uuid, Uuid) {
        let session = store
            .create_session(draft_session("Seeded"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        let chunk = store
            .append_chunk(draft_chunk(&meeting_id, 0))
            .expect("chunk");
        let job = store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id.clone()]))
            .expect("job");
        (meeting_id, chunk.envelope.id, job.id)
    }

    /// Full DDL fingerprint of the database, used to prove that a second
    /// migration run changes nothing.
    fn schema_fingerprint(path: &PathBuf) -> (i64, Vec<(String, String)>) {
        let conn = Connection::open(path).expect("open raw");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        let mut stmt = conn
            .prepare(
                "SELECT name, COALESCE(sql, '') FROM sqlite_master
                 WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
            )
            .expect("prepare");
        let objects = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<Vec<(String, String)>>>()
            .expect("collect");
        (user_version, objects)
    }

    fn count(store: &SqliteMeetingStore, sql: &str) -> i64 {
        let conn = store.conn.lock().expect("lock");
        conn.query_row(sql, [], |row| row.get(0)).expect("count")
    }

    // -- migrations --------------------------------------------------------

    /// Running the chain twice is a no-op: same `user_version`, byte-identical
    /// set of tables and indexes.
    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(MEETING_DB_FILE);

        drop(SqliteMeetingStore::open(path.clone()).expect("first open"));
        let first = schema_fingerprint(&path);

        drop(SqliteMeetingStore::open(path.clone()).expect("second open"));
        let second = schema_fingerprint(&path);

        assert_eq!(first, second, "second migration run changed the schema");
        assert_eq!(
            first.0,
            MIGRATIONS.len() as i64,
            "user_version tracks the chain length"
        );
        let names: Vec<&str> = first.1.iter().map(|(n, _)| n.as_str()).collect();
        for table in [
            "meeting_sessions",
            "meeting_audio_chunks",
            "meeting_stt_jobs",
            "meeting_stt_job_chunks",
            "meeting_transcript_segments",
            "meeting_summaries",
            "change_log",
        ] {
            assert!(names.contains(&table), "missing table {table}: {names:?}");
        }
    }

    /// The regression that matters for shipped installations: a
    /// `meeting_sessions` row written by the wave-1 schema (chain step 1 only)
    /// must come back unchanged after the new steps 2–6 are applied.
    #[test]
    fn legacy_session_row_survives_new_migrations() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(MEETING_DB_FILE);
        let legacy_id = "11111111-1111-4111-8111-111111111111";

        {
            // Exactly the old database: step 1 of the chain, nothing else.
            let mut conn = Connection::open(&path).expect("open legacy");
            conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
            Migrations::new(vec![MIGRATIONS[0].clone()])
                .to_latest(&mut conn)
                .expect("legacy migration");
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .expect("version");
            assert_eq!(version, 1, "fixture must sit on the wave-1 schema");

            conn.execute(
                "INSERT INTO meeting_sessions (id, title, state, started_at, created_at, updated_at)
                 VALUES (?1, 'Legacy kickoff', 'draft', 1700000000000, 1700000000000, 1700000000000)",
                params![legacy_id],
            )
            .expect("insert legacy row");
        }

        // Upgrade through the full chain.
        let store = SqliteMeetingStore::open(path.clone()).expect("upgrade open");
        let (version, _) = schema_fingerprint(&path);
        assert_eq!(version, MIGRATIONS.len() as i64);

        let session = store
            .get_session(Uuid(legacy_id.to_string()))
            .expect("get_session")
            .expect("legacy row still present");
        assert_eq!(session.title, "Legacy kickoff");
        assert_eq!(session.state, "draft");
        assert_eq!(
            session.envelope.revision, 1,
            "migration must not bump revisions"
        );
        assert_eq!(
            session.envelope.updated_at.timestamp_millis(),
            1_700_000_000_000
        );
        assert!(session.envelope.deleted_at.is_none());
        // Column defaults from the wave-1 DDL survive too.
        assert_eq!(session.stt_provider, "local");
        assert_eq!(session.summary_state, "none");
        assert_eq!(session.provider_config_snapshot, serde_json::json!({}));
        assert_eq!(count(&store, "SELECT COUNT(*) FROM meeting_sessions"), 1);
    }

    /// `MeetingManager` and the store share one file and one chain, in either
    /// opening order.
    #[test]
    fn manager_and_store_share_one_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(MEETING_DB_FILE);

        let manager = crate::managers::meeting::MeetingManager::open(path.clone())
            .expect("manager opens the migrated db");
        let created = manager
            .create_session(Some("Shared".into()))
            .expect("manager writes");

        let store = SqliteMeetingStore::open(path).expect("store opens the same db");
        let seen = store
            .get_session(Uuid(created.id.clone()))
            .expect("store reads")
            .expect("row visible through the seam");
        assert_eq!(seen.title, "Shared");
    }

    // -- sessions ----------------------------------------------------------

    #[test]
    fn create_session_round_trips_and_logs_one_create_row() {
        let (_dir, store) = temp_store();
        let created = store
            .create_session(draft_session("Kickoff"))
            .expect("create");

        assert_eq!(created.envelope.revision, 1);
        assert!(created.envelope.deleted_at.is_none());
        assert!(!created.envelope.deleted());

        let read = store
            .get_session(created.envelope.id.clone())
            .expect("get")
            .expect("present");
        assert_eq!(read.title, "Kickoff");
        assert_eq!(read.language.as_deref(), Some("de"));
        assert_eq!(read.microphone_device.as_deref(), Some("mic-0"));
        assert!(read.has_microphone_audio);
        assert!(!read.has_system_audio);
        assert_eq!(read.recording_retention_policy, "keep");
        assert_eq!(
            read.provider_config_snapshot,
            serde_json::json!({ "model": "parakeet" }),
            "JSON snapshot survives the text column round trip"
        );

        // W2: the change row is committed together with the data row.
        let logged = count(
            &store,
            "SELECT COUNT(*) FROM change_log
             WHERE entity = 'meeting_sessions' AND operation = 'create' AND revision = 1",
        );
        assert_eq!(logged, 1);
    }

    #[test]
    fn update_session_bumps_revision_and_leaves_unpatched_columns_alone() {
        let (_dir, store) = temp_store();
        let created = store
            .create_session(draft_session("Before"))
            .expect("create");
        let id = created.envelope.id.clone();

        let updated = store
            .update_session(
                id.clone(),
                SessionPatch {
                    title: Some("After".to_string()),
                    state: Some("recording".to_string()),
                    ..SessionPatch::default()
                },
            )
            .expect("update");

        assert_eq!(updated.title, "After");
        assert_eq!(updated.state, "recording");
        assert_eq!(updated.envelope.revision, 2, "W4: exactly one bump");
        assert_eq!(
            updated.language.as_deref(),
            Some("de"),
            "None patch fields must not clear columns"
        );
        assert_eq!(
            updated.created_at, created.created_at,
            "created_at is immutable"
        );

        assert!(store
            .update_session(Uuid("no-such-id".into()), SessionPatch::default())
            .is_err());
    }

    #[test]
    fn soft_delete_session_tombstones_and_keeps_children() {
        let (_dir, store) = temp_store();
        let (meeting_id, chunk_id, job_id) = seeded(&store);
        store
            .insert_segments(
                job_id,
                vec![NewTranscriptSegment {
                    source: "microphone".to_string(),
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "hello".to_string(),
                    word_timestamps: None,
                    confidence: None,
                    provider: "local".to_string(),
                    model_id: None,
                    sequence: 0,
                }],
            )
            .expect("segments");

        store
            .soft_delete_session(meeting_id.clone())
            .expect("soft delete");

        // Invisible to reads...
        assert!(store
            .get_session(meeting_id.clone())
            .expect("get")
            .is_none());
        assert!(store
            .list_sessions(Page {
                offset: 0,
                limit: 10
            })
            .expect("list")
            .items
            .is_empty());

        // ...but the row and every child row are still there: v1 has no hard
        // delete for tier A, so the FK cascades never fire (contract §5.4).
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM meeting_sessions WHERE deleted_at IS NOT NULL"
            ),
            1
        );
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM meeting_audio_chunks"),
            1
        );
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM meeting_transcript_segments"),
            1
        );
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM change_log
                 WHERE entity = 'meeting_sessions' AND operation = 'tombstone'"
            ),
            1
        );
        // Deleting twice is a NotFound, not a silent second tombstone.
        assert!(store.soft_delete_session(meeting_id).is_err());
        let _ = chunk_id;
    }

    #[test]
    fn list_sessions_paginates_newest_first() {
        let (_dir, store) = temp_store();
        for n in 0..3 {
            store
                .create_session(draft_session(&format!("S{n}")))
                .expect("create");
            std::thread::sleep(std::time::Duration::from_millis(3));
        }

        let page = store
            .list_sessions(Page {
                offset: 0,
                limit: 2,
            })
            .expect("page 1");
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].title, "S2", "newest first");

        let rest = store
            .list_sessions(Page {
                offset: 2,
                limit: 2,
            })
            .expect("page 2");
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.items[0].title, "S0");
    }

    // -- audio chunks ------------------------------------------------------

    #[test]
    fn chunk_insert_and_finalize_round_trip() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Capture"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();

        let chunk = store
            .append_chunk(draft_chunk(&meeting_id, 0))
            .expect("append");
        assert_eq!(chunk.state, ChunkState::Writing);
        assert_eq!(chunk.envelope.revision, 1);
        assert_eq!(chunk.sample_rate, 16_000);
        assert_eq!(chunk.file_size, 0);
        assert!(chunk.checksum.is_none());
        assert!(
            !chunk.storage_key.starts_with('/') && !chunk.storage_key.contains(':'),
            "storage keys stay relative (plan lines 1158-1159): {}",
            chunk.storage_key
        );

        let checksum = "a".repeat(64);
        let finalized = store
            .finalize_chunk(
                chunk.envelope.id.clone(),
                ChunkFacts {
                    file_size: 320_044,
                    sample_count: 160_000,
                    checksum: checksum.clone(),
                    gap_drift_metadata: Some(serde_json::json!({ "drift_ms": 4 })),
                },
            )
            .expect("finalize");

        assert_eq!(finalized.state, ChunkState::Ready);
        assert_eq!(finalized.envelope.revision, 2);
        assert_eq!(finalized.file_size, 320_044);
        assert_eq!(finalized.sample_count, 160_000);
        assert_eq!(finalized.checksum.as_deref(), Some(checksum.as_str()));
        assert_eq!(
            finalized.gap_drift_metadata,
            Some(serde_json::json!({ "drift_ms": 4 }))
        );

        // Finalizing twice would rewrite the damage-detection checksum.
        assert!(store
            .finalize_chunk(
                chunk.envelope.id,
                ChunkFacts {
                    file_size: 1,
                    sample_count: 1,
                    checksum: "b".repeat(64),
                    gap_drift_metadata: None,
                },
            )
            .is_err());
    }

    #[test]
    fn mark_chunk_damaged_keeps_row_alive_and_deleted_tombstones() {
        let (_dir, store) = temp_store();
        let (meeting_id, _, _) = seeded(&store);

        let damaged = store
            .append_chunk(draft_chunk(&meeting_id, 1))
            .expect("chunk 1");
        store
            .mark_chunk(
                damaged.envelope.id.clone(),
                ChunkState::Damaged,
                Some("short read at tail".to_string()),
            )
            .expect("mark damaged");

        let removed = store
            .append_chunk(draft_chunk(&meeting_id, 2))
            .expect("chunk 2");
        store
            .mark_chunk(removed.envelope.id.clone(), ChunkState::Deleted, None)
            .expect("mark deleted");

        let conn = store.conn.lock().expect("lock");
        let damaged_row = read_chunk(&conn, &damaged.envelope.id.0)
            .expect("read")
            .expect("present");
        assert_eq!(damaged_row.state, ChunkState::Damaged);
        assert!(
            !damaged_row.envelope.deleted(),
            "damaged is a quality verdict, not a deletion"
        );
        assert_eq!(damaged_row.envelope.revision, 2);

        let deleted_row = read_chunk(&conn, &removed.envelope.id.0)
            .expect("read")
            .expect("row is kept, only the content goes");
        assert_eq!(deleted_row.state, ChunkState::Deleted);
        assert!(deleted_row.envelope.deleted());
        drop(conn);

        // U7 covers damaged/deleted only.
        assert!(store
            .mark_chunk(damaged.envelope.id, ChunkState::Ready, None)
            .is_err());
    }

    /// Foreign keys are enforced on every connection: a chunk for a session
    /// that does not exist is rejected rather than orphaned.
    #[test]
    fn foreign_keys_reject_orphan_rows() {
        let (_dir, store) = temp_store();
        let orphan = draft_chunk(&Uuid("00000000-0000-4000-8000-0000000000ff".into()), 0);
        let err = store.append_chunk(orphan).expect_err("must be rejected");
        assert!(
            matches!(err, StoreError::Conflict { .. }),
            "expected a Conflict, got {err:?}"
        );
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM meeting_audio_chunks"),
            0
        );
    }

    // -- STT jobs ----------------------------------------------------------

    #[test]
    fn stt_job_round_trips_with_its_chunk_references() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Jobs"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        let first = store.append_chunk(draft_chunk(&meeting_id, 0)).expect("c0");
        let second = store.append_chunk(draft_chunk(&meeting_id, 1)).expect("c1");

        let job = store
            .enqueue_stt_job(draft_job(
                &meeting_id,
                vec![first.envelope.id.clone(), second.envelope.id.clone()],
            ))
            .expect("enqueue");

        assert_eq!(job.state, SttJobState::Queued);
        assert_eq!(job.attempt_count, 0);
        assert_eq!(job.chunk_ids, vec![first.envelope.id, second.envelope.id]);
        assert_eq!(job.config_snapshot, serde_json::json!({ "beam": 5 }));

        let due = store.due_stt_jobs(Utc::now(), 10).expect("due");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, job.id);
        assert_eq!(due[0].chunk_ids.len(), 2);

        // Tier B: job churn never reaches the change log (rule W6).
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM change_log WHERE entity = 'meeting_stt_jobs'"
            ),
            0
        );
    }

    /// U4's core guarantee: one job goes to exactly one worker.
    #[test]
    fn claim_hands_each_job_to_exactly_one_worker() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Queue"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        // Two distinct chunks: one chunk maps to exactly one job, enforced by
        // `idx_meeting_stt_job_chunks_unique`, so a second enqueue for the same
        // chunk returns the first job instead of creating a rival.
        let chunk_a = store
            .append_chunk(draft_chunk(&meeting_id, 0))
            .expect("chunk a");
        let chunk_b = store
            .append_chunk(draft_chunk(&meeting_id, 1))
            .expect("chunk b");
        let job_a = store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_a.envelope.id.clone()]))
            .expect("job a");
        let job_b = store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_b.envelope.id]))
            .expect("job b");

        assert_eq!(
            store
                .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_a.envelope.id]))
                .expect("re-enqueue")
                .id,
            job_a.id,
            "a chunk that is already covered must not spawn a second job"
        );

        let now = Utc::now();
        let first = store
            .claim_due_stt_job(WorkerId("worker-1".into()), now)
            .expect("claim 1")
            .expect("a job was due");
        let second = store
            .claim_due_stt_job(WorkerId("worker-2".into()), now)
            .expect("claim 2")
            .expect("the other job was due");

        assert_ne!(
            first.job.id, second.job.id,
            "the same job must never be handed to two workers"
        );
        let claimed = [first.job.id.clone(), second.job.id.clone()];
        assert!(claimed.contains(&job_a.id) && claimed.contains(&job_b.id));

        assert_eq!(first.job.state, SttJobState::Running);
        assert_eq!(first.job.attempt_count, 1, "claiming counts as an attempt");
        assert_eq!(first.worker, WorkerId("worker-1".into()));

        // Nothing is due any more: both jobs are running.
        assert!(store
            .claim_due_stt_job(WorkerId("worker-3".into()), now)
            .expect("claim 3")
            .is_none());
        assert!(store.due_stt_jobs(now, 10).expect("due").is_empty());
    }

    #[test]
    fn claim_respects_next_attempt_at_backoff() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session(draft_session("Backoff"))
            .expect("session");
        let meeting_id = session.envelope.id.clone();
        let chunk = store
            .append_chunk(draft_chunk(&meeting_id, 0))
            .expect("chunk");

        let now = Utc::now();
        let mut draft = draft_job(&meeting_id, vec![chunk.envelope.id]);
        draft.next_attempt_at = Some(now + chrono::Duration::seconds(30));
        store.enqueue_stt_job(draft).expect("enqueue");

        assert!(store
            .claim_due_stt_job(WorkerId("w".into()), now)
            .expect("claim")
            .is_none());
        assert!(store
            .claim_due_stt_job(WorkerId("w".into()), now + chrono::Duration::seconds(31))
            .expect("claim later")
            .is_some());
    }

    #[test]
    fn transition_releases_the_lease_and_terminal_states_are_final() {
        let (_dir, store) = temp_store();
        let (_, _, job_id) = seeded(&store);
        let now = Utc::now();
        store
            .claim_due_stt_job(WorkerId("w".into()), now)
            .expect("claim")
            .expect("due");

        store
            .transition_stt_job(
                job_id.clone(),
                SttJobState::RetryWait,
                Some("timeout".into()),
            )
            .expect("retry_wait");
        let conn = store.conn.lock().expect("lock");
        let lease: Option<String> = conn
            .query_row(
                "SELECT claimed_by FROM meeting_stt_jobs WHERE id = ?1",
                params![job_id.0],
                |row| row.get(0),
            )
            .expect("lease");
        assert!(lease.is_none(), "leaving running releases the worker lease");
        drop(conn);

        // retry_wait is due again, so the job can be re-claimed.
        let again = store
            .claim_due_stt_job(WorkerId("w2".into()), now)
            .expect("reclaim")
            .expect("due again");
        assert_eq!(again.job.attempt_count, 2);

        store
            .transition_stt_job(job_id.clone(), SttJobState::Succeeded, None)
            .expect("succeed");
        let err = store
            .transition_stt_job(job_id, SttJobState::Queued, None)
            .expect_err("terminal states are final");
        assert!(matches!(err, StoreError::Conflict { .. }), "got {err:?}");
    }

    /// Crash-recovery guarantee (plan §13 Phase 6): re-running a job whose
    /// segments already committed must not duplicate them.
    #[test]
    fn insert_segments_is_idempotent_for_an_already_persisted_job() {
        let (_dir, store) = temp_store();
        let (_meeting_id, _, job_id) = seeded(&store);

        let drafts = |text: &str| {
            vec![NewTranscriptSegment {
                source: "microphone".to_string(),
                start_ms: 0,
                end_ms: 900,
                text: text.to_string(),
                word_timestamps: None,
                confidence: Some(0.5),
                provider: "local".to_string(),
                model_id: Some("parakeet-v3".to_string()),
                sequence: 0,
            }]
        };

        let first = store
            .insert_segments(job_id.clone(), drafts("first attempt"))
            .expect("insert");
        assert_eq!(first.len(), 1);

        // A resumed job produces fresh engine output under the same job id.
        let again = store
            .insert_segments(job_id.clone(), drafts("second attempt"))
            .expect("re-insert");
        assert_eq!(again.len(), 1);
        assert_eq!(
            again[0].envelope.id, first[0].envelope.id,
            "the original row is returned untouched"
        );
        assert_eq!(again[0].text, "first attempt");
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM meeting_transcript_segments"),
            1
        );
    }

    // -- sources (Phase 9) ----------------------------------------------------

    #[test]
    fn repeated_whatsapp_import_creates_no_duplicates() {
        let (_dir, store) = temp_store();
        let draft = NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Team Chat".to_string(),
            parser_version: "wa-v1".to_string(),
            content_hash: "deadbeef".to_string(),
            messages: vec![
                (Some("Alice".to_string()), "Hallo".to_string(), false),
                (
                    Some("Bob".to_string()),
                    "<Medien ausgeschlossen>".to_string(),
                    true,
                ),
            ],
        };

        let (first, created) = store.create_chat_source(draft).expect("import");
        assert!(created);
        let (second, created_again) = store
            .create_chat_source(NewChatSource {
                source_type: "whatsapp_export".to_string(),
                title: "Team Chat (1)".to_string(),
                parser_version: "wa-v1".to_string(),
                content_hash: "deadbeef".to_string(),
                messages: vec![],
            })
            .expect("re-import");
        assert!(!created_again, "same hash must be a no-op");
        assert_eq!(first.id, second.id, "the original source is returned");

        // Distinct content still imports as a new source.
        let (_other, created_other) = store
            .create_chat_source(NewChatSource {
                source_type: "whatsapp_export".to_string(),
                title: "Anders".to_string(),
                parser_version: "wa-v1".to_string(),
                content_hash: "cafebabe".to_string(),
                messages: vec![],
            })
            .expect("other import");
        assert!(created_other);

        let items = store
            .list_source_items(Uuid(first.id.clone()))
            .expect("items");
        assert_eq!(items.len(), 2);
        assert!(items[1].is_media);
        assert_eq!(store.list_sources().expect("sources").len(), 2);
    }

    // -- tasks & snapshots (Phase 10) ------------------------------------------

    #[test]
    fn ai_suggestions_dedupe_and_never_overwrite_manual_edits() {
        let (_dir, store) = temp_store();
        let project = store
            .create_project(NewProject {
                name: "P".into(),
                description: String::new(),
            })
            .expect("project");
        let pid = Uuid(project.id.clone());

        let (first, created) = store
            .suggest_task(NewProjectTask {
                project_id: pid.clone(),
                title: "Protokoll senden".to_string(),
                description: "An alle Teilnehmer".to_string(),
                origin: "ai",
            })
            .expect("suggest");
        assert!(created);
        assert_eq!(first.state, "suggested");

        let edited = store
            .decide_task(
                Uuid(first.id.clone()),
                "edited",
                Some("Protokoll bis Freitag senden".to_string()),
                None,
            )
            .expect("edit");
        assert!(edited.human_edited_at.is_some());

        // A later AI run proposing the SAME task must not duplicate or
        // overwrite the user's edit.
        let (again, created_again) = store
            .suggest_task(NewProjectTask {
                project_id: pid.clone(),
                title: "Protokoll senden".to_string(),
                description: "An alle Teilnehmer".to_string(),
                origin: "ai",
            })
            .expect("re-suggest");
        assert!(!created_again);
        assert_eq!(again.id, first.id);
        assert_eq!(
            again.title, "Protokoll bis Freitag senden",
            "the manual edit survives"
        );

        let manual = store
            .create_manual_task(NewProjectTask {
                project_id: pid.clone(),
                title: "Raum buchen".to_string(),
                description: String::new(),
                origin: "manual",
            })
            .expect("manual task");
        assert_eq!(manual.state, "accepted");
        assert_eq!(store.list_tasks(pid).expect("tasks").len(), 2);
    }

    #[test]
    fn snapshots_are_versioned_append_only() {
        let (_dir, store) = temp_store();
        let project = store
            .create_project(NewProject {
                name: "P".into(),
                description: String::new(),
            })
            .expect("project");
        let pid = Uuid(project.id.clone());

        let v1 = store
            .create_snapshot(
                pid.clone(),
                "Stand 1".into(),
                "pv1".into(),
                Some("m".into()),
                "r1".into(),
            )
            .expect("snapshot 1");
        let v2 = store
            .create_snapshot(
                pid.clone(),
                "Stand 2".into(),
                "pv1".into(),
                Some("m".into()),
                "r2".into(),
            )
            .expect("snapshot 2");
        assert_ne!(v1.id, v2.id, "snapshots are append-only versions");

        let listed = store.list_snapshots(pid).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].summary_markdown, "Stand 2", "newest first");
    }

    fn mark_session_completed(store: &SqliteMeetingStore, id: &str) {
        let conn = store.locked("test_mark_session").expect("lock");
        conn.execute(
            "UPDATE meeting_sessions SET state = 'completed' WHERE id = ?1",
            params![id],
        )
        .expect("mark completed");
    }

    #[test]
    fn project_crud_and_assignment_round_trip() {
        let (_dir, store) = temp_store();
        let project = store
            .create_project(NewProject {
                name: "Alpha".to_string(),
                description: "First".to_string(),
            })
            .expect("project");
        let session = store
            .create_session(draft_session("Sync"))
            .expect("session");

        // Unprocessed drafts never appear in the inbox.
        assert!(store
            .list_unassigned_meeting_ids()
            .expect("inbox")
            .is_empty());
        mark_session_completed(&store, &session.envelope.id.0);
        let project_id = Uuid(project.id.clone());
        let meeting_id = session.envelope.id.clone();

        store
            .assign_meeting_to_project(project_id.clone(), meeting_id.clone())
            .expect("assign");
        // Idempotent re-assignment.
        store
            .assign_meeting_to_project(project_id.clone(), meeting_id.clone())
            .expect("re-assign");

        assert_eq!(
            store
                .list_meeting_ids_for_project(project_id.clone())
                .expect("members"),
            vec![meeting_id.0.clone()]
        );
        assert!(store
            .list_unassigned_meeting_ids()
            .expect("inbox")
            .is_empty());

        store
            .unassign_meeting_from_project(project_id.clone(), meeting_id.clone())
            .expect("unassign");
        assert_eq!(
            store.list_unassigned_meeting_ids().expect("inbox"),
            vec![meeting_id.0.clone()],
            "unassigned meetings return to the Eingang"
        );

        let renamed = store
            .rename_project(project_id.clone(), "  Beta  ".to_string())
            .expect("rename");
        assert_eq!(renamed.name, "Beta");
        assert!(store
            .rename_project(project_id.clone(), "   ".to_string())
            .is_err());

        store.soft_delete_project(project_id).expect("delete");
        assert!(store.list_projects().expect("list").is_empty());
    }

    #[test]
    fn deleted_project_releases_meetings_to_the_inbox() {
        let (_dir, store) = temp_store();
        let project = store
            .create_project(NewProject {
                name: "Temp".into(),
                description: String::new(),
            })
            .expect("project");
        let session = store.create_session(draft_session("Meeting")).expect("s");
        mark_session_completed(&store, &session.envelope.id.0);
        store
            .assign_meeting_to_project(Uuid(project.id.clone()), session.envelope.id.clone())
            .expect("assign");
        assert!(store
            .list_unassigned_meeting_ids()
            .expect("inbox")
            .is_empty());

        store.soft_delete_project(Uuid(project.id)).expect("delete");
        // Unlinking never deletes the meeting (plan §11).
        assert_eq!(
            store.list_unassigned_meeting_ids().expect("inbox"),
            vec![session.envelope.id.0.clone()]
        );
    }

    // -- transcript segments -----------------------------------------------

    #[test]
    fn segments_round_trip_and_come_back_in_chronological_order() {
        let (_dir, store) = temp_store();
        let (meeting_id, _, job_id) = seeded(&store);

        // Deliberately inserted out of order: the store orders by the audio
        // timeline, not by insertion order.
        let drafts = [(3_000_i64, "third"), (1_000, "first"), (2_000, "second")]
            .into_iter()
            .enumerate()
            .map(|(index, (start_ms, text))| NewTranscriptSegment {
                source: "microphone".to_string(),
                start_ms,
                end_ms: start_ms + 900,
                text: text.to_string(),
                word_timestamps: Some(serde_json::json!([{ "w": text, "t": start_ms }])),
                confidence: Some(0.75),
                provider: "local".to_string(),
                model_id: Some("parakeet-v3".to_string()),
                sequence: index as i64,
            })
            .collect::<Vec<_>>();

        let inserted = store
            .insert_segments(job_id.clone(), drafts)
            .expect("insert");
        assert_eq!(inserted.len(), 3);
        assert!(inserted.iter().all(|s| s.envelope.revision == 1));
        assert_eq!(
            inserted[0].meeting_id, meeting_id,
            "meeting comes from the job"
        );

        let read = store
            .segments_for_meeting(meeting_id.clone())
            .expect("segments");
        let texts: Vec<&str> = read.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
        let starts: Vec<i64> = read.iter().map(|s| s.start_ms).collect();
        assert!(
            starts.windows(2).all(|w| w[0] <= w[1]),
            "not sorted: {starts:?}"
        );

        let first = &read[0];
        assert_eq!(first.job_id, job_id);
        assert_eq!(first.confidence, Some(0.75));
        assert_eq!(first.model_id.as_deref(), Some("parakeet-v3"));
        assert_eq!(
            first.word_timestamps,
            Some(serde_json::json!([{ "w": "first", "t": 1000 }]))
        );

        // U8: one log row per inserted segment, all in one transaction.
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM change_log
                 WHERE entity = 'meeting_transcript_segments' AND operation = 'create'"
            ),
            3
        );

        // Unknown job: nothing is written.
        assert!(store
            .insert_segments(Uuid("nope".into()), Vec::new())
            .is_err());
    }

    // -- summaries ---------------------------------------------------------

    #[test]
    fn upsert_summary_inserts_then_updates_the_same_row() {
        let (_dir, store) = temp_store();
        let (meeting_id, _, _) = seeded(&store);

        let draft = NewMeetingSummary {
            meeting_id: meeting_id.clone(),
            provider: "local-llm".to_string(),
            model: Some("qwen3".to_string()),
            prompt_version: "v1".to_string(),
            state: "running".to_string(),
            summary_markdown: "## Draft".to_string(),
            decisions: serde_json::json!([]),
            action_items: serde_json::json!([]),
            open_questions: serde_json::json!([]),
            last_error: None,
        };

        let created = store.upsert_summary(draft.clone()).expect("insert");
        assert_eq!(created.envelope.revision, 1);
        assert_eq!(created.state, "running");

        let finished = store
            .upsert_summary(NewMeetingSummary {
                state: "completed".to_string(),
                summary_markdown: "## Result".to_string(),
                decisions: serde_json::json!([{ "text": "ship it" }]),
                action_items: serde_json::json!([{ "text": "write docs", "owner": "pat" }]),
                open_questions: serde_json::json!(["budget?"]),
                ..draft
            })
            .expect("update");

        assert_eq!(
            finished.envelope.id, created.envelope.id,
            "U9 updates the existing row instead of adding a second one"
        );
        assert_eq!(finished.envelope.revision, 2);
        assert_eq!(finished.summary_markdown, "## Result");
        assert_eq!(
            finished.decisions,
            serde_json::json!([{ "text": "ship it" }])
        );
        assert_eq!(finished.open_questions, serde_json::json!(["budget?"]));
        assert_eq!(finished.created_at, created.created_at);
        assert!(finished.updated_at >= finished.created_at);

        let read = store
            .summary_for_meeting(meeting_id)
            .expect("read")
            .expect("present");
        assert_eq!(read.envelope.id, created.envelope.id);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM meeting_summaries"), 1);
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM change_log WHERE entity = 'meeting_summaries'"
            ),
            2,
            "one create plus one update row"
        );
    }

    #[test]
    fn summary_is_absent_until_written() {
        let (_dir, store) = temp_store();
        let (meeting_id, _, _) = seeded(&store);
        assert!(store
            .summary_for_meeting(meeting_id)
            .expect("read")
            .is_none());
    }

    // -- injected ports ----------------------------------------------------

    /// The store must take ids and timestamps from its ports, so tests (and a
    /// later replay tool) can make writes deterministic (contract §5.1/§5.3).
    #[test]
    fn injected_clock_and_uuid_source_are_honoured() {
        struct FixedClock(DateTime<Utc>);
        impl Clock for FixedClock {
            fn now(&self) -> DateTime<Utc> {
                self.0
            }
        }
        struct FixedUuid;
        impl UuidSource for FixedUuid {
            fn new_uuid(&self) -> Uuid {
                Uuid("22222222-2222-4222-8222-222222222222".to_string())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let fixed = from_ms(1_600_000_000_000);
        let store = SqliteMeetingStore::open_with_ports(
            dir.path().join(MEETING_DB_FILE),
            Arc::new(FixedClock(fixed)),
            Arc::new(FixedUuid),
        )
        .expect("open");

        let created = store
            .create_session(draft_session("Fixed"))
            .expect("create");
        assert_eq!(
            created.envelope.id.0,
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(created.envelope.updated_at, fixed);
        assert_eq!(created.created_at, fixed);
    }

    /// The store is usable as the `Arc<dyn MeetingRepository>` that command
    /// handlers and services will hold (contract §2.1 rule).
    #[test]
    fn store_is_usable_behind_the_trait_object() {
        let (_dir, store) = temp_store();
        let repo: Arc<dyn MeetingRepository> = Arc::new(store);
        let created = repo.create_session(draft_session("Dyn")).expect("create");
        assert_eq!(
            repo.get_session(created.envelope.id)
                .expect("get")
                .expect("present")
                .title,
            "Dyn"
        );
    }
}
