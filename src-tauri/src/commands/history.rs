use crate::actions::{process_transcription_output, process_transcription_output_with_translation};
use crate::managers::{
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    azure_stream: State<'_, Arc<crate::speech_pipeline::AzureSpeechStreamManager>>,
    id: i64,
) -> Result<(), String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let audio_path = history_manager.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| format!("Failed to load audio: {}", e))?;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    let settings = crate::settings::get_settings(&app);
    let (transcription, live_translation) = match settings.speech_input_provider {
        crate::settings::SpeechInputProvider::Azure => {
            azure_stream
                .start(&settings)
                .map_err(|error| error.to_string())?;
            for chunk in samples.chunks(480) {
                azure_stream.feed(chunk);
            }
            match azure_stream
                .finalize_with_timeout(Duration::from_secs(10 * 60))
                .await
            {
                Ok(result) => (result.source_text, result.translated_text),
                Err(error) => {
                    azure_stream.cancel();
                    return Err(error.to_string());
                }
            }
        }
        crate::settings::SpeechInputProvider::Custom => {
            let text = crate::speech_pipeline::transcribe_cloud(&samples, &settings)
                .await
                .map_err(|error| error.to_string())?;
            (text, None)
        }
        crate::settings::SpeechInputProvider::Local => {
            transcription_manager.initiate_model_load();
            let tm = Arc::clone(&transcription_manager);
            let text = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
                .await
                .map_err(|e| format!("Transcription task panicked: {}", e))?
                .map_err(|e| e.to_string())?;
            (text, None)
        }
    };

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed = if live_translation.is_some() {
        process_transcription_output_with_translation(
            &app,
            &transcription,
            entry.post_process_requested,
            live_translation,
        )
        .await
    } else {
        process_transcription_output(&app, &transcription, entry.post_process_requested).await
    };
    history_manager
        .update_transcription(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    crate::settings::update_settings(&app, |settings| settings.history_limit = limit);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    crate::settings::update_settings(&app, |settings| {
        settings.recording_retention_period = retention_period
    });

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}
