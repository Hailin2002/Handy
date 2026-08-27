# Phase 1 Spike Tooling — Analyzer SUMMARY, Fixture Validation, and events.log Markers

Status: **reference documentation** for the Phase 1 loopback-spike tooling in
this directory. Everything below was cross-checked against the actual scripts
and the committed run artifacts (see the audit in §4/§5).

Contents:

1. [Tool inventory](#1-tool-inventory)
2. [`analyze-spike-csv.ps1` — SUMMARY field reference](#2-analyze-spike-csvps1--summary-field-reference)
3. [Known-answer fixture validation](#3-known-answer-fixture-validation)
4. [events.log marker vocabulary and run audit](#4-eventslog-marker-vocabulary-and-run-audit)
5. [Aborted captures — do not pool as 120-min evidence](#5-aborted-captures--do-not-pool-as-120-min-evidence)

---

## 1. Tool inventory

| Script                                                 | Purpose                                                                                                                                                    |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run-120min.ps1`                                       | Drives the full 120-min spike run: starts the binary, alternates silence/tone periods, writes `events.log` (run-120min.ps1:1–5).                           |
| `device-loss-test.ps1`                                 | Elevated; emulates device unplug by disabling/re-enabling the PnP MEDIA device node; appends markers to the run's `events.log` (device-loss-test.ps1:1–6). |
| `sleep-wake-test.ps1`                                  | Elevated; performs a real S3 sleep/RTC-wake cycle during a run and logs the markers (sleep-wake-test.ps1:1–7).                                             |
| `log-event.ps1`                                        | Appends one free-form timestamped marker to `events.log` (log-event.ps1:1–3).                                                                              |
| `play-tone.ps1`                                        | Plays the scenario tone (spawned by `run-120min.ps1:72–74`).                                                                                               |
| `set-default-output.ps1`, `set-default-output-uia.ps1` | Change the Windows default output device before a run.                                                                                                     |
| `analyze-spike-csv.ps1`                                | Post-run analysis: CSV + optional `events.log` → markdown SUMMARY (analyze-spike-csv.ps1:1–4).                                                             |

Usage (analyze-spike-csv.ps1:4):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File analyze-spike-csv.ps1 -CsvPath <log.csv> [-EventsPath <events.log>] -OutPath <SUMMARY.md>
```

---

## 2. `analyze-spike-csv.ps1` — SUMMARY field reference

All citations refer to `docs/phase1-spike/tools/analyze-spike-csv.ps1`.
Per-source metrics are computed over **non-error rows only** (line 79);
rows with a non-empty `error` column are counted separately (line 80).
Every value below was verified against the known-answer fixture (§3).

### 2.1 Run metadata section

Emitted from the CSV's `# `-prefixed lines (lines 19–25, emitted at 73–75).

| Field            | Unit      | Computation                                                                                                                                                                                                                                                                                                                                                                               |
| ---------------- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<key>: <value>` | as logged | Parsed by regex `^([A-Za-z_]+)[ =](.*)$` into an ordered hashtable (line 22). **Limitation:** duplicate `# key=` lines collapse **last-wins** — each match overwrites the previous value (line 22). Keys must match `[A-Za-z_]+`; keys containing digits or hyphens are dropped (lines 22–23). Unparseable values become empty strings, not errors. Demonstrated by fixture check 1 (§3). |

### 2.2 Per-source metric table

| SUMMARY row (emit line)                                           | Unit                                        | Computation source                                                                                                                                                                                                                                                                                                                           |
| ----------------------------------------------------------------- | ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Callbacks` (119)                                                 | count                                       | Number of non-error rows for the source (line 79).                                                                                                                                                                                                                                                                                           |
| `Stream errors` (120)                                             | count                                       | Rows of the source with non-empty `error` column (line 80).                                                                                                                                                                                                                                                                                  |
| `Config` (121)                                                    | Hz / channels / format                      | `sampleRate`, `channels`, `sampleFormat` copied from the **first** non-error row (lines 82–83). Not re-derived.                                                                                                                                                                                                                              |
| `Measurement span (last callback)` (122)                          | seconds (1 decimal)                         | Last non-error row's `t_rel_ms` ÷ 1000 (line 84, emit 122).                                                                                                                                                                                                                                                                                  |
| `Total frames` (123)                                              | frames                                      | Last row's cumulative `totalFrames` column (line 112).                                                                                                                                                                                                                                                                                       |
| `Max gap` (124)                                                   | milliseconds (3 decimals)                   | Maximum `gapMs` over non-error rows via `Measure-Object -Maximum` (line 85). **Includes the first row's startup latency** — for the first row of a source, `gap_ms` equals its own `t_rel_ms`, not a real gap (verified in the committed CSVs, e.g. `run-20260822` first microphone row: `t_rel_ms=14.339, gap_ms=14.339`).                  |
| `Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s)` (125) | counts                                      | Counts of the `gap_class` **string** per row (lines 100–102): `normal` / `sparse` / `gap`. The analyzer trusts the classification written by the spike binary; it does **not** recompute class boundaries from `gap_ms`.                                                                                                                     |
| `Time in sparse gaps` (126)                                       | seconds (3 decimals)                        | Sum of `gapMs` over `sparse` rows ÷ 1000 (lines 105–107).                                                                                                                                                                                                                                                                                    |
| `Time in long gaps (>1 s)` (127)                                  | seconds (3 decimals)                        | Sum of `gapMs` over `gap` rows ÷ 1000 (lines 108–110).                                                                                                                                                                                                                                                                                       |
| `Drift at end` (128)                                              | frames (1 decimal) and ms (3 decimals)      | Last row's `drift` (line 86); ms conversion `drift / sampleRate × 1000` (line 113). Negative = source clock lags wall clock.                                                                                                                                                                                                                 |
| `Drift range` (129)                                               | frames (1 decimal)                          | `min .. max` of `drift` over non-error rows (lines 87–91).                                                                                                                                                                                                                                                                                   |
| `Drift trend (stable window >10 s)` (130)                         | frames (1 decimal) and frames/h (1 decimal) | Over rows with `t_rel_ms > 10000` (line 93): first stable `drift` → last `drift` (lines 94, emit 130); rate = `(last_drift − first_drift) / Δt_hours` with `Δt` between first and last stable row (lines 95–99). With fewer than two stable rows the rate is `0` (line 96) — the fixture shows this degenerate case for the loopback source. |
| `Silent fraction (callbacks)` (131)                               | percent (2 decimals) + raw fraction         | Count of rows with `silent == 'true'` (line 103) ÷ non-error row count (line 104). The analyzer reads the `silent` column; it does not re-evaluate `peak` against the threshold.                                                                                                                                                             |

### 2.3 Conditional sections

| Section                                     | Emitted when                                                    | Content                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `### Stream errors` (133–140)               | ≥ 1 error row                                                   | Table of `t_rel_ms` + verbatim `error` text per error row.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `### Top 10 largest gaps` (141–148)         | any `gap`-class row, **or** the largest gap > 250 ms (line 141) | Top 10 rows by `gapMs` descending (line 111): `t_rel_ms`, `gap_ms`, class, `silent`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `## Cross-source comparison` (151–168)      | both sources have non-error rows                                | `Relative drift loopback vs microphone at end`: last-row drift difference in frames and ms — ms uses the **loopback** sample rate (lines 157, 159); `after <span> s` uses the **minimum** of the two sources' last `t_rel_ms` (line 158). `Relative drift rate`: drift difference ÷ (min-span in hours) (lines 161–163). **Caution:** with unequal spans (one source dying early) this rate is inflated by construction — the fixture demonstrates an extreme value (7,674,480 frames/h from a 0.5 s overlap). `Max gap microphone / loopback` (164–166). |
| `## Event log (scenario markers)` (171–178) | `-EventsPath` given **and** file exists (line 171)              | The events file is embedded **verbatim** inside a fenced `text` block (lines 174–176). No parsing, filtering, or correlation happens — correlation is done by the human reader.                                                                                                                                                                                                                                                                                                                                                                           |

Numeric parsing note: unparseable numeric fields silently become `0` /
`0.0` (`ToD`/`ToI`, lines 16–17) — a corrupted row degrades quietly instead
of failing the analysis. Output is written UTF-8 with BOM (line 181).

---

## 3. Known-answer fixture validation

The analyzer is validated against a hand-computed known-answer fixture. The
validation artifacts live in:

```
src-tauri/target/spike-analysis/analyzer-fixtures/
├── known-answer.csv            fixture input (2 sources, all 3 gap classes,
│                               an error row, silent rows, drift trend rows,
│                               a duplicated `# duration_s=` meta line)
├── known-answer-events.log     3-line events fixture (RUN_LAUNCH/TONE_START/TONE_END)
├── SUMMARY-known-answer.md     analyzer output under test
└── verdict.md                  the validation verdict
```

**Verdict: PASS — 20/20 checks** (`verdict.md`, generated 2026-08-22). Each
check asserts one hand-computed SUMMARY substring (e.g. gap histogram
`1 / 2 / 3`, drift trend rate `-124800 frames/h`, silent fraction
`33.33 % (2/6)`, cross-source drift `1065.9 frames (24.17 ms)`); the full
expected-vs-found table is in the verdict file.

**Caveat — outside version control:** this directory sits under
`src-tauri/target/`, which is git-ignored (`src-tauri/.gitignore`,
line `/target/`). The verdict is therefore **not committed evidence**; it
must be regenerated locally after any analyzer change. It was regenerated
from scratch on 2026-08-22 by running the analyzer on the fixture in a clean
worktree (no prior `analyzer-fixtures` artifacts existed anywhere in the
mission worktrees or the main checkout).

**Documented meta limitation (demonstrated, not just claimed):** the fixture
contains `# duration_s=60` followed by `# duration_s=99`. Because the parser
assigns by key (analyze-spike-csv.ps1:22), the emitted Run-metadata section
shows only `duration_s: 99` — the first value is silently lost (verdict
check 1, "meta last-wins collapse"). Consequence for operators: a CSV with
repeated metadata keys reports only the last value; do not rely on duplicate
meta lines to record corrections.

---

## 4. events.log marker vocabulary and run audit

`events.log` is a sidecar file written by `run-120min.ps1` (one per run,
run-120min.ps1:16–17) and appended to by the fault-injection tools. Line
format: `yyyy-MM-dd HH:mm:ss.fffzzz <MESSAGE>` (run-120min.ps1:19–23).

### 4.1 Marker vocabulary

Every marker the three tools can emit, with its definition site:

| Marker                                                    | Tool                 | Defined at                         |
| --------------------------------------------------------- | -------------------- | ---------------------------------- |
| `RUN_LAUNCH`                                              | run-120min.ps1       | line 28                            |
| `RUN_STARTED`                                             | run-120min.ps1       | line 33                            |
| `TONE_START`                                              | run-120min.ps1       | line 71                            |
| `SILENCE_START`                                           | run-120min.ps1       | line 76                            |
| `TONE_KILLED_AFTER_TIMEOUT`                               | run-120min.ps1       | line 83                            |
| `TONE_END`                                                | run-120min.ps1       | line 86                            |
| `RUN_WAIT_EXIT`                                           | run-120min.ps1       | line 98                            |
| `RUN_DID_NOT_EXIT_AFTER_TIMEOUT`                          | run-120min.ps1       | line 99                            |
| `RUN_FINISHED`                                            | run-120min.ps1       | line 100                           |
| `DEVICE_LOSS_ABORT`                                       | device-loss-test.ps1 | line 24                            |
| `DEVICE_LOSS_START`                                       | device-loss-test.ps1 | line 25                            |
| `DEVICE_DISABLE_FAILED`                                   | device-loss-test.ps1 | line 28                            |
| `DEVICE_DISABLED`                                         | device-loss-test.ps1 | line 30                            |
| `DEVICE_ENABLE_FAILED`                                    | device-loss-test.ps1 | line 35                            |
| `DEVICE_REENABLED`                                        | device-loss-test.ps1 | line 37                            |
| `DEVICE_RECONNECT_CONFIRMED` / `DEVICE_RECONNECT_TIMEOUT` | device-loss-test.ps1 | line 47                            |
| `RESUMED_FROM_SLEEP`                                      | sleep-wake-test.ps1  | line 24                            |
| `SLEEP_TEST_PREP`                                         | sleep-wake-test.ps1  | line 40                            |
| `SLEEP_ARMED`                                             | sleep-wake-test.ps1  | line 51                            |
| `SLEEP_ENTER`                                             | sleep-wake-test.ps1  | line 53                            |
| `SLEEP_SCRIPT_RESUMED`                                    | sleep-wake-test.ps1  | line 56                            |
| `SLEEP_TEST_DONE`                                         | sleep-wake-test.ps1  | lines 62, 64                       |
| _(free-form message)_                                     | log-event.ps1        | line 9 (`-Message` passed through) |

### 4.2 Audit: markers actually present in the four committed runs

**None of the four committed run directories contains an `events.log` at
all.** `git ls-files docs/phase1-spike` lists exactly one file per run
directory — `meeting_loopback_spike_120min.csv` — and `events.log` matches
the root `.gitignore` rule `*.log` (line 3, verified with
`git check-ignore -v`). The committed marker count per run is therefore
**zero for every marker in the table above**:

| Run directory                     | Committed files | TONE_START | DEVICE_LOSS\* | SLEEP/WAKE\* | RUN_FINISHED | Any marker |
| --------------------------------- | --------------- | ---------- | ------------- | ------------ | ------------ | ---------- |
| `run-20260822`                    | CSV only        | 0          | 0             | 0            | 0            | **0**      |
| `run-20260822-r2`                 | CSV only        | 0          | 0             | 0            | 0            | **0**      |
| `run-20260822-r2-attempt1-reaped` | CSV only        | 0          | 0             | 0            | 0            | **0**      |
| `run-20260822-r3`                 | CSV only        | 0          | 0             | 0            | 0            | **0**      |

Working-copy cross-check (verified inside the isolated task worktree): no
`events.log` exists **anywhere on disk** in the worktree either — a recursive
search over `docs/phase1-spike/` finds only the four CSVs and the tool
scripts. Consequently **zero markers from the table above are observable in
any committed or worktree artifact**. That the runs died during the first
silence window follows from the committed data alone: every capture span
(97.4 / 179.7 / 10.9 / 146.6 s, §5) is below the first scheduled tone at
t=300 s (run-120min.ps1:39), so no tone marker could ever have been logged,
and the missing `# run_finished` metadata (§5) shows the binary never
completed — matching a runner killed mid-silence with no exit marker written.
Note: the directory name `run-20260822-r2-attempt1-reaped` records that this
was the first r2 attempt whose process was reaped; its CSV was subsequently
moved into its own directory.

---

## 5. Aborted captures — do not pool as 120-min evidence

**All four committed runs are aborted captures and are unsuitable for the
120-minute acceptance criterion.** Evidence, per run (measured from the
committed CSVs; requested duration is `# duration_s=7200` in every file):

| Run directory                     | Requested | Actual capture span (last `t_rel_ms`, both sources) | `# run_finished` line | `# SUMMARY` blocks |
| --------------------------------- | --------- | --------------------------------------------------- | --------------------- | ------------------ |
| `run-20260822`                    | 7200 s    | **97.4 s**                                          | absent                | absent             |
| `run-20260822-r2`                 | 7200 s    | **179.7 s**                                         | absent                | absent             |
| `run-20260822-r2-attempt1-reaped` | 7200 s    | **10.9 s**                                          | absent                | absent             |
| `run-20260822-r3`                 | 7200 s    | **146.6 s**                                         | absent                | absent             |

In the spike binary (`src-tauri/src/bin/meeting_loopback_spike.rs`),
`# run_finished elapsed_s=…` and both per-source `# SUMMARY` blocks are
written **only after the bounded duration is reached** (binary lines
747–750); an interrupted capture skips them ("Ctrl+C aborts without
summary", binary line 713). All four CSVs end after
`# streams_started` with no summary — i.e. all four were terminated early
(97.4 s / 179.7 s / 10.9 s / 146.6 s of a requested 7200 s, i.e. 1.4 % /
2.5 % / 0.15 % / 2.0 % of the target duration), consistent with the
truncated `events.log` files described in §4.2.

**Consequence:** these directories must not be pooled, aggregated, or cited
as 120-minute evidence for the Phase 1 acceptance (plan §13 Phase 1). A
valid 120-min evidence run requires: full `# run_finished elapsed_s=…`
metadata, both `# SUMMARY` blocks, and an `events.log` ending in
`RUN_FINISHED exitcode=0`.
