# Device Change During Active Meeting Sessions — Behavior Delta

Status: **spec contract — strict additive delta, no implementation yet** (Phase 2/4
prerequisite; changes no source code).
Base contracts (unchanged, authority over this document wherever text overlaps):
[recorder-contract.md](recorder-contract.md) (**RC**) — sections 5–7 — and
[coexistence-contract.md](coexistence-contract.md) (**CC**) — sections 4 (S2) and
6 (S4).
Source-of-truth anchors in `docs/meeting-mode-plan.md`: **§6.x** "Fehlerverhalten"
(lines 943–951: "Mikrofon fällt aus", "Systemgerät fällt aus",
"Standardgerät wechselt"), "Pause" (lines 929–934: "ein Gerätewechsel wird als
neuer Quellenabschnitt dokumentiert"), **§6.11** (lines 635–637: "Provider,
Modell, Sprache und Geräte sind während der Aufnahme lesbar, aber nicht direkt
veränderbar"), **§7** Loopback spike obligation (line 976: "Verhalten bei
Kopfhörer- und Standardgerätewechsel") and the manual Windows matrix rows
(lines 1888–1889: "Output-Gerät wird entfernt und neu verbunden";
"Standardgerät wird während der Aufnahme geändert").

This document exists because RC §7 specifies what happens when a device
_fails_, but not what happens when the user performs a _routine_ device action
(unplug a headset to walk away, plug it back, pick another output in the OS or
in Speak's settings) while a meeting records. It fixes exactly that delta: the
four trigger classes × the two sources, the resulting continuation/pause/fail
decision, the timeline continuity of the affected source, and the boundary at
which settings changes do or do not apply to the running session.

Normative language: **must** = gate; **should** = default with documented
exception path; **may** = permitted option.

---

## 1. Delta discipline (what this document may and may not do)

1. This document **adds** decision rows and boundary rules on top of RC §5–§7
   and CC §4/§6. It **modifies no rule** of either base contract. Where a base
   contract already answers a case (e.g. RC §7.1 row 3 for an enabled
   follow-default option), that answer is restated here only for tabular
   completeness and marked as such.
2. Every mechanism used below is an **existing RC primitive**: `RecorderEvent`
   variants (RC §4), `RecorderError` variants (RC §4), `SourceState` values
   (RC §4), `open_source`/`close_source` (RC §4), source segments and markers
   (RC §6), gap/drift bookkeeping (RC §5). Section 5 proves this by
   enumeration; introducing new event/error/marker kinds here is forbidden
   (see NG-4).
3. Error-code surface stays closed: the taxonomy mapping in
   [error-taxonomy-contract.md](error-taxonomy-contract.md)
   (`meeting.recorder.mic_device_lost`, `meeting.recorder.system_device_lost`,
   `meeting.recorder.default_device_changed` with its `followMode`
   = `bound`/`followed` param selector, `meeting.recorder.stream_error`)
   already covers every event this delta emits. No NEW copy-key family is
   introduced by this document.

## 2. Trigger inventory and detection surface

Exactly four trigger classes exist. Anything else (sleep/wake, disk pressure,
driver crash without removal) is owned by RC §7.1 as-is.

| Class | Name                                                                                                                                       | Sources affected                                                       | Detection surface (existing)                                                                                                                                                                                                                                                                                                                                                                          |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TC1   | **Unplugged** — the currently bound physical device disappears (USB/BT removal, BT link loss, driver disable)                              | Microphone, System                                                     | cpal stream error callback on the affected source → `StreamError { source }` plus `MicDeviceLost`/`SystemDeviceLost` (RC §7.1 rows 1–2)                                                                                                                                                                                                                                                               |
| TC2   | **Re-plugged** — a previously lost device reappears in enumeration while its source is `Lost`                                              | Microphone, System                                                     | device-enumeration change observed while `SourceState::Lost` (via the reused `list_input_devices`/`list_output_devices`, RC reuse obligations). Reappearance alone triggers **nothing** — see R-TC2                                                                                                                                                                                                   |
| TC3   | **Switched in settings** — user changes the meeting device selection                                                                       | Microphone, System                                                     | two disjoint paths: (a) settings/commands path writing `meeting_selected_microphone`/`meeting_selected_output_device`/`meeting_follow_default_output`; (b) live-view repair actions named in RC §7.1 (_Wiederverbinden_, _Gerät wechseln_, _Loopback neu öffnen_, _Ohne Systemaudio fortführen_). Path (a) never touches capture (§3); path (b) maps onto explicit `close_source`/`open_source` calls |
| TC4   | **Default device vanishes/changes** — OS default output (or default microphone at snapshot-resolution time) changes identity or disappears | System primarily; Microphone only via the frozen start-time resolution | `DefaultDeviceChanged { observed_default }` (RC §7.1 row 3) for changes; TC1 detection for vanishing without a successor                                                                                                                                                                                                                                                                              |

**R-TC2 (no silent reopen).** Reappearance of a previously lost device
changes no capture state by itself; it only enables the repair affordances
(rows M3/S3). Reconnection is always an explicit user decision.

Debounce: transient enumerator churn around physical replug must not flap
state machines. A single confirmation window applies:

```rust
/// Minimum span a removed device must stay absent before TC1 loss handling
/// runs its segment-close path, and a reappeared device must stay present
/// before it is offered as reconnectable. Product constant, provisional;
/// decided by the plan-line-976 spike obligation ("Verhalten bei Kopfhörer-
/// und Standardgerätewechsel"). Lives in the RC §5.1 `tolerances.rs` module.
pub const DEVICE_SWITCH_CONFIRM_MS: u64 = 1_500;
```

The constant gates only _event emission and segment closing_, never the
capture callback path (structural rule RC §1/plan §8: callbacks never block or
wait).

## 3. Settings hot-apply boundary (normative)

The boundary has exactly five rules:

- **H1 — Snapshot binding.** While a session is active, capture reads device
  parameters exclusively from the immutable session snapshot (CC §6.3).
  Changes to `meeting_selected_microphone`,
  `meeting_selected_output_device` or `meeting_follow_default_output` during a
  session **must not** affect any open stream, neither immediately nor at the
  next chunk rollover nor at pause/resume (pause/resume re-validation compares
  against the snapshot, RC §6 `resume()`).
- **H2 — No live-view writes.** Consistent with plan line 635–637, the running
  view exposes no control that writes the meeting device setting keys. The
  settings surface reachable during a recording ("Für nächstes Meeting ändern")
  navigates away and applies to the **next** session only.
- **H3 — The one sanctioned mid-session change path.** The only way a device
  changes mid-session is an explicit consumer action already named by RC §7.1
  (_Wiederverbinden_ / _Gerät wechseln_ for the mic source; _Loopback neu
  öffnen_ / _Ohne Systemaudio fortführen_ / device switch for the system
  source). Each such action maps onto exactly one explicit
  `MeetingRecorder::open_source` (new source segment, RC §6) and — where the
  old segment must end — exactly one `close_source`, ordered per H4. These are
  deliberate user decisions, not settings propagation; they therefore do not
  violate H1.
- **H4 — Validate-before-close.** A mid-session switch **must** attempt the
  new `open_source` validation/open first and only close the previous segment
  once the new one is open (or schedule the close atomically with a successful
  open). A failed switch leaves the previously active source capturing
  unchanged. Losing a working source to a failed switch is a defect.
- **H5 — Dictation feedback key untouched.** `selected_output_device` keeps
  its sole feedback-sound meaning (CC §6.2 hard rule). Changing it mid-meeting
  moves feedback-sound playback only and never appears in any table below.

## 4. Decision tables (Gate G1)

Common column semantics: _Capture effect_ cites the RC primitive invoked;
_Timeline effect_ is defined precisely once in §5 and referenced here as
**TL-closed** (segment cleanly ended, interim silence, no markers) or
**TL-none** (timeline unaffected). "Consumer" names the component per RC §7
convention.

### 4.1 SourceKind::Microphone

| #   | Trigger (class)                                                       | Guard / precondition                        | Capture effect                                                                                                                                                      | Timeline                                                | Events surfaced                                                                                                                       | Consumer                                                                                                                                                                         |
| --- | --------------------------------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | Unplugged (TC1)                                                       | mic source `Active`/`Degraded`              | confirm after `DEVICE_SWITCH_CONFIRM_MS`; finalize open chunk to `ready`; `close_source(Mic)`; state `Lost`. System source unaffected (per-source continuation, P1) | TL-closed                                               | `MicDeviceLost`, `SourceStateChanged { Mic, Lost }`, final `ChunkFinalized`(s)                                                        | `MeetingManager` keeps session `recording`; `MeetingRecorderView` orange warning with _Wiederverbinden_/_Gerät wechseln_ (RC §7.1 row 1 verbatim); Tray/Overlay via `snapshot()` |
| M2  | Unplugged while a dictation tap is subscribed (TC1 ∧ CC C1)           | as M1, plus live dictation                  | identical to M1; the tap closes with the stream                                                                                                                     | TL-closed                                               | M1 events (tap consumers get none beyond the tap closing)                                                                             | dictation side follows CC §4.5/R2 (drain buffer, frame-boundary fallback to `DeviceSource`) — **no meeting-side special case**                                                   |
| M3  | Re-plugged (TC2), user does nothing                                   | source `Lost`, device back ≥ confirm window | **none.** Reappearance alone never reopens the stream                                                                                                               | TL-none                                                 | none                                                                                                                                  | warning persists; _Wiederverbinden_ button becomes enabled (R-TC2)                                                                                                               |
| M4  | _Wiederverbinden_ succeeds (TC2/TC3b)                                 | source `Lost`, chosen device passes open    | `open_source(Mic, request)` → new source segment                                                                                                                    | TL-closed then new segment at current timeline position | `SourceSegmentStarted { Mic, … }`, `SourceStateChanged { Mic, Active }`                                                               | warning clears; transcript shows "ab hier: anderes Gerät"-capable segment metadata (RC §6)                                                                                       |
| M5  | _Wiederverbinden_ fails (device gone again, permission revoked)       | as M4, open error                           | **no state change**: source stays `Lost`; nothing was closed                                                                                                        | TL-none                                                 | `open_source` returns `Err` with `RecorderError::MicDeviceUnavailable` or `RecorderError::CapturePermissionDenied` — see §6 note E-N1 | same warning with failure reason; retry remains available                                                                                                                        |
| M6  | Switched in global settings (TC3a)                                    | session active                              | **none** (hot-apply boundary H1/H2)                                                                                                                                 | TL-none                                                 | none                                                                                                                                  | settings UI marks the pending change "gilt ab dem nächsten Meeting"                                                                                                              |
| M7  | _Gerät wechseln_ to a different device, succeeds (TC3b)               | mic source `Active`/`Degraded`/`Lost`       | validate-open-first (H4): open new segment, then finalize+close old segment                                                                                         | old part TL-closed; new segment from switch point       | `SourceSegmentStarted { Mic, new device }` (+ `ChunkFinalized` of the closed tail)                                                    | notice "Mikrofon gewechselt"; segments renderable per RC §6                                                                                                                      |
| M8  | Default microphone changes in OS (TC4)                                | session active                              | **none.** The snapshot froze one concrete mic identity at start; the mic request carries `follow_default_device = false` always (RC §2.2)                           | TL-none                                                 | none                                                                                                                                  | none (not even a notice — the binding never followed)                                                                                                                            |
| M9  | Bound mic is _also_ the OS default and that device vanishes (TC4→TC1) | snapshot-resolved mic == vanished default   | exactly M1. Default-vanish adds nothing beyond TC1 for the mic                                                                                                      | TL-closed                                               | M1 events                                                                                                                             | M1 consumers                                                                                                                                                                     |

### 4.2 SourceKind::System (loopback)

| #   | Trigger (class)                                                                                                                 | Guard / precondition                                                    | Capture effect                                                                                                                                                                             | Timeline                   | Events surfaced                                                                                                      | Consumer                                                                                                                   |
| --- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------- | -------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| S1  | Bound output unplugged (TC1)                                                                                                    | sys source `Active`/`Degraded`, `follow_default_device = false` (bound) | as M1 mirrored: finalize, `close_source(System)`, state `Lost`. Mic continues (P1)                                                                                                         | TL-closed                  | `SystemDeviceLost`, `SourceStateChanged { System, Lost }`                                                            | warning with _Loopback neu öffnen_ / device switch / _Ohne Systemaudio fortführen_ (RC §7.1 row 2 verbatim)                |
| S2  | Bound output unplugged, `follow_default_device = true` and OS migrated default **before** removal reached us (TC4 precedes TC1) | follow enabled                                                          | the `DefaultDeviceChanged` handling of RC §7.1 row 3 applies first: old segment closes cleanly, new segment on the new default. If the new default also fails to open → fall through to S1 | old TL-closed, new segment | `DefaultDeviceChanged` + `SourceSegmentStarted`                                                                      | RC §7.1 row 3 consumer (non-blocking notice either way)                                                                    |
| S3  | Re-plugged (TC2), no action                                                                                                     | source `Lost`                                                           | **none** (R-TC2, mirrors M3)                                                                                                                                                               | TL-none                    | none                                                                                                                 | _Loopback neu öffnen_ becomes enabled                                                                                      |
| S4  | _Loopback neu öffnen_ succeeds                                                                                                  | source `Lost`                                                           | `open_source(System, request)` → new segment                                                                                                                                               | TL-closed then new segment | `SourceSegmentStarted { System, … }`                                                                                 | system levels resume; warning clears                                                                                       |
| S5  | _Ohne Systemaudio fortführen_                                                                                                   | any post-loss or pre-loss state                                         | explicit `close_source(System)`; session flagged `has_system_audio = false` from this point (RC §7.1 row 2 wording); no further system segments this session                               | TL-closed (if it was open) | `SourceStateChanged { System, Closed }`                                                                              | warning resolves into persistent "ohne Systemaudio" badge; reversible only by explicit re-open action, never automatically |
| S6  | Switched in global settings (TC3a)                                                                                              | session active                                                          | **none** (H1/H2)                                                                                                                                                                           | TL-none                    | none                                                                                                                 | "gilt ab dem nächsten Meeting" marking                                                                                     |
| S7  | Explicit loopback device switch, succeeds (TC3b)                                                                                | any non-`Closed` state                                                  | H4 ordering: open new segment on chosen output, then finalize+close old                                                                                                                    | as M7                      | `SourceSegmentStarted { System, … }`                                                                                 | notice; segment metadata per RC §6                                                                                         |
| S8  | Default output changes, `follow = true`, successor exists (TC4)                                                                 | follow enabled                                                          | restated from RC §7.1 row 3 / RC §6: old segment closes, new segment begins — never silently                                                                                               | TL-closed + new segment    | `DefaultDeviceChanged` + `SourceSegmentStarted`                                                                      | non-blocking notice (existing behavior)                                                                                    |
| S9  | Default output vanishes, `follow = true`, **no successor default** (TC4)                                                        | follow enabled, enumerator reports no usable default                    | degrades to S1: TC1 handling on the bound stream                                                                                                                                           | TL-closed                  | `SystemDeviceLost` (+ `DefaultDeviceChanged` if the OS announced one first)                                          | S1 consumers                                                                                                               |
| S10 | Default output changes/vanishes, `follow = false` (bound mode)                                                                  | bound device ≠ changed default                                          | **none** if the _bound_ device still exists (the binding does not follow; RC §7.1 row 3: "bleibt das ausgewählte Gerät bindend"); exactly S1 if the **bound device itself** vanished       | TL-none / TL-closed        | `DefaultDeviceChanged` always (informational) when a change was observed; `SystemDeviceLost` only in the vanish case | non-blocking notice; S1 consumers in the vanish case                                                                       |

### 4.3 Continuation vs. pause vs. fail (cross-cutting rules)

These four rules bind every row above:

- **P1 — Per-source continuation.** Loss or change of one source never
  pauses, degrades the session state machine, or stops the other source
  (plan lines 945–948); the session-level state machine is unaffected by
  single-source loss. The both-sources case is P4.
- **P2 — Markers are user-pause-only.** Device events never write
  `TimelineMarker { Pause }`/`{ Resume }`. Markers remain exclusively the
  record of user pause/resume (RC §6); device continuity is expressed through
  **source segments**, which is exactly the primitive the plan reserves for
  this (line 933–934).
- **P3 — No new session-failure paths.** Device removal alone never produces
  `ControlledStopInitiated`. Controlled stop remains reserved for
  `DiskFull` and `FatalStreamError` (RC §4 `StopReason`). A `FatalStreamError`
  declared by the OS for one source escalates only that source to the loss
  rows above, unless the stop originates from the existing disk/fatal paths.
- **P4 — Both sources lost simultaneously.** Each source is processed
  independently by its own row; there is no combined "session lost" state, no
  automatic stop, and no automatic pause. The session stays `recording` with
  both sources `Lost` (inter-segment silence accrues per §5 item 2) until the
  user stops or repairs a source; the fixed control surfaces (UI, tray,
  shortcut snapshot) remain reachable (CC §7.5 edge rule analog).

## 5. Affected-source timeline semantics (Gate G2)

Every timeline consequence in this delta is expressed with **only** these
RC §5–§6 primitives; this subsection is the exhaustive enumeration:

1. **Segment close on loss/switch.** The open chunk of the affected source is
   finalized to `ready` (RC §3.1 transition via `close_source`, which
   "finalizes the open chunk of that source"). The segment
   `(source, device_identity, [from_ms, to_ms))` ends at the loss/switch
   instant (RC §6 definition).
2. **Interim hole representation.** The span between the closed segment's end
   and the next segment's start is **not** gap-filled incrementally and **not**
   marked with pause markers. It remains timeline silence carrying
   segment-boundary metadata, exactly per RC §5.2 rule 5 ("the hole between
   segments remains as timeline silence with segment-boundary metadata instead
   of a growing gap"). No `TimelineGap` event is emitted for the loss interval
   itself; `TimelineGap` keeps its RC §5.2 meaning inside _live_ segments
   only.
3. **Reopen/replacement.** Any mid-session reopen — same device re-plugged and
   reconnected, or different device chosen — calls `open_source`, which starts
   a new source segment and emits `SourceSegmentStarted` at the current
   timeline position (RC §6: segments begin at "re-open after device loss with
   the same or a different device"). Same-device reopens are still **new
   segments**: device identity equality never merges segments, because the
   hole between them is real recorded-history information.
4. **Track invariants untouched.** Existing audio is never shifted or
   shortened; the affected track's length always equals elapsed timeline time
   (RC §2.2 rule 4, RC §5.2 rule 3). The other source's track is untouched in
   both bytes and timing (RC §5.2 rule 6).
5. **Pause/resume interplay.** If the user pauses while a source is `Lost`,
   `resume()` performs its normal re-validation (RC §6) and may start the
   replacement segment — the same primitives as rows M4/S4, just triggered
   from the resume path. The `[pause, resume)` span is excluded by
   construction (RC §5.2 rule 5, RC §6 pause semantics); the earlier loss
   interval is unaffected.
6. **Prohibition list.** For clarity, the following are **forbidden** as
   device-change consequences: new marker kinds; new `RecorderEvent`/
   `RecorderError` variants; re-timestamping of persisted chunks; merging or
   splitting existing segments retroactively; inserting silence into the
   unaffected source; emitting `TimelineGap` for a whole loss interval.

## 6. Coexistence conformance (Gate G3)

The single-device-open invariant (CC §4.1: "At most one cpal input stream per
physical microphone device exists process-wide at any instant") holds under
every row above by construction:

- **C-1 All midsession opens are arbitrated.** Rows M4/M7 (and their system
  counterparts, which are render-device captures and thus outside the
  invariant per CC §4.1's loopback exclusion) open streams only through the
  normal `open_source` path, which consults the ownership registry (CC §4.2).
  No device-change row ever bypasses the arbiter, so a second stream on an
  owned device cannot be created by recovery logic.
- **C-2 Post-loss dictation ownership.** If the mic is lost while a dictation
  holds a tap (row M2), the tap dies with the stream; the dictation falls back
  per CC §4.5/R2 and may legitimately own the device via `DeviceSource`.
  A later _Wiederverbinden_ (M4) requests the device through the arbiter: if
  the dictation still holds it, the arbiter refuses and the mic source stays
  `Lost` with the warning intact — the exact resolution CC R1 prescribes for
  the mirror case. When migration succeeds, it happens at a frame boundary
  (CC §4.4 R1/R2), and the meeting's new segment starts only after the
  arbiter hands over.
- **C-3 No silent tap resurrection.** Reopening the meeting mic never
  implicitly re-subscribes a dead or ended tap (CC R5: taps are bounded to one
  dictation lifetime). A subsequent dictation simply takes a fresh tap via the
  normal C1 path.
- **C-4 Settings boundary consistency.** Rule H1 restates CC §6.3 snapshot
  isolation for the device keys specifically; the CC §6.4 independence matrix
  rows for `meeting_selected_output_device` (effect: none) are thereby
  completed by this delta with the _unplug/replug/switch_ dimensions that
  matrix did not cover. Nothing here gives meeting capture any read of
  `selected_output_device` (CC §6.2 violation check unchanged).

Note E-N1 (variant reuse ruling): errors returned by mid-session
`open_source` attempts reuse the RC §7.2 Startvalidierung variants
(`MicDeviceUnavailable`, `SystemOutputUnavailable`,
`CapturePermissionDenied`, `LoopbackSharedModeRequired`). Read RC §7.2's label
as _device-open validation_ — the same variants already serve `resume()`'s
re-validation (RC §6), so this reuse follows the contract's own precedent and
does not widen §7.2's consumer-table scope; the consuming UI remains the
repair affordance already named in the RC §7.1 rows.

## 7. Complexity budget & non-goals (binding; Gate G4)

- **NG-1 — No automatic reconnection.** Reappearance of a lost device never
  reopens a stream by itself (rows M3/S3). Rationale: the plan mandates a
  visible warning with explicit _Wiederverbinden_/switch offers (lines
  945–947); silent auto-reconnect would also make segment boundaries
  unpredictable for the transcript view.
- **NG-2 — No hot-apply of settings.** No setting key, including device keys,
  reaches a running session outside the snapshot (H1). There is no
  "apply-now" button and no diff-and-migrate machinery.
- **NG-3 — No device roaming or aggregation.** One bound device per source
  per segment; no multi-microphone mixing, no automatic failover to "some
  other available mic", no OS virtual-audio-device tricks (mirrors CC §9).
- **NG-4 — No new interface surface.** Zero new `RecorderEvent` variants,
  zero new `RecorderError` variants, zero new event topics, zero new marker
  kinds, zero new i18n key families (§1 item 3). The only new constant is
  `DEVICE_SWITCH_CONFIRM_MS`, homed in the RC §5.1 `tolerances.rs` module.
- **NG-5 — No per-app/per-process loopback and no routing changes.** TC4
  handling works purely at the capture-stream level of the v1 CPAL loopback
  (plan §7 "Loopback").
- **NG-6 — No diarization or speaker claims.** Segment metadata records
  technical provenance only (RC §2.1); "wer gewechselt hat" is never inferred.
- **NG-7 — No dictation-side behavior change.** Every dictation-visible
  effect routes through CC §4.5/R1/R2 as-is; this delta adds no dictation
  code path.
- **NG-8 — No configurable debounce UI.** `DEVICE_SWITCH_CONFIRM_MS` is a
  testable constant, not a setting; revisiting it follows the RC §5.1
  measurement process (spike obligation plan line 976).
- **NG-9 — No retroactive transcript edits.** Device changes annotate history
  via segments; they never rewrite, split or re-order persisted transcript
  segments (aligns with transcript-rendering-contract immutability).

## 8. Gate coverage

| Gate                                                                                                           | Where satisfied                                                                                                                              |
| -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 decision table covers unplugged, re-plugged, switched-in-settings, default-device-vanishes for both sources | §4.1 rows M1–M9 (microphone), §4.2 rows S1–S10 (system); trigger classes TC1–TC4 defined in §2; cross-cutting continue/pause/fail rules §4.3 |
| G2 affected-source timeline semantics reuse RC §5–6 primitives only                                            | §5 exhaustive enumeration + prohibition list; §1 item 2 restricts the vocabulary up front                                                    |
| G3 no contradiction with the CC §4 single-device-open invariant                                                | §6 C-1–C-4, incl. the arbitrated-reopen argument and the R1-mirror refusal path                                                              |
| G4 non-goals section; documentation index updated                                                              | §7 NG-1–NG-9; index row + reading-order entry in `docs/meeting-mode-documentation-index.md`                                                  |

## Appendix A — Evidence basis (read at authoring time)

- RC §4 `SourceOpenRequest.follow_default_device` doc comment: mic flag always
  `false` (recorder-contract.md lines 270–279) — basis for M8.
- RC §5.2 rule 5 and RC §6 segment-begin list (lines 714–717, 749–766) —
  basis for §5 items 2–3 and the "same device ⇒ new segment" ruling.
- RC §7.1 rows 1–3 consumer actions (lines 793–795) — the complete set of
  sanctioned mid-session actions used by TC3b (H3).
- Plan lines 635–637, 929–934, 943–951, 976, 1888–1889 (quoted in the header)
  — behavioral anchors and test obligations.
- CC §4.1 invariant wording, §4.2 registry, §4.4 R1/R2, §4.5 fallbacks,
  §6.3 snapshot isolation, §6.4 independence matrix (coexistence-contract.md
  lines 254–360, 439–473) — basis for §6 and H1.
- error-taxonomy-contract.md lines 207, 216–218 — existing code mappings for
  every emitted event (`followMode` selector `bound`/`followed` matches the
  S-row split).
