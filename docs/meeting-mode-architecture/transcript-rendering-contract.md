# Transcript Rendering Contract — Virtualized Multi-Hour Transcript, Append-Only Live Path & Scroll Anchoring

Status: **spec contract — no implementation yet** (Phase 4 prerequisite; changes
no source code)
Source of truth: `docs/meeting-mode-plan.md` — §6.11 "Laufende Meetingansicht"
(plan lines 613–637, decisive: line 627 "keine automatisch springende Ansicht,
sobald der Nutzer nach oben scrollt", line 628 "Button `Zum aktuellen Text`
erscheint dann separat"), §6.13 `Transkript` tab (plan line 673:
"chronologische Segmente, Suche, Zeitstempel und Audioposition"), §8
"das chronologische Transkript wird aus persistierten Segmenten abgeleitet"
(plan line 1065), §9 `meeting_transcript_segments` (plan lines 1201–1208),
§6.20 responsive rules (plan lines 837–852), §6.14 accessibility rules
(plan lines 683–695).
Planned implementation home: one shared windowed-transcript module family
under `src/components/meetings/transcript/` (see §1.3), consumed by F7
`MeetingRecorderView.tsx` and F9 `MeetingDetail.tsx`
([module-placement-map.md](module-placement-map.md)); state lives exclusively
in the S1 `meetingStore.transcript[meetingId]` slice
([frontend-state-contract.md](frontend-state-contract.md) §2.2 row 4).

This document fixes the **rendering seam** for transcripts that grow to
multi-hour length: the windowed/virtualized row model that bounds DOM nodes to
a constant, the append-only update path from the live streaming events, the
segment identity/key scheme, scroll-anchor preservation while the user is
reading above the tail, and the webview memory ceiling for a 4-hour session.
It specifies frontend rendering behavior only; it defines no event payload,
no schema, and no backend behavior.

Sibling contracts consumed by this document (read-only):

- [live-view-streaming-contract.md](live-view-streaming-contract.md) —
  abbreviated **LV-§n**. Its E1 `meeting://transcript-delta` channel
  (§2.1), ordering rules D1–D8 (§4), snapshot + tail-page handshake (§6.2),
  and healing rules (§6.3) are the *only* live input path; this contract
  defines how a renderer consumes them without redefining any of it.
- [frontend-state-contract.md](frontend-state-contract.md) — abbreviated
  **FSC-§n**. Store boundaries (§2), listener ownership L1–L8 (§3), envelope
  and tier-B rules (§4), command choke point (§5) all apply unchanged.
- [session-state-contract.md](session-state-contract.md) — abbreviated
  **SSC-§n**. UI-rebuild rule (§5: UI is a pure function of persistence plus
  events) and O1–O7 apply unchanged.
- [meeting-mode-phase2-contracts.md](../meeting-mode-phase2-contracts.md) —
  abbreviated **P2C-§n**. Its §6 rule 5 fixes the chronological merge order;
  its §6 rule 6 fixes segment size bounds used in the memory computation.
- [stt-provider-contract.md](stt-provider-contract.md) — abbreviated
  **STT-§n**. Its §3.3 segment-insert transaction is the persistence fact the
  live path mirrors.
- [module-placement-map.md](module-placement-map.md) — F7/F9 placements and
  canonical-path discipline.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 1. Scope and additivity

### 1.1 What this contract specifies

1. The **row model and ordering** of every transcript surface (§2), including
   the React key scheme derived from persisted segment identity (gate G3).
2. The **windowed rendering model** that bounds mounted DOM nodes to a stated
   constant independent of transcript length (gate G1, §3).
3. The **append-only live update path** from E1 deltas into the rendered
   window, with the scroll-anchor rules for detached reading (gate G2, §4).
4. The **memory ceiling** for the transcript slice in the webview, computed
   for a 4-hour worst-case session (§5).
5. Reload/resubscribe and healing behavior as consumed from LV-§6 (§6), and
   the accessibility posture of the windowed list (§7).

### 1.2 Surfaces covered

Exactly two surfaces render transcripts, and both use the same module family
with different initial anchors:

| Surface | Plan anchor | Component | Initial anchor |
| --- | --- | --- | --- |
| Live recording main area | §6.11 line 624 | F7 `MeetingRecorderView.tsx` | anchored to tail (follow) |
| Detail `Transkript` tab | §6.13 line 673 | F9 `MeetingDetail.tsx` | top of transcript |

No third transcript renderer may exist (rule R1, §3.3). The processing view
(plan §6.12) shows a checklist, not segments, and is out of scope.

### 1.3 Additivity statement

- **No event payload or topic is defined or changed.** E1/E2/E3 remain owned
  by LV-§2; this contract only prescribes how their payloads reach pixels.
- **No store boundary is moved.** Transcript pages stay in the FSC-§2.2 row 4
  slice; this contract adds the page/cache shape inside that slice and the
  rendering rules above it.
- **No backend behavior is specified.** Page queries are served by the
  existing paged segment query of FSC-§2.2 row 4 / LV-§6.2 step 3; their SQL
  shape is a repository concern (repository-boundary-contract), not this
  document's.
- **Plan rules are restated only where they are made testable** (the
  no-auto-jump rule of plan line 627 becomes rule A2 in §4.2 with a
  measurable anchor definition).

---

## 2. Row model, ordering, and segment identity (gate G3)

### 2.1 The single identity: persisted segment UUID

The React key of every transcript row is the persisted
`meeting_transcript_segments.id` (plan line 1203) — the same UUID the E1
payload carries in its `id` field (LV-§2.1). Rules:

- **R-K1 — Keys are UUIDs, verbatim.** `key={segment.id}`. No
  derivation, no prefixing, no composite strings such as
  `` `${source}-${sequence}` ``, no rowids, no array indices (FSC-B3 already
  bans rowids/paths for state; this extends the ban to React keys, where an
  index key silently corrupts reconciliation on prepend).
- **R-K2 — No second ID format.** No rendering-layer identifier (row handle,
  virtualizer item key, DOM `data-*` attribute, scroll-anchor token) may be
  minted from anything other than the persisted `id`. Anchors reference
  segments by `id`; pages reference segments by `id`. If two artifacts of
  this seam need to agree on a segment, they exchange the UUID — never a
  positional or derived token.
- **R-K3 — Keys survive re-delivery.** D3 stale-discard (LV-§4 D3) and the
  idempotent insert of STT-§3.3 guarantee a segment is inserted once; a
  re-delivered batch therefore cannot produce a duplicate key. A duplicate
  key observed at render time is a defect to report, never to deduplicate
  with a derived fallback key.

### 2.2 Ordering: the shared chronological tuple, unchanged

Display order is exactly the P2C-§6 rule 5 order, consumed without
modification:

```text
(timeline_start_ms, timeline_end_ms, sequence)   ascending
```

`sequence` is the persisted `meeting_transcript_segments.sequence` (plan
line 1207) and serves only as the tie-break of the shared tuple — the
renderer never invents a second ordering. Mic and system segments interleave
freely; equal-time overlaps are handled per §2.4, never merged into one text
run (P2C-§6 rule 5).

**Consumed guarantee (dependency note).** The live path relies on LV-§4 D2
("strictly increasing `last_sequence` per meeting"). The renderer treats
meeting-scoped monotonicity of `sequence` as an input guarantee of the
streaming contract; it does not re-derive or enforce it. If that guarantee
changes, rule A4/R3 below and the D3 guard must be re-adjudicated — this
document registers the dependency so the drift cannot happen silently.

### 2.3 RowModel

The store selector produces a flat, ordered list of row view-models:

```text
TranscriptRow = {
  id: Uuid,               // meeting_transcript_segments.id — the only key
  source: "microphone" | "system",
  startMs: number,        // timeline position (plan §9)
  endMs: number,
  text: string,
  confidence?: number
}
```

Rows are immutable once created: E1 deltas carry finished, persisted segments
only (LV-§2.1 "Append-only"), and re-transcription is *not* a live event
(LV-§8 NG-4 — it belongs to the detail view's envelope/refetch machinery).
Immutability of rendered rows is what makes the anchor compensation of §4.3
sound; any future feature that mutates delivered segment text in place must
re-open this contract.

### 2.4 Exact-time overlap groups

P2C-§6 rule 5 requires equal-time overlaps to render as parallel tracks,
never as one merged text run. The renderer resolves this deterministically:

- Segments whose `(timeline_start_ms, timeline_end_ms)` are **exactly equal**
  form one *track group*. The group renders at most one row per `source`;
  if persistence ever delivered more than one same-source segment with
  byte-identical bounds (no known producer path does — VAD segmentation
  yields disjoint ranges per source, and job idempotency excludes retry
  duplicates per STT-§3.3), the renderer surfaces them as additional rows in
  the group rather than collapsing text: the renderer never merges or drops
  persisted segments to make the display fit its model. The group is the
  unit the windower measures and positions; inside it the rows render side
  by side with their source labels.
- Near-overlaps (timestamps not exactly equal) simply interleave
  chronologically. No fuzzy grouping, no timeline clustering — a heuristic
  overlap detector is rejected as complexity without a plan anchor (§8).

Track groups are computed in the store selector, not in the component, so
the windowed list stays a flat measured list.

---

## 3. Windowed rendering model (gate G1)

### 3.1 Constants (testable, UI constants not spike-derived)

| Constant | Value | Meaning |
| --- | --- | --- |
| `TR_PAGE_SIZE` | 250 | segments per paged fetch (store action, FSC-§2.2 row 4 query) |
| `TR_MAX_CACHED_PAGES` | 8 | LRU cap on loaded pages per meeting ⇒ ≤ 2000 segments resident |
| `TR_OVERSCAN_ROWS` | 10 | rows rendered beyond each viewport edge |
| `TR_MIN_ROW_PX` | 28 | lower bound of any row/track-group height (timestamp line + text line at minimum font size) |
| `TR_TAIL_ANCHOR_SLACK_PX` | 48 | distance-from-bottom below which the view counts as "anchored to tail" (§4.1) |
| `TR_MAX_VIEWPORT_PX` | 2000 | viewport height assumed for the fixed DOM budget (§3.2); taller viewports re-derive by the same formula |
| `TR_MAX_NODES_PER_ROW` | 8 | upper bound of DOM nodes per row (container, timestamp button, source badge, text spans, icon) |

### 3.2 The DOM bound (G1 statement)

The windowed list mounts only rows whose measured range intersects
`[scrollTop − overscan, scrollTop + viewportHeight + overscan]`. Therefore:

```text
mounted rows     ≤ ⌈TR_MAX_VIEWPORT_PX / TR_MIN_ROW_PX⌉ + 2 × TR_OVERSCAN_ROWS
                 = ⌈2000 / 28⌉ + 20 = 72 + 20 = 92

mounted DOM nodes ≤ 92 × TR_MAX_NODES_PER_ROW + chrome
                 = 736 + ~16 = ≤ 752
```

**Gate G1: the transcript renders at most ~752 DOM nodes — a constant that
depends on the viewport height only, never on transcript length.** A
4-hour session with 5,760 persisted segments (§5.1 worst case) and a
20-hour one produce byte-identical DOM budgets. The windower must be a
measured-window implementation (rows measured as they mount, positions
derived from a prefix of measurements); both a hand-rolled window and a
single virtualization utility satisfy this — two coexisting windowing
implementations do not (rule R1 below).

### 3.3 Structural rules

- **R1 — One windowing implementation.** Exactly one module family under
  `src/components/meetings/transcript/` (list window, row, anchor controller)
  is consumed by F7 and F9. Duplicating windowing logic per surface is a
  defect: two scroll/anchor code paths are how the plan's no-auto-jump rule
  (line 627) gets violated in one view while passing tests in the other.
  The child paths under `transcript/` follow the canonical-path discipline of
  the module-placement-map; registering them there is a hand-off to that
  document's maintainer (same posture as LV-§8 NG-9).
- **R2 — The windower reads store state through selectors only.** No segment
  data is copied into component state or refs (FSC-B6, §2.3 selector
  discipline). Scroll state (`scrollTop`, anchor mode) is view-local
  `useRef`/local state and is **never** written to any store (SSC-§5 forbids
  persisted UI-session state; FSC-B4 forbids store persistence — a scroll
  position has no restart meaning).
- **R3 — Append-only structure.** The ordered row list is maintained as an
  append-dominant structure: applying an E1 batch inserts by the §2.2 tuple
  and never re-sorts, re-keys, or re-creates existing rows. With the default
  local provider this is a pure tail append (STT-§8: mic/system segments are
  serialized by timeline and fairness); cloud providers may complete out of
  timeline order, which §4.3 classifies instead of special-casing.
- **R4 — No unbounded list anywhere.** No surface may render the full
  segment list "because it is simpler" — export/copy paths (plan §6.13)
  serialize from the store/repository, not from the DOM.

---

## 4. Live-append update path and scroll anchoring (gate G2)

### 4.1 The path, end to end

1. E1 `meeting://transcript-delta` arrives; the S1 registrar normalizes the
   payload and calls the store action `applyTranscriptDelta(batch)`
   (FSC-L1/L7 — the component never listens, FSC-L2).
2. The action applies the D3 guard (LV-§4 D3): `batch.last_sequence` ≤
   highest applied sequence ⇒ discard, no state change.
3. The batch's segments are inserted into the page structures by the §2.2
   tuple; the per-meeting list version counter bumps once per applied batch
   (FSC-§4.2 invalidation counters).
4. Sequence-gap or `truncated` handling schedules the debounced tail-page
   refetch of LV-§6.3 — the renderer adds no healing logic of its own.
5. Selectors re-derive the visible window; the windower mounts/unmounts rows
   per §3; the anchor controller of §4.2/§4.3 decides whether any scroll
   adjustment happens.

The consumer retains nothing from the batch after step 3: no event log, no
pending queue, no raw payload cache (§5.3). "Append-only" describes the
update path — new segments enter, nothing is rewritten — not a retained
append-log in memory.

### 4.2 Anchor rules (G2 statement)

The anchor controller classifies the view **before** applying any layout
change:

```text
anchoredToTail ⇔ scrollTop + clientHeight ≥ scrollHeight − TR_TAIL_ANCHOR_SLACK_PX
```

- **A1 — Anchored to tail: follow.** While `anchoredToTail`, each applied
  batch is followed by a scroll to the new bottom in the same frame —
  instant (no smooth scrolling) when `prefers-reduced-motion` is set, a
  short ease otherwise (plan §6.14 "reduzierte Bewegung respektieren").
  This is the live view's default state: the user watching the transcript
  grow sees "zuletzt fertiggestellte Transkriptsegmente" (plan line 624).
- **A2 — Detached: never touch scroll.** Once the user scrolls up beyond
  `TR_TAIL_ANCHOR_SLACK_PX`, applying a batch **must not** call
  `scrollTo`, `scrollIntoView`, or mutate `scrollTop`. New segments
  accumulate below the fold; the `Zum aktuellen Text` button appears
  (plan line 628). **Rationale:** plan line 627 forbids the auto-jump
  outright, and for a reason that survives contact with users — a
  programmatic jump discards the reader's position *twice* (the viewport
  moves and the eye must re-find the line it was reading), and during a
  multi-hour meeting the tail grows for hours; losing one's place to every
  finished segment makes reading the transcript impossible. The slack of
  48 px absorbs sub-pixel rounding and bottom-padding so that a user who
  has scrolled to the bottom but not "past" it is not punished with a
  permanently detached view.
- **A3 — The button is the only re-anchor.** `Zum aktuellen Text` performs
  the jump to the tail and re-enters A1 mode. Keyboard activation moves
  focus to the tail region (`tabindex="-1"` target, per the activation
  pattern of evidence-navigation-contract §"Aktivierungssequenz"), so the
  jump is announced and reversible, not a silent viewport teleport.
- **A4 — Insertions above the viewport: compensate.** Two events insert
  rows *above* the current top edge: loading an older history page
  (scroll-up pagination) and an out-of-timeline-order cloud completion
  (§4.3). In both cases the controller measures the inserted block height
  `ΔH` and applies `scrollTop += ΔH` in the same frame, so the content
  under the reader's eye does not move. This is the standard
  scroll-anchoring compensation; it exists here as an explicit rule
  because browsers' native CSS scroll anchoring is unreliable inside
  virtualized lists (the anchored nodes unmount during the same mutation).
- **A5 — Insertions inside the viewport: allowed, logged as rare.** An
  out-of-order segment landing inside the visible window shifts layout
  minimally; no compensation is attempted (compensating mid-viewport
  insertions requires guessing which edge the reader is attached to and is
  wrong half the time). Chronological truth (plan line 1065, P2C-§6 rule 5)
  outranks a momentary shift; §4.3 bounds how often this can happen.
- **A6 — Row heights never change retroactively.** Because rendered rows are
  immutable (§2.3), no compensation is needed for height changes of already
  positioned rows. A user font-size setting change re-measures the window
  and re-anchors to the nearest segment `id` (R-K2: the anchor token is the
  UUID, so the re-anchor survives re-layout).

### 4.3 Arrival-order cases (why A2 is not naive)

With the local provider, completion is timeline-serialized
(STT-§8 "Mic- und Systemsegmente werden nach Timeline und Fairness
serialisiert"), so every applied batch is a pure tail append — case I1.

| Case | Where the tuple lands | Frequency (default local path) | Rule applied |
| --- | --- | --- | --- |
| I1 | at/after the absolute tail | every batch | A1 or A2 |
| I2 | below the viewport, above the tail | rare (multi-segment batches) | A2 (no scroll action) |
| I3 | above the viewport top | cloud providers only | A4 compensation |
| I4 | inside the viewport | cloud providers only | A5 |

Cases I3/I4 require a provider that completes jobs out of timeline order
(bounded by JSC-§1 concurrency classes); they cannot occur in the
timeline-serialized local path. The renderer handles them with the same two
mechanisms as everything else (tuple insert + compensation) — no per-provider
branching exists in the component.

---

## 5. Memory ceiling for a 4-hour session (gate G1 companion)

### 5.1 Worst-case population (computed)

Segment sizes are bounded by P2C-§6 rule 6 (VAD segments 5–30 s starting
point). For a 4-hour session with both sources:

```text
theoretical worst:  2 sources × 4 h × 3600 s ÷ 5 s/segment = 5,760 segments
realistic:          ~20 s average segment, both sources    ≈ 1,400 segments
```

The renderer's bound is **independent of both numbers**: the webview never
holds all persisted segments.

### 5.2 Resident-state bound

| Consumer | Bound | Computation |
| --- | --- | --- |
| Loaded pages (store slice) | ≤ `TR_MAX_CACHED_PAGES × TR_PAGE_SIZE` = 2,000 segments | LRU over pages; the page intersecting the viewport and the anchor row's page are pinned, all others evictable (§5.4) |
| Per-segment VM cost | ≤ ~1 KiB conservative (36-char UUID string, two timestamps, source enum, ~300-char text as JS string, object + Map overhead) | 2,000 × 1 KiB ≈ **2.0 MiB** |
| Event batches | applied and discarded (§4.1 step 5); upstream producer buffer already bounded by `LV_DELTA_PENDING_CAP` = 1024 segments (LV-§3.1) | no consumer-side queue exists ⇒ **0** steady-state |
| Mounted DOM/window | ≤ 752 nodes (§3.2) | constant, viewport-bound |

**Ceiling statement: the transcript slice stays under a 4 MiB steady-state
ceiling in the webview regardless of session length — the 2.0 MiB computed
worst case leaves ≥ 2× headroom for transient measurement and selector
garbage.** A 4-hour session at the 5,760-segment worst case exceeds the
page cache by design (23 pages vs. 8); the LRU keeps the resident set at the
cap while any visible region remains instantly loadable by one page query.

### 5.3 Forbidden accumulations

- No raw E1 payload log ("we might need it later") — healing is query-based
  (LV-§6.3), not replay-based (LV-§8 NG-5).
- No per-batch derived arrays retained across renders; selectors memoize per
  list version only.
- No second copy of segment text (e.g., a search-highlight copy) — highlight
  ranges are computed at render time for visible rows only.

### 5.4 Eviction rules

- Eviction is page-granular and LRU, executed inside store actions after a
  page fetch or anchor move — never during render.
- Pinned: the page(s) intersecting the current viewport, the page containing
  the anchor segment (A6), and — in the live view — the tail page (so the
  follow mode of A1 never evicts what it scrolls to).
- Evicting a page the user is scrolling toward is harmless: the windower
  requests it via the same store action path (FSC-C1) and renders a
  fixed-height placeholder until it resolves; no layout shift occurs because
  placeholders have measured, stable heights.

---

## 6. Reload, resubscribe, and deep jumps (consumed, not redefined)

- The LV-§6.2 handshake is the only initialization path: listeners first,
  snapshot (`transcript_head_sequence`), **one tail page**, then live deltas
  under the D3 guard. This contract adds: after the handshake, the live view
  initializes anchored to tail (A1) and the detail view at the top page —
  both without any additional backend round-trip.
- LV-§6.3 healing (sequence-gap debounce refetch, `truncated` immediate
  refetch, terminal-count refetch) is consumed verbatim; the renderer's only
  obligation is that a refetch replaces pages wholesale under stable keys
  (R-K1), so reconciliation cannot duplicate or drop rows.
- Deep jumps from evidence navigation and search hits
  (evidence-navigation-contract click surfaces) resolve to a segment `id`
  (R-K2), fetch its page through the store action, and scroll it into view
  with the A4/A6 machinery — focus lands on the target row per that
  contract's activation sequence. The renderer exposes no second jump
  mechanism.

---

## 7. Accessibility posture of the windowed list

Brief by design — plan §6.14 and the a11y baseline govern; these are the
windowing-specific obligations:

- The list is a single logical region (`role="log"` with a label in the live
  view; `role="list"` in the detail tab) so screen readers see one
  transcript, not a mutating window of it.
- New segments while **detached** are not announced per row; the
  `Zum aktuellen Text` button carries a count ("3 neue Abschnitte") updated
  via a polite live region — restrained announcement per plan line 689, and
  it prevents a 4-hour session from producing thousands of utterances.
- While **anchored** (A1), the live region announces appended text politely
  at batch granularity (one announcement per applied batch, not per row).
- Unmounted off-screen rows are unreachable by keyboard precisely because
  they are unmounted; `Tab` order within the mounted window is DOM order and
  therefore visual order (§2.2 tuple) — no `tabindex` surgery, no roving
  tabindex beyond the timestamp buttons' natural order.
- Timestamps are buttons (audio-position jump, plan line 673) with accessible
  names built from timeline position and source label, never from the UUID.

---

## 8. Complexity budget / non-goals (gate G4)

In line with the company complexity budget this contract deliberately does
**not** specify:

- **NG-1 — No virtualization framework requirement and no second windowing
  implementation.** One measured-window module family (R1); whether it is
  hand-rolled or a single utility is the implementer's choice, but two
  coexisting implementations (or a framework plus a special case for the
  live view) are rejected.
- **NG-2 — No CSS-only substitute (`content-visibility`, overscroll
  hacks).** They reduce paint cost but not mounted-node count, so they fail
  gate G1's constant-DOM statement.
- **NG-3 — No persisted scroll position.** Restart rebuilds from snapshot +
  tail page (LV-§6.2); a restored pixel offset against a rebuilt page
  layout is false precision (SSC-§5, FSC-B4).
- **NG-4 — No live edit/update/delete rendering.** Re-transcription and
  edits belong to the detail view's envelope/refetch machinery (LV-§8 NG-4);
  this contract's immutability rule (§2.3) is what keeps anchoring sound.
- **NG-5 — No fuzzy overlap grouping or timeline clustering.** Exact-tie
  track groups (§2.4) only; anything else invents display semantics without
  a plan anchor.
- **NG-6 — No per-row virtualization of the §6.12 checklist, status bar, or
  level meters.** They are bounded-size surfaces; windowing applies to
  segments only.
- **NG-7 — No renderer-side ordering, filtering, or search logic.** The
  tuple comes from P2C-§6 rule 5, search from the search-index contract —
  the frontend never re-implements backend queries (FSC-§4.2).
- **NG-8 — No configurable constants.** The §3.1 values are fixed UI
  constants (LV-§8 NG-7 posture); user-tunable page sizes or overscan are
  rejected as scope bloat.
- **NG-9 — No adaptive/measured memory governor.** The §5 ceiling is
  structural (page cap × row cost), not a runtime feedback loop.

Anything not in §2–§7 is a non-goal until the plan changes.

---

## Appendix A — Gate traceability

| Gate | Coverage |
| --- | --- |
| G1 DOM bound constant regardless of transcript length | §3.1 constants, §3.2 formula (92 rows ⇒ ≤ ~752 nodes, viewport-bound only), R4 no-unbounded-list rule; §5 shows the same structural bound holds for the 5,760-segment 4-hour worst case |
| G2 live-append never resets scroll when anchored above the tail | §4.2 A2 (never touch scroll when detached) with the measurable `anchoredToTail` definition and explicit rationale grounded in plan line 627; A4/A6 compensation for above-viewport insertions; §4.3 case table bounding when each rule fires |
| G3 segment identity/key scheme matches the shared segment model | §2.1 R-K1/R-K2 (persisted UUID verbatim, no second format, no index/rowid/composite keys), §2.2 ordering tuple consumed unchanged from P2C-§6 rule 5, dependency note on LV-D2 sequence semantics registered |
| G4 non-goals section; documentation index updated | §8 NG-1–NG-9; row appended to `docs/meeting-mode-documentation-index.md` §3 table and reading-order entry per its Pflegeregel |
| G5 git status confined to declared scope | verified after authoring via `git status --porcelain`; recorded in the mission result evidence |

## Appendix B — Testability (mechanical checks once the FSC §0.2 item 4 harness exists)

| Rule | Test shape |
| --- | --- |
| G1 DOM bound | mount the window with 5,760 synthetic segments; assert `container.querySelectorAll("[data-segment-id]").length` ≤ 92 across scroll positions swept top→bottom |
| R-K1/R-K3 keys | apply a re-delivered E1 batch (D3 says discard) and an older-page prepend; assert zero duplicate-key warnings and stable row identity (`id` preserved across reconciliation) |
| A2 detached no-jump | render live view, scroll up, apply 50 batches; assert `scrollTop` unchanged and the `Zum aktuellen Text` button visible with a correct count |
| A1 follow | assert bottom-follow after each batch while `anchoredToTail`, instant under `prefers-reduced-motion` |
| A4 prepend compensation | load an older page while scrolled; assert the first visible segment `id` is unchanged and `scrollTop` increased by the measured `ΔH` |
| §5.2 page cap | fetch pages while scrolling through 23 synthetic pages; assert store slice never exceeds 2,000 segments and the viewport/tail pages stay pinned |
