# Meeting Mode — Known Limitations & Unverified Claims Register

- **Audit date:** 2026-08-23 (re-audit of the full register; original audit 2026-08-22)
- **HEAD commit at audit time:** `b4b0d344af0741cda28fbb10a2d1ba339499f214` ("company(task-0345): Reconcile meeting-mode-documentation-index.md with the actual docs tree at HEAD")
- **Previous audit:** 2026-08-22 at `d3cf5c755aa612e2836085e78a61e45a589aab91` ("company(task-0046): Write the meeting-mode data and storage developer reference"). The re-audit re-verified every source citation below by direct file reads/greps at HEAD; all cited paths resolve (none moved except where annotated inline). No entry was flipped to `resolved`: none of the expiry conditions is met at HEAD. Facts that changed between the two commits are reflected in the updated entries (L2, L4, L7).
- **Scope of this register:** every verified limitation or unverified claim that affects how much confidence the current tree may justifiably claim for Meeting Mode (Phase 0/1 evidence, contracts, build constraints). One entry per limitation/claim, each with source citation(s), current impact, and a recheck/expiry condition.
- **Status legend:** `open` — the limitation/claim stands as written; `resolved` — a cited artifact or commit in the tree proves the limitation no longer applies. **No entry below may be flipped to `resolved` without a citation to the resolving artifact or commit.**
- **Method:** all citations were verified against this worktree at audit time by direct file reads, greps, and one behavioral reproduction (L4). External web sources were _not_ consulted during this audit (see L3). Re-audit 2026-08-23: citations re-verified by direct file reads/greps and the L4 behavioral reproduction was re-run at HEAD with identical result.

---

## L1 — All four loopback runs are aborted captures; they cannot support the 120-min Phase-1 acceptance

- **Status:** open
- **Type:** verified limitation
- **Sources:**
  - `docs/phase1-spike/run-20260822/meeting_loopback_spike_120min.csv` — header declares `duration_s=7200`, but the last data row has `t_rel_ms = 97440.201` (~97 s span).
  - `docs/phase1-spike/run-20260822-r2/meeting_loopback_spike_120min.csv` — last row `t_rel_ms = 179683.364` (~180 s span).
  - `docs/phase1-spike/run-20260822-r2-attempt1-reaped/meeting_loopback_spike_120min.csv` — last row `t_rel_ms = 10928.809` (~11 s span).
  - `docs/phase1-spike/run-20260822-r3/meeting_loopback_spike_120min.csv` — last row `t_rel_ms = 146580.501` (~147 s span).
  - Each CSV header records the intended 7200 s (`duration_s=7200`) while the actual capture ends orders of magnitude earlier; no completion marker or SUMMARY analysis artifact exists anywhere under `docs/phase1-spike/`.
- **Current impact:** none of the four committed runs reaches even 3 % of the intended 120-minute duration. Phase-1 acceptance ("120 min clean dual-source capture") is therefore **not evidenced** by any artifact in the tree; any doc citing these runs as a passing 120-min soak is overstating the evidence. Observed spans range from ~11 s to ~180 s.
- **Recheck / expiry condition:** expires when a run CSV with a final `t_rel_ms ≥ 7 200 000` (plus its analyzer SUMMARY artifact) is committed under `docs/phase1-spike/`; until then any acceptance statement referencing "the 120-min run" must carry this caveat.

## L2 — No DEVICE_LOSS / SLEEP / WAKE measurement exists

- **Status:** open
- **Type:** verified limitation
- **Sources:**
  - Measurement tooling exists but was never run to completion with committed results: `docs/phase1-spike/tools/device-loss-test.ps1`, `docs/phase1-spike/tools/sleep-wake-test.ps1`, `docs/phase1-spike/tools/log-event.ps1`.
  - Repo-wide grep for `DEVICE_LOSS|SLEEP|RESUMED|WAKE` inside `docs/phase1-spike/` matches only inside `tools/*.ps1` scripts — zero matches in any `run-*/` CSV or other artifact; no `events.log` file is committed anywhere under `docs/phase1-spike/`.
  - The four run directories (`docs/phase1-spike/run-*/`) contain exactly one file each (`meeting_loopback_spike_120min.csv`); see L1 for their aborted spans.
- **Current impact:** behavior after audio-device loss, system sleep, and wake — which the recorder contract depends on (`docs/meeting-mode-architecture/recorder-contract.md:715`, re-verified at HEAD, specifies device-loss gap handling) — is entirely unmeasured on target hardware. Contract decisions in this area rest on design intent, not measurement. Re-audit 2026-08-23 update: committed tooling documentation has since grown (`docs/phase1-spike/tools/README.md` documents the `events.log` marker contract incl. `DEVICE_LOSS_START`/`SLEEP_ENTER`/`RESUMED_FROM_SLEEP`; `docs/phase1-spike/tools/device-switch-test.ps1` adds a mid-recording default-device switch test for the _dictation_ path), but this changes nothing material: still no `events.log` and no run CSV containing any scenario marker is committed, so the limitation stands.
- **Recheck / expiry condition:** expires when a committed events log containing `DEVICE_LOSS_START`/`DEVICE_LOSS_*` or `SLEEP_ENTER`/`RESUMED_FROM_SLEEP` markers plus the corresponding spike CSV segment appears under `docs/phase1-spike/`.

## L3 — Azure Fast Transcription plan assertions are unverified against live Microsoft docs

- **Status:** open
- **Type:** unverified claim
- **Sources:**
  - `docs/meeting-mode-plan.md:1108` asserts "der aktuelle Short-Audio-Endpunkt erlaubt maximal 60 Sekunden" and line 1113 asserts "eine Datei muss unter fünf Stunden und unter 500 MB liegen" (Fast Transcription).
  - `docs/meeting-mode-plan.md:1938-1941` cites `https://learn.microsoft.com/en-us/azure/ai-services/speech-service/fast-transcription-create` and related URLs as references only.
  - `docs/meeting-mode-architecture/stt-provider-contract.md:278` repeats the "offiziellen Grenzen von 500 MB und 5 Stunden" as internal hard caps; `docs/meeting-mode-phase2-contracts.md:304` builds package-sizing rules on the "5 h/500 MB Fast Transcription limits".
  - Repo-wide grep for `learn.microsoft.com`: the URL appears only as citations in `docs/meeting-mode-plan.md`; **no fetched/snapshotted copy of any Microsoft documentation page exists in the tree**, and this audit did not access the live web.
- **Current impact:** the 60 s short-audio limit and the 5 h/500 MB Fast Transcription limits are load-bearing for chunk/package sizing in two contract documents, but are currently sourced from memory/plan notes rather than a verifiable Microsoft document. If the real limits differ, package-sizing constants and the STT provider contract would need revision.
- **Recheck / expiry condition:** expires when a committed verification note quotes the limits together with a dated retrieval of the official Microsoft page (URL + retrieval date + quoted text), stored e.g. under `docs/reviews/`.

## L4 — Spike CSV analyzer silently collapses duplicate meta keys

- **Status:** open
- **Type:** verified limitation (behaviorally reproduced)
- **Sources:**
  - `docs/phase1-spike/tools/analyze-spike-csv.ps1:13` ($meta is a single `[ordered]@{}` dictionary) and lines 22–23 (each `# key=value` header comment assigns `$meta[$Matches[1]] = $Matches[2]`, overwriting any previous value for the same key without warning).
  - Behavioral reproduction (audit-time): feeding the analyzer a synthetic CSV whose header contains `# keyA=first`, `# keyB=only`, `# keyA=second` produced a SUMMARY listing only `- keyA: second` and `- keyB: only`; the first value disappears silently.
- **Current impact:** if the spike binary ever emits the same meta key twice (e.g., repeated device or restart markers across stream restarts), the analysis SUMMARY will report only the last occurrence, and downstream readers may mistake a partial metadata record for a complete one. Current committed CSVs contain no duplicate keys (8 unique header meta lines per run), so no existing artifact is known-corrupted — the defect is latent in the tool. Re-audit 2026-08-23 update: the analyzer script is unchanged since the original audit, the defect was re-reproduced at HEAD with identical result (`# keyA=first` … `# keyA=second` yields only `- keyA: second`), and `docs/phase1-spike/tools/README.md:50,119` now _documents_ the last-wins collapse as a known limitation of the SUMMARY semantics — but documentation is not a fix, so the entry stays open.
- **Recheck / expiry condition:** expires when the analyzer either rejects duplicate meta keys loudly or preserves all occurrences (verifiable by rerunning the same synthetic-input test above).

## L5 — Native builds are pinned to serial execution (`CMAKE_BUILD_PARALLEL_LEVEL=1`) as a machine-specific workaround

- **Status:** open
- **Type:** verified limitation
- **Sources:**
  - `.cargo/config.toml:3-11` — env pin `CMAKE_BUILD_PARALLEL_LEVEL = "1"` with comment: any MSBuild worker parallelism makes the cmake-driven native build fail silently here, while serial succeeds.
  - `vendor/cmake/src/lib.rs:896-906` — local patch making the vendored cmake-rs honor `CMAKE_BUILD_PARALLEL_LEVEL` ahead of `NUM_JOBS` precisely because cargo always overwrites `NUM_JOBS` for build scripts.
  - `BUILD.md:303-322` and `BUILD.md:368-369` document the constraint and the override procedure.
  - `docs/meeting-mode-phase0-baseline.md` §2.3 and §5 item 3 record the original `--parallel 20` failure and the serial workaround as resolved-by-workaround (not resolved-by-fix).
- **Current impact:** native dependency builds are strictly serial on this machine, so full builds are significantly slower than hardware capability, and the constraint is machine-specific — it does not transfer to contributor machines, and raising the value re-exposes the original silent MSBuild failure. The root cause in the transcribe-cpp-sys cmake configuration remains unfixed.
- **Recheck / expiry condition:** expires when either the underlying cmake/MSBuild parallelism bug is fixed upstream (evidenced by a passing `--parallel N>1` build on this machine) or the workaround is confirmed unnecessary on current toolchain versions.

## L6 — Manual dictation baseline (Local/Azure/Custom) never executed on hardware

- **Status:** open
- **Type:** verified limitation
- **Sources:**
  - `docs/meeting-mode-phase0-baseline.md` §3 defines the three manual dictation checkpoints (§3.1 Local, §3.2 Azure Short Audio, §3.3 Custom STT) as step-by-step procedures.
  - Same file, §5 item 4: "Diktier-Baseline manuell nach Abschnitt 3 auf der Zielhardware ausführen und Ergebnisse ergänzen (**in dieser Baseline nicht ausgeführt**) — **offen**, Owner: task-0009."
  - Same file, §4: the entire manual Windows test matrix M1–M20 is marked `pending` with empty result columns ("es liegt kein Testergebnis vor").
- **Current impact:** the guarantee that the existing short-dictation path keeps working unchanged through Meeting Mode work (plan line 1734: "der bestehende Diktierpfad besteht seine Baseline erneut") has no recorded starting point — there is no before-state against which regressions from meeting-mode changes could be detected.
- **Partial evidence 2026-08-27 (stays open):** the _local model_ leg of §3.1 was
  executed on this hardware for the first time, headlessly via
  `speak.exe --transcribe-file <wav> --model <id>`:
  - `whisper-tiny.en-Q8_0` — bound to Vulkan0 (NVIDIA GeForce RTX 2080), load
    372 ms, 0.58 s audio, transcript returned.
  - `whisper-large-v3-turbo-Q8_0` — same backend, load 1173 ms, 0.52 s audio,
    transcript returned; `supports_language_detect` correctly differs from the
    tiny model.

  So model registry, model load, GPU binding, decode, resampling and the
  transcription manager are known to work on this machine. This does **not**
  close L6: §3.1 is a _dictation_ checkpoint, and microphone capture, the
  shortcut path and paste delivery were not exercised. Azure (§3.2) and Custom
  (§3.3) remain untouched. The value here is that a failure in the remaining
  legs can now be localised — the model and inference half is no longer a
  suspect.

- **Recheck / expiry condition:** expires when §3 checkpoints and/or §4 matrix rows in `docs/meeting-mode-phase0-baseline.md` are filled with observed `pass`/`fail` results and a date.

## L7 — No meeting-mode runtime capability exists beyond the Phase-1 spike binary (production code: spike binary + compile-only repository seam + flag-gated placeholder UI)

- **Status:** open
- **Type:** verified limitation
- **Sources:**
  - Glob of `src-tauri/src/**/*.rs` (original audit, HEAD `d3cf5c7`): the only meeting-related source file was `src-tauri/src/bin/meeting_loopback_spike.rs`, a standalone measurement harness whose own doc comment states it "exists to answer … before the MeetingRecorder" is built.
  - **Re-audit 2026-08-23 correction:** this is no longer literally true on two fronts —
    - _Backend:_ commit `9dc1163` (task-0134) added `src-tauri/src/managers/meeting_store.rs` and `src-tauri/src/managers/knowledge_store.rs` (plus `mod.rs` wiring) as a _compile-only_ repository trait/entity seam transcribed from `docs/meeting-mode-architecture/repository-boundary-contract.md`. Its own header states: "Traits and entity types only. There is **no SQL**, no concrete `MeetingStore` struct, no constructor and no manager wiring". Commit `7ef3bdc` (task-0201) additionally added `src-tauri/tests/stt_negative_path.rs` (negative-path tests of the existing dictation cloud-STT client against a mock HTTP server — test coverage of the dictation path, not new meeting-mode capability).
    - _Frontend:_ commits `c2026a1` (task-0171), `165b41b` (task-0172) and `f0f0295` (task-0175) added the product-navigation shell (`src/components/shell/`, incl. `ActiveMeetingNavItem.tsx`), a Meetings settings tab (`src/components/settings/tabs/MeetingsSettingsTab.tsx`) and a Meetings-page empty state (`src/components/meetings/MeetingList.tsx`). Every one of these is an explicit placeholder/empty-state component gated behind a build-time feature flag (`MEETING_MODE_ENABLED` in `src/components/shell/productNav.ts`); their own doc comments state "Meeting mode does not exist yet" and that no caller can report an active meeting today. No recording, transcription, storage or summary UI exists.
  - Still absent at HEAD: no `MeetingRecorder`, no concrete `MeetingStore` struct, no `MeetingSummaryService` implementation, no `AzureMeetingClient`, and no functional meeting UI (recording controls, live view, transcripts, summaries) exists anywhere in the tree.
  - `docs/meeting-mode-documentation-index.md:58` itself marks the STT provider contract as "Architekturvertrag, Phase 5 nicht implementiert" _(citation updated at re-audit: this statement sat at line 49 in the original-audit index; the index was rewritten by commit `b4b0d344` / task-0345 and the statement now lives at line 58)_.
- **Current impact:** every user-facing meeting-mode capability described in `docs/meeting-mode-plan.md` (recording, projects, summaries, Azure packages, import) remains design-only. The new repository seam and the flag-gated navigation/empty-state UI narrow the gap on paper but deliver no runtime behavior: nothing can start, record, store, transcribe, or summarize a meeting. Documentation that speaks of meeting mode in the present tense still describes planned behavior; executable reality remains limited to the loopback spike binary with its measurement artifacts, compile-time type definitions, and inert placeholder UI behind a disabled feature flag.
- **Recheck / expiry condition:** expires per feature when a production module implementing the corresponding contract lands in `src-tauri/src/` or `src/` (e.g., first non-bin `MeetingRecorder` implementation commit).

## L8 — Unverified claim in the UX copy catalog: sleep/wake & device-loss raw data asserted to exist

- **Status:** open
- **Type:** unverified claim found in tree (contradicted by available evidence)
- **Sources:**
  - `docs/meeting-mode-ux-copy-catalog.md:28` claims "Messwerkzeuge und Rohdaten existieren (Phase-1-Spike)" for Sleep/Wake- und Geräteverlust-Messung, pointing at "`CSV-Ergebnisse in docs/phase1-spike/run-*/`".
  - Contradicting observation: grep of `docs/phase1-spike/` finds `DEVICE_LOSS|SLEEP|RESUMED|WAKE` markers only inside `tools/*.ps1`; the `run-*/` CSVs contain no such markers, no events log is committed, and the CSV captures are aborted early (see L1, L2). Line 373 of the same catalog similarly asserts these states were "gemessen", which no committed artifact supports.
- **Current impact:** a reader trusting the catalog would believe device-loss/sleep/wake behavior has been measured when it has not. This register entry corrects the record; the catalog file itself was outside this task's write scope. Re-audit 2026-08-23: both catalog passages (`docs/meeting-mode-ux-copy-catalog.md:28` and `:372-373`) were re-read at HEAD and remain uncorrected, so the contradiction stands.
- **Recheck / expiry condition:** expires when either the catalog wording is corrected to match L2, or genuine DEVICE_LOSS/SLEEP/WAKE measurement artifacts are committed (per L2's expiry condition).

---

## Non-goals / complexity-budget cross-reference

This register intentionally documents limitations only. It does not propose features. Per `docs/meeting-mode-plan.md` §2 "Bewusst nicht Bestandteil von v1" (the non-goals table) and the post-Boss discovery discipline, gaps identified here (e.g., missing measurements, unverified external limits) are to be closed by **evidence-gathering**, not by scope growth: no entry above justifies adding new user-facing functionality.
