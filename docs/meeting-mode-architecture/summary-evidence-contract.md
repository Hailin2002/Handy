# Summary/Evidence Contract — MeetingSummaryService / ProjectIntelligenceService

Status: **spec contract — no implementation yet** (Phase 7/Phase 10 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 (`MeetingSummaryService`, plan
lines 253–259; `ProjectIntelligenceService`, plan lines 293–299), §9
(`task_evidence` plan lines 1295–1301, `project_snapshots`/`snapshot_evidence`
plan lines 1303–1311, `processing_jobs` plan lines 1313–1318, search/evidence
rule plan lines 1329–1332), §10 (Map-Reduce plan lines 1349–1358,
untrusted-transcript rule plan lines 1360–1362, Projektintelligenz plan lines
1380–1403), principle 8 "Belege vor KI-Automatik" (plan line 135), §12 storage/
privacy logic (plan lines 1436–1476), §13 Phase 7 (tasks plan lines 1629–1638,
acceptance plan lines 1640–1645) and Phase 10 (tasks plan lines 1694–1703,
acceptance plan lines 1705–1712).
Planned implementation homes: `src-tauri/src/managers/meeting_summary.rs`,
`src-tauri/src/managers/project_intelligence.rs` (plan lines 259, 299).
This document defines the cross-module contracts of the two AI subsystems only;
it changes no source code.

Sibling contracts consumed by this document (read-only):

- [stt-provider-contract.md](stt-provider-contract.md) — job state machine
  (§3.1/3.2), idempotency-key discipline (§3.3), restart resumption (§3.4).
  Below abbreviated **STT-§n**.
- [repository-boundary-contract.md](repository-boundary-contract.md) —
  MeetingStore/KnowledgeStore trait seam, entity-ownership matrix (§3),
  unit-of-work operations K7/K8/K10 (§4), choke-point rules (§7). Below
  abbreviated **RB-§n**.
- [session-state-contract.md](session-state-contract.md) — conflict rule and
  documentation conventions. Below abbreviated **SSC-§n**.

Conflict rule (identical to SSC head and the documentation index): the plan
wins over this contract, this contract wins over catalogs, and nothing wins
over actual code.

External-input disclosure (gate G5): see §8.

---

## 0. Scope, subsystems, and non-goals

### 0.1 The two subsystems under contract

Both services share one architectural skeleton — **stage pipeline +
persisted intermediates + idempotent publication + evidence-backed claims** —
but differ in scope and cadence:

| | `MeetingSummaryService` | `ProjectIntelligenceService` |
| --- | --- | --- |
| Input | full sorted transcript of exactly one meeting (`meeting_transcript_segments`) | changed/new `source_items` of one project since last run (`change_log`-driven) |
| Output | one `meeting_summaries` row (markdown + structured JSON) | one new `project_snapshots` version + `tasks` suggestions |
| Trigger | after transcription completes; explicit re-summarize (plan line 1417) | incremental after source changes (plan lines 1382–1390) |
| Provider freedom | independent of STT choice (plan lines 1340–1347) | local/cloud per project run (plan line 1703) |

### 0.2 Explicitly out of scope (non-goals)

- STT job semantics, audio chunking, engine leases — owned by STT-§3.
- SQLite schema ownership and migration layering — owned by RB-§0/§3; this
  contract *consumes* RB seams and proposes two additional tables (§2.3).
- Concrete prompt texts, model recommendations, cost estimates — product
  decisions outside an architecture contract.
- Discovery-cycle feature additions: this contract implements no capability
  beyond what plan lines 253–259, 293–299 and 1349–1403 already name. Any
  future extension (e.g. vector search behind `SearchService`) stays behind
  the interfaces named here, per plan lines 106–110.
- UI copy and confirmation flows — proposal-only catalogs own those.

---

## 1. Pipeline stage diagram and boundaries (gates G1)

### 1.1 MeetingSummaryService stages

```text
meeting_transcript_segments (sorted, immutable)
  │
  ▼
M0  WINDOW          deterministic split into time-contiguous windows
  │                 (plan step 1, line 1351). NO model call. Pure function
  │                 of (segments, windowing parameters). Persisted.
  ▼
M1  MAP             per-window extraction: Fakten, Entscheidungen, Aufgaben,
  │                 offene Punkte (plan step 2, line 1352). One LLM request
  │                 per window. Each result persisted immediately.
  ▼
M2  REDUCE          deduplicate + merge all map results into one protocol
  │                 (plan steps 3–4, lines 1353–1354). May itself be chunked
  │                  recursively for very long meetings; each partial reduce
  │                 is a persisted stage instance of the same shape.
  ▼
M3  PUBLISH         validate evidence pointers (§3) and write the final
                    `meeting_summaries` row in ONE transaction.
```

Stage boundary rules:

1. **M0 is deterministic and replayable.** Given the same segment set and
   windowing parameters it yields byte-identical windows. It never calls a
   model, so a crash inside M0 loses nothing that cannot be recomputed; the
   persisted window plan exists so M1 jobs have stable identities (§2.2).
2. **Each M1 window is independently retryable** (STT-style job semantics:
   `queued → running → retry_wait → succeeded/failed/cancelled`, STT-§3.1).
   A window that has succeeded is never re-sent to the provider on re-run.
3. **M2 starts only when all M1 stages of the run are `succeeded`.**
   Partial reduces persist their output so a crash between M2 chunks resumes
   at the next chunk, not at zero.
4. **M3 is atomic.** Evidence validation, summary-row insert and job-state
   update happen in one SQLite transaction (same pattern as STT-§3.3 rule 2).
   There is no observable state "summary published without evidence check".

### 1.2 ProjectIntelligenceService stages

```text
change_log + sources + current project_snapshots + open tasks
  │
  ▼
P0  DELTA           select new/changed source_items up to a cutoff;
                    persist covered_through candidate. Deterministic.
  ▼
P1  EXCERPT         build source excerpts WITH evidence positions
                    (plan line 1387: "Quellenauszug mit Belegen").
                    Optional per-batch LLM summarization. Persisted per batch.
  ▼
P2  RECONCILE       compare excerpts against current snapshot and open
                    tasks (plan line 1388). Detects contradictions and
                    matches old/new tasks via stable IDs + semantic
                    fingerprints (plan lines 1393–1394). Persisted.
  ▼
P3  PUBLISH         conflict-check against protected manual edits (§5),
                    then write new project_snapshots version + task
                    suggestions + snapshot_evidence in ONE transaction
                    (plan line 1389).
```

Stage boundary rules mirror §1.1 rules 2–4: per-stage persistence, per-stage
retryability, sequential gating, atomic publish.

### 1.3 Persisted intermediate-result record (shared)

One generic table serves both services (proposed name:
`ai_pipeline_stages`; schema authority stays with RB-§0):

```sql
CREATE TABLE ai_pipeline_stages (
  id                      TEXT PRIMARY KEY,      -- stable UUID (plan line 1151)
  job_id                  TEXT NOT NULL,         -- FK -> processing_jobs
  service                 TEXT NOT NULL,         -- 'meeting_summary' | 'project_intelligence'
  stage                   TEXT NOT NULL,         -- 'window'|'map'|'reduce'|'publish'|'delta'|'excerpt'|'reconcile'
  idempotency_key         TEXT NOT NULL,         -- derivation per §2
  subject_id              TEXT NOT NULL,         -- meeting_id | project_id
  ordinal                 INTEGER,               -- window/chunk index within the run
  payload_schema_version  INTEGER NOT NULL,
  payload                 TEXT NOT NULL,         -- JSON, never contains secrets
  state                   TEXT NOT NULL,         -- 'pending'|'succeeded'; failures live on the job
  provider                TEXT,                  -- pinned per §6 (null for deterministic stages)
  model                   TEXT,
  prompt_version          TEXT,
  prompt_content_hash     TEXT,
  created_at              TEXT NOT NULL,
  updated_at              TEXT NOT NULL,
  UNIQUE (job_id, stage, ordinal),
  UNIQUE (idempotency_key)
);
```

Properties:

- **Intermediates are write-once.** A stage row transitions
  `pending → succeeded` in the same transaction that stores its payload;
  there is no in-place mutation of a succeeded payload. A re-run that wants a
  different result creates a NEW run (new `job_id`), never overwrites (STT-§3.3
  rule 1 logic: attempts share the key; deliberate re-runs get a new
  generation).
- **Payloads carry provenance**: every model-touched payload records
  provider/model/prompt version at write time (§6), satisfying Phase 10
  line 1703 ("persistierte Prompt-/Modellversion") at stage granularity, not
  only at artifact granularity.
- **No secrets** in payloads or keys (plan lines 1334–1336).

### 1.4 Job envelope

Every run is wrapped in a `processing_jobs` row (plan lines 1313–1318) with
the state/retry semantics of `meeting_stt_jobs` (plan lines 1316–1317, i.e.
STT-§3 applies verbatim). The generic idempotency key demanded by plan line
1317 ("ein idempotency key verhindert doppelte Ergebnisse") is derived per
§2 below.

---

## 2. Idempotency-key derivation per stage (gate G1)

All keys are `SHA-256` over the listed components joined with `\x1f`
(unit-separator), following the formation style of STT-§3.3 rule 1.
`pipeline_version` denotes the compiled stage-graph version (bumped whenever
stage inputs/outputs change meaning). Keys never contain raw transcript text —
only hashes and identifiers.

### 2.1 Run-level keys

| Run | Key derivation |
| --- | --- |
| Meeting summary run | `SHA-256("meeting-summary", meeting_id, transcript_basis_hash, summary_provider_id, provider_settings_snapshot, prompt_version, rerun_generation)` where `transcript_basis_hash = SHA-256(sorted (segment_id, content-hash) pairs of all segments included)`. A plain retry after failure/crash keeps the key (`rerun_generation` unchanged); an explicit "erneute Zusammenfassung" (plan line 1417) increments it — same distinction as STT-§3.3 rule 1. |
| Project intelligence run | `SHA-256("project-intelligence", project_id, delta_basis, provider_id, provider_settings_snapshot, prompt_version, rerun_generation)` where `delta_basis = SHA-256(sorted (source_id, source_revision, per-source item-content-hash-set))` of everything P0 selected. |

`UNIQUE(idempotency_key)` on `processing_jobs` prevents double-enqueue of the
same logical run (analog of STT-§3.3 rule 6).

### 2.2 Stage-level keys

| Stage | Key derivation | Duplicate guard |
| --- | --- | --- |
| M0 window | `SHA-256(run_key, "window", window_ordinal, first_segment_id, last_segment_id)` | `UNIQUE(job_id, stage, ordinal)` |
| M1 map | `SHA-256(run_key, "map", window_id)` | `UNIQUE(idempotency_key)`; pre-insert existence check makes a replayed provider response a no-op (analog of STT-§3.3 rule 5) |
| M2 reduce | `SHA-256(run_key, "reduce", reduce_ordinal, sorted child-stage keys)` | as above; recursive reduces chain deterministically |
| M3 publish | `SHA-256(run_key, "publish")` | success is booked atomically with the summary row (§1.1 rule 4); a post-commit crash finds the job `succeeded` and does nothing (STT-§3.3 rules 3–4) |
| P0 delta | `SHA-256(run_key, "delta", covered_through_cursor_value)` | as above |
| P1 excerpt | `SHA-256(run_key, "excerpt", source_id, source_revision, sorted item content-hashes of the batch)` | as above — re-imports that produce identical item hashes reuse, not duplicate (plan line 1687 analog) |
| P2 reconcile | `SHA-256(run_key, "reconcile", sorted excerpt-stage keys, current_snapshot_id)` | as above |
| P3 publish | `SHA-256(run_key, "publish", conflict_scan_result_hash)` | atomic with snapshot insert; `UNIQUE(project_id, idempotency_key)` on snapshots prevents two "current" versions from one logical run |

### 2.3 Task-suggestion deduplication (Phase 10 line 1709)

A retry must not create duplicate task suggestions (plan lines 1393–1394).
Rules:

1. Each suggestion carries `dedupe_fingerprint =
   SHA-256(project_id, normalized_title, normalized_assignee_or_null,
   due_date_or_null, semantic_feature_set_hash)` computed by P2 from the
   merged evidence.
2. Before insert, P3 resolves the fingerprint against existing tasks:
   - match on a previous **AI suggestion** (same fingerprint, still
     `suggested`) → update-in-place of that suggestion row, no new row;
   - match on an **open human task** → no new suggestion; the excerpt is
     attached as additional evidence/comment to the existing task instead;
   - match on a `done`/`dismissed` human decision → no resurrection (§5).
3. The insert of `(task, task_evidence, fingerprint uniqueness)` is one
   transaction with `UNIQUE(project_id, dedupe_fingerprint)` on AI-origin
   task rows.

This is the intelligence-side counterpart of STT's
"Neuversuche duplizieren keine Aufgaben oder Zwischenergebnisse"
(Phase 7 acceptance, plan line 1643).

---

## 3. Evidence pointer model (gate G2)

### 3.1 Principle

Plan line 135: *"Jede KI-generierte Statusaussage und Aufgabe verweist auf
Meeting-Zeitstempel, Chatnachricht oder Datei."* Plan line 1300:
*"ein KI-Vorschlag ohne mindestens einen gültigen Beleg wird nicht als
Aufgabe veröffentlicht."* This contract generalizes that rule from tasks to
all publishable AI claims of both services.

### 3.2 Required fields

An evidence pointer (`task_evidence`, plan lines 1295–1301;
`snapshot_evidence`, plan lines 1303–1311; summary-item evidence below) MUST
contain:

| Field | Requirement |
| --- | --- |
| `subject` | entity-ID of the claim carrier (task ID, snapshot-statement ID, summary-item ID) |
| `source_id` | FK to `sources`; always required |
| `locator` | exactly one of `source_item_id` OR `meeting_transcript_segment_id` (plan line 1297) |
| `time_range` | optional `[start_ms, end_ms]`; REQUIRED when locator is a transcript segment, must lie within that segment's `[start_ms, end_ms]` |
| `quote_excerpt` | short verbatim excerpt from the located source text |
| `rationale` | why this location supports the claim (plan line 1299 "Erklärungsgrund") |

### 3.3 Resolution invariant (service-boundary rejection)

**Definition (valid pointer):** a pointer is *valid* iff at publish time:

1. the referenced `sources` row exists and is not tombstoned
   (`deleted_at IS NULL`);
2. the referenced locator row exists, is not tombstoned, and belongs to that
   source;
3. if a `quote_excerpt` is given, the current text of the locator still
   contains it (whitespace-normalized comparison) — a stale quote means the
   source changed underneath the run and the pointer is invalid;
4. if `time_range` is given, it lies within the segment bounds.

**Invariant E1:** *The publish transaction of every stage M3/P3 rejects the
whole publication if any AI claim it would make public carries zero valid
evidence pointers.* Rejection is at the service boundary (inside the
KnowledgeStore unit-of-work, extending RB-§4 operation K8
`attach_task_evidence`, which already enforces "an AI task without ≥1 valid
evidence is never published" per RB-§4 K8 and plan lines 1300–1301). A
rejected publication marks the job `failed` with a distinct error class
(`evidence_validation_failed`) so the UI can explain it; it never silently
strips individual claims.

**Invariant E2:** *Search and UI jump targets resolve.* Consistent with plan
lines 1329–1332 ("Suchtreffer liefern immer Entity-ID und Belegposition"),
every stored pointer must be resolvable to a UI navigation target
(meeting time position while audio exists, chat message, file region).

**Invariant E3:** *Contradiction is preserved, not resolved.* Where P2 finds
conflicting sources, the snapshot records BOTH positions with their pointers
as a conflict entry (plan lines 1397–1398); a conflict entry needs valid
pointers like any claim and must never be collapsed into one invented truth.

### 3.4 Meeting-summary application

The structured `decisions`, `action_items`, `open_questions` JSON arrays of
`meeting_summaries` (plan line 1216) each embed per-item evidence pointers
into `meeting_transcript_segments` (entity-ID + `[start_ms, end_ms]` +
excerpt), so Phase 8's acceptance "Status- und Aufgabenansichten zeigen keine
Aussage ohne erreichbaren Beleg" (plan line 1668) and Phase 10's "jede
KI-Aussage und Aufgabe hat mindestens einen gültigen Beleg" (plan line 1708)
hold uniformly. Free prose inside `summary_markdown` is exempt from per-
sentence pointers but inherits traceability from the structured sections.

---

## 4. Protected manual changes and conflict detection (gate G3)

### 4.1 What is protected

Plan lines 263–265 ("Manuelle Nutzeränderungen besitzen Vorrang vor späteren
KI-Läufen"), 1224–1225 (manual project status note may not be overwritten by
AI), 1395–1396 ("Nutzeränderungen sind geschützte Fakten … weder
Abschlussstatus noch Priorität unbemerkt zurücksetzen"), 1709–1710.

Protected column set:

| Table | Protected columns (when touched by a human) |
| --- | --- |
| `tasks` | `title`, text/description, `state`, `priority`, due date, assignee — flagged by `origin='human'` OR `human_edited_at IS NOT NULL` (columns exist per plan lines 1286–1287) |
| `projects` | manually maintained status note (plan lines 1224–1225) |
| `project_sources` | manual assignment/confirmation (`origin='manual'`, `confirmed_at`) |
| meeting summaries | anything the user edited after publication is a UI-layer annotation, not an AI-writable surface; AI re-runs create NEW summary rows and never mutate user annotations |

### 4.2 Precedence rules

1. **AI runs never issue UPDATE statements against protected columns of
   human-flagged rows.** AI influence is expressed exclusively through:
   new suggestion rows, `dismissed→suggested`-style proposals that require
   explicit user confirmation, and additive `ai_comment` annotations.
2. **Human decisions are terminal against equal evidence:** a `dismissed`
   or `done` human decision suppresses re-suggestion with the same
   `dedupe_fingerprint` (§2.3, resolution rule 2, third case).
3. **AI may comment, never silently revert** (plan line 1396): if new
   sources contradict a human decision, the snapshot shows a conflict entry
   referencing both the human decision and the contradicting evidence.

### 4.3 Conflict-detection seam

Named seam: **`manual_change_guard`, executed inside the P3 publish
transaction of the KnowledgeStore unit-of-work** (extension of RB-§4; the
KnowledgeStore remains the only writer, RB-§7 choke-point rules apply).

Protocol:

1. During P0/P2 the run records `baseline_revision` for every entity it
   reads (entities carry monotone `revision`, plan line 1152–1153).
2. In the publish transaction, before any write, the guard re-reads the
   current `revision` of each target entity and compares:
   - `revision == baseline_revision` → entity writable by this run;
   - `revision > baseline_revision` → **conflict**: the entity is excluded
     from AI writes; the snapshot records the entity, both revisions and a
     human-readable conflict notice (plan line 1397: contradictions become
     visible rather than flattened).
3. Because check and write share one transaction (SQLite serializes
   writers), no interleaved user edit can slip between guard and commit —
   the same atomicity argument as STT-§3.3 rule 3.
4. The guard's decision (per-entity allow/conflict) is part of the P3 stage
   payload, making every precedence decision auditable.

For meeting summaries the equivalent seam is trivial: M3 inserts a new
`meeting_summaries` row keyed to a new run; it holds no write path to
user-facing edits at all.

---

## 5. Data-privacy rule: local mode sends nothing off-device

Plan line 1644 (Phase 7) and line 1711 (Phase 10):
*"lokale Verarbeitung sendet nachweislich keine Nutzdaten an Cloud-Provider."*
"Nachweislich" makes this an architectural property, not a configuration
promise:

1. **Type-level separation.** Both services depend on a summary/intelligence
   provider trait with a local variant whose implementation performs no HTTP
   client construction. The cloud paths go through the existing
   OpenAI-compatible client infrastructure (`src-tauri/src/llm_client.rs`
   `send_chat_completion_with_schema` — verified present at authoring time;
   secrets enter only as function arguments and are never persisted, matching
   plan lines 1334–1336).
2. **Egress assertion test.** A mandatory integration test runs a full
   local-mode summary and a full local-mode intelligence pass against a
   fixture corpus while a test-installed socket guard fails the process on
   any outbound TCP connect. The test passes only with zero connects. This
   is the executable form of "nachweislich".
3. **UI symmetry.** Whenever a cloud provider is selected, the pre-run and
   in-run disclosures follow plan lines 1454–1456 and 1690 (provider and
   affected data types shown before every cloud run); the provider pinning of
   §6 makes each run's data flow reconstructible afterwards (plan lines
   1470–1476).
4. **Cloud minimization.** Cloud models receive only the chunks/excerpts the
   current stage needs, never the whole archive (plan lines 1402–1403);
   window/batch sizing is the mechanism, evidenced by the per-stage payloads
   of §1.3.

---

## 6. Prompt/model version pinning (gate G4)

### 6.1 Fields persisted per run

Phase 10 line 1703 demands "persistierte Prompt-/Modellversion". Every run
(M1/M2 resp. P1/P2 model-touching stages, plus M3/P3 publications) persists:

| Field | Meaning | Anchor |
| --- | --- | --- |
| `provider` | provider identifier + non-secret settings snapshot | plan lines 1196, 1213, 1308 |
| `model` | exact model identifier sent to the endpoint | plan lines 1213, 1286 |
| `prompt_version` | human-meaningful template version (e.g. `meeting-map@3`) | plan lines 1213, 1286, 1308 |
| `prompt_content_hash` | SHA-256 of the exact rendered instruction block | strengthens reproducibility beyond the plan minimum |
| `payload_schema_version` | version of the structured-output schema used | plan line 1154 (schema/parser versions) |

Storage locations: `meeting_summaries` (plan lines 1210–1217),
`project_snapshots` (plan lines 1305–1308), `tasks` origin metadata
(plan line 1286), and every `ai_pipeline_stages` payload (§1.3). Secrets are
never among these fields (plan lines 1334–1336).

### 6.2 Retention tie-in (plan §12)

- Version/provenance fields are **part of the artifact**, not an independent
  log: deleting a summary/snapshot deletes exactly its provenance fields with
  it, honoring Phase 7 acceptance "Löschung entfernt exakt die in der
  Bestätigung genannten Dateien und Datensätze" (plan line 1645).
- Intermediates inherit the retention policy of their subject (audio/
  transcript retention per plan lines 1444–1452): once a run reaches a
  terminal state (`succeeded`/`failed`-terminal/`cancelled`), its
  intermediates are deletable under the same rule; a failed-but-retryable
  job blocks intermediate cleanup exactly as failed STT jobs block audio
  deletion (plan lines 1451–1452).
- For imported sources, the §12 provenance listing (plan lines 1470–1476:
  parser, model, prompt version, retention rule per import) consumes these
  same fields — one pinning mechanism feeds both obligations.

---

## 7. Prompt-injection boundary: transcripts and imports are untrusted input

Normative basis: plan lines 1360–1362 (*"Transkripttext wird als nicht
vertrauenswürdiger Inhalt klar vom Systemprompt getrennt. Gesprochene Sätze
wie ‚Ignoriere alle vorherigen Anweisungen' dürfen die Summary-Anweisungen
nicht überschreiben"*), plan lines 1399–1401 (imported text/files/metadata
must not trigger tools, provider switches or data sharing), §12 line 1463
(WhatsApp archives and imported files are untrusted input).

Boundary rules for both services:

1. **Structural separation.** Untrusted content enters only inside delimited,
   labeled blocks of the user message (e.g. fenced `<transcript>` /
   `<source_item>` regions with per-item IDs). Instruction-like text inside
   these blocks is data. The system message contains all instructions and is
   assembled exclusively from trusted, version-pinned templates (§6).
2. **No agency.** The models behind M1/M2/P1/P2 receive no tool definitions,
   no ability to select providers, and no ability to widen data scope. The
   only effects a model response can have are: parsed structured output and
   evidence-pointer candidates — both validated before persistence.
3. **Output validation.** Structured outputs are schema-validated; extracted
   evidence locators are resolved against the database (§3.3). Injection
   text cannot mint valid evidence: a pointer must resolve to real stored
   entities, and a fabricated segment ID fails validation and blocks
   publication (E1) rather than degrading gracefully.
4. **Scope containment.** A poisoned transcript can corrupt at most its own
   run's outputs; per-meeting and per-project scoping of runs (run keys, §2.1)
   prevents cross-entity contamination, and P2 conflict detection surfaces
   contradictions instead of merging attacker-favored "truths" (§3.3 E3).

Mitigations researched by task-0052 (prompt-injection defense research) are
consumed as **input** to implementation hardening and Phase 10's
"Prompt-Injection-Tests" (plan line 1702); this contract deliberately does
not re-derive them. Their citation status is disclosed in §8.

---

## 8. External-citation disclosure (gate G5)

| Citation | Status at authoring time |
| --- | --- |
| task-0052 (prompt-injection mitigation research) | **NOT-LIVE-VERIFIED.** The task-0052 output artifact was not present in this workspace at authoring time (searched: no file matching `*0052*`; no `docs/` hit besides unrelated matches). §7 cites it by reference only. Nothing from task-0052 is relied upon normatively here; implementers must locate and verify the artifact before using it. |
| `src-tauri/src/llm_client.rs` (existing provider/secret infrastructure, §5.1) | **LIVE-VERIFIED** — file read in this workspace: `send_chat_completion_with_schema(provider, api_key, model, user_content, system_prompt, json_schema, disable_reasoning)` exists; API key is passed per-call, never persisted. |
| All other claims | Derived solely from `docs/meeting-mode-plan.md` (line-numbered quotes above) and sibling architecture contracts in this repository. No new external (web/vendor) claims are made. |

---

## 9. Gate verification checklist

| Gate | Requirement | Where satisfied |
| --- | --- | --- |
| G1 | Stage diagram covers map/reduce/snapshot with persisted-intermediate record + per-stage idempotency-key derivation | §1.1 (M0–M3), §1.2 (P0–P3), §1.3 (record), §2.2 (derivations) |
| G2 | Evidence pointer fields + boundary rejection invariant citing plan line 1300 | §3.2 (fields), §3.3 (invariant E1, plan lines 1300–1301, RB-§4 K8) |
| G3 | Protected-manual-edit detection/preservation with named columns/seam | §4.1 (columns), §4.3 (`manual_change_guard` seam, transactional revision compare) |
| G4 | Prompt/model version fields persisted per run + §12 retention tie-in | §6.1 (fields, plan line 1703), §6.2 (retention, plan lines 1444–1452, 1645, 1470–1476) |
| G5 | task-0052 citations marked not-live-verified; no new unverified external claims | §8 (disclosure table) |
