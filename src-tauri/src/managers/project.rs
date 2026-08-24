//! Thin project manager (plan §5 `ProjectManager`, Phase 8): owns no state of
//! its own — every operation forwards to the shared meeting repository so the
//! projects tables live in the same SQLite file and migration chain.

use super::meeting_store::{MeetingRepository, NewProject, Project, StoreError, Uuid};
use std::sync::Arc;

#[derive(Clone)]
pub struct ProjectManager {
    store: Arc<dyn MeetingRepository>,
}

impl ProjectManager {
    pub fn new(store: Arc<dyn MeetingRepository>) -> Self {
        Self { store }
    }

    pub fn create(&self, name: String, description: String) -> Result<Project, StoreError> {
        self.store.create_project(NewProject { name, description })
    }

    pub fn list(&self) -> Result<Vec<Project>, StoreError> {
        self.store.list_projects()
    }

    pub fn rename(&self, id: &str, name: String) -> Result<Project, StoreError> {
        self.store.rename_project(Uuid(id.to_string()), name)
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.store.soft_delete_project(Uuid(id.to_string()))
    }

    pub fn assign_meeting(&self, project_id: &str, meeting_id: &str) -> Result<(), StoreError> {
        self.store
            .assign_meeting_to_project(Uuid(project_id.to_string()), Uuid(meeting_id.to_string()))
    }

    pub fn unassign_meeting(&self, project_id: &str, meeting_id: &str) -> Result<(), StoreError> {
        self.store.unassign_meeting_from_project(
            Uuid(project_id.to_string()),
            Uuid(meeting_id.to_string()),
        )
    }

    /// Meetings linked to one project (chronological by link insertion).
    pub fn meeting_ids(&self, project_id: &str) -> Result<Vec<String>, StoreError> {
        self.store
            .list_meeting_ids_for_project(Uuid(project_id.to_string()))
    }

    /// The "Eingang": processed meetings with no project assignment.
    pub fn inbox(&self) -> Result<Vec<String>, StoreError> {
        self.store.list_unassigned_meeting_ids()
    }
}
