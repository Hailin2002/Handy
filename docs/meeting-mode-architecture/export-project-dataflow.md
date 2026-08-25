# Export & Project-Unassignment Dataflow — Meeting Library × Serializers × Task Evidence

Status: **spec contract, Phase 7 not implemented — changes no source code**
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md), Abschnitt 11
„Meeting-Bibliothek und Export" (lines 1405–1434), Abschnitt 12 (Aufbewahrung,
lines 1444–1452), Abschnitt 7 „Persistenzformat" (lines 1007–1017),
Abschnitt 9 (Datenmodell, lines 1143–1336), Abschnitt 10 (Standard-Zusammenfassung,
lines 1364–1378)
Builds on:
[phase2-contracts](../meeting-mode-phase2-contracts.md) (session
`manifest.json`, chunk naming, atomic-finalize pattern),
[summary-evidence-contract.md](summary-evidence-contract.md) §3 (evidence
pointer model, invariants E1/E2),
[repository-boundary-contract.md](repository-boundary-contract.md)
(KnowledgeStore unit-of-work),
[startup-recovery-dataflow.md](startup-recovery-dataflow.md) (chunk state
reconciliation feeding audio availability)

This document defines **one end-to-end dataflow** for everything that leaves a
meeting as an artifact (Markdown/TXT/JSON export, optional audio export,
clipboard copy) and for the one flow that changes project linkage without
touching content (unassignment, plan lines 1431–1434). It also fixes the
single canonical deep-link type that connects extracted tasks, transcript
search hits, and the audio player to the same meeting timeline position.

---

## 1. Scope and precedence

- **In scope:** export query → serialize → write pipelines for the three
  formats of plan line 1419; the optional audio export (plan line 1420);
  copy-to-clipboard paths (plan line 1418); the canonical position reference;
  the project-unassignment flow including the `zu prüfende Belege` marking
  (plan lines 1431–1434); provenance metadata on every exported artifact.
- **Out of scope:** controlled deletion itself (only referenced where audio
  availability and the irreversible-delete confirmation interact with these
  flows); retention scheduling; summary generation; WhatsApp/media export;
  future formats (SRT/PDF); sync or server transfer of exports.
- **Precedence:** plan beats this contract; the evidence-pointer validity
  rules of summary-evidence-contract §3 (E1/E2) stay normative — this
  document only _adds_ a review marker and fixes join order/ownership, it
  never relaxes pointer validation.

## 2. Participants and ownership

| Component                | Planned home                                                                                                                                                                     | Role in this dataflow                                                                         |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `MeetingExportService`   | `src-tauri/src/managers/meeting_export.rs` (**gap vs. plan §5** — plan §5 names no export component; this contract assigns the home, to be ratified with the next plan revision) | Owns all queries, serializers, provenance envelope, file writes, clipboard payloads           |
| `MeetingStore`           | `src-tauri/src/managers/meeting_store.rs` (plan line 243)                                                                                                                        | Authoritative reads: sessions, chunks, segments, summaries                                    |
| `KnowledgeStore`         | `src-tauri/src/managers/knowledge_store.rs` (plan line 275)                                                                                                                      | Unit-of-work for unassignment transaction; owns `task_evidence.review_state` transition       |
| `ProjectManager`         | `src-tauri/src/managers/project.rs` (plan line 267)                                                                                                                              | Orchestrates unassignment; inbox visibility rule; event emission                              |
| `AssetStore`             | `src-tauri/src/storage/asset_store.rs` (plan line 308)                                                                                                                           | Relative storage keys; byte sinks under the portable-aware data dir                           |
| `portable.rs` (existing) | `src-tauri/src/portable.rs`                                                                                                                                                      | Path resolution authority: `app_data_dir()` (lines 69–75), `resolve_app_data()` (lines 88–90) |
| `MeetingDetail.tsx`      | `src/components/meetings/MeetingDetail.tsx` (plan line 320)                                                                                                                      | Triggers export/copy/unassign/jump; renders review badges                                     |

The React layer never touches SQLite or file paths directly (plan lines
216–217): it receives finished payloads (clipboard strings, saved-file
confirmation with display name) and entity IDs — never absolute paths.

## 3. Canonical position reference (deep-link contract)

### 3.1 One type, three consumers

Plan lines 1415, 1423 require jumping to time positions while audio exists,
including from an extracted task back to its evidencing position. All three
consumers — **task evidence**, **transcript search hits**, and the **audio
player** — MUST use exactly this reference type and no other:

```text
MeetingPositionRef {
  meeting_id:        UUID,          // meeting_sessions.id
  start_ms:          i64,           // shared timeline, plan §7
  end_ms:            i64 | null,    // null = play/read to segment end
  anchor_segment_id: UUID | null,   // meeting_transcript_segments.id
}
```

Canonical wire serialization (used in exported files, event payloads, and
logs; contains **no** filesystem path):

```text
speak-meeting-position://<meeting-id>?t=<start_ms>[&until=<end_ms>][&seg=<segment-id>]
```

### 3.2 Producer obligations

- `task_evidence` rows with locator `meeting_transcript_segment_id`
  (summary-evidence-contract §3.2) derive their `time_range` into a
  `MeetingPositionRef` at render time; they do not store a second format.
- FTS search hits over transcript segments (plan lines 1327–1332) carry
  `(segment entity-ID, Belegposition)` and convert to a
  `MeetingPositionRef` before navigation.
- The audio player accepts **only** `MeetingPositionRef` at its boundary and
  converts `start_ms` to a media clock offset itself (mapping through
  `meeting_audio_chunks.timeline_start_ms/end_ms`). Bare seconds-offsets or
  chunk-file references MUST NOT be persisted anywhere as links.

### 3.3 Resolution matrix (incl. deleted-audio behavior)

Resolution is a pure function returning exactly one outcome:

| Outcome                         | Precondition                                                                                                             | Player behavior                                                  | Transcript/UI behavior                             |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------- | -------------------------------------------------- |
| `Playable`                      | ≥1 chunk with `state = 'ready'` covers `start_ms` (chunks not yet purged under the retention rule, plan lines 1444–1452) | seek to `start_ms`, stop at `end_ms`                             | highlight anchor segment                           |
| `AudioDeleted`                  | meeting + transcript exist, covering audio was deleted („nur Rohaufnahme löschen", plan lines 1427–1428, or retention)   | player control hidden/disabled with explanatory copy             | scroll to and highlight anchor segment             |
| `SegmentMissing`                | meeting exists, anchor segment gone (e.g., re-transcription replaced it)                                                 | nearest segment containing `start_ms`, else seek by raw timeline | jump lands at nearest position, marked approximate |
| `Unresolvable(meeting_deleted)` | whole meeting finally deleted (plan line 1429)                                                                           | inert                                                            | inert, dead-link explanation; never an error crash |

Because failed or unfinished STT jobs prevent automatic audio deletion (plan
lines 1451–1452), every resolvable task produced during normal processing
keeps its `Playable` path until the user explicitly deletes raw audio — at
which point the UI switches to `AudioDeleted` deterministically. This
implements invariant E2 ("search and UI jump targets resolve",
summary-evidence-contract §3.3) across the retention boundary instead of
silently breaking links.

## 4. Text export pipelines (Markdown, TXT, JSON)

All three formats share one pipeline; only the serializer differs. There is
exactly one query stage, one provenance stage, one write stage.

```text
MeetingStore queries ──> ProvenanceEnvelope ──> Serializer(Md|Txt|Json)
        │                                              │
        │                                              ├─> clipboard payload (copy path, §6)
        │                                              └─> atomic file write (export path, §4.4)
        └─ (read-only; no locks, safe during recording)
```

### 4.1 Query source (identical for all three formats)

Executed inside one read snapshot against `MeetingStore` / `KnowledgeStore`:

1. `meeting_sessions` row for the meeting (title, `started_at`, `ended_at`,
   language, devices, providers);
2. the linked `sources` row (`source_type = 'meeting'`, plan lines
   1237–1239) — supplies `source_id` for provenance and unassignment;
3. `meeting_transcript_segments` ordered by `sequence`/`start_ms`;
4. the current successful `meeting_summaries` row (structured
   `decisions`, `action_items`, `open_questions` JSON plus
   `summary_markdown`), or `null` if none;
5. audio availability facts (distinct chunk states) for the
   `audio_availability` flag and the deep-link resolution matrix.

Exports of meetings still being transcribed are allowed and simply reflect
the current snapshot; partial coverage is visible through
`coverage.segment_count` vs. persisted job totals.

### 4.2 Serializer output shapes

**Markdown** (default; human-readable protocol):

- H1 title = meeting title, followed by a provenance header block (§5) as
  HTML comments;
- summary rendered per the plan §10 standard outline (lines 1364–1378);
  structured items carry `[mm:ss]` (or `[h:mm:ss]` beyond one hour) markers
  linked to their evidence `time_range`;
- chronological transcript section: one paragraph per segment, prefixed
  `[mm:ss–mm:ss] (microphone|system)`;
- no invented speaker names (plan lines 93–95, 1356–1358).

**TXT** (plain-text interchange):

- fixed header lines (`Titel`, `Datum`, `Dauer`, `Quelle-ID`,
  `Export-Version`);
- transcript lines `start–end\tquelle\ttext` in strict chronological order;
- no Markdown syntax, no HTML comments; provenance reduced to the header
  fields so the file survives naive text tooling.

**JSON** (machine-readable, round-trippable):

```json
{
  "export_format": "speak-meeting-export",
  "export_version": 1,
  "provenance": { "...": "see §5 — identical object in all formats" },
  "meeting": {
    "id": "<uuid>",
    "title": "...",
    "started_at": "...",
    "ended_at": "...",
    "language": "...",
    "stt_provider": "...",
    "has_microphone_audio": true,
    "has_system_audio": true,
    "recording_retention_policy": "..."
  },
  "summary": {
    "id": "<uuid>",
    "provider": "...",
    "model": "...",
    "prompt_version": "...",
    "summary_markdown": "...",
    "decisions": [],
    "action_items": [],
    "open_questions": []
  },
  "segments": [
    {
      "id": "<uuid>",
      "sequence": 1,
      "source": "microphone",
      "start_ms": 0,
      "end_ms": 4200,
      "text": "...",
      "provider": "...",
      "model_id": "..."
    }
  ],
  "audio_availability": { "playable": true, "ready_chunks": 42 }
}
```

Every JSON entity carries its stable entity ID (plan line 1151); every
segment carries its timeline range, satisfying gate G4 per element.

### 4.3 File-write location rule and the no-absolute-path rule

Two destinations exist; both obey the same rules:

1. **Default destination:** the session export directory from the plan §7
   layout (line 1015), resolved exclusively through the existing
   portable-aware helper — `portable::resolve_app_data(app,
"meetings/<meeting-id>/export/")` (`portable.rs` lines 86–90, delegating
   to `app_data_dir()` lines 69–75). This is the same resolution logic the
   plan mandates for all meeting storage (plan lines 167–170).
2. **User-chosen destination:** a frontend save-dialog picks a folder for
   one explicit export. The resulting absolute path is consumed exactly once
   by the write call and **never persisted** — not in SQLite, not in
   manifests, not in `change_log`, not in Tauri events, not in logs.

Filename convention: `<slug>--<meeting-id-short>--<yyyymmdd-hhmmss>.<md|txt|json>`
where `<slug>` is a filesystem-sanitized title prefix and the (shortened)
meeting ID is mandatory, making names self-tracing and collision-free; on
the rare residual collision the writer appends `-2`, `-3`, … instead of
overwriting. Writes are atomic: temp file in the target directory → fsync →
rename, mirroring the chunk-finalization pattern of
[phase2-contracts](../meeting-mode-phase2-contracts.md) §6.

**No-absolute-path invariant:** the database continues to store only asset
IDs and relative storage keys (plan lines 1158–1159). Exported documents may
reference each other only by relative filenames within their own bundle.
Any serializer output that would contain an absolute path is a contract
violation (test: serialize, scan output for the resolved data-dir prefix).

### 4.4 Manifest/reference integrity for multi-file exports

Single-document text exports need no manifest. Any export producing **more
than one file** (audio export, §5) MUST additionally write
`export-manifest.json` beside the members:

```json
{
  "export_manifest_version": 1,
  "provenance": { "...": "same envelope as §5" },
  "members": [
    { "name": "000001-microphone.wav", "bytes": 3842044, "sha256": "<hex>" }
  ],
  "gaps": [
    {
      "timeline_start_ms": 3600000,
      "timeline_end_ms": 3660000,
      "reason": "chunk_deleted_by_retention"
    }
  ]
}
```

Rules: members are listed by **relative name only**; each carries size and
SHA-256; consumers verify hashes before trusting any member; deleted or
damaged chunks (states `deleted`/`damaged`) appear in `gaps` with timeline
ranges rather than being silently omitted. The name
`export-manifest.json` is deliberately distinct from the session
`manifest.json` (phase2-contracts §2) so recovery reconciliation never
confuses the two; the session manifest remains authoritative for capture
facts, the export manifest only describes the bundle.

## 5. Provenance envelope (every artifact, gate G4)

One `ProvenanceEnvelope` object, embedded in every exported artifact —
inline JSON field, Markdown HTML-comment block, TXT header lines, audio
sidecar manifest:

| Field                                        | Source                                                           | Purpose                           |
| -------------------------------------------- | ---------------------------------------------------------------- | --------------------------------- |
| `meeting_id`, `source_id`                    | `meeting_sessions.id`, `sources.id`                              | primary traceability keys         |
| `generated_at_utc`                           | export time                                                      | reproducibility                   |
| `exporter`                                   | app version + `export_pipeline_version` constant                 | which code wrote the file         |
| `export_version` / `export_manifest_version` | constants                                                        | schema evolution (plan line 1154) |
| `stt_runs`                                   | distinct `(provider, model_id)` pairs from segments              | which engines produced which text |
| `summary_run`                                | `(provider, model, prompt_version)` of current summary or `null` | which model produced the summary  |
| `coverage`                                   | `{first_start_ms, last_end_ms, segment_count}`                   | what the export actually spans    |
| `content_hash`                               | SHA-256 over serialized body (JSON/audio)                        | detect later tampering/truncation |

With these fields any exported statement traces back to: the meeting, the
segment range (`[start_ms, end_ms]` in JSON segments and Markdown/TXT time
markers), and the generator version chain (STT provider/model, summary
model/prompt, exporter build).

## 6. Copy-to-clipboard paths

Two commands share the exact §4 query + serializer stages, minus the write
stage:

- `copy_meeting_summary(meeting_id)` → Markdown serializer restricted to the
  summary sections → clipboard as `text/markdown` (fallback `text/plain`);
- `copy_meeting_transcript(meeting_id, range?)` → TXT serializer restricted
  to the transcript section (optional timeline sub-range) → clipboard as
  `text/plain`.

Clipboard payloads contain entity IDs and `[mm:ss]` markers (optionally
`speak-meeting-position://` URIs) but **never** filesystem paths, so the
no-absolute-path rule holds trivially on this branch. The clipboard variant
omits the provenance comment block (pasted text must be clean prose);
traceability of clipboard content relies on the meeting title/date header
and time markers, matching the library UX of plan line 1418.

## 7. Optional audio export

- **v1 shape:** a ZIP bundle written like any other export (default dir §4.3
  or user-chosen). Members: all `state='ready'` chunk files of the selected
  sources, original PCM16-WAV bytes (no re-encode), plus
  `export-manifest.json` (§4.4) carrying the provenance envelope.
- **Derived stereo WAV** (mic left, system right; plan lines 1028–1031) is a
  permitted alternative bundle form, generated from the same ready chunks
  and declared in the manifest via a `derived` member entry with the mixer
  parameters; v1 ships the copy-first bundle because it is lossless-by-
  construction and needs no new DSP code.
- Availability gating: if zero ready chunks exist (raw audio already
  deleted), the audio-export action is disabled with explanatory copy; a
  partial bundle is only produced after explicit confirmation listing the
  `gaps`.
- Integrity: bundle consumers verify per-member SHA-256 before playback or
  re-import; a hash mismatch marks the member unusable instead of guessing.

## 8. Project-unassignment flow (plan lines 1431–1434)

> Plan lines 1432–1434: _„Wird ein Meeting aus einem Projekt gelöst, wird es
> nicht gelöscht, sondern erscheint im Eingang. Verweise aus bereits
> bestätigten Aufgaben werden dabei als zu prüfende Belege markiert und
> nicht still entfernt."_

### 8.1 Schema addition (additive, no existing column repurposed)

`task_evidence` gains two columns (plan table lines 1295–1301 stays intact;
this extends, not redefines):

- `review_state TEXT NOT NULL DEFAULT 'confirmed' CHECK (review_state IN
('confirmed','needs_review'))`;
- `needs_review_reason TEXT NULL` (machine-readable cause, e.g.
  `'source_unassigned:<project_id>'`) and `needs_review_at` timestamp.

### 8.2 Transaction (single KnowledgeStore unit of work)

Trigger: user selects „Projektzuordnung lösen" for meeting M (linked source
S) currently joined to project P. A confirmation dialog lists the concrete
consequences (count of affected tasks/evidence), consistent with the
confirm-before-change discipline of plan line 1431.

Inside one transaction, in order:

| Step   | Table             | Transition                                                                                                                                                                                                                                          |
| ------ | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| U1     | `project_sources` | read join row `(P, S)`; absent ⇒ fail `not_assigned`, no-op                                                                                                                                                                                         |
| U2     | `task_evidence`   | for every row E where `E.source_id = S` **and** E's task belongs to P: set `review_state='needs_review'`, `needs_review_reason`, `needs_review_at=now`. Rows are **updated, never deleted**; `quote_excerpt`, `time_range`, locators stay untouched |
| U3     | `project_sources` | DELETE row `(P, S)` (join-row removal only; `sources` and `meeting_sessions` rows are untouched)                                                                                                                                                    |
| U4     | `change_log`      | append one entry for the join removal and one batch entry (entity `task_evidence`, affected count, revision bump)                                                                                                                                   |
| commit | —                 | on success emit event `project_source_unassigned { source_id, project_id, needs_review_count }`                                                                                                                                                     |

### 8.3 Resulting semantics

- **Inbox appearance** is derived, not stored: a `sources` row with no
  active `project_sources` join renders in the Eingang
  (`UnassignedInbox.tsx`, plan line 329). Re-assigning later simply inserts
  a fresh join row (`origin = 'manual'`) — the meeting itself was never
  deleted (plan line 1432).
- **„Zu prüfende Belege" surface:** affected tasks show a review badge with
  the count; expanding it lists the marked evidence rows with their reason.
  Resolution is manual only: confirm relevance (back to `'confirmed'`) or
  dismiss the evidence/task. **Re-assignment never auto-resets**
  `review_state` — clearing the marker is always a human decision, so no
  state flips silently in either direction.
- **Scope correctness:** because `task_evidence` reaches tasks through
  `project_tasks.project_id`, only tasks of the abandoned project P are
  marked. Identical evidence cited by another project's task stays
  `confirmed` while that project still holds the source.
- **Suggested (unconfirmed) tasks:** their evidence rows are marked too
  (they now point outside the project's scope), but suggestions receive no
  badge churn — they were never accepted knowledge. They remain dismissible
  through the normal suggestion flow.
- **Interaction with irreversible deletion:** the whole-meeting delete
  dialog (plan lines 1425–1429, 1431) MUST display the count of
  `needs_review` and `confirmed` evidence rows across all projects that
  reference S, so the user confirms the concrete blast radius before the
  destructive step (deletion itself is owned by the retention/deletion flow
  and out of scope here).
- **Snapshot evidence:** `snapshot_evidence` rows pointing at S are handled
  by the next incremental intelligence run (conflicts stay visible, plan
  lines 1397–1398); unassignment does not rewrite historical snapshots.

## 9. Non-goals (complexity budget)

Rejected for v1 to prevent scope bloat; revisit only with user evidence:

- additional export formats (PDF, SRT/VTT, DOCX), template customization,
  scheduled/bulk exports, direct cloud/upload targets;
- an exports bookkeeping table in SQLite (files self-describe via §5);
- a second position-reference format or per-chunk deep links;
- automatic evidence resolution on reassignment, automatic snapshot rewrites
  on unassignment, or silent evidence pruning anywhere.

## 10. Gate verification checklist

| Gate                                                                                                                                        | Where satisfied        |
| ------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| G1 formats: query source §4.1; serializer shapes §4.2; write rule + `portable::resolve_app_data` citation + no-absolute-path invariant §4.3 | §4                     |
| G2 unassignment: exact row/state transitions U1–U4, update-not-delete evidence marking, plan lines 1431–1434 quoted                         | §8                     |
| G3 deep link: single canonical `MeetingPositionRef` for tasks/search/player, four-outcome resolution incl. deleted audio per retention      | §3                     |
| G4 provenance: envelope fields tracing to meeting ID, segment ranges, generator versions, on every artifact incl. audio sidecar             | §4.2, §4.4, §5, §7     |
| G5 hygiene: prettier-clean, touches only this file                                                                                          | verification run below |
