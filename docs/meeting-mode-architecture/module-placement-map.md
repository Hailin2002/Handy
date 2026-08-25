# Canonical Module Placement Map — Meeting Mode Plan §5

Status: **spec contract — no implementation yet** (Phase 2/3 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 ("Zielarchitektur", backend
component sections at plan lines 219–308, frontend component list at plan
lines 310–333).
Purpose: this document assigns **every** plan-§5 component to exactly one
canonical crate-module path and fixes re-export visibility rules, reconciled
against the existing tree (`src-tauri/src/managers/`, `src-tauri/src/audio_toolkit/`,
`src-tauri/src/commands/`, `src/components/`) and against all committed
meeting-mode contracts. It changes no source code.

Conflict rule (identical to `docs/meeting-mode-documentation-index.md` §4 and
`repository-boundary-contract.md` header): **the plan wins over contracts,
contracts win over catalogs, and nothing wins over actual code.** This map is a
contract-level document; where it restates plan §5 homes it adds precision, not
new authority.

---

## 1. Authority order applied to module placement

1. **Plan §5 "Vorgesehener Ort" lines are the primary authority** for every
   component home. They were written against the verified repository state of
   plan §4 and are internally consistent with the committed contracts.
2. **Committed contracts may narrow but never move a plan-§5 home.**
   `recorder-contract.md` (line 9), `asset-store-contract.md` (line 8) and
   `repository-boundary-contract.md` (lines 12–17) each name the same homes as
   the plan; no contract proposes an alternative path for any §5 component.
3. **External task ledgers are below contracts and the plan.** Where a mission
   ledger entry (task-0011) names a deviating file scope for `MeetingRecorder`,
   it is adjudicated in §3 below — it cannot override plan §5.
4. **The existing codebase is the reconciliation baseline**, not an authority:
   new components must follow the *conventions* of existing modules (manager
   pattern under `managers/`, selective re-export in
   `audio_toolkit/audio/mod.rs`) without modifying existing dictation-path
   files (plan §13 Phase 2 acceptance: "der bestehende Mikrofon-Diktierpfad
   wurde nicht verändert").

## 2. Backend placements

Legend: **Status** = does the parent module exist at HEAD `63ac2dc`
(`existing` = directory already present; **planned-new** = module/file must be
created). Every canonical path is relative to the repository root.

| # | Component (plan §5 heading) | Canonical path | Status | Primary consumers | Contract anchor |
| --- | --- | --- | --- | --- | --- |
| B1 | `MeetingManager` | `src-tauri/src/managers/meeting.rs` | planned-new (under existing `managers/`) | `commands/` handlers, `lib.rs` setup, shortcut handler | session-state-contract.md line 11; recorder-contract §1 table |
| B2 | `MeetingRecorder` | `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` | planned-new (under existing `audio_toolkit/audio/`) | `MeetingManager` only | recorder-contract.md line 9; adjudication in §3 |
| B3 | `MeetingStore` | `src-tauri/src/managers/meeting_store.rs` | planned-new (under existing `managers/`) | `MeetingManager`, `MeetingTranscriptionQueue`, `MeetingSummaryService` | repository-boundary-contract lines 14, 104 |
| B4 | `MeetingTranscriptionQueue` | `src-tauri/src/managers/meeting_transcription.rs` | planned-new (under existing `managers/`) | `MeetingManager`; consumes chunks in state `ready` | startup-recovery-dataflow.md line 46 |
| B5 | `MeetingSummaryService` | `src-tauri/src/managers/meeting_summary.rs` | planned-new (under existing `managers/`) | `MeetingManager`; reads via `MeetingRepository` | summary-evidence-contract.md line 14 |
| B6 | `ProjectManager` | `src-tauri/src/managers/project.rs` | planned-new (under existing `managers/`) | `commands/` handlers | repository-boundary-contract §6 |
| B7 | `KnowledgeStore` | `src-tauri/src/managers/knowledge_store.rs` | planned-new (under existing `managers/`) | `ProjectManager`, `SourceIngestionService`, `ProjectIntelligenceService` | repository-boundary-contract lines 15, 105 |
| B8 | `SourceIngestionService` | `src-tauri/src/managers/source_ingestion.rs` | planned-new (under existing `managers/`) | import wizard UI via commands | repository-boundary-contract §6 (K4 commit unit) |
| B9 | `WhatsAppExportImporter` | `src-tauri/src/importers/whatsapp.rs` | planned-new (**new top-level module `importers/`**) | `SourceIngestionService` only | plan lines 289–291; SQL-free per §5 rule E4 |
| B10 | `ProjectIntelligenceService` | `src-tauri/src/managers/project_intelligence.rs` | planned-new (under existing `managers/`) | project detail UI via commands | summary-evidence-contract.md line 15 |
| B11 | `AssetStore` | `src-tauri/src/storage/asset_store.rs` | planned-new (**new top-level module `storage/`**) | `MeetingStore`/`KnowledgeStore` call it; never the frontend | asset-store-contract.md line 8 |

### 2.1 Supporting modules already anchored by committed contracts

These are not plan-§5 headings but have binding homes from committed
documents; they are restated here so the map is complete and nothing needs to
be re-decided:

| Supporting item | Canonical path | Anchor |
| --- | --- | --- |
| Change-log append helper | `src-tauri/src/managers/change_log.rs` | repository-boundary-contract §2 (last sketch block), §3 row 16, R1 allow-list item 3 |
| Legacy dictation history (untouched) | `src-tauri/src/managers/history.rs` | repository-boundary-contract §0.1 item 3, R1 item 4 (carve-out) |
| Meeting SQLite migrations module | designated by task-0020 (`db-schema.md`), not by this map | repository-boundary-contract §0.1 item 1, R1 item 5 |

### 2.2 Plan-referenced helpers without a §5 home (non-normative recommendation)

Plan §8 names an `AzureMeetingClient` and plan §9 names a `SearchService`
interface without assigning either a home. To prevent uncoordinated invention:
the recommended placement is a submodule of its owning queue/store —
`src-tauri/src/managers/meeting_transcription/azure.rs` (converting B4 into a
directory module is then part of that phase's change) and the search seam on
`KnowledgeStore` itself (`knowledge_store::search`, already sketched in
repository-boundary-contract §2). These recommendations are subordinate to any
future contract that covers those phases explicitly.

## 3. Adjudication: `MeetingRecorder` path conflict (G2)

### 3.1 The conflicting claims

| Claimant | Path | Nature |
| --- | --- | --- |
| Plan §5, `MeetingRecorder` section (plan lines 229–236): "Vorgesehener Ort: `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`" | `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` | normative plan text |
| Mission ledger task-0011 scope | `src-tauri/src/meeting_recorder` | external work-item scoping, not committed repo documentation (no occurrence of `src-tauri/src/meeting_recorder` anywhere under `docs/` at HEAD `63ac2dc`; verified by grep) |

### 3.2 Ruling

**Canonical path: `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`.**
The ledger task-0011 path `src-tauri/src/meeting_recorder` is rejected as a
module location.

Rationale, in authority order (§1):

1. **Plan beats everything below it.** The documentation index's Kurzregel
   ("Plan schlägt Verträge, Verträge schlagen Kataloge") places the plan above
   every other artifact; a ledger entry is execution bookkeeping, strictly
   weaker than both the plan and the contracts. A ledger scope can *describe*
   work; it cannot *re-home* a plan-§5 component.
2. **The plan's choice is load-bearing inside its own architecture.** Plan §5
   justifies the home functionally: the recorder is a low-level capture
   building block that "wiederverwendet Geräteauflistung,
   Sample-Konvertierung, `FrameResampler` und VAD-Bausteine" — all of which
   live in `audio_toolkit` (`audio/device.rs`, `audio/resampler.rs`,
   `vad/`). Placing the recorder beside its reused parts keeps the reuse
   obligations mechanically visible and avoids a new top-level module whose
   only content would be imports from `audio_toolkit`.
3. **All committed contracts already assume the plan path.** Four independent
   committed documents reference it: `recorder-contract.md` line 9,
   `meeting-mode-phase2-contracts.md` line 22,
   `startup-recovery-dataflow.md` line 44, and
   `coexistence-contract.md` line 588. Adopting the ledger path would force a
   contradicting edit to four committed contracts — exactly what G3 forbids.
4. **Dictation-path isolation.** The plan's Phase-2 acceptance requires the
   dictation recorder (`audio_toolkit/audio/recorder.rs`) to remain untouched;
   a sibling file inside the same module makes shared-type reuse explicit
   while keeping the two recorders separable in review, matching the plan's
   §4 conclusion that meeting capture is "ein eigener Lebenszyklus".

Consequences for the ledger:

- Task-0011's declared write scope `src-tauri/src/meeting_recorder` is
  interpreted as covering the canonical file
  `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` (scope alias); the
  supervisor owns updating the ledger wording.
- **No compatibility shim is created**: there must be no
  `src-tauri/src/meeting_recorder.rs` or `src-tauri/src/meeting_recorder/mod.rs`
  re-export facade, because §5 rule E5 forbids second paths for one component
  and a dead shim module is scope creep (complexity budget).

## 4. Frontend placements (plan lines 312–333)

All frontend paths are taken verbatim from the plan's component list; none of
the directories `shell/`, `dictation/`, `meetings/`, `projects/`, `import/`
exists under `src/components/` at HEAD `63ac2dc` (verified: only `footer`,
`icons`, `model-selector`, `onboarding`, `settings`, `shared`, `ui`,
`update-checker`, `whats-new`). `settings/` exists and gains two subpaths.

| # | Component | Canonical path | Status |
| --- | --- | --- | --- |
| F1 | App shell frame | `src/components/shell/AppShell.tsx` | planned-new |
| F2 | Product navigation | `src/components/shell/PrimaryNavigation.tsx` | planned-new |
| F3 | Dictation home | `src/components/dictation/DictationHome.tsx` | planned-new |
| F4 | Dictation status card | `src/components/dictation/DictationStatusCard.tsx` | planned-new |
| F5 | Meeting setup dialog | `src/components/meetings/MeetingSetup.tsx` | planned-new |
| F6 | Meeting preflight | `src/components/meetings/MeetingPreflight.tsx` | planned-new |
| F7 | Live recording view | `src/components/meetings/MeetingRecorderView.tsx` | planned-new |
| F8 | Post-stop processing view | `src/components/meetings/MeetingProcessingView.tsx` | planned-new |
| F9 | Meeting detail view | `src/components/meetings/MeetingDetail.tsx` | planned-new |
| F10 | Meeting list | `src/components/meetings/MeetingList.tsx` | planned-new |
| F11 | Recovery banner | `src/components/meetings/MeetingRecoveryBanner.tsx` | planned-new (startup-recovery-dataflow.md line 47) |
| F12 | Projects home | `src/components/projects/ProjectsHome.tsx` | planned-new |
| F13 | Project create dialog | `src/components/projects/ProjectCreateDialog.tsx` | planned-new |
| F14 | Project detail shell | `src/components/projects/ProjectDetail.tsx` | planned-new |
| F15 | Project overview tab | `src/components/projects/ProjectOverview.tsx` | planned-new |
| F16 | Project tasks tab | `src/components/projects/ProjectTasks.tsx` | planned-new |
| F17 | Project sources tab | `src/components/projects/ProjectSources.tsx` | planned-new |
| F18 | Unassigned inbox | `src/components/projects/UnassignedInbox.tsx` | planned-new |
| F19 | WhatsApp import wizard | `src/components/import/WhatsAppImportWizard.tsx` | planned-new |
| F20 | Shortcut recorder | `src/components/settings/ShortcutRecorder.tsx` | planned-new (under existing `settings/`) |
| F21 | Dictation settings | `src/components/settings/dictation/DictationSettings.tsx` | planned-new (under existing `settings/`) |
| F22 | Meeting settings | `src/components/settings/meeting/MeetingSettings.tsx` | planned-new (under existing `settings/`) |

## 5. Re-export visibility rules

Backend conventions observed at HEAD `63ac2dc`: `lib.rs` declares
`mod managers;` privately (line 16) while `managers/mod.rs` lists its children
as `pub mod …`; `audio_toolkit` is `pub mod audio_toolkit` with private child
modules and **selective** `pub use` in `audio/mod.rs` (lines 8–13). The rules
below extend these conventions; they do not modify any existing line of code.

**E1 — Domain services and stores are crate-internal.** All B1–B8, B10 items
live under the private `managers` module; their effective visibility stops at
the crate boundary regardless of inner `pub mod` declarations. The frontend
reaches them exclusively through Tauri command handlers under
`src-tauri/src/commands/` (repository-boundary-contract R3/R5; plan lines
216–217). None of them may be re-exported through `lib.rs`.

**E2 — `MeetingRecorder` follows the `audio_toolkit/audio/mod.rs` pattern.**
`mod meeting_recorder;` stays private in
`audio_toolkit/audio/mod.rs`; the public surface (at minimum the types named in
recorder-contract §2: `MeetingRecorder`, `SourceKind`, `TimelineRange`,
`TimelineMarker`, `RecorderEvent`, `RecorderState`, `RecorderError`) is
selectively re-exported there alongside the existing `pub use` lines. No
wildcard `pub use meeting_recorder::*`.

**E3 — New top-level modules mirror `managers`.** `lib.rs` declares
`mod storage;` and `mod importers;` privately; `storage/mod.rs` selectively
re-exports the `AssetStore` trait and its request/result types (per
asset-store-contract §5.1); `importers/mod.rs` re-exports the importer trait /
`WhatsAppExportImporter` type consumed by `SourceIngestionService`. Neither
module may contain SQL (repository-boundary-contract R1 allow-list is
exhaustive) and neither may be re-exported through `lib.rs` beyond what
command wiring needs.

**E4 — Import direction is fixed by layer.** `commands/* → managers::* →
{storage::*, audio_toolkit::*}` and `managers/source_ingestion.rs →
importers::*`. No reverse edge exists; in particular `audio_toolkit` must never
depend on `managers` (recorder-contract §1 keeps the recorder
storage-agnostic via the `ChunkStateStore` trait owned by the caller side).

**E5 — Exactly one canonical path per component; no shims.** For every B/F
row there is exactly one defining file. Re-exports are permitted only as the
selective `pub use` statements of E2/E3 in the component's own parent
`mod.rs`. Any additional alias module, `pub use` chain, or deprecated shim
(e.g. a `src-tauri/src/meeting_recorder.rs` stub) is a defect against this map.

**F1 — Frontend canonical paths are the import sites.** Components import each
other by relative path or the `@/` alias pointing at the exact canonical file;
optional per-directory barrel files (`index.ts`, as used by existing
`components/settings/index.ts`) may re-export, but must not define components
themselves and must not be the only route to a component (direct canonical
imports stay valid).

## 6. Cross-check against committed contract scope sections (G3)

| Committed contract | Scope statement checked | Placement-map verdict |
| --- | --- | --- |
| `recorder-contract.md` §1 + line 9 | Recorder home `audio_toolkit/audio/meeting_recorder.rs`; recorder owns only chunk writing; session state machine = `MeetingManager` (B1 home `managers/meeting.rs`, also session-state-contract line 11); persistence = `MeetingStore` (B3) | ✅ identical; B2 ruling in §3 preserves it |
| `recorder-contract.md` reuse obligations (lines 12–17) | Reuses `audio_toolkit::audio::device` enumeration and `FrameResampler`; `AudioRecorder` unmodified | ✅ E2/E4 keep reuse intra-module and add no edits to `recorder.rs` |
| `asset-store-contract.md` §1–§2 + line 8 | Home `storage/asset_store.rs`; single resolution point via `portable.rs`; frontend never touches files; recorder is sole writer of `audio/` during live capture | ✅ B11 matches; E3 confines byte access to storage; E4 keeps stores calling AssetStore, not vice versa |
| `asset-store-contract.md` §7–§9 | Recovery scan owned by `MeetingManager` executed through AssetStore + MeetingStore; retention sweep in storage layer | ✅ B1/B3/B11 triangle unchanged by this map |
| `repository-boundary-contract.md` lines 12–17, §1 | Homes for `MeetingStore`, `KnowledgeStore`, `AssetStore` verbatim; both stores "live under `managers/` next to the services" | ✅ B3/B7/B11 verbatim; B5/B6/B8/B10 sit beside them as that contract's §6 consumer table already states |
| `repository-boundary-contract.md` §7.1 R1 (SQL allow-list) | Only `meeting_store.rs`, `knowledge_store.rs`, `change_log.rs`, legacy `history.rs`, task-0020 migration module may touch SQL | ✅ this map adds no SQL-capable file; `importers/whatsapp.rs` (B9) commits exclusively through `KnowledgeRepository::commit_source` |
| `startup-recovery-dataflow.md` lines 43–47 | Homes for `MeetingManager`, `MeetingRecorder`, `MeetingStore`, `MeetingTranscriptionQueue`, `MeetingRecoveryBanner.tsx` | ✅ all match B1, B2, B3, B4, F11 |
| `coexistence-contract.md` line 588 | "`MicTapHandle` exposure lives in `meeting_recorder.rs` (planned new, plan §5); `MeetingManager` consumes it" | ✅ consistent with B1/B2 and §3 ruling |
| `summary-evidence-contract.md` lines 14–15 | Homes `managers/meeting_summary.rs`, `managers/project_intelligence.rs` | ✅ B5/B10 verbatim |

No path in this map contradicts any committed contract's module boundaries.

## 7. Non-goals of this document

- It does not create any directory or file listed above; all rows are
  planned-new until their implementing phase lands (Phase 2+: B2; Phase 3:
  B3, B7, B11; Phase 4+: B1, B4–B6, F1–F22; Phase 7+: B5; Phase 8+: B10;
  Phase 9: B8, B9).
- It does not assign database schema ownership (task-0020 / `db-schema.md`)
  or migration-module placement beyond restating the repository-boundary
  carve-out.
- It does not adjudicate identifier-level naming findings (owned by
  `identifier-registry.md` and its defining documents).
- It introduces no new component, abstraction, or compatibility layer beyond
  what plan §5 and the committed contracts already require (complexity
  budget: zero net additions).

## Appendix — Gate traceability

- **G1** (one canonical path + re-export rules per §5 component): tables §2
  (B1–B11), §4 (F1–F22); rules E1–E5, F1.
- **G2** (recorder conflict adjudicated with plan-authority rationale): §3.
- **G3** (no contradiction with recorder/asset-store/repository-boundary
  scope sections): §6 matrix.
- **G4** (index table row): added to `docs/meeting-mode-documentation-index.md`
  §3 in the same change as this file.
