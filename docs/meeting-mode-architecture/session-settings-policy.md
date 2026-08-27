# Mid-Session Settings-Mutation Policy — Hot-Apply vs Deferred (Meeting Mode)

Status: **spec contract — no implementation yet** (Phase 4 prerequisite;
changes no source code).
Source of truth: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) §6.11
(live-view rule "lesbar, aber nicht direkt veränderbar", Zeilen 635–637),
§6.7 (start dialog remembers configuration, snapshots isolate sessions,
Zeilen 531–533), §6.9 (shortcut changes take effect after meeting end,
Zeile 570), §6.10 (settings separation, Zeile 594), §12 (retention defaults,
Zeilen 1444–1452).
Sibling contracts consumed read-only below, abbreviated:

- **CC** — [coexistence-contract.md](coexistence-contract.md), specifically
  S4/§6 (settings separation + per-session snapshot isolation) and §7.5
  (mid-meeting shortcut stability).
- **DCD** — [device-change-delta.md](device-change-delta.md), specifically §3
  (Settings hot-apply boundary H1–H5) and decision-table rows M6/S6.
- **TAX** — [error-taxonomy-contract.md](error-taxonomy-contract.md)
  (closed code namespace, envelope rule, registration duty).
- **SPC** — [summary-provider-contract.md](summary-provider-contract.md),
  §2 (cloud consent gate, per-session pinning, withdrawal).
- **AS** — [asset-store-contract.md](asset-store-contract.md), §8.1
  (`RetentionPolicy`, `meeting_sessions.recording_retention_policy` session
  column).
- **RQC** — [retention-quota-contract.md](retention-quota-contract.md), §4
  (per-session stored policy, exemption dimensions).
- **SSC** — [session-state-contract.md](session-state-contract.md), T01
  (snapshot columns written at session creation).

Authority order unchanged: Plan > contracts > catalogs > nothing beats actual
code. This document adds **no new setting key** and **no new enforcement
mechanism**; it classifies the keys already defined by their owning contracts
and names the seam where each class is enforced.

---

## 1. Purpose and scope

The owning contracts fix _what_ each meeting setting is and _where it lives_
(CC §6.1 enumerates the meeting keys; SPC §2.1 fixes the consent key; AS §8.1
fixes the per-session retention column; CC §7.1 fixes the shortcut ids). None
of them answers, in one place, the operator-facing question:

> When a user changes this setting while a meeting session exists, what
> happens — immediately, at the next natural boundary, or nothing until the
> next session?

This document answers it for **every meeting-relevant setting** with a total,
three-way classification, and names the exact UI and backend enforcement point
for each class. It is consistent with DCD §3 by construction: DCD already
fixed the device-key boundary (H1–H5); this document generalizes that boundary
to all settings without altering any H-rule.

### 1.1 Baseline verified in code (read-only)

- `src-tauri/src/settings/types.rs` lines 310–317: existing
  `SpeechInputProvider { Local, Azure, Custom }` — the shape CC §6.1 mirrors
  for the planned `MeetingSttProvider`.
- `src-tauri/src/settings/types.rs` lines 175–183: existing
  `RecordingRetentionPeriod` — a **dictation-history** retention setting;
  distinct from the meeting per-session policy (AS §8.1). Listed to prevent
  conflation, not classified as a meeting key.
- `src-tauri/src/settings/types.rs` lines 319–361: `SecretMap` serializes as
  `{}` — secrets never persist through settings; relevant to §6 item 2.
- No meeting-mode source module exists yet (index rule 1); every enforcement
  seam below is a _future implementation home_, cited to its owning contract.

---

## 2. The three mutation classes (normative definitions)

Every meeting-relevant setting belongs to exactly one **primary class**, with
at most one declared directional exception (§4 table, last column):

| Class                      | Definition                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **hot-apply**              | A mid-session change takes effect inside the running session (or its processing pipeline) without starting a new session. Any such effect must name its exact boundary (e.g. "next stage gate") — unspecified immediacy is forbidden.                                                                                                                                                                                                                                             |
| **apply-next-session**     | A mid-session change persists immediately and legally; the running session is unaffected because it resolves the setting through its session-start capture; the change binds at the next session's validation/snapshot time. No marking duty beyond the standard save acknowledgment.                                                                                                                                                                                             |
| **locked-while-recording** | The session-_effective_ value is frozen by the immutable session snapshot (CC §6.3, DCD H1). Three consequences: (a) the live view renders the value read-only (plan §6.11); (b) a write to the global key while a session records succeeds but is inert for that session and the settings surface marks it "gilt ab dem nächsten Meeting" (DCD M6/S6, H2); (c) any command path that attempts to _apply_ the new value to the running session is rejected with the code of §5.3. |

Decision rule for future keys: if the value feeds capture, inference, summary
routing, or data-deletion obligations of the _running_ session →
**locked-while-recording**; if the running session merely holds a stable
reference but the setting is safe to flip anytime → **apply-next-session**;
if the change must reach the running session for safety or correctness →
**hot-apply**, and the defining contract must own the boundary. A key absent
from §4's table and from its owning contract is forbidden (CC §6.1 closing
rule: free-floating keys are forbidden).

---

## 3. Why there is almost no hot-apply (structural argument)

Plan §6.11 (Zeilen 635–637) makes provider/model/language/devices read-only in
the live view, and plan §6.7 (Zeilen 532–533) states snapshots isolate running
meetings from global changes. CC §6.3 makes the snapshot the _exclusive_ read
source after start, and DCD H1 extends this to pause/resume re-validation.
Together these leave no read path from live settings into a running session —
so a hot-apply classification for any capture-affecting key would require
building a second read path, which violates the complexity budget (§7) and
has zero user value: mid-meeting provider/model swaps would create mixed-
provenance segments (SEC run-key reproducibility forbids this for summaries,
and STT job provenance inherits the same discipline). The single sanctioned
immediate effect is the cloud-consent _withdrawal_ (§4 row 8): it closes a
data-flow risk and therefore must not wait for a session boundary (SPC §2.2
rule 5 owns its semantics; this document only classifies it).

---

## 4. Complete classification table (gate G1)

Columns: **Setting** · Owner · Primary class · UI enforcement seam · Backend
enforcement seam · Directional exceptions.

| #   | Setting                                                                                                             | Owner                           | Class                               | UI enforcement seam                                                                                                                                                                                       | Backend enforcement seam                                                                                                                                                                                                                                              | Exception                                                                                                                                                                   |
| --- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------- | ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `meeting_stt_provider` (`Local/Azure/Custom`)                                                                       | CC §6.1                         | locked-while-recording              | Live view status row read-only (plan §6.11); settings write during session marked "ab nächstem Meeting"                                                                                                   | Session snapshot column (SSC T01) is the sole read source (CC §6.3, DCD H1); apply attempt rejected per §5                                                                                                                                                            | none                                                                                                                                                                        |
| 2   | `meeting_model_id`                                                                                                  | CC §6.1                         | locked-while-recording              | as row 1                                                                                                                                                                                                  | as row 1; local worker lease re-acquisition compares against the snapshot, not live settings (CC §3.5 step 2)                                                                                                                                                         | none                                                                                                                                                                        |
| 3   | `meeting_language`                                                                                                  | CC §6.1                         | locked-while-recording              | as row 1                                                                                                                                                                                                  | as row 1; resolved language frozen in snapshot at start                                                                                                                                                                                                               | none                                                                                                                                                                        |
| 4   | `meeting_selected_microphone`                                                                                       | CC §6.1                         | locked-while-recording              | Live view shows resolved device read-only; mid-session changes happen **only** via RC §7.1 actions (_Wiederverbinden_/_Gerät wechseln_), which are recorder operations, not settings propagation (DCD H3) | DCD H1/H4: capture reads snapshot exclusively; switch = validate-before-close `open_source`, never a settings read                                                                                                                                                    | none                                                                                                                                                                        |
| 5   | `meeting_selected_output_device`                                                                                    | CC §6.1                         | locked-while-recording              | as row 4 (_Loopback neu öffnen_/_Ohne Systemaudio fortführen_ are RC §7.1 actions, DCD H3)                                                                                                                | as row 4 for `SourceKind::System`                                                                                                                                                                                                                                     | none                                                                                                                                                                        |
| 6   | `meeting_follow_default_output`                                                                                     | CC §6.1                         | locked-while-recording              | as row 1                                                                                                                                                                                                  | as row 1; the follow flag is snapshotted, so mid-session flips cannot re-bind streams (DCD M8/S1–S2 semantics)                                                                                                                                                        | none                                                                                                                                                                        |
| 7   | Summary choice of the session (`none \| local \| cloud`, start dialog)                                              | SPC §2.2 rule 3                 | locked-while-recording              | Start dialog is the only writer; processing checklist shows the pinned choice read-only                                                                                                                   | Pinned in the session snapshot; a session started local/none never becomes cloud mid-flight (SPC §2.2 rule 3)                                                                                                                                                         | none                                                                                                                                                                        |
| 8   | `meeting_summary_cloud_consent` (bool)                                                                              | SPC §2.1                        | apply-next-session                  | Settings toggle always enabled; enabling shows no "pending" marker (it never affects a running session)                                                                                                   | Enabling passes only future validations/re-summarize runs (SPC §2.2 rules 2–3)                                                                                                                                                                                        | **hot-apply on withdrawal**: turning the flag off takes effect at the next M1/M2 stage gate evaluation; it never aborts a running cloud job retroactively (SPC §2.2 rule 5) |
| 9   | Per-session audio retention policy (`Unanswered/KeepPermanent/DeleteImmediately/After7Days/After30Days`, AS §8.1)   | AS §8.1, RQC §4                 | locked-while-recording              | Chosen once in the start dialog (first-meeting question per plan §12); not editable in the live view                                                                                                      | Stored on the session record (`recording_retention_policy`); RQC evaluates exclusively against the stored per-session value — a global-default change can never reach a stored session row                                                                            | none                                                                                                                                                                        |
| 10  | Global retention default (the answer to the plan §12 first-meeting question, applied to new sessions)               | plan §12; RQC §4                | apply-next-session                  | Normal settings control; changing it mid-session needs no marking (it seeds only future sessions' dialog default)                                                                                         | Applied only when a new session row is created (SSC T01); RQC never reads it for existing rows                                                                                                                                                                        | none                                                                                                                                                                        |
| 11  | `meeting_toggle` binding (bindings map id)                                                                          | CC §7.1                         | apply-next-session                  | Shortcut editor freely editable mid-meeting; editor shows no lock                                                                                                                                         | Global rebind allowed anytime (CC §7.5); the running session resolves stop/countdown handlers through the session-start binding snapshot (CC §6.3 last bullet, §7.5); if the old binding is deleted mid-meeting, stop stays reachable via UI/tray (CC §7.5 edge rule) | none                                                                                                                                                                        |
| 12  | `meeting_pause` binding (reserved id)                                                                               | CC §7.1                         | apply-next-session                  | as row 11                                                                                                                                                                                                 | as row 11                                                                                                                                                                                                                                                             | none                                                                                                                                                                        |
| 13  | Dictation-path settings (`selected_model`, `selected_microphone`, `selected_language`, post-process providers/keys) | existing code (phase0 baseline) | hot-apply — **dictation tool only** | Unchanged legacy dictation settings UI                                                                                                                                                                    | Dictation keeps its per-run `get_settings()` behavior (CC §6.3 bullet 4)                                                                                                                                                                                              | Hard boundary: never read by meeting capture or the meeting snapshot (CC §6.2 hard rule; DCD H5 for `selected_output_device`)                                               |

Totality claim: rows 1–13 enumerate every meeting-relevant key defined by the
owning contracts as of this document — CC §6.1 (rows 1–6, 11–12), SPC §2
(rows 7–8), AS §8.1 + plan §12 (rows 9–10), plus the dictation boundary row 13
required by CC §6.3/DCD H5. Overlay/tray and other UX keys are deliberately
absent: they belong to the UX phase contracts (CC §6.1 closing paragraph) and
must be added here when those contracts define them.

### 4.1 Enforcement-seam summary by class

| Class                  | UI enforcement seam                                                                                                                                         | Backend enforcement seam                                                                                                                                                          |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| hot-apply              | The owning contract's own surface carries the effect (row 8 withdrawal needs no settings UI change; row 13 is the unchanged legacy dictation UI)            | The named gate/boundary of the owning contract (SPC §2.2 rule 5 stage gate; dictation per-run `get_settings()`, CC §6.3 bullet 4)                                                 |
| apply-next-session     | Normal settings controls, no lock/marking duty beyond the standard save acknowledgment                                                                      | Existing typed-settings persistence path (`AppSettings`, tauri-plugin-store); binding at next session's Startvalidierung/snapshot (SSC T01) or shortcut re-registration (CC §7.5) |
| locked-while-recording | Live-view control plane renders snapshot values read-only; `Für nächstes Meeting ändern` link; pending-change marking "gilt ab dem nächsten Meeting" (§5.1) | Snapshot authority as sole read source + command guard returning `meeting.session.settings_locked_while_recording` (§5.2–§5.3)                                                    |

---

## 5. Enforcement of locked-while-recording (gate G2)

### 5.1 UI enforcement point

One seam: the **live-view control plane** (planned
`src/components/meetings/`, F-layer per module-placement-map). Normative:

- Values of locked rows (1–7, 9) render from the session snapshot query
  (`get_meeting_snapshot`, SSC O5), never from live settings stores.
- No control in the live view writes a locked key (plan §6.11; DCD H2 for
  devices, generalized here).
- The only escape hatch is the link `Für nächstes Meeting ändern`
  (plan §6.11), which navigates to settings without touching the snapshot.
- Settings pages that write a locked key while a session records mark the
  pending change "gilt ab dem nächsten Meeting" (DCD M6/S6 consumer column,
  generalized to all locked rows).

### 5.2 Backend enforcement point

Two seams, both already mandated by owning contracts — this document binds
them to the classification, adding no mechanism:

1. **Snapshot authority (primary).** After successful start, the running
   session reads locked rows (1–7, 9) exclusively from the immutable session
   snapshot
   (CC §6.3; SSC T01 snapshot columns; DCD H1). This makes deferred behavior
   structural: there is no code path through which a settings write could
   reach the session, so most violations are unrepresentable rather than
   rejected.
2. **Command guard (secondary, for explicit apply attempts).** Any Tauri
   command whose semantics are "apply this setting value to the active
   session" (e.g. a future remote-control or CLI path attempting a mid-session
   provider/model/device swap) validates the target session state and, while
   a session is recording, returns the error envelope instead of acting.

### 5.3 Error code (declared new, within the closed namespace)

Following TAX §3 (format `meeting.<origin>.<snake_case_name>`; origin set
`{recorder, session, stt, summary}` is **not** extended — origin `session` is
reused):

```jsonc
{
  "code": "meeting.session.settings_locked_while_recording",
  "params": { "settingKey": "meeting_stt_provider" },
}
```

| Property     | Value                                                                                                               | Basis                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| Severity     | `warning`                                                                                                           | progress intent refused; recording unaffected (TAX §4.1) |
| Action class | `none` (primary), `repair` (`openSettings` for next-session change)                                                 | TAX §4.2                                                 |
| Auto-retry   | `n/a` (session-origin codes are never auto-retried, TAX §4.3)                                                       | TAX §4.3                                                 |
| Copy key     | NEW family `meeting.errors.settingsLocked.*` (`title`, `body`), hand-off to the catalog owner per TAX §6 convention | —                                                        |

Registration duty (same pattern as prior contracts' NEW-key hand-offs): this
row must be added to TAX §5 and flagged for the next identifier-registry pass
before the code may be emitted by any implementation; until then the code is
reserved, not registered. The primary UI experience for locked rows is
disabled/read-only controls (§5.1), so normal users should never see this
code — it exists for programmatic callers and defense in depth, mirroring how
RC guards (`SessionAlreadyActive`) are dev-facing.

---

## 6. Snapshot-isolation rules restated, not altered (gate G3)

Restated verbatim-consistent from CC §6.3 (authority remains CC; this section
adds nothing and overrides nothing):

1. At successful meeting start, `MeetingManager` writes an immutable snapshot
   into the session record: provider enum + non-secret provider configuration,
   `meeting_model_id`, resolved effective language, resolved mic/output device
   identities, `has_microphone_audio`/`has_system_audio`, retention policy.
2. Secrets are never copied into the snapshot; only references through the
   existing secret store (consistent with `SecretMap` serializing as `{}`,
   types.rs lines 323–337).
3. After start, changes to global settings — including "Für nächstes Meeting
   ändern" — affect the **next** meeting only; the running session reads
   provider/model/device parameters exclusively from its snapshot.
4. The dictation side keeps its existing live-settings behavior unchanged; no
   snapshot is introduced for dictation.
5. Shortcut binding ids are captured at session start, so a mid-meeting
   rebinding cannot alter countdown/stop semantics of the running session
   (CC §7.5).

And from DCD §3, unchanged: H1 (snapshot binding incl. pause/resume
re-validation), H2 (no live-view writes), H3 (RC §7.1 actions are the one
sanctioned mid-session change path — user decisions, not settings
propagation), H4 (validate-before-close), H5 (`selected_output_device` keeps
its feedback-sound meaning). This policy classifies settings; it does not
create any second route around H1–H5.

Consequence for the classification's integrity: the CC §6.4 independence
matrix (changes during recording have "none" effect on the running meeting)
holds for every locked row of §4 by construction, because the snapshot is the
only read source.

---

## 7. Complexity budget & non-goals (gate G4)

Deliberately **not** specified, with reasons:

- **No generic "settings watcher"/reactive-binding framework** that routes
  per-key apply policies. Thirteen rows resolve to three classes enforced at
  two seams; a registry pattern would add indirection with no behavioral
  difference.
- **No hot-apply for capture/inference keys, no mid-session model or provider
  switching via settings.** Mixed-provenance segments and snapshot bypasses;
  the only sanctioned mid-session device/model changes are the RC §7.1
  recorder actions (DCD H3), which are deliberate user decisions with
  validate-before-close semantics, not settings mutations.
- **No retroactive consent revocation** of running cloud jobs, and no
  retroactive retention tightening/loosening of recorded sessions. Withdrawal
  semantics stay owned by SPC §2.2 rule 5; retention by RQC §4.
- **No per-user/per-profile setting layers, no import/export of settings
  profiles, no admin policy store** — SAP-like configurability rejected;
  single flat typed `AppSettings` plus session snapshots (SPC §2.1 home rule).
- **No second settings store** for meetings; all keys live in the existing
  typed struct (CC §6.1 conventions; SPC §2.1 "no second settings store").
- **No new setting keys, no new events, no new i18n families** beyond the one
  reserved error code (§5.3) and its one copy-key family; no changes to the
  dictation path (AGENTS/i18n rules still govern the eventual copy keys).
- **No editing of the in-flight settings refactor**: this task read
  `src-tauri/src/settings/types.rs` only; planned additions remain in their
  owning contracts (CC §6.1, SPC §2.1).

## 8. Gate coverage

| Gate                                                          | Where satisfied                                                                                                                                                                                 |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 complete classification + named enforcement seam per class | §2 (class definitions + decision rule), §4 (total table, UI + backend seam columns, totality claim)                                                                                             |
| G2 defined error for locked attempts                          | §5 (UI seam, backend seams, reserved code `meeting.session.settings_locked_while_recording` within the closed TAX namespace, with severity/action/retry/copy-key mapping and registration duty) |
| G3 snapshot-isolation restated, not altered                   | §6 (verbatim-consistent restatement of CC §6.3 items 1–5 and DCD H1–H5, with authority pointers)                                                                                                |
| G4 non-goals + index updated                                  | §7; table row and reading-order entry added in `docs/meeting-mode-documentation-index.md`                                                                                                       |

## Appendix A — Consistency cross-checks performed

- DCD M6/S6 ("capture effect none, gilt-ab-nächstem-Meeting marking") ⊨ §4
  rows 4–6 class assignment; no contradiction with H1–H5.
- CC §7.5 ("shortcut module free to rebind globally; session resolves through
  snapshot") ⊨ §4 rows 11–12 apply-next-session assignment.
- SPC §2.2 rules 3 and 5 ⊨ §4 rows 7–8, including the single directional
  hot-apply exception.
- RQC §4 (evaluation reads only the stored per-session policy) ⊨ §4 rows 9–10.
- CC §6.3 bullet 4 (dictation stays live-settings) ⊨ §4 row 13 with the hard
  meeting-capture boundary from CC §6.2/DCD H5.
