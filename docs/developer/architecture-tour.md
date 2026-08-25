# Developer Architecture Tour — Meeting Mode

Status: **developer documentation tour** (this document describes other
documents and the verified repository state; it defines nothing normative and
changes no source code).
Author-time verification basis: **HEAD `61237b1554d3451a8cfb9e4ef3c858591e213ff8`**
(`git rev-parse HEAD` on a clean worktree, `git status --porcelain` empty).
Audience: a developer who is new to the Handy meeting-mode effort and needs a
reading path from the plan to the contracts to the future code locations.

Plan: [../meeting-mode-plan.md](../meeting-mode-plan.md)
Index: [../meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)

---

## 1. The state you are stepping into: no implemented behavior yet

**No meeting-mode production code exists yet** in any behaviorally meaningful
sense: every document under
[../meeting-mode-architecture/](../meeting-mode-architecture/) is planning or a
`spec contract` for future implementation. None of them has an implementation
behind it today, and this tour describes *future* code paths — not existing
ones. Do not expect to find `MeetingManager`, `MeetingRecorder`, the session
state machine, STT jobs, summaries, projects or imports anywhere in the app.

Two narrow, non-behavioral exceptions exist at HEAD `61237b1`, and honesty
requires naming them precisely so you do not mistake them for a start of the
pipeline:

1. **Compile-only repository trait scaffolds** (task-0134):
   [../../src-tauri/src/managers/meeting_store.rs](../../src-tauri/src/managers/meeting_store.rs)
   and
   [../../src-tauri/src/managers/knowledge_store.rs](../../src-tauri/src/managers/knowledge_store.rs)
   contain traits and entity types transcribed from
   [repository-boundary-contract.md](../meeting-mode-architecture/repository-boundary-contract.md).
   Their own headers state: *"no SQL, no concrete struct, no constructor and no
   manager wiring"*. Nothing calls them.
2. **A build-flag-gated UI placeholder** (task-0181 prototype):
   [../../src/components/meetings/MeetingList.tsx](../../src/components/meetings/MeetingList.tsx)
   renders an empty-state list only when
   `MEETING_MODE_ENABLED` is on (`VITE_ENABLE_MEETING_MODE === "true"` in
   [productNav.ts](../../src/components/shell/productNav.ts); default off in
   production). The `meetings` route otherwise shows a static empty-state
   placeholder in `src/App.tsx`. It implements no contract behavior.

Everything else that plan §5 names — `meeting_recorder.rs`,
`managers/meeting.rs`, `storage/asset_store.rs`, `importers/whatsapp.rs` and 20
of the 22 frontend placements F1–F22 — does not exist yet. Of the frontend
placements exactly two exist at author time: the flag-gated F10 prototype
above and the working dictation home
[../../src/components/dictation/DictationHome.tsx](../../src/components/dictation/DictationHome.tsx)
(F3), which serves the existing dictation product, not meeting mode (verified;
see the checklist in §7). The existing, fully working product remains the
**dictation path**, documented as implemented behavior only in the Phase-0
baseline and operator guides registered in the documentation index.

## 2. The authority order you must internalize first

Before reading any contract, memorize the conflict rule used across the whole
corpus ([documentation index §4 Kurzregel](../meeting-mode-documentation-index.md),
restated verbatim in every contract header):

> **The plan wins over contracts, contracts win over catalogs, and nothing
> wins over actual code.**

Concretely:

| Rank | Artifact | Role |
| --- | --- | --- |
| 1 | [../meeting-mode-plan.md](../meeting-mode-plan.md) | Sole normative source for goals, scope, architecture (§5), phases (§13) and acceptance (§14/§17). |
| 2 | `spec contract`s under [../meeting-mode-architecture/](../meeting-mode-architecture/) | Narrow, binding interface/data/state decisions for future implementation. They may refine but never move or contradict a plan decision. |
| 3 | Catalogs and proposals (e.g. [UX copy catalog](../meeting-mode-ux-copy-catalog.md)) | Non-binding drafts (copy texts, suggested identifiers) until adopted by a contract or the plan. |
| — | Actual code | The final arbiter once something is implemented. Only documents citing concrete code may claim *implemented behavior*. |

A worked example of this order in action is the `MeetingRecorder` home
adjudication in [module-placement-map.md §3](../meeting-mode-architecture/module-placement-map.md):
an external work-item ledger had named a different directory than plan §5;
because the plan outranks everything below it, the plan's path
(`src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`) won and no shim was
created. When you disagree with a contract, your recourse is the same: argue
from the plan, not from convenience.

## 3. Reading order: plan → placement map → contract graph

Read in this order. Each step says why it comes there and what you should be
able to answer afterwards.

### Step 1 — Plan §5 "Zielarchitektur" (plan lines ~185–337)

[../meeting-mode-plan.md §5](../meeting-mode-plan.md) gives you the mental
model in three layers:

```
React UI -> Tauri commands/events -> domain services -> repositories -> SQLite + AssetStore (v1)
```

and the component inventory: backend components B1–B11 (`MeetingManager`,
`MeetingRecorder`, `MeetingStore`, `MeetingTranscriptionQueue`,
`MeetingSummaryService`, `ProjectManager`, `KnowledgeStore`,
`SourceIngestionService`, `WhatsAppExportImporter`,
`ProjectIntelligenceService`, `AssetStore`) plus frontend components F1–F22.
Note the two structural invariants stated right in §5: the React layer never
reads SQLite or asset files directly, and the existing dictation `AudioRecorder`
is never overloaded with meeting concerns.

After this step you can answer: *what are the moving parts and which layer does
each live in?*

### Step 2 — Module placement map: where future code will land

[module-placement-map.md](../meeting-mode-architecture/module-placement-map.md)
assigns exactly one canonical path to every §5 component (tables §2 and §4),
fixes re-export visibility rules E1–E5/F1, and adjudicates the one recorded
path conflict (§3, see §2 above). Key placements to remember:

- Capture building block beside its reused parts:
  `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` (B2).
- Domain services/stores under the existing private `managers/` module (B1,
  B3–B8, B10); byte storage gets the new top-level `src-tauri/src/storage/`
  (B11); the WhatsApp importer the new top-level `src-tauri/src/importers/`
  (B9).
- Import direction is fixed: `commands/* → managers::* → {storage::*, audio_toolkit::*}`;
  `audio_toolkit` never depends on `managers` (rule E4).

After this step you can answer: *for any component name, which exact file will
implement it?*

### Step 3 — The contract graph

The six core contracts form a dependency graph. Read them roughly bottom-up in
this diagram (arrows mean "consumes/builds on"):

```
                       plan §5 (+ §6–§12, §14 anchors per contract)
                                      |
              module-placement-map.md  (canonical paths for all of the below)
                                      |
     +----------------+----------------+----------------+
     |                |                |                |
recorder-contract  stt-provider   (phase2 data      coexistence-
     (RC)           -contract      contracts)        contract (CC)
     |                |                |                |
     +--------+-------+                |        (builds on RC + STT;
              |                        |         seams S1–S5 to dictation)
    session-state-contract (SSC)       |
              |                        |
    repository-boundary-contract (RBC) |
              |                        |
    frontend-state-contract (FSC) -----+
```

1. **[recorder-contract.md](../meeting-mode-architecture/recorder-contract.md)** (RC)
   — the first contract to read. One concern: turn two live capture streams
   into crash-safe PCM16 WAV chunk files plus level/state/error events. The
   recorder owns *only* chunk writing; it stays storage-agnostic through a
   `ChunkStateStore` trait and is the single writer of `meetings/<id>/audio/`.
   Anchors: plan §5, §6.x, §7, §14. Future home: B2.
2. **[stt-provider-contract.md](../meeting-mode-architecture/stt-provider-contract.md)** (STT)
   — exactly one trait, `MeetingSttProvider`, behind Local, Azure Fast
   Transcription and Custom Multipart providers. Providers return segments
   *relative to the audio they were given*; only the queue maps them onto the
   meeting timeline. Callers outside the meeting transcription component never
   see provider details. Anchors: plan §8, §9. Future home: B4/B5 consumers.
3. **[coexistence-contract.md](../meeting-mode-architecture/coexistence-contract.md)** (CC)
   — fixes the seams *between* meeting mode and the untouched dictation path:
   S1 engine lease, S2 microphone tap with the single-device-open invariant,
   S3 mutual exclusion of simultaneous starts, S4 settings separation with
   snapshot isolation, S5 shortcut namespace. Structural law throughout:
   *capture never waits on inference*. Anchors: plan §6, §6.9, §6.10, §8, §13
   Phase 4/5.
4. **[session-state-contract.md](../meeting-mode-architecture/session-state-contract.md)** (SSC)
   — the internal seam of `MeetingManager`: it is the single authoritative
   owner of the session state machine and the only writer of
   `meeting_sessions.state`. Consumes RC, STT and the Phase-2 data contracts.
   Anchors: plan §5 (`MeetingManager`), §6.x states, §9 tables, §13 Phase 4.
   Future home: B1.
5. **[repository-boundary-contract.md](../meeting-mode-architecture/repository-boundary-contract.md)** (RBC)
   — who owns which entity, through which interface, with which transactional
   guarantees. `MeetingStore` and `KnowledgeStore` are the only SQL seams; no
   frontend code ever touches SQLite or asset bytes (plan line 1556). Its
   ownership matrix is the pivot for the whole data plane. Anchors: plan §5,
   §9, §13 Phase 3.
6. **[frontend-state-contract.md](../meeting-mode-architecture/frontend-state-contract.md)** (FSC)
   — the React-side mirror: three domain Zustand stores whose ownership matrix
   mirrors RBC 1:1, exclusive `listen()` ownership per event topic, change-envelope
   consumption, optimistic-update whitelist. Builds on SSC's event guarantees.
   Anchors: plan §5 layering, §6 UX principles, §13 Phase 3/4.

Beyond these six, the documentation index §3 table registers the remaining
contracts (asset store, disk guard, scheduler, summary/evidence, search index,
snapshots, task proposals, streaming, error taxonomy, navigation, rendering,
schema versioning, device-change delta, settings policy, diagnostics, i18n,
formatting, ingestion/importers) plus the dataflows. You do not need them for
the core mental model; open them when you enter their phase (see Step 4).

### Step 4 — Enter by phase, not by curiosity

Work lands phase by phase (plan §13: Phase 0 done, Phase 1 spike evidence
committed, Phases 2–11 pending). Each phase has a designated contract entry
point; read that contract before touching its future canonical paths:

| Phase | Entry contracts | First files that will appear |
| --- | --- | --- |
| 2 — dual-source recorder | RC, [phase2 data contracts](../meeting-mode-phase2-contracts.md) | B2 `audio_toolkit/audio/meeting_recorder.rs`; chunk/manifest layout |
| 3 — storage foundation & app shell | RBC, FSC, placement map | B3/B7/B11 store seams; three-area app-shell groundwork (F1/F2) |
| 4 — session & recording UI | SSC, CC | B1 `managers/meeting.rs`; F5–F7, F10, F11 |
| 5 — local meeting STT | STT, CC, [scheduler](../meeting-mode-architecture/job-scheduler-contract.md) | B4 queue; provider implementations |
| 7+ — summary/library/export | [summary provider/evidence/pipeline](../meeting-mode-architecture/summary-provider-contract.md), export dataflow | B5; library/export surfaces |
| 9 — WhatsApp import | [ingestion pipeline](../meeting-mode-architecture/ingestion-pipeline-contract.md), [WhatsApp importer](../meeting-mode-architecture/whatsapp-importer-contract.md) | B8/B9; F19 wizard |

## 4. From contracts to future code paths — how the handoff will work

When implementation starts, the expected mechanics are:

1. **Path**: each landed file appears at its canonical placement-map path —
   never a second location (rule E5 forbids shims and aliases).
2. **Authority check**: a reviewer resolves disputes by the §2 order — plan
   text beats contract text; both beat catalogs/proposals; actual code beats
   everything once it exists.
3. **Status flip**: a document may move from `spec contract` to describing
   *implemented behavior* only with concrete code citations
   ([documentation index §1 rule](../meeting-mode-documentation-index.md)).
   Until then, treat every behavioral sentence you read as a promise, not a fact.
4. **Dictation isolation**: the existing dictation path stays untouched by
   construction — Phase-2 acceptance explicitly requires the dictation
   microphone path unchanged, and CC arbitrates every shared resource instead
   of modifying dictation internals.

## 5. What NOT to do (common newcomer mistakes)

- Do not implement against a catalog value (UX copy keys, proposed event topic
  names) before a contract adopts it — catalogs rank below contracts.
- Do not create `src-tauri/src/meeting_recorder.rs` as a top-level module; the
  placement-map §3 ruling fixed the home inside `audio_toolkit/audio/`.
- Do not let frontend code call SQLite or read asset files; route through the
  RBC store seams and Tauri commands.
- Do not add a second recording state in the frontend; FSC gives every entity
  projection exactly one owning store.
- Do not describe existing behavior without a code citation; the index treats
  that as a documentation defect.

## 6. Where to check your understanding

- Status and registry of every document: [documentation index §3 table](../meeting-mode-documentation-index.md).
- Reading-order canon for all contracts (superset of this tour): index §4.
- Verified environment quirks during verification runs:
  [verification environment](../meeting-mode-verification-environment.md).
- Known, evidenced limitations register:
  [known limitations](../meeting-mode-known-limitations.md).

## 7. Appendix: verify the "no implementation yet" claim yourself

Commands used at author time (run them yourself; expect the results shown):

```powershell
# Canonical meeting-mode homes must not exist yet:
Test-Path src-tauri/src/audio_toolkit/audio/meeting_recorder.rs   # False
Test-Path src-tauri/src/managers/meeting.rs                       # False
Test-Path src-tauri/src/storage                                   # False
Test-Path src-tauri/src/importers                                 # False
Test-Path src/components/projects                                 # False
Test-Path src/components/import                                   # False

# The only meeting-related Rust files are compile-only trait scaffolds:
Get-ChildItem src-tauri/src -Recurse -Filter *.rs | Where-Object Name -match 'meeting'
#   -> managers/meeting_store.rs, managers/knowledge_store.rs (headers: "no SQL,
#      no concrete struct, no constructor and no manager wiring")

# The only meeting UI behind a default-off build flag:
Select-String src/components/shell/productNav.ts 'VITE_ENABLE_MEETING_MODE'

# Of the 22 plan-§5 frontend placements (F1–F22), only these two exist:
Test-Path src/components/dictation/DictationHome.tsx   # True  (F3, dictation product)
Test-Path src/components/meetings/MeetingList.tsx      # True  (F10, flag-gated prototype)
Test-Path src/components/shell/AppShell.tsx            # False
Test-Path src/components/projects/ProjectsHome.tsx     # False
```

If any of these checks starts returning different results, meeting-mode
implementation has begun: re-read this tour's §4, then trust code citations
over this document.
