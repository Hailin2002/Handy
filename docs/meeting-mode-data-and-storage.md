# Meeting Mode — Data & Storage Developer Reference

Status: **target design consolidated from the plan — nothing in this document is
implemented code or a contract of implemented behavior** (see §1 and §8).
Author working base: HEAD of this worktree, verified clean via
`git status --porcelain` (empty) on 2026-08-22.

Sources (all citations refer to [`meeting-mode-plan.md`](meeting-mode-plan.md)):

| Source | Range | Content |
| --- | --- | --- |
| Plan §5 "Zielarchitektur" | `meeting-mode-plan.md:185-338` | Layered architecture, backend components incl. `MeetingStore`, `KnowledgeStore`, `AssetStore` |
| Plan §9 "Datenmodell" | `meeting-mode-plan.md:1143-1332` | Full SQLite table model, identity conventions, `change_log`, FTS5 |
| Plan §12 "Speicher- und Datenschutzlogik" | `meeting-mode-plan.md:1436-1477` | Retention, disk budget, import safety, audit trail |
| Supporting: Plan §4 | `meeting-mode-plan.md:166-170` | Verified current state: `history.db` under portable-resolved app-data dir |
| Supporting: Plan §7 "Persistenzformat" | `meeting-mode-plan.md:1002-1031` | On-disk session directory layout, atomic chunk finalization |
| Supporting: Plan §13 Phase 3 | `meeting-mode-plan.md:1536-1558` | Delivery phase that introduces all of the above |

Related normative detail documents (spec contracts, also not implemented):
[`meeting-mode-architecture/repository-boundary-contract.md`](meeting-mode-architecture/repository-boundary-contract.md)
(owning component per table),
[`meeting-mode-architecture/asset-store-contract.md`](meeting-mode-architecture/asset-store-contract.md)
(storage-key grammar, put/get/delete/verify trait, atomic-commit mechanics),
[`meeting-mode-phase2-contracts.md`](meeting-mode-phase2-contracts.md)
(manifest and chunk finalization contract).

---

## 1. What exists today vs. what is target design

**Read this first.** As of the author working base there is **no migration and no
schema** for any meeting/project/knowledge table anywhere in the repository, and
nothing in this document is a contract of implemented behavior. Every statement
below except the items explicitly marked "exists today" is **target design from
the plan**, to be built in Phase 3 (`meeting-mode-plan.md:1536-1558`) and later
phases.

What actually exists in code today is exactly one persistence stack:

- `HistoryManager` (`src-tauri/src/managers/history.rs`) with the flat
  `transcription_history` table (migrations at `history.rs:20-34`, manager at
  `history.rs:68-97`), storing WAV files in `<app-data>/recordings`
  (`history.rs:77-78`) and SQLite in `<app-data>/history.db` (`history.rs:79`),
  both resolved through the portable-aware `crate::portable::app_data_dir`
  (`history.rs:77`).
- The portable-aware path helpers in `src-tauri/src/portable.rs`
  (`app_data_dir` at `portable.rs:69-75`, `resolve_app_data` at
  `portable.rs:88-90`, headless `data_dir()` at `portable.rs:64-66`).
- Settings/secrets persistence outside this doc's scope (tauri-plugin-store,
  `secret_store.rs`).

This matches the plan's own verified baseline: *"HistoryManager speichert bereits
in `history.db` unter dem von `portable.rs` aufgelösten App-Datenverzeichnis.
Projekt- und Assetspeicherung muss dieselbe portable-aware Pfadlogik
verwenden."* (`meeting-mode-plan.md:168-170`) and its assessment that the flat
dictation table cannot represent meeting states, audio segments, or retry jobs
(`meeting-mode-plan.md:166-167`).

Per-component status summary (details per table in §3):

| Piece | Plan citation | Status |
| --- | --- | --- |
| `transcription_history` table + migrations | n/a (pre-existing, kept unchanged) | **exists today**: `managers/history.rs:20-34` |
| `HistoryManager` (dictation path) | kept per `meeting-mode-plan.md:1145` | **exists today**: `managers/history.rs:68-97` |
| portable.rs app-data-dir logic | `meeting-mode-plan.md:168-170`, `304-306` | **exists today**: `portable.rs:64-90` |
| Single shared SQLite file, separate migration layers | `meeting-mode-plan.md:1145-1147` | planned, not implemented |
| `MeetingStore`, `KnowledgeStore`, repositories | `meeting-mode-plan.md:238-243,269-275` | planned, not implemented (`managers/meeting_store.rs`, `managers/knowledge_store.rs` do not exist) |
| All §9 tables (`meeting_sessions` … `snapshot_evidence`) | `meeting-mode-plan.md:1164-1311` | planned, not implemented |
| `processing_jobs`, `change_log` | `meeting-mode-plan.md:1313-1325` | planned, not implemented |
| FTS5 search index | `meeting-mode-plan.md:1327-1332` | planned, not implemented |
| `AssetStore` (`storage/asset_store.rs`) | `meeting-mode-plan.md:301-308,1547` | planned, not implemented (`src-tauri/src/storage/` does not exist) |
| `meetings/<id>/` on-disk layout, manifest | `meeting-mode-plan.md:1004-1021` | planned, not implemented |

---

## 2. Database topology

Target design (`meeting-mode-plan.md:1145-1147`):

- The existing dictation stack stays untouched: `transcription_history` remains
  the dictation store and is **not** repurposed for meetings or knowledge data
  (also `meeting-mode-plan.md:272-273`).
- Meetings get their own migrations and tables. Projects and imported content
  use the **same SQLite database file** but their own repository and migration
  layers. Concretely that means separate migration sequences per domain layer
  tracked independently, not one monolithic schema version for everything.
- Access is layered (plan §5, `meeting-mode-plan.md:205-217`): React UI → Tauri
  commands/events → domain services → repository interfaces → SQLite +
  local AssetStore. **No frontend code reads SQLite or asset files directly**
  (acceptance in `meeting-mode-plan.md:1556`). The owning components are:
  - `MeetingStore` — sessions, chunks, STT jobs, transcript segments,
    summaries; transactional writes
    (`meeting-mode-plan.md:238-243`, planned home
    `src-tauri/src/managers/meeting_store.rs`);
  - `KnowledgeStore` — projects, sources, normalized content, assets metadata,
    tasks, snapshots, change revisions
    (`meeting-mode-plan.md:269-275`, planned home
    `src-tauri/src/managers/knowledge_store.rs`).

---

## 3. Table model (plan §9, `meeting-mode-plan.md:1143-1332`)

All of the following is **planned, not implemented**. Field lists are
consolidations of the plan bullets; the plan deliberately does not fix SQL types
or exact column names — those are Phase 3 design decisions. Naming note: the
leaf term "meetings" corresponds to the plan's `meeting_sessions` table.

### 3.1 Meeting-domain tables

**`meeting_sessions`** (`meeting-mode-plan.md:1164-1178`)
`id` (stable UUID), `title`, `started_at`/`ended_at`, `state`,
`stt_provider` plus a snapshot of the non-secret provider configuration,
`meeting_model_id` (local processing), `language`, used microphone/output
devices, `has_microphone_audio`/`has_system_audio`,
`recording_retention_policy`, `summary_provider`, `summary_state`,
`last_error`, `created_at`, `updated_at`.

**`meeting_audio_chunks`** (`meeting-mode-plan.md:1180-1188`)
`id`, `meeting_id`, `sequence`, `source`, `timeline_start_ms`/`timeline_end_ms`,
`path` (**relative storage key**, see §5), `sample_rate`, `sample_count`,
`file_size`, checksum, `state` ∈ {`writing`, `ready`, `damaged`, `deleted`},
device identifier and gap/drift metadata. The `writing → ready` transition is
the atomic rename described in §5.3; `damaged` covers crash-interrupted chunks
(`meeting-mode-plan.md:1019-1021`).

**`meeting_stt_jobs`** (`meeting-mode-plan.md:1190-1199`)
`id`, `meeting_id`, referenced chunk IDs, `source` or Azure channel mapping,
timeline span, `provider` plus secret-free config snapshot, `state` ∈ {`queued`,
`running`, `retry_wait`, `succeeded`, `failed`, `cancelled`}, `attempt_count`,
`next_attempt_at`, `last_error`, timestamps. Failed/unconfirmed jobs block
automatic audio deletion (`meeting-mode-plan.md:1451-1452`).

**`meeting_transcript_segments`** (`meeting-mode-plan.md:1201-1208`)
`id`, `meeting_id`, `job_id`, `source`, `start_ms`/`end_ms`, `text`, optional
word timestamps and confidence, `provider`, `model_id`, `sequence`,
`created_at`. Segments stay in this specialized table because of their audio
timeline and are exposed as searchable/citable `source_item`s via stable
reference (`meeting-mode-plan.md:1264-1266`).

**`meeting_summaries`** (`meeting-mode-plan.md:1210-1217`)
`id`, `meeting_id`, `provider`, `model`, prompt version, `state`,
`summary_markdown`, structured `decisions`/`action_items`/`open_questions` as
JSON, `last_error`, timestamps.

### 3.2 Project/knowledge tables

**`projects`** (`meeting-mode-plan.md:1219-1225`)
`id`, `name`, optional `description`, `color`; `state` ∈ {`active`,
`archived`}; `created_at`, `updated_at`, `deleted_at`, `revision`; an optional
manually maintained status note that AI output must never overwrite.

**`sources`** (`meeting-mode-plan.md:1227-1239`)
Generic origin record for `source_type` ∈ {`meeting`, `whatsapp_export`,
`manual_note`, `file`, … future types}: `id`, `title`, `occurred_at`, optional
time span, `ingestion_state`, `parser_version`, `content_hash`,
`raw_asset_id`, import/privacy metadata, sync-identity columns. Each
`meeting_session` gets **exactly one** associated `source`; the source row
references the meeting logically without duplicating audio/STT rows.

**`project_sources`** (`meeting-mode-plan.md:1241-1251`)
`project_id`, `source_id`, `is_primary`, `origin` ∈ {`manual`,
`ai_suggested`, `import_default`}, `confidence`, `confirmed_at`, `created_at`;
unique constraint on (project, source). Deliberately many-to-many so v1 can
show one primary project while later cross-project meetings need no data copy.

**`source_items`** (`meeting-mode-plan.md:1253-1266`)
Normalized addressable units (chat message, transcript segment, document
section, note): `id`, `source_id`, `item_type`, `external_key`,
`author_label`, `occurred_at`, `sequence`, `text`, structured metadata,
`content_hash`, sync-identity columns; unique dedup constraints per source and
external key/hash.

**`assets`** and **`source_item_assets`** (`meeting-mode-plan.md:1268-1275`)
`assets`: `id`, storage key, MIME type, file size, hash, original filename,
width/height/duration when detected, processing state, retention rule.
`source_item_assets`: maps one medium/document to one or more normalized
messages/sources. **No binary blobs in SQLite** — SQLite holds metadata only;
files live in the AssetStore (§5).

**`project_tasks`** (`meeting-mode-plan.md:1277-1293`)
`id`, `project_id`, `title`, optional description; v1 `state` ∈ {`suggested`,
`open`, `done`, `dismissed`} (later execution adds `in_progress`,
`waiting_for_approval`, `failed`, `cancelled`); `priority` ∈ {`low`,
`normal`, `high`, `urgent`}; optional due date and assignee; `owner_type` ∈
{`unassigned`, `human`} in v1 (`assistant` later); `origin` ∈ {`human`,
`ai`}, `confidence`, `model`, prompt version; `human_edited_at`,
`completed_at`, sync-identity columns. Executability is never stored as a
persistent truth column; it is evaluated at runtime against installed
capabilities (`meeting-mode-plan.md:1290-1293`).

**`task_evidence`** (`meeting-mode-plan.md:1295-1301`)
`task_id`, `source_id`, optional `source_item_id` or
`meeting_transcript_segment_id`, optional timestamp range, short evidence
excerpt, explanation basis. An AI suggestion without at least one valid
evidence row is never published as a task.

**`project_snapshots`** and **`snapshot_evidence`**
(`meeting-mode-plan.md:1303-1311`)
Versioned project status: `summary_markdown`, decisions, risks, open
questions; `covered_through`, involved source revisions, provider, model,
prompt version. Exactly one snapshot is `current`; older ones are retained for
auditability. `snapshot_evidence` links individual statements to sources or
source items.

### 3.3 Cross-cutting tables

**`processing_jobs`** (`meeting-mode-plan.md:1313-1318`)
Generic persistent jobs for import, media analysis, source summary, project
snapshot, and later connector sync. State/retry semantics identical to
`meeting_stt_jobs`; an **idempotency key** prevents duplicate results.

**`change_log`** (`meeting-mode-plan.md:1320-1325`)
Small transactional change rows: entity, ID, revision, timestamp. In v1 it
feeds incremental project analysis and diagnostics; a later sync client may
build on it. It contains **no secrets and no full copies of large content**.

### 3.4 Search: FTS5 (`meeting-mode-plan.md:1327-1332`)

SQLite FTS5 indexes project names, source items, transcripts, tasks, and
snapshots. Hits always return entity ID plus evidence position. A future
embedding/vector search sits behind a `SearchService` interface and is
explicitly **not** pulled forward while full-text search meets real demand.

---

## 4. Sync-ready identity conventions (plan §9 preamble,
`meeting-mode-plan.md:1149-1162`)

Applies to every synchronizable core record (projects, sources, source items,
tasks, snapshots, summaries, jobs) already in v1:

1. **Stable UUIDs** — IDs never depend on SQLite AUTOINCREMENT. (Contrast with
   the existing `transcription_history`, whose `id INTEGER PRIMARY KEY
   AUTOINCREMENT` at `history.rs:23` is fine for the dictation path precisely
   because that table stays out of scope.)
2. **Timestamps + tombstones + revisions** — `created_at`, `updated_at`,
   optional `deleted_at` tombstone marker, and a **monotone `revision`** on
   every sync-ready record.
3. **Schema and parser versions** — stored so migrations and re-parses are
   reproducible.
4. **Content hashes** — for files (via the AssetStore, §5) and for normalized
   source elements (dedup, `source_items.content_hash`).
5. **Real relational integrity** — foreign keys, transactions, and unique
   constraints instead of relationships hidden inside JSON blobs.
6. **No absolute paths as durable identity** — the database stores asset ID
   plus relative storage key only (`meeting-mode-plan.md:1158-1159`).

Explicit boundary (`meeting-mode-plan.md:1161-1162`): this is **not**
synchronization and not a CRDT. It creates a unique change basis for a later
server, nothing more. No sync protocol, server API, or merge logic is part of
v1 (also `meeting-mode-plan.md:1557-1558`).

---

## 5. AssetStore layout (plan §5 `meeting-mode-plan.md:301-308`,
§7 `1002-1031`, §9 `1268-1275`, Phase 3 `1547`)

Planned home: `src-tauri/src/storage/asset_store.rs`. **Not implemented.**

### 5.1 Responsibility

Stores audio, images, documents, and raw imports under **stable asset IDs and
content hashes** (`meeting-mode-plan.md:303-304`). v1 implements a local store
rooted in the directory determined by `portable.rs`; a later object-storage
implementation must not change the callers (`meeting-mode-plan.md:304-306`).
SQLite stores metadata rows only; binaries live in the AssetStore
(`meeting-mode-plan.md:1274-1275`). Ownership boundaries (who writes `audio/`,
who commits/deletes, who decides retention) are specified in the
[AssetStore contract](meeting-mode-architecture/asset-store-contract.md);
this doc records the plan-level requirements.

### 5.2 Relative keys

Every persistent file reference is a **relative storage key** resolved through
the portable app-data root (§6). Absolute paths are forbidden as durable
identity (`meeting-mode-plan.md:1158-1159`); chunk rows carry the relative
`path` (`meeting-mode-plan.md:1185`). This keeps the whole dataset relocatable
(portable stick, backup restore, future sync target).

### 5.3 Atomic commit

Chunk files are written to a temporary name first, the header is finalized on
completion, and the file is **atomically renamed** into place
(`meeting-mode-plan.md:1019-1021`). A process crash can therefore damage at
most the currently open chunk, never the whole recording. The same
temp-file → fsync → atomic-rename pattern is the AssetStore's commit primitive
for general assets ("`AssetStore` mit relativen Storage-Keys, Hashing und
atomarem Commit", `meeting-mode-plan.md:1547`); mechanics are detailed in the
AssetStore contract §6.

### 5.4 On-disk session layout (plan §7)

```text
meetings/<meeting-id>/
  manifest.json
  audio/
    000001-microphone.wav
    000001-system.wav
    ...
  export/
  recovery/
```

(`meeting-mode-plan.md:1007-1017`.) Initial roll time is a testable constant,
first value 60 s (`meeting-mode-plan.md:1023-1025`). Capture chunks and STT
voice segments are explicitly different things. Azure runs use a temporary
stereo package assembled from the synchronized mic/system chunks; local runs
feed the sources separately to the STT queue (`meeting-mode-plan.md:1028-1031`).

### 5.5 Import admission (plan §12, `1463-1468`)

WhatsApp archives and all imported files are **untrusted input**. Before
extraction: normalize target paths; reject path traversal, absolute paths, and
symlinks; cap file count, per-file size, total size, and compression ratio
against ZIP bombs; refuse damaged or encrypted archives without partial
import. Files move from an isolated temp directory into the AssetStore only
after validation succeeds.

---

## 6. Portable-aware data-path rules (must-haves for any implementation)

These rules bind the planned stores to the **existing** portable logic — the
same logic `history.db` already uses:

1. Resolve every storage root through `crate::portable::app_data_dir(app)`
   (`src-tauri/src/portable.rs:69-75`), exactly like `HistoryManager` does for
   `recordings/` and `history.db` (`src-tauri/src/managers/history.rs:77-79`).
   Never call Tauri's `app.path().app_data_dir()` directly for meeting/project
   data — in portable installs (marker file `portable` next to the exe, data
   under `<exe-dir>/Data`, `portable.rs:15-49`) that would silently split the
   dataset across two roots.
2. For path composition from a stored relative key use
   `portable::resolve_app_data(app, relative)` (`portable.rs:88-90`); for
   headless contexts (tests, CLI) use `portable::data_dir()`
   (`portable.rs:64-66`).
3. Store only relative keys in the database (§5.2); resolution happens at
   runtime through rule 2, so the dataset moves with the portable root without
   any migration.
4. This is a plan requirement verbatim: "Projekt- und Assetspeicherung muss
   dieselbe portable-aware Pfadlogik verwenden"
   (`meeting-mode-plan.md:169-170`).

Storage sizing context for implementers (plan §12, `1438-1442`): PCM16 @ 16 kHz
is ≈ 115 MB/h per mono track, ≈ 230 MB/h dual-track; temporary Azure packages
and exports add short-term overhead. The start dialog must show per-hour need,
free space, and rough feasible recording duration. Retention defaults and the
rule that failed/unconfirmed STT jobs block automatic audio deletion are in
`meeting-mode-plan.md:1444-1452`; per-import auditability (original file,
local processing, cloud payloads, parser/model/prompt versions, retention
rules) in `meeting-mode-plan.md:1470-1476`.

---

## 7. Non-goals (complexity budget)

To keep the storage layer simple, the following are explicitly out of scope for
v1 per the plan:

- Any synchronization, CRDT, or server-side merge — only the change basis
  (§4) is built (`meeting-mode-plan.md:1161-1162`).
- Binary blobs inside SQLite (`meeting-mode-plan.md:1274-1275`).
- Vector/embedding search ahead of FTS5 (`meeting-mode-plan.md:1330-1332`).
- Repurposing `transcription_history` for meeting/knowledge data
  (`meeting-mode-plan.md:272-273`).
- Overloading the dictation `AudioRecorder`/`HistoryManager` with meeting
  concerns — meeting lifecycle gets its own components
  (`meeting-mode-plan.md:181-183,229-236`).

## 8. Contract disclaimer

**No meeting/project/knowledge migration, schema, table, or AssetStore exists
in the repository today** (verified: no `meeting_store.rs`, `knowledge_store.rs`,
or `storage/` module under `src-tauri/src/`; a repo-wide grep for `M::up`,
`Migrations::new`, and `CREATE TABLE` finds SQLite schema definitions only in
`history.rs`, whose migrations at `history.rs:20-34` define only
`transcription_history`; `managers/model/migrations.rs` migrates bundled model
*files*, not SQLite schemas). Nothing in this document is a contract of
implemented behavior;
it consolidates target design from plan §5, §9, and §12 so implementers have a
single entry point. When Phase 3 lands, re-verify every "exists today" claim in
§1 against the actual tree before relying on this document.
