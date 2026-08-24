//! Project task & snapshot commands (Phase 10).

use crate::managers::meeting_store::{NewProjectTask, ProjectSnapshot, ProjectTask, Uuid};
use crate::managers::project::ProjectManager;
use std::sync::Arc;
use tauri::State;

/// Manual task creation (origin `manual`, immediately accepted).
#[tauri::command]
#[specta::specta]
pub async fn task_create_manual(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    title: String,
    description: Option<String>,
) -> Result<ProjectTask, String> {
    project_manager
        .store()
        .create_manual_task(NewProjectTask {
            project_id: Uuid(project_id),
            title,
            description: description.unwrap_or_default(),
            origin: "manual",
        })
        .map_err(|e| e.to_string())
}

/// AI suggestion ingestion: insert-only, idempotent per content hash.
#[tauri::command]
#[specta::specta]
pub async fn task_suggest(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    title: String,
    description: Option<String>,
) -> Result<ProjectTask, String> {
    let (task, _created) = project_manager
        .store()
        .suggest_task(NewProjectTask {
            project_id: Uuid(project_id),
            title,
            description: description.unwrap_or_default(),
            origin: "ai",
        })
        .map_err(|e| e.to_string())?;
    Ok(task)
}

/// Alive tasks of one project.
#[tauri::command]
#[specta::specta]
pub async fn task_list(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
) -> Result<Vec<ProjectTask>, String> {
    project_manager
        .store()
        .list_tasks(Uuid(project_id))
        .map_err(|e| e.to_string())
}

/// Applies a user decision: `accepted | edited | rejected`.
#[tauri::command]
#[specta::specta]
pub async fn task_decide(
    project_manager: State<'_, Arc<ProjectManager>>,
    task_id: String,
    state: String,
    title: Option<String>,
    description: Option<String>,
) -> Result<ProjectTask, String> {
    project_manager
        .store()
        .decide_task(Uuid(task_id), &state, title, description)
        .map_err(|e| e.to_string())
}

/// Creates a new versioned snapshot for a project (append-only).
#[tauri::command]
#[specta::specta]
pub async fn snapshot_create(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
    summary_markdown: String,
    prompt_version: String,
    model: Option<String>,
    source_marker: String,
) -> Result<ProjectSnapshot, String> {
    project_manager
        .store()
        .create_snapshot(
            Uuid(project_id),
            summary_markdown,
            prompt_version,
            model,
            source_marker,
        )
        .map_err(|e| e.to_string())
}

/// Snapshots of one project, newest first.
#[tauri::command]
#[specta::specta]
pub async fn snapshot_list(
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
) -> Result<Vec<ProjectSnapshot>, String> {
    project_manager
        .store()
        .list_snapshots(Uuid(project_id))
        .map_err(|e| e.to_string())
}
