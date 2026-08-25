# Evidence-Citation → UI-Navigation Contract — Clickable Belege in Meeting- und Projekt-Detail

Status: **spec contract — no implementation yet** (Phase 7/Phase 10 prerequisite;
changes no source code)

Source of truth: `docs/meeting-mode-plan.md` — principle 8 „Belege vor
KI-Automatik" (line 135), §6.13 Meeting-Detailansicht (lines 659–681,
Transkript-Tab mit „Zeitstempeln und Audioposition", line 673), §6.14
Zugänglichkeit und Fehlbedienungsschutz (lines 683–695), §9 `task_evidence`
(lines 1295–1301, locator rule line 1297), `snapshot_evidence`
(lines 1303–1311), Such-/Belegregel (lines 1329–1332), §11 Löschung
(lines 1425–1434, „zu prüfende Belege" lines 1431–1434), §12 Aufbewahrung
(lines 1444–1452), §13 Phase 7 acceptance (line 1668: „Status- und
Aufgabenansichten zeigen keine Aussage ohne erreichbaren Beleg"), §14 M20
(line 1902: „Suche springt zur richtigen Chatnachricht oder
Meeting-Zeitposition").

This document fixes the **evidence-citation-to-navigation seam**: how a stored
evidence pointer (summary-evidence-contract §3.2 field set) becomes a
clickable citation in the React frontend, how the pointer resolves to a
scrollable transcript segment or a source item, what the user sees when the
target has since been deleted or trimmed by retention, and how focus and
screen-reader announcement behave on activation consistent with plan §6.14.
It defines no backend implementation, no schema change, and no visual design.

Sibling contracts consumed by this document (read-only):

- [summary-evidence-contract.md](summary-evidence-contract.md) — abbreviated
  **SEC**. Its pointer field set (§3.2), validity conditions (§3.3 rules 1–4)
  and invariants E1/E2 are the normative basis of §2/§3 below.
- [export-project-dataflow.md](export-project-dataflow.md) — abbreviated
  **EPD**. Its canonical `MeetingPositionRef` (§3.1) and four-outcome
  resolution matrix incl. deleted audio (§3.3) are adopted **verbatim** as the
  meeting-side sub-matrix (§3.2); its `needs_review` marking (§8) appears in
  §3.5 as orthogonal decoration.
- [project-snapshot-contract.md](project-snapshot-contract.md) — abbreviated
  **PSC**. Its read-time pin-aging statuses `live` / `stale_quote` /
  `locator_gone` / `source_tombstoned` (§3.3) supply the stale/dangling
  vocabulary reused in §3.
- [navigation-seam-contract.md](navigation-seam-contract.md) — abbreviated
  **NSC**. Its route model (§2), tabs-are-not-routes ruling (R3), back
  affordance (N-R4) and persistence allowlist (§3.2) bind where a citation
  may navigate and what it may persist.
- [frontend-state-contract.md](frontend-state-contract.md) — abbreviated
  **FSC**. Command choke point C1/C3 (§5), selector discipline (§2.3),
  revision guard and listener ownership rules govern every code path named
  here.
- [repository-boundary-contract.md](repository-boundary-contract.md) —
  abbreviated **RBC**. Choke-point rules (§7.1) and typed reads over the
  trait seam (§2) are why §4 can forbid direct DB/file access mechanically.
- [search-index-contract.md](search-index-contract.md) — abbreviated **SIC**.
  Its `HitPosition` union (§6.1) and jump-target convergence guarantee
  (§6.2) mean search hits enter this contract already carrying a canonical
  position type.
- [retention-quota-contract.md](retention-quota-contract.md) — abbreviated
  **RQC**. Its decision matrix (§6.1: automatic deletion touches audio only;
  transcript deletion is manual level 2/3) determines which dangling cases
  can occur without user action (§3.4).
- [error-taxonomy-contract.md](error-taxonomy-contract.md) — abbreviated
  **ETC**. Resolution failures surface as copy keys, never raw errors; new
  keys follow its namespace discipline as catalog hand-offs (§5.5).

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual
code. Where this contract restates SEC/EPD/PSC rules it does not relax them;
on any divergence the owning contract's wording prevails.

---

## 0. Scope, non-goals

### 0.1 In scope

1. The closed enumeration of citation surfaces — which UI elements become
   clickable and from which data fields they render (§2).
2. The resolution algorithm: one typed outcome per pointer covering valid,
   stale, and dangling targets, including retention-trimmed audio and
   re-transcribed segments (§3, gate G1).
3. The access rule: resolution exclusively through store actions over the
   repository seam — never direct DB/file access from components (§4, gate
   G3).
4. Activation behavior: navigation, programmatic focus move, screen-reader
   announcement, highlight, and focus return, per plan §6.14 (§5, gate G2).

### 0.2 Explicitly out of scope

1. **Pointer storage, validation at publish time, and pin semantics** — owned
   by SEC §3 and PSC §3; this contract consumes the stored rows read-only.
2. **The backend resolution implementation** (SQL, chunk-state queries,
   whitespace-normalized quote comparison) — owned by the implementing
   store/repository task; §3 fixes only the typed outcome contract.
3. **Visual design of citations** (colors, icons, exact copy) — UX workstream
   and UX copy catalog; §2 fixes structure and accessible names only.
4. **Export serialization** of positions (`[mm:ss]` markers,
   `speak-meeting-position://` URIs) — owned by EPD §4/§6.
5. **Search result lists themselves** — owned by SIC; this contract starts
   where a hit's `HitPosition` has already been produced.

---

## 1. Position in the architecture and verified baseline

Citations sit entirely inside the React projection layer (FSC §1). They hold
no domain authority: a citation is a *view* over an already-persisted pointer
row plus one resolution query. Losing navigation state never loses evidence —
the pointer row is the truth, the citation is one of its renderings.

Verified observations at authoring time (HEAD):

| Observation | Location | Consequence |
| --- | --- | --- |
| All citation-bearing components (F9 `MeetingDetail`, F14 `ProjectDetail`, F15–F17 tabs) are `planned-new`; F10 `MeetingList` exists as Phase 3 work-in-progress, but repo-wide grep for `evidence`/`Beleg`/`citation` under `src/` returns zero matches | module-placement-map lines 154, 159–162; verified: only `src/components/meetings/{index.ts, MeetingList.tsx, meetingList.render.test.tsx}` exist, no `src/components/projects/`; grep empty | §2 defines the seam before any citation implementation; no citation code exists to migrate. |
| The app already owns a proven programmatic-focus pattern: save `document.activeElement` on open, restore on teardown | `src/components/ui/Dialog.tsx` lines 68–93 (save 71–74, restore 90) | §5.4 focus return adopts this exact pattern for the jump-back case. |
| Sidebar navigation is currently mouse-driven (`onClick`, no keyboard path) | `src/components/Sidebar.tsx` line 45; a11y-baseline §6.14.2 FAIL, delta D1 | Citations must not inherit this defect: §5.1 mandates real `<button>` semantics from day one. |
| Beyond the sonner toaster there is no `aria-live` region anywhere in `src/` | a11y-baseline §6.14.5 (repo-wide grep, zero matches in source) | §5.1 step 5 introduces exactly one polite announcement region for jumps — no scattered live regions. |

Layer position relative to the deep-open seam: citations are *in-app*
navigation. They dispatch through the same single `navigate(route)` door as
nav clicks (NSC §3.1); the `main-window-navigate` topic plays no role here
and none is added.

---

## 2. Citation surfaces (what becomes clickable)

### 2.1 Closed enumeration

Exactly five surfaces render stored pointers as citations. Anything else is
out of scope until this table grows:

| # | Surface (component home) | Pointer source | Renders from |
| --- | --- | --- | --- |
| K1 | Structured summary items — Entscheidungen / Aufgaben / offene Fragen in F9 `MeetingDetail` Übersicht tab | per-item evidence arrays embedded in `meeting_summaries.decisions/action_items/open_questions` (SEC §3.4) | `locator`, `time_range`, `quote_excerpt`, `rationale` (SEC §3.2) |
| K2 | Task cards in F16 `ProjectTasks` | `task_evidence` rows (plan lines 1295–1301) | same field set + `review_state` (EPD §8.1) |
| K3 | Snapshot statements in F15 `ProjectOverview` | `snapshot_evidence` rows with pinned revisions (PSC §3.2) | field set + pinned revisions |
| K4 | Transcript/search hits in F9 Transkript-Tab search and project search results | `HitPosition::MeetingPosition(MeetingPositionRef)` resp. `HitPosition::SourceItem` (SIC §6.1) — converted to the §3 input shape before navigation (EPD §3.2) | position ref + fragment |
| K5 | Conflict entries in snapshot views (both positions cited separately) | two pointer sets per conflict (SEC §3.3 E3) | field set, one citation per side |

Free prose inside `summary_markdown` is **never** parsed for links or made
clickable (SEC §3.4 exempts it); its traceability rides on the structured
sections above. A renderer that injects anchors into the markdown body is a
defect against this contract.

### 2.2 Citation element requirements (structural, not visual)

Every citation regardless of surface:

1. Is a real `<button>` (keyboard operable and space-activatable by platform
   default — no click-only `div`, per a11y-baseline §6.14.2 counter-example
   class).
2. Hit target ≥ 44 × 44 CSS px (plan line 685), compact visuals permitted via
   transparent padding per a11y-baseline delta D2/D13.
3. Carries a visible `focus-visible` indicator (plan line 686).
4. Has an accessible name built from structured fields, never from the raw
   UUID: meeting-side `„Beleg, mm:ss im Transkript"` (or `h:mm:ss`),
   source-side `„Beleg, <Quellname>, Abschnitt"`; the name localizes through
   i18next (AGENTS.md i18n rule; a11y-baseline D10 defect class banned).
5. Exposes `rationale` and `quote_excerpt` as complementary description
   (`aria-describedby` or tooltip that opens on focus as well as hover —
   plan line 692: tooltips complement labels, never replace them).
6. Announces its own degradation state in the name when the last known
   resolution was stale/dangling (§3.6), so screen-reader users hear
   `„Beleg nicht mehr erreichbar"` before activating, not silence.

---

## 3. Pointer-resolution algorithm (gate G1)

### 3.1 Shape

Resolution is a request/response query returning exactly one typed outcome;
the frontend renders purely from it and computes nothing about persistence
state itself:

```ts
// All meeting-side positions use EPD §3.1 MeetingPositionRef verbatim.
type ResolvedKind =
  | { kind: "resolved_playable";  position: MeetingPositionRef }   // EPD §3.3 row 1
  | { kind: "resolved_text_only"; position: MeetingPositionRef }   // EPD §3.3 row 2 (AudioDeleted)
  | { kind: "approximate_jump";   position: MeetingPositionRef;    // EPD §3.3 row 3 (SegmentMissing)
      nearestSegmentId: Uuid }
  | { kind: "resolved_item";      sourceId: Uuid; itemId: Uuid };

type CitationOutcome =
  // meeting-side (locator = meeting_transcript_segment_id)
  | ResolvedKind
  | { kind: "unresolvable";     reason: "meeting_deleted" }          // EPD §3.3 row 4
  // source-side dangling targets — two distinct reasons, never collapsed
  | { kind: "item_gone";        reason: "locator_gone" | "source_tombstoned" }
  // cross-cutting staleness overlay (SEC §3.3 rule 3 / PSC §3.3):
  // staleness wraps any *resolved* outcome above with its fields unchanged;
  // it never wraps a dangling outcome.
  | { kind: "resolved_stale_quote"; inner: ResolvedKind };

`resolved_stale_quote` wraps a successful outcome whose `quote_excerpt` no
longer matches the current locator text under the whitespace-normalized
comparison of SEC §3.3 rule 3. That comparison runs **inside the repository
seam** (same logic as publish-time validation, SEC §3.3), not in JavaScript
over fetched full texts — the frontend receives the verdict, never performs
it.

### 3.2 Decision table — meeting-side pointers

Adopts EPD §3.3 verbatim; each row names the distinct user-visible outcome
that satisfies G1:

| Precondition | Outcome | User-visible behavior |
| --- | --- | --- |
| Segment exists; ≥1 `ready` chunk covers `start_ms` (audio not purged, plan lines 1444–1452) | `resolved_playable` | Jump lands on segment, highlight on; player offers seek to position (media clock owned by player, EPD §3.2) |
| Segment exists; covering audio deleted (automatic level-1 retention per RQC §6.1 R1/R2, or manual „nur Rohaufnahme löschen") | `resolved_text_only` | Jump still lands and highlights — transcript survives audio deletion (RQC §6.1: „transcript + summary kept"); player control hidden with explanatory copy |
| Meeting exists; anchor segment gone (re-transcription replaced it, or manual level-2 transcript deletion survived partially) | `approximate_jump` | Lands at nearest segment containing `start_ms`, visibly marked approximate („ungefähr") |
| Whole meeting finally deleted (plan line 1429) | `unresolvable(meeting_deleted)` | Inert dead-link explanation at the citation itself; never an error crash, never silent no-op |

### 3.3 Decision table — source-side pointers

| Precondition (checked in order) | Outcome | User-visible behavior |
| --- | --- | --- |
| Source live (`deleted_at IS NULL`, SEC rule 1) and item exists, belongs to that source (rule 2); quote matches (rule 3) | `resolved_item` | Navigate to project detail Quellen tab (F17), scroll to and highlight the item |
| Source live, item exists, but item content revised so the quote no longer matches | `resolved_stale_quote` wrapping `resolved_item` | Normal jump; citation and claim display carry the stale badge; claim marked unverifiable per PSC §3.3 |
| Source live, item tombstoned/deleted | `item_gone(locator_gone)` | Inert citation with explanation naming the cause; claim's evidence display degrades (PSC §3.3) — the claim itself stays visible, never silently vanishes |
| Source tombstoned | `item_gone(source_tombstoned)` | Distinct explanation (whole source removed vs. single item); otherwise as above |

The two `item_gone` reasons are deliberately separate outcomes: „this chat
message was deleted" and „this entire WhatsApp archive was removed" answer
different user questions, and collapsing them would hide which cleanup
action happened (G1: dangling targets get a *distinct* outcome, not a shared
error).

### 3.4 Retention interplay (why the tables close)

RQC §6.1 fixes what can dangle without user action: automatic deletion
touches **audio only** (level 1) and never transcript, summary, tasks or
sources. Therefore:

- Automatic retention can produce only `resolved_playable → resolved_text_only`
  transitions (plus the N1/N2 notices, RQC §6.2) — never a silent dead
  citation. This is invariant E2 („search and UI jump targets resolve",
  SEC §3.3) holding across the retention boundary, restated from EPD §3.3.
- Every dangling outcome requires an explicit user deletion (levels 2/3,
  source/item removal) or a re-transcription replacement — all of which emit
  change envelopes that invalidate cached resolutions (§3.6).
- Level-2/level-3 deletes remove the claim carrier together with its
  embedded citations when they live in the same artifact (summary rows die
  with the meeting tree); cross-artifact survivors (tasks of other projects
  citing a deleted meeting) resolve to `unresolvable`/`item_gone` and keep
  the claim visible with degraded evidence, per PSC §3.3.

### 3.5 Orthogonal decoration: `needs_review`

A `review_state = 'needs_review'` mark (EPD §8, unassignment flow) changes
only the citation's badge and accessible-name suffix („zu prüfender Beleg");
it never alters the resolution outcome. A needs-review pointer whose target
is intact resolves fully navigable — marking means *provenance under
review*, not broken.

### 3.6 Currency and race rules

1. Resolution results are cached per `(pointer identity, target revision)`
   inside the owning store (S1/S2) and invalidated by the change-envelope
   rules of FSC §4 (tombstone/revision bump on any cited entity). No polling
   (FSC B5 spirit; NSC §6.2 analog).
2. A stale-cache window is acceptable only toward the *honest* direction:
   if events arrive between resolution and landing showing the target just
   became dangling, the landing view renders the dead-link/approximate
   explanation instead of focusing a nonexistent element (§5.3). A
   jump must never land silently wrong.
3. Resolution failures (store error, command rejection) map to ETC-style
   error classes with retry affordance at the citation — raw backend strings
   never reach JSX (FSC C2).

---

## 4. Access discipline — repository seam only (gate G3)

1. Components call one store action per jump —
   `useMeetingStore(s => s.resolveEvidence)` /
   `useProjectStore(s => s.resolveEvidence)` — and receive the §3.1 outcome.
   They never import `commands`, never touch `invoke`/`plugin-sql`/fs APIs
   (FSC C1/C3, RBC §7.1 R1/R4/R5).
2. Store actions call generated bindings only; the backing Tauri commands
   delegate to `MeetingRepository`/`KnowledgeRepository` reads (RBC §2 trait
   seam). Chunk/audio availability facts come from the same repositories'
   projections — the frontend never inspects asset keys, file existence, or
   SQLite rows (RBC §7.1; EPD §2: entity IDs, never absolute paths).
3. The command surface exchanges only typed values (outcome shapes of §3.1,
   `MeetingPositionRef` of EPD §3.1) — no paths, no rowids, no internals
   (SIC §6.2 command-layer shape applies verbatim).
4. No new event topic is proposed by this contract: resolution is strictly
   request/response; currency comes from existing envelope channels (FSC
   §4). Nothing to register in identifier-registry.md.

---

## 5. Activation: navigation, focus, and announcement (gate G2)

### 5.1 Activation sequence (normative order)

On citation activation (click, Enter, Space, assistive-tech activation):

1. **Resolve** via §4 (cached outcome may serve immediately).
2. **Navigate** within the already-open detail route per NSC R3: switch the
   local tab state of F9 to `Transkript` (meeting-side) or of F14 to
   `Quellen` (source-side). Tabs are local view state, not routes — a
   citation never dispatches a route change unless the required detail view
   is not yet open, in which case it opens
   `{ meetings, child: { detail, meetingId } }` resp.
   `{ projects, child: { detail, projectId } }` through the shell's single
   `navigate()` (NSC §3.1). An `unresolvable`/`item_gone` outcome skips
   steps 2–3 entirely.
3. **Scroll**: bring the target element into view (`block: "center"`),
   instant scroll when `prefers-reduced-motion` is set (plan line 691).
4. **Focus**: move focus programmatically to the target anchor element,
   which carries `tabindex="-1"` and a stable id. Focus moves *after* the
   element exists in the DOM (post-render effect), satisfying plan §6.14
   „vollständige Tastaturbedienung": keyboard users end up *at* the beleg,
   with the visible-focus indicator of §2.2 item 3 applied to the target.
5. **Announce**: one polite live-region message describing the landing
   target — meeting-side `„Beleg bei mm:ss"` plus the segment's opening
   words, source-side `„Beleg: <item context>"`. Restrained per plan line
   689: announced once per activation, never on scroll, never per frame,
   and never as a focus steal.
6. **Highlight** the target non-color-only (outline plus background token,
   plus `aria-current="true"` while active so SR users relocating by
   browsing find it); auto-clear after a fixed dwell or on the next
   interaction. Color alone is insufficient (plan §6.14 contrast/name rules;
   a11y-baseline §6.14.10 color-alone defect class).

### 5.2 What focus must never do

- Passive state changes never move focus: a citation turning stale or
  dangling because an envelope arrived announces nothing and steals nothing;
  only its rendered state changes (plan line 689).
- Deletion confirmations around cited entities follow their own flows
  (plan line 1431) and restore focus per the Dialog pattern; this contract
  adds nothing to them.

### 5.3 Landing-race handling

If step 2–4 raced a deletion (§3.6 rule 2), the transcript/source view
renders the honest degraded explanation at the top of the target list and
focus moves to that explanation element instead of a vanished anchor — the
user always lands somewhere focusable and truthful, never on a blank scroll.

### 5.4 Focus return

When the user leaves the landed context back toward the originating view
(back affordance, NSC N-R4, or closing the split view), focus returns to the
invoking citation button using the proven save-on-depart / restore-on-return
pattern already coded in `Dialog.tsx` (lines 71–74, 90). No global history
stack is introduced; the return target lives in local component state of
the hosting view and dies with it (NSC §3.2 allowlist untouched).

### 5.5 Copy hand-off

All user-visible strings (accessible names, stale/dangling explanations,
approximate marker, announcement texts) are i18next keys handed to the UX
copy catalog owner as proposals, namespaced consistently with ETC and RQC
§6.2 conventions (e.g. `meeting.evidence.*`, `projects.evidence.*`). This
contract fixes none of their wording.

### 5.6 Testability obligations (for the implementing tasks)

1. Activating a citation moves DOM focus to the target element (assert
   `document.activeElement`), announces exactly one polite message per
   activation, and restores focus on back.
2. Each §3 outcome produces its distinct rendering (snapshot per outcome);
   dangling outcomes render explanation + degraded claim without navigation.
3. Static gates hold: no `commands` import under `src/components/**` for
   evidence resolution (FSC C4 lint rule extends unchanged); no `listen()`
   in citation components (FSC L2).
4. Reduced-motion setting switches scroll behavior to instant.

---

## 6. Complexity budget and non-goals (binding)

Consistent with the company complexity budget and anti-bloat mandate, this
contract rejects:

- **No markdown-prose linking.** Free summary text gains no inline anchors
  (§2.1); traceability stays in structured sections (SEC §3.4).
- **No second position format.** Everything meeting-side flows through
  EPD §3.1 `MeetingPositionRef`; no per-chunk deep links, no ad-hoc
  `{seconds}` tuples (EPD §9).
- **No router/URL integration.** Citations never produce URLs, hash routes,
  or OS-level deep links (NSC §8); `speak-meeting-position://` remains a wire
  serialization, not a webview address.
- **No client-side media math.** The player converts positions itself
  (EPD §3.2); citation code never maps timeline offsets to chunks.
- **No fuzzy relocation beyond EPD.** Nearest-segment fallback is EPD §3.3
  row 3, nothing smarter; no similarity scoring, no ML re-anchoring.
- **No citation history stack.** One save/restore return target per hosting
  view (§5.4); no back-button semantics, no persisted last-jump state (NSC
  §3.2 allowlist stays closed).
- **No polling, no new topics.** Currency via existing envelopes (§4 item 4,
  §3.6); zero identifier-registry additions.
- **No auto-navigation.** Citations act only on explicit activation;
  rendering a summary never scrolls or focuses anything (plan line 689).
- **No visual design here.** Badges, colors, iconography belong to the UX
  workstream; only structure, outcomes, and accessible semantics are
  normative.

---

## 7. External-citation disclosure

| Citation | Status at authoring time |
| --- | --- |
| `src/components/ui/Dialog.tsx` focus save/restore (§1, §5.4) | **LIVE-VERIFIED** — file read in this workspace; save at lines 71–74, restore at line 90. |
| `src/components/Sidebar.tsx` click-driven nav entry (§1) | **LIVE-VERIFIED** — grep shows `onClick={() => onViewChange(entry.id)}` at line 45; keyboard-path absence attributed to a11y-baseline §6.14.2. |
| Meeting/detail components `planned-new` (§1) | **LIVE-VERIFIED** — module-placement-map lines 154, 159–162; `src/components/meetings/` contains only Phase 3 list scaffolding (`MeetingList.tsx` + test), `src/components/projects/` absent; repo-wide grep `evidence|Beleg|citation` under `src/` → zero matches. |
| EPD §3 resolution matrix, SEC §3 validity rules, PSC §3.3 aging statuses, RQC §6.1 deletion matrix, SIC §6.1/6.2 types | Consumed by reference as sibling spec contracts in this repository; their internal verification obligations remain theirs. No new external (web/vendor) claims are made. |

---

## Appendix A — Gate verification checklist

| Gate | Requirement | Where satisfied |
| --- | --- | --- |
| G1 | Valid, stale, and dangling targets yield distinct user-visible outcomes | §3.1 typed outcomes; §3.2 meeting-side table adopting EPD §3.3 verbatim (playable / text-only / approximate / unresolvable); §3.3 source-side table with two distinct dangling reasons; staleness as wrapping overlay (SEC §3.3 rule 3); closure argument over retention cases §3.4; needs_review orthogonality §3.5 |
| G2 | Focus management satisfies plan §6.14: focus moves to target and is announced | §5.1 activation sequence (real button, ≥44 px, visible focus, post-render `tabindex="-1"` focus move, polite one-shot announcement, non-color highlight, reduced-motion scroll); §5.2 no focus stealing; §5.3 honest race landing; §5.4 focus return via verified Dialog pattern; §2.2 structural citation requirements |
| G3 | No direct DB/file access — resolution through repository-seam queries only | §4.1 store-action choke point (FSC C1/C3); §4.2 repository-trait delegation, no asset-key/file/row exposure (RBC §7.1); §4.3 typed command surface (SIC §6.2); §4.4 no new IPC/event channel |
| G4 | Non-goals section; documentation index updated | §6 complexity budget/non-goals; row appended to `docs/meeting-mode-documentation-index.md` §3 table plus reading-order entry per its Pflegeregeln |
