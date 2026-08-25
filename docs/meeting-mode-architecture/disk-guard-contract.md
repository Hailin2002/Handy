# Disk-Capacity & IO-Failure Guard Contract for Meeting Recordings

Status: **contract draft — no implementation yet** (Phase 2 prerequisite)
Source of truth: `docs/meeting-mode-plan.md`, section 6.x ("Startvalidierung":
free-disk check, plan lines 903–922; "Fehlerverhalten": "Datenträger fast voll",
plan lines 954–956), section 7 ("Persistenzformat": rolling PCM16 chunks,
atomic finalize, plan lines 1004–1030), section 8 ("Azure-Verarbeitung": file
limits, plan lines 1106–1119) and section 12 ("Speicher- und Datenschutzlogik":
~115 MB/h per mono track, ~230 MB/h dual track, retention defaults, plan lines
1438–1453). UX outcomes map to
[docs/meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md)
entries **E7** and **E8**.
Planned implementation homes (all design-phase, none exists):

| Concern                                     | Planned module                                                                                                                             |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Threshold constants                         | `src-tauri/src/audio_toolkit/audio/tolerances.rs` (extends recorder-contract §5.1 table)                                                   |
| Free-space query helper                     | `src-tauri/src/storage/disk_free.rs` (platform wrapper; crate choice — `fs4` vs. direct `GetDiskFreeSpaceExW` — decided at implementation) |
| Pre-flight + monitoring policy              | `src-tauri/src/managers/meeting.rs` (`MeetingManager`)                                                                                     |
| Recorder-side write guard, salvage finalize | `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs` (`MeetingRecorder` writer tasks)                                                   |

This document defines the reliability contract only; it changes no source code.

Sibling contracts consumed read-only:

- [recorder-contract.md](recorder-contract.md) — chunk lifecycle and states
  (§3), rollover/finalize order (§3.1), events/errors (§4, §7), crash-recovery
  protocol (§8). Abbreviated **RC-§n**.
- [asset-store-contract.md](asset-store-contract.md) — retention hooks and the
  automatic-deletion enforcement point (§8), startup scan (§7), atomic commit
  (§5.2). Abbreviated **AS-§n**.
- [session-state-contract.md](session-state-contract.md) — session-state
  authority of `MeetingManager`. Abbreviated **SSC-§n**.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 1. Scope and module boundaries

The disk guard owns exactly one concern: **guaranteeing that a long dual-source
capture degrades predictably under disk exhaustion and write failure — warn
early, never lose finalized audio, stop in a controlled way, and prevent
automatic deletion of sessions that ended in a disk-full state.**

It does _not_ own:

| Concern                                          | Owner                                                    | Interaction                                                                                                               |
| ------------------------------------------------ | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Chunk files, finalize order, recovery protocol   | `MeetingRecorder` (RC-§3, RC-§8)                         | executes the guard's stop/salvage decisions; reports via `RecorderEvent`                                                  |
| Session state machine, Tauri commands/events     | `MeetingManager` (SSC-§1)                                | consumes guard events, drives pre-flight, publishes UX outcomes                                                           |
| Retention policy decision and automatic deletion | `AssetStore`-adjacent sweep + `RetentionGuard` (AS-§8.2) | consults the disk-full end-state flag defined here (§6)                                                                   |
| Storage keys, on-disk layout                     | `AssetStore` (AS-§2, AS-§4)                              | the guarded volume is always the portable-resolved app-data volume (AS-§2.1); there is exactly one monitored volume in v1 |

The guard monitors the volume containing the app-data root only. Other volumes
are irrelevant: every byte the meeting mode writes (audio chunks, manifest,
stereo upload packages, exports) lands under `<app-data-dir>/meetings/<id>/`
(plan §7 layout, AS-§4).

## 2. Volume budget — derived byte figures (gate G1 basis)

Every threshold in this document derives from three plan figures plus
arithmetic. None is invented.

### 2.1 Base rates (plan §12, plan lines 1438–1439)

Plan §12: "PCM16 mit 16 kHz benötigt pro Mono-Spur ungefähr 115 MB pro Stunde,
bei zwei Spuren ungefähr 230 MB pro Stunde." Exact arithmetic behind those
figures (plan §7 fixes the sample format: PCM16, 16 kHz, mono per source):

```text
bytes_per_sample            = 2                    (PCM16, plan §7)
timeline_sample_rate_hz     = 16_000               (plan §7, RC-§2.2)
PER_SOURCE_RATE_BYTES_PER_S = 2 × 16_000           =        32_000 B/s
PER_SOURCE_BYTES_PER_HOUR   = 32_000 × 3_600       =   115_200_000 B  (≈115 MB, plan §12)
DUAL_TRACK_BYTES_PER_HOUR   = 2 × 115_200_000      =   230_400_000 B  (≈230 MB, plan §12)
```

### 2.2 Chunk size (plan §7 Persistenzformat, RC-§5.1)

```text
CHUNK_ROLLOVER_TARGET_MS    = 60_000               (plan §7; RC CHUNK_ROLLOVER_TARGET_MS)
CHUNK_BYTES_MAX_PER_SOURCE  = 32_000 B/s × 60 s    =     1_920_000 B  per finalized chunk
```

A chunk's real payload is marginally larger (44-byte WAV header); the guard
always uses the ceiling above, never a measured average.

### 2.3 Transient cloud-package headroom (plan §8, plan line 1113)

Plan §8: "Eine Datei muss unter fünf Stunden und unter 500 MB liegen."
Stereo upload packages duplicate the audio of their window briefly on disk.
The guard reserves the hard API ceiling as transient headroom whenever the
selected STT provider uploads audio (Azure or Custom):

```text
CLOUD_PACKAGE_CEILING_BYTES = 500_000_000 B          (plan §8 "< 500 MB")
```

Local-only STT adds no such reserve: nothing is packaged for upload.

### 2.4 Manifest upper bound

AS-§6.2 sizes a manifest entry at well under 400 bytes per chunk. A maximal
v1 session (10 h, two sources, 60 s rollover ⇒ ≤ 1200 chunks, AS-§7 V5
rationale) yields ≤ ~480 kB. With a 2× safety factor for markers and session
snapshot:

```text
MANIFEST_RESERVE_BYTES      =      1_048_576 B       (1 MiB ceiling, derived above)
```

## 3. Thresholds (gate G1)

### 3.1 Constant summary

| Constant                               | Value (bytes / ms)                                | Derivation                                                                                                                                                                                                                                                                                                                                                                                                                  |
| -------------------------------------- | ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DISK_STOP_WATERMARK_BYTES`            | 4_888_576                                         | `2 × CHUNK_BYTES_MAX_PER_SOURCE + MANIFEST_RESERVE_BYTES` = `2 × 1_920_000 + 1_048_576`: finalize both open chunks (one per source) plus one full manifest rewrite without ever touching finalized data. Must never be rounded down; implementations may round up only.                                                                                                                                                     |
| `DISK_WARN_WATERMARK_BYTES`            | 235_288_576                                       | `DUAL_TRACK_BYTES_PER_HOUR + DISK_STOP_WATERMARK_BYTES` = `230_400_000 + 4_888_576`: crossing downward guarantees at least **60 minutes** of remaining dual-track capture before a controlled stop becomes necessary — this is the "früh warnen" obligation of plan Fehlerverhalten (line 955).                                                                                                                             |
| `DISK_MONITOR_INTERVAL_MS`             | 30_000                                            | Worst-case consumption between checks is `2 × PER_SOURCE_RATE_BYTES_PER_S × 30 s` = 1_920_000 B — exactly one maximum-size chunk of one source, i.e. ≤ 39 % of even the smallest watermark. Cost: one `stat` call twice per minute. Additionally a check is piggybacked onto every rollover (each source rolls every ~60 s, so rollovers alone yield ≥ 1 check / 30 s on average); the timer is the quiet-session fallback. |
| `DISK_WARN_CLEAR_HYSTERESIS_BYTES`     | 19_200_000                                        | `10 × DISK_MONITOR_INTERVAL_MS × DUAL_TRACK_RATE` where `DUAL_TRACK_RATE = 64_000 B/s`: the warning clears only when free space exceeds the warn watermark by more than five minutes of worst-case recording. This prevents flapping when free space hovers at the watermark while another process churns the disk.                                                                                                         |
| `CLOUD_UPLOAD_TRANSIENT_RESERVE_BYTES` | 500_000_000                                       | plan §8 Azure file ceiling (§2.3). Added to the pre-flight requirement only when the session's provider snapshot uploads audio.                                                                                                                                                                                                                                                                                             |
| `DISK_PREFLIGHT_REQUIRED_FREE_BYTES`   | 235_288_576 (local STT) · 735_288_576 (cloud STT) | `DISK_WARN_WATERMARK_BYTES` (+ `CLOUD_UPLOAD_TRANSIENT_RESERVE_BYTES`): starting inside the warning zone is forbidden, so a session can never begin with less than the guaranteed 60-minute budget.                                                                                                                                                                                                                         |

### 3.2 Pre-flight rule (before session start)

1. `MeetingManager` resolves the app-data volume via the portable-aware logic
   (AS-§2.1) and queries free space through `disk_free`.
2. If `free_bytes < DISK_PREFLIGHT_REQUIRED_FREE_BYTES`, the start button is
   disabled and the focused-repair flow shows the storage explanation
   (§5, outcome U1). This realizes plan line 917–920: "Der Startbutton ist erst
   aktiv, wenn … ausreichend Speicher verfügbar ist" and maps to
   `RecorderError::InsufficientDiskSpace` (RC-§4, RC-§7.2 last row: checked by
   MeetingManager using the §12 budget).
3. The start dialog additionally displays the coarse achievable duration from
   plan §12 (lines 1440–1442): `est_min = floor((free − DISK_STOP_WATERMARK_BYTES) /
3_840_000)` minutes (one dual-track hour = 230_400_000 B / 60).
4. TOCTOU re-check: the same comparison runs again immediately after device
   pre-flight succeeds and before the first chunk is opened; a failure between
   the two checks aborts the start with the identical UX outcome (no partial
   session directory is kept beyond what AS-§4 invariant 5 allows, which the
   startup scan cleans).
5. Resume-after-pause repeats step 1 (cheap `stat`); a session can never
   transition `paused → recording` with free space below the pre-flight
   requirement.

The pre-flight is deliberately conservative about _other_ processes: the
watermarks are lower bounds against the meeting's own consumption; external
consumers are handled by the runtime monitoring of actual free space (§3.3),
never by a modeled budget.

### 3.3 During-capture monitoring and hysteresis

While the recorder state is `Recording` (and only then — a paused session
writes nothing, plan §6.x "Pause"):

1. Every `DISK_MONITOR_INTERVAL_MS` and on every rollover, query free space on
   the session volume.
2. **Warning latch:** `free < DISK_WARN_WATERMARK_BYTES` emits
   `RecorderEvent::DiskSpaceLow { free_bytes }` (RC-§4) exactly once per
   descent; `MeetingManager` surfaces catalog entry E7 (§5, outcome U2). The
   latch clears only when
   `free > DISK_WARN_WATERMARK_BYTES + DISK_WARN_CLEAR_HYSTERESIS_BYTES`,
   which emits a corresponding all-clear event.
3. **Stop tripwire:** `free < DISK_STOP_WATERMARK_BYTES` triggers the
   controlled-stop sequence (§4.2). There is **no upward hysteresis and no
   recovery**: the tripwire is a one-way latch for the lifetime of the session.
   A user's earlier `Keep recording` choice (catalog E7 button) is overridden;
   keeping the guarantee "at least everything captured so far is saved"
   (catalog E8 body) outranks continuation. A new recording requires a fresh
   pre-flight (§3.2), which will refuse to start inside the reserve.
4. Between two checks the modeled drift is bounded (≤ 1_920_000 B, §3.1), so
   the tripwire can overshoot by at most one chunk of one source — which is
   exactly the reserve the watermark carries (two chunks, one per source).
5. Monitoring failures (volume temporarily unreportable) are treated as
   `free = unknown`; two consecutive unknown readings escalate to the
   controlled-stop path with `StopReason::FatalStreamError` semantics — an
   unmonitorable volume during a durability-critical capture is a defect, not
   a comfort problem.

## 4. Write-failure state machine (gate G2)

### 4.1 Error classification

Every filesystem error on the chunk-write path is classified at the four
failure points below:

```text
transient   : retryable once after 500 ms (momentary lock, AV scanner, share hiccup);
              a transient failure that recurs once is reclassified fatal.
fatal_space : ENOSPC / ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL — space will not
              reappear on its own; never retried mid-write.
fatal_io    : any other persistent write/read/fsync/rename error (medium error,
              volume unmounted, quota hard-fail).
```

### 4.2 Controlled-stop sequence (shared tail of every fatal outcome)

Ordered; each step is best-effort but attempted in full:

```text
1. stop ingesting capture frames (drop live audio from now on — past audio is durable)
2. finalize the open chunk of EACH active source per RC-§3.1 property 2 order
   (patch header → fsync file → rename .part→.wav → fsync dir → checksum → DB writing→ready)
   — using salvage-finalize (§4.3) when the ordinary finalize hits a fatal error
3. emit RecorderEvent::ControlledStopInitiated { reason: StopReason::DiskFull }   (RC-§4)
4. persist session end state incl. stopped_reason = DiskFull and ended_disk_full = true (§6)
5. emit Stopped { finalized_chunks }; MeetingManager routes to UX outcomes U3/U4 (§5)
```

Steps 2–5 require no additional data-block allocations beyond
`MANIFEST_RESERVE_BYTES` (header patch and rename are metadata operations on
NTFS; the manifest rewrite fits its reserve), which is why the stop watermark
carries exactly `2 × CHUNK_BYTES_MAX_PER_SOURCE + MANIFEST_RESERVE_BYTES`.

### 4.3 Failure-point matrix

Chunk states are the RC-§3 states `writing | ready | damaged | deleted`.
"Residue" is what exists on disk after the handler completes; "next start" is
the outcome of RC-§8 recovery for that residue.

| #   | Failure point                                                                                                                                    | Immediate handling                                                                                                                                                                                                                                                                                                                                                       | Resulting chunk state(s)                                                                                                                                                    | Filesystem residue                                    | Next-start recovery                                                                                                                                                                                                                          |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F1  | **open** — creating the next temp file at rollover or segment start (incl. its DB tx A)                                                          | Rollover order is finalize-first (RC-§3.1 property 3): the current chunk is finalized _before_ the open is attempted, so F1 never endangers it. Transient → retry open once; `fatal_space`/second failure → controlled stop (§4.2). If the DB insert committed but the file create failed, the `writing` row is rolled back; if rollback itself fails the row survives.  | previous chunk `ready`; no new chunk (or an orphan `writing` row whose file never existed)                                                                                  | none, or at most a stray `writing` row without a file | RC-§8 case 2c: `writing` row without file → `damaged`; nothing was ever written                                                                                                                                                              |
| F2  | **write** — mid-chunk append to the open `.part`                                                                                                 | `fatal_space` is never retried: run salvage-finalize (below) for this chunk only, then controlled stop. `transient` → retry the write once; recurring/`fatal_io` → salvage-finalize, controlled stop with `StopReason::FatalStreamError`. Data-loss bound: the writer flushes at least once per second, so ≤ 32_000 B (= 0.5 s per source) of unflushed frames are lost. | success path: `writing → ready` with `recovered=true` (salvaged tail); salvage failure: `writing → damaged`, file moved to `recovery/` (same-volume rename, needs no space) | either a complete `.wav` or a quarantined `.part`     | none needed (already resolved); if the process died mid-handler instead, RC-§8 case 2a/2b applies to the surviving `.part`                                                                                                                   |
| F3  | **rollover boundary** — old chunk finalized OK, opening the next fails                                                                           | Exactly F1; called out separately because RC-§3.1 property 3 mandates this ordering and forbids losing the just-finalized chunk.                                                                                                                                                                                                                                         | old chunk `ready`; no new chunk                                                                                                                                             | none beyond F1                                        | as F1                                                                                                                                                                                                                                        |
| F4  | **finalize** — header patch / fsync / rename / dir-fsync / checksum / DB commit fails during rollover, pause, stop or the controlled stop itself | Sub-cases: (a) pre-rename failure → file stays `.part`, row stays `writing`; no retry loop (forensics beats tidiness, mirroring AS-§5.2), controlled stop proceeds with the other source. (b) post-rename checksum/DB failure → RC-§3.1 property 2 retry-once, then `ChunkStatePersistFailed`; the renamed file is untouched.                                            | (a) chunk remains `writing`; (b) file complete, row lags                                                                                                                    | (a) `.wav.part`; (b) `.wav` without `ready` row       | (a) RC-§8 case 2a: parseable `.part` → header-patched, truncated to whole frames, renamed → `ready`, `recovered=true`; unparseable → case 2b `damaged`. (b) RC-§8 step 3 reverse reconciliation adopts the file as `ready`, `recovered=true` |
| F5  | **manifest update** during the controlled stop                                                                                                   | Manifest rewrite fits `MANIFEST_RESERVE_BYTES`; if it still fails, SQLite rows remain authoritative (SSC-§1 persistence boundary) and the manifest is rewritten by the AS-§7 startup scan.                                                                                                                                                                               | unaffected                                                                                                                                                                  | stale/absent manifest update                          | AS-§7 reconciles DB ∖ manifest as "manifest stale" and rewrites                                                                                                                                                                              |

### 4.4 Why finalized chunks stay playable (invariant)

1. **Finality is the rename.** A chunk becomes consumable exclusively through
   the atomic `.part → .wav` rename after its WAV header was patched and
   fsynced (RC-§3.1 properties 1–2, AS-§5.2 step 4). After the rename, no code
   path in the meeting mode ever opens a finalized chunk for writing — every
   handler in §4.3 operates only on the currently open `.part` of its source.
2. **Single-writer isolation.** The recorder is the only writer of `audio/`
   (RC-§1); the guard adds no second writer, so no failure handler can race a
   consumer. Consumers observe readiness only via `ChunkFinalized` events /
   the store (RC-§3.1 property 4), never by watching files, so they can never
   observe a half-written final name (the `.part` suffix guarantees this,
   AS-§5.2 Windows note).
3. **Detectability.** Each finalized chunk's SHA-256 is recorded at finalize
   and re-verified at job pickup (AS-§3.2 V4); any post-hoc damage to a
   finalized chunk is surfaced as `damaged`, never silently transcribed.
4. Therefore a disk exhaustion or write failure, however timed, leaves **all
   previously finalized chunks bit-identical and playable** — the strongest
   form of plan §7's guarantee: "Ein Prozessabsturz kann damit höchstens den
   aktuell offenen Chunk beschädigen, nicht die gesamte Aufnahme."

### 4.5 Single recovery chunk on next start

At most one `.part` per source exists at any instant (RC-§3.1 property 1), and
F1/F3 never leave a `.part` behind. Consequently a session that dies in any
disk-full or write-failure state carries **at most one recoverable `.part`
per source**, and RC-§8 produces **at most one recovered chunk per source**
(`ready, recovered=true`, or `damaged` quarantined into `recovery/`) — i.e.
bounded recovery work and a bounded "what happened to my last seconds?"
answer for the recovery banner (catalog E12).

## 5. User-visible outcomes → copy catalog mapping (gate G3)

Catalog: [docs/meeting-mode-ux-copy-catalog.md](../meeting-mode-ux-copy-catalog.md)
(proposal-only; none of these keys exist yet — catalog §1). Surfaces follow
catalog §2.1.

| #   | User-visible outcome                                                     | Trigger (this contract)                     | Catalog entry    | i18n key paths                                                                                                                                   | Notes                                                                                                                                                                                                                                                                                                                                                             |
| --- | ------------------------------------------------------------------------ | ------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| U1  | Start blocked: insufficient space (pre-flight or TOCTOU/resume re-check) | §3.2 step 2                                 | **E7**           | `meeting.warnings.diskLow.title`, `meeting.warnings.diskLow.body`; buttons `meeting.actions.openSettings` (+ `Cancel`)                           | Mapping decision: the catalog defines E7 only as a _running_ warning (surfaces "Startdialog und Statusleiste"); there is no dedicated start-blocked entry. Reusing E7 in the start dialog keeps one vocabulary for "storage almost full"; the disabled primary button carries the blocking semantics (plan line 917–920). Flagged to product/catalog owner in §7. |
| U2  | Low-space warning during capture                                         | `RecorderEvent::DiskSpaceLow` → §3.3 step 2 | **E7**           | as U1; buttons `meeting.actions.keepRecording`, `meeting.actions.endMeeting`, `meeting.actions.openSettings`                                     | E7's interpolated need figure (`0.25 GB/h`) matches `DUAL_TRACK_BYTES_PER_HOUR`; catalog §8 item 3 already ties it to plan §12 measurement confirmation.                                                                                                                                                                                                          |
| U3  | Controlled stop executed, everything captured is intact                  | §4.2 steps 3–5, all chunks `ready`          | **E8**           | `meeting.errors.diskFull.title`, `meeting.errors.diskFull.body`; buttons `meeting.actions.openMeeting`, `meeting.actions.openSettings`           | Surfaces overlay/main/tray/notification per E8. Copy-integrity rule: E8's body claims "Everything captured so far is saved" — it may be shown **verbatim only when zero chunks ended `damaged`**.                                                                                                                                                                 |
| U4  | Controlled stop executed, open chunk could not be salvaged               | §4.2 with ≥ 1 chunk `damaged`               | **E8 + S9**      | E8 title keys; body replaced by `meeting.errors.recoverable.title` + `meeting.errors.recoverable.detail` = `{{detail}}` naming the affected span | Honest-copy rule (plan §6.2 principle 5 "Fortschritt ist ehrlich"): the unsalvageable tail (≤ 60 s + ≤ 0.5 s flush window per source) must not be claimed as saved.                                                                                                                                                                                               |
| U5  | Post-stop processing view after a disk-full end                          | session enters `transcribing`               | **S6**           | `meeting.processing.step.saved`, `meeting.processing.step.prepared_one`, `…transcribed_one`, actions `workInBackground` / `retryFailedSegments`  | Recovered chunks (`recovered=true`) flow into normal STT jobs (RC-§8 step 4); no special UI branch.                                                                                                                                                                                                                                                               |
| U6  | Process died during the disk-full episode; recovery banner at next start | RC-§8 at startup                            | **E12**          | `meeting.recovery.title`, `meeting.recovery.body`; actions `recover` / `decideLater`                                                             | E12's body text is accurate for this case by construction (finalized chunks survive, §4.4).                                                                                                                                                                                                                                                                       |
| U7  | Automatic deletion suppressed for this session                           | §6 guard block                              | none (invisible) | —                                                                                                                                                | Surfaced only indirectly: audio simply persists; the sweep logs the block reason into `change_log` (AS-§8.2 rule 5). No user-facing copy is required or defined.                                                                                                                                                                                                  |

Every user-visible outcome of this contract appears in exactly one row above;
rows without a catalog entry state why (U7), mirroring the RC-§7.2 rule that
variants without consumers are forbidden.

## 6. AssetStore retention interaction (gate G4)

### 6.1 Enforcement point (cross-reference)

The **only** code path that deletes meeting audio without an explicit
user action in the same UI session is the retention sweep described in
[asset-store-contract.md §8.2](asset-store-contract.md): the sweep consults
`RetentionGuard::audio_deletion_allowed(meeting_id)` (implemented by
`MeetingStore`) **inside the sweep loop, per meeting, immediately before the
first unlink**, with no default "allow" implementation. That enforcement point
is adopted here unchanged; this contract adds one input to its decision.

### 6.2 Sessions that ended in a disk-full state

At controlled stop (§4.2 step 4) the session row is persisted with
`ended_disk_full = true`. Normative rule:

1. While `ended_disk_full = true`, `RetentionGuard` MUST return
   `Blocked(SessionEndedDiskFull)` regardless of the stored retention policy —
   including `DeleteImmediately` and expired 7-day/30-day windows. Rationale:
   a disk-full end means the user may currently be _troubleshooting space
   pressure_; silently deleting this session's audio — possibly the exact
   evidence they need — during that episode violates plan §12's spirit
   ("fehlgeschlagene oder noch nicht bestätigte STT-Jobs verhindern
   automatische Audiolöschung") and §6.2 principle 6 (no silent automation for
   sensitive actions). The unanswered-policy default (AS-§8.1) already blocks;
   this rule closes the answered-policy gap.
2. The flag is cleared **only** by an explicit user action: opening the
   meeting and confirming its content is complete (or an explicit level 1–3
   delete, plan §11, which bypasses the guard anyway per AS-§8.2 rule 4).
   Passive completion of background STT jobs does NOT clear it.
3. Interface note: AS-§8.2's `RetentionBlockReason` enum currently has no such
   variant. Adding `SessionEndedDiskFull` is a required extension owned by the
   storage workstream; until it lands, implementors MUST treat this contract as
   authoritative on the decision and MUST NOT ship a guard that ignores the
   flag. Recorded as an open interface item in §7.
4. The block writes a `change_log` row with the reason on every skipped sweep
   pass (AS-§8.2 rule 5), keeping "warum ist mein Audio noch da?" answerable in
   both directions.

## 7. Implementation status and open items (gate G4)

**Nothing in this contract is implemented.** Verified against the working tree
on 2026-08-22: no `MeetingRecorder`, `MeetingManager`, `disk_free` module or
`DISK_*` constant exists under `src-tauri/src/` (grep over the backend tree
matches only the Phase-1 spike `src-tauri/src/bin/meeting_loopback_spike.rs`);
no meeting i18n keys exist under `src/i18n/locales/en/translation.json`. All
modules named in the header table are design-phase targets.

Open items handed to the implementing phases:

1. `RetentionBlockReason::SessionEndedDiskFull` extension to AS-§8.2 plus the
   `ended_disk_full` column on `meeting_sessions` (§6.2) — both touch the
   plan-§9 session schema, so the storage workstream owns the migration.
2. Catalog addition for an explicit start-blocked copy distinct from E7, if
   product review wants one (§5 U1 note); until then E7 is binding.
3. Free-space query mechanism (`fs4` crate vs. direct Win32
   `GetDiskFreeSpaceExW`) decided at implementation; the contract depends only
   on a `Result<u64>` free-bytes query of the app-data volume.
4. The 0.25 GB/h figure embedded in E7's copy text must be re-measured on
   target hardware in Phase 2 alongside RC-§5.1 pending constants; the
   thresholds themselves derive arithmetically from the fixed sample format
   and change only if the format changes.

## 8. Verification hooks (maps to plan §14 automated tests)

- Constant tests assert the §3.1 values equal their §2 derivations (formula
  literals in `tolerances.rs`, no magic numbers elsewhere).
- Fault-injection filesystem seam (same pattern as RC-§9): inject
  `fatal_space`/`fatal_io` at each of F1–F4 and assert the exact chunk states,
  residue and §4.2 ordering; assert zero writes touch any `ready` chunk file
  after its rename (instrument the seam).
- Hysteresis tests: descend below warn watermark once → one event; oscillation
  within `DISK_WARN_CLEAR_HYSTERESIS_BYTES` → no clear; tripwire crossing →
  exactly one controlled stop; `Keep recording` cannot suppress it (§3.3 step 3).
- Recovery tests: kill the process between every pair of §4.2 steps and confirm
  ≤ one recovered chunk per source after RC-§8 (§4.5).
- Sweep tests: `ended_disk_full = true` blocks every policy including
  `DeleteImmediately`; explicit user confirm clears the flag; blocked passes
  log to `change_log`.

---

## Appendix A — Gate traceability

- **G1** (numeric thresholds with plan-derived values): §2 arithmetic from plan
  §12 (115/230 MB per hour, lines 1438–1439), §7 (PCM16 16 kHz mono, 60 s
  rollover) and §8 (500 MB package ceiling, line 1113); §3.1 table evaluates
  every constant from those figures.
- **G2** (write-failure state machine over open/write/rollover/finalize):
  §4.1 classification, §4.3 matrix F1–F4 with resulting chunk states and
  residues, §4.4 finalized-chunk playability invariant, §4.5 single-recovery-
  chunk bound.
- **G3** (every user-visible outcome mapped to a catalog ID and i18n key path):
  §5 table U1–U7 citing entries E7, E8, S6, S9, E12 with concrete
  `meeting.*` key paths and the honest-copy rules for E8.
- **G4** (retention cross-reference + nothing-implemented statement +
  prettier): §6 cites AS-§8.2's enforcement point verbatim and extends it with
  `SessionEndedDiskFull`; §7 states the not-implemented status with grep
  evidence; formatting verified with `prettier --check` (see mission report).
