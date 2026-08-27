# Dictation ↔ Meeting Coexistence & Resource Arbitration Contract

Status: **spec contract — no implementation yet** (Phase 4/5 prerequisite;
changes no source code).
Source of truth: `docs/meeting-mode-plan.md`, sections **§6.10** ("Gleichzeitige
Nutzung beider Werkzeuge"), **§6** ("Startvalidierung"), **§6.9**
("Shortcut-Konzept für beide Werkzeuge"), **§8** ("Ressourcenlogik zwischen
Meeting und Diktat", "Lokale Verarbeitung"), **§13** Phase 4/5 acceptance rows,
and **§14** test-matrix row "Diktat wird während lokaler Meetingverarbeitung
ausgelöst".
Sibling contracts this document builds on:
[recorder-contract.md](recorder-contract.md) (capture → chunks → events) and
[stt-provider-contract.md](stt-provider-contract.md) (queue ↔ providers). This
document defines the seams _between_ those contracts and the untouched dictation
path.

Planned implementation homes (interface notes only, see §8):
`src-tauri/src/managers/transcription.rs`,
`src-tauri/src/managers/audio.rs`, `src-tauri/src/settings/schema.rs`,
`src-tauri/src/shortcut/`. **This task makes zero edits to any source file.**

---

## 1. Scope and ownership

This contract fixes exactly four concurrency/configuration seams:

| #   | Seam                                                                                              | Owner after implementation                                    | Section |
| --- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ------- |
| S1  | Engine-lease extension (meeting-STT worker vs. interactive dictation on the single shared engine) | `TranscriptionManager` (extended)                             | §3      |
| S2  | Microphone tap (dictation reusing the meeting's open mic stream) + single-device-open invariant   | `AudioRecordingManager` + `MeetingRecorder` tap handle        | §4      |
| S3  | Mutual exclusion of simultaneous start attempts                                                   | `MeetingManager` (decider) consulting `AudioRecordingManager` | §5      |
| S4  | Settings separation + per-session snapshot isolation                                              | `settings::schema` + `MeetingManager`                         | §6      |
| S5  | Shortcut namespace + availability pre-check                                                       | `shortcut` module                                             | §7      |

Explicitly **not** owned here (and therefore non-goals for any implementer of
this contract):

- Session/job persistence semantics → `stt-provider-contract.md`.
- Chunk writing, timeline, recovery → `recorder-contract.md` and
  `meeting-mode-phase2-contracts.md`.
- Any change to the dictation product behavior, latency profile, or the Azure/
  Custom short-audio paths (`speech_pipeline.rs` is untouched by this contract).
- Diarization, per-process loopback, a second local engine instance (plan §2,
  §3: exactly one engine, no competing model instances — "genau ein
  Engine-Lease").

Normative language: **must** = gate; **should** = default with documented
exception path; **may** = permitted option.

---

## 2. Baseline: the existing engine protocol (verified code, HEAD)

All statements in this section describe _current_ dictation-path code and are
cited so the extension in §3 can be reviewed against reality.

`src-tauri/src/managers/transcription.rs` (HEAD, read in full for this task):

1. **Single engine slot.** `TranscriptionManager.engine:
Arc<Mutex<Option<LoadedEngine>>>` (line ~249) holds at most one loaded model.
   Batch runs `take()` the engine out of the mutex under `lock_engine()`
   (line ~1245-1260) and put it back via `return_engine()` (line ~1087), which
   **drops a stale engine** if `current_model_id` changed mid-run.
2. **Lease token.** `active_engine_lease: Arc<AtomicU64>` (line ~279) is nonzero
   while a streaming worker has taken the engine out; `is_model_loaded()`
   reports `true` while leased (line ~384-388). Worker IDs come from
   `next_stream_worker_id` and CAS `0 → worker_id` to acquire
   (line ~850-859); `StreamWorkerGuard::drop` releases via CAS back to `0`
   (line ~227-245). This is the existing "engine-lease protocol" this contract
   extends.
3. **Load mutual exclusion.** `try_start_loading()` + `LoadingGuard`
   (line ~195-214, ~401-411) guarantee at most one model load; consumers wait on
   `loading_condvar` (batch: line ~1193-1196; stream worker: line ~837-842).
4. **Idle unload.** A watcher thread unloads the model after
   `model_unload_timeout` of inactivity, but refreshes the timer while the
   recorder reports `is_recording()` (line ~309-366);
   `maybe_unload_immediately()` handles `ModelUnloadTimeout::Immediately` after
   each run (line ~461-471).
5. **Non-blocking frame route.** `StreamRouter.feed()` (line ~164-171) is the
   precedent that a capture-side callback costs one relaxed atomic load plus an
   mpsc send and never blocks on model compute. This is the structural argument
   reused in §3.6.

`src-tauri/src/audio_toolkit/audio/recorder.rs`: `AudioRecorder` owns the cpal
`Device` and stream inside a dedicated worker thread fed by an `mpsc` `Cmd`
channel (lines ~22-29, ~73-91); 16 kHz mono frames reach the router through the
optional `audio_cb: AudioFrameCallback` (line ~79, type at line ~71).

`src-tauri/src/managers/audio.rs`: `AudioRecordingManager` resolves the device
from `settings.selected_microphone` with an explicit
`unavailable_selected_microphone` fallback (lines ~441-535), supports an
always-on stream (`start_microphone_stream`/`stop_microphone_stream`,
lines ~627/739), and gates starts through `try_start_recording`
(line ~806) with generation-token readiness (`RecordingReadiness`, lines ~332-346).

---

## 3. S1 — Engine-lease extension (Gate G1)

### 3.1 Participants

Exactly two lease clients exist:

| Client                                                                      | Priority class | Typical hold                                              |
| --------------------------------------------------------------------------- | -------------- | --------------------------------------------------------- |
| Dictation (batch `transcribe()` or streaming worker)                        | `Interactive`  | < a few seconds (batch), seconds-to-minutes (live stream) |
| `LocalMeetingProvider` worker (one per app, per stt-provider-contract §4.1) | `Background`   | one VAD segment per acquisition                           |

There is **no third client** and **no second engine instance**, ever (plan §3
item 3). Cloud meeting jobs (Azure/Custom) never touch this lease
(stt-provider-contract §4.2/§4.3) — they proceed independently while local work
pauses, per plan §6.10.

### 3.2 Lease states

```text
                 acquire(class)
    ┌──────────┐ ─────────────► │Held{class}│
    │  Idle    │                └────┬──────┘
    └──────────┘     Interactive acquires while Background holds
                          │  (only at a segment boundary, §3.4)
                          ▼
                   │BgPauseRequested│  (worker finishes current segment,
                          │           persists result, releases engine)
                          ▼
                     │BgPaused│ ── Interactive acquires ──► dictation runs
                          ▲                                       │
                          │  worker re-acquires automatically      │ engine
                          │  (auto-resume, §3.5)                   │ released
                          └────────────────────────────────────────┘
```

Rules:

- At most one client holds the engine at any time (invariant inherited from the
  existing CAS protocol, §2 item 2).
- `Interactive` acquisition while `Background` holds **never aborts, splits, or
  restarts** the running meeting-job inference. It sets `BgPauseRequested` and
  waits for the boundary (§3.4).
- `Background` acquisition never displaces `Interactive`: if dictation holds
  the engine, the meeting worker queues its next segment and retries.

### 3.3 Acquire semantics

The extension replaces the bare `u64` CAS with a small ticket carrying the
priority class. Normative behavior:

1. **Fast path, uncontended:** identical to today — CAS `0 → ticket`, take
   engine from the mutex, run.
2. **Contended, Interactive requester:** set `BgPauseRequested`, then block on
   the existing lease-release notification (the Condvar/notify point where
   `StreamWorkerGuard::drop` / `return_engine` release). No polling loops.
3. **Contended, Background requester:** register as waiting-background and
   block on the same release notification; on wake, re-CAS. Interactive
   requesters always win ties.
4. **Model identity:** if the requesting client needs a different model than
   currently loaded, the switch goes through the existing
   `load_model`/`try_start_loading` machinery (§2 item 3) — never two models
   resident in parallel (plan §8: "darf das System nicht beide ungeprüft
   parallel laden"). Model switches are counted against the acquiring side's
   budget, not the releasing side's.
5. **Fairness/starvation:** a `Background` waiter that lost ≥ N consecutive
   races (N = 8, testable constant) escalates to a logged warning event
   `meeting_backlog_starved`; the UI shows a growing backlog count (plan §8:
   "Die UI zeigt einen wachsenden Backlog") — it does **not** change priority.

### 3.4 Pause-at-segment-boundary semantics

A "safe segment boundary" is defined by the stt-provider-contract segment model:
the current VAD segment's audio is fully ingested by the engine call AND its
result is durably persisted (segment row committed to SQLite). Only then may the
worker honor `BgPauseRequested`.

- The worker checks `BgPauseRequested` **between** engine calls, never inside
  one. An in-flight `session.run(...)`/`transcribe_with(...)` always runs to
  completion.
- Bound on wait: segments are bounded by the plan §8 segmentation window
  (initial 5–30 s). Therefore the worst-case time from
  `BgPauseRequested` to engine release is one segment duration. The constant

  ```rust
  /// Worst-case wait for a Background lease holder to reach a segment boundary
  /// and release. Derived from plan §8 max segment length (30 s) plus persist
  /// headroom; a violation is a bug, surfaced as telemetry.
  pub const MEETING_LEASE_PAUSE_DEADLINE: Duration = Duration::from_secs(60);
  ```

  is asserted in tests with a mocked engine.

- If the deadline passes (pathological slow inference), the Interactive
  requester **does not preempt and does not double-load**: it surfaces a
  recoverable error to the dictation UI ("Spracherkennung durch Meeting
  ausgelastet — erneut versuchen") and cancels its acquisition. Rationale:
  killing mid-inference risks the engine state and violates the
  never-interrupt rule; loading a second model violates the one-engine rule.
  This error path must be reachable in tests via a stalled fake engine.

### 3.5 Resume (auto-resume) semantics

After the dictation client releases (its `return_engine` /
`StreamWorkerGuard::drop` equivalent), the background worker:

1. Re-acquires (rule §3.3-3).
2. If the loaded model differs from the session's snapshotted
   `meeting_model_id`, triggers exactly one model switch through the shared
   loader (this switch is _allowed_ to evict the dictation model — dictation is
   idle by definition at this point).
3. Resumes draining its persistent queue oldest-first; no queued segment is
   dropped or reordered by the pause (queue semantics owned by
   stt-provider-contract §3).
4. Refreshes the idle-unload timer (`touch_activity`) while it holds or waits
   for the lease, so the idle watcher (§2 item 4) never unloads the engine out
   from under an active meeting queue. Conversely, the lease holder must be
   reported by `is_model_loaded()` (already true via `active_engine_lease`,
   §2 item 2).

### 3.6 Gate G1 proof obligation: capture threads never wait on inference

The claim "Aufnahme-Threads warten niemals auf Inferenz" (plan §8) holds
structurally because the capture path and the lease share **no synchronization
object**:

| Path                                           | Objects touched                                                           | Blocking primitives on path                                                                            |
| ---------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Meeting capture callbacks → rolling WAV chunks | recorder-internal channels, disk writer thread                            | none on the callback (bounded channel send; recorder-contract §1 makes the recorder the single writer) |
| Dictation capture callback → frames            | `StreamRouter`-style atomic + mpsc send (§2 item 5) or tap fan-out (§4.3) | none; a slow consumer drops/lags, it never back-pressures the device callback                          |
| Lease acquire/pause/resume                     | lease ticket, engine mutex, loading Condvar                               | confined to the two **worker threads** (dictation runner, meeting-STT worker)                          |

Every blocking primitive in the lease extension lives on worker threads that
consume already-persisted chunks/segments. The device callback chain reaches
neither the engine mutex nor the loading Condvar. Verification requirements
(Phase 5 acceptance):

- A unit test drives the meeting worker against a mock engine whose inference
  blocks indefinitely, and proves the (mocked) capture producer keeps making
  progress (chunks keep being finalized) — i.e., pause-request propagation
  never crosses into capture.
- A stress test interleaves N simulated dictations during continuous meeting
  processing and asserts every dictation completes within its deadline bound
  and every meeting segment eventually completes (no loss, no reorder).

### 3.7 Timeout summary table

| Timer                          | Value (initial, testable constant)                                                   | On expiry                                                                                                                                                                       |
| ------------------------------ | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MEETING_LEASE_PAUSE_DEADLINE` | 60 s (§3.4)                                                                          | Dictation acquisition fails with recoverable busy error; meeting job unaffected                                                                                                 |
| Segment inference watchdog     | 3 × real-time factor budget of the segment (constant, benchmark-derived per plan §8) | Job marked `failed` per stt-provider-contract §3 retry rules; lease released via guard drop even on panic (pattern of §2 items 1–2: panicked engines are dropped, not returned) |
| Idle-model unload grace        | existing `model_unload_timeout`, refreshed by both clients' activity                 | unchanged behavior                                                                                                                                                              |

---

## 4. S2 — Microphone tap and the single-device-open invariant (Gate G2)

### 4.1 Invariant (exact wording)

> **At most one cpal input stream per physical microphone device exists
> process-wide at any instant.** Every component that needs that device's audio
> either owns the only stream or consumes a tap from its owner. Violations are
> defects regardless of what the OS would tolerate: WASAPI shared mode permits
> multiple captures, but (a) Bluetooth headsets switch to low-quality HFP
> profiles under some dual-open patterns, (b) exclusive-mode consumers elsewhere
> fail unpredictably, and (c) plan §6.10 mandates the tap ("zeitlich begrenzten
> Tap des vorhandenen Meeting-Mikrofonstreams, statt dasselbe Gerät ein zweites
> Mal zu öffnen").

The invariant covers three potential openers: the meeting recorder's mic
source, the dictation `AudioRecorder`, and the always-on dictation stream
(`start_microphone_stream`, §2). The system-audio loopback stream is a
_render-device_ capture and is outside this invariant.

### 4.2 Ownership registry

`AudioRecordingManager` gains the role of arbiter: a small registry mapping
device identity → current owner (`Meeting` | `Dictation` | `AlwaysOn`),
consulted by every open path. Opening a second stream for an owned device is a
programming error (assert in debug builds, logged fallback in release).

### 4.3 Tap mechanism

`MeetingRecorder` exposes a tap handle alongside its chunk-writing sink
(recorder-contract §1 keeps it the single _writer_; a tap is a reader, which
does not violate that role):

```rust
/// Fan-out of the live 16 kHz mono timeline frames of the meeting microphone
/// source. Subscribing costs one channel send per frame batch on the recorder's
/// writer thread — never on the device callback.
pub struct MicTapHandle { /* ... */ }

impl MicTapHandle {
    /// Frame-sink contract: receives 16 kHz mono f32 frames already resampled
    /// onto the meeting timeline (recorder-contract §2.2). Delivery is
    /// best-effort: a lagging subscriber's backlog is dropped at batch
    /// granularity and the subscriber is notified, mirroring how StreamRouter
    /// isolates capture from compute.
    pub fn subscribe(&self, sink: DictationFrameSink) -> TapSubscription;
}
```

Dictation integration: `AudioRecorder` gains a _frame-source_ abstraction with
two producers behind the identical `AudioFrameCallback` consumer surface it
already has (§2): `DeviceSource` (today's cpal path, unchanged) and
`TapSource(MicTapHandle)`. Everything downstream — VAD policy, level meter,
streaming feed, stop→`Vec<f32>` collection — behaves identically for both,
because both deliver the same 16 kHz mono frame stream. Dictation applies its
own `VadPolicy` on tapped frames exactly as on device frames.

### 4.4 Case analysis (normative)

| Case                  | Meeting state                                                             | Dictation mic path                                                                                                                                       | Device opens                                     |
| --------------------- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **C1 meeting-active** | `recording`                                                               | `TapSource` subscribed for the dictation's lifetime                                                                                                      | **0 additional** (invariant trivially preserved) |
| **C2 meeting-paused** | `paused` (mic stream closed per pause semantics, recorder-contract/pause) | Tap unavailable → fallback: dictation uses its normal `DeviceSource` (plan §6.10: "Ist das Meeting pausiert, verwendet Diktat seinen normalen Recorder") | **exactly 1** (dictation's own)                  |
| **C3 no-meeting**     | absent                                                                    | normal `DeviceSource` (unchanged legacy behavior)                                                                                                        | **exactly 1**                                    |

Additional rules:

- **R1 — resume-vs-fallback migration.** If the meeting resumes (C2 → C1)
  while a fallback dictation is still recording, the meeting's mic source may
  not open while the dictation holds the device, yet the meeting must not stay
  paused because of a dictation (plan §6.10: "ein Diktat pausiert oder beendet
  die Meetingaufnahme nicht"). Resolution: at resume, the meeting first reopens
  **system audio** unconditionally; for the microphone it requests the device.
  The arbiter then migrates the dictation from `DeviceSource` to `TapSource` at
  a frame boundary (both producers emit the identical frame format, so the
  switchover is seamless for VAD/streaming). If migration fails, dictation wins
  the device and the meeting mic source enters the same reconnect/warn state
  defined for a failed microphone in plan §6 "Fehlerverhalten" (system audio
  continues, warning shown) — recorded as a new source interval when it later
  reopens.
- **R2 — no mid-dictation surprise downgrades.** A dictation that started as a
  tap (C1) and observes the meeting go to `paused` keeps consuming buffered tap
  frames until the tap closes; if the dictation outlives the pause, it
  transitions to `DeviceSource` at a frame boundary (mirror of R1). The
  transition points are exactly two and both are frame-boundary atomic swaps of
  the frame-source enum.
- **R3 — always-on stream.** Before a meeting opens the microphone, the
  always-on dictation stream (if enabled) must be stopped and is restarted
  after the meeting's mic source closes (or converted to a tap while the
  meeting records, if implemented that way — implementer's choice, invariant is
  what is normative).
- **R4 — device mismatch.** The tap carries the meeting's mic device. If the
  dictation setting names a _different_ microphone than the meeting's, the tap
  is not applicable: dictation opens its own device (which is a different
  physical device, so the invariant is untouched). Same-device comparison uses
  the same device-identity string the arbiter registers.
- **R5 — tap lifetime bound.** A tap exists only for the duration of one
  dictation session ("zeitlich begrenzter Tap", plan §6.10); no permanent
  subscriptions, no taps opened by anything except a live dictation.

### 4.5 Failure handling

- Tap subscription fails (meeting stops between check and subscribe): dictation
  falls back to `DeviceSource` once, via the arbiter; if the arbiter refuses
  (owner changed), the dictation start fails with the standard
  device-unavailable UX, identical to today's missing-microphone path
  (`unavailable_selected_microphone` handling, §2).
- Device unplugged mid-dictation: existing per-stream `stream_error` handling
  (§2, recorder.rs field) applies to whichever producer is active; the meeting
  side follows plan §6 "Mikrofon fällt aus".

---

## 5. S3 — Mutual exclusion of simultaneous start attempts (Gate G2 co-rule)

### 5.1 Decision matrix

| Situation                                                | Behavior                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Dictation recording, user starts meeting                 | **No automatic action.** `MeetingManager.start()` detects `is_recording()` (via `AudioRecordingManager`, §2) and returns a `RequiresUserDecision` outcome; the frontend shows the plan §6.10 choice "`Diktat abschließen`" / "`Meetingstart abbrechen`". The backend never force-cancels a dictation (plan §6.2 rule 6, §6.10). Choosing "abschließen" means: dictation finalizes normally, then meeting start is retried and revalidated. |
| Meeting recording, user starts dictation                 | Always allowed; routed through the tap (§4.4 C1). No prompt, no countdown interference: the meeting countdown overlay and the dictation overlay are independent states.                                                                                                                                                                                                                                                                    |
| Meeting `starting` (countdown running), dictation starts | Allowed; dictation uses `DeviceSource` if the meeting hasn't opened the mic yet, else tap — decided by the arbiter at dictation start, not pre-declared.                                                                                                                                                                                                                                                                                   |
| Meeting recording, user starts meeting (again)           | Single-active-meeting invariant (MeetingManager state machine); second start rejected with the standard busy message. Out of scope beyond this sentence.                                                                                                                                                                                                                                                                                   |

### 5.2 Race discipline

Both start paths converge on the same two state machines. Rules:

1. All start attempts serialize on the `MeetingManager` transition lock; there
   is exactly one place that flips `draft → starting`.
2. The `is_recording()` check and the meeting's mic open happen under that
   serialization, so the matrix above is evaluated exactly once per attempt.
   Between the user decision and the retry, conditions may have changed; the
   retry re-runs the full Startvalidierung (plan §6) — decisions are never
   cached.
3. If a dictation stops _by itself_ while the decision dialog is open, the
   dialog self-dismisses and the pending start proceeds through normal
   validation (no stale "conflict" state).
4. The dictation side needs no new lock: `try_start_recording` already
   generation-guards concurrent starts (§2), and it is never blocked by
   meeting state (worst case it takes the tap).

---

## 6. S4 — Settings separation and snapshot isolation (Gate G3)

### 6.1 New setting keys

Added to `AppSettings` (`src-tauri/src/settings/schema.rs`) following the
existing conventions (field-level `#[serde(default = …)]` next to the struct +
entry in `get_default_settings()`; container-level `#[serde(default)]` already
guarantees forward-compatible loads per schema.rs lines ~14-20, so no schema-
version bump is strictly required for additions — but the version increments if
any migration logic accompanies them):

| Key                              | Type                                                                      | Default                                                             | Purpose                                                                                                                                                                   |
| -------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `meeting_selected_microphone`    | `Option<String>`                                                          | `None`                                                              | Meeting mic device. `None` = fall back to the dictation `selected_microphone` resolution at meeting start (displayed as such in the UI), never a silent alias at runtime. |
| `meeting_selected_output_device` | `Option<String>`                                                          | `None`                                                              | Loopback **capture** target (WASAPI loopback of this render endpoint, plan §7). `None` = system default output at meeting start.                                          |
| `meeting_follow_default_output`  | `bool`                                                                    | `false`                                                             | Opt-in "Standardgerät folgen" (plan §6 Fehlerverhalten); when false the snapshotted device stays binding.                                                                 |
| `meeting_stt_provider`           | `MeetingSttProvider` enum `{Local, Azure, Custom}`                        | `Local`                                                             | Per-default provider; each meeting may override (snapshot, §6.3).                                                                                                         |
| `meeting_model_id`               | `String`                                                                  | `""` (empty = unset → Startvalidierung prompts)                     | Local meeting model, independent of dictation `selected_model` (plan §8).                                                                                                 |
| `meeting_language`               | `String`                                                                  | `"auto"` (mirrors `default_selected_language`, schema.rs line ~229) | Independent language intent for meetings; coerced per model exactly like `selected_language` (capability-aware, never written back).                                      |
| `meeting_shortcut_binding`       | carried in the existing `bindings` map under id `"meeting_toggle"` (§7.1) | platform candidate (§7.2)                                           | Meeting start/stop shortcut; no new settings field, reuse of the `HashMap<String, ShortcutBinding>` mechanism.                                                            |

Deliberately **not** introduced by this contract (belongs to other contracts /
phases, listed to prevent drift): summary-provider keys (summary contract),
retention keys beyond the per-session policy snapshot (phase2/data contract),
overlay/tray keys (UX phase). Any future meeting key must be added to this
table or to the owning contract — free-floating keys are forbidden.

### 6.2 `selected_output_device` preservation (hard rule)

- `selected_output_device` **keeps its sole meaning: the playback device for
  feedback sounds.** Verified current usage: `src-tauri/src/audio_feedback.rs`
  line ~93 reads it for sound playback; `commands/audio.rs`
  `set_selected_output_device`/`get_selected_output_device` (lines ~271-289)
  expose exactly that; nothing else reads it (workspace grep, 10 hits total,
  all listed).
- Meeting loopback capture **must only ever read
  `meeting_selected_output_device`** (and its session snapshot). Any import of
  `selected_output_device` into the meeting capture path is a contract
  violation.
- Required regression evidence when implemented: a settings test asserting
  that changing `selected_output_device` leaves the effective meeting loopback
  device unchanged, and vice versa (mirrors existing
  `settings/tests.rs` style, e.g. the partial-store test around line ~79).

### 6.3 Per-session snapshot isolation

At successful meeting start, `MeetingManager` writes an immutable snapshot into
the session record (fields already reserved in plan §9 `meeting_sessions`:
`stt_provider` + snapshot, `meeting_model_id`, `language`, devices,
retention policy):

- Snapshot contents: provider enum, non-secret provider configuration,
  `meeting_model_id`, resolved effective language, resolved mic/output device
  identities, `has_microphone_audio`/`has_system_audio`, retention policy.
- Secrets are never copied; only references through the existing secret store
  (plan §9: "Secrets werden ausschließlich über die vorhandene Secret-
  Verwaltung referenziert").
- After start, changes to global settings (including "Für nächstes Meeting
  ändern", plan §6.11) affect the **next** meeting only. The running session
  reads provider/model/device parameters exclusively from its snapshot; the
  local worker's lease re-acquisition (§3.5 step 2) compares against the
  snapshot, not live settings.
- The dictation side keeps its existing live-settings behavior unchanged
  (dictation reads `get_settings()` per run today; no snapshot is introduced
  for it).
- Shortcut bindings: `MeetingManager` captures the meeting binding ids at
  session start (§7.5), so a mid-meeting rebinding cannot alter countdown/stop
  semantics of the running session.

### 6.4 Independence test matrix (required when implemented)

| Change during recording          | Effect on running meeting                               | Effect on dictation                        |
| -------------------------------- | ------------------------------------------------------- | ------------------------------------------ |
| `selected_model`                 | none (snapshot)                                         | next dictation uses it (existing behavior) |
| `meeting_model_id`               | none (snapshot); applied at next start                  | none                                       |
| `selected_output_device`         | none — feedback sounds move (intended legacy semantics) | feedback sounds move                       |
| `meeting_selected_output_device` | none (snapshot)                                         | none                                       |
| `meeting_toggle` binding         | none (session-start snapshot, §7.5)                     | n/a                                        |

---

## 7. S5 — Shortcut namespace and availability pre-check (Gate G4)

### 7.1 Namespace rules

1. All meeting binding ids live in the existing flat `bindings:
HashMap<String, ShortcutBinding>` and **must** carry the reserved prefix
   `meeting_`. Initial ids: `meeting_toggle` (start/stop). `meeting_pause` is
   reserved for the opt-in power-user binding (plan §6.9) and may remain
   unassigned by default.
2. Reserved prefixes/names may never collide with existing ids
   (`transcribe`, `transcribe_with_post_process`, `cancel` — verified in
   `settings/schema.rs` `get_default_settings()` lines ~561-601 and
   `shortcut/handler.rs` `ACTION_MAP` usage).
3. The `cancel` binding (default `escape`, present in the bindings map and
   additionally registered dynamically per recording via
   `register_cancel_shortcut`, `shortcut/mod.rs` lines ~59-78) participates in
   conflict checks like any other binding: an identical candidate is blocked
   even though its OS registration is only resident while a recording is
   active.

### 7.2 Candidate and platform defaults

Recommended candidate per plan §6.6/§6.9: **`Ctrl+Alt+Space`** on Windows
(dictation keeps `Ctrl+Space`, AI post-processing `Ctrl+Shift+Space`). Platform
analogues follow the existing `get_default_settings()` pattern
(schema.rs lines ~551-600: mac uses `option+…`):

| Platform | Candidate          | Known-risk neighbors checked by the pre-check            |
| -------- | ------------------ | -------------------------------------------------------- |
| Windows  | `ctrl+alt+space`   | IME/language switchers on some layouts                   |
| macOS    | `option+cmd+space` | Spotlight family (`cmd+space`) — avoided by construction |
| Linux    | `ctrl+alt+space`   | desktop-environment workspace shortcuts                  |

The candidate is a _recommendation_, never silently forced: it is only offered
if it survives §7.3, otherwise conflict-free alternatives from an ordered
candidate list are offered (plan §6.6: "andernfalls werden konfliktfrei
registrierbare Alternativen angeboten").

### 7.3 Availability pre-registration check (mandatory order)

Before any UI offers `Ctrl+Alt+Space` (onboarding step 3, shortcut editor
suggestion), the following check runs, in order:

1. **In-app conflict scan.** Normalize the candidate with the same parser used
   by `validate_shortcut` (present in both `tauri_impl.rs` line ~44 and
   `handy_keys.rs` line ~413) and compare against every Speak binding,
   including the virtual `cancel` binding. Identical normalized string ⇒
   conflict (see §7.4), candidate not offered.
2. **OS-level probe.** Register the candidate through the _active_ keyboard
   implementation (`KeyboardImplementation::Tauri` →
   `app.global_shortcut().register`, which already rejects already-registered
   combos — tauri_impl.rs line ~97; `HandyKeys` → its manager register path),
   then immediately unregister. Success ⇒ the combo is genuinely available on
   this device; failure ⇒ not offered.
3. **Transient-state hygiene.** The probe must run while no shortcut recording
   session is active (`suspend_all_shortcuts`/`resume_all_shortcuts`,
   mod.rs lines ~235-265, must not be mid-cycle), and must be failure-safe:
   any probe error unregisters best-effort and reports "unavailable" (fail
   closed — an unregistrable shortcut is never presented as offerable, plan
   §6.9: "ein nicht registrierbarer Shortcut bleibt nicht scheinbar erfolgreich
   gespeichert").

Only candidates passing both steps are displayed. Final registration happens
at user confirmation through the existing `change_binding` flow, which already
implements save-time verification: unregister old → register new → on failure
`restore_registration` of the previous binding (mod.rs lines ~113-224).

### 7.4 Identical-binding blocking with both conflict sites

When a user attempts to save a binding identical (after normalization) to an
existing Speak binding:

- The save is **blocked** (pre-save, not post-hoc), matching plan §6.9
  "identische Speak-Bindings werden blockiert und beide Konfliktstellen
  angezeigt".
- The error result must identify **both** sites: (a) the existing action —
  human-readable action name plus where it lives (e.g. "Diktat starten/
  stoppen — Einstellungen → Kurzbefehle"; tray and overlay usages noted), and
  (b) the attempted meeting action entry. Machine shape: a typed
  `{ existing_binding_id, existing_action_label_key, attempted_binding_id }`
  response the frontend renders via i18n (no hardcoded strings, AGENTS i18n
  rule).
- Subset/superset modifier combinations are **not** conflicts: `Ctrl+Space`
  and `Ctrl+Alt+Space` can coexist at the OS layer and are allowed. Only
  normalized-equal combos block.
- Conflict scanning includes bindings that are currently disabled (e.g.
  post-processing shortcut when post-processing is off) — disabling hides a
  binding, it does not vacate it.

### 7.5 Mid-meeting stability

Shortcut changes (including `meeting_toggle` itself) made while a meeting is
running take effect only after that meeting ends (plan §6.9). Implementation
note: `MeetingManager` snapshots the relevant binding ids at session start
(§6.3 last bullet); the shortcut module remains free to rebind globally, but
the running session's stop/countdown handlers resolve through the snapshot.
Edge rule: if the _old_ meeting binding is deleted mid-meeting, the running
meeting's stop remains reachable via UI and tray (which are always available),
and the new binding activates for the next session.

---

## 8. Touch-point inventory (Gate G5) — interface notes only

**Zero edits were made to any of these files in this task.** Every row is
future-phase work; the "Seam" column links the change to this contract.

| File (exists at HEAD)                                         | Existing interface involved (verified)                                                                                                           | Future change (phase)                                                                                                  | Seam   |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- | ------ |
| `src-tauri/src/managers/transcription.rs`                     | `active_engine_lease` CAS protocol; `try_start_loading`/`LoadingGuard`; `return_engine` staleness drop; idle watcher activity refresh            | Add priority-class lease tickets + `BgPauseRequested`/auto-resume; export lease status for diagnostics (Phase 5)       | S1     |
| `src-tauri/src/managers/audio.rs`                             | `try_start_recording`, `start_/stop_microphone_stream`, device resolution w/ `unavailable_selected_microphone`, `RecordingReadiness` generations | Owner registry for the single-device-open invariant; tap-aware dictation start; always-on suspension hook (Phases 4–5) | S2, S3 |
| `src-tauri/src/audio_toolkit/audio/recorder.rs`               | `AudioFrameCallback`, `Cmd` worker, `VadPolicy`, `config_cache`                                                                                  | `DeviceSource`/`TapSource` frame-source abstraction behind the unchanged callback surface (Phase 5)                    | S2     |
| `src-tauri/src/managers/meeting.rs` _(planned new, plan §5)_  | —                                                                                                                                                | `MicTapHandle` exposure lives in `meeting_recorder.rs` (planned new, plan §5); `MeetingManager` consumes it (Phase 4)  | S2     |
| `src-tauri/src/settings/schema.rs` (+ `types.rs`, `tests.rs`) | `AppSettings` serde-default pattern; container `#[serde(default)]` partial-store tolerance; `get_default_settings()`                             | Add §6.1 keys + `MeetingSttProvider` type + independence tests (Phase 4)                                               | S4     |
| `src-tauri/src/commands/audio.rs`                             | `set_/get_selected_output_device`                                                                                                                | Parallel meeting-device getters/setters operating **only** on the new keys (Phase 4)                                   | S4     |
| `src-tauri/src/shortcut/mod.rs`                               | `change_binding` + `restore_registration`, `suspend_all_shortcuts`/`resume_all_shortcuts`, implementation dispatch                               | Meeting binding init; conflict-check command returning both conflict sites (Phase 4)                                   | S5     |
| `src-tauri/src/shortcut/handler.rs`                           | `handle_shortcut_event`, `ACTION_MAP`                                                                                                            | Route `meeting_*` ids to `MeetingManager` actions (Phase 4)                                                            | S5     |
| `src-tauri/src/shortcut/tauri_impl.rs`, `handy_keys.rs`       | `validate_shortcut`, register-with-duplicate-rejection (tauri_impl.rs line ~97), `start_recording` suspend behavior                              | Availability probe primitive per implementation (Phase 4)                                                              | S5     |
| `src-tauri/src/lib.rs`                                        | manager construction/state registration                                                                                                          | Wire `MeetingManager`, pass tap/arbitration handles (Phase 4)                                                          | S2, S3 |
| `src-tauri/src/audio_feedback.rs`                             | reads `selected_output_device` (line ~93)                                                                                                        | **None.** Listed as the preserved-semantics anchor; must not gain any meeting branch                                   | S4     |
| `src-tauri/src/speech_pipeline.rs`                            | Azure short-audio dictation path                                                                                                                 | **None.** Untouched by this contract                                                                                   | —      |

Verification of this inventory: each "existing interface" cell was read at
HEAD in this worktree (files listed in §2 and the greps cited inline); no
source file under `src-tauri/src` or `src/` was modified — the only file
created/changed by this task is this document plus the mandated index row in
`docs/meeting-mode-documentation-index.md` (see header note below).

---

## 9. Complexity budget & non-goals (binding for implementers)

To keep the coexistence layer simple, these rejections are part of the
contract:

- **No second engine instance, no engine pool, no VRAM co-residency.** One
  engine, one lease, two priority classes (plan §3, §8).
- **No generic "resource scheduler" framework.** Two clients with a fixed
  priority relation do not need one; adding a third client requires revisiting
  this contract first.
- **No OS-level device virtualization / no audio driver tricks** for the tap —
  it is an in-process fan-out.
- **No repurposing of `selected_output_device`**, no hidden fallbacks from
  meeting settings to dictation settings at runtime (fallback happens once, at
  snapshot time, visibly).
- **No silent preemption in either direction.** Dictation never kills a meeting
  segment; meetings never delay a dictation beyond the §3.4 deadline; both
  deadlines surface honest errors instead.
- **No new global mutable state** beyond the owner registry (§4.2) and lease
  ticket — both owned by existing managers.

## 10. Gate coverage

| Gate                                                                                                     | Where satisfied                                                                                                 |
| -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| G1 acquire/pause-at-segment/resume/timeout + capture-never-waits-on-inference proof                      | §3.2–§3.7 (proof structure §3.6, tests mandated there)                                                          |
| G2 mic-tap three cases + exact single-device-open invariant                                              | §4.1 (exact invariant), §4.3 (mechanism), §4.4 (C1/C2/C3 table), §5 (mutual exclusion co-rule)                  |
| G3 settings keys + `selected_output_device` preservation per plan §6 Startvalidierung                    | §6.1 (key enumeration), §6.2 (preservation with code citations + required test), §6.3–§6.4 (snapshot isolation) |
| G4 shortcut availability pre-registration check + identical-binding blocking showing both conflict sites | §7.3 (ordered pre-check incl. OS probe), §7.4 (blocking + both sites), §7.1/§7.2 (namespace, candidate)         |
| G5 touch-point inventory, future-phase marked, zero edits                                                | §8 (every row marked; header states zero source edits; index-table update disclosed)                            |
