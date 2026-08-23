//! KnowledgeStore repository seam — compile-only scaffold.
//!
//! Transcribed from `docs/meeting-mode-architecture/repository-boundary-contract.md`:
//! §2 (trait seam sketch), §2.1 (trait form), §4.2 (units of work K1–K10) and
//! §5 (change-envelope semantics). Entity fields follow
//! `docs/meeting-mode-plan.md` §9, lines 1219–1325; the planned home is plan
//! §5 lines 269–275. This file owns ownership-matrix rows 6–17 (`projects`,
//! `sources`, `project_sources`, `source_items`, `assets` metadata,
//! `source_item_assets`, `project_tasks`, `task_evidence`,
//! `project_snapshots` + `snapshot_evidence`, `processing_jobs`,
//! `change_log`, FTS5 search index; contract §3).
//!
//! Scaffold status (task-0134):
//!
//! - Traits and entity types only. There is **no SQL**, no concrete
//!   `KnowledgeStore` struct, no constructor and no manager wiring.
//! - Asset **bytes** are out of scope here entirely: metadata rows live in
//!   this store's future tables, byte placement goes through `AssetStore`
//!   (`src-tauri/src/storage/asset_store.rs`, rule R4 / note N3).
//! - [`ChangeLogEntry`] is parked here temporarily; ownership-matrix row 16
//!   designates a shared append-only helper at
//!   `src-tauri/src/managers/change_log.rs` as its home once scaffolded.
//! - Places whose exact shape depends on the pending schema authority
//!   (task-0020 / `db-schema.md`) carry *parity notes* instead of guessing DDL.
//!
//! Enforcement: the `const` assertions near the bottom
//! fail the build if any type crossing the seam stops being `Send + Sync`;
//! `rusqlite::Connection` is `!Sync`, so no seam type can embed a database
//! connection.

// Compile-only scaffold: consumers (ProjectManager, SourceIngestionService,
// ProjectIntelligenceService) arrive with the implementation task, so every
// entity field and trait method is intentionally unused for now. Remove this
// allow when the concrete store is wired in lib.rs.
#![allow(dead_code)]

use chrono::{DateTime, Utc};

use super::meeting_store::{ChangeEnvelope, StoreError, Uuid, WorkerId};

// ---------------------------------------------------------------------------
// Enums fixed by plan §9 value sets
// ---------------------------------------------------------------------------

/// Project lifecycle state; plan line 1222 (`active`, `archived`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    /// Visible, active project.
    Active,
    /// Archived, read-mostly project.
    Archived,
}

/// Generic origin record type; plan lines 1229–1230. Later types (`email`,
/// `whatsapp_api`) are roadmap and extend this enum then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceType {
    /// A captured meeting session (note N1 linkage).
    Meeting,
    /// An imported WhatsApp chat export.
    WhatsAppExport,
    /// A manually written note.
    ManualNote,
    /// An arbitrary imported file.
    File,
}

/// Assignment origin of a project↔source link; plan line 1245.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOrigin {
    /// Set by the user.
    Manual,
    /// Proposed by AI, pending confirmation.
    AiSuggested,
    /// Assigned by an import flow.
    ImportDefault,
}

/// Task lifecycle states in v1; plan line 1280–1281. Later execution phases
/// add `in_progress`, `waiting_for_approval`, `failed`, `cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskState {
    /// AI-suggested, not yet published as open.
    Suggested,
    /// Open task.
    Open,
    /// Completed.
    Done,
    /// Rejected without completion.
    Dismissed,
}

/// Task priority ladder; plan line 1282.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Low priority.
    Low,
    /// Normal priority (default).
    Normal,
    /// High priority.
    High,
    /// Urgent priority.
    Urgent,
}

/// Who owns/acts on a task; plan line 1284. v1 UI uses only
/// `Unassigned` and `Human`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnerType {
    /// No owner yet.
    Unassigned,
    /// A human owner.
    Human,
    /// Later execution phase.
    Assistant,
}

/// Whether a human or the AI authored a task; plan line 1286. Human edits win
/// over AI output (plan lines 264–265, K7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Authored or edited by a human.
    Human,
    /// Proposed by AI.
    Ai,
}

/// Actor of a task transition ([`KnowledgeRepository::transition_task`], K7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// A human acted.
    Human,
    /// The AI pipeline acted (never overwrites manual status notes, plan line 1224).
    Ai,
}

/// Processing-job states mirror the STT job machine; plan lines 1316–1317.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingJobState {
    /// Waiting for a worker.
    Queued,
    /// Claimed and running.
    Running,
    /// Backing off before retry.
    RetryWait,
    /// Finished successfully.
    Succeeded,
    /// Terminally failed.
    Failed,
    /// Cancelled.
    Cancelled,
}

/// Terminal report of [`KnowledgeRepository::finish_processing_job`] (K10,
/// tier B — no change_log, W6).
#[derive(Debug, Clone)]
pub enum JobOutcome {
    /// Job completed successfully.
    Succeeded,
    /// Job failed terminally with a reason (never secrets).
    Failed {
        /// Failure reason text.
        reason: String,
    },
}

/// Operation recorded per change-log row; contract §4 W2/W3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOperation {
    /// Row created (revision 1).
    Create,
    /// Row mutated (revision bumped).
    Update,
    /// Soft-delete tombstone written (W3).
    Tombstone,
}

// ---------------------------------------------------------------------------
// Entity types — ownership-matrix rows 6–15 (+ row 16 type, parked)
// ---------------------------------------------------------------------------

/// Row 6 — `projects` (plan lines 1219–1225; tier A, K1–K3).
#[derive(Debug, Clone)]
pub struct Project {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Project name (searched by FTS, plan line 1329).
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional display color.
    pub color: Option<String>,
    /// Lifecycle state.
    pub state: ProjectState,
    /// Manually maintained status note. **AI outputs must never overwrite
    /// it** (plan lines 1224–1225; enforced in K2/K2' implementations).
    pub manual_status_note: Option<String>,
    /// Set once at creation, never modified afterwards (§5.3).
    pub created_at: DateTime<Utc>,
}

/// Row 7 — `sources` (plan lines 1227–1239; tier A, K4/K5).
#[derive(Debug, Clone)]
pub struct Source {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Origin kind (plan line 1230).
    pub source_type: SourceType,
    /// Display title.
    pub title: String,
    /// When the source content occurred (meeting start, message date, …).
    pub occurred_at: DateTime<Utc>,
    /// End of an optional occurrence period (plan line 1232).
    pub period_end: Option<DateTime<Utc>>,
    /// Ingestion pipeline position (parity note: value set pending task-0020 /
    /// ingestion-pipeline-contract alignment).
    pub ingestion_state: String,
    /// Parser version for reproducible re-imports (plan line 1154).
    pub parser_version: String,
    /// SHA-256 over canonical content serialization, lowercase hex (§5.5);
    /// deduplication basis (plan line 1262).
    pub content_hash: String,
    /// Asset holding raw import bytes (metadata id + relative key live in
    /// [`AssetMetadata`]; bytes via AssetStore, N3).
    pub raw_asset_id: Option<Uuid>,
    /// Import and privacy metadata (plan line 1234; shape pending).
    pub import_metadata: serde_json::Value,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Row 8 — `project_sources` (plan lines 1241–1251; tier A, K6). Deliberate
/// many-to-many link; unique on (project_id, source_id) per plan line 1247.
#[derive(Debug, Clone)]
pub struct ProjectSourceLink {
    /// Linked project.
    pub project_id: Uuid,
    /// Linked source.
    pub source_id: Uuid,
    /// Marks the v1-visible primary assignment (plan line 1244).
    pub is_primary: bool,
    /// How the link came into existence.
    pub origin: LinkOrigin,
    /// AI confidence, if AI-proposed.
    pub confidence: Option<f32>,
    /// When a human confirmed an AI suggestion.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Creation timestamp (plan line 1246).
    pub created_at: DateTime<Utc>,
}

/// Row 9 — `source_items` (plan lines 1253–1266; tier A, written inside K4).
/// Normalized addressable units: chat messages, transcript-segment
/// projections (N2), document sections, notes.
#[derive(Debug, Clone)]
pub struct SourceItem {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Owning source.
    pub source_id: Uuid,
    /// Item kind label (chat message, segment, section, note; parity note:
    /// value registry pending identifier-registry/task-0020).
    pub item_type: String,
    /// External dedup key within the source (plan line 1258).
    pub external_key: Option<String>,
    /// Display author label.
    pub author_label: Option<String>,
    /// Occurrence instant used for ordering.
    pub occurred_at: DateTime<Utc>,
    /// Ordering sequence within the source.
    pub sequence: i64,
    /// Normalized text content.
    pub text: String,
    /// Structured metadata (plan line 1260).
    pub metadata: serde_json::Value,
    /// SHA-256 over canonical content serialization (§5.5); unique-dedup
    /// constraint basis (plan line 1262).
    pub content_hash: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Row 10 — `assets` metadata rows (plan lines 1270–1271; tier A). SQLite
/// holds metadata only — binary bytes live in the AssetStore keyed by asset
/// id and relative storage key, never absolute paths (N3, plan lines
/// 1158–1159, 1274–1275). No BLOBs (plan line 1274).
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Relative storage key inside the AssetStore root.
    pub storage_key: String,
    /// MIME type.
    pub mime_type: String,
    /// Stored file size in bytes.
    pub file_size: i64,
    /// SHA-256 over the exact stored bytes (matches AssetStore hashing duty,
    /// plan line 1547), lowercase hex.
    pub hash: String,
    /// Original upload/import filename.
    pub original_filename: Option<String>,
    /// Image width if detected.
    pub width: Option<u32>,
    /// Image height if detected.
    pub height: Option<u32>,
    /// Media duration if detected, milliseconds.
    pub duration_ms: Option<i64>,
    /// Media-analysis progress (parity note: value set pending).
    pub processing_state: String,
    /// Retention rule applied (parity note: policy value set pending;
    /// retention itself is a separate concern, plan lines 1468 ff.).
    pub retention_policy: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Row 11 — `source_item_assets` (plan lines 1272–1273; tier A, written
/// inside K4): maps a medium/document asset to one or more normalized items.
#[derive(Debug, Clone)]
pub struct SourceItemAsset {
    /// Normalized item owning the attachment.
    pub source_item_id: Uuid,
    /// Referenced asset (metadata row; bytes via AssetStore).
    pub asset_id: Uuid,
}

/// Row 12 — `project_tasks` (plan lines 1277–1288; tier A, K7).
#[derive(Debug, Clone)]
pub struct ProjectTask {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Owning project.
    pub project_id: Uuid,
    /// Task title.
    pub title: String,
    /// Optional description.
    pub description: Option<String>,
    /// Lifecycle state (v1 value set).
    pub state: TaskState,
    /// Priority.
    pub priority: Priority,
    /// Optional due date.
    pub due_date: Option<DateTime<Utc>>,
    /// Optional responsible party label.
    pub assignee: Option<String>,
    /// Owner classification.
    pub owner_type: OwnerType,
    /// Authorship origin; drives the human-wins merge rule (plan lines 264–265).
    pub origin: Origin,
    /// AI confidence, if AI-originated.
    pub confidence: Option<f32>,
    /// Producing model, if AI-originated.
    pub model: Option<String>,
    /// Prompt version, if AI-originated.
    pub prompt_version: Option<String>,
    /// Last human edit (human edits outrank AI runs, plan lines 264–265).
    pub human_edited_at: Option<DateTime<Utc>>,
    /// Completion timestamp.
    pub completed_at: Option<DateTime<Utc>>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp (parity note: refreshed together with the
    /// envelope's revision bump, §5.3).
    pub updated_at: DateTime<Utc>,
}

/// Row 13 — `task_evidence` (plan lines 1295–1301; tier A, K8). An AI task
/// without ≥1 valid evidence is never published (enforced before K7
/// publishes AI-origin tasks).
#[derive(Debug, Clone)]
pub struct TaskEvidence {
    /// Evidenced task.
    pub task_id: Uuid,
    /// Source backing the claim.
    pub source_id: Uuid,
    /// Optional normalized item cited.
    pub source_item_id: Option<Uuid>,
    /// Optional transcript segment cited (stable reference across stores,
    /// plan line 1297 / note N2).
    pub meeting_transcript_segment_id: Option<Uuid>,
    /// Optional evidence period start.
    pub range_start: Option<DateTime<Utc>>,
    /// Optional evidence period end.
    pub range_end: Option<DateTime<Utc>>,
    /// Short supporting excerpt.
    pub excerpt: Option<String>,
    /// Why this supports the task.
    pub rationale: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Row 14a — `project_snapshots` (plan lines 1303–1311; tier A, K9).
/// Versioned project status; exactly one snapshot is `current`.
#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    /// Envelope: id, revision, updated_at, deleted_at (§5).
    pub envelope: ChangeEnvelope,
    /// Snapshot project.
    pub project_id: Uuid,
    /// Whether this is the current snapshot; publishing demotes the previous
    /// one in the same transaction (plan lines 1310–1311).
    pub is_current: bool,
    /// Status summary markdown.
    pub summary_markdown: String,
    /// Decisions captured in this snapshot.
    pub decisions: serde_json::Value,
    /// Risks captured in this snapshot.
    pub risks: serde_json::Value,
    /// Open questions captured in this snapshot.
    pub open_questions: serde_json::Value,
    /// Data horizon covered by the analysis (plan line 1307).
    pub covered_through: DateTime<Utc>,
    /// Participating sources with their revisions (plan line 1307).
    pub covered_source_revisions: Vec<SourceRevision>,
    /// Analysis provider.
    pub provider: String,
    /// Analysis model.
    pub model: Option<String>,
    /// Prompt version.
    pub prompt_version: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Participating-source revision marker embedded in snapshots (plan line 1307).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRevision {
    /// Participating source.
    pub source_id: Uuid,
    /// Source revision observed when the snapshot was produced.
    pub revision: i64,
}

/// Row 14b — `snapshot_evidence` (plan lines 1309–1310; tier A, written in
/// the same transaction as the snapshot, K9): links individual statements to
/// sources or source items.
#[derive(Debug, Clone)]
pub struct SnapshotEvidence {
    /// Owning snapshot.
    pub snapshot_id: Uuid,
    /// Statement anchor inside the snapshot payload (parity note: anchor
    /// scheme pending summary-evidence-contract alignment).
    pub statement_anchor: String,
    /// Backing source.
    pub source_id: Uuid,
    /// Backing normalized item, if item-level.
    pub source_item_id: Option<Uuid>,
    /// Short excerpt proving the statement.
    pub excerpt: Option<String>,
    /// Explanation why this backs the statement.
    pub rationale: String,
}

/// Row 15 — `processing_jobs` (plan lines 1313–1318; tier B, K10 — no
/// envelope, no change_log, W6). Generic persistent jobs for import, media
/// analysis, source summaries, project snapshots and later connector syncs.
#[derive(Debug, Clone)]
pub struct ProcessingJob {
    /// Stable job id (hard deletes permitted once finished, §5.4/W6).
    pub id: Uuid,
    /// Idempotency key preventing duplicate results (plan line 1318).
    pub idempotency_key: String,
    /// Job kind label (import/media_analysis/source_summary/project_snapshot/
    /// connector_sync; parity note: registry pending task-0020).
    pub kind: String,
    /// Kind-specific payload.
    pub payload: serde_json::Value,
    /// State/retry semantics mirror `meeting_stt_jobs` (plan line 1317).
    pub state: ProcessingJobState,
    /// Attempts made so far.
    pub attempt_count: u32,
    /// Earliest next attempt instant.
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Last error text, never secrets.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Row 16 — `change_log` entry type (plan lines 1320–1325; tier C).
///
/// Temporary home: ownership-matrix row 16 places the append-only writer
/// helper at `src-tauri/src/managers/change_log.rs`; that helper consumes
/// this type (or relocates it) once scaffolded. Append-only; written only by
/// the stores inside tier-A units of work (W2/W3); local ordering comes from
/// the SQLite rowid, not from these fields (N5).
#[derive(Debug, Clone)]
pub struct ChangeLogEntry {
    /// Entity table name addressed (e.g. `"projects"`).
    pub entity: &'static str,
    /// Stable UUID of the changed row.
    pub entity_id: Uuid,
    /// Revision after the mutation.
    pub revision: i64,
    /// Mutation timestamp (RFC3339 UTC, ms precision).
    pub occurred_at: DateTime<Utc>,
    /// Operation class (create/update/tombstone).
    pub operation: ChangeOperation,
}

// ---------------------------------------------------------------------------
// Search (row 17 — FTS5 derived index; reads only above the seam)
// ---------------------------------------------------------------------------

/// Scope limiter for [`KnowledgeRepository::search`]; FTS indexes project
/// names, source items, transcripts, tasks and snapshots (plan line 1329).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchScope {
    /// Search everything visible to the user.
    All,
    /// Restrict hits to one project's linked content.
    WithinProject(Uuid),
}

/// Which indexed corpus a hit came from (plan line 1329).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHitKind {
    /// Project name hit.
    Project,
    /// Source-item hit.
    SourceItem,
    /// Transcript hit (via the stable segment→`source_item` projection, N2).
    TranscriptSegment,
    /// Task hit.
    Task,
    /// Snapshot hit.
    Snapshot,
}

/// Row 17 projection — search result. Every hit carries entity id AND badge
/// position (plan lines 1330–1331); embedding/vector search stays behind a
/// future `SearchService` seam and is not preempted (plan lines 1331–1332).
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Hit corpus.
    pub kind: SearchHitKind,
    /// Stable entity id of the hit.
    pub entity_id: Uuid,
    /// Item-level id when the hit points into a specific item/segment.
    pub item_id: Option<Uuid>,
    /// Match start offset within the hit text (badge position).
    pub start_offset: u32,
    /// Match end offset within the hit text (badge position).
    pub end_offset: u32,
    /// Renderable snippet around the match.
    pub snippet: String,
}

// ---------------------------------------------------------------------------
// Drafts and payloads (parameters of units of work K1–K10)
// ---------------------------------------------------------------------------

/// Draft for [`KnowledgeRepository::create_project`] (K1).
#[derive(Debug, Clone)]
pub struct NewProject {
    /// Initial name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional color.
    pub color: Option<String>,
}

/// Partial update for [`KnowledgeRepository::update_project`] (K2).
/// `None` fields leave values unchanged; revision/timestamp handling is the
/// store's (W4, §5.3).
#[derive(Debug, Default, Clone)]
pub struct ProjectPatch {
    /// New name.
    pub name: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New color.
    pub color: Option<String>,
    /// Manual status note — writable by humans only; AI paths must pass
    /// `None` (plan lines 1224–1225).
    pub manual_status_note: Option<String>,
}

/// Single unit of work for `SourceIngestionService.commit()` (K4): source
/// row, normalized items, asset metadata rows and item–asset links commit in
/// ONE transaction with one change_log row each; any failure rolls back the
/// whole unit. Byte placement must have succeeded beforehand (R4/N3).
#[derive(Debug, Clone)]
pub struct SourceCommit {
    /// Source header draft.
    pub source: NewSource,
    /// Normalized items belonging to the source.
    pub items: Vec<NewSourceItem>,
    /// Asset metadata rows whose bytes already sit in the AssetStore.
    pub assets: Vec<NewAssetMetadata>,
    /// Item↔asset links to insert.
    pub links: Vec<NewSourceItemAsset>,
}

/// Source header draft (row 7) without store-assigned fields.
#[derive(Debug, Clone)]
pub struct NewSource {
    /// Origin kind.
    pub source_type: SourceType,
    /// Display title.
    pub title: String,
    /// Occurrence instant.
    pub occurred_at: DateTime<Utc>,
    /// Optional period end.
    pub period_end: Option<DateTime<Utc>>,
    /// Ingestion state label.
    pub ingestion_state: String,
    /// Parser version.
    pub parser_version: String,
    /// Content hash (§5.5).
    pub content_hash: String,
    /// Raw-bytes asset id, if imported raw material exists.
    pub raw_asset_id: Option<Uuid>,
    /// Import/privacy metadata.
    pub import_metadata: serde_json::Value,
}

/// Result of [`KnowledgeRepository::commit_source`] (K4): everything the unit
/// persisted, with store-assigned ids/envelopes filled in.
#[derive(Debug, Clone)]
pub struct CommittedSource {
    /// Persisted source header.
    pub source: Source,
    /// Persisted items, order preserved from the commit payload.
    pub items: Vec<SourceItem>,
    /// Persisted asset metadata rows.
    pub assets: Vec<AssetMetadata>,
    /// Persisted links.
    pub links: Vec<SourceItemAsset>,
}

/// Normalized item draft (row 9) without store-assigned fields.
#[derive(Debug, Clone)]
pub struct NewSourceItem {
    /// Item kind label.
    pub item_type: String,
    /// External dedup key.
    pub external_key: Option<String>,
    /// Author label.
    pub author_label: Option<String>,
    /// Occurrence instant.
    pub occurred_at: DateTime<Utc>,
    /// Sequence within the source.
    pub sequence: i64,
    /// Text content.
    pub text: String,
    /// Structured metadata.
    pub metadata: serde_json::Value,
    /// Content hash (§5.5).
    pub content_hash: String,
}

/// Asset metadata draft (row 10) without store-assigned fields; bytes must
/// already be placed via AssetStore before K4 commits (R4/N3).
#[derive(Debug, Clone)]
pub struct NewAssetMetadata {
    /// Relative storage key.
    pub storage_key: String,
    /// MIME type.
    pub mime_type: String,
    /// File size in bytes.
    pub file_size: i64,
    /// SHA-256 over the stored bytes (lowercase hex).
    pub hash: String,
    /// Original filename.
    pub original_filename: Option<String>,
    /// Width if detected.
    pub width: Option<u32>,
    /// Height if detected.
    pub height: Option<u32>,
    /// Duration if detected, milliseconds.
    pub duration_ms: Option<i64>,
    /// Processing-state label.
    pub processing_state: String,
    /// Retention rule label.
    pub retention_policy: String,
}

/// Link draft (row 11) for [`SourceCommit`].
#[derive(Debug, Clone)]
pub struct NewSourceItemAsset {
    /// Owning item placeholder index into [`SourceCommit::items`]; resolved to
    /// a real id by the store inside the K4 transaction (ids do not exist
    /// before persist).
    pub item_index: usize,
    /// Referenced asset metadata index into [`SourceCommit::assets`].
    pub asset_index: usize,
}

/// Upsert payload for [`KnowledgeRepository::upsert_task`] (K7); `None` id
/// creates a new task, `Some` updates the existing one.
#[derive(Debug, Clone)]
pub struct TaskUpsert {
    /// Existing task id, or `None` to create.
    pub id: Option<Uuid>,
    /// Owning project.
    pub project_id: Uuid,
    /// Title.
    pub title: String,
    /// Description.
    pub description: Option<String>,
    /// Priority.
    pub priority: Priority,
    /// Due date.
    pub due_date: Option<DateTime<Utc>>,
    /// Assignee label.
    pub assignee: Option<String>,
}

/// Evidence draft for [`KnowledgeRepository::attach_task_evidence`] (K8).
#[derive(Debug, Clone)]
pub struct NewTaskEvidence {
    /// Evidenced task.
    pub task_id: Uuid,
    /// Backing source.
    pub source_id: Uuid,
    /// Backing item, if item-level.
    pub source_item_id: Option<Uuid>,
    /// Backing transcript segment, if segment-level.
    pub meeting_transcript_segment_id: Option<Uuid>,
    /// Evidence period start.
    pub range_start: Option<DateTime<Utc>>,
    /// Evidence period end.
    pub range_end: Option<DateTime<Utc>>,
    /// Supporting excerpt.
    pub excerpt: Option<String>,
    /// Rationale.
    pub rationale: String,
}

/// Publish payload for [`KnowledgeRepository::publish_snapshot`] (K9):
/// snapshot + evidence rows + previous-`current` demotion happen in ONE
/// transaction (plan lines 1303–1311).
#[derive(Debug, Clone)]
pub struct SnapshotPublish {
    /// Snapshot project.
    pub project_id: Uuid,
    /// Status markdown.
    pub summary_markdown: String,
    /// Decisions.
    pub decisions: serde_json::Value,
    /// Risks.
    pub risks: serde_json::Value,
    /// Open questions.
    pub open_questions: serde_json::Value,
    /// Coverage horizon.
    pub covered_through: DateTime<Utc>,
    /// Participating source revisions.
    pub covered_source_revisions: Vec<SourceRevision>,
    /// Analysis provider.
    pub provider: String,
    /// Analysis model.
    pub model: Option<String>,
    /// Prompt version.
    pub prompt_version: String,
    /// Evidence rows to attach.
    pub evidence: Vec<NewSnapshotEvidence>,
}

/// Evidence draft inside [`SnapshotPublish`].
#[derive(Debug, Clone)]
pub struct NewSnapshotEvidence {
    /// Statement anchor within the published snapshot.
    pub statement_anchor: String,
    /// Backing source.
    pub source_id: Uuid,
    /// Backing item, if item-level.
    pub source_item_id: Option<Uuid>,
    /// Excerpt.
    pub excerpt: Option<String>,
    /// Rationale.
    pub rationale: String,
}

/// Lease handed out by [`KnowledgeRepository::claim_processing_job`] (K10,
/// tier B — no change_log, W6).
#[derive(Debug, Clone)]
pub struct ProcessingJobClaim {
    /// Claimed job with post-claim state.
    pub job: ProcessingJob,
    /// Claiming worker.
    pub worker: WorkerId,
    /// Claim instant (lease semantics follow STT-style rules, plan line 1317).
    pub claimed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Trait seam — contract §2 KnowledgeStore block mirrored in §2.1 trait form
// ---------------------------------------------------------------------------

/// Repository seam for projects, sources, normalized content, assets
/// metadata, tasks, snapshots, processing jobs and change revisions.
///
/// Consumed by `ProjectManager`, `SourceIngestionService` and
/// `ProjectIntelligenceService` (contract §6) as
/// `Arc<dyn KnowledgeRepository>`.
///
/// Contract sources: §2 (sketch), §2.1 (trait form), §4.2 (units of work
/// K1–K10). Every write member is one unit of work with an internal
/// transaction (W1); tier-A members append same-transaction change-log rows
/// (W2) and tombstones on delete (W3); tier-B members (K10) write no log
/// (W6). Reads return owned typed values; search reads the tier-D FTS index
/// and never writes it independently (W5).
///
/// Enforcement: object-safe by construction, and no signature type can embed
/// a `rusqlite::Connection` — see the `const` assertions below.
pub trait KnowledgeRepository: Send + Sync {
    // Projects ------------------------------------------------------------
    /// K1 — INSERT `projects` (rev = 1) + `change_log`(create).
    fn create_project(&self, draft: NewProject) -> Result<Project, StoreError>;
    /// K2 — UPDATE + rev++ + `change_log`(update).
    fn update_project(&self, id: Uuid, patch: ProjectPatch) -> Result<Project, StoreError>;
    /// K2' — archive transition incl. status-note protection (AI never
    /// overwrites the manual note, plan lines 1224–1225).
    fn archive_project(&self, id: Uuid) -> Result<Project, StoreError>;
    /// K3 — tombstone per W3 + `change_log`(tombstone).
    fn soft_delete_project(&self, id: Uuid) -> Result<(), StoreError>;

    // Sources & ingestion commit -------------------------------------------
    /// K4 — single-transaction ingestion commit (see [`SourceCommit`]).
    fn commit_source(&self, payload: SourceCommit) -> Result<CommittedSource, StoreError>;
    /// K5 — soft delete on `sources` with cascade decision recorded + log rows.
    fn soft_delete_source(&self, id: Uuid) -> Result<(), StoreError>;
    /// K6 — INSERT/UPDATE `project_sources` + rev semantics + `change_log`.
    fn link_project_source(&self, link: ProjectSourceLink) -> Result<(), StoreError>;
    /// K6 — remove a project↔source link (log-recorded, W2).
    fn unlink_project_source(&self, project_id: Uuid, source_id: Uuid) -> Result<(), StoreError>;

    // Tasks ----------------------------------------------------------------
    /// K7 — INSERT/UPDATE `project_tasks` + rev++ + `change_log`; human edits
    /// win over AI (plan lines 264–265).
    fn upsert_task(&self, task: TaskUpsert) -> Result<ProjectTask, StoreError>;
    /// K7 — state-machine transition with actor attribution.
    fn transition_task(
        &self,
        id: Uuid,
        to: TaskState,
        actor: Actor,
    ) -> Result<ProjectTask, StoreError>;
    /// K8 — INSERT `task_evidence` + `change_log`; publication gate for
    /// AI-origin tasks (plan lines 1300–1301).
    fn attach_task_evidence(&self, ev: NewTaskEvidence) -> Result<(), StoreError>;

    // Snapshots --------------------------------------------------------------
    /// K9 — publish snapshot + evidence and demote the previous `current` in
    /// ONE transaction (plan lines 1310–1311).
    fn publish_snapshot(&self, snap: SnapshotPublish) -> Result<ProjectSnapshot, StoreError>;
    /// Read-only: the current snapshot of a project, if any.
    fn current_snapshot(&self, project_id: Uuid) -> Result<Option<ProjectSnapshot>, StoreError>;

    // Processing jobs (operational, no change_log — §4 tier B) ----------------
    /// K10 — atomically claim the next due processing job (tier B, no log).
    fn claim_processing_job(
        &self,
        worker: WorkerId,
        now: DateTime<Utc>,
    ) -> Result<Option<ProcessingJobClaim>, StoreError>;
    /// K10 — record a terminal outcome (tier B, no log).
    fn finish_processing_job(&self, id: Uuid, outcome: JobOutcome) -> Result<(), StoreError>;

    // Reads (hits always carry entity id + position, plan lines 1329–1331)
    /// Full-text search over the tier-D FTS index (read-only).
    fn search(&self, q: &str, scope: SearchScope) -> Result<Vec<SearchHit>, StoreError>;
}

// ---------------------------------------------------------------------------
// Compile-time seam enforcement
// ---------------------------------------------------------------------------

/// See the twin helper in `super::meeting_store` for the rationale: any seam
/// type embedding a `rusqlite::Connection` (`!Sync`) fails these checks.
const fn assert_send_sync<T: Send + Sync + ?Sized>() {}

// Checked on every build of this module — not just under `cargo test`.
const _: () = {
    let _ = assert_send_sync::<dyn KnowledgeRepository>;
    let _ = assert_send_sync::<ProjectState>;
    let _ = assert_send_sync::<SourceType>;
    let _ = assert_send_sync::<LinkOrigin>;
    let _ = assert_send_sync::<TaskState>;
    let _ = assert_send_sync::<Priority>;
    let _ = assert_send_sync::<OwnerType>;
    let _ = assert_send_sync::<Origin>;
    let _ = assert_send_sync::<Actor>;
    let _ = assert_send_sync::<ProcessingJobState>;
    let _ = assert_send_sync::<JobOutcome>;
    let _ = assert_send_sync::<ChangeOperation>;
    let _ = assert_send_sync::<Project>;
    let _ = assert_send_sync::<Source>;
    let _ = assert_send_sync::<ProjectSourceLink>;
    let _ = assert_send_sync::<SourceItem>;
    let _ = assert_send_sync::<AssetMetadata>;
    let _ = assert_send_sync::<SourceItemAsset>;
    let _ = assert_send_sync::<ProjectTask>;
    let _ = assert_send_sync::<TaskEvidence>;
    let _ = assert_send_sync::<ProjectSnapshot>;
    let _ = assert_send_sync::<SourceRevision>;
    let _ = assert_send_sync::<SnapshotEvidence>;
    let _ = assert_send_sync::<ProcessingJob>;
    let _ = assert_send_sync::<ChangeLogEntry>;
    let _ = assert_send_sync::<SearchScope>;
    let _ = assert_send_sync::<SearchHitKind>;
    let _ = assert_send_sync::<SearchHit>;
    let _ = assert_send_sync::<NewProject>;
    let _ = assert_send_sync::<ProjectPatch>;
    let _ = assert_send_sync::<SourceCommit>;
    let _ = assert_send_sync::<NewSource>;
    let _ = assert_send_sync::<CommittedSource>;
    let _ = assert_send_sync::<NewSourceItem>;
    let _ = assert_send_sync::<NewAssetMetadata>;
    let _ = assert_send_sync::<NewSourceItemAsset>;
    let _ = assert_send_sync::<TaskUpsert>;
    let _ = assert_send_sync::<NewTaskEvidence>;
    let _ = assert_send_sync::<SnapshotPublish>;
    let _ = assert_send_sync::<NewSnapshotEvidence>;
    let _ = assert_send_sync::<ProcessingJobClaim>;
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Domain-service stand-in proving this seam is implementable **without
    /// any database** (contract §2.1 motivation).
    struct FakeKnowledgeRepository;

    #[allow(unused_variables)]
    impl KnowledgeRepository for FakeKnowledgeRepository {
        fn create_project(&self, draft: NewProject) -> Result<Project, StoreError> {
            unimplemented!("fake")
        }
        fn update_project(&self, id: Uuid, patch: ProjectPatch) -> Result<Project, StoreError> {
            unimplemented!("fake")
        }
        fn archive_project(&self, id: Uuid) -> Result<Project, StoreError> {
            unimplemented!("fake")
        }
        fn soft_delete_project(&self, id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn commit_source(&self, payload: SourceCommit) -> Result<CommittedSource, StoreError> {
            unimplemented!("fake")
        }
        fn soft_delete_source(&self, id: Uuid) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn link_project_source(&self, link: ProjectSourceLink) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn unlink_project_source(
            &self,
            project_id: Uuid,
            source_id: Uuid,
        ) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn upsert_task(&self, task: TaskUpsert) -> Result<ProjectTask, StoreError> {
            unimplemented!("fake")
        }
        fn transition_task(
            &self,
            id: Uuid,
            to: TaskState,
            actor: Actor,
        ) -> Result<ProjectTask, StoreError> {
            unimplemented!("fake")
        }
        fn attach_task_evidence(&self, ev: NewTaskEvidence) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn publish_snapshot(&self, snap: SnapshotPublish) -> Result<ProjectSnapshot, StoreError> {
            unimplemented!("fake")
        }
        fn current_snapshot(
            &self,
            project_id: Uuid,
        ) -> Result<Option<ProjectSnapshot>, StoreError> {
            unimplemented!("fake")
        }
        fn claim_processing_job(
            &self,
            worker: WorkerId,
            now: DateTime<Utc>,
        ) -> Result<Option<ProcessingJobClaim>, StoreError> {
            unimplemented!("fake")
        }
        fn finish_processing_job(&self, id: Uuid, outcome: JobOutcome) -> Result<(), StoreError> {
            unimplemented!("fake")
        }
        fn search(&self, q: &str, scope: SearchScope) -> Result<Vec<SearchHit>, StoreError> {
            unimplemented!("fake")
        }
    }

    /// Object safety: usable as `Arc<dyn KnowledgeRepository>` (contract §2.1).
    #[test]
    fn seam_is_object_safe_and_fake_substitutable_without_a_database() {
        let repo: Arc<dyn KnowledgeRepository> = Arc::new(FakeKnowledgeRepository);
        let moved = Arc::clone(&repo);
        let _ = (repo, moved);
    }

    /// Cross-store references stay UUID-only: neither store parses or
    /// duplicates partner fields (contract §3 preamble; note N1).
    #[test]
    fn cross_store_references_are_uuid_only() {
        let link = ProjectSourceLink {
            project_id: Uuid("00000000-0000-4000-8000-00000000000a".to_string()),
            source_id: Uuid("00000000-0000-4000-8000-00000000000b".to_string()),
            is_primary: true,
            origin: LinkOrigin::Manual,
            confidence: None,
            confirmed_at: None,
            created_at: Utc::now(),
        };
        let evidence = NewTaskEvidence {
            task_id: Uuid("00000000-0000-4000-8000-00000000000c".to_string()),
            source_id: link.source_id.clone(),
            meeting_transcript_segment_id: Some(Uuid(
                "00000000-0000-4000-8000-00000000000d".to_string(),
            )),
            source_item_id: None,
            range_start: None,
            range_end: None,
            excerpt: None,
            rationale: "cites the meeting decision verbatim".to_string(),
        };
        assert_eq!(evidence.source_id, link.source_id);
    }
}
