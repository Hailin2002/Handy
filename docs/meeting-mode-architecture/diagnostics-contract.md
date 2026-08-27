# Session Diagnostics & Log-Correlation Contract — Meeting Mode

Status: **spec contract — no implementation yet** (Phase 2+ prerequisite; changes
no source code).

Source of truth: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) —
§6.3 line 435 ("Debug-Funktionen bleiben wie heute nur im aktivierten
Debug-Modus sichtbar"), §9 line 1334–1336 ("Secrets werden … niemals in
Meeting-, Projekt-, Importtabellen, Manifesten, Events oder Logs geschrieben"),
§12 (Speicher- und Datenschutzlogik, lines 1436–1476, namentlich lines
1454–1456: lokale Verarbeitung verlässt den Rechner nicht), §9 `change_log`
line 1323 ("In v1 dient das Log der inkrementellen Projektanalyse und Diagnose").

Sibling contracts consumed read-only below, abbreviated:

- **TAX** — [error-taxonomy-contract.md](error-taxonomy-contract.md)
  (the closed error-code namespace this contract's log/event tags align with)
- **SSC** — [session-state-contract.md](session-state-contract.md)
  (`meeting://*` topics, T01–T18 transitions, O1–O7 ordering/payload rules)
- **RC** — [recorder-contract.md](recorder-contract.md) (`RecorderEvent`,
  chunk ids, timeline markers)
- **STT** — [stt-provider-contract.md](stt-provider-contract.md)
  (job state machine; §7 secret-handling and the existing `compact_error`
  redaction practice)
- **JSC** — [job-scheduler-contract.md](job-scheduler-contract.md)
  (queue/lease/cancellation surfaces that must stay correlatable)
- **SPD** — [summary-pipeline-dataflow.md](summary-pipeline-dataflow.md),
  **IMP** — [import-pipeline-dataflow.md](import-pipeline-dataflow.md)
  (downstream stages of the correlation chain)

Conflict rule (identical to the documentation index): plan wins over this
contract, this contract wins over catalogs, nothing wins over actual code.
Where TAX assigns error codes, TAX stays authoritative for the _code_; this
contract is authoritative for how a code appears in logs, events, and the
diagnostics bundle.

---

## 1. Purpose and seam ownership

Meeting mode processes hours of spoken content per session. When a user reports
"my meeting failed at some point", support must be able to answer three
questions from artifacts the user can hand over — without ever receiving the
spoken content itself:

1. _Which session?_ — every log line and every event about that session must be
   identifiable as belonging to it (correlation).
2. _What happened?_ — failures must appear under the same names everywhere
   (taxonomy alignment with TAX).
3. _Is it safe to share?_ — the diagnostics bundle must provably exclude
   transcripts, audio, summaries, and secrets at **every** verbosity tier.

This contract defines the single **diagnostics seam** between meeting-mode
subsystems and the two existing output channels — the `log` facade drained by
`tauri_plugin_log` (console / file / webview targets configured in
`src-tauri/src/lib.rs:357–402`, levels plumbed through
`src-tauri/src/logging.rs`) and the Tauri event bus (`meeting://*` topics per
SSC §4). Normative claims:

1. Every log record and every `meeting://*` event belonging to a session
   carries exactly one session correlation id (**§2**).
2. Every meeting-mode log record carries a structured tag whose name is drawn
   from one namespace aligned 1:1 with TAX §3 (**§3**).
3. Verbosity is tiered; tiers change _quantity_, never _category_ of logged
   content (**§4**).
4. Transcript-class content cannot reach any log tier or event payload; only
   whitelisted metadata fields may be logged (**§5**, §6 — the core gate).
5. The diagnostics bundle is a fixed, enumerable artifact set that any support
   procedure may rely on (**§7**).

## 2. Session correlation id (gate G1)

### 2.1 Definition — one id, zero new identifiers

The session correlation id (**cid**) **is** the `meeting_sessions` primary key
(`meeting_id`, UUID v4 per plan §9). This contract deliberately introduces **no
second identifier scheme**: a cid that can drift from the row users point at in
the UI ("this meeting") would be worse than none. Imports correlate through the
meeting (or project source item) the import produces — see §2.4.

### 2.2 Canonical rendering

Exactly one canonical rendering exists, appended by the logging wrapper as the
last token of the record's message field:

```text
2026-08-22T10:31:02.114Z INFO meeting::stt: job leased attempt=1 backoff_ms=0 cid=a1b2c3d4-e5f6-7890-abcd-ef0123456789
```

Rules:

- R2.1 — the token is always spelled `cid=<full lowercase uuid>`. No short
  forms, no `[mtg: …]` brackets, no per-target variants. One grep pattern
  (`cid=[0-9a-f-]{36}`) must find every correlated record in every sink.
- R2.2 — records outside any session context carry **no** cid token at all.
  A placeholder (`cid=none`, `cid=?`) is forbidden: it would make
  "uncorrelated" and "correlated to a deleted session" indistinguishable.
- R2.3 — a record may carry at most one cid. Work that spans two meetings
  (retention sweeps, quota evaluation, scheduler ticks) is uncorrelated by
  definition (R2.2); per-meeting outcomes inside such work are logged as
  separate correlated records.
- R2.4 — every `meeting://*` event payload gains one optional top-level field
  `"cid": "<uuid>"` (same value). Absence means "not session-scoped". This
  extends SSC §4 payload shapes additively; SSC stays authority for ordering,
  this contract for the extra field.

### 2.3 Propagation rule per stage (end-to-end, G1)

The cid is stamped once at the **single entry seam** — `MeetingManager`, the
owner named by SSC §1 — and travels with work explicitly; ambient/task-local
propagation across `.await` points and worker threads is explicitly rejected
(NG-2). Concretely:

| #   | Stage                        | Carrier of the cid                                                                                                                                                                     | Stamp duty                                                                                                                                                                                                                                                                                                                                                                                |
| --- | ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1  | Recorder capture             | `MeetingRecorder` calls are issued by `MeetingManager`, which holds the active `meeting_id`; `RecorderEvent`s are re-emitted by the manager's event sink with the cid already attached | manager stamps every record it writes while handling recorder events/events it forwards                                                                                                                                                                                                                                                                                                   |
| P2  | Chunk persistence & recovery | chunk rows belong to `meeting_id` (plan §9); startup recovery (RC §8, SSC §6) iterates per meeting                                                                                     | recovery loop logs per-meeting with that meeting's cid, including `meeting.recorder.recovery_failed` paths                                                                                                                                                                                                                                                                                |
| P3  | STT queue                    | `meeting_stt_jobs` rows carry `meeting_id` (plan §9); JSC leases are keyed by job row                                                                                                  | queue workers read the cid from the leased job row and stamp every attempt/retry/terminal record (`meeting.stt.*` codes included)                                                                                                                                                                                                                                                         |
| P4  | Summary                      | summary work is requested per meeting (SPC/SPD); the request carries `meeting_id`                                                                                                      | `MeetingSummaryService` stamps all stage records, incl. map-reduce window records (window indexes allowed, window text never — §5)                                                                                                                                                                                                                                                        |
| P5  | Import                       | an import creates (or attaches to) a project source item; before a meeting exists there is no cid                                                                                      | two-phase rule: pre-ingest records (validation, zip-bomb checks, asset extraction) are uncorrelated (R2.2) and tagged `evt=meeting.import.*`; from the moment the target meeting/source row exists, subsequent records carry its cid. An import that fails before creating a row stays uncorrelated — its diagnostics link is the import job id, which is whitelisted metadata (§5.2 W12) |
| P6  | Frontend events              | every `meeting://*` payload carries `"cid"` per R2.4                                                                                                                                   | emitters are the same manager sinks; the debug panel may filter the live log viewer by the cid of the meeting currently open                                                                                                                                                                                                                                                              |

**End-to-end claim (G1):** for any meeting M, the set
`{records mentioning M} ⊆ {records carrying cid=M}` holds by construction,
because every code path that knows about M reached it via one of P1–P5's
carriers, and each carrier is stamped at acquisition time. §8 specifies the
test that verifies this property on a fixture run rather than trusting the
argument.

### 2.4 Cross-stage join keys

Where a record must reference related entities, they are additional whitelisted
fields next to `cid` (chunk id, job id, segment id — §5.2), never substitutes
for it. Support joins _from_ the meeting detail view outward: meeting → jobs →
chunks → attempts. Reverse joins (job id alone → meeting) are possible because
P3/P4 carriers embed both ids.

## 3. Event-name taxonomy aligned with TAX error codes

Every meeting-mode log record carries a structured tag `evt=<name>` rendered
immediately after the log target, before free-text fields. The namespace is
**closed** and mirrors TAX §3 exactly:

1. **Fault records reuse the TAX code verbatim:** whenever a subsystem emits a
   `MeetingErrorEnvelope { code, params }` (TAX §2), the accompanying log
   record uses `evt=<that exact code>` and logs the envelope params — the
   whitelisted ones (§5.2) — plus the code itself. There is no second naming
   for the same fault: `meeting.recorder.stream_error` in the UI envelope is
   `meeting.recorder.stream_error` in the log. This makes log-to-UI
   reconciliation mechanical.
2. **Lifecycle/progress records use `<origin>.<verb>`** within the same four
   TAX origins `{recorder, session, stt, summary}` extended by `{import}`
   (TAX §3.1 already reserves origin-set extension via registration duty;
   `import` is hereby registered for diagnostics tags only — TAX §5 tables
   remain untouched until import fault codes actually exist):

   | Origin     | Closed verb set                                                                                         |
   | ---------- | ------------------------------------------------------------------------------------------------------- |
   | `recorder` | `started`, `paused`, `resumed`, `stopping`, `stopped`, `recovery_scan`, `source_segment_opened`         |
   | `session`  | `draft_created`, `state_changed`, `snapshot_taken`, `consent_gate`, `retention_decided`                 |
   | `stt`      | `enqueued`, `leased`, `attempt_done`, `backoff_wait`, `cancelled`, `completed`                          |
   | `summary`  | `window_started`, `window_persisted`, `reduce_started`, `completed`, `fallback_used`, `consent_checked` |
   | `import`   | `archive_validating`, `extracting`, `item_parsed`, `asset_ingested`, `completed`                        |

3. **Registration duty (mirrors TAX §3.5):** a verb not in the table above, or
   a tag outside the namespace, is a defect; adding one requires an edit to
   this section first. Verbs are snake_case, ASCII, stable once shipped.
4. **Tauri event emission logging:** emitting a `meeting://*` topic logs
   `evt=<origin>.state_changed`-class records with field `topic="meeting://…"`
   (`topic` is whitelist field W13, §5.2). The topic name itself is never used
   as the evt tag — one naming system per artifact kind.
5. **Non-meeting code is out of scope:** dictation-path records keep their
   current plain formatting (no `evt`, no cid) — the same carve-out TAX §9
   makes for the dictation error path. Mixing is visible, not silent: any
   record with a `meeting.` prefix must carry `evt=` and, if session-scoped,
   `cid=` (testable, §8.4).

## 4. Verbosity tiers

Tiers map onto the **existing** plumbing only (`logging.rs` atomics, settings
`log_level`, debug-mode toggle, CLI `--debug`). No new setting is introduced.

| Tier                | Trigger                                                                          | File-log level                                                 | Webview stream                          | Content delta vs. lower tier                                                                                                    |
| ------------------- | -------------------------------------------------------------------------------- | -------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| **T0 — normal**     | default (`log_level` = Info per current defaults; file level per stored setting) | Info (user-adjustable via existing `set_log_level`, unchanged) | off                                     | lifecycle + faults only (`evt` records at Info/Warn/Error)                                                                      |
| **T1 — user debug** | Debug mode toggled (Ctrl/Cmd+Shift+D)                                            | Debug                                                          | on (`WEBVIEW_LOG_STREAMING`, unchanged) | adds Debug records: retry/backoff arithmetic, lease waits, queue depths, coalescing stats                                       |
| **T2 — trace**      | `--debug` flag or explicit Trace selection (existing behavior, lib.rs:524–533)   | Trace                                                          | on                                      | adds Trace records: timing spans, byte offsets/sizes, provider HTTP status _classes_ (not bodies), internal scheduler decisions |

Normative rules:

- R4.1 — tiers are **monotone in quantity only**. Any record legal at T2 is
  legal (modulo level) at T0 _after_ redaction; nothing exists that only T2
  may log but T0 forbids _by category_. This is what makes the §5 prohibition
  enforceable once instead of per tier.
- R4.2 — the transcript prohibition (§5) binds **identically at T0, T1, T2**.
  There is no "trace may include content" escape hatch (G2). If debugging ever
  genuinely needs content, the sanctioned tool is the user exporting their own
  meeting data (plan §11 export), not the log.
- R4.3 — the webview stream inherits the file-level filter and additionally
  the redaction layer (§6): debug-panel visibility never widens what may be
  emitted, consistent with the existing design note in `logging.rs:16–22`
  (records "can include file paths or transcribed text" today — meeting-mode
  records must not, so the caveat shrinks instead of grows).
- R4.4 — verbosity is process-global exactly as today; per-module or
  per-session verbosity is a non-goal (NG-5).

## 5. Redaction rule — allowlist, not blacklist (gate G2)

### 5.1 The rule

**Only fields enumerated in §5.2 may appear in any meeting-mode log record,
event payload extension, or diagnostics artifact. Everything else is
forbidden by construction.** This is an allowlist discipline: the question is
never "is X sensitive?" but "is X on the list?". It subsumes blacklist rules —
transcript substrings, summary text, chat content, contact names, secrets —
because none of them are on the list, and it stays correct when novel
sensitive categories appear.

Specifically prohibited (non-exhaustive, by way of illustration — absence from
§5.2 is the binding rule, this list is documentation):

- any substring of any transcript segment, interim hypothesis, or VAD/audio
  metadata derived from speech content;
- `summary_markdown`, window extraction texts, task titles/descriptions,
  snapshot statements (project intelligence outputs are content too);
- WhatsApp/import message bodies, sender display names, original attachment
  filenames chosen by users (ingest keeps only size/type/count facts);
- secret material in any form — extends STT §7 verbatim: keys, tokens,
  `SecretMap` values (only `{}`-serialized, redacted-reference form may be
  mentioned), authorization headers, full provider URLs with credentials;
- clipboard content, window titles, file system absolute paths outside the
  app-data dir (they leak usernames/project names).

### 5.2 Metadata whitelist (complete, enumerated)

| #   | Field                                           | Type/shape                                                                                                                                        | Notes                                                                                                                               |
| --- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| W1  | `cid`                                           | UUID v4 string                                                                                                                                    | §2                                                                                                                                  |
| W2  | `chunk_id`, `segment_id`, `job_id`, `marker_id` | ids as defined by RC/STT/SSC                                                                                                                      | opaque ids, safe by construction                                                                                                    |
| W3  | state values                                    | session/job/chunk/source states (SSC §2, STT §3, RC enums)                                                                                        | closed vocabularies                                                                                                                 |
| W4  | `code` + envelope `params`                      | TAX code string; params per TAX §2.1 flat scalar shape                                                                                            | params values are device names, counts, durations — TAX already bounds them; a param key not declared in a TAX §5 row is a defect   |
| W5  | `provider`                                      | enum {local, azure, custom} + model id                                                                                                            | model id is a catalog name, not user content                                                                                        |
| W6  | counts and durations                            | integers, ms/bytes                                                                                                                                | backlog depth, attempt counts, segment counts, timeline ms, byte sizes, free/required disk bytes                                    |
| W7  | timing                                          | timestamps, elapsed ms, monotonic deltas                                                                                                          | wall-clock timestamps fine (local artifact)                                                                                         |
| W8  | error classification                            | retryability class (TAX §4.3), HTTP status _class_ (4xx/5xx/timeout/tls), WASAPI/cpal error _kind discriminant_                                   | never raw HRESULT/HTTP bodies — mirrors TAX §3.4 provider-detail ban                                                                |
| W9  | language/locale codes                           | BCP-47 tags                                                                                                                                       | e.g. `de-DE`                                                                                                                        |
| W10 | versions                                        | app version, git hash, schema/parser/prompt/model versions                                                                                        | supports §7 bundle                                                                                                                  |
| W11 | relative storage paths                          | path relative to the app-data dir, forward slashes                                                                                                | e.g. `meetings/<cid>/audio/chunk_0042.wav` — the cid segment is W1; absolute prefixes are stripped before logging                   |
| W12 | import job facts                                | archive element count, total/uncompressed byte sizes, compression ratio, parser id, rejection reason class (zip-bomb/traversal/encrypted/damaged) | plan §12 lines 1463–1468; classes only, never offending names                                                                       |
| W13 | `topic`                                         | `meeting://*` topic string                                                                                                                        | §3.4                                                                                                                                |
| W14 | device names                                    | user-visible audio device name strings                                                                                                            | already travel as TAX `params.deviceName`; needed to act on device-lost reports; they are the _only_ free-form strings on this list |

Nothing else. A field proposal not in this table requires an edit here first
(same registration duty as §3.3).

### 5.3 Why this is _provable_, not aspirational (G2)

Three independent layers, each testable (hooks in §8):

1. **Structural (compile-time intent):** meeting-mode code does not call
   `log::*!` directly with arbitrary payloads; it emits through one module
   (proposed home: `src-tauri/src/meeting_diagnostics.rs`, placement consistent
   with the module-placement-map B-seam conventions) whose API accepts fields
   only as a typed, closed `SafeField` enum mirroring §5.2. Constructing a log
   record from a raw transcript `String` has no expression path — the type
   system, not reviewer diligence, carries the invariant.
2. **Order-of-filters (tier independence):** redaction happens inside the
   emission module, i.e. _before_ any `tauri_plugin_log` target filter runs
   (lib.rs:373–402). Filters select among already-safe records; therefore no
   tier configuration, env var (`RUST_LOG`), or future target can re-enable
   content. This closes the classic "verbose mode leaks" hole by ordering, not
   policy.
3. **Negative tests (empirical):** §8.2 runs a fixture session end-to-end at
   every tier and asserts none of the fixture's known transcript substrings
   appear in any captured sink; §8.3 asserts the same for the webview event
   stream payloads.

Residual risk disclosed honestly: layer 1 covers meeting-mode emissions only;
pre-existing dictation-path records keep today's behavior (carve-out §3.5) and
remain outside this contract's guarantee, matching the plan's scope (meeting
mode is the privacy-sensitive new surface; plan §12).

## 6. Emission pipeline (where the pieces sit)

```text
subsystem ──► meeting_diagnostics.rs
              │  builds SafeField set (+cid from explicit argument)
              │  renders canonical form: evt=… field=… … cid=…
              ▼
        log::record() ─► tauri_plugin_log targets (unchanged)
              │                console (RUST_LOG) · file (FILE_LOG_LEVEL) · webview (streaming flag)
              ▼
        meeting://* events: same SafeField set serialized, + "cid" field (R2.4)
```

The existing `logging.rs` atomics, `set_log_level` command, debug toggle, and
CLI override are reused untouched (their doc comments already anticipate this
seam). The dictation pipeline is untouched (NG-7 analog of TAX §9).

## 7. Diagnostics bundle (gate G3)

### 7.1 Contents — closed enumeration

A user-triggered "support report" for meeting M produces exactly:

| #   | Artifact                                                             | Source                                                                                                        | Bound by                                                                                                                                 |
| --- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| B1  | `speak.log` — current file-log file(s) as written at the user's tier | existing log dir (`commands::get_log_dir_path` target)                                                        | safe by §5 at any tier                                                                                                                   |
| B2  | `environment.json`                                                   | generated                                                                                                     | app version/commit (W10), OS name/version/arch, locale (W9), tier history if recorded (W7/W3)                                            |
| B3  | `settings-redacted.json`                                             | tauri-plugin-store settings minus a declared exclusion set                                                    | excludes: provider secrets/keys (referenced by id only, STT §7), selected device ids may stay as _names_ (W14), everything else retained |
| B4  | `session-metadata.json`                                              | `meeting_sessions` row for M + snapshot/device columns                                                        | metadata columns only; no transcript/summary joins                                                                                       |
| B5  | `jobs.jsonl`                                                         | `meeting_stt_jobs` + relevant `processing_jobs` rows for M                                                    | states, attempts, timings, codes (W2–W8); result columns excluded                                                                        |
| B6  | `manifest.json`                                                      | the session directory manifest (phase2-contracts §2)                                                          | already metadata-only by that contract's own rules                                                                                       |
| B7  | `events-recent.ndjson`                                               | optional attach: the last N (cap 500) `meeting://*` payloads with `cid=M` if the user opts in from debug mode | payloads already conform to §2/§5                                                                                                        |

Explicitly **never** in the bundle: audio chunks (any retention state),
transcript segments, summaries/snapshots, imported originals/assets, secret
store contents, `change_log` dumps beyond counts (its rows can quote entity
titles — content-adjacent — so only aggregate counts per entity type ship).

### 7.2 Relationship to the operations guide (task-0048) — mapping, not duplication

The operations guide (authored separately; not present in this worktree at
authoring time — verified by glob against `docs/`, see Appendix A) owns the
_procedure_: how a support agent asks for the bundle, how the user produces
it, how artifacts are read in which order. This contract owns the _supply
side_: the fixed artifact set above is what any such procedure may assume.
The binding rule in both directions:

- **G3-M1:** the guide may rely on B1–B6 existing verbatim and B7 being
  opt-in; it must not require artifacts outside this table (if it needs more,
  this §7.1 is edited first — single reviewable change, mirroring TAX §3's
  registration duty).
- **G3-M2:** the guide's reading instructions reference artifact numbers
  (B1…B7), not bespoke exports; this contract does not document any
  procedural step, menu path, or copy string — those live only in the guide
  and COPY respectively.
- **G3-M3 (disclosed dependency):** because the guide did not exist in this
  worktree, G3 is satisfied as a _forward mapping_: §7.1 defines the complete
  assumption surface, and the guide inherits conformance duties G3-M1/M2 at
  its authoring time. Any later divergence is resolved by the precedence
  rule of the header (contracts over catalogs; the plan over both) plus the
  edit-first rule of G3-M1.

Bundle production itself is one command surface (proposed:
`export_diagnostics_bundle(meeting_id)` beside the existing
`get_log_dir_path`/`open_log_dir` commands, `src-tauri/src/commands/mod.rs:46–90`)
— assembly from local files only, no network upload (NG-1).

## 8. Verification hooks (maps to plan §14 discipline)

1. **Correlation completeness test (G1):** fixture run drives one session
   through P1→P4 (and a P5 import fixture); afterwards, scan all captured
   records mentioning fixture ids: every such record must contain
   `cid=<fixture uuid>` (property of §2.3), and no record may contain two cids
   (R2.3) or placeholder forms (R2.2).
2. **Redaction negative test at all tiers (G2):** run the fixture session at
   T0, T1, T2; capture console/file/webview sinks; assert none of ~20 planted
   transcript substrings, the summary text, a planted secret value, or an
   absolute-path username appears anywhere. Runs in CI (mirrors the
   STT §7 negative-test pattern and TAX §8.4 hygiene test).
3. **Type-level guard:** `SafeField` is `#[non_exhaustive]` internally but
   closed for meeting code; a compile-time assertion enumerates W1–W14 ↔
   variant parity so the whitelist table and the enum cannot drift silently.
4. **Tag totality test:** grep-based lint over `src-tauri/src/meeting*/`:
   every `log::` macro call outside `meeting_diagnostics.rs` is a defect
   (structural layer 1 enforcement); every emitted `evt=` matches §3's closed
   tables.
5. **Bundle conformance test:** generate a bundle for a fixture meeting;
   assert presence of B1–B6, absence of the never-list contents, and JSON
   parseability of B2–B5.

## 9. Complexity budget / non-goals (gate G4)

Deliberately **not** specified, with reasons:

- **NG-1 — No telemetry, crash reporting, or upload service.** Bundles are
  user-assembled, user-sent (out of band); automatic shipping would create a
  data-exit channel the plan's privacy posture (§12) refuses. Consistent with
  TAX §9.
- **NG-2 — No async task-local/span-context magic** (no `tracing` instrument
  forest, no thread-local cid). Explicit carrier propagation (§2.3) is verbose
  but auditable; implicit contexts silently drop across spawn boundaries and
  are unreviewable.
- **NG-3 — No second correlation id scheme** (no trace-id/span-id hierarchy):
  the meeting id suffices; hierarchical ids solve distributed-system problems
  a single-process desktop app does not have.
- **NG-4 — No blacklist-based redaction engine, no regex scrubber, no ML PII
  filter.** Allowlist-by-construction (§5) makes them unnecessary; scrubbers
  give false safety.
- **NG-5 — No new verbosity settings UI, no per-module log levels, no runtime
  per-session tracing switch.** Existing `log_level` + debug mode cover the
  user-debug-vs-trace need (plan line 435 keeps debug features behind debug
  mode).
- **NG-6 — No structured binary/OTLP log format, no log aggregation server
  vocabulary.** Canonical text rendering (R2.1) is greppable by humans and
  scripts alike.
- **NG-7 — No changes to dictation-path logging.** Its records keep current
  format and remain outside this contract's guarantees (§5.3 residual risk).
- **NG-8 — No retroactive redaction of old log files, no bundle encryption,
  no signed bundles.** The bundle contains no content requiring protection
  beyond transport the user chooses.
- **NG-9 — No diagnostics taxonomy for project-intelligence internals**
  (`meeting.project.*` verbs) yet — registered when that subsystem defines
  its observability needs, same edit-first duty as §3.3. Pre-registering
  speculative tags is bloat.

---

## Appendix A — Gate traceability

| Gate                                                                                   | Coverage                                                                                                                                                                                        |
| -------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 — correlation-id propagation across recorder, STT queue, summary, import end-to-end | §2.1 (id = meeting_id), §2.2 (canonical rendering rules R2.1–R2.4), §2.3 (stage table P1–P6 + end-to-end claim), §2.4 (join keys); verification §8.1                                            |
| G2 — transcript substrings provably absent from any log tier; whitelist enumerated     | §5.1 (allowlist rule), §5.2 (whitelist W1–W14, complete), §5.3 (three-layer proof: structure, filter order, negative tests; tier independence also fixed by §4 R4.1/R4.2); hooks §8.2/§8.3/§8.4 |
| G3 — bundle contents mapped to operations-guide expectations without duplicating it    | §7.1 (closed artifact table B1–B7 + never-list), §7.2 (mapping rules G3-M1/M2, disclosed dependency G3-M3 — guide file absent from this worktree, verified via glob during authoring)           |
| G4 — non-goals; documentation index updated                                            | §9 (NG-1–NG-9); index table row and reading-order entry added in `docs/meeting-mode-documentation-index.md`                                                                                     |
