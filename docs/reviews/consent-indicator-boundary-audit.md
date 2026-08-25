# Consent & Visible-Indicator Boundary Audit — All Non-UI Recording Entry Points

Role: Security Engineering (read-only audit)
Scope honored: `src-tauri/src/cli.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/signal_handle.rs`, `src-tauri/src/transcription_coordinator.rs`, `src-tauri/src/actions.rs`, `src-tauri/src/shortcut/*`, `src-tauri/src/tray.rs`, `src-tauri/src/managers/audio.rs`, `docs/reviews` (this file). No code was modified; findings only.
Requirement audited: `docs/meeting-mode-plan.md` lines 1458–1461 — recording non-public conversations requires consent; a visible indicator must be shown for the entire recording; the feature must not be usable as a stealth recorder.

## 1. Enumeration of every recording entry point (Gate G1)

All paths below were verified in the current worktree source. "Reaches capture?" means: can this path cause `AudioRecordingManager::try_start_recording` (microphone capture into the transcription buffer)?

| # | Entry point | Location | Mechanism | Reaches capture? | Consent/indicator gate on path? |
|---|---|---|---|---|---|
| 1 | Global shortcut `transcribe` / `transcribe_with_post_process` (Tauri implementation) | `src-tauri/src/shortcut/tauri_impl.rs:106–125` → `shortcut/handler.rs:29–45` | `on_shortcut` callback → `handle_shortcut_event` → `TranscriptionCoordinator::send_input` | Yes | **No.** No consent check anywhere on the chain; indicator is advisory (see §2). |
| 2 | Global shortcuts (handy-keys implementation) | `src-tauri/src/shortcut/handy_keys.rs:126–137` | Manager thread polls hotkey events → same `handle_shortcut_event` | Yes | Same as #1 — none. |
| 3 | CLI flags `--toggle-transcription` / `--toggle-post-process` relayed via `tauri_plugin_single_instance` | `src-tauri/src/lib.rs:420–430` → `signal_handle.rs:18–24` | Second instance forwards raw argv strings; running instance calls `send_transcription_input(app, binding, "CLI")` with `is_pressed=true, push_to_talk=false` → coordinator | Yes | **No**, and additionally the trigger requires no UI interaction at all (see exploit E1). The relay matches raw argv substrings (`args.iter().any(\|a\| a == "--toggle-transcription")`), no authentication of the caller. |
| 4 | Unix signals SIGUSR2 (all Unix) and SIGUSR1 (macOS only) | `src-tauri/src/signal_handle.rs:34–57` | Signal thread → `send_transcription_input` → coordinator | Yes (Unix builds only) | None. Any local process able to `kill` the Speak PID toggles recording. |
| 5 | Tray menu actions | `src-tauri/src/tray.rs:402–462` (`handle_menu_event`) | Items: settings, secure-input warning, check-updates, copy-last-transcript, unload-model, cancel, quit, model-select | **No** — the tray cannot *start* recording today. It is only an indicator channel (Recording icon, `actions.rs:523`) and a cancel path (`cancel_current_operation`). | n/a |
| 6 | Frontend Tauri commands | `src-tauri/src/lib.rs:204–334` (full command registry) | Grep confirms **no command starts transcription recording**. Adjacent: `update_microphone_mode` (`commands/audio.rs:157–178`) can hold the microphone stream permanently open in AlwaysOn mode (`managers/audio.rs:391–418`) without any indicator — see §5. | No direct capture start | n/a (but see §5) |
| 7 | Headless CLI `--transcribe-file` / `--list-devices` / `--list-models` | `src-tauri/src/lib.rs:452–492` | Dedicated headless setup that deliberately skips recorder, overlay, tray, signal handlers ("so it never opens the mic") | No (no mic) | n/a |
| 8 | Direct manager call `AudioRecordingManager::try_start_recording` | `src-tauri/src/managers/audio.rs:806–852`, declared `pub` | Sole caller today: `actions.rs:593`, reached only via the coordinator's `start()` (`transcription_coordinator.rs:259–273` → `ACTION_MAP` → `TranscribeAction::start`, `actions.rs:497`) | Yes (today exactly one caller) | None enforced inside the manager itself — visibility of the future risk in §4/B3. |
| 9 | Planned meeting-session start commands (Phase 4) | Do not exist yet | Current meeting-mode code is data-store scaffolding only (`managers/meeting_store.rs`, `managers/knowledge_store.rs`) plus the offline spike binary `bin/meeting_loopback_spike.rs`; no Tauri command or runtime flag starts a meeting capture. | Not yet | Must be designed in — see §4. |

**Convergence fact:** every existing capture start (#1–#4) funnels through the single coordinator thread function `start()` at `src-tauri/src/transcription_coordinator.rs:259–273`, then `TranscribeAction::start` (`actions.rs:498–686`). There is **no consent check and no mandatory-indicator check** at any point on this funnel.

## 2. Current indicator channels and their suppression switches

During an active dictation recording there are exactly three observable signals, each independently disableable through persisted settings or CLI flags:

| Channel | Set where | Suppressed by |
|---|---|---|
| Recording overlay window | `actions.rs:567–571` (`OverlayStyle::None => {}`) and `overlay.rs:486–488` early-return | Setting `overlay_style = "none"` (`shortcut/mod.rs:787–807`), migrated legacy value (`settings/store.rs:190–200`, `schema.rs:267`) |
| Tray icon → Recording state | `actions.rs:523` (`change_tray_icon(app, TrayIconState::Recording)`) | `show_tray_icon = false` setting (`lib.rs:158–160`, `shortcut/mod.rs:1368–1375`) or `--no-tray` flag (`lib.rs:569–572`) |
| Start/stop chime | `actions.rs:640` (`play_feedback_sound_blocking`) | `audio_feedback = false` setting (`shortcut/mod.rs:540–543`) |

Additional degradation path: if tray icon initialization fails at startup the error is logged and startup continues (`lib.rs:149–151`); `change_tray_icon` likewise logs and continues (`tray.rs:138–145`), and `set_tray_visibility` failures are non-fatal (`tray.rs:464–471`). Recording therefore proceeds even when its last remaining indicator channel is dead.

There is **no consent artifact** of any kind today: grep over `src/` and `src-tauri/src/` finds no consent state, token, or dialog (the only "Consent" hits are the Windows CapabilityAccessManager registry key names in `commands/audio.rs:82–84`). For dictation this is arguably acceptable — the hotkey press itself is the user's act — but nothing prevents a *third party's* trigger (#3/#4) from starting capture on the user's machine.

## 3. Identified bypasses with concrete exploit sequences (Gate G2)

### B1 — Fully stealth recording reachable today (settings + CLI combination)

Flags/state, exact sequence on Windows (analogous on other platforms):

1. Suppress every indicator channel: set `overlay_style="none"`, `show_tray_icon=false`, `audio_feedback=false` (either by editing the persisted settings store JSON while the app is stopped, or programmatically via the first-party commands `change_overlay_style_setting`, `change_show_tray_icon_setting`, `change_audio_feedback_setting` — both webview windows hold the full command capability set per `docs/reviews/tauri-attack-surface-audit.md`).
2. Launch hidden: `speak.exe --start-hidden --no-tray` (`cli.rs:9–13`; applied at `lib.rs:569–585`; main window stays hidden, tray never built visibly).
3. From any unprivileged shell: `speak.exe --toggle-transcription`.

Result: the single-instance plugin relays the flag (`lib.rs:420–422`), the coordinator starts microphone capture (`transcription_coordinator.rs:170→259`, `actions.rs:593`) with **zero visible indicator**: no overlay (style None), no tray (hidden), no chime (disabled), main window hidden. Capture continues indefinitely until a follow-up `--toggle-transcription` / `--cancel` or process kill. This satisfies the plan's definition of a stealth recorder for anything recorded this way; plan lines 1458–1461 currently bind only to meeting mode, but the mechanism would carry into meeting mode unchanged unless the Phase 4 gate (§4) forbids it.

### B2 — Remote toggle without any local user action (timing/state)

No timing race is even required: `send_transcription_input` (`signal_handle.rs:18–24`) unconditionally forwards to the coordinator with `is_pressed=true, push_to_talk=false`; the coordinator's Idle-state arm (`transcription_coordinator.rs:177–181`) starts capture. Any process that can execute `speak.exe --toggle-transcription` or signal SIGUSR2 on Unix starts recording regardless of what the human at the keyboard is doing. The debounce/PTT logic cannot distinguish "user pressed the key" from "external flag/signal".

### B3 — Latent bypass for meeting mode (structural, not yet exploitable)

`AudioRecordingManager::try_start_recording` is `pub` (`managers/audio.rs:806`) and performs no gate of its own; correctness today rests solely on the convention that only `TranscribeAction::start` calls it. Nothing structural prevents a Phase 4 meeting-session start command from invoking the audio manager (or a future meeting recorder) directly, skipping any consent dialog or indicator the UI layer believes it displayed. Likewise `change_tray_icon`/overlay calls are best-effort decorations, not preconditions of capture.

### Non-findings checked (counterexample hunt)

- Tray cannot start recording (§1 #5) — verified by reading every `handle_menu_event` arm.
- No Tauri command reaches `try_start_recording` — verified by grep over `commands/`.
- Headless mode cannot open the mic — verified `lib.rs:452–492` skips the recorder and `headless.rs` consumes files only.
- Mid-recording style change to `None`: the already-shown overlay is not instantly torn down by `change_overlay_style_setting` (it only flips the cache used by *subsequent* state changes), so this is not an independent hide-vector beyond B1's pre-set variant.
- Cancel paths (`--cancel`, tray cancel, cancel shortcut) all terminate capture; they cannot be abused to *hide* an ongoing session.

## 4. Required enforcement point for Phase 4 (Gate G3)

One choke function must become the only way capture begins — for dictation today and for meeting sessions in Phase 4. Recommended shape (location: new `src-tauri/src/recording_gate.rs`, or as a first-class function beside `ACTION_MAP` in `actions.rs` if a new module is unwanted):

```rust
pub fn begin_recording(
    app: &AppHandle,
    request: RecordingRequest,   // Dictation { source } | Meeting { session_id, consent }
) -> Result<RecordingSession, GateError>
```

Hard requirements, all fail-closed (abort the start, never degrade silently):

1. **Consent precondition:** a `Meeting` request carries a consent proof issued only by the confirmed start dialog (short-lived token bound to the `meeting_session` id). Absent/expired token ⇒ `Err(GateError::ConsentRequired)` and no mic open. `Dictation` requests record their trigger `source` (hotkey / `"CLI"` / signal name) for the log line.
2. **Guaranteed-visible indicator before first sample:** for `Meeting` requests, `OverlayStyle::None` must not suppress the indicator — the gate forces at least one channel (minimal overlay *or* tray Recording state) and verifies the channel actually took effect (`set_tray_visibility`/`set_icon` returned `Ok`, overlay `show()` succeeded). If every channel fails, the start is refused. This closes the `tray.rs:138–145` silent-degradation hole for meetings.
3. **Single wiring:** `TranscriptionCoordinator::start()` (`transcription_coordinator.rs:259`) and the future meeting-session start command must both call `begin_recording`; `AudioRecordingManager::try_start_recording` loses `pub` visibility (becomes private to the audio module, callable only via the gate) so a direct call cannot compile. A unit/integration test asserts that `ACTION_MAP` transcribe actions route through the gate, and Phase 4 acceptance must reference `begin_recording` by name as the sole permitted capture entry point.
4. **Indicator persistence:** the gate owns teardown symmetry — every stop/cancel/error path that ends capture must pass through the matching `end_recording(session)` so the indicator cannot outlive or precede capture.

Phase 4 acceptance criterion to reference: *"A meeting recording can only be started through `begin_recording`; a test demonstrates that a start attempt with a missing consent token or with all indicator channels unavailable fails closed, and that `--toggle-*`/signal/shortcut triggers cannot start a meeting session."*

## 5. Out-of-scope observations (disclosed, not fixed)

- Always-on microphone mode keeps the capture device open continuously (`managers/audio.rs:391–418`, toggled by `update_microphone_mode`, `commands/audio.rs:157–178) with no persistent indicator; live frames feed the VAD/stream preview. Not a transcription recording, but adjacent to the stealth concern and worth an explicit product decision.
- The single-instance relay authenticates nothing about the second process (any local account/process can invoke it). Acceptable desktop-app posture, but the gate's source logging (§4 item 1) will make such triggers auditable.

## Verification method

Static read of the named sources in this worktree plus greps: `try_start_recording|start_stream\(|always_on_microphone`, `handle_shortcut_event|send_transcription_input|send_input\(`, `meeting|consent`, `show_recording_overlay|hide_recording_overlay|OverlayStyle::None`, `cancel_current_operation` across `src-tauri/src` and `src`. No build or test run was required or performed (read-only audit); all line numbers cite files as they exist in this worktree.
