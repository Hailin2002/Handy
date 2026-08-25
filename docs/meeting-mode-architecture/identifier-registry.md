# Cross-Contract Identifier Registry — Meeting Mode

Status: **registry only — this document defines nothing.** It inventories the
named identifiers introduced by independently authored contracts so that naming
drift is caught before Phase 2 implementation. The **defining documents remain
the sole authority** for every identifier listed here; where this registry and
a defining document disagree, the defining document wins (same precedence rule
as [session-state-contract.md](session-state-contract.md): plan wins over
contracts, contracts win over catalogs).

Inputs inventoried (worktree state of 22.08.2026):

| Abbrev. | Document |
| --- | --- |
| PLAN | [docs/meeting-mode-plan.md](../meeting-mode-plan.md) — "Session-Zustände" block, lines 877–896 (plus §9 cross-checks cited per row) |
| RC | [docs/meeting-mode-architecture/recorder-contract.md](recorder-contract.md) |
| STT | [docs/meeting-mode-architecture/stt-provider-contract.md](stt-provider-contract.md) |
| COPY | [docs/meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md) |

Related documents read for conflict detection only (not registried here):
[session-state-contract.md](session-state-contract.md) (**SSC**, abbreviated
citations in findings) and the Phase-1 spike harness
`docs/phase1-spike/tools/analyze-spike-csv.ps1` (`gap_class` values).

Legend — Kind: `state` = lifecycle/session state, `variant` = enum variant,
`type` = named type/trait/struct, `const` = named constant, `job-state` =
persisted job state value, `err-class` = error classification category,
`entry-id` = copy-catalog entry identifier, `i18n-key` = proposed i18n key,
`token` = persisted string value. Line references are `file:start-line`
(single-line definitions) or `file:a–b` (multi-line).

---

## Findings (open, unresolved)

Every inconsistency or duplicate concept detected is registered here first;
the master table references findings by `F-nn`. **No finding has been applied
to any source document** — fixing them is owned by the respective owning
document's author(s), not by this registry.

| ID | Finding | Citations | Owning document |
| --- | --- | --- | --- |
| F-01 | **Session lifecycle vs. recorder state mapping incomplete.** PLAN defines 11 session states; RC's `RecorderState` covers only the capture subset (`Idle`, `Recording`, `Paused`, `Stopping`, `Recovering`). `Idle` and `Recovering` have no PLAN counterpart; `draft`/`starting`/`transcribing`/`summarizing`/`completed`/`stopped_unprocessed`/`cancelled` have no `RecorderState` counterpart. No mapping table exists in RC or PLAN. (SSC §2.3 asserts a partial 1:1 for `Recording/Paused/Stopping`, but SSC is outside this registry's defining set.) | PLAN 880–891; RC 523–528; SSC 165–167 | recorder-contract.md (mapping owner), plan (state-set owner) |
| F-02 | **Identifier collision on `cancelled`.** The identical string denotes a *session* terminal state (user abort) and a *job* state (user aborts meeting/job processing). Events, logs and UI copy can conflate them. | PLAN 889 (session) vs. PLAN 1197 + STT 88 (job); COPY 208 (S10 covers both readings' UI) | plan §9 (disambiguation), stt-provider-contract.md |
| F-03 | **No casing/serialization rule Rust enum ↔ persisted string.** RC defines PascalCase `ChunkState::{Writing, Ready, Damaged, Deleted}` while PLAN §9 persists snake_case `writing/ready/damaged/deleted`; RC 135–138 asserts one-to-one correspondence but never fixes the mapping mechanism (serde rename, explicit conversion). Job states exist only as snake_case strings (no Rust enum anywhere yet). | RC 141–153 vs. PLAN 1187; STT 88; PLAN 1197 | recorder-contract.md (chunk), stt-provider-contract.md (job) |
| F-04 | **Near-duplicate gap-class names:** RC enum `GapClass::Long` vs. spike-harness class value `gap` (column `gap_class`). Same classification point, two names across artifacts; only RC §5.1 prose bridges them. | RC 312–319 and RC 577–580 vs. `analyze-spike-csv.ps1` `gap_class` values | recorder-contract.md |
| F-05 | **Copy/i18n key scheme drift around the processing stage.** S1–S5/S8 derive keys from the state name (`meeting.session.<state>.*`), but S6 (`transcribing`) and S7 (`summarizing`) use `meeting.processing.*`, and the tray uses `tray.meetingProcessing`. The PLAN state names `transcribing`/`summarizing` (884–885) never appear in their copy keys; the word "processing" exists nowhere in the PLAN state set. | COPY 155–173 (S6), 175–186 (S7), 158, 166 vs. PLAN 884–885; contrast COPY 98–105, 117–133, 188–195 | ux-copy-catalog.md |
| F-06 | **One copy entry for two session states.** S10 is the single entry for both `stopped_unprocessed` and `cancelled`, but proposes only `meeting.session.cancelled.*` copy. There is no distinct copy id or i18n key set for `stopped_unprocessed`, although PLAN treats them as distinct terminal states (889). | COPY 208–217 vs. PLAN 889 | ux-copy-catalog.md |
| F-07 | **Three names for one loopback failure:** preflight outcome `SharedModeRequired`, error `LoopbackSharedModeRequired`, and user-facing copy `meeting.errors.exclusiveMode.*`. "Shared mode" (contract wording) and "exclusive mode" (copy wording) are complements, never reconciled in one glossary entry. | RC 293–295; RC 414–416; COPY 283–292 | ux-copy-catalog.md (user-facing term), recorder-contract.md (technical term) |
| F-08 | **Loopback-unavailable naming split:** copy key root `meeting.errors.noLoopback.*` vs. error variant `RecorderError::LoopbackUnavailable`. Same Startvalidierung case, unrelated names; a Phase-2 implementer cannot derive the key from the variant. | COPY 270–281 vs. RC 419–421 | ux-copy-catalog.md |
| F-09 | **Disk-condition identifier sprawl:** four recorder-side identifiers (`InsufficientDiskSpace`, `DiskSpaceLow`, `StopReason::DiskFull`, `DISK_LOW_WATERMARK_BYTES`) map onto two copy entries (E7 `diskLow`, E8 `diskFull`) with no documented which-maps-to-which table. Note additionally `InsufficientDiskSpace` (start validation) vs. `DiskSpaceLow` (runtime warning) encode the low/full distinction differently than COPY's `diskLow`/`diskFull`. | RC 422–423, 393, 344–345, 689 vs. COPY 294–302 (E7), 304–311 (E8) | recorder-contract.md |
| F-10 | **"Recover*" word-family overload:** `RecorderState::Recovering` (transient activity), `RecorderError::RecoveryFailed`, `recover_open_chunks`, `RecoveryReport` versus the session-level *condition* `recoverable_error`. Similar spellings, different layers; risk of misreading `Recovering` as the session being in `recoverable_error`. | RC 528, 432, 557, 654–658 vs. PLAN 888; COPY 197–206 (S9) | recorder-contract.md |
| F-11 | **Two `*State` enums in one contract:** `SourceState` (`Active/Degraded/Lost/Closed`) and `RecorderState` coexist; `RecorderEvent::SourceStateChanged` carries a `SourceState` while `RecorderSnapshot::state` carries a `RecorderState`. Not a conflict, but a documented confusion hazard for Phase 2 reviewers. | RC 321–331 vs. RC 522–529; RC 361, 541–547 | recorder-contract.md |
| F-12 | **Session states have no planned symbol.** None of the four inputs assigns a Rust enum name or TS type to the PLAN session states; they exist only as prose/string tokens. (SSC uses bare string literals too.) First implementer will have to invent `SessionState` uncoordinated. | PLAN 877–896 (prose only); SSC 109–121 (string literals) | plan (owner), otherwise session-state-contract.md |
| F-13 | **`recoverable_error`: state value or overlay flag?** PLAN phrasing ("kann zusätzlich … besitzen", 888) and COPY S9 ("Muster, überlagerbar auf jeden aktiven Zustand", 197) describe an *overlay*; both also list it as a first-class entry/state (COPY S9; PLAN names it alongside states). SSC models it as both (state value in its §2.1 table, overlay semantics via T13/T17). The persisted representation (one `state` column vs. state + flag) is undecided. | PLAN 888; COPY 197; SSC 121, 143, 147 | plan (data-model owner) |
| F-14 | **Backlog naming split:** recorder exposes `SourceStatus::backlog_chunks_ready`; copy entry E11 speaks of "backlog" (`meeting.warnings.backlog.*`) and S6 of `meeting.processing.backlog`. Concept matches, names differ; no cross-reference in either direction. | RC 532–538 vs. COPY 342–349 (E11), 165 (S6) | ux-copy-catalog.md |
| F-15 | **Duplicate i18n keys defined in two copy entries each.** `meeting.cloud.active.azureProcessing` is defined in both S6 (169–171) and P4 (469–470); `meeting.cloud.active.llm` in both S7 (183–184) and P5 (482–483). Single-source-of-truth for keys is violated inside the catalog. | COPY 169–171 vs. 469–470; 183–184 vs. 482–483 | ux-copy-catalog.md |
| F-16 | **Inline action keys outside the shared §2.3 table.** Nine action keys are introduced inline in entries instead of the reusable-actions table: `startMeeting` (102), `testSystemAudio` (266), `recover` (358), `decideLater` (359), `resumeWithDevices` (385), `useLocal` (459), `useCloudSummary` (484), `summarizeLocally` (485), `disableSummary` (486). The catalog's own convention (§2.3) is the shared table. | COPY 57–83 (table) vs. the nine inline lines | ux-copy-catalog.md |

Verified-consistent correspondences (registered to prevent re-litigation):
chunk states `writing/ready/damaged/deleted` identical in PLAN 1187 and RC
137–153; job states `queued/running/retry_wait/succeeded/failed/cancelled`
identical in PLAN 1197 and STT 87–92; source tokens `microphone`/`system`
identical in RC 47–61 (`file_tag`), STT 67 and PLAN 1183; provider tokens
`local|azure|custom` only defined once (STT 72); recorder events
`MicDeviceLost`/`SystemDeviceLost`/`DefaultDeviceChanged`/`SleepWake`/
`DiskSpaceLow`/`ControlledStopInitiated` align 1:1 with COPY entries E1/E2/E3/
E13/E7+E8/E8 respectively; component name `MeetingRecoveryBanner` identical in
RC 800 and COPY 353.

---

## Master identifier table

Single table, ordered by defining document, then by line of definition.
"Planned symbol" reproduces the name stated in the defining document; "—"
means the document names no implementation symbol. `F-nn` references findings
above; "✓" means cross-document consistency was positively verified.

### Session states (PLAN, "Session-Zustände")

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `draft` | state | PLAN:880 | — (F-12) | start-dialog state; COPY S1 |
| `starting` | state | PLAN:881 | — (F-12) | validation/countdown; COPY S2; RC §7.2 Startvalidierung cases surface here |
| `recording` | state | PLAN:882 | — (F-12) | overlaps `RecorderState::Recording` (F-01, F-03) |
| `paused` | state | PLAN:882 | — (F-12) | overlaps `RecorderState::Paused` (F-01, F-03) |
| `stopping` | state | PLAN:883 | — (F-12) | overlaps `RecorderState::Stopping` (F-01, F-03) |
| `transcribing` | state | PLAN:884 | — (F-12) | copy keys avoid this name (F-05) |
| `summarizing` | state | PLAN:885 | — (F-12) | optional; mirrored in `summary_state` (PLAN 1176) |
| `completed` | state | PLAN:886 | — (F-12) | terminal success |
| `stopped_unprocessed` | state | PLAN:889 | — (F-12) | user-abort terminal; shares single copy entry S10 (F-06); used by RC 854–857 ✓ |
| `cancelled` | state | PLAN:889 | — (F-12) | collides with job state `cancelled` (F-02); COPY S10 |
| `recoverable_error` | state | PLAN:888 | — (F-12) | overlay-vs-value ambiguity (F-13); word-family overlap with RC recovery identifiers (F-10) |

### Recorder contract (RC) — enum variants

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `SourceKind` | type (enum) | RC:47 | `SourceKind` | persisted as `meeting_audio_chunks.source` ✓ |
| `Microphone` | variant | RC:48 | `SourceKind::Microphone` | token `microphone` ✓ STT 67, PLAN 1183 |
| `System` | variant | RC:49 | `SourceKind::System` | token `system` ✓ |
| `MarkerKind` | type (enum) | RC:85 | `MarkerKind` | pause/resume boundary pairs |
| `Pause` (MarkerKind) | variant | RC:86 | `MarkerKind::Pause` | distinct from `RecorderEvent::Paused` and session state `paused` |
| `Resume` (MarkerKind) | variant | RC:87 | `MarkerKind::Resume` | distinct from `RecorderEvent::Resumed` |
| `ChunkState` | type (enum) | RC:142 | `ChunkState` | one-to-one with PLAN 1187 (✓ values), serialization rule missing (F-03) |
| `Writing` | variant | RC:144 | `ChunkState::Writing` | DB `writing`; `.wav.part` phase |
| `Ready` | variant | RC:146 | `ChunkState::Ready` | DB `ready`; only consumable state (RC 226–228) ✓ STT consumption rule |
| `Damaged` | variant | RC:148 | `ChunkState::Damaged` | DB `damaged`; forensics-retained |
| `Deleted` | variant | RC:150 | `ChunkState::Deleted` | DB `deleted`; content removed, row kept |
| `PreflightOutcome` | type (enum) | RC:289 | `PreflightOutcome` | preflight check results |
| `Available` | variant | RC:290 | `PreflightOutcome::Available` | — |
| `Missing` | variant | RC:291 | `PreflightOutcome::Missing` | maps to RC 807–808 repair dialogs; COPY E5/E16 adjacent |
| `PermissionDenied` | variant | RC:292 | `PreflightOutcome::PermissionDenied` | pairs with `CapturePermissionDenied`; COPY E17 |
| `SharedModeRequired` | variant | RC:294 | `PreflightOutcome::SharedModeRequired` | naming drift vs. COPY `exclusiveMode` (F-07) |
| `GapClass` | type (enum) | RC:312 | `GapClass` | spike-derived classification |
| `Normal` | variant | RC:314 | `GapClass::Normal` | spike class `normal` ✓ |
| `Sparse` | variant | RC:316 | `GapClass::Sparse` | spike class `sparse` ✓ |
| `Long` | variant | RC:318 | `GapClass::Long` | spike class is `gap`, not `long` (F-04) |
| `SourceState` | type (enum) | RC:322 | `SourceState` | second `*State` enum (F-11) |
| `Active` | variant | RC:324 | `SourceState::Active` | — |
| `Degraded` | variant | RC:326 | `SourceState::Degraded` | threshold `SEGMENT_DEGRADED_GAP_MS` pending task-0008 (RC 683) |
| `Lost` | variant | RC:328 | `SourceState::Lost` | device loss; pairs with `MicDeviceLost`/`SystemDeviceLost` events ✓ COPY E1/E2 |
| `Closed` | variant | RC:330 | `SourceState::Closed` | deliberate close (mic-only fallback, controlled stop) ✓ COPY E5/E8 |
| `StopReason` | type (enum) | RC:343 | `StopReason` | — |
| `Requested` | variant | RC:344 | `StopReason::Requested` | — |
| `DiskFull` | variant | RC:345 | `StopReason::DiskFull` | drives COPY E8 (F-09) |
| `FatalStreamError` | variant | RC:346 | `StopReason::FatalStreamError` | — |
| `RecorderEvent` | type (enum) | RC:354 | `RecorderEvent` | event channel to MeetingManager |
| `Started` | variant | RC:355 | `RecorderEvent::Started` | triggers session `starting→recording` (SSC T03) |
| `Paused` (event) | variant | RC:356 | `RecorderEvent::Paused` | distinct from `MarkerKind::Pause` |
| `Resumed` (event) | variant | RC:357 | `RecorderEvent::Resumed` | distinct from `MarkerKind::Resume` |
| `LevelUpdate` | variant | RC:360 | `RecorderEvent::LevelUpdate` | throttled by `LEVEL_EVENT_INTERVAL_MS` |
| `SourceStateChanged` | variant | RC:361 | `RecorderEvent::SourceStateChanged` | carries `SourceState` (F-11) |
| `SourceSegmentStarted` | variant | RC:362 | `RecorderEvent::SourceSegmentStarted` | segment model §6; ✓ COPY E3/E14 "new source section" wording |
| `ChunkFinalized` | variant | RC:368 | `RecorderEvent::ChunkFinalized` | sole readiness signal for STT jobs ✓ |
| `TimelineGap` | variant | RC:370 | `RecorderEvent::TimelineGap` | gated by pending `GAP_EVENT_MIN_MS` |
| `DriftCorrectionApplied` | variant | RC:376 | `RecorderEvent::DriftCorrectionApplied` | bounded by pending `DRIFT_CORRECTION_MAX_PPM` |
| `DriftToleranceExceeded` | variant | RC:378 | `RecorderEvent::DriftToleranceExceeded` | threshold `DRIFT_EXCEEDED_FRAMES` pending |
| `MicDeviceLost` | variant | RC:385 | `RecorderEvent::MicDeviceLost` | ✓ COPY E1 |
| `SystemDeviceLost` | variant | RC:386 | `RecorderEvent::SystemDeviceLost` | ✓ COPY E2 |
| `DefaultDeviceChanged` | variant | RC:387 | `RecorderEvent::DefaultDeviceChanged` | ✓ COPY E3 |
| `SleepWake` | variant | RC:390 | `RecorderEvent::SleepWake` | ✓ COPY E13 |
| `DiskSpaceLow` | variant | RC:393 | `RecorderEvent::DiskSpaceLow` | runtime warning side of F-09; ✓ COPY E7 |
| `ControlledStopInitiated` | variant | RC:394 | `RecorderEvent::ControlledStopInitiated` | ✓ COPY E8 |
| `Stopped` | variant | RC:395 | `RecorderEvent::Stopped` | — |
| `RecorderError` | type (enum) | RC:409 | `RecorderError` | one variant per Fehlerverhalten/Startvalidierung case (RC rule G2) |
| `MicDeviceUnavailable` | variant | RC:411 | `RecorderError::MicDeviceUnavailable` | Startvalidierung; COPY E1-adjacent (start-time) |
| `SystemOutputUnavailable` | variant | RC:413 | `RecorderError::SystemOutputUnavailable` | Startvalidierung |
| `LoopbackSharedModeRequired` | variant | RC:416 | `RecorderError::LoopbackSharedModeRequired` | vs. COPY `exclusiveMode` (F-07) |
| `CapturePermissionDenied` | variant | RC:418 | `RecorderError::CapturePermissionDenied` | ✓ COPY E17 |
| `LoopbackUnavailable` | variant | RC:421 | `RecorderError::LoopbackUnavailable` | vs. COPY `noLoopback` (F-08) |
| `InsufficientDiskSpace` | variant | RC:423 | `RecorderError::InsufficientDiskSpace` | start-validation disk case (F-09) |
| `ChunkCreateFailed` | variant | RC:425 | `RecorderError::ChunkCreateFailed` | mid-session create failure |
| `ChunkStatePersistFailed` | variant | RC:427 | `RecorderError::ChunkStatePersistFailed` | feeds SSC T17 example |
| `StreamError` | variant | RC:429 | `RecorderError::StreamError` | live stream error |
| `SessionAlreadyActive` | variant | RC:430 | `RecorderError::SessionAlreadyActive` | guard |
| `NoActiveSession` | variant | RC:431 | `RecorderError::NoActiveSession` | guard |
| `RecoveryFailed` | variant | RC:432 | `RecorderError::RecoveryFailed` | word-family overlap (F-10) |
| `RecorderState` | type (enum) | RC:523 | `RecorderState` | capture-subset machine; mapping gaps (F-01) |
| `Idle` | variant | RC:524 | `RecorderState::Idle` | no PLAN counterpart (F-01) |
| `Recording` (RecorderState) | variant | RC:525 | `RecorderState::Recording` | same name as session state (F-01, F-03) |
| `Paused` (RecorderState) | variant | RC:526 | `RecorderState::Paused` | same name as session state (F-01, F-03) |
| `Stopping` (RecorderState) | variant | RC:527 | `RecorderState::Stopping` | same name as session state (F-01, F-03) |
| `Recovering` | variant | RC:528 | `RecorderState::Recovering` | no PLAN counterpart; word-family (F-10) |

### Recorder contract (RC) — types, traits, constants

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `TIMELINE_SAMPLE_RATE_HZ` | const | RC:73 | same | fixed by PLAN §7 |
| `TIMELINE_CHANNELS` | const | RC:74 | same | fixed by PLAN §7 |
| `TimelineRange` | type | RC:78 | same | half-open ms range on shared timeline |
| `TimelineMarker` | type | RC:91 | same | persisted by manager (RC 781) |
| `ChunkId` | type (alias) | RC:156 | `ChunkId = i64` | — |
| `ChunkDescriptor` | type | RC:159 | same | mirrors `meeting_audio_chunks` row ✓ |
| `SessionLayout` | type | RC:250 | same | filesystem layout |
| `SourceOpenRequest` | type | RC:270 | same | includes `follow_default_device` (E3 option) ✓ |
| `SourceDeviceSet` | type | RC:282 | same | mic-only fallback carrier ✓ COPY E5 |
| `PreflightCheck` | type | RC:299 | same | carries `PreflightOutcome` |
| `FinalizedChunkInfo` | type | RC:334 | same | payload of `ChunkFinalized` |
| `RecorderEventSink` | type (trait) | RC:400 | same | installed by MeetingManager |
| `ChunkPatch` | type | RC:490 | same | includes `gap_stats` |
| `ChunkStateStore` | type (trait) | RC:502 | same | implemented by MeetingStore |
| `SourceStatus` | type | RC:532 | same | contains `backlog_chunks_ready` (F-14) |
| `RecorderSnapshot` | type | RC:541 | same | single backend view for UI ✓ |
| `SessionSummary` | type | RC:550 | same | returned by `stop()` |
| `RecoveryReport` | type | RC:557 | same | word-family (F-10) |
| `MeetingRecorder` | type (trait) | RC:597 | same | incl. `pause`/`resume`/`stop`/`recover_open_chunks` (654) |
| `CHUNK_ROLLOVER_TARGET_MS` | const | RC:572 | same | 60 000; revisit after Phase 1 |
| `LEVEL_EVENT_INTERVAL_MS` | const | RC:575 | same | UI throttle |
| `GAP_CLASS_NORMAL_MAX_MS` | const | RC:579 | same | spike-fixed ✓ F-04 bridge |
| `GAP_CLASS_SPARSE_MAX_MS` | const | RC:580 | same | spike-fixed ✓ |
| `SILENCE_PEAK_THRESHOLD` | const | RC:586 | same | pending task-0008 confirmation |
| `GAP_EVENT_MIN_MS` | const (pending) | RC:682 | same | pending task-0008 |
| `SEGMENT_DEGRADED_GAP_MS` | const (pending) | RC:683 | same | pending task-0008 |
| `CALLBACK_JITTER_NORMAL_MAX_MS` | const (pending) | RC:684 | same | pending task-0008 |
| `INITIAL_OFFSET_FILL_TOLERANCE_MS` | const (pending) | RC:685 | same | pending task-0008 |
| `DRIFT_CORRECTION_MAX_PPM` | const (pending) | RC:686 | same | pending task-0008 |
| `DRIFT_WARN_FRAMES` | const (pending) | RC:687 | same | pending task-0008 |
| `DRIFT_EXCEEDED_FRAMES` | const (pending) | RC:688 | same | pending task-0008 |
| `DISK_LOW_WATERMARK_BYTES` | const (pending) | RC:689 | same | product constant; COPY E7 numbers (F-09) |

### STT provider contract (STT) — job states, error classes, symbols

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `queued` | job-state | STT:88 | — (no enum yet, F-03) | ✓ identical PLAN 1197; crash-stable (STT 199) |
| `running` | job-state | STT:88 | — (F-03) | ✓; crash-recovery demotes to `queued` (STT 151) |
| `retry_wait` | job-state | STT:88 | — (F-03) | ✓; referenced by PLAN 957 and COPY E9 heading (317) ✓ |
| `succeeded` | job-state | STT:88 | — (F-03) | terminal (STT 152) |
| `failed` | job-state | STT:88 | — (F-03) | terminal until explicit user re-queue (149) |
| `cancelled` | job-state | STT:88 | — (F-03) | collides with session state `cancelled` (F-02) |
| `permanent` | err-class | STT:354 | `ProviderError` category | no auto retry; criteria STT 356–366 |
| `transitory` | err-class | STT:374 | `ProviderError` category | backoff retry; criteria STT 377–382; unclassifiable defaults here (STT 391–393) |
| `MeetingSttProvider` | type (trait concept) | STT:15, 23 | `MeetingSttProvider` | sole common interface |
| `validate` | op | STT:25 | method | returns `ProviderReadiness`/`ProviderError` |
| `transcribe` | op | STT:27 | method | consumes `SttSegmentJob` |
| `capabilities` | op | STT:30 | method | returns `ProviderCapabilities` |
| `LocalMeetingProvider` | type | STT:16, 237 | same | provider token `local` |
| `AzureMeetingClient` | type | STT:17, 264 | same | provider token `azure` |
| `CustomMultipartProvider` | type | STT:17, 298 | same | provider token `custom` |
| `ProviderReadiness` | type | STT:24 | same | validate() success type |
| `ProviderError` | type | STT:24, 348 | same | carries permanent/transitory category |
| `SttSegmentJob` | type | STT:27 | same | input job model |
| `ProviderResult` | type | STT:27, 43 | same | relative segments only |
| `ProviderCapabilities` | type | STT:30 | same | limits declaration |
| `ProviderSegment` | type | STT:46 | same | `start_offset_ms`/`end_offset_ms` relative |
| `idempotency_key` | token | STT:162 | column/field | SHA-256 derivation (STT 162–168) |
| `requeue_generation` | token | STT:166 | column/field | bumped only on explicit re-queue |
| `microphone` (source token) | token | STT:67 | — | ✓ RC `file_tag`, PLAN 1183 |
| `system` (source token) | token | STT:67 | — | ✓ |
| `local` \| `azure` \| `custom` | token | STT:72 | — | single definition point |

### UX copy catalog (COPY) — entry ids

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `S1` | entry-id | COPY:98 | `meeting.session.draft.*` | PLAN `draft` ✓ |
| `S2` | entry-id | COPY:107 | `meeting.session.starting.*` | PLAN `starting` ✓ |
| `S3` | entry-id | COPY:117 | `meeting.session.recording.*`, `tray.meetingRecording` | PLAN `recording` ✓ |
| `S4` | entry-id | COPY:135 | `meeting.session.paused.*`, `tray.meetingPaused` | PLAN `paused` ✓ |
| `S5` | entry-id | COPY:144 | `meeting.session.stopping.*` | PLAN `stopping` ✓ |
| `S6` | entry-id | COPY:155 | `meeting.processing.*`, `tray.meetingProcessing` | PLAN `transcribing` — key/name drift (F-05); duplicate key with P4 (F-15) |
| `S7` | entry-id | COPY:175 | `meeting.processing.summarizing`, `meeting.notify.completed.*` | PLAN `summarizing` — drift (F-05); duplicate key with P5 (F-15) |
| `S8` | entry-id | COPY:188 | `meeting.session.completed.*` | PLAN `completed` ✓ |
| `S9` | entry-id | COPY:197 | `meeting.errors.recoverable.*` | PLAN `recoverable_error` (F-13) |
| `S10` | entry-id | COPY:208 | `meeting.session.cancelled.*` | covers TWO PLAN states (F-06) |
| `E1` | entry-id | COPY:225 | `meeting.errors.micLost.*` | ✓ RC `MicDeviceLost` |
| `E2` | entry-id | COPY:236 | `meeting.errors.systemLost.*` | ✓ RC `SystemDeviceLost` |
| `E3` | entry-id | COPY:246 | `meeting.errors.defaultChanged.*` | ✓ RC `DefaultDeviceChanged` |
| `E4` | entry-id | COPY:260 | `meeting.warnings.silence.*` | ✓ RC "silence is legitimate" (796) |
| `E5` | entry-id | COPY:270 | `meeting.errors.noLoopback.*`, `meeting.session.micOnly.badge` | vs. RC `LoopbackUnavailable` (F-08) |
| `E6` | entry-id | COPY:283 | `meeting.errors.exclusiveMode.*` | vs. RC `SharedModeRequired`/`LoopbackSharedModeRequired` (F-07) |
| `E7` | entry-id | COPY:294 | `meeting.warnings.diskLow.*` | vs. RC `DiskSpaceLow`/`DISK_LOW_WATERMARK_BYTES` (F-09) |
| `E8` | entry-id | COPY:304 | `meeting.errors.diskFull.*` | vs. RC `StopReason::DiskFull`/`ControlledStopInitiated` (F-09) |
| `E9` | entry-id | COPY:317 | `meeting.errors.cloudUnreachable.*` | job state `retry_wait` ✓ STT |
| `E10` | entry-id | COPY:331 | `meeting.errors.cloudConfig.*` | STT err-class `permanent` ✓ |
| `E11` | entry-id | COPY:342 | `meeting.warnings.backlog.*` | vs. RC `backlog_chunks_ready` (F-14) |
| `E12` | entry-id | COPY:351 | `meeting.recovery.*` | ✓ RC §8 recovery protocol, `MeetingRecoveryBanner` |
| `E13` | entry-id | COPY:365 | `meeting.errors.slept.*` | ✓ RC `SleepWake` |
| `E14` | entry-id | COPY:377 | `meeting.warnings.devicesChanged.*` | ✓ RC §6 resume re-validation |
| `E15` | entry-id | COPY:390 | `meeting.conflicts.dictation.*` | PLAN §6.10 |
| `E16` | entry-id | COPY:399 | `meeting.errors.missingKey.*` | PLAN Startvalidierung; explicitly non-recorder (RC 813) |
| `E17` | entry-id | COPY:410 | `meeting.errors.micPermission.*` | ✓ RC `CapturePermissionDenied` |
| `P1` | entry-id | COPY:429 | `meeting.consent.beforeStart.*` | consent seam owned by task-0042 (SSC §7) |
| `P2` | entry-id | COPY:441 | `meeting.consent.indicator.label` | also embedded in S3 (126) |
| `P3` | entry-id | COPY:449 | `meeting.cloud.notice.azure.*` | — |
| `P4` | entry-id | COPY:466 | `meeting.cloud.active.azureProcessing` | same key already defined in S6 (169) — F-15 |
| `P5` | entry-id | COPY:476 | `meeting.cloud.notice.llm.*`, `meeting.cloud.active.llm` | `meeting.cloud.active.llm` duplicated with S7 (183) — F-15 |
| `P6` | entry-id | COPY:492 | `meeting.retention.question.*` | — |
| `P7` | entry-id | COPY:509 | `meeting.stopConfirm.*` | PLAN §6.9 shortcut confirmation |
| `P8` | entry-id | COPY:519 | `meeting.delete.level1.*` | PLAN §11 deletion levels |
| `P9` | entry-id | COPY:528 | `meeting.delete.level2.*` | — |
| `P10` | entry-id | COPY:537 | `meeting.delete.level3.*` | PLAN §11/§6.14 |

### UX copy catalog (COPY) — reusable action keys (§2.3) and inline action keys

Inline keys (marked below) are outside the shared §2.3 table — see F-16.

| Identifier | Kind | Defining doc:line | Planned symbol | Notes / conflicts |
| --- | --- | --- | --- | --- |
| `meeting.actions.tryAgain` | i18n-key | COPY:64 | TS i18n key | reused by S9, E9, E10, E16 |
| `meeting.actions.switchDevice` | i18n-key | COPY:65 | TS i18n key | German contract wording "Gerät wechseln" (RC 793) ✓ concept |
| `meeting.actions.openSettings` | i18n-key | COPY:66 | TS i18n key | — |
| `meeting.actions.openSystemSettings` | i18n-key | COPY:67 | TS i18n key | E17 primary |
| `meeting.actions.reconnect` | i18n-key | COPY:68 | TS i18n key | E1/E2 primary; equals `tryAgain` role per COPY 232 |
| `meeting.actions.recordMicOnly` | i18n-key | COPY:69 | TS i18n key | E5; RC fallback "Nur Mikrofon aufnehmen" (810) ✓ |
| `meeting.actions.continueWithoutSystemAudio` | i18n-key | COPY:70 | TS i18n key | E2; maps to `close_source(System)` (RC 794) ✓ |
| `meeting.actions.keepRecording` | i18n-key | COPY:71 | TS i18n key | E4, E7, P7 |
| `meeting.actions.endMeeting` | i18n-key | COPY:72 | TS i18n key | S3, S4, E7, P7 |
| `meeting.actions.pause` | i18n-key | COPY:73 | TS i18n key | S3 |
| `meeting.actions.resume` | i18n-key | COPY:74 | TS i18n key | S4, E13 |
| `meeting.actions.workInBackground` | i18n-key | COPY:75 | TS i18n key | S6, S7, E9, E10, E11 |
| `meeting.actions.retryFailedSegments` | i18n-key | COPY:76 | TS i18n key | S6; triggers STT `failed → queued` (STT 149) ✓ |
| `meeting.actions.finishDictation` | i18n-key | COPY:77 | TS i18n key | E15 |
| `meeting.actions.cancelMeetingStart` | i18n-key | COPY:78 | TS i18n key | E15 |
| `meeting.actions.openMeeting` | i18n-key | COPY:79 | TS i18n key | S8, S10, E8 |
| `meeting.actions.deleteRecording` | i18n-key | COPY:80 | TS i18n key | S10, E12; routes to P8–P10 |
| `meeting.actions.cancel` | i18n-key | COPY:81 | TS i18n key | S1, E16, E17, P1, P8–P10 |
| `meeting.actions.close` | i18n-key | COPY:82 | TS i18n key | S8 |
| `meeting.actions.confirmDelete` | i18n-key | COPY:83 | TS i18n key | P10 final confirmation text |
| `meeting.actions.startMeeting` | i18n-key (inline) | COPY:102 | TS i18n key | S1 |
| `meeting.actions.testSystemAudio` | i18n-key (inline) | COPY:266 | TS i18n key | E4; RC 796 "Systemaudio testen" ✓ concept, key absent from §2.3 table (scheme inconsistency, minor) |
| `meeting.actions.recover` | i18n-key (inline) | COPY:358 | TS i18n key | E12 |
| `meeting.actions.decideLater` | i18n-key (inline) | COPY:359 | TS i18n key | E12, P6 |
| `meeting.actions.resumeWithDevices` | i18n-key (inline) | COPY:385 | TS i18n key | E14 |
| `meeting.actions.useLocal` | i18n-key (inline) | COPY:459 | TS i18n key | P3 |
| `meeting.actions.useCloudSummary` | i18n-key (inline) | COPY:484 | TS i18n key | P5 |
| `meeting.actions.summarizeLocally` | i18n-key (inline) | COPY:485 | TS i18n key | P5 |
| `meeting.actions.disableSummary` | i18n-key (inline) | COPY:486 | TS i18n key | P5 |

---

## Coverage statement (gate G1)

Extracted and registered:

- **PLAN Session-Zustände block:** 11 of 11 state names (lines 880–889).
- **RC:** all 10 enums — 56 of 56 variants (`SourceKind` 2, `MarkerKind` 2,
  `ChunkState` 4, `PreflightOutcome` 4, `GapClass` 3, `SourceState` 4,
  `StopReason` 3, `RecorderEvent` 17, `RecorderError` 12, `RecorderState` 5),
  plus 17 supporting non-enum types/traits and 13 named constants (5 defined
  values + 8 pending task-0008 placeholders).
- **STT:** all 6 job states, both error classes (`permanent`, `transitory`),
  the trait concept, 3 provider impls, 3 interface operations, 6 result/input
  types, and the persisted value tokens
  (`microphone`/`system`, `local|azure|custom`, `idempotency_key`,
  `requeue_generation`).
- **COPY:** the complete entry-id scheme — S1–S10, E1–E17, P1–P10 (37 entry
  ids) plus all 20 reusable action keys (§2.3) and 8 inline action keys.

Nothing else in the four inputs introduces a named enum variant, state name,
or catalog id. Constants whose *names* are pending (e.g. deadband width,
RC 733–735) are described in RC prose but carry no identifier yet and are
therefore not registrable.

## Gate self-check

| Gate | Evidence |
| --- | --- |
| G1 — 100 % coverage with file:line | Coverage statement above; every row carries `doc:line` verified against the worktree files on 22.08.2026 |
| G2 — all conflicts listed as findings | Findings F-01…F-16, each with ≥ 2 citations; verified-consistent list prevents silent drops in the other direction |
| G3 — registry, not definition | Header status paragraph and precedence rule |
| G4 — no other file modified | Verified via `git status --porcelain` after authoring; recorded in the mission result |
