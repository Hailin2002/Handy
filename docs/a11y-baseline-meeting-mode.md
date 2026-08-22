# Accessibility Baseline Audit — Plan §6.14 (Meeting Mode)

- **Audit date:** 2026-08-22
- **HEAD commit:** `d74a0da65b51d57abc747d0e9ce326c558c8fa47` ("company(task-0023): Define the dictation-meeting coexistence and resource arbitration contract")
- **Scope:** read-only audit of the current `src/` frontend tree against every bullet of `docs/meeting-mode-plan.md` §6.14 ("Zugänglichkeit und Fehlbedienungsschutz", plan lines 683–695). No source files were modified.
- **Method:** manual code inspection of all shared UI primitives (`src/components/ui/*`), navigation (`Sidebar`, `Footer`), feature components (model selector, history, settings, onboarding), and the recording overlay (`src/overlay/*`), plus repo-wide greps for `aria-*`, `role=`, `sr-only`, `focus-visible`, `prefers-reduced-motion`, `motion-reduce`. Contrast ratios were computed programmatically (WCAG relative-luminance formula) from the exact palette values in `src/styles/theme.css`; the computed ratios are quoted below.

**Legend:** PASS / PARTIAL / FAIL. Verdicts are for the *current* app as baseline; the delta list at the end defines what the Phase 3 App-Shell rework must implement.

---

## 1. Per-requirement verdicts

### 6.14.1 Interactive targets at least 44 × 44 CSS px — **FAIL**

Nearly every interactive control in the current tree is far below 44 × 44:

| Component | Evidence | Approx. size |
|---|---|---|
| `Button` sizes | `src/components/ui/Button.tsx:45-49` (`sm: px-2 py-1 text-xs`, `md: px-4 py-[5px]`) | ~24–32 px tall |
| Dialog close | `src/components/ui/Dialog.tsx:189` (`p-1` around a 16 px `X` icon, line 191) | ~24 px |
| History icon actions | `src/components/settings/history/HistorySettings.tsx:28` (`p-1.5` around 16 px icons; used at lines 363–411 incl. copy/save/retranscribe/**delete**) | ~28 px |
| ResetButton | `src/components/ui/ResetButton.tsx:17` (`p-1` around 16 px icon) | ~24 px |
| Overlay cancel `.sx` | `src/components/overlay/RecordingOverlay.css:404-408` (`width: 22px; height: 22px`) with 9 px glyph; rendered from `src/overlay/RecordingOverlay.tsx:186-201` | 22 px |
| SettingContainer info icon | `src/components/ui/SettingContainer.tsx:71-92` and `144-165` (bare 16 px SVG, no padding) | ~16 px |
| Shortcut recorder chip | `src/components/settings/shared/GlobalShortcutInput.tsx:287-293`, `HandyKeysShortcutInput.tsx:335-340` (`px-2 py-1 text-sm`) | ~28 px |
| Model status button | `src/components/model-selector/ModelStatusButton.tsx:52-58` (text-sm row) | ~28 px |

No `min-h`/`min-w` guard, hit-area pseudo-element, or spacing compensation exists anywhere (`grep min-h-11|min-w-11|44` → no matches).

### 6.14.2 Full keyboard operability and visible focus — **FAIL**

Keyboard-operability defects:

- **Sidebar navigation is mouse-only:** sections are `<div onClick>` with no `tabIndex`, no role, no key handler — the entire settings area is unreachable by keyboard. `src/components/Sidebar.tsx:119-127`.
- **Shortcut recorders are click-only `<div>`s:** `GlobalShortcutInput.tsx:287-293` and `HandyKeysShortcutInput.tsx:335-340` (no `tabIndex`, no key handler).
- **Onboarding model cards:** `role="button"` + `tabIndex` exist, but only `Enter` is handled — Space does not activate, and there is no focus style: `src/components/onboarding/ModelCard.tsx:148-162`.

Visible-focus defects (repo-wide `focus-visible` appears exactly once, in `Dialog.tsx:189`):

- `Button` sets `focus:outline-none` globally (`Button.tsx:22-23`). The `secondary` variant adds **no** replacement indicator (`Button.tsx:30-31`), and `danger-ghost`/`ghost` only swap hover-class backgrounds (`Button.tsx:39-42`) — focus is invisible or indistinguishable from hover.
- `Input` removes the outline leaving only a subtle tint/border change: `src/components/ui/Input.tsx:16-18`.
- `ModelDropdown` rows: `focus:outline-none` with no substitute (`src/components/model-selector/ModelDropdown.tsx:43`).
- Positive exceptions: `Select` (react-select wrapper) shows focus via border + box-shadow (`src/components/ui/Select.tsx:56-58`); `Slider`/range inputs keep `focus:ring` (`src/components/ui/Slider.tsx:61`, `AudioPlayer.tsx:310`); `SecureInputWarning` buttons use `focus:ring-1` (`SecureInputWarning.tsx:101,111`); Dialog close uses `focus-visible:ring-1` (`Dialog.tsx:189`). These are the pattern the rest must adopt.

### 6.14.3 Sensible tab order and focus return after dialogs — **PARTIAL**

- The shared `Dialog` is genuinely good: saves `document.activeElement` on open and restores it on close (`src/components/ui/Dialog.tsx:70-73` and restore at line 89), traps Tab cyclically including Shift+Tab (`Dialog.tsx:104-131`), supports `initialFocusRef` (line 25), Escape-to-close when dismissible (98-102), `role="dialog"`/`aria-modal`/`aria-labelledby`/`aria-describedby` (166-169).
- **Gap:** the portable-update modal in `UpdateChecker` is a hand-rolled fixed overlay with none of this — no role, no focus trap, no focus restore, no Escape handling: `src/components/update-checker/UpdateChecker.tsx:204-236`.
- Tab order follows DOM order everywhere else; the Sidebar failure in 6.14.2 makes "sensible tab order" moot until that is fixed.

### 6.14.4 Screen-reader names for all icons, levels and status changes — **PARTIAL**

Present (good examples to build on):
- Toggle switches wrap an `sr-only` checkbox inside a labelled setting row: `src/components/ui/ToggleSwitch.tsx:39-46`.
- `CustomWords` remove button: `aria-label={t("settings.advanced.customWords.remove", { word })}` — `src/components/settings/advanced/CustomWords.tsx:102`.
- Secure-input dismiss: `aria-label={t("secureInput.dismiss")}` — `src/components/SecureInputWarning.tsx:110`.
- Models page rescan + filters: `aria-label` / `aria-pressed` — `src/components/settings/models/ModelsSettings.tsx:275, 289-304`.
- Dialog close: `aria-label={closeLabel}` — `Dialog.tsx:188`; decorative lucide icons marked `aria-hidden` (e.g. `Dialog.tsx:191`).

Missing / broken:
- **History icon actions rely on `title` only** (copy, save, retranscribe, delete): `HistorySettings.tsx:25-36` (component takes `title`, no `aria-label`) used at lines 363–411.
- **ResetButton's `ariaLabel` is optional and never passed** at call sites (`GlobalShortcutInput.tsx:294-297`, `Slider.tsx:75-77`), so unnamed-icon-button: `src/components/ui/ResetButton.tsx:13-17`.
- **Model status is conveyed by a colored dot only**, with an untranslated hardcoded `title={`Model status: ${displayText}`}` and no `aria-expanded` for the attached dropdown: `src/components/model-selector/ModelStatusButton.tsx:28-49, 52-58`. Dropdown rows lack `aria-current` for the active model (`ModelDropdown.tsx:31-47`).
- **Overlay mic level meter is purely visual** (9 styled `<i>` bars, no text alternative): `src/overlay/RecordingOverlay.tsx:173-184`; overlay cancel label is hardcoded lowercase English `"cancel"` (`RecordingOverlay.tsx:189`).
- **Progress bars are bare `<progress>` elements with no accessible name**: `src/components/shared/ProgressBar.tsx:46-50, 79-87` (multi-bar variant uses `title` only, line 83).
- Hardcoded non-i18n SR strings: `"Pause"/"Play"` (`AudioPlayer.tsx:287`), `"More information"` twice (`SettingContainer.tsx:76, 149`), `"cancel"` (`RecordingOverlay.tsx:189`) — violates the repo's own i18n rule (AGENTS.md) and produces English names in all UI languages.

### 6.14.5 Status changes via restrained live regions instead of focus stealing — **PARTIAL**

- Toasts are announced: the single sonner `<Toaster>` (`src/App.tsx:261-276`) renders `aria-live="polite"` (verified in `node_modules/sonner/dist/index.mjs:1084`). All error paths route through `toast.error` (`App.tsx:104-176`, `GlobalShortcutInput.tsx:110-122`, `HistorySettings.tsx:340,350`).
- Beyond toasts there is **no `aria-live` region anywhere in `src/`** (repo-wide grep: zero matches in source). Specifically missing: model load/download state flips (`ModelSelector.tsx:71-141`), streaming transcript text in the overlay (`RecordingOverlay.tsx:250-266`), phase changes (`RecordingOverlay.tsx:118-122`). Nothing steals focus on status change (good), but nothing announces persistent-state changes either.

### 6.14.6 Text contrast per WCAG AA — **FAIL**

Ratios below were computed with the WCAG relative-luminance formula from the exact hex values in `src/styles/theme.css:7-28` (opacity classes blended over the theme backgrounds). AA requires 4.5:1 normal text, 3:1 large text (≥18 pt / 14 pt bold).

| Pair (light theme) | Ratio | Verdict | Example usages |
|---|---|---|---|
| `--color-text` #0f0f0f on `--color-background` #fbfbfb | 18.52 | ✅ | body text |
| `text-text/60` blended | 4.97 | ✅ (barely) | `Footer.tsx:26`, `UpdateChecker.tsx:245` |
| `text-text/50` blended | 3.57 | ❌ normal | `ModelDropdown.tsx:64` desc, `ModelCard.tsx:265,301,308,337` |
| `text-text/40` blended | 2.63 | ❌ even large | `ModelDropdown.tsx:54,64`, `ModelCard.tsx:273` |
| `mid-gray` #808080 | 3.82 | ❌ normal | `Dialog.tsx:179`, `Dropdown.tsx:91`, descriptions in `HandyKeysShortcutInput.tsx:268` etc. |
| white on `background-ui` #da5893 | 3.62 | ❌ normal (14 px medium button labels) | primary `Button` (`Button.tsx:26-27`) |
| `logo-primary` #faa2ca **as text** | 1.83 | ❌ severe | active model row `ModelDropdown.tsx:45`, update link `UpdateChecker.tsx:244` |

| Pair (dark theme) | Ratio | Verdict |
|---|---|---|
| `--color-text` on `--color-background` | 13.67 | ✅ |
| `text-text/60` | 5.94 | ✅ |
| `text-text/50` | 4.61 | ✅ (marginal) |
| `text-text/40` | 3.50 | ❌ normal |
| `mid-gray` | 3.58 | ❌ normal |
| `logo-primary` #f28cbb as text | 6.23 | ✅ (dark only — shows light theme is the outlier) |

Additional failures:
- `Alert` variant texts use raw Tailwind `red-400/yellow-400/blue-400/green-400` on tinted surfaces (`src/components/ui/Alert.tsx:18-37`) — e.g. yellow-400/red-400 on light backgrounds fall well under 4.5:1; status also carried by color alone (see 6.14.10 context).
- Overlay timer uses `--s-faint` #9a9a9a on the near-white surface = **2.72** (`src/overlay/RecordingOverlay.css:30, 338-345`); dark-theme faint #6f6c66 = **2.70** (`RecordingOverlay.css:31, 89-95`). Muted tokens pass (4.93 / 5.42).
- Warning icon token `#d97706` = 3.08 on light bg (`theme.css:20`) — acceptable for icons/UI elements (≥3:1) but not for text.
- Disabled controls at `disabled:text-text/20` (`HistorySettings.tsx:28`) and `opacity-50` buttons are exempt from AA (WCAG exception), noted for completeness.

### 6.14.7 Reduced motion respected — **FAIL**

Repo-wide grep for `prefers-reduced-motion` / `motion-reduce`: **zero matches** in `src/` (only Tailwind's own machinery). Continuous/decorative animations currently always run:

- `animate-pulse` status dots: `ModelStatusButton.tsx:33-39` (loading/downloading/verifying/extracting states pulse indefinitely).
- `animate-spin` loaders: `ToggleSwitch.tsx:51`, `ModelCard.tsx:190`.
- Inline CSS animations: retranscribe spinner `animation: "spin 1s linear infinite reverse"` (`HistorySettings.tsx:398-402`) and injected `transcribe-pulse` keyframes (`HistorySettings.tsx:430-435`).
- Overlay: card pop-in + width morph (`RecordingOverlay.css:174-190, 212-218`), pulsing record dot `sdot-pulse` (`:330-337`), arming wave `swave-arming` infinite loop (`:372-403`), caret blink `scaret-blink` infinite (`:269-283`), spinner `sspinn` infinite (`:439-452`).
- Hover scale transforms (`ModelCard.tsx:130`, `RecordingOverlay.css:432`).

### 6.14.8 Tooltips complement labels, do not replace them — **PARTIAL**

- `Tooltip` portal has positioning logic but **no `role="tooltip"`, no `aria-describedby` wiring, no Escape dismissal, and does not open on keyboard focus**: `src/components/ui/Tooltip.tsx:93-113`.
- `SettingContainer` defaults `descriptionMode="tooltip"` (`SettingContainer.tsx:19`), hiding every setting's explanation behind a hover/click-only info icon. Keyboard toggling of the icon exists (Enter/Space handlers, `SettingContainer.tsx:79-84, 152-157` — better than nothing) but the popup isn't focus-linked, lacks roles, and stays open until outside-click.
- Native `title` tooltips are the *only* name for several icon actions (history actions `HistorySettings.tsx:366, 378-382, 393, 408`; model delete `ModelCard.tsx:282`; model status `ModelStatusButton.tsx:55`) — titles don't replace labels per the requirement and aren't reliably exposed to SRs/touch.

### 6.14.9 Delete and stop actions remain reachable without hover — **PARTIAL**

- All destructive/stop controls are real `<button>` elements, so they are keyboard-reachable where the surrounding widget is: history delete (`HistorySettings.tsx:405-411`), post-processing prompt delete (`PostProcessingSettings.tsx:336-341`), model delete (`ModelsSettings.tsx:414, 436` via `ModelCard.tsx:277-288`), overlay stop/cancel (`RecordingOverlay.tsx:186-201`, always rendered in every overlay form).
- Gaps: the click-only `div` widgets (Sidebar, shortcut recorders — see 6.14.2) would strand keyboard users on any future destructive action placed in them; destructive icon buttons are visually distinguished only on hover (`danger-ghost`, `Button.tsx:39-40`), so their affordance depends on hover discovery.

### 6.14.10 Irreversible deletion names audio, transcript and summary concretely — **FAIL**

- History deletion today is immediate and silent: optimistic removal then `commands.deleteHistoryEntry(id)` with **no confirmation dialog and no statement of what is lost** (the recording audio and its transcript together): `src/components/settings/history/HistorySettings.tsx:204-214, 335-342, 405-411`.
- Counter-example in-tree showing the intended pattern: model deletion asks via `ask()` with a named model (`src/components/settings/models/ModelsSettings.tsx:149-159`).
- Meeting summaries do not exist yet; the requirement binds fully in Phase 3, but the current nearest analog already violates it.

### 6.14.11 During recording/processing no action is lost through navigation — **PASS (by current architecture)**

- Recording is driven by OS-global shortcuts and a separate always-on-top overlay window (`src/overlay/RecordingOverlay.tsx`; architecture per AGENTS.md); browsing settings cannot cancel a capture, and the stop/cancel control lives permanently in the overlay (`RecordingOverlay.tsx:186-201, 213, 226, 276, 299`). There is no in-window recording state to navigate away from yet.
- This verdict is architectural, not implemented for the planned meeting UI: the Phase 3 meeting view (tabs for Transkript/Audio/Verarbeitung, plan lines 671–675) must re-satisfy this with explicit evidence.

---

## 2. Delta list for the Phase 3 App-Shell rework (actionable)

Each item names affected component(s) and the concrete change. Items D1–D3 are prerequisites most other items build on.

1. **D1 — Sidebar.tsx:** replace the clickable `<div>` (Sidebar.tsx:119-127) with a `<nav>` containing `<button>`s; set `aria-current="page"` on the active section, add a visible `focus-visible` ring, and give items ≥44 px height.
2. **D2 — New shared `IconButton` primitive:** create `src/components/ui/IconButton.tsx` requiring an `aria-label` (typed as mandatory), enforcing a ≥44 × 44 hit area (compact visuals allowed via transparent padding/pseudo-element), and a standard `focus-visible` ring. Migrate onto it: local `IconButton` in `HistorySettings.tsx:18-37`, `ResetButton.tsx`, Dialog close (`Dialog.tsx:185-192`), overlay `.sx` (`RecordingOverlay.tsx:186-201`), `SettingContainer` info icon (`SettingContainer.tsx:64-100, 137-173`), `ModelStatusButton` trigger.
3. **D3 — ui/Button.tsx + ui/Input.tsx:** drop blanket `focus:outline-none` (Button.tsx:23, Input.tsx:18); apply a uniform `focus-visible:ring-2 ring-logo-primary outline-none` to **all** variants (especially `secondary`, `danger-ghost`, `ghost` — Button.tsx:30-42); introduce `size="md"|"lg"` minimum heights of 44 px and reserve `sm` for dense toolbars with extended hit areas.
4. **D4 — Shortcut inputs:** convert the recorder chips to `<button type="button">` with `aria-label={t("settings.general.shortcut.change", { name })}` in `GlobalShortcutInput.tsx:287-293` and `HandyKeysShortcutInput.tsx:335-340`; announce recording state via `aria-live="polite"` on the chip.
5. **D5 — Live regions:** add one polite `aria-live` region for persistent status (model ready/loading/error, processing phases) fed from `ModelSelector`'s existing display text (`ModelSelector.tsx:156-235`); make the overlay live-text cap (`RecordingOverlay.tsx:250-266`) `aria-live="polite"` with the phase label as `role="status"`. Keep everything else out of live regions (restrained announcements only).
6. **D6 — Contrast tokens:** introduce `--color-text-muted` defined per theme to meet ≥4.5:1 on both backgrounds (light needs darker than #808080; measured 3.82) and replace `text-mid-gray`, `text-text/50`, `text-text/40` text usages; darken light-theme `--color-background-ui` (or its button foreground) so primary-button text reaches 4.5:1 (currently 3.62, `Button.tsx:26-27`); forbid `logo-primary` as text color on light surfaces (`ModelDropdown.tsx:45,69`, `UpdateChecker.tsx:244`); remap `Alert.tsx:18-37` to the semantic `warning`/`error` tokens from `theme.css:20-23`; darken overlay `--s-faint` to ≥4.5:1 (`RecordingOverlay.css:30-31`, measured 2.72/2.70).
7. **D7 — Reduced motion:** add a global `@media (prefers-reduced-motion: reduce)` block in `App.css` (and mirror in `RecordingOverlay.css`) that disables `animate-pulse`, decorative transforms, card pop/morph, `sdot-pulse`, `swave-arming`, `scaret-blink`, `spin` decorations, and the inline `transcribe-pulse`/`spin` styles (`HistorySettings.tsx:398-402, 430-435`); keep short opacity fades (≤200 ms).
8. **D8 — Tooltip semantics:** in `Tooltip.tsx` add `role="tooltip"` + stable id, wire `aria-describedby` from each trigger, open on trigger focus as well as hover, close on Escape and blur; flip `SettingContainer` default `descriptionMode` to `"inline"` so tooltips complement rather than replace visible descriptions (plan §6.14 bullet 8); eliminate title-only labeling for icon actions (see D2/D10).
9. **D9 — Destructive-action confirmation:** route history deletion through the existing `ui/Dialog` with a confirmation that names exactly what will be deleted (audio recording, transcript, and summary once summaries ship), mirroring the `ask()` pattern of `ModelsSettings.tsx:149-159`; apply to any Phase 3 delete/stop. Ensure destructive buttons carry text labels or mandatory aria-labels, never `title` alone.
10. **D10 — Screen-reader names & localization:** add accessible names to `ProgressBar` bars (`aria-label` per bar, `ProgressBar.tsx:46-50, 79-87`); `ModelStatusButton`: localized `aria-label` including status text plus `aria-expanded`/`aria-haspopup` (`ModelStatusButton.tsx:52-72`); `aria-current="true"` on the active row in `ModelDropdown.tsx:31-47`; localize the hardcoded English SR strings `"Play"/"Pause"` (`AudioPlayer.tsx:287`), `"More information"` (`SettingContainer.tsx:76,149`), `"cancel"` (`RecordingOverlay.tsx:189`), `Model status:` (`ModelStatusButton.tsx:55`) through i18next per AGENTS.md.
11. **D11 — Replace ad-hoc modal:** rebuild the portable-update dialog on `ui/Dialog` to inherit focus trap, focus restore, Escape and labelling (`UpdateChecker.tsx:204-236`).
12. **D12 — Onboarding ModelCard:** add Space activation alongside Enter, a `focus-visible` ring, and an action-describing `aria-label` on the card button (`ModelCard.tsx:148-162`).
13. **D13 — Overlay cancel target:** enlarge `.sx` to an effective ≥44 px hit area (visual circle may stay ~22 px with transparent padding) and add a visible focus ring (`RecordingOverlay.css:404-436`).

**Explicit non-goals (complexity budget, per the mission's simplicity mandate):** no full ARIA-widget rewrite of react-select internals (it is already keyboard-accessible, Select.tsx); no custom roving-tabindex management beyond what Dialog already does; no animated focus indicators or extra toast channels; no new settings surface for accessibility toggles beyond honoring the OS `prefers-reduced-motion` and existing theme settings.

---

## 3. Summary scorecard

| §6.14 requirement | Verdict |
|---|---|
| 44×44 targets | FAIL |
| Keyboard operability + visible focus | FAIL |
| Tab order + focus return | PARTIAL |
| SR names for icons/levels/status | PARTIAL |
| Restrained live regions | PARTIAL |
| WCAG AA contrast | FAIL |
| Reduced motion | FAIL |
| Tooltips complement labels | PARTIAL |
| Delete/stop without hover | PARTIAL |
| Deletion names what is lost | FAIL |
| Nothing lost via navigation during recording | PASS (architectural) |

Verification of gates: every verdict cites ≥1 current-tree `file:line` (G1); the delta list names components and required changes per item (G2); audit date and exact HEAD commit stated above (G3); `git status` after this task shows only the new untracked `docs/a11y-baseline-meeting-mode.md`, no modified source files (G4).
