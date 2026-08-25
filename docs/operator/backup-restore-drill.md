# Operator Drill — Backup/Restore Rehearsal (History Database + Settings)

**Audience:** operators and support staff who need to prove, on a schedule and
on demand, that a real backup of the dictation history and settings can be
restored successfully — before the day a real restore is needed.

**Scope:** the _current_ dictation data footprint (`history.db` plus
`settings_store.json`). This drill is a repeatable exercise; it does not change
any procedure. The **why** behind every rule used here — where the data lives,
why the app must be closed first, why all `history.db*` files travel together,
and what future migrations must not break — is documented in the
[Operator Guide: Dictation History Data Safety, Backup, and Migration
Safety](../meeting-mode-data-migration-operator-guide.md). That guide is the
reference for rationale; this document only adds the rehearsal steps and
verdict criteria, and points at the guide instead of restating it.

**Status:** describes the application as it exists today. Paths and behaviors
cited below were verified against this source tree and a live normal-mode
installation on Windows (see the evidence notes per step).

---

## 1. What this drill proves

A successful full pass demonstrates all of the following:

1. A backup taken with the app closed contains a database that passes SQLite's
   own integrity check.
2. That backup can be restored into a **fresh, empty profile directory**
   without touching the live profile.
3. The restored files are byte-identical to the backup set (SHA-256 round
   trip) and the restored settings file parses as valid JSON with the expected
   top-level shape.
4. (Optional full-fidelity variant, §7) The application itself opens the
   restored profile and shows the expected history entries.

What it does **not** prove: cloud provider API keys (Windows Credential
Manager / macOS Keychain, `src-tauri/src/secret_store.rs:1-13`) are not part
of the file backup and are out of scope for this drill.

## 2. Preconditions (all must hold before Phase A)

Check each one; the "expected" column is the pass signal.

| #   | Check                          | Command                                                                                                | Expected observable outcome                                                                                                                                                                                                                                                                                                                                |
| --- | ------------------------------ | ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1  | `sqlite3` CLI available        | `Get-Command sqlite3 -ErrorAction SilentlyContinue`                                                    | A result with a `Source` path. Nothing printed → install the [sqlite-tools](https://www.sqlite.org/download.html) package and re-run.                                                                                                                                                                                                                      |
| P2  | App **not** running            | `Get-Process -Name 'Speak','speak','Handy','handy' -ErrorAction SilentlyContinue`                      | Nothing printed. The installed binary name is the Tauri `productName` `Speak` (`src-tauri/tauri.conf.json:3`); `Handy` is included for older builds. If a process is listed, quit it (tray icon → Quit) and re-check — a running app is exactly the torn-copy risk described in the [operator guide §1](../meeting-mode-data-migration-operator-guide.md). |
| P3  | Correct data folder identified | `Get-ChildItem -Path $liveDataDir -Filter 'history.db*'`                                               | At least one file listed. `$liveDataDir` is set in §3; for the two layouts (normal vs. portable) and how to tell them apart, see [operator guide §1](../meeting-mode-data-migration-operator-guide.md) — do not guess.                                                                                                                                     |
| P4  | Enough scratch space           | `"{0:N1} MB" -f ((Get-ChildItem $liveDataDir -Recurse -File \| Measure-Object Length -Sum).Sum / 1MB)` | A size figure; the drill root (§3) needs at least ~3× this (live copy + backup set + fresh profile).                                                                                                                                                                                                                                                       |

## 3. Setup — drill variables (run once per session)

```powershell
# Live profile — normal install. For a portable install substitute the
# <exe folder>\Data path (operator guide §1).
$liveDataDir  = Join-Path $env:APPDATA 'com.haeni.speak'

# Drill scratch area — must be OUTSIDE the live profile (operator guide §2,
# Step 0). Change the root and date to match your run.
$drillRoot    = 'D:\Drills\Speak\2026-08-22'
$backupDir    = Join-Path $drillRoot 'backup-set'
$freshProfile = Join-Path $drillRoot 'fresh-profile'
New-Item -ItemType Directory -Force -Path $drillRoot | Out-Null
```

**Expected outcome:** the four variables are set, the drill root exists
(`Test-Path $drillRoot` → `True`; Phase A writes its manifest there before
the backup directory is created), and `$freshProfile` does not exist yet (`Test-Path $freshProfile` → `False`). The fresh profile is created
empty in Phase B; nothing in this drill ever writes to `$liveDataDir`.

## 4. Phase A — Backup while the app is closed

### A1 — Record a baseline manifest

```powershell
$rowCount = sqlite3 (Join-Path $liveDataDir 'history.db') `
    'SELECT COUNT(*) FROM transcription_history;'
$rowCount
Get-ChildItem -Path $liveDataDir -Filter 'history.db*' |
    Get-FileHash -Algorithm SHA256 |
    Export-Csv (Join-Path $drillRoot 'manifest-historydb.csv') -NoTypeInformation
Get-ChildItem -Path (Join-Path $liveDataDir 'recordings') -Filter '*.wav' -ErrorAction SilentlyContinue |
    Measure-Object | Select-Object -ExpandProperty Count
```

**Expected outcome:** `$rowCount` prints a non-negative integer — write it
down (e.g. `42`); it is the pass reference for Phase B/C. The CSV exists and
has one row per `history.db*` file. The recording count prints a number
(record it too; recordings are optional to copy but the count is a useful
cross-check). If `sqlite3` errors on the live database here, stop — that is a
**source** problem, not a drill problem; escalate per the
[operator guide §5](../meeting-mode-data-migration-operator-guide.md) quick
reference card.

### A2 — Copy the backup set

Copies **all** `history.db*` files (database plus any journal sidecars — the
reason for copying the whole set, never just the `.db`, is in
[operator guide §1](../meeting-mode-data-migration-operator-guide.md)) plus
the settings store:

```powershell
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
Get-ChildItem -Path $liveDataDir -Filter 'history.db*' |
    Copy-Item -Destination $backupDir -Force
Copy-Item (Join-Path $liveDataDir 'settings_store.json') $backupDir -Force
Get-ChildItem $backupDir
```

**Expected outcome:** the listing shows `history.db` (plus sidecars only if
they existed in the live folder — on a healthy idle system, none; verified:
the backend does not enable WAL, [operator guide §1](../meeting-mode-data-migration-operator-guide.md))
and `settings_store.json`, all with sizes > 0.

### A3 — Verify the backup copy itself

```powershell
sqlite3 (Join-Path $backupDir 'history.db') 'PRAGMA integrity_check;'
$backupHash = Get-ChildItem $backupDir -Filter 'history.db*' |
    Get-FileHash -Algorithm SHA256
$liveHash = Import-Csv (Join-Path $drillRoot 'manifest-historydb.csv')
Compare-Object $backupHash.Hash $liveHash.Hash
```

**Expected outcome:** `integrity_check` prints exactly `ok`, and
`Compare-Object` prints **nothing** (hash sets identical — with the app closed
the live files did not change during the copy). Any other integrity output or
a hash difference is a **FAIL**; see the criteria table in §6.

## 5. Phase B — Restore into a fresh profile

The fresh profile simulates a blank machine (or a blank portable `Data/`
directory). It is a plain directory tree; the app never runs against it until
the optional Phase C.

### B1 — Create the empty fresh profile

```powershell
New-Item -ItemType Directory -Force -Path $freshProfile | Out-Null
Get-ChildItem $freshProfile
```

**Expected outcome:** listing prints nothing — the profile is empty.

### B2 — Restore the whole backup set together

```powershell
Get-ChildItem $backupDir -Filter 'history.db*' |
    Copy-Item -Destination $freshProfile -Force
Copy-Item (Join-Path $backupDir 'settings_store.json') $freshProfile -Force
Get-ChildItem $freshProfile
```

**Expected outcome:** the fresh profile now contains the same file set as the
backup listing in A2. The `.db` and any sidecars travel together — they are
one consistent snapshot ([operator guide §3, step 2](../meeting-mode-data-migration-operator-guide.md)).
No stray-sidecar cleanup is needed here because the fresh profile started
empty (the cleanup rule for real restores with pre-existing files is
[operator guide §3, step 3](../meeting-mode-data-migration-operator-guide.md)).

### B3 — Round-trip integrity: database

```powershell
sqlite3 (Join-Path $freshProfile 'history.db') 'PRAGMA integrity_check;'
$restoredCount = sqlite3 (Join-Path $freshProfile 'history.db') `
    'SELECT COUNT(*) FROM transcription_history;'
$restoredCount
```

**Expected outcome:** `ok`, and `$restoredCount` equals the `$rowCount`
recorded in A1.

### B4 — Round-trip integrity: byte equality

```powershell
Compare-Object `
    (Get-ChildItem $freshProfile -Filter 'history.db*' | Get-FileHash -Algorithm SHA256 | ForEach-Object Hash) `
    (Get-ChildItem $backupDir   -Filter 'history.db*' | Get-FileHash -Algorithm SHA256 | ForEach-Object Hash)
```

**Expected outcome:** nothing printed — restored files are byte-identical to
the backup set.

### B5 — Round-trip integrity: settings store

```powershell
$s = Get-Content (Join-Path $freshProfile 'settings_store.json') -Raw |
    ConvertFrom-Json
$s.settings -is [pscustomobject]
$s.settings.settings_schema_version
```

**Expected outcome:** `True` (top-level `settings` object present — this is
the shape the app writes, `src-tauri/src/settings/store.rs:16` and
`store.rs:74`), and an integer schema version prints (a live normal-mode
profile currently shows `2`; any integer is acceptable, the value itself is
not the pass signal). A JSON parse error here is a **FAIL** — see §6.

## 6. Pass/fail criteria

The drill **passes** only if every row below holds. One failed row fails the
drill.

| #   | Check                                            | Pass                                                                   | Fail — what it indicates                                                                                                                                                                                                                                                                                                                |
| --- | ------------------------------------------------ | ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F1  | App closed (P2) stayed closed through Phase A    | No `Speak`/`Handy` process listed at any check                         | Something restarted the app mid-drill; the backup may be torn. Quit it and rerun from A1.                                                                                                                                                                                                                                               |
| F2  | `integrity_check` on the **backup** copy (A3)    | `ok`                                                                   | The copy is bad. If rerunning A2–A3 still fails while `sqlite3` on the _live_ db (A1) was fine, the copy step was interrupted — rerun. If `integrity_check` fails on the _live_ db too, the source database is damaged; escalate per the [operator guide §5](../meeting-mode-data-migration-operator-guide.md) card.                    |
| F3  | Backup hashes equal live manifest (A3)           | `Compare-Object` empty                                                 | A file changed during the copy — an app instance was running (see F1) or another process touched the folder. Rerun from A1.                                                                                                                                                                                                             |
| F4  | Restored `integrity_check` + row count (B3)      | `ok` and count equals A1                                               | Restore incomplete or wrong file set restored (e.g. a `.db` copied without its `-wal` sidecar in a real-world case). Recheck B2 copied every file listed in A2.                                                                                                                                                                         |
| F5  | Restored hashes equal backup set (B4)            | `Compare-Object` empty                                                 | The restore copy is not byte-identical — storage fault on the drill disk or a partial copy. Rerun B2.                                                                                                                                                                                                                                   |
| F6  | Settings JSON parses with `settings` object (B5) | `True`, integer schema version                                         | `settings_store.json` was truncated or corrupted in copy. Rerun B2; if it recurs, suspect the drill disk. Note the app itself salvages partial settings (`store.rs:100`), but a drill must not rely on salvage — the file must round-trip cleanly.                                                                                      |
| F7  | Fresh profile untouched live data                | `$liveDataDir` file hashes still match the A1 manifest after the drill | The drill wrote into the live profile — a variable pointed at the wrong directory. Investigate before any further drill. Re-verify with: `Compare-Object (Get-ChildItem $liveDataDir -Filter 'history.db*' \| Get-FileHash \| ForEach-Object Hash) (Import-Csv (Join-Path $drillRoot 'manifest-historydb.csv') \| ForEach-Object Hash)` |

## 7. Optional Phase C — full-fidelity launch smoke test (portable clone)

This proves the _app itself_ opens the restored profile, using portable mode
as the mechanism for pointing an installed binary at the fresh profile
(marker file → `Data/` directory, `src-tauri/src/portable.rs:15-49`). Do this
only if you can copy the installed application files (read-only copy; the
original install is not modified). The live app must stay closed throughout
(P2): the clone carries the same app identifier, so a running live instance
would win the single-instance handover and the clone would exit without
opening the restored profile.

```powershell
$portableRoot = Join-Path $drillRoot 'portable-clone'
Copy-Item 'C:\Program Files\Speak\*' $portableRoot -Recurse -Force   # adjust to your install dir
Set-Content (Join-Path $portableRoot 'portable') 'Handy Portable Mode'
New-Item -ItemType Directory -Force -Path (Join-Path $portableRoot 'Data') | Out-Null
Copy-Item (Join-Path $freshProfile '*') (Join-Path $portableRoot 'Data') -Force
Start-Process (Join-Path $portableRoot 'Speak.exe')
```

**Expected outcome:** the app starts against the clone's `Data` directory
(the startup log line `[portable] data dir:` names it,
`portable.rs:42`) and the history list shows the A1 row count of entries.
Playback is not expected to work here: this drill's backup set does not
include `recordings\`, so entries may show without audio — that is fine; copy
`recordings\` into the clone's `Data` folder too if playback should
round-trip ([operator guide §3, step 6](../meeting-mode-data-migration-operator-guide.md)).
Settings changed in the clone's UI must not appear in the live profile —
confirm one value differs or simply quit without changing anything. Quit the
app, then delete `$portableRoot`.

**Fail indication:** the app shows an empty history list → the clone's `Data`
folder does not match the restored layout (check the marker file content is
exactly `Handy Portable Mode`, `portable.rs:105-109`, and that
`history.db` sits directly in `Data\`, not a subfolder).

## 8. Cleanup

```powershell
Remove-Item $drillRoot -Recurse -Force
```

**Expected outcome:** `$drillRoot` gone (`Test-Path $drillRoot` → `False`).
The live profile was never touched (F7 already re-proved this).

## 9. When to run this drill

- On a fixed schedule (recommended: quarterly) per operator.
- After every application update that touches storage — the
  [operator guide §4](../meeting-mode-data-migration-operator-guide.md)
  requires backups taken with this procedure to stay restorable across future
  migrations; the drill is how that requirement is actually verified.
- After any change to the data folder layout (e.g. the app identifier in
  `src-tauri/tauri.conf.json:5` or the portable logic in
  `src-tauri/src/portable.rs`).

A drill pass is the evidence that the procedures in the
[operator guide](../meeting-mode-data-migration-operator-guide.md) still work
as written. If the drill fails on a current app version, update the operator
guide in the same change — that is the guide's own §4.3 rule.
