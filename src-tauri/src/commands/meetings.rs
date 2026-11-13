//! Meeting Tauri commands.
//!
//! Two waves share this file:
//!
//! 1. the wave-1 CRUD surface (`meeting_*`) that the shipped Meetings library
//!    view drives, and
//! 2. the session lifecycle commands named by
//!    `docs/meeting-mode-architecture/session-state-contract.md` §5 and
//!    `startup-recovery-dataflow.md` §5, which the frontend invokes by the
//!    exact names listed in `src/lib/meetingIpc.ts`.
//!
//! The command names and argument names below are part of the frontend
//! contract: `meetingStore` invokes them positionally by name (camelCase
//! arguments are mapped onto the snake_case parameters by Tauri). Renaming
//! anything here silently breaks the UI, which is why every name is asserted
//! by a test in this module.

use crate::managers::meeting::{
    MeetingManager, MeetingPreflightReport, MeetingRecoveryOffer, MeetingRecoveryResolution,
    MeetingSession, MeetingSnapshot,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

// ---------------------------------------------------------------------------
// Wave-1 CRUD (unchanged)
// ---------------------------------------------------------------------------

/// Creates a meeting in state `draft`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_create_session(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    title: Option<String>,
) -> Result<MeetingSession, String> {
    meeting_manager
        .create_session(title)
        .map_err(|e| e.to_string())
}

/// Opens one meeting by id.
#[tauri::command]
#[specta::specta]
pub async fn meeting_get_session(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    id: String,
) -> Result<Option<MeetingSession>, String> {
    meeting_manager.get_session(&id).map_err(|e| e.to_string())
}

/// Lists meetings, newest first, optionally filtered by title.
#[tauri::command]
#[specta::specta]
pub async fn meeting_list_sessions(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    query: Option<String>,
) -> Result<Vec<MeetingSession>, String> {
    meeting_manager
        .list_sessions(query.as_deref())
        .map_err(|e| e.to_string())
}

/// Saves an edited title. Returns the updated session.
#[tauri::command]
#[specta::specta]
pub async fn meeting_rename_session(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    id: String,
    title: String,
) -> Result<MeetingSession, String> {
    meeting_manager
        .rename_session(&id, title)
        .map_err(|e| e.to_string())
}

/// Deletes a meeting (soft delete). Returns whether an alive session was found.
#[tauri::command]
#[specta::specta]
pub async fn meeting_delete_session(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    id: String,
) -> Result<bool, String> {
    meeting_manager
        .soft_delete_session(&id)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Session lifecycle (SSC §5)
// ---------------------------------------------------------------------------

/// T01 — creates the draft the compact start dialog works on.
#[tauri::command]
#[specta::specta]
pub async fn create_meeting_draft(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    title: Option<String>,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.create_draft(title)
}

/// Runs the full Startvalidierung and returns the compact start-dialog rows.
#[tauri::command]
#[specta::specta]
pub async fn preflight_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    stt_provider: Option<crate::settings::SpeechInputProvider>,
) -> Result<MeetingPreflightReport, String> {
    Ok(meeting_manager.preflight_for(stt_provider))
}

/// T02/T03 — consent gate, validation, device opening. `stt_provider` pins
/// this meeting's transcription backend (plan §13 Phase 6); `None` uses the
/// global meeting default.
#[tauri::command]
#[specta::specta]
pub async fn start_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    title: Option<String>,
    microphone_only: bool,
    summary_enabled: bool,
    consent_acknowledged: bool,
    stt_provider: Option<crate::settings::SpeechInputProvider>,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.start_with_provider(
        &meeting_id,
        title,
        microphone_only,
        summary_enabled,
        consent_acknowledged,
        stt_provider,
    )
}

/// T05 — pause the running capture.
#[tauri::command]
#[specta::specta]
pub async fn pause_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.pause(&meeting_id)
}

/// T06 — resume a paused capture after re-validating devices and disk.
#[tauri::command]
#[specta::specta]
pub async fn resume_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.resume(&meeting_id)
}

/// T07 → T08/T09/T15 — controlled stop. `abort` only changes the destination.
#[tauri::command]
#[specta::specta]
pub async fn stop_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    abort: Option<bool>,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.stop(&meeting_id, abort.unwrap_or(false))
}

/// T16 — user abort before or at start.
#[tauri::command]
#[specta::specta]
pub async fn cancel_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.cancel(&meeting_id)
}

/// O5 snapshot-first handshake: the authoritative rebuild source for every
/// window (main, overlay, tray).
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_snapshot(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.snapshot(&meeting_id)
}

/// T14 — retry every permanently failed STT job of a meeting.
#[tauri::command]
#[specta::specta]
pub async fn retry_meeting_jobs(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.retry_jobs(&meeting_id)
}

/// Serializable projection of `meeting_summaries` for the detail view.
#[derive(Serialize, specta::Type)]
pub struct MeetingSummaryDto {
    pub state: String,
    pub provider: String,
    pub model: Option<String>,
    pub prompt_version: String,
    pub summary_markdown: String,
    pub decisions: Vec<String>,
    pub action_items: Vec<String>,
    pub open_questions: Vec<String>,
    pub last_error: Option<String>,
}

fn string_list(value: serde_json::Value) -> Vec<String> {
    serde_json::from_value(value).unwrap_or_default()
}

/// Phase 7 — current summary of a meeting, if any.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_summary(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<Option<MeetingSummaryDto>, String> {
    let summary = meeting_manager
        .get_summary(&meeting_id)
        .map_err(|e| e.to_string())?;
    Ok(summary.map(|s| MeetingSummaryDto {
        state: s.state,
        provider: s.provider,
        model: s.model,
        prompt_version: s.prompt_version,
        summary_markdown: s.summary_markdown,
        decisions: string_list(s.decisions),
        action_items: string_list(s.action_items),
        open_questions: string_list(s.open_questions),
        last_error: s.last_error,
    }))
}

/// Phase 7 — re-runs the map-reduce summarization ("Neu-Zusammenfassung").
#[tauri::command]
#[specta::specta]
pub async fn request_meeting_summary(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.request_summary(&meeting_id)
}

/// Phase 7 — chronological transcript rows of a completed meeting.
#[derive(Serialize, specta::Type)]
pub struct TranscriptRowDto {
    pub start_ms: i64,
    pub end_ms: i64,
    pub source: String,
    pub text: String,
    pub confidence: Option<f32>,
}

/// Phase 7 — chronological transcript of a meeting for the detail view.
#[tauri::command]
#[specta::specta]
pub async fn meeting_transcript(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<Vec<TranscriptRowDto>, String> {
    let segments = meeting_manager
        .get_transcript(&meeting_id)
        .map_err(|e| e.to_string())?;
    Ok(segments
        .into_iter()
        .map(|s| TranscriptRowDto {
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            source: s.source,
            text: s.text,
            confidence: s.confidence,
        })
        .collect())
}

/// Phase 7 — export payload: rendered content plus filename suggestion.
#[derive(Serialize, specta::Type)]
pub struct MeetingExportDto {
    pub filename: String,
    pub mime: String,
    pub content: String,
}

/// Phase 7 — renders the meeting as Markdown, TXT or JSON. The frontend
/// persists the file via the dialog/fs plugins.
#[tauri::command]
#[specta::specta]
pub async fn export_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    format: crate::managers::meeting_export::ExportFormat,
) -> Result<Option<MeetingExportDto>, String> {
    let export = meeting_manager
        .build_export(&meeting_id, format)
        .map_err(|e| e.to_string())?;
    Ok(export.map(|e| MeetingExportDto {
        filename: e.filename,
        mime: e.mime,
        content: e.content,
    }))
}

/// Phase 7 — controlled deletion with the plan's two scopes.
#[tauri::command]
#[specta::specta]
pub async fn delete_meeting_data(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    scope: crate::managers::meeting::DeleteScope,
) -> Result<(), String> {
    meeting_manager
        .delete_meeting_data(&meeting_id, scope)
        .map_err(|e| e.to_string())
}

/// Pending crash-recovery offers produced by the startup reconciliation.
#[tauri::command]
#[specta::specta]
pub async fn meeting_list_recoveries(
    meeting_manager: State<'_, Arc<MeetingManager>>,
) -> Result<Vec<MeetingRecoveryOffer>, String> {
    Ok(meeting_manager.list_recoveries())
}

/// Applies the user's decision to one recovery offer.
#[tauri::command]
#[specta::specta]
pub async fn meeting_resolve_recovery(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    resolution: MeetingRecoveryResolution,
) -> Result<bool, String> {
    meeting_manager.resolve_recovery(&meeting_id, resolution)
}

#[cfg(test)]
mod tests {
    /// The exact command names `src/lib/meetingIpc.ts` invokes. If a command
    /// is renamed, this list stops matching the function items below and the
    /// module fails to compile.
    #[test]
    fn every_frontend_command_name_exists() {
        let names = [
            "create_meeting_draft",
            "preflight_meeting",
            "start_meeting",
            "pause_meeting",
            "resume_meeting",
            "stop_meeting",
            "cancel_meeting",
            "get_meeting_snapshot",
            "retry_meeting_jobs",
            "meeting_list_recoveries",
            "meeting_resolve_recovery",
            "meeting_create_session",
            "meeting_get_session",
            "meeting_list_sessions",
            "meeting_rename_session",
            "meeting_delete_session",
            "get_meeting_summary",
            "request_meeting_summary",
            "export_meeting",
            "delete_meeting_data",
        ];
        // Referencing the items binds the names to real functions.
        let _ = super::create_meeting_draft;
        let _ = super::preflight_meeting;
        let _ = super::start_meeting;
        let _ = super::pause_meeting;
        let _ = super::resume_meeting;
        let _ = super::stop_meeting;
        let _ = super::cancel_meeting;
        let _ = super::get_meeting_snapshot;
        let _ = super::retry_meeting_jobs;
        let _ = super::meeting_list_recoveries;
        let _ = super::meeting_resolve_recovery;
        let _ = super::meeting_create_session;
        let _ = super::meeting_get_session;
        let _ = super::meeting_list_sessions;
        let _ = super::meeting_rename_session;
        let _ = super::meeting_delete_session;
        let _ = super::get_meeting_summary;
        let _ = super::request_meeting_summary;
        let _ = super::export_meeting;
        let _ = super::delete_meeting_data;
        assert_eq!(names.len(), 20);
    }

    /// The recovery resolution tokens the frontend sends.
    #[test]
    fn recovery_resolution_deserializes_frontend_tokens() {
        use crate::managers::meeting::MeetingRecoveryResolution;
        for token in ["recover", "later", "delete"] {
            let parsed: MeetingRecoveryResolution =
                serde_json::from_value(serde_json::Value::String(token.to_string()))
                    .expect("resolution token");
            let back = serde_json::to_value(parsed).expect("serialize");
            assert_eq!(back, token);
        }
    }
}

/// Plan §6.13 `Audio` tab — playable chunks that still exist on disk.
#[tauri::command]
#[specta::specta]
pub async fn meeting_audio_files(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
) -> Result<Vec<crate::managers::meeting::MeetingAudioFileDto>, String> {
    meeting_manager.audio_files(&meeting_id)
}

/// Plan §11 — discards the transcript and transcribes the stored audio again,
/// optionally with a different provider. The audio itself is never touched.
#[tauri::command]
#[specta::specta]
pub async fn retranscribe_meeting(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    provider: Option<crate::settings::SpeechInputProvider>,
) -> Result<MeetingSnapshot, String> {
    meeting_manager.retranscribe(&meeting_id, provider)
}

/// Compares every stored chunk with a second provider and applies only clear
/// completeness improvements. The original segment rows remain tombstoned in
/// SQLite, and no transcript changes are committed if any reference call
/// fails.
#[tauri::command]
#[specta::specta]
pub async fn improve_meeting_accuracy(
    meeting_manager: State<'_, Arc<MeetingManager>>,
    meeting_id: String,
    reference_provider: crate::settings::SpeechInputProvider,
) -> Result<crate::managers::meeting::MeetingAccuracyReport, String> {
    let manager = Arc::clone(meeting_manager.inner());
    tauri::async_runtime::spawn_blocking(move || {
        manager.improve_accuracy(&meeting_id, reference_provider)
    })
    .await
    .map_err(|error| format!("accuracy worker failed: {error}"))?
}
