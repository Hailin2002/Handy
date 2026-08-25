# Shutdown Dataflow — Ordered Teardown × Startup-Recovery Convergence

Status: **spec contract, Phase 4 not implemented — changes no source code**
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) §6.2 rule 10
(lines 396–398: „Die App darf geschlossen werden … und verhindert einen
versehentlichen vollständigen Prozess-Exit"), §6.12 (line 654: „Du kannst dieses
Fenster schließen"), „Stoppen" (lines 936–941), Session-Zustände
(lines 877–896), Fehlerverhalten „App-Absturz" (lines 959–961), §13 Phase 4
(line 1571), §17 DoD item 2 (lines 1957–1959)
Complements: [startup-recovery-dataflow.md](startup-recovery-dataflow.md)
(its §3 sequence **S0–S7 is the only recovery mechanism this document relies
on** — no new recovery rule is introduced here)
Builds on:
[recorder-contract.md](recorder-contract.md) §4 (`stop()` → `SessionSummary`
only after durability) and §8 (open-chunk recovery),
[../meeting-mode-phase2-contracts.md](../meeting-mode-phase2-contracts.md)
§5 (atomic finalization, manifest update rule), §7 (crash-recovery rules),
§8 (source-failure isolation),
[job-scheduler-contract.md](job-scheduler-contract.md) §4 (cancellation),
[session-state-contract.md](session-state-contract.md) §2.2 (T07/T08/T09/T15),
§3.3, §6.2 (R1/R5)

This document defines **one ordered teardown sequence** that runs when the app
quits during an active, paused, or processing meeting, plus the proof that
every state an interrupted teardown can leave behind converges through the
**existing** startup recovery sequence. It introduces no new persistence
format, no new component, and no new recovery rule; every durable fact it
leaves behind is one that startup-recovery-dataflow.md §4 already classifies.

---

## 1. Scope and precedence

- **In scope:** full process exit while a meeting session is in
  `{recording, paused, stopping, transcribing, summarizing}` or while only
  background STT/summary/import jobs run; the four quit entry points (tray
  quit, window close, OS logout/shutdown, crash mid-teardown); per-step
  timeouts and forced-exit fallback; convergence of every interrupted-teardown
  intermediate state.
- **Out of scope:** in-recording device/disk errors (recorder-contract §7);
  user-initiated stop that keeps the app running (session-state-contract
  §3.3); retry/backoff policy (stt-provider-contract §3.5); deletion flows;
  dictation-path recording teardown (unchanged existing behavior).
- **Precedence** (documentation index §4): plan beats contracts, contracts beat
  catalogs, nothing beats real code. Where this document repeats rules from
  recorder-contract §4/§8, phase2-contracts §5/§7, or the session-state
  contract, those sections stay normative; this document fixes only the
  **quit-time join order, the deadlines, and the fallback proof**.

## 2. Entry points (gate G3) — current code facts and target seams

The four entry points below are exhaustive for a Windows-first Tauri desktop
app; each cites the actually existing code seam today (HEAD `75a6f1ef`,
clean tree, verified via `git rev-parse HEAD` and empty
`git status --porcelain`) and names where Phase 4 attaches the teardown call.
No other quit path exists or may be added without extending this section.

| #   | Entry point                       | Existing code seam (implemented behavior, cited)                                                                                                                                                                     | Target behavior in meeting mode                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| --- | --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| E1  | Tray-menu quit                    | `src-tauri/src/tray.rs` lines 438–440: menu id `"quit"` calls `app.exit(0)`; `RunEvent::Exit` (lib.rs lines 622–627) then unloads the transcription model. Today this exits instantly regardless of activity.        | Before `app.exit`, the manager runs sequence **T** (§3) if any session is active/paused/stopping, or **P** if only background jobs exist. Plan lines 396–398 forbid a silent full-process exit during recording: the tray quit item on an active session first surfaces the explicit plan-§6.9 confirm flow („Meeting wirklich beenden?", plan line 582) unless the session is already past capture.                                                                                                          |
| E2  | Window close (X button)           | `src-tauri/src/lib.rs` lines 585–606: `CloseRequested` → `api.prevent_close()` + hide window; the process never exits via window close today (with `--no-tray`, cli.rs line 13, the dock/taskbar remains reachable). | Unchanged mechanics: closing never kills the process. During active/paused capture the close is accompanied by the explicit notice demanded by plan lines 396–398; during processing-only the checklist statement „Du kannst dieses Fenster schließen" (plan line 654) applies and jobs keep running in the tray. E2 therefore never triggers teardown **T**; it can trigger **P**'s notification branch only (no durable writes needed — jobs are already persistent rows, scheduler-contract invariant P1). |
| E3  | OS logout / shutdown / restart    | No graceful hook exists today for SIGTERM/SIGINT/WM_ENDSESSION: `signal_handle.rs` lines 26–55 registers **only** SIGUSR1/SIGUSR2 as remote-toggle signals; `RunEvent::Exit` fires only on orderly Tauri exit.       | Phase 4 obligation: register an end-session/signal hook that invokes sequence **E3-T** (§6): a time-budgeted subset of T that secures capture first, then lets the OS kill the process. Everything not finished falls back to recovery-on-next-start (that fallback is the entire reason the sequence is designed recovery-compatible).                                                                                                                                                                       |
| E4  | Crash mid-teardown (or any crash) | No handler possible by definition.                                                                                                                                                                                   | Not an entry point but the correctness target: any kill between two steps of §3 must leave exactly one classified residue of startup-recovery-dataflow §4 dimensions A/B/C. Proof in §4/§5.                                                                                                                                                                                                                                                                                                                   |

Headless one-shot mode (`--transcribe-file`, lib.rs lines 454–488) never opens
capture or meeting sessions and uses `std::process::exit(code)` deliberately;
it is out of scope and unaffected.

## 3. The ordered teardown sequences

Two variants share one spine:

- **T (full quit with capture or unsettled session)** — entered from E1/E3
  when `meeting_sessions` holds a session in
  `{starting, recording, paused, stopping}`.
- **P (processing-only quit)** — entered when all sessions are in
  `{transcribing, summarizing, completed, stopped_unprocessed, cancelled,
recoverable_error}` or none exists. P consists of D0, D5–D7 only; capture
  steps are skipped because there is nothing open to flush.

Steps are executed strictly in numeric order; each step is individually
idempotent or transactional so that a kill anywhere leaves a state the §4
table maps onto startup-recovery-dataflow §3. Timeouts are testable constants
(one definition site per step, asserted by the fault-injection tests of §7).

| Step | Action (owner)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Deadline       | On timeout / forced exit: exact state left behind                                                                                                                                                                                  | Recovered by                                                             |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| D0   | **Quit arbitration.** Manager enumerates open sessions and selects T or P; refuses/defers quit while a modal stop-confirm is pending (plan §6.9). Pure read + in-memory decision.                                                                                                                                                                                                                                                                                                                                                                                                        | 250 ms         | Nothing written. Process continues quitting via later steps' defaults; worst case behaves like E4.                                                                                                                                 | n/a (no durable change)                                                  |
| D1   | **Persist `stopping`.** For T: execute T07 (session-state-contract §2.2) — `UPDATE meeting_sessions SET state='stopping'` committed **before** touching the recorder, mirroring start-order rule §3.1 step 3 (persist before hardware work). Freeze intake: recorder stops accepting rollover/new chunks.                                                                                                                                                                                                                                                                                | 500 ms         | Row says `stopping`; chunks still rolling as `writing`/`ready` mix. Candidate-set membership proven in §5.                                                                                                                         | S0/S1/S2 via R1 superset; case Q1                                        |
| D2   | **Stop capture, finalize open chunks.** Call `stop()` (recorder-contract §4, line 642: returns `SessionSummary` only after every open chunk is durably `ready`). Per-source independence per phase2-contracts §8 rule 2: source A's failure never blocks source B. Budget **2000 ms per source**, sequential mic→system.                                                                                                                                                                                                                                                                 | 2 × 2000 ms    | Killed mid-finalization ⇒ at most one `.wav.part` with a `writing` row per source; all earlier renames valid forever (phase2-contracts §5.1 single commit point). Timeout ⇒ same residue; the deadline miss itself writes nothing. | S1 cases A2–A6; cases Q2/Q3                                              |
| D3   | **Manifest finalize.** Atomic `manifest.json` replace (phase2-contracts §5.3: write tmp → fsync → rename → fsync dir). Manifest is a cross-check artifact only, never authoritative.                                                                                                                                                                                                                                                                                                                                                                                                     | 1000 ms        | Tmp file present, old manifest stale. Both are ignored by recovery by construction.                                                                                                                                                | S1 (manifest consulted only as cross-check, session-state-contract §6.1) |
| D4   | **Job-state persistence + session disposition.** One SQLite reconciliation transaction: enqueue remaining `ready`-chunk jobs with deterministic idempotency keys (S5 semantics, stt-provider-contract §3.3) and flip the session per T08/T09/T15 — destination `transcribing`, `completed`, or `stopped_unprocessed`; `ended_at` filled.                                                                                                                                                                                                                                                 | 1500 ms        | Transaction either fully committed or rolled back (SQLite atomicity): residue is either „still `stopping`, chunks ready" (case Q4) or the fully post-D4 state. Never a torn pair.                                                  | R5(b)/T08-equivalent advance at next start; S4 join invariant            |
| D5   | **Summary-job deferral.** Summary is never started during teardown. If `summary_state='pending'`/configured, ensure its `processing_jobs` row exists in a non-running persistent state (scheduler-contract §1: job rows are the only persistence point; SEC state/retry semantics equal `meeting_stt_jobs` verbatim). A summary already `running` is left as-is on purpose: killing the process converts it via restart mapping, not via cancel — see note below.                                                                                                                        | 300 ms         | Row absent ⇒ S5-style re-enumeration recreates it from persisted transcript completeness; row present ⇒ B-dimension handling.                                                                                                      | S3/S5; B-cases                                                           |
| D6   | **Queue/engine cooperative release.** Signal workers to finish the current segment boundary and drop leases (scheduler-contract §3.4/§4.2 lease semantics). Local inference aborts only at the safe boundary (coexistence-contract pause-at-segment-boundary rule); cloud HTTP futures are dropped best-effort. **No job rows are mutated here** — persistence stays with D4/D5.                                                                                                                                                                                                         | 1000 ms        | Engine lease rows/locks vanish with the process; job rows unchanged (`running` included).                                                                                                                                          | S3 restart mapping `running → queued` without attempt penalty (B2)       |
| D7   | **UI/event/tray teardown, release single-instance lock, `app.exit(0)`.** Mirrors today's `RunEvent::Exit` model-unload (lib.rs lines 622–627) as a subset.                                                                                                                                                                                                                                                                                                                                                                                                                               | 500 ms         | Trivial residues only (windows gone, lock file stale — liveness is decided by pid/lock ownership at S0, not by cleanliness of removal).                                                                                            | S0 liveness scan                                                         |
| W    | **Watchdog (forced-exit fallback).** One armed timer over the whole sequence: **8000 ms** for interactive quit (E1), **4000 ms** for end-session (E3, see §6). On expiry: log, then immediate `std::process::exit`. Exit code 0 is deliberate: the OS-visible quit result must never depend on teardown success — durability is owned by the recovery-compatible residues, not by clean exits. Justification for the budget: every step above is designed so its partial state converges (§4); waiting longer cannot improve durability beyond D2+D3, which get the front of the budget. | 8000 / 4000 ms | Whatever prefix of D1–D7 committed. Always one of the §4 rows.                                                                                                                                                                     | respective row                                                           |

Notes:

1. **Why D4 does not cancel running jobs:** cancellation is a _user_ semantic
   (plan line 940; scheduler-contract §4.2). A quit is not a cancel: audio and
   queued work must survive, so jobs stay non-terminal and resume next launch.
   Only the in-flight engine lease is released cooperatively (D6).
2. **Paused meetings:** entering T from `paused` skips nothing — pause already
   finalized open chunks as a durability checkpoint (RC-§6.1; phase2-contracts
   §5.2), so D2 finds zero open chunks and degenerates to a fast no-op.
   Timeline markers were persisted at T05 and need no shutdown-time write.
3. **Degenerate sessions:** a session killed in `starting` before any chunk
   existed takes the same path with empty A/B dimensions — C4 of
   startup-recovery-dataflow §4 („banner explains safe early end").

## 4. Interrupted-teardown case table (gates G2 + G3)

Claim: for every entry point (E1–E4) and every kill point inside T/P, the
durable residue is a tuple (Dimension-C session state, Dimension-A chunk
residue, Dimension-B job residue) that startup-recovery-dataflow §3 already
classifies. The table enumerates each kill interval once; intervals are
exhaustive because they partition the step list. "Convergence" column names
the existing steps only — **no new rule appears in this column.**

| Case | Entry × kill interval                                   | Residue: session                                        | Residue: chunks                                               | Residue: jobs                        | Converged by (startup-recovery-dataflow §3)                                                                                |
| ---- | ------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------- | ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Q0   | Any × idle quit (no open session, no non-terminal jobs) | none                                                    | none                                                          | none                                 | whole sequence no-ops (C1)                                                                                                 |
| Q1   | T × kill in D1 (before/inside the T07 commit)           | `recording`/`paused` (commit rolled back) or `stopping` | mixed `writing`/`ready` incl. current open chunk              | queued/running as-is                 | S0 candidate (C2/C3, or `stopping` per §5 alignment) → S1 (A1–A6) → S2 flip → banner E12                                   |
| Q2   | T × kill in D2, before rename                           | `stopping`                                              | open `.part` + `writing` row(s)                               | unchanged                            | S1 A3/A4/A5 → R5(b)/T08 advance → S5/S6                                                                                    |
| Q3   | T × kill in D2, after rename before DB/manifest append  | `stopping`                                              | orphan-or-unrowed `.wav` (= A2)                               | unchanged                            | S1 reverse reconciliation (§8 step 3) → as Q2                                                                              |
| Q4   | T × kill in D3/D4 before D4 commit                      | `stopping`                                              | all durably `ready` (D2 done) or partial if D2 also cut short | unchanged                            | composition of Q2/Q3 residues; then R5(b) advances `stopping → transcribing/completed` (session-state-contract §6.2 R5(b)) |
| Q5   | T/P × kill after D4 commit, before D7                   | `transcribing`/`completed`/`stopped_unprocessed`        | all `ready`                                                   | some `queued`/`retry_wait`/`running` | C5/C1 session side; S3 mapping B2–B4; S5 dedup by unique `(meeting_id, idempotency_key)`                                   |
| Q6   | P × kill anywhere in D5–D7                              | `transcribing`/`summarizing`                            | all `ready` (no capture ever at risk)                         | queued/running/retry_wait            | identical to Q5 minus S1/S2 (nothing to repair)                                                                            |
| Q7   | E3 end-session expiry mid-D2/D3                         | as Q2/Q3                                                | as Q2/Q3                                                      | unchanged                            | identical rows; OS kill ≡ E4 kill for state purposes                                                                       |
| Q8   | E4 crash during any interval                            | = the interval's row above                              | = ditto                                                       | = ditto                              | by reference: E4 adds no new residue class                                                                                 |

Spot-checked combinations (mirroring startup-recovery-dataflow §4 style):

| Combination                                                              | End state after one restart pass                                                                                                                                             |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q1 (was `recording`, mic mid-chunk, system just rolled) + queued backlog | mic chunk `ready(recovered)`, system intact; jobs re-queued unpenalized; session `stopped_unprocessed`; banner offers recover/process                                        |
| Q4 + summary configured                                                  | session advanced to `transcribing`; deferred summary job materialized by S5-equivalent enumeration; completes to `completed` without user action beyond ordinary progress UI |
| Q6 with a `running` local job holding the engine lease                   | lease died with process; job `running → queued` (B2, no attempt penalty); resumes after restart under normal L1/C1/S1 ceilings                                               |
| Q0 repeated twice (double-run)                                           | second pass byte-empty (S-step guards all miss)                                                                                                                              |

Exhaustiveness argument: the kill-interval list partitions D0–D7 ∪ {before D0,
after D7}; before D0 ≡ no teardown ran (plain crash, covered by
startup-recovery-dataflow §4 wholesale), after D7 ≡ Q0. Each interval's
residue is the union of the previous interval's residue plus at most one
partially-committed unit whose atomicity boundary (rename, SQLite transaction,
tmp-file discard) is owned by an existing contract — hence no composite can
escape the three dimensions. This mirrors and therefore inherits the
composition argument of startup-recovery-dataflow §6.

## 5. Cross-contract alignment: the `stopping` candidate set (disclosure)

startup-recovery-dataflow §3 S0 enumerates candidate set A as
`{starting, recording, paused}`. The session-state contract's reconciliation
R1 (session-state-contract §6.2) uses the wider set
`{starting, recording, paused, stopping}`, and its R5(b) defines the
disposition for `stopping` rows (advance to `transcribing`/`completed`).
Both documents implement the same plan anchor (lines 893–896); this document
**adopts the wider R1 set as normative** because teardown interruption makes
`stopping` a first-class residue (cases Q2–Q4), and declares the narrower S0
enumeration in startup-recovery-dataflow.md to be an under-specification to
align there (flagged to the integration owner; no behavioral conflict —
S1/S2 bodies already handle these rows since S1 keys on chunk rows and R5(b)
owns the flip). No new recovery rule is created by this adoption; it is
selection between two published statements of the same sequence.

## 6. OS logout/shutdown specifics (E3, gate G3)

Windows grants a bounded response window to `WM_QUERYENDSESSION`/
`WM_ENDSESSION` (default regime ~5 s per application before forced
termination); POSIX logout paths deliver SIGTERM with similar brevity. The
contract therefore defines variant **E3-T**: the watchdog budget shrinks to
**4000 ms** and step priority is fixed as D1 → D2 → D3 (capture safety first,
per the plan's own ranking: „Stoppen beendet zuerst sicher beide
Capture-Streams", lines 938–940), then whatever of D4–D7 fits. D4's
transactional shape guarantees the choice „fully applied or not at all", so a
budget cut between D3 and D4 is exactly case Q4. The end-session hook must be
registered on the same thread-safety terms as the existing signal_hook usage
(signal_handle.rs lines 35–47) and must never block on UI. There is no
auto-restart, no session save prompt, and no OS-specific service integration —
the recovery-on-next-start property is the fallback, by design.

## 7. Verification hooks

Extends recorder-contract §9 and startup-recovery-dataflow §7:

- Kill-matrix test: for each row of §4, inject process termination at the
  named interval (fault-injection seam per D-step), relaunch, run the startup
  sequence twice, assert: (a) convergence to predicate P of
  startup-recovery-dataflow §6, (b) byte-identical DB diff on the second run.
- Deadline tests: stub each owner with a blocked future; assert watchdog fires
  within budget + jitter and that the resulting residue matches the step's
  „state left behind" cell verbatim.
- E3 simulation: send WM_QUERYENDSESSION (Windows) / SIGTERM (CI harness) at
  each interval; assert the ≤4 s budget holds and residue equals the Q-row.
- Double-entry assertion: no code path outside D1–D7 writes
  `meeting_sessions.state` during quit (guards against a second teardown
  authority, mirroring session-state-contract §1).
- Idempotent re-quit: issuing E1 twice concurrently yields exactly one T run
  (single-instance lock held until D7).

## 8. Non-goals (complexity budget)

Rejected explicitly, consistent with the mission complexity budget and the
plan's anti-bloat stance:

- no shutdown-progress persistence format, journal, or "shutdown state
  machine" alongside the existing ones — the residue classes of §4 are
  exactly the startup dimensions, and that is the design;
- no second watchdog/threading framework: one armed deadline timer;
- no waiting for cloud uploads or long local inference before exiting (jobs
  are persistent rows; waiting would punish mobile users for nothing);
- no auto-cancel of running jobs on quit (cancel is a user semantic,
  scheduler-contract §4.2);
- no OS auto-restart/resume-after-logon choreography;
- no custom modal "saving…" wizard during logout (E3 must stay silent and
  fast; the recovery banner on next launch is the user-facing surface);
- no macOS/Linux system-audio parity work and no per-process loopback hooks
  (plan §2 non-goals, untouched);
- no SAP-like configurable teardown pipeline/stage registry: steps are fixed
  constants; changing the order requires editing this contract, not config.

## Appendix A — Gate traceability (this document)

- **G1** numbered teardown order with per-step timeout and state left behind
  on timeout: §3 table (D0–D7 + watchdog W, deadline and residue columns).
- **G2** case table proving every interrupted-teardown state converges via
  startup-recovery-dataflow §3 with no new recovery rules: §4 (Q0–Q8 +
  exhaustiveness argument); the single alignment adoption of §5 selects
  between two published statements instead of inventing a rule.
- **G3** covers tray-quit (E1), window-close (E2), OS logout/shutdown (E3/§6),
  crash-mid-teardown (E4/Q8): §2, §4, §6.
- **G4** non-goals section and documentation-index update: §8 here; row added
  to [../meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)
  §3 in the same change set.
