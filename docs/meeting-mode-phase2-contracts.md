# Phase-2/5 Dual-Source Audio Data Contract

Status: **contract draft — no implementation yet** (defines the Phase‑2
`MeetingRecorder` output consumed by the Phase‑5 STT pipeline)
Companion documents: [recorder-contract.md](meeting-mode-architecture/recorder-contract.md)
(interface/state machine), [stt-provider-contract.md](meeting-mode-architecture/stt-provider-contract.md)
(provider abstraction). This document owns the **data** contract: what exists on
disk, what the in-memory descriptors contain, and how consumers merge them.
Source of truth: [meeting-mode-plan.md](meeting-mode-plan.md) §7 "Persistenzformat"
and "Gemeinsame Zeitachse", §8 "Gemeinsames Segmentmodell", §9
(`meeting_audio_chunks`), §13 Phase 2 and Phase 5, §14 Testmatrix rows
"Timeline-, Gap- und Drift-Berechnungen", "Chunk-Rollover und atomare
Finalisierung", "Recovery beschädigter/offener Chunks".
This document specifies contracts only; it changes no source code.

---

## 0. Contract parties

| Direction | Producer | Consumer |
| --- | --- | --- |
| Capture → disk | `MeetingRecorder` (Phase 2, planned home `src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`) | `MeetingStore` (persistence), crash-recovery scan |
| Disk → transcription | `MeetingStore` + `RecorderEvent::ChunkFinalized` | `MeetingTranscriptionQueue` (Phase 5), VAD segmenter |
| Chunks → providers | Queue-built segment jobs / Azure stereo packages | `LocalMeetingProvider`, `AzureMeetingClient`, `CustomMultipartProvider` |

The recorder is the **single writer** of `meetings/<meeting-id>/audio/` while a
session is active (recorder-contract.md §1). Everything below is normative.

## 1. On-disk layout and chunk naming scheme

Fixed layout (plan §7 "Persistenzformat", verbatim):

```text
meetings/<meeting-id>/
  manifest.json
  audio/
    000001-microphone.wav
    000001-system.wav
    000002-microphone.wav
    000002-system.wav
  export/
  recovery/
```

Naming rules (normative):

1. **Pattern:** `<sequence:%06u>-<file_tag>.wav` with `file_tag ∈ { microphone,
   system }` (plan §2: technical provenance `microphone` | `system`, never speaker
   identification). The tags match `SourceKind::file_tag()` in
   recorder-contract.md §2.1.
2. **Sequence numbers** are `u32`, zero-padded to six digits, **monotonic and
   independent per source** within one session. Mic chunk `000007` may exist
   without system chunk `000007`; nothing may assume cross-source sequence
   equality (plan §5: two technically separated sources; recorder-contract §2.1).
   Sequence numbers are never reused within a session, including across recovery
   (Testmatrix: "Chunk-Rollover und atomare Finalisierung").
3. **Temp name:** a chunk being written lives as `<sequence>-<tag>.wav.part`.
   `.wav.part` is never a valid input for consumers (§6).
4. **Identity:** the durable on-disk identity of a chunk is the pair
   `(source, sequence)` inside one `<meeting-id>`. The SQLite primary key of
   `meeting_audio_chunks` (plan §9) is store-owned; manifests additionally carry
   `chunk_uuid` (stable UUID, plan §9 "stabile UUIDs … keine
   Autoincrement-Abhängigkeit") so later server/sync layers can reference chunks
   without exposing row ids.
5. **Storage keys** stored anywhere (DB or manifest) are always relative to the
   meeting directory (`audio/000001-microphone.wav`). Absolute paths are
   forbidden as persistent identity (plan §9).
6. `recovery/` receives unparseable/orphan `.part` files during crash recovery,
   keeping their original names for forensics (Testmatrix: "Recovery
   beschädigter/offener Chunks"; protocol detail: recorder-contract.md §8).

### WAV payload format (every chunk)

| Property | Value | Trace |
| --- | --- | --- |
| Encoding | PCM16 little-endian (`RIFF/WAVE`) | plan §7 "rollierenden PCM16-WAV-Dateien" |
| Sample rate | 16 000 Hz exactly | plan §7 "gemeinsame Zielzeitachse arbeitet mit 16 kHz Mono" |
| Channels | 1 (mono per source) | plan §7; plan §3.5 keeps sources separated even for Azure |
| Normalization | downmix per dictation semantics; `f32` clamp `[-1,1]`, ×32767, round-to-nearest | recorder-contract.md §2.3 (reuse obligation, plan §5) |

The WAV header of a *writing* chunk carries provisional sizes; they become
authoritative only through finalization (§5).

## 2. `manifest.json` schema (session manifest)

Purpose: a crash-recoverable, human-inspectable index of the session's chunks on
the same storage medium as the audio. During normal operation `MeetingStore`
(SQLite, plan §9) is authoritative; the manifest is written at session start and
atomically appended after every chunk state change (§5.3). After a process crash,
recovery reconciles manifest + filesystem against the database (§7).

```jsonc
{
  "manifest_version": 1,              // u32, bumps on breaking change (plan §9 "Schema- und Parser-Versionen")
  "parser_min_version": 1,            // u32, lowest version a reader accepts
  "meeting_id": "<uuid-v4>",          // string, == directory name, == meeting_sessions.id (plan §9)
  "created_wall_clock": "<RFC3339>",  // display/metadata only; NEVER used for audio math
  "timeline": {                       // see §3
    "sample_rate_hz": 16000,          // u32 const
    "origin_monotonic": "session_start",
    "initial_offset_fill_ms": 12.4,   // f64 ≥ 0; leading silence pre-filled so both tracks share t=0
    "markers": [                      // pause/resume pairs, oldest first
      { "kind": "pause",  "timeline_ms": 1234567.5 },
      { "kind": "resume", "timeline_ms": 1234567.5 }
    ]
  },
  "sources": {
    "microphone": {                   // optional: absent if session is mic-only fallback
      "native_sample_rate_hz": 48000,// u32, first-callback observation (spike col `sample_rate`)
      "native_channels": 2,           // u16 (spike col `channels`)
      "native_sample_format": "f32",  // string (spike col `sample_format`)
      "device_label": "Analogue 1 + 2 (Focusrite USB Audio)"
    },
    "system": { "...same shape...": 0 }
  },
  "chunks": [ /* ChunkManifestEntry, one per chunk ever opened */ ]
}
```

Rules:

- Unknown fields MUST be ignored by readers (forward compatibility via
  `manifest_version`); writers only append, never rewrite history entries.
- Secrets are never written into manifests (plan §9: "Secrets werden … niemals in
  … Manifesten … geschrieben"). Device labels are non-secret configuration
  snapshots as permitted for `meeting_sessions`.

## 3. Shared monotone timeline semantics (mic vs. loopback)

Normative semantics — identical to recorder-contract.md §2.2, restated because
every timestamped field in this contract depends on them:

1. **One origin per session.** A single monotonic clock reading
   (`std::time::Instant`) captured at session start defines `timeline_ms = 0`.
   Every capture callback of both sources is stamped at callback entry;
   `timeline_ms = elapsed_since_origin_ms`. Wall-clock values are display
   metadata only (plan §7: "Beide Streams erhalten beim Eingang einen Zeitstempel
   aus derselben monotonen Uhr").
2. **Per-source delivery accounting.** Each source independently counts delivered
   device frames (`total_frames`) at its native rate and computes
   `expected(t) = (t − t_first_callback_ms)/1000 × native_sample_rate` — exactly
   the spike computation behind columns `frames`, `total_frames`,
   `expected_frames`, `drift_frames`
   (`docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv`, header line 6).
3. **Capture-time timeline; pauses removed.** Paused intervals contribute no
   samples and no time: audio after a resume continues at the frozen position
   (plan §6.x "Pause": "Pause schließt den gerade offenen Aufnahmeabschnitt sauber
   ab und erzeugt einen Timeline-Marker. Während der Pause wird kein Audio
   gespeichert"). Gap detection must never classify a paused span as a gap.
4. **Tracks are never shortened.** Missing time on a source is represented as
   digital silence at its correct timeline position; written audio is never
   shifted or re-timestamped (plan §7 sync task 2: "fehlende Zeitbereiche … mit
   Stille repräsentieren, statt die Spur zu verkürzen").
5. **Initial offset.** Whichever source starts first defines t=0 content; the
   other source's lead time is pre-filled with silence up to the measured startup
   offset (recorded once per session as
   `timeline.initial_offset_fill_ms`) — plan §7 sync task 1
   ("Anfangsversatz … durch Stille auffüllen"). Measured magnitudes come from
   `cb_index=1` rows' `t_rel_ms` in the spike runs (~11 ms observed on the
   Focusrite pairing, runs r2/r3 of 2026‑08‑22).
6. **Ranges are half-open** `[timeline_start_ms, timeline_end_ms)`; all timeline
   fields are milliseconds as `f64`.
7. **Cross-source independence.** There is no cross-source frame interleaving
   guarantee; synchronization happens exclusively through shared timeline
   coordinates. A defect in one source's accounting can therefore never shift the
   other source's timestamps (supports gate G3; plan Phase‑2 acceptance "Ausfall
   einer Quelle zerstört die andere nicht").

## 4. Per-chunk manifest entry (`ChunkManifestEntry`)

Every field carries type + semantics + trace. In-memory this is
`ChunkDescriptor`/`ChunkPatch` in recorder-contract.md §3; the JSON form below is
its persisted projection. Optional (`Option<T>` / absent-in-JSON) fields are
marked.

| Field | Type | Semantics | Trace |
| --- | --- | --- | --- |
| `chunk_uuid` | string (UUIDv4) | stable cross-store identity (§1 rule 4) | plan §9 sync-ready IDs |
| `source` | `"microphone" \| "system"` | technical source provenance; persisted verbatim as `meeting_audio_chunks.source` | plan §2, §8 `source`, §9 |
| `sequence` | u32 | `%06u` filename component, monotonic per source, never reused | plan §7 layout, §9 `sequence`; Testmatrix "Chunk-Rollover…" |
| `storage_key` | string | relative path of the final file, e.g. `audio/000001-microphone.wav` | plan §9 `path` + "keine absoluten Dateipfade" |
| `state` | `"writing" \| "ready" \| "damaged" \| "deleted"` | lifecycle state; only `ready` is consumable (§6). For `deleted`, `storage_key` remains as a record but the file content is unlinked (retention/user deletion, plan §9) | plan §9 `state` |
| `finalized` | bool | **finalization marker**: `true` iff the atomic rename succeeded and header sizes are authoritative. Implies `state ∈ {ready, deleted}`; `false` ⇔ `writing`/`damaged` | Testmatrix "Chunk-Rollover und atomare Finalisierung"; plan §7 "atomar umbenannt" |
| `timeline_start_ms` | f64 | monotone timeline position of the chunk's first sample (half-open range start) | plan §9 `timeline_start_ms`; §7 "Gemeinsame Zeitachse" |
| `timeline_end_ms` | f64 | `start + sample_count / 16000 × 1000`; range end (exclusive). For `damaged`/`writing`: last known intended value, marked non-authoritative by `finalized:false` | plan §9 `timeline_end_ms`; Testmatrix "Timeline-, Gap- und Drift-Berechnungen" |
| `sample_rate_hz` | u32 (=16000) | timeline rate of contained audio | plan §9 `sample_rate`; §7 |
| `channels` | u16 (=1) | mono track | plan §7 |
| `sample_format` | `"pcm16le"` | payload encoding | plan §7 |
| `sample_count` | u64 | PCM16 frames contained (bytes/2). Invariant: `(end−start)` ms ≡ `sample_count` @16 kHz | plan §9 `sample_count` |
| `byte_size` | u64 | finalized file size in bytes | plan §9 `file_size` |
| `checksum_sha256` | Option\<string hex\> | SHA‑256 over finalized file bytes; `None` while `writing`/`damaged` | plan §9 "Prüfsumme" |
| `source_segment_id` | string (UUIDv4) | device-continuity segment this chunk belongs to (new id at open/reconnect/device-change/resume-with-different-device) | plan §6.x "Pause": "ein Gerätewechsel wird als neuer Quellenabschnitt dokumentiert"; Fehlerverhalten "Standardgerät wechselt" |
| `device_label` | string | non-secret device snapshot of that segment | plan §9 "Gerätekennung" |
| `gap_stats.gaps_filled` | u32 | count of input gaps silence-filled inside this chunk | Testmatrix "Timeline-, Gap- und Drift-Berechnungen"; plan §7 sync task 2 |
| `gap_stats.total_filled_ms` | f64 ≥ 0 | sum of filled gap durations (spike `gap_ms` aggregate) | same |
| `gap_stats.max_gap_ms` | f64 ≥ 0 | largest single gap in the chunk | same |
| `gap_stats.classes` | `{normal: u32, sparse: u32, long: u32}` | histogram using the **spike harness classes**: ≤250 ms / ≤1000 ms / >1000 ms (boundaries implemented in `docs/phase1-spike/tools/analyze-spike-csv.ps1`, column `gap_class`) | Testmatrix row above; Phase‑1 acceptance "gemessene Gap-/Drift-Werte … dokumentiert" |
| `drift.delivered_frames_end` | i64 | cumulative delivered device frames at chunk end (spike `total_frames` trend) | Testmatrix row above |
| `drift.frames_delta_end_ms` | f64 | net drift at chunk end expressed in ms (`drift_frames/native_rate×1000`, spike formula used by `analyze-spike-csv.ps1`) | same |
| `drift.correction_applied` | bool | whether resampler-ratio compensation was active at any point in this chunk | plan §7 sync task 3 ("Langsame Clock-Abweichung über den vorhandenen Resampler kontrolliert ausgleichen") |
| `drift.applied_ratio_ppm` | Option\<f64\> | last applied ratio offset in ppm when `correction_applied` | same |
| `drift.corrected_samples` | u64 | total samples produced by corrected-ratio resampling in this chunk | same |
| `drift.tolerance_exceeded` | bool | drift left the correction envelope; chunk contains uncorrected audio, source flagged degraded | recorder-contract.md §5.3(4); pending-value constant `DRIFT_EXCEEDED_FRAMES` (task-0008) |
| `recovered` | bool | chunk was completed by crash recovery, not by clean finalization | Testmatrix "Recovery beschädigter/offener Chunks" |
| `created_wall_clock` | RFC3339 | display metadata only | plan §9 `created_at` analog |

Field-to-spike mapping (gate G4). The verified spike CSV column set — identical
header in all four runs (`run-20260822`, `-r2`, `-r2-attempt1-reaped`, `-r3`;
header at line 6):
`source, cb_index, t_rel_ms, gap_ms, frames, total_frames, sample_rate, channels,
sample_format, expected_frames, drift_frames, gap_class, silent, peak, rms, error`
plus `#`-metadata lines (`silence_threshold=0.0001`, per-source config lines) —
maps onto contract fields as:

| Spike column / meta | Contract consumer |
| --- | --- |
| `source` (`microphone`/`loopback`) | `source` (`loopback` is renamed to the product term `system`) |
| `sample_rate`, `channels`, `sample_format`, config meta lines | `sources.<s>.native_*` (session manifest, §2) |
| `t_rel_ms` | timeline stamping model (§3); replay-test input for `timeline_start_ms` math |
| `gap_ms`, `gap_class` | `gap_stats.*` incl. class histogram; classification constants reused verbatim |
| `expected_frames`, `total_frames`, `frames`, `drift_frames` | `drift.*` (drift = delivered − expected, computed identically to the harness) |
| `silent`, `peak`, `rms`, meta `silence_threshold` | telemetry only (level events, `SILENCE_PEAK_THRESHOLD`); **not** chunk manifest fields — silence is legitimate audio, not an error condition (plan Fehlerverhalten "Keine Systemlautstärke") |
| `error` column | feeds `state: damaged` decisions and stream-error events, not manifest fields themselves |

Gap/drift classes measured in Phase 1 (histogram buckets, drift trend in
frames/hour per `analyze-spike-csv.ps1`) therefore map 1:1 onto manifest fields;
no new classification scheme is introduced at Phase 2 (Phase‑1 acceptance:
"gemessene Gap-/Drift-Werte und Korrekturgrenzen sind dokumentiert").

## 5. Atomic finalization rules

(Testmatrix: "Chunk-Rollover und atomare Finalisierung"; plan §7: "Jeder Chunk
wird zunächst als temporäre Datei geschrieben, sein Header beim Abschluss
finalisiert und anschließend atomar umbenannt.")

### 5.1 Finalization order (normative)

```text
patch WAV header sizes → fsync file → rename <seq>-<tag>.wav.part → <seq>-<tag>.wav
→ fsync containing directory → compute checksum → persist state transition
(DB tx + manifest append)
```

The **rename is the single atomic commit point**. Before it: crash leaves a
`.part` recoverable per §7. After it: the chunk is valid forever, even if the DB/
manifest step never ran (§7 reverse reconciliation). Only after the rename may
`finalized:true` / `state:ready` be recorded anywhere.

### 5.2 Rollover

- Target duration 60 s of **timeline samples**, not wall time (plan §7 initial
  constant; gaps must not produce junk short chunks). Revisitable per plan after
  measurement.
- Rollover finalizes the current chunk then opens the next sequence number as a
  fresh `.part`. Fully per source and asynchronous between sources.
- If opening the next `.part` fails (e.g. disk full), the current chunk is
  finalized **first**, then the controlled-stop path runs — no finalized data is
  sacrificed (plan Fehlerverhalten "Datenträger fast voll": "Aufnahme kontrolliert
  stoppen und vorhandene Daten erhalten").
- Pause also finalizes both open chunks (durability checkpoint, plan §6.x
  "Pause"); stop finalizes all open chunks before the UI may report
  `Aufnahme beendet` (plan §6.9: "Erst nach sicherer Chunk-Finalisierung").

### 5.3 Manifest update rule

`manifest.json` is replaced atomically (write `manifest.json.tmp` → fsync →
rename → fsync dir) after each state transition. If the app crashes mid-update
the tmp file is discarded; §7 reconciliation repairs the manifest from
filesystem + DB. The manifest is a recovery/index artifact — it never becomes a
second writer authority over chunk files.

## 6. Consumption contract for the Phase-5 STT queue

(plan §8 "Gemeinsames Segmentmodell", §13 Phase 5; stt-provider-contract.md §2.)

1. **Readiness.** The queue consumes only chunks with `state:"ready"` /
   `finalized:true`. Readiness is learned from `RecorderEvent::ChunkFinalized`
   or the store — never by watching the filesystem (mirrors recorder-contract.md
   §3.1 rule 4). `.part` and `damaged` chunks are invisible to jobs.
2. **Job references.** Every STT job pins its inputs as `audio_chunk_ids`
   (plan §8) resolved to `chunk_uuid`s; the evidence chain text→audio survives
   retention decisions (plan §12: failed jobs prevent audio deletion).
3. **Intra-chunk addressing.** VAD segment boundaries inside a chunk convert to
   timeline positions by
   `timeline_ms = chunk.timeline_start_ms + sample_index / 16000 × 1000`.
   Because gap-filled silence occupies real timeline space (§3 rule 4), this
   linear map is exact everywhere in the chunk — no per-gap correction tables
   needed downstream.
4. **Segment model.** Segments carry exactly the plan §8 fields
   (`meeting_id, source, timeline_start_ms, timeline_end_ms, audio_chunk_ids,
   language, provider, provider_settings_snapshot, attempt, state`);
   `source` inherits the chunk's `microphone|system` value.
5. **Chronological merge.** The transcript view is derived by sorting persisted
   segments by `timeline_start_ms` (ties: `timeline_end_ms`, then insertion
   `sequence`). Mic and system segments interleave freely; equal-time overlaps
   are rendered as parallel tracks, never merged into one text run. Duplicate
   words at VAD segment boundaries are removed during merge per plan §8
   ("Doppelte Wörter an Segmentgrenzen werden beim Merge entfernt"); retry-
   induced duplicates are excluded by job idempotency
   (stt-provider-contract.md §3.3), not by text heuristics.
6. **Bounded units.** Local jobs take one-to-few ready chunks (VAD segments
   5–30 s starting point, plan §8); Azure packages are built **from finalized
   chunks only** as temporary stereo (left=mic, right=system) well under the
   5 h/500 MB Fast Transcription limits (plan §7/§8). Package building reads
   immutable `ready` files, so it can never race a live writer.
7. **Recovery transparency.** Recovered chunks (`recovered:true`) enter the
   queue like any `ready` chunk; their truncated tail simply appears as timeline
   silence followed by the next chunk. Consumers need no special case beyond
   ordinary gap handling (Testmatrix: "Recovery beschädigter/offener Chunks").

## 7. Crash-recovery rules for the data contract

(Full protocol: recorder-contract.md §8; restated as the data-visible rules.)

1. At startup, any session whose persisted state claims `starting/recording/
   paused` without a live owner runs recovery. A session is never left in
   `recording` (plan §6.x Session-Zustände).
2. Each `writing` chunk resolves to: parseable `.part` → header patched to whole
   frames, renamed, `ready` with `recovered:true`, `timeline_end_ms` recomputed
   from actual `sample_count`; unparseable/missing → `damaged` (+ quarantine
   into `recovery/`).
3. Reverse reconciliation adopts orphan `.wav` files (rename succeeded, record
   didn't) as `ready, recovered:true`; stray `.part` without a record is
   quarantined. Filesystem is authoritative for what survived; DB+manifest for
   what was attempted.
4. Recovery is idempotent and per source: repairing the microphone's open chunk
   never touches system chunks (see §8).

## 8. Source-failure isolation (gate G3)

Plan Phase‑2 acceptance: **"Ausfall einer Quelle zerstört die andere nicht."**
The contract enforces this structurally:

1. **Independent persistence units.** Each source has its own sequence counter,
   its own open `.part` file handle, its own writer path and its own rollover
   timer. No lock, buffer, or file is shared between sources on the write path;
   a panic or hang in one source's writer cannot hold the other's I/O.
2. **Independent finalization.** `close_source(source)` finalizes only that
   source's open chunk (recorder-contract.md §4). Stop/pause finalize each
   source independently; a failure finalizing source A must not prevent
   attempting source B's finalization, and B's result is recorded regardless.
3. **No cross-source coupling in data.** Timeline stamps derive from the shared
   origin (§3), but gap stats, drift stats, segments, and states are strictly
   per source fields (§4). Silence-fill inserted for mic gaps never enters the
   system track and vice versa.
4. **Failure containment examples.**
   - Mic device lost → mic chunk finalized (or `damaged` if unrepairable),
     system chunks continue unaffected; system `sequence` numbering continues
     undisturbed.
   - System device lost → identical isolation mirrored; the session may continue
     mic-only (`has_system_audio=false` from that point, plan Fehlerverhalten).
   - One source's disk-full on rollover → controlled stop still preserves the
     *other* source's already-finalized chunks (§5.2).
   - Crash during either source's finalization → at most that one chunk is
     affected (§5.1, §7); all previously renamed chunks of both sources remain
     playable and transcribable (plan §7: "Ein Prozessabsturz kann damit höchstens
     den aktuell offenen Chunk beschädigen, nicht die gesamte Aufnahme").
5. **Downstream isolation.** Azure stereo packaging (§6 rule 6) is built from
   already-immutable `ready` chunks; a later failure of one source can therefore
   never invalidate packages or jobs of the other source.

## 9. Non-goals (complexity budget)

Explicitly out of scope for this contract, per plan §2 "Bewusst nicht Bestandteil":

- speaker/diarization metadata in chunks or manifests (only `microphone|system`);
- per-process loopback attribution, app names, or participant identifiers;
- live/realtime transcript fields in the chunk format;
- encryption-at-rest or compression schemes inside the chunk format (retention
  and deletion operate on plain files per plan §11/§12);
- cross-session chunk merging or a global chunk registry beyond the per-meeting
  directory.

## Appendix A — Gate traceability

- **G1** (document defines naming, manifest fields w/ type+semantics,
  finalization/recovery, STT consumption): §1, §2, §4, §5, §7, §6 respectively.
- **G2** (each element cites its plan anchor): "Trace" column in §4, plus inline
  citations in §1–§3, §5–§8; consolidated anchors are plan §7, §8, §9, §13
  Phase 2/5 and the three named Testmatrix rows of §14.
- **G3** (source-failure isolation): §8, grounded in plan Phase‑2 acceptance.
- **G4** (reviewed against actual spike CSV column set): §4 mapping table based
  on the verified identical header line 6 of all four
  `docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv` files and the
  class boundaries implemented in `analyze-spike-csv.ps1`; terminology kept
  aligned with recorder-contract.md §5.1 (`GAP_CLASS_NORMAL_MAX_MS = 250.0`,
  `GAP_CLASS_SPARSE_MAX_MS = 1000.0`).
