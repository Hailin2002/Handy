use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info, warn};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_specta::Event;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
/// Note: For users upgrading from tauri-plugin-sql, migrate_from_tauri_plugin_sql()
/// converts the old _sqlx_migrations table tracking to the user_version pragma,
/// ensuring migrations don't re-run on existing databases.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
];

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

/// Busy timeout applied to every history connection (milliseconds). Short
/// lock contention between concurrent writers (e.g. save + retention cleanup)
/// is retried by SQLite instead of failing immediately with SQLITE_BUSY.
const HISTORY_BUSY_TIMEOUT_MS: u64 = 5000;

/// Open a history database connection with concurrency-friendly pragmas:
///
/// - `journal_mode = WAL`: readers never block the writer and the writer
///   never blocks readers, so the UI can page through history while a
///   transcription is being saved.
/// - `busy_timeout = 5s`: SQLite waits on a locked database instead of
///   returning SQLITE_BUSY immediately.
///
/// Used for both the migration connection and every per-call connection so
/// all access paths share the same configuration.
pub fn open_configured_connection(db_path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    configure_connection(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> Result<()> {
    // WAL is persistent in the database file, but setting it on every open is
    // cheap and guarantees fresh databases are correct from the first write.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(std::time::Duration::from_millis(HISTORY_BUSY_TIMEOUT_MS))?;
    Ok(())
}

/// Outcome of probing an existing history database file for usability.
enum DbHealth {
    /// The file opens, is readable, and passes `PRAGMA quick_check`.
    Healthy,
    /// The file is unreadable or corrupt. Carries a human-readable reason.
    Corrupt(String),
    /// The failure is unrelated to corruption (e.g. filesystem permissions).
    /// Must NOT trigger quarantine — the cause has to be surfaced honestly.
    Unrelated(anyhow::Error),
}

/// Classify an error observed while opening/probing the history database:
/// only SQLite's explicit "not a database" / "database corrupt" codes count
/// as corruption; everything else stays inconclusive.
fn classify_probe_error(err: anyhow::Error) -> DbHealth {
    let is_corruption = err
        .downcast_ref::<rusqlite::Error>()
        .map(|e| {
            matches!(
                e,
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.code == rusqlite::ErrorCode::DatabaseCorrupt
                        || failure.code == rusqlite::ErrorCode::NotADatabase
            )
        })
        .unwrap_or(false);

    if is_corruption {
        DbHealth::Corrupt(err.to_string())
    } else {
        DbHealth::Unrelated(err)
    }
}

/// Probe `db_path` without modifying it (beyond SQLite's normal WAL setup on
/// an already-valid database). A file is considered corrupt when:
///
/// - opening/configuring it fails with SQLITE_NOTADB / SQLITE_CORRUPT
///   (garbage bytes, wrong-format files),
/// - reading the schema catalog fails,
/// - or `PRAGMA quick_check` reports anything other than "ok" (truncation,
///   malformed page chains). `quick_check` is a read-only integrity scan;
///   at startup this runs once against a single-digit-MB database.
fn probe_database_health(db_path: &Path) -> DbHealth {
    let conn = match open_configured_connection(db_path) {
        Ok(conn) => conn,
        Err(e) => return classify_probe_error(e),
    };

    // The schema catalog must be readable at all.
    if let Err(e) = conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |_| Ok(())) {
        return classify_probe_error(anyhow::Error::new(e));
    }

    match conn.query_row::<String, _, _>("PRAGMA quick_check", [], |row| row.get(0)) {
        Ok(result) if result == "ok" => DbHealth::Healthy,
        Ok(result) => DbHealth::Corrupt(format!("PRAGMA quick_check reported: {}", result)),
        Err(e) => classify_probe_error(anyhow::Error::new(e)),
    }
}

/// Move a corrupt history database aside under a timestamped name so the user
/// can attempt recovery, instead of deleting it. The `-wal` and `-shm`
/// sidecars are moved alongside so no stale WAL frames can ever be applied to
/// the freshly created replacement database. Returns the quarantine path of
/// the main database file.
fn quarantine_corrupt_database(db_path: &Path) -> Result<PathBuf> {
    let dir = db_path
        .parent()
        .ok_or_else(|| anyhow!("database path {:?} has no parent directory", db_path))?;
    let file_name = db_path
        .file_name()
        .ok_or_else(|| anyhow!("database path {:?} has no file name", db_path))?
        .to_string_lossy()
        .into_owned();

    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let mut target = dir.join(format!("{}.corrupt-{}.quarantined", file_name, timestamp));
    let mut attempt: u32 = 0;
    while target.exists() {
        attempt += 1;
        if attempt > 1000 {
            return Err(anyhow!(
                "could not find a free quarantine name for {:?}",
                db_path
            ));
        }
        target = dir.join(format!(
            "{}.corrupt-{}-{}.quarantined",
            file_name, timestamp, attempt
        ));
    }

    fs::rename(db_path, &target)
        .with_context(|| format!("quarantining corrupt database to {:?}", target))?;
    debug!("Quarantined corrupt history database: {:?}", target);

    // Sidecars are moved best-effort: their absence on the quarantine copy is
    // harmless (WAL data was already part of the corrupt picture), but they
    // must not linger next to the new database.
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        if sidecar.exists() {
            let sidecar_target = PathBuf::from(format!("{}{}", target.display(), suffix));
            if let Err(e) = fs::rename(&sidecar, &sidecar_target) {
                warn!(
                    "Could not move sidecar {} out of the way: {}",
                    sidecar.display(),
                    e
                );
            }
        }
    }

    Ok(target)
}

/// Apply all schema migrations to the database at `db_path`, creating the
/// file if it does not exist yet. Shared by normal startup and by the fresh
/// rebuild after a quarantine.
pub fn create_and_migrate_history_db(db_path: &Path) -> Result<()> {
    let mut conn = open_configured_connection(db_path)?;

    // Handle migration from tauri-plugin-sql to rusqlite_migration:
    // tauri-plugin-sql used _sqlx_migrations table, rusqlite_migration uses
    // the user_version pragma.
    HistoryManager::migrate_from_tauri_plugin_sql(&conn)?;

    // Create migrations object and run to latest version
    let migrations = Migrations::new(MIGRATIONS.to_vec());

    // Validate migrations in debug builds
    #[cfg(debug_assertions)]
    migrations.validate().expect("Invalid migrations");

    // Get current version before migration
    let version_before: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    debug!("Database version before migration: {}", version_before);

    // Apply any pending migrations
    migrations.to_latest(&mut conn)?;

    // Get version after migration
    let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if version_after > version_before {
        info!(
            "Database migrated from version {} to {}",
            version_before, version_after
        );
    } else {
        debug!("Database already at latest version {}", version_after);
    }

    Ok(())
}

/// Ensure the history database at `db_path` is usable, quarantining and
/// rebuilding it if it is corrupt or unreadable:
///
/// 1. Probe the existing file (`PRAGMA quick_check` + schema readability).
/// 2. On corruption: rename it (plus `-wal`/`-shm`) to a timestamped
///    `<name>.corrupt-<timestamp>.quarantined` file next to the original —
///    never delete it, the user may still want to attempt recovery — then log
///    a warning stating explicitly that old data did NOT survive in the new
///    database.
/// 3. Recreate a fresh schema via the standard migrations and continue.
///
/// Non-corruption failures (e.g. permission errors) are returned as errors so
/// the real cause surfaces instead of silently masking data loss.
pub fn ensure_usable_history_db(db_path: &Path) -> Result<()> {
    match probe_database_health(db_path) {
        DbHealth::Healthy => {
            create_and_migrate_history_db(db_path)?;
            Ok(())
        }
        DbHealth::Unrelated(err) => Err(
            err.context("history database could not be opened for reasons unrelated to corruption")
        ),
        DbHealth::Corrupt(reason) => {
            warn!(
                "History database at {:?} is unreadable/corrupt ({}); \
                 quarantining it and starting with an empty history. \
                 Old entries did NOT survive into the new database and can be \
                 found in the .quarantined file for manual recovery.",
                db_path, reason
            );
            let quarantined = quarantine_corrupt_database(db_path)?;
            warn!(
                "Corrupt history database quarantined at {:?}; created a fresh one.",
                quarantined
            );
            create_and_migrate_history_db(db_path)?;
            Ok(())
        }
    }
}

impl HistoryManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create recordings directory in app data dir
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let recordings_dir = app_data_dir.join("recordings");
        let db_path = app_data_dir.join("history.db");

        // Ensure recordings directory exists
        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        // Health-check the existing database first. A corrupt/unreadable file
        // is quarantined (never deleted) and a fresh schema is created in its
        // place so startup can continue; see `ensure_usable_history_db`.
        ensure_usable_history_db(&self.db_path)
    }

    /// Migrate from tauri-plugin-sql's migration tracking to rusqlite_migration's.
    /// tauri-plugin-sql used a _sqlx_migrations table, while rusqlite_migration uses
    /// SQLite's user_version pragma. This function checks if the old system was in use
    /// and sets the user_version accordingly so migrations don't re-run.
    fn migrate_from_tauri_plugin_sql(conn: &Connection) -> Result<()> {
        // Check if the old _sqlx_migrations table exists
        let has_sqlx_migrations: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_sqlx_migrations {
            return Ok(());
        }

        // Check current user_version
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > 0 {
            // Already migrated to rusqlite_migration system
            return Ok(());
        }

        // Get the highest version from the old migrations table
        let old_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if old_version > 0 {
            info!(
                "Migrating from tauri-plugin-sql (version {}) to rusqlite_migration",
                old_version
            );

            // Set user_version to match the old migration state
            conn.pragma_update(None, "user_version", old_version)?;

            // Optionally drop the old migrations table (keeping it doesn't hurt)
            // conn.execute("DROP TABLE IF EXISTS _sqlx_migrations", [])?;

            info!(
                "Migration tracking converted: user_version set to {}",
                old_version
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        open_configured_connection(&self.db_path)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            post_processed_text: row.get("post_processed_text")?,
            post_process_prompt: row.get("post_process_prompt")?,
            post_process_requested: row.get("post_process_requested")?,
        })
    }

    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    /// Save a new history entry to the database.
    /// The WAV file should already have been written to the recordings directory.
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &file_name,
                timestamp,
                false,
                &title,
                &transcription_text,
                &post_processed_text,
                &post_process_prompt,
                post_process_requested,
            ],
        )?;

        let entry = HistoryEntry {
            id: conn.last_insert_rowid(),
            file_name,
            timestamp,
            saved: false,
            title,
            transcription_text,
            post_processed_text,
            post_process_prompt,
            post_process_requested,
        };

        debug!("Saved history entry with id {}", entry.id);

        self.cleanup_old_entries()?;

        // Emit typed event for real-time frontend updates
        if let Err(e) = (HistoryUpdatePayload::Added {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription(
        &self,
        id: i64,
        transcription_text: String,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE transcription_history
             SET transcription_text = ?1,
                 post_processed_text = ?2,
                 post_process_prompt = ?3
             WHERE id = ?4",
            params![
                transcription_text,
                post_processed_text,
                post_process_prompt,
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = conn
            .query_row(
                "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                 FROM transcription_history WHERE id = ?1",
                params![id],
                Self::map_history_entry,
            )?;

        debug!("Updated transcription for history entry {}", id);

        if let Err(e) = (HistoryUpdatePayload::Updated {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period = crate::settings::get_recording_retention_period(&self.app_handle);

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Use the old count-based logic with history_limit
                let limit = crate::settings::get_history_limit(&self.app_handle);
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    fn delete_entries_and_files(&self, entries: &[(i64, String)]) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let conn = self.get_connection()?;
        let mut deleted_count = 0;

        for (id, file_name) in entries {
            // Delete database entry
            conn.execute(
                "DELETE FROM transcription_history WHERE id = ?1",
                params![id],
            )?;

            // Delete WAV file
            let file_path = self.recordings_dir.join(file_name);
            if file_path.exists() {
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to delete WAV file {}: {}", file_name, e);
                } else {
                    debug!("Deleted old WAV file: {}", file_name);
                    deleted_count += 1;
                }
            }
        }

        Ok(deleted_count)
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

        // Get all entries that are not saved, ordered by timestamp desc
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries.push(row?);
        }

        if entries.len() > limit {
            let entries_to_delete = &entries[limit..];
            let deleted_count = self.delete_entries_and_files(entries_to_delete)?;

            if deleted_count > 0 {
                debug!("Cleaned up {} old history entries by count", deleted_count);
            }
        }

        Ok(())
    }

    fn cleanup_by_time(
        &self,
        retention_period: crate::settings::RecordingRetentionPeriod,
    ) -> Result<()> {
        let conn = self.get_connection()?;

        // Calculate cutoff timestamp (current time minus retention period)
        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60), // 3 days in seconds
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60), // 2 weeks in seconds
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60), // 3 months in seconds (approximate)
            _ => unreachable!("Should not reach here"),
        };

        // Get all unsaved entries older than the cutoff timestamp
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
        )?;

        let rows = stmt.query_map(params![cutoff_timestamp], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries_to_delete: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;

        if deleted_count > 0 {
            debug!(
                "Cleaned up {} old history entries based on retention period",
                deleted_count
            );
        }

        Ok(())
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Toggled { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    pub fn get_audio_file_path(&self, file_name: &str) -> PathBuf {
        self.recordings_dir.join(file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE id = ?1",
        )?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get the entry to find the file name
        if let Some(entry) = self.get_entry_by_id(id).await? {
            // Delete the audio file first
            let file_path = self.get_audio_file_path(&entry.file_name);
            if file_path.exists() {
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to delete audio file {}: {}", entry.file_name, e);
                    // Continue with database deletion even if file deletion fails
                }
            }
        }

        // Delete from database
        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        debug!("Deleted history entry with id: {}", id);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    /// G1: every history connection must report WAL journal mode.
    #[test]
    fn configured_connection_reports_wal_journal_mode() {
        let tmp = tempfile::TempDir::new().expect("create temp dir");
        let db_path = tmp.path().join("history.db");

        let conn = open_configured_connection(&db_path).expect("open configured connection");

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("read journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");
    }

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );",
        )
        .expect("create transcription_history table");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str, post_processed: Option<&str>) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                format!("handy-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
                post_processed,
                Option::<String>::None,
                false,
            ],
        )
        .expect("insert history entry");
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first", None);
        insert_entry(&conn, 200, "second", Some("processed"));

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
        assert_eq!(entry.post_processed_text.as_deref(), Some("processed"));
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed", None);
        insert_entry(&conn, 200, "", None);

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
    }
}
