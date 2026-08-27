# Project-Snapshot Contract — Lifecycle, Versioning and Diff

Status: **spec contract — no implementation yet** (Phase 10 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §9 (`project_snapshots`/
`snapshot_evidence`, plan lines 1303–1311; `processing_jobs` plan lines
1313–1318; `change_log` plan lines 1320–1325; change-envelope rules plan
lines 1149–1159; manual status note plan lines 1224–1225), §10
("Projektintelligenz", plan lines 1380–1403), §5 (`ProjectIntelligenceService`
plan lines 293–299; `ProjectManager` plan lines 261–265), §13 Phase 10
(tasks plan lines 1694–1703, acceptance plan lines 1705–1712) and §14
(snapshot test-matrix row, plan line 1879).
Planned implementation home: `src-tauri/src/managers/project_intelligence.rs`
(plan line 299); persistence exclusively through the `KnowledgeStore` seam
(`publish_snapshot`, K9, repository-boundary-contract §2/§4).
This document defines the snapshot lifecycle, identity, immutability, diff and
retention contracts only; it changes no source code.

Sibling contracts consumed by this document (read-only):

- [summary-evidence-contract.md](summary-evidence-contract.md) — pipeline
  stages P0–P3 (SEC-§1.2), idempotency-key algebra (SEC-§2), evidence-pointer
  model (SEC-§3), protected-manual-change rules (SEC-§4), prompt/model
  pinning (SEC-§6). Below abbreviated **SEC-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) —
  KnowledgeStore units of work K1–K10 (RB-§4), change-envelope semantics
  (RB-§5), consumer matrix incl. `ProjectManager` and
  `ProjectIntelligenceService` duties (RB-§6), SQL choke-point rules
  (RB-§7). Below abbreviated **RB-§n**.
- [retention-quota-contract.md](retention-quota-contract.md) — policy-engine
  rules R1–R3, trigger set T1–T4, `PolicyAction` exhaustiveness, deletion
  matrix and change-log pruning protocol (RQ-§n). Abbreviated **RQ-§n**.
- [search-index-contract.md](search-index-contract.md) — corpus C1 indexes
  only the `current` snapshot; K9 swaps index rows atomically with demotion;
  K3 removes a deleted project's snapshot index rows. Below abbreviated
  **SI-§n**.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.
This document deliberately does **not** restate SEC-§6 (prompt/model version
pinning) or the SEC-§2 key algebra beyond instantiating it for snapshots;
where those sections fix a value, SEC is the authority (§6 here).

---

## 0. Scope

A **project snapshot** is one immutable, versioned projection of a project's
AI-assessed status (summary markdown, decisions, risks, open questions,
conflict entries) together with the evidence pointers and provenance that make
every claim checkable. The contract answers five questions:

1. When does a new snapshot version get created, and why is creation
   idempotent per `(project, source-revision-set)`? (§1, §2)
2. What identity does a snapshot carry, and how does the parent chain form?
   (§2)
3. What is pinned immutably at publication, and how do pointers age? (§3)
4. How are consecutive snapshots compared — at entity level, not text-patch
   level? (§5)
5. How long do old versions live, expressed through the existing
   retention/quota hook names? (§7)

### 0.1 Non-goals

See §8 for the full complexity-budget list. Headlines: no text patches, no
snapshot branching/merge, no automatic pruning in v1, no re-specification of
SEC-owned mechanisms.

---

## 1. When snapshots are created (gate G1, part 1)

### 1.1 Trigger inventory (exhaustive in v1)

A snapshot publication is always the terminal stage P3 of a project-intelligence
run (SEC-§1.2). Runs are caused by exactly three trigger classes:

| #    | Trigger              | Origin                                                                                                                                                                     | Notes                                                                               |
| ---- | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| T-S1 | Source delta         | Any committed tier-A mutation under `sources`/`source_items`/`project_sources`/`project_tasks` lands in `change_log` (plan lines 1382–1386: "neue/geänderte Source-Items") | Debounced per §1.3; never synchronously inside the mutating transaction             |
| T-S2 | Restart resumption   | Startup recovery finds a non-terminal intelligence job (same rule class as STT-§3.4)                                                                                       | Re-executes deterministically from persisted stages; publishes nothing twice (§2.3) |
| T-S3 | Explicit user action | `Projektstatus aktualisieren` command routed through `ProjectManager`                                                                                                      | Increments `rerun_generation` exactly as SEC-§2.1 prescribes for deliberate re-runs |

There is **no timer-driven re-analysis**: a run without a source delta cannot
produce a different snapshot (its inputs are identical), so scheduling one
would be cost without value (NG-7).

### 1.2 What counts as "the source-revision set"

The delta input of a run is fixed once, at P0 execution time: P0 reads
`change_log` up to a cutoff cursor and selects every live/tombstoned
`source_items` group keyed by `(source_id, source_revision,
item-content-hash-set)`. The cutoff is recorded as:

```text
covered_through := (max_change_log_rowid_included, wall_clock_captured_at)
```

`change_log` order is its local rowid (RB-§3 note N5), so the pair is a
deterministic resume position. `covered_through` is persisted **only** inside
the published snapshot row (plan line 1307) — a run that fails before P3
persists no cursor, and the next run simply re-selects the delta.

### 1.3 Enqueue discipline and coalescing

1. At most one **non-terminal** (queued/running) intelligence job per project
   exists (enforced by a partial uniqueness constraint on `processing_jobs`,
   service `project_intelligence`). This is an operational tier-B constraint
   (RB-§4 W6); it bounds work without changing any domain entity.
2. A T-S1 delta arriving while such a job exists therefore enqueues **nothing**
   new: a queued job re-runs P0 at execution start and picks up the larger
   delta automatically (P0's cutoff is taken at execution time, §1.2).
   A running job is never widened mid-flight — a follow-up job is enqueued
   instead, and its P0 selects whatever remains past the published
   `covered_through`. No delta is ever skipped or processed twice.
3. The run-level identity materializes when P0 persists its stage payload
   (write-once, SEC-§1.3): from that moment the job's
   `idempotency_key`/`run_key` is frozen. A crash before that point loses
   nothing (pure recomputation); a crash afterwards resumes at the persisted
   stage boundary (SEC-§1.2 boundary rules).

### 1.4 Creation is idempotent per (project, source-revision-set)

Restated for the snapshot artifact, authority SEC-§2 (not duplicated there
beyond instantiation):

```text
run_key          = SHA-256("project-intelligence", project_id, delta_basis,
                           provider_id, provider_settings_snapshot,
                           prompt_version, rerun_generation)
                   where delta_basis =
                   SHA-256(sorted (source_id, source_revision,
                                   per-source item-content-hash-set))
                             -- SEC-§2.1: exactly the (project,
                             -- source-revision-set) identity

snapshot_key     = SHA-256(run_key, "publish", conflict_scan_result_hash)
                   -- SEC-§2.2 P3 stage key
```

Guarantees:

- **G1a.** Two runs over the same source-revision set with the same provider
  settings, prompt version and generation produce the same `snapshot_key`;
  `UNIQUE(project_id, idempotency_key)` on `project_snapshots` makes the
  second publication a structural impossibility (SEC-§2.2 duplicate-guard
  column). A retried job finds the snapshot already published, books itself
  `succeeded` and stops (STT-§3.3 rules 3–4 pattern).
- **G1b.** A changed source revision set changes `delta_basis`, hence the key
  — a genuinely new snapshot version is expected and allowed.
- **G1c.** An explicit user refresh increments `rerun_generation` (T-S3), so
  intentional re-publication over identical sources remains possible without
  colliding — the same distinction as SEC-§2.1's rerun rule.

---

## 2. Snapshot identity and parent chain (gate G1, part 2)

### 2.1 Identity fields

| Field                | Rule                                                                                                                                                               |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`                 | UUIDv4, store-generated at persist, lowercase hyphenated (RB-§5.1); never reused                                                                                   |
| `project_id`         | FK to `projects`; scoping boundary for every read and write                                                                                                        |
| `version_seq`        | Per-project monotone integer starting at 1, assigned inside the K9 transaction as `previous.version_seq + 1` (or 1 for genesis); `UNIQUE(project_id, version_seq)` |
| `idempotency_key`    | The §1.4 `snapshot_key`; `UNIQUE(project_id, idempotency_key)`                                                                                                     |
| `parent_snapshot_id` | See §2.2; `NULL` iff `version_seq = 1`                                                                                                                             |
| `current`            | Exactly one row per project; demotion/promotion only inside K9 (RB-§4)                                                                                             |

### 2.2 Parent-chain rules

1. **Linear chain, no branches.** At publication, the K9 transaction reads
   the current `current` snapshot of the project; that row's `id` becomes the
   new row's `parent_snapshot_id`, and the same transaction demotes it
   (plan lines 1310–1311). Because SQLite serializes writers and K9 is one
   unit of work (RB-§4 W1), two publications can never observe the same
   parent — the chain is provably linear per project.
2. **Genesis.** The first published snapshot of a project has
   `version_seq = 1`, `parent_snapshot_id = NULL`.
3. **No gaps, no reordering.** `version_seq` increments by exactly 1 per
   publication; historical rows are never rewritten (§3.1), so walking
   `parent_snapshot_id` from `current` reproduces the exact publication
   order, independently of `created_at` clock values.
4. **Project deletion.** `soft_delete_project` (K3) tombstones the project;
   snapshot rows are tier-A and follow the W3 tombstone pattern — they are
   never hard-deleted (RB-§5.4), and SI-§D-rules remove their derived index
   rows. Chain integrity survives deletion; a restored project continues the
   same chain.

---

## 3. Immutable pinning (what freezes at publication)

### 3.1 Write-once rule

A published snapshot row is **write-once**: no code path issues `UPDATE`
against its content columns (`summary_markdown`, structured statements,
conflicts, `covered_through`, provenance, evidence rows). Correction happens
exclusively by publishing a successor version through the §1/§2 machinery.
This mirrors SEC-§1.3's write-once intermediates and gives the plan's
traceability demand (plan line 1311: "ältere bleiben für Nachvollziehbarkeit
erhalten") a mechanical basis rather than a convention.

The only mutable bit is the `current` flag, and only inside K9's atomic
demote/promote swap.

### 3.2 What each publication pins

| Pinned artifact          | Content                                                                                                                                                                                                                                                                          | Anchor                                            |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `covered_through`        | `(change_log_rowid, captured_at)` of the P0 cutoff (§1.2) — the auditable answer to "bis welchem Stand?" (plan line 1707)                                                                                                                                                        | plan line 1307                                    |
| Source-revision set      | Every `(source_id, source_revision)` that P0 selected                                                                                                                                                                                                                            | SEC-§2.1 `delta_basis` inputs                     |
| `snapshot_evidence` rows | Per claim: `subject_statement_lineage_id`, `source_id`, `source_revision_at_publish`, locator (`source_item_id` or `meeting_transcript_segment_id`), optional `time_range`, verbatim `quote_excerpt`, `rationale` — SEC-§3.2 field set **plus** the pinned source/item revisions | plan lines 1309–1310, SEC-§3.2                    |
| Provenance               | `provider` (+ non-secret settings snapshot), `model`, `prompt_version`, `prompt_content_hash`, `payload_schema_version`                                                                                                                                                          | plan lines 1305–1308; authority SEC-§6.1 — see §6 |
| Conflict entries         | Both contradicting positions, each with valid evidence pointers (invariant E3)                                                                                                                                                                                                   | SEC-§3.3 E3, plan lines 1397–1398                 |

Publication validity: invariant **E1** applies verbatim — the P3 transaction
rejects the whole publication if any claim lacks a valid evidence pointer
(SEC-§3.3), marking the job `failed` with error class
`evidence_validation_failed`.

### 3.3 How pins age (read-time resolution, never rewriting)

Evidence pins are historical records of _why the snapshot said what it said_.
They are never repaired, re-resolved or deleted when sources change later:

- Resolution status (`live` / `stale_quote` / `locator_gone` / `source_tombstoned`)
  is computed at read time against current state using the SEC-§3.3 validity
  conditions plus the pinned revisions. A pin whose `quote_excerpt` still
  matches and whose locator still exists resolves even many revisions later.
- A dangling pin degrades the _display_ of the affected claim (UI marks the
  statement's evidence as unverifiable); it never invalidates the snapshot as
  a historical artifact and never triggers automatic correction.
- The successor run sees the tombstoned/changed source as delta input
  (T-S1) and produces the corrected version through the normal pipeline —
  repair flows forward through versions, never backward into history.

---

## 4. Manual user changes survive re-runs (gate G2)

Precedence is inherited from two authorities and closed here with a
structural argument; nothing new is invented.

1. **Ownership separation (RB authority).** All human writes go through
   `ProjectManager` onto KnowledgeStore operations K1–K6/K7-human
   (RB-§6: "Manual user changes outrank AI runs", plan lines 263–265). The
   complete set of AI write surfaces is: insert `project_snapshots`, insert
   `snapshot_evidence`, insert/update-in-place of **own** AI-suggestion task
   rows (SEC-§2.3), additive `ai_comment` annotations (SEC-§4.2 rule 1).
   That set is disjoint from every protected column of SEC-§4.1 — there is
   no UPDATE path from any AI surface to a human-flagged row at all.
2. **Transactional race closure (SEC authority).** The `manual_change_guard`
   inside the P3 transaction compares baseline vs. current revisions
   (SEC-§4.3); entities edited between P0 and P3 are excluded from AI writes
   and surface as visible conflict entries in the snapshot instead
   (plan line 1397). Guard and write share one transaction, so no interleaved
   user edit can slip between check and commit.
3. **Snapshot-specific consequences.**
   - The manually maintained status note on `projects`
     (plan lines 1224–1225) lives outside every AI-writable surface; it is
     never part of `summary_markdown`, never appears as a snapshot statement,
     and therefore never appears as a diff entry (§5).
   - Snapshots reference tasks read-only (lineage/state observation at
     publish). A human `done`/`dismissed` decision suppresses re-suggestion
     via the SEC-§2.3 resolution rules; re-runs resurrect nothing.
   - Because snapshots are append-only versions, a re-run can never
     _overwrite_ the previous view of anything — worst case it publishes a
     wrong _new_ version, which the user replaces via T-S3 while the old
     version stays inspectable.

---

## 5. Diff between consecutive snapshots (gate G3, entity level)

### 5.1 Principle

Diffs compare **entities across snapshot versions**, not text. There is no
line/word patch, no LCS, no stored diff blob (NG-1). Consumers (UI
"Was hat sich seit der letzten Analyse geändert?", P2 reconciliation,
diagnostics) receive typed entity-level operations with evidence-aligned
references.

### 5.2 Comparable content model

Every AI-authored assertion in a snapshot is a **statement row**:

```text
statement_lineage_id   stable logical identity across versions (see 5.3)
kind                   status_claim | decision | risk | open_question
content                normalized statement text
content_hash           SHA-256 canonical content serialization (RB-§5.5 rules)
evidence               ≥1 snapshot_evidence row (§3.2); E1-checked
conflict               flag + counterpart pointer for conflict entries
```

Task references and coverage metadata participate in the diff as aggregates,
not as statements (§5.5).

### 5.3 Lineage assignment (how identities survive versions)

Lineage is decided once, inside the P2/P3 pipeline of each run, against the
previous `current` snapshot:

1. Compute each candidate statement's `match_fingerprint =
SHA-256(kind, fold_case(trim_whitespace(content)),
sorted evidence locator ids)`.
2. Match against the previous snapshot's statements:
   - equal fingerprint → same lineage: carry the previous
     `statement_lineage_id` forward;
   - differing text but ≥1 shared evidence locator → same lineage
     (Changed): carry the id, the content hash difference drives the diff;
   - otherwise → new lineage: mint a fresh `statement_lineage_id`.
3. Tie-breaks are total: candidates are processed in
   `(kind_rank, normalized_content)` order; a previous statement matches at
   most one candidate. Given the same two snapshots' inputs the assignment is
   deterministic — no embedding similarity, no fuzzy thresholds (NG-6).

### 5.4 Diff output model

Computed on demand by a pure function over two stored snapshot versions
(typically `current` and its parent). Never persisted in v1 (NG-1); cheap to
recompute because both inputs are immutable.

```rust
//! Sketch of the planned diff half of
//! src-tauri/src/managers/project_intelligence.rs.
//! PURE: no I/O, no clock, no randomness. Deterministic total order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    StatusClaim,
    Decision,
    Risk,
    OpenQuestion,
}

/// One statement row as projected out of an immutable snapshot version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatementRow {
    /// Stable logical identity across versions (contract §5.3).
    pub lineage_id: String,
    pub kind: StatementKind,
    /// SHA-256 over the canonical content serialization (RB-§5.5).
    pub content_hash: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffOp {
    Added,
    Removed,
    Changed,
}

/// One entity-level diff entry. Deliberately NOT a text patch: consumers
/// render entity cards and jump to pinned evidence, never merged prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffEntry {
    pub op: DiffOp,
    pub kind: StatementKind,
    pub lineage_id: String,
    pub old_content_hash: Option<String>,
    pub new_content_hash: Option<String>,
}

pub struct SnapshotDiff {
    pub from_snapshot_id: String,
    pub to_snapshot_id: String,
    /// Statements present identically on both sides (count only).
    pub unchanged_count: u64,
    pub entries: Vec<DiffEntry>,
}

/// Fixed presentation/reconciliation order of statement kinds.
fn kind_rank(k: StatementKind) -> u8 {
    match k {
        StatementKind::StatusClaim => 0,
        StatementKind::Decision => 1,
        StatementKind::Risk => 2,
        StatementKind::OpenQuestion => 3,
    }
}

pub fn diff_snapshots(
    from_snapshot_id: String,
    to_snapshot_id: String,
    previous: &[StatementRow],
    next: &[StatementRow],
) -> SnapshotDiff {
    let prev_by_lineage: std::collections::BTreeMap<&str, &StatementRow> =
        previous.iter().map(|s| (s.lineage_id.as_str(), s)).collect();
    let next_lineages: std::collections::BTreeSet<&str> =
        next.iter().map(|s| s.lineage_id.as_str()).collect();

    let mut entries: Vec<DiffEntry> = Vec::new();
    let mut unchanged_count: u64 = 0;

    for s in next {
        match prev_by_lineage.get(s.lineage_id.as_str()) {
            None => entries.push(DiffEntry {
                op: DiffOp::Added,
                kind: s.kind,
                lineage_id: s.lineage_id.clone(),
                old_content_hash: None,
                new_content_hash: Some(s.content_hash.clone()),
            }),
            Some(old) if old.content_hash != s.content_hash => entries.push(DiffEntry {
                op: DiffOp::Changed,
                kind: s.kind,
                lineage_id: s.lineage_id.clone(),
                old_content_hash: Some(old.content_hash.clone()),
                new_content_hash: Some(s.content_hash.clone()),
            }),
            Some(_) => unchanged_count += 1,
        }
    }
    for s in previous {
        if !next_lineages.contains(s.lineage_id.as_str()) {
            entries.push(DiffEntry {
                op: DiffOp::Removed,
                kind: s.kind,
                lineage_id: s.lineage_id.clone(),
                old_content_hash: Some(s.content_hash.clone()),
                new_content_hash: None,
            });
        }
    }

    entries.sort_by(|a, b| {
        (kind_rank(a.kind), a.lineage_id.as_str())
            .cmp(&(kind_rank(b.kind), b.lineage_id.as_str()))
    });

    SnapshotDiff {
        from_snapshot_id,
        to_snapshot_id,
        unchanged_count,
        entries,
    }
}
```

Properties (each testable):

- **Total and deterministic:** same input pair ⇒ byte-identical
  `Vec<DiffEntry>` in fixed `(kind_rank, lineage_id)` order.
- **Entity-level only:** entries address whole statements by lineage id +
  content hashes; no fragment offsets exist anywhere in the type.
- **Evidence-alignable:** every entry's `lineage_id` resolves in both
  snapshots to rows carrying `snapshot_evidence` pins (§3.2), so a UI can
  offer "Beleg zeigen" for both sides of a `Changed`.

### 5.5 Aggregate diff dimensions (non-statement)

Reported alongside `entries`, computed from the same two immutable rows:

| Dimension         | Comparison                                                                                                                     |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Task population   | counts by state (`suggested`/`open`/`done`/`dismissed`) plus added/removed AI-suggestion lineage ids (SEC-§2.3 fingerprints)   |
| Coverage          | `covered_through` advanced or unchanged (§1.2)                                                                                 |
| Conflicts         | count of conflict entries opened/resolved between the versions                                                                 |
| Provenance change | provider/prompt-version equality flag — informational only; a provider switch alone must NEVER mark content statements Changed |

### 5.6 Explicitly rejected: text-patch semantics

No stored or computed unified diff, no word-level highlighting contract, no
"merged markdown" rendering obligation. Free prose inside `summary_markdown`
inherits traceability from the statement rows exactly as meeting-summary
prose does (SEC-§3.4); sentence-level prose diffing is rejected as fragile
and unactionable (NG-1).

---

## 6. Prompt/model versioning — pointer, not duplication

All prompt/model pinning obligations for snapshots are owned by
[summary-evidence-contract.md](summary-evidence-contract.md) §6 and are **not
restated normatively here**. Binding facts for implementers of this contract:

1. `project_snapshots` persists `provider`, `model`, `prompt_version`,
   `prompt_content_hash` and `payload_schema_version` per SEC-§6.1's field
   table (plan lines 1305–1308 name these columns; SEC fixes their
   semantics).
2. These fields are part of the artifact (SEC-§6.2): deleting a snapshot —
   which in v1 means only tombstoning with its project (§2.2 rule 4) — takes
   the provenance along; nothing else may remove it.
3. The §1.4 run key binds `provider_id`, `provider_settings_snapshot` and
   `prompt_version`, so provenance is consistent with idempotency by
   construction, not by after-the-fill bookkeeping.

Any future change to pinning semantics happens in SEC, never here.

---

## 7. Retention of old snapshots (gate G3, retention half)

### 7.1 v1 rule: keep every version

Normative basis: plan line 1311 keeps older snapshots "für Nachvollziehbarkeit";
Phase 10 acceptance (plan line 1707) makes the version chain the audit trail
for "bis zu welchem Stand wurde ausgewertet". Therefore:

- **All versions of a project are retained for the life of the project.**
  Old snapshots are small structured JSON/markdown rows (kilobytes), so the
  growth bound is usage-shaped, not runaway; inventing a cap now would be
  complexity without a single evidence-backed complaint.
- Old snapshots are **never auto-deleted**: they fall outside ledger L1
  (audio bytes, RQ-§2), so rules R1/R2 cannot select them; RQ-§11 NG-7
  explicitly rejects generic GC for other tables; and tier-A hard deletes do
  not exist in v1 (RB-§5.4/W3).
- In RQ-§6.1 matrix terms, old-snapshot deletion is classified
  **"Never automatic"** alongside imported assets — the same
  reported-not-acted-upon philosophy (RQ-§2 L2 rationale).

### 7.2 Where snapshots DO meet the retention hooks today

Even with keep-all, the lifecycle touches the named retention seams at three
exact points:

| Hook (retention-quota-contract name)                                       | Snapshot interaction                                                                                                                                                                                                                      |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Trigger **T2 Post-Finalize** / evaluation sequence A→B→C (RQ-§3, RQ-§5.2)  | Not extended: snapshot publication is a KnowledgeStore K9 event, not a sweep concern; the sweeper never reads snapshots in v1                                                                                                             |
| **`change_log`** audit rows (RQ-§9 protocol: marker-first, 180-day window) | Every K9 publication and K3 project tombstone writes its tier-A `change_log` row (RB-§4 W2); those rows age out under R3 exactly like all others — snapshot traceability rests on the immutable rows themselves, not on their log entries |
| Delete levels / `RetentionGuard` (RQ-§7.2, AS-§8.2)                        | Untouched: `DeleteAudioLevel1` semantics cover only meeting audio/export files; no snapshot file exists to unlink                                                                                                                         |

### 7.3 Reserved extension path (right-column, NOT built in v1)

If real usage later shows unbounded-history pain, the sanctioned mechanism is
fixed **now** so an ad-hoc GC can never appear:

1. New `PolicyAction::PruneSupersededSnapshots { project_id, keep_last }`
   appended to RQ-§5.1's enum — declared here as an interface addition owned
   by the storage workstream, exactly the precedent of
   `RetentionBlockReason::SessionEndedDiskFull` (RQ-§4.5 adoption pattern).
   Enum exhaustiveness (RQ-§5.1) is preserved because this declaration is
   normative text in a contract, not silent code drift.
2. Evaluated in RQ-§5.2 stage B **after R3**, triggered only by **T4 daily
   timer**, with a product constant (e.g. `SNAPSHOT_HISTORY_KEEP`) derived
   and tested like RQ-§4.2 constants — never user-configurable (RQ NG-1).
3. Executed in stage C via a new KnowledgeStore unit of work
   `prune_superseded_snapshots` (extension of RB-§4, K-numbered): one
   transaction, marker-first `change_log` row (RQ-§9 rule 2), tombstone-not-
   delete (W3), and the Gate-2 analog: the same transaction re-checks
   `current = 0` for every victim immediately before writing, so a snapshot
   promoted between evaluation and execution is skipped, mirroring
   RQ-§7.2's guard-before-unlink logic at row level.
4. Until that evidence exists, step 0 applies: ship nothing (§8 NG-4).

---

## 8. Complexity budget and non-goals (gate G4)

Rejected for v1, each with the reason it is SAP-like scope rather than
evidence-backed value:

- **NG-1 — no text-patch diffing or stored diff blobs.** Entity operations
  plus pinned evidence answer "what changed and why"; character-level diffs
  of AI prose are noise. §5.6.
- **NG-2 — no snapshot branching, merging or DAG.** One linear chain per
  project (§2.2); branches solve multi-editor concurrency that v1 does not
  have (single-device, plan §1).
- **NG-3 — no arbitrary pairwise/range diffs** ("diff version 3 vs. 17").
  Only `current ↔ parent` is exposed; older comparisons are diagnostics via
  direct row reads, not a product feature.
- **NG-4 — no automatic pruning of old snapshots.** Keep-all (§7.1); the
  pruner exists only as a declared reservation (§7.3) until complaints prove
  the need.
- **NG-5 — no per-statement global history table.** Lineage lives in the
  chain itself; a side table would duplicate what `parent_snapshot_id` walks
  already answer.
- **NG-6 — no embeddings/fuzzy similarity in lineage matching.** Exact
  fingerprints + shared-evidence fallback (§5.3) are deterministic and
  testable; similarity thresholds would make diffs unreproducible.
- **NG-7 — no scheduled re-analysis without deltas.** §1.1: identical inputs
  cannot yield a different snapshot; timers would burn provider cost for
  nothing.
- **NG-8 — no local restatement of SEC-§2 key algebra or SEC-§6 pinning
  semantics.** Single-authority citation (§1.4, §6); duplicated norms drift.

---

## 9. Verification hooks (maps to plan §14, line 1879)

- **Idempotency (G1):** publish the same source-revision set twice with equal
  generation → second P3 hits `UNIQUE(project_id, idempotency_key)` and books
  success without a new row; increment `rerun_generation` → exactly one new
  version.
- **Chain (G1/G3):** concurrent simulated publications serialize into a gapless
  linear chain; walking parents from `current` equals descending
  `version_seq`.
- **Immutability (§3):** a store-level test asserts no UPDATE statement on
  snapshot content columns exists (static allow-list grep in the K9 file) and
  that post-publish evidence pins survive subsequent source edits/re-imports
  byte-identically.
- **Precedence (G2):** fuzz the SEC-§4.1 protected columns between P0 and P3;
  assert exclusion + conflict entry, never an AI write (manual_change_guard
  tests, SEC-§4.3 protocol).
- **Diff (G3):** golden-vector tests over constructed statement sets pin the
  full `SnapshotDiff` including ordering; property test
  `diff(x, x) == empty entries`; lineage determinism test re-runs §5.3
  assignment on shuffled candidate order.
- **Retention (G3/§7):** negative-space test — no public API accepts a
  snapshot id for deletion; RQ golden vectors confirm no action variant can
  address snapshots.

---

## 10. Gate verification checklist

| Gate | Requirement                                                                                                      | Where satisfied                                                                                                          |
| ---- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| G1   | Idempotent creation per (project, source-revision-set) with stated key derivation; identity + parent-chain rules | §1.4 (key instantiation of SEC-§2), §1.3 (coalescing), §2.1 (identity), §2.2 (chain), §9 tests                           |
| G2   | Manual changes provably survive re-runs, precedence cross-referenced to ProjectManager duties                    | §4 (ownership disjointness argument citing RB-§6, plan lines 263–265; SEC-§4 guard), §9 fuzz hook                        |
| G3   | Entity-level diff model (not text patches); old-snapshot retention tied to retention-quota-contract hook names   | §5.2–§5.4 (typed entity model), §5.6 (patch rejection), §7.1–§7.3 (keep-all + T4/stage-B-C/marker-row/Gate-2 hook names) |
| G4   | Non-goals section; documentation index updated                                                                   | §8 (NG-1–NG-8); `docs/meeting-mode-documentation-index.md` gains this document's row and reading-order slot              |

## Appendix A — Sketch compilability

The Rust block of §5.4 compiles standalone via the shared helper:

```powershell
pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1 -ContractPath docs/meeting-mode-architecture/project-snapshot-contract.md
# OK: 1 rust block(s) from 'project-snapshot-contract.md' compiled as a library crate
#     (rustc 1.98.0 (88d9e12ae 2026-08-18)); executed via
#     `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ...` on this host.
```
