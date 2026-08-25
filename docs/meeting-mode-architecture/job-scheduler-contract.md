# Job-Scheduler- und Cancellation-Vertrag (Meeting Mode)

Status: **spec contract — no implementation yet** (Phase 5/Phase 7 prerequisite;
changes no source code).
Source of truth: `docs/meeting-mode-plan.md` §5 (`MeetingTranscriptionQueue`,
plan lines 245–252: „Persistente, abbrechbare Queue mit genau einem lokalen
Engine-Lease oder begrenzten Cloud-Requests. Audioaufnahme darf niemals auf STT
warten."), §8 („Lokale Verarbeitung" plan lines 1059–1067,
„Ressourcenlogik zwischen Meeting und Diktat" plan lines 1089–1099,
Azure-Job-Eigenschaften plan lines 1121–1128), §9 (`meeting_stt_jobs`
plan lines 1190–1199, `processing_jobs` plan lines 1313–1318),
„Fehlerverhalten" (plan lines 955–961: Backlog wächst nur auf Disk, Jobs
wechseln in `retry_wait`), „Stoppen" (plan lines 936–941), §13 Phase 5.
Planned implementation home: `src-tauri/src/managers/meeting_transcription.rs`
(plan line 251). This document defines the cross-subsystem scheduling seam
only; it changes no source code.

Sibling contracts consumed by this document (read-only):

- [stt-provider-contract.md](stt-provider-contract.md) — `MeetingSttProvider`
  trait (§1), job state machine (§3.1/3.2), idempotency (§3.3), restart
  resumption (§3.4), retry parameters (§3.5), error classification (§6).
  Below abbreviated **STT-§n**.
- [summary-evidence-contract.md](summary-evidence-contract.md) — pipeline
  stages (§1), idempotency keys (§2), `processing_jobs` envelope (§1.4).
  Below abbreviated **SEC-§n**.
- [coexistence-contract.md](coexistence-contract.md) — engine-lease extension
  S1 (§3), pause-at-segment-boundary and deadlines (§3.4–§3.7), capture-
  never-waits-on-inference proof structure (§3.6). Below abbreviated
  **COEX-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) — store
  units of work U4/U8 (§4) and K10 (§4), choke-point rules (§7). Below
  abbreviated **RB-§n**.
- [session-state-contract.md](session-state-contract.md) — single-owner rule
  for session state (§1). Below abbreviated **SSC-§n**.

Conflict rule (identical to SSC head and the documentation index): the plan
wins over this contract, this contract wins over catalogs, and nothing wins
over actual code.

---

## 0. Scope, the seam, and the single-persistence-point rule

### 0.1 The seam

One worker-pool/lease seam sits **above** `MeetingSttProvider` (STT-§1) and
`MeetingSummaryService` (SEC-§1.1). Its implementation name is the planned
`MeetingTranscriptionQueue` module (plan line 251), referred to below as the
**scheduler**. The scheduler owns exactly three things:

1. **Admission control** — deciding *when* a persisted job may execute, by
   acquiring a concurrency-class slot (§1) for it;
2. **Worker pool** — the bounded set of worker threads that pull claimed jobs
   to execution and call the provider/service exactly once per claim;
3. **Timer wheel** — `retry_wait`/lease-deadline bookkeeping (§5, §3.3).

The scheduler owns **none** of: provider communication details (STT-§4),
pipeline stage logic (SEC-§1), evidence validation (SEC-§3), session state
(SSC-§1), or persistence schemas (RB-§2/§3).

### 0.2 Job-row families and the single persistence point

Plan §9 defines two persistent job tables. This contract fixes their division
of labor:

| Job family | Table | Examples | Schema/state-machine authority |
| --- | --- | --- | --- |
| STT jobs | `meeting_stt_jobs` (plan lines 1190–1199) | one VAD segment / Azure package / Custom segment | STT-§2/§3 |
| Generic AI-pipeline & ingestion jobs | `processing_jobs` (plan lines 1313–1318) | meeting-summary runs, WhatsApp import, media analysis, source summaries, project snapshots, later connector syncs | SEC-§1.4 (state/retry semantics of `meeting_stt_jobs` apply **verbatim**) |

**Single-persistence-point invariant P1:** *All durable scheduler state lives
exactly in these job rows. There is no scheduler-private queue file, no broker,
no OS-level queue, no secondary journal, and no durable in-memory registry.*
Every enqueue, state flip, `next_attempt_at`, attempt counter, cancel flag and
error record is written inside the store units of work (RB-§4 U4/U8 for
`meeting_stt_jobs`, K10 for `processing_jobs`, tier-B operational writes per
RB line 228 — no `change_log` rows). Everything else the scheduler holds is
volatile and reconstructible from rows (§6.2).

*Reconciliation note:* the task brief phrases P1 as "`processing_jobs` rows are
the single persistence point." Read against the plan, STT jobs have their own
specialized table (plan lines 1190–1199), while plan line 1317 explicitly ties
`processing_jobs` to the *same* state and retry semantics. P1 above is the
faithful reading: **one** persistence discipline, **two** row families sharing
one state machine, one claim protocol and one idempotency regime — never a
second mechanism beside the rows. Within the non-STT subsystems,
`processing_jobs` is indeed the single persistence point (SEC-§1.4).

### 0.3 Explicitly out of scope (non-goals)

See §8 (binding complexity budget). Nothing in this contract introduces a
distributed scheduler, cron/recurring jobs, priority preemption of running
jobs, dynamic concurrency auto-scaling, or user-facing scheduler tuning.

Normative language: **must** = gate; **should** = default with documented
exception path; **may** = permitted option.

---

## 1. Concurrency classes and ceilings (gate G1)

### 1.1 Class table

There are exactly **three** schedulable concurrency classes. Adding a fourth
requires revisiting this contract first (mirror of COEX §9 rule 2).

| Class | Resource | Ceiling | Slot owner | Covers |
| --- | --- | --- | --- | --- |
| **L1** — local engine lease | the single shared STT engine (`TranscriptionManager`) | **exactly 1**, process-wide, shared with interactive dictation | `TranscriptionManager` via the COEX-§3 lease extension | `LocalMeetingProvider` jobs (STT-§4.1) |
| **C1** — cloud STT slots | outbound Fast-Transcription / Custom-Multipart requests | **2** concurrent (`MAX_CONCURRENT_CLOUD_STT_JOBS`, testable internal constant, plan lines 1116–1117: Parallelität ist konfigurierbarer interner Wert) | scheduler | `AzureMeetingClient` + `CustomMultipartProvider` jobs (STT-§4.2/4.3) |
| **S1** — summary/intelligence slots | outbound/local LLM requests of `MeetingSummaryService` and `ProjectIntelligenceService` | **2** concurrent (`MAX_CONCURRENT_LLM_JOBS`, testable internal constant) | scheduler | SEC M1/M2 map+reduce model calls, P1/P2 batches |

Class rules:

1. **L1 is not owned by the scheduler.** The scheduler requests the lease for a
   claimed local job through the COEX-§3.3 acquire protocol (Background
   class) and waits like any Background waiter. The ceiling "exactly one"
   (plan line 247: „genau einem lokalen Engine-Lease") and the dictation
   priority relation are inherited from COEX-§3.2, never restated here.
2. **C1 and S1 are independent pools.** A cloud STT job never consumes an S1
   slot and vice versa. Plan §10 (lines 1340–1347) makes summary independent
   of STT choice; the slot pools encode that independence mechanically.
3. **Stage gating beats slots.** SEC-§1.1 rule 3 (M2 starts only when all M1
   stages succeeded) and the sequential P-stage gating are enforced by the
   *services*, not by slot counts; the scheduler never runs a stage whose
   predecessor set is incomplete. Slots only bound *how many* ready jobs run
   at once.
4. **Within a run, ready-before-pending.** Among the jobs of one
   `processing_jobs` run, reduce/publish stages that became ready are dequeued
   before still-pending map/excerpt windows of the same run, so a run always
   drains toward publication instead of hoarding slots for fresh windows.

### 1.2 Invariant CAP1: capture never waits on scheduling (G1)

Exact wording (plan lines 248–249): *"Audioaufnahme darf niemals auf STT
warten"*; plan line 1063: *"Aufnahme-Threads warten niemals auf Inferenz"*;
task gate: capture must never block **on scheduling** either.

**Proof structure** (same shape as COEX-§3.6; each row names the objects the
path touches):

| Path | Objects touched | Blocking primitives on path |
| --- | --- | --- |
| Device callback → recorder writer → rolling WAV chunks | recorder-internal channels, disk writer (recorder-contract §1) | none on the callback (bounded channel send) |
| Chunk finalization → VAD segmentation worker | chunk rows (RC/DC contracts), CPU-bound VAD off the callback | none shared with the scheduler |
| Segment/package enqueue (`enqueue_stt_job`) | ONE short SQLite INSERT transaction (job row, family per §0.2) + condvar notify | SQLite write lock only — bounded by a single small transaction, and already on the persistence path those contracts mandate; **never** a lease acquire, never a slot wait, never a lock also held across provider I/O |
| Summary/import enqueue (`enqueue_processing_job`) | ONE short SQLite INSERT (+`UNIQUE(idempotency_key)` guard) + notify | same as above |

Three structural facts make the invariant hold:

1. **Queues are the rows themselves.** A job that cannot be admitted now is
   simply a row in `queued`/`retry_wait`. There is no bounded in-memory queue
   whose fullness could back-pressure the producer — the backlog grows on disk
   only (plan line 958: „Backlog wächst nur auf Disk, nicht im RAM"; plan
   lines 56–59 allow processing to lag behind the recording).
2. **Admission happens on scheduler threads only.** Lease/slot acquisition
   (L1/C1/S1) occurs exclusively inside scheduler workers between claims. No
   capture-path function ever names a slot, a lease, or a worker handle.
3. **No shared lock spans enqueue and execute.** Enqueue transactions touch
   only the job tables; execution-time locks (engine mutex, loading Condvar,
   HTTP clients) live entirely on worker threads (COEX-§3.6 table). A slow or
   stalled worker can therefore delay transcription of already-captured audio,
   but can never delay chunk writing or further enqueues.

Consequences (test obligations, Phase 5 acceptance):

- A unit test drives the scheduler against an engine whose inference blocks
  indefinitely and asserts chunk finalization + enqueues keep progressing
  (extends the COEX-§3.6 mock-engine test to the scheduler seam).
- A unit test fills C1/S1 completely with stalled fake providers and asserts
  `enqueue_*` latency stays bounded by the SQLite write, i.e. no caller blocks
  on slot availability.

---

## 2. Priority and fairness rules

### 2.1 Global priority relation

```text
Interactive dictation (engine lease, COEX class Interactive)
  > meeting local STT jobs (L1, COEX class Background)
  ─────────────────────────────────────────────────────────
  cloud STT jobs (C1)   ⊥   summary/intelligence jobs (S1)
  (disjoint pools — no priority relation needed or defined)
```

- The only contended resource is L1, and its arbitration is COEX-§3.2–§3.5
  verbatim: dictation wins ties, background pauses at segment boundaries,
  auto-resume afterwards, `MEETING_LEASE_PAUSE_DEADLINE` bounds the wait. The
  scheduler **must not** add preemption, aging, or priority escalation on top;
  starvation surfaces as the honest backlog counter + `meeting_backlog_starved`
  warning event (COEX-§3.3 rule 5), never as silent priority changes.
- Cloud and summary work proceeds independently while local work pauses
  (plan lines 610–611; COEX-§3.1 note).

### 2.2 Fairness inside the STT queue

1. **Primary order:** ascending `timeline_start_ms` (plan line 1062: „nach
   Timeline … serialisiert"), tie-broken by `id` for determinism.
2. **Source fairness (mic ↔ system):** when the head candidates of both
   sources lie within `SOURCE_FAIRNESS_WINDOW = 2000 ms` of each other, the
   scheduler alternates sources; outside the window strict timeline order
   applies. This realizes plan line 1062 („Timeline und Fairness") without
   allowing one source to starve the other.
3. **Starvation guard:** a due job that lost admission for
   `BACKLOG_STARVE_WARN_ROUNDS = 8` consecutive due-poll rounds raises the
   same `meeting_backlog_starved` diagnostic event used by COEX-§3.3 rule 5.
   It changes no ordering by itself.
4. **Per-meeting rotation:** with backlogs from several finished meetings
   present (single-active-recording invariant, COEX-§5.1, still permits old
   backlogs), the due-job scan rotates across `meeting_id`s and claims at most
   `PER_MEETING_CLAIM_BURST = 8` consecutive jobs from one meeting before
   serving the next, so a huge old backlog cannot indefinitely delay a newly
   finished meeting. Within a meeting, rule 1 applies unchanged.

### 2.3 Fairness inside `processing_jobs`

- Ready jobs are served oldest-first by `(next_attempt_at, created_at, id)`.
- No size-based or cost-based scheduling (no shortest-job-first): predictable
  progress beats average-latency optimization at v1 scale.
- Rule §1.1-4 (ready-before-pending within a run) is the only intra-run
  override.

### 2.4 Admission protocol

Claiming is one atomic store operation per class (RB-§4 U4
`claim_due_stt_job`, RB-§4 K10 `claim_processing_job`):

```sql
-- conceptual shape; the store owns the real statement (RB-§7 choke point)
UPDATE <job table>
   SET state='running', attempt_count=attempt_count+1, worker=:w,
       next_attempt_at=NULL, lease_expires_at=:now+lease_duration
 WHERE id = :oldest_due_and_admissible AND state IN ('queued','retry_wait');
```

A claim that cannot obtain its class slot simply does not happen — the row
stays due. There is no spinning waiter on slots: workers wake on the notify
from §1.2 enqueues, on timer-wheel expiry (retry due dates, lease deadlines),
and otherwise sleep.

Schema note: the `worker`/`lease_expires_at` marks shown here are operational
bookkeeping columns **proposed** for the two job tables; schema authority
stays with RB-§0/§3, and the columns are kept minimal (no payload duplication,
no secrets). If RB elects a different mechanism (e.g. lease marks in a side
column set or claim tokens), P1 is unaffected as long as the marks live in
the same SQLite file and are cleared by §3.4 startup reconciliation.

---

## 3. Lease semantics and timeouts

### 3.1 L1 — engine lease

Fully delegated to COEX-§3. The scheduler's obligations are: request with
Background class, honor `BgPauseRequested` only at safe segment boundaries
(persisted result, COEX-§3.4), auto-resume per COEX-§3.5, and treat lease
unavailability as "remain queued": for class L1 the store claim (§2.4) is only
executed once the engine lease can actually be granted, so a local job waiting
for the engine never carries the `running` mark. No
scheduler timer ever aborts a local job for *waiting*.

### 3.2 C1/S1 — slot leases

- A slot lease is granted **by the claim itself** (§2.4: `lease_expires_at`)
  and lasts `attempt_timeout(job) + LEASE_GRACE = 60 s`, where
  `attempt_timeout` is the per-provider value from STT-§3.5 (for S1 jobs: the
  LLM request budget, initial constant 300 s).
- No heartbeat protocol in v1: the attempt timeout bounds execution, so a
  deadline sweep (§3.3) suffices. Introducing heartbeats requires a contract
  revision (complexity budget, §8).
- Expiry handling: the sweep transitions an expired `running` job to
  `retry_wait` with `last_error` = classified **transitory** timeout (STT-§6.3
  conservatism applies if the cause is unknown), incrementing `attempt_count`
  exactly as a normal timeout would (this is a stall, not a crash — the
  no-penalty demotion of STT-§3.4 is reserved for process restarts). The
  orphaned worker's provider call is aborted best-effort (dropped future ⇒
  connection closed).
- **Late results are no-ops.** A worker delivering a result for a job whose
  lease was reclaimed, or which was cancelled meanwhile, finds the row no
  longer `running`-under-its-claim and performs the STT-§3.3 rule 5 /
  SEC-§2.2 duplicate-guard check: result discarded, state untouched.

### 3.3 Deadline sweep

One scheduler timer pass per second (constant `LEASE_SWEEP_INTERVAL`) handles,
in one read-only query + minimal writes: due `retry_wait` jobs (make them
claimable — they already are by virtue of `next_attempt_at <= now`, so the
sweep only wakes workers), expired slot leases (§3.2), and overdue cancel
acknowledgments (§4.2 bounds). All writes go through the store units of work;
the sweep holds no locks across iterations.

### 3.4 Startup recovery

Extended verbatim from STT-§3.4 to **both** job tables, executed in the same
startup transaction as the session reconciliation (STT-§3.4 last bullet,
SSC-§6):

- `queued` stays `queued`; `retry_wait` keeps `next_attempt_at`;
  `succeeded`/`failed`/`cancelled` untouched.
- `running` rows are interrupted claims (no living worker exists at startup):
  demoted to `queued` **without** `attempt_count` increment and without
  backoff penalty; idempotency keys unchanged.
- Any residual `lease_expires_at`/worker marks are cleared during the same
  reconciliation — after startup completes, no row references a dead worker.

---

## 4. Cancellation propagation (gate G2)

### 4.1 Entry points

Exactly three, all converging on one scheduler API
(`cancel(scope, reason) -> CancelOutcome`), reached through `MeetingManager` /
Tauri commands (the frontend never touches job rows directly — SSC-§1.1
pattern):

| Scope | Meaning |
| --- | --- |
| `Meeting(meeting_id)` | cancel all non-terminal jobs of one meeting (plan line 940: „Der Nutzer kann die Verarbeitung abbrechen…"); audio is **not** deleted |
| `Job(job_id)` | cancel one job („Fehlerhafte Abschnitte erneut versuchen"-Flows, per-segment abort) |
| `Run(run_id)` | cancel one `processing_jobs` run incl. its unfinished `ai_pipeline_stages` (SEC-§1.3); published artifacts of already-succeeded stages remain |

### 4.2 Semantics per job state

Cancel intent is written transactionally (state flip guarded by a
`WHERE state = :expected` predicate), so concurrent cancel requests produce
**exactly one** flip; losers observe the already-cancelled row.

| Current state | Effect of cancel |
| --- | --- |
| `queued` | flip to `cancelled` immediately; nothing to release; `next_attempt_at` cleared |
| `retry_wait` | flip to `cancelled` immediately; backoff timer discarded |
| **leased** (claimed in §2.4 sense, worker not yet past its first checkpoint) | cancel intent recorded; worker observes the token at its start checkpoint and exits without invoking provider/service ⇒ flip to `cancelled`. If the worker misses the checkpoint (race), the running rules below apply — the outcome is `cancelled` either way |
| **running** (provider/service call in flight) | cooperative cancellation: the in-flight call is aborted best-effort (drop of the HTTP future / engine call finishes at the next safe boundary for local inference, mirroring COEX-§3.4's never-interrupt rule), partial results are **discarded** — inserts are atomic per STT-§3.3 rule 2 / SEC-§1.1 rule 4, so no partial state can exist — then flip to `cancelled` |
| `succeeded` / `failed` / `cancelled` | **no-op**: returns `Ok(current_state)`; success is never undone by a late cancel |

Bound on take-effect time:

- local running: ≤ one segment duration (safe-boundary check, COEX-§3.4
  argument);
- leased: ≤ worker start checkpoint (sub-second);
- cloud running: the HTTP future is dropped promptly after token observation;
  overall bound `CANCEL_ACK_DEADLINE = 60 s` (same magnitude as
  COEX-§3.7's pause deadline); a breach is logged as a defect telemetry event,
  it never force-kills threads.

### 4.3 Idempotent cancel results

`CancelOutcome` reports the resulting state, not a delta:

1. Cancelling an already-cancelled, `succeeded`, or `failed` job is a **success
   no-op** — identical request, identical result, no error, no state change.
2. Two racing cancels converge on one flip (guarded transition, §4.2); both
   callers receive the same outcome.
3. Re-delivered cancel events (UI retries, event replays) are absorbed by rule 1.
4. A cancel that arrives after success leaves artifacts intact and reports
   `succeeded`; it does not roll back segments/summaries (consistent with
   STT-§3.2: `succeeded` is terminal).

### 4.4 Downstream effects

- **Session state:** job cancellation never writes `meeting_sessions.state`;
  `MeetingManager` derives the visible session conclusion from job rows
  (SSC-§1.1 table row 2). The identifier-registry finding F-02
  (session-`cancelled` vs job-`cancelled` collision) stands; events must
  qualify which domain a `cancelled` refers to.
- **Requeue after cancel:** `cancelled → queued` remains an explicit user
  action (STT-§3.2) with a fresh `requeue_generation`/rerun generation
  (STT-§3.3 rule 1, SEC-§2.1).
- **Retention:** a `cancelled` job counts as non-succeeded and therefore
  blocks automatic audio deletion exactly like a failed job (plan lines
  1451–1452).
- **Events:** cancellations publish the usual post-commit, secret-free status
  events (STT-§7 invariant).

---

## 5. Retry-backoff ownership

Ownership is split along one line: **classify at the edges, own the clock in
the middle.**

1. **Classification — providers/services.** STT providers classify errors as
   permanent/transitory per STT-§6.1/6.2/6.3. Summary/intelligence stages use
   the same criteria lists applied to their LLM endpoints (HTTP 401/403 =
   permanent; 408/429/5xx/network/timeout = transitory; unclassifiable =
   transitory per STT-§6.3). SEC deliberately defines no divergent scheme;
   inventing one here would violate gate-consistency (§7).
2. **Decisions and clocks — scheduler only.** The scheduler translates a
   classification into the state machine outcomes of STT-§3.2
   (`failed` vs `retry_wait`), computes `next_attempt_at` from the parameter
   tables of STT-§3.5 (which own the values verbatim for both job families per
   SEC-§1.4), persists it (survives restart), applies the deterministic ±20 %
   job-ID jitter, respects capped `Retry-After` values, and runs the retry
   timer wheel.
3. **Single-attempt discipline.** Providers and services execute **one**
   attempt per claim and return; retry loops, sleep/retry wrappers, and
   ad-hoc backoff anywhere else (provider impls, services, commands) are
   contract violations. This is what makes attempt counters and idempotency
   keys trustworthy.
4. **Permanent errors** go straight to `failed` with an actionable
   `last_error`, no automatic retry, and — until explicitly re-queued — they
   block audio cleanup like any non-succeeded job (plan lines 1451–1452).

---

## 6. Persistence rules (P1 in practice)

### 6.1 What is durable

Everything another component could legitimately observe about scheduling:

| Fact | Where | Writer |
| --- | --- | --- |
| existence, payload refs, idempotency key | job row at enqueue (UNIQUE guards per STT-§3.3 rule 6 / SEC-§2.1) | store units of work |
| state, attempt_count, next_attempt_at, last_error, worker/lease marks | job row at every transition (STT-§3 header: „transaktional zusammen mit seinen Nebenwirkungen") | store units of work U4/K10 |
| stage payloads | `ai_pipeline_stages` (SEC-§1.3) keyed by `job_id` | services inside their publish transactions |

Operational rows are RB tier B: no `change_log` entries for job-state churn
(RB line 228).

### 6.2 What is volatile (and must stay volatile)

Worker thread handles, condvar/waker registration, in-flight claim guards,
cancellation tokens, the timer wheel's in-memory index. Each is reconstructible
from rows after any crash; losing them loses nothing but a wakeup. A code
review finding any of these being persisted outside the job rows is a P1
violation.

### 6.3 Events are derived, never authoritative

Tauri status/progress events are published **after** the committing
transaction, carry no secrets (STT-§7), and are advisory: every consumer
(frontend views per plan §6.11/§6.12) must tolerate missing events and re-read
rows. An event is never the only copy of a scheduling fact.

---

## 7. Consistency with sibling contracts (gate G3 support)

| Concern | STT contract | Summary/Evidence contract | This contract |
| --- | --- | --- | --- |
| State machine | STT-§3.1/3.2 (authoritative) | SEC-§1.4 applies STT-§3 verbatim | consumed unchanged; scheduler adds no states, no transitions |
| Idempotency keys | STT-§3.3 (formation + UNIQUE guards) | SEC-§2 (run/stage derivations, `UNIQUE(idempotency_key)` on `processing_jobs`) | enforced at enqueue via the store; late-result absorption §3.2 |
| Retry/backoff values | STT-§3.5 (parameter tables) | inherited via SEC-§1.4 | scheduler computes/persists timers only |
| Error classification | STT-§6 (criteria lists) | same lists for LLM endpoints | scheduler consumes categories, never classifies itself |
| Restart resumption | STT-§3.4 | implied verbatim | §3.4 extends to both tables in one startup transaction |
| Engine lease | STT-§4.1 single-worker rule | n/a | delegated to COEX-§3; L1 ceiling 1 |
| Persistence seams | STT-§2 (queue persists job fields) | SEC-§1.3/§1.4 | RB units of work U4/U8/K10 are the only writers (RB-§7 choke points) |

---

## 8. Complexity budget and non-goals (binding)

To keep the scheduling layer simple, these rejections are part of the contract
(mission complexity-budget rule: reject SAP-like scope; every addition must be
evidence-backed and preserve simplicity):

- **No distributed/broker-backed queue, no external message bus, no second
  persistence mechanism.** One process, one SQLite file, two row families
  (P1).
- **No fourth concurrency class without a contract revision.** Three classes,
  one fixed, enumerated set of testable constants (`MAX_CONCURRENT_CLOUD_STT_JOBS`,
  `MAX_CONCURRENT_LLM_JOBS`, `LEASE_GRACE`, `LEASE_SWEEP_INTERVAL`,
  `SOURCE_FAIRNESS_WINDOW`, `BACKLOG_STARVE_WARN_ROUNDS`,
  `PER_MEETING_CLAIM_BURST`, `CANCEL_ACK_DEADLINE`) — each documented where it
  is defined, none user-facing.
- **No heartbeat/keepalive protocols** for slot leases in v1 (deadline sweep
  suffices, §3.2).
- **No preemption of running jobs** in any direction (dictation↔meeting
  handled by COEX boundaries; cloud calls end cooperatively, §4.2).
- **No DAG/workflow engine.** Stage gating stays inside the services (SEC-§1);
  the scheduler sees only flat due-job sets.
- **No cron/recurring jobs, no delayed-except-backoff scheduling** — a job's
  only future timestamp is `next_attempt_at`.
- **No user-visible priority or concurrency settings.** Users choose
  providers (plan §6.7), not thread pools; all constants are internal
  (STT-§3.5 preamble pattern).
- **No priority inheritance, aging, or dynamic auto-scaling of pools.**
  Starvation is surfaced honestly (§2.2 rule 3), never silently re-prioritized.
- **No scheduler-owned business logic.** Evidence checks (SEC-§3), prompt
  pinning (SEC-§6), timeline correction (STT-§5), and session conclusions
  (SSC-§1) stay in their owners.

## 9. Gate verification checklist

| Gate | Requirement | Where satisfied |
| --- | --- | --- |
| G1 | Concurrency ceilings stated; 'capture never waits on STT' proven under the scheduler, consistent with coexistence-contract.md | §1.1 (class table: L1=1, C1=2, S1=2), §1.2 (CAP1 proof table + test obligations, extending COEX-§3.6) |
| G2 | Cancellation semantics for queued, leased, and running states with idempotent cancel results | §4.2 (per-state table incl. leased), §4.3 (idempotency rules 1–4), §4.4 (downstream effects) |
| G3 | Explicit non-goals; consistent with stt-provider-contract.md job state machine and summary-evidence-contract.md idempotency keys | §8 (non-goals/complexity budget), §7 (consistency matrix), §0.2 (P1 reconciliation) |
| G4 | Documentation index updated | Row added to `docs/meeting-mode-documentation-index.md` §3 (done in this task) |
