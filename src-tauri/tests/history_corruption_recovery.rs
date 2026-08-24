//! Corrupt history.db startup resilience: quarantine and rebuild.
//!
//! Gates asserted here:
//! - G1: garbage-byte AND truncated-file fixtures each produce a quarantine
//!   copy plus a working fresh database (schema usable for reads/writes).
//! - G2: the quarantined original remains on disk untouched, byte-for-byte,
//!   for user recovery.
//! - G3: startup (`ensure_usable_history_db`, the exact function
//!   `HistoryManager::new` runs) completes with a logged warning and the app
//!   functions normally afterwards (insert/read round-trip succeeds).
//!
//! Additionally: a healthy database must never be quarantined, and its data
//! must survive the health check unchanged.

use speak_app_lib::{
    create_and_migrate_history_db, ensure_usable_history_db, open_configured_connection,
};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, Once, OnceLock};

// ---------------------------------------------------------------------------
// Minimal log capture so the warning emitted during recovery can be asserted
// without adding a new dev-dependency. `log::set_boxed_logger` may only be
// called once per process, hence the global state.
// ---------------------------------------------------------------------------

static WARNINGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

struct WarnCaptureLogger;

impl log::Log for WarnCaptureLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let sink = WARNINGS.get_or_init(|| Mutex::new(Vec::new()));
            if let Ok(mut guard) = sink.lock() {
                guard.push(record.args().to_string());
            }
        }
    }

    fn flush(&self) {}
}

fn install_warn_capture_logger() -> &'static Mutex<Vec<String>> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = log::set_boxed_logger(Box::new(WarnCaptureLogger));
    });
    log::set_max_level(log::LevelFilter::Warn);
    WARNINGS.get_or_init(|| Mutex::new(Vec::new()))
}

// ---------------------------------------------------------------------------
// Fixtures and helpers
// ---------------------------------------------------------------------------

/// Deterministic garbage bytes: an LCG-driven pattern that is definitely not
/// a SQLite database (no magic header) yet non-uniform, like real disk rot.
fn garbage_bytes(len: usize) -> Vec<u8> {
    let mut state: u32 = 0x1234_5678;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) {
    let mut f = fs::File::create(path).expect("create fixture file");
    f.write_all(bytes).expect("write fixture file");
    f.sync_all().expect("flush fixture file");
}

fn read_file(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read file")
}

fn list_quarantined(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = fs::read_dir(dir)
        .expect("list dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("history.db.corrupt-") && n.ends_with(".quarantined")
                })
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found
}

/// Insert a row through the app-configured connection and read it back,
/// proving the rebuilt database is fully functional.
fn round_trip_entry(db_path: &Path) {
    let conn = open_configured_connection(db_path).expect("open configured connection");
    conn.execute(
        "INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text,
            post_processed_text, post_process_prompt, post_process_requested
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "recovered-test.wav",
            1_700_000_000_i64,
            false,
            "Recovered",
            "round trip",
            Option::<String>::None,
            Option::<String>::None,
            false,
        ],
    )
    .expect("insert into rebuilt database");

    let (title, text): (String, String) = conn
        .query_row(
            "SELECT title, transcription_text FROM transcription_history WHERE file_name = ?1",
            rusqlite::params!["recovered-test.wav"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read back from rebuilt database");
    assert_eq!(title, "Recovered");
    assert_eq!(text, "round trip");
}

// ---------------------------------------------------------------------------
// G1 + G2 + G3: garbage-byte fixture
// ---------------------------------------------------------------------------

#[test]
fn garbage_byte_fixture_is_quarantined_and_replaced_with_working_db() {
    install_warn_capture_logger();
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    // Fixture: a file full of deterministic garbage where history.db lives.
    let original = garbage_bytes(16 * 1024);
    write_file(&db_path, &original);

    // Startup must complete (never panic, never hard-fail on corruption).
    ensure_usable_history_db(&db_path).expect("startup must survive a corrupt database");

    // G1: a working fresh database now exists at the original location...
    round_trip_entry(&db_path);

    // ...and the corrupt file was quarantined (exactly once).
    let quarantined = list_quarantined(tmp.path());
    assert_eq!(
        quarantined.len(),
        1,
        "exactly one quarantine copy must exist; got {:?}",
        quarantined
    );

    // G2: the quarantined original is untouched, byte-for-byte.
    assert_eq!(
        read_file(&quarantined[0]),
        original,
        "quarantined file must preserve the corrupt original bytes"
    );
    // The original location must hold the NEW database, not the garbage.
    assert_ne!(
        read_file(&db_path),
        original,
        "original path must have been replaced by a fresh database"
    );

    // No stray WAL/SHM sidecars from the corrupt file may linger next to the
    // fresh database.
    for suffix in ["-wal", "-shm"] {
        let sidecar = tmp.path().join(format!("history.db{}", suffix));
        assert!(
            !sidecar.exists(),
            "stale sidecar {} must be moved away with the quarantine",
            sidecar.display()
        );
    }
}

// ---------------------------------------------------------------------------
// G1 + G2: truncated-file fixture
// ---------------------------------------------------------------------------

#[test]
fn truncated_database_fixture_is_quarantined_and_rebuilt() {
    install_warn_capture_logger();
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    // Build a genuinely valid database first, with real data in it.
    create_and_migrate_history_db(&db_path).expect("migrate fresh database");
    {
        let conn = open_configured_connection(&db_path).expect("open connection");
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text,
                post_processed_text, post_process_prompt, post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "lost.wav",
                1_700_000_001_i64,
                false,
                "Lost entry",
                "this data will be lost to truncation",
                Option::<String>::None,
                Option::<String>::None,
                false,
            ],
        )
        .expect("seed entry");
        // Dropping the last connection checkpoints the WAL back into the
        // main file and removes the sidecars, so truncating the main file
        // deterministically damages the database image.
    }

    let full_len = fs::metadata(&db_path).expect("stat db").len() as usize;
    assert!(full_len > 0, "valid database must not be empty");

    // Truncate well into the first page: a classic interrupted-write injury.
    let truncated_len = (full_len / 2).min(1024);
    let f = fs::OpenOptions::new()
        .write(true)
        .open(&db_path)
        .expect("open db for truncation");
    f.set_len(truncated_len as u64).expect("truncate db file");
    drop(f);

    let truncated_bytes = read_file(&db_path);

    // Startup recovers instead of failing.
    ensure_usable_history_db(&db_path).expect("startup must survive a truncated database");

    // G1: fresh working database at the original location.
    round_trip_entry(&db_path);

    // Quarantine exists exactly once and preserves the truncated original.
    let quarantined = list_quarantined(tmp.path());
    assert_eq!(
        quarantined.len(),
        1,
        "exactly one quarantine copy must exist; got {:?}",
        quarantined
    );
    let kept = read_file(&quarantined[0]);
    assert_eq!(
        kept.len(),
        truncated_len,
        "quarantine must preserve the truncated file's length"
    );
    assert_eq!(
        kept, truncated_bytes,
        "G2: quarantined original must remain untouched for user recovery"
    );
}

// ---------------------------------------------------------------------------
// G3: warning is logged and the app functions normally after recovery
// ---------------------------------------------------------------------------

#[test]
fn recovery_logs_warning_and_app_functions_normally_afterwards() {
    let warnings = install_warn_capture_logger();
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");
    write_file(&db_path, &garbage_bytes(8 * 1024));

    ensure_usable_history_db(&db_path).expect("startup completes despite corruption");

    let captured = warnings.lock().expect("lock warnings").clone();
    let joined = captured.join("\n");
    assert!(
        joined.contains("quarantin"),
        "a warning mentioning the quarantine must be logged; got: {:?}",
        captured
    );
    assert!(
        joined.contains("did NOT survive"),
        "the warning must state explicitly that old data did not survive; got: {:?}",
        captured
    );

    // App functions normally: normal reads/writes work against the rebuild.
    round_trip_entry(&db_path);
}

// ---------------------------------------------------------------------------
// Negative control: healthy databases must never be touched
// ---------------------------------------------------------------------------

#[test]
fn healthy_database_is_not_quarantined_and_keeps_its_data() {
    install_warn_capture_logger();
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    create_and_migrate_history_db(&db_path).expect("migrate fresh database");
    {
        let conn = open_configured_connection(&db_path).expect("open connection");
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text,
                post_processed_text, post_process_prompt, post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "keep.wav",
                1_700_000_002_i64,
                true,
                "Kept entry",
                "precious user data",
                Option::<String>::None,
                Option::<String>::None,
                false,
            ],
        )
        .expect("seed entry");
    }

    ensure_usable_history_db(&db_path).expect("healthy db passes the startup check");

    assert!(
        list_quarantined(tmp.path()).is_empty(),
        "a healthy database must never be quarantined"
    );

    // Old data survives intact when the database was healthy.
    let conn = open_configured_connection(&db_path).expect("reopen connection");
    let text: String = conn
        .query_row(
            "SELECT transcription_text FROM transcription_history WHERE file_name = 'keep.wav'",
            [],
            |row| row.get(0),
        )
        .expect("old data still readable");
    assert_eq!(text, "precious user data");
}
