# Operator Guide — Dictation History Data Safety, Backup, and Migration Safety

**Audience:** operators and support staff. No development environment or Rust
knowledge is required to follow this guide.

**Scope:** the _current_ dictation data footprint only — where it lives, how to
back it up safely, how to restore it, and what any future migration must not
break. This guide describes the application as it exists today.

---

## 1. Where your dictation history actually lives

Handy stores all dictation history in a single SQLite database file named
`history.db`, plus audio recordings in a `recordings/` subfolder next to it.
The folder containing them depends on whether the installation runs in
**portable mode**:

| Install type     | History database location                                                                                 |
| ---------------- | --------------------------------------------------------------------------------------------------------- |
| Normal install   | `%APPDATA%\com.haeni.speak\history.db` (i.e. `C:\Users\<you>\AppData\Roaming\com.haeni.speak\history.db`) |
| Portable install | `<exe folder>\Data\history.db`                                                                            |

How this is decided, verified in the current source tree:

- On startup the app checks for a marker file named `portable` next to the
  executable (`src-tauri/src/portable.rs:20`). If that marker is valid and
  contains the string `Handy Portable Mode`
  (`src-tauri/src/portable.rs:105-109`), all user data goes into a `Data/`
  directory next to the executable (`src-tauri/src/portable.rs:21`,
  created at `src-tauri/src/portable.rs:36-39`).
- The portable-aware path resolver returns the portable `Data/` dir when
  active and otherwise falls back to Tauri's standard app-data directory
  (`src-tauri/src/portable.rs:69-75`).
- The history manager resolves its base directory through exactly that
  resolver (`src-tauri/src/managers/history.rs:77`), puts recordings in an
  `app_data_dir\recordings` subfolder
  (`src-tauri/src/managers/history.rs:78`), and the database at
  `app_data_dir\history.db` (`src-tauri/src/managers/history.rs:79`).
- For normal installs the app-data directory is derived from the Tauri
  identifier `com.haeni.speak` (`src-tauri/tauri.conf.json:5`), which yields
  `%APPDATA%\com.haeni.speak` on Windows.

> **Support tip:** if the user is unsure which mode they have, have them check
> for a file named `portable` (containing the text `Handy Portable Mode`) next
> to `Handy.exe`. Present → portable layout; absent → `%APPDATA%` layout.

### Related files in the same folder

| File                         | What it is                                                                                               |
| ---------------------------- | -------------------------------------------------------------------------------------------------------- |
| `history.db`                 | The transcription-history database itself.                                                               |
| `history.db-journal`         | SQLite rollback journal; only present while a write transaction is in flight or after a crash mid-write. |
| `recordings\*.wav`           | The audio files referenced by `transcription_history.file_name` rows.                                    |
| `settings.json`, other files | App settings and models — separate from history; back up separately if you want a full machine restore.  |

### About WAL / `-shm` sidecar files

SQLite can run in different _journal modes_. In **WAL mode** two extra sidecar
files appear next to the database: `history.db-wal` (pending write-ahead log)
and `history.db-shm` (shared-memory index for the log).

**Current status, verified:** Handy's Rust backend does **not** enable WAL mode
today — there is no `PRAGMA journal_mode` anywhere under `src-tauri/src/`
(verified by searching the whole backend source). The database therefore uses
SQLite's default rollback-journal mode, and on a healthy idle system you will
normally see **only** `history.db`.

Why this matters even so:

1. A plain `.db`-only copy taken **while the app is running** can still be
   inconsistent: committed-but-not-yet-flushed pages may live in OS buffers or
   in an open rollback journal (`history.db-journal`), so the copied `.db` can
   be missing the newest entries or, worse, be torn mid-write.
2. If WAL mode is ever enabled (by the app or by a future migration), copying
   **only** `history.db` while `-wal`/`-shm` exist silently loses every
   transaction still sitting in the `-wal` file.

The safe procedure below handles both cases by closing the app first and by
copying **all** `history.db*` files, never just the bare `.db`.

---

## 2. Backup procedure (step by step)

Prerequisite: the [`sqlite3` command-line shell](https://www.sqlite.org/download.html)
(prebuilt "sqlite-tools" for your platform) must be available as `sqlite3` in
your `PATH` for step 5. Steps 1–4 need nothing but PowerShell.

### Step 0 — Pick a backup destination

Choose a destination **outside** the app data folder (e.g. another disk or a
network share). Everything below touches only files inside that destination
plus reads from the source folder.

```powershell
$backupDir = 'D:\Backups\Handy\2026-08-22'   # <-- change me: user-chosen destination
```

### Step 1 — Locate the data folder

```powershell
# Normal install:
$dataDir = Join-Path $env:APPDATA 'com.haeni.speak'

# OR portable install (uncomment and point at the folder that contains Handy.exe):
# $dataDir = 'C:\Tools\Handy\Data'
```

Confirm you picked the right folder — this should list `history.db`:

```powershell
Get-ChildItem -Path $dataDir -Filter 'history.db*'
```

If nothing is listed, you are in the wrong layout; switch between the two
variants of Step 1.

### Step 2 — Quiesce the database (checkpoint / close)

Fully quit Handy (tray icon → Quit), then confirm no process remains:

```powershell
Get-Process -Name 'Handy','handy' -ErrorAction SilentlyContinue
```

That command printing nothing means the app is closed. Then check the live
folder for leftover journals:

```powershell
Get-ChildItem -Path $dataDir -Filter 'history.db-*'
```

- Nothing listed → clean state, continue.
- A `history.db-wal` exists → the app was built with WAL enabled or left one
  behind. If you have `sqlite3`, fold it into the main file before copying:
  `sqlite3 (Join-Path $dataDir 'history.db') 'PRAGMA wal_checkpoint(TRUNCATE);'`
  then re-check. Do **not** delete sidecar files by hand.
- Only a stale `history.db-journal` with the app closed → leave it in place and
  copy it too (Step 3 copies all `history.db*` files anyway); SQLite will roll
  the incomplete transaction back on next open.

### Step 3 — Copy all three files

Copy everything matching `history.db*` (the database plus any journal/WAL/SHM
sidecars) into the backup destination:

```powershell
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
Get-ChildItem -Path $dataDir -Filter 'history.db*' |
    Copy-Item -Destination $backupDir -Force
Get-ChildItem -Path $backupDir   # sanity check: files are there, sizes > 0
```

Optionally also archive the recordings referenced by the history (audio files
are not inside the database):

```powershell
Compress-Archive -Path (Join-Path $dataDir 'recordings') `
    -DestinationPath (Join-Path $backupDir 'recordings.zip') -Force
```

### Step 4 — Verify the copy with `integrity_check`

Always verify the **copy**, not the original:

```powershell
$dbCopy = Join-Path $backupDir 'history.db'
if (-not (Get-Command sqlite3 -ErrorAction SilentlyContinue)) {
    throw 'sqlite3 CLI not found. Install the sqlite-tools package from https://www.sqlite.org/download.html and re-run.'
}
sqlite3 $dbCopy 'PRAGMA integrity_check;'
```

Expected output is exactly:

```
ok
```

Anything else (e.g. `*** in-database corruption ***`, page errors) → the copy
is bad; do not trust it, fix the source state (redo from Step 2) and repeat.
For extra confidence you may also count rows in the copy:

```powershell
sqlite3 $dbCopy 'SELECT COUNT(*) FROM transcription_history;'
```

The number should match what the user sees as their history entry count
(subject to the app's automatic retention cleanup).

---

## 3. Restore procedure

Use this when `history.db` is lost, corrupted, or a user moved machines.

1. **Quit Handy completely** (tray icon → Quit); verify with
   `Get-Process -Name 'Handy','handy'` returning nothing.
2. **Restore the whole backup set together** — the `.db` plus any
   `-journal`/`-wal`/`-shm` files that were captured with it. They belong to
   one consistent snapshot and must not be mixed across backups:
   ```powershell
   $dataDir   = Join-Path $env:APPDATA 'com.haeni.speak'   # adjust per Section 1
   $backupDir = 'D:\Backups\Handy\2026-08-22'              # your backup set
   Get-ChildItem -Path $backupDir -Filter 'history.db*' |
       Copy-Item -Destination $dataDir -Force
   ```
3. **Remove orphaned sidecars that were NOT part of the backup set.** A
   leftover `-wal`/`-shm` from a newer state combined with an older `.db` will
   corrupt or confuse the restore. With the app closed, delete only those two
   if they exist and were not restored:
   ```powershell
   Remove-Item (Join-Path $dataDir 'history.db-wal'), `
               (Join-Path $dataDir 'history.db-shm') -ErrorAction SilentlyContinue
   ```
   Never delete `history.db` itself here, and never delete a `-journal` that
   came _from_ the backup.
4. **Verify before letting the user rely on it:** run
   `sqlite3 (Join-Path $dataDir 'history.db') 'PRAGMA integrity_check;'` —
   expected output `ok`.
5. **Start Handy.** On launch the app opens the database and applies its own
   migrations to whatever version it finds
   (`src-tauri/src/managers/history.rs:99-136`); migrations are version-tracked
   via the SQLite `user_version` pragma
   (`src-tauri/src/managers/history.rs:116-133`), so restoring an older backup
   simply migrates forward and does not re-run old steps. Confirm the history
   list shows the expected entries.
6. Restore `recordings.zip` (if archived) by extracting it so the WAV files sit
   back in `<dataDir>\recordings\`; entry playback needs the WAV named in each
   row's `file_name` column to exist there
   (`src-tauri/src/managers/history.rs:584-586`).

Rolling back to a _newer_ backup after having already started the app on an
older one is safe as long as the app is closed during the copy (Step 1).

---

## 4. What the Phase 3 meeting-mode migration must preserve

> ⚠️ **This section is forward-looking. It describes requirements for planned
> work; none of it is implemented today. It is not a contract of future
> behavior — the actual Phase 3 design may differ.**

As of this documentation, **no meeting-mode schema of any kind exists**: the
complete migration set in the current tree consists of three steps that create
and alter the single `transcription_history` table
(`src-tauri/src/managers/history.rs:20-34`), tracked by `user_version`
(currently at 3 once migrated). There are no meeting, project, knowledge, or
change-log tables, and the only "meeting" code in the backend is a standalone
measurement-spike binary unrelated to storage
(`src-tauri/src/bin/meeting_loopback_spike.rs`).

Any future Phase 3 storage migration (per
`docs/meeting-mode-plan.md` §Phase 3) must preserve, for existing dictation
users:

1. **The dictation database keeps working unchanged.** Existing
   `transcription_history` rows, their columns, and their `user_version`
   tracking must survive; the app must keep opening and migrating the very
   same `history.db` resolved through the same portable-aware path logic
   (`src-tauri/src/portable.rs:69-75`,
   `src-tauri/src/managers/history.rs:77-79`). Users must not lose dictation
   history when upgrading.
2. **Recordings stay resolvable.** Every history row's `file_name` must keep
   pointing at an existing file under the resolved `recordings/` directory.
3. **Backups remain valid.** A backup taken with the procedure in Section 2
   must remain restorable across the migration: the migration must tolerate a
   database restored from an older snapshot and bring it forward cleanly, and
   must not introduce consistency requirements that break "close the app,
   copy all `history.db*` files" as a sufficient backup method (or, if it
   does — e.g. by enabling WAL — this guide must be updated in the same
   change).
4. **No silent relocation.** If meeting-mode data moves to new files or
   folders, the existing dictation files must stay where Sections 1–3 say they
   are, or the operator guide must be updated in the same release.

---

## 5. Quick reference card (for support tickets)

```
Locate:    %APPDATA%\com.haeni.speak\          (normal)
           <exe folder>\Data\                  (portable — look for "portable" marker file next to exe)
Backup:    quit app -> copy ALL history.db* files -> sqlite3 <copy> "PRAGMA integrity_check;"  => expect "ok"
Restore:   quit app -> copy backup set back -> remove stray -wal/-shm not in backup -> integrity_check -> start app
Escalate:  integrity_check reports anything other than "ok"
```
