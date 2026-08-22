# SourceIngestionService Stage Interface & Normalized-Source Contract

Status: **spec contract — no implementation yet** (Phase 9 prerequisite; changes
no source code)

Plan anchors: [docs/meeting-mode-plan.md](../meeting-mode-plan.md)

- §5 "Zielarchitektur": `SourceIngestionService` (plan lines 277–283: _„Jeder
  Importer implementiert `inspect`, `preview`, `normalize`, `deduplicate` und
  `commit`; nachgelagerte Suche und KI kennen das ursprüngliche Transportformat
  nicht."_) and `WhatsAppExportImporter` (plan lines 285–291)
- Phase 9 (plan lines 1671–1692): safe ZIP/TXT inspection, preview, hashing +
  idempotent re-import, local FTS, `SourceIngestionService` and persistent
  import/analysis jobs; acceptance plan line 1687 (_„derselbe oder überlappende
  Export erzeugt keine sichtbaren Dubletten"_)
- §9 data model: `sources` (plan lines 1227–1239), `source_items`
  (plan lines 1253–1266), `assets` / `source_item_assets`
  (plan lines 1268–1275), `processing_jobs` (plan lines 1313–1318),
  no-absolute-path rule (plan lines 1158–1159)
- Release gate (plan line 872) and §12 untrusted-input auditability
  (plan lines 1463–1476)

Sibling contracts consumed read-only:

- [import-pipeline-dataflow.md](import-pipeline-dataflow.md) (**IF**) —
  end-to-end dataflow, crash-window table (IF §6.4), UX-copy mapping (IF §9),
  dedup-key grammar origin (IF §6.1). This document **fixes the typed seam**
  those flows run through; where IF describes orchestration, this document
  describes the interface the orchestrator calls.
- [asset-store-contract.md](asset-store-contract.md) (**AS**) — storage-key
  grammar (AS §2.2), `AssetId` / `ContentHash` schemes (AS §3), six-step
  atomic put (AS §5.2), single resolution point (AS §2.1 rule 2).
- [stt-provider-contract.md](stt-provider-contract.md) (**STT**) — persistent
  job state machine (STT §3), reused verbatim for `processing_jobs` (§9 here).
- [repository-boundary-contract.md](repository-boundary-contract.md) (**RB**) —
  unit of work K4 (`KnowledgeRepository::commit_source`), SQL confinement R1/R4.

Task-scope citations (leaves authored outside this worktree; cited, not
duplicated):

- **task-0024** — AssetStore storage-key discipline; honored in §8 here.
- **task-0028** — adversarial/overlap fixture design intent for the dedup
  proofs; honored in §7.4 here.
- **task-0040** — security validation rules (ZIP safety corpus, limits);
  pointed to, never re-specified (§10, gate G5).

Precedence (documentation index §4): plan beats contracts, contracts beat
catalogs, nothing beats real code. Where this document and IF overlap, this
document governs the **type-level seam** and IF the **flow-level behavior**;
a genuine contradiction is a defect to be registered in the identifier
registry, not silently resolved either way.

---

## 1. Scope

### 1.1 In scope

1. A compilable-shape Rust trait for the five-stage pipeline
   (`inspect`, `preview`, `normalize`, `deduplicate`, `commit`) with per-stage
   input/output types and failure semantics (§3, §4).
2. The mutation rule: **exactly one stage — `commit` — mutates stores**, and it
   is the single transactional point of the whole pipeline (INV-1, §4.5).
3. The **normalized-source record** — the only shape downstream FTS indexing
   and AI analysis consume — with the transport-format firewall and its
   negative-testability rule (§6).
4. Dedup-key derivation rules with worked-example proofs for (a) identical
   re-import as a no-op and (b) overlapping exports converging without visible
   duplicates (§7), aligned with task-0028's fixture design intent.
5. Media/asset hand-off to the AssetStore using **only relative storage keys**
   per task-0024 / AS §2.2 (§8).
6. Persistent import/analysis job semantics reusing the
   stt-provider-contract job state machine (§9).

### 1.2 Out of scope

| Concern                                                                                                        | Owner          | Relationship to this document                                                                                      |
| -------------------------------------------------------------------------------------------------------------- | -------------- | ------------------------------------------------------------------------------------------------------------------ |
| Security validation rules: traversal/symlink/bomb/encryption criteria, limit values, malicious-corpus fixtures | **task-0040**  | cited only (§10) — this contract names the enforcement points and the opaque `SafetyVerdict` carrier, nothing else |
| Orchestration, crash-window table, compensation sweeps, startup reconciliation                                 | IF §6.4–§6.5   | referenced; §4.5 restates the ordering normatively because it is the commit stage's own semantics                  |
| SQL DDL and uniqueness constraints                                                                             | task-0020 / RB | this document constrains semantics, not schema text                                                                |
| WhatsApp format-variant grammars (date/locale/multiline parsing)                                               | task-0054      | consumed behind `parser_version`; §3's trait makes the parser swappable without touching callers                   |
| UX copy, wizard steps, Tauri command surface                                                                   | IF §9          | not repeated here                                                                                                  |

---

## 2. Pipeline shape and invariants

```text
StagedUpload ─▶ inspect ─▶ preview ─▶ normalize ─▶ deduplicate ─▶ commit
               (safety     (read      (pure        (pure reads     (ONLY writer:
                verdict)    only)      function)    vs. committed)  AssetStore bytes
                                                                    + one SQLite
                                                                    unit of work)
```

Normative invariants (restated at the seam; flow-level originals in IF §2):

- **INV-1 (single write boundary).** `inspect`, `preview`, `normalize` and
  `deduplicate` never mutate the KnowledgeStore, the AssetStore or any job
  row. All durable effects happen inside `commit`, exactly once, in one
  ordered sequence (§4.5). Gate G1.
- **INV-2 (side-effect-free preview).** `preview` performs no writes at all —
  no rows, no files outside staging, no events. Long-parse survivability is
  achieved by the **service wrapper** persisting a tier-B `processing_jobs`
  row _before_ invoking `preview` and updating it _after_ it returns (§9.3);
  the stage itself stays a pure query. (This refines IF §4.2, which placed the
  tier-B row insert "optionally" in stage 2: the row belongs to the wrapper,
  so the stage contract can guarantee purity.)
- **INV-3 (determinism).** Given the same staged bytes and the same committed
  database state, `preview`, `normalize` and `deduplicate` produce equal
  outputs. This is what makes the §7.2 no-op proof possible.
- **INV-4 (transport firewall).** Nothing downstream of `normalize` — FTS
  indexing, AI analysis, UI lists, exports — sees a WhatsApp-, Teams- or
  transport-specific field, type or filename pattern. Enforced mechanically
  per §6.3. Gate G2.

---

## 3. Stage-interface trait (G1)

The following Rust sketch compiles standalone (std only) via the shared
harness (Appendix B). Type spellings owned by sibling contracts are repeated
here as opaque newtypes with their authoritative home in a doc comment; the
sketch defines no behavior (`unimplemented!()`-free: method bodies live in the
trait's default-free declarations, i.e. the sketch declares signatures only).

```rust
//! Sketch of the SourceIngestionService seam (plan lines 277–291).
//! Std-only; compiles via docs/meeting-mode-architecture/verify-sketch.ps1.

// ---------- Opaque stand-ins for sibling-contract-owned types ----------
// (authoritative definitions live in the cited sections; duplicated here
//  ONLY as minimal newtypes so this sketch is self-contained.)

/// Authoritative: AS §3.2 — lowercase `sha256-<64 hex>` over exact bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentHash(pub String);

/// Authoritative: AS §5.1 — relative forward-slash path, valid per the
/// AS §2.2 grammar, checked by `StorageKey::parse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageKey(pub String);

/// Authoritative: AS §3.1 — `asst_<uuid-v4>`, never content-derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetId(pub String);

/// Semver string of the parser variant set in use (plan line 1233:
/// `parser_version`). Kept as a token here so the sketch needs no crates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParserVersion(pub String);

// ---------- Run and staging types ----------

/// Process-local identity of one quarantined ingest run. Never persisted as
/// identity; the frontend holds only this opaque value (RB rule R5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestRunId(pub String);

/// Stage-1..3 input. `staging_path` points into the run's isolated temp
/// directory created by the service before `inspect` (plan lines 1467–1468);
/// it is process-local and deleted when the run ends. It is NEVER persisted
/// and NEVER handed to `commit` — commit receives hashes and staging-relative
/// paths only (see `AssetPutRequest`).
#[derive(Debug)]
pub struct StagedUpload {
    pub run: IngestRunId,
    pub staging_path: std::path::PathBuf,
    /// Display/provenance metadata only; never part of any key (AS §2.2).
    pub original_filename: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImporterKind {
    WhatsAppZipTxt,
    ManualFile,
}

// ---------- Stage 1: inspect ----------

/// Container sniff result. Safety criteria and limit values are task-0040's
/// scope; this type carries only the verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    ZipArchive,
    PlainText,
    Other,
}

/// Opaque safety verdict. `Rejected` carries the task-0040 rule identifier so
/// the UI can render the matching copy entry, but this contract neither
/// defines nor interprets any rule (gate G5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SafetyVerdict {
    Safe,
    Rejected { rule_id: String },
}

pub struct InspectReport {
    pub container: ContainerKind,
    pub entry_count: u32,
    pub total_bytes: u64,
    /// Compression-ratio observation, evaluated against task-0040 limits.
    pub max_ratio_observed: f64,
    pub safety: SafetyVerdict,
}

// ---------- Stage 2: preview ----------

pub struct ChatOutline {
    /// Display label as found; never a dedup input (dedup keys are
    /// content-derived, §7 rule K1 — chat renames must not break convergence).
    pub display_name: String,
    pub participant_labels: Vec<String>,
    pub first_occurred_at: String,
    pub last_occurred_at: String,
    pub message_estimate: u64,
    pub media_estimate: u64,
}

pub struct MediaOutline {
    pub content_hash: ContentHash,
    pub mime_hint: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Non-fatal parse finding; individual malformed items degrade to these
/// (plan lines 813–815: defensive parsing, missing attachments tolerated).
pub struct ParseWarning {
    /// Stable warning code owned by the importer/parser version.
    pub code: String,
    pub item_sequence: Option<u64>,
    pub detail: String,
}

pub struct DuplicateEstimate {
    pub identical: u64,
    pub conflicts: u64,
}

/// Reference to an already-committed source whose archive content hash equals
/// the staged file's hash — the fast no-op path P0 of §7.2.
pub struct ExistingSourceRef {
    pub source_id: String,
    pub title: String,
    pub committed_at: String,
}

pub struct ImportPreview {
    pub importer: ImporterKind,
    pub parser_version: ParserVersion,
    pub chats: Vec<ChatOutline>,
    pub media: Vec<MediaOutline>,
    pub warnings: Vec<ParseWarning>,
    pub duplicate_estimate: DuplicateEstimate,
    /// Set iff a committed `sources.content_hash` equals the staged file's
    /// SHA-256; the wizard then skips steps 3–4 entirely (INV: preview found
    /// this by READ-ONLY lookup, §4.2).
    pub fast_no_op: Option<ExistingSourceRef>,
}

// ---------- Stage 3: normalize — the normalized-source record ----------

/// Item-type vocabulary of the normalized source model (plan line 1258).
/// Transport-neutral: "ChatMessage" covers WhatsApp, Teams, Discord, Skype …
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemType {
    ChatMessage,
    TranscriptSegmentRef,
    DocumentSection,
    Note,
}

/// Distinguishes user-authored from system-generated items. Generic across
/// transports; the WhatsApp-specific rendering of system lines never
/// survives `normalize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    System,
}

/// Attachment reference, pre-commit form: content-addressed only. Bytes live
/// in staging until `commit` places them (§8); nothing here is a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRef {
    /// SHA-256 over the attachment bytes; part of dedup identity (§7 K2).
    pub content_hash: ContentHash,
    pub mime_hint: Option<String>,
    pub size_bytes: Option<u64>,
}

/// THE normalized-source record (gate G2). This is the only item shape that
/// crosses the seam into FTS indexing and AI analysis. Field rules in §6.2;
/// transport-firewall enforcement in §6.3.
#[derive(Clone, Debug)]
pub struct NormalizedItem {
    /// Dedup identity per §7; e.g. `wa1-<32 hex>` / `man1-<32 hex>`.
    pub external_key: String,
    pub item_type: ItemType,
    pub role: MessageRole,
    /// NFC, trimmed; `None` for system-role items without an actor label.
    pub author_label: Option<String>,
    /// ISO-8601 UTC, millisecond precision (minute-precision parser variants
    /// normalize to `:00.000`, mirroring IF §6.1).
    pub occurred_at: String,
    /// Ordering position within the source; explicitly NOT identity.
    pub sequence: u64,
    /// NFC text; CRLF→LF, trailing whitespace stripped.
    pub text: String,
    /// SHA-256 over the canonical serialization of every content field above
    /// (canonicalization owner: RB §5.5).
    pub content_hash: ContentHash,
    pub media: Vec<MediaRef>,
    /// Bounded extension point: `(key, value)` pairs from a closed,
    /// transport-neutral vocabulary registered per parser version (e.g.
    /// `duration_ms`, `image_width`, `geo_lat`). Free-form transport payloads
    /// are forbidden here — see §6.3's negative test.
    pub metadata: Vec<(String, String)>,
}

pub struct NormalizedBatch {
    pub importer: ImporterKind,
    pub parser_version: ParserVersion,
    pub items: Vec<NormalizedItem>,
    /// Extracted attachments still living in staging (hash-stamped).
    pub staged_media: Vec<StagedMedia>,
    pub warnings: Vec<ParseWarning>,
}

pub struct StagedMedia {
    pub media: MediaRef,
    /// Path INSIDE the run's staging directory, relative to its root.
    /// Resolved only against that temp root during `commit`; never persisted
    /// (INV-2 of IF; plan lines 1158–1159).
    pub staging_relative_path: String,
}

// ---------- Stage 4: deduplicate ----------

/// Read-only port over committed `source_items`. Implemented by the
/// KnowledgeStore; handed to `deduplicate` so the stage provably cannot write.
pub trait CommittedItemIndex {
    fn find_by_external_key(&self, external_key: &str) -> Option<ExistingItemRef>;
    /// Count of live items sharing this key across supersession chains
    /// (conflict suffixes, §7 rule K3).
    fn count_live_by_external_key(&self, external_key: &str) -> u64;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingItemRef {
    pub source_id: String,
    pub item_id: String,
    pub content_hash: ContentHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateClass {
    New,
    IdenticalDuplicate,
    KeyConflict,
}

pub struct KeyConflict {
    /// Incoming candidate with equal external_key, different content_hash.
    pub incoming: NormalizedItem,
    pub existing: ExistingItemRef,
}

/// The exact plan `commit` will execute — nothing else (INV-1: produced by a
/// read-only stage, executed by the only writing stage).
pub struct CommitPlan {
    pub importer: ImporterKind,
    pub parser_version: ParserVersion,
    pub new_items: Vec<NormalizedItem>,
    /// Candidates classified IdenticalDuplicate; commit touches nothing for
    /// these (their media is not re-stored, §8.2).
    pub duplicate_skips: Vec<ExistingItemRef>,
    pub conflicts: Vec<KeyConflict>,
    /// Media of NEW items only (§8.2 corollary).
    pub media_to_store: Vec<StagedMedia>,
    /// Idempotency key for the tier-B job protecting commit (§9.4).
    pub idempotency_key: String,
}

// ---------- Stage 5: commit ----------

/// Write ports handed ONLY to `commit`. No other stage can obtain one; the
/// service constructs the sink for the duration of stage 5 exclusively.
pub trait CommitSink {
    /// Place one blob into the AssetStore under a fresh
    /// `assets/<asst_id>/original.<ext>` key (AS §2.2 `asset-key` production),
    /// hash-verified against `expected_hash` at AS verification point V1.
    /// Returns the stored identity; NEVER accepts or returns absolute paths.
    fn put_asset(&mut self, request: AssetPutRequest) -> Result<StoredMedia, IngestionError>;
    /// The single transactional point: RB unit of work K4 — sources row,
    /// N × source_items, M × assets metadata, K × source_item_assets, FTS5
    /// ride-along, change_log rows — ONE SQLite transaction, rolled back
    /// wholly on any failure. Returns the fresh source id.
    fn commit_source_rows(
        &mut self,
        plan: &CommitPlan,
        stored: &[StoredMedia],
        decision: &CommitDecision,
    ) -> Result<String, IngestionError>;
}

pub struct AssetPutRequest {
    /// Relative to the run's staging root; the sink streams bytes from there.
    pub staging_relative_path: String,
    pub expected_hash: ContentHash,
    /// ≤ 8 chars alnum extension for the `original.<ext>` key component.
    pub ext: String,
}

pub struct StoredMedia {
    pub asset_id: AssetId,
    /// Relative storage key per AS §2.2. Never an absolute path (gate G4).
    pub storage_key: StorageKey,
}

/// User decisions captured in wizard step 3 (plan lines 807–809); carried
/// into the sources row and post-commit job gating.
pub struct CommitDecision {
    /// Project UUID or inbox sentinel.
    pub target_project: String,
    /// Retention-rule token chosen by the user (plan line 1476).
    pub retention_rule: String,
    /// Explicit consent flag for any later cloud processing of this source.
    pub cloud_consent: bool,
}

pub struct CommittedSource {
    pub source_id: String,
    pub raw_asset_id: AssetId,
    pub imported_items: usize,
    pub skipped_duplicates: usize,
    pub conflicts: usize,
    pub stored_media_assets: usize,
}

// ---------- Typed errors (plan §6.2 rule 8: never raw backend strings) -----

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestionError {
    /// Stage 1; `rule_id` resolves to task-0040's catalog. Terminal for the
    /// run — damaged/encrypted archives are rejected without partial import
    /// (plan line 1466).
    SafetyRejected { rule_id: String },
    /// Stage 2: readable container but no parser variant matches at all.
    UnparsableFormat { parser_version: ParserVersion },
    /// Stage 2: zero usable items (empty export).
    EmptyImport,
    /// Stage 5: an AssetStore put refused or failed mid-sequence.
    AssetPutFailed { attempted_keys: Vec<String> },
    /// Stage 5: K4 rolled back wholly (constraint violation or tx error).
    CommitRolledBack,
    /// Any stage: staging IO failure / disk full.
    StagingIo,
}

// ---------- The trait (plan lines 279–281) -------------------------------

/// One importer = one implementation. Planned implementations:
/// `WhatsAppExportImporter` (src-tauri/src/importers/whatsapp.rs, plan line
/// 291) and `ManualFileImporter` (same file, IF §1.1).
///
/// MUTATION RULE (gate G1): of these five methods, ONLY `commit` mutates any
/// store — KnowledgeStore rows, AssetStore bytes, or job rows. The four
/// earlier stages receive read-only inputs (`&` references, `&dyn
/// CommittedItemIndex`) and return values; the compiler-enforced shape makes
/// a writing stage-1..4 signature a contract violation.
pub trait SourceIngestionPipeline: Send + Sync {
    /// Stage 1. Pure read over staged bytes; no DB, no asset writes.
    fn inspect(
        &self,
        upload: &StagedUpload,
    ) -> Result<InspectReport, IngestionError>;

    /// Stage 2. Side-effect-free (INV-2): no writes, no events; the
    /// `fast_no_op` lookup is a read-only `sources.content_hash` query.
    fn preview(
        &self,
        upload: &StagedUpload,
        inspected: &InspectReport,
    ) -> Result<ImportPreview, IngestionError>;

    /// Stage 3. Pure function over (staged bytes, parser code,
    /// parser_version) — deterministic per INV-3; per-item problems degrade
    /// to warnings, never silent drops (plan line 1691: unsupported media is
    /// retained and marked, never dropped).
    fn normalize(
        &self,
        upload: &StagedUpload,
        previewed: &ImportPreview,
    ) -> Result<NormalizedBatch, IngestionError>;

    /// Stage 4. Pure reads against committed items via the index port.
    /// Cannot fail on data: worst case is a non-empty `conflicts` vector.
    fn deduplicate(
        &self,
        batch: &NormalizedBatch,
        committed: &dyn CommittedItemIndex,
    ) -> Result<CommitPlan, IngestionError>;

    /// Stage 5. The single mutating, transactional stage. Executes the
    /// normative C1–C3 order of §4.5. Re-entry after a crash-after-commit
    /// short-circuits on `plan.idempotency_key` (§9.4) instead of
    /// re-committing.
    fn commit(
        &self,
        plan: &CommitPlan,
        decision: &CommitDecision,
        sink: &mut dyn CommitSink,
    ) -> Result<CommittedSource, IngestionError>;
}
```

Failure semantics summary (per stage; flow-level crash windows remain IF §6.4):

| Stage         | Fatal conditions                                                                                   | Non-fatal degradation                                                           | Side effects                                                  |
| ------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `inspect`     | `SafetyRejected` (any task-0040 rule) terminates the run with zero partial import (plan line 1466) | none                                                                            | none (run-scoped staging cleanup is the service's RAII guard) |
| `preview`     | `UnparsableFormat`, `EmptyImport`                                                                  | per-item `ParseWarning`s                                                        | **none** (INV-2)                                              |
| `normalize`   | none once stage 2 succeeded                                                                        | per-item warnings; lossy-decoded text imports with a warning                    | none (pure)                                                   |
| `deduplicate` | none on data; transport-class failure ends the run before any write                                | `conflicts` non-empty is data, not failure                                      | none (reads via `&dyn CommittedItemIndex`)                    |
| `commit`      | `AssetPutFailed`, `CommitRolledBack`, `StagingIo`                                                  | conflicts are committed as `-r2` suffixed items (§7 K3), surfaced in the result | **all durable effects of the run**                            |

---

## 4. The commit stage as the single transactional point (normative order)

Restated from IF §6.4 because it is the `commit` method's own contract; on any
discrepancy between the two texts this order wins for the stage signature:

```text
C1  sink.put_asset(raw original)          — AS §5.2 six-step protocol,
                                            expected_hash = archive hash
C2  sink.put_asset(media of NEW items)    — per file, hash-checked
C3  sink.commit_source_rows(plan, stored) — ONE SQLite transaction (RB K4):
                                            sources + source_items +
                                            assets metadata +
                                            source_item_assets + FTS5 +
                                            change_log
```

Rules:

1. Bytes before rows (mirrors AS §9.2): a crash inside C1/C2 leaves orphan
   blobs and zero rows; compensation is IF §6.4's sweep, not this trait's
   concern. A crash inside C3 rolls back wholly — SQLite journal recovery —
   leaving the same orphan blobs and no rows.
2. There is no state „rows committed without bytes“ and none „bytes committed
   twice for the same logical run“: re-entry after a successful C3 is caught
   by the idempotency key (§9.4) and returns the recorded result instead of
   executing C1 again.
3. `commit` is the **only** place in the entire import domain where a SQLite
   transaction and an AssetStore write are both reachable. This is checkable
   in review: any code path outside `SourceIngestionPipeline::commit` holding
   a `CommitSink` is a defect.

---

## 5. Per-stage ownership map

| Concern            | `inspect`                 | `preview`                          | `normalize`   | `deduplicate`                    | `commit`                   |
| ------------------ | ------------------------- | ---------------------------------- | ------------- | -------------------------------- | -------------------------- |
| Reads staged bytes | ✔                        | ✔                                 | ✔            | –                                | via sink only              |
| Reads committed DB | –                         | `sources.content_hash` only        | –             | ✔ via index port                | implicit (K4)              |
| Writes anything    | ✗                         | ✗                                  | ✗             | ✗                                | ✔ (sole)                  |
| Owner (plan §5)    | service + importer        | importer                           | importer      | service logic, importer-agnostic | service orchestrating sink |
| Failure class      | `SafetyRejected` terminal | `UnparsableFormat` / `EmptyImport` | warnings only | transport errors only            | rollback + compensation    |

The importer-kind split follows the plan literally (plan line 280: every
importer implements all five), while cross-importer machinery — the index
port implementation and the `CommitSink` construction — lives once in
`SourceIngestionService` (planned home
`src-tauri/src/managers/source_ingestion.rs`, plan line 283), so a second
importer cannot fork dedup or commit semantics (IF §4.4 agrees: classification
machinery is cross-importer logic in the service).

---

## 6. The normalized-source record — transport firewall (G2)

### 6.1 Downstream consumers and what they may know

| Consumer                                   | May use                                                                                                                | Must never see                                                          |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| FTS5 indexing (plan lines 1327–1332)       | `item_type`, `author_label`, `occurred_at`, `sequence`, `text`, `metadata` pairs, evidence link `(source_id, item_id)` | any transport field; indexing is over `NormalizedItem` projections only |
| AI analysis / `ProjectIntelligenceService` | the same projection, plus `MediaRef.content_hash` for consented multimodal jobs (IF §7.4)                              | transport formats, original filenames as structure, chat-export quirks  |
| UI lists / evidence jumps                  | `(source_id, item_id, sequence, occurred_at)` for jump targets                                                         | transport-specific rendering hints                                      |

Concretely (plan line 281: _„nachgelagerte Suche und KI kennen das
ursprüngliche Transportformat nicht"_): a future Teams or email importer adds
an implementation of `SourceIngestionPipeline` and, at most, new entries in
the closed `metadata` vocabulary — **zero changes** to FTS code, AI prompts,
or UI item rendering.

### 6.2 Field rules

1. `external_key` is identity; `sequence` is presentation order only. Two
   records with equal `external_key` ARE the same message regardless of
   position (§7 K1).
2. `occurred_at` is always ISO-8601 UTC ms — timezone and locale conversion
   happened in `normalize` and is unit-tested per parser version (task-0054).
3. `author_label` is a display string, never a protocol address. Transport
   identifiers (phone-number JIDs, team handles, e-mail addresses) must be
   reduced to display labels or dropped in `normalize`; storing a raw JID in
   `metadata` violates rule 4.
4. `metadata` is a closed vocabulary: every `(key, value)` pair must be
   registered in the parser-version's vocabulary table. Unregistered keys are
   a `ParseWarning` at stage 3 and are dropped before stage 4 — this is the
   pressure valve that keeps the record transport-neutral without freezing
   the schema.
5. Media is referenced only as `MediaRef` (content hash + inert hints);
   filenames of attachments exist solely in `assets.original_filename`
   (provenance display, plan line 1270), never in `source_items`.

### 6.3 Negative testability (mandatory, mechanical)

The firewall is enforced by two automated checks (plan §14 hooks; harness
details are task-0028/task-0040 fixture territory — the assertions themselves
are specified here because they guard THIS boundary):

1. **Symbol scan:** outside `src-tauri/src/importers/**`, no Rust identifier
   (type, field, module, function) matches
   `(?i)(whatsapp|wa[_-]|jid|vcard|oom_|_export_file)` — with the documented
   exceptions of the `ImporterKind::WhatsAppZipTxt` variant, the
   `sources.source_type` token `"whatsapp_export"` (a plan-mandated
   discriminator, plan line 1229), and `assets.original_filename` values.
   Rationale: those three are inert discriminators/provenance; everything
   else structural must be transport-blind.
2. **Serialization scan:** during the §10 integration tests, serialize every
   persisted row (`sources`, `source_items`, `assets`,
   `source_item_assets`, `change_log`), every event payload and every FTS
   document produced by a full import, and assert no string field contains a
   WhatsApp attachment-filename pattern (e.g. `IMG-\d{4}-WA\d{4}`,
   `PTT-\d{8}-WA\d{4}`, `.opus`, `.webp` attachment naming) or a JID-shaped
   token (`\d+@\w+\.\w+`), except the two allowlisted inert fields above.

A failing assertion is a release blocker for Phase 9 — it means a transport
detail leaked past `normalize`.

---

## 7. Dedup-key derivation (G3)

### 7.1 Grammar and derivation

Adopted from IF §6.1 verbatim (single source of truth; repeated here only as
the derivation contract of `NormalizedItem.external_key`):

```text
external-key  := "wa1-" 32HEXDIG | "man1-" 32HEXDIG

canonical-input (UTF-8, "\u{1F}" separator):
    occurred_at            ISO-8601 UTC ms (minute-precision variants → ":00.000")
  \u{1F} author_label      NFC, trimmed (empty for system-role items)
  \u{1F} body_text         NFC; CRLF→LF; trailing whitespace stripped
  \u{1F} media_hash        lowercase hex sha256 of primary attachment bytes, "-" if none

external-key value = first 128 bits of SHA-256(canonical-input), lowercase hex
item content_hash  = sha256 over canonical JSON of ALL normalized content fields
```

Normative rules:

- **K1 — content equality is identity.** Line numbers, attachment numbering
  and chat names never enter the key. This is precisely why convergence
  survives renames and export chunk boundaries (plan lines 815–817).
- **K2 — media enters by bytes, not by name.** WhatsApp renumbers
  attachments per export (`IMG-20260304-WA0012.jpg` in export A,
  `…-WA0047.jpg` in export B); the hash is invariant, the name is provenance.
- **K3 — conflicts surface, never merge.** Equal key + different
  `content_hash` (edited or deleted-and-resent messages) yields
  `DuplicateClass::KeyConflict`; commit stores the incoming variant under a
  `-r2`/`-r3` suffix chain via `superseded_by`, keeping both versions
  addressable (binding backstops: IF §6.3 constraints, schema authority
  task-0020).
- **K4 — grammar versioning.** `wa1-`/`man1-` prefixes ARE the grammar
  version; a future incompatible derivation ships `wa2-` etc. Old keys are
  never rewritten (plan line 1151: stable identities).

### 7.2 Worked example A — re-importing the identical file is a no-op

Setup: export `A.zip` contains, among 500 messages, on line 42:

```text
[04.03.26, 21:41] Anna: Weekly sync moved to 16:00
```

First import commits source S1 with, among others, this item:

```text
canonical-input = 2026-03-04T21:41:00.000Z ␟ Anna ␟ Weekly sync moved to 16:00 ␟ -
external_key    = wa1-3f9c… (first 128 bits of SHA-256 of that string)
```

Second import of the byte-identical `A.zip`:

| Step                               | Behavior                                                                                    | Evidence                              |
| ---------------------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------- |
| `preview`                          | computes `sha256(A.zip)`, finds `sources.content_hash` equal → sets `fast_no_op = Some(S1)` | read-only lookup (INV-2)              |
| wizard                             | shows „already imported“, steps 3–4 skipped                                                 | IF §9 IMP-R2                          |
| `normalize`/`deduplicate`/`commit` | **never invoked**                                                                           | zero writes, DB fingerprint unchanged |

Defense-in-depth branch (fast-path row absent, e.g. restored partial backup):
`normalize` reproduces the identical batch (INV-3 determinism — same bytes,
same parser version ⇒ same keys and hashes); `deduplicate` classifies all 500
items `IdenticalDuplicate`; `CommitPlan.new_items = ∅` and
`media_to_store = ∅`; `commit` stores only the raw-original asset and its
envelope rows. Visible message set: unchanged. ∎

Test hook (mandatory): fixture „same file twice“ asserts
`CommittedSource.imported_items == 0` on run 2 and an unchanged row-count
fingerprint (both branches).

### 7.3 Worked example B — two overlapping exports yield no visible duplicates

Setup: export `A.zip` (March 4) holds messages m1…m100. Export `B.zip`
(March 11, same chat renamed from „Projekt X“ to „Projekt-X-Bau“, attachments
renumbered) holds m51…m150 — the user deleted nothing, the exporter simply
windowed differently. Line positions shift arbitrarily.

Three representative items:

```text
m75 in A: line 88,  [09.03.26, 10:07] Ben: Ich übernehme die Folgenoten
m75 in B: line 213, [09.03.26, 10:07] Ben: Ich übernehme die Folgenoten
          (+ attachment renamed WA0007.pdf → WA0102.pdf, hash identical)

m120 only in B: line 260, [10.03.26, 08:30] Anna: Danke!
```

Derivation:

```text
m75(A) key = wa1-H(SHA256("2026-03-09T10:07:00.000Z␟Ben␟Ich übernehme die Folgenoten␟<pdf-hash>"))
m75(B) key = wa1-H(same canonical input — chat name absent, attachment name absent)
       ⇒ equal keys AND equal content_hash ⇒ IdenticalDuplicate → skipped
m120(B) key = wa1-H("2026-03-10T08:30:00.000Z␟Anna␟Danke!␟-")
       ⇒ unseen key ⇒ New → imported into source S_B
```

Result: m75 is visible **once** (attributed to S_A, evidence links intact);
m120 appears exactly once under S_B; the visible set equals the union of both
message sets partitioned across per-export sources — the plan-line-1687
acceptance. Enforcement is twofold: stage 4 advisory, and the DB constraints
(`UNIQUE(source_id, external_key)`, partial unique on live `external_key`)
binding even under concurrent imports (IF §6.3). ∎

Test hooks (mandatory): fixtures „overlapping exports“, „renamed chat
overlap“ (K1 regression: rename alone must not produce duplicates), and
„edited resend“ (K3: conflict surfaced, `-r2` item stored, both retrievable).

### 7.4 Fixture-design alignment (task-0028)

task-0028's adversarial/overlap fixtures are designed against exactly these
derivations; this contract requires the fixture set to cover, at minimum:
byte-identical re-import (both §7.2 branches), window-overlap with shifted
line numbers, attachment-renumber overlap, chat-rename overlap,
mixed-precision timestamps (a minute-precision export overlapping a
second-precision export surfaces as `KeyConflict`, never as silent
duplication — the deliberate consequence of K1 + millisecond
normalization), and empty/degenerate archives.

Accepted limitation (complexity budget, mirroring IF §6.1): two genuinely
different chats containing byte-identical (timestamp + author + body + media)
messages converge to one item. Chat-scoped keys were rejected because chat
renames between exports are strictly more common than cross-chat
coincidences.

---

## 8. Media/asset hand-off to the AssetStore (G4)

Per task-0024 and AS §2.2, the hand-off obeys:

1. **Keys only from the grammar.** Every blob `commit` places uses the
   `asset-key` production `assets/<asset-id>/original.<ext>` with a fresh
   `asst_<uuid-v4>` (identity never content-derived, AS §3.1 — two imports of
   equal bytes keep separate histories). No import-domain code composes any
   other key form; session-scoped keys (`meetings/…`) are out of reach by
   construction since `CommitSink::put_asset` generates the key internally.
2. **Hash-checked placement.** Each put carries
   `expected_hash = MediaRef.content_hash` (raw original: the archive hash
   from stage 2), verified free at AS verification point V1 during the AS §5.2
   six-step protocol.
3. **No absolute paths, anywhere.** Persisted location identity is exactly
   `(asset_id, storage_key)` with `storage_key` matching the AS §2.2 grammar
   (plan lines 1158–1159). `StagedUpload.staging_path` and
   `StagedMedia.staging_relative_path` are process-local and die with the
   run; the only code turning a key into a filesystem path is
   `AssetStore::resolve` (AS §2.1 rule 2). Mechanical check: the §6.3-2
   serialization scan doubles as the absolute-path scan
   (`[A-Za-z]:\\|^/` patterns must not occur).
4. **Link rows ride K4.** `source_item_assets (source_item_id, asset_id)`
   rows are created inside the same C3 transaction as the items (plan line
   1272); many-to-many in both directions.
5. **No-duplicate-blobs corollary.** `media_to_store` contains media of NEW
   items only; skipped duplicates keep pointing at the first import's assets
   (their link rows already exist there). Identical re-import ⇒ zero new
   blobs; overlapping import ⇒ blobs only for genuinely new attachments.
6. **Downstream binary access** is exclusively
   `AssetStore::get(key, ReadVerification::Full)` behind the domain services;
   frontends use the app's asset/serve channel, never filesystem paths
   (AS §2.1, RB R5).
7. **Unsupported media** (video, unknown MIME) is stored like any asset and
   flagged unanalyzed in `assets.processing_state` — retained, displayed as
   metadata, never silently dropped nor reported as analyzed
   (plan lines 1691–1692).

---

## 9. Persistent import/analysis job semantics

### 9.1 State machine — adopted verbatim

`processing_jobs` (plan lines 1313–1318: _„Die Zustands- und Retry-Semantik
entspricht `meeting_stt_jobs`“_) reuses the stt-provider-contract machine
without modification:

- States: `queued`, `running`, `retry_wait`, `succeeded`, `failed`,
  `cancelled` (STT §3.1; plan line 1197).
- Allowed transitions: exactly STT §3.2's list, including
  `running → queued` demotion on app restart **without** attempt-count
  penalty (STT §3.4), `succeeded` strictly terminal, and
  `failed`/`cancelled → queued` only by explicit user re-queue with a bumped
  requeue generation and a fresh idempotency key (STT §3.2).
- Error classification permanent/transitory: STT §6 applies with the obvious
  domain mapping (disk full / staging IO ⇒ transitory-with-backoff up to
  max attempts; unparsable format / rejected archive ⇒ permanent).

### 9.2 Job kinds covered

| Kind                                                  | Created by                                   | Payload carries                                                       |
| ----------------------------------------------------- | -------------------------------------------- | --------------------------------------------------------------------- |
| `import` (tier B)                                     | service wrapper, before stage 2 (INV-2 note) | idempotency key, staged-run reference (process-local), target project |
| `media_analysis`                                      | service, AFTER C3, per §9.5                  | asset **ids** + required input modalities, consent flags              |
| `source_summary`, `project_snapshot`, connector syncs | their owning services                        | out of scope here; same machine                                       |

### 9.3 Wrapper responsibility vs. stage purity

The service persists the tier-B import row (create `queued` → `running`
before stage 2; outcome flip in the same transaction as the final effect)
so that a long parse survives restart. The stages themselves stay
side-effect-free: the wrapper observes results and writes transitions. This
keeps INV-1/INV-2 intact while preserving IF §4.2's restartability intent —
the difference (row written by wrapper, not by `preview`) is deliberate and
recorded in §2.

### 9.4 Idempotency key

For import jobs (extending IF §6.4's formula, plan line 1318: an idempotency
key prevents duplicate results):

```text
idempotency_key = "import-wa1-" | "import-man1-"
                ⊕ first 128 bits of sha256(file bytes)
                ⊕ "-" ⊕ target(project|inbox)
```

Before executing C1, `commit` re-checks the key: a `succeeded` job with an
equal key short-circuits to its recorded `CommittedSource` instead of
re-committing (this closes the crash-after-C3 window of §4 rule 2). Analysis
jobs derive theirs analogously from `(kind, asset_ids sorted, prompt/model
version)` so replayed multimodal runs cannot double-charge or double-write.

### 9.5 Consent gating

Analysis jobs that would send content off-device are enqueued only when the
corresponding `CommitDecision.cloud_consent` (or a later explicit per-run
consent, plan lines 1689–1690) is set; modality filtering of models happens
in those jobs, not in ingestion (IF §1.2 item 3).

---

## 10. Security validation rules — pointer (G5)

All ZIP/archive/file safety **rules** — traversal and symlink criteria,
absolute-member rejection, decompression-bomb ratios and size/count caps,
encrypted-archive handling, the malicious-corpus fixtures, and their exact
limit values — are owned by **task-0040** and are not restated, re-derived,
or given default values anywhere in this document. This contract contributes
to that scope only three **enforcement points**, which task-0040's rules
inhabit:

1. `inspect` runs before any other stage touches the bytes
   (plan lines 1463–1467) and communicates exclusively through the opaque
   `SafetyVerdict` / `SafetyRejection.rule_id` carrier — the trait makes a
   partially-imported rejected archive unrepresentable.
2. The isolated staging directory is created by the service before stage 1
   and destroyed by run-scoped RAII; stages never receive paths outside it.
3. `StorageKey::parse` (AS §2.2) remains the single choke point rejecting any
   escaped or absolute key at `CommitSink::put_asset`.

If task-0040 changes a rule, this document requires **no edit**: the
`rule_id` namespace and the three points above are the entire coupling.

---

## 11. Verification hooks (maps to plan §14)

Tests this contract makes mandatory (fixtures per task-0028/task-0040):

1. **No-op re-import** — §7.2 both branches; asserts zero items, zero blobs,
   unchanged fingerprint on run 2.
2. **Overlap convergence** — §7.3 union property; skipped items' evidence
   links resolve; A-only items survive.
3. **Determinism** — `normalize` output equal across two runs on identical
   staging input (INV-3).
4. **Conflict surfacing** — crafted equal-key/different-hash pair yields
   `KeyConflict`, `-r2` item, both retrievable.
5. **Transport firewall** — §6.3 scans 1 and 2 pass over a full import
   (release-blocking).
6. **Mutation confinement** — instrumented store seams assert zero writes
   (DB, fs, job rows) during stages 1–4 for a full happy-path run; all writes
   observable inside `commit` only (INV-1 regression guard).
7. **Crash windows** — fault injection at C1/C2/C3 boundaries reproduces the
   IF §6.4 table's promised end states, including idempotency-key
   short-circuit on retry.
8. **Job-machine conformance** — `processing_jobs` transitions exercised
   against the STT §3.2 allowed-list; illegal transitions rejected;
   restart-demotion preserves attempt count.

---

## Appendix A — Gate traceability

- **G1** (five stages, typed, only `commit` mutates): §3 trait with
  per-stage signatures; INV-1 + mutation rule in the trait doc comment;
  §4 normative commit order; §11 test 6.
- **G2** (normalized-source record, no WhatsApp fields downstream, negative
  testability): §6.1 consumer table, §6.2 field rules, §6.3 mechanical scans
  with allowlist rationale; §11 test 5.
- **G3** (dedup rules provably support identical re-import no-op and overlap
  convergence, each with a worked example): §7.2 example A with proof table,
  §7.3 example B with key derivations, rules K1–K4, binding constraints via
  IF §6.3; §11 tests 1–4.
- **G4** (media hand-off via asset-store storage-key grammar, never absolute
  paths): §8 rules 1–3 and 6, citing AS §2.2 grammar, AS §3.1 identity,
  AS §5.2 protocol, AS §2.1 rule 2 resolution point, plan lines 1158–1159.
- **G5** (security rules = pointer to task-0040): §10 — three enforcement
  points named, zero validation logic re-specified; §3's `SafetyVerdict`
  carries only an opaque `rule_id`.

## Appendix B — Sketch compilability

The §3 Rust block is extracted and compiled as a standalone library crate
with the shared helper:

```powershell
pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1 `
  -ContractPath docs/meeting-mode-architecture/ingestion-pipeline-contract.md
```

Result recorded in the mission report (std-only, no external crates).

Scope note for the documentation index: this document belongs in the index's
contract table next to `import-pipeline-dataflow.md`; the index file is
outside this leaf's declared conflict scope and is left to the supervisor's
integration pass.
