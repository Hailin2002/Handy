# Output-Channel Privacy Audit — Clipboard, Paste Injection, Secure Input

- **Task:** task-0073 (Security Engineering leaf) — read-only audit
- **Scope:** `src-tauri/src/clipboard.rs`, `src-tauri/src/input.rs`, `src-tauri/src/secure_input.rs`, `src-tauri/src/signal_handle.rs`, plus the flow into them from `src-tauri/src/actions.rs` and the receipt-sequenced path in `src-tauri/src/paste_tx/` (read for evidence, not modified)
- **Out of scope (declared conflicts respected):** history/storage of transcripts, secret storage (covered by task-0069 audit), meeting-mode backend design
- **Source modifications:** none. This document is the sole artifact added by this task.
- **Audit date / basis:** 2026-08-22, worktree `task-0073-attempt-1`. All `file:line` citations verified by reading the files in this worktree.

---

## 0. Executive summary

The dictation output path round-trips every transcript through the system clipboard. In the **legacy** clipboard-paste path the transcript sits as plain, unmarked clipboard text for a ≥ ~220 ms window per utterance, with **no crash handler that restores or clears it**, silent swallowing of restore failures, permanent destruction of any non-text/non-image clipboard content, and **no clipboard-history exclusion markers** — so clipboard managers (Windows Win+V, Ditto, macOS clipboard managers) can retain transcript fragments indefinitely. The **receipt-sequenced "reliable paste"** path (default on Windows) fixes most of this for the pending window (delayed render + history/cloud-exclusion markers + sequence-guarded restore) but still has a kill-window and partial-restore failure modes.

For multi-hour, conversationally sensitive meeting transcripts, none of the current channels is acceptable without change: there is no user-visible target confirmation, no way to avoid the clipboard round-trip for full transcripts except per-utterance `PasteMethod::Direct`/`None`, and no cleanup guarantee after crashes. Requirements are listed in §6, split into current-path fixes and meeting-mode contracts tied to copy-catalog entries.

---

## 1. End-to-end text flow table (Gate G1)

Flow: transcription result → output dispatch → clipboard/injection → target application focus.

### 1.1 Main success path

| # | Step | Location | What happens |
|---|---|---|---|
| 1 | Recording stops, samples retrieved | `actions.rs:742` | `rm.stop_recording(&binding_id, cancel_generation)` returns samples on an async task spawned at `actions.rs:734`. |
| 2 | Transcription / post-process → `final_text` | `actions.rs:839-869` | `transcription_result` → `process_transcription_output_with_translation` → `processed.final_text`. |
| 3 | Empty-text early return | `actions.rs:891-893` | `final_text.is_empty()` → overlay hidden, tray idle, **no output channel touched**. |
| 4 | Paste dispatched on main thread | `actions.rs:899-907` | `ah.run_on_main_thread(...)`; final cancellation check at `actions.rs:900-905`; then `utils::paste(final_text, …)` — which is `clipboard::paste` re-exported at `utils.rs:41`. |
| 5 | Settings read, trailing space | `clipboard.rs:765-776` | Reads `paste_method`, `paste_delay_ms`, `paste_delay_after_ms`, `append_trailing_space`, `auto_submit`, `clipboard_handling`. |
| 6a | Method `None` | `clipboard.rs:785-787` | Early return; text goes nowhere. |
| 6b | Method `Direct` | `clipboard.rs:788-794` → `paste_direct` `clipboard.rs:702-716` | Linux native tools first (`try_direct_typing_linux`, `clipboard.rs:162-238`), else enigo `text()` (`input.rs:268-274`). **No clipboard involvement.** |
| 6c | Chord methods (`CtrlV`/`CtrlShiftV`/`ShiftInsert`) — reliable branch | `clipboard.rs:802-821` → `paste_tx::try_reliable_paste` (`mod.rs:194-212`) | Windows/macOS only, gated on `settings.reliable_paste` (default `cfg!(windows)`, `schema.rs:300-302`). Publishes delayed-render promise + exclusion formats (`windows.rs:357-411`), waits for publish on worker thread (`windows.rs:601-605`), injects chord (`windows.rs:611-621`). On `Err` falls through to legacy untouched (`clipboard.rs:817-820`). |
| 6d | Chord methods — legacy branch | `clipboard.rs:822-828` → `paste_via_clipboard` (`clipboard.rs:55-118`) | Save prior text/image (`clipboard.rs:63-71`) → write transcript to clipboard (`clipboard.rs:74`) → sleep `paste_delay_ms` (`clipboard.rs:76`, default 60 ms, `schema.rs:292-294`) → send chord via enigo (`clipboard.rs:90-97`, modifier hold 100 ms at `input.rs:196/228/257`) → sleep `paste_delay_after_ms` (`clipboard.rs:49`) → restore prior content (`clipboard.rs:103-117`). |
| 6e | Method `ExternalScript` | `clipboard.rs:830-837` → `paste_via_external_script` (`clipboard.rs:678-699`) | Transcript passed as argv to external script. |
| 7 | Auto-submit Enter | `clipboard.rs:840-847` | If enabled, sleeps 50 ms then presses Return/Ctrl+Return/Cmd+Return via enigo. Failures logged, not fatal (`clipboard.rs:845`). |
| 8 | ClipboardHandling write-back | `clipboard.rs:850-852` | `ClipboardHandling::CopyToClipboard` re-writes the transcript to the clipboard **after** restore/auto-submit; `DontModify` (default, `types.rs:158-164`) does nothing extra. |
| 9 | Teardown | `actions.rs:908-918` | Success/failure logged; `"paste-error"` event emitted on failure (`actions.rs:914`); overlay hidden, tray idle. |

Target-application focus: never queried. The chord is injected into whatever window has keyboard focus at injection time; `paste_delay_ms` is the only focus-settling mechanism (`clipboard.rs:76`).

### 1.2 Every early-return / error path between result and output

| Path | Location | Effect on clipboard / transcript |
|---|---|---|
| Cancelled after stop | `actions.rs:749-756` | Return before any output. Clipboard untouched. |
| Empty samples | `actions.rs:758-765` | Same. |
| Cancelled before output handling / before paste | `actions.rs:832-837`, `871-876`, `900-905` | Same — all precede `utils::paste`. |
| Transcription error | `actions.rs:927-949` | Emits `transcription-error`; no output channel touched. |
| `run_on_main_thread` fails | `actions.rs:920-924` | Paste closure never runs; error logged only. |
| `paste_via_clipboard`: clipboard write fails | `clipboard.rs:74` (`?`) | Early return **before** anything was overwritten; saved values discarded; clipboard unchanged. Correct but note: no retry, user gets `paste-error`. |
| Key-injection error (enigo or Linux tool) | captured at `clipboard.rs:80-101`, restored at `103-117`, propagated via `finish_clipboard_paste` (`44-52`) | Restore runs **before** the error propagates (unit-tested at `clipboard.rs:959-967`). But see §2 finding F3: restore failures themselves are swallowed. |
| Reliable paste unavailable at startup | `clipboard.rs:817-820` | Falls back to legacy `paste_via_clipboard` — i.e. the weaker privacy properties of §2 apply silently whenever the worker thread/window creation fails (`windows.rs:501-555`). |
| Reliable paste: chord injection fails after publish | `windows.rs:615-620` | Transaction stays alive; worker restores after `FAILED_INJECTION_TIMEOUT` = 500 ms (`mod.rs:63`, `evaluate` at `mod.rs:148-152`). Restore is guarded by sequence number (`windows.rs:463-469`). |
| External script not configured | `clipboard.rs:831-835` | Error returned; clipboard untouched. |
| Auto-submit fails after successful paste | `clipboard.rs:842-847` | Logged warning; transcript already delivered. |
| CopyToClipboard write-back fails | `clipboard.rs:850-852` (`?`) | Error propagates to `paste-error` even though the paste itself succeeded. |

---

## 2. Clipboard-loss window characterization (Gate G2)

### 2.1 Legacy path (`paste_via_clipboard`, `clipboard.rs:55-118`)

The exposure window opens at the transcript write (`clipboard.rs:74`) and closes when the restore closure finishes inside `finish_clipboard_paste` (`clipboard.rs:49-51`). Typical duration ≈ `paste_delay_ms` (60 ms default) + chord hold (100 ms, `input.rs:196/228/257`) + `paste_delay_after_ms` (60 ms default) ≥ ~220 ms per utterance, longer under enigo-lock contention (`with_enigo`, `clipboard.rs:17-29`) or scheduling stalls.

Stranding / loss points, precisely:

- **F1 — Process kill or crash inside the window strands the transcript and destroys the user's clipboard.** There is no panic hook, `SetUnhandledExceptionFilter`, or exit handler that restores or clears the clipboard (grep over `src-tauri/src` for panic hooks / unhandled-exception filters: no hits). A crash anywhere between `clipboard.rs:74` and the restore leaves the full transcript as plain clipboard text and the user's prior content permanently gone.
- **F2 — Panic unwinding skips the restore.** The restore closure is invoked by `finish_clipboard_paste` *after* the injection closure returns normally (`clipboard.rs:49-51`); a panic inside `with_enigo`/enigo unwinds past it. (Same class as F1 but does not even require process death.)
- **F3 — Restore failure is swallowed silently.** All three restore branches use `let _ =` (`clipboard.rs:109,112,115`): if another application holds the clipboard open during restore, the write fails with **no log, no retry, and no error to the caller** — the paste reports success while the user's clipboard content is lost and the transcript remains on the clipboard. This is the highest-frequency real-world loss point because clipboard managers and remote-desktop sessions transiently lock the clipboard.
- **F4 — Format-fidelity loss even on success.** Only UTF-8 text (`clipboard.rs:63`) or a single image (`clipboard.rs:67-68`) is saved; everything else the user had copied (files, HTML with styles, custom formats, cloud-synced rich content on Windows) is destroyed by the plain-text write at `clipboard.rs:74` and never restored. The reliable path's full-format snapshot (`windows.rs:298-352`) demonstrates this is solvable; the legacy path simply doesn't.
- **F5 — No clipboard-history exclusion in the legacy path.** The transcript is written as ordinary text with none of the markers Chrome uses for private copies. Contrast the reliable path, which sets `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=0`, `CanUploadToCloudClipboard=0` (`windows.rs:370-395`). Consequence: Win+V history, Ditto/ClipClip, and Windows cloud clipboard can permanently retain dictation fragments — including anything dictated in the seconds before a meeting starts. This is a standing privacy leak, not just a crash-window risk.
- **F6 — `ClipboardHandling::CopyToClipboard` intentionally persists the full transcript** to the clipboard (`clipboard.rs:850-852`), and therefore to every history manager, with no confirmation and no expiry. It also runs *after* auto-submit (`clipboard.rs:840-852`), so Enter may already have been sent when the copy-back happens. Opt-in, but the privacy cost is nowhere surfaced.

### 2.2 Receipt-sequenced path (Windows default; macOS analog)

Default on Windows (`schema.rs:300-302`, `types.rs:201-215` makes `ShiftInsert` the default method). Properties: delayed-render promise instead of real data (`windows.rs:405`), exclusion markers at publish (`windows.rs:373-395`), restore guarded by ownership-lost + clipboard sequence number so a newer user copy always wins (`windows.rs:463-469`, `WM_DESTROYCLIPBOARD` at `179-186`), bounded occupancy (`RESTORE_TIMEOUT` 8 s, `QUIET_PERIOD` 200 ms, `mod.rs:53-59`), settle-time `CopyToClipboard` handling replaces the promise with plain text (`windows.rs:246-259`).

Remaining stranding points:

- **R1 — Kill window.** Process death between publish (`windows.rs:561-562`) and settle (timer-driven, ≤ ~8 s) kills the owner window without `WM_RENDERALLFORMATS`; the promise becomes unreadable and nothing restores the snapshot → user's prior clipboard effectively emptied until their next copy. If the target already read the transcript, `render_text` placed real plain text in the clipboard (`windows.rs:119-138,148-159`) which now stays until overwritten. Shorter than the legacy window but nonzero and unguarded.
- **R2 — Partial snapshot restore.** `restore_snapshot` continues past individual `GlobalAlloc`/`SetClipboardData` failures (`windows.rs:273-287`) and warns-only if the clipboard cannot be opened (`windows.rs:263-265`): the clipboard may end up holding a subset of the user's original formats, or empty, with only a log line.
- **R3 — Graceful teardown renders the transcript as real data.** `WM_RENDERALLFORMATS` deliberately writes the full transcript when the window is destroyed with a pending promise (`windows.rs:160-177`). Combined with R1-class timing this means the transcript is materialized exactly when the process is going away — the exclusion markers remain on the clipboard, which mitigates history capture but is not guaranteed across third-party managers.
- **R4 — `flush_pending` cancels a previous transaction without restoring** when a new paste starts (`windows.rs:216-240`); correct-by-design (the new transaction snapshots afterwards), but note the previous transcript's promise is abandoned — again relying on marker formats being honored.

macOS mirrors this design (`paste_tx/macos.rs:104-176`, guarded by `changeCount`), not independently exercised in this audit.

### 2.3 Timing windows in input injection

- **T1 — Mistimed chord pastes the wrong content (#502).** The legacy path restores the clipboard on a timer while the target reads asynchronously; a slow target pastes the user's *old* clipboard content into their document (`paste_tx/mod.rs:5-7`). Privacy-inverse leak: unrelated prior clipboard content lands where the user expected the transcript.
- **T2 — Wrong-target injection.** Nothing verifies the focused window between transcription completion (`actions.rs:899`) and injection; a focus change during the 60 ms delays delivers the full transcript into whatever gained focus. With auto-submit enabled (`clipboard.rs:840-847`), an Enter follows ~50 ms later — a misdirected transcript can be *sent* (chat message, form submit), not merely typed.
- **T3 — Partial direct typing.** `PasteMethod::Direct` types character streams with no rollback: enigo `text()` (`input.rs:268-274`) and the Linux tools (`xdotool type …`, `clipboard.rs:415-471`) leave a prefix of the transcript in the target on mid-stream failure; the error surfaces only afterwards (`paste-error`). A half-transcript stranded in an unrelated editor is undeletable by Handy.
- **T4 — Modifier races with physical typing.** Documented for xdotool (#1817, `clipboard.rs:424-433`); enigo paths hold modifiers for 100 ms (`input.rs:196`) during which the user's own keystrokes combine with synthetic modifiers.

---

## 3. What `secure_input.rs` actually guards (Gate G3)

**Purpose: shortcut availability, not confidentiality.** The module doc states its scope explicitly: macOS Secure Event Input (password fields, Terminal "Secure Keyboard Entry", stuck `loginwindow`) stops CGEventTap-based *listening* for keyed shortcuts, so Handy's recording shortcuts die silently (`secure_input.rs:1-19`, issue #1578). It contains **no logic that touches the clipboard, transcripts, or output channels** — grep confirms zero references to `secure_input::*` from `clipboard.rs`, `actions.rs`, or `input.rs`.

Coverage, with citations:

- Detection: polls `IsSecureEventInputEnabled()` (Carbon link at `secure_input.rs:140-147`) every 1 s (`POLL_INTERVAL`, `secure_input.rs:135`) in a monitor thread (`secure_input.rs:298-375`).
- Sustain threshold: 3 s before treating it as "stuck" rather than a momentary password field (`SUSTAIN_THRESHOLD`, `secure_input.rs:138`).
- Mitigation: shadow-registers vulnerable *keyed* bindings through the Carbon-backed Tauri global-shortcut path, immune to secure input (`register_fallback_binding`, `secure_input.rs:412-486`; reconciliation `secure_input.rs:491-567`). Side-specific modifiers are widened (degraded, `carbon_equivalent`, `secure_input.rs:387-408`); fn-key bindings cannot be covered (`secure_input.rs:390-392`).
- Lifecycle: dynamic Cancel-binding shadow during recordings (`secure_input.rs:576-594`, wired from `shortcut/mod.rs:62,73`).
- Diagnostics: count-only keyboard listener report — key identity deliberately never captured (`secure_input.rs:600-642`, doc `17-19`).

**Platform gaps:**

- **Non-macOS stubs.** The entire `imp` for Windows/Linux is a stub: `is_enabled()` always false, no monitor, status all-zeros, diagnostic errors ("only supported on macOS") (`secure_input.rs:645-691`). There is no Windows equivalent of Secure Event Input to poll, so this is a documented platform asymmetry rather than a missing port — but it means the debug diagnostics and tray warning surface exist on macOS only.
- **Relevance gap for output privacy.** Secure Event Input blocks event *taps* (listening); it does not block synthetic key *posting*. Neither the macOS nor any platform paste path consults secure-input state, so Handy will paste transcripts directly into password fields or other secure-input contexts with no guard. For meeting-mode this matters: there is currently no mechanism — secure-input-related or otherwise — preventing sensitive transcript delivery into credential fields.
- `signal_handle.rs` (in scope): purely a Unix SIGUSR1/SIGUSR2 → `send_transcription_input` bridge (`signal_handle.rs:18-57`); it triggers recordings but handles no text, clipboard, or secrets. Its only security-relevant property is that SIGUSR1 is deliberately not handled on Linux due to WebKitGTK GC collisions (#1660, `signal_handle.rs:26-33`) — correctness note, no privacy impact.

---

## 4. Meeting-mode impact assessment

Meeting transcripts differ from dictation fragments in three ways that break current assumptions:

1. **Volume and duration.** Multi-hour conversational text maximizes both the number of legacy exposure windows (one per utterance, §2.1) and the damage of a single one: a stranded multi-KB transcript is far more sensitive than a sentence. `PasteMethod::Direct` avoids the clipboard but makes T3 (partial typing) worse at length.
2. **Conversational sensitivity.** Third-party speech with consent obligations (copy catalog P1/P2). Persistent retention by clipboard managers (F5, F6) defeats the app-side deletion guarantees the catalog promises (P8–P10 delete levels delete *app* data; they cannot recall Win+V history or Ditto archives).
3. **No natural "target".** Dictation has a focused target; meetings produce a transcript whose destination is the meeting view, not the focused window. Every existing output channel assumes focus — using any of them for full meeting transcripts would create exactly the T2 wrong-target hazard, amplified by auto-submit.

Verified against the plan and catalog: `docs/meeting-mode-plan.md` contains **no mention** of clipboard/Zwischenablage/output channels (grep: 0 hits for `Clipboard|Zwischenablage|output channel|Paste`), and the copy catalog (`docs/meeting-mode-ux-copy-catalog.md`) has no output-channel entries — its closest anchors are P1/P2 (consent), P7 (stop confirmation), P8–P10 (deletion scopes), E12 (crash recovery), and S8 (`completed.saved` = "Transcript and summary are stored on this device", which would be false if a transcript also lives in OS clipboard history).

---

## 5. Complexity budget / non-goals

Rejected as low-value bloat (per the mission's complexity budget):

- Per-application paste allow-lists or focus-content inspection before injection (fragile, accessibility-permission-heavy, solves T2 only marginally — a visible target confirmation is cheaper and honest).
- Restoring arbitrary multi-format clipboard fidelity in the *legacy* path (superseded by steering users to the reliable path; duplicating `paste_tx`'s snapshotter twice is scope bloat).
- Cross-platform secure-input emulation on Windows (no OS concept to emulate; the honest fix is documenting the asymmetry, G3).
- Encrypted in-app clipboard alternatives (any local "private clipboard" still loses to OS-level managers reading the real clipboard; the win is in *not using the clipboard*, i.e. M3 below).

---

## 6. Requirements (Gate G4)

Severity scale: **High** = plausible conversationally-sensitive disclosure or irreversible loss; **Medium** = recoverable loss or opt-in exposure; **Low** = hygiene.

### 6.1 Current-path fixes (dictation, existing code)

| ID | Requirement | Severity | Evidence |
|----|-------------|----------|----------|
| C1 | Log and surface restore failures: replace `let _ =` at `clipboard.rs:109,112,115` with logged warnings and a retry-once; emit `paste-error` (or a dedicated event) when the user's clipboard could not be restored. | High | F3, §2.1 |
| C2 | Install a best-effort crash/exit hook that, while a clipboard transaction is open, restores the last snapshot or clears the clipboard (covers F1/F2 for the legacy path and R1 for the reliable path at least on graceful unwind). | High | F1, F2, R1; grep shows no such hook exists |
| C3 | Set clipboard-history/cloud-exclusion markers (same formats as `windows.rs:373-395`) on the legacy write path too, or make the reliable path non-debug-gated so the legacy chord path becomes exceptional on Windows/macOS. | High | F5 |
| C4 | Preserve full format fidelity in the save/restore (reuse the `paste_tx` snapshot approach) instead of text-or-single-image. | Medium | F4 |
| C5 | Document the `CopyToClipboard` setting's retention consequence in settings UI copy (transcript persists in OS clipboard history; deletion in Handy cannot recall it). | Medium | F6 |
| C6 | Guard auto-submit: require same-window continuity (or at minimum make auto-submit opt-in per surface) so Enter cannot transmit a misdirected transcript. | High | T2 |
| C7 | Verify target focus immediately before chord injection and abort-with-toast on focus change during the delay window. | Medium | T2 |
| C8 | State the secure-input platform asymmetry in troubleshooting docs: detection/fallback is macOS-only (`secure_input.rs:645-691`); no platform guards transcript entry into password fields. | Low | §3 |

### 6.2 Meeting-mode contracts (new, tied to copy-catalog entries)

| ID | Contract | Severity | Catalog anchor |
|----|----------|----------|----------------|
| M1 | **Explicit user-visible target confirmation for meeting output**: any action that exports meeting transcript text outside the app names the destination and requires confirmation (i18n proposal: `meeting.output.confirm.title/body`, button reuse `meeting.actions.close` / new `meeting.output.confirm.send`). Never inject into the focused window implicitly. | High | extends P7 pattern (confirmation-before-consequence) |
| M2 | **No silent clipboard round-trip for full transcripts**: meeting export must not route the complete transcript through the system clipboard by default; use explicit copy-to-clipboard only on a per-section, user-initiated action, with the exclusion markers of `windows.rs:373-395` applied. (i18n: `meeting.output.copiedSection`.) | High | extends P8–P10 (deletion must be able to keep its promise) |
| M3 | **Cleanup guarantee**: if any clipboard transaction involving meeting text is interrupted (crash, kill, force-quit), next launch must detect it (e.g. persisted transaction journal like the recovery banner concept) and offer clear-or-restore (i18n: `meeting.output.cleanupNotice`), consistent with E12's "nothing is silently lost/orphaned" stance. | High | pairs with E12 |
| M4 | **Retention honesty**: S8's "stored on this device" must be accompanied by an output-channels caveat wherever transcript text left copies in OS-managed stores (clipboard history) that Handy cannot delete (i18n: `meeting.output.retentionCaveat`). | Medium | amends S8 |
| M5 | **Password-field guard**: refuse paste/injection targets while a password field or (macOS) sustained secure input is detected; show `meeting.output.blockedSecureField` instead of injecting. | Medium | extends §3 gap |
| M6 | Non-goal reaffirmation: no automatic export of meeting transcripts to arbitrary focused applications; meeting output lives in the meeting view, full stop. | — | §5 |

---

## 7. Gate self-check

| Gate | Evidence |
|------|----------|
| G1 — end-to-end table with file:line incl. success + each early-return | §1.1 (steps 1–9), §1.2 (11 error/early-return paths), all cited |
| G2 — precise characterization of clip-loss windows | §2.1 F1–F6, §2.2 R1–R4, §2.3 T1–T4, with exact lines and timing constants |
| G3 — secure_input purpose/coverage + platform gaps | §3, cited module-wide incl. non-macOS stubs and the output-path relevance gap |
| G4 — requirements split current-path vs meeting contracts, severity, zero source mods | §6.1 C1–C8, §6.2 M1–M6; only file changed is this document |

Final improvement pass: re-read all four in-scope files and every citation after drafting; no remaining material corrections found.
