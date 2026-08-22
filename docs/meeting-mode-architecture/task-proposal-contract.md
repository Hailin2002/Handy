# Task-Proposal Precedence and Evidence-Linking Contract — ProjectIntelligenceService

Status: **spec contract — no implementation yet** (Phase 8/Phase 10 prerequisite)

Source of truth: `docs/meeting-mode-plan.md` §9 (`project_tasks`, plan lines
1277–1293; `task_evidence`, plan lines 1295–1301), §10 Projektintelligenz
(plan lines 1380–1403), precedence principles (plan lines 263–265
"Manuelle Nutzeränderungen besitzen Vorrang vor späteren KI-Läufen",
1395–1396 "Nutzeränderungen sind geschützte Fakten", 1224–1225 manual status
note), Phase 8 acceptance (plan lines 1656–1657, 1667–1668), Phase 10 tasks
and acceptance (plan lines 1694–1712, especially the workflow list of line
1701: "Vorschlag-, Annahme-, Bearbeiten-, Verwerfen- und Merge-Workflow für
Aufgaben"), envelope rules (plan lines 1149–1159) and principle 8 "Belege vor
KI-Automatik" (plan line 135).

Planned implementation home: `src-tauri/src/managers/project_intelligence.rs`
(plan line 299), writing exclusively through the `KnowledgeStore`
(`src-tauri/src/managers/knowledge_store.rs`, plan line 275).

Naming ruling: the plan names the table **`project_tasks`** (§9 heading, line
1277). SEC-§4.1 abbreviates it as `tasks`. This contract always uses the plan
name; where sibling contracts say "tasks" they mean the same entity.

Sibling contracts consumed by this document (read-only):

- [summary-evidence-contract.md](summary-evidence-contract.md) — evidence
  pointer model (SEC-§3), protected columns (SEC-§4), dedupe fingerprints
  (SEC-§2.3), pipeline stages P0–P3 (SEC-§1.2). Below abbreviated **SEC-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) —
  KnowledgeStore seam, units of work K1–K10 (RB-§4), consumer matrix (RB-§6),
  choke-point rules (RB-§7). Below abbreviated **RB-§n**.
- [project-snapshot-contract.md](project-snapshot-contract.md) — snapshot
  lifecycle that consumes task aggregates read-only (PSC-§4 item 3, PSC §5.5).
  Below abbreviated **PSC-§n**.
- [session-state-contract.md](session-state-contract.md) — conflict rule and
  documentation conventions (**SSC** head).

Conflict rule (identical to SSC head and the documentation index): the plan
wins over contracts, contracts win over catalogs, and nothing wins over actual
code. Within the contract layer specifically: where SEC-§3 defines the pointer
model and SEC-§2.3 defines the fingerprint algebra, **SEC wins** and this
contract only consumes and tightens them (see §4.1, §5.1); nothing here
redefines either.

---

## 0. Scope, subject, and non-goals

### 0.1 Subject

This contract fixes how `ProjectIntelligenceService` turns extracted task
candidates into **proposals** on `project_tasks`, how humans resolve them,
which side wins when both act, and what proof each AI-written field must
carry. It is the normative answer to five questions:

1. What states does a proposal move through, and who may move it (§2)?
2. Who wins a race between a user edit and a running/publishing KI run (§3)?
3. Which fields of an AI-originated proposal need evidence, of what shape,
   and where is that enforced (§4)?
4. How are repeated proposals across runs deduplicated without resurrecting
   human decisions (§5)?
5. Which repository-seam operations may `ProjectIntelligenceService` call,
   and which are forbidden to it (§6)?

### 0.2 Out of scope (non-goals)

- **Task execution.** Later-lifecycle states `in_progress`,
  `waiting_for_approval`, `failed`, `cancelled` (plan lines 1280–1281),
  `owner_type='assistant'`, `CapabilityRegistry` executability scoring
  (plan lines 1290–1293, 1812–1818) — future phases; this contract ends at
  proposal resolution plus the ordinary human task lifecycle
  (`open`/`done`/`dismissed`).
- **All DDL.** Schema authority is task-0020 / `db-schema.md` (RB-§0.1);
  this contract names tables, columns and constraints but defines none.
- **Sync/server/MCP merge semantics.** Conflict resolution across devices is
  explicitly deferred (plan lines 1764–1766); the fingerprint rules below are
  local-run discipline, not a sync protocol.
- **Prompt texts, model choice, extraction quality.** Product decisions
  outside an architecture contract (same exclusion as SEC-§0.2).
- **UI copy, confirmation dialogs, i18n keys** — proposal-only catalogs own
  those; this contract defines behavior, not wording.
- **Snapshot content.** Snapshot identity, diff and pinning live in PSC;
  snapshots observe task aggregates read-only (PSC-§4 item 3, PSC-§5.5).
- **New evidence-pointer formats or storage columns.** None are introduced;
  see §4.1.
- **Complexity budget.** This contract adds exactly two refinements beyond
  what plan + siblings already fix: (a) the per-field evidence-coverage
  clause TP-E1b (§4.3), (b) the atomic-proposal embedding of evidence into
  the K7 unit of work (§6.3). Both close concrete enforcement gaps that plan
  line 1708 ("jede KI-Aussage und Aufgabe hat mindestens einen gültigen
  Beleg") would otherwise leave unenforceable at field granularity. Any
  further mechanism (merge heuristics beyond the fingerprint, priority
  learning, cross-project suggestions) is rejected as scope creep until
  evidence shows a real need.

---

## 1. Data basis (fields this contract relies on)

From plan lines 1277–1293 (`project_tasks`) and 1295–1301 (`task_evidence`):

| Field | Role in this contract |
| --- | --- |
| `state`: `suggested`, `open`, `done`, `dismissed` (v1) | `suggested` marks a live proposal; `open`/`done`/`dismissed` are task lifecycle after human resolution |
| `origin`: `human` \| `ai` | who created the row |
| `confidence`, `model`, prompt version | mandatory provenance metadata on AI-origin rows (plan line 1286); pinned per SEC-§6 |
| `human_edited_at` | set by the store on every human mutation of any column; the protection trigger shared with SEC-§4.1 |
| `owner_type`: `unassigned` \| `human` (v1, plan lines 1284–1285) | assignee display; AI may write a proposed **assignee label** only when it appears verbatim in a cited source (plan line 1358: otherwise "Verantwortlich: nicht erkannt"); it never mints person identities and never sets `owner_type` itself — that stays `unassigned` until a human resolves the proposal |
| `revision`, `updated_at`, `deleted_at` | change envelope (RB-§5); basis for the race closure of §3.3 |

A **proposal** is not a second table. It is the projection
`origin='ai' AND state='suggested'` of a `project_tasks` row, resolved
through the transitions of §2.

---

## 2. Proposal state machine (gate G1)

### 2.1 Proposal states

The four proposal outcomes demanded by the leaf — `suggested`, `accepted`,
`rejected`, `edited` — map onto `project_tasks` columns exactly as follows:

| Proposal state | `project_tasks` projection | Meaning |
| --- | --- | --- |
| `suggested` | `origin='ai'` ∧ `state='suggested'` ∧ `human_edited_at IS NULL` | live AI proposal, untouched by humans; the only state an AI run may still modify (WR-2, §3.2) |
| `accepted` | task transitioned `suggested → open` by `Actor::Human` (RB-§4 K7) | user adopted the proposal into their task list |
| `rejected` | task transitioned `suggested → dismissed` by `Actor::Human` | user declined; terminal against re-suggestion (WR-3, §3.2) |
| `edited` | `human_edited_at IS NOT NULL` ∧ `state='suggested'` | user changed one or more protected columns (SEC-§4.1: title, text/description, state, priority, due date, assignee) of a still-open proposal; the row keeps `origin='ai'` for provenance but is now human-owned for every write rule |

`accepted` and `rejected` are terminal **proposal** outcomes; the underlying
task continues its ordinary human lifecycle (`open → done`, `open →
dismissed`, …) through ProjectManager/K7, which this contract does not
re-specify. `edited` is terminal for AI-write purposes (WR-2 fails from then
on) while the proposal itself remains resolvable by the user.

### 2.2 Transition table (complete enumeration)

Actors: `H` = human via ProjectManager commands, `AI` = a
ProjectIntelligenceService run inside its P3 publish transaction (SEC-§1.2).
Every transition below goes through KnowledgeStore unit-of-work K7
(RB-§4.2), i.e. one transaction, revision bump, `change_log` row.

| # | From | Actor | Event | To | Side effects / guards |
| --- | --- | --- | --- | --- | --- |
| T0 | *(no row)* | AI | publish candidate proposal | `suggested` | dedup resolution first (§5.2); requires ≥1 valid evidence pointer (TP-E1a, §4.3); provenance pinned (§1) |
| T1 | `suggested` | H | accept | `accepted` (task `open`) | K7 `transition_task`; revision++ |
| T2 | `suggested` | H | reject | `rejected` (task `dismissed`) | K7 `transition_task`; fingerprint recorded as suppressed (§5.2 R3) |
| T3 | `suggested` | H | edit any column | `edited` | K7 `upsert_task` with `Actor::Human`; store sets `human_edited_at` |
| T4 | `edited` | H | edit again / accept / reject | `edited` / `accepted` / `rejected` | as T1–T3; `human_edited_at` refreshed |
| T5 | `suggested` | AI | refresh (same `dedupe_fingerprint`, §5.1) | `suggested` | update-in-place of the row's own AI fields **only if** WR-2 holds at write time (§3.2); evidence may be extended additively via K8 |
| T6 | `edited`, `accepted`, `rejected` | AI | *(any write)* | **forbidden** | no AI code path exists; a violation attempt is a contract breach and a test target (§3.2 WR-2) |

There are no other transitions. In particular: no AI-initiated
`suggested → dismissed`, no automatic expiry, no automatic acceptance, and no
transition out of `rejected` back to anything (resurrection ban, §5.2 R3).

### 2.3 User edits during a pending KI run (gate G1, race matrix)

A KI run spans P0…P3 with persisted intermediates (SEC-§1.2); a human edit is
one K7 unit of work. SQLite serializes writers, so every interleaving lands
in exactly one of these cases — the enumeration is exhaustive because both
sides write only through whole transactions:

| Case | Ordering | Outcome | Enforced by |
| --- | --- | --- | --- |
| E-a | edit commits **before** P0 reads | edit is part of the run's baseline; nothing special | normal pipeline |
| E-b | edit commits between P0/P2 reads and P3 publish | the edited entity's `revision > baseline_revision` → excluded from all AI writes of this run; snapshot records a visible conflict entry (plan line 1397) instead | `manual_change_guard` inside the P3 transaction (SEC-§4.3) |
| E-c | edit attempts **during** the P3 transaction | impossible to interleave; SQLite allows one writer, so the edit either commits fully before the guard check (→ case E-b) or fully after commit (→ next run's baseline) | W1 unit-of-work isolation (RB-§4.1) + SEC-§4.3 protocol step 3 |
| E-d | user **rejects** the proposal while P2 is still reconciling | committed before P3 → fingerprint resolution sees `dismissed` and drops the candidate (R3, §5.2); committed after P3 → the rejection stands untouched, the published suggestion remains resolvable by the user afterwards (it never overwrites the dismissal) | §5.2 R3 + disjoint AI write surfaces (§3.1) |
| E-e | user **edits** the still-`suggested` row and then a retry/re-run reaches T5 | WR-2 precondition fails (`human_edited_at IS NOT NULL`) → update-in-place degrades to a no-op plus an additive `ai_comment`/conflict annotation; the user's field values survive byte-identically | WR-2 checked inside the same K7 transaction that would write (§3.2) |
| E-f | user deletes/tombstones a source the pending run cites | pointers fail validity at publish → the affected claims are dropped by the E1-style whole-publication rejection, job fails with `evidence_validation_failed` | SEC-§3.3 validity rule 1–3 applied at P3 |

**Winner rule (explicit):** in every case the last committed **human**
decision wins. An AI run never observes a human edit and then overwrites it;
the only observable orders are "AI output incorporates the edit", "AI output
excludes and flags the entity", or "AI output loses". There is no path by
which a later KI run mutates a human-touched value.

Compilable-shape sketch of the machine (interface contract, not an
implementation; std-only so `verify-sketch.ps1` can type-check it):

```rust
// src-tauri/src/managers/project_intelligence.rs (sketch — not yet implemented)
use std::collections::BTreeSet;

/// Proposal-level states (§2.1). Stored projection lives on `project_tasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalState {
    Suggested,
    Accepted,
    Rejected,
    Edited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Human,
    Ai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalEvent {
    Accept,
    Reject,
    EditProtectedColumn,
}

#[derive(Debug)]
pub enum ProposalError {
    /// T6 / total-function default: this actor may not drive this transition.
    IllegalTransition { from: ProposalState, by: Actor },
    /// WR-2 violated: AI write aimed at a human-flagged row.
    AiWriteToHumanFlaggedRow,
    /// TP-E1a: zero valid evidence pointers at publish time.
    EvidenceValidationFailed,
    /// TP-E1b: an AI-authored field lacks a covering valid pointer.
    EvidenceCoverageFailed { missing_fields: BTreeSet<&'static str> },
}

/// WR-2 precondition: an AI run may touch a suggestion row only while it is
/// pure-AI (`origin='ai'`, never human-edited, still `suggested`). Real impl
/// uses `chrono::DateTime<Utc>` for the edited marker (RB-§5.3); `Option<()>`
/// stands in for "set" in this dependency-free sketch.
fn ai_refresh_allowed(origin_is_ai: bool, human_edited: Option<()>, s: ProposalState) -> bool {
    origin_is_ai && human_edited.is_none() && s == ProposalState::Suggested
}

/// Total human transition function (T1–T4). AI actors are rejected outright:
/// AI influence flows exclusively through T0/T5, never through this function.
fn transition(from: ProposalState, by: Actor, ev: ProposalEvent) -> Result<ProposalState, ProposalError> {
    match by {
        Actor::Ai => Err(ProposalError::IllegalTransition { from, by }),
        Actor::Human => match (from, ev) {
            (ProposalState::Suggested, ProposalEvent::Accept)
            | (ProposalState::Edited, ProposalEvent::Accept) => Ok(ProposalState::Accepted),
            (ProposalState::Suggested, ProposalEvent::Reject)
            | (ProposalState::Edited, ProposalEvent::Reject) => Ok(ProposalState::Rejected),
            (ProposalState::Suggested, ProposalEvent::EditProtectedColumn)
            | (ProposalState::Edited, ProposalEvent::EditProtectedColumn) => Ok(ProposalState::Edited),
            // `accepted` and `rejected` are terminal proposal outcomes: no
            // outgoing transitions exist (§2.2); the task's own lifecycle
            // continues through ProjectManager/K7, not through this machine.
            (ProposalState::Accepted, _)
            | (ProposalState::Rejected, _) => Err(ProposalError::IllegalTransition { from, by }),
        },
    }
}
```

---

## 3. Precedence: manual edits win over later KI runs

### 3.1 Structural precedence (disjoint write surfaces)

Inherited unchanged from SEC-§4.1/§4.2 and PSC-§4 item 1, restated here as
the task-side closure: the complete set of AI-writable surfaces on
`project_tasks` is

1. insert of a **new** suggestion row (T0);
2. update-in-place of the row's **own** AI fields while the row is still
   `suggested` and unedited (T5);
3. additive `task_evidence` rows and additive `ai_comment` annotations on
   existing tasks (never mutating protected columns).

This set is disjoint from every protected column of human-flagged rows
(SEC-§4.1: rows where `origin='human'` OR `human_edited_at IS NOT NULL`).
There is no UPDATE path from any AI surface to such a row — precedence is a
type/module property, not a convention.

### 3.2 Operational precedence rules

- **WR-1 (plan authority).** Manual user changes outrank later KI runs
  (plan lines 263–265, 1395–1396, 1709–1710). Normative for all of §2–§5.
- **WR-2 (mechanical form of WR-1 for T5).** An AI update-in-place is
  permitted iff, evaluated **inside the writing transaction**, the target row
  satisfies `origin='ai' AND human_edited_at IS NULL AND
  state='suggested'`. The check shares the transaction with the write, so no
  interleaved human edit can slip between check and commit (same atomicity
  argument as SEC-§4.3 step 3). A failed check aborts the update portion and
  records the entity in the run's conflict payload — it never silently
  skips, and never partially applies.
- **WR-3 (terminal human decisions).** A human `dismissed` (= proposal
  `rejected`) or `done` decision suppresses all future suggestions carrying
  the same `dedupe_fingerprint`, permanently within v1 (no resurrection;
  plan line 1709 "überschreibt keine Nutzerentscheidung"). New contradicting
  evidence is surfaced as a snapshot conflict entry with pointers (E3 of
  SEC-§3.3), not as a new task row.
- **WR-4 (comment-only contradiction).** If new sources contradict a human
  decision or an edited proposal, the KI may comment (`ai_comment`) and the
  snapshot may flag the conflict; it may not revert status, priority, due
  date, assignee or text (plan line 1396 "Die KI darf sie kommentieren, aber
  weder Abschlussstatus noch Priorität unbemerkt zurücksetzen").

---

## 4. Evidence-linking requirements (gates G2 and G3)

### 4.1 Pointer model — consumed, not redefined (gate G3)

Every `task_evidence` row uses the SEC-§3.2 field set verbatim — `subject`
(the task UUID), `source_id` (required FK), `locator` = exactly one of
`source_item_id` OR `meeting_transcript_segment_id` (plan line 1297),
optional-but-bound `time_range`, `quote_excerpt`, `rationale` — and is
*valid* under the SEC-§3.3 definition (source alive and untombstoned,
locator exists/belongs to the source, quote still matches whitespace-
normalized, time range inside segment bounds). This contract introduces **no
new pointer format, no alternative locator kind, and no additional storage
columns**; divergence from SEC-§3 is a defect of this contract. Wherever SEC
and this document could be read differently on pointers, SEC wins.

### 4.2 Which fields need evidence

Classify every column an AI run writes on a proposal:

| Class | Fields | Obligation |
| --- | --- | --- |
| Claim fields | `title`, description/text | each requires coverage by ≥1 **valid** pointer (TP-E1b) |
| Derived metadata fields | `priority`, due date, proposed assignee label (verbatim from a cited source, §1) | each, when written non-null by AI, requires coverage like a claim field (TP-E1b) |
| Provenance metadata | `origin`, `confidence`, `model`, prompt version, envelope fields | exempt from evidence (they are *about* the write, not claims *in* the world); instead mandatory and pinned per SEC-§6 |
| Lifecycle fields | `state`, timestamps, revision | set by the store/human flow, never AI-authored |

"Coverage" means: the field is named in the `rationale` of at least one
valid pointer attached to this task (the rationale therefore doubles as the
field-to-proof link; a pointer may cover several fields, e.g. one chat
message proving both title and due date).

### 4.3 Enforceable publication invariant (gate G2)

**Invariant TP-E1:** *the insertion/update of an AI-originated proposal row
is rejected in full unless both clauses hold at write time, inside the same
transaction.*

- **TP-E1a (row level, plan lines 1300–1301).** The proposal carries ≥1
  evidence pointer that is valid per SEC-§3.3. This is the K8 rule of
  RB-§4.2 ("an AI task without ≥1 valid evidence is never published"),
  enforced at the KnowledgeStore boundary, not by caller discipline.
- **TP-E1b (field level, refinement).** Every non-null AI-authored field of
  the classes above is covered per §4.2. A write failing either clause is
  rolled back entirely (W1) and the job is marked `failed` with distinct
  error classes `evidence_validation_failed` (invalid/zero pointers) and
  `evidence_coverage_failed` (pointers exist but a field is uncovered) — the
  UI can explain the difference; the pipeline never silently strips single
  fields to force publication (mirroring SEC-§3.3 invariant E1's
  "never silently strips individual claims").

Rejection happens at the service boundary inside KnowledgeStore (extending
RB-§4 operation K8), consistent with SEC-§3.3 invariant E1 for M3/P3
publications: a task proposal that cannot prove itself is not degraded,
withheld silently, or published partially — the run fails loudly.

Sketch (continues the §2.3 module; typed shape of the TP-E1b check):

```rust
// src-tauri/src/managers/project_intelligence.rs (sketch — not yet implemented)

/// Fields whose AI-authored values demand pointer coverage (§4.2).
const COVERABLE_FIELDS: [&str; 5] =
    ["title", "description", "priority", "due_date", "assignee"];

/// Coverage inputs collected by the caller from the staged proposal.
struct FieldCoverage<'a> {
    /// Fields the AI actually sets non-null on this proposal.
    authored: BTreeSet<&'a str>,
    /// Fields cited in the `rationale` of at least one pointer that passed
    /// SEC-§3.3 validity at write time.
    covered_by_valid_pointers: BTreeSet<&'a str>,
}

/// TP-E1b: every AI-authored coverable field must be covered, or the whole
/// proposal write is rejected (§4.3). Pure function; the store invokes it
/// inside the K7/K8 transaction.
fn validate_field_coverage(
    cov: &FieldCoverage<'_>,
) -> Result<(), ProposalError> {
    let missing: BTreeSet<&str> = COVERABLE_FIELDS
        .into_iter()
        .filter(|f| cov.authored.contains(f) && !cov.covered_by_valid_pointers.contains(f))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ProposalError::EvidenceCoverageFailed { missing_fields: missing })
    }
}
```

### 4.4 Post-publication evidence decay

Pointers are pinned at publish time; sources may later be tombstoned or
edited (meeting unlink, plan lines 1432–1434: references become "zu prüfende
Belege markiert und nicht still entfernt"). Consistent with PSC-§3
(read-time aging of pins), a stored task pointer whose validity decays is
displayed as unverifiable — it is never auto-deleted, never rewritten by a
later run, and its decay blocks nothing except fresh reliance: a *new*
proposal must re-establish validity at its own publish time.

---

## 5. Deduplication of repeated proposals across runs (plan lines 1393–1394, 1643, 1709)

### 5.1 Fingerprint — consumed from SEC-§2.3

Each AI proposal carries
`dedupe_fingerprint = SHA-256(project_id, normalized_title,
normalized_assignee_or_null, due_date_or_null, semantic_feature_set_hash)`
computed by P2 from the merged evidence, exactly as SEC-§2.3 rule 1 defines.
This contract adds no alternative key; normalization details inherit from
SEC-§2.3. Storage: a `UNIQUE(project_id, dedupe_fingerprint)` constraint on
AI-origin rows (schema authority task-0020), inserted atomically with the
proposal (SEC-§2.3 rule 3).

### 5.2 Resolution rules (complete, ordered)

Before inserting a candidate (inside the P3 publish transaction):

| R# | Existing match | Action |
| --- | --- | --- |
| R1 | previous **AI suggestion**, same fingerprint, still `suggested`, `human_edited_at IS NULL` | **update-in-place** of that row's own AI fields (T5) under WR-2; stable UUID, revision and history preserved; no new row |
| R2 | **open human task** (`origin='human'`, `state='open'`) matched semantically | **merge**: no new task row; the excerpt is attached as additional `task_evidence`/`ai_comment` on the existing task (Phase 10 "Merge-Workflow", plan line 1701; SEC-§2.3 case 2) |
| R3 | human terminal decision (`done` or `dismissed`) with same fingerprint | **drop** the candidate; no resurrection, ever (WR-3) |
| R4 | no match | insert as new proposal (T0) under TP-E1 |

Two subtleties closed explicitly:

- **Retry vs. deliberate re-run.** A crashed/retried run keeps its
  idempotency key (SEC-§2.1) and replays idempotently; a deliberate rerun
  increments `rerun_generation`. Either way, publication resolves through
  R1–R4 against the *current* table state — generation affects run identity,
  never the right to duplicate a human decision.
- **Edited-then-reproposed.** A row in `edited` state with the same
  fingerprint is neither R1 (WR-2 fails) nor a resurrection target: the
  candidate is dropped and recorded as a conflict annotation on the snapshot,
  so the user sees that the KI still "sees" the task but did not touch their
  edits (WR-4). This prevents both silent divergence and silent overwrite.

### 5.3 Cross-run identity guarantees

- A suggestion's UUID survives refreshes (R1) — jump targets, comments and
  audit history stay attached (plan line 1393 "über stabile IDs …
  abgeglichen").
- Every T0/T5/R2 action produces its own `change_log` rows (tier-A, RB-§4.2
  K7/K8), so "which run wrote what when" is reconstructible from the log plus
  the pinned provider/model/prompt-version fields (§1, SEC-§6).
- The uniqueness constraint makes a double-insert a hard failure, not a UI
  filter: duplicates cannot exist even under concurrent runs on the same
  project (coalescing discipline of PSC-§1 admits at most one non-terminal
  intelligence job per project anyway).

---

## 6. Repository-seam operations available to ProjectIntelligenceService (RB-§6 closure)

### 6.1 Allowed operations

Per RB-§6 (line: "`ProjectIntelligenceService` … K7–K9, K10, reads") the
service — consuming `Arc<dyn KnowledgeRepository>`, never the concrete store
(RB-§2.1) — may call exactly:

| Seam op | Used for | Restrictions specific to this contract |
| --- | --- | --- |
| `upsert_task` (K7) | T0 insert, T5 refresh | AI calls carry `Actor::Ai`; TP-E1 validated in-transaction (§4.3); WR-2 prechecked and re-checked in-transaction (§3.2) |
| `attach_task_evidence` (K8) | R2 merge additions, additive pointers/comments on existing tasks | additive only; never removes or rewrites existing evidence |
| `publish_snapshot` (K9) | P3 publication incl. `snapshot_evidence` | owned by SEC-§1.2/PSC; listed here because it shares the publish transaction family |
| `claim_processing_job` / `finish_processing_job` (K10) | run envelope on `processing_jobs` | tier-B semantics, STT-style states (SEC-§1.4) |
| Reads: change_log-driven delta queries, `current_snapshot`, `search`, duplicate/fingerprint lookups, source/source-item/task queries | P0 delta, P2 reconcile | read-only; hits carry entity id + position (plan lines 1329–1331) |

### 6.2 Forbidden operations

- K1–K6 (project/source/link mutations) — owned by ProjectManager /
  SourceIngestionService; the intelligence service only reads those domains
  (RB-§6 matrix).
- Any `MeetingStore` operation — different repository, different domain.
- `transition_task` with `Actor::Ai` toward any human-flagged row (T6).
- Direct SQL, connections, or asset-byte access anywhere in
  `project_intelligence.rs` (RB-§7.1 R1/R4 allow-list excludes it by name).
- Human-facing transitions (T1–T4) issued by anything other than
  ProjectManager commands: the frontend reaches proposal resolution only via
  commands onto ProjectManager → K7 with `Actor::Human` (RB-§6, RB-§7.1 R5).

### 6.3 Atomicity clarification: evidence rides with the proposal

RB-§4.2 K8 already demands "an AI task without ≥1 valid evidence is never
published … enforced before K7 publishes AI-origin tasks". To satisfy that
*atomically* despite W1 (one transaction per public method), this contract
fixes the seam extension: **the `TaskUpsert` payload for AI-origin inserts
(T0) embeds the complete evidence set** (`Vec<NewTaskEvidence>`), and K7
validates TP-E1a+TP-E1b in the same transaction it inserts the task — the
K8 rules applied in-place. Standalone `attach_task_evidence` (K8 proper)
remains the seam for post-publication additive evidence (R2 merges, human
corrections). This is an interface extension of the RB-§2 sketches ("method
sets will be extended during implementation"), not a re-specification of the
store; schema and final signatures stay with task-0020 / implementation.

---

## 7. Worked counterexamples (defect-hunt record)

Checked deliberately while authoring; each is closed by a cited rule:

1. *"KI run finishes after user rejected the task."* → R3/E-d: candidate
   dropped; rejection survives byte-identically.
2. *"User accepts; a queued older run still wants to refresh."* → WR-2
   fails in-transaction (`state != 'suggested'` after T1); update aborts to
   conflict annotation.
3. *"Model outputs a plausible task with a fabricated segment id."* →
   locator resolution fails SEC-§3.3 rule 2 → TP-E1a rejects publication;
   injection cannot mint evidence (SEC-§7 rule 3).
4. *"Same meeting transcribed twice, two overlapping proposals."* → same
   fingerprint → R1 update-in-place; UNIQUE constraint backstops races
   (§5.3).
5. *"AI sets priority='urgent' citing only a message that proves the
   title."* → TP-E1b: `priority` uncovered → `evidence_coverage_failed`;
   whole write rolled back.
6. *"User edits title of a suggestion; next run extracts the corrected
   title independently."* → new fingerprint ≠ old → R4 would insert a near-
   duplicate; mitigated, not eliminated, in v1: the semantic feature-set hash
   component of the fingerprint (SEC-§2.3) is computed from evidence
   locators as well as text, so identical evidence usually re-matches the
   edited row's original fingerprint → R1 blocked by WR-2 → §5.2 subtlety 2
   applies (drop + visible annotation). Residual cosmetic duplication risk
   is accepted and listed in §8 as budget, not solved with fuzzy matching
   machinery.

---

## 8. Non-goals and complexity budget (restated, gate G4)

Beyond §0.2: no fuzzy/ML duplicate merging (fingerprint only, §7 item 6
records the residual), no proposal expiry/TTL, no bulk accept-all UX
contract, no notification scheduling, no cross-project suggestion sharing,
no learning loops over accept/reject statistics, no MCP write tools. Each is
rejected for v1 as low-evidence, high-complexity surface; revisit only via
the documented discovery cycle with evidence-backed need.

## 9. Gate verification checklist

| Gate | Requirement | Where satisfied |
| --- | --- | --- |
| G1 | State machine enumerates all transitions incl. user edits during a pending KI run, winner rule explicit | §2.1 (states/projections), §2.2 (T0–T6 complete table), §2.3 (exhaustive race cases E-a…E-f), §3.2 (WR-1…WR-4) |
| G2 | Every AI-written field carries a `task_evidence` row or the write is rejected; enforceable rule stated | §4.2 (field classification), §4.3 (TP-E1a/TP-E1b, error classes, in-transaction enforcement at K7/K8 boundary) |
| G3 | Consistent with summary-evidence-contract §3 pointer model — no divergent format | §4.1 (verbatim adoption of SEC-§3.2 fields and SEC-§3.3 validity; explicit no-new-format ruling; precedence clause) |
| G4 | Non-goals section; documentation index updated | §0.2 + §8; row added to `docs/meeting-mode-documentation-index.md` §3 table and reading-order list |
