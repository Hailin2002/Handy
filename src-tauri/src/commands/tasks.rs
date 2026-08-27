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

/// Citations behind one AI task (plan §6.17 "Warum dringend?").
#[tauri::command]
#[specta::specta]
pub async fn task_evidence(
    project_manager: State<'_, Arc<ProjectManager>>,
    task_id: String,
) -> Result<Vec<crate::managers::meeting_store::Evidence>, String> {
    project_manager
        .store()
        .list_task_evidence(Uuid(task_id))
        .map_err(|e| e.to_string())
}

/// Citations behind one project status snapshot.
#[tauri::command]
#[specta::specta]
pub async fn snapshot_evidence(
    project_manager: State<'_, Arc<ProjectManager>>,
    snapshot_id: String,
) -> Result<Vec<crate::managers::meeting_store::Evidence>, String> {
    project_manager
        .store()
        .list_snapshot_evidence(Uuid(snapshot_id))
        .map_err(|e| e.to_string())
}

/// Runs one Phase 10 analysis for a project.
///
/// The summary provider is reused deliberately: the plan wants one deliberate
/// provider choice per run, not a second key mechanism. A project with no
/// material never spends a model call.
#[tauri::command]
#[specta::specta]
pub async fn project_analyse(
    app: tauri::AppHandle,
    project_manager: State<'_, Arc<ProjectManager>>,
    project_id: String,
) -> Result<ProjectAnalysisDto, String> {
    use crate::managers::project_intelligence::ProjectIntelligenceService;

    let store = project_manager.store();
    let project = store
        .list_projects()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "unknownProject".to_string())?;

    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| "noSummaryProvider".to_string())?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| "noSummaryModel".to_string())?;
    // Cloud runs stay behind the same explicit consent as meeting summaries; a
    // provider on this machine is not a cloud run.
    if !crate::managers::meeting::is_local_provider_url(&provider.base_url)
        && !settings.meeting_summary_cloud_consent
    {
        return Err("cloudConsentRequired".to_string());
    }
    let api_key =
        crate::secret_store::get_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider.id)
            .unwrap_or_default();
    let llm = crate::managers::meeting_summary::CloudSummarizer::new(
        provider.clone(),
        api_key,
        model.clone(),
    );

    let service = ProjectIntelligenceService::new(store);
    let outcome = service
        .analyse(&project_id, &project.name, &llm)
        .map_err(|e| e.to_string())?;

    Ok(ProjectAnalysisDto {
        snapshot: outcome.snapshot,
        task_count: outcome.tasks.len() as i64,
        rejected_without_evidence: outcome.rejected_without_evidence as i64,
        provider: provider.label,
        model,
    })
}

/// Result of one analysis run, as the UI reports it.
#[derive(serde::Serialize, specta::Type)]
pub struct ProjectAnalysisDto {
    pub snapshot: Option<ProjectSnapshot>,
    pub task_count: i64,
    /// Suggestions dropped for lack of a usable citation — surfaced, not hidden.
    pub rejected_without_evidence: i64,
    pub provider: String,
    pub model: String,
}
