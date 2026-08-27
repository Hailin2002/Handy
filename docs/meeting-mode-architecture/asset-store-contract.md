# AssetStore Interface, Storage-Key & Meeting-Layout Contract

Status: **contract draft — no implementation yet** (Phase 3 prerequisite)
Source of truth: `docs/meeting-mode-plan.md`, section 5 ("Zielarchitektur" →
`AssetStore`), 7 ("Persistenzformat"), 9 ("Datenmodell": `assets`,
`meeting_audio_chunks`, storage-key rule), 11 ("Meeting-Bibliothek und Export":
three-level delete) and 12 ("Speicher- und Datenschutzlogik": retention).
Planned implementation home: `src-tauri/src/storage/asset_store.rs`.
This document defines the interface contract only; it changes no source code.

Sibling contracts: `recorder-contract.md` (chunk files, `.part` → `.wav`
finalize, recovery) and `stt-provider-contract.md`. Where this document
references chunk states, checksums or the `.part` suffix, it adopts the
recorder contract's definitions verbatim.

Reuse obligations (plan §5, §4): all path resolution goes through the existing
portable-aware helpers in [`portable.rs`](../../src-tauri/src/portable.rs) —
`portable::app_data_dir(app)` (line 69), `portable::resolve_app_data(app,
relative)` (line 88) and, for non-Tauri contexts, `portable::data_dir()`
(line 64). The store must never call `app.path().app_data_dir()` directly, so
portable installs (`Data/` next to the exe) and normal installs share one code
path (plan §4: "Projekt- und Assetspeicherung muss dieselbe portable-aware
Pfadlogik verwenden").

---

## 1. Scope and module boundaries

`AssetStore` owns exactly one concern: **durable, content-addressed storage of
binary blobs (meeting audio chunks, exports, imports, media) under relative
storage keys, with hash-verified integrity and explicit, auditable deletion.**
It does _not_ own:

| Concern                                          | Owner                                                    | Interaction                                                                                                      |
| ------------------------------------------------ | -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| SQL rows for chunks/jobs/segments/summaries      | `MeetingStore`                                           | AssetStore is told _when_ to commit/delete; it never opens the DB itself                                         |
| Session state machine, Tauri commands/events     | `MeetingManager`                                         | calls put/delete/verify, receives typed reports                                                                  |
| Chunk file writing during live capture           | `MeetingRecorder`                                        | is the single writer of `audio/` while a session is active; AssetStore only finalizes/verifies/deletes on demand |
| Retention _policy decision_ (which rule applies) | settings + `meeting_sessions.recording_retention_policy` | AssetStore executes the decision through the §8 sweep                                                            |
| STT job state (what counts as "unconfirmed")     | `MeetingStore` via the `RetentionGuard` trait (§8)       | sweep consults the guard before every automatic audio deletion                                                   |
| Secrets, provider payloads, transcripts (text)   | elsewhere                                                | never pass through the AssetStore                                                                                |

Frontend code never touches SQLite or asset files directly (plan §3 Phase-3
acceptance: "kein Frontendcode greift direkt auf SQLite oder Assetdateien zu").
Everything reaches the store through domain services behind Tauri commands.

---

## 2. On-disk root, resolution, and the no-absolute-path rule (G2)

### 2.1 Root resolution

The store root is the app data directory joined with nothing else:

```text
<app-data-dir>/meetings/<meeting-id>/…   (meeting sessions, §6)
<app-data-dir>/assets/<asset-id>/…       (generic assets, §4)
```

`<app-data-dir>` is produced exclusively by:

```text
// src-tauri/src/portable.rs (existing code, cited not changed)
pub fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, tauri::Error>   // line 69
pub fn resolve_app_data(app: &tauri::AppHandle, relative: &str)
    -> Result<PathBuf, tauri::Error>                                           // line 88
pub fn data_dir() -> Option<&'static PathBuf>  // line 64, non-Tauri contexts (tests, CLI)
```

Normative rules:

1. **Every absolute path in the process is derived, never stored.** SQLite
   rows, `manifest.json`, events, logs and exports persist only asset IDs and
   relative storage keys (plan §9: "keine absoluten Dateipfade als dauerhafte
   Identität; Datenbank speichert Asset-ID und relativen Storage-Key").
2. **Single resolution point.** The only function in the codebase that turns a
   storage key into an absolute path is
   `AssetStore::resolve(&self, key: &StorageKey) -> Result<PathBuf, AssetStoreError>`.
   In Tauri contexts it is implemented as
   `portable::resolve_app_data(self.app, key.as_str())`; headless contexts
   (tests, CLI) use `portable::data_dir().join(key)`; the §5.1 production
   constructor obtains its root once via `portable::app_data_dir(app)?` so the
   injected-root variant is the same resolution with the portable decision
   already applied. Code review gate:
   any other `join` of a stored string onto a directory is a defect.
3. **Re-location safety.** Because nothing absolute is persisted, the data
   directory can move (portable stick, restored backup, future sync target)
   and every key remains valid. A one-time migration that _does_ know both
   roots lives outside the store and rewrites nothing but the root.

### 2.2 Storage-key grammar

A storage key is a **relative, forward-slash path** with this grammar:

```text
storage-key  := meeting-key | asset-key
meeting-key  := "meetings/" meeting-id "/" meeting-rel
meeting-rel  := "manifest.json"
              | "audio/" chunk-file
              | "export/" export-file
              | "recovery/" recovery-file
chunk-file   := seq "-" source (".wav" | ".wav.part")        ; seq = 6*DIGIT
source       := "microphone" | "system"
export-file  := export-name "." ("md" | "txt" | "json" | "wav")
recovery-file:= quarantined-name                             ; never re-ingested silently
asset-key    := "assets/" asset-id "/" asset-file
asset-file   := "original" ( "." ext )?                      ; ext = 1*8(ALPHA / DIGIT)
```

Validation (function `StorageKey::parse`, normative):

- components match `[A-Za-z0-9._-]+`; the fixed components
  `meetings`, `assets`, `audio`, `export`, `recovery`, `manifest.json`,
  `microphone`, `system` are matched exactly (lowercase);
- rejects: empty key, leading `/`, drive letters (`C:`), backslashes,
  `.` or `..` components, trailing `/`, any component containing `\0`,
  keys outside the two grammars above;
- maximum length 512 characters (UTF-8 bytes);
- `meeting-id` and `asset-id` components must be lower-case UUID hyphenated
  form (`[0-9a-f]{8}-[0-9a-f]{4}-…`), so user-controlled filenames can never
  escape their directory.

`export-name` and `quarantined-name` are generated by the store (timestamp +
slug + counter), never taken verbatim from user or import filenames; imported
original names are metadata in `assets.original_filename`, never key parts.
This keeps the plan-§12 untrusted-input rules (path traversal, absolute paths,
symlinks rejected before commit) enforceable at one choke point:
`AssetStore::put` refuses any `PutRequest` whose proposed key fails
`StorageKey::parse`, and `commit_import` refuses symlinks/reparse points
encountered inside the staging directory.

---

## 3. Asset identity and content hashing

### 3.1 Asset-ID scheme

```rust
/// Stable asset identity. UUID v4, hyphenated lower-case, prefixed for
/// grep-ability in logs and support tickets. Not derived from file content
/// (the same bytes imported twice are two assets with two histories), not
/// derived from SQLite autoincrement (plan §9).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetId(String); // "asst_<uuid-v4>"
```

- Generated by the caller (domain service) so a DB row and its files can be
  committed in either order without the store inventing identity.
- Meeting audio chunks are _not_ `AssetId`s: they are identified by
  `meeting_id + source + sequence` and live inside the session directory
  (§6). The generic `assets` table (plan §9) covers imports, exports and
  media; chunk rows stay in `meeting_audio_chunks` with their relative
  `storage_key`.

### 3.2 Content-hash algorithm

```rust
/// Lower-case `sha256-<64 hex>` over the exact file byte range on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentHash(String);

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub const HASH_ALGORITHM: &str = "sha256"; // single algorithm for v1; the
                                           // string is persisted so a later
                                           // algorithm is a migration, not a
                                           // silent change
```

SHA-256 is chosen because it is already the recorder contract's chunk checksum
(`checksum_sha256`), ships in RustCrypto `sha2` (already in the dependency
tree via Tauri's stack — no new dependency), and is verified at these points:

| #   | Verification point                                                                                          | What is compared                                                                           | Cost control                                        |
| --- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| V1  | `put` commit (§5.2 step 3)                                                                                  | hash accumulated during the streamed write vs. caller-supplied expected hash (if provided) | free — computed while writing                       |
| V2  | `verify(key, expected)` — explicit                                                                          | fresh full read vs. stored/expected hash                                                   | on demand only                                      |
| V3  | chunk finalize hand-off (recorder → store, per recorder contract §3.1)                                      | recorder-computed checksum recorded in DB + manifest                                       | free at finalize                                    |
| V4  | STT job pickup (`MeetingTranscriptionQueue`) — before a chunk's bytes are uploaded or fed to a local engine | `verify` of the chunk key vs. `meeting_audio_chunks.checksum_sha256`                       | ~7 MB per 60 s chunk, negligible                    |
| V5  | startup recovery scan (§7) — cheap tier                                                                     | file existence + size vs. manifest/DB                                                      | no audio parsing, no hashing                        |
| V6  | retention sweep before unlink (§8)                                                                          | existence + size only                                                                      | full hash only when `verify_on_delete` is requested |

A mismatch at V2/V4 marks the row `damaged` (chunk) or `corrupted` (asset) and
surfaces a named error; it never silently re-downloads, re-transcribes or
deletes.

---

## 4. Canonical on-disk meeting layout

Per plan §7 "Persistenzformat", one directory per session under the store root:

```text
<app-data-dir>/
  meetings/
    <meeting-id>/                  ; UUID v4, hyphenated lower-case
      manifest.json                ; versioned session manifest (§6)
      audio/
        000001-microphone.wav      ; finalized PCM16 16 kHz mono chunks
        000001-system.wav
        000004-microphone.wav.part ; exactly one open .part per live source
      export/
        2026-08-22_weekly-sync.md        ; user exports, regenerable
        stereo-mixdown.wav               ; optional audio export
      recovery/
        000002-system.wav          ; quarantined damaged/orphan files
  assets/
    <asset-id>/
      original.pdf                 ; committed import/export/media blobs
```

Directory invariants:

1. `manifest.json` exists from session creation onward and is updated
   atomically (§6.3); its absence for an existing `meetings/<id>/` directory
   marks the session as _unfinalized crash residue_ for the §7 scan.
2. `audio/` is written **only** by the `MeetingRecorder` while the session is
   live (recorder contract §1). The AssetStore touches `audio/` only for
   verify, selective delete, retention unlink and recovery quarantine.
3. `export/` is fully regenerable from transcript + audio; deleting it never
   destroys user data, and it is therefore included in every delete level
   that removes audio or the whole meeting (§9).
4. `recovery/` is a quarantine, not a workspace: files land there only from
   the recovery protocols and are removed only by explicit user confirmation
   or the whole-meeting delete.
5. The whole `meetings/<meeting-id>/` directory is created with
   `create_dir_all` at session start and removed only by delete level 3 (§9).

---

## 5. AssetStore interface (G1)

### 5.1 Types and trait sketch

```rust
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// Generic committed blob (import, export, media). Key: assets/<id>/…
    Blob,
    /// Meeting-audio chunk. Key: meetings/<mid>/audio/<file>
    MeetingAudio,
    /// Regenerable export artifact. Key: meetings/<mid>/export/<file>
    MeetingExport,
    /// Quarantined recovery file. Key: meetings/<mid>/recovery/<file>
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageKey(String);

impl StorageKey {
    /// Parses and validates against the §2.2 grammar. `Err` on any
    /// absolute, traversing or out-of-grammar key.
    pub fn parse(raw: &str) -> Result<StorageKey, AssetStoreError> {
        let _ = raw;
        unimplemented!("sketch only")
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StorageKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Not `Clone`: the `Reader` variant of `PutSource` is single-consumption.
#[derive(Debug)]
pub struct PutRequest {
    /// Pre-generated by the caller for `Blob`; ignored (derived) for
    /// meeting-scoped kinds whose key is fully determined by the grammar.
    pub key: StorageKey,
    pub kind: AssetKind,
    pub expected_hash: Option<ContentHash>,
    /// Streamed content source; the store never requires the whole blob
    /// in RAM (plan §7: two-track audio is ~230 MB/h).
    pub content: PutSource,
}

/// Not `Clone`: a reader variant can be consumed exactly once.
pub enum PutSource {
    File(PathBuf),
    /// Reader variant for in-memory producers (e.g. Azure stereo package).
    Reader(Box<dyn std::io::Read + Send>),
}

impl std::fmt::Debug for PutSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutSource::File(p) => write!(f, "PutSource::File({p:?})"),
            PutSource::Reader(_) => write!(f, "PutSource::Reader(Box<dyn Read>)"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PutResult {
    pub key: StorageKey,
    pub hash: ContentHash,
    pub size_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadVerification {
    /// Stat-only open; caller hashes later via `verify`.
    None,
    /// Hash the whole content on open (V2/V4 path).
    Full,
}

#[derive(Clone, Debug)]
pub struct VerifyReport {
    pub key: StorageKey,
    pub expected: ContentHash,
    pub actual: Option<ContentHash>, // None = file missing
    pub size_bytes: Option<u64>,
    pub ok: bool,
}

#[derive(Clone, Debug)]
pub struct DeleteReport {
    pub removed_keys: Vec<StorageKey>,
    pub removed_bytes: u64,
    /// Keys that were already absent (row said present, file gone).
    pub already_missing: Vec<StorageKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetStoreError {
    InvalidKey(String),
    /// Import staging contained a symlink/reparse point or escaped path.
    UntrustedPathRejected { key: StorageKey, detail: String },
    HashMismatch { key: StorageKey, expected: ContentHash, actual: Option<ContentHash> },
    NotFound { key: StorageKey },
    Io { key: StorageKey, detail: String },
    /// Parent directory fsync or rename failed after the temp write.
    CommitFailed { key: StorageKey, temp_path: String, detail: String },
}

impl std::fmt::Display for AssetStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetStoreError::InvalidKey(k) => write!(f, "invalid storage key: {k}"),
            AssetStoreError::UntrustedPathRejected { key, detail } => {
                write!(f, "untrusted path rejected for {key}: {detail}")
            }
            AssetStoreError::HashMismatch { key, expected, actual } => match actual {
                Some(a) => write!(f, "hash mismatch for {key}: expected {expected}, got {a}"),
                None => write!(f, "hash mismatch for {key}: expected {expected}, file unreadable"),
            },
            AssetStoreError::NotFound { key } => write!(f, "asset not found: {key}"),
            AssetStoreError::Io { key, detail } => write!(f, "io error for {key}: {detail}"),
            AssetStoreError::CommitFailed { key, temp_path, detail } => {
                write!(f, "commit failed for {key} (temp {temp_path}): {detail}")
            }
        }
    }
}

impl std::error::Error for AssetStoreError {}

/// Persistence seam for v1: the store is constructed with the resolved root
/// so tests can point it at a temp dir; production constructs it with
/// `portable::app_data_dir(app)?`.
pub struct LocalAssetStore {
    root: PathBuf,
}

pub trait AssetStore: Send + Sync {
    /// Resolve a key to its absolute path. The single resolution point (§2.1).
    fn resolve(&self, key: &StorageKey) -> Result<PathBuf, AssetStoreError>;

    /// Atomic write-commit (§5.2). Creates parent directories as needed.
    fn put(&self, req: PutRequest) -> Result<PutResult, AssetStoreError>;

    /// Open a bounded reader. `ReadVerification::Full` hashes while reading
    /// and returns `AssetStoreError::HashMismatch` on first read error.
    fn get(
        &self,
        key: &StorageKey,
        verification: ReadVerification,
    ) -> Result<Box<dyn std::io::Read + Send>, AssetStoreError>;

    fn exists(&self, key: &StorageKey) -> Result<bool, AssetStoreError>;
    fn size(&self, key: &StorageKey) -> Result<u64, AssetStoreError>;

    /// V2 verification: full read + hash compare.
    fn verify(&self, key: &StorageKey, expected: &ContentHash)
        -> Result<VerifyReport, AssetStoreError>;

    /// Unlink a single key. Content-first deletion: the caller commits the
    /// DB/manifest state change *after* this returns Ok (§9.2).
    fn delete(&self, key: &StorageKey) -> Result<(), AssetStoreError>;

    /// Remove an entire session directory tree (delete level 3 only, §9).
    /// Refuses keys that are not `meetings/<id>` roots.
    fn delete_meeting_tree(&self, meeting_id: &str) -> Result<DeleteReport, AssetStoreError>;
}
```

### 5.2 Atomic write-commit protocol (normative order)

Every `put`, and every manifest update (§6.3), follows the same six steps.
The rename is the single atomic commit point:

```text
1. resolve    key -> absolute path via portable::resolve_app_data
              (rejects invalid keys before any filesystem call)
2. stage      write bytes to <key>.part in the SAME directory
              (same volume ⇒ rename is atomic; hash accumulated
              while streaming; create parent dirs first)
3. seal       flush + fsync the temp file
              (compare accumulated hash against expected_hash when given)
4. commit     rename <key>.part -> <key>            ← atomic commit point
5. durabilize fsync the parent directory
6. report     return PutResult { hash, size } — the CALLER now commits
              the DB row / manifest entry that references the key
```

Crash windows and their outcomes:

| Crash between | Result on disk                                 | Recovery behavior                                                                                                          |
| ------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| 2–3           | `.part` remains, no final key                  | orphan `.part` — §7 scan quarantines/removes it                                                                            |
| 3–4           | sealed `.part`, no final key                   | same as above                                                                                                              |
| 4–5           | final key present, dir entry maybe not durable | after reboot the file either exists (fine) or the pre-rename state holds; §7 reconciles against DB/manifest                |
| 5–6           | file fully durable, metadata row missing       | §7 "reverse reconciliation" (recorder contract §8 step 3 pattern): file exists, row missing → adopt or quarantine per kind |

On step-5 failure (`CommitFailed`) the temp path is reported so the caller can
retry or clean up; the store never deletes the temp file on failure paths
other than explicit retry, because forensics beats tidiness for audio.

Windows note: `std::fs::rename` on NTFS is atomic for the directory-entry
swap used here; the `.part` suffix doubles as a crash marker and prevents
consumers from ever observing a half-written final name.

---

## 6. `manifest.json` schema (G3)

### 6.1 Purpose and recovery guarantee

The manifest is the **crash-scan index** of a session. Hard requirement: the
§7 startup scan must be able to identify every unfinalized chunk (state
`writing`, missing file, size mismatch) **without reading or parsing any
audio bytes** — manifest + `stat` only. Audio parsing happens later, and only
on chunks the scan flagged, inside the recorder's recovery protocol.

### 6.2 Schema (versioned)

```json
{
  "schema": "handy.meeting-manifest",
  "manifest_version": 1,
  "meeting_id": "0b7f8a52-1c4d-4e6a-9f20-7c1b2d3e4f55",
  "created_at": "2026-08-22T09:12:04.312Z",
  "updated_at": "2026-08-22T10:47:55.001Z",
  "timeline": {
    "sample_rate_hz": 16000,
    "channels": 1,
    "origin_wall_clock": "2026-08-22T09:12:04.301Z",
    "started_at_ms": 0.0,
    "ended_at_ms": 5731000.0,
    "paused_total_ms": 124000.0,
    "markers": [
      {
        "kind": "pause",
        "timeline_ms": 1841000.0,
        "wall_clock": "2026-08-22T09:42:44Z"
      },
      {
        "kind": "resume",
        "timeline_ms": 1841000.0,
        "wall_clock": "2026-08-22T09:44:10Z"
      }
    ]
  },
  "session_snapshot": {
    "title": "Weekly Sync",
    "state": "stopped_unprocessed",
    "stt_provider": "local:whisper-large-v3",
    "language": "de",
    "summary_provider": "openrouter",
    "recording_retention_policy": null,
    "has_microphone_audio": true,
    "has_system_audio": true,
    "microphone_device": "USB Microphone",
    "system_device": "Speakers (Realtek)"
  },
  "source_segments": [
    {
      "segment_id": 1,
      "source": "microphone",
      "device_name": "USB Microphone",
      "from_ms": 0.0,
      "to_ms": 5731000.0
    }
  ],
  "chunks": [
    {
      "chunk_id": 7,
      "source": "microphone",
      "sequence": 3,
      "state": "ready",
      "storage_key": "audio/000003-microphone.wav",
      "timeline_start_ms": 120000.0,
      "timeline_end_ms": 180000.0,
      "sample_count": 960000,
      "file_size_bytes": 1922044,
      "checksum_sha256": "sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "recovered": false,
      "source_segment_id": 1,
      "gap_stats": { "count": 1, "filled_samples": 4800 },
      "drift": {
        "max_abs_drift_frames": 42,
        "applied_ppm_min": -3.0,
        "applied_ppm_max": 2.0
      }
    },
    {
      "chunk_id": 8,
      "source": "system",
      "sequence": 4,
      "state": "writing",
      "storage_key": "audio/000004-system.wav.part",
      "timeline_start_ms": 180000.0,
      "timeline_end_ms": null,
      "sample_count": null,
      "file_size_bytes": null,
      "checksum_sha256": null,
      "recovered": false,
      "source_segment_id": 2,
      "gap_stats": { "count": 0, "filled_samples": 0 },
      "drift": {
        "max_abs_drift_frames": 0,
        "applied_ppm_min": 0.0,
        "applied_ppm_max": 0.0
      }
    }
  ],
  "integrity": {
    "self_hash_sha256": "sha256-…",
    "chunk_count": 8,
    "ready_chunk_bytes": 443812048
  }
}
```

Field rules:

- **Key scoping inside the manifest:** chunk `storage_key` values are
  relative to the session directory (`audio/000003-microphone.wav`), exactly
  as in the recorder contract's `ChunkDescriptor.storage_key`. The canonical
  app-data-relative `StorageKey` (§2.2) is the composition
  `meetings/<meeting-id>/` + manifest key; only `AssetStore::resolve`
  performs that composition, so no consumer ever concatenates paths itself.
- `manifest_version` is a positive integer; a reader must refuse versions it
  does not know (forward-compat gate) and a writer only ever emits `1` in v1.
- `chunks[]` mirrors `meeting_audio_chunks` one-to-one (plan §9): same states
  `writing | ready | damaged | deleted`, same checksums, plus the gap/drift
  metadata the plan requires for the chunk table ("Gerätekennung und
  Gap-/Drift-Metadaten").
- A `writing` row carries the `.part` key and `null` sizes/checksum — this is
  exactly the information the §7 scan needs to classify it unfinalized.
- `session_snapshot` duplicates the non-secret columns of
  `meeting_sessions` (never secrets — plan §9: no secrets in manifests) so
  the recovery banner and post-crash flow can render without the DB; the DB
  remains authoritative where both exist.
- `integrity.self_hash_sha256` is the SHA-256 of the manifest bytes with the
  `integrity.self_hash_sha256` member set to the empty string; it detects a
  truncated or externally edited manifest even though the §5.2 protocol makes
  that unlikely.
- `recording_retention_policy: null` means "user has not answered yet" — the
  §8 sweep treats it as _never delete automatically_.

### 6.3 Manifest write protocol

The manifest is updated with the §5.2 protocol itself: write
`manifest.json.part` → fsync → rename → fsync dir. Update points (all after
the corresponding DB transaction succeeded, so the manifest never runs ahead
of SQLite):

1. session created (initial manifest with empty `chunks`);
2. chunk opened (`writing` row inserted);
3. chunk finalized (`ready` + checksum);
4. chunk marked `damaged`/`deleted`, recovery quarantine moves;
5. pause/resume markers;
6. stop / recovery completion (`ended_at_ms`, session state).

A crash between DB commit and manifest update is benign: §7 treats
DB ∖ manifest differences as "manifest stale" and rewrites it, and
manifest ∖ DB differences as "crash residue" to reconcile.

---

## 7. Startup recovery scan (manifest-based, no audio reads)

Runs once at app startup (owned by `MeetingManager`, executed through the
AssetStore + MeetingStore), for every `meetings/<id>/` directory:

1. **Load manifest** (§6.3 protocol guarantees it is never torn). Missing
   manifest → the directory is crash residue: enumerate `audio/*.wav` files,
   adopt them into a fresh manifest as `damaged` (pending user decision via
   the `MeetingRecoveryBanner`), quarantine `.part` files into `recovery/`.
2. **Cheap-tier check per chunk row (V5)** — `stat` only:
   - `writing` row → unfinalized: hand to the recorder's
     `recover_open_chunks` protocol (recorder contract §8), which does the
     audio-aware repair; the scan itself did not read a single audio byte.
   - `ready` row with missing file or size mismatch → mark `damaged` in DB +
     manifest, surface named error.
   - `ready` row matching size → leave untouched; full hash verification
     happens lazily at V4 (job pickup), not at startup (a 10 h meeting would
     otherwise hash ~2.3 GB before the window opens).
3. **Reverse reconciliation:** `audio/*.wav` files with no `ready` chunk row →
   recorder contract §8 step 3 (adopt with `recovered=true` or quarantine).
4. **Rewrite manifest** if anything changed (§6.3 update points 2–4).

The scan never deletes audio content automatically. Orphan `.part` temp files
are moved into `recovery/` like every other quarantine case; nothing under a
session directory is removed except by the §9 delete levels (or the
whole-meeting tree delete).

---

## 8. Retention-policy hooks (G4)

### 8.1 Policy values

Per plan §12, stored as `meeting_sessions.recording_retention_policy`
(NULL until the user answers the first-meeting question):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionPolicy {
    /// "sofort löschen" — audio removed right after confirmed processing.
    DeleteImmediately,
    /// "7 Tage"
    After7Days,
    /// "30 Tage"
    After30Days,
    /// "dauerhaft behalten" — never swept.
    KeepPermanent,
}
```

(`NULL` in the DB maps to a fourth in-memory state `Unanswered`, which is
never persisted: plan §12 — "ohne beantwortete Auswahl wird nichts
automatisch gelöscht".)

Normative defaults from plan §12: **nothing is deleted automatically while
the policy is unanswered**, and audio survives through recording,
transcription and summary regardless of policy — the sweep clock starts at
the _confirmed completion of processing_, never at recording start.

### 8.2 Enforcement point and guard

The **only** code path that deletes meeting audio without an explicit user
action in the same UI session is `AssetStore`-adjacent sweep entry:

```rust
/// Answers: "may automatic retention delete this meeting's audio now?"
/// Implemented by MeetingStore. One query, transactional read.
pub trait RetentionGuard: Send + Sync {
    fn audio_deletion_allowed(
        &self,
        meeting_id: &str,
    ) -> Result<RetentionDecision, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionDecision {
    Allowed,
    /// Named reason; the sweep logs it and leaves the audio untouched.
    Blocked(RetentionBlockReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionBlockReason {
    /// Any meeting_stt_jobs row in state queued | running | retry_wait.
    UnconfirmedSttJob,
    /// Any meeting_stt_jobs row in state failed — plan §12 verbatim:
    /// "fehlgeschlagene … STT-Jobs verhindern automatische Audiolöschung".
    FailedSttJob,
    /// Summary/processing still pending for the session.
    ProcessingIncomplete,
}
```

The sweep (executed by a `RetentionSweeper` in the storage layer, invoked at
app start and on a daily timer):

```text
for each session where retention policy is due (now >= completed_at + delay):
    decision = guard.audio_deletion_allowed(meeting_id)?
    match decision:
        Allowed            -> delete level 1 (audio only, §9) via AssetStore,
                              write change_log row, emit event
        Blocked(reason)    -> skip, log reason, retry next sweep
```

Rules:

1. **Enforcement point (named):** the check happens _inside the sweep loop,
   per meeting, immediately before the first unlink_ — not in the UI, not in
   MeetingManager — so no future caller can bypass it without consciously
   reimplementing it. `RetentionSweeper` takes `&dyn RetentionGuard` in its
   constructor; there is no default "allow" implementation.
2. `succeeded` and user-`cancelled` jobs do not block; `failed` and all
   not-yet-terminal states do (plan §12 sentence quoted above).
3. The guard also blocks when `meeting_summaries`/processing state for the
   session is still incomplete, because "vollständig erfolgreiche
   Verarbeitung" is the sweep clock's precondition (plan §12).
4. **User-initiated deletes are never blocked by the guard** — they are
   explicit, confirmed actions (plan §11). But the confirmation dialog must
   _display_ existing unconfirmed/failed STT jobs as a warning before the user
   confirms (plan §11: irreversible deletes show their concrete components).
5. Every automatic deletion writes a `change_log` row (plan §9) with meeting
   id, policy, freed byte count and the guard decision, so "warum ist mein
   Audio weg?" is always answerable.

---

## 9. Selective deletion — mapping the plan-§11 delete levels (G5)

Plan §11: "Beim Löschen unterscheidet die UI klar zwischen: nur Rohaufnahme
löschen; Transkript und Zusammenfassung behalten; gesamtes Meeting endgültig
löschen." The contract implements this as **three monotone delete levels**
(bullets 1+2 form level 1's contract: audio goes, transcript and summary are
kept; the middle level is the minimal consistent reading that yields three
levels — flagged in Appendix A for architecture review).

### 9.1 Level → files and rows mapping

|                                           | **Level 1 — „Nur Rohaufnahme löschen"**                   | **Level 2 — „Rohaufnahme, Transkript und Zusammenfassung löschen"**                                                               | **Level 3 — „Gesamtes Meeting endgültig löschen"**                                                         |
| ----------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Files removed                             | all `audio/*.wav` (+ `.part`), all `export/*`             | level 1 + nothing extra on disk (transcript lives in SQLite)                                                                      | entire `meetings/<meeting-id>/` tree via `delete_meeting_tree`                                             |
| `meeting_audio_chunks`                    | rows → state `deleted`, `storage_key` kept, checksum kept | same                                                                                                                              | rows deleted                                                                                               |
| `meeting_stt_jobs`                        | kept (evidence of processing history)                     | kept                                                                                                                              | deleted                                                                                                    |
| `meeting_transcript_segments`             | **kept**                                                  | deleted                                                                                                                           | deleted                                                                                                    |
| `meeting_summaries`                       | **kept**                                                  | deleted                                                                                                                           | deleted                                                                                                    |
| `sources` / `source_items`                | **kept** (meeting stays searchable as text source)        | `source_items` deleted; `sources` row kept with `ingestion_state = audio_purged`                                                  | `sources` + `source_items` deleted                                                                         |
| `meeting_sessions`                        | **kept**, `has_microphone_audio=false`                    | kept                                                                                                                              | row deleted                                                                                                |
| `project_sources`, tasks, `task_evidence` | **kept**; evidence time-jumps degrade to text-only        | **kept**, but `task_evidence` rows referencing deleted segments are flagged "Beleg zu prüfend" (plan §11: never silently removed) | `project_sources` deleted; `task_evidence` rows referencing the source are flagged for review, not deleted |
| FTS5 index                                | unchanged                                                 | re-indexed (segments gone)                                                                                                        | re-indexed (all text gone)                                                                                 |
| `change_log`                              | append delete-level row                                   | append                                                                                                                            | append (row survives the meeting)                                                                          |

### 9.2 Ordering and crash-safety (normative)

1. **Content first, rows second** (mirrors recorder contract §3.1 property 5):
   unlink files → fsync directory → commit row/state changes in one
   transaction → append `change_log`. A crash in between leaves orphan files
   (cleaned by the §7 scan) or `deleted` rows with files already gone
   (harmless, reported as `already_missing`).
2. **Level 3 two-phase:** the confirmation dialog lists the concrete
   components (plan §11); on confirm, the tree delete runs _before_ the row
   deletions so a crash can never leave rows whose content is gone _and_ no
   files: worst case is an intact session with a missing directory, which the
   §7 scan reports as damaged rather than silently hiding.
3. **Guard interaction:** levels 1–3 are user-initiated and bypass
   `RetentionGuard` (§8.2 rule 4) but require the confirmation dialog; level 1
   triggered by the _retention sweep_ is guard-checked.
4. **Unlinking a meeting from a project** (plan §11 last paragraph) deletes
   _nothing_: it only removes/changes `project_sources` and flags affected
   `task_evidence` as "zu prüfend". It is not a delete level and must not
   touch the AssetStore.
5. Every level returns a `DeleteReport` (§5.1) that the UI renders verbatim
   in the completion notice — the user sees exactly what was removed.

---

## 10. Verification hooks (maps to plan §14 automated tests)

- Key-grammar tests: traversal (`../x`), absolute (`C:\`, `/x`), backslash,
  uppercase fixed components, overlong keys — all rejected; round-trip
  `parse(resolve-relative)` identity for every grammar production.
- Atomic-commit tests with a fault-injection filesystem seam (same pattern as
  recorder contract §9): crash at each of the six §5.2 steps leaves exactly
  the state the crash-window table promises.
- Manifest tests: torn-manifest detection via `self_hash`; `writing` chunk
  detection from manifest + `stat` only (test asserts **zero** audio bytes
  read — instrument the fs seam).
- Sweep tests: each `RetentionBlockReason` blocks; `Unanswered` policy never
  deletes; `DeleteImmediately` deletes right after confirmed processing;
  blocked sweep retries later and logs.
- Delete-level tests: the §9.1 table asserted cell by cell, including the
  "evidence flagged, not deleted" rule and the level-3 two-phase crash cases.

---

## Appendix A — Gate traceability

- **G1** (put/get/delete/verify with hash checks + atomic-commit steps):
  §5.1 trait, §3.2 verification points V1–V6, §5.2 six-step protocol and
  crash-window table.
- **G2** (key grammar, no absolute paths, portable.rs citation): §2.1 rules
  1–3 citing `portable::app_data_dir` (line 69), `resolve_app_data`
  (line 88), `data_dir` (line 64); §2.2 grammar and `StorageKey::parse`.
- **G3** (versioned manifest, startup recovery without reading audio):
  §6.2 versioned schema with `manifest_version`, per-chunk state/checksum/
  gap/drift/session snapshot; §6.1 + §7 hard requirement and stat-only scan.
- **G4** (failed/unconfirmed jobs block automatic deletion, enforcement
  point named): §8.2 `RetentionGuard`, `RetentionBlockReason::FailedSttJob`
  / `UnconfirmedSttJob`, enforcement inside the sweep loop before the first
  unlink.
- **G5** (three-level delete mapped to exact files and rows): §9.1 table,
  §9.2 ordering rules.

Open item flagged for architecture review: plan §11's second bullet
("Transkript und Zusammenfassung behalten") is implemented as the invariant
of delete level 1; the three-level reading (§9) treats the middle level as
the minimal monotone completion. If product review decides for a two-option
UI, level 2 is simply not surfaced; the storage contract is unaffected.

## Appendix B — Sketch compilability

The Rust blocks of §5.1, §8.2 (types) and §3 are extracted and compiled with
the shared helper:

```powershell
pwsh -File docs/meeting-mode-architecture/verify-sketch.ps1 -ContractPath docs/meeting-mode-architecture/asset-store-contract.md
```

Result recorded in the mission report (std-only, no external crates).
