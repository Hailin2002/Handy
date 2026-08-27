# Frontend State Contract — Zustand Store Boundaries, Listener Ownership & Change-Envelope Consumption

Status: **spec contract — no implementation yet** (Phase 3/4 prerequisite;
changes no source code)
Source of truth: `docs/meeting-mode-plan.md` — §5 layering ("React-Oberfläche
-> Tauri-Commands und Events -> Domain-Services …", plan lines 205–217),
frontend component list (plan lines 310–333), §6 UX principles ("Aufnahmezustand
ist überall konsistent", plan lines 394–397; "Es gibt keinen zweiten lokalen
Recording-State im Frontend", plan lines 924–927), §9 change-envelope fields
(plan lines 1149–1159), Phase 3 acceptance (plan line 1556: "kein Frontendcode
greift direkt auf SQLite oder Assetdateien zu") and Phase 4 acceptance (plan
line 1577: "UI und persistierter Zustand stimmen nach Neustart überein").

This document fixes the **React-side seams**: which Zustand stores exist, which
store owns which backend entity's frontend projection, who may call
`listen()`, how repository change envelopes invalidate store state, when
optimistic updates are permitted, and why components never invoke Tauri
commands directly. It defines no backend behavior and no schema.

Sibling contracts consumed by this document (read-only):

- [repository-boundary-contract.md](repository-boundary-contract.md) —
  abbreviated **RBC-§n**. Its §3 entity-ownership matrix is mirrored 1:1 by
  §2.2 below; its §5 envelope semantics are what §4 consumes; its §7.1 rule
  R5 (frontend isolation) is restated and extended by §5.
- [session-state-contract.md](session-state-contract.md) — abbreviated
  **SSC-§n**. Its event-publication guarantees O1–O7 and UI rebuild rule (§5)
  are the foundation of §3 and §4 below.
- [module-placement-map.md](module-placement-map.md) — frontend component
  placements F1–F22; store homes below follow its canonical-path discipline.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 0. Scope, non-goals

### 0.1 In scope

1. The set of domain Zustand stores and their exclusive ownership slices (§2).
2. Exclusive `listen()` ownership per event topic with a full unlisten
   lifecycle (§3).
3. The consumption contract turning repository change envelopes into store
   invalidations (§4).
4. The command-invocation boundary between components and stores (§5).
5. The whitelist of optimistic operations with mandatory rollback (§6).

### 0.2 Explicitly out of scope

1. **Any Tauri command or event payload definition** — owned by the backend
   contracts (SSC, RBC, job-scheduler-contract) and their implementing
   phases. This contract only names the topics stores subscribe to.
2. **Visual/interaction design** of F1–F22 — owned by the UX workstream;
   only the state seams those components consume are fixed here.
3. **The identifier registry.** New event-topic names proposed in §3.2 are
   proposals until registered in `identifier-registry.md`.
4. **Component-test infrastructure.** The repo currently ships Playwright
   only (`package.json`, script `test:playwright`; no vitest/React Testing
   Library in `devDependencies` — verified 22.08.2026). The implementing
   task must add vitest + `@testing-library/react`; §3.4 specifies each rule
   so it becomes a mechanical test once that harness exists.

---

## 1. Position in the architecture and verified baseline

Plan lines 205–217 fix the top layer as pure consumers:

```text
React-Oberfläche          <-- THIS CONTRACT (stores, listeners, projections)
  -> Tauri-Commands und Events
    -> Domain-Services …
      -> Repository-Schnittstellen (RBC)
        -> SQLite + lokaler AssetStore (v1)
```

The webview holds **projections**, never authority. Every domain fact lives in
SQLite behind RBC-§2 stores; the frontend rebuilds views from snapshot
commands plus events exactly as SSC-§5 prescribes ("the UI is a pure function
of persistence plus events"). There is no second authoritative recording
state in the frontend (plan line 926) and there will be none for projects,
tasks, sources, or imports.

### 1.1 Baseline observations at authoring time (verified code, HEAD)

These findings motivate the rules of §3; they are recorded as evidence, not
as defects to fix in this task (this contract changes no source code):

| Observation | Location | Consequence |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Correct unlisten lifecycle exists: `const unlisten = listen(…)` inside `useEffect` with `return () => { unlisten.then(fn => fn()) }` | `src/App.tsx` lines 104–128, 134–143, 147–156, 159–175 | This is the pattern §3 mandates — but scoped per component mount, which §3 restricts further. |
| Fire-and-forget store listeners: `initialize()` calls `listen("model-state-changed", …)` and `listen("settings-changed", …)` discarding the promise and the `UnlistenFn`; no dedup guard | `src/stores/settingsStore.ts` lines 674–683 | Calling `initialize()` twice duplicates delivery of both topics forever; nothing can ever unsubscribe (leak class LK-1/DUP-1 in §3.4). |
| Guarded but undisposable store listeners: `modelStore.initialize()` registers 12 topics behind an `initialized` flag, no unlisten retained | `src/stores/modelStore.ts` lines 268–428 | Duplicate delivery is prevented, but teardown for tests/HMR/window close is impossible (leak class LK-1). |
| Component-level `listen()` for transient toasts and hardware events | `App.tsx` (above), `SecureInputWarning.tsx` line 36, `RecordingOverlay.tsx` lines 57–118, `UpdateChecker.tsx` line 60, `LiveLogViewer.tsx` line 103, `ModelSelector.tsx` lines 73+, `HandyKeysShortcutInput.tsx` line 140, `HistorySettings.tsx` line 135, `useAzureSpeechStream.ts` lines 372–417 | Each remount resubscribes; correctness depends on every effect cleaning up. §3 turns this convention into a registry with exactly one owner per topic. |
| Version-guarded optimistic update with per-key serialization already exists for settings | `src/stores/settingsStore.ts` lines 85–108 (`enqueueSettingUpdate`, `settingUpdateVersions`) and `updateSetting` lines 342–385 | This is the rollback template §6 adopts; it correctly prevents an older failed request from rolling back a newer value. |
| No direct SQL channel in the webview | grep `plugin-sql                                                                                                                                                                                                                                                                                   | invoke\(`under`src/`→ no matches; all backend calls go through generated`commands`from`src/bindings.ts` | Confirms RBC-§7.1 R5 at HEAD; §5 keeps it true mechanically for meeting-mode UI. Note: `@tauri-apps/plugin-sql` is present in `package.json` dependencies (line 33) — meeting-mode code must never import it (rule C3, §5). |

---

## 2. Zustand store boundaries (gate G1)

### 2.1 Store inventory

Exactly three domain stores join the two existing infrastructure stores. All
live flat under `src/stores/` beside them (no subdirectory split — the
existing convention, verified: `src/stores/{settingsStore,modelStore}.ts`).

| #   | Store (canonical path)         | Domain                   | Status                                | Owns                                                                                                                                              |
| --- | ------------------------------ | ------------------------ | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| S1  | `src/stores/meetingStore.ts`   | Meetings                 | planned-new (Phase 4)                 | Frontend projections of all `MeetingStore` entities (RBC rows 1–5); live session view-model; listener registrar for `meeting://*`                 |
| S2  | `src/stores/projectStore.ts`   | Projects & knowledge     | planned-new (Phase 8/9)               | Frontend projections of all `KnowledgeStore` entities (RBC rows 6–15); search-result cache; listener registrar for `project://*` and `import://*` |
| S3  | `src/stores/dictationStore.ts` | Dictation                | planned-new (Phase 3 shell work)      | Dictation-history projection (legacy `HistoryManager` domain, RBC note N4) and dictation runtime status card model (plan §6.4)                    |
| S4  | `src/stores/settingsStore.ts`  | Settings (cross-cutting) | existing — unchanged by this contract | App settings mirror incl. meeting settings keys (coexistence-contract §6.1); devices; post-process options                                        |
| S5  | `src/stores/modelStore.ts`     | Models (cross-cutting)   | existing — unchanged by this contract | Model catalogue/download progress                                                                                                                 |

Boundary rules:

- **B1 — one domain, one store.** A component never imports two domain stores
  for the same entity kind. Cross-domain composition happens in selectors or
  container components (F1/F2), not by copying state between stores.
- **B2 — no entity appears in two stores.** The matrix in §2.2 is exhaustive;
  adding a mirror elsewhere is a defect. Derived display values (durations,
  sorted/filter lists) are computed by selectors from the owning store.
- **B3 — stores hold projections keyed by stable identity.** Keys are the
  UUIDs of RBC-§5.1 — never rowids, never absolute paths (plan lines
  1151, 1158–1159). Asset references render via the app's asset/serve
  mechanism (RBC-§7.1 R4), never via filesystem paths.
- **B4 — stores are not persisted.** SSC-§5 forbids persisted UI-session
  state; consequently no domain store uses `zustand/persist` or writes itself
  to `tauri-plugin-store`. After restart everything rebuilds from snapshot
  commands (plan line 1577 acceptance holds by construction).
- **B5 — high-frequency telemetry stays out of React-subscribed state.**
  Audio level bars (plan §6.11) and the running duration clock are rendered
  from refs/rAF or a dedicated throttled hook, never written into a Zustand
  store per audio frame or per tick; only coarse state transitions
  (recording/paused/backlog-count) enter `meetingStore`.
- **B6 — actions are the only mutators.** State changes flow exclusively
  through store actions (which wrap commands or apply listener payloads);
  components cannot `setState` their way around the contract. Internal setters
  stay unexported from the public hook surface where possible.

### 2.2 Store-ownership matrix — 1:1 mirror of RBC-§3 (gate G1)

Every row cites its backend twin. "Frontend owner" names the only store that
may hold state derived from that entity. Rows 16–17 have **no** frontend
mirror by design.

| RBC row | Backend entity (owner)                                     | Frontend owner (slice)                                                                     | Projection form                                                                                                                       | Change channel (§4)                                                         |
| ------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| 1       | `meeting_sessions` (MeetingStore)                          | S1 `meetingStore.sessions`                                                                 | `Map<Uuid, MeetingSessionVm>` incl. state, revisions, provider label                                                                  | `meeting://session-*` envelopes + `get_meeting_snapshot`                    |
| 2       | `meeting_audio_chunks` (MeetingStore)                      | S1 `meetingStore.chunkStats[meetingId]`                                                    | Aggregate counts by state + last-finalized marker; individual chunks only where the Processing tab needs them (F-view "Verarbeitung") | aggregated tier-A events; snapshot on open                                  |
| 3       | `meeting_stt_jobs` (MeetingStore, tier B)                  | S1 `meetingStore.jobBacklog[meetingId]`                                                    | Counts `{queued,running,retryWait,failed}` + oldest-failed summary — never per-job churn                                              | coalesced `meeting://jobs-updated` (tier B, no envelope)                    |
| 4       | `meeting_transcript_segments` (MeetingStore)               | S1 `meetingStore.transcript[meetingId]`                                                    | Paged chronological segments (windowed, scroll-anchored per plan §6.11)                                                               | segment-append events + paged refetch                                       |
| 5       | `meeting_summaries` (MeetingStore)                         | S1 `meetingStore.summary[meetingId]`                                                       | Current summary VM incl. structured decisions/tasks/questions                                                                         | `meeting://summary-available` + snapshot                                    |
| 6       | `projects` (KnowledgeStore)                                | S2 `projectStore.projects`                                                                 | `Map<Uuid, ProjectVm>` incl. archived/deleted flags                                                                                   | `project://entity-changed` envelopes                                        |
| 7       | `sources` (KnowledgeStore)                                 | S2 `projectStore.sources`                                                                  | `Map<Uuid, SourceVm>`                                                                                                                 | same                                                                        |
| 8       | `project_sources` (KnowledgeStore)                         | S2 `projectStore.links`                                                                    | Link records keyed `${projectId}:${sourceId}`                                                                                         | same                                                                        |
| 9       | `source_items` (KnowledgeStore)                            | S2 `projectStore.items[sourceId]`                                                          | Paged normalized items per opened source                                                                                              | same + paged refetch                                                        |
| 10      | `assets` metadata (KnowledgeStore)                         | S2 `projectStore.assetMeta`                                                                | `Map<Uuid, AssetMetaVm>`; bytes only via serve URL (RBC R4)                                                                           | same                                                                        |
| 11      | `source_item_assets` (KnowledgeStore)                      | folded into S2 item/asset VMs                                                              | Association lists on the item VM                                                                                                      | same                                                                        |
| 12      | `project_tasks` (KnowledgeStore)                           | S2 `projectStore.tasks[projectId]`                                                         | `Map<Uuid, TaskVm>` with origin/confidence/evidence refs                                                                              | same                                                                        |
| 13      | `task_evidence` (KnowledgeStore)                           | folded into S2 task VMs                                                                    | Evidence array on `TaskVm`                                                                                                            | same                                                                        |
| 14      | `project_snapshots` + `snapshot_evidence` (KnowledgeStore) | S2 `projectStore.snapshot[projectId]`                                                      | Current snapshot + evidence; history count only                                                                                       | same + `current_snapshot` refetch                                           |
| 15      | `processing_jobs` (KnowledgeStore, tier B)                 | S2 `projectStore.importProgress`                                                           | Per-source progress checklist model (plan §6.12 style)                                                                                | coalesced `import://progress` (tier B)                                      |
| 16      | `change_log` (append-only helper)                          | **no frontend mirror** — diagnostic/incremental-analysis basis only (plan lines 1322–1325) | —                                                                                                                                     | —                                                                           |
| 17      | FTS5 index (derived)                                       | S2 `projectStore.searchResults`                                                            | Query-keyed result cache (hits carry entity id + position per plan lines 1329–1331); a cache, not an entity mirror                    | refetch per query; invalidated wholesale on any tier-A project-domain event |
| N4      | `transcription_history` (legacy HistoryManager)            | S3 `dictationStore.history`                                                                | Existing paginated history projection                                                                                                 | existing history events (legacy carve-out, untouched)                       |

Mirror proof (gate G1): rows 1–15 of RBC-§3 map onto exactly one S1/S2 slice
each, rows 16–17 map onto "no mirror"/"cache" with an explicit reason, and
the legacy carve-out N4 maps onto S3 — the matrix is total and injective.
No other store may add a slice for any listed entity (rule B2).

### 2.3 Selector discipline

Components read via narrow selectors (`useMeetingStore(s =>
s.sessions.get(id))`). Selectors return stable references; list views select
derived arrays produced by memoized selectors so that unrelated envelope
events do not re-render visible lists. This is a performance rule, not an
authority rule — B2 remains the correctness boundary.

---

## 3. Exclusive listener ownership (gate G2)

### 3.1 Rules

Each rule eliminates a named defect class and is component-testable (§3.4).

- **L1 — One owner per topic.** Every Tauri event topic has exactly one
  registering module: the listener registrar exported by its domain store
  (`meetingStore.initListeners()` for `meeting://*`,
  `projectStore.initListeners()` for `project://*`/`import://*`,
  `dictationStore.initListeners()` for dictation-history topics). No other
  module may pass that topic to `listen()`. Fan-out to interested parties
  happens _inside_ the store action the registrar calls — additional UI
  reactions subscribe to the store, not to the topic.
- **L2 — Components never call `listen()`.** Components (all of F1–F22 and
  all future meeting-mode components) consume state through hooks only. Two
  sanctioned exceptions, enumerable and closed:
  1. **Window-bootstrap modules** (`src/overlay/main.tsx` pattern): topics
     that exist solely for a specific webview window's lifecycle
     (`theme-changed`, `show-overlay`, `hide-overlay`) are owned by that
     window's entry module, registered once at boot.
  2. **Legacy dictation-era components** listed in §1.1 remain as-is; the
     prohibition binds all new code and any refactor that touches an old
     listener site (convert, don't copy).
- **L3 — Registration is awaited, retained, and idempotent.** A registrar
  keeps `Map<topic, UnlistenFn>`, `await`s every `listen()` promise before
  `initListeners()` resolves, refuses a second registration for an owned
  topic (dev: thrown assertion; prod: logged no-op returning the existing
  handle). This kills the duplicate-delivery class demonstrated by
  `settingsStore.initialize()` (§1.1).
- **L4 — Symmetric teardown.** Every registrar exports
  `disposeListeners()` that calls and drops every retained `UnlistenFn`;
  `init` after `dispose` is valid. Window unload and test teardown must call
  it. A registrar whose listeners cannot be disposed is a contract violation
  — this is what makes the leak class structurally impossible instead of
  review-dependent.
- **L5 — Stale-revision discard.** Every envelope-bearing handler applies
  SSC-O3 verbatim: ignore payloads whose `(entity)` revision is ≤ the
  revision already applied for that entity id. Out-of-order or duplicated
  delivery therefore cannot regress store state.
- **L6 — Subscribe-then-snapshot handshake.** Store initialization registers
  listeners (L3) _before_ issuing the snapshot command, then applies the
  snapshot atomically under the same revision guard. A gap
  (`payload.revision > known + 1`) triggers a snapshot refetch instead of
  guessing intermediate states (SSC-O5/O6). Missed-event recovery needs no
  frontend counter-state.
- **L7 — Handlers delegate, never decide.** A listener body only normalizes
  its typed payload and calls a store action; no toasts, navigation, or
  business branching live in the registrar. Reactions (recovery banner F11,
  toasts) are effects on store state, keeping handlers trivially testable.
- **L8 — Payload hygiene.** Handlers persist nothing from payloads except
  ids, states, revisions, levels, and redacted messages (SSC-O7). Secrets or
  provider keys arriving on any topic are a defect to report, never to store.

### 3.2 Topic registry

Owned topics at contract time (any new topic must be added here and in
`identifier-registry.md` before first use):

| Topic                                                                                          | Producer (contract)                  | Owner (registrar)                 | Notes                                                                                                                                       |
| ---------------------------------------------------------------------------------------------- | ------------------------------------ | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `meeting://session-created`                                                                    | MeetingManager (SSC-§2.2 T01)        | S1                                |                                                                                                                                             |
| `meeting://session-state`                                                                      | MeetingManager (T02–T18)             | S1                                | carries `(meeting_id, state_revision)` per SSC-O2                                                                                           |
| `meeting://session-error`                                                                      | MeetingManager (T04/T13/T17)         | S1                                |                                                                                                                                             |
| `meeting://summary-available`                                                                  | MeetingManager (T12)                 | S1                                | triggers summary refetch (payload carries ids only)                                                                                         |
| `meeting://session-recovered`                                                                  | startup reconciliation (SSC-§6.2 R6) | S1                                | drives F11 `MeetingRecoveryBanner`                                                                                                          |
| `meeting://jobs-updated`                                                                       | queue/manager (SSC-§3.3 step 3)      | S1                                | tier-B aggregates; coalescing allowed (§4.3)                                                                                                |
| `project://entity-changed` _(proposed)_                                                        | KnowledgeStore/services (Phase 8+)   | S2                                | generic batched envelope feed `{entries: [{entity, id, revision, deleted}]}` mirroring `change_log` tails; name pending identifier-registry |
| `import://progress` _(proposed)_                                                               | SourceIngestionService (Phase 9+)    | S2                                | tier-B progress/checklist aggregates                                                                                                        |
| dictation-history update event                                                                 | HistoryManager (existing)            | S3                                | legacy topic, name unchanged                                                                                                                |
| window-scoped: `show-overlay`, `hide-overlay`, `recording-ready`, `mic-level`, `theme-changed` | overlay/tray plumbing (existing)     | window bootstrap (exception L2.1) | never domain data                                                                                                                           |

Recorder level/stream events stay behind `MeetingManager` (SSC-O4): the
frontend subscribes to manager-published topics only, never to raw recorder
events.

### 3.3 Enforcement

Mechanical, following the RBC-§7.3 script precedent: a lint/static gate may
grep `src/components/**` and `src/hooks/**` for `from "@tauri-apps/api/event"`
and fail on any hit (the closed exceptions of L2 live outside those globs).
Like RBC-R5 this turns a convention into a checkable invariant.

### 3.4 Testability — each rule vs. its defect class (gate G2)

Written for vitest + React Testing Library (harness to be added by the
implementing task, §0.2 item 4); each test mocks the Tauri event API with an
in-memory emitter.

| Defect class                                      | Test (per rule)                                                                                                                                                                                                                                                 |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Second owner on an owned topic (L1)               | Static-gate check (§3.3): the lint rule fails when a file outside the registry's owner set passes a registered topic to `listen()`; in the test suite this is a fixture assertion over the registry table (every topic maps to exactly one registrar constant). |
| Leak (LK-1): unsubscribable store listeners       | L4: `initListeners()` → `disposeListeners()` → assert emitter has zero remaining handlers for every owned topic; repeat init/dispose twice, assert still zero.                                                                                                  |
| Duplicate delivery (DUP-1): double `initialize()` | L3: call `initListeners()` twice, emit one event, assert the store action ran exactly once. Component variant: render two mounted consumers, emit, assert one state change.                                                                                     |
| Component resubscription storms                   | L2: mount/unmount a consumer 3× around emits, assert handler count for the topic never exceeds 1 (registration lives at store level, unaffected by mounts).                                                                                                     |
| Stale/out-of-order application                    | L5: emit revision 5, then 3, then 5 again; assert applied state equals the rev-5 payload and rev-3 was discarded.                                                                                                                                               |
| Missed events during init                         | L6: emit an event _between_ subscription and snapshot resolution; assert final state reflects the event, and a forced revision gap triggers exactly one snapshot refetch.                                                                                       |
| Handler side effects                              | L7: fire each topic with a malformed payload; assert store records an ingest error and no toast/navigation fired (those belong to effects on state).                                                                                                            |
| Secret ingress                                    | L8: payload containing a key-shaped field; assert the field is absent from all store slices afterwards.                                                                                                                                                         |

---

## 4. Change-envelope consumption (how RBC-§5 reaches the stores)

### 4.1 Envelope application (tier A)

Every `project://entity-changed` entry and every `meeting://session-*`
payload carries the RBC-§5 envelope projection
`(id, revision, updatedAt?, deleted?)`. Application algorithm per store
action `applyEnvelope(entry)`:

1. Let `known = appliedRevisions.get(entry.id)`. If `entry.revision <= known`
   → discard (L5). Else proceed.
2. If `entry.deleted` → remove the entity from its `byId` map, drop it from
   all derived lists, bump the touched list versions (§4.2). Tombstones are
   never stored as rows — deletion surfaces as absence _after_ the tombstone
   event, never as inferred absence (RBC-§5.4: consumers never infer
   deletion from silence).
3. Else upsert the payload (or, for thin notifications carrying only ids,
   mark the entity stale and refetch it) and record the revision.

Because RBC-W4 guarantees revision increments of exactly 1 per commit, the
guard is exact: equality means "already applied", superiority means "newer".

### 4.2 List and page invalidation

Domain stores keep small, explicit invalidation counters
(`listVersion.projects`, `listVersion.meetings`, per-meeting transcript
pages, per-project task pages). Any applied envelope bumps the counters of
every list whose predicate could include the entity. List components issue
await-authoritative refetches through store actions when the counter they
selected changes; they never mutate list caches themselves. Sorting,
filtering (plan §6.5 filter chips), and search-result assembly happen in
selectors over refetched pages — the frontend never re-implements backend
queries.

### 4.3 Tier-B operational channels (rows 3, 15)

Job-state churn carries no envelope (RBC-W6) and would flood rendering if
applied per transition. Rules:

- Progress topics (`meeting://jobs-updated`, `import://progress`) carry
  aggregates; stores may coalesce bursts to at most one applied update per
  animation frame window (trailing edge).
- Terminal states (job `failed`, import finished) are never coalesced away:
  the last message of a burst is always applied, and the consuming view
  refetches authoritative counts on terminal signals (checklist of plan
  §6.12 must end truthful: "✓ 42 von 42").
- Backlog counters drive honest UI states ("Abschnitt wird transkribiert",
  backlog badge) but never gate user actions on their own — gating states
  come from session/job snapshots.

### 4.4 What stores must never do

- Never derive an entity's existence, order, or deletion from event silence.
- Never write projected state back toward the backend implicitly (every
  backend write goes through an explicit command action, §5).
- Never store secrets, absolute paths, or rowids (B3, L8, RBC-§5.1/§7.1 R4).
- Never keep a second copy of an entity in another store (B2) — cross-store
  references are UUIDs only (mirroring RBC-§3's cross-store ruling).

---

## 5. Command invocation — components never call commands (extends RBC-R5)

- **C1 — Single choke point.** All `commands.*` (generated bindings,
  `src/bindings.ts`) invocations for meeting/project/import domains occur
  inside domain-store actions. Components express intent:
  `useProjectStore(s => s.renameProject)` — never
  `import { commands } from "@/bindings"` for a domain entity.
- **C2 — Actions own error translation.** An action either succeeds, or
  records a typed failure on its entity/request slot (`errors[requestKey]`)
  and rethrows for callers that asked to know. Raw backend error strings
  never reach JSX directly; mapping to actionable i18n copy follows plan §6.2
  principle 8 and the UX copy catalog.
- **C3 — No new IPC channels.** Meeting-mode code uses only generated
  bindings; importing `@tauri-apps/plugin-sql` (present in package.json for
  legacy reasons), raw `invoke()`, or `@tauri-apps/api/fs` for storage-root
  access is prohibited (RBC-§7.1 R1/R4/R5; verified at HEAD that `invoke(`
  has zero matches under `src/` — keep it that way).
- **C4 — Enforcement.** Same static-gate approach as §3.3: ESLint
  `no-restricted-imports` (or equivalent) bans `@tauri-apps/api/core`,
  `@tauri-apps/api/event`, and `plugin-sql` under `src/components/**`,
  `src/hooks/**`, and `src/overlay/**` components; only `src/stores/**` and
  window-bootstrip modules may import them.
- **Legacy carve-out:** existing dictation-era components that call
  `commands.*` directly (settings forms, model selector) are out of scope and
  remain governed by their current patterns; refactors touching them should
  migrate toward C1 but are not obligated by this contract.

---

## 6. Optimistic updates — whitelist and rollback (gate G3)

Default posture: **await-authoritative**. A UI element shows a pending state
(the `isUpdating[key]` pattern of `settingsStore` lines 342–385) until the
command's typed result arrives; the store applies the returned authoritative
value. Optimism is the exception, allowed only when **all** of the following
hold:

1. The operation touches exactly one entity via one command (no sequences);
2. failure is recoverable by exact rollback to the pre-call snapshot;
3. success/failure does not gate safety-relevant behavior (never recording
   control, never deletion, never money/cloud/data-exit);
4. the entity is not gated by a backend state machine transition (SSC §2.2).

### 6.1 Whitelist (exhaustive)

| #   | Operation                                                                       | Command family  | Rollback              |
| --- | ------------------------------------------------------------------------------- | --------------- | --------------------- |
| O-1 | Rename a meeting (`title` patch)                                                | session update  | restore prior `title` |
| O-2 | Edit project `name`/`description`/`color`                                       | project update  | restore prior fields  |
| O-3 | Edit fields of a `human`-origin task (`title`, description, priority, due date) | task upsert     | restore prior fields  |
| O-4 | Toggle a human task between `open` ↔ `done`                                     | task transition | restore prior state   |

Everything else is await-authoritative, notably: start/pause/resume/stop
meeting (backend state machine owns truth — plan lines 924–927; the Start
button reflects `starting/recording` only via events, never local prediction),
deletes of any kind (plan §11 confirmation flows), imports and their commits,
project link/unlink and assignment accept/dismiss (confirmation-required per
plan lines 788–798), job retries, provider/model/language changes, summary
regeneration, and anything touching consent or cloud disclosure (plan §12).

### 6.2 Rollback mechanics

Adopt the proven settings pattern verbatim in shape:

- Capture the entity snapshot before the optimistic `set`.
- Serialize per entity (one in-flight optimistic mutation per entity id;
  later edits queue behind it — analogue of `enqueueSettingUpdate`,
  `settingsStore.ts` lines 88–108).
- Keep a per-entity monotonic request version; on failure, roll back only if
  the version still matches, so an older failed request can never revert a
  newer accepted value (bug class already prevented there, lines 349–371).
- On rollback, surface an actionable error with `Erneut versuchen` (plan §6.2
  principle 8); the retried attempt is a fresh await-authoritative call.
- An envelope arriving mid-flight (revision bumped by another writer) wins
  over both the optimistic value and the rollback target (§4.1 guard runs
  first); the optimistic path detects the newer revision and abandons
  silently apart from disabling its pending flag.

---

## 7. Complexity budget and non-goals (binding)

Consistent with the company complexity budget, this contract rejects:

- **No second state library.** No Redux/RTK-Query/Jotai/MobX alongside
  Zustand; no normalization layer (normalizr-style) — plain per-domain maps
  suffice for v1 data volumes.
- **No client-side caching/sync framework.** No React Query/SWR introduction
  for domain data: the envelope invalidation of §4 is deliberately smaller
  than any generic cache-invalidation engine, and the plan forbids pretending
  sync exists before the server phase (plan lines 1161–1162, 1764–1766).
- **No offline mutation queue, outbox, or retry-on-behalf-of-user logic in
  the frontend.** Durability and retry belong to persistent jobs (RBC tier B,
  job-scheduler-contract).
- **No persisted frontend state** (B4) and no frontend-side reconciliation
  protocol — restart truth comes from snapshots (SSC §5/§6).
- **No frontend-derived authority:** no local "recording state" machine, no
  predicted task IDs, no client-generated UUIDs for core records (ids are
  minted by stores, RBC-§5.1).
- **No speculative stores** for future roadmap items (server sync, iPhone,
  MCP — plan lines 1738–1783). When those arrive they consume the same
  services; no frontend seam reserved now.
- **No per-frame React state.** Level meters/timers bypass Zustand entirely
  (B5) rather than introducing throttled store traffic.

Anything not stated as owned in §2.2 or permitted in §6.1 is a non-goal until
the plan changes.

---

## Appendix A — Gate verification checklist

| Gate                                                                                         | Where satisfied                                                                                                                                                                                                                                                          |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| G1 store-ownership matrix mirrors backend entity-ownership 1:1                               | §2.2 (total, injective mapping of RBC-§3 rows 1–17 + N4 carve-out, with explicit "no mirror"/"cache" rulings for rows 16–17); boundary rules B1/B2 make divergence a defect                                                                                              |
| G2 listener lifecycle eliminates leak/duplicate classes, each rule testable                  | §3.1 rules L1–L8; §3.4 defect-class table naming one mechanical component test per class (LK-1, DUP-1, resubscription, stale-apply, missed-event, side-effect, secret-ingress); baseline defects evidenced in §1.1 (`settingsStore.ts` 674–683, `modelStore.ts` 268–428) |
| G3 optimistic updates limited to stated safe operations, everything else await-authoritative | §6 default posture with four-part admissibility test; exhaustive whitelist O-1–O-4; explicit forbidden list (recording control, deletes, imports, assignments, provider changes); rollback mechanics with version guard cited from existing code                         |
| G4 non-goals section + documentation index updated                                           | §7 complexity budget/non-goals; row appended to `docs/meeting-mode-documentation-index.md` §3 table per its Pflegeregel                                                                                                                                                  |

## Appendix B — Implementation prerequisites (for the implementing tasks)

1. Add vitest + `@testing-library/react` (+ `jsdom`) and a Tauri-event mock
   harness; port §3.4's seven tests as the acceptance net for S1–S3.
2. Register proposed topics `project://entity-changed` and
   `import://progress` in `identifier-registry.md` when their producing
   phases are contracted.
3. Wire the ESLint restriction of §5/C4 when the first store lands (cheap
   before violations exist).
