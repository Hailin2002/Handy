//! Project Tauri commands (Phase 8): CRUD plus meeting assignment. The
//! frontend lists meetings itself, so assignments exchange plain ids.

use crate::managers::project::ProjectManager;
use std::sync::Arc;
use tauri::State;

/// Creates a project and returns it.
#[tauri::command]
#[specta::specta]
pub async fn project_create(
    project_manager: State<'_, Arc<ProjectManager>>,
    name: String,
    description: Option<String>,
) -> Result<crate::managers::meeting_store::Project, String> {
    project_manager
        .create(name, description.unwrap_or_default())
        .map_err(|e| e.to_string())
}

/// Lists alive projects, alphabetical.
#[tauri::command]
#[specta::specta]
pub async fn project_list(
    project_manager: State<'_, Arc<ProjectManager>>,
) -> Result<Vec<crate::managers::meeting_store::Project>, String> {
    project_manager.list().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn project_rename(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    name: String,
) -> Result<crate::managers::meeting_store::Project, String> {
    project_manager.rename(&project_id, name).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn project_delete(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
) -> Result<(), String> {
    project_manager.delete(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn project_assign_meeting(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    meeting_id: String,
) -> Result<(), String> {
    project_manager
        .assign_meeting(&project_id, &meeting_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn project_unassign_meeting(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    meeting_id: String,
) -> Result<(), String> {
    project_manager
        .unassign_meeting(&project_id, &meeting_id)
        .map_err(|e| e.to_string())
}

/// Meeting ids linked to one project.
#[tauri::command]
#[specta::specta]
pub async fn project_meeting_ids(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
) -> Result<Vec<String>, String> {
    project_manager.meeting_ids(&project_id).map_err(|e| e.to_string())
}

/// The "Eingang": processed meetings without any project link.
#[tauri::command]
#[specta::specta]
pub async fn project_inbox_meeting_ids(
    project_manager: State<'_, Arc<ProjectManager>>,
) -> Result<Vec<String>, String> {
    project_manager.inbox().map_err(|e| e.to_string())
}
