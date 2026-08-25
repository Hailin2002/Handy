//! Concurrency regression tests for the transcription-history SQLite database.
//!
//! The history manager opens a fresh connection per operation
//! (`open_configured_connection`), so parallel writers (save + retention
//! cleanup, rapid successive transcriptions) previously raced on the default
//! rollback-journal lock and could surface SQLITE_BUSY to callers.
//!
//! Gates asserted here:
//! - G1: `journal_mode` reports `wal` on a history connection.
//! - G2: a multi-writer stress test (>=4 threads, each with its own
//!   connection) completes with zero SQLITE_BUSY errors and all rows present.
//! - G3 (access while writing): a reader can query the database while a
//!   writer holds an open transaction, and a second writer waits out the lock
//!   via busy_timeout instead of failing.

use speak_app_lib::open_configured_connection;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

const HISTORY_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS transcription_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_name TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    saved BOOLEAN NOT NULL DEFAULT 0,
    title TEXT NOT NULL,
    transcription_text TEXT NOT NULL,
    post_processed_text TEXT,
    post_process_prompt TEXT,
    post_process_requested BOOLEAN NOT NULL DEFAULT 0
);";

fn is_busy_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

fn insert_entry(conn: &rusqlite::Connection, file_name: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text,
            post_processed_text, post_process_prompt, post_process_requested
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            file_name,
            1_000_000_i64,
            false,
            format!("Recording {}", file_name),
            "hello",
            Option::<String>::None,
            Option::<String>::None,
            false,
        ],
    )
}

/// G1: the history connection must run in WAL journal mode.
#[test]
fn history_connection_reports_wal_journal_mode() {
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    let conn = open_configured_connection(&db_path).expect("open history connection");

    let journal_mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("read journal_mode");
    assert_eq!(
        journal_mode.to_lowercase(),
        "wal",
        "history connections must use WAL journal mode"
    );
}

// G1 (persistence): WAL is stored in the database file, so a *second*
// connection opened later — as every per-call connection in the app is —
// must still report `wal` without any extra setup.
#[test]
fn wal_persists_across_subsequent_connections() {
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    // First open creates and switches the file into WAL mode.
    drop(open_configured_connection(&db_path).expect("first connection"));

    let second = open_configured_connection(&db_path).expect("second connection");
    let journal_mode: String = second
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("read journal_mode");
    assert_eq!(journal_mode.to_lowercase(), "wal");
}

/// G2: >=4 threads each write through their own connection concurrently.
/// Every insert must succeed — zero SQLITE_BUSY errors — and all written
/// rows must be visible afterwards.
#[test]
fn parallel_writers_complete_without_busy_errors() {
    const WRITERS: usize = 6;
    const ROWS_PER_WRITER: usize = 40;

    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = Arc::new(tmp.path().join("history.db"));

    {
        let setup = open_configured_connection(&db_path).expect("setup connection");
        setup
            .execute_batch(HISTORY_TABLE_DDL)
            .expect("create table");
    }

    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);

    for writer in 0..WRITERS {
        let db_path = Arc::clone(&db_path);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let conn = open_configured_connection(&db_path).expect("writer connection");
            barrier.wait(); // maximize overlap of the write windows

            for row in 0..ROWS_PER_WRITER {
                let result = insert_entry(&conn, &format!("writer-{}-row-{}.wav", writer, row));
                if let Err(err) = &result {
                    assert!(
                        !is_busy_error(err),
                        "SQLITE_BUSY leaked to a caller: {}",
                        err
                    );
                }
                result.unwrap_or_else(|e| panic!("writer {} row {} failed: {}", writer, row, e));
            }
        }));
    }

    for handle in handles {
        handle.join().expect("writer thread panicked");
    }

    let check = open_configured_connection(&db_path).expect("check connection");
    let count: i64 = check
        .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
            row.get(0)
        })
        .expect("count rows");
    assert_eq!(
        count as usize,
        WRITERS * ROWS_PER_WRITER,
        "every concurrent write must be durable"
    );
}

/// G3 (reader-during-writer): with WAL, readers are never blocked by the
/// writer's open transaction. A reader must be able to query consistently
/// while uncommitted writes are in flight.
#[test]
fn reader_can_query_while_writer_holds_transaction() {
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("history.db");

    {
        let setup = open_configured_connection(&db_path).expect("setup connection");
        setup
            .execute_batch(HISTORY_TABLE_DDL)
            .expect("create table");
    }

    let writer = open_configured_connection(&db_path).expect("writer connection");
    writer.execute_batch("BEGIN IMMEDIATE").expect("begin txn");

    // Stage 50 rows inside the uncommitted transaction.
    for row in 0..50 {
        insert_entry(&writer, &format!("staged-{}.wav", row)).expect("stage row");
    }

    // Reader on an independent connection: must not block or fail even
    // though the writer holds the write lock with pending changes.
    let reader = open_configured_connection(&db_path).expect("reader connection");
    let started = Instant::now();
    let visible: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE file_name LIKE 'staged-%'",
            [],
            |row| row.get(0),
        )
        .expect("read while writer holds transaction");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "reader blocked behind the writer; WAL is not in effect"
    );
    assert_eq!(
        visible, 0,
        "uncommitted writes must be invisible to other connections"
    );

    // After commit, the same reader sees all staged rows.
    writer.execute_batch("COMMIT").expect("commit txn");
    let after_commit: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE file_name LIKE 'staged-%'",
            [],
            |row| row.get(0),
        )
        .expect("read after commit");
    assert_eq!(after_commit, 50);
}

/// G3 (writer-during-writer): when two writers collide, the loser must wait
/// out the lock via busy_timeout and then succeed — instead of surfacing
/// SQLITE_BUSY immediately.
#[test]
fn contending_writer_waits_out_lock_instead_of_failing() {
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let db_path = Arc::new(tmp.path().join("history.db"));

    {
        let setup = open_configured_connection(&db_path).expect("setup connection");
        setup
            .execute_batch(HISTORY_TABLE_DDL)
            .expect("create table");
    }

    let holder = open_configured_connection(&db_path).expect("holder connection");
    holder.execute_batch("BEGIN IMMEDIATE").expect("begin txn");

    let db_path_for_writer = Arc::clone(&db_path);
    let contender = thread::spawn(move || {
        let conn = open_configured_connection(&db_path_for_writer).expect("contender connection");
        // This would fail instantly without busy_timeout; with it, SQLite
        // retries until the holder commits.
        conn.execute_batch("BEGIN IMMEDIATE; COMMIT;")
            .expect("contending writer should acquire the lock after the holder commits");
    });

    // Give the contender time to hit the held lock before releasing it.
    thread::sleep(Duration::from_millis(200));
    holder.execute_batch("COMMIT").expect("holder commit");

    contender.join().expect("contender thread panicked");
}
