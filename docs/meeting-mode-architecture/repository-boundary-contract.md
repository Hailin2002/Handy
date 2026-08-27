# Repository Boundary Contract — MeetingStore / KnowledgeStore

Status: **spec contract — no implementation yet** (Phase 3 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 ("Zielarchitektur", repository
layer at plan lines 205–217; `MeetingStore` at plan lines 238–243;
`KnowledgeStore` at plan lines 269–275; `AssetStore` at plan lines 301–308),
§9 ("Datenmodell", plan lines 1143–1336), §13 Phase 3 tasks and acceptance
(plan lines 1546–1558, especially line 1556: "kein Frontendcode greift direkt
auf SQLite oder Assetdateien zu"), Zukunftsroadmap "Spätere Server- und
Sync-Grenze" (plan lines 1758–1766) and "Späterer MCP-Zugriff" (plan lines
1768–1783).
Planned implementation homes (per plan §5):

- `src-tauri/src/managers/meeting_store.rs` — `MeetingStore`
- `src-tauri/src/managers/knowledge_store.rs` — `KnowledgeStore`
- `src-tauri/src/storage/asset_store.rs` — `AssetStore` (binary bytes only;
  this contract constrains how stores call it, it does not respecify it)

This document defines the repository seam below the domain services only; it
changes no source code.

Sibling contracts consumed by this document (read-only):

- [session-state-contract.md](session-state-contract.md) — `MeetingManager`
  as single session-state authority; this contract's `MeetingStore` is the
  persistence projection it writes through (SSC-§1.1, SSC-§1.2 item 4).
- [recorder-contract.md](recorder-contract.md) — chunk lifecycle feeding
  `MeetingStore` chunk mutations. Below abbreviated **RC-§n**.
- [stt-provider-contract.md](stt-provider-contract.md) — job state machine
  whose durable half lives in `MeetingStore`. Below abbreviated **STT-§n**.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 0. Scope, non-goals, and schema authority

This contract specifies **who** owns which data, **through which interface**,
and **with which transactional and identity guarantees**. It deliberately does
not specify the database layout.

### 0.1 Explicitly out of scope

1. **All DDL.** This document contains no table-creation, index-creation,
   column-type, or migration-order statements. **Task-0020 and its deliverable
   `docs/meeting-mode-architecture/db-schema.md` are the sole schema
   authority**; wherever this contract names a table or field, the concrete
   definition lives there. If this contract and `db-schema.md` ever disagree on
   shape, `db-schema.md` wins for schema questions and this contract wins for
   ownership, transaction, and envelope-semantics questions.
2. **Any sync or server implementation.** The envelope defined in §5 makes
   data _consumable_ by the plan's later server and MCP clients
   (plan lines 1758–1783), but implements none of it. No sync protocol, no
   conflict resolution, no CRDT, no network code, no MCP tools are in scope
   (plan lines 1161–1162: "Das ist noch keine Synchronisation und kein CRDT";
   Phase 3 acceptance plan lines 1557–1558: "ohne dass Server- oder Sync-Code
   implementiert wird"; the server itself is roadmap, plan line 1745).
3. **The existing dictation history.** `transcription_history` and
   `HistoryManager` (`src-tauri/src/managers/history.rs`) remain untouched and
   outside both new stores (plan lines 1145–1147: "Die bestehende
   `transcription_history` bleibt für Diktate erhalten … eigene Repository-
   und Migrationsebenen").
4. **AssetStore internals** (hashing algorithm details, directory layout,
   atomic-commit mechanics) — owned by its own contract/task; this contract
   only fixes the _call direction_ (§7.1, rule R4).

### 0.2 In scope

- The trait seam below the domain services (§2).
- The entity-ownership matrix: exactly one owning store per §9 entity (§3).
- Unit-of-work rules: which mutations require a same-transaction
  `change_log`/tombstone write, which are read-only (§4).
- Change-envelope semantics: id scheme, revision, `updated_at`, deletion
  marker, content hash (§5, §6).
- The enforcement seam that keeps frontend and command-layer code away from
  SQLite and asset files (§7).

---

## 1. Position in the architecture

Plan lines 205–214 fix five layers:

```text
React-Oberfläche
  -> Tauri-Commands und Events
    -> Domain-Services (MeetingManager, ProjectManager, SourceIngestionService,
       MeetingSummaryService, ProjectIntelligenceService, …)
      -> Repository-Schnittstellen        <-- THIS CONTRACT
        -> SQLite + lokaler AssetStore (v1)
```

The repository layer is the **only** code in the process permitted to execute
SQL against the meeting/knowledge database and — via `AssetStore` only — to
place or remove persisted asset bytes. Domain services see typed Rust methods;
the React surface sees Tauri commands and events (plan lines 216–217: "Die
React-Oberfläche liest weder SQLite noch lokale Dateien direkt").

Two repositories exist, split along the plan's own component boundary:

| Repository       | File (plan §5)                              | Domain                                                                                                      |
| ---------------- | ------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `MeetingStore`   | `src-tauri/src/managers/meeting_store.rs`   | Meeting capture pipeline: sessions, audio chunks, STT jobs, transcript segments, summaries                  |
| `KnowledgeStore` | `src-tauri/src/managers/knowledge_store.rs` | Projects, sources, normalized content, assets metadata, tasks, snapshots, processing jobs, change revisions |

Both live under `managers/` next to the services that consume them, exactly as
plan lines 238–243 and 269–275 prescribe ("SQLite-Zugriff für Sessions …
Schreiboperationen erfolgen transaktional" / "Transaktionale Repository-Schicht
für Projekte, Quellen, normalisierte Inhalte, Assets, Aufgaben,
Status-Snapshots und Änderungsrevisionen").

---

## 2. Trait seam (compilable-shape Rust sketches)

These sketches are interface contracts, not implementations. They are written
to be compilable in shape (named types, explicit errors, no pseudo-code), but
they are **not** compiled in this task and method sets will be extended during
implementation. Blocking I/O is intentional: stores perform synchronous
rusqlite work, and callers (domain services behind Tauri commands) offload to
async runtimes — mirroring the existing `HistoryManager` pattern in
`src-tauri/src/managers/history.rs`.

```rust
// src-tauri/src/managers/meeting_store.rs  (sketch — not yet implemented)

use std::path::PathBuf;

/// Envelope carried by (or projected onto) every syncable core record.
/// Semantics: §5 of repository-boundary-contract.md.
#[derive(Debug, Clone)]
pub struct ChangeEnvelope {
    pub id: Uuid,             // UUIDv4, text-stable, never autoincrement-derived
    pub revision: i64,        // per-entity monotone, starts at 1
    pub updated_at: DateTime<Utc>, // RFC3339 UTC, ms precision
    pub deleted_at: Option<DateTime<Utc>>, // tombstone; None == alive
}

#[derive(Debug)]
pub enum StoreError {
    NotFound { entity: &'static str, id: Uuid },
    Conflict { entity: &'static str, id: Uuid, reason: String },
    Sql(rusqlite::Error),
    Asset(asset_store::AssetError),
}

pub struct MeetingStore {
    // private by construction; see §7.1 rule R1
    db_path: PathBuf,
    clock: Arc<dyn Clock>,
    uuid: Arc<dyn UuidSource>,
}

impl MeetingStore {
    /// Only callable from `lib.rs` setup (pub-in-crate visibility, §7.1 R2).
    pub(crate) fn open(db_path: PathBuf, deps: StoreDeps) -> Result<Self, StoreError>;
}

/// Read side: every query returns owned, typed values — never row handles,
/// never `Connection`, never raw SQL strings crossing this boundary.
impl MeetingStore {
    pub fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError>;
    pub fn list_sessions(&self, page: Page) -> Result<PageOf<MeetingSession>, StoreError>;
    pub fn segments_for_meeting(&self, meeting_id: Uuid) -> Result<Vec<TranscriptSegment>, StoreError>;
    pub fn summary_for_meeting(&self, meeting_id: Uuid) -> Result<Option<MeetingSummary>, StoreError>;
    /// Queue feed for MeetingTranscriptionQueue (STT-§3): claim happens in §4 U4.
    pub fn due_stt_jobs(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<SttJob>, StoreError>;
}

/// Write side: every mutating method is a unit of work (§4) — the store opens,
/// commits or rolls back the transaction internally. Callers cannot inject a
/// connection and cannot compose partial writes.
impl MeetingStore {
    pub fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError>;   // U1
    pub fn update_session(&self, id: Uuid, patch: SessionPatch) -> Result<MeetingSession, StoreError>; // U2
    pub fn soft_delete_session(&self, id: Uuid) -> Result<(), StoreError>;                          // U3
    pub fn append_chunk(&self, chunk: NewAudioChunk) -> Result<AudioChunk, StoreError>;             // U5
    pub fn finalize_chunk(&self, id: Uuid, facts: ChunkFacts) -> Result<AudioChunk, StoreError>;    // U6
    pub fn mark_chunk(&self, id: Uuid, state: ChunkState, reason: Option<String>) -> Result<(), StoreError>; // U7
    pub fn claim_due_stt_job(&self, worker: WorkerId, now: DateTime<Utc>) -> Result<Option<SttJobClaim>, StoreError>; // U4
    pub fn transition_stt_job(&self, id: Uuid, to: SttJobState, err: Option<String>) -> Result<(), StoreError>;      // U4
    pub fn insert_segments(&self, job_id: Uuid, segments: Vec<NewTranscriptSegment>) -> Result<Vec<TranscriptSegment>, StoreError>; // U8
    pub fn upsert_summary(&self, summary: NewMeetingSummary) -> Result<MeetingSummary, StoreError>; // U9
}
```

```rust
// src-tauri/src/managers/knowledge_store.rs  (sketch — not yet implemented)

pub struct KnowledgeStore {
    db_path: PathBuf,          // same SQLite file, separate repository layer (plan line 1147)
    assets: Arc<AssetStore>,   // bytes only; metadata rows live here
    clock: Arc<dyn Clock>,
    uuid: Arc<dyn UuidSource>,
}

impl KnowledgeStore {
    pub(crate) fn open(db_path: PathBuf, assets: Arc<AssetStore>, deps: StoreDeps)
        -> Result<Self, StoreError>;

    // Projects ------------------------------------------------------------
    pub fn create_project(&self, draft: NewProject) -> Result<Project, StoreError>;            // K1
    pub fn update_project(&self, id: Uuid, patch: ProjectPatch) -> Result<Project, StoreError>;// K2
    pub fn archive_project(&self, id: Uuid) -> Result<Project, StoreError>;                    // K2'
    pub fn soft_delete_project(&self, id: Uuid) -> Result<(), StoreError>;                     // K3

    // Sources & ingestion commit -------------------------------------------
    /// Single unit of work for SourceIngestionService.commit(): source row,
    /// normalized source_items, asset metadata rows, item–asset links — all in
    /// one transaction with one change_log append per row (U-K4).
    pub fn commit_source(&self, payload: SourceCommit) -> Result<CommittedSource, StoreError>;
    pub fn soft_delete_source(&self, id: Uuid) -> Result<(), StoreError>;                      // K5
    pub fn link_project_source(&self, link: ProjectSourceLink) -> Result<(), StoreError>;      // K6
    pub fn unlink_project_source(&self, project_id: Uuid, source_id: Uuid) -> Result<(), StoreError>;

    // Tasks ----------------------------------------------------------------
    pub fn upsert_task(&self, task: TaskUpsert) -> Result<ProjectTask, StoreError>;            // K7
    pub fn transition_task(&self, id: Uuid, to: TaskState, actor: Actor) -> Result<ProjectTask, StoreError>; // K7
    pub fn attach_task_evidence(&self, ev: NewTaskEvidence) -> Result<(), StoreError>;         // K8

    // Snapshots --------------------------------------------------------------
    /// Publishes snapshot + snapshot_evidence and demotes the previous
    /// `current` snapshot in ONE transaction (plan lines 1310–1311); K9.
    pub fn publish_snapshot(&self, snap: SnapshotPublish) -> Result<ProjectSnapshot, StoreError>;
    pub fn current_snapshot(&self, project_id: Uuid) -> Result<Option<ProjectSnapshot>, StoreError>; // read-only

    // Processing jobs (operational, no change_log — §4 tier B) ----------------
    pub fn claim_processing_job(&self, worker: WorkerId, now: DateTime<Utc>)
        -> Result<Option<ProcessingJobClaim>, StoreError>;
    pub fn finish_processing_job(&self, id: Uuid, outcome: JobOutcome) -> Result<(), StoreError>;

    // Reads (search hits always carry entity id + position, plan lines 1329–1331)
    pub fn search(&self, q: &str, scope: SearchScope) -> Result<Vec<SearchHit>, StoreError>;
}
```

Shared infrastructure used by both stores (append-only; the only code allowed
to write `change_log`, see §4):

```rust
// src-tauri/src/managers/change_log.rs  (sketch)
pub(crate) fn append(tx: &rusqlite::Transaction<'_>, entry: ChangeLogEntry) -> rusqlite::Result<()>;
```

Sketch-level notes, binding once implemented:

- `Uuid` = `uuid::Uuid` (v4); `DateTime<Utc>` = `chrono`. Exact crates are an
  implementation choice as long as §5 semantics hold.
- Neither trait/struct exposes any type containing a `Connection`, transaction
  handle, SQL fragment, or filesystem path of the database. Callers receive
  and return domain values plus `ChangeEnvelope`.
- Errors are typed (`StoreError`); no panics across the seam; no `unwrap()`
  in store code (AGENTS.md code-style rules apply).

### 2.1 Trait form of the seam

Domain services must depend on **traits**, not the concrete structs, so that
service tests can substitute fakes without SQLite. The concrete stores above
are the sole production implementations; each trait mirrors its store's full
public method set 1:1 (only representative members shown here):

```rust
// src-tauri/src/managers/meeting_store.rs  (sketch — not yet implemented)
/// Consumed by MeetingManager, MeetingTranscriptionQueue, MeetingSummaryService.
pub trait MeetingRepository: Send + Sync {
    // Read side — mirror of `impl MeetingStore` read block:
    fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError>;
    fn segments_for_meeting(&self, meeting_id: Uuid) -> Result<Vec<TranscriptSegment>, StoreError>;
    // Write side — every member is a §4 unit of work; the full set is the
    // 1:1 mirror of the write block above (U1–U9):
    fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError>;
    fn claim_due_stt_job(&self, worker: WorkerId, now: DateTime<Utc>)
        -> Result<Option<SttJobClaim>, StoreError>;
}

// src-tauri/src/managers/knowledge_store.rs  (sketch — not yet implemented)
/// Consumed by ProjectManager, SourceIngestionService, ProjectIntelligenceService.
pub trait KnowledgeRepository: Send + Sync {
    fn commit_source(&self, payload: SourceCommit) -> Result<CommittedSource, StoreError>;   // K4
    fn publish_snapshot(&self, snap: SnapshotPublish) -> Result<ProjectSnapshot, StoreError>;// K9
    fn search(&self, q: &str, scope: SearchScope) -> Result<Vec<SearchHit>, StoreError>;
    // remaining members mirror the `impl KnowledgeStore` block above 1:1
}

impl MeetingRepository for MeetingStore { /* 1:1 delegation */ }
impl KnowledgeRepository for KnowledgeStore { /* 1:1 delegation */ }
```

Rule: command handlers and services take `Arc<dyn MeetingRepository>` /
`Arc<dyn KnowledgeRepository>` (or generics bounded by them); only `lib.rs`
setup constructs the concrete types (rule R2). The traits are object-safe by
construction — no generic methods, no borrowed-lifetime returns.

---

## 3. Entity-ownership matrix (gate G1)

Every entity named in plan §9 appears below with **exactly one owning store**.
"Owns" means: the only module whose code contains SQL statements touching that
entity's rows; the only place where that entity's revision is bumped and its
`change_log` entries are appended. There are **no shared write paths**: no
method of one store reads-or-writes another store's entities, and no third
module issues SQL at all (§7). Cross-store relationships reference the partner
row exclusively by its stable UUID (plan line 1151); neither store parses or
duplicates partner fields.

| #   | Entity (plan §9 heading)                                    | Owning store                                                                | Owning file (plan §5)                                                         | Tier (§4) |
| --- | ----------------------------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | --------- |
| 1   | `meeting_sessions` (lines 1164–1178)                        | MeetingStore                                                                | `src-tauri/src/managers/meeting_store.rs`                                     | A         |
| 2   | `meeting_audio_chunks` (lines 1180–1188)                    | MeetingStore                                                                | `src-tauri/src/managers/meeting_store.rs`                                     | A         |
| 3   | `meeting_stt_jobs` (lines 1190–1199)                        | MeetingStore                                                                | `src-tauri/src/managers/meeting_store.rs`                                     | B         |
| 4   | `meeting_transcript_segments` (lines 1201–1208)             | MeetingStore                                                                | `src-tauri/src/managers/meeting_store.rs`                                     | A         |
| 5   | `meeting_summaries` (lines 1210–1217)                       | MeetingStore                                                                | `src-tauri/src/managers/meeting_store.rs`                                     | A         |
| 6   | `projects` (lines 1219–1225)                                | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 7   | `sources` (lines 1227–1239)                                 | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 8   | `project_sources` (lines 1241–1251)                         | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 9   | `source_items` (lines 1253–1266)                            | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 10  | `assets` (metadata rows; lines 1268–1275)                   | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs` (bytes: `storage/asset_store.rs`) | A         |
| 11  | `source_item_assets` (lines 1268–1275)                      | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 12  | `project_tasks` (lines 1277–1293)                           | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 13  | `task_evidence` (lines 1295–1301)                           | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 14  | `project_snapshots` + `snapshot_evidence` (lines 1303–1311) | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | A         |
| 15  | `processing_jobs` (lines 1313–1318)                         | KnowledgeStore                                                              | `src-tauri/src/managers/knowledge_store.rs`                                   | B         |
| 16  | `change_log` (lines 1320–1325)                              | Append-only shared helper `managers/change_log.rs`; schema owner: task-0020 | `src-tauri/src/managers/change_log.rs` (called only inside §4 units of work)  | C         |
| 17  | FTS5 search index ("Suche", lines 1327–1332)                | KnowledgeStore maintains it; derived data                                   | `src-tauri/src/managers/knowledge_store.rs`                                   | D         |

Notes and boundary rulings:

- **N1 — meeting ↔ source link.** Every `meeting_sessions` row gets exactly
  one `sources` row after successful creation (plan lines 1237–1239). The
  `sources` row belongs to **KnowledgeStore**; the linkage is written by the
  orchestration flow (MeetingManager calling KnowledgeStore after
  `create_session` returns the stable id), never by MeetingStore. Each side
  stores only the partner's UUID.
- **N2 — transcript segments vs. source_items.** Segments stay in the
  specialized meeting table because of their audio timeline; their
  `source_item` representation is a stable-reference projection maintained by
  KnowledgeStore (plan lines 1264–1266). MeetingStore never writes
  `source_items`.
- **N3 — assets.** SQLite holds metadata only; binary bytes live in
  `AssetStore` keyed by asset id and relative storage key, never absolute
  paths (plan lines 1158–1159, 1274–1275). KnowledgeStore writes the metadata
  row and calls `AssetStore` for byte placement inside its commit flow (§4,
  K4 ordering rule).
- **N4 — dictation history.** `transcription_history` / `HistoryManager`
  (`src-tauri/src/managers/history.rs`) is pre-existing, remains untouched,
  and appears in the §7 SQL allow-list only as legacy carve-out.
- **N5 — change_log rowids.** `change_log` is a local, append-only diagnostic
  and incremental-analysis basis (plan lines 1322–1325); its internal ordering
  key may be a plain SQLite rowid because it is not itself a syncable core
  record. Sync clients later derive order from `(entity, revision)` pairs.

---

## 4. Unit-of-work rules (gate G2)

### 4.1 Rules

- **W1 — one transaction per public mutating method.** Every mutating method
  in §2 opens exactly one transaction and commits or rolls it back before
  returning. Partial writes across two transactions are impossible by
  construction; callers cannot pass connections in.
- **W2 — change_log is same-transaction.** For every tier-A mutation, the
  `change_log` row(s) — entity, id, revision, timestamp, operation — are
  inserted **inside the same transaction** as the mutation (plan line 1322:
  "transaktional geschriebene Änderungszeilen"). A committed data change
  without its log row, or a log row without its committed change, is a
  contract violation and a test target (§7.3).
- **W3 — tombstones are same-transaction too.** Soft deletes set
  `deleted_at` (and bump revision) and append the corresponding
  tombstone `change_log` entry in the same transaction. Hard `DELETE` on
  tier-A entities is forbidden in v1; physical asset-byte reclamation follows
  retention policy and is deferred to its own task.
- **W4 — revision bump per commit.** Each committed mutation of a tier-A row
  increments that row's `revision` by exactly 1 (see §5.2).
- **W5 — FTS updates ride along.** Search-index maintenance for a tier-A
  mutation happens in the same transaction as the mutation (tier D is never
  independently writable).
- **W6 — operational records are exempt.** Tier-B rows (STT jobs, processing
  jobs) are transient work items, not syncable core records: they get
  idempotency keys and retry semantics (plan lines 1315–1318, STT-§3) but no
  envelope obligation and **no `change_log` writes**. Their state churn would
  otherwise flood the log that later sync clients consume.
- **W7 — crash consistency.** Because of W1/W2/W3, restart recovery can trust:
  any visible tier-A row carries a matching log tail; any claimed-but-crashed
  tier-B job is recovered by job-state rules (STT-§3.4), not by the log.

### 4.2 Mutation classification

Tier definitions:

- **Tier A** — syncable core record (plan lines 1149–1159): envelope fields
  mandatory; every mutation needs same-transaction `change_log` (+ tombstone
  handling on delete).
- **Tier B** — operational record: no envelope, no `change_log`.
- **Tier C** — `change_log` itself: append-only, written only by W2/W3 inside
  tier-A units of work.
- **Tier D** — derived index: written only alongside its source mutation (W5),
  rebuildable.

MeetingStore units of work:

| Unit | Mutation                                                     | Transaction contents                                                                                             | Tier / change_log? |
| ---- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- | ------------------ |
| U1   | `create_session`                                             | INSERT `meeting_sessions` (rev = 1) + `change_log`(create)                                                       | A / yes            |
| U2   | `update_session` (title, state, provider snapshot, error, …) | UPDATE + rev++ + `change_log`(update)                                                                            | A / yes            |
| U3   | `soft_delete_session`                                        | UPDATE `deleted_at` + rev++ + `change_log`(tombstone)                                                            | A / yes            |
| U4   | `claim_due_stt_job` / `transition_stt_job`                   | UPDATE `meeting_stt_jobs` state/lease/retry fields                                                               | B / **no**         |
| U5   | `append_chunk`                                               | INSERT `meeting_audio_chunks` (state `writing`, rev = 1) + `change_log`(create)                                  | A / yes            |
| U6   | `finalize_chunk`                                             | UPDATE chunk → `ready` + checksum + sizes + rev++ + `change_log`(update)                                         | A / yes            |
| U7   | `mark_chunk` (`damaged`/`deleted`)                           | UPDATE + rev++ + `change_log`(update; tombstone when `deleted`)                                                  | A / yes            |
| U8   | `insert_segments`                                            | Batched INSERT `meeting_transcript_segments` + one `change_log`(create) per inserted segment, single transaction | A / yes            |
| U9   | `upsert_summary`                                             | INSERT or UPDATE `meeting_summaries` + rev++ + `change_log`(create/update)                                       | A / yes            |

Read-only MeetingStore operations (no transaction writes, no log rows):
`get_session`, `list_sessions`, `segments_for_meeting`,
`summary_for_meeting`, passive `due_stt_jobs` inspection.

KnowledgeStore units of work:

| Unit   | Mutation                                                                                                               | Transaction contents                                                                                                                                                                                                                                             | Tier / change_log? |
| ------ | ---------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| K1     | `create_project`                                                                                                       | INSERT `projects` (rev = 1) + `change_log`(create)                                                                                                                                                                                                               | A / yes            |
| K2/K2' | `update_project` / `archive_project` (incl. manual status note; AI output must not overwrite it, plan lines 1224–1225) | UPDATE + rev++ + `change_log`                                                                                                                                                                                                                                    | A / yes            |
| K3     | `soft_delete_project`                                                                                                  | tombstone per W3 + `change_log`(tombstone)                                                                                                                                                                                                                       | A / yes            |
| K4     | `commit_source`                                                                                                        | INSERT `sources` + N× INSERT `source_items` + M× INSERT `assets` metadata (after `AssetStore` byte placement succeeded) + K× INSERT `source_item_assets` + one `change_log` row per inserted row, **single transaction**, rolled back entirely if any part fails | A / yes            |
| K5     | `soft_delete_source`                                                                                                   | tombstone cascade decision recorded on `sources` + `change_log` (item-level tombstones follow the same W3 pattern)                                                                                                                                               | A / yes            |
| K6     | `link_project_source` / unlink                                                                                         | INSERT/UPDATE `project_sources` (+ rev semantics per plan lines 1241–1247) + `change_log`                                                                                                                                                                        | A / yes            |
| K7     | `upsert_task` / `transition_task`                                                                                      | INSERT/UPDATE `project_tasks` + rev++ + `change_log`; human edits win over AI (plan lines 264–265)                                                                                                                                                               | A / yes            |
| K8     | `attach_task_evidence`                                                                                                 | INSERT `task_evidence` + `change_log`; an AI task without ≥1 valid evidence is never published (plan lines 1300–1301) — enforced before K7 publishes AI-origin tasks                                                                                             | A / yes            |
| K9     | `publish_snapshot`                                                                                                     | INSERT `project_snapshots` (`current`) + `snapshot_evidence` rows + demote previous `current` + `covered_through` bookkeeping + `change_log`, single transaction (plan lines 1303–1311)                                                                          | A / yes            |
| K10    | `claim_processing_job` / `finish_processing_job`                                                                       | UPDATE `processing_jobs`                                                                                                                                                                                                                                         | B / **no**         |

Read-only KnowledgeStore operations: all project/source/item/task/snapshot/
asset queries, `search` (reads tier-D index), duplicate checks
(`content_hash`/`external_key` lookups, plan line 1262).

---

## 5. Change-envelope semantics (gate G3)

Every tier-A record carries the envelope introduced by plan lines
1149–1159. Field semantics, fixed here so that the plan's later server and
MCP clients can consume the data unchanged (plan lines 1760–1766: "Clients
synchronisieren Änderungen über UUID, Revision, `updated_at`, Löschmarker und
Inhalts-Hash"):

### 5.1 `id` — stable identity

- UUID **version 4**, generated by the store at first persist, rendered as
  lowercase hyphenated text. Never derived from SQLite autoincrement
  (plan line 1151), never recycled, never reused after tombstone.
- External references (across stores, from services, in exports, later from
  server/MCP clients) use this UUID only. Rowids of tier-A tables are private
  implementation detail of task-0020's schema and never cross the seam.
- `change_log` entries address entities by `(entity_name, uuid)`.

### 5.2 `revision` — change counter

- Per-entity monotonic integer, starting at **1** on creation, incremented by
  exactly 1 on every committed tier-A mutation (rule W4). It expresses _that_
  and _how often_ a row changed — not wall-clock time.
- Comparison semantics for later consumers: higher revision wins as "newer
  state of the same entity"; equality means identical known state. Actual
  merge/conflict policy is explicitly deferred (plan lines 1764–1766) and not
  part of v1.
- The global ordering of independent changes is recoverable from `change_log`
  append order (local rowid, note N5); it is not encoded into `revision`.

### 5.3 `updated_at` — last-write timestamp

- UTC, RFC 3339, millisecond precision, set by the store from an injected
  `Clock` port (testability: fixed clocks in unit tests). Refreshed on every
  committed mutation together with the revision bump.
- `created_at` is set once at creation and never modified afterwards.
- Timestamps are advisory/debugging aids; **`revision` is the authoritative
  change indicator** (clock skew between machines must never break later
  sync reasoning).

### 5.4 Deletion — tombstones, not holes

- Tier-A deletion is a soft delete: set `deleted_at` (UTC, RFC 3339, ms),
  bump `revision`, append tombstone `change_log` row (rules W3/U3/K3/K5).
- Across the seam and toward later clients, deletion surfaces as the boolean
  projection `deleted == true` alongside `deleted_at`; consumers never infer
  deletion from absence.
- Physical row deletion of tier-A entities does not exist in v1. Asset _byte_
  reclamation is a retention-policy concern (plan lines 1468 ff.) handled
  separately and never by ad-hoc `DELETE`s in store code.
- Hard deletes remain legitimate for tier-B operational rows once finished —
  they leave no sync obligations precisely because they never entered the
  change log (W6).

### 5.5 `content_hash` — integrity and deduplication

- Algorithm: SHA-256, rendered as 64-char lowercase hex.
- For `assets`: hash over the exact stored byte sequence (matches the
  `AssetStore` hashing duty, plan line 1547).
- For normalized records (`sources`, `source_items`, snapshots, summaries):
  hash over a canonical serialization of the _content_ fields — UTF-8 JSON
  with lexicographically sorted keys, excluding envelope fields
  (`id`, `revision`, `created_at`, `updated_at`, `deleted_at`) so that
  re-saving unchanged content does not alter the hash. Canonicalization
  details belong to task-0020's schema document; the exclusion rule is fixed
  here.
- Uses in v1: ingestion deduplication constraints (plan line 1262), damage
  detection for chunks (checksum column of `meeting_audio_chunks`, plan line
  1186), and incremental project analysis inputs. Later sync clients may use
  hashes to skip identical payloads — without any v1 code depending on that.

### 5.6 Explicit non-scope statement

**No sync, server, transport, or MCP code is in scope for v1.** The envelope
exists solely so that the data _can_ be consumed later by the server API and
MCP tooling sketched in plan lines 1758–1783. Nothing in this contract
implements synchronization, conflict resolution, encryption, networking, or
MCP tools; those are roadmap items (plan lines 1745–1753) gated by separate
future decisions (plan lines 1764–1766: "Konfliktregeln und Ende-zu-Ende-
Verschlüsselung werden vor der Serverphase gesondert entschieden und nicht in
v1 vorgetäuscht").

---

## 6. Consumers above the seam

For completeness, the intended callers — all of them domain services, never
the frontend directly:

| Caller                                                            | Uses                            | Notes                                                                                 |
| ----------------------------------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------- |
| `MeetingManager` (`managers/meeting.rs`)                          | MeetingStore units U1–U3, U5–U7 | Writes session state through the store synchronously with decisions (SSC-§1.2 item 4) |
| `MeetingTranscriptionQueue` (`managers/meeting_transcription.rs`) | U4, U8                          | Claim/complete cycles only; STT-§3                                                    |
| `MeetingSummaryService` (`managers/meeting_summary.rs`)           | U9, reads                       | Persists summary jobs/results                                                         |
| `ProjectManager` (`managers/project.rs`)                          | KnowledgeStore K1–K6, reads     | Manual user changes outrank AI runs (plan lines 264–265)                              |
| `SourceIngestionService` (`managers/source_ingestion.rs`)         | K4                              | `commit` step maps onto the single unit of work                                       |
| `ProjectIntelligenceService` (`managers/project_intelligence.rs`) | K7–K9, K10, reads               | Incremental analysis consumes the change log (plan line 1323)                         |
| Future server/MCP clients                                         | typed service layer only        | Out of scope for v1 (§5.6)                                                            |

---

## 7. Enforcement seam — no direct DB or asset access (gate G4)

Phase 3 acceptance (plan line 1556): "kein Frontendcode greift direkt auf
SQLite oder Assetdateien zu". This contract names the choke point and the
mechanics that keep it true mechanically, not just by convention.

### 7.1 The choke-point rules

- **R1 — SQL confinement.** `rusqlite` usage and `Connection::open` are
  permitted in exactly these files:
  1. `src-tauri/src/managers/meeting_store.rs`
  2. `src-tauri/src/managers/knowledge_store.rs`
  3. `src-tauri/src/managers/change_log.rs` (append-only helper, §2)
  4. `src-tauri/src/managers/history.rs` (**legacy carve-out** — existing
     dictation history, unchanged; verified present at HEAD, see §7.3)
  5. the migration module designated by task-0020 (schema authority, §0.1)
     No other file — in particular nothing under `commands/`, `managers/*.rs`
     beyond the list, `audio_toolkit/`, or anything frontend-side — may name
     `rusqlite` types or open the database.
- **R2 — constructor confinement.** `MeetingStore::open` and
  `KnowledgeStore::open` are `pub(crate)` and are called exactly once each,
  in Tauri setup in `lib.rs`, then handed to services/command handlers as
  managed `State`. There is no second way to obtain a connection.
- **R3 — command-layer rule.** Every Tauri command handler receives typed
  manager/store handles (`State<…>`) and returns typed values (the pattern
  already used by `src-tauri/src/commands/history.rs`, which imports only
  `history::{HistoryManager, PaginatedHistory}` — verified at HEAD).
  Commands never build SQL, never touch `PathBuf`s of the database, and
  never receive raw row structures.
- **R4 — asset-bytes confinement.** All persisted asset byte placement,
  replacement, and removal goes through `AssetStore`
  (`src-tauri/src/storage/asset_store.rs`). Stores write metadata rows and
  call `AssetStore`; they do not open asset files themselves. Frontend code
  accesses media exclusively via the app's existing asset/serve mechanism —
  never via direct filesystem paths into the storage root (absolute paths
  are not durable identity, plan lines 1158–1159).
- **R5 — frontend isolation.** The React layer communicates only via Tauri
  commands/events (plan lines 208–209, 216–217). Verified at HEAD: no
  `sqlite`, no `plugin-sql`, and no direct history-table invocation exists
  anywhere under `src/`. New meeting-mode UI must keep it that way: the only
  sanctioned channel is `invoke()` of the §6-exposed commands.

### 7.2 Why the seam holds (defense in depth)

1. **Module visibility (R1/R2):** even backend code outside the two stores
   cannot reach a `Connection` because none is exposed; `db_path` fields are
   private and constructors are crate-internal. Accidental SQL in a command
   fails to compile for lack of types, not merely review.
2. **API shape (§2):** mutating methods own their transactions (W1), so
   callers physically cannot interleave foreign writes into a unit of work.
3. **Process boundary (R5):** the webview has no SQLite driver, no plugin-sql
   grant, and no filesystem scope over the storage root; the Tauri
   capability/CSP surface (audited against meeting-mode needs in company task
   0068, commit `adc89df`) is the outer wall.
4. **CI/test net:** catches violations of R1/R4 that compile fine, e.g. a
   manager reaching for `rusqlite` directly.

### 7.3 Testability note

The invariant is mechanically checkable, following the repo's existing
script-verification precedent (`docs/meeting-mode-architecture/verify-sketch.ps1`):

- **Static allow-list gate.** A small script (or `cargo test`-invoked check)
  greps `src-tauri/src` and `src/` for `rusqlite`, `Connection::open`,
  and `std::fs::write` against the R1/R4 allow-list and fails on any hit
  outside it. Baseline evidence at authoring time: `grep rusqlite` currently
  matches only `managers/history.rs`; `grep 'plugin-sql|sqlite'` under `src/`
  matches nothing (verified 22.08.2026 in this clean worktree at HEAD
  `0a1d89b`). The gate turns those observations into standing requirements.
- **Behavioral tests.** With an in-memory SQLite (same pattern as
  `managers/history.rs` tests, lines ~652 ff.), store unit tests assert
  W2/W3 atomics: simulate a failure mid-unit-of-work (e.g. forced
  `AssetStore` error in K4) and prove neither data rows nor `change_log`
  rows survived; and conversely prove every committed tier-A mutation has
  exactly one log row with the correct revision.
- **Command-surface test.** A compile-time/binding-level check (tauri-specta
  bindings, `src/bindings.ts`) that every new meeting-mode command's argument
  and return types contain no SQL/path/row types — keeping R3 verifiable
  from the generated TypeScript alone.

---

## 8. Gate verification checklist

| Gate                                                                                         | Where satisfied                                                                                                                |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| G1 ownership matrix, one owning store per §9 entity, file per plan §5                        | §3 matrix rows 1–17; homes per plan lines 243, 275, 308                                                                        |
| G2 unit-of-work enumeration incl. same-transaction change_log/tombstones and read-only lists | §4.1 rules W1–W7; §4.2 tables U1–U9, K1–K10 with explicit "**no**" rows (U4, K10) and read-only lists                          |
| G3 envelope fields with semantics + explicit no-sync-code statement                          | §5.1–§5.5; §5.6 citing plan lines 1758–1783 and Phase 3 acceptance lines 1557–1558                                             |
| G4 named choke point enforcing no-direct-DB-access + testability note                        | §7.1 rules R1–R5 (module visibility + command-layer rule); §7.3 static allow-list gate and behavioral tests                    |
| G5 zero table DDL, task-0020/db-schema.md cited as schema authority                          | §0.1 item 1; the document defines ownership, transactions and envelope semantics only — no table or index definitions anywhere |
