# Unified Error Taxonomy ↔ i18n Copy Mapping Contract — Meeting Mode

Status: **spec contract — no implementation yet** (Phase 2+ prerequisite)
Source of truth: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) §6.2
(UX-Prinzipien, Regel 8: "Fehler erklären die nächste Handlung"), §6.8
(Farb-/Aktionslogik), "Fehlerverhalten" and "Startvalidierung" blocks,
§8 (Transkriptionspipeline), §12 (Datenschutz).
Sibling contracts consumed read-only below, abbreviated:

- **RC** — [recorder-contract.md](recorder-contract.md)
- **SSC** — [session-state-contract.md](session-state-contract.md)
- **STT** — [stt-provider-contract.md](stt-provider-contract.md)
- **SPC** — [summary-provider-contract.md](summary-provider-contract.md)
- **COPY** — [../meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md)
  (proposal-only; remains the sole authority for key _text_)
- **REG** — [identifier-registry.md](identifier-registry.md)

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.
Where this contract assigns copy keys, the catalog stays the authority for the
_wording_; this contract is the authority for the _code ↔ key binding_.

---

## 1. Purpose and seam ownership

Every meeting-mode subsystem reports failures today through its own vocabulary:
`RecorderError` variants (RC §4), `RecorderEvent` failure cases (RC §7.1),
session-state failure transitions (SSC §2.2), `ProviderError` categories
(STT §6), and the summary terminal class `summary_no_path_available`
(SPC §6). The frontend, however, must render exactly one thing: localized copy
with a concrete next action (plan §6.2 rule 8).

This contract defines the **single seam** between the two worlds:

```text
Backend subsystems                     Frontend (React/i18next)
RecorderError ─┐
RecorderEvent ─┤                        MeetingErrorEnvelope {code, params}
ProviderError ─┼─► adapter: to_envelope() ──► taxonomy table lookup
SSC last_error ┘   (one code per fault)        │
                                               ├─► i18n copy key (existing or proposed)
                                               ├─► severity (info | warning | error)
                                               └─► action class (buttons per COPY §2.3)
```

Normative claims:

1. There is **exactly one** machine error-code namespace for all meeting-mode
   subsystems (`meeting.<origin>.<name>`, §3).
2. Raw error strings never cross the Tauri boundary (§2).
3. Every named failure in RC §7 and in the SSC failure behaviors maps to
   exactly one code, and every code maps to at least one copy-catalog key
   (§5, §7 — zero unmapped entries).
4. Retryability of STT-origin codes is inherited verbatim from STT §6
   (§4).

This contract resolves the _mapping half_ of REG findings F-07, F-08, F-09,
F-10 and F-14 (which technical name binds to which user-facing key) without
editing those documents; REG remains the inventory owner.

## 2. The Tauri-boundary envelope rule (gate G3)

### 2.1 Wire format

Every meeting-domain Tauri command error and every error-bearing event payload
uses exactly this shape:

```jsonc
{
  "code": "meeting.recorder.system_device_lost",
  "params": { "source": "system", "segmentId": "42" },
}
```

- `code`: a registered code from §5 (string, stable, machine-comparable).
- `params`: flat `record<string, string | number>` of interpolation values,
  consumed exclusively as i18next `{{param}}` placeholders. No nested
  objects, no arrays, no message fields.

Rust side (normative signature intent):

```rust
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct MeetingErrorEnvelope {
    pub code: String, // constant from the generated code table (§5)
    pub params: std::collections::BTreeMap<String, String>,
}
```

Event embedding: `meeting://session-error` (SSC §2.2) carries the envelope in
an `error` field next to `meeting_id`/`state_revision`; all other
`meeting://*` topics stay error-free except where §5 marks a state event as
carrying one.

### 2.2 Untagged-string prohibition

- `std::fmt::Display` output of `RecorderError`, `ProviderError` and friends
  is **log-and-DB only**. It may populate `meeting_sessions.last_error` and
  log lines (redacted per the `compact_error` practice, STT §7) but never a
  command result or event payload.
- No meeting command returns `Result<_, String>`; no meeting event carries a
  bare `message`/`error` string field.
- Human-readable fragments that the UI genuinely needs verbatim (device
  names, counts, durations) travel **only** as named `params` values, always
  referenced by a key. A payload without `code` is a contract violation.
- Secrets never appear in `code` or `params` (extends the STT §7 invariant;
  consistent with SSC O7).

### 2.3 Frontend consumption

- One proposed module, `src/lib/meetingErrors.ts`, contains the generated
  lookup `code -> { keys, severity, actionClass, retryability }` mirroring §5.
  Components branch on `severity`/`actionClass` and render
  `t(key, params)`; they never parse or display `code` or any raw string.
- **Unknown-code fallback:** a code not present in the table renders
  `meeting.errors.generic.*` (new key, §6) with `actionClass: "repair"` and
  logs a development-mode warning. The taxonomy is closed for producers but
  forward-compatible for consumers.

### 2.4 Key selection by parameter

A few codes legitimately render different existing keys depending on a
declared parameter (e.g. `source`, `followMode`). This is allowed **only**
where §5 names the selector and the candidate keys; ad-hoc branching on other
param values is forbidden.

## 3. Code naming scheme and collision prevention (gate G2)

Format: **`meeting.<origin>.<snake_case_name>`**

1. `<origin>` is from the **closed set** `{ recorder, session, stt, summary }`.
   A new subsystem (e.g. `import`, `project`) must extend this set in §3 and
   add its rows to §5 of this document first — one reviewable edit — before
   its codes may be sent.
2. `<snake_case_name>` is lowercase `snake_case`, unique within its origin,
   ASCII letters/digits/underscore only, matching `[a-z][a-z0-9_]*`.
3. Codes are **stable once shipped**: renaming a code is a breaking change
   requiring a migration note here; deprecation keeps the old code rendering
   the generic fallback rather than reusing the name.
4. Provider-specific signals (HTTP status, Azure error subcodes, WASAPI HRESULTs)
   are translated **inside** the owning adapter into taxonomy codes; they are
   never forwarded as codes or as `code`-like params (STT §1.1: the queue sees
   no HTTP status codes; likewise the frontend sees no provider internals).
   Distinct provider faults with identical user remedy share one code and
   distinguish themselves via `params.provider`.
5. Registration duty: every new code is added to the §5 table **and** flagged
   for the next REG inventory pass (REG is read-only for this task). A code
   present in code but absent from §5 is a defect; a test hook is specified
   in §8.

## 4. Severity, action class, retryability

### 4.1 Severity (drives color/icon per plan §6.8; never color alone)

| Value     | Meaning                                                       |
| --------- | ------------------------------------------------------------- |
| `info`    | State notice; no fault (e.g. default-device change handled).  |
| `warning` | Attention/degraded, orange; capture or processing continues.  |
| `error`   | Progress blocked or stopped; data preserved; action required. |

### 4.2 Action class (plan §6.2 rule 8: every error explains the next action)

| Value      | Buttons (all from COPY §2.3 unless noted)                                                                        |
| ---------- | ---------------------------------------------------------------------------------------------------------------- |
| `none`     | passive indicator only                                                                                           |
| `retry`    | `tryAgain` / `reconnect` / `retryFailedSegments`                                                                 |
| `repair`   | focused repair dialog: `switchDevice`, `openSettings`, `openSystemSettings`, `recordMicOnly`                     |
| `fallback` | alternative path offered: `continueWithoutSystemAudio`, `keepRecording`, `workInBackground`, `resumeWithDevices` |
| `block`    | start/primary action disabled until repaired (Startvalidierung)                                                  |

A row may combine (e.g. `repair` + `fallback`); the primary class is listed
first.

### 4.3 Retryability — inherited verbatim from STT §6 (G4 anchor)

| Table value                 | Semantics                                                                                                                                         | Source   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| `never`                     | permanent classification; automatic retry is pointless; only an explicit user re-queue creates a fresh job (STT §3.2 `failed → queued`)           | STT §6.1 |
| `auto_backoff`              | transitory classification; bounded exponential backoff per STT §3.5 while `attempt_count < max_attempts`; afterwards `failed` + manual retry hint | STT §6.2 |
| `conservative_auto_backoff` | unclassifiable faults treated as transitory (better a wasted attempt than a lost segment) until classification is refined                         | STT §6.3 |
| `n/a`                       | recorder/session-origin conditions; retry is a user gesture (Reconnect/Resume/Try again), never an automatic loop                                 | RC §7    |

Recorder- and session-origin codes are **never** auto-retried by background
machinery; their retry affordances are the visible buttons of the mapped
action class.

## 5. Master mapping table (gates G1 + G3)

Columns: **Code** · defined by (source surface) · Sev · Action · Auto-retry ·
**Copy key(s)** — `existing` = already proposed in COPY; `NEW` = newly
proposed here for the catalog owner (§6). Key paths sit under the catalog's
`meeting.*` convention (COPY §2.2).

### 5.1 Origin `recorder` — `RecorderError` variants (RC:409–433)

| Code                                             | Defined by                                              | Sev     | Action                                 | Auto-retry                                         | Copy key(s)                                                                                                                            | Params                  |
| ------------------------------------------------ | ------------------------------------------------------- | ------- | -------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| `meeting.recorder.mic_unavailable`               | `MicDeviceUnavailable` RC:411; RC §7.2 row 1            | error   | block, repair                          | n/a                                                | NEW `meeting.errors.deviceUnavailable.title` + `.body.mic`                                                                             | `deviceName`            |
| `meeting.recorder.system_output_unavailable`     | `SystemOutputUnavailable` RC:413; RC §7.2 row 2         | error   | block, repair                          | n/a                                                | NEW `.body.system` of the same family                                                                                                  | `deviceName`            |
| `meeting.recorder.loopback_shared_mode_required` | `LoopbackSharedModeRequired` RC:416; RC §7.2 row 3      | error   | repair, fallback                       | n/a                                                | existing `meeting.errors.exclusiveMode.*` (E6; wording bridge for REG F-07)                                                            | `deviceName`            |
| `meeting.recorder.capture_permission_denied`     | `CapturePermissionDenied` RC:418; RC §7.2 row 5         | error   | repair (`openSystemSettings`)          | n/a                                                | existing `meeting.errors.micPermission.*` (E17); selector `params.source`: `microphone` → existing body, `system` → NEW `.body.system` | `source`                |
| `meeting.recorder.loopback_unavailable`          | `LoopbackUnavailable` RC:421; RC §7.2 row 4             | error   | fallback (`recordMicOnly`), repair     | n/a                                                | existing `meeting.errors.noLoopback.*`, `meeting.session.micOnly.badge` (E5; REG F-08 bridge)                                          | —                       |
| `meeting.recorder.insufficient_disk_space`       | `InsufficientDiskSpace` RC:423; RC §7.2 row 6           | warning | block (start disabled, RC:812), repair | n/a                                                | existing `meeting.warnings.diskLow.*` (E7)                                                                                             | `freeGb`, `requiredGb`  |
| `meeting.recorder.chunk_create_failed`           | `ChunkCreateFailed` RC:425; RC §3.1 property 3          | error   | fallback (`openMeeting`)               | n/a                                                | selector `params.reason`: `disk_full` → existing `meeting.errors.diskFull.*` (E8); otherwise NEW `meeting.errors.storageWriteFailed.*` | `source`, `reason`      |
| `meeting.recorder.chunk_state_persist_failed`    | `ChunkStatePersistFailed` RC:427; feeds SSC T17 example | error   | retry, fallback                        | n/a (single retry already done in RC §3.1 prop. 2) | existing `meeting.errors.recoverable.*` (S9)                                                                                           | `chunkId`               |
| `meeting.recorder.stream_error`                  | `StreamError` RC:429; mid-session stream fault          | warning | retry (`reconnect`), repair            | n/a                                                | selector `params.source`: `microphone` → existing `meeting.errors.micLost.*` (E1), `system` → `meeting.errors.systemLost.*` (E2)       | `source`, `detailCode?` |
| `meeting.recorder.session_already_active`        | `SessionAlreadyActive` RC:430 (guard)                   | error   | none (dev-facing guard)                | n/a                                                | NEW `meeting.errors.actionUnavailable.*`                                                                                               | —                       |
| `meeting.recorder.no_active_session`             | `NoActiveSession` RC:431 (guard)                        | error   | none                                   | n/a                                                | NEW `meeting.errors.actionUnavailable.*` (shared)                                                                                      | —                       |
| `meeting.recorder.recovery_failed`               | `RecoveryFailed` RC:432; RC §8                          | error   | retry (`recover`), repair              | n/a                                                | existing `meeting.errors.recoverable.*` (S9) + `meeting.recovery.*` (E12) in banner context                                            | `chunkId`               |

### 5.2 Origin `recorder` — failure-bearing `RecorderEvent`s and stop reasons (RC §7.1)

| Code                                                  | Defined by                                                                                                    | Sev     | Action                                                 | Auto-retry | Copy key(s)                                                                                                                      | Params                          |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------- | ------------------------------------------------------ | ---------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| `meeting.recorder.mic_device_lost`                    | `MicDeviceLost` RC:385; RC §7.1 row 1                                                                         | warning | retry (`reconnect`), repair, fallback                  | n/a        | existing `meeting.errors.micLost.*` (E1)                                                                                         | `segmentId`, `atTimelineMs`     |
| `meeting.recorder.system_device_lost`                 | `SystemDeviceLost` RC:386; RC §7.1 row 2                                                                      | warning | retry, fallback (`continueWithoutSystemAudio`), repair | n/a        | existing `meeting.errors.systemLost.*` (E2)                                                                                      | `segmentId`, `atTimelineMs`     |
| `meeting.recorder.default_device_changed`             | `DefaultDeviceChanged` RC:387; RC §7.1 row 3                                                                  | info    | none / fallback                                        | n/a        | selector `params.followMode`: `bound` → existing `meeting.errors.defaultChanged.*` (E3), `followed` → `.defaultChanged.followed` | `observedDefault`, `followMode` |
| `meeting.recorder.disk_space_low`                     | `DiskSpaceLow` RC:393; RC §7.1 row 5 early warning                                                            | warning | fallback (`keepRecording`, `endMeeting`), repair       | n/a        | existing `meeting.warnings.diskLow.*` (E7)                                                                                       | `freeBytes`                     |
| `meeting.recorder.controlled_stop_disk_full`          | `ControlledStopInitiated { DiskFull }` RC:394 with `StopReason::DiskFull` RC:345; RC §7.1 row 5               | error   | fallback (`openMeeting`), repair                       | n/a        | existing `meeting.errors.diskFull.*` (E8)                                                                                        | —                               |
| `meeting.recorder.controlled_stop_fatal_stream_error` | `ControlledStopInitiated { FatalStreamError }` RC:346                                                         | error   | fallback (`openMeeting`), repair                       | n/a        | NEW `meeting.errors.captureStoppedUnexpectedly.*`                                                                                | `source?`                       |
| `meeting.recorder.sleep_wake_interrupted`             | `SleepWake` RC:390; RC §7.1 row 9                                                                             | warning | fallback (`resume`, `endMeeting`)                      | n/a        | existing `meeting.errors.slept.*` (E13)                                                                                          | `gapCount`                      |
| `meeting.recorder.capture_degraded`                   | `DriftToleranceExceeded` RC:378 / `SourceState::Degraded` (beyond-G1 completeness; §7.1 has no dedicated row) | info    | none                                                   | n/a        | NEW `meeting.warnings.degradedCapture.*`                                                                                         | `source`                        |

Explicit non-errors preserved from RC §7.1: **"Keine Systemlautstärke" is not
a fault** (silence is legitimate, RC:796) — it surfaces through the setup test
flow with existing `meeting.warnings.silence.*` (E4) bound to the _test_
outcome, not to an error code. **Backlog growth** ("Lokales Modell zu langsam",
RC §7.1 row 7) is likewise state-derived information, not an error: the
existing `meeting.warnings.backlog.*` (E11) binds to
`snapshot().sources[*].backlog_chunks_ready` (REG F-14 bridge), not to a code.

### 5.3 Origin `session` — SSC failure behaviors

| Code                                          | Defined by                                                                                                                                                            | Sev     | Action                                                                   | Auto-retry | Copy key(s)                                                                                                                                                         | Params                                   |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------ | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| `meeting.session.start_validation_failed`     | SSC T04 (validation fails)                                                                                                                                            | error   | block                                                                    | n/a        | **Composition:** publishes the _child_ code (recorder §5.1 / stt §5.4 cause); this code itself is a fallback rendering existing `meeting.errors.missingKey.*` (E16) | `causeCode`                              |
| `meeting.session.resume_validation_failed`    | SSC §3.2 step 4 (resume re-validation fails, session stays `paused`)                                                                                                  | warning | fallback (`resumeWithDevices`), repair                                   | n/a        | context existing `meeting.warnings.devicesChanged.*` (E14) + published child code                                                                                   | `micDevice`, `outputDevice`, `causeCode` |
| `meeting.session.consent_gate_refused`        | SSC §7 gate point G-start, fail-closed refusal of T02                                                                                                                 | error   | none (fail-closed; no bypass)                                            | n/a        | NEW `meeting.errors.consentRequired.*`                                                                                                                              | —                                        |
| `meeting.session.recovery_disposition_failed` | SSC §6.2 R5(c) (recovery failed irrecoverably)                                                                                                                        | error   | retry (`recover`), repair                                                | n/a        | existing `meeting.errors.recoverable.*` (S9) + `meeting.recovery.*` (E12)                                                                                           | `sessionId`                              |
| `meeting.session.stt_jobs_exhausted`          | SSC T13 bullet 1 (jobs exhausted retries → `failed`)                                                                                                                  | error   | retry (`retryFailedSegments`), fallback (`workInBackground`)             | user-only  | NEW `meeting.errors.segmentsFailed.*`                                                                                                                               | `failedCount`, `totalCount`              |
| `meeting.session.summary_no_path_available`   | SPC §6 error class `summary_no_path_available`; SSC T13 bullet 2                                                                                                      | error   | retry (`tryAgain`), repair (`openSettings`), fallback (`disableSummary`) | user-only  | existing-as-proposed `meeting.summary.failed.*` (owned by SPC §6.2; selector `params.reason` → `.reason.noPath` / `.reason.consentWithdrawn`)                       | `reason`                                 |
| `meeting.session.recoverable_error_generic`   | SSC T17 overlay when a cause arrives **without** its own code (defensive totality; must stay empty if §5 is complete — a hit is a defect filed against this contract) | error   | retry, repair                                                            | n/a        | existing `meeting.errors.recoverable.*` (S9)                                                                                                                        | —                                        |

User-abort terminals `stopped_unprocessed`/`cancelled` (SSC T15/T16) are
deliberate user actions, not faults: they carry **no error code** and render
through existing `meeting.session.cancelled.*` (COPY S10) directly.

### 5.4 Origin `stt` — `ProviderError` categories (STT §6; retryability per §4.3)

Permanent (STT §6.1 — all `never`):

| Code                                  | Covers STT §6.1 rows                                                                                                            | Sev   | Action                                  | Copy key(s)                                              | Params                  |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ----- | --------------------------------------- | -------------------------------------------------------- | ----------------------- |
| `meeting.stt.auth_invalid`            | Azure key invalid/revoked (401/403); Custom token wrong (401/403)                                                               | error | repair (`openSettings`)                 | existing `meeting.errors.cloudConfig.*` (E10)            | `provider`              |
| `meeting.stt.endpoint_invalid`        | Azure endpoint/region wrong (400/404/region resolution); Custom endpoint wrong (404/400/405)                                    | error | repair                                  | existing `meeting.errors.cloudConfig.*` (E10)            | `provider`              |
| `meeting.stt.provider_not_configured` | validate(): Secret/Endpoint missing before job start (Azure + Custom row)                                                       | error | block, repair                           | existing `meeting.errors.missingKey.body.cloud` (E16)    | `provider`              |
| `meeting.stt.package_limit_exceeded`  | Azure format/limit violated (HTTP 415 or internal 400 MB/4 h caps, STT §4.2)                                                    | error | repair                                  | NEW `meeting.errors.audioPackageRejected.*`              | `provider`, `limitKind` |
| `meeting.stt.model_missing`           | Local model missing/not loadable                                                                                                | error | block, repair                           | existing `meeting.errors.missingKey.body.local` (E16)    | `modelId`               |
| `meeting.stt.language_unsupported`    | Provider rejects the language                                                                                                   | error | repair                                  | NEW `meeting.errors.languageUnsupported.*`               | `language`, `provider`  |
| `meeting.stt.source_chunk_damaged`    | queued job whose chunk turned out `damaged` — failed by pre-run readiness validation as permanent (SSC §6.3; RC §8 steps 2b/2c) | error | retry (`retryFailedSegments`), fallback | NEW `meeting.errors.segmentsFailed.*` (shared with §5.3) | `chunkId`               |

Transitory (STT §6.2 — all `auto_backoff`, bounded by STT §3.5):

| Code                                 | Covers STT §6.2 rows                                                                 | Sev     | Action                               | Copy key(s)                                         | Params                       |
| ------------------------------------ | ------------------------------------------------------------------------------------ | ------- | ------------------------------------ | --------------------------------------------------- | ---------------------------- |
| `meeting.stt.network_unreachable`    | connection/DNS/refused-reset                                                         | warning | fallback (`workInBackground`), retry | existing `meeting.errors.cloudUnreachable.*` (E9)   | `provider`, `waitingCount`   |
| `meeting.stt.provider_overloaded`    | HTTP 408/429 (`Retry-After` respected)/5xx                                           | warning | same as above                        | existing `meeting.errors.cloudUnreachable.*` (E9)   | `provider`, `retryInSeconds` |
| `meeting.stt.attempt_timeout`        | attempt exceeded STT §3.5 timeout                                                    | warning | same as above                        | existing `meeting.errors.cloudUnreachable.*` (E9)   | `provider`                   |
| `meeting.stt.tls_disrupted`          | handshake/certificate-time faults (durable invalid certificate = `endpoint_invalid`) | warning | same as above                        | existing `meeting.errors.cloudUnreachable.*` (E9)   | `provider`                   |
| `meeting.stt.local_engine_transient` | intercepted inference panic/OOM-like fault of a single segment with valid model      | warning | fallback (`workInBackground`), retry | NEW `meeting.warnings.localTranscriptionRetrying.*` | `attempt`, `maxAttempts`     |

Unclassifiable (STT §6.3):

| Code                       | Meaning                                                       | Sev     | Action          | Auto-retry                  | Copy key(s)                                                                                                                                            |
| -------------------------- | ------------------------------------------------------------- | ------- | --------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `meeting.stt.unclassified` | fault the provider cannot classify; conservatively transitory | warning | fallback, retry | `conservative_auto_backoff` | selector `params.provider`: `local` → NEW `meeting.warnings.localTranscriptionRetrying.*`; otherwise existing `meeting.errors.cloudUnreachable.*` (E9) |

While any `retry_wait` exists, capture continues untouched and no segment is
lost (plan Fehlerverhalten; STT §6.2 closing rule) — the envelope informs the
processing checklist, it never interrupts recording.

## 6. Newly proposed copy keys (hand-off to the catalog owner)

All NEW keys follow COPY §2.2 conventions (top-level `meeting` section,
English source file first, `{{…}}` interpolation). Text drafts are
deliberately **not** invented here — wording stays owned by the catalog;
this section fixes only key paths and consumption. Proposed skeleton:

| Family                                          | Needed keys                        | Consumed by codes                           |
| ----------------------------------------------- | ---------------------------------- | ------------------------------------------- |
| `meeting.errors.deviceUnavailable.*`            | `title`, `body.mic`, `body.system` | mic/system_output_unavailable               |
| `meeting.errors.storageWriteFailed.*`           | `title`, `body`                    | chunk_create_failed (non-disk-full)         |
| `meeting.errors.actionUnavailable.*`            | `title`, `body`                    | session_already_active, no_active_session   |
| `meeting.errors.captureStoppedUnexpectedly.*`   | `title`, `body`                    | controlled_stop_fatal_stream_error          |
| `meeting.errors.consentRequired.*`              | `title`, `body`                    | consent_gate_refused                        |
| `meeting.errors.segmentsFailed.*`               | `title`, `body`                    | stt_jobs_exhausted, source_chunk_damaged    |
| `meeting.errors.audioPackageRejected.*`         | `title`, `body`                    | package_limit_exceeded                      |
| `meeting.errors.languageUnsupported.*`          | `title`, `body`                    | language_unsupported                        |
| `meeting.warnings.degradedCapture.*`            | `title`, `body`                    | capture_degraded                            |
| `meeting.warnings.localTranscriptionRetrying.*` | `title`, `body`                    | local_engine_transient, unclassified(local) |
| `meeting.errors.generic.*`                      | `title`, `body`                    | unknown-code fallback (§2.3)                |

Buttons for every family come exclusively from COPY §2.3 per the action-class
column — no new action keys are proposed. This keeps the complexity budget:
eleven small families instead of per-code bespoke entries.

## 7. Coverage proof (gate G1 — zero unmapped entries)

Inventories verified against the worktree files on 22.08.2026; every entry
cites its defining line and its mapping row above.

**A. All 12 `RecorderError` variants** (RC:411–432): `MicDeviceUnavailable` →
§5.1 r1; `SystemOutputUnavailable` → r2; `LoopbackSharedModeRequired` → r3;
`CapturePermissionDenied` → r4; `LoopbackUnavailable` → r5;
`InsufficientDiskSpace` → r6; `ChunkCreateFailed` → r7;
`ChunkStatePersistFailed` → r8; `StreamError` → r9; `SessionAlreadyActive` →
r10; `NoActiveSession` → r11; `RecoveryFailed` → r12. **12/12 mapped.**

**B. All 9 RC §7.1 rows** (RC:793–801): row 1 → §5.2 `mic_device_lost`;
row 2 → `system_device_lost`; row 3 → `default_device_changed`; row 4
("Keine Systemlautstärke") is a documented non-error (§5.2 closing note);
row 5 → `disk_space_low` and/or `controlled_stop_disk_full`; rows 6–7
(provider unreachable / local too slow) route to the §5.4 transitory codes
and the state-bound E11 copy respectively; row 8 (App-Absturz) →
`meeting.recorder.recovery_failed` (§5.1) plus
`meeting.session.recovery_disposition_failed` (§5.3); row 9 →
`sleep_wake_interrupted`. **9/9 resolved (7 coded, 2 documented
non-errors).**

**C. All 7 RC §7.2 rows** (RC:807–813): six map to §5.1 codes; row 7
(provider/model/Azure-Custom/summary/retention validated by MeetingManager)
routes to `meeting.stt.provider_not_configured`, `meeting.stt.model_missing`,
and SPC §6 respectively. Preflight outcomes (`PreflightOutcome::Missing`,
`::PermissionDenied`, `::SharedModeRequired`, RC:289–295) are start-time views
of the same faults and therefore reuse the code of their §5.1 counterpart row;
`::Available` is not a fault and carries no code. **7/7 resolved.**

**D. SSC failure behaviors**: T04 → §5.3 (composition); T13 bullet 1 →
`meeting.session.stt_jobs_exhausted`; T13 bullet 2 →
`meeting.session.summary_no_path_available`; T17 → cause code or
`meeting.session.recoverable_error_generic`; R4 damaged-chunk routing →
`meeting.stt.source_chunk_damaged`; R5(c) →
`meeting.session.recovery_disposition_failed`; §3.2 step 4 resume-validation
failure → `meeting.session.resume_validation_failed`; §7 consent refusal →
`meeting.session.consent_gate_refused`; O7 payload hygiene → adopted and
tightened by §2.2. T15/T16 excluded as user actions (§5.3 closing note).
**8/8 mapped + 2 documented exclusions.**

**E. STT §6.1 (9 rows), §6.2 (5 rows), §6.3 (1 row)**: mapped 1:1 onto the
§5.4 tables (row-level citations in-table); SPC §6 class
`summary_no_path_available` → `meeting.session.summary_no_path_available`.
**15/15 + 1 mapped.**

No named error in RC §7 or in the SSC failure behaviors lacks a row above;
no row above lacks at least one copy key (existing or NEW §6).

## 8. Verification hooks (maps to plan §14 discipline)

1. **Totality test (Rust):** the adapter functions
   `impl From<RecorderError> for MeetingErrorEnvelope` etc. are `match`-
   exhaustive over the source enums; adding a variant without a mapping is a
   compile error.
2. **Table/code agreement test:** every code constant emitted anywhere must
   appear in the generated table consumed by `src/lib/meetingErrors.ts`
   (single generated source, e.g. one TS constant file produced from the same
   Rust definitions); a mismatch fails CI.
3. **Key-existence test:** for every table row, each referenced key must
   exist in `src/i18n/locales/en/translation.json` (extension of the existing
   `scripts/check-translations.ts` practice, COPY §8.4).
4. **Boundary-hygiene test:** serialization of every meeting command error
   and error-bearing event payload must deserialize to exactly
   `{code, params}`; a negative test asserts no known secret material appears
   (mirrors the STT §7 negative-test pattern).

## 9. Complexity budget / non-goals (gate G4)

Deliberately **not** specified, with reasons:

- **No provider-detail codes at the boundary** (no `azure_401`,
  no `wasapi_0x8889000A`): adapters translate; otherwise provider internals
  leak into UI logic and i18n (violates STT §1.1 and plan §6.2 principle 4).
- **No error inheritance/chaining framework**: a flat code + `causeCode`
  param covers the two composition sites; a hierarchy adds structure without
  user value.
- **No server/localization of errors backend-side**: copy lives only in
  i18next; the backend never ships sentences.
- **No telemetry/analytics pipeline, no error-reporting service integration**:
  out of v1 scope (plan §2).
- **No configurable/user-editable error messages, no per-code override
  store**: SAP-style configurability rejected; the catalog is the single
  copy authority.
- **No taxonomy for WhatsApp-import or project-intelligence faults yet**:
  those subsystems extend the §3 namespace (`meeting.import.*`,
  `meeting.project.*`) in their own contracts when they exist; pre-creating
  codes for unbuilt subsystems is speculative.
- **No change to the dictation path's existing error handling** (`errors.*`
  keys, `speech_pipeline.rs` behavior remain untouched).

Anything not in §5's tables is a non-goal until a defining contract adds it
via the §3 registration duty.

---

## Appendix A — Gate traceability

| Gate                                                                                                        | Coverage                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| G1 — every RC §7 and SSC failure-behavior error mapped, zero unmapped                                       | §5 tables + §7 inventories A–E with line citations                                                                           |
| G2 — collision-proof naming scheme registered                                                               | §3 (closed origin set, uniqueness, stability, registration duty, provider-signal ban)                                        |
| G3 — frontend receives `{code, params}` only; localization consumes keys, verified against catalog coverage | §2 (envelope, untagged-string prohibition, fallback), §5 key columns (existing vs NEW), §6 hand-off, §8.3 key-existence test |
| G4 — non-goals + documentation index updated                                                                | §9; index table row and reading-order entry added in `docs/meeting-mode-documentation-index.md`                              |
