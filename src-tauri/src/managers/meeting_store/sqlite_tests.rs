//! SQLite-backed tests for [`super`] — migrations, transactions and the
//! concrete store's behaviour against a real database.
//!
//! Split out alongside `tests.rs` so meeting_store.rs holds implementation.

use super::*;
use rusqlite_migration::Migrations;
use std::path::PathBuf;

// -- fixtures ----------------------------------------------------------

fn temp_store() -> (tempfile::TempDir, SqliteMeetingStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store =
        SqliteMeetingStore::open(dir.path().join(MEETING_DB_FILE)).expect("open meeting store");
    (dir, store)
}

fn draft_session(title: &str) -> NewMeetingSession {
    NewMeetingSession {
        title: title.to_string(),
        started_at: Utc::now(),
        stt_provider: "local".to_string(),
        provider_config_snapshot: serde_json::json!({ "model": "parakeet" }),
        meeting_model_id: Some("parakeet-v3".to_string()),
        language: Some("de".to_string()),
        microphone_device: Some("mic-0".to_string()),
        output_device: None,
        has_microphone_audio: true,
        has_system_audio: false,
        recording_retention_policy: "keep".to_string(),
        summary_provider: Some("local-llm".to_string()),
    }
}

fn draft_chunk(meeting: &Uuid, sequence: i64) -> NewAudioChunk {
    NewAudioChunk {
        meeting_id: meeting.clone(),
        sequence,
        source: "microphone".to_string(),
        timeline_start_ms: sequence * 60_000,
        timeline_end_ms: (sequence + 1) * 60_000,
        storage_key: format!("meetings/{}/chunks/{sequence:05}.wav", meeting.0),
        sample_rate: 16_000,
        sample_count: 0,
        device_id: Some("mic-0".to_string()),
    }
}

fn draft_job(meeting: &Uuid, chunks: Vec<Uuid>) -> NewSttJob {
    NewSttJob {
        meeting_id: meeting.clone(),
        chunk_ids: chunks,
        source: "microphone".to_string(),
        timeline_start_ms: 0,
        timeline_end_ms: 60_000,
        provider: "local".to_string(),
        config_snapshot: serde_json::json!({ "beam": 5 }),
        next_attempt_at: None,
    }
}

/// Plan §6.19: a file only counts as understood once a model actually read
/// it, and the result must be searchable like any other material.
#[test]
fn recording_an_analysis_marks_the_asset_and_makes_it_searchable() {
    let (_dir, store) = temp_store();
    let (source, _) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Chat".into(),
            parser_version: "v1".into(),
            content_hash: "hash-media".into(),
            messages: vec![(Some("Ana".into()), "Bild folgt".into(), false)],
        })
        .expect("source");
    let source_id = Uuid(source.id.clone());
    let assets = store
        .attach_source_assets(
            source_id.clone(),
            vec![NewSourceAsset {
                storage_key: "assets/asst-1/original.jpg".into(),
                original_name: "vertrag.jpg".into(),
                mime: "image/jpeg".into(),
                size_bytes: 1_024,
                content_hash: "media-hash".into(),
            }],
        )
        .expect("attach");
    assert_eq!(assets[0].analysis_state, "not_analysed");

    store
        .record_asset_analysis(Uuid(assets[0].id.clone()), "Ein unterschriebener Vertrag")
        .expect("record");

    let after = store.list_source_assets(source_id.clone()).expect("list");
    assert_eq!(after[0].analysis_state, "analysed");

    store.reindex_search().expect("reindex");
    let hits = store.search("unterschriebener", None, 10).expect("search");
    assert_eq!(hits.len(), 1, "the description must be findable");
    assert_eq!(hits[0].entity_kind, "source_item");
}

/// Re-running the same analysis must not add a second description.
#[test]
fn recording_the_same_analysis_twice_adds_one_item() {
    let (_dir, store) = temp_store();
    let (source, _) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Chat".into(),
            parser_version: "v1".into(),
            content_hash: "hash-media-2".into(),
            messages: vec![(Some("Ana".into()), "Bild".into(), false)],
        })
        .expect("source");
    let source_id = Uuid(source.id.clone());
    let assets = store
        .attach_source_assets(
            source_id.clone(),
            vec![NewSourceAsset {
                storage_key: "assets/asst-2/original.png".into(),
                original_name: "plan.png".into(),
                mime: "image/png".into(),
                size_bytes: 10,
                content_hash: "media-hash-2".into(),
            }],
        )
        .expect("attach");
    let asset_id = Uuid(assets[0].id.clone());

    store
        .record_asset_analysis(asset_id.clone(), "Ein Grundriss")
        .expect("first");
    store
        .record_asset_analysis(asset_id, "Ein Grundriss")
        .expect("second");

    let items = store.list_source_items(source_id).expect("items");
    assert_eq!(
        items
            .iter()
            .filter(|i| i.text.starts_with("plan.png"))
            .count(),
        1
    );
}

/// An unknown asset must fail loudly rather than write a description that
/// belongs to nothing.
#[test]
fn recording_an_analysis_for_an_unknown_asset_fails() {
    let (_dir, store) = temp_store();
    assert!(store
        .record_asset_analysis(Uuid("missing".into()), "irgendwas")
        .is_err());
}

// -- evidence and search (Phase 10) ------------------------------------

fn evidence(quote: &str) -> NewEvidence {
    NewEvidence {
        meeting_id: Some("m-1".to_string()),
        segment_start_ms: Some(1_000),
        segment_end_ms: Some(2_000),
        quote: quote.to_string(),
        ..NewEvidence::default()
    }
}

fn task_draft(project: &Uuid, title: &str) -> NewProjectTask {
    NewProjectTask {
        project_id: project.clone(),
        title: title.to_string(),
        description: String::new(),
        origin: "ai",
    }
}

/// The plan's central rule: an AI suggestion without a citation is not
/// published at all. Enforcing it in the store means no caller can forget.
#[test]
fn an_ai_task_without_evidence_is_never_written() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "BDNA".into(),
            description: String::new(),
        })
        .expect("project");
    let id = Uuid(project.id.clone());

    let result = store
        .suggest_task_with_evidence(task_draft(&id, "Angebot senden"), Vec::new())
        .expect("call succeeds");

    assert!(result.is_none(), "no evidence means no task");
    assert!(
        store.list_tasks(id).expect("list").is_empty(),
        "nothing may have been written"
    );
}

#[test]
fn an_evidenced_task_is_published_with_its_citations() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "BDNA".into(),
            description: String::new(),
        })
        .expect("project");
    let id = Uuid(project.id.clone());

    let (task, created) = store
        .suggest_task_with_evidence(
            task_draft(&id, "Angebot senden"),
            vec![evidence("wir schicken das Angebot bis Freitag")],
        )
        .expect("call")
        .expect("task published");
    assert!(created);

    let cited = store
        .list_task_evidence(Uuid(task.id.clone()))
        .expect("evidence");
    assert_eq!(cited.len(), 1);
    assert_eq!(cited[0].meeting_id.as_deref(), Some("m-1"));
    assert_eq!(cited[0].segment_start_ms, Some(1_000));
}

/// Re-running an analysis must not pile a second set of citations onto a
/// task the user may already have accepted.
#[test]
fn re_running_the_analysis_does_not_duplicate_citations() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "BDNA".into(),
            description: String::new(),
        })
        .expect("project");
    let id = Uuid(project.id.clone());

    let (first, created) = store
        .suggest_task_with_evidence(task_draft(&id, "Angebot senden"), vec![evidence("a")])
        .expect("call")
        .expect("published");
    assert!(created);

    let (again, created_again) = store
        .suggest_task_with_evidence(task_draft(&id, "Angebot senden"), vec![evidence("a")])
        .expect("call")
        .expect("published");

    assert_eq!(first.id, again.id, "the same suggestion is one task");
    assert!(!created_again);
    assert_eq!(
        store
            .list_task_evidence(Uuid(first.id))
            .expect("evidence")
            .len(),
        1,
        "citations must not accumulate on repeat runs"
    );
    assert_eq!(store.list_tasks(id).expect("list").len(), 1);
}

#[test]
fn a_snapshot_without_evidence_is_never_written() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "BDNA".into(),
            description: String::new(),
        })
        .expect("project");
    let id = Uuid(project.id.clone());

    assert!(store
        .create_snapshot_with_evidence(
            id.clone(),
            "Status".into(),
            "v1".into(),
            None,
            "marker".into(),
            Vec::new(),
        )
        .expect("call")
        .is_none());
    assert!(store.list_snapshots(id).expect("list").is_empty());
}

/// A source may belong to several projects without being copied (plan §9).
#[test]
fn a_source_can_belong_to_several_projects_without_duplication() {
    let (_dir, store) = temp_store();
    let alpha = store
        .create_project(NewProject {
            name: "Alpha".into(),
            description: String::new(),
        })
        .expect("alpha");
    let beta = store
        .create_project(NewProject {
            name: "Beta".into(),
            description: String::new(),
        })
        .expect("beta");
    let (source, _) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Chat".into(),
            parser_version: "v1".into(),
            content_hash: "hash-1".into(),
            messages: vec![(Some("Ana".into()), "Hallo".into(), false)],
        })
        .expect("source");
    let source_id = Uuid(source.id.clone());

    store
        .link_source_to_project(Uuid(alpha.id.clone()), source_id.clone())
        .expect("link alpha");
    store
        .link_source_to_project(Uuid(beta.id.clone()), source_id.clone())
        .expect("link beta");
    // Linking twice must stay a no-op rather than fail.
    store
        .link_source_to_project(Uuid(alpha.id.clone()), source_id.clone())
        .expect("relink alpha");

    assert_eq!(
        store
            .list_source_ids_for_project(Uuid(alpha.id.clone()))
            .expect("alpha"),
        vec![source.id.clone()]
    );
    assert_eq!(
        store
            .list_source_ids_for_project(Uuid(beta.id.clone()))
            .expect("beta"),
        vec![source.id.clone()]
    );

    store
        .unlink_source_from_project(Uuid(alpha.id.clone()), source_id)
        .expect("unlink");
    assert!(store
        .list_source_ids_for_project(Uuid(alpha.id))
        .expect("alpha after unlink")
        .is_empty());
    assert_eq!(
        store
            .list_source_ids_for_project(Uuid(beta.id))
            .expect("beta unaffected")
            .len(),
        1,
        "unlinking one project must not affect the other"
    );
}

/// Search must find chat text and name where the hit came from.
#[test]
fn search_finds_imported_chat_text_and_names_its_origin() {
    let (_dir, store) = temp_store();
    let (source, _) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Chat".into(),
            parser_version: "v1".into(),
            content_hash: "hash-2".into(),
            messages: vec![
                (
                    Some("Ana".into()),
                    "Der Vertrag liegt beim Notar".into(),
                    false,
                ),
                (Some("Ben".into()), "Alles klar".into(), false),
            ],
        })
        .expect("source");

    store.reindex_search().expect("reindex");
    let hits = store.search("Notar", None, 10).expect("search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, "source_item");
    assert!(hits[0].snippet.contains("Notar"));
    let items = store.list_source_items(Uuid(source.id)).expect("items");
    assert!(
        items.iter().any(|i| i.id == hits[0].entity_id),
        "a hit must point at a real item"
    );
}

/// Free text is not FTS5 syntax: a query full of operators must return no
/// results rather than raise an error the user cannot act on.
#[test]
fn punctuation_heavy_queries_never_error() {
    let (_dir, store) = temp_store();
    store.reindex_search().expect("reindex");

    for query in ["", "   ", "-", "\"", "NEAR(", "a:b", "***"] {
        let hits = store.search(query, None, 10);
        assert!(hits.is_ok(), "query {query:?} must not error");
    }
}

#[test]
fn a_prefix_query_matches_as_the_user_types() {
    let (_dir, store) = temp_store();
    store
        .create_project(NewProject {
            name: "Kundenprojekt".into(),
            description: "Angebotsphase".into(),
        })
        .expect("project");
    store.reindex_search().expect("reindex");

    let hits = store.search("Kunden", None, 10).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, "project");
}

/// Restricting to a project must not leak another project's content.
#[test]
fn search_can_be_narrowed_to_one_project() {
    let (_dir, store) = temp_store();
    let alpha = store
        .create_project(NewProject {
            name: "Alpha Angebot".into(),
            description: String::new(),
        })
        .expect("alpha");
    store
        .create_project(NewProject {
            name: "Beta Angebot".into(),
            description: String::new(),
        })
        .expect("beta");
    store.reindex_search().expect("reindex");

    assert_eq!(store.search("Angebot", None, 10).expect("all").len(), 2);
    let narrowed = store
        .search("Angebot", Some(Uuid(alpha.id.clone())), 10)
        .expect("narrowed");
    assert_eq!(narrowed.len(), 1);
    assert_eq!(narrowed[0].entity_id, alpha.id);
}

/// Reindexing is idempotent: running it twice must not double every hit.
#[test]
fn reindexing_replaces_rather_than_appends() {
    let (_dir, store) = temp_store();
    store
        .create_project(NewProject {
            name: "Einmalig".into(),
            description: String::new(),
        })
        .expect("project");

    let first = store.reindex_search().expect("first");
    let second = store.reindex_search().expect("second");
    assert_eq!(first, second);
    assert_eq!(store.search("Einmalig", None, 10).expect("search").len(), 1);
}

/// Regression: a chunk is inserted empty (`timeline_end_ms` ==
/// `timeline_start_ms`) and only its finalized sample count says how far
/// along the timeline it reaches. Finalisation used to leave the end at
/// the insert value, so every chunk stayed zero-length and every STT job
/// range and transcript timestamp derived from it collapsed onto the chunk
/// start (observed in the field: all segments reported start == end == 0).
#[test]
fn finalize_chunk_derives_the_timeline_end_from_the_sample_count() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Timeline"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    let mut draft = draft_chunk(&meeting_id, 1);
    draft.timeline_start_ms = 60_000;
    draft.timeline_end_ms = 60_000;
    let chunk = store.append_chunk(draft).expect("chunk");

    // 16 kHz mono, 48 000 samples = exactly 3 s of audio.
    let finalized = store
        .finalize_chunk(
            chunk.envelope.id.clone(),
            ChunkFacts {
                file_size: 48_000 * 2 + 44,
                sample_count: 48_000,
                checksum: "a".repeat(64),
                gap_drift_metadata: None,
            },
        )
        .expect("finalize");

    assert_eq!(finalized.timeline_start_ms, 60_000);
    assert_eq!(
        finalized.timeline_end_ms, 63_000,
        "a 3 s chunk starting at 60 s must end at 63 s, not at its start"
    );
}

/// Regression: `has_microphone_audio` / `has_system_audio` were written
/// once at session creation and never again, so a session always claimed
/// it had captured nothing and the mic-only badge could not render.
#[test]
fn finalizing_a_chunk_records_which_sources_produced_audio() {
    let (_dir, store) = temp_store();
    let mut draft = draft_session("Sources");
    draft.has_microphone_audio = false;
    draft.has_system_audio = false;
    let session = store.create_session(draft).expect("session");
    let meeting_id = session.envelope.id.clone();

    let mut system_chunk = draft_chunk(&meeting_id, 0);
    system_chunk.source = "system".to_string();
    let chunk = store.append_chunk(system_chunk).expect("chunk");
    store
        .finalize_chunk(
            chunk.envelope.id,
            ChunkFacts {
                file_size: 1_644,
                sample_count: 800,
                checksum: "b".repeat(64),
                gap_drift_metadata: None,
            },
        )
        .expect("finalize");

    let read = store.get_session(meeting_id).expect("read").expect("row");
    assert!(
        read.has_system_audio,
        "a finalized system chunk proves system audio was captured"
    );
    assert!(
        !read.has_microphone_audio,
        "no microphone chunk was finalized, so the flag must stay false"
    );
}

/// Regression: the recorder's per-chunk enqueue and the stop path's sweep
/// pre-check coverage on a different connection, so their checks can
/// interleave. Three chunks in a real recording ended up with two jobs
/// created in the same millisecond, and every one of their segments landed
/// in the transcript twice.
#[test]
fn enqueueing_a_covered_chunk_returns_the_existing_job() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Race"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    let chunk = store
        .append_chunk(draft_chunk(&meeting_id, 0))
        .expect("chunk");

    let first = store
        .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id.clone()]))
        .expect("first enqueue");
    let second = store
        .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id]))
        .expect("second enqueue");

    assert_eq!(first.id, second.id, "the losing caller must be a no-op");
    let jobs = store.due_stt_jobs(Utc::now(), 10).expect("due jobs");
    assert_eq!(jobs.len(), 1, "exactly one job may own a chunk");
}

/// A session plus one finalized chunk plus one claimed-ready job — the
/// minimum context most write units need.
fn seeded(store: &SqliteMeetingStore) -> (Uuid, Uuid, Uuid) {
    let session = store
        .create_session(draft_session("Seeded"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    let chunk = store
        .append_chunk(draft_chunk(&meeting_id, 0))
        .expect("chunk");
    let job = store
        .enqueue_stt_job(draft_job(&meeting_id, vec![chunk.envelope.id.clone()]))
        .expect("job");
    (meeting_id, chunk.envelope.id, job.id)
}

/// Full DDL fingerprint of the database, used to prove that a second
/// migration run changes nothing.
fn schema_fingerprint(path: &PathBuf) -> (i64, Vec<(String, String)>) {
    let conn = Connection::open(path).expect("open raw");
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    let mut stmt = conn
        .prepare(
            "SELECT name, COALESCE(sql, '') FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .expect("prepare");
    let objects = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .expect("collect");
    (user_version, objects)
}

fn count(store: &SqliteMeetingStore, sql: &str) -> i64 {
    let conn = store.conn.lock().expect("lock");
    conn.query_row(sql, [], |row| row.get(0)).expect("count")
}

// -- migrations --------------------------------------------------------

/// Running the chain twice is a no-op: same `user_version`, byte-identical
/// set of tables and indexes.
#[test]
fn migrations_are_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(MEETING_DB_FILE);

    drop(SqliteMeetingStore::open(path.clone()).expect("first open"));
    let first = schema_fingerprint(&path);

    drop(SqliteMeetingStore::open(path.clone()).expect("second open"));
    let second = schema_fingerprint(&path);

    assert_eq!(first, second, "second migration run changed the schema");
    assert_eq!(
        first.0,
        MIGRATIONS.len() as i64,
        "user_version tracks the chain length"
    );
    let names: Vec<&str> = first.1.iter().map(|(n, _)| n.as_str()).collect();
    for table in [
        "meeting_sessions",
        "meeting_audio_chunks",
        "meeting_stt_jobs",
        "meeting_stt_job_chunks",
        "meeting_transcript_segments",
        "meeting_summaries",
        "change_log",
    ] {
        assert!(names.contains(&table), "missing table {table}: {names:?}");
    }
}

/// The regression that matters for shipped installations: a
/// `meeting_sessions` row written by the wave-1 schema (chain step 1 only)
/// must come back unchanged after the new steps 2–6 are applied.
#[test]
fn legacy_session_row_survives_new_migrations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(MEETING_DB_FILE);
    let legacy_id = "11111111-1111-4111-8111-111111111111";

    {
        // Exactly the old database: step 1 of the chain, nothing else.
        let mut conn = Connection::open(&path).expect("open legacy");
        conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
        Migrations::new(vec![MIGRATIONS[0].clone()])
            .to_latest(&mut conn)
            .expect("legacy migration");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, 1, "fixture must sit on the wave-1 schema");

        conn.execute(
            "INSERT INTO meeting_sessions (id, title, state, started_at, created_at, updated_at)
             VALUES (?1, 'Legacy kickoff', 'draft', 1700000000000, 1700000000000, 1700000000000)",
            params![legacy_id],
        )
        .expect("insert legacy row");
    }

    // Upgrade through the full chain.
    let store = SqliteMeetingStore::open(path.clone()).expect("upgrade open");
    let (version, _) = schema_fingerprint(&path);
    assert_eq!(version, MIGRATIONS.len() as i64);

    let session = store
        .get_session(Uuid(legacy_id.to_string()))
        .expect("get_session")
        .expect("legacy row still present");
    assert_eq!(session.title, "Legacy kickoff");
    assert_eq!(session.state, "draft");
    assert_eq!(
        session.envelope.revision, 1,
        "migration must not bump revisions"
    );
    assert_eq!(
        session.envelope.updated_at.timestamp_millis(),
        1_700_000_000_000
    );
    assert!(session.envelope.deleted_at.is_none());
    // Column defaults from the wave-1 DDL survive too.
    assert_eq!(session.stt_provider, "local");
    assert_eq!(session.summary_state, "none");
    assert_eq!(session.provider_config_snapshot, serde_json::json!({}));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM meeting_sessions"), 1);
}

/// `MeetingManager` and the store share one file and one chain, in either
/// opening order.
#[test]
fn manager_and_store_share_one_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(MEETING_DB_FILE);

    let manager = crate::managers::meeting::MeetingManager::open(path.clone())
        .expect("manager opens the migrated db");
    let created = manager
        .create_session(Some("Shared".into()))
        .expect("manager writes");

    let store = SqliteMeetingStore::open(path).expect("store opens the same db");
    let seen = store
        .get_session(Uuid(created.id.clone()))
        .expect("store reads")
        .expect("row visible through the seam");
    assert_eq!(seen.title, "Shared");
}

// -- sessions ----------------------------------------------------------

#[test]
fn create_session_round_trips_and_logs_one_create_row() {
    let (_dir, store) = temp_store();
    let created = store
        .create_session(draft_session("Kickoff"))
        .expect("create");

    assert_eq!(created.envelope.revision, 1);
    assert!(created.envelope.deleted_at.is_none());
    assert!(!created.envelope.deleted());

    let read = store
        .get_session(created.envelope.id.clone())
        .expect("get")
        .expect("present");
    assert_eq!(read.title, "Kickoff");
    assert_eq!(read.language.as_deref(), Some("de"));
    assert_eq!(read.microphone_device.as_deref(), Some("mic-0"));
    assert!(read.has_microphone_audio);
    assert!(!read.has_system_audio);
    assert_eq!(read.recording_retention_policy, "keep");
    assert_eq!(
        read.provider_config_snapshot,
        serde_json::json!({ "model": "parakeet" }),
        "JSON snapshot survives the text column round trip"
    );

    // W2: the change row is committed together with the data row.
    let logged = count(
        &store,
        "SELECT COUNT(*) FROM change_log
         WHERE entity = 'meeting_sessions' AND operation = 'create' AND revision = 1",
    );
    assert_eq!(logged, 1);
}

#[test]
fn update_session_bumps_revision_and_leaves_unpatched_columns_alone() {
    let (_dir, store) = temp_store();
    let created = store
        .create_session(draft_session("Before"))
        .expect("create");
    let id = created.envelope.id.clone();

    let updated = store
        .update_session(
            id.clone(),
            SessionPatch {
                title: Some("After".to_string()),
                state: Some("recording".to_string()),
                ..SessionPatch::default()
            },
        )
        .expect("update");

    assert_eq!(updated.title, "After");
    assert_eq!(updated.state, "recording");
    assert_eq!(updated.envelope.revision, 2, "W4: exactly one bump");
    assert_eq!(
        updated.language.as_deref(),
        Some("de"),
        "None patch fields must not clear columns"
    );
    assert_eq!(
        updated.created_at, created.created_at,
        "created_at is immutable"
    );

    assert!(store
        .update_session(Uuid("no-such-id".into()), SessionPatch::default())
        .is_err());
}

#[test]
fn soft_delete_session_tombstones_and_keeps_children() {
    let (_dir, store) = temp_store();
    let (meeting_id, chunk_id, job_id) = seeded(&store);
    store
        .insert_segments(
            job_id,
            vec![NewTranscriptSegment {
                source: "microphone".to_string(),
                start_ms: 0,
                end_ms: 1_000,
                text: "hello".to_string(),
                word_timestamps: None,
                confidence: None,
                provider: "local".to_string(),
                model_id: None,
                sequence: 0,
            }],
        )
        .expect("segments");

    store
        .soft_delete_session(meeting_id.clone())
        .expect("soft delete");

    // Invisible to reads...
    assert!(store
        .get_session(meeting_id.clone())
        .expect("get")
        .is_none());
    assert!(store
        .list_sessions(Page {
            offset: 0,
            limit: 10
        })
        .expect("list")
        .items
        .is_empty());

    // ...but the row and every child row are still there: v1 has no hard
    // delete for tier A, so the FK cascades never fire (contract §5.4).
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM meeting_sessions WHERE deleted_at IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM meeting_audio_chunks"),
        1
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM meeting_transcript_segments"),
        1
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM change_log
             WHERE entity = 'meeting_sessions' AND operation = 'tombstone'"
        ),
        1
    );
    // Deleting twice is a NotFound, not a silent second tombstone.
    assert!(store.soft_delete_session(meeting_id).is_err());
    let _ = chunk_id;
}

#[test]
fn list_sessions_paginates_newest_first() {
    let (_dir, store) = temp_store();
    for n in 0..3 {
        store
            .create_session(draft_session(&format!("S{n}")))
            .expect("create");
        std::thread::sleep(std::time::Duration::from_millis(3));
    }

    let page = store
        .list_sessions(Page {
            offset: 0,
            limit: 2,
        })
        .expect("page 1");
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].title, "S2", "newest first");

    let rest = store
        .list_sessions(Page {
            offset: 2,
            limit: 2,
        })
        .expect("page 2");
    assert_eq!(rest.items.len(), 1);
    assert_eq!(rest.items[0].title, "S0");
}

// -- audio chunks ------------------------------------------------------

#[test]
fn chunk_insert_and_finalize_round_trip() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Capture"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();

    let chunk = store
        .append_chunk(draft_chunk(&meeting_id, 0))
        .expect("append");
    assert_eq!(chunk.state, ChunkState::Writing);
    assert_eq!(chunk.envelope.revision, 1);
    assert_eq!(chunk.sample_rate, 16_000);
    assert_eq!(chunk.file_size, 0);
    assert!(chunk.checksum.is_none());
    assert!(
        !chunk.storage_key.starts_with('/') && !chunk.storage_key.contains(':'),
        "storage keys stay relative (plan lines 1158-1159): {}",
        chunk.storage_key
    );

    let checksum = "a".repeat(64);
    let finalized = store
        .finalize_chunk(
            chunk.envelope.id.clone(),
            ChunkFacts {
                file_size: 320_044,
                sample_count: 160_000,
                checksum: checksum.clone(),
                gap_drift_metadata: Some(serde_json::json!({ "drift_ms": 4 })),
            },
        )
        .expect("finalize");

    assert_eq!(finalized.state, ChunkState::Ready);
    assert_eq!(finalized.envelope.revision, 2);
    assert_eq!(finalized.file_size, 320_044);
    assert_eq!(finalized.sample_count, 160_000);
    assert_eq!(finalized.checksum.as_deref(), Some(checksum.as_str()));
    assert_eq!(
        finalized.gap_drift_metadata,
        Some(serde_json::json!({ "drift_ms": 4 }))
    );

    // Finalizing twice would rewrite the damage-detection checksum.
    assert!(store
        .finalize_chunk(
            chunk.envelope.id,
            ChunkFacts {
                file_size: 1,
                sample_count: 1,
                checksum: "b".repeat(64),
                gap_drift_metadata: None,
            },
        )
        .is_err());
}

#[test]
fn mark_chunk_damaged_keeps_row_alive_and_deleted_tombstones() {
    let (_dir, store) = temp_store();
    let (meeting_id, _, _) = seeded(&store);

    let damaged = store
        .append_chunk(draft_chunk(&meeting_id, 1))
        .expect("chunk 1");
    store
        .mark_chunk(
            damaged.envelope.id.clone(),
            ChunkState::Damaged,
            Some("short read at tail".to_string()),
        )
        .expect("mark damaged");

    let removed = store
        .append_chunk(draft_chunk(&meeting_id, 2))
        .expect("chunk 2");
    store
        .mark_chunk(removed.envelope.id.clone(), ChunkState::Deleted, None)
        .expect("mark deleted");

    let conn = store.conn.lock().expect("lock");
    let damaged_row = read_chunk(&conn, &damaged.envelope.id.0)
        .expect("read")
        .expect("present");
    assert_eq!(damaged_row.state, ChunkState::Damaged);
    assert!(
        !damaged_row.envelope.deleted(),
        "damaged is a quality verdict, not a deletion"
    );
    assert_eq!(damaged_row.envelope.revision, 2);

    let deleted_row = read_chunk(&conn, &removed.envelope.id.0)
        .expect("read")
        .expect("row is kept, only the content goes");
    assert_eq!(deleted_row.state, ChunkState::Deleted);
    assert!(deleted_row.envelope.deleted());
    drop(conn);

    // U7 covers damaged/deleted only.
    assert!(store
        .mark_chunk(damaged.envelope.id, ChunkState::Ready, None)
        .is_err());
}

/// Foreign keys are enforced on every connection: a chunk for a session
/// that does not exist is rejected rather than orphaned.
#[test]
fn foreign_keys_reject_orphan_rows() {
    let (_dir, store) = temp_store();
    let orphan = draft_chunk(&Uuid("00000000-0000-4000-8000-0000000000ff".into()), 0);
    let err = store.append_chunk(orphan).expect_err("must be rejected");
    assert!(
        matches!(err, StoreError::Conflict { .. }),
        "expected a Conflict, got {err:?}"
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM meeting_audio_chunks"),
        0
    );
}

// -- STT jobs ----------------------------------------------------------

#[test]
fn stt_job_round_trips_with_its_chunk_references() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Jobs"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    let first = store.append_chunk(draft_chunk(&meeting_id, 0)).expect("c0");
    let second = store.append_chunk(draft_chunk(&meeting_id, 1)).expect("c1");

    let job = store
        .enqueue_stt_job(draft_job(
            &meeting_id,
            vec![first.envelope.id.clone(), second.envelope.id.clone()],
        ))
        .expect("enqueue");

    assert_eq!(job.state, SttJobState::Queued);
    assert_eq!(job.attempt_count, 0);
    assert_eq!(job.chunk_ids, vec![first.envelope.id, second.envelope.id]);
    assert_eq!(job.config_snapshot, serde_json::json!({ "beam": 5 }));

    let due = store.due_stt_jobs(Utc::now(), 10).expect("due");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, job.id);
    assert_eq!(due[0].chunk_ids.len(), 2);

    // Tier B: job churn never reaches the change log (rule W6).
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM change_log WHERE entity = 'meeting_stt_jobs'"
        ),
        0
    );
}

/// U4's core guarantee: one job goes to exactly one worker.
#[test]
fn claim_hands_each_job_to_exactly_one_worker() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Queue"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    // Two distinct chunks: one chunk maps to exactly one job, enforced by
    // `idx_meeting_stt_job_chunks_unique`, so a second enqueue for the same
    // chunk returns the first job instead of creating a rival.
    let chunk_a = store
        .append_chunk(draft_chunk(&meeting_id, 0))
        .expect("chunk a");
    let chunk_b = store
        .append_chunk(draft_chunk(&meeting_id, 1))
        .expect("chunk b");
    let job_a = store
        .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_a.envelope.id.clone()]))
        .expect("job a");
    let job_b = store
        .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_b.envelope.id]))
        .expect("job b");

    assert_eq!(
        store
            .enqueue_stt_job(draft_job(&meeting_id, vec![chunk_a.envelope.id]))
            .expect("re-enqueue")
            .id,
        job_a.id,
        "a chunk that is already covered must not spawn a second job"
    );

    let now = Utc::now();
    let first = store
        .claim_due_stt_job(WorkerId("worker-1".into()), now)
        .expect("claim 1")
        .expect("a job was due");
    let second = store
        .claim_due_stt_job(WorkerId("worker-2".into()), now)
        .expect("claim 2")
        .expect("the other job was due");

    assert_ne!(
        first.job.id, second.job.id,
        "the same job must never be handed to two workers"
    );
    let claimed = [first.job.id.clone(), second.job.id.clone()];
    assert!(claimed.contains(&job_a.id) && claimed.contains(&job_b.id));

    assert_eq!(first.job.state, SttJobState::Running);
    assert_eq!(first.job.attempt_count, 1, "claiming counts as an attempt");
    assert_eq!(first.worker, WorkerId("worker-1".into()));

    // Nothing is due any more: both jobs are running.
    assert!(store
        .claim_due_stt_job(WorkerId("worker-3".into()), now)
        .expect("claim 3")
        .is_none());
    assert!(store.due_stt_jobs(now, 10).expect("due").is_empty());
}

#[test]
fn claim_respects_next_attempt_at_backoff() {
    let (_dir, store) = temp_store();
    let session = store
        .create_session(draft_session("Backoff"))
        .expect("session");
    let meeting_id = session.envelope.id.clone();
    let chunk = store
        .append_chunk(draft_chunk(&meeting_id, 0))
        .expect("chunk");

    let now = Utc::now();
    let mut draft = draft_job(&meeting_id, vec![chunk.envelope.id]);
    draft.next_attempt_at = Some(now + chrono::Duration::seconds(30));
    store.enqueue_stt_job(draft).expect("enqueue");

    assert!(store
        .claim_due_stt_job(WorkerId("w".into()), now)
        .expect("claim")
        .is_none());
    assert!(store
        .claim_due_stt_job(WorkerId("w".into()), now + chrono::Duration::seconds(31))
        .expect("claim later")
        .is_some());
}

#[test]
fn transition_releases_the_lease_and_terminal_states_are_final() {
    let (_dir, store) = temp_store();
    let (_, _, job_id) = seeded(&store);
    let now = Utc::now();
    store
        .claim_due_stt_job(WorkerId("w".into()), now)
        .expect("claim")
        .expect("due");

    store
        .transition_stt_job(
            job_id.clone(),
            SttJobState::RetryWait,
            Some("timeout".into()),
        )
        .expect("retry_wait");
    let conn = store.conn.lock().expect("lock");
    let lease: Option<String> = conn
        .query_row(
            "SELECT claimed_by FROM meeting_stt_jobs WHERE id = ?1",
            params![job_id.0],
            |row| row.get(0),
        )
        .expect("lease");
    assert!(lease.is_none(), "leaving running releases the worker lease");
    drop(conn);

    // retry_wait is due again, so the job can be re-claimed.
    let again = store
        .claim_due_stt_job(WorkerId("w2".into()), now)
        .expect("reclaim")
        .expect("due again");
    assert_eq!(again.job.attempt_count, 2);

    store
        .transition_stt_job(job_id.clone(), SttJobState::Succeeded, None)
        .expect("succeed");
    let err = store
        .transition_stt_job(job_id, SttJobState::Queued, None)
        .expect_err("terminal states are final");
    assert!(matches!(err, StoreError::Conflict { .. }), "got {err:?}");
}

/// Crash-recovery guarantee (plan §13 Phase 6): re-running a job whose
/// segments already committed must not duplicate them.
#[test]
fn insert_segments_is_idempotent_for_an_already_persisted_job() {
    let (_dir, store) = temp_store();
    let (_meeting_id, _, job_id) = seeded(&store);

    let drafts = |text: &str| {
        vec![NewTranscriptSegment {
            source: "microphone".to_string(),
            start_ms: 0,
            end_ms: 900,
            text: text.to_string(),
            word_timestamps: None,
            confidence: Some(0.5),
            provider: "local".to_string(),
            model_id: Some("parakeet-v3".to_string()),
            sequence: 0,
        }]
    };

    let first = store
        .insert_segments(job_id.clone(), drafts("first attempt"))
        .expect("insert");
    assert_eq!(first.len(), 1);

    // A resumed job produces fresh engine output under the same job id.
    let again = store
        .insert_segments(job_id.clone(), drafts("second attempt"))
        .expect("re-insert");
    assert_eq!(again.len(), 1);
    assert_eq!(
        again[0].envelope.id, first[0].envelope.id,
        "the original row is returned untouched"
    );
    assert_eq!(again[0].text, "first attempt");
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM meeting_transcript_segments"),
        1
    );
}

// -- sources (Phase 9) ----------------------------------------------------

#[test]
fn repeated_whatsapp_import_creates_no_duplicates() {
    let (_dir, store) = temp_store();
    let draft = NewChatSource {
        source_type: "whatsapp_export".to_string(),
        title: "Team Chat".to_string(),
        parser_version: "wa-v1".to_string(),
        content_hash: "deadbeef".to_string(),
        messages: vec![
            (Some("Alice".to_string()), "Hallo".to_string(), false),
            (
                Some("Bob".to_string()),
                "<Medien ausgeschlossen>".to_string(),
                true,
            ),
        ],
    };

    let (first, created) = store.create_chat_source(draft).expect("import");
    assert!(created);
    let (second, created_again) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Team Chat (1)".to_string(),
            parser_version: "wa-v1".to_string(),
            content_hash: "deadbeef".to_string(),
            messages: vec![],
        })
        .expect("re-import");
    assert!(!created_again, "same hash must be a no-op");
    assert_eq!(first.id, second.id, "the original source is returned");

    // Distinct content still imports as a new source.
    let (_other, created_other) = store
        .create_chat_source(NewChatSource {
            source_type: "whatsapp_export".to_string(),
            title: "Anders".to_string(),
            parser_version: "wa-v1".to_string(),
            content_hash: "cafebabe".to_string(),
            messages: vec![],
        })
        .expect("other import");
    assert!(created_other);

    let items = store
        .list_source_items(Uuid(first.id.clone()))
        .expect("items");
    assert_eq!(items.len(), 2);
    assert!(items[1].is_media);
    assert_eq!(store.list_sources().expect("sources").len(), 2);
}

// -- tasks & snapshots (Phase 10) ------------------------------------------

#[test]
fn ai_suggestions_dedupe_and_never_overwrite_manual_edits() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "P".into(),
            description: String::new(),
        })
        .expect("project");
    let pid = Uuid(project.id.clone());

    let (first, created) = store
        .suggest_task(NewProjectTask {
            project_id: pid.clone(),
            title: "Protokoll senden".to_string(),
            description: "An alle Teilnehmer".to_string(),
            origin: "ai",
        })
        .expect("suggest");
    assert!(created);
    assert_eq!(first.state, "suggested");

    let edited = store
        .decide_task(
            Uuid(first.id.clone()),
            "edited",
            Some("Protokoll bis Freitag senden".to_string()),
            None,
        )
        .expect("edit");
    assert!(edited.human_edited_at.is_some());

    // A later AI run proposing the SAME task must not duplicate or
    // overwrite the user's edit.
    let (again, created_again) = store
        .suggest_task(NewProjectTask {
            project_id: pid.clone(),
            title: "Protokoll senden".to_string(),
            description: "An alle Teilnehmer".to_string(),
            origin: "ai",
        })
        .expect("re-suggest");
    assert!(!created_again);
    assert_eq!(again.id, first.id);
    assert_eq!(
        again.title, "Protokoll bis Freitag senden",
        "the manual edit survives"
    );

    let manual = store
        .create_manual_task(NewProjectTask {
            project_id: pid.clone(),
            title: "Raum buchen".to_string(),
            description: String::new(),
            origin: "manual",
        })
        .expect("manual task");
    assert_eq!(manual.state, "accepted");
    assert_eq!(store.list_tasks(pid).expect("tasks").len(), 2);
}

#[test]
fn snapshots_are_versioned_append_only() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "P".into(),
            description: String::new(),
        })
        .expect("project");
    let pid = Uuid(project.id.clone());

    let v1 = store
        .create_snapshot(
            pid.clone(),
            "Stand 1".into(),
            "pv1".into(),
            Some("m".into()),
            "r1".into(),
        )
        .expect("snapshot 1");
    let v2 = store
        .create_snapshot(
            pid.clone(),
            "Stand 2".into(),
            "pv1".into(),
            Some("m".into()),
            "r2".into(),
        )
        .expect("snapshot 2");
    assert_ne!(v1.id, v2.id, "snapshots are append-only versions");

    let listed = store.list_snapshots(pid).expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].summary_markdown, "Stand 2", "newest first");
}

fn mark_session_completed(store: &SqliteMeetingStore, id: &str) {
    let conn = store.locked("test_mark_session").expect("lock");
    conn.execute(
        "UPDATE meeting_sessions SET state = 'completed' WHERE id = ?1",
        params![id],
    )
    .expect("mark completed");
}

#[test]
fn project_crud_and_assignment_round_trip() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "Alpha".to_string(),
            description: "First".to_string(),
        })
        .expect("project");
    let session = store
        .create_session(draft_session("Sync"))
        .expect("session");

    // Unprocessed drafts never appear in the inbox.
    assert!(store
        .list_unassigned_meeting_ids()
        .expect("inbox")
        .is_empty());
    mark_session_completed(&store, &session.envelope.id.0);
    let project_id = Uuid(project.id.clone());
    let meeting_id = session.envelope.id.clone();

    store
        .assign_meeting_to_project(project_id.clone(), meeting_id.clone())
        .expect("assign");
    // Idempotent re-assignment.
    store
        .assign_meeting_to_project(project_id.clone(), meeting_id.clone())
        .expect("re-assign");

    assert_eq!(
        store
            .list_meeting_ids_for_project(project_id.clone())
            .expect("members"),
        vec![meeting_id.0.clone()]
    );
    assert!(store
        .list_unassigned_meeting_ids()
        .expect("inbox")
        .is_empty());

    store
        .unassign_meeting_from_project(project_id.clone(), meeting_id.clone())
        .expect("unassign");
    assert_eq!(
        store.list_unassigned_meeting_ids().expect("inbox"),
        vec![meeting_id.0.clone()],
        "unassigned meetings return to the Eingang"
    );

    let renamed = store
        .rename_project(project_id.clone(), "  Beta  ".to_string())
        .expect("rename");
    assert_eq!(renamed.name, "Beta");
    assert!(store
        .rename_project(project_id.clone(), "   ".to_string())
        .is_err());

    store.soft_delete_project(project_id).expect("delete");
    assert!(store.list_projects().expect("list").is_empty());
}

#[test]
fn deleted_project_releases_meetings_to_the_inbox() {
    let (_dir, store) = temp_store();
    let project = store
        .create_project(NewProject {
            name: "Temp".into(),
            description: String::new(),
        })
        .expect("project");
    let session = store.create_session(draft_session("Meeting")).expect("s");
    mark_session_completed(&store, &session.envelope.id.0);
    store
        .assign_meeting_to_project(Uuid(project.id.clone()), session.envelope.id.clone())
        .expect("assign");
    assert!(store
        .list_unassigned_meeting_ids()
        .expect("inbox")
        .is_empty());

    store.soft_delete_project(Uuid(project.id)).expect("delete");
    // Unlinking never deletes the meeting (plan §11).
    assert_eq!(
        store.list_unassigned_meeting_ids().expect("inbox"),
        vec![session.envelope.id.0.clone()]
    );
}

// -- transcript segments -----------------------------------------------

#[test]
fn segments_round_trip_and_come_back_in_chronological_order() {
    let (_dir, store) = temp_store();
    let (meeting_id, _, job_id) = seeded(&store);

    // Deliberately inserted out of order: the store orders by the audio
    // timeline, not by insertion order.
    let drafts = [(3_000_i64, "third"), (1_000, "first"), (2_000, "second")]
        .into_iter()
        .enumerate()
        .map(|(index, (start_ms, text))| NewTranscriptSegment {
            source: "microphone".to_string(),
            start_ms,
            end_ms: start_ms + 900,
            text: text.to_string(),
            word_timestamps: Some(serde_json::json!([{ "w": text, "t": start_ms }])),
            confidence: Some(0.75),
            provider: "local".to_string(),
            model_id: Some("parakeet-v3".to_string()),
            sequence: index as i64,
        })
        .collect::<Vec<_>>();

    let inserted = store
        .insert_segments(job_id.clone(), drafts)
        .expect("insert");
    assert_eq!(inserted.len(), 3);
    assert!(inserted.iter().all(|s| s.envelope.revision == 1));
    assert_eq!(
        inserted[0].meeting_id, meeting_id,
        "meeting comes from the job"
    );

    let read = store
        .segments_for_meeting(meeting_id.clone())
        .expect("segments");
    let texts: Vec<&str> = read.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, vec!["first", "second", "third"]);
    let starts: Vec<i64> = read.iter().map(|s| s.start_ms).collect();
    assert!(
        starts.windows(2).all(|w| w[0] <= w[1]),
        "not sorted: {starts:?}"
    );

    let first = &read[0];
    assert_eq!(first.job_id, job_id);
    assert_eq!(first.confidence, Some(0.75));
    assert_eq!(first.model_id.as_deref(), Some("parakeet-v3"));
    assert_eq!(
        first.word_timestamps,
        Some(serde_json::json!([{ "w": "first", "t": 1000 }]))
    );

    // U8: one log row per inserted segment, all in one transaction.
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM change_log
             WHERE entity = 'meeting_transcript_segments' AND operation = 'create'"
        ),
        3
    );

    // Unknown job: nothing is written.
    assert!(store
        .insert_segments(Uuid("nope".into()), Vec::new())
        .is_err());
}

// -- summaries ---------------------------------------------------------

#[test]
fn upsert_summary_inserts_then_updates_the_same_row() {
    let (_dir, store) = temp_store();
    let (meeting_id, _, _) = seeded(&store);

    let draft = NewMeetingSummary {
        meeting_id: meeting_id.clone(),
        provider: "local-llm".to_string(),
        model: Some("qwen3".to_string()),
        prompt_version: "v1".to_string(),
        state: "running".to_string(),
        summary_markdown: "## Draft".to_string(),
        decisions: serde_json::json!([]),
        action_items: serde_json::json!([]),
        open_questions: serde_json::json!([]),
        last_error: None,
    };

    let created = store.upsert_summary(draft.clone()).expect("insert");
    assert_eq!(created.envelope.revision, 1);
    assert_eq!(created.state, "running");

    let finished = store
        .upsert_summary(NewMeetingSummary {
            state: "completed".to_string(),
            summary_markdown: "## Result".to_string(),
            decisions: serde_json::json!([{ "text": "ship it" }]),
            action_items: serde_json::json!([{ "text": "write docs", "owner": "pat" }]),
            open_questions: serde_json::json!(["budget?"]),
            ..draft
        })
        .expect("update");

    assert_eq!(
        finished.envelope.id, created.envelope.id,
        "U9 updates the existing row instead of adding a second one"
    );
    assert_eq!(finished.envelope.revision, 2);
    assert_eq!(finished.summary_markdown, "## Result");
    assert_eq!(
        finished.decisions,
        serde_json::json!([{ "text": "ship it" }])
    );
    assert_eq!(finished.open_questions, serde_json::json!(["budget?"]));
    assert_eq!(finished.created_at, created.created_at);
    assert!(finished.updated_at >= finished.created_at);

    let read = store
        .summary_for_meeting(meeting_id)
        .expect("read")
        .expect("present");
    assert_eq!(read.envelope.id, created.envelope.id);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM meeting_summaries"), 1);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM change_log WHERE entity = 'meeting_summaries'"
        ),
        2,
        "one create plus one update row"
    );
}

#[test]
fn summary_is_absent_until_written() {
    let (_dir, store) = temp_store();
    let (meeting_id, _, _) = seeded(&store);
    assert!(store
        .summary_for_meeting(meeting_id)
        .expect("read")
        .is_none());
}

// -- injected ports ----------------------------------------------------

/// The store must take ids and timestamps from its ports, so tests (and a
/// later replay tool) can make writes deterministic (contract §5.1/§5.3).
#[test]
fn injected_clock_and_uuid_source_are_honoured() {
    struct FixedClock(DateTime<Utc>);
    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }
    struct FixedUuid;
    impl UuidSource for FixedUuid {
        fn new_uuid(&self) -> Uuid {
            Uuid("22222222-2222-4222-8222-222222222222".to_string())
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let fixed = from_ms(1_600_000_000_000);
    let store = SqliteMeetingStore::open_with_ports(
        dir.path().join(MEETING_DB_FILE),
        Arc::new(FixedClock(fixed)),
        Arc::new(FixedUuid),
    )
    .expect("open");

    let created = store
        .create_session(draft_session("Fixed"))
        .expect("create");
    assert_eq!(
        created.envelope.id.0,
        "22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(created.envelope.updated_at, fixed);
    assert_eq!(created.created_at, fixed);
}

/// The store is usable as the `Arc<dyn MeetingRepository>` that command
/// handlers and services will hold (contract §2.1 rule).
#[test]
fn store_is_usable_behind_the_trait_object() {
    let (_dir, store) = temp_store();
    let repo: Arc<dyn MeetingRepository> = Arc::new(store);
    let created = repo.create_session(draft_session("Dyn")).expect("create");
    assert_eq!(
        repo.get_session(created.envelope.id)
            .expect("get")
            .expect("present")
            .title,
        "Dyn"
    );
}
