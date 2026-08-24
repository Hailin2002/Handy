//! Append-only `change_log` writer — ownership-matrix row 16 (tier C).
//!
//! `docs/meeting-mode-architecture/repository-boundary-contract.md` §3 row 16
//! designates this file as the *only* code allowed to INSERT into
//! `change_log`, and §4 rules W2/W3 require every tier-A mutation to append its
//! log row **inside the same transaction** as the mutation itself. The helper
//! therefore takes a borrowed [`rusqlite::Transaction`] instead of opening its
//! own connection: a caller physically cannot commit the data change without
//! the log row (and vice versa).
//!
//! The entry type lives in [`super::knowledge_store::ChangeLogEntry`] (parked
//! there by the scaffold task); it is re-exported here so callers depend on the
//! helper rather than on the other store.
//!
//! Ordering is the SQLite rowid, not the fields (contract §3 note N5).
//! The log holds entity name, stable UUID, revision, operation and timestamp
//! only — never content, never secrets (plan §9 lines 1320–1325).

use rusqlite::{params, Transaction};

pub use super::knowledge_store::{ChangeLogEntry, ChangeOperation};

/// Persisted token for a [`ChangeOperation`] (closed enum; readers reject
/// unknown values per schema-versioning-policy UFR-2).
pub(crate) fn operation_token(operation: ChangeOperation) -> &'static str {
    match operation {
        ChangeOperation::Create => "create",
        ChangeOperation::Update => "update",
        ChangeOperation::Tombstone => "tombstone",
    }
}

/// Appends one change row inside the caller's transaction (W2/W3).
///
/// Timestamps are Unix epoch milliseconds, matching every other timestamp
/// column in `meetings.db`.
pub(crate) fn append(tx: &Transaction<'_>, entry: ChangeLogEntry) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO change_log (entity, entity_id, revision, operation, occurred_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            entry.entity,
            entry.entity_id.0,
            entry.revision,
            operation_token(entry.operation),
            entry.occurred_at.timestamp_millis(),
        ],
    )?;
    Ok(())
}
