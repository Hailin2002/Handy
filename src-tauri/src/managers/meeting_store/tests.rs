//! Tests for [`super`] — the SQLite meeting store.
//!
//! Extracted from meeting_store.rs, which had grown past 5000 lines with a
//! third of that being this module. Same arrangement the project already
//! uses for `settings/tests.rs` and `model/download/tests.rs`.

use super::*;
use std::sync::Arc;

/// Domain-service stand-in proving the seam is implementable **without
/// any database**: a fake needs no connection, no path, no SQL (contract
/// §2.1 motivation).
struct FakeMeetingRepository;

#[allow(unused_variables)]
impl SessionRepository for FakeMeetingRepository {
    fn get_session(&self, id: Uuid) -> Result<Option<MeetingSession>, StoreError> {
        unimplemented!("fake")
    }

    fn list_sessions(&self, page: Page) -> Result<PageOf<MeetingSession>, StoreError> {
        unimplemented!("fake")
    }

    fn create_session(&self, draft: NewMeetingSession) -> Result<MeetingSession, StoreError> {
        unimplemented!("fake")
    }

    fn update_session(&self, id: Uuid, patch: SessionPatch) -> Result<MeetingSession, StoreError> {
        unimplemented!("fake")
    }

    fn soft_delete_session(&self, id: Uuid) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn segments_for_meeting(&self, meeting_id: Uuid) -> Result<Vec<TranscriptSegment>, StoreError> {
        unimplemented!("fake")
    }

    fn max_segment_sequence(&self, _meeting_id: Uuid) -> Result<i64, StoreError> {
        unimplemented!("fake")
    }

    fn insert_segments(
        &self,
        job_id: Uuid,
        segments: Vec<NewTranscriptSegment>,
    ) -> Result<Vec<TranscriptSegment>, StoreError> {
        unimplemented!("fake")
    }

    fn summary_for_meeting(&self, meeting_id: Uuid) -> Result<Option<MeetingSummary>, StoreError> {
        unimplemented!("fake")
    }

    fn upsert_summary(&self, summary: NewMeetingSummary) -> Result<MeetingSummary, StoreError> {
        unimplemented!("fake")
    }

    fn purge_meeting_content(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn delete_meeting_audio_rows(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
        unimplemented!("fake")
    }
}

#[allow(unused_variables)]
impl CaptureRepository for FakeMeetingRepository {
    fn append_chunk(&self, chunk: NewAudioChunk) -> Result<AudioChunk, StoreError> {
        unimplemented!("fake")
    }

    fn finalize_chunk(&self, id: Uuid, facts: ChunkFacts) -> Result<AudioChunk, StoreError> {
        unimplemented!("fake")
    }

    fn mark_chunk(
        &self,
        id: Uuid,
        state: ChunkState,
        reason: Option<String>,
    ) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn due_stt_jobs(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<SttJob>, StoreError> {
        unimplemented!("fake")
    }

    fn claim_due_stt_job(
        &self,
        worker: WorkerId,
        now: DateTime<Utc>,
    ) -> Result<Option<SttJobClaim>, StoreError> {
        unimplemented!("fake")
    }

    fn transition_stt_job(
        &self,
        id: Uuid,
        to: SttJobState,
        err: Option<String>,
    ) -> Result<(), StoreError> {
        let _ = (id, to, err);
        unimplemented!("fake")
    }

    fn reschedule_stt_job(
        &self,
        id: Uuid,
        next_attempt_at: DateTime<Utc>,
        err: Option<String>,
    ) -> Result<(), StoreError> {
        let _ = (id, next_attempt_at, err);
        unimplemented!("fake")
    }

    fn chunks_for_job(&self, job_id: Uuid) -> Result<Vec<AudioChunk>, StoreError> {
        let _ = job_id;
        unimplemented!("fake")
    }
}

#[allow(unused_variables)]
impl ProjectRepository for FakeMeetingRepository {
    fn create_project(&self, _draft: NewProject) -> Result<Project, StoreError> {
        unimplemented!("fake")
    }

    fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        unimplemented!("fake")
    }

    fn rename_project(&self, _id: Uuid, _name: String) -> Result<Project, StoreError> {
        unimplemented!("fake")
    }

    fn soft_delete_project(&self, _id: Uuid) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn assign_meeting_to_project(
        &self,
        _project_id: Uuid,
        _meeting_id: Uuid,
    ) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn unassign_meeting_from_project(
        &self,
        _project_id: Uuid,
        _meeting_id: Uuid,
    ) -> Result<(), StoreError> {
        unimplemented!("fake")
    }

    fn list_meeting_ids_for_project(&self, _project_id: Uuid) -> Result<Vec<String>, StoreError> {
        unimplemented!("fake")
    }

    fn list_unassigned_meeting_ids(&self) -> Result<Vec<String>, StoreError> {
        unimplemented!("fake")
    }

    fn delete_project_links_for_meeting(&self, _meeting_id: Uuid) -> Result<(), StoreError> {
        unimplemented!("fake")
    }
}

#[allow(unused_variables)]
impl SourceRepository for FakeMeetingRepository {
    fn create_chat_source(&self, _draft: NewChatSource) -> Result<(Source, bool), StoreError> {
        unimplemented!("fake")
    }

    fn list_sources(&self) -> Result<Vec<Source>, StoreError> {
        unimplemented!("fake")
    }

    fn list_source_items(&self, _source_id: Uuid) -> Result<Vec<SourceItem>, StoreError> {
        unimplemented!("fake")
    }

    fn attach_source_assets(
        &self,
        _source_id: Uuid,
        _assets: Vec<NewSourceAsset>,
    ) -> Result<Vec<SourceAsset>, StoreError> {
        unimplemented!("fake")
    }

    fn list_source_assets(&self, _source_id: Uuid) -> Result<Vec<SourceAsset>, StoreError> {
        unimplemented!("fake")
    }
}

#[allow(unused_variables)]
impl TaskRepository for FakeMeetingRepository {
    fn create_manual_task(&self, _draft: NewProjectTask) -> Result<ProjectTask, StoreError> {
        unimplemented!("fake")
    }

    fn suggest_task(&self, _draft: NewProjectTask) -> Result<(ProjectTask, bool), StoreError> {
        unimplemented!("fake")
    }

    fn list_tasks(&self, _project_id: Uuid) -> Result<Vec<ProjectTask>, StoreError> {
        unimplemented!("fake")
    }

    fn decide_task(
        &self,
        _id: Uuid,
        _state: &str,
        _title: Option<String>,
        _description: Option<String>,
    ) -> Result<ProjectTask, StoreError> {
        unimplemented!("fake")
    }

    fn create_snapshot(
        &self,
        _project_id: Uuid,
        _summary_markdown: String,
        _prompt_version: String,
        _model: Option<String>,
        _source_marker: String,
    ) -> Result<ProjectSnapshot, StoreError> {
        unimplemented!("fake")
    }

    fn list_snapshots(&self, _project_id: Uuid) -> Result<Vec<ProjectSnapshot>, StoreError> {
        unimplemented!("fake")
    }
}

/// The trait is object-safe: it can be used as `Arc<dyn MeetingRepository>`,
/// exactly the shape command handlers and services will take (contract
/// §2.1 rule).
#[test]
fn seam_is_object_safe_and_fake_substitutable_without_a_database() {
    let repo: Arc<dyn MeetingRepository> = Arc::new(FakeMeetingRepository);
    // Same handle flows through a clone, as `State<Arc<dyn …>>` would.
    let moved = Arc::clone(&repo);
    let _ = (repo, moved);
}

/// Boolean tombstone projection per §5.4.
#[test]
fn envelope_deleted_projection_follows_tombstone_marker() {
    let alive = ChangeEnvelope {
        id: Uuid("00000000-0000-4000-8000-000000000001".to_string()),
        revision: 1,
        updated_at: Utc::now(),
        deleted_at: None,
    };
    let mut gone = alive.clone();
    gone.deleted_at = Some(Utc::now());
    gone.revision = 2;
    assert!(!alive.deleted());
    assert!(gone.deleted());
}
