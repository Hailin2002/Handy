# Summary-Pipeline Data Flow — MeetingSummaryService

Status: **spec contract — no implementation yet** (Phase 7 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 (`MeetingSummaryService`, plan
lines 253–259), §8 (segment model, plan lines 1035–1057), §9
(`meeting_transcript_segments` plan lines 1201–1208, `meeting_summaries` plan
lines 1210–1217, `processing_jobs` plan lines 1313–1318), §10 (Map-Reduce plan
lines 1349–1358, untrusted-transcript rule plan lines 1360–1362, standard
output plan lines 1364–1378), §12 storage logic (volume figures plan line
1438), session states (`summarizing`, plan line 885), §11 jump-to-position
(plan lines 1415, 1423), §13 Phase 7 (acceptance plan lines 1640–1645).
Planned implementation home: `src-tauri/src/managers/meeting_summary.rs`
(plan line 259). This document specifies the **data flow only**; it changes no
source code.

Sibling documents consumed read-only:

- [stt-provider-contract.md](stt-provider-contract.md) — job state machine
  (§3.1–§3.5), timeline correction (§5). Below abbreviated **STT-§n**.
- [summary-evidence-contract.md](summary-evidence-contract.md) — pipeline
  stage skeleton M0–M3 (§1), idempotency keys (§2), evidence pointer model
  (§3), pinning (§6), injection boundary (§7). Below abbreviated **SEC-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) — store
  seams. Below abbreviated **RB-§n**.

Conflict rule: the plan wins over this document, this document wins over
catalogs, nothing wins over actual code.

Out of scope (non-goals, see §7): **provider/model selection stays out of
scope entirely** — it is owned by the task-0052 research line and the phase
7/10 decisions. Everything below is defined so that any provider satisfying
the existing client infrastructure can be plugged in without changing this
data flow.

---

## 1. Input and deterministic transcript assembly (gate G1)

### 1.1 Input set

The sole input of every summary run is the persisted, immutable segment set:

```text
SELECT * FROM meeting_transcript_segments
WHERE meeting_id = :meeting_id
  AND job_id IN (SELECT id FROM meeting_stt_jobs WHERE state = 'succeeded')
ORDER BY start_ms ASC, end_ms ASC, sequence ASC, id ASC;
```

Properties:

1. **Total order.** `(start_ms, end_ms, sequence, id)` is a total order
   because `id` is the unique primary key (plan line 1202). Two segments can
   share `start_ms`/`end_ms` (mic + system overlap); the tie-breakers make
   their relative order stable forever. No wall-clock column participates in
   ordering.
2. **Only succeeded contributions.** Segments of `failed`, `cancelled`,
   `retry_wait` or still-running STT jobs are excluded, matching the plan's
   checklist where summarization waits for transcription (plan lines
   642–649, 885). A re-run after additional segments succeed simply sees a
   larger sorted set; the run key changes accordingly (SEC-§2.1
   `transcript_basis_hash`).
3. **No text mutation.** Assembly copies `text` verbatim. Overlap dedup of
   doubled boundary words already happened at the STT merge layer (STT-§5,
   plan line 1057). The summary layer never edits, merges or re-segments
   transcript text.

### 1.2 Canonical segment rendering

Each segment is rendered as exactly one line:

```text
[<t(start_ms)>-<t(end_ms)> <source>] <text>\n
```

with these fixed rules:

| Rule       | Definition                                                                                                                                                                                                                                                       |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `t(ms)`    | `HH:MM:SS`, decimal, zero-padded to two digits per field; `HH = floor(ms / 3600000)` (may exceed 23 — hours are **not** taken modulo 24, keeping the axis strictly monotone), `MM`, `SS` from the remaining remainder. Truncating division only, never rounding. |
| `<source>` | verbatim value of `meeting_transcript_segments.source`: `microphone` or `system` (plan lines 1041, 94–95). Never a speaker name.                                                                                                                                 |
| separator  | ASCII `-` between timestamps, single space before `<source>`, `]` + single space before text.                                                                                                                                                                    |
| encoding   | UTF-8, no BOM, LF line endings, regardless of platform.                                                                                                                                                                                                          |

### 1.3 Deterministic token estimation

Window sizing must not depend on a specific model tokenizer (providers are
out of scope). Sizing uses this pure function instead:

```text
EST_TOKENS(segment) = ceil(codepoint_count(rendered_line) * 26 / 100) + 24
```

Derivation of the two constants (see §3.1 for the full rate chain):
sustained German meeting speech is bounded at 2.5 words/s and ≈ 17.25
characters/s including spaces, i.e. ≈ 4.5 subword tokens/s
(1.8 tokens/word), giving 4.5 / 17.25 ≈ 0.26 tokens per codepoint. The flat
+24 covers the fixed framing (`[00:00:00-00:00:00 system] `). The estimate is
deliberately conservative; real tokenizers land below it for German prose,
and the utilization margin of §3 absorbs residual drift.

### 1.4 Byte-identical request invariant

> **INV-A1 (testable):** For a fixed `pipeline_version` (SEC-§2), a fixed
> windowing-parameter set (§3.4) and a fixed segment set, the assembled M1
> request bytes — system block from the pinned prompt template plus user
> block of concatenated rendered lines (§1.2) — have an identical SHA-256 on
> every computation, on every machine, after app restarts. Golden-file tests
> compare the hash; any difference is a defect.

There is deliberately no nondeterminism anywhere in the path: no locale
formatting, no floating point in rendering (integer arithmetic only), no
hash-map iteration order in assembly, no timestamps-of-day in the payload.

---

## 2. Pipeline stages (data-flow view)

The stage graph is owned by SEC-§1.1 (M0 window → M1 map → M2 reduce →
M3 publish). This document fixes only the data that crosses those edges:

| Edge          | Payload                                                                                                                   | Producer/consumer rules                                                                                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| segments → M0 | sorted segment set (§1.1)                                                                                                 | pure read; M0 writes the window plan (window ordinals, first/last segment IDs, estimated token sums) as a `window` stage row (SEC-§1.3).                                                     |
| M0 → M1       | one window = contiguous slice of the sorted set, segment-aligned (boundaries fall **between** segments, never inside one) | each M1 job receives its window pre-rendered as §1.2 lines plus the trusted instruction block; per-window evidence candidates returned by the model carry segment IDs from that window only. |
| M1 → M2       | per-window extraction result, capped at `MAP_RESULT_CAP` tokens (§3.3)                                                    | persisted immediately as a `map` stage row (SEC-§1.1 rule 2); a crash never loses succeeded windows.                                                                                         |
| M2 → M3       | merged protocol draft: Kernaussagen, Entscheidungen, Aufgaben, Offene Fragen, Verlauf (plan lines 1366–1378)              | element order in the draft follows window ordinal, then in-window position — deterministic input to publication.                                                                             |
| M3 → DB       | one `meeting_summaries` row: `summary_markdown` + structured JSON arrays (plan lines 1215–1216)                           | atomic insert with evidence validation (SEC-§3.3 invariant E1).                                                                                                                              |

Recursive reduce (SEC-§1.1 rule 3) chains partial results in ascending
ordinal order; each partial reduce persists a `reduce` stage row of the same
shape, so resume continues at the next chunk.

---

## 3. Token-budget windowing (gate G2)

### 3.1 Rate constants and their derivation from plan §12

Plan line 1438: _"PCM16 mit 16 kHz benötigt pro Mono-Spur ungefähr 115 MB pro
Stunde, bei zwei Spuren ungefähr 230 MB pro Stunde."_

```text
audio rate        = 16,000 samples/s * 2 bytes (PCM16) = 32,000 bytes/s
per-track volume  = 32,000 * 3,600 s = 115,200,000 bytes/h = 115.2 MB/h
two-track volume  = 2 * 115.2 MB/h                    = 230.4 MB/h
                    (matches plan line 1438: "ungefähr 115 / 230 MB")

recording hours h(free) = free_disk_MB / 230.4        (two tracks, §12 figure)
worst-case transcript tokens T(h) = h * T_H           (T_H below)

speech bound      = 2.5 words/s          (~150 wpm upper bound, meeting speech)
subword inflation = 1.8 tokens/word      (German BPE, conservative)
text token rate   = 2.5 * 1.8            = 4.5 tokens/s of speech
segments per hour = ceil(3,600 / 17.5)   = 206         (17.5 s = midpoint of the
                                         plan §8 VAD bounds 5–30 s, line 1054)
framing per hour  = 206 * 24             = 4,944 tokens (framing per §1.2)
text per hour     = 3,600 * 4.5          = 16,200 tokens
T_H (planning)    = 16,200 + 4,944       = 21,144 -> rounded UP to 22,000 tokens/h
```

`T_H = 22,000` assumes 100 % duty cycle inside windows (VAD output is
speech-only, so this is the correct upper bound, not pessimism). All
downstream numbers derive from `T_H` by the arithmetic shown.

### 3.2 Budget constants

| Constant                | Value        | Meaning                                                                                                  |
| ----------------------- | ------------ | -------------------------------------------------------------------------------------------------------- |
| `U`                     | 0.85         | fraction of the context window usable for input (rest: output + safety)                                  |
| `MAP_PROMPT`            | 700 tokens   | pinned map instruction block (system + rules)                                                            |
| `MAP_OUTPUT_RESERVE`    | 2,000 tokens | reserved for one M1 response incl. evidence candidates                                                   |
| `SINGLE_PROMPT`         | 900 tokens   | pinned single-pass instruction block                                                                     |
| `SINGLE_OUTPUT_RESERVE` | 4,000 tokens | reserved for one complete single-pass response                                                           |
| `REDUCE_PROMPT`         | 600 tokens   | pinned reduce instruction block                                                                          |
| `REDUCE_OUTPUT_RESERVE` | 4,000 tokens | reserved for one reduce response                                                                         |
| `MAP_RESULT_CAP`        | 800 tokens   | hard cap enforced on accepted M1 outputs (over-long map results fail validation, they are not truncated) |

### 3.3 Derived budgets (arithmetic shown)

```text
WINDOW_INPUT_BUDGET(C) = floor(U * C) - MAP_PROMPT - MAP_OUTPUT_RESERVE

C =    8,192 : floor(0.85 * 8,192) = 6,963  ->  6,963 -   700 - 2,000 =  4,263
C =   32,768 : floor(0.85 * 32,768)        = 27,852 -> 27,852 - 700 - 2,000 = 25,152
C =  131,072 : floor(0.85 * 131,072)       =111,411 -> 111,411 - 700 - 2,000 = 108,711

REDUCE_FANIN(C) = floor((floor(U*C) - REDUCE_PROMPT - REDUCE_OUTPUT_RESERVE) / MAP_RESULT_CAP)

C =    8,192 : (6,963 - 600 - 4,000) / 800   = 2,363 / 800 = 2.95 ->  2
C =   32,768 : (27,852 - 600 - 4,000) / 800  = 23,252 / 800 = 29.06 -> 29
C =  131,072 : (111,411 - 600 - 4,000) / 800 = 106,811 / 800 = 133.5 -> 133
```

### 3.4 Window-count table (gate G2 core)

Transcript token totals from §3.1: 1 h = 22,000; 2 h = 44,000; 4 h = 88,000.
Window count = `ceil(total / WINDOW_INPUT_BUDGET)`.

| Context C | Window budget | Windows @1 h                | Windows @2 h                | Windows @4 h                |
| --------- | ------------- | --------------------------- | --------------------------- | --------------------------- |
| 8,192     | 4,263         | ceil(22,000/4,263) = **6**  | ceil(44,000/4,263) = **11** | ceil(88,000/4,263) = **21** |
| 32,768    | 25,152        | ceil(22,000/25,152) = **1** | ceil(44,000/25,152) = **2** | ceil(88,000/25,152) = **4** |
| 131,072   | 108,711       | **1**                       | **1**                       | **1**                       |

Worked example tying §12 storage to token load: with 2 GB free disk,
h = 2,048 MB / 230.4 MB/h = 8.89 h of two-track audio (plan line 1438
arithmetic), so T = 8.89 \* 22,000 ≈ 195,600 transcript tokens — above every
single-pass threshold below, i.e. a full-disk-length meeting always runs
map-reduce even on a 128 k-context model.

### 3.5 Map-reduce vs single-pass decision rule

Evaluated once inside M0 and persisted with the window plan; re-evaluated
only when `pipeline_version` changes (then a new run starts anyway):

```text
single_pass  <=>  T_total + SINGLE_PROMPT + SINGLE_OUTPUT_RESERVE <= floor(U * C)

max single-pass duration h_max(C) = (floor(U*C) - 4,900) / 22,000 hours
  C =    8,192 : (6,963 - 4,900) / 22,000   = 0.094 h  ~= 5.6 min
  C =   32,768 : (27,852 - 4,900) / 22,000  = 1.043 h  ~= 62.6 min
  C =  131,072 : (111,411 - 4,900) / 22,000 = 4.841 h  ~= 4 h 50 min
```

Rule consequences:

1. If `single_pass`, M0 emits exactly one window covering all segments and
   M1+M2 collapse into that single request; the standard output shape
   (plan lines 1366–1378) is unchanged.
2. Otherwise M0 emits `ceil(T_total / WINDOW_INPUT_BUDGET)` segment-aligned
   windows (greedy fill in sort order: append the next rendered segment while
   the running `EST_TOKENS` sum plus the next segment stays within budget;
   else close the window). Greedy fill over a total-order input is
   deterministic — part of INV-A1.
3. Reduce fan-in wider than `REDUCE_FANIN(C)` recurses: partial results are
   grouped `REDUCE_FANIN` at a time in ascending ordinal order until one root
   remains (e.g. 4 h @ 8,192: 21 windows -> 11 -> 6 -> 3 -> 2 -> 1). The
   depth explosion at small contexts is intentional visibility, not a bug:
   it is the quantitative argument for preferring larger-context providers
   for long meetings (decision itself out of scope, §9).

---

## 4. Summary job persistence model (gate G3)

### 4.1 Distinct identity, reused state machine

Summary jobs persist in the generic `processing_jobs` table with
`service = 'meeting_summary'`, `subject_id = meeting_id`
(plan lines 1313–1318; SEC-§1.4) — **distinct rows from `meeting_stt_jobs`**
(plan lines 1190–1199), which keeps STT retries and summary retries
independent as demanded by plan §10 (provider independence of summarization,
plan lines 1340–1347).

The **state vocabulary and transition legality are reused verbatim** from the
STT contract: `queued`, `running`, `retry_wait`, `succeeded`, `failed`,
`cancelled` with exactly the allowed transitions of STT-§3.2 and the
restart-resumption rules of STT-§3.4 (`running -> queued` on app start
without attempt penalty; `retry_wait` keeps its persisted
`next_attempt_at`; terminal states stay untouched). Reuse is deliberate:
plan line 1317 declares the state/retry semantics of `meeting_stt_jobs`
normative for `processing_jobs`, and one state machine across both job
families keeps the processing checklist UI (plan lines 642–657) uniform.

### 4.2 Justified divergences from the STT job machine

| #   | Divergence                                                                                                                                                                                                                                                                                               | Justification                                                                                                                                                                                                                                             |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | `queued -> running` requires **no engine lease** (contrast STT-§4.1). Only a per-service concurrency limit of 1 applies.                                                                                                                                                                                 | The single-engine lease exists solely because `TranscriptionManager` holds one shared local STT engine (STT-§4.1, plan lines 1061–1063). Summaries share no such resource; a lease gate would add dead complexity (complexity budget, §9).                |
| D2  | Restart resumption is **stage-granular**: recovery maps an interrupted `running` job to `queued`, and execution resumes at the first stage row that is not `succeeded` (SEC-§1.3); succeeded windows/reduces are never re-sent.                                                                          | STT jobs are atomic per segment, so job-level resume suffices there (STT-§3.4). A summary run spans many provider calls; redoing them would violate Phase 7 acceptance "Neuversuche duplizieren keine Aufgaben oder Zwischenergebnisse" (plan line 1643). |
| D3  | Additional **permanent** error class `evidence_validation_failed`: M3 publication rejected by invariant E1 (SEC-§3.3) maps to `failed` with that `last_error` class and no automatic retry.                                                                                                              | STT-§6 has no analog: missing evidence is a content defect of the model output, not a transient condition; retrying unchanged inputs cannot fix it. It surfaces as a distinct, explainable failure (plan principle 8, line 388–393).                      |
| D4  | Own timeout/backoff parameters (mechanism identical to STT-§3.5: persisted `next_attempt_at`, bounded exponential backoff, deterministic jitter from the job ID). Start values: timeout 90 s + 1 s per 1,000 estimated input tokens, max 600 s; `max_attempts` 5; backoff base 5 s, factor 2, cap 5 min. | STT timeouts scale with audio minutes (STT-§3.5); LLM latency scales with window tokens. Same machinery, differently parameterized — no second mechanism introduced.                                                                                      |
| D5  | Cancellation leaves **already-succeeded stage rows in place** (usable by a future re-run via a new `rerun_generation`, SEC-§2.1); the job row itself goes to terminal `cancelled` exactly like STT-§3.2.                                                                                                 | Matches the write-once intermediate discipline (SEC-§1.3) and avoids paying twice for work the user already approved; differs from STT where partial results are discarded because none exist mid-job.                                                    |

Every other transition condition (user cancel from `queued`/`running`/
`retry_wait`; explicit user re-queue from `failed`/`cancelled` creating a new
generation; forbidden transitions) is inherited without modification.

### 4.3 Session-state interplay

The session column `summary_state` / session state `summarizing`
(plan lines 1176, 885) is a **derived view**, never independently writable:
a session is `summarizing` iff at least one `meeting_summary` processing job
is in `queued`/`running`/`retry_wait`, and `completed` only when the latest
run is `succeeded` (or the user disabled summarization). The derivation runs
inside the same startup reconciliation transaction as the STT job recovery
(STT-§3.4 last rule), so UI, tray and detail view can never disagree with
persisted reality.

---

## 5. Evidence citation format (gate G4)

### 5.1 Structured element record

Every element of the four structured arrays stored in `meeting_summaries`
(plan line 1216 — `decisions`, `action_items`, `open_questions`, plus
`key_points` for the Kernaussagen section) carries:

```text
SummaryElement {
  element_id   : string            // "<kind>-<4-digit ordinal>", assigned in
                                   // M2 output order (window ordinal, then
                                   // in-window position) — deterministic
  kind         : "key_point" | "decision" | "action_item" | "open_question"
  text         : string            // claim as shown in the UI
  evidence     : EvidencePointer[] // >= 1 entry; enforced at M3 (SEC-§3.3 E1)
}

EvidencePointer {
  segment_id     : uuid           // FK -> meeting_transcript_segments.id
  meeting_id     : uuid           // redundant for link construction
  start_ms, end_ms : integer      // segment.start_ms <= start <= end <=
                                  // segment.end_ms (SEC-§3.2 time_range rule)
  source         : "microphone" | "system"   // must equal segment.source
  quote_excerpt  : string         // verbatim excerpt, whitespace-normalized
                                  // containment check per SEC-§3.3 rule 3
  rationale      : string         // why this location supports the claim
}
```

Validation at M3 resolves every pointer against the database (existence,
non-tombstoned, ownership, excerpt containment, range containment — the five
checks of SEC-§3.3). Injection text cannot mint evidence: a fabricated
segment ID fails resolution and blocks the whole publication instead of
degrading gracefully (SEC-§7 rule 3).

### 5.2 Consumability contracts

1. **Detail view.** `element.evidence[i] -> segment_id` resolves to the
   transcript tab row; `start_ms/end_ms` select the highlight range. The
   transcript tab needs nothing beyond these two fields.
2. **Jump-to-position deep link.** The minimal navigation payload is the
   tuple `(meeting_id, start_ms)` — sufficient to address an audio playback
   position while audio exists (plan line 1415). The concrete URI/deep-link
   syntax is owned by integration-leaf-4 and is **pending**; this document
   freezes only the payload contract above so the link builder can be
   implemented independently. Owner: integration-leaf-4.
3. **Rendered markdown.** `summary_markdown` renders each element with a
   trailing citation label `[Beleg · HH:MM:SS]` produced from the structured
   record by the single `t(ms)` renderer of §1.2. Labels are **generated,
   never parsed back**; the structured JSON columns remain the canonical
   data, so export/copy flows cannot corrupt evidence.
4. **Task extraction.** Every `action_item` maps one-to-one onto a
   `project_tasks` suggestion; each evidence entry becomes a `task_evidence`
   row with locator `meeting_transcript_segment_id` and optional time range
   (plan lines 1295–1301), enabling the "von der Aufgabe zur belegenden
   Zeitposition springen" flow (plan line 1423) through the same §5.2
   resolution path.

> **INV-D (testable):** For every published summary, resolving any
> pointer's `segment_id` yields a segment row of the same meeting whose
> bounds contain the pointer range:
> `segment.start_ms <= pointer.start_ms` and
> `pointer.end_ms <= segment.end_ms`.
> Enforced in the M3 transaction; asserted by a post-publish integrity test.

---

## 6. Untrusted-transcript posture (gate G5)

Normative basis (reused, not re-derived): plan lines 1360–1362 (spoken
"instructions" must never override summary instructions), plan lines
1399–1403, SEC-§7 (structural separation, no agency, output validation,
scope containment). Application to this data flow: untrusted rendered
segments (§1.2) enter only inside delimited, labeled regions of the user
message; all instructions come from the version-pinned trusted templates
(SEC-§6); model outputs can become only schema-validated elements and
database-resolved evidence pointers (§5.1).

**task-0052 citation status: PENDING.** The task-0052 prompt-injection memo
was searched for and is **not present in this workspace** at authoring time
(`grep` over `docs/` finds the string only inside
`summary-evidence-contract.md`'s own disclosure table; no artifact file).
This document therefore marks the citation as pending with **owner: task-0052
research agent**. When the memo lands, §6 gains a one-line reference to it
and the mitigation list is consumed by Phase 10's injection tests
(plan line 1702) — no re-derivation happens here.

---

## 7. Complexity budget and non-goals

Consistent with the mission's simplicity mandate, the following are
explicitly rejected for this pipeline, each with its reason:

| Non-goal                                           | Reason for rejection                                                                                                                 |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Provider/model selection logic in this dataflow    | owned by task-0052 research + phases 7/10; the flow is provider-agnostic by construction (§3.3 estimator avoids tokenizer coupling). |
| Live/incremental re-summarization during recording | plan §6.12 forbids presenting results while jobs run; adds state-machine surface with no v1 user value.                              |
| Speaker names / diarization-aware windows          | excluded product-wide (plan lines 76–79); `source` is the only safe label (plan lines 93–95).                                        |
| Cross-meeting deduplication or memory              | belongs to ProjectIntelligenceService (SEC-§1.2); duplicating it here doubles conflict surfaces.                                     |
| Vector/embedding recall over transcripts           | plan lines 1329–1332 keep FTS5 sufficient for v1.                                                                                    |
| Mid-segment window splitting, sliding overlaps     | breaks segment-aligned evidence ranges (INV-D) for negligible budget gain.                                                           |
| Per-window adaptive prompting                      | nondeterministic prompt selection would break INV-A1 and SEC-§6 pinning for no evidenced benefit.                                    |

---

## 8. Gate verification checklist

| Gate | Requirement                                                                                             | Where satisfied                                                                                       |
| ---- | ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| G1   | Deterministic assembly + byte-identical request invariant                                               | §1.1–§1.4 (INV-A1), §2 edge table, §3.5 rule 2                                                        |
| G2   | Token-budget table for ≥ 2 context lengths from §12 figures with arithmetic                             | §3.1 (rate chain from plan line 1438), §3.3, §3.4 (8,192 / 32,768 / 131,072)                          |
| G3   | Job lifecycle reuses/diverges from STT state machine, divergences justified and cited                   | §4.1 (verbatim reuse of STT-§3.2/§3.4), §4.2 (D1–D5 with citations), §4.3                             |
| G4   | Citation format resolves to segment ID + time range consumable by detail view and jump-to-position link | §5.1 (record), §5.2 (consumers; deep-link syntax pending, owner integration-leaf-4), INV-D            |
| G5   | Injection posture referenced, task-0052 cited or pending with owner                                     | §6 (pending, owner task-0052)                                                                         |
| G6   | Prettier-clean, touches only this file                                                                  | verified via `node node_modules/prettier/bin/prettier.cjs --check` and `git status` at authoring time |
