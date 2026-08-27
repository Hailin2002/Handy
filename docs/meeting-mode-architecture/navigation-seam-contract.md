# Navigation Seam Contract — Route Model, AppShell Window Seam & PrimaryNavigation Session Consumption

Status: **spec contract — no implementation yet** (Phase 3 shell prerequisite;
changes no source code)

Source of truth: `docs/meeting-mode-plan.md` — §6.3 new information
architecture (plan lines 400–435), §6.11 running meeting view (plan lines
613–637), §6.13 meeting detail view (plan lines 659–681), §6.14 accessibility
("während Aufnahme und Verarbeitung gehen keine Aktionen durch Navigation
verloren", plan line 695), §6.16 projects page (plan lines 724–748), §6.17
project detail (plan lines 750–782), §6.19 WhatsApp import wizard (plan lines
800–820), §6.20 window and responsive logic (plan lines 837–852), §6.21 gate 8
("dasselbe Projekt in kleinem Fenster und maximiert bedienen", plan line 865),
§6.2 principle 9 ("Aufnahmezustand ist überall konsistent", plan lines
394–397).

This document fixes the **navigation seam**: the route model for the
three-page IA (Diktieren / Meetings / Projekte plus detail views), where
navigation state lives and what survives a restart, the window seam for
resize/min-size/responsive collapse points, whether (and how) a running
meeting restricts navigation, and how `PrimaryNavigation` consumes live
session state without polling. It defines no backend behavior beyond one
proposed event emission at existing call sites, no schema, and no visual
design.

Sibling contracts consumed by this document (read-only):

- [frontend-state-contract.md](frontend-state-contract.md) — abbreviated
  **FSC-§n**. Its store inventory S1–S5 (§2.1), listener-ownership rules L1/L2
  (§3.1), selector discipline (§2.3), high-frequency-telemetry rule B5, and
  no-persisted-stores rule B4 are the foundation of §2, §5 and §6 below.
- [session-state-contract.md](session-state-contract.md) — abbreviated
  **SSC-§n**. Its publication guarantees O1–O7 (§4) and the UI rebuild rule
  (§5: "the UI is a pure function of persistence plus events") are why the
  navigation layer needs neither polling nor its own session truth (§6).
- [module-placement-map.md](module-placement-map.md) — frontend component
  placements F1–F22 and rule F1 (canonical import paths); route targets below
  reference those components exclusively.
- [coexistence-contract.md](coexistence-contract.md) — start mutual exclusion
  between dictation and meeting; §5 below defers action-level gating to it.
- [shutdown-dataflow.md](shutdown-dataflow.md) — close-during-recording
  teardown variants T/P; §5 rule N4 restates that navigation adds nothing to
  that sequence.
- [identifier-registry.md](identifier-registry.md) — the one event topic
  proposed in §4 is a **proposal** until registered there.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 0. Scope, non-goals

### 0.1 In scope

1. The typed route model covering every plan §6 page and detail view, each
   with its required stores (gate G1).
2. Ownership, lifetime, and restart persistence of navigation state.
3. The deep-open seam: tray click during recording, notification open,
   single-instance reopen, macOS Reopen — each with a defined landing route
   (gate G3).
4. The ruling on meeting-running state vs. navigation restriction.
5. The no-polling consumption seam for `PrimaryNavigation`'s pinned
   active-meeting entry.
6. Window resize/min-size behavior and responsive collapse points per plan
   §6.20, using only an enumerated, closed set of window primitives
   (gate G2).

### 0.2 Explicitly out of scope

1. **Visual/interaction design** of F1–F22 (colors, spacing, copy) — owned by
   the UX workstream and the UX copy catalog; only the structural seams are
   fixed here.
2. **Store internals** — owned by FSC; this contract only names which stores a
   route requires and which selectors `PrimaryNavigation` may read.
3. **Window teardown/quit sequencing** — owned by shutdown-dataflow; §5 N4
   merely states that navigation participates in none of it.
4. **The identifier registry entry** for the topic proposed in §4.2 — outside
   this task's declared file scope; registration is a hand-off (§4.5).
5. **Backend command/event payload definitions** — SSC/RBC territory; §4
   defines only the single thin payload the navigation seam needs.

---

## 1. Position in the architecture and verified baseline

The main webview already has a two-group product sidebar and a content router
(plan §6.3 shape). This contract formalizes that existing skeleton instead of
replacing it. Baseline observations at authoring time (verified code, HEAD):

| Observation                                                                                                                                                                 | Location                                                                                                                                 | Consequence                                                                                                                                                                |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Content routing is a `switch` over a `ProductViewId` union (`"dictate" \| "meetings" \| "projects" \| "history" \| "settings" \| "about"`); current view held in `useState` | `src/App.tsx` lines 40–65 (`renderProductContent`), line 76 (`useState<ProductViewId>`)                                                  | §2 keeps exactly these six top-level views and adds typed children — no router library, no history API.                                                                    |
| Sidebar is a fixed-width column (`w-40`) with `PRIMARY_NAV`/`SECONDARY_NAV` entries from a static table                                                                     | `src/components/Sidebar.tsx` lines 60–72; `src/components/shell/productNav.ts` lines 43–54                                               | Top-level view identity alone determines nav highlighting; child routes never add or remove nav entries (§2.4 R6).                                                         |
| Pinned `Aufnahme läuft · mm:ss` slot exists below "Meetings", double-gated on a build flag and a non-null `ActiveMeetingInfo`; today always null                            | `src/components/shell/ActiveMeetingNavItem.tsx` lines 24–31; `src/components/shell/productNav.ts` lines 62–63; `Sidebar.tsx` lines 47–57 | §6 defines the single sanctioned source of `ActiveMeetingInfo` once meeting mode lands (meetingStore selector + rAF clock), replacing the prop drilling with a store read. |
| Main window is created programmatically: 680 × 570 default **and** minimum size, resizable, maximizable, initially hidden                                                   | `src-tauri/src/lib.rs` lines 493–500                                                                                                     | §7 fixes 680 × 570 as the v1 operating floor; all IA screens must be fully operable there (plan line 865).                                                                 |
| Window close is intercepted and hides the window (app stays alive in tray)                                                                                                  | `src-tauri/src/lib.rs` lines 585–606                                                                                                     | Closing ≠ exiting; navigation state survives a hide untouched (§3.2).                                                                                                      |
| Tray left-click/double-click calls `show_main_window` — unminimize/show/focus only, **no navigation signal of any kind reaches the webview**                                | `src-tauri/src/tray.rs` lines 376–387; `src-tauri/src/utils.rs` lines 12–21                                                              | Today a tray click cannot land the user anywhere specific; §4 defines the one seam that changes this without new windows or plugins.                                       |
| Tray menu items (`settings`, `check_updates`, …) likewise only show the main window                                                                                         | `src-tauri/src/tray.rs` lines 402–417                                                                                                    | §4's emitter enumeration covers these call sites.                                                                                                                          |
| Second launch / macOS dock Reopen also just show the main window                                                                                                            | `src-tauri/src/lib.rs` lines 424, 617–621                                                                                                | Defined as no-navigation deep-open cases (§4.3 D5/D6).                                                                                                                     |

Layer position: navigation state sits entirely inside the React surface layer
(FSC §1). It holds **no domain authority**: losing it must never lose data,
and every fact it displays about meetings comes from `meetingStore`
projections of backend truth (SSC §5).

---

## 2. Route model (gate G1)

### 2.1 Route type

A route is a plain value, not a URL:

```ts
type Route =
  | { view: "dictate" } // F3/F4 home (plan §6.4)
  | { view: "meetings"; child?: MeetingsChild } // plan §6.5/§6.11–§6.13
  | { view: "projects"; child?: ProjectsChild } // plan §6.16/§6.17/§6.19
  | { view: "history" } // Diktatverlauf (plan §6.3)
  | { view: "settings"; section?: SettingsSectionId } // Einstellungen (plan §6.3)
  | { view: "about" }; // Hilfe & Über (plan §6.3)

// Child routes inside the Meetings page. Absent child = list (default).
type MeetingsChild =
  | { kind: "live" } // F7 running-meeting view (plan §6.11)
  | { kind: "detail"; meetingId: Uuid } // F9 detail; tab is local view state (plan §6.13)
  | { kind: "processing"; meetingId: Uuid }; // F8 post-stop checklist (plan §6.12)

// Child routes inside the Projects page. Absent child = home (default).
type ProjectsChild =
  | { kind: "inbox" } // F18 „Nicht zugeordnet“ (plan §6.16/§6.18)
  | { kind: "detail"; projectId: Uuid }; // F14; tabs Übersicht/Aufgaben/Meetings/
//    Quellen are local view state (plan §6.17)
```

Design rulings:

- **R1 — Six top-level views, forever.** The top-level discriminant is
  byte-compatible with the existing `ProductViewId` (`App.tsx` line 76,
  `productNav.ts` lines 28–34). Detail views are _children_ rendered inside
  their parent page's content area. The primary navigation therefore never
  grows a second-order menu, and nav highlighting is a pure function of
  `route.view` (R6).
- **R2 — Ids are UUIDs, never indices or paths** (FSC B3): `meetingId` /
  `projectId` are the stable identifiers minted by the backend (RBC-§5.1). A
  route referencing an id the corresponding store cannot resolve falls back
  per §3.4 F3.
- **R3 — Tabs are not routes.** Meeting-detail tabs (`Übersicht`/`Transkript`/
  `Audio`/`Verarbeitung`, optional `Geteilte Ansicht`; plan §6.13) and project
  tabs (`Übersicht`/`Aufgaben`/`Meetings`/`Quellen`; plan §6.17) are local
  component state inside F9/F14. They are never deep-linkable and never
  persisted. Rationale: tab position carries zero user intent worth restoring;
  persisting it would violate FSC B4's spirit while adding settings keys for
  no user value.
- **R4 — Dialogs are not routes.** The following are dialogs/modal flows
  hosted on a route, unreachable by navigation and excluded from the route
  table by design:
  - Meeting setup dialog F5 and compact start dialog (plan §6.6, §6.7) →
    hosted on `{ view: "meetings" }`;
  - Project create dialog F13 (plan §6.16) → hosted on
    `{ view: "projects" }` and `{ view: "projects", child: inbox }`;
  - WhatsApp import wizard F19 (plan §6.19, four-step dialog) → hosted on
    `{ view: "projects" }`. Losing the wizard by navigating away is safe: the
    import pipeline owns progress durably (`import://progress` aggregates,
    FSC §2.2 row 15), and re-opening the wizard shows resumable state.
  - `Zusammenfassung neu erstellen` dialog (plan §6.13) → hosted on
    `{ view: "meetings", child: detail }`.
- **R5 — Exactly one canonical definition site.** The `Route` type, child
  types, and the reducer of §3 live in `src/components/shell/AppShell.tsx`
  (F1's canonical path, module-placement-map line 146). `App.tsx` stops owning
  `currentView` when F1 absorbs the frame; until then `App.tsx`'s
  `useState<ProductViewId>` is the provisional carrier and must not gain
  params ad hoc.
- **R6 — Highlighting rule.** `NavItem.active === (entry.id === route.view)`
  for every entry of `PRIMARY_NAV`/`SECONDARY_NAV`; the pinned
  active-meeting entry highlights only when
  `route.view === "meetings" && route.child?.kind === "live"`.

### 2.2 Route table — every plan §6 page/detail view (gate G1)

Required stores use the FSC §2.1 inventory (S1 `meetingStore`,
S2 `projectStore`, S3 `dictationStore`, S4 `settingsStore`, S5 `modelStore`).
"Snapshot" names the await-authoritative fetch the route issues on mount via
its store actions (FSC C1/L6 handshake; components never call commands).

| Plan source                         | Route                                             | Component(s)                                            | Required stores | Mount snapshot / notes                                                                                                                            |
| ----------------------------------- | ------------------------------------------------- | ------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| §6.4 Diktieren-Seite                | `{ dictate }`                                     | F3 `DictationHome`, F4 `DictationStatusCard`            | S3, S4, S5      | dictation runtime status card model; no meeting actions exist here (plan lines 451–452)                                                           |
| §6.5 Meetings-Seite (list)          | `{ meetings }` (no child)                         | F10 `MeetingList`                                       | S1              | meeting-list page refetch on `listVersion.meetings` bump (FSC §4.2); hosts F5/F6 dialogs                                                          |
| §6.11 Laufende Meetingansicht       | `{ meetings, child: { live } }`                   | F7 `MeetingRecorderView`                                | S1              | `get_meeting_snapshot` under the O5 handshake; levels/duration bypass stores (FSC B5, §6 below)                                                   |
| §6.12 Nach dem Stoppen              | `{ meetings, child: { processing, meetingId } }`  | F8 `MeetingProcessingView`                              | S1              | checklist model from job aggregates; terminal counts always refetched (FSC §4.3)                                                                  |
| §6.13 Meeting-Detailansicht         | `{ meetings, child: { detail, meetingId } }`      | F9 `MeetingDetail` (+ transcript/audio/processing tabs) | S1              | session + summary + paged segments snapshots; tabs local (R3); split view opt-in (plan lines 677–681)                                             |
| §6.16 Projekte-Seite                | `{ projects }` (no child)                         | F12 `ProjectsHome`                                      | S2              | project list pages; search via S2 cache (FSC row 17); hosts F13                                                                                   |
| §6.16/§6.18 Nicht zugeordnet        | `{ projects, child: { inbox } }`                  | F18 `UnassignedInbox`                                   | S2              | unassigned meetings/import queue; batch assignment preview rules per plan lines 786–798                                                           |
| §6.17 Projekt-Detailansicht         | `{ projects, child: { detail, projectId } }`      | F14 shell + F15/F16/F17 tabs                            | S2              | current-snapshot refetch (FSC row 14); task pages; tabs local (R3)                                                                                |
| §6.19 WhatsApp-Importassistent      | dialog over `{ projects }` — **not a route** (R4) | F19 `WhatsAppImportWizard`                              | S2              | progress via `import://progress` aggregates; durable across navigation loss                                                                       |
| §6.3 Diktatverlauf                  | `{ history }`                                     | existing `HistorySettings` content, later F3-adjacent   | S3              | legacy history projection (FSC N4 carve-out)                                                                                                      |
| §6.3 Einstellungen + sekundäre Tabs | `{ settings, section? }`                          | `SettingsPage`, sections per `settingsSections.ts`      | S4, S5          | section ids are the existing `SettingsSectionId` keys; debug section visibility unchanged (plan line 435); F20/F21/F22 land inside their sections |
| §6.3 Hilfe & Über                   | `{ about }`                                       | existing `AboutSettings`                                | S4              | update-checker state as today                                                                                                                     |

Coverage proof (G1): every page/detail view named in plan §6.3–§6.19 appears
exactly once above; the three dialogs of R4 are explicitly routed to their
host routes; no route exists without a plan anchor. The table is total over
plan §6's navigable surfaces.

### 2.3 Store-required invariant

A component rendering under a route must obtain all domain facts from the
route's required stores — never by importing a second store for the same
entity (FSC B1/B2 apply unchanged). The route table is documentation of that
dependency, not a permission slip: adding a store to a route requires the
entity to appear in exactly one store per FSC §2.2.

### 2.4 Navigation transition rules

- **N-R1:** Navigation is synchronous, in-memory, and infallible: applying a
  route updates the reducer and renders. No route change awaits I/O. Slow
  data arrives through the store snapshot handshake; the route renders its
  honest loading state meanwhile (plan §6.2 principle 5).
- **N-R2:** No confirmation dialog ever guards pure navigation (plan line
  695). Destructive actions (`Meeting beenden`, deletes) confirm at their own
  buttons, never on `NavItem` clicks.
- **N-R3:** Unknown child kinds / malformed routes are impossible by typing;
  defensively, the reducer normalizes them to the parent view with absent
  child (same fallback semantics as §3.4 F3).
- **N-R4:** The back concept for stacked layers at narrow widths (§7.3) is a
  view affordance that sets `child = undefined`; it introduces no history
  stack.

---

## 3. Navigation-state ownership and persistence across restarts

### 3.1 Ownership

Navigation state = the current `Route` value plus nothing else. It lives in
F1 `AppShell` (`useReducer` over the §2.1 type). It is deliberately **not**
a Zustand store: FSC §2.1 fixes the store inventory at five, navigation has no
domain entities, and no other component may mutate the route behind the
shell's back. Components request navigation through the shell's single
`navigate(route)` callback (props/context), which is the only dispatcher into
the reducer. `PrimaryNavigation` (F2) receives it exactly as `Sidebar`
receives `onViewChange` today (`Sidebar.tsx` lines 24–29).

### 3.2 What persists across restarts — allowlist (closed)

Restart persistence rides the **existing app-settings mechanism**
(`tauri-plugin-store` via `settings.rs`/`settingsStore`), which is already
per-device (local profile, portable-mode aware). The allowlist is exhaustive:

| Settings key   | Type                                                   | Restored                                       | Never contains                   |
| -------------- | ------------------------------------------------------ | ---------------------------------------------- | -------------------------------- |
| `ui.last_view` | one of the six top-level view ids, default `"dictate"` | at boot, after onboarding gate                 | any child route, any id, any tab |
| `window_state` | `{ width, height, x?, y?, maximized }` (§7.5)          | at window creation in `lib.rs` before `show()` | workspace-specific anything else |

Everything else — child routes, detail ids, tabs, scroll positions, wizard
steps, expanded list filters — is intentionally lost on restart. Justification:
plan line 1577 requires UI ≡ persisted state after restart; that equality is
achieved by rebuilding from snapshots (SSC §5), and the only navigation facts
with cross-restart user intent are "which tool was I in" and "how big was my
window". Persisting more would be complexity-budget bloat (§8).

Reconciliation with sibling contracts: SSC §5.1 forbids persisted
_UI-session_ state and FSC B4 forbids persisted _stores_. The keys above are
app preferences in `AppSettings` — the same class as the existing
`start_hidden` flag — written by the settings mechanism, never by a Zustand
store, and never derived from session/job state. A `ui.last_view` value is
invalidated (reset to `"dictate"`) if the settings schema rejects it.

### 3.3 Restart landing procedure

Order matters; each step can override the previous:

1. Window is created hidden; `window_state` applied (if present and sane —
   clamp to the monitor work area, discard off-screen positions).
2. Onboarding gate runs exactly as today (`App.tsx` `checkOnboardingStatus`);
   onboarding steps render before any route restore.
3. `ui.last_view` restored into the reducer.
4. Startup reconciliation result applies its override (§3.4 O1).
5. Deep-open events received during boot queue and apply last (§4.4).

### 3.4 Overrides and fallbacks

- **O1 — Recovery overrides persistence.** If startup reconciliation reports
  recovered sessions (SSC-§6, F11 banner), boot lands on
  `{ view: "meetings" }` (list) regardless of `ui.last_view`, with the
  recovery banner visible. It never auto-routes into `{ live }`: after a
  process restart nothing is capturing anymore, so the live view would lie
  about a recording in progress (plan §6.2 principle 5, "Fortschritt ist
  ehrlich").
- **F1 — Missing/corrupt key** → `"dictate"` (the proven home page).
- **F2 — Flag-off meeting mode** → meeting children (`live`, `detail`,
  `processing`) are unreachable (no producer sets them); the same
  `MEETING_MODE_ENABLED` gate
  that hides the pinned nav item (`productNav.ts` lines 62–63) gates the
  reducer's acceptance of meeting children.
- **F3 — Unresolvable id.** A deep-open route whose `meetingId`/`projectId`
  has no entity in the store after its mount snapshot resolves → normalize to
  the parent view with absent child and let the list's empty/error state
  speak (never a blank screen, never an error toast for a stale link).

---

## 4. Deep-open seam (gate G3)

### 4.1 Principle

Exactly one path exists from "something outside the webview wants the main
window to open somewhere specific" to the route reducer:

```text
backend emitter (closed enumeration, §4.3)
  → utils::show_main_window(app, route_hint?)     [existing show/unminimize/focus]
  → emit "main-window-navigate" { route }         [single proposed topic, §4.2]
  → AppShell bootstrap listener (FSC L2.1 window-lifecycle exception)
  → navigate() reducer dispatch                    [same door as nav clicks]
```

No second mechanism (no CLI-style arg channel into the webview, no query
params on the window URL, no global shortcut handler reaching into React, no
per-emitter bespoke events) may navigate. This is what makes G3 provable by
enumeration rather than by review.

### 4.2 The proposed topic

- Name: `main-window-navigate` (plain window-scoped name, matching the
  existing `show-overlay`/`theme-changed` family of FSC §3.2, not a
  domain namespace — it carries no domain data, only a route value).
- Payload: `{ route: Route | null }` — `null` means "show only, keep the
  current route".
- Owner: the main window's entry module (`src/main.tsx`, mirroring the
  `src/overlay/main.tsx` pattern), registered once at boot under FSC rule L2
  exception 1 (topics that exist solely for one webview window's lifecycle)
  and handed to F1 before first render. Components never listen (L2 holds).
- Status: **proposal** pending registration in `identifier-registry.md`
  (hand-off §4.5; registry outside this task's declared edit scope, mirroring
  how FSC §3.2 handled `project://entity-changed`).
- Hygiene: the payload is a typed route value only — ids and enum literals
  (SSC O7 hygiene analog). No titles, transcripts, or provider data ever ride
  this topic; the receiving store fetches everything itself.

### 4.3 Emitter enumeration (closed) with landing routes

| #   | Trigger (existing code site)                                                                                 | Route hint                                       | Landing                                                                                                                                                          |
| --- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Tray icon left-click / double-click **while a session is `recording` or `paused`** (`tray.rs` lines 376–387) | `{ meetings, child: { live } }`                  | Running-meeting view with fixed status bar (plan §6.11); the exact expectation a user has clicking a tray that shows recording state (plan §6.2 principles 9/10) |
| D2  | Tray click while **no** session is active, meeting mode off, or flag disabled                                | `null`                                           | Show only, route untouched — preserves today's verified behavior (`tray.rs` 386)                                                                                 |
| D3  | Completion notification click (system notification after processing finishes, plan line 657)                 | `{ meetings, child: { detail, meetingId } }`     | Meeting detail, default tab `Übersicht` (plan line 661)                                                                                                          |
| D4  | Processing-failed notification click (retry-worthy failure)                                                  | `{ meetings, child: { processing, meetingId } }` | Checklist view with `Fehlerhafte Abschnitte erneut versuchen` reachable (plan line 655)                                                                          |
| D5  | Tray menu items (`settings`, `secure_input_warning`, `check_updates` — `tray.rs` lines 402–417)              | `{ settings }` for `settings`; `null` otherwise  | Settings lands on its first section; warnings/updates stay on the current view (banners/toasts handle context, as today)                                         |
| D6  | Single-instance relaunch and macOS `RunEvent::Reopen` (`lib.rs` lines 424, 619–621)                          | `null`                                           | Show/unminimize/focus only; a user summoning the app does not want their context teleported                                                                      |
| D7  | In-app requests (`showMainWindowCommand` from permission onboarding, `lib.rs` line 574–581 flow)             | `null`                                           | Onboarding flow controls the screen anyway                                                                                                                       |

Adding an eighth emitter requires extending this table; an emitter outside it
is a defect against this contract.

### 4.4 Application rules

- The bootstrap listener validates the payload shape; invalid payloads are
  dropped (log only) — never partially applied.
- A route hint referencing an id applies F3 (unresolvable-id fallback) after
  the target store's mount snapshot resolves; the fallback decision is made
  once, not repeatedly on refetches.
- Hints arriving before the reducer exists (boot race) are queued by the
  bootstrap module and applied in arrival order after step 5 of §3.3; the
  last hint wins, matching "most recent user intent".

### 4.5 Hand-offs

1. Register `main-window-navigate` in `identifier-registry.md` when Phase 4
   wires the first emitting call site.
2. Backend implementation extends `utils::show_main_window` with an optional
   route-hint parameter and emits after `set_focus` — ordering note: emit
   **after** the window is shown so a hidden webview never drops the event
   (Tauri delivery to a non-yet-visible webview is the failure mode; the
   §4.4 queue is belt-and-braces, not license to emit early).
3. Copy for notification actions belongs to the UX copy catalog owner
   (notification click targets D3/D4 are named here so catalog entries can
   anchor to them).

---

## 5. Meeting-running state vs. navigation

Ruling: **a running meeting never restricts navigation.** It enriches it.
Deriving from plan line 695 ("während Aufnahme und Verarbeitung gehen keine
Aktionen durch Navigation verloren") and plan §6.11's status bar
("minimieren/zum Tray wechseln"):

- **M1 — All six views stay reachable while `recording`/`paused`.** Capture
  is backend-owned (RC/SSC); leaving `{ live }` loses nothing. There are no
  route guards, no modal blockers, no "are you sure you want to leave the
  meeting" — the destructive act is stopping, and M4 keeps that distinct.
- **M2 — The pinned entry is the return path.** While
  `meetingStore.activeSession ∈ {recording, paused}`, `PrimaryNavigation`
  renders the pinned `Aufnahme läuft · mm:ss` entry directly below "Meetings"
  (plan lines 426–427); activating it navigates to `{ live }` (§6). Its red
  dot reflects session state, its clock the elapsed time — nothing else.
- **M3 — Actions gate, routes don't.** Starting a _second_ meeting from
  `{ meetings }` while one runs hits the coexistence-contract start mutual
  exclusion at the action level (visible user decision in the setup dialog);
  the navigation layer implements none of that and must not pre-emptively
  hide the `Neues Meeting` button based on guesses about device availability.
- **M4 — Navigation is never a side effect of recording control.** Stop/pause
  buttons live inside F7/F8 and the overlay/tray; pressing them may _change_
  what the pinned entry shows (via `meeting://session-state` events) but no
  recording control may dispatch a route, and no route change may stop,
  pause, or finalize anything. After T08/T09 the UI _may_ leave `{ live }` —
  session-state-contract §3.3 (line 216: after T08/T09 "the UI may close
  immediately") sanctions exactly one automatic navigation in the app:
  `{ live }` → `{ processing, meetingId }`, driven by the store state
  transition, never by the stop button's click handler.
- **M5 — Window close during recording** follows shutdown-dataflow variant T
  verbatim; the navigation layer contributes no logic, no prompts, and no
  state (rule N4 restated as a boundary).

---

## 6. PrimaryNavigation consumes session state without polling

The pinned entry (M2) is the only place product navigation touches live
session state. The seam:

1. **Source: `meetingStore` selector.** F2 reads
   `useMeetingStore(s => s.activeSessionSummary)` — a small projected value
   `{ meetingId, state: "recording" | "paused", startedAt }` maintained by
   S1 from `meeting://session-state` events under SSC O2/O3 revision guard
   and the O5 subscribe-then-snapshot handshake (FSC L5/L6). F2 never calls
   commands, never listens to topics (FSC L2/C1), and never imports
   recorder-level streams (SSC O4).
2. **No polling, mechanically.** No `setInterval`, `setTimeout` chain,
   refetch timer, or focus/visibility-triggered refresh exists anywhere in
   F1/F2 for session state. Currency is guaranteed by the event feed plus
   handshake, not by asking again (SSC §4 preamble: three independent windows
   converge without authoritative copies).
3. **Clock without store traffic.** The `mm:ss` duration is computed in a
   rAF-driven hook from `startedAt` (monotonic against `Date.now()` deltas),
   rendered at ≤1 Hz granularity; it never writes into any store (FSC B5).
   Pausing freezes the displayed duration only when the store's
   `activeSessionSummary.state` says `paused` — accumulated pause time is the
   backend timeline's truth, not a frontend subtraction.
4. **State dot honesty.** Red dot ⇔ `state === "recording"`; a paused meeting
   shows the amber/paused treatment defined by the color/action logic owner
   (plan §6.8) — the navigation layer only maps state → variant, never
   decides truth.
5. **Flag discipline.** Until `MEETING_MODE_ENABLED`, the summary selector
   yields `null` by construction (S1 doesn't exist yet), preserving the
   double gate already coded in `ActiveMeetingNavItem.tsx` lines 30–31.

Testability (for the harness FSC §0.2 item 4 mandates): emit a session-state
burst around a mounted `Sidebar`, assert (a) the pinned entry appears/disappears
exactly with `recording`/terminal transitions, (b) no `listen()` call
originates from `components/**` (static gate FSC §3.3 passes), (c) advancing
fake rAF ticks updates the label without any store write (spy on `setState`),
(d) a revision-regressed event (O3 violation simulation) leaves the displayed
state unchanged.

---

## 7. Window seam and responsive logic (gate G2)

### 7.1 Closed set of window-management primitives

Plan §6.20 line 839 forbids a "second management app"; this contract forbids
ad-hoc window machinery. The complete, closed set of primitives meeting-mode
UI may rely on:

| #   | Primitive                                                                                                           | Site today                                |
| --- | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| W1  | Programmatic window creation options (`inner_size`, `min_inner_size`, `resizable`, `maximizable`, `visible(false)`) | `lib.rs` lines 493–500                    |
| W2  | `show_main_window` = unminimize + show + focus (+ §4.2 route-hint extension)                                        | `utils.rs` lines 12–21                    |
| W3  | CloseRequested → hide (app persists in tray)                                                                        | `lib.rs` lines 585–606                    |
| W4  | Overlay window, fully independent webview                                                                           | `overlay.rs` (untouched by this contract) |
| W5  | Settings persistence via `tauri-plugin-store` (carries `window_state`, §7.5)                                        | existing settings mechanism               |
| W6  | One proposed event topic `main-window-navigate` (§4.2)                                                              | planned-new                               |

Anything beyond W1–W6 — new Tauri plugins (notably
`tauri-plugin-window-state`), additional windows, JS-side window-position
polling, custom title-bar resize handles, protocol handlers — is rejected
(§8). Responsive behavior itself is **CSS work**, not window work.

### 7.2 Min-size and the operating floor

The v1 minimum stays exactly today's `min_inner_size(680.0, 570.0)`
(`lib.rs` line 497); this contract neither raises nor lowers it. Consequences,
binding for every route of §2.2:

- Every screen — including project detail with its four tabs and meeting
  detail with its transcript — must be fully operable at 680 × 570 CSS px
  (plan line 865 gate 8). At the floor, the two-column layouts of §7.3 are
  collapsed; nothing is cut off, only stacked/scrolled.
- Standard maximize and fullscreen remain permitted OS behaviors (plan lines
  841–842); no in-app fullscreen toggle is required or added (W-set has none).
- Touch targets ≥ 44 px and keyboard reachability apply at every width (plan
  §6.14) — narrowness is never an excuse for hover-only affordances.

### 7.3 Breakpoints and collapse points

Measured with **CSS container queries on the content-area container** (the
flex child right of the sidebar), not viewport media queries: the sidebar is
fixed-width today but need not stay so forever, and container queries keep
collapse decisions true to the space content actually has. Three named
breakpoints, defined once as custom media/container conditions in the shell's
stylesheet:

| Condition | Content-container width                  | Behavior                                                                                                                                                                                                                                                                  |
| --------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `narrow`  | < 720 px (≈ the 680 floor minus sidebar) | Single-column everything. Stacked layers get explicit sequential navigation (§6.20 line 845: "Bei schmalen Fenstern werden diese Ebenen nacheinander gezeigt"): project list ↔ detail swap in place via the R4 back affordance; meeting detail tabs scroll horizontally. |
| `regular` | 720 – 959 px                             | Default single-pane layout per page; lists gain breathing room; still no side-by-side editing.                                                                                                                                                                            |
| `wide`    | ≥ 960 px                                 | Project area renders two columns — list/navigation pane and detail pane (plan lines 844–845); meeting detail may offer the opt-in `Geteilte Ansicht` (plan lines 677–681, UX-owned placement).                                                                            |

Collapse rules binding across breakpoints:

- **C1 — Nothing truncates silently.** Detail tabs overflow horizontally
  scrollable (keyboard-reachable, focus visible) or switch into an accessible
  overflow menu; clipped/inaccessible tabs are defects (plan lines 846–847).
- **C2 — Collapse is layout-only.** Narrow mode removes columns, never
  features: every action available wide remains reachable narrow (plan §6.21
  release gate: "keine Hauptfunktion ist nur über ein Drei-Punkte-Menü oder
  Hover erreichbar").
- **C3 — One name per action.** Primary action top-right per page, optional
  repeat at the bottom of long views, never two differently-named buttons for
  the same action (plan lines 850–852). The breakpoint system must not fork
  button sets.
- **C4 — No JS resize listeners in components.** Components consume container
  conditions declaratively; a `ResizeObserver`/`resize`-event spider across
  F1–F22 is rejected (complexity budget §8). The shell may own exactly one
  container element that defines the query containment context.

### 7.4 Independent overlay

The recording overlay stays a separate webview showing status, duration,
pause and stop even with the main window closed or hidden (plan lines
848–849; W4). No navigation state is shared with it — it has no routes. The
pinned nav entry and the overlay are two projections of the same
`meetingStore`/backend truth and must agree because both derive from SSC
events, not because they synchronize (plan §6.2 principle 9).

### 7.5 Window geometry persistence (`window_state`)

- Written by the Rust side (not the webview): debounced save ≥ 1 s after the
  last resize/move event and once on hide/quit, into the existing settings
  store under the §3.2 key.
- Applied at creation (W1) before first show; maximized flag restores
  maximization, not fake-maximized geometry.
- Sanitization mandatory: clamp size to ≥ min_inner_size; drop positions
  outside the current monitor union (unplugged-monitor case) and fall back
  to the 680 × 570 centered default.
- Per-device by construction: the settings store is local-profile data
  (portable mode redirects it too) — no sync, deliberately (plan line 842
  "pro Gerät gespeichert").

---

## 8. Complexity budget and non-goals (binding)

Consistent with the company complexity budget and the anti-bloat mandate,
this contract rejects:

- **No router library.** No react-router/@wouter/history API/URL-hash
  routing. The webview has exactly one window and one route value; a router
  would add URL semantics nobody can link to.
- **No browser-history integration.** No popstate handling, no swipe-back,
  no undo-style route history stack (N-R4).
- **No persisted navigation beyond the §3.2 allowlist.** No child/tab/scroll/
  wizard-step persistence; no "reopen last meeting" heuristics.
- **No new window plugin or primitive** beyond W1–W6: explicitly no
  `tauri-plugin-window-state` dependency, no second window, no
  always-on-top main window, no JS window-position polling.
- **No route guards/middleware/permission system.** M1 fixes that recording
  never blocks navigation; there is nothing for a guard framework to do.
- **No polling of any kind in navigation** (§6.2) and no per-frame store
  writes (B5 stands).
- **No navigation-side business logic.** F1/F2 decide nothing about sessions,
  devices, jobs, or providers; they display projections and dispatch intents.
- **No speculative seams.** No multi-window route synchronization, no OS
  protocol/deep-link handlers, no widget/shortcuts surface — roadmap items
  (plan lines 1738–1783) consume §4's seam when they arrive; nothing is
  reserved now.
- **No visual design in this document.** Colors, copy, motion belong to the
  UX workstream; only structure is normative here.

---

## Appendix A — Gate verification checklist

| Gate                                                                               | Where satisfied                                                                                                                                                                                                                                                                           |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 route table maps every plan §6 page/detail view to a route with required stores | §2.1 route type (six top-level views byte-compatible with `ProductViewId`); §2.2 table citing plan anchors §6.3–§6.19, components F1–F22 homes, and stores S1–S5 per FSC §2.1, with dialogs explicitly hosted (R4); coverage-proof sentence closes the totality argument                  |
| G2 breakpoints/collapse per §6.20, no ad-hoc window primitives                     | §7.1 closed primitive set W1–W6 with code sites; §7.2 min-size floor kept at verified `lib.rs` values; §7.3 named container-query breakpoints `narrow/regular/wide` with collapse rules C1–C4; §7.5 geometry persistence without new plugins                                              |
| G3 deep-open scenarios each have a defined landing route                           | §4.1 single-seam diagram; §4.3 emitter enumeration D1–D7 (tray click during recording → live view; completion notification → meeting detail; failures → processing view; D2/D6 preserve current verified no-navigate behavior); §4.4 application/boot-race rules; §3.3/O1 restart landing |
| G4 non-goals section; documentation index updated                                  | §8 complexity budget/non-goals; row appended to `docs/meeting-mode-documentation-index.md` §3 table plus reading-order entry per its Pflegeregeln                                                                                                                                         |

## Appendix B — Implementation prerequisites (for the implementing tasks)

1. Phase 3 shell task: introduce the `Route` type/reducer at F1 (§2.1 R5),
   migrate `App.tsx`'s `useState<ProductViewId>` onto it, add the
   `ui.last_view` settings key (write on navigate, read per §3.3).
2. Phase 4 task: register `main-window-navigate` in identifier-registry,
   extend `show_main_window` with the route hint, wire emitters D1/D3/D4/D5,
   implement the bootstrap listener + boot queue (§4.4), and move
   `ActiveMeetingInfo` sourcing from props to the §6 store selector.
3. Same phase: `window_state` save/apply/sanitize in Rust (§7.5) and the
   container-query scaffolding with the three named conditions (§7.3).
