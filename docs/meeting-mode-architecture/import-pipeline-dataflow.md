# Import Ingestion Dataflow — `SourceIngestionService` Stage Contract

Status: **spec contract — no implementation yet** (Phase 9 prerequisite; changes
no source code)

Plan anchors: [docs/meeting-mode-plan.md](../meeting-mode-plan.md)

- §5 "Zielarchitektur": `SourceIngestionService` (plan lines 277–283),
  `WhatsAppExportImporter` (plan lines 285–291), `AssetStore` (plan lines
  301–308)
- §6.19 "WhatsApp-Importassistent": four-step wizard, defensive parsing,
  idempotent re-import (plan lines 800–835)
- Release gate (plan line 872): _"ein wiederholter WhatsApp-Export erzeugt
  keine sichtbaren Nachrichtendubletten"_ — restated as Phase-9 acceptance
  (plan line 1687: "derselbe oder überlappende Export erzeugt keine sichtbaren
  Dubletten") and Definition-of-Done item 10 (plan lines 1974–1975)
- §9 data model: `sources` (plan lines 1227–1239), `source_items` (plan lines
  1253–1266), `assets`/`source_item_assets` (plan lines 1268–1275),
  `processing_jobs` (plan lines 1313–1318)
- §12 untrusted-input rules and per-import auditability (plan lines
  1463–1476)

Sibling contracts consumed read-only (abbreviated below):

- [repository-boundary-contract.md](repository-boundary-contract.md) (**RB**) —
  `KnowledgeRepository::commit_source` as unit of work K4; envelope semantics;
  SQL confinement rules R1/R4.
- [asset-store-contract.md](asset-store-contract.md) (**AS**) — storage-key
  grammar (AS §2.2), atomic six-step write-commit (AS §5.2), `AssetId`
  scheme `asst_<uuid-v4>` and `ContentHash` `sha256-<64hex>` (AS §3),
  content-first deletion ordering (AS §9.2).
- [../meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md) —
  entry-ID and i18n conventions extended by §9 of this document.

Precedence (documentation index §4): plan beats contracts, contracts beat
catalogs, nothing beats real code.

---

## 1. Scope

### 1.1 In scope

This document fixes the **data-flow contract** of the import pipeline: the
five `SourceIngestionService` stages (`inspect`, `preview`, `normalize`,
`deduplicate`, `commit`) with typed inputs/outputs, failure semantics and
transaction/compensation boundaries; the dedup-key grammar that makes
re-imports provably no-op and overlapping exports converge; the media-linking
rule onto `AssetStore` storage keys; original-file retention as a citable
source; and the mapping of preview/progress/error states onto UX-copy catalog
entries and future Tauri commands.

Two importers are specified against the same stage interface:

| Importer                       | Planned home (plan §5)                                | Source type                                                     |
| ------------------------------ | ----------------------------------------------------- | --------------------------------------------------------------- |
| `WhatsAppExportImporter`       | `src-tauri/src/importers/whatsapp.rs` (plan line 291) | `.zip` / `.txt` WhatsApp export                                 |
| `ManualFileImporter` (generic) | same file, second impl of the same trait              | arbitrary user-chosen file, `source_type ∈ {manual_note, file}` |

### 1.2 Explicitly out of scope

1. **Format research** — date/locale/format-variant grammars of WhatsApp
   exports are task-0054's leaf. This contract consumes its output as
   `parser_version`-scoped behavior behind the `normalize` stage and fixes
   none of it.
2. **Adversarial fixtures** — malicious/bomb/corrupt archive test corpora are
   task-0040's leaf. This contract names the enforcement points (§5 stage
   failures, AS §2.2 key validation) but defines no fixtures.
3. **Multimodal/cloud analysis** — OpenRouter modalität filtering, cost
   estimation and selective upload happen _after_ `commit`, driven by
   `processing_jobs` (plan lines 1313–1318); only the job hand-off is fixed
   here (§7.4).
4. **SQL DDL** — schema authority is task-0020 / `db-schema.md` (RB §0.1);
   this document constrains flow and uniqueness semantics only.

---

## 2. End-to-end dataflow overview

```text
user picks .zip/.txt/file
        │  (copy into isolated temp staging dir — plan §12 rules)
        v
┌─────────────────────────── SourceIngestionService ───────────────────────────┐
│                                                                              │
│  inspect ──> preview ──> normalize ──> deduplicate ──> commit                 │
│  (safety)   (read      (pure       (read vs.       (bytes → one            │
│             only)       function)   committed         SQLite unit of          │
│                                    items)            work, K4)               │
│                                                                              │
│  every stage failure = terminal for the run; nothing is written              │
│  except inside commit                                                        │
└──────────────────────────────────────────────────────────────────────────────┘
        │                                     │
        v                                     v
  KnowledgeStore                        AssetStore (bytes only:
  sources, source_items,                assets/<asst_id>/original.ext,
  assets metadata,                      hash-verified, AS §5.2)
  source_item_assets, change_log
        │
        v
  FTS5 indexing (rides along in K4, RB rule W5) → ProjectIntelligenceService
```

Invariants that hold across the whole pipeline:

- **INV-1 (single write boundary).** Stages 1–4 never mutate persistent
  state. All durable effects happen in stage 5, and inside it exactly once
  (§6.4).
- **INV-2 (no absolute paths persisted).** Every record that survives the
  run stores asset IDs and relative storage keys only (AS §2.1 rule 1; plan
  lines 1158–1159). `PathBuf`s exist solely inside the run, referencing the
  temp staging directory.
- **INV-3 (untrusted until inspected).** The upload is quarantined in an
  isolated staging directory; ZIP-safety checks (path traversal, absolute
  paths, symlinks, bomb limits) run in stage 1 _before_ anything else touches
  the bytes (plan lines 1463–1468).
- **INV-4 (determinism).** Given the same staged bytes and the same
  committed database state, stages 2–4 produce byte-identical outputs. This
  is what makes §6's no-op proof possible.

---

## 3. Stage interface (typed)

```rust
// src-tauri/src/managers/source_ingestion.rs  (sketch — not yet implemented)

/// Identity of one quarantined upload for the duration of a wizard run.
pub struct IngestRunId(String); // "ing_<uuid-v4>", never persisted as identity

/// Stage 1 input. Absolute path is process-local (temp staging dir) and
/// intentionally unreachable from the frontend; the frontend holds only the
/// opaque run id (RB rule R5).
pub struct StagedUpload {
    pub run: IngestRunId,
    pub staging_path: std::path::PathBuf, // temp dir, deleted after the run
    pub original_filename: String,        // display metadata only, never a key
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImporterKind {
    WhatsAppZipTxt,
    ManualFile,
}

/// Stage 1 output.
pub struct InspectReport {
    pub kind: InspectedKind, // container sniffed: ZipArchive | PlainText | Other
    pub entry_count: u32,
    pub total_bytes: u64,
    pub max_ratio_observed: f64, // compression ratio, bomb-limit check
    pub safety: SafetyVerdict,   // Safe | Rejected(SafetyRejection)
}

/// Stage 2 output (rendered verbatim by WhatsAppImportWizard step 2).
pub struct ImportPreview {
    pub importer: ImporterKind,
    pub parser_version: semver::Version,
    pub chats: Vec<ChatOutline>,     // name, participant labels, span, counts
    pub media_inventory: Vec<MediaOutline>,
    pub warnings: Vec<ParseWarning>, // per-item, non-fatal
    pub fast_no_op: Option<ExistingSourceRef>, // archive hash already imported
    pub duplicate_estimate: DuplicateEstimate,
}

/// Stage 3 output: fully normalized, hash-stamped candidates.
pub struct NormalizedBatch {
    pub importer: ImporterKind,
    pub parser_version: semver::Version,
    pub items: Vec<NormalizedItem>, // includes ExternalKey + ContentHash
    pub media: Vec<StagedMedia>,    // staged bytes + ContentHash, not yet stored
    pub warnings: Vec<ParseWarning>,
}

/// Stage 4 output: the exact plan stage 5 will execute — nothing else.
pub struct CommitPlan {
    pub new_items: Vec<NormalizedItem>,
    pub duplicate_skips: Vec<DuplicateSkip>, // candidate -> existing item ref
    pub conflicts: Vec<KeyConflict>,         // same key, different hash
    pub assets_to_store: Vec<StagedMedia>,   // only media of NEW items (§7.2)
    pub source_row: NewSourceDraft,          // title, span, retention rule, …
    pub idempotency_key: IdempotencyKey,     // for processing_jobs (tier B)
}

/// Stage 5 output.
pub struct CommittedSource {
    pub source_id: uuid::Uuid,
    pub raw_asset_id: String, // "asst_<uuid-v4>"
    pub imported_items: usize,
    pub skipped_duplicates: usize,
    pub conflicts: usize,
}
```

Errors are typed per stage and never raw backend strings (plan §6.2 rule 8):

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestionError {
    /// Stage 1: traversal, symlink/reparse point, absolute member path,
    /// bomb limit, encrypted archive, unreadable container (task-0040 corpus).
    SafetyRejected { reason: SafetyRejection },
    /// Stage 2/3: file readable but no parser variant matches it at all.
    UnparsableFormat { parser_version: semver::Version, detail: String },
    /// Stage 2: parse produced zero usable items (empty export).
    EmptyImport,
    /// Stage 5: AssetStore refused or failed a put (carries PutResult list
    /// for compensation, see §6.4).
    AssetPutFailed { attempted: Vec<String>, detail: String },
    /// Stage 5: K4 rolled back; asset compensation was attempted.
    CommitRolledBack { compensated_assets: usize, detail: String },
    /// Stage 5: K4 rolled back and compensation could not run (crash-risk
    /// window; startup reconciliation handles the residue, §6.5).
    CommitOrphanedAssets { keys: Vec<String> },
    /// Any stage: disk full, IO error inside staging.
    StagingIo { detail: String },
}
```

Every variant maps 1:1 onto a UX-copy catalog entry (§9).

---

## 4. Stage specifications (G1)

Each stage below states: owner, typed input/output (from §3), failure
semantics, and its transaction or compensation boundary.

### 4.1 Stage 1 — `inspect`

| Aspect               | Contract                                                                                                                                                                                                                                                                              |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owner                | `SourceIngestionService`, delegating container checks to the importer                                                                                                                                                                                                                 |
| Input                | `StagedUpload`                                                                                                                                                                                                                                                                        |
| Output               | `Result<InspectReport, IngestionError>`                                                                                                                                                                                                                                               |
| Reads/writes         | Read-only over staged bytes; **no DB access, no asset writes**                                                                                                                                                                                                                        |
| Failure semantics    | Any `SafetyRejection` (traversal member, symlink, absolute path, ratio/count/size limit, encryption) terminates the run terminally — plan line 1467: damaged/encrypted archives are rejected _without partial import_. Zero-value archives fail as `EmptyImport` at stage 2, not here |
| Transaction boundary | None (pure). **Compensation:** the whole staging directory is deleted when the run ends, successfully or not (run-scoped RAII guard in the service)                                                                                                                                   |

The staging copy itself is created before stage 1 by the service (the
frontend never passes paths; it passes a file picked through the Tauri dialog,
and the backend copies it — RB rule R5). The copy step enforces a maximum
upload size _before_ copying completes.

### 4.2 Stage 2 — `preview`

| Aspect               | Contract                                                                                                                                                                                                                              |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owner                | Importer implementation (`whatsapp.rs` / manual)                                                                                                                                                                                      |
| Input                | `InspectReport` + staged bytes                                                                                                                                                                                                        |
| Output               | `Result<ImportPreview, IngestionError>` (`UnparsableFormat`, `EmptyImport`, per-item `warnings`)                                                                                                                                      |
| Reads/writes         | Read-only. Optionally inserts **one tier-B `processing_jobs` row** (`state = running`) so a long parse survives restart; that row carries no envelope and no `change_log` (RB W6)                                                     |
| Failure semantics    | Fatal only when no format variant matches at all or zero items parse. Individual malformed lines/messages become `ParseWarning`s — the export stays importable (plan lines 813–815: defensive parsing, missing attachments tolerated) |
| Transaction boundary | Tier-B row insert/update is its own auto-commit unit; a crashed parse leaves a stale `running` job that startup recovery resets per STT-job rules (RB W7 pattern). **No compensation needed — nothing else exists yet**               |

`preview` also computes the archive content hash (SHA-256 over staged bytes)
and consults `sources.content_hash`: an exact hit yields
`fast_no_op = ExistingSourceRef` and the wizard shows the already-imported
state instead of step 3/4 (§6.2, no-op path P0).

### 4.3 Stage 3 — `normalize`

| Aspect               | Contract                                                                                                                                                                                                                           |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owner                | Importer implementation                                                                                                                                                                                                            |
| Input                | `ImportPreview`-consistent parsed model + staged bytes (media extracted to staging subdirectory)                                                                                                                                   |
| Output               | `NormalizedBatch` — cannot fail wholesale once stage 2 succeeded; per-item problems degrade to warnings                                                                                                                            |
| Reads/writes         | Pure function over (staged bytes, parser code, `parser_version`). Deterministic under INV-4                                                                                                                                        |
| Key work             | Map every message to `NormalizedItem { external_key, content_hash, author_label, occurred_at, sequence, text, metadata }` per the §6.1 grammar; hash every media file (`ContentHash`, AS §3.2); detect MIME/sniffed type per media |
| Failure semantics    | Per-item only. An item whose text cannot be decoded is imported with lossy-decoded text plus a warning; an unrecoverable item becomes a `ParseWarning`, never a silent drop                                                        |
| Transaction boundary | None (pure). **Compensation:** staging-directory cleanup as in 4.1                                                                                                                                                                 |

### 4.4 Stage 4 — `deduplicate`

| Aspect               | Contract                                                                                                                                                                                                                                                    |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owner                | `SourceIngestionService` (cross-importer logic lives here, not in the importers)                                                                                                                                                                            |
| Input                | `NormalizedBatch`                                                                                                                                                                                                                                           |
| Output               | `CommitPlan`                                                                                                                                                                                                                                                |
| Reads/writes         | Read-only queries against committed `source_items` by `(external_key)` and `(source_id, external_key)` (both indexes exist for plan-line-1262 uniqueness); no writes                                                                                        |
| Failure semantics    | Cannot fail on data — worst case is `conflicts` non-empty, which the UI surfaces and the user confirms (§9 entry IMP-I3). DB connectivity failure surfaces as `StagingIo`-class transport error and ends the run before any write                           |
| Transaction boundary | None (pure reads). Snapshot-consistency note: between stage 4 and stage 5 another import could commit overlapping items; the DB-level uniqueness constraints (§6.3) make the outcome converge anyway — stage 4 is advisory planning, stage 5 is enforcement |

Classification per candidate item:

| Case                | Condition                                 | Disposition                                                                            |
| ------------------- | ----------------------------------------- | -------------------------------------------------------------------------------------- |
| New                 | no committed row with this `external_key` | `new_items`                                                                            |
| Identical duplicate | row exists, `content_hash` equal          | `duplicate_skips` (its media is **not** re-stored, §7.2)                               |
| Key conflict        | row exists, `content_hash` differs        | `conflicts`; kept under the disambiguation rule of §6.1 rule K3, never silently merged |

### 4.5 Stage 5 — `commit`

| Aspect               | Contract                                                                                                                               |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Owner                | `SourceIngestionService` orchestrating `AssetStore` + `KnowledgeRepository::commit_source` (RB K4)                                     |
| Input                | `CommitPlan` (+ user decisions: target project / inbox, retention rule, local-vs-cloud consent flags captured in step 3 of the wizard) |
| Output               | `Result<CommittedSource, IngestionError>`                                                                                              |
| Reads/writes         | The only writing stage (INV-1)                                                                                                         |
| Failure semantics    | See §6.4 crash/failure windows                                                                                                         |
| Transaction boundary | Two-phase with strict order, normative in §6.4                                                                                         |

---

## 5. Original-file retention — the belegbare Quelle

Every successfully committed import retains the **original uploaded file
byte-for-byte** as the citable origin (plan line 289: "Das Original bleibt
entsprechend der Aufbewahrungsregel als belegbare Quelle erhalten"; auditability
list plan lines 1470–1476):

1. During stage 5, the staged original is `put` as the source's raw asset:
   - `AssetId`: fresh `asst_<uuid-v4>` (AS §3.1 — identity is never
     content-derived, so two imports of the same bytes keep separate
     histories);
   - storage key: `assets/<asset-id>/original.zip` / `original.txt` /
     `original.<ext>` — exactly the AS §2.2 `asset-key` production;
   - expected hash: archive `ContentHash` computed at stage 2 (verified free
     of charge at AS verification point V1).
2. The `sources` row stores `raw_asset_id` and `content_hash` (plan lines
   1233–1234); the `assets` metadata row stores MIME type, size, hash,
   `original_filename` (display provenance, never part of any key — AS §2.2)
   and the retention rule chosen in wizard step 3 (plan line 1271).
3. Evidence chains resolve item → `source_item.source_id` →
   `sources.raw_asset_id` → `assets.storage_key` → `AssetStore::resolve`.
   Nothing in that chain contains an absolute path (INV-2); resolution is the
   store's single choke point (AS §2.1 rule 2).
4. Deleting an import follows the retention/delete flows owned elsewhere
   (AS §8/§9 pattern); this contract only guarantees the raw asset exists and
   hash-verifies while `sources.deleted_at IS NULL`.

---

## 6. Deduplication and idempotence (G2)

Target: plan line 872 — a repeated WhatsApp export produces **no visible
message duplicates**, for both the identical-file case and overlapping
exports (Phase-9 acceptance, plan line 1687; DoD item 10, plan lines
1974–1975).

### 6.1 Dedup-key grammar

Identity of a normalized item is **content-derived**, never positional (file
line numbers and media filenames differ between exports — plan lines
816–817 require stable external keys precisely for this reason):

```text
external-key  := "wa1-" 32HEXDIG | "man1-" 32HEXDIG

canonical-input (UTF-8, "\u{1F}" separator):
    occurred_at            ISO-8601 UTC, millisecond precision
                           (WhatsApp minute precision → ":00.000")
  \u{1F} author_label      NFC, trimmed
  \u{1F} body_text         NFC; CRLF → LF; trailing whitespace stripped;
                           system messages included with empty author_label
                           distinguished by item_type
  \u{1F} media_hash        lowercase hex sha256 of primary attachment bytes,
                           or "-" when the message has none

external-key value = first 128 bits of SHA-256(canonical-input), lowercase hex
item content_hash  = sha256 over the canonical JSON of the full normalized
                     item content fields (RB §5.5 canonicalization)
```

Normative rules:

- **K1 — content equality is identity.** Two candidates with equal
  `external-key` and equal `content_hash` are the same message, wherever they
  appeared. This is deliberate: it makes convergence independent of chat
  renames, participant changes, deletions and export chunk boundaries.
- **K2 — media filenames never enter the key.** WhatsApp renumbers
  attachments per export; the key uses attachment _bytes_ (media hash), and
  the WhatsApp-side filename is retained only as `assets.original_filename`
  metadata (§7).
- **K3 — conflicts are surfaced, not merged.** Equal key, different hash
  (edited/deleted-and-resent edge cases) yields a `KeyConflict` shown in the
  wizard; the incoming variant is stored with a `-r2`, `-r3` … suffix on the
  external key, preserving both versions as separate addressable items.
- **K4 — importer versioning.** The `wa1-`/`man1-` prefixes are the key
  grammar version. A future grammar change ships a new prefix; old keys are
  never rewritten (stable identity, plan line 1151).
- **Known accepted limitation (complexity budget):** two _different_ chats
  containing byte-identical (minute-precision timestamp + author label +
  body + media) messages merge into one item. The alternative — chat-scoped
  keys — breaks convergence whenever chats are renamed between exports,
  which is strictly more common; rejected per the mission complexity budget.

### 6.2 No-op proof for re-import of an identical file

```text
Claim (P0): importing file F twice produces no visible message duplicates.
Strong form (fast path): the second run performs zero writes and leaves
the database byte-identical. Weak form (defense-in-depth path): even if
the fast-path lookup misses, the second run inserts zero source_items
and zero blobs.

Proof sketch:
  Stage 2 computes h(F) = sha256(F). By INV-4 the run is deterministic.
  Case a: a committed sources row with content_hash = h(F) exists
          → fast_no_op fires; stages 3–5 are not entered; zero writes
          (strong form holds).
  Case b (defense in depth, e.g. h(F) row absent but items present):
          stage 3 derives the same NormalizedBatch B(F);
          stage 4 classifies every item of B(F) as IdenticalDuplicate
          (same keys, same hashes — determinism);
          CommitPlan.new_items = ∅ and assets_to_store = ∅;
          stage 5 stores only the raw-original asset (C1) plus its
          sources/assets-metadata/change_log envelope rows (which case a
          normally prevents) — no source_items, no media blobs, hence no
          visible message duplication (weak form holds). □
```

Test hooks are mandatory (plan §14 lines 1876–1878): fixture "same file
twice" asserts `CommittedSource.imported_items == 0` on run 2 and an unchanged
row-count fingerprint; fixture "overlapping exports" asserts convergence
below.

### 6.3 Overlapping exports converge

Export B overlaps export A (B exported later, possibly after deletions):

- Messages present in both: equal canonical inputs → classified identical
  duplicates → skipped. They remain visible **once**, attributed to source A
  (the first importer); evidence links keep pointing at A's items.
- Messages only in B: imported as new items of source B.
- Messages only in A (deleted from the chat before B was exported): remain
  intact in source A — history is never retro-deleted.

Consequence: the visible message set after any sequence of imports equals the
**union** of message sets, partitioned across per-export source rows —
exactly the plan's convergence requirement. Enforcement points: stage-4
classification (advisory) **and** DB constraints (binding):

```sql
-- binding backstops (schema authority: task-0020; semantics fixed here)
UNIQUE (source_id, external_key)                 -- plan line 1262 (per-source)
UNIQUE (external_key) WHERE superseded_by IS NULL -- live item holds the key;
                                                  -- conflict suffixes (K3)
                                                  -- chain via superseded_by
```

If stage 5 hits a constraint violation despite stage 4 (concurrent import
won the race), K4 rolls back wholly and the service retries stage 4→5 once;
a second violation surfaces as `CommitRolledBack` (§9 entry IMP-E6).

### 6.4 Commit ordering and crash/failure windows (normative)

Stage 5 runs three phases in fixed order — bytes first, rows second
(mirrors AS §9.2 and RB K4's "after AssetStore byte placement succeeded"):

```text
C1  put raw original asset           (AS §5.2 six-step protocol)
C2  put media assets for NEW items   (AS §5.2, per file, hash-checked)
C3  KnowledgeStore.commit_source()   (RB unit of work K4: sources row,
                                     N × source_items, M × assets metadata,
                                     K × source_item_assets, FTS5 ride-along,
                                     one change_log row each — ONE SQLite
                                     transaction, rolled back entirely on
                                     any failure)
```

Failure/crash windows:

| Window                               | State left behind                                      | Resolution                                                                                                                                  |
| ------------------------------------ | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| during C1/C2 (put fails)             | earlier puts are orphan blobs                          | compensating `AssetStore::delete` over the recorded `PutResult` keys (best-effort, logged); run ends `AssetPutFailed` (IMP-E5)              |
| C2/C3 boundary, process crash        | orphan blobs, no DB rows                               | same compensating sweep at next service start; undeletable residue is reported, never adopted silently                                      |
| during C3 (tx fails)                 | C1/C2 blobs orphaned                                   | rollback (automatic) + compensation as above → `CommitRolledBack` (IMP-E6)                                                                  |
| during C3, process crash             | SQLite rolls back on restart (journal); blobs orphaned | startup reconciliation sweep (§6.5)                                                                                                         |
| after C3 commit, before result event | everything committed                                   | run resumes as success on retry — the idempotency key (below) makes re-entry return the existing `CommittedSource` instead of re-committing |

Tier-B `processing_jobs` protection against double-execution: the import job's
`idempotency_key := "import-wa1-" | "import-man1-" + first 128 bits of
sha256(file bytes) + "-" + target(project|inbox)` (plan line 1318: an
idempotency key prevents duplicate results). Before executing C1, the service
re-checks this key; a finished job with equal key short-circuits to its
recorded result.

### 6.5 Startup reconciliation sweep

At app start the service performs one cheap sweep: list `assets/` directories
with no corresponding `assets` metadata row and older than the staging TTL →
report via `change_log` + named log entry; deletion only after a grace period,
never mid-run of an active ingest run (active-run registry in process memory;
single-instance app makes this sound). This is the import-side analogue of
AS §7 reverse reconciliation.

---

## 7. Media linking to the AssetStore (G3)

### 7.1 Linkage rule

For every attachment of a **new** item:

1. Stage 3 extracts the attachment into staging and computes its
   `ContentHash` (SHA-256, AS §3.2).
2. Stage 5 (C2) stores it via `AssetStore::put(PutRequest { key:
"assets/<fresh asst_id>/original.<ext>", kind: AssetKind::Blob,
expected_hash: Some(media_hash), content: File(staged path) })` — the
   `asset-key` production of AS §2.2, with the extension-sniffed MIME type
   recorded in the `assets` metadata row alongside width/height/duration when
   detected (plan lines 1270–1271).
3. The link rows are `source_item_assets (source_item_id, asset_id)`
   (plan line 1272), created inside the same K4 transaction as the items.
   One medium may link to several items; one item may carry several media.
4. Downstream consumers (UI rendering, FTS snippets, later multimodal jobs)
   receive media exclusively as `asset_id` + relative `storage_key`;
   binary access goes through the AssetStore reader (RB rule R4). Frontends
   use the app's asset/serve channel, never filesystem paths (AS §2.1,
   RB R5).

### 7.2 No-duplicate-blobs corollary

Media is stored **only for items in `new_items`**. Skipped duplicate items
keep pointing at the assets stored by the first import (their
`source_item_assets` rows already exist there). Together with the P0
short-circuit this yields: identical re-import → zero new blobs; overlapping
import → blobs only for genuinely new attachments.

### 7.3 No absolute paths — normative statement

No persisted record of the import domain — `sources`, `source_items`,
`assets`, `source_item_assets`, `change_log`, events, logs, exports — may
contain an absolute filesystem path. Stored location identity is exactly
`(asset_id, storage_key)` with `storage_key` matching the AS §2.2 grammar
(plan lines 1158–1159: "keine absoluten Dateipfade als dauerhafte Identität;
Datenbank speichert Asset-ID und relativen Storage-Key"). The only code that
turns a key into a path is `AssetStore::resolve` (AS §2.1 rule 2). Violations
are mechanically greppable and belong in the task-0020 static-gate checklist.

### 7.4 Hand-off to post-commit analysis

After C3, the service enqueues tier-B `processing_jobs` (media analysis,
multimodal understanding — only for consented items) carrying asset **IDs**
and the modalität requirements; those jobs re-read bytes via
`AssetStore::get(key, ReadVerification::Full)` (verification point V2/V4
pattern, AS §3.2). Cloud transfer accounting per plan lines 1473–1474 is
written by those jobs, not by ingestion.

---

## 8. Generic manual-file import mapping

`ManualFileImporter` implements the identical trait with degenerate stages:

| Stage       | Specialization                                                                                                                    |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------- |
| inspect     | MIME/type sniff, size cap, no container expansion                                                                                 |
| preview     | single-card outline: detected type, size, hash, optional text-extraction feasibility; `fast_no_op` via file hash                  |
| normalize   | one `NormalizedItem` (or section-split items for documents, task-0054 scope) with `man1-` keys; whole-file/media hash = file hash |
| deduplicate | identical machinery, same classification table                                                                                    |
| commit      | identical C1–C3; raw asset is the file itself                                                                                     |

No second persistence shape, no second dedup rule — plan line 133: after
ingestion, meeting, WhatsApp, file and later connectors share one source
model.

---

## 9. Preview, progress, and error states → UX copy and commands (G4)

New top-level i18n namespace `import` (these surfaces belong to the
Projects/import product area — plan §5 component
`src/components/import/WhatsAppImportWizard.tsx` — not to `meeting.*`);
catalog entry IDs extend the UX-copy catalog's numbering with the `IMP-`
series. All entries are proposals (catalog status `proposal-only`); none
exists in `src/i18n/locales/en/translation.json` today.

### 9.1 Wizard step and progress states

| Catalog ID | State / trigger                           | Proposed i18n key               | English text (proposal)                                                      |
| ---------- | ----------------------------------------- | ------------------------------- | ---------------------------------------------------------------------------- |
| IMP-S1     | Step 1: file chosen, staging copy running | `import.wizard.step1.staging`   | `Checking the file…`                                                         |
| IMP-S2     | Step 2 header: detected summary           | `import.wizard.step2.title`     | `Review what was found`                                                      |
| IMP-I1     | Info: N chats, M messages, time span      | `import.preview.summary`        | `{{chats}} chats · {{messages}} messages · {{media}} media · {{span}}`       |
| IMP-I2     | Info: duplicate estimate > 0              | `import.preview.duplicates`     | `{{count}} messages already exist from earlier imports and will be skipped.` |
| IMP-I3     | Info: key conflicts detected              | `import.preview.conflicts`      | `{{count}} messages changed between exports. Both versions will be kept.`    |
| IMP-P1     | Progress: parsing (indeterminate count)   | `import.progress.parsing`       | `Reading messages… ({{processed}})`                                          |
| IMP-P2     | Progress: copying media (determinate)     | `import.progress.media`         | `Saving media {{done}}/{{total}}`                                            |
| IMP-P3     | Progress: committing (indeterminate)      | `import.progress.committing`    | `Saving to your library…`                                                    |
| IMP-R1     | Result summary after commit               | `import.result.summary`         | `Imported {{imported}} · skipped {{skipped}} · needs review {{conflicts}}`   |
| IMP-R2     | Result: already imported (P0 path)        | `import.result.alreadyImported` | `This export is already in your library. Nothing was duplicated.`            |

### 9.2 Error and failure states

Every error offers a concrete next action (plan §6.2 rule 8); shared buttons
reuse `meeting.actions.*` patterns under `import.actions.*`
(`tryAgain`, `chooseOtherFile`, `openSettings`, `cancel`).

| Catalog ID | Trigger (error variant)                              | Proposed i18n key                 | Next action offered                                     |
| ---------- | ---------------------------------------------------- | --------------------------------- | ------------------------------------------------------- |
| IMP-E1     | `SafetyRejected` — traversal/symlink/absolute member | `import.error.unsafeArchive`      | `chooseOtherFile`                                       |
| IMP-E2     | `SafetyRejected` — size/count/ratio limit            | `import.error.tooLarge`           | `chooseOtherFile`                                       |
| IMP-E3     | `SafetyRejected` — encrypted/unreadable container    | `import.error.cannotOpen`         | `chooseOtherFile`                                       |
| IMP-E4     | `UnparsableFormat`                                   | `import.error.unrecognizedFormat` | `chooseOtherFile`                                       |
| IMP-E5     | `AssetPutFailed`                                     | `import.error.storageFailed`      | `tryAgain` (resume at failed phase via idempotency key) |
| IMP-E6     | `CommitRolledBack`                                   | `import.error.commitFailed`       | `tryAgain`                                              |
| IMP-E7     | `CommitOrphanedAssets`                               | `import.error.cleanupPending`     | `close` (background sweep finishes)                     |
| IMP-E8     | `EmptyImport`                                        | `import.error.empty`              | `chooseOtherFile`                                       |
| IMP-E9     | `StagingIo` — disk full / staging IO failure         | `import.error.diskFull`           | `tryAgain` after freeing space; `openSettings`          |
| IMP-W1     | Warning banner: k malformed items tolerated          | `import.warning.partialParse`     | continue / cancel (both explicit)                       |

### 9.3 Future commands and events

Proposed Tauri command surface (tauri-specta bindings, RB rule R3 — typed
values only, no paths):

```text
import_begin(pickerHandle)        -> { runId }                    ; copies into staging
import_inspect(runId)             -> InspectReport                ; wizard step 1
import_preview(runId, options)    -> ImportPreview                ; wizard step 2
import_commit(runId, decision)    -> CommittedSource | ImportError; wizard step 4
import_cancel(runId)              -> {}                           ; triggers staging cleanup
```

Events: `import://{runId}/progress` (stage, processed/total where known —
never a fabricated percentage, plan line 656 analogy) and
`import://{runId}/finished`. All user-visible payloads render through the
§9.1/§9.2 catalog entries; backend errors cross the seam as the typed
`IngestionError` discriminant, and the frontend maps discriminant → i18n key.

---

## 10. Verification hooks (maps to plan §14)

Automated tests this contract makes mandatory (fixtures themselves are
task-0040's deliverable):

1. **No-op:** same `.zip` imported twice — run 2 asserts
   `alreadyImported` fast path taken, DB fingerprint unchanged, zero new
   blob keys (P0, case a) and, with the `sources.content_hash` row removed,
   still zero items/blobs (case b).
2. **Overlap:** export A then overlapping B — union property asserted;
   skipped items' evidence links resolve; A-only messages survive.
3. **Determinism:** stage 3 output byte-equal across two runs on identical
   staging input (INV-4 regression guard).
4. **Conflict:** crafted same-key/different-hash pair → `KeyConflict`,
   `-r2` item stored, both retrievable.
5. **Crash windows:** fault-injected fs/DB seams per §6.4 table — each row's
   promised end state asserted, including compensation deletes and the
   idempotency-key short-circuit.
6. **No absolute paths:** serialize every persisted row of a full import and
   assert no string field matches a drive-letter/rooted-path pattern; keys
   round-trip `StorageKey::parse` (AS §2.2).
7. **Safety rejection:** every `SafetyRejection` reason produces its IMP-E
   mapping and leaves zero DB rows and no leftover staging directory.

---

## Appendix A — Gate traceability

- **G1** (five stages, typed I/O, failure semantics, transaction/
  compensation boundary): §3 typed interface and error enum; §4.1–§4.5
  per-stage tables; §6.4 commit phases and crash-window table.
- **G2** (dedup key grammar, provable no-op, overlap convergence, release
  gate cited): §6.1 grammar and rules K1–K4; §6.2 proof P0 citing plan line
  872 (and Phase-9 acceptance line 1687, DoD lines 1974–1975); §6.3
  convergence + binding constraints; §10 tests 1–4.
- **G3** (media linkage onto AssetStore storage-key scheme, no absolute
  paths): §7.1–§7.3, citing AS §2.2 `asset-key`, AS §3.2 hashing, AS §5.2
  protocol, plan lines 1158–1159; §10 test 6.
- **G4** (every preview/progress/error state names catalog entry ID and
  i18n path): §9.1–§9.3 tables covering all states of §4 and all
  `IngestionError` variants.
- **G5** (prettier-clean, single-file change): verified via
  `bun run prettier --check` on this file and `git status --porcelain`
  showing only `docs/meeting-mode-architecture/import-pipeline-dataflow.md`
  (see mission report).

Scope note for the documentation index: per the index's own maintenance rule
(§5) this document belongs in the §3 table; the index file is outside this
leaf's declared conflict scope and is therefore left to the supervisor's
integration pass.
