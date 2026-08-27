# Live-Meeting-View Streaming Delta Contract — Transcript Deltas, Progress Events, Cadence & Coalescing

Status: **spec contract — no implementation yet** (Phase 4 prerequisite; changes
no source code)
Source of truth: `docs/meeting-mode-plan.md` — §6.11 "Laufende
Meetingansicht" (plan lines 613–637: fertiggestellte Transkriptsegmente,
`Hört zu`/`Abschnitt wird transkribiert`/`Verarbeitung liegt zurück`,
Mic-/System-Pegel, wartende Abschnitte), §6.12 processing checklist (plan
lines 639–657), §9 data model incl. change-baseline rules (plan lines
1149–1162) and `meeting_transcript_segments` (plan lines 1201–1208),
"Stoppen" (plan lines 936–941: UI may close while processing continues), §5
"Capture wartet nie auf Inferenz" (plan line 248), Phase 2 "Pegel- und
Zustandsereignisse" (plan line 1525).
Planned implementation home: publisher logic inside
`src-tauri/src/managers/meeting.rs` (same owner as the session-state events —
`MeetingManager`); consumer side is governed by
[frontend-state-contract.md](frontend-state-contract.md) (FSC) §3/§4.

This document is a **strict delta to
[session-state-contract.md](session-state-contract.md) (SSC)**: it specifies
the event channels that feed the _running meeting view_ (plan §6.11) — the
streamed appearance of finished transcript segments, per-source job/progress
aggregates, and audio level meters — with payload schemas, update cadence,
coalescing rules, worst-case backlog bounds, reload/resubscribe behavior, and
ordering guarantees relative to the SSC §4 session-state events. It modifies
nothing that SSC §4 already defines; every rule below is additive (§1.2).

Sibling contracts consumed by this document (read-only):

- [session-state-contract.md](session-state-contract.md) — event-publication
  guarantees O1–O7, UI rebuild rule (§5). Below abbreviated **SSC-§n**.
- [frontend-state-contract.md](frontend-state-contract.md) — store boundaries
  (B5: level meters bypass React state), listener ownership (L1–L8), tier-B
  coalescing posture (FSC-§4.3). Below abbreviated **FSC-§n**.
- [recorder-contract.md](recorder-contract.md) — `RecorderEvent::LevelUpdate`
  and `LEVEL_EVENT_INTERVAL_MS` (RC-§5.1 constants). Below abbreviated
  **RC-§n**.
- [stt-provider-contract.md](stt-provider-contract.md) — job state machine and
  the segment-insert transaction (STT-§3.3). Below abbreviated **STT-§n**.
- [job-scheduler-contract.md](job-scheduler-contract.md) — concurrency classes
  L1/C1/S1 that bound realistic completion rates. Below abbreviated
  **JSC-§n**.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 1. Scope and additivity (gate G3)

### 1.1 What this contract specifies

Three event channels toward the running meeting view, all published by
`MeetingManager` (the single serialization point of SSC-§1):

| #   | Topic _(proposed names — pending `identifier-registry.md` registration, see §8 NG-9)_                                | Feeds plan §6.11 element                                                                                                                                                                                   | Tier                      |
| --- | -------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| E1  | `meeting://transcript-delta`                                                                                         | "zuletzt fertiggestellte Transkriptsegmente mit Zeitstempeln" (plan line 624)                                                                                                                              | tier A data, delta-shaped |
| E2  | `meeting://jobs-updated` (topic already named in FSC-§3.2; **this contract defines its payload for the first time**) | "klarer Zustand `Hört zu`, `Abschnitt wird transkribiert`, `Verarbeitung liegt zurück`" (plan line 625) and "Anzahl wartender Abschnitte" (plan line 633); also the §6.12 checklist counts (plan line 647) | tier B aggregate          |
| E3  | `meeting://level-update`                                                                                             | "getrennte kleine Mic-/System-Pegel mit Textlabel" (plan line 632)                                                                                                                                         | tier C telemetry          |

The three §6.11 status words are **derived, never published**: the frontend
computes `Hört zu` (backlog = 0, capture live), `Abschnitt wird
transkribiert` (0 < backlog), `Verarbeitung liegt zurück` (backlog above a
UI-local threshold, FSC-§4.3) from E2 aggregates plus the session state from
SSC events. No fourth "view status" event exists (§8).

### 1.2 Additivity statement (gate G3, checked against SSC §4 verbatim)

- **No SSC rule is restated, weakened, or redefined.** O1–O7 apply to the
  session-state events exactly as written; the rules in §5 below (D1–D8) are
  _additional_ guarantees for the new channels, each of which either mirrors
  an SSC pattern onto a new stream or states cross-stream independence.
- **No existing topic changes payload or semantics.**
  `meeting://session-created`, `meeting://session-state`,
  `meeting://session-error`, `meeting://summary-available`,
  `meeting://session-recovered` are untouched. `meeting://jobs-updated` was
  named but its payload was explicitly left to backend contracts
  (FSC-§0.2 item 1); defining it here is additive, not a change.
- **No new authority.** Deltas are lossy-tolerant notifications; SQLite
  remains the only authority for transcript content and job counts
  (SSC-§1.1, FSC-§4.4). A consumer that receives no delta has learned
  nothing about absence (mirrors FSC-§4.4 "never derive from event silence").
- **No new frontend state machine.** Consumers apply FSC-§3 rules L1–L8
  unchanged; this contract only adds the payload shapes they normalize.

---

## 2. Payload schemas (normative)

All payloads are JSON via Tauri events. Field names are binding; unknown
fields must be ignored by consumers (forward compatibility). Payload hygiene
follows SSC-O7: ids, states, revisions, levels, redacted messages only — no
secrets, no provider keys, no absolute paths (segment text is user content,
not a secret, and is included deliberately to avoid a refetch per segment).

### 2.1 E1 — `meeting://transcript-delta`

Emitted when one or more `meeting_transcript_segments` rows become visible in
a committed transaction (STT-§3.3 inserts segments in the job's success
transaction; D1 in §5 ties publication to that commit).

```jsonc
{
  "type": "transcript-delta",
  "meeting_id": "uuid", // owning session
  "segments": [
    // 1..=LV_DELTA_MAX_SEGMENTS entries
    {
      "id": "uuid", // meeting_transcript_segments.id
      "job_id": "uuid", // producing meeting_stt_jobs.id
      "source": "microphone", // "microphone" | "system"
      "start_ms": 0, // timeline position (plan §9)
      "end_ms": 0,
      "sequence": 1, // persisted segment sequence, see D2
      "text": "…", // final segment text (not partial words)
      "confidence": 0.0, // optional, plan §9 "optional … Confidence"
    },
  ],
  "last_sequence": 1, // max `sequence` in this batch
  "state_revision": 7, // session state_revision at publish time —
  // correlation only, NEVER applied by the
  // consumer's session guard (SSC-O3)
  "truncated": false, // true ⇒ producer dropped older batches
  // (congestion, §7.3); consumer must resync
}
```

Semantics:

- **Append-only.** A delta only ever introduces new segments. Edits or
  replacements (re-transcription, plan §11) are _not_ deltas: they mutate
  persisted rows and are surfaced by the existing envelope/refetch machinery
  of the detail view, not by the live channel (§8 NG-4).
- **"Partial transcript" means partially _delivered session transcript_,
  not partial text.** Every segment in a delta is a finished, persisted
  segment (plan §6.11 shows finished segments with timestamps; word-level
  live output is explicitly out of scope for v1, plan lines 1130–1133).

### 2.2 E2 — `meeting://jobs-updated`

Emitted by `MeetingManager` after any committed change to
`meeting_stt_jobs` of one meeting (queue callback, retry scheduling, retry
exhaustion, cancellation). Aggregates only — never per-job churn
(FSC-§2.2 row 3).

```jsonc
{
  "type": "jobs-updated",
  "meeting_id": "uuid",
  "per_source": {
    // one entry per source with eligible jobs
    "microphone": {
      "queued": 2,
      "running": 1,
      "retry_wait": 0,
      "failed": 0,
      "succeeded": 9,
    },
    "system": {
      "queued": 1,
      "running": 0,
      "retry_wait": 0,
      "failed": 1,
      "succeeded": 8,
    },
  },
  "total_backlog": 4, // sum of queued + running + retry_wait
  "oldest_failed": {
    // present iff failed > 0 (FSC S1 slice)
    "count": 1,
    "first_failed_at": "2026-08-22T10:00:00Z",
  },
  "state_revision": 7, // correlation only (§2.1)
}
```

The §6.12 checklist ("37 von 42 Abschnitten transkribiert", plan line 647)
is computed from `succeeded` vs. total eligible jobs; the totals are part of
the snapshot (§6.2), not of every event, so the event stays small.

### 2.3 E3 — `meeting://level-update`

Forwarded from `RecorderEvent::LevelUpdate` (RC-§5.1). The recorder already
throttles to `LEVEL_EVENT_INTERVAL_MS = 100` ms per source; the manager adds
no second timer (§8 NG-6).

```jsonc
{
  "type": "level-update",
  "meeting_id": "uuid",
  "source": "microphone", // "microphone" | "system"
  "peak": 0.0, // normalized mono signal, RC scale
  "rms": 0.0,
  "at_timeline_ms": 0.0,
}
```

Suppression rules (inherited, restated for completeness): no level events
outside `recording` — the pause freeze of SSC-§3.2 step 2 ("no level events
forwarded") and the absence of capture after T08 leave the meters at their
last value; the UI renders them from refs/rAF, never from React state
(FSC-B5).

---

## 3. Cadence and coalescing (gate G1)

### 3.1 Constants (testable, UI constants not spike-derived)

| Constant                | Value                                | Applies to                                        |
| ----------------------- | ------------------------------------ | ------------------------------------------------- |
| `LV_DELTA_COALESCE_MS`  | 250                                  | E1 trailing-edge coalescing window per meeting    |
| `LV_DELTA_MAX_SEGMENTS` | 32                                   | hard batch cap per E1 event                       |
| `LV_DELTA_PENDING_CAP`  | 1024                                 | producer-side pending-segment buffer bound (§7.3) |
| `LV_JOBS_COALESCE_MS`   | 500                                  | E2 trailing-edge coalescing window per meeting    |
| `LV_LEVEL_INTERVAL_MS`  | 100 (= RC `LEVEL_EVENT_INTERVAL_MS`) | E3 per source                                     |

### 3.2 Coalescing algorithm

Per `(topic, meeting_id)` the publisher runs one trailing-edge coalescer:

1. A committed fact (segment insert / job-state change) enters the pending
   buffer and starts (or restarts) the window timer if none is running.
2. When the window expires, exactly one event is emitted carrying the whole
   pending batch (E1: up to `LV_DELTA_MAX_SEGMENTS` segments; E2: the latest
   aggregate snapshot — aggregates collapse, so E2 never batches).
3. E1 overflow: more than 32 pending segments at emit time ⇒ each
   subsequent window tick emits one more full batch (32 segments) until the
   buffer is drained; the _rate_ cap of §3.3 (≥ `LV_DELTA_COALESCE_MS`
   between events) always holds, so a huge backlog drains over consecutive
   windows instead of bursting.
4. Terminal facts are never coalesced away: the trailing-edge rule already
   guarantees the last state of a burst is always emitted (matches
   FSC-§4.3 "the last message of a burst is always applied"); additionally a
   `failed` job transition flushes the E2 window immediately so error UI is
   not delayed by up to 500 ms.

### 3.3 Maximum event rates and worst-case backlog bound (gate G1)

**Rate caps (per meeting):**

| Channel | Max emission rate                          | Source of the bound            |
| ------- | ------------------------------------------ | ------------------------------ |
| E1      | 4 events/s (= 1000/`LV_DELTA_COALESCE_MS`) | coalescing window              |
| E2      | 2 events/s (= 1000/`LV_JOBS_COALESCE_MS`)  | coalescing window              |
| E3      | 10 events/s per source, ≤ 20/s per meeting | recorder throttle, two sources |

Worst case for the webview is therefore **≤ 26 events/s across all three
channels while a session is live**, and ≤ 6 events/s after capture ends
(E3 stops). For comparison, the session-state channel is bounded by the
transition table itself (a handful of events per meeting lifetime).

**Worst-case backlog bound, computed (G1):**

- _Instantaneous burst of M finished segments_ (worst realistic shape: a
  re-transcription run or many cloud jobs returning inside one poll —
  bounded by JSC-§1 concurrency classes, but the bound must not _depend_ on
  that): emissions = ⌈M/32⌉ events, spaced ≥ 250 ms ⇒ drain time
  ≈ 0.25 s × (⌈M/32⌉ − 1). Concrete worst case: an entire 120-minute
  two-source meeting (240 chunks at the 60 s rollover target, RC constants)
  re-transcribed "instantaneously": M = 240 ⇒ ⌈240/32⌉ = **8 delta events
  over ≈ 1.75 s**. The webview performs ≤ 8 store actions; no render storm
  is possible because coalescing is upstream of IPC.
- _Sustained throughput ceiling_: 4 events/s × 32 segments = **128
  segments/s** sustainable. Realistic completion is ≤ ~2 segments/s (one
  local engine lease at a time, JSC-L1; bounded cloud slots, JSC-C1) — a
  ≥ 64× headroom. Input above 128 segments/s sustained is structurally
  implausible and additionally handled by the congestion rule of §7.3
  (bounded buffer + `truncated` flag), so no unbounded queue can exist on
  either side of the seam.

---

## 4. Ordering guarantees relative to session-state events (gates G1, G3)

Rules D1–D8. D1–D3 mirror SSC O1–O3 onto the new streams; D4–D6 govern the
cross-stream relationship; D7–D8 cover delivery semantics. Nothing here
modifies O1–O7.

- **D1 — Persist before publish (mirror of O1).** An E1 event is published
  strictly after the transaction that inserted its segments committed
  (STT-§3.3); an E2 event strictly after the job-row transaction committed.
  A consumer can therefore always read the causing rows from the store.
- **D2 — Per-stream total order via persisted keys (mirror of O2).** E1
  events for one meeting carry strictly increasing `last_sequence` values,
  where `sequence` is the persisted `meeting_transcript_segments.sequence`
  (plan line 1207): the publisher serializes through the same per-session
  actor that SSC-O2 mandates, so no concurrent emission can reorder the
  stream. E2 carries no counter because aggregates are idempotent
  (re-applying a newer aggregate subsumes an older one).
- **D3 — Stale-discard (mirror of O3).** Consumers ignore an E1 batch whose
  `last_sequence` ≤ the highest applied sequence for that meeting.
  Duplicate or out-of-order delivery is thereby harmless.
- **D4 — Cross-stream independence.** A session-state event and a live-view
  event never depend on each other for correct application: consumers must
  not derive session state from E1/E2 (SSC-§1: the manager is the only
  authority) and must not gate delta application on session state. The
  `state_revision` field on E1/E2 exists for logging/correlation only and
  MUST NOT be fed into the SSC-O3 session guard.
- **D5 — Cross-source ordering (extends O4's pattern).** Recorder level
  events arrive at the manager unordered across sources (RC-§5.1);
  `MeetingManager` is the serialization point and publishes under D1–D3, so
  consumers never observe cross-source races on any live-view channel.
- **D6 — Pause freeze.** During `paused`, E3 is suppressed (SSC-§3.2) and
  E1/E2 quiesce naturally (no capture ⇒ no new chunks ⇒ at most straggler
  job completions, which are still published — the checklist must stay
  truthful during a paused session).
- **D7 — At-least-once delivery, idempotent application (mirror of O6).**
  Tauri delivery is best-effort; D3 + the resync rules of §6 make
  re-delivery safe and missed delivery recoverable. No consumer may keep
  its own authoritative counter-state.
- **D8 — Payload hygiene (mirror of O7).** §2 schemas carry ids, states,
  revisions, levels, segment text, redacted error metadata only.

---

## 5. What each §6.11 surface consumes (traceability)

| Plan §6.11 element                                                       | Consumes                                                                                  |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| Status bar "Aufnahme läuft"/"Pausiert", Dauer                            | SSC events + snapshot only (unchanged)                                                    |
| "Audio gespeichert" / warning                                            | snapshot chunk aggregates + `meeting://session-error`; no dedicated chunk event (§8 NG-3) |
| Finished transcript segments with timestamps                             | E1 + tail page (§6.3)                                                                     |
| `Hört zu` / `Abschnitt wird transkribiert` / `Verarbeitung liegt zurück` | E2 aggregates + session state (derived, §1.1)                                             |
| Mic/system level meters                                                  | E3 (refs/rAF, FSC-B5)                                                                     |
| "Anzahl wartender Abschnitte" (only when backlog exists)                 | E2 `total_backlog`                                                                        |
| §6.12 processing checklist after stop                                    | E2 + snapshot totals (plan lines 939–941: view may close; checklist continues)            |

---

## 6. Webview reload / resubscribe: snapshot + delta model (gate G2)

### 6.1 Principle: no replay, no per-client buffers

The backend keeps **no per-subscriber cursor and no replay buffer** (§8
NG-5). A reloading webview reconstructs its view from persistence plus
newly arriving events, exactly as SSC-§5 prescribes ("the UI is a pure
function of persistence plus events") — the live view is no exception; it
merely needs a _bounded_ initial read instead of the whole session.

### 6.2 The handshake (extends FSC-L6, which stays in force)

On every window creation, reload, or re-subscription:

1. Register listeners first (FSC-L3), _then_ call `get_meeting_snapshot`.
2. The snapshot response for a live/processing meeting includes, next to
   the SSC fields (`state`, `state_revision`, job aggregates):
   - `transcript_head_sequence`: max persisted `sequence` for the meeting;
   - checklist totals (eligible-job count) for §6.12;
   - optionally the last level sample per source (so meters are not empty
     until the next E3 tick); this is an additive extension of the
     snapshot response and changes no SSC-defined field.
3. The transcript component fetches **one tail page** (the existing paged
   segment query of FSC-§2.2 row 4) ending at `transcript_head_sequence` —
   the visible window of plan §6.11, not the whole transcript. Older
   history loads on demand by scrolling, as the detail view already does.
4. Live events apply on top: E1 batches with `last_sequence` >
   `transcript_head_sequence` (D3 guard against the fetched page), E2/E3
   replace their aggregates/levels wholesale.

The snapshot is O(1) in session length: one aggregate query plus one max
lookup — no transcript scan, no replay of N hours of events.

### 6.3 Event-loss healing while subscribed

- **Sequence-gap rule.** If an applied E1 batch's `last_sequence` jumps
  (next expected ≤ received − 1 beyond the in-batch contiguity), the
  consumer schedules **one debounced tail-page refetch** (≥ 250 ms quiet
  period), not one per gap. The refetch re-anchors the cursor; deltas keep
  flowing meanwhile and are re-discarded by D3 where already covered.
- **Truncation rule.** An E1 event with `truncated: true` forces the same
  refetch immediately (§7.3).
- **Terminal rule.** On `succeeded`/`failed` terminal signals (FSC-§4.3)
  the consuming view refetches authoritative counts, so the §6.12 checklist
  ends truthful ("✓ 42 von 42") regardless of any dropped E2 event.
- **Revision gap rule.** Session-state gaps keep using SSC-O5 (snapshot
  refetch); live-view gaps never trigger a session snapshot and vice versa
  (D4).

Because every healing path is a _query_, not a replayed event, event loss
degrades latency-to-truth, never correctness.

---

## 7. Failure and degradation behavior

### 7.1 Publisher-side

- Publication happens after commit (D1); a crash between commit and publish
  loses only the notification (healed by §6.3), mirroring SSC-O1's crash
  argument verbatim.
- The coalescer must never block STT workers: enqueueing into the pending
  buffer is non-blocking; if the buffer is full the congestion rule of §7.3
  applies. Capture never waits on STT (plan line 248), and STT never waits
  on the UI.

### 7.2 Consumer-side

- Malformed payloads: record an ingest error, apply nothing (FSC-L7).
- Unknown `source` values: ignore the entry, keep the batch (forward
  compatibility with future sources).

### 7.3 Congestion rule (bounded memory, lossy by design)

If pending E1 segments exceed `LV_DELTA_PENDING_CAP` (1024), the publisher
drops the _oldest_ pending batches and sets `truncated: true` on the next
emitted event. Consumers respond per §6.3. This keeps both the manager's
memory and the webview's event load bounded under any input rate, at the
price of a refetch — the correct trade for a channel whose truth lives in
SQLite (§1.2).

---

## 8. Complexity budget / non-goals (gate G4)

In line with the company complexity budget this contract deliberately does
**not** specify:

- **NG-1 — No word-level live streaming.** No partial-hypothesis events, no
  streaming STT protocol, no WebSocket realtime channel (plan lines
  1130–1133 reserve that for an explicit future spike). Deltas carry
  finished segments only.
- **NG-2 — No per-segment "typing" progress.** No event per job start
  targeted at the transcript view; the `Abschnitt wird transkribiert`
  state is an aggregate derivation (§1.1).
- **NG-3 — No chunk-level live event.** "Audio gespeichert" and backlog
  badges come from aggregates/snapshots; `ChunkFinalized` stays internal to
  the manager (SSC-§2.3).
- **NG-4 — No live edit/delete stream.** Re-transcription and segment edits
  belong to the detail view's existing envelope/refetch machinery, not to
  the live channel.
- **NG-5 — No replay buffer, per-client cursors, or delivery
  acknowledgements.** Snapshot + tail page + D3 healing is strictly simpler
  and cannot drift from persistence.
- **NG-6 — No second level throttle.** The recorder's
  `LEVEL_EVENT_INTERVAL_MS` is the only level rate limiter; the manager
  forwards or suppresses, it never re-times.
- **NG-7 — No configurable cadence.** The §3.1 constants are fixed UI
  constants; making them user settings is rejected as scope bloat.
- **NG-8 — No adaptive/backpressure protocol between webview and backend.**
  The congestion rule (§7.3) is the entire flow-control story.
- **NG-9 — Topic names are proposals** until registered in
  `identifier-registry.md` (same posture as FSC-§0.2 item 3); registration
  is owned by that registry's maintainer task and is intentionally not
  performed in this document's scope.

Anything not in §2's three schemas is a non-goal until the plan changes.

---

## Appendix A — Gate traceability

| Gate                                                                      | Coverage                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 max event rate + coalescing window + computed worst-case backlog bound | §3.1 constants, §3.2 algorithm, §3.3 rate table with the 240-chunk ⇒ 8-events/≈1.75 s computation and the 128 seg/s ceiling vs. ≥64× headroom argument                                                                    |
| G2 reload/resubscribe without whole-session replay (snapshot + delta)     | §6.1 principle (no replay buffers), §6.2 four-step handshake with `transcript_head_sequence` + one tail page + O(1) snapshot argument, §6.3 gap/truncation/terminal healing                                               |
| G3 no contradiction with SSC §4; additive only                            | §1.2 additivity statement (no O-rule restated, no existing payload changed, no new authority); §4 D1–D3 explicitly mirror O1–O3, D4–D8 additive; D4 pins cross-stream independence so neither guard can corrupt the other |
| G4 non-goals + documentation index updated                                | §8 NG-1–NG-9; row appended to `docs/meeting-mode-documentation-index.md` §3 table per its Pflegeregel                                                                                                                     |
| G5 git status confined to declared scope                                  | verified after authoring via `git status --porcelain`; recorded in the mission result evidence                                                                                                                            |
