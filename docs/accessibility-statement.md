# Accessibility Statement

- **Status:** Honest current-state statement — **not** a conformance claim.
- **Date:** 2026-08-22
- **Verified against:** HEAD commit `e8278628d30e59408ca9b80d1dea35d2c3502f1b` (clean worktree, `git status --porcelain` empty).
- **Scope:** the _shipped_ dictation application as it exists today, plus the
  _planned_ meeting-mode accessibility targets. This document makes no claim
  about any meeting-mode feature, because meeting mode is not implemented
  (see `docs/meeting-mode-documentation-index.md`, section 1).

## 0. What this statement does not claim

No WCAG conformance level (A, AA, AAA) is claimed or implied for any part of
this application. No formal WCAG audit, certification, or third-party review
has been performed on the shipped app. What exists is a single internal,
read-only code-level baseline audit (`docs/a11y-baseline-meeting-mode.md`,
audit date 2026-08-22, HEAD `d74a0da65b51d57abc747d0e9ce326c558c8fa47`),
which inspected the frontend source against every bullet of plan §6.14
("Zugänglichkeit und Fehlbedienungsschutz",
`docs/meeting-mode-plan.md` lines 683–695) and produced verdicts plus a
remediation delta list **D1–D13** (baseline document, section 2). Every gap
below cites that baseline audit or one of its named delta items; where we say
"a gap is still open," we re-verified it against the current tree at the HEAD
commit above.

---

## 1. Current known gaps in the shipped dictation app

The baseline audit's summary scorecard (`docs/a11y-baseline-meeting-mode.md`,
section 3) records five FAIL, five PARTIAL, and one PASS(architectural)
verdict against the eleven §6.14 requirements. The three most significant
open gaps, each tied to a named pending remediation task, are:

### 1.1 Contrast tokens do not meet WCAG AA — pending task **D6**

- Baseline evidence: `docs/a11y-baseline-meeting-mode.md` §1 "Text contrast
  per WCAG AA" — **FAIL**. Computed ratios (WCAG relative-luminance formula,
  exact palette values): light-theme `text-text/50` = 3.57:1, `text-text/40`
  = 2.63:1, `mid-gray` #808080 as text = 3.82:1, white-on-primary-button =
  3.62:1, `logo-primary` as text = 1.83:1 — all below the 4.5:1 AA threshold
  for normal text.
- Pending fix: delta item **D6** ("Contrast tokens", baseline section 2,
  item 6): introduce a `--color-text-muted` token meeting ≥4.5:1 in both
  themes, darken primary-button foreground, forbid `logo-primary` as text on
  light surfaces, remap `Alert` variants to semantic tokens, darken overlay
  `--s-faint`.
- Still open at HEAD `e8278628`: `src/styles/theme.css:28` still defines
  `--color-mid-gray: #808080`; no `--color-text-muted` token exists anywhere
  in `src/styles/theme.css`; `text-mid-gray` is still used as body/description
  text color across the UI (e.g. `src/components/ui/Dialog.tsx:180`,
  `src/components/ui/Dropdown.tsx:91`, `src/components/shell/ProductPlaceholder.tsx:22`,
  `src/components/settings/post-processing/PostProcessingSettings.tsx:349`).
  No contrast token has been introduced since the audit.

### 1.2 Reduced motion is not respected — pending task **D7**

- Baseline evidence: `docs/a11y-baseline-meeting-mode.md` §1 "Reduced motion
  respected" — **FAIL** (repo-wide grep found zero matches for
  `prefers-reduced-motion` / `motion-reduce` in `src/`). Indefinitely running
  animations listed there include `animate-pulse` status dots
  (`ModelStatusButton.tsx`), overlay pulse/wave/blink/spin keyframes
  (`RecordingOverlay.css`), and inline spin/pulse styles
  (`HistorySettings.tsx`).
- Pending fix: delta item **D7** ("Reduced motion", baseline section 2,
  item 7): global `@media (prefers-reduced-motion: reduce)` block in
  `App.css` mirrored in `RecordingOverlay.css`, disabling decorative
  animation while keeping short opacity fades ≤200 ms.
- Still open at HEAD `e8278628`: grep for
  `prefers-reduced-motion|motion-reduce` over `src/` returns **zero
  matches**. The OS reduce-motion preference has no effect on the UI today.

### 1.3 Screen-reader naming is incomplete — pending task **D10**

- Baseline evidence: `docs/a11y-baseline-meeting-mode.md` §1 "Screen-reader
  names for all icons, levels and status changes" — **PARTIAL**, with named
  defects including history icon actions labeled by `title` only, an optional
  never-supplied `ariaLabel` on `ResetButton`, model status conveyed by a
  colored dot with a hardcoded untranslated `title`, bare `<progress>`
  elements without accessible names, a purely visual overlay mic-level meter,
  and hardcoded English strings (`"Pause"/"Play"`, `"More information"`,
  `"cancel"`).
- Pending fix: delta item **D10** ("Screen-reader names & localization",
  baseline section 2, item 10).
- Partially addressed since the audit (stated for honesty, not claimed as a
  fix): history icon actions now carry `aria-label`s alongside `title`
  (`src/components/settings/history/HistorySettings.tsx:430,443,463,479`);
  new polite `aria-live` regions exist in
  `src/components/model-selector/ModelSelector.tsx:293` (with render test),
  `src/components/settings/shared/GlobalShortcutInput.tsx:311`,
  `src/components/settings/shared/HandyKeysShortcutInput.tsx:360`, and
  `src/overlay/RecordingOverlay.tsx:258`.
- Still open at HEAD `e8278628` (verified):
  - `src/components/model-selector/ModelStatusButton.tsx:55`: the trigger's
    only name is the hardcoded, untranslated `title={`Model status:
    ${displayText}`}`; there is no `aria-label`, no `aria-expanded`, and the
    status dot remains purely visual (lines 28–49, 52–58).
  - `src/components/shared/ProgressBar.tsx:46–50`: single `<progress>` has no
    accessible name; the multi-bar variant (line 83) relies on `title` alone.
  - The overlay mic-level meter remains a purely visual bar row
    (`src/overlay/RecordingOverlay.tsx` level-meter markup; baseline §1
    6.14.4) — no text alternative was added.

Because remediation is partially underway elsewhere, the baseline audit's
per-item verdicts should be read together with the re-verifications above;
a fresh full audit has **not** been run since `d74a0da6`.

### 1.4 Other recorded findings

The remaining §6.14 verdicts (44×44 targets: FAIL; keyboard operability and
visible focus: FAIL; tab order/focus return: PARTIAL; tooltips: PARTIAL;
delete/stop reachability: PARTIAL; deletion names what is lost: FAIL;
nothing lost via navigation during recording: PASS, architectural) are
recorded, with per-item code citations and remediation items **D1–D5,
D8–D13**, exclusively in `docs/a11y-baseline-meeting-mode.md`. This statement
does not restate their current status because it re-verified only the three
gaps above; treat all of them as unresolved until individually re-verified
against the tree.

---

## 2. Planned meeting-mode accessibility targets

Meeting mode is **not implemented**. The following are planning targets from
`docs/meeting-mode-plan.md`; none of them is evidenced by working code today
(`docs/meeting-mode-documentation-index.md`, section 1).

- **Requirement set (plan §6.14, lines 683–695):** interactive targets ≥44×44
  CSS px; full keyboard operability and visible focus; sensible tab order and
  focus return after dialogs; screen-reader names for all icons, levels and
  status changes; restrained live regions instead of permanent focus moves;
  WCAG AA text contrast; reduced motion respected; tooltips complement rather
  than replace labels; delete/stop reachable without hover; irreversible
  deletion names audio, transcript and summary concretely; no action lost
  through navigation during recording/processing.
- **Gate §6.15 ("Anfänger- und Usability-Gates", lines 697–721):** before
  Phase 3, a clickable prototype must be tested with at least five people
  unfamiliar with Speak/Meetily plus experienced dictation users; eight
  mandatory unassisted tasks (start/stop dictation; start a local meeting
  with mic + system audio; choose Azure for one meeting and understand the
  cloud notice; pause and safely end; copy a finished summary; change the
  meeting shortcut without clobbering dictation; recover from a device or
  network error; resume interrupted processing after restart), plus release
  gates including unambiguous recording status in main window, overlay and
  tray, and every critical error message naming a recognizable next action.
- **Gate §6.21 ("Erweiterte Usability-Gates", lines 854–875):** eight further
  unassisted tasks (project creation, assignment/removal at meeting start,
  assigning finished meetings, WhatsApp import with preview comprehension,
  recognizing what would be uploaded for cloud processing, accepting/editing/
  discarding AI task proposals, jumping from a status statement to its
  evidencing message or meeting position, operating the same project in a
  small and a maximized window) and release gates including: no main function
  reachable only via a three-dot menu or hover, and **project-, meeting- and
  dictation-navigation remains understandable with keyboard and screen reader**
  (plan lines 874–875).

These gates are acceptance criteria for future phases; until they are tested
and passed, they constitute intent, not capability.

---

## 3. Accessibility feedback channel

Accessibility barriers in the shipped dictation app are accepted and wanted
as bug reports:

- **Bug reports / barrier reports:** GitHub Issues at
  <https://github.com/cjpais/Handy/issues> using the bug report template
  (blank issues are disabled; see `.github/ISSUE_TEMPLATE/config.yml`).
- **Feature requests / improvement suggestions:** GitHub Discussions at
  <https://github.com/cjpais/Handy/discussions>, per the repository's
  contribution workflow (`AGENTS.md`, `.github/PULL_REQUEST_TEMPLATE.md`
  "Community Feedback").
- When reporting, please describe the affected component, the interaction
  (input method, assistive technology and version, platform), and the expected
  vs. actual behavior. Reports referencing the §6.14 requirement list or
  delta items D1–D13 of `docs/a11y-baseline-meeting-mode.md` help map the
  report onto an already-tracked remediation item.

This channel is the project's own public repository workflow; there is no
dedicated accessibility team, SLA, or separate contact address.

---

## 4. Claim-to-evidence verification table

| Claim in this statement                                            | Evidence                                                                                                                                                                               |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No WCAG conformance claimed; only internal code-level audit exists | Absence of any audit artifacts besides `docs/a11y-baseline-meeting-mode.md` (HEAD-pinned in its header)                                                                                |
| Contrast gap open (task D6 pending)                                | Baseline §1 6.14.6 FAIL + §2 item 6; `src/styles/theme.css:28` (#808080), no `--color-text-muted` match in `src/styles/theme.css`, live `text-mid-gray` text usages cited above        |
| Reduced-motion gap open (task D7 pending)                          | Baseline §1 6.14.7 FAIL + §2 item 7; grep `prefers-reduced-motion\|motion-reduce` over `src/` → 0 matches at HEAD `e8278628`                                                           |
| SR-naming gap open (task D10 pending)                              | Baseline §1 6.14.4 PARTIAL + §2 item 10; `ModelStatusButton.tsx:55`, `ProgressBar.tsx:46–50,83` read directly at HEAD `e8278628`                                                       |
| Partial remediation happened post-audit                            | `HistorySettings.tsx:430–480` aria-labels; `aria-live` matches in `ModelSelector.tsx:293`, `GlobalShortcutInput.tsx:311`, `HandyKeysShortcutInput.tsx:360`, `RecordingOverlay.tsx:258` |
| Meeting-mode targets are plans, not features                       | `docs/meeting-mode-plan.md` §6.14 (lines 683–695), §6.15 (697–721), §6.21 (854–875); index section 1 "no implementation yet"                                                           |
| Feedback channels                                                  | `.github/ISSUE_TEMPLATE/config.yml`, `AGENTS.md` GitHub workflow section                                                                                                               |

_End of statement._
