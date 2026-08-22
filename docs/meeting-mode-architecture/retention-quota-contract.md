# Retention, Quota & Change-Log Pruning Policy-Engine Contract

Status: **spec contract — no implementation yet** (Phase 3 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §12 ("Speicher- und
Datenschutzlogik", lines 1436–1476), §11 ("Meeting-Bibliothek und Export",
lines 1405–1434), §9 (`change_log`, lines 1320–1326) and §2 ("Bewusst nicht
Bestandteil von v1"). Normative inputs consumed read-only:

- [asset-store-contract.md](asset-store-contract.md) — retention hooks and the
  automatic-deletion enforcement point (**AS-§8**), startup scan (**AS-§7**),
  selective deletion levels and their ordering (**AS-§9**). Abbreviated
  **AS-§n**.
- [disk-guard-contract.md](disk-guard-contract.md) — byte-rate derivations
  (**DG-§2**), the `ended_disk_full` retention block (**DG-§6**). Abbreviated
  **DG-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) — trait
  seam and the SQL choke-point rules (**RBC-§7.1**, R1 SQL allow-list).
- [module-placement-map.md](module-placement-map.md) — canonical component
  homes (B3 `MeetingStore`, B11 `AssetStore`, `storage/` module root).

Planned implementation homes (all design-phase, none exists):

| Concern | Planned module |
| --- | --- |
| Pure policy evaluation (this contract's core) | `src-tauri/src/storage/retention_policy.rs` |
| Trigger wiring + action executor (the "sweep") | `src-tauri/src/storage/retention_sweeper.rs` |

Both files sit under the `storage/` module root introduced by B11
(module-placement-map §3); neither file may contain SQL — snapshot assembly
and row mutations go exclusively through `MeetingStore` (B3) and the
`change_log` writer module allowed by RBC-§7.1 R1. The frontend never calls
either module directly (AS-§1, plan §3 Phase-3 acceptance).

Conflict rule (identical to the documentation index): the plan wins over the
sibling contracts, the sibling contracts win over catalogs, and nothing wins
over actual code. Where this contract extends AS-§8 (one new enum variant,
one new trigger set), the extension is declared normative here and flagged as
an interface addition owned by the storage workstream (§11).

This document defines the policy-engine contract only; it changes no source
code.

---

## 1. Scope and module boundaries

The retention/quota policy engine owns exactly one concern: **deciding, as a
pure function of stored state, which maintenance actions (automatic audio
deletion, `change_log` pruning, advisory warnings) are due, in which fixed
order — and nothing else.**

It does _not_ own:

| Concern | Owner | Interaction |
| --- | --- | --- |
| Unlinking files, delete levels 1–3 | `AssetStore` (AS-§5.1, AS-§9) | executes `PolicyAction::DeleteAudioLevel1`; refuses anything but level-1 keys from the sweeper |
| "May this meeting's audio be auto-deleted?" at execution time | `RetentionGuard` (AS-§8.2, implemented by `MeetingStore`) | consulted inside the sweep loop immediately before the first unlink; binding, no default-allow |
| Session/job/summary state truth | `MeetingStore` (B3) | produces the `PolicyInput` snapshot via read-only queries |
| `change_log` writes and row deletion | `change_log` module (RBC-§7.1 R1 allow-list) | appends audit rows; executes the prune under transaction |
| Disk-full end-state flag | disk guard (DG-§4.2 step 4, DG-§6.2) | sets `meeting_sessions.ended_disk_full`; this contract consumes it |
| UX rendering of notices | frontend via events | receives typed sweep-result events; renders the §6 notice keys |
| Import admission limits (ZIP bombs, per-file caps) | import admission (plan §12 lines 1463–1468) | orthogonal: bounds one import; never consults or mutates retention policy |

---

## 2. Quota accounting units (the three ledgers)

All quantities are **derived by query, never stored as counters**. A stored
counter can drift from reality; a `SUM` cannot. There are no ledger tables
(non-goal NG-5). Every figure below is assembled by `MeetingStore` /
`KnowledgeStore` read queries at trigger time.

| # | Ledger | Unit | Definition (exact source rows) | Consumed by |
| --- | --- | --- | --- | --- |
| L1 | Audio bytes | bytes | `SUM(file_size_bytes)` over `meeting_audio_chunks` rows in state `ready` across all meetings. `.part` files and `writing` rows are excluded (transient, owned by the recorder, AS-§4 invariant 2). `damaged` rows excluded (quarantined; resolved by AS-§7, not by quota). | Rules R1/R2 (§4), `PolicyInput.audio_bytes_total` |
| L2 | Asset bytes | bytes | `SUM(size_bytes)` recorded on `assets` rows (imports, exports-as-assets, media blobs) plus the `export/` and `recovery/` files reported by the last per-meeting tree stat. Accounted **only** for diagnostics and the post-import trigger report; no v1 rule acts on it. | Post-import report (T3); user-facing storage info |
| L3 | Change-log rows | rows | `COUNT(*)` over `change_log`, split by age against the §9 cutoff. | Rule R3 (§4) |

Why L2 carries no rule: plan §12 defines retention rules for **meeting audio**
("Aufbewahrungsregel" applies to recordings; lines 1444–1452) and per-import
admission limits (line 1465), but no aggregate asset budget. Imported originals
are irreplaceable user data; inventing an auto-delete rule for them would be
feature-bloat with data-loss risk (rejected explicitly in §11, NG-6). L2 is
therefore measured and reported, never acted upon automatically.

Byte figures reuse DG-§2 arithmetic verbatim:

```text
PER_SOURCE_BYTES_PER_HOUR   = 115_200_000 B          (DG-§2.1, plan §12)
DUAL_TRACK_BYTES_PER_HOUR   = 230_400_000 B          (DG-§2.1, plan §12)
```

---

## 3. Evaluation triggers

Policy evaluation runs at four named points — the three from the tasking
(startup, post-finalize, post-import) plus the daily timer that AS-§8.2
mandates ("invoked at app start and on a daily timer"); a long-running app
without the timer would never age out anything between restarts, so T4 is
retained verbatim from AS-§8.2, not invented here.

| # | Trigger | Fires | Mandatory preceding condition | Notes |
| --- | --- | --- | --- | --- |
| T1 | Startup | Once per app start, after init | **The AS-§7 startup recovery scan has completed for every `meetings/<id>/` directory.** The scan owns crash-residue classification; evaluating policy against unscanned state could act on rows whose files are quarantined or missing. Order is normative: scan → evaluate → UI. | Same entry point AS-§8.2 already names |
| T2 | Post-finalize | Immediately after the transaction that marks a session's processing **confirmed complete** (sets `completed_at`, session → `completed`) | That transaction committed. This is the same instant at which the sweep clock starts (AS-§8.1) and at which the first-meeting retention question (catalog P6) may be shown. | Guarantees `DeleteImmediately` takes effect without waiting a day |
| T3 | Post-import | Immediately after `commit_import` (AS-§2.2) and its DB transaction commit | Import admitted through the plan-§12 checks | Produces an L2 ledger report row in `change_log` only; **no rule acts on L2** (§2). No automatic asset deletion ever |
| T4 | Daily timer | Every 24 h while the app runs | None (runs after the previous pass ends; passes never overlap) | AS-§8.2 obligation |

Every trigger performs the identical three-stage sequence (§5.2). Triggers
never run concurrently with each other: the sweeper holds a process-wide lock
and a trigger arriving mid-pass is coalesced into the running pass (worst case
it repeats on T4). This makes "the evaluation order" a total order, which is
what gate G2 requires.

---

## 4. Policy rule model (v1 fixed set — no DSL)

### 4.1 The rules

Exactly three rules exist in v1. They are code, not configuration; there is no
user-visible or file-based rule definition (gate G4, NG-1).

| # | Rule | Acts on | Semantics |
| --- | --- | --- | --- |
| R1 | **Age rule** (plan §12 lines 1444–1452) | L1, per session | For each session whose stored `recording_retention_policy` is `delete_immediately` / `after_7_days` / `after_30_days` and whose sweep clock (`completed_at`, AS-§8.1) is at least the policy's delay in the past → emit `DeleteAudioLevel1 { reason: RetentionAge }`. |
| R2 | **Quota rule** | L1, global | When the **projected post-pass** audio total — the raw ledger minus bytes freed by already-emitted R1 actions, so a meeting selected by both rules frees its bytes exactly once — exceeds `AUDIO_QUOTA_HARD_BYTES` (§4.2), emit `DeleteAudioLevel1 { reason: QuotaPressure }` for eligible sessions oldest-first until the projection is at or below `AUDIO_QUOTA_SOFT_BYTES`, or candidates are exhausted (then: `QuotaStuckWarning`). |
| R3 | **Change-log prune** | L3 | When `change_log` holds rows older than `CHANGE_LOG_RETENTION_DAYS`, emit one `PruneChangeLog` action (§9). |

### 4.2 Constants and their derivations

No constant is invented freely; each is an arithmetic multiple of the DG-§2.1
rate or a stated product decision tied to a plan sentence.

| Constant | Value | Derivation |
| --- | --- | --- |
| `AUDIO_QUOTA_HARD_BYTES` | `100 × 230_400_000 = 23_040_000_000` (~21.5 GiB) | Plan fixes the byte rate (§12 lines 1438–1439) but no quota number; 100 dual-track hours is the smallest round multiple that is provably irrelevant to normal use: the maximal modeled v1 session is 10 h (AS-§7 V5 rationale), so even ten back-to-back maximal meetings stay under half the hard quota. Product decision recorded here, changeable only in code with a test update (NG-1). |
| `AUDIO_QUOTA_SOFT_BYTES` | `80 × 230_400_000 = 18_432_000_000` | Hard − soft = 20 h of headroom, strictly greater than one maximal 10 h session: a single pass can never thrash — after reaching soft, the next pass cannot cross hard again without ≥ 10 h of new audio. |
| `CHANGE_LOG_RETENTION_DAYS` | `180` | Plan §9: `change_log` serves incremental project analysis and diagnosis in v1; sync clients are an explicit plan non-goal (§2 line 85). 180 days bounds growth while keeping every audit row written by AS-§8.2 rule 5 answerable for half a year ("warum ist mein Audio (noch da/weg)?"). |
| `CHANGE_LOG_PRUNE_MAX_ROWS_PER_PASS` | `1000` | Bounds a single pass's transaction size and lock hold time; leftover expired rows are pruned on the next trigger (T4 recurs daily). |
| Policy delays | `0 / 7 d / 30 d` | Plan §12 line 1449 verbatim option list; `keep_permanent` and unanswered → no delay, never swept. |

### 4.3 Exemptions (who is never swept)

Two dimensions, both plain stored flags — no rule objects, no expressions:

1. **Per-session policy exemptions** (plan-native): `Unanswered` (NULL
   `recording_retention_policy`) and `KeepPermanent` are never eligible, for
   R1 *and* R2 alike. "Dauerhaft behalten" (plan §12 line 1449) is a promise;
   quota pressure does not override promises.
2. **Project-scoped exemption**: a new boolean column
   `projects.protect_meeting_audio INTEGER NOT NULL DEFAULT 0`. While set,
   every meeting assigned to the project (via `project_sources`) is ineligible
   for R1 and R2. This is the minimal hook for "this project's raw audio must
   survive"; per-project quotas, per-project ages or any richer scoping are
   rejected (NG-2).

Additionally the disk-guard block input is consumed (not an exemption but a
hard block, see §7): `ended_disk_full = true` (DG-§6.2) makes a session
ineligible regardless of policy.

### 4.4 Early deletion under quota pressure (normative clarification)

R1 never deletes before a session's chosen window elapses. R2 is different by
design: under hard-quota pressure it may select an eligible session whose
`After7Days`/`After30Days` window has **not yet fully elapsed** (oldest sweep
clock first), because after R1 has exhausted every due session the only
remaining candidates are not-yet-due ones — an R2 restricted to due sessions
could never free a single byte. This early deletion applies **only** to
sessions with an answered timed policy; `Unanswered`, `KeepPermanent`,
project-protected and disk-full-flagged sessions are excluded exactly as in
R1 (§4.3), so no promise and no default is ever overridden. The user is told
afterwards, honestly and with real numbers, via N2 (§6.2) and the per-meeting
`change_log` rows (AS-§8.2 rule 5); the alternative — silently filling the
disk until capture fails mid-recording (the disk-guard emergency) — is the
worse failure. This sentence is the normative basis for R2's candidate filter;
it is deliberately explicit rather than implied by the code sketch.

### 4.5 Schema additions (owned by the storage workstream)

| Change | Table | Reason |
| --- | --- | --- |
| `protect_meeting_audio INTEGER NOT NULL DEFAULT 0` | `projects` | §4.3 item 2 |
| `ended_disk_full INTEGER NOT NULL DEFAULT 0` | `meeting_sessions` | **Adoption of DG-§7 open item 1** (the interface note in DG-§6.2 rule 3). DG-§6.2 correctly notes AS-§8.2's `RetentionBlockReason` lacks a variant; this contract declares the variant `SessionEndedDiskFull` (§5.1 sketch) and thereby closes that open interface item. Both migrations land with the Phase-3 storage work. |

No further columns: the ledgers are queried, not cached (§2).

---

## 5. The pure evaluation function (gate G2)

### 5.1 Types and sketch

The Rust blocks below are extracted and compiled std-only (Appendix B).
Types that mirror AS-§8.2/DG-§6.2 are restated locally so each contract
sketch compiles standalone; the single authoritative definition lives in the
named contract.

```rust
//! Sketch of the planned `src-tauri/src/storage/retention_policy.rs`.
//! PURE: no I/O, no clock access, no randomness, no global state.
//! Determinism contract: identical `PolicyInput` ⇒ identical `Vec<PolicyAction>`.

/// Dual-track rate, adopted from disk-guard-contract §2.1 (plan §12).
pub const DUAL_TRACK_BYTES_PER_HOUR: u64 = 230_400_000;

/// R2 trigger threshold: 100 dual-track hours (derivation §4.2).
pub const AUDIO_QUOTA_HARD_BYTES: u64 = 100 * DUAL_TRACK_BYTES_PER_HOUR;

/// R2 deletion floor: 80 dual-track hours (derivation §4.2).
pub const AUDIO_QUOTA_SOFT_BYTES: u64 = 80 * DUAL_TRACK_BYTES_PER_HOUR;

/// R3 cutoff in days (derivation §4.2).
pub const CHANGE_LOG_RETENTION_DAYS: u64 = 180;

/// R3 per-pass row cap (derivation §4.2).
pub const CHANGE_LOG_PRUNE_MAX_ROWS_PER_PASS: u64 = 1000;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const DELAY_7_DAYS_MS: u64 = 7 * DAY_MS;
const DELAY_30_DAYS_MS: u64 = 30 * DAY_MS;
const RETENTION_180_DAYS_MS: u64 = 180 * DAY_MS;

/// Per-session stored policy. Mirrors AS-§8.1 (`RetentionPolicy`) plus its
/// in-memory `Unanswered` state; `NULL` in the DB maps to `Unanswered`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredRetentionPolicy {
    Unanswered,
    DeleteImmediately,
    After7Days,
    After30Days,
    KeepPermanent,
}

/// Mirrors AS-§8.2 `RetentionBlockReason` extended by the DG-§6.2 variant
/// this contract adopts (§4.5). Authoritative listing lives in AS-§8.2 +
/// DG-§6.2; repeated here only for sketch compilability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionBlockReason {
    UnconfirmedSttJob,
    FailedSttJob,
    ProcessingIncomplete,
    SessionEndedDiskFull,
}

/// One session's decision-relevant projection of stored state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Lower-case hyphenated UUID (AS-§2.2); lexicographic order is the
    /// deterministic tie-break everywhere below.
    pub meeting_id: String,
    pub policy: StoredRetentionPolicy,
    /// Sweep-clock origin: confirmed completion of processing (AS-§8.1).
    /// `None` ⇒ never due (audio survives through processing, plan §12).
    pub completed_at_unix_ms: Option<u64>,
    /// Session state is an active state (draft..summarizing, SSC states).
    pub session_open: bool,
    /// STT jobs in queued | running | retry_wait (AS-§8.2 rule 2).
    pub unfinished_stt_jobs: u32,
    /// STT jobs in failed (plan §12 line 1451–1452).
    pub failed_stt_jobs: u32,
    /// Summary/processing pending for the session (AS-§8.2 rule 3).
    pub processing_incomplete: bool,
    /// Disk-guard end-state flag (DG-§6.2).
    pub ended_disk_full: bool,
    /// Any assigned project has `protect_meeting_audio = 1` (§4.3).
    pub project_protected: bool,
    /// This session's contribution to ledger L1 (state `ready` bytes only).
    pub ready_audio_bytes: u64,
}

/// Everything `evaluate` may read. Assembled by the sweeper from stored
/// state only (§5.2 stage A). The wall clock enters exclusively as
/// `now_unix_ms` — the function itself reads no clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyInput {
    pub now_unix_ms: u64,
    /// Ledger L1 total across all sessions (§2).
    pub audio_bytes_total: u64,
    pub sessions: Vec<SessionSnapshot>,
    /// Ledger L3 rows older than the §9 cutoff.
    pub change_log_expired_rows: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletionReason {
    /// R1: stored policy window elapsed.
    RetentionAge,
    /// R2: global audio quota pressure, oldest-first.
    QuotaPressure,
}

/// The complete set of actions the engine can emit. Anything not in this
/// enum is by construction impossible for the policy layer to request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyAction {
    /// Automatic delete **level 1** (AS-§9.1, column 1) — the only automatic
    /// file-deletion action this engine can ever emit. Execution additionally
    /// requires the runtime guard (§7); this action is necessary, not sufficient.
    DeleteAudioLevel1 { meeting_id: String, reason: DeletionReason },
    /// DB-row-only pruning inside change_log (§9). Never touches files.
    PruneChangeLog { older_than_unix_ms: u64, max_rows: u64 },
    /// Advisory only: quota exceeded but every candidate was blocked or
    /// exempt. Deletes nothing; surfaces §6 row N4.
    QuotaStuckWarning { audio_bytes_total: u64 },
}

impl SessionSnapshot {
    fn delay_ms(&self) -> Option<u64> {
        match self.policy {
            StoredRetentionPolicy::DeleteImmediately => Some(0),
            StoredRetentionPolicy::After7Days => Some(DELAY_7_DAYS_MS),
            StoredRetentionPolicy::After30Days => Some(DELAY_30_DAYS_MS),
            StoredRetentionPolicy::Unanswered | StoredRetentionPolicy::KeepPermanent => None,
        }
    }

    /// Snapshot-time half of the G3 double gate (§7). Execution-time half is
    /// `RetentionGuard::audio_deletion_allowed` (AS-§8.2 rule 1).
    /// Policy exemptions apply to BOTH rules (§4.3): `Unanswered` and
    /// `KeepPermanent` are never deletable, not even under quota pressure.
    fn deletion_eligible(&self) -> bool {
        !matches!(
            self.policy,
            StoredRetentionPolicy::Unanswered | StoredRetentionPolicy::KeepPermanent
        ) && !(self.session_open
            || self.unfinished_stt_jobs > 0
            || self.failed_stt_jobs > 0
            || self.processing_incomplete
            || self.ended_disk_full
            || self.project_protected)
    }

    fn due_at(&self) -> Option<u64> {
        self.delay_ms()?.checked_add(self.completed_at_unix_ms?)
    }
}

/// The policy evaluation. Fixed internal order (§5.2 stage B); total
/// determinism; total order over emitted actions.
pub fn evaluate(input: &PolicyInput) -> Vec<PolicyAction> {
    let mut actions: Vec<PolicyAction> = Vec::new();

    // Rule R1 — age. Order: due_at ascending, tie-break meeting_id ascending.
    let mut aged: Vec<&SessionSnapshot> = input
        .sessions
        .iter()
        .filter(|s| s.deletion_eligible())
        .filter(|s| matches!(s.due_at(), Some(due) if due <= input.now_unix_ms))
        .collect();
    aged.sort_by(|a, b| {
        (a.due_at().unwrap_or(0), a.meeting_id.as_str())
            .cmp(&(b.due_at().unwrap_or(0), b.meeting_id.as_str()))
    });
    for s in &aged {
        actions.push(PolicyAction::DeleteAudioLevel1 {
            meeting_id: s.meeting_id.clone(),
            reason: DeletionReason::RetentionAge,
        });
    }

    // Rule R2 — quota. Operates on the PROJECTED POST-PASS total: the raw
    // ledger minus bytes already freed by R1 actions above (a meeting
    // selected by both rules frees its bytes exactly once). Only considered
    // when even that projection crosses the hard threshold; deletes
    // oldest-first down to the soft floor, skipping sessions already
    // selected by R1 (no meeting is deleted twice in one pass).
    let r1_freed: u64 = aged.iter().map(|s| s.ready_audio_bytes).sum();
    let mut remaining = input.audio_bytes_total.saturating_sub(r1_freed);
    if remaining > AUDIO_QUOTA_HARD_BYTES {
        let already_selected: std::collections::BTreeSet<String> = actions
            .iter()
            .filter_map(|a| match a {
                PolicyAction::DeleteAudioLevel1 { meeting_id, .. } => Some(meeting_id.clone()),
                _ => None,
            })
            .collect();
        let mut candidates: Vec<&SessionSnapshot> = input
            .sessions
            .iter()
            .filter(|s| s.deletion_eligible())
            .filter(|s| s.completed_at_unix_ms.is_some()) // audio survives processing
            .collect();
        candidates.sort_by(|a, b| {
            (
                a.completed_at_unix_ms.unwrap_or(0),
                a.meeting_id.as_str(),
            )
                .cmp(&(
                    b.completed_at_unix_ms.unwrap_or(0),
                    b.meeting_id.as_str(),
                ))
        });
        for s in &candidates {
            if remaining <= AUDIO_QUOTA_SOFT_BYTES {
                break;
            }
            if already_selected.contains(&s.meeting_id) {
                continue;
            }
            remaining = remaining.saturating_sub(s.ready_audio_bytes);
            actions.push(PolicyAction::DeleteAudioLevel1 {
                meeting_id: s.meeting_id.clone(),
                reason: DeletionReason::QuotaPressure,
            });
        }
        if remaining > AUDIO_QUOTA_SOFT_BYTES {
            actions.push(PolicyAction::QuotaStuckWarning {
                audio_bytes_total: input.audio_bytes_total,
            });
        }
    }

    // Rule R3 — change_log prune. Always last (§5.2 stage B order).
    if input.change_log_expired_rows > 0 {
        actions.push(PolicyAction::PruneChangeLog {
            older_than_unix_ms: input.now_unix_ms.saturating_sub(RETENTION_180_DAYS_MS),
            max_rows: input
                .change_log_expired_rows
                .min(CHANGE_LOG_PRUNE_MAX_ROWS_PER_PASS),
        });
    }

    actions
}
```

### 5.2 The three-stage sequence (fixed, normative)

```text
Stage A — snapshot assembly (sweeper, ordered):
  A1. acquire the sweep lock (coalesce concurrent triggers, §3)
  A2. read sessions + jobs + summaries + project flags via MeetingStore
      (read-only, one consistent snapshot per SQLite's serializable reads)
  A3. read change_log age split via the change_log module (RBC-§7.1 R1)
  A4. build PolicyInput; now_unix_ms taken ONCE here

Stage B — pure evaluation (retention_policy::evaluate, §5.1):
  fixed internal emission order: R1 actions → R2 actions → QuotaStuckWarning
  → R3 action. Within R1: (due_at asc, meeting_id asc). Within R2:
  (completed_at asc, meeting_id asc). R3 emits at most one action.

Stage C — execution (sweeper, in emitted list order):
  for each action, §8's ordered procedure; results aggregated into one
  sweep report event + one change_log summary row.
```

### 5.3 Why this satisfies G2

1. **Pure function of stored state.** `evaluate` reads only `&PolicyInput`;
   `PolicyInput` is assembled exclusively from SQLite rows and the single
   `now_unix_ms` captured in stage A4. No environment, filesystem, RNG or
   clock access exists in the module (compilable proof: the sketch above
   references no `std::time`, `std::fs` or `std::io` symbol).
2. **Deterministic and testable.** Same input ⇒ same output vector,
   including order: every iteration walks a vec sorted by a total order
   (UUID tie-break), and `BTreeSet` iteration is not relied upon for order
   anywhere. Golden tests pin whole output vectors for constructed inputs;
   a property test asserts `evaluate(x) == evaluate(x)` and that re-running
   with `now` rewound never produces actions that a forward run would not.
3. **Fixed evaluation order.** §5.2 stages A→B→C and stage B's internal
   order are normative text; tests assert positional expectations (age
   actions before quota actions, prune last).

---

## 6. What is auto-deleted vs. user-confirm-only — and every notice key (gate G1)

### 6.1 The decision matrix

| Action class | Automatic? | Files touched | Confirmation |
| --- | --- | --- | --- |
| R1/R2 `DeleteAudioLevel1` | **Yes — the only automatic file deletion in the product** | `audio/*.wav` + `export/*` of that meeting (AS-§9.1 column 1 exactly); chunks → `deleted`, transcript + summary kept | none (that is its purpose, plan §12); guarded per §7, noticed per §6.2 N1/N2 |
| R3 `PruneChangeLog` | Yes | none (DB rows only) | none; invisible by design (N3) |
| `QuotaStuckWarning` | n/a (advisory) | none | surfaces a warning (N4) |
| Recovery quarantine removal (`recovery/` contents, orphan `.part`) | **Never automatic** (AS-§7 verbatim: "The scan never deletes audio content automatically") | — | explicit user confirmation only |
| Delete level 1 (manual) | No | as above | confirmation dialog (catalog P8) |
| Delete level 2 | No | transcript + summary rows | dialog (P9) |
| Delete level 3 | No | entire meeting tree | two-step dialog (P10) |
| Imported assets / media (L2 ledger) | **Never automatic** | — | user-initiated only; overage is reported, not acted on (§2, NG-6) |

### 6.2 Notice-key table (G1: every automatic deletion names its key)

Catalog convention: existing reusable keys come from
[docs/meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md)
§2.3; proposed-new keys follow its §2.2 namespace rules and carry the same
proposal-only status. None of these keys exists in
`src/i18n/locales/en/translation.json` today (catalog §1 verification stands).

| # | User-visible outcome | Trigger | Catalog entry | i18n key paths |
| --- | --- | --- | --- | --- |
| N1 | Audio auto-deleted because the retention window elapsed | R1 executed | **new, proposed:** `meeting.retention.autoDeleted.title` / `.bodyAge` | title: `“{{title}}”: old recording audio was removed.`; body: `{{count}} meeting(s) cleaned up — {{freedMb}} MB freed. Transcript and summary were kept.` Buttons: `meeting.actions.openMeeting` (exists), `meeting.actions.close` (exists) |
| N2 | Audio auto-deleted because of storage quota | R2 executed | **new, proposed:** `meeting.retention.autoDeleted.title` (shared) / `.bodyQuota` | body: `Storage was running low, so audio of {{count}} oldest finished meeting(s) ({{freedMb}} MB) was removed. Transcript and summary were kept.` Same buttons as N1 |
| N3 | change_log prune executed | R3 executed | none (invisible) | No user-facing copy is required or defined — same justification pattern as disk-guard U7: the prune is bookkeeping with zero user-visible effect; it is auditable via its own `change_log` marker row (§9) |
| N4 | Quota exceeded but nothing could be deleted safely | `QuotaStuckWarning` | **new, proposed:** `meeting.retention.quotaStuck.title` / `.body` | body: `Meeting audio is using {{usedGb}} GB. Nothing could be cleaned up automatically because every meeting is protected, kept permanently, or still processing. Review storage in settings.` Buttons: `meeting.actions.openSettings` (exists), `meeting.actions.close` (exists) |
| N5 | Sweep skipped a meeting (guard block) | §7 block at execution | none (invisible) | Surfaced only indirectly: audio simply persists; the block reason lands in `change_log` per AS-§8.2 rule 5 (identical to disk-guard U7). No notification for a *non*-event |
| N6 | Retention question after first successful processing | T2, first completion | **exists (proposal):** catalog **P6** | `meeting.retention.question.title/.body`, options `meeting.retention.option.deleteNow|.7days|.30days|.forever`, `meeting.actions.decideLater` |

Honesty constraints on N1/N2 copy (plan §6.2 principle 5): the notice MUST be
rendered from the sweeper's aggregate `DeleteReport` data (removed bytes =
`DeleteReport.removed_bytes` summed over the pass, AS-§5.1) and MUST be shown
only when at least one deletion actually happened; a pass with zero applied
actions produces no notification at all.

---

## 7. The G3 invariant — automatic deletion can never hit open sessions or unfinished STT jobs

**Claim:** no code path can automatically unlink audio of a meeting that (a)
has an open session, (b) has an unfinished (queued/running/retry_wait) STT
job, (c) has a failed STT job, (d) has incomplete processing, (e) ended in a
disk-full state, or (f) is project-protected / policy-exempt.

### 7.1 Enumeration of every unlink path for meeting audio

| # | Path | Automatic? | Protection |
| --- | --- | --- | --- |
| U-P1 | Sweeper executing `DeleteAudioLevel1` | Yes | **Double gate** — §7.2 |
| U-P2 | User-initiated delete levels 1–3 | No (explicit, confirmed in the same UI session, plan §11 line 1431) | Outside the invariant's scope by definition; the confirmation dialog displays unconfirmed/failed jobs as a warning (AS-§8.2 rule 4) |
| U-P3 | AS-§7 startup scan | n/a | Deletes nothing, verbatim: "The scan never deletes audio content automatically" (AS-§7 closing paragraph) |
| U-P4 | Recovery quarantine | n/a | Moves files (`recovery/`), removes nothing without explicit confirmation (AS-§4 invariant 4) |
| U-P5 | `AssetStore::delete` / `delete_meeting_tree` called by anyone else | n/a | No such caller exists or may exist: the sweeper is the only automatic client, and module-placement-map E-rules confine `storage/` access to the stores; a new caller would be a reviewable defect against AS-§8.2's named-enforcement-point rule |

The enumeration is exhaustive because `AssetStore` is the single resolution
and deletion choke point (AS-§2.1 rule 2) and its delete methods have exactly
the callers listed (verified against the §1 boundary tables of AS and this
contract; no other module may reach `storage/`).

### 7.2 The double gate on U-P1

Gate 1 — **snapshot-time eligibility** (`SessionSnapshot::deletion_eligible`,
§5.1): the engine cannot even emit `DeleteAudioLevel1` for a meeting unless
its snapshot shows no open session, zero unfinished jobs, zero failed jobs,
complete processing, no disk-full flag, and no exemption. Note the open-session
case is doubly covered: an active session has `completed_at = None` (clock
never started, AS-§8.1) *and* `processing_incomplete = true`.

Gate 2 — **execution-time guard** (binding, AS-§8.2 rule 1 adopted verbatim):
inside the sweep loop, per meeting, *immediately before the first unlink*, the
sweeper calls `RetentionGuard::audio_deletion_allowed(meeting_id)`; there is
no default-allow implementation, and `Blocked(_)` skips the meeting and logs
the reason. The guard's block-reason set is extended here by
`SessionEndedDiskFull` (the DG-§6.2 rule-3 interface note is closed by
§4.5).

Why two gates: the snapshot can go stale between stage A and stage C (e.g. a
retry of a previously failed job flips a session back to unfinished). Gate 1
makes the *decision* conservative and testable; Gate 2 makes the *deletion*
safe against races, because it re-reads the live job/session state
transactionally at the last possible moment. A meeting that becomes blocked
after snapshotting is skipped and retried on the next trigger; a meeting that
becomes *more* deletable can only do so by passing through a state transition
which itself occurs after a fresh guard read on some future pass — it can
never retroactively legitimize an already-made deletion.

### 7.3 Proof obligations turned into tests

1. Property test: for any `PolicyInput` where session X has `session_open`,
   any unfinished job, any failed job, `processing_incomplete`,
   `ended_disk_full`, `project_protected`, `Unanswered` or `KeepPermanent`,
   `evaluate` contains no `DeleteAudioLevel1` for X (each flag fuzzed
   independently).
2. Ordering test with an instrumented fs seam: for each executed deletion,
   `audio_deletion_allowed` is observed called *strictly before* the first
   `unlink` syscall of that meeting.
3. Race test: enqueue an STT job for meeting Y between stage A and stage C;
   assert Y's files untouched and a `change_log` block row with reason
   `UnconfirmedSttJob` exists.
4. Negative-space test: no API on the sweeper accepts a raw storage key or
   meeting id for deletion — the only public entry point is
   `run_triggered(trigger)` with no parameters, so no caller can inject a
   target that skipped stage B.

Together these prove the invariant structurally (single automatic path,
double-gated) rather than by convention.

---

## 8. Ordered interplay with the AS-§9 deletion levels

Normative rules, in order:

1. **Automatic deletions are exactly level 1.** The sweeper requests, and
   `AssetStore` executes, AS-§9.1 column 1 semantics and nothing else: all
   `audio/*.wav` (+ stray `.part`), all `export/*`; chunk rows → `deleted`
   with checksums kept; transcripts, summaries, sources kept; session row
   kept with `has_microphone_audio = false`; FTS unchanged; `change_log`
   append. Levels 2 and 3 are **unreachable** from the policy engine — they
   exist only behind the user dialogs P9/P10 (§6.1). The type system backs
   this: `PolicyAction` offers no level-2/3 variant, and
   `delete_meeting_tree` is callable only from the user-action path.
2. **Content-first ordering per deletion** (AS-§9.2 rule 1, adopted): unlink
   files → fsync directory → commit row/state changes in one transaction →
   append `change_log`. Crash windows resolve exactly as AS-§9.2 describes
   (orphan files cleaned by AS-§7; `deleted` rows with missing files reported
   as `already_missing`).
3. **Guard interaction** (AS-§9.2 rule 3, adopted): sweep-triggered level 1 is
   guard-checked (Gate 2, §7.2); user-initiated levels bypass the guard but
   display warnings (P8–P10).
4. **Pass-level order.** Within one sweep pass, actions execute in the stage-B
   emission order (age deletions, quota deletions, stuck-warning, prune).
   Each level-1 deletion is independent; a failure mid-pass aborts the
   remaining deletions of the pass (fail-stop, retry on next trigger), never
   skips ahead to R3.
5. **Reporting.** The pass aggregates `DeleteReport`s (AS-§5.1) and renders
   N1/N2 from real numbers (§6.2 honesty constraint); every applied action
   and every guard-block writes a `change_log` row (AS-§8.2 rule 5) *before*
   R3 could ever prune it — R3's 180-day window guarantees freshly written
   audit rows survive (§9).
6. **Interaction with the retention question (P6).** T2 fires after the
   completing transaction; the question is presented by the UI from the same
   event. Until answered, the stored policy is `Unanswered` and R1/R2 are
   inert for that session — plan §12 line 1450 ("ohne beantwortete Auswahl
   wird nichts automatisch gelöscht") holds by construction.

---

## 9. change_log pruning semantics

Plan §9: small, transactional rows with entity, id, revision, timestamp; no
secrets, no bulk content copies; v1 purpose is incremental project analysis
and diagnostics.

1. **What is pruned:** `change_log` rows with timestamp <
   `now − CHANGE_LOG_RETENTION_DAYS` (180 days), deleted in ascending
   `(timestamp, revision)` order, at most `CHANGE_LOG_PRUNE_MAX_ROWS_PER_PASS`
   rows per pass, inside one transaction.
2. **Marker first:** before deleting, the pass appends one `change_log` row
   describing the prune (entity `change_log`, action `pruned`, row count,
   cutoff timestamp). Audit trail stays unbroken: any observer sees "rows up
   to X were removed here", never a silent hole.
3. **What is never pruned in v1:** nothing else. Tombstones of domain
   entities (RBC-§5.4), `assets` rows, chunk `deleted` rows, session rows —
   all untouched by R3. Row-count growth elsewhere is bounded by usage, not
   by this engine (NG-7 rejects speculative generic GC).
4. **Determinism:** the cutoff and cap are computed inside `evaluate`
   (stage B), so the prune is as testable as every other action; the executor
   adds no policy of its own.
5. **Failure handling:** if the prune transaction fails, the pass logs the
   failure and continues nothing further (it is the last action anyway, §8
   rule 4); the next trigger retries. A failed prune never blocks or undoes
   deletions executed earlier in the pass.

---

## 10. Verification hooks (maps to plan §14 automated tests)

- **Purity/determinism (G2):** golden-vector tests over constructed
  `PolicyInput`s (empty library; single overdue meeting; mixed policies;
  quota-crossing library; fully exempt library); property tests for
  determinism and for the §7.3 item-1 claim; a compile-lint assertion that
  `retention_policy.rs` imports neither `std::fs` nor `std::time` nor
  `std::io`.
- **Constant tests (§4.2):** each constant equals its derivation formula in
  code (`100 * DUAL_TRACK_BYTES_PER_HOUR`, …), mirroring the DG-§8 pattern.
- **Invariant tests (G3):** §7.3 items 1–4, including the seam-instrumented
  guard-before-unlink ordering assertion.
- **Level confinement (§8):** the sweeper's public surface exposes no way to
  request level 2/3; a test attempts to construct such a call and fails to
  compile (type-level) or panics (runtime seam).
- **Trigger-order tests:** T1 fires only after the AS-§7 scan completes
  (inject a slow scan; assert no evaluation ran); T2 fires on the completing
  transaction; T4 coalesces with a running pass.
- **Notice tests (G1):** a pass with ≥1 deletion emits exactly one N1/N2
  event carrying real `DeleteReport` numbers; a zero-action pass emits no
  notification; N4 only when `QuotaStuckWarning` was emitted.
- **Prune tests (§9):** cutoff respected; ascending order; cap honored with
  remainder retried next pass; marker row precedes deletion; failed prune
  leaves rows intact.

---

## 11. Complexity budget and non-goals (gate G4)

Rejected for v1, each with the reason it would be SAP-like scope rather than
evidence-backed value:

- **NG-1 — no configurable rule DSL.** No user-editable JSON/TOML policies,
  no rule objects, no expression language. The entire policy space is: four
  stored policy values (plan §12), two compile-time quota constants, one
  180-day prune constant, one exemption flag. A DSL would multiply test
  surface by the grammar size for a feature no plan sentence asks for.
- **NG-2 — no per-project quotas or per-project retention windows.** Only the
  single `projects.protect_meeting_audio` boolean (§4.3). Richer scoping is
  deferred until real complaints show the boolean is insufficient.
- **NG-3 — no forecasting, compression, tiering or cold storage.** The disk
  guard handles space emergencies during capture; quota cleanup is the
  steady-state complement, not a storage-management subsystem.
- **NG-4 — no dedicated background threads or wake locks.** Four triggers
  (§3) on existing control flow; the daily timer rides the app's existing
  timer infrastructure.
- **NG-5 — no ledger/counter tables.** Ledgers are derived queries (§2);
  drift-proof by construction.
- **NG-6 — no automatic deletion of imports/assets/media or of anything on
  ledger L2.** Irreplaceable user data; overage is reported (N4-class),
  deletion stays user-confirm-only.
- **NG-7 — no generic GC for other tables, no sync-oriented log-compaction
  protocol.** Sync is a plan non-goal (§2 line 85); R3 is sized for v1
  diagnostics only.
- **NG-8 — no notifications for non-events.** Blocks and prunes are silent
  (N3/N5) per the disk-guard U7 precedent; notifying "nothing happened"
  trains users to ignore the notices that matter.

Interface additions handed to implementers (all owned by the Phase-3 storage
work, all declared above):

1. `RetentionBlockReason::SessionEndedDiskFull` added to AS-§8.2's enum
   (closes DG-§7 open item 1 together with the `ended_disk_full` column).
2. New module pair `storage/retention_policy.rs` (pure) +
   `storage/retention_sweeper.rs` (executor) beside B11, SQL-free per
   RBC-§7.1 R1.
3. Proposed-new copy keys N1/N2/N4 for the UX catalog owner (catalog §8
   workflow); N3/N5 deliberately have no keys.
4. Identifier registration for `PolicyAction`, `DeletionReason`,
   `StoredRetentionPolicy`, `AUDIO_QUOTA_*`, `CHANGE_LOG_*` in the
   cross-contract identifier registry (registry owners; this contract does
   not edit it).

---

## Appendix A — Gate traceability

- **G1** (every auto-deletion path names its notice key or proposes one):
  §6.1 matrix separates the two automatic classes from everything
  confirm-only; §6.2 maps N1 (R1), N2 (R2), N3 (prune, justified keyless),
  N4 (stuck warning), N5 (block, keyless per precedent), N6 (existing P6),
  each with concrete `meeting.*` key paths and existence status.
- **G2** (pure, deterministic, fixed order): §5.1 sketch with no I/O/clock
  symbols; §5.2 three-stage normative sequence; §5.3 three-point argument +
  test hooks in §10.
- **G3** (invariant): §7.1 exhaustive unlink-path enumeration, §7.2 double
  gate (engine filter + AS-§8.2 guard at the named enforcement point,
  extended by `SessionEndedDiskFull`), §7.3 proof-obligation tests.
- **G4** (no DSL; index updated): §11 NG-1 rejects the DSL with the full
  enumerated policy space; `docs/meeting-mode-documentation-index.md` gains
  this document's row and reading-order slot (same commit).

## Appendix B — Sketch compilability and behavioral verification

The Rust block of §5.1 is extracted and compiled with the shared helper:

```powershell
pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1 -ContractPath docs/meeting-mode-architecture/retention-quota-contract.md
# OK: 1 rust block(s) from 'retention-quota-contract.md' compiled as a library crate (rustc 1.98.0)
```

Beyond compilation, the sketch was verified behaviorally: the verbatim §5.1
block plus seven `#[cfg(test)]` functions was compiled with
`rustc --edition 2021 --test` and executed — **7 passed, 0 failed**
(rustc 1.98.0, std-only). The tests pin, in order: t01 constant derivations;
t02 determinism (same input ⇒ identical output vector); t03 every blocking/
exemption flag independently suppresses deletion (G3 snapshot gate); t04 the
R2 projection counts R1-freed bytes exactly once (no double deletion, no
false quota trigger); t05 oldest-first selection stopping at the soft floor;
t06 `QuotaStuckWarning` when every candidate is blocked; t07 prune cutoff,
row cap, and last-position emission. The test file itself is scratch
verification tooling, not part of this contract's deliverable; the normative
test obligations for implementers remain §7.3 and §10.
