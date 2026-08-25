# Meeting Mode — Error-Code → Operator-Action Crosswalk

Status: **lookup companion to a spec contract — nothing here describes
implemented behavior.** The error taxonomy itself is
[spec contract — no implementation yet](../meeting-mode-architecture/error-taxonomy-contract.md);
no `meeting.*` error code is emitted by any shipped code today (the
UX-Copy-Katalog, Abschnitt 1, verifies that no meeting-mode production code
exists). This document becomes the operator-facing lookup companion **once
errors ship**; until then every row is a contract reading, not a runtime fact.

Source of truth (authority order: Plan → contract → catalogs → code):

- **TAX** — [error-taxonomy-contract.md](../meeting-mode-architecture/error-taxonomy-contract.md)
  §5 master mapping table (authority for code ↔ key binding, severity,
  action class, retryability). This crosswalk is a *re-keyed view* of TAX §5
  for operators; it adds no codes and changes no mappings.
- **COPY** — [meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md)
  (proposal-only; authority for key *wording*). Entries marked `(E<n>)` /
  `(S<n>)` cite the catalog entry that owns the key text.
- **SPC** — [summary-provider-contract.md](../meeting-mode-architecture/summary-provider-contract.md)
  §6.2 (owner of the `meeting.summary.failed.*` keys).
- Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) §6.2 rule 8
  ("every error explains the next action"), §6.8, "Fehlerverhalten",
  "Startvalidierung".

---

## 1. How to read this document

### 1.1 Columns

| Column | Meaning | Authority |
| --- | --- | --- |
| Code | `meeting.<origin>.<name>` machine code, stable once shipped (TAX §3) | TAX §5 |
| Sev | `info` / `warning` / `error` per TAX §4.1 | TAX §5 |
| Action class | `none` / `retry` / `repair` / `fallback` / `block`, primary first (TAX §4.2) | TAX §5 |
| Copy key(s) | i18n key path(s) the frontend renders; **`existing`** = already proposed in COPY (entry cited), **`NEW`** = newly proposed by TAX §6 or SPC §6.2, **not yet in COPY** | TAX §5/§6, COPY, SPC |
| Operator action | What a support operator tells the user to do, in plain operational terms (§1.2 legend) | this document (derived from the action class + copy body) |

### 1.2 Operator-action legend

The operator-level actions used in the tables, in decreasing order of
specificity. "Operator action" always names the *next concrete step* for the
user (plan §6.2 rule 8) — it never names provider internals (TAX §3.4).

| Legend term | Operator instruction |
| --- | --- |
| **Retry** | User gesture only: press the visible button (`Try again`, `Retry failed segments`, `Recover and process`). Recorder/session codes are never auto-retried by background machinery (TAX §4.3). |
| **Wait for auto-retry** | Nothing to do; the queue retries with bounded backoff (STT §3.5) while capture continues untouched. No segment is lost. |
| **Reconnect device** | Replug/reselect the microphone or output device, then press `Reconnect` / `Resume`. |
| **Switch device** | Choose a different device in settings (exclusive-mode or unavailable device in use). |
| **Free disk space** | Free storage; the app blocks start below the threshold (Startvalidierung) or stops controllably at disk-full. |
| **Check key** | Verify the provider API key / endpoint / region in settings (`Open settings`). No audio was uploaded for permanent cloud-config errors. |
| **Download model** | Download the local meeting model via the model selector. |
| **Grant OS permission** | Grant microphone access in Windows/macOS privacy settings (`Open system settings`). |
| **Open meeting** | Data is preserved; open the meeting to inspect what was captured so far. |
| **None (passive)** | Informational state or dev-facing guard; no user action exists or is needed. |

---

## 2. Master crosswalk — origin `recorder`

### 2.1 `RecorderError` variants (TAX §5.1; RC:411–433)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 1 | `meeting.recorder.mic_unavailable` | error | block, repair | NEW `meeting.errors.deviceUnavailable.title` + `.body.mic` (TAX §6) | **Reconnect device** (mic); start stays blocked until repaired |
| 2 | `meeting.recorder.system_output_unavailable` | error | block, repair | NEW `meeting.errors.deviceUnavailable.body.system` (same family, TAX §6) | **Reconnect device** (system audio); start stays blocked |
| 3 | `meeting.recorder.loopback_shared_mode_required` | error | repair, fallback | existing `meeting.errors.exclusiveMode.*` (E6) | **Switch device** (output runs in exclusive mode); mic-only recording possible |
| 4 | `meeting.recorder.capture_permission_denied` | error | repair | existing `meeting.errors.micPermission.*` (E17); selector `params.source`: `microphone` → existing body, `system` → NEW `.body.system` (TAX §6) | **Grant OS permission**; start stays blocked |
| 5 | `meeting.recorder.loopback_unavailable` | error | fallback, repair | existing `meeting.errors.noLoopback.*` + `meeting.session.micOnly.badge` (E5) | **Switch device**, or continue **mic-only** (`Record microphone only`) |
| 6 | `meeting.recorder.insufficient_disk_space` | warning | block, repair | existing `meeting.warnings.diskLow.*` (E7) | **Free disk space**; start disabled below threshold (RC:812) |
| 7 | `meeting.recorder.chunk_create_failed` | error | fallback | selector `params.reason`: `disk_full` → existing `meeting.errors.diskFull.*` (E8); otherwise NEW `meeting.errors.storageWriteFailed.*` (TAX §6) | `disk_full` → **Free disk space**; other → **Open meeting** (data preserved), retry after storage is healthy |
| 8 | `meeting.recorder.chunk_state_persist_failed` | error | retry, fallback | existing `meeting.errors.recoverable.*` (S9) | **Retry** (one automatic persist retry already happened, RC §3.1 prop. 2) |
| 9 | `meeting.recorder.stream_error` | warning | retry, repair | selector `params.source`: `microphone` → existing `meeting.errors.micLost.*` (E1), `system` → `meeting.errors.systemLost.*` (E2) | **Reconnect device**, then `Reconnect` |
| 10 | `meeting.recorder.session_already_active` | error | none | NEW `meeting.errors.actionUnavailable.*` (TAX §6) | **None (passive)** — dev-facing guard; means two start commands raced |
| 11 | `meeting.recorder.no_active_session` | error | none | NEW `meeting.errors.actionUnavailable.*` (shared, TAX §6) | **None (passive)** — dev-facing guard; command hit without an active session |
| 12 | `meeting.recorder.recovery_failed` | error | retry, repair | existing `meeting.errors.recoverable.*` (S9) + `meeting.recovery.*` (E12) in banner context | **Retry** (`Recover and process`); if it persists, collect diagnostics |

### 2.2 Failure-bearing `RecorderEvent`s and stop reasons (TAX §5.2; RC §7.1)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 13 | `meeting.recorder.mic_device_lost` | warning | retry, repair, fallback | existing `meeting.errors.micLost.*` (E1) | **Reconnect device** (mic); system audio keeps recording |
| 14 | `meeting.recorder.system_device_lost` | warning | retry, fallback, repair | existing `meeting.errors.systemLost.*` (E2) | **Reconnect device** (output); or `Continue without system audio` |
| 15 | `meeting.recorder.default_device_changed` | info | none / fallback | selector `params.followMode`: `bound` → existing `meeting.errors.defaultChanged.*` (E3), `followed` → `.defaultChanged.followed` | **None (passive)** — recording continues on the new default; switch device only if the wrong one is captured |
| 16 | `meeting.recorder.disk_space_low` | warning | fallback, repair | existing `meeting.warnings.diskLow.*` (E7) | **Free disk space** soon, or `Keep recording` / `End meeting` deliberately |
| 17 | `meeting.recorder.controlled_stop_disk_full` | error | fallback, repair | existing `meeting.errors.diskFull.*` (E8) | **Free disk space**; **Open meeting** — everything captured so far is saved |
| 18 | `meeting.recorder.controlled_stop_fatal_stream_error` | error | fallback, repair | NEW `meeting.errors.captureStoppedUnexpectedly.*` (TAX §6) | **Open meeting** (data preserved); reconnect device before next start |
| 19 | `meeting.recorder.sleep_wake_interrupted` | warning | fallback | existing `meeting.errors.slept.*` (E13) | **Retry** via `Resume`, or `End meeting`; recording stopped safely |
| 20 | `meeting.recorder.capture_degraded` | info | none | NEW `meeting.warnings.degradedCapture.*` (TAX §6) | **None (passive)** — capture continues degraded; check device if quality complaints follow |

**Documented non-errors (no code exists — do not triage these as faults):**
"Keine Systemlautstärke" (silence is legitimate, RC:796) renders through
existing `meeting.warnings.silence.*` (E4) bound to the *setup-test outcome*;
backlog growth ("lokales Modell zu langsam") renders through existing
`meeting.warnings.backlog.*` (E11) bound to snapshot state, not to a code
(TAX §5.2 closing note).

---

## 3. Master crosswalk — origin `session` (TAX §5.3)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 21 | `meeting.session.start_validation_failed` | error | block | **Composition**: publishes the *child* code (recorder §2.1 / STT §5 cause); this code itself renders existing `meeting.errors.missingKey.*` (E16) | Follow the **child code's** operator action (typically **Check key** / **Download model** / **Grant OS permission** / **Free disk space**); start stays blocked |
| 22 | `meeting.session.resume_validation_failed` | warning | fallback, repair | existing `meeting.warnings.devicesChanged.*` (E14) + published child code | **Reconnect device** or accept the change via `Continue with these devices`; session stays `paused` |
| 23 | `meeting.session.consent_gate_refused` | error | none (fail-closed) | NEW `meeting.errors.consentRequired.*` (TAX §6) | **None** — no bypass exists; recording consent must be acknowledged before start |
| 24 | `meeting.session.recovery_disposition_failed` | error | retry, repair | existing `meeting.errors.recoverable.*` (S9) + `meeting.recovery.*` (E12) | **Retry** (`Recover and process`); if it persists, collect diagnostics |
| 25 | `meeting.session.stt_jobs_exhausted` | error | retry, fallback | NEW `meeting.errors.segmentsFailed.*` (TAX §6) | **Retry** (`Retry failed segments`) after the underlying provider issue is fixed; `Continue in background` meanwhile |
| 26 | `meeting.session.summary_no_path_available` | error | retry, repair, fallback | SPC §6.2 keys `meeting.summary.failed.title` / `.body` / `.reason.noPath` / `.reason.consentWithdrawn` — **NEW proposals owned by SPC §6.2, not yet in COPY** | **Check key**/summary provider config (`Open settings`), **Retry** (`Try again`), or `Don't summarize` — recording, transcript and all data are safe |
| 27 | `meeting.session.recoverable_error_generic` | error | retry, repair | existing `meeting.errors.recoverable.*` (S9) | **Retry**; a hit on this defensive-fallback code is itself a contract defect (TAX §5.3) — file it with the `cid` from diagnostics |

**Documented non-errors:** user-abort terminals `stopped_unprocessed` /
`cancelled` (SSC T15/T16) are deliberate user actions with **no error code**;
they render existing `meeting.session.cancelled.*` (S10) directly. Operator
action: none — nothing was deleted.

---

## 4. Master crosswalk — origin `stt` (TAX §5.4; STT §6)

### 4.1 Permanent classifications (STT §6.1 — auto-retry: `never`)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 28 | `meeting.stt.auth_invalid` | error | repair | existing `meeting.errors.cloudConfig.*` (E10) | **Check key** (invalid/revoked provider key); no audio was uploaded |
| 29 | `meeting.stt.endpoint_invalid` | error | repair | existing `meeting.errors.cloudConfig.*` (E10) | **Check key** (endpoint/region wrong); no audio was uploaded |
| 30 | `meeting.stt.provider_not_configured` | error | block, repair | existing `meeting.errors.missingKey.body.cloud` (E16) | **Check key** (no key configured); start stays blocked |
| 31 | `meeting.stt.package_limit_exceeded` | error | repair | NEW `meeting.errors.audioPackageRejected.*` (TAX §6) | **Check key**/provider plan — audio package violated a provider format/size/duration limit (STT §4.2); retry without change will fail again |
| 32 | `meeting.stt.model_missing` | error | block, repair | existing `meeting.errors.missingKey.body.local` (E16) | **Download model**; start stays blocked |
| 33 | `meeting.stt.language_unsupported` | error | repair | NEW `meeting.errors.languageUnsupported.*` (TAX §6) | **Check key**/provider language support; pick a supported language or provider |
| 34 | `meeting.stt.source_chunk_damaged` | error | retry, fallback | NEW `meeting.errors.segmentsFailed.*` (shared with row 25, TAX §6) | **Retry** (`Retry failed segments`); chunk failed pre-run readiness validation as damaged (SSC §6.3) |

### 4.2 Transitory classifications (STT §6.2 — auto-retry: `auto_backoff`, bounded by STT §3.5)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 35 | `meeting.stt.network_unreachable` | warning | fallback, retry | existing `meeting.errors.cloudUnreachable.*` (E9) | **Wait for auto-retry**; capture continues locally, nothing leaves the device |
| 36 | `meeting.stt.provider_overloaded` | warning | fallback, retry | existing `meeting.errors.cloudUnreachable.*` (E9) | **Wait for auto-retry** (HTTP 408/429 `Retry-After` respected / 5xx) |
| 37 | `meeting.stt.attempt_timeout` | warning | fallback, retry | existing `meeting.errors.cloudUnreachable.*` (E9) | **Wait for auto-retry**; if it exhausts max attempts, **Retry** manually |
| 38 | `meeting.stt.tls_disrupted` | warning | fallback, retry | existing `meeting.errors.cloudUnreachable.*` (E9) | **Wait for auto-retry**; persistent certificate problems degrade to row 29 (**Check key**/endpoint) |
| 39 | `meeting.stt.local_engine_transient` | warning | fallback, retry | NEW `meeting.warnings.localTranscriptionRetrying.*` (TAX §6) | **Wait for auto-retry** (single-segment local inference fault with valid model) |

### 4.3 Unclassifiable (STT §6.3 — auto-retry: `conservative_auto_backoff`)

| # | Code | Sev | Action class | Copy key(s) | Operator action |
| --- | --- | --- | --- | --- | --- |
| 40 | `meeting.stt.unclassified` | warning | fallback, retry | selector `params.provider`: `local` → NEW `meeting.warnings.localTranscriptionRetrying.*`; otherwise existing `meeting.errors.cloudUnreachable.*` (E9) | **Wait for auto-retry** (treated as transitory — better a wasted attempt than a lost segment) |

**Transitory-state rule for operators:** while any `retry_wait` exists,
capture continues untouched and no segment is lost (plan Fehlerverhalten;
STT §6.2 closing rule) — these envelopes never interrupt recording.

---

## 5. Reserved but not yet registered code

`meeting.session.settings_locked_while_recording` is **reserved** by the
session-settings-policy (§5, §G2) inside the closed TAX namespace but is
**not yet a row in the TAX §5 master table**. Per TAX §3 registration duty
it must be added to TAX §5 (one reviewable edit) before it may be emitted.
It appears here only so operators recognize it if a build ships it early:
severity/action/copy mapping per session-settings-policy — operator action:
change takes effect at the **next** meeting; end the meeting first.

---

## 6. Unknown-code fallback (TAX §2.3)

A code **not** present in the taxonomy table renders
`meeting.errors.generic.*` (NEW, TAX §6) with action class `repair` and logs
a development-mode warning. Operator action: treat as a defect — capture the
`cid`, the raw `code` from diagnostics, and file it against the taxonomy
contract. Never guess a mapping for an unregistered code.

---

## 7. Gate self-check

| Gate | Evidence |
| --- | --- |
| **G1 — exhaustive against the contract's master mapping table** | Rows 1–40 above mirror TAX §5 one-to-one: §5.1 → rows 1–12 (12 `RecorderError` variants, RC:411–432), §5.2 → rows 13–20 (8 failure-bearing events/stop reasons), §5.3 → rows 21–27 (7 SSC failure behaviors), §5.4 → rows 28–40 (7 permanent + 5 transitory + 1 unclassified). **40/40 codes tabulated; zero added, zero dropped.** Documented non-codes (silence E4, backlog E11, user aborts S10) and the reserved `settings_locked_while_recording` are called out in prose (§2, §3, §5), not as table rows. |
| **G2 — copy keys exist in the catalog or are flagged NEW** | Every `existing` key cites its COPY entry: E1–E3, E5–E14, E16, E17, S9, S10 — all verified present in `docs/meeting-mode-ux-copy-catalog.md` (entries E1–E17, S1–S10). Every key absent from COPY is explicitly flagged: eleven NEW families per TAX §6 (`deviceUnavailable`, `storageWriteFailed`, `actionUnavailable`, `captureStoppedUnexpectedly`, `consentRequired`, `segmentsFailed`, `audioPackageRejected`, `languageUnsupported`, `warnings.degradedCapture`, `warnings.localTranscriptionRetrying`, `generic`) and the four SPC §6.2 keys `meeting.summary.failed.*` (verified absent from COPY, owned by SPC — SPC line 394 self-declares them as NEW). Note: per COPY Abschnitt 1 **no** key exists in `src/i18n/locales/en/translation.json` yet — "existing" always means *proposed in the catalog*, never *implemented*. |
| **G3 — registered in the documentation index** | Row added to the document table in `docs/meeting-mode-documentation-index.md` §3 plus an inventory-count update line, per the index's Pflegeregeln (Abschnitt 5). |
