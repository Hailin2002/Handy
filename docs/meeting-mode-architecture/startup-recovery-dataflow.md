# Startup-Recovery Dataflow — Recorder Chunks × STT Jobs × Session State

Status: **spec contract, Phase 3/4/5 not implemented — changes no source code**
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md), Abschnitt „Session-Zustände"
(lines 893–896), Abschnitt 5 (Zielarchitektur, component homes), Fehlerverhalten
„App-Absturz" (lines 959–960)
Builds on:
[recorder-contract.md](recorder-contract.md) §8 ("Open-chunk crash-recovery
protocol"), [stt-provider-contract.md](stt-provider-contract.md) §3.4
("Neustart-Wiederaufnahme") and §3.2/§3.3,
[../meeting-mode-phase2-contracts.md](../meeting-mode-phase2-contracts.md) §7,
[../meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md) E12,
E13, S9, S10, S6, P8–P10

This document defines **one ordered startup sequence** that joins the four
previously separately specified recovery concerns into a single end-to-end
dataflow. It introduces no new persistence format and no new component; every
step is owned by a component whose home is fixed in plan §5.

---

## 1. Scope and precedence

- **In scope:** what happens once, at app startup, to (a) orphan `.part`/`.wav`
  chunk files, (b) interrupted `running` and pending `queued`/`retry_wait` STT
  jobs, (c) persisted sessions stuck in `starting`/`recording`/`paused`/
  `transcribing` after a dead process, and (d) the user-facing recovery banner.
- **Out of scope:** live-recording error behavior (recorder-contract §7),
  provider retry/backoff policy (stt-provider-contract §3.5), summary-job
  recovery (same reconciliation rules apply, owned by `MeetingSummaryService`,
  but its job table mirrors `meeting_stt_jobs` semantics and is not re-specified
  here), retention/deletion flows (only referenced where the banner links into
  them).
- **Precedence** (per documentation index §4): plan beats contracts, contracts
  beat catalogs, nothing beats real code. Where this document repeats a rule
  from recorder-contract §8 or stt-provider-contract §3.4, those sections stay
  normative; this document only fixes the _join order and ownership_.

## 2. Participants and ownership (plan §5 homes)

| Component                   | Planned home (plan §5, lines cited)                                         | Role in this sequence                                                                  |
| --------------------------- | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `MeetingManager`            | `src-tauri/src/managers/meeting.rs` (plan line 227)                         | Orchestrates S0–S7; liveness; session disposition; event emission                      |
| `MeetingRecorder`           | `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` (plan line 235–236) | S1 chunk/file recovery via `recover_open_chunks` (recorder-contract §4, lines 652–658) |
| `MeetingStore`              | `src-tauri/src/managers/meeting_store.rs` (plan line 243)                   | Transaction boundaries; authoritative chunk/job/session rows                           |
| `MeetingTranscriptionQueue` | `src-tauri/src/managers/meeting_transcription.rs` (plan line 250–251)       | S3 job-state reconciliation; S5 enqueue; S6 resumption                                 |
| `MeetingRecoveryBanner`     | `src/components/meetings/MeetingRecoveryBanner.tsx` (plan line 322)         | Renders recovery outcome; task-0021 incorporation                                      |

Process liveness (pid/lock file) is established by `MeetingManager`
(recorder-contract §8, lines 822–825: "out of recorder scope"). No other
component may decide liveness.

## 3. The ordered startup recovery sequence

The sequence runs **unconditionally on every startup**, in this order. Steps
are self-guarding: on a clean shutdown every step below degenerates to a no-op
(no candidates found, no rows match, no event emitted). There is no separate
"clean path" branch — the same code path proves convergence (§6).

### S0 — Liveness scan and candidate enumeration

- **Owner:** `MeetingManager`.
- **Inputs:** pid/lock-file registry; `meeting_sessions` rows with `state ∈
{starting, recording, paused}` (candidate set A); rows with `state =
transcribing` (candidate set B, see S3/S7 — no capture is at risk there).
- **Outputs:** per candidate session: `session_id`, persisted state, whether a
  live process owns it. Sessions with a live owner are skipped entirely
  (single-instance enforcement makes this exactly the current process's own
  fresh sessions).
- **Source contract:** recorder-contract §8 preamble (lines 820–825).
- **Atomicity boundary:** pure read; no state change.
- **Idempotence:** recomputed from disk + DB on every run; result depends only
  on immutable facts (row states, lock file).

### S1 — Open-chunk crash recovery per candidate session

- **Owner:** `MeetingRecorder`, called by `MeetingManager` as
  `recover_open_chunks(session_id, store)` for every set-A session.
- **Inputs:** chunk rows in state `writing` (`ChunkStateStore::open_chunks`);
  files `<seq>-<tag>.wav.part` and `.wav` under `meetings/<id>/audio/`.
- **Outputs:** `RecoveryReport { finalized, damaged, quarantined_files }`
  (recorder-contract §4, lines 556–564); chunk rows now in `{ready, damaged}`;
  orphan `.part` quarantined into `recovery/`; reverse reconciliation inserts
  rows for renamed-but-uncommitted `.wav` files (§8 step 3).
- **Protocol:** recorder-contract §8 steps 1–4 verbatim (DB row list is
  authoritative for intent, filesystem for survival; truncate to whole PCM16
  frame; patch header; fsync; atomic rename; fsync dir; checksum;
  `writing → ready` with `recovered=true`, else `writing → damaged`).
- **Source contract:** recorder-contract §8 (lines 827–851);
  phase2-contracts §7 rules 1–4.
- **Atomicity boundary:** per chunk, the atomic rename `.part → .wav` is the
  single commit point of the file (recorder-contract §3.1 property 1, lines
  209–213); the accompanying row flip is one SQLite transaction. Filesystem and
  DB commits are deliberately **not** one transaction — §8 step 3 exists
  precisely to reconcile the gap.
- **Idempotence:** §8 step 5 (lines 858–859): `ready` chunks are skipped,
  renames of already-renamed files are no-ops, `damaged` rows are stable,
  quarantine of an already-quarantined file is a skip. A crash **inside** S1
  (mid-list) leaves earlier chunks finalized and the rest still recoverable;
  the rerun of S1 reaches the identical end set because each chunk is decided
  from its own durable facts, never from sequence progress.

### S2 — Session disposition flip

- **Owner:** `MeetingManager` writing through `MeetingStore`.
- **Inputs:** set-A sessions from S0 with their `RecoveryReport`s.
- **Outputs:** each set-A session leaves `starting`/`recording`/`paused` and
  lands in `stopped_unprocessed` (user did not choose an abort) with
  `ended_at` filled if null; sessions whose report contains `damaged > 0` or
  `quarantined_files` additionally carry the `recoverable_error` marker
  (plan Session-Zustände, line 888). This implements the hard requirement:
  "A session is never left in `recording` after a crashed process"
  (recorder-contract §8 step 4, lines 852–857; plan lines 893–896).
- **Source contract:** recorder-contract §8 step 4; plan lines 893–896.
- **Atomicity boundary:** shares the single **startup reconciliation
  transaction** with S3 (see S4). The guard is
  `UPDATE … WHERE state IN ('starting','recording','paused') AND id = ?`;
  a session already flipped matches zero rows on rerun.
- **Idempotence:** guarded monotone transition toward a settled state; second
  execution is a no-op by predicate.

### S3 — STT job-state reconciliation

- **Owner:** `MeetingTranscriptionQueue` (invoked by `MeetingManager` during
  startup), reading/writing through `MeetingStore`.
- **Inputs:** `meeting_stt_jobs` rows in any non-terminal state; liveness fact
  from S0 (no worker of the dead process exists).
- **Outputs:** exactly the restart mapping of stt-provider-contract §3.4
  (lines 194–208):
  - `running → queued`, **without** `attempt_count` increment and **without**
    backoff penalty (transition sanctioned in §3.2, line 151: "nur beim
    App-Neustart");
  - `queued` stays `queued`;
  - `retry_wait` keeps its persisted `next_attempt_at`, immediately due when
    in the past;
  - `succeeded`, `failed`, `cancelled` untouched.
- Set-B sessions (stuck `transcribing`) get no session flip but do go through
  this step: their jobs resume and drive the session forward through the
  normal pipeline (plan Session-Zustände diagram, lines 879–891).
- **Source contract:** stt-provider-contract §3.4, §3.2 row `running → queued`.
- **Atomicity boundary:** same startup reconciliation transaction as S2 —
  mandated verbatim by §3.4: "Der Abgleich wird in derselben
  Migration/Startup-Transaktion wie der Session-Zustandsabgleich … ausgeführt"
  (lines 207–208).
- **Idempotence:** every mapping is a guarded predicate over the _current_
  state; after one pass no row is `running`, so a repeated pass matches only
  the stable classes (`queued`, `retry_wait`, terminals) and changes nothing.
  The unchanged `attempt_count` and idempotency key make even an interrupted
  partial application equivalent to a full one.

### S4 — Single commit of the startup reconciliation transaction

- **Owner:** `MeetingStore`.
- **Inputs:** pending S2 + S3 writes.
- **Outputs:** one atomic commit covering session dispositions **and** job
  reconciliations together.
- **Atomicity boundary:** this commit is _the_ join point demanded by plan
  lines 893–896 ("persistierter Session-Zustand und die tatsächlichen
  Jobzustände werden beim Programmstart abgeglichen"): after S4 there is no
  observable moment where the session claims `recording` while its jobs are
  already re-queued, or vice versa. Frontends can never read a torn pair
  because readers use ordinary SQLite snapshot isolation against committed
  state only.
- **Crash behavior:** crash **before** S4 → transaction rolls back → S2/S3
  rerun identically (their inputs are unchanged facts). Crash **after** S4 →
  S2/S3 guards miss everything → rerun commits an empty transaction. Both
  converge to the same end state (formal argument in §6).

### S5 — Enqueue recovered work

- **Owner:** `MeetingTranscriptionQueue`.
- **Inputs:** all `ready` chunks (freshly recovered ones included —
  phase2-contracts §6 rule 7: recovered chunks enter STT consumption
  transparently) that are not yet referenced by a non-cancelled job; the
  session's provider snapshot from `meeting_sessions`.
- **Outputs:** `queued` jobs created with the deterministic idempotency key of
  stt-provider-contract §3.3 rule 1; jobs then flow through the ordinary
  pipeline (no special "recovery mode").
- **Source contract:** stt-provider-contract §3.3 (rules 1, 6), §3.4;
  phase2-contracts §6 rule 7.
- **Atomicity boundary:** each insert is its own transaction protected by
  `UNIQUE(meeting_id, idempotency_key)` (§3.3 rule 6, lines 184–186).
- **Idempotence:** a rerun recomputes the same key from stable inputs
  (meeting_id, provider, source, timeline range, sorted chunk ids,
  settings snapshot, requeue generation) and hits the unique constraint →
  documented no-op ("bereits gebucht", §3.3 rule 5).

### S6 — Ordinary queue resumption

- **Owner:** `MeetingTranscriptionQueue` workers.
- **Behavior:** no dedicated recovery path: due `queued`/`retry_wait` jobs are
  picked up under the normal engine-lease/cloud-quota rules
  (stt-provider-contract §3.1, §4.1). Recording never waits on inference
  (plan §8); at startup there is no recording anyway.
- **Atomicity/idempotence:** inherited from §3.3 — success books segments +
  `succeeded` in one transaction; replays cannot duplicate segments
  (§3.3 rules 2–5).

### S7 — User-facing recovery banner

- **Owner:** emission by `MeetingManager`; rendering by
  `MeetingRecoveryBanner.tsx` (frontend reads via command/event only, never
  SQLite — plan §5 layering, lines 205–217).
- **Trigger condition (exact):** at least one set-A session was flipped in S2,
  or any `RecoveryReport` field was non-empty. Stuck-`transcribing` sessions
  (set B) alone do **not** raise the banner; they surface as ordinary
  processing progress (catalog S6).
- **Payload:** per affected session: title, count of recovered/damaged/quarantined
  chunks, and interruption cause (`crash` | `sleep_wake` | `unknown`; the
  sleep/wake detection hook is defined in §5, mapping E13).
- **Copy and actions:** see §5 (catalog E12 primary, E13 for sleep/wake).
- **Atomicity/idempotence:** event emission is at-least-once; the banner is
  rendered from an idempotent pull query (`meeting_list_recoveries`, §5), so a
  duplicated or lost event cannot double-render or lose the offer. Dismissing
  is a persisted per-session resolution, making dismissal itself crash-safe.

## 4. Case coverage (gate G1)

The sequence must cover every combination of the gate's five phenomena
(orphan `.part`/`.wav` chunks, interrupted running job, queued job,
session stuck in recording/paused/transcribing/starting, clean shutdown).
These group into three independently keyed dimensions: S1 keys on chunk ids +
files, S2/S4 on session rows, S3/S5 on job rows + idempotency keys. **Any**
combination resolves to the composition of its per-dimension cases below; no
interaction produces a case outside these tables. The per-dimension enumeration
is therefore exhaustive over the product.

Dimension A — chunk residue (drives S1):

| Case | Situation                                                | S1 outcome                                                     |
| ---- | -------------------------------------------------------- | -------------------------------------------------------------- |
| A1   | no chunk files, no `writing` rows                        | no-op                                                          |
| A2   | `.wav` present, no `ready` row (crash rename↔DB commit) | row inserted/updated to `ready`, `recovered=true` (§8 step 3)  |
| A3   | parseable `.part` with `writing` row                     | truncated, finalized to `ready`, `recovered=true` (§8 step 2a) |
| A4   | unparseable/headerless/<44-byte `.part` with row         | row → `damaged`, file to `recovery/` (§8 step 2b)              |
| A5   | `writing` row, file missing                              | row → `damaged` (§8 step 2c)                                   |
| A6   | orphan `.part` with no row                               | quarantined to `recovery/` (§8 step 3)                         |

Dimension B — STT job residue (drives S3):

| Case | Situation                                   | S3/S5/S6 outcome                      |
| ---- | ------------------------------------------- | ------------------------------------- |
| B1   | no non-terminal jobs                        | no-op                                 |
| B2   | interrupted `running` job                   | `→ queued`, no attempt penalty (§3.4) |
| B3   | `queued` job                                | stays `queued`, processed normally    |
| B4   | `retry_wait`, `next_attempt_at` past/future | kept; immediately due if past         |

Dimension C — session state (drives S2/S7):

| Case | Situation                                                                            | Outcome                                                                                   |
| ---- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| C1   | clean shutdown (`completed`, `stopped_unprocessed`, `cancelled`, or no open session) | whole sequence no-ops; no banner                                                          |
| C2   | stuck `recording`                                                                    | S2 flip to `stopped_unprocessed`; banner (E12)                                            |
| C3   | stuck `paused`                                                                       | S2 flip; banner (E12); pause markers preserved                                            |
| C4   | stuck `starting`                                                                     | S2 flip; banner (E12); A-cases may be empty (nothing captured yet)                        |
| C5   | stuck `transcribing`                                                                 | no session flip needed; B-cases resume pipeline; no recovery banner (catalog S6 progress) |

Spot-checked representative combinations (not exhaustive, illustrative):

| Combination     | End state after one full pass                                                                                                     |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| C2 + A3 + B2    | recovered mic chunk `ready(recovered)`; job re-queued; session `stopped_unprocessed`; banner offers "Recover and process"         |
| C3 + A4 + B3    | one `damaged` row in `recovery/`; queued job intact; session flipped with `recoverable_error` marker; banner + damaged-count note |
| C4 + A1 + B1    | session flipped; nothing recovered; banner explains safe early end (no audio existed)                                             |
| C5 + A1 + B2/B4 | session stays `transcribing`; jobs resume; transcript completes; normal progress UI                                               |
| C1 + anything   | no-op everywhere (dead process implies C≠C1 by definition; listed for the clean-shutdown gate row)                                |

## 5. User-visible outcomes → copy catalog + Tauri surface (gates G4)

All identifiers below are **placeholders flagged for task-0021 incorporation**
(component `MeetingRecoveryBanner.tsx`, plan line 322; commands/events to be
generated via the existing tauri-specta `bindings.ts` toolchain).

| #   | User-visible outcome                        | Catalog entry                                                                                                  | i18n keys                                                                              | Tauri surface (placeholder)                                                                   |
| --- | ------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| U1  | Crashed meeting found, audio saved          | **E12** (`meeting.recovery.title/body`)                                                                        | `meeting.recovery.*`, actions `meeting.actions.recover`, `meeting.actions.decideLater` | event `meeting://recovery-report`; command `meeting_list_recoveries`                          |
| U2  | Resolve: recover and process                | E12 button → jobs enqueued per S5                                                                              | `meeting.actions.recover`                                                              | command `meeting_resolve_recovery(session_id, "recover")`                                     |
| U3  | Resolve: decide later                       | E12 secondary                                                                                                  | `meeting.actions.decideLater`                                                          | command `meeting_resolve_recovery(…, "later")`; persists resolution, re-offers on next launch |
| U4  | Resolve: delete recording                   | E12 tertiary → P8–P10 staged deletion                                                                          | catalog §6 P8/P9/P10                                                                   | command `meeting_resolve_recovery(…, "delete")` → existing planned delete flow                |
| U5  | Interruption cause was sleep/wake           | **E13** (`meeting.errors.slept.title/body`) rendered inside/beside the banner; overlay + notification surfaces | `meeting.errors.slept.*`                                                               | same event payload field `cause = "sleep_wake"`                                               |
| U6  | Damaged chunks exist (A4/A5)                | **S9** `recoverable_error` overlay wording on the banner/detail view                                           | catalog S9 pattern                                                                     | included in `meeting://recovery-report` payload (`damaged_count`)                             |
| U7  | Stuck `transcribing` resumed (C5)           | **S6** `transcribing` status text; no banner                                                                   | catalog S6                                                                             | existing planned processing-progress events (§6.12 checklist)                                 |
| U8  | Resumed job later fails/transiently retries | existing catalog **E9/E10/E11** paths                                                                          | `meeting.warnings.backlog.*` etc.                                                      | existing planned job-status events (stt contract §7 event rules)                              |
| U9  | Clean shutdown (C1)                         | none — silence is the spec                                                                                     | —                                                                                      | no event emitted                                                                              |

**Catalog-ID discrepancy disclosure:** the mission brief named "copy catalog
E13" for the recovery banner. In the catalog as authored, the crash-recovery
banner is **E12** (catalog lines 351–363); **E13** is Sleep/Wake (lines
365–375). This document therefore uses E12 for the banner and E13 for the
sleep/wake cause variant, and flags the brief's off-by-one to task-0021 rather
than renumbering the shared catalog.

## 6. Convergence proof (gate G3)

Claim: after a process crash at **any** point of S0–S7, rerunning the full
sequence on restart yields the identical end state, where the end state is the
predicate **P**:

> every chunk row ∈ `{ready, damaged, deleted}` and no `.part` file outside
> `recovery/`; no session row remains in `{starting, recording, paused}`
> without a live owner; no job row is `running` without a live worker; every
> `ready` chunk is covered by at least one non-cancelled job; the banner
> resolution is persisted exactly once per affected session.

Per-step arguments (each step re-derived, never resumed):

1. **S0:** pure function of (lock file ∪ session rows). Deterministic per run.
2. **S1:** each chunk is classified solely from its own durable facts (row
   state + file bytes), so partial progress is invisible to the rerun
   (recorder-contract §8 step 5). Rename-vs-DB-commit races are absorbed by
   reverse reconciliation (§8 step 3), which is why A2 and A6 exist as
   first-class cases instead of error paths.
3. **S2/S3:** guarded, monotone transitions (`WHERE state IN …`). Applied
   twice → second application matches zero rows. Partially applied inside a
   rolled-back transaction → nothing applied.
4. **S4:** the transaction is the only join point; SQLite gives exactly-once
   commit semantics. Before-commit crash ⇒ repeat S2/S3; after-commit crash ⇒
   S2/S3 become no-ops. Identical P either way.
5. **S5:** deterministic idempotency key plus the job-level unique constraint
   on `(meeting_id, idempotency_key)` ⇒ re-enumeration inserts each logical
   job at most once (§3.3 rules 1, 5, 6).
6. **S6:** segment booking is transactional with the state flip (§3.3 rule 2);
   replay of a provider response is a checked no-op (rule 5).
7. **S7:** at-least-once event + idempotent pull query + persisted
   resolution ⇒ duplicates collapse, losses self-heal on next render.

Composition: since every step individually maps any intermediate state to the
same fixpoint and no step consumes another step's volatile progress (only
committed rows, files, and the lock file), the sequence is a contraction over
the finite state space toward P. One full pass after any mid-sequence crash
reaches P; a further pass leaves P invariant (all guards miss, unique
constraints hold, reports are empty). QED-style argument to be backed by the
fault-injection tests of §7.

## 7. Verification hooks

Extends recorder-contract §9 and stt-provider-contract §3 test obligations:

- Double-run idempotence across the **whole** sequence: run startup twice per
  scenario from §4; assert byte-identical DB diff on the second run (empty).
- Fault-injection seams: kill between S1 per-chunk commits, before S4 commit,
  after S4 before S5, after S5 before S7; assert convergence to P in each case
  (mirrors recorder-contract §9 "kill writer between header-patch/rename/
  db-commit").
- Cross-module assertion: after S4 there exists no readable state pair
  (session `recording`, jobs already `queued`) or (session
  `stopped_unprocessed`, jobs still `running`) — the join invariant of §6.
- Banner logic: U-table rows unit-tested against catalog IDs (E12/E13/S9/S6)
  and the `cause` payload classification.

## 8. Non-goals (complexity budget)

Rejected as low-value or scope-bloat, consistent with the mission complexity
budget: no second recovery daemon/thread (startup-only pass); no network or
multi-machine recovery; no repair attempt beyond WAV-header truncation (damaged
stays damaged, forensics only); no checksum regeneration for damaged files; no
automatic deletion of damaged/quarantined material (retention owns deletion);
no generic "replay journal" abstraction alongside the two existing contracts;
no banner for clean shutdowns or for stuck-`transcribing` sessions (ordinary
progress UI suffices).

## Appendix A — Gate traceability (this document)

- **G1** every combination covered: §4 (exhaustive per-dimension tables A/B/C,
  composition argument, spot-check combinations incl. clean shutdown).
- **G2** one owner per step, consistent with plan §5 homes, source-contract
  citations inline: §2 (ownership table), §3 S0–S7 (each step cites
  recorder-contract §8, stt-provider-contract §3.2/§3.3/§3.4, or
  phase2-contracts §6/§7).
- **G3** convergence under repeated crashes: §6 (end-state predicate P +
  per-step idempotence + composition argument), anchored in recorder-contract
  §8 step 5 and stt-provider-contract §3.3.
- **G4** outcomes → catalog IDs + Tauri placeholders, flagged task-0021: §5
  (U1–U9 table, including the E12/E13 discrepancy disclosure).
- **G5** reviewable contract, prettier-clean: verified via
  `bunx prettier --check docs/meeting-mode-architecture/startup-recovery-dataflow.md`
  after authoring (result recorded in mission evidence).
