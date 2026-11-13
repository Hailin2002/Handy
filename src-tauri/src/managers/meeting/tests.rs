//! Tests for [`super`] — meeting session lifecycle and orchestration.
//!
//! Extracted so the implementation file stays implementation. Same
//! arrangement as settings/tests.rs and meeting_store/.

use super::*;

#[test]
fn accuracy_overlap_counts_repeated_words_without_overmatching() {
    let (local, reference, overlap) =
        word_overlap("eins zwei zwei vier", "eins zwei drei zwei drei");
    assert_eq!((local, reference, overlap), (4, 5, 3));
}

#[test]
fn accuracy_rms_distinguishes_silence_from_speech() {
    assert_eq!(audio_rms(&[0.0; 32]), 0.0);
    assert!((audio_rms(&[0.5, -0.5]) - 0.5).abs() < f32::EPSILON);
}
use crate::managers::meeting_store::{
    NewProjectTask, NewTranscriptSegment, SttJobState, TaskRepository, Uuid, WorkerId,
};
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------- doubles

/// Records every publication so tests can assert ordering and revisions.
#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl RecordingSink {
    fn topics(&self, topic: &str) -> Vec<serde_json::Value> {
        lock(&self.events)
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

impl MeetingEventSink for RecordingSink {
    fn publish(&self, topic: &'static str, payload: serde_json::Value) {
        lock(&self.events).push((topic.to_string(), payload));
    }
}

/// Recorder double implementing the contract surface only.
struct FakeRecorder {
    state: rec::RecorderState,
    fail_open: bool,
    opened: Arc<AtomicBool>,
}

impl rec::MeetingRecorder for FakeRecorder {
    fn preflight(
        &self,
        _devices: &rec::SourceDeviceSet,
    ) -> Result<Vec<rec::PreflightCheck>, rec::RecorderError> {
        Ok(Vec::new())
    }

    fn start_session(
        &mut self,
        _layout: rec::SessionLayout,
        _store: Arc<dyn rec::ChunkStateStore>,
    ) -> Result<(), rec::RecorderError> {
        Ok(())
    }

    fn open_source(
        &mut self,
        _source: rec::SourceKind,
        _request: rec::SourceOpenRequest,
    ) -> Result<i64, rec::RecorderError> {
        if self.fail_open {
            return Err(rec::RecorderError::MicDeviceUnavailable {
                device_name: "test".into(),
                detail: "unplugged".into(),
            });
        }
        self.state = rec::RecorderState::Recording;
        self.opened.store(true, Ordering::SeqCst);
        Ok(1)
    }

    fn close_source(
        &mut self,
        _source: rec::SourceKind,
        _reason: rec::StopReason,
    ) -> Result<(), rec::RecorderError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<rec::TimelineMarker, rec::RecorderError> {
        self.state = rec::RecorderState::Paused;
        Ok(marker(rec::MarkerKind::Pause))
    }

    fn resume(
        &mut self,
        _devices: rec::SourceDeviceSet,
    ) -> Result<rec::TimelineMarker, rec::RecorderError> {
        self.state = rec::RecorderState::Recording;
        Ok(marker(rec::MarkerKind::Resume))
    }

    fn stop(&mut self) -> Result<rec::SessionSummary, rec::RecorderError> {
        self.state = rec::RecorderState::Idle;
        Ok(rec::SessionSummary::default())
    }

    fn subscribe(&mut self, _sink: Arc<dyn rec::RecorderEventSink>) {}

    fn snapshot(&self) -> rec::RecorderSnapshot {
        rec::RecorderSnapshot {
            state: self.state,
            timeline_now_ms: 0.0,
            sources: Vec::new(),
            markers: Vec::new(),
        }
    }

    fn recover_open_chunks(
        &self,
        _session_id: &str,
        _store: Arc<dyn rec::ChunkStateStore>,
    ) -> Result<rec::RecoveryReport, rec::RecorderError> {
        Ok(rec::RecoveryReport::default())
    }
}

fn marker(kind: rec::MarkerKind) -> rec::TimelineMarker {
    rec::TimelineMarker {
        kind,
        timeline_ms: 0.0,
        wall_clock: std::time::SystemTime::now(),
    }
}

fn fake_factory(fail_open: bool) -> RecorderFactory {
    Arc::new(move || {
        Box::new(FakeRecorder {
            state: rec::RecorderState::Idle,
            fail_open,
            opened: Arc::new(AtomicBool::new(false)),
        })
    })
}

fn healthy_env() -> StaticEnvironment {
    StaticEnvironment {
        devices: rec::SourceDeviceSet {
            microphone: Some(rec::SourceOpenRequest::new("Mic")),
            system: Some(rec::SourceOpenRequest::new("Speakers")),
        },
        checks: vec![
            rec::PreflightCheck {
                source: rec::SourceKind::Microphone,
                device_name: "Mic".into(),
                outcome: rec::PreflightOutcome::Available,
                native_sample_rate_hint: Some(48_000),
                detail: String::new(),
            },
            rec::PreflightCheck {
                source: rec::SourceKind::System,
                device_name: "Speakers".into(),
                outcome: rec::PreflightOutcome::Available,
                native_sample_rate_hint: Some(48_000),
                detail: String::new(),
            },
        ],
        free_bytes: Some(50_000_000_000),
        provider: ProviderSnapshot::default(),
        permission_denied: false,
        summary_consent: std::sync::atomic::AtomicBool::new(false),
        llm: std::sync::Mutex::new(None),
    }
}

struct Harness {
    dir: tempfile::TempDir,
    manager: MeetingManager,
    sink: Arc<RecordingSink>,
    db_path: PathBuf,
}

fn harness_with(env: StaticEnvironment, fail_open: bool) -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("meetings.db");
    let sink = Arc::new(RecordingSink::default());
    let manager = MeetingManager::open_with(
        db_path.clone(),
        dir.path().join("assets"),
        sink.clone(),
        Arc::new(env),
        fake_factory(fail_open),
    )
    .expect("open manager");
    Harness {
        dir,
        manager,
        sink,
        db_path,
    }
}

fn harness() -> Harness {
    harness_with(healthy_env(), false)
}

fn temp_manager() -> (tempfile::TempDir, MeetingManager) {
    let dir = tempfile::tempdir().expect("tempdir");
    let manager = MeetingManager::open(dir.path().join("meetings.db")).expect("open meeting db");
    (dir, manager)
}

// ------------------------------------------------------- wave-1 CRUD

/// Journey step 1+2: a created session can be opened again by id.
#[test]
fn create_then_open_returns_draft_session() {
    let (_dir, manager) = temp_manager();
    let before = chrono::Utc::now().timestamp_millis();
    let created = manager.create_session(Some("Kickoff".into())).unwrap();

    assert_eq!(created.title, "Kickoff");
    assert_eq!(created.state, SESSION_STATE_DRAFT);
    assert!(created.started_at >= before);

    let opened = manager.get_session(&created.id).unwrap().expect("found");
    assert_eq!(opened.id, created.id);
    assert_eq!(opened.title, "Kickoff");
}

/// Empty/absent titles fall back to a non-empty neutral title.
#[test]
fn create_with_blank_title_falls_back_to_neutral_title() {
    let (_dir, manager) = temp_manager();
    assert!(!manager
        .create_session(Some("   ".into()))
        .unwrap()
        .title
        .is_empty());
    assert!(!manager.create_session(None).unwrap().title.is_empty());
}

/// Journey step 3 (save): renaming persists across a close/reopen.
#[test]
fn rename_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("meetings.db");

    let created = {
        let manager = MeetingManager::open(db_path.clone()).unwrap();
        manager.create_session(Some("Before".into())).unwrap()
    };

    let reopened = MeetingManager::open(db_path.clone()).unwrap();
    let saved = reopened
        .rename_session(&created.id, "Real title".into())
        .unwrap();
    assert_eq!(saved.title, "Real title");

    let third = MeetingManager::open(db_path).unwrap();
    assert_eq!(
        third.get_session(&created.id).unwrap().unwrap().title,
        "Real title"
    );
}

/// Renaming rejects blank titles and unknown ids.
#[test]
fn rename_rejects_blank_title_and_unknown_id() {
    let (_dir, manager) = temp_manager();
    let created = manager.create_session(None).unwrap();
    assert!(manager.rename_session(&created.id, "  ".into()).is_err());
    assert!(manager
        .rename_session("00000000-0000-4000-8000-00000000dead", "x".into())
        .is_err());
}

/// Journey step 4 (list): newest first, query filters on the title.
#[test]
fn lists_newest_first_and_filters_by_query() {
    let (_dir, manager) = temp_manager();
    let alpha = manager.create_session(Some("Alpha Sync".into())).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let beta = manager
        .create_session(Some("Budget Review".into()))
        .unwrap();

    let all = manager.list_sessions(None).unwrap();
    let ids: Vec<_> = all.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![beta.id.as_str(), alpha.id.as_str()]);

    let filtered = manager
        .list_sessions(Some("budget".to_string()).as_deref())
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, beta.id);
}

/// Pen-test: hostile titles (empty, oversized) are rejected at the
/// boundary instead of flowing into the database and every event.
#[test]
fn titles_are_length_capped_and_non_empty() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();

    let long = "x".repeat(MAX_TITLE_CHARS + 1);
    assert!(h.manager.rename_session(&draft.meeting_id, long).is_err());
    assert!(h
        .manager
        .create_session(Some("y".repeat(MAX_TITLE_CHARS + 1)))
        .is_err());

    let ok = "x".repeat(MAX_TITLE_CHARS);
    h.manager
        .rename_session(&draft.meeting_id, ok)
        .expect("exactly-at-limit title accepted");
}

// ------------------------------------------------------- summary (Phase 7)

/// Scripted LLM shared with the summary unit tests' contract.
struct FakeSummaryLlm {
    answer: String,
}
impl summarize::SummaryLlm for FakeSummaryLlm {
    fn complete(&self, _prompt: String) -> Result<String, String> {
        Ok(self.answer.clone())
    }
}

const SUMMARY_JSON: &str = r##"{"summary_markdown": "# Besprechung", "decisions": ["OK"],
    "action_items": ["Protokoll senden"], "open_questions": ["Budget?"]}"##;

fn env_with_summary(answer: &'static str) -> StaticEnvironment {
    let mut env = healthy_env();
    env.summary_consent
        .store(true, std::sync::atomic::Ordering::SeqCst);
    env.llm = std::sync::Mutex::new(Some((
        "openrouter".to_string(),
        "test-model".to_string(),
        Box::new(FakeSummaryLlm {
            answer: answer.to_string(),
        }),
    )));
    env
}

#[test]
fn settle_with_summary_enabled_walks_transcribing_summarizing_completed() {
    let h = harness_with(env_with_summary(SUMMARY_JSON), false);
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, true, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Transcribing
    );
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);

    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Completed
    );

    let summary = h
        .manager
        .store
        .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
        .unwrap()
        .expect("summary persisted");
    assert_eq!(summary.state, "completed");
    assert_eq!(summary.decisions, serde_json::json!(["OK"]));
    assert_eq!(
        summary.action_items,
        serde_json::json!(["Protokoll senden"])
    );
    assert_eq!(summary.open_questions, serde_json::json!(["Budget?"]));
    assert!(summary.prompt_version.contains("meeting-summary-v1"));
}

#[test]
fn without_consent_the_summary_never_runs() {
    let env = healthy_env();
    // The LLM slot is left empty on purpose: if the pipeline ever tried to
    // reach an LLM without consent it would fail loudly here instead of
    // silently skipping — and the state assertions below pin the skip.
    let h = harness_with(env, false);
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, true, true)
        .unwrap();
    h.manager.stop(&draft.meeting_id, false).unwrap();
    // Straight to Completed: no cloud call, no Summarizing state.
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Completed
    );
    assert!(h
        .manager
        .store
        .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
        .unwrap()
        .is_none());
}

#[test]
fn llm_failure_lands_in_recoverable_error_and_persists_a_failed_row() {
    // Consent on, LLM answers empty -> parse fails after the map call.
    let mut env = healthy_env();
    env.summary_consent
        .store(true, std::sync::atomic::Ordering::SeqCst);
    env.llm = std::sync::Mutex::new(Some((
        "openrouter".to_string(),
        "m".to_string(),
        Box::new(FakeSummaryLlm {
            answer: String::new(),
        }),
    )));
    let h = harness_with(env, false);
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, true, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Transcribing
    );
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);

    // With transcript content and consent present, settlement enters
    // Summarizing; the broken LLM answer must land the meeting in
    // recoverable_error and persist the failed row.
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::RecoverableError
    );
    let summary = h
        .manager
        .store
        .summary_for_meeting(StoreUuid(draft.meeting_id.clone()))
        .unwrap()
        .expect("failed row is persisted");
    assert_eq!(summary.state, "failed");
}

/// Seeds one ready chunk + one persisted segment through the store API so
/// summary flows have real content to work on.
fn seed_ready_chunk(manager: &MeetingManager, meeting_id: &str) {
    let chunk = manager
        .store
        .append_chunk(NewAudioChunk {
            meeting_id: StoreUuid(meeting_id.to_string()),
            sequence: 0,
            source: "microphone".to_string(),
            timeline_start_ms: 0,
            timeline_end_ms: 2_000,
            storage_key: format!("audio/{meeting_id}/000000-microphone.wav"),
            sample_rate: 16_000,
            sample_count: 32_000,
            device_id: Some("test-mic".to_string()),
        })
        .expect("chunk");
    manager
        .store
        .finalize_chunk(
            chunk.envelope.id.clone(),
            ChunkFacts {
                file_size: 64_064,
                sample_count: 32_000,
                checksum: "a".repeat(64),
                gap_drift_metadata: None,
            },
        )
        .expect("finalize");
}

/// Persists one segment for the job created by the stop path and marks the
/// job succeeded, mirroring what the real queue worker would do before
/// settlement runs.
fn seed_segment_for_first_job(manager: &MeetingManager, meeting_id: &str) {
    let _ = meeting_id;
    let now = chrono::Utc::now();
    let claim = manager
        .store
        .claim_due_stt_job(WorkerId("test-worker".to_string()), now)
        .expect("claim")
        .expect("a queued job");
    let job = claim.job;
    manager
        .store
        .insert_segments(
            job.id.clone(),
            vec![NewTranscriptSegment {
                source: "microphone".to_string(),
                start_ms: 0,
                end_ms: 1_500,
                text: "Wir einigen uns auf Montag.".to_string(),
                word_timestamps: None,
                confidence: Some(0.9),
                provider: "Local".to_string(),
                model_id: Some("local-whisper".to_string()),
                sequence: 0,
            }],
        )
        .expect("segments");
    manager
        .store
        .transition_stt_job(job.id, SttJobState::Succeeded, None)
        .expect("succeed");
}

/// Full-journey smoke (plan §13 end-to-end): record → STT → summary →
/// export → project assignment → WhatsApp dedupe → tasks → controlled
/// deletion. One chain, so a regression anywhere breaks loudly here.
#[test]
fn the_whole_plan_journey_works_end_to_end() {
    let h = harness_with(env_with_summary(SUMMARY_JSON), false);

    // -- 1. Meeting lifecycle with pinned provider ----------------------
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start_with_provider(
            &draft.meeting_id,
            None,
            false,
            true,
            true,
            Some(crate::settings::SpeechInputProvider::Azure),
        )
        .unwrap();
    assert_eq!(
        h.manager.session_provider(&draft.meeting_id).0,
        "Azure Speech"
    );

    // -- 2. Capture + local STT queue path ------------------------------
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);

    // -- 3. Summary pipeline --------------------------------------------
    let meeting = StoreUuid(draft.meeting_id.clone());
    let summary = h
        .manager
        .store
        .summary_for_meeting(meeting.clone())
        .unwrap()
        .expect("summary persisted");
    assert_eq!(summary.state, "completed");

    // -- 4. Export -------------------------------------------------------
    for format in [
        export::ExportFormat::Markdown,
        export::ExportFormat::Txt,
        export::ExportFormat::Json,
    ] {
        let payload = h
            .manager
            .build_export(&draft.meeting_id, format)
            .unwrap()
            .unwrap();
        assert!(!payload.content.is_empty(), "{format:?} rendered empty");
    }

    // -- 5. Projects: create + assign + inbox -----------------------------
    let store = h.manager.store_handle();
    let project = store
        .create_project(crate::managers::meeting_store::NewProject {
            name: "Journey".to_string(),
            description: String::new(),
        })
        .unwrap();
    store
        .assign_meeting_to_project(StoreUuid(project.id.clone()), meeting.clone())
        .unwrap();
    assert_eq!(
        store
            .list_meeting_ids_for_project(StoreUuid(project.id.clone()))
            .unwrap(),
        vec![draft.meeting_id.clone()]
    );

    // -- 6. WhatsApp import is idempotent ---------------------------------
    let ingestion = crate::managers::source_ingestion::SourceIngestionService::new(store.clone());
    let chat = "31.12.24, 23:59 - Alice: Journeys end";
    let first = ingestion.ingest_text("Chat", chat, None).unwrap();
    assert!(first.created);
    let second = ingestion.ingest_text("Chat", chat, None).unwrap();
    assert!(!second.created, "same content must not duplicate");
    assert_eq!(first.source.id, second.source.id);

    // -- 7. Task workflow protects manual edits ---------------------------
    let pid = StoreUuid(project.id.clone());
    let (task, created) = store
        .suggest_task(NewProjectTask {
            project_id: pid.clone(),
            title: "Follow up".to_string(),
            description: String::new(),
            origin: "ai",
        })
        .unwrap();
    assert!(created);
    store
        .decide_task(
            Uuid(task.id.clone()),
            "accepted",
            Some("Follow up Friday".into()),
            None,
        )
        .unwrap();
    let (_same, not_created) = store
        .suggest_task(NewProjectTask {
            project_id: pid,
            title: "Follow up".to_string(),
            description: String::new(),
            origin: "ai",
        })
        .unwrap();
    assert!(!not_created);
    let tasks = store.list_tasks(Uuid(project.id.clone())).unwrap();
    assert_eq!(tasks[0].title, "Follow up Friday", "manual edit survives");

    // -- 8. Controlled deletion keeps transcript on AudioOnly --------------
    h.manager
        .delete_meeting_data(&draft.meeting_id, DeleteScope::AudioOnly)
        .unwrap();
    assert!(h.manager.get_transcript(&draft.meeting_id).unwrap().len() == 1);
    assert!(h.manager.get_summary(&draft.meeting_id).unwrap().is_some());

    // Everything removes session + content.
    h.manager
        .delete_meeting_data(&draft.meeting_id, DeleteScope::Everything)
        .unwrap();
    assert!(h.manager.get_session(&draft.meeting_id).unwrap().is_none());
    // The project survives and simply has no members anymore.
    assert_eq!(
        store
            .list_meeting_ids_for_project(StoreUuid(project.id))
            .unwrap(),
        Vec::<String>::new()
    );
}

/// Dumb-user combination matrix: every wild click sequence must end in a
/// sane state without corrupting the store.
#[test]
fn hostile_click_matrix_stays_consistent() {
    let h = harness();

    // Double-start on the same meeting.
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    assert!(h
        .manager
        .start(&draft.meeting_id, None, false, false, true)
        .is_err());

    // Start a second meeting while the first records.
    let second = h.manager.create_draft(None).unwrap();
    assert!(h
        .manager
        .start(&second.meeting_id, None, false, false, true)
        .is_err());

    // Pause/pause/resume/resume spam.
    h.manager.pause(&draft.meeting_id).unwrap();
    assert!(h.manager.pause(&draft.meeting_id).is_err());
    h.manager.resume(&draft.meeting_id).unwrap();
    assert!(h.manager.resume(&draft.meeting_id).is_err());

    // Cancel is illegal mid-recording (must stop first) — refused.
    assert!(h.manager.cancel(&draft.meeting_id).is_err());

    // Stop, then stop again: terminal states reject further stops.
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert!(h.manager.stop(&draft.meeting_id, false).is_err());

    // Cancel AFTER completion is refused (terminal is terminal).
    assert!(h.manager.cancel(&draft.meeting_id).is_err());

    // The second draft is unaffected and can start after the first ended.
    h.manager
        .start(&second.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.stop(&second.meeting_id, false).unwrap();

    // Cancel from a terminal state is refused; from a fresh draft it works.
    assert!(h.manager.cancel(&second.meeting_id).is_err());
    let third = h.manager.create_draft(None).unwrap();
    h.manager.cancel(&third.meeting_id).unwrap();

    // Pause→stop and resume→stop are legal exits from both states.
    let fourth = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&fourth.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.pause(&fourth.meeting_id).unwrap();
    h.manager.stop(&fourth.meeting_id, false).unwrap();
    let fifth = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&fifth.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.pause(&fifth.meeting_id).unwrap();
    h.manager.resume(&fifth.meeting_id).unwrap();
    h.manager.stop(&fifth.meeting_id, false).unwrap();

    // Deleting while transcription is in flight must be refused: the STT
    // jobs would write into a tombstoned session. After settlement the
    // meeting becomes deletable again.
    let sixth = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&sixth.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &sixth.meeting_id);
    h.manager.stop(&sixth.meeting_id, false).unwrap();
    assert_eq!(
        h.manager.snapshot(&sixth.meeting_id).unwrap().state,
        MeetingState::Transcribing
    );
    assert!(h.manager.soft_delete_session(&sixth.meeting_id).is_err());
    assert!(h
        .manager
        .delete_meeting_data(&sixth.meeting_id, DeleteScope::Everything)
        .is_err());
    seed_segment_for_first_job(&h.manager, &sixth.meeting_id);
    h.manager.settle_after_queue(&sixth.meeting_id);
    assert!(h.manager.soft_delete_session(&sixth.meeting_id).unwrap());

    // Deleting a cancelled meeting works; deleting again is a no-op false.
    assert!(h.manager.soft_delete_session(&second.meeting_id).unwrap());

    // Rename on a deleted session is refused.
    assert!(h
        .manager
        .rename_session(&second.meeting_id, "Ghost".to_string())
        .is_err());
}

/// Everything removes all content rows and the session itself.
#[test]
fn controlled_deletion_respects_the_two_scopes() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    // Settle like the real queue would — deletion during `transcribing`
    // is refused (see hostile click matrix).
    h.manager.settle_after_queue(&draft.meeting_id);

    // A raw audio directory exists for this meeting before deletion.
    let meeting_tree = h
        .dir
        .path()
        .join("assets")
        .join("meetings")
        .join(&draft.meeting_id);
    std::fs::create_dir_all(&meeting_tree).expect("create tree");

    // AudioOnly keeps transcript rows and the session.
    h.manager
        .delete_meeting_data(&draft.meeting_id, DeleteScope::AudioOnly)
        .unwrap();
    assert!(!meeting_tree.exists(), "audio files are removed");
    {
        let conn = lock(&h.manager.conn);
        let alive_segments: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meeting_transcript_segments
                 WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![draft.meeting_id],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        assert_eq!(alive_segments, 1, "transcript survives AudioOnly");
    }

    // Everything additionally tombstones content and the session.
    h.manager
        .delete_meeting_data(&draft.meeting_id, DeleteScope::Everything)
        .unwrap();
    assert!(h.manager.get_session(&draft.meeting_id).unwrap().is_none());

    let conn = lock(&h.manager.conn);
    let alive_segments: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM meeting_transcript_segments WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![draft.meeting_id],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    assert_eq!(alive_segments, 0, "segments are gone after full purge");
}

/// Pen-test: deleting the session that currently owns the capture must be
/// refused — the recorder would keep writing into a tombstoned row.
#[test]
fn delete_refuses_an_actively_capturing_session() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();

    let err = h
        .manager
        .soft_delete_session(&draft.meeting_id)
        .expect_err("active capture must block deletion");
    assert!(err.to_string().contains("meetingIsActive"));

    // After stopping, deletion works as usual.
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert!(h.manager.soft_delete_session(&draft.meeting_id).unwrap());
}

/// Delete hides the session from get/list but keeps the tombstoned row.
#[test]
fn delete_is_soft_and_hides_session() {
    let (_dir, manager) = temp_manager();
    let created = manager.create_session(Some("Doomed".into())).unwrap();

    assert!(manager.soft_delete_session(&created.id).unwrap());
    assert!(!manager.soft_delete_session(&created.id).unwrap());
    assert!(manager.get_session(&created.id).unwrap().is_none());
    assert!(manager.list_sessions(None).unwrap().is_empty());

    let conn = lock(&manager.conn);
    let tombstones: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM meeting_sessions WHERE id = ?1 AND deleted_at IS NOT NULL",
            params![created.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tombstones, 1);
}

/// Migrations are idempotent: reopening an up-to-date database is a no-op.
#[test]
fn reopen_does_not_duplicate_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("meetings.db");
    drop(MeetingManager::open(db_path.clone()).unwrap());
    drop(MeetingManager::open(db_path.clone()).unwrap());

    let conn = Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meeting_sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// -------------------------------------------------- state machine table

/// The SSC §2.2 table, asserted row by row, plus the closed complement:
/// every pair not in the table is rejected.
#[test]
fn transition_table_matches_the_contract_and_rejects_everything_else() {
    use MeetingState::*;
    let legal = [
        (Draft, Starting),
        (Draft, Cancelled),
        (Starting, Recording),
        (Starting, Draft),
        (Starting, Cancelled),
        (Starting, RecoverableError),
        (Starting, StoppedUnprocessed),
        (Recording, Paused),
        (Recording, Stopping),
        (Recording, RecoverableError),
        (Recording, StoppedUnprocessed),
        (Paused, Recording),
        (Paused, Stopping),
        (Paused, RecoverableError),
        (Paused, StoppedUnprocessed),
        (Stopping, Transcribing),
        (Stopping, Completed),
        (Stopping, StoppedUnprocessed),
        (Stopping, RecoverableError),
        (Transcribing, Summarizing),
        (Transcribing, Completed),
        (Transcribing, RecoverableError),
        (Summarizing, Completed),
        (Summarizing, RecoverableError),
        (RecoverableError, Transcribing),
        (RecoverableError, Summarizing),
        (RecoverableError, Completed),
        (Completed, Transcribing),
        (StoppedUnprocessed, Transcribing),
        (StoppedUnprocessed, Completed),
    ];
    for (from, to) in legal {
        assert!(
            transition_allowed(from, to),
            "{} -> {} must be legal",
            from.as_str(),
            to.as_str()
        );
    }

    let all = [
        Draft,
        Starting,
        Recording,
        Paused,
        Stopping,
        Transcribing,
        Summarizing,
        Completed,
        StoppedUnprocessed,
        Cancelled,
        RecoverableError,
    ];
    for from in all {
        for to in all {
            let expected = legal.contains(&(from, to));
            assert_eq!(
                transition_allowed(from, to),
                expected,
                "{} -> {} classified wrongly",
                from.as_str(),
                to.as_str()
            );
        }
    }

    // The forbidden shortcuts the contract calls out by name.
    assert!(!transition_allowed(Recording, Completed));
    assert!(!transition_allowed(Completed, Recording));
    assert!(!transition_allowed(Cancelled, Draft));
}

/// Every state token round-trips through the persisted representation and
/// matches the TypeScript union exactly.
#[test]
fn state_tokens_match_the_frontend_union() {
    for (state, token) in [
        (MeetingState::Draft, "draft"),
        (MeetingState::Starting, "starting"),
        (MeetingState::Recording, "recording"),
        (MeetingState::Paused, "paused"),
        (MeetingState::Stopping, "stopping"),
        (MeetingState::Transcribing, "transcribing"),
        (MeetingState::Summarizing, "summarizing"),
        (MeetingState::Completed, "completed"),
        (MeetingState::StoppedUnprocessed, "stopped_unprocessed"),
        (MeetingState::Cancelled, "cancelled"),
        (MeetingState::RecoverableError, "recoverable_error"),
    ] {
        assert_eq!(state.as_str(), token);
        assert_eq!(MeetingState::parse(token), Some(state));
        assert_eq!(serde_json::to_value(state).unwrap(), token);
    }
    assert_eq!(MeetingState::parse("processing"), None);
}

// ---------------------------------------------------------- lifecycle

#[test]
fn start_pause_resume_stop_walks_the_contract_path() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    assert_eq!(draft.state, MeetingState::Draft);

    let started = h
        .manager
        .start(&draft.meeting_id, Some("Weekly".into()), false, false, true)
        .unwrap();
    assert_eq!(started.state, MeetingState::Recording);
    assert!(started.running_since.is_some());

    let paused = h.manager.pause(&draft.meeting_id).unwrap();
    assert_eq!(paused.state, MeetingState::Paused);
    assert!(paused.running_since.is_none());

    let resumed = h.manager.resume(&draft.meeting_id).unwrap();
    assert_eq!(resumed.state, MeetingState::Recording);

    // No chunks were captured by the fake recorder, so T09 applies.
    let stopped = h.manager.stop(&draft.meeting_id, false).unwrap();
    assert_eq!(stopped.state, MeetingState::Completed);

    let states: Vec<String> = h
        .sink
        .topics(TOPIC_SESSION_STATE)
        .iter()
        .filter_map(|v| v["to"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        states,
        vec![
            "starting",
            "recording",
            "paused",
            "recording",
            "stopping",
            "completed"
        ]
    );
}

/// Phase 6: the provider chosen for one meeting is persisted on start and
/// drives later STT job routing — without touching dictation settings.
#[test]
fn start_pins_the_chosen_provider_for_the_session() {
    let h = harness();
    let pinned = h.manager.create_draft(None).unwrap();
    let started = h
        .manager
        .start_with_provider(
            &pinned.meeting_id,
            None,
            false,
            false,
            true,
            Some(crate::settings::SpeechInputProvider::Azure),
        )
        .unwrap();
    assert_eq!(started.provider_kind, "cloud");
    assert_eq!(started.provider_name.as_deref(), Some("Azure Speech"));

    // The queue resolves jobs through this stored snapshot.
    let (name, config) = h.manager.session_provider(&pinned.meeting_id);
    assert_eq!(name, "Azure Speech");
    assert_eq!(config["kind"], "cloud");

    // A sibling meeting keeps the global default.
    let _ = h.manager.stop(&pinned.meeting_id, false);
    let plain = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&plain.meeting_id, None, false, false, true)
        .unwrap();
    let (default_name, _) = h.manager.session_provider(&plain.meeting_id);
    assert_eq!(default_name, "Local");
}

/// Fail-testing: a corrupted or hostile provider snapshot in the database
/// must degrade to an empty config, never panic the queue thread.
#[test]
fn session_provider_survives_a_corrupted_snapshot() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    {
        let conn = lock(&h.manager.conn);
        conn.execute(
            "UPDATE meeting_sessions
             SET stt_provider = 'Azure Speech',
                 provider_config_snapshot = '{not json at all',
                 language = NULL
             WHERE id = ?1",
            params![draft.meeting_id],
        )
        .expect("corrupt snapshot");
    }
    let (name, config) = h.manager.session_provider(&draft.meeting_id);
    assert_eq!(name, "Azure Speech");
    assert_eq!(
        config,
        serde_json::json!({}),
        "unparsable config degrades to empty"
    );
}

/// O2: revisions are strictly increasing, and exactly one event is
/// published per transition (no gaps caused by concurrent emissions).
#[test]
fn revisions_are_strictly_monotone_with_one_event_per_transition() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.pause(&draft.meeting_id).unwrap();
    h.manager.resume(&draft.meeting_id).unwrap();
    h.manager.stop(&draft.meeting_id, true).unwrap();

    let events = h.sink.topics(TOPIC_SESSION_STATE);
    let revisions: Vec<i64> = events
        .iter()
        .filter_map(|v| v["state_revision"].as_i64())
        .collect();
    assert_eq!(revisions.len(), events.len());
    for pair in revisions.windows(2) {
        assert!(
            pair[1] > pair[0],
            "revisions must strictly increase, saw {:?}",
            revisions
        );
    }
    // Each published transition consumed exactly one revision.
    for pair in revisions.windows(2) {
        assert_eq!(pair[1] - pair[0], 1, "revision gap in {:?}", revisions);
    }
    assert_eq!(
        h.manager
            .snapshot(&draft.meeting_id)
            .unwrap()
            .state_revision,
        *revisions.last().unwrap()
    );
}

/// Illegal transitions are refused and leave the persisted state alone.
#[test]
fn illegal_transition_is_refused_and_does_not_persist() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    // pause is only legal from `recording`.
    let err = h.manager.pause(&draft.meeting_id).unwrap_err();
    assert!(err.contains("illegal meeting transition"), "{err}");
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Draft
    );

    // stop from draft is equally illegal.
    assert!(h.manager.stop(&draft.meeting_id, false).is_err());
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Draft
    );
}

/// Gate G4 is fail-closed: without consent no capture resource is opened.
#[test]
fn start_without_consent_is_refused_before_any_device_opens() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    let err = h
        .manager
        .start(&draft.meeting_id, None, false, false, false)
        .unwrap_err();
    assert_eq!(err, "consentRequired");
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Draft
    );
    assert!(h.sink.topics(TOPIC_SESSION_STATE).is_empty());
    let errors = h.sink.topics(TOPIC_SESSION_ERROR);
    assert_eq!(errors[0]["code"], "consentRequired");
}

/// T04: a failing device open returns the session to `draft` with a code.
#[test]
fn device_failure_returns_to_draft_with_error() {
    let h = harness_with(healthy_env(), true);
    let draft = h.manager.create_draft(None).unwrap();
    let err = h
        .manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap_err();
    assert_eq!(err, "micUnavailable");
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Draft
    );
    let states: Vec<String> = h
        .sink
        .topics(TOPIC_SESSION_STATE)
        .iter()
        .filter_map(|v| v["to"].as_str().map(str::to_string))
        .collect();
    assert_eq!(states, vec!["starting", "draft"]);
}

/// A stop that the user aborts still finalizes, but lands in
/// `stopped_unprocessed` (T15).
#[test]
fn aborted_stop_lands_in_stopped_unprocessed() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    let stopped = h.manager.stop(&draft.meeting_id, true).unwrap();
    assert_eq!(stopped.state, MeetingState::StoppedUnprocessed);
}

/// A second session cannot seize the capture devices while one records.
#[test]
fn a_second_meeting_cannot_start_while_one_records() {
    let h = harness();
    let first = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&first.meeting_id, None, false, false, true)
        .unwrap();
    let second = h.manager.create_draft(None).unwrap();
    let err = h
        .manager
        .start(&second.meeting_id, None, false, false, true)
        .unwrap_err();
    assert_eq!(err, "anotherMeetingIsRecording");
}

/// Plan §6.10: a dictation stays possible while a meeting records. The
/// meeting path owns no dictation resource, so the dictation binding is
/// unaffected by an active capture session.
#[test]
fn an_active_meeting_leaves_the_dictation_path_untouched() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    assert_eq!(
        h.manager.active_capture_state(),
        Some(MeetingState::Recording)
    );
    // The dictation shortcut ids are still routed to the dictation
    // coordinator, and the meeting ids are disjoint from them.
    assert!(crate::transcription_coordinator::is_pipeline_binding(
        "transcribe"
    ));
    assert!(!crate::shortcut::is_meeting_binding("transcribe"));
    assert!(!crate::transcription_coordinator::is_pipeline_binding(
        crate::settings::MEETING_BINDING_START_STOP
    ));
}

// ---------------------------------------------------------- persistence

/// Phase 4 acceptance (plan line 1577): after a simulated restart the
/// persisted state and the snapshot agree.
#[test]
fn state_survives_a_simulated_restart() {
    let h = harness();
    let draft = h.manager.create_draft(Some("Board".into())).unwrap();
    h.manager
        .start(&draft.meeting_id, None, true, false, true)
        .unwrap();
    h.manager.stop(&draft.meeting_id, true).unwrap();
    let before = h.manager.snapshot(&draft.meeting_id).unwrap();

    let reopened = MeetingManager::open(h.db_path.clone()).unwrap();
    let after = reopened.snapshot(&draft.meeting_id).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.state, MeetingState::StoppedUnprocessed);
    assert_eq!(after.state_revision, before.state_revision);
    assert_eq!(after.title, "Board");
}

// ------------------------------------------------------------- preflight

#[test]
fn preflight_passes_with_a_healthy_environment() {
    let h = harness();
    let report = h.manager.preflight();
    assert!(report.can_start);
    assert_eq!(report.checks.len(), 6);
    assert_eq!(report.microphone_only_available, Some(false));
}

#[test]
fn preflight_blocks_on_a_missing_microphone() {
    let mut env = healthy_env();
    env.checks[0].outcome = rec::PreflightOutcome::Missing;
    let h = harness_with(env, false);
    let report = h.manager.preflight();
    assert!(!report.can_start);
    let mic = report
        .checks
        .iter()
        .find(|c| c.id == MeetingPreflightCheckId::Microphone)
        .unwrap();
    assert_eq!(mic.status, MeetingPreflightStatus::Error);
    assert_eq!(mic.code.as_deref(), Some("deviceUnavailable"));
}

/// Plan E5: a missing loopback only warns and offers the mic-only fallback.
#[test]
fn preflight_offers_microphone_only_when_loopback_is_missing() {
    let mut env = healthy_env();
    env.checks[1].outcome = rec::PreflightOutcome::Missing;
    let h = harness_with(env, false);
    let report = h.manager.preflight();
    assert!(report.can_start);
    assert_eq!(report.microphone_only_available, Some(true));
}

#[test]
fn preflight_blocks_on_low_disk_and_on_unconfigured_provider() {
    let mut env = healthy_env();
    env.free_bytes = Some(1_000_000);
    let h = harness_with(env, false);
    let report = h.manager.preflight();
    assert!(!report.can_start);
    assert!(report
        .checks
        .iter()
        .any(|c| c.code.as_deref() == Some("diskFull")));

    let mut env = healthy_env();
    env.provider.configured = false;
    let h = harness_with(env, false);
    assert!(!h.manager.preflight().can_start);

    let mut env = healthy_env();
    env.permission_denied = true;
    let h = harness_with(env, false);
    assert!(!h.manager.preflight().can_start);
}

/// Cloud providers must reserve the upload budget on top of the local one.
#[test]
fn preflight_requires_the_cloud_reserve_for_cloud_providers() {
    let mut env = healthy_env();
    env.free_bytes = Some(DISK_PREFLIGHT_REQUIRED_FREE_BYTES_LOCAL + 1);
    env.provider.kind = "cloud".to_string();
    let h = harness_with(env, false);
    assert!(!h.manager.preflight().can_start);
}

/// Start refuses when the preflight fails, and rewinds `starting → draft`.
#[test]
fn start_refuses_when_the_preflight_fails() {
    let mut env = healthy_env();
    env.free_bytes = Some(1);
    let h = harness_with(env, false);
    let draft = h.manager.create_draft(None).unwrap();
    let err = h
        .manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap_err();
    assert_eq!(err, "diskFull");
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Draft
    );
}

// -------------------------------------------------------------- recovery

/// R5(a): a session left in `recording` by a crash is offered for
/// processing and never stays active.
#[test]
fn startup_recovery_disposes_active_sessions_and_produces_offers() {
    let h = harness();
    let draft = h.manager.create_draft(Some("Crashed".into())).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();

    // Simulate the crash: the process dies, the row stays `recording`.
    drop(h.manager);
    let sink = Arc::new(RecordingSink::default());
    let restarted = MeetingManager::open_with(
        h.db_path.clone(),
        PathBuf::from("assets"),
        sink.clone(),
        Arc::new(healthy_env()),
        fake_factory(false),
    )
    .unwrap();

    let offers = restarted.run_startup_recovery().unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].meeting_id, draft.meeting_id);
    assert_eq!(offers[0].title, "Crashed");
    assert_eq!(offers[0].cause, MeetingRecoveryCause::Crash);
    assert_eq!(
        restarted.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::StoppedUnprocessed
    );
    assert_eq!(restarted.list_recoveries().len(), 1);
    // R6 publishes per session and once consolidated.
    assert_eq!(sink.topics(TOPIC_SESSION_RECOVERED).len(), 1);
    assert_eq!(sink.topics(TOPIC_RECOVERY_REPORT).len(), 1);

    // Idempotence: a second run finds no candidates.
    assert!(restarted.run_startup_recovery().unwrap().is_empty());
}

/// R3: crashed `running` jobs return to `queued` without a retry penalty.
#[test]
fn startup_recovery_requeues_running_jobs_without_penalty() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    {
        let conn = lock(&h.manager.conn);
        conn.execute(
            "INSERT INTO meeting_stt_jobs
               (id, meeting_id, source, timeline_start_ms, timeline_end_ms, provider,
                state, attempt_count, created_at, updated_at)
             VALUES ('job-1', ?1, 'microphone', 0, 1000, 'local', 'running', 2, 0, 0)",
            params![draft.meeting_id],
        )
        .unwrap();
    }
    h.manager.run_startup_recovery().unwrap();
    let conn = lock(&h.manager.conn);
    let (state, attempts): (String, i64) = conn
        .query_row(
            "SELECT state, attempt_count FROM meeting_stt_jobs WHERE id = 'job-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(attempts, 2, "crash recovery must not count as an attempt");
}

/// Resolutions: "later" keeps the offer, "delete" removes the session.
#[test]
fn recovery_resolutions_behave_as_the_dataflow_specifies() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    drop(h.manager);
    let restarted = MeetingManager::open_with(
        h.db_path.clone(),
        PathBuf::from("assets"),
        Arc::new(RecordingSink::default()),
        Arc::new(healthy_env()),
        fake_factory(false),
    )
    .unwrap();
    restarted.run_startup_recovery().unwrap();

    assert!(restarted
        .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Later)
        .unwrap());
    assert_eq!(restarted.list_recoveries().len(), 1);

    assert!(restarted
        .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Delete)
        .unwrap());
    assert!(restarted.list_recoveries().is_empty());
    assert!(restarted.get_session(&draft.meeting_id).unwrap().is_none());
}

/// T18: "recover" with no eligible audio completes instead of hanging in
/// `transcribing` forever.
#[test]
fn recovering_without_eligible_audio_completes() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.stop(&draft.meeting_id, true).unwrap();
    assert!(h
        .manager
        .resolve_recovery(&draft.meeting_id, MeetingRecoveryResolution::Recover)
        .unwrap());
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Completed
    );
}

// ---------------------------------------------------------- wire shapes

/// The snapshot serializes exactly the field names the frontend reads.
#[test]
fn snapshot_payload_matches_the_typescript_contract() {
    let h = harness();
    let draft = h.manager.create_draft(Some("Sync".into())).unwrap();
    let value = serde_json::to_value(h.manager.snapshot(&draft.meeting_id).unwrap()).unwrap();
    for field in [
        "meeting_id",
        "title",
        "state",
        "state_revision",
        "elapsed_ms",
        "running_since",
        "microphone_only",
        "provider_kind",
        "provider_name",
        "summary_enabled",
        "transcript_head_sequence",
        "jobs",
        "levels",
        "last_error",
    ] {
        assert!(value.get(field).is_some(), "snapshot lacks {field}");
    }
    assert_eq!(value["state"], "draft");
}

/// The state event carries `meeting_id`/`from`/`to`/`state_revision`.
#[test]
fn session_state_event_matches_the_typescript_contract() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, true, false, true)
        .unwrap();
    let event = &h.sink.topics(TOPIC_SESSION_STATE)[0];
    assert_eq!(event["meeting_id"], draft.meeting_id);
    assert_eq!(event["from"], "draft");
    assert_eq!(event["to"], "starting");
    assert!(event["state_revision"].as_i64().is_some());
    // A microphone-only session says so on the wire (plan E5).
    let recording = &h.sink.topics(TOPIC_SESSION_STATE)[1];
    assert_eq!(recording["microphone_only"], true);
}

/// Preflight rows use the camelCase ids and lowercase statuses of the
/// TypeScript union.
#[test]
fn preflight_payload_matches_the_typescript_contract() {
    let h = harness();
    let value = serde_json::to_value(h.manager.preflight()).unwrap();
    let ids: Vec<&str> = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "microphone",
            "systemAudio",
            "transcription",
            "language",
            "disk",
            "permission"
        ]
    );
    assert_eq!(value["checks"][0]["status"], "ok");
    assert_eq!(value["can_start"], true);
}

/// Recovery offers use the field names `MeetingRecoveryOffer` declares.
#[test]
fn recovery_offer_payload_matches_the_typescript_contract() {
    let offer = MeetingRecoveryOffer {
        meeting_id: "m".into(),
        title: "t".into(),
        cause: MeetingRecoveryCause::SleepWake,
        recovered_chunks: 1,
        damaged_chunks: 2,
        quarantined_files: 3,
        interrupted_at: Some("2026-01-01T00:00:00Z".into()),
    };
    let value = serde_json::to_value(&offer).unwrap();
    assert_eq!(value["meeting_id"], "m");
    assert_eq!(value["cause"], "sleep_wake");
    assert_eq!(value["recovered_chunks"], 1);
    assert_eq!(value["damaged_chunks"], 2);
    assert_eq!(value["quarantined_files"], 3);
}

/// A summary produced on this machine is not a cloud upload, so it must not
/// be gated behind cloud consent (plan §10: local STT + local summary is a
/// supported combination).
#[test]
fn loopback_providers_count_as_local() {
    for url in [
        "http://localhost:11434/v1",
        "http://127.0.0.1:8080/v1",
        "https://127.0.0.5/v1",
        "http://[::1]:1234/v1",
    ] {
        assert!(is_local_provider_url(url), "{url} must count as local");
    }
}

#[test]
fn remote_providers_are_never_mistaken_for_local() {
    for url in [
        "https://api.openai.com/v1",
        "https://openrouter.ai/api/v1",
        // Deliberately adversarial: a hostname that merely *contains* a
        // loopback-looking label must not pass.
        "https://localhost.evil.example/v1",
        "https://127.0.0.1.evil.example/v1",
    ] {
        assert!(!is_local_provider_url(url), "{url} must count as remote");
    }
}

/// Seeds one finalized chunk plus its backing file, which is what both the
/// `Audio` tab and re-transcription operate on.
fn seed_playable_chunk(
    manager: &MeetingManager,
    db_path: &Path,
    assets_root: &Path,
    session_id: &str,
) -> PathBuf {
    use rec::ChunkStateStore;
    let store = SqliteChunkStateStore::open(manager.store.clone(), db_path).unwrap();
    let storage_key = "audio/000001-microphone.wav";
    let descriptor = rec::ChunkDescriptor {
        chunk_id: 0,
        source: rec::SourceKind::Microphone,
        sequence: 1,
        source_segment_id: 1,
        state: rec::ChunkState::Writing,
        range: rec::TimelineRange {
            start_ms: 0.0,
            end_ms: 0.0,
        },
        sample_count: 0,
        storage_key: storage_key.into(),
        checksum_sha256: None,
        recovered: false,
    };
    let id = store.insert_writing_chunk(session_id, &descriptor).unwrap();
    store
        .transition(
            id,
            rec::ChunkState::Writing,
            rec::ChunkState::Ready,
            &rec::ChunkPatch {
                sample_count: Some(16_000),
                checksum_sha256: Some("a".repeat(64)),
                ..rec::ChunkPatch::default()
            },
        )
        .unwrap();

    let path = assets_root.join(session_id).join(storage_key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"RIFF").unwrap();
    path
}

/// Drives one meeting all the way to `completed` with a stored chunk and a
/// succeeded job — the only shape retention is allowed to touch.
fn completed_meeting_with_audio(h: &Harness) -> String {
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Completed
    );
    draft.meeting_id
}

fn audio_rows(h: &Harness, meeting_id: &str) -> i64 {
    let conn = lock(&h.manager.conn);
    conn.query_row(
        "SELECT COUNT(*) FROM meeting_audio_chunks
         WHERE meeting_id = ?1 AND deleted_at IS NULL",
        params![meeting_id],
        |row| row.get(0),
    )
    .unwrap()
}

/// Plan §12: "ohne beantwortete Auswahl wird nichts automatisch gelöscht".
/// The default must never cost a user their recordings.
#[test]
fn retention_deletes_nothing_until_a_rule_was_chosen() {
    let h = harness();
    let id = completed_meeting_with_audio(&h);
    for policy in [
        crate::settings::MeetingRetentionPolicy::Ask,
        crate::settings::MeetingRetentionPolicy::Forever,
    ] {
        assert_eq!(
            h.manager
                .sweep_retention(policy, chrono::Utc::now() + chrono::Duration::days(365))
                .unwrap(),
            0
        );
    }
    assert_eq!(audio_rows(&h, &id), 1);
}

/// A chosen age only bites once it actually elapsed.
#[test]
fn retention_keeps_audio_until_the_chosen_age_elapsed() {
    let h = harness();
    let id = completed_meeting_with_audio(&h);
    let policy = crate::settings::MeetingRetentionPolicy::Days7;

    assert_eq!(
        h.manager
            .sweep_retention(policy, chrono::Utc::now() + chrono::Duration::days(6))
            .unwrap(),
        0
    );
    assert_eq!(audio_rows(&h, &id), 1);

    assert_eq!(
        h.manager
            .sweep_retention(policy, chrono::Utc::now() + chrono::Duration::days(8))
            .unwrap(),
        1
    );
    assert_eq!(audio_rows(&h, &id), 0);
}

/// Only the raw recording goes; the transcript is what the user kept the
/// meeting for.
#[test]
fn retention_keeps_the_transcript_when_it_drops_the_recording() {
    let h = harness();
    let id = completed_meeting_with_audio(&h);
    assert_eq!(h.manager.get_transcript(&id).unwrap().len(), 1);

    h.manager
        .sweep_retention(
            crate::settings::MeetingRetentionPolicy::Immediately,
            chrono::Utc::now(),
        )
        .unwrap();

    assert_eq!(audio_rows(&h, &id), 0);
    assert_eq!(
        h.manager.get_transcript(&id).unwrap().len(),
        1,
        "retention must never take the transcript with it"
    );
}

/// Deleting the audio of a meeting whose transcription failed would make
/// that failure permanent, so such a meeting is skipped entirely.
#[test]
fn retention_spares_a_meeting_with_an_unfinished_job() {
    let h = harness();
    let id = completed_meeting_with_audio(&h);
    {
        let conn = lock(&h.manager.conn);
        conn.execute(
            "UPDATE meeting_stt_jobs SET state = 'failed' WHERE meeting_id = ?1",
            params![id],
        )
        .unwrap();
    }

    assert_eq!(
        h.manager
            .sweep_retention(
                crate::settings::MeetingRetentionPolicy::Immediately,
                chrono::Utc::now(),
            )
            .unwrap(),
        0
    );
    assert_eq!(audio_rows(&h, &id), 1);
}

/// A meeting still being processed keeps its audio no matter how old it is.
#[test]
fn retention_only_touches_completed_meetings() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Transcribing
    );

    assert_eq!(
        h.manager
            .sweep_retention(
                crate::settings::MeetingRetentionPolicy::Immediately,
                chrono::Utc::now() + chrono::Duration::days(365),
            )
            .unwrap(),
        0
    );
    assert_eq!(audio_rows(&h, &draft.meeting_id), 1);
}

/// The `Audio` tab exists "solange Audio vorhanden ist" (plan §6.13), so a
/// chunk row whose file is gone must not be offered as playable.
#[test]
fn audio_files_lists_only_chunks_whose_file_still_exists() {
    let h = harness();
    let assets_root = h.dir.path().join("assets");
    let session = h.manager.create_session(Some("Audio".into())).unwrap();
    let path = seed_playable_chunk(&h.manager, &h.db_path, &assets_root, &session.id);

    let files = h.manager.audio_files(&session.id).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].source, "microphone");
    assert_eq!(files[0].sequence, 1);
    // 16 000 samples at 16 kHz = exactly one second on the timeline.
    assert_eq!(files[0].end_ms - files[0].start_ms, 1_000);
    assert!(Path::new(&files[0].path).exists());

    std::fs::remove_file(&path).unwrap();
    assert!(
        h.manager.audio_files(&session.id).unwrap().is_empty(),
        "a deleted recording must not stay playable"
    );
}

/// Re-transcription discards the old transcript and queues the stored audio
/// again (plan §11). Appending instead would leave two copies of every line.
#[test]
fn retranscribe_replaces_the_transcript_and_keeps_the_audio() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Completed
    );
    assert_eq!(
        h.manager.get_transcript(&draft.meeting_id).unwrap().len(),
        1
    );

    h.manager.retranscribe(&draft.meeting_id, None).unwrap();

    assert!(
        h.manager
            .get_transcript(&draft.meeting_id)
            .unwrap()
            .is_empty(),
        "the stale transcript must be gone, not appended to"
    );
    let queued: i64 = {
        let conn = lock(&h.manager.conn);
        conn.query_row(
            "SELECT COUNT(*) FROM meeting_stt_jobs
             WHERE meeting_id = ?1 AND state = 'queued'",
            params![draft.meeting_id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(queued, 1, "exactly one fresh job owns the chunk again");
    assert_eq!(
        h.manager.chunk_counts(&draft.meeting_id).unwrap().0,
        1,
        "re-transcription must never touch the audio"
    );
    assert_eq!(
        h.manager.snapshot(&draft.meeting_id).unwrap().state,
        MeetingState::Transcribing
    );
}

#[test]
fn accuracy_pass_replaces_only_after_the_reference_succeeds() {
    struct ReferenceEngine;
    impl MeetingSttEngine for ReferenceEngine {
        fn transcribe(
            &self,
            _ctx: &JobContext,
            _audio: &[f32],
        ) -> Result<Vec<EngineSegment>, super::super::meeting_transcription::EngineError> {
            Ok(vec![EngineSegment {
                start_ms_rel: 100,
                end_ms_rel: 1_900,
                text: "Wir einigen uns auf Montag und liefern Version am Dienstag.".to_string(),
                confidence: Some(0.95),
            }])
        }

        fn model_label(&self) -> String {
            "reference-test".to_string()
        }
    }

    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    seed_ready_chunk(&h.manager, &draft.meeting_id);
    h.manager.stop(&draft.meeting_id, false).unwrap();
    seed_segment_for_first_job(&h.manager, &draft.meeting_id);
    h.manager.settle_after_queue(&draft.meeting_id);

    let path = h
        .manager
        .assets_root
        .join(&draft.meeting_id)
        .join(format!("audio/{}/000000-microphone.wav", draft.meeting_id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        super::super::meeting_cloud::encode_wav_pcm16_mono(&vec![0.1; 32_000]),
    )
    .unwrap();
    *lock(&h.manager.stt_engine) = Some(Arc::new(ReferenceEngine));

    let report = h
        .manager
        .improve_accuracy(
            &draft.meeting_id,
            crate::settings::SpeechInputProvider::Azure,
        )
        .unwrap();
    assert_eq!(report.chunks_checked, 1);
    assert_eq!(report.chunks_corrected, 1);
    let transcript = h.manager.get_transcript(&draft.meeting_id).unwrap();
    assert_eq!(transcript.len(), 1);
    assert!(transcript[0].text.contains("Dienstag"));

    let archived: i64 = lock(&h.manager.conn)
        .query_row(
            "SELECT COUNT(*) FROM meeting_transcript_segments
             WHERE meeting_id = ?1 AND deleted_at IS NOT NULL",
            params![draft.meeting_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1, "the original must remain recoverable");
}

/// A meeting that never finished capturing has no transcript to replace,
/// and the refusal must come *before* anything is deleted.
#[test]
fn retranscribe_refuses_a_meeting_that_never_finished() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    assert_eq!(
        h.manager.retranscribe(&draft.meeting_id, None).unwrap_err(),
        "meetingNotTranscribable"
    );
}

/// Without stored audio there is nothing to transcribe, and the user gets a
/// named reason instead of a silently empty run.
#[test]
fn retranscribe_refuses_when_no_audio_remains() {
    let h = harness();
    let draft = h.manager.create_draft(None).unwrap();
    h.manager
        .start(&draft.meeting_id, None, false, false, true)
        .unwrap();
    h.manager.stop(&draft.meeting_id, false).unwrap();
    assert_eq!(
        h.manager.retranscribe(&draft.meeting_id, None).unwrap_err(),
        "noAudioToTranscribe"
    );
}

/// The chunk-state seam maps recorder ids onto store UUID rows and back.
#[test]
fn chunk_state_store_round_trips_through_the_repository() {
    use rec::ChunkStateStore;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("meetings.db");
    let manager = MeetingManager::open(db_path.clone()).unwrap();
    let session = manager.create_session(Some("Chunks".into())).unwrap();
    let store = SqliteChunkStateStore::open(manager.store.clone(), &db_path).unwrap();

    let descriptor = rec::ChunkDescriptor {
        chunk_id: 0,
        source: rec::SourceKind::Microphone,
        sequence: 0,
        source_segment_id: 1,
        state: rec::ChunkState::Writing,
        range: rec::TimelineRange {
            start_ms: 0.0,
            end_ms: 1000.0,
        },
        sample_count: 16_000,
        storage_key: "audio/000000-microphone.wav".into(),
        checksum_sha256: None,
        recovered: false,
    };
    let id = store
        .insert_writing_chunk(&session.id, &descriptor)
        .unwrap();
    let open = store.open_chunks(&session.id).unwrap();
    assert_eq!(open.len(), 1);

    store
        .transition(
            id,
            rec::ChunkState::Writing,
            rec::ChunkState::Ready,
            &rec::ChunkPatch {
                sample_count: Some(16_000),
                checksum_sha256: Some("a".repeat(64)),
                ..rec::ChunkPatch::default()
            },
        )
        .unwrap();
    assert!(store.open_chunks(&session.id).unwrap().is_empty());
    assert_eq!(manager.chunk_counts(&session.id).unwrap(), (1, 0));

    // A finalized chunk makes the stop path enqueue an STT job (T08).
    assert_eq!(manager.enqueue_ready_chunks(&session.id).unwrap(), 1);
}
