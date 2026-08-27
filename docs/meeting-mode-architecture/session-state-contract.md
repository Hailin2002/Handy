# MeetingManager Internal Session-State Contract

Status: **spec contract — no implementation yet** (Phase 4 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 ("Zielarchitektur" →
`MeetingManager`, plan lines 221–227), §6.x ("Session-Zustände", plan lines
877–896; "Laufende Aufnahme", plan line 926; "Pause", "Stoppen",
"Fehlerverhalten"), §9 (`meeting_sessions`, `meeting_audio_chunks`,
`meeting_stt_jobs`, plan lines 1143–1199), §13 Phase 4 acceptance (plan line
1577: "UI und persistierter Zustand stimmen nach Neustart überein"), §12
consent/indicator obligation (plan lines 1458–1461).
Planned implementation home: `src-tauri/src/managers/meeting.rs`.
This document defines the internal seam of `MeetingManager` only; it changes no
source code.

Sibling contracts consumed by this document (read-only):

- [recorder-contract.md](recorder-contract.md) — chunk lifecycle (§3),
  recorder events (§4), pause/resume semantics (§6), open-chunk crash-recovery
  protocol (§8). Below abbreviated **RC-§n**.
- [stt-provider-contract.md](stt-provider-contract.md) — persistent job state
  machine (§3.1/3.2), idempotency (§3.3), restart resumption (§3.4). Below
  abbreviated **STT-§n**.
- [../meeting-mode-phase2-contracts.md](../meeting-mode-phase2-contracts.md) —
  on-disk layout, `manifest.json` schema, data-contract recovery rules (§7).
  Below abbreviated **DC-§n**.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 1. Single authoritative owner of session state (gate G3)

### 1.1 The owner

**`MeetingManager` is the single authoritative owner of the meeting session
state machine.** It is the only component that decides session-state
transitions, the only component that writes `meeting_sessions.state`, and the
only component that publishes session-state changes toward the Tauri layer.
This is the direct consequence of plan lines 223–225: the `MeetingManager` is
"Zentrale Zustandsmaschine und einzige öffentliche Koordinationsstelle".

Every other component holds at most a _subordinate_ view, never an independent
authoritative copy:

| Component                              | What it holds                                                      | Why this is not a second session-state copy                                                                                                                                                                                                                       |
| -------------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MeetingRecorder`                      | `RecorderState` (RC-§4: Idle/Recording/Paused/Stopping/Recovering) | A hardware capture view, strictly narrower than the session state. It changes only when `MeetingManager` calls `pause()`/`resume()`/`stop()`/`open_source()`. It cannot initiate a session transition; `MeetingManager` translates its events into session facts. |
| `MeetingTranscriptionQueue`            | Per-job states (`queued`/…, STT-§3)                                | Job state is per-audio-segment, not per-session. The session-level conclusion ("is everything transcribed?") is derived by `MeetingManager` from store queries.                                                                                                   |
| `MeetingStore`                         | Persisted projection (`meeting_sessions.state` etc.)               | Pure persistence. Every write to these columns is issued inside a `MeetingManager`-initiated transaction; the store never mutates session state on its own authority.                                                                                             |
| Frontend (Hauptfenster, Overlay, Tray) | None                                                               | Plan line 926: all three "lesen denselben Zustand aus dem `MeetingManager`". Plan line 927 forbids a second frontend-local recording state outright. The frontend holds no session-state variable that survives a re-fetch; see §5.                               |

### 1.2 Proof sketch: no second in-memory copy exists

The claim "`MeetingManager` is the only in-memory session-state authority"
holds because each candidate for a second copy is eliminated by construction:

1. **Recorder:** its `RecorderState` is derivable from the session state plus
   recorder-local stream facts, and it is only ever _read back_ by
   `MeetingManager` via `snapshot()` (RC-§4). No code path lets a recorder
   callback set session state; recorder threads communicate exclusively
   through `RecorderEvent`s into the single installed `RecorderEventSink`
   (RC-§4), which is owned by `MeetingManager`.
2. **Queue/provider:** STT-§1.1 gives the queue no session handle at all; it
   knows jobs, engine leases and providers. Provider results never address
   session state.
3. **Frontend surfaces:** plan line 926–927 is normative: Hauptfenster,
   Overlay and Tray read the _same_ state from `MeetingManager`, and there is
   "kein zweiter lokaler Recording-State im Frontend". This contract enforces
   it mechanically in §5 (snapshot-first handshake + revision-stamped events;
   any UI cache is a memoization of the last observed revision, invalidated —
   never decided — by events).
4. **Persistence boundary:** SQLite columns are the durable projection of the
   manager's decision, written synchronously with the decision (§4, rule O1).
   A durable copy is required for crash safety (plan lines 893–896) and is
   explicitly _not_ an independent authority: on restart, the manager — not
   the database — runs the reconciliation protocol of §6 that decides what the
   persisted rows mean.

Consequently exactly one write path exists:

```text
user/system trigger
      │
      ▼
MeetingManager::transition(session_id, from, to)     ← sole decision point
      │  1. guard: current persisted+in-memory state == from
      │  2. side effects ordered per §4 (store tx first, then emit)
      ▼
meeting_sessions.state (SQLite)   +   Tauri event meeting://session-state
```

---

## 2. State set (gate G1)

### 2.1 States

The plan's Session-Zustände block (lines 879–891) names:
`draft → starting → recording ↔ paused → stopping → transcribing →
summarizing (optional) → completed`, plus `recoverable_error` reachable from
every active state, plus the user-abort terminals `stopped_unprocessed` and
`cancelled`. This contract uses exactly that state set. Where the mission
briefing speaks of a single `processing` state, that word denotes the
composite of the two plan states `transcribing` and `summarizing`; the
implementation keeps them separate because the processing checklist (plan
§6.12) renders the two stages distinctly.

| State                 | Meaning                                                                                                                                                                                 | Persisted in `meeting_sessions.state`  |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `draft`               | Start dialog open, nothing captured yet.                                                                                                                                                | yes                                    |
| `starting`            | Validation/opening of devices in progress (plan §6.x Startvalidierung).                                                                                                                 | yes                                    |
| `recording`           | Both configured sources capturing; chunks rolling.                                                                                                                                      | yes                                    |
| `paused`              | Capture frozen at marker; no audio stored or transmitted (plan "Pause").                                                                                                                | yes                                    |
| `stopping`            | Controlled finalization of all open chunks (plan "Stoppen").                                                                                                                            | yes                                    |
| `transcribing`        | Capture done; STT jobs outstanding.                                                                                                                                                     | yes                                    |
| `summarizing`         | Optional; summary provider running over complete transcript.                                                                                                                            | yes (also mirrored in `summary_state`) |
| `completed`           | All jobs succeeded; terminal success.                                                                                                                                                   | yes                                    |
| `stopped_unprocessed` | User aborted after audio was secured; processing not started. Terminal until the user starts processing.                                                                                | yes                                    |
| `cancelled`           | User aborted; no further automatic work. Terminal.                                                                                                                                      | yes                                    |
| `recoverable_error`   | Overlay condition on any active state: a failure occurred that preserves data but blocks progress (e.g. recovery failure, disk condition). Never silent data loss (plan lines 888–890). | yes (`last_error` carries the cause)   |

### 2.2 Full transition table

Emitter column: the component whose action _emits_ the transition. Every row
names the exact persisted effect and the published Tauri event. Event payloads
and ordering rules are defined in §4.

| #   | From → To                                                      | Trigger                                                                                                                                                                                                                                                              | Emitter                                                                                         | Persisted effect (table.column)                                                                                                                                                                                                                                                                                                                                       | Published Tauri event                                                                                     |
| --- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| T01 | (none) → `draft`                                               | User opens the start dialog                                                                                                                                                                                                                                          | `MeetingManager` (command `create_meeting_draft`)                                               | `INSERT meeting_sessions(state='draft', …)` incl. device/provider/language snapshot columns                                                                                                                                                                                                                                                                           | `meeting://session-created`                                                                               |
| T02 | `draft` → `starting`                                           | Start button pressed (enabled only when preflight passes, plan lines 917–919)                                                                                                                                                                                        | `MeetingManager` (command `start_meeting`)                                                      | `UPDATE meeting_sessions SET state='starting'`                                                                                                                                                                                                                                                                                                                        | `meeting://session-state` `{from:'draft',to:'starting'}`                                                  |
| T03 | `starting` → `recording`                                       | Recorder reports first source(s) live (`RecorderEvent::Started`, RC-§4)                                                                                                                                                                                              | `MeetingManager` (event sink)                                                                   | `UPDATE meeting_sessions SET state='recording', started_at=now`                                                                                                                                                                                                                                                                                                       | `meeting://session-state`                                                                                 |
| T04 | `starting` → `draft` (or `cancelled`)                          | Validation fails (device/disk/provider/permission)                                                                                                                                                                                                                   | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='draft', last_error=…`                                                                                                                                                                                                                                                                                                             | `meeting://session-state` + `meeting://session-error`                                                     |
| T05 | `recording` → `paused`                                         | Pause requested (UI/shortcut/tray)                                                                                                                                                                                                                                   | `MeetingManager` calling `MeetingRecorder::pause()` (RC-§4); emitted on `RecorderEvent::Paused` | `UPDATE meeting_sessions SET state='paused'`; `INSERT` pause `TimelineMarker` (markers persisted by manager per RC-§6.4)                                                                                                                                                                                                                                              | `meeting://session-state`                                                                                 |
| T06 | `paused` → `recording`                                         | Resume requested; devices re-validated (RC-§6.3)                                                                                                                                                                                                                     | `MeetingManager` calling `MeetingRecorder::resume()`; emitted on `RecorderEvent::Resumed`       | `UPDATE meeting_sessions SET state='recording'`; `INSERT` resume marker; device change ⇒ new source segment row                                                                                                                                                                                                                                                       | `meeting://session-state`                                                                                 |
| T07 | `recording`/`paused` → `stopping`                              | Stop requested (UI/shortcut/tray/DiskFull controlled stop)                                                                                                                                                                                                           | `MeetingManager` (command `stop_meeting`, or sink on `ControlledStopInitiated`)                 | `UPDATE meeting_sessions SET state='stopping'`                                                                                                                                                                                                                                                                                                                        | `meeting://session-state`                                                                                 |
| T08 | `stopping` → `transcribing`                                    | `MeetingRecorder::stop()` returned `SessionSummary` — i.e. all open chunks durably `ready` (RC-§4 contract: returns only after durability) **and** ≥1 STT job was enqueued                                                                                           | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='transcribing', ended_at=now`                                                                                                                                                                                                                                                                                                      | `meeting://session-state` (this event is the UI's permission to close the recording view, plan "Stoppen") |
| T09 | `stopping` → `completed`                                       | Same as T08 but zero STT-eligible chunks (empty/degenerate session, no summarizing configured)                                                                                                                                                                       | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='completed', ended_at=now`                                                                                                                                                                                                                                                                                                         | `meeting://session-state`                                                                                 |
| T10 | `transcribing` → `summarizing`                                 | Queue reports all STT jobs terminal-successful and a summary provider is configured                                                                                                                                                                                  | `MeetingManager` (queue callback)                                                               | `UPDATE meeting_sessions SET state='summarizing', summary_state='running'`                                                                                                                                                                                                                                                                                            | `meeting://session-state`                                                                                 |
| T11 | `transcribing` → `completed`                                   | As T10 but no summary provider configured (`summarizing` optional, plan line 885)                                                                                                                                                                                    | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='completed'`                                                                                                                                                                                                                                                                                                                       | `meeting://session-state`                                                                                 |
| T12 | `summarizing` → `completed`                                    | Summary persisted successfully                                                                                                                                                                                                                                       | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='completed', summary_state='succeeded'` (+ `meeting_summaries` row)                                                                                                                                                                                                                                                                | `meeting://session-state` + `meeting://summary-available`                                                 |
| T13 | `transcribing`/`summarizing` → `recoverable_error`             | Jobs exhausted retries (`failed`) or summary permanently failed                                                                                                                                                                                                      | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='recoverable_error', last_error=…`; audio retention protected (plan §12: failed jobs prevent deletion)                                                                                                                                                                                                                             | `meeting://session-error`                                                                                 |
| T14 | `recoverable_error` → `transcribing`/`summarizing`/`completed` | User retries failed jobs (STT-§3.2 `failed → queued` on explicit user action) or dismisses with processing complete                                                                                                                                                  | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state=… , last_error=NULL`                                                                                                                                                                                                                                                                                                               | `meeting://session-state`                                                                                 |
| T15 | `stopping` → `stopped_unprocessed`                             | User chose abort during stop (plan lines 889–890); the stop itself still runs the full safe-finalization of §3.3 — abort changes the _destination_, never the durability work. Requested from `recording`/`paused`, the request enters T07 first with an abort flag. | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='stopped_unprocessed', ended_at=now`; chunks stay `ready`, no jobs enqueued                                                                                                                                                                                                                                                        | `meeting://session-state`                                                                                 |
| T16 | `starting`/`draft` → `cancelled`                               | User cancels before/at start                                                                                                                                                                                                                                         | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='cancelled', ended_at=now`                                                                                                                                                                                                                                                                                                         | `meeting://session-state`                                                                                 |
| T17 | any active → `recoverable_error` (overlay)                     | Recoverable fault during active state (e.g. `ChunkStatePersistFailed` twice, recovery failure RC-§8) — capture/data preserved where possible                                                                                                                         | `MeetingManager`                                                                                | `UPDATE meeting_sessions SET state='recoverable_error', last_error=…`. To make T14/T18 resumption deterministic, this contract proposes one schema addition to plan §9: column `previous_active_state` on `meeting_sessions` (written by T17, read by T14). Flagged here explicitly as a contract proposal for the Phase-4 migration, not as an existing plan column. | `meeting://session-error`                                                                                 |
| T18 | `stopped_unprocessed` → `transcribing`                         | User later chooses "process now"                                                                                                                                                                                                                                     | `MeetingManager` (enqueues jobs from `ready` chunks)                                            | `UPDATE meeting_sessions SET state='transcribing'`                                                                                                                                                                                                                                                                                                                    | `meeting://session-state`                                                                                 |

Non-listed transitions are forbidden. In particular there is no
`recording → completed` shortcut, no way to leave `stopping` before
`MeetingRecorder::stop()` has returned (T08/T09/T15 all consume its result),
and no transition out of `completed` except through an explicitly separate
re-transcription flow (STT-§3.2, terminal `succeeded` semantics).

### 2.3 Relationship to subordinate state machines

- **Chunk states** (`writing/ready/damaged/deleted`, RC-§3): advanced by the
  recorder, reported via `RecorderEvent::ChunkFinalized`; they constrain
  transitions T08/T09/T15/T18 (only durably `ready` chunks let a session leave
  `stopping`).
- **Job states** (STT-§3): advanced by the queue. They constrain T10–T14.
  `MeetingManager` derives "all jobs terminal" exclusively from
  `meeting_stt_jobs.state` queries — never from queue memory.
- **Recorder state** (RC-§4 `RecorderState`): mirrors capture mechanics only;
  `Recording/Paused/Stopping` correspond 1:1 to the session states of the same
  name and are changed only by manager calls.

---

## 3. Orchestration duties: start / pause / resume / stop

### 3.1 Start (`draft → starting → recording`)

Order is normative; each step's failure maps to T04.

1. Manager validates non-recorder concerns (provider/model/language, free
   disk per plan §12 budget, permissions, retention policy) — RC-§7.2 assigns
   these to the manager deliberately, the recorder only does device preflight.
2. `preflight(&devices)` (RC-§4). Failure ⇒ repair dialog data, T04.
3. Persist `state='starting'` **before** opening streams, so a crash during
   startup lands on a state the restart protocol understands (§6).
4. `start_session(layout, store)` (RC-§4) — creates layout, fixes timeline
   origin, runs stale-chunk recovery if reusing a session id.
5. `open_source(…)` for microphone, then system (or mic-only fallback after
   explicit user choice, which is persisted visibly in the session per plan
   lines 919–921).
6. On first `Started` event(s): T03. Only now may the UI report "Aufnahme
   läuft".
7. Enqueue rule: from the first finalized chunk onward, `ready` chunks become
   STT jobs (DC-§6 consumption contract); capture never waits on STT (plan
   line 248).

Consent gate: before step 2 the consent choke-point of §7 must evaluate; see
§7 for the seam marking (final placement owned by the task-0042 audit).

### 3.2 Pause (`recording → paused`) and resume (`paused → recording`)

1. Manager calls `pause()`; recorder finalizes both open chunks to `ready`
   (durability checkpoint, RC-§6.1) and emits `Paused { marker }`.
2. Manager persists the pause marker row and flips state (T05). During pause:
   no level events forwarded, backlog counts frozen, timeline frozen.
3. Resume: manager re-runs the device subset of validation, calls
   `resume(devices)`; recorder re-validates, optionally opens new source
   segments on identity change (RC-§6), emits `Resumed { marker }`.
4. Manager persists resume marker and flips state (T06). A resume-validation
   failure leaves the session in `paused` (RC-§6.3) and surfaces the
   corresponding `RecorderError` to the UI.

### 3.3 Stop (`recording|paused → stopping → transcribing|completed`)

1. Manager flips to `stopping` (T07) and calls `stop()`.
2. Recorder finalizes all open chunks; `stop()` returns `SessionSummary` only
   after the last chunk is durably `ready` (RC-§4). The manager treats this
   return as the sole permission to execute T08/T09.
3. After T08/T09 the UI may close immediately while STT/summary jobs continue
   in background (plan lines 939–941); the processing checklist of §6.12 is
   driven by job-state queries + `meeting://jobs-updated` events.
4. Disk-full controlled stop: `ControlledStopInitiated { DiskFull }` routes
   through the same T07→T08 path so "Audio gesichert" is truthful.

---

## 4. Event-publication ordering guarantees toward the Tauri layer

These rules govern every `meeting://*` event. They exist so that three
independent windows (main, overlay, tray) plus late subscribers converge on
the same state without ever keeping their own authoritative copy (plan lines
925–927).

- **O1 — Persist before publish.** An event for a transition is published
  strictly after the transition's SQLite transaction committed. Consequences:
  a subscriber that reacts to an event can always read the causing state from
  the store; a crash between commit and publish loses only the notification,
  never the fact, and is healed by O5/snapshot queries.
- **O2 — Total per-session order.** Each session has a monotonically
  increasing integer `state_revision` (persisted alongside
  `meeting_sessions.state`). Events carry `(meeting_id, state_revision)`;
  publishers serialize transitions per session (one mutex/actor), so
  revisions on the wire are strictly increasing with no gaps caused by
  concurrent emissions.
- **O3 — Stale-revision discard.** Any listener (including frontend stores)
  must ignore events whose `state_revision` ≤ its last applied revision. This
  makes duplicate or out-of-order delivery harmless.
- **O4 — Cross-source recorder events are unordered inbound, ordered outbound.**
  RC-§4 guarantees ordering only within one source. `MeetingManager` is the
  serialization point: it converts recorder events into session facts and
  publishes them under O1–O3, so consumers never observe cross-source races.
- **O5 — Snapshot-first handshake.** A newly created/visible window must call
  the snapshot command (`get_meeting_snapshot`) before or concurrently with
  subscribing; the snapshot response includes the current `state_revision`.
  If a received event's revision exceeds last-known + 1 gap tolerance, the
  listener re-fetches the snapshot instead of guessing intermediate states.
- **O6 — At-least-once delivery, idempotent application.** Because Tauri
  event delivery is best-effort, O3+O5 make re-delivery safe and missed
  delivery recoverable; no consumer may implement its own counter-state.
- **O7 — Payload hygiene.** Event payloads contain ids, states, revisions,
  levels, error _messages_ (redacted per the compact-error practice) — never
  secrets or provider keys (extends the STT-§7 invariant to all meeting
  events).

---

## 5. UI rebuild rule (plan line 1577)

Phase 4 acceptance requires "UI und persistierter Zustand stimmen nach
Neustart überein". Rule: **the UI is a pure function of persistence plus
events.**

1. There is no persisted UI-session-state anywhere (no settings key, no
   frontend store that outlives its snapshot).
2. On app start and on every window creation, Hauptfenster/Overlay/Tray render
   from `get_meeting_snapshot` (which reads `meeting_sessions`,
   `meeting_audio_chunks` aggregates, `meeting_stt_jobs` aggregates and
   recorder `snapshot()` where a live recorder exists).
3. Afterwards, §4 events keep views current; any inconsistency resolves in
   favor of the store (O5 resync).
4. Therefore, after a restart, the UI shows exactly the post-reconciliation
   persistence state produced by §6 — the acceptance criterion holds by
   construction, and any violation is by definition a defect in this contract
   or the snapshot query, not in a window.

---

## 6. Restart reconciliation protocol (gate G2)

Goal (plan lines 893–896): the persisted session state and the real job/chunk
states are reconciled at program start; a session never remains stuck in
`recording` after a crashed process; open files are marked recoverable,
finalized and offered for processing.

### 6.1 Consulted persistence, exhaustively

| Store                                           | Columns/fields consulted                                                                                                                                                        | Role in reconciliation                                                    |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `meeting_sessions`                              | `id`, `state`, `started_at`, `ended_at`, `last_error`, `summary_state`, `has_microphone_audio`, `has_system_audio`, `updated_at` (plan §9)                                      | Candidate selection and final disposition                                 |
| `meeting_audio_chunks`                          | `id`, `meeting_id`, `sequence`, `source`, `state`, `timeline_start_ms`, `timeline_end_ms`, `path` (storage key), `sample_count`, Prüfsumme (checksum), `recovered` flag (DC-§4) | Open-chunk repair input; DB rows are authoritative for _attempted_ chunks |
| Filesystem `meetings/<id>/audio/`, `/recovery/` | `.wav`/`.wav.part` presence, parseability, byte lengths                                                                                                                         | Authoritative for _survived_ bytes (RC-§8 step 1 principle)               |
| `manifest.json`                                 | DC-§2 schema (chunks[], markers, sources)                                                                                                                                       | Index/cross-check artifact only — **never** authoritative (DC-§5.3)       |
| `meeting_stt_jobs`                              | `id`, `meeting_id`, `state`, `attempt_count`, `next_attempt_at`, `last_error`, referenzierte Chunk-IDs (`audio_chunk_ids`), Idempotency-Key (STT-§2/§3.3)                       | Job resumption per STT-§3.4                                               |
| Process liveness                                | single-instance lock / pid file (established by `MeetingManager`, RC-§8 preamble: out of recorder scope)                                                                        | Decides whether `recording` rows describe a live or dead process          |

### 6.2 Deterministic ordering

Executed once per app start, single-threaded, after migrations, in one
logical startup-reconciliation unit (the job abgleich shares this transaction
scope per STT-§3.4 last bullet):

```text
R0  Acquire startup reconciliation exclusivity (same single-instance
    mechanism as above). Abort if another instance owns it.

R1  SELECT candidates FROM meeting_sessions
    WHERE state IN ('starting','recording','paused','stopping')
      AND no live recorder exists for them            -- always true at boot

R2  For each candidate, oldest updated_at first:
    run MeetingRecorder::recover_open_chunks(session_id, store)
    -- implements RC-§8 verbatim:
       2a parseable .part  -> finalize writing->ready, recovered=true
       2b unparseable .part-> writing->damaged, move to recovery/
       2c row w/o file     -> writing->damaged
       3  orphan .wav w/o ready row -> insert/update ready, recovered=true
          orphan .part w/o row        -> quarantine into recovery/
    Result: RecoveryReport { finalized, damaged, quarantined_files }.
    Idempotent (RC-§8 step 5); a repeated run is a no-op.

R3  Reconcile meeting_stt_jobs for ALL meetings (not only candidates),
    exactly STT-§3.4:
       queued     -> stays queued
       running    -> queued  (no attempt_count increment, no backoff penalty;
                              unchanged idempotency_key)
       retry_wait -> keep next_attempt_at; overdue becomes due now
       succeeded/failed/cancelled -> untouched

R4  Post-condition check per candidate: no meeting_audio_chunks row of this
    session may remain state='writing'. If R2 could not achieve this
    (RecoveryFailed), mark the offending chunk 'damaged' if the evidence
    permits (2b/2c rules) else route to disposition (c) below.

R5  Session disposition (one UPDATE per session, inside the same
    reconciliation unit):
    (a) session was captured mid-flight ('starting'/'recording'/'paused') and
        R2/R4 completed without fatal error -> state='stopped_unprocessed';
        this applies even when the RecoveryReport is empty (a crash right
        after a clean chunk rollover leaves no .part behind); audio secured so
        far is offered for processing (plan lines 895–896; RC-§8 step 4).
    (b) session had already progressed past capture (stopping) or holds only
        terminal jobs -> advance per §2.2 (e.g. stopping->transcribing if
        eligible jobs exist, else completed).
    (c) recovery failed irrecoverably
        -> state='recoverable_error' with last_error; data preserved for
           forensics; never silent loss.
    HARD REQUIREMENT (plan line 894, RC-§8 step 4): no session may remain in
    'recording' (or 'paused' pretending liveness) after R5.

R6  Emit a single consolidated event per affected session
    (meeting://session-recovered {meeting_id, RecoveryReport-summary,
    state_revision}) AFTER the disposition commit (rule O1), so a freshly
    started UI rebuilds per §5 and shows the banner.
```

### 6.3 Consistency with sibling contracts

- **Recorder contract (RC-§8):** steps R2/R3/R4 are RC-§8 executed under a
  defined scheduling order; R5(a)/(c) realize RC-§8 step 4's
  `stopped_unprocessed` / `recoverable_error` outcomes. The pid/liveness
  precondition RC-§8 defers to the manager is exactly R0/R1.
- **Data contract (DC-§7):** DC-§7's rules (DB lists attempted chunks, FS
  decides survival, recovery idempotent and per-source, sessions never left
  in `recording`/`paused` without a live owner) are respected; R2 runs
  per-session and per-source inside the recorder, and the manifest is used
  only as a cross-check, matching DC-§5.3.
- **STT contract (STT-§3.4):** R3 is STT-§3.4 verbatim, including "no
  attempt-count increase on crash" and shared transaction scope with the
  session-state reconciliation (R5). Ordering R2-before-R3 additionally
  guarantees that jobs enqueued _after_ recovery reference chunks whose final
  `ready`/`damaged` status is already fixed, preserving the STT consumption
  rule "only `ready` chunks may be referenced" (RC-§3 property 4): a queued
  job whose chunk turned out `damaged` is failed by the worker's pre-run
  readiness validation as a permanent configuration/segmentation error
  (STT-§6.1 pattern), never silently dropped.

---

## 7. Consent/indicator gate — named choke-point seam (gate G4)

> ⚠️ **Seam marker, not an implementation.** The final location, wording and
> bypass rules of the consent prompt and the recording indicator are owned by
> the **task-0042 audit** (privacy/consent recommendation). This section only
> pins the seam so no implementation can scatter the check.

Plan obligations feeding the seam: plan lines 1458–1461 — recording
non-public conversations requires consent; Speak shows a **Consent-Hinweis
before start** and a **visible indicator during the entire recording**; the
feature must not be a stealth recorder. Plan line 1570 adds "Consent-Prüfungen"
to Phase 4 tasks.

**Seam name:** `session_gate_consent`, implemented as a choke point inside
`MeetingManager::transition` evaluated at:

1. **Gate point G-start:** transition T02 (`draft → starting`). The transition
   is refused (stays `draft`, reason surfaced to the start dialog) unless the
   consent acknowledgment for this start is recorded. Refusal is a
   _transition guard_, so no capture resource is opened before consent —
   making a stealth path structurally impossible rather than policy-enforced.
2. **Gate point G-indicator:** entry into any state in which capture is live
   (`starting → recording`, T03) requires the indicator surface to have been
   armed; and exit from all such states is the only place where the indicator
   may be cleared. Because T03/T08 are the sole capture-boundary transitions
   (§2.2), the indicator lifetime is pinned to the state machine itself and
   cannot drift from it.

Constraints inherited regardless of task-0042's final recommendation:

- The gate lives in `MeetingManager` (single owner, §1), never in the
  frontend (a frontend gate would be bypassable and duplicated) and never in
  the recorder (wrong layer, no session context).
- The gate decision must be logged (without new sensitive content) together
  with the transition revision, so audits can prove the gate ran.
- Until task-0042 lands, implementations must treat the gate as
  **fail-closed**: missing/unresolved consent recommendation ⇒ refuse T02.

---

## 8. Complexity budget / non-goals

In line with the company complexity budget, this contract deliberately does
**not** specify:

- distributed-consensus machinery around the state machine (a single-process
  mutex/actor plus SQLite transactions suffice);
- a general-purpose workflow engine or plugin hook system around transitions;
- speculative states beyond the plan's block (no `uploading`, no
  per-source sub-states at session level);
- frontend-owned state mirrors, optimistic UI state, or retry logic in the UI
  for state convergence (O3/O5 make them unnecessary);
- any consent UX content (deferred to task-0042).

Anything not in §2.2's table is a non-goal until the plan changes.

---

## Appendix A — Gate traceability

| Gate                                                                                                                                    | Coverage                                                                                                      |
| --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| G1 full transition table incl. `recoverable_error`, emitter + persisted effect + event per row                                          | §2.1, §2.2 (T01–T18; `recoverable_error` as overlay via T13/T17, user-abort terminals per plan lines 889–890) |
| G2 restart reconciliation with exact tables/columns, deterministic order, consistency with recorder recovery and STT restart-resumption | §6.1 (column inventory), §6.2 (R0–R6), §6.3 (explicit mapping to RC-§8, DC-§7, STT-§3.4)                      |
| G3 single owner + proof of no second in-memory copy, citing plan line 926                                                               | §1.1, §1.2 (plan lines 925–927 cited; per-component elimination argument)                                     |
| G4 consent/indicator gate as marked seam owned by task-0042 audit                                                                       | §7 (fail-closed, location constraints, no implementation of wording/flow)                                     |
| G5 git status confined to `docs/meeting-mode-architecture/`                                                                             | verified after authoring via `git status --porcelain`; recorded in the mission result evidence                |
