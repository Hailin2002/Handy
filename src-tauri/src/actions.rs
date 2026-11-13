#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::apple_intelligence;
use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error, VadPolicy};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::managers::transcription::StreamWorkKind;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{
    get_settings, AppSettings, AssistantOutputMode, OverlayStyle, SpeechInputProvider,
    APPLE_INTELLIGENCE_PROVIDER_ID,
};
use crate::shortcut;
use crate::tray::{change_tray_icon, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use crate::TranscriptionCoordinator;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, Emitter};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
struct FinishGuard(AppHandle);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        if let Some(c) = self.0.try_state::<TranscriptionCoordinator>() {
            c.notify_processing_finished();
        }
        // The pipeline just freed its large transient buffers (captured PCM,
        // WAV copy, engine scratch); hand the cached pages back to the OS so
        // they don't sit in malloc arenas until they get swapped out (#1792).
        crate::memory::trim_freed_memory();
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Transcribe Action
struct TranscribeAction {
    post_process: bool,
}

/// Field name for structured output JSON schema
const TRANSCRIPTION_FIELD: &str = "transcription";

/// Strip invisible Unicode characters that some LLMs may insert
pub(crate) fn strip_invisible_chars(s: &str) -> String {
    s.replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
}

fn azure_recovery_is_better(streamed: &str, recovered: &str) -> bool {
    let visible_len = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .count()
    };
    visible_len(recovered) > visible_len(streamed).saturating_add(8)
}

/// Strip a leading `<think>...</think>` block. Some endpoints can't disable
/// reasoning, and some local servers put the reasoning text into `content`
/// instead of a separate field — without this the user would get the model's
/// chain of thought pasted along with the cleaned transcription.
pub(crate) fn strip_think_block(s: &str) -> &str {
    if let Some(rest) = s.trim_start().strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim_start();
        }
    }
    s
}

/// Build a system prompt from the user's prompt template.
/// Removes `${output}` placeholder since the transcription is sent as the user message.
fn build_system_prompt(prompt_template: &str) -> String {
    prompt_template.replace("${output}", "").trim().to_string()
}

/// Returns `true` when a transcription has no meaningful content to
/// post-process (empty or whitespace-only). Used to skip the post-processing
/// LLM call when nothing was actually transcribed, which would otherwise make
/// the model reply with an error message such as "you need to provide the
/// transcription".
pub(crate) fn is_blank_transcription(transcription: &str) -> bool {
    transcription.trim().is_empty()
}

async fn complete_unless_cancelled<F, C>(operation: F, is_cancelled: C) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}

fn should_use_streaming_overlay(style: OverlayStyle, is_streaming: bool) -> bool {
    style == OverlayStyle::Live && is_streaming
}

async fn post_process_transcription(settings: &AppSettings, transcription: &str) -> Option<String> {
    if is_blank_transcription(transcription) {
        debug!("Post-processing skipped because the transcription is empty");
        return None;
    }

    let provider = match settings.active_post_process_provider().cloned() {
        Some(provider) => provider,
        None => {
            debug!("Post-processing enabled but no provider is selected");
            return None;
        }
    };

    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    if model.trim().is_empty() {
        debug!(
            "Post-processing skipped because provider '{}' has no model configured",
            provider.id
        );
        return None;
    }

    let selected_prompt_id = match &settings.post_process_selected_prompt_id {
        Some(id) => id.clone(),
        None => {
            debug!("Post-processing skipped because no prompt is selected");
            return None;
        }
    };

    let prompt = match settings
        .post_process_prompts
        .iter()
        .find(|prompt| prompt.id == selected_prompt_id)
    {
        Some(prompt) => prompt.prompt.clone(),
        None => {
            debug!(
                "Post-processing skipped because prompt '{}' was not found",
                selected_prompt_id
            );
            return None;
        }
    };

    if prompt.trim().is_empty() {
        debug!("Post-processing skipped because the selected prompt is empty");
        return None;
    }

    debug!(
        "Starting LLM post-processing with provider '{}' (model: {})",
        provider.id, model
    );

    let api_key =
        crate::secret_store::get_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider.id)
            .unwrap_or_default();

    // Ask these providers to skip reasoning/thinking — post-processing rarely
    // benefits from it and it adds seconds of latency. llm_client picks the
    // field the endpoint understands and retries without it if rejected.
    let disable_reasoning = matches!(provider.id.as_str(), "custom" | "openrouter");

    if provider.supports_structured_output {
        debug!("Using structured outputs for provider '{}'", provider.id);

        let system_prompt = build_system_prompt(&prompt);
        let user_content = transcription.to_string();

        // Handle Apple Intelligence separately since it uses native Swift APIs
        if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                if !apple_intelligence::check_apple_intelligence_availability() {
                    debug!(
                        "Apple Intelligence selected but not currently available on this device"
                    );
                    return None;
                }

                let token_limit = model.trim().parse::<i32>().unwrap_or(0);
                return match apple_intelligence::process_text_with_system_prompt(
                    &system_prompt,
                    &user_content,
                    token_limit,
                ) {
                    Ok(result) => {
                        if result.trim().is_empty() {
                            debug!("Apple Intelligence returned an empty response");
                            None
                        } else {
                            let result = strip_invisible_chars(&result);
                            debug!(
                                "Apple Intelligence post-processing succeeded. Output length: {} chars",
                                result.len()
                            );
                            Some(result)
                        }
                    }
                    Err(err) => {
                        error!("Apple Intelligence post-processing failed: {}", err);
                        None
                    }
                };
            }

            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                debug!("Apple Intelligence provider selected on unsupported platform");
                return None;
            }
        }

        // Define JSON schema for transcription output
        let json_schema = serde_json::json!({
            "type": "object",
            "properties": {
                (TRANSCRIPTION_FIELD): {
                    "type": "string",
                    "description": "The cleaned and processed transcription text"
                }
            },
            "required": [TRANSCRIPTION_FIELD],
            "additionalProperties": false
        });

        match crate::llm_client::send_chat_completion_with_schema(
            &provider,
            api_key.clone(),
            &model,
            user_content,
            Some(system_prompt),
            Some(json_schema),
            disable_reasoning,
        )
        .await
        {
            Ok(Some(content)) => {
                // Parse the JSON response to extract the transcription field
                let content = strip_think_block(&content);
                match serde_json::from_str::<serde_json::Value>(content) {
                    Ok(json) => {
                        if let Some(transcription_value) =
                            json.get(TRANSCRIPTION_FIELD).and_then(|t| t.as_str())
                        {
                            let result = strip_invisible_chars(transcription_value);
                            debug!(
                                "Structured output post-processing succeeded for provider '{}'. Output length: {} chars",
                                provider.id,
                                result.len()
                            );
                            return Some(result);
                        } else {
                            error!("Structured output response missing 'transcription' field");
                            return Some(strip_invisible_chars(content));
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to parse structured output JSON: {}. Returning raw content.",
                            e
                        );
                        return Some(strip_invisible_chars(content));
                    }
                }
            }
            Ok(None) => {
                error!("LLM API response has no content");
                return None;
            }
            Err(e) => {
                warn!(
                    "Structured output failed for provider '{}': {}. Falling back to legacy mode.",
                    provider.id, e
                );
                // Fall through to legacy mode below
            }
        }
    }

    // Legacy mode: Replace ${output} variable in the prompt with the actual text
    let processed_prompt = prompt.replace("${output}", transcription);
    debug!("Processed prompt length: {} chars", processed_prompt.len());

    match crate::llm_client::send_chat_completion(
        &provider,
        api_key,
        &model,
        processed_prompt,
        disable_reasoning,
    )
    .await
    {
        Ok(Some(content)) => {
            let content = strip_invisible_chars(strip_think_block(&content));
            debug!(
                "LLM post-processing succeeded for provider '{}'. Output length: {} chars",
                provider.id,
                content.len()
            );
            Some(content)
        }
        Ok(None) => {
            error!("LLM API response has no content");
            None
        }
        Err(e) => {
            error!(
                "LLM post-processing failed for provider '{}': {}. Falling back to original transcription.",
                provider.id,
                e
            );
            None
        }
    }
}

async fn maybe_convert_chinese_variant(
    effective_language: &str,
    transcription: &str,
) -> Option<String> {
    // Gate on the language the model actually transcribed in (the effective
    // language), not the persisted intent. A leftover zh-Hans/zh-Hant intent
    // from a previously selected model must not run OpenCC S2T/T2S over output a
    // non-Chinese model produced — that would silently rewrite any shared CJK
    // characters (e.g. Japanese kanji) in the result.
    let is_simplified = effective_language == "zh-Hans";
    let is_traditional = effective_language == "zh-Hant";

    if !is_simplified && !is_traditional {
        debug!("effective language is not Simplified or Traditional Chinese; skipping conversion");
        return None;
    }

    debug!(
        "Starting Chinese variant conversion using OpenCC for language: {}",
        effective_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2tw
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

pub(crate) struct ProcessedTranscription {
    pub final_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
}

/// Resolve the persisted language *intent* into the language the currently-loaded
/// model will actually use — the same capability-aware coercion the transcription
/// paths apply (see [`crate::managers::model::effective_language`]). Post-processing
/// resolves it independently so it agrees with the language the transcription ran
/// in, without threading a value through the pipeline.
fn resolve_effective_language(app: &AppHandle, settings: &AppSettings) -> String {
    let tm = app.state::<Arc<TranscriptionManager>>();
    let model_manager = app.state::<Arc<ModelManager>>();
    let active_model = tm
        .get_current_model()
        .unwrap_or_else(|| settings.selected_model.clone());
    match model_manager.get_model_info(&active_model) {
        Some(info) => crate::managers::model::effective_language(
            &settings.selected_language,
            &info.supported_languages,
            info.supports_language_detection,
        ),
        None => settings.selected_language.clone(),
    }
}

pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    post_process: bool,
) -> ProcessedTranscription {
    process_transcription_output_with_translation(app, transcription, post_process, None).await
}

pub(crate) async fn process_transcription_output_with_translation(
    app: &AppHandle,
    transcription: &str,
    post_process: bool,
    live_translation: Option<String>,
) -> ProcessedTranscription {
    let settings = get_settings(app);
    let mut final_text = transcription.to_string();
    let mut post_processed_text: Option<String> = None;
    let mut post_process_prompt: Option<String> = None;

    // Resolve the language the transcription actually ran in (the persisted
    // intent coerced against the loaded model's capabilities) so OpenCC keys off
    // the effective language rather than a possibly-stale intent.
    let effective_language = resolve_effective_language(app, &settings);
    if let Some(converted_text) =
        maybe_convert_chinese_variant(&effective_language, transcription).await
    {
        final_text = converted_text;
    }

    if let Some(translated) = live_translation.filter(|text| !text.trim().is_empty()) {
        post_processed_text = Some(translated.clone());
        final_text = translated;
    } else {
        match crate::speech_pipeline::maybe_translate_with_deepl(&final_text, &settings).await {
            Ok(Some(translated)) => {
                post_processed_text = Some(translated.clone());
                final_text = translated;
            }
            Ok(None) => {}
            Err(err) => {
                warn!("DeepL translation failed; using the source transcript: {err}");
                let _ = app.emit(
                    crate::events::TRANSCRIPTION_ERROR,
                    format!("DeepL translation failed: {err}"),
                );
            }
        }
    }

    if post_process {
        if let Some(processed_text) = post_process_transcription(&settings, &final_text).await {
            post_processed_text = Some(processed_text.clone());
            final_text = processed_text;

            if let Some(prompt_id) = &settings.post_process_selected_prompt_id {
                if let Some(prompt) = settings
                    .post_process_prompts
                    .iter()
                    .find(|prompt| &prompt.id == prompt_id)
                {
                    post_process_prompt = Some(prompt.prompt.clone());
                }
            }
        }
    } else if post_processed_text.is_none() && final_text != transcription {
        post_processed_text = Some(final_text.clone());
    }

    ProcessedTranscription {
        final_text,
        post_processed_text,
        post_process_prompt,
    }
}

/// The recording-start sequence shared by every action that captures audio.
///
/// Model-load kickoff, VAD preload, tray state, the VAD/streaming policy, the
/// overlay, the Azure stream, `try_start_recording`, the cancel shortcut and the
/// readiness thread that emits the ready cue and applies ducking. There is no
/// dictation-specific branch in here — `TranscribeAction::start` never read
/// `self.post_process` — so the assistants get a byte-identical start by calling
/// this rather than reimplementing it.
fn start_recording_session(app: &AppHandle, binding_id: &str) {
    let start_time = Instant::now();
    debug!("start_recording_session called for binding: {}", binding_id);

    // Load model in the background
    let tm = app.state::<Arc<TranscriptionManager>>();
    let rm = app.state::<Arc<AudioRecordingManager>>();
    let azure_stream = app.state::<Arc<crate::speech_pipeline::AzureSpeechStreamManager>>();
    let settings = get_settings(app);

    // Load ASR model and VAD model in parallel
    let kickoff_started = Instant::now();
    if settings.speech_input_provider == SpeechInputProvider::Local {
        tm.initiate_model_load();
    }
    let rm_clone = Arc::clone(&rm);
    std::thread::spawn(move || {
        if let Err(e) = rm_clone.preload_vad() {
            debug!("VAD pre-load failed: {}", e);
        }
    });
    let kickoff_elapsed = kickoff_started.elapsed();

    let binding_id = binding_id.to_string();
    let tray_started = Instant::now();
    change_tray_icon(app, TrayIconState::Recording);
    let tray_elapsed = tray_started.elapsed();

    // Get the microphone mode to determine audio feedback timing
    let plan_started = Instant::now();
    let is_always_on = settings.always_on_microphone;

    let selected_model_info = if settings.speech_input_provider == SpeechInputProvider::Local {
        app.state::<Arc<ModelManager>>()
            .get_model_info(&settings.selected_model)
    } else {
        None
    };

    // Use the app-facing model capability as the single pre-recording source
    // for live streaming decisions. Unknown support is represented as false
    // until the model registry is updated by discovery or runtime load.
    let model_supports_streaming = selected_model_info
        .as_ref()
        .map(|m| m.supports_streaming)
        .unwrap_or(false);
    let azure_supports_streaming = settings.speech_input_provider == SpeechInputProvider::Azure;
    let supports_live_stream = model_supports_streaming || azure_supports_streaming;
    let vad_policy = if !settings.vad_enabled {
        VadPolicy::Disabled
    } else if azure_supports_streaming {
        // Azure performs its own endpointing. Preserve real silence so the
        // service can finalize phrases reliably instead of concatenating
        // VAD-filtered speech into one artificial utterance.
        VadPolicy::Disabled
    } else if model_supports_streaming {
        VadPolicy::Streaming
    } else {
        VadPolicy::Offline
    };
    if model_supports_streaming {
        tm.start_stream();
    }
    let plan_elapsed = plan_started.elapsed();

    // Sizing the overlay follows the same advertised capability. A model that
    // doesn't stream (or whose capability is not known yet) gets the compact
    // pill instead of an oversized transparent live window.
    let overlay_started = Instant::now();
    match settings.overlay_style {
        OverlayStyle::Live if supports_live_stream => utils::show_streaming_overlay(app),
        OverlayStyle::Live | OverlayStyle::Minimal => show_recording_overlay(app),
        OverlayStyle::None => {} // show_overlay_state no-ops on None anyway
    }
    // Everything above runs before capture can begin, so each span here is
    // added keypress->capture latency.
    debug!(
            "start-path pre-recording steps: model_kickoff={:?} tray={:?} settings+stream_plan={:?} overlay={:?}",
            kickoff_elapsed,
            tray_elapsed,
            plan_elapsed,
            overlay_started.elapsed()
        );
    debug!("Microphone mode - always_on: {}", is_always_on);

    let mut recording_error: Option<String> = if azure_supports_streaming {
        azure_stream
            .start(&settings)
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    let recording_start_time = Instant::now();
    let recording_start = if recording_error.is_none() {
        Some(rm.try_start_recording(&binding_id, vad_policy))
    } else {
        None
    };
    match recording_start {
        Some(Ok(readiness)) => {
            debug!(
                "Recording request accepted in {:?}; waiting for first microphone samples",
                recording_start_time.elapsed()
            );
            let generation = readiness.generation();
            let app_clone = app.clone();
            let rm_clone = Arc::clone(&rm);
            std::thread::spawn(move || {
                if !readiness.wait() {
                    debug!("Microphone readiness wait ended without receiving samples");
                    return;
                }

                // Development-only preview hook for evaluating the brief
                // arming animation on hardware that normally starts too fast
                // to make it visible.
                #[cfg(debug_assertions)]
                if let Ok(delay_ms) = std::env::var("HANDY_DEBUG_MIC_READY_DELAY_MS")
                    .unwrap_or_default()
                    .parse::<u64>()
                {
                    let delay_ms = delay_ms.min(10_000);
                    if delay_ms > 0 {
                        debug!("Delaying microphone-ready cue by {delay_ms}ms for UI preview");
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }

                if !rm_clone.is_recording_readiness_current(generation) {
                    debug!("Microphone became ready for an inactive recording");
                    return;
                }

                debug!("Microphone is receiving samples; recording is ready");
                utils::emit_recording_ready(&app_clone);

                // The start chime is a readiness cue, so it must follow the
                // first real input callback rather than Stream::play() or a
                // fixed delay. The helper returns immediately when feedback
                // is disabled; mute still follows the same readiness point.
                if rm_clone.is_recording_readiness_current(generation) {
                    play_feedback_sound_blocking(&app_clone, SoundType::Start);
                }
                if rm_clone.is_recording_readiness_current(generation) {
                    rm_clone.apply_recording_audio(generation);
                }
            });
        }
        Some(Err(e)) => {
            debug!("Failed to start recording: {}", e);
            recording_error = Some(e);
        }
        None => {}
    }

    if recording_error.is_none() {
        // Dynamically register the cancel shortcut in a separate task to avoid deadlock
        shortcut::register_cancel_shortcut(app);
    } else {
        // Starting failed (for example due to blocked microphone permissions).
        // Revert UI state so we don't stay stuck in the recording overlay.
        tm.cancel_stream();
        azure_stream.cancel();
        utils::hide_recording_overlay(app);
        change_tray_icon(app, TrayIconState::Idle);
        if let Some(err) = recording_error {
            let error_type = if is_microphone_access_denied(&err) {
                "microphone_permission_denied"
            } else if is_no_input_device_error(&err) {
                "no_input_device"
            } else {
                "unknown"
            };
            let _ = app.emit(
                "recording-error",
                RecordingErrorEvent {
                    error_type: error_type.to_string(),
                    detail: Some(err),
                },
            );
        }
    }

    debug!(
        "start_recording_session completed in {:?}",
        start_time.elapsed()
    );
}

/// Obtain the transcript for a finished recording.
///
/// This is the whole `speech_input_provider` fork — local stream finalize with
/// batch fallback, Azure continuous recognition with its two recovery paths, and
/// the generic cloud path — lifted verbatim out of `TranscribeAction::stop` so
/// the assistant actions reach a transcript through the identical code rather
/// than a second, subtly different copy of it.
///
/// Returns the transcript alongside any translation the live stream already
/// produced; a recovery that replaced the streamed text also drops that
/// translation, because it only ever covered the incomplete stream.
async fn acquire_transcript(
    tm: &Arc<TranscriptionManager>,
    azure_stream: &Arc<crate::speech_pipeline::AzureSpeechStreamManager>,
    settings: &AppSettings,
    samples: Vec<f32>,
) -> (anyhow::Result<String>, Option<String>) {
    if settings.speech_input_provider == SpeechInputProvider::Local {
        match tm.finalize_stream() {
            // A finalized stream with usable text wins. An empty result
            // (no active stream, produced nothing, or a finalize error
            // after the engine was returned) falls back to a full batch
            // transcription of the same audio. A finalize timeout is
            // surfaced instead — the worker may still hold the engine,
            // so a batch fallback would contend with it.
            Ok(Some(text)) if !text.trim().is_empty() => (Ok(text), None),
            Ok(_) => (tm.transcribe(samples), None),
            Err(err) => (Err(err), None),
        }
    } else if settings.speech_input_provider == SpeechInputProvider::Azure {
        tm.cancel_stream();
        match azure_stream.finalize().await {
            Ok(result) => {
                if crate::speech_pipeline::azure_stream_needs_recovery(
                    &samples,
                    &result.source_text,
                    result.recognized_audio_end_ms,
                ) {
                    warn!(
                            "Azure continuous result looks incomplete ({} chars, coverage through {}ms for {:.1}s); retrying from the complete recording",
                            result.source_text.chars().count(),
                            result.recognized_audio_end_ms,
                            samples.len() as f64 / 16_000.0
                        );
                    match crate::speech_pipeline::transcribe_azure_recording(&samples, settings)
                        .await
                    {
                        Ok(recovered)
                            if azure_recovery_is_better(&result.source_text, &recovered) =>
                        {
                            debug!(
                                    "Azure recording recovery replaced {} streamed chars with {} recovered chars",
                                    result.source_text.chars().count(),
                                    recovered.chars().count()
                                );
                            // Any live DeepL output only covers the
                            // incomplete stream. Passing None makes the
                            // normal output stage translate the full text.
                            (Ok(recovered), None)
                        }
                        Ok(recovered) => {
                            debug!(
                                    "Azure recording recovery was not more complete ({} chars); keeping the streamed result",
                                    recovered.chars().count()
                                );
                            (Ok(result.source_text), result.translated_text)
                        }
                        Err(error) => {
                            warn!(
                                    "Azure recording recovery failed; keeping the streamed result: {error}"
                                );
                            (Ok(result.source_text), result.translated_text)
                        }
                    }
                } else {
                    (Ok(result.source_text), result.translated_text)
                }
            }
            Err(stream_error) => {
                warn!(
                        "Azure continuous recognition failed; retrying from the complete recording: {stream_error}"
                    );
                match crate::speech_pipeline::transcribe_azure_recording(&samples, settings).await {
                    Ok(recovered) => (Ok(recovered), None),
                    Err(recovery_error) => {
                        warn!("Azure recording recovery also failed: {recovery_error}");
                        (Err(stream_error), None)
                    }
                }
            }
        }
    } else {
        tm.cancel_stream();
        (
            crate::speech_pipeline::transcribe_cloud(&samples, settings).await,
            None,
        )
    }
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        start_recording_session(app, binding_id);
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        // Prevent a slow microphone from emitting a ready event or start chime
        // after the user has already requested stop.
        app.state::<Arc<AudioRecordingManager>>()
            .invalidate_recording_readiness();

        // Unregister the cancel shortcut when transcription stops
        shortcut::unregister_cancel_shortcut(app);

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let azure_stream =
            Arc::clone(&app.state::<Arc<crate::speech_pipeline::AzureSpeechStreamManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);
        // Stop should give immediate visual feedback. Live streaming can keep
        // the larger panel, but it still switches from listening to a working
        // spinner while the stream finalizes. Non-streaming paths use the
        // compact transcribing pill (None no-ops in show_*).
        let settings = get_settings(app);
        let style = settings.overlay_style;
        // Capture this before finalizing the stream so every later working state
        // targets the same overlay that was shown for this transcription.
        let use_streaming_overlay =
            should_use_streaming_overlay(style, tm.is_streaming() || azure_stream.is_active());
        if use_streaming_overlay {
            tm.emit_stream_working(StreamWorkKind::Transcribing);
        } else {
            show_transcribing_overlay(app);
        }

        // Restore system audio before playing audio feedback so the stop sound
        // is audible. The duck ramp runs on its own thread, so this returns
        // immediately and the chime plays while the volume comes back up.
        rm.remove_recording_audio();

        // Play audio feedback for recording stop
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string(); // Clone binding_id for the async task
        let post_process = self.post_process;
        let cancel_generation = rm.cancel_generation();

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id, cancel_generation) {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if rm.was_cancelled_since(cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    tm.cancel_stream();
                    azure_stream.cancel();
                    utils::hide_recording_overlay(&ah);
                    change_tray_icon(&ah, TrayIconState::Idle);
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    // Tear down any streaming worker so its channel doesn't leak
                    // and block the next start_stream.
                    tm.cancel_stream();
                    azure_stream.cancel();
                    utils::hide_recording_overlay(&ah);
                    change_tray_icon(&ah, TrayIconState::Idle);
                } else {
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let file_name = format!("speak-{}.wav", chrono::Utc::now().timestamp());
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let wav_path_for_verify = wav_path.clone();
                    let samples_for_wav = samples.clone();
                    let wav_handle = tauri::async_runtime::spawn_blocking(move || {
                        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
                    });

                    // Transcribe concurrently with WAV save. If a live stream was
                    // running, finalize it and use its text (all audio was already
                    // fed to the stream); otherwise batch-transcribe the samples.
                    let transcription_time = Instant::now();
                    let (transcription_result, live_translation) =
                        acquire_transcript(&tm, &azure_stream, &settings, samples).await;

                    // Await WAV save and verify
                    let wav_saved = match wav_handle.await {
                        Ok(Ok(())) => {
                            match crate::audio_toolkit::verify_wav_file(
                                &wav_path_for_verify,
                                sample_count,
                            ) {
                                Ok(()) => true,
                                Err(e) => {
                                    error!("WAV verification failed: {}", e);
                                    false
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save WAV file: {}", e);
                            false
                        }
                        Err(e) => {
                            error!("WAV save task panicked: {}", e);
                            false
                        }
                    };

                    if rm.was_cancelled_since(cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        utils::hide_recording_overlay(&ah);
                        change_tray_icon(&ah, TrayIconState::Idle);
                        return;
                    }

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?}: {} chars",
                                transcription_time.elapsed(),
                                transcription.chars().count()
                            );

                            if post_process {
                                if use_streaming_overlay {
                                    tm.emit_stream_working(StreamWorkKind::Polishing);
                                } else {
                                    show_processing_overlay(&ah);
                                }
                            }
                            let Some(processed) = complete_unless_cancelled(
                                process_transcription_output_with_translation(
                                    &ah,
                                    &transcription,
                                    post_process,
                                    live_translation,
                                ),
                                || rm.was_cancelled_since(cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                                return;
                            };

                            if rm.was_cancelled_since(cancel_generation) {
                                debug!("Transcription operation cancelled before paste");
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                                return;
                            }

                            // Save to history if WAV was saved
                            if wav_saved {
                                if let Err(err) = hm.save_entry(
                                    file_name,
                                    transcription,
                                    post_process,
                                    processed.post_processed_text.clone(),
                                    processed.post_process_prompt.clone(),
                                ) {
                                    error!("Failed to save history entry: {}", err);
                                }
                            }

                            if processed.final_text.is_empty() {
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                            } else {
                                let ah_clone = ah.clone();
                                let paste_time = Instant::now();
                                let final_text = processed.final_text;
                                let rm_for_paste = Arc::clone(&rm);
                                ah.run_on_main_thread(move || {
                                    if rm_for_paste.was_cancelled_since(cancel_generation) {
                                        debug!("Transcription operation cancelled before paste");
                                        utils::hide_recording_overlay(&ah_clone);
                                        change_tray_icon(&ah_clone, TrayIconState::Idle);
                                        return;
                                    }

                                    match utils::paste(final_text, ah_clone.clone()) {
                                        Ok(()) => debug!(
                                            "Text pasted successfully in {:?}",
                                            paste_time.elapsed()
                                        ),
                                        Err(e) => {
                                            error!("Failed to paste transcription: {}", e);
                                            let _ = ah_clone.emit(crate::events::PASTE_ERROR, ());
                                        }
                                    }
                                    utils::hide_recording_overlay(&ah_clone);
                                    change_tray_icon(&ah_clone, TrayIconState::Idle);
                                })
                                .unwrap_or_else(|e| {
                                    error!("Failed to run paste on main thread: {:?}", e);
                                    utils::hide_recording_overlay(&ah);
                                    change_tray_icon(&ah, TrayIconState::Idle);
                                });
                            }
                        }
                        Err(err) => {
                            azure_stream.cancel();
                            if rm.was_cancelled_since(cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                                return;
                            }

                            error!("Transcription failed: {}", err);
                            // Surface the failure to the UI (toast). The full
                            // message is also in handy.log via the line above.
                            let _ = ah.emit(crate::events::TRANSCRIPTION_ERROR, err.to_string());
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if let Err(save_err) = hm.save_entry(
                                    file_name,
                                    String::new(),
                                    post_process,
                                    None,
                                    None,
                                ) {
                                    error!("Failed to save failed history entry: {}", save_err);
                                }
                            }
                            utils::hide_recording_overlay(&ah);
                            change_tray_icon(&ah, TrayIconState::Idle);
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
                // Tear down any streaming worker so its channel doesn't leak.
                tm.cancel_stream();
                azure_stream.cancel();
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Assistant Action

/// One of the two shipped assistants, bound to a shortcut.
///
/// The recording half is literally the dictation path: [`start_recording_session`]
/// and [`acquire_transcript`] are the same functions `TranscribeAction` calls.
/// Only what happens to the finished transcript differs, and it differs in three
/// deliberate ways:
///
/// - **No history entry.** A question asked of an assistant is not something the
///   user dictated, and silently filing it under "your transcriptions" would put
///   text the user never intended to keep into an exportable list.
/// - **No WAV.** Nothing references the recording afterwards, so persisting it
///   would leave audio of every question on disk with nothing to delete it.
/// - **No post-processing or translation.** Those rewrite a transcript for
///   *insertion*. Running a "clean up my dictation" prompt over a question would
///   change what is being asked before it is asked.
struct AssistantAction {
    profile_id: &'static str,
}

/// Deliver an assistant answer according to the profile's output mode.
///
/// `Show` leaves the processing overlay up: Phase E renders the answer card into
/// that same window and owns hiding it, so tearing it down here would make the
/// card flash and vanish. `Copy` and `Paste` complete without a card, so they
/// hide the overlay themselves.
/// How long the frontend gets to turn the processing spinner into the card.
///
/// Normally the card appears within a frame or two; this only fires when the
/// IPC call failed or the webview was mid-reload, in which case nothing else
/// would ever hide the overlay.
const ANSWER_CARD_WATCHDOG: std::time::Duration = std::time::Duration::from_millis(2500);

const ANSWER_COPIED_FALLBACK_NOTICE: &str =
    "The overlay is switched off, so the assistant's answer was copied to the clipboard instead.";

/// Surface a message when no answer card will carry it.
///
/// Lands as a toast in the main window (`App.tsx` listens for it). With the
/// main window closed to the tray this is still invisible; that limit is
/// stated in the plan rather than hidden, and a tray notification is the next
/// step if it matters in practice.
fn report_without_card(app: &AppHandle, message: &str) {
    if let Err(err) = app.emit(crate::events::ASSISTANT_NOTICE, message.to_string()) {
        error!("Failed to emit assistant notice: {err}");
    }
}

fn deliver_assistant_answer(
    app: &AppHandle,
    mut event: crate::assistant::AssistantAnswerEvent,
    output_mode: AssistantOutputMode,
) {
    use tauri_specta::Event as _;

    // Decided once, here, and carried in the payload. The frontend used to ask
    // Rust to re-read the output mode at present time; a setting changed while
    // the request was in flight, or a failed IPC call, left the processing
    // spinner up with nothing ever replacing it.
    let card_available = crate::overlay::answer_card_can_be_shown(app);
    let delivery = plan_answer_delivery(output_mode, event.answer.as_deref(), card_available);
    event.expects_card = matches!(delivery, AnswerDelivery::ShowCard);
    let is_fallback = matches!(delivery, AnswerDelivery::CopyFallback(_));

    // Emitted first and unconditionally: the event is the only report the user
    // gets, so it must not be contingent on a clipboard or paste succeeding.
    if let Err(err) = event.emit(app) {
        error!("Failed to emit assistant answer event: {err}");
    }

    match delivery {
        AnswerDelivery::ShowCard => {
            // The frontend turns the spinner into the card. If it never does,
            // hide the overlay ourselves. `show_overlay_state` bumps the overlay
            // generation when the card appears, which is what makes this a
            // no-op in the normal case and keeps it from tearing down a later,
            // unrelated state.
            let generation = crate::overlay::overlay_generation();
            let app_for_watchdog = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(ANSWER_CARD_WATCHDOG);
                crate::overlay::hide_overlay_if_unchanged(&app_for_watchdog, generation);
            });
        }
        AnswerDelivery::NothingToDeliver => {
            // No card will carry the error string, so say it somewhere.
            if let Some(message) = event.error.as_deref() {
                report_without_card(app, message);
            }
            utils::hide_recording_overlay(app);
        }
        AnswerDelivery::Copy(text) | AnswerDelivery::CopyFallback(text) => {
            // Reuses the dictation paste machinery's clipboard writer, which
            // owns the platform quirks (Windows delayed rendering, Wayland
            // wl-copy). Unlike paste, the answer is meant to stay on the
            // clipboard, so nothing is restored afterwards.
            match crate::clipboard::write_text_to_clipboard(app, &text) {
                Ok(()) if is_fallback => report_without_card(app, ANSWER_COPIED_FALLBACK_NOTICE),
                Ok(()) => {}
                Err(details) => {
                    // Silent failure here means the user pastes the *previous*
                    // clipboard content believing it is the answer.
                    error!("Failed to copy assistant answer to the clipboard: {details}");
                    let message =
                        crate::assistant::AssistantError::ClipboardFailed { details }.message();
                    report_without_card(app, &message);
                }
            }
            utils::hide_recording_overlay(app);
        }
        AnswerDelivery::Paste(text) => {
            // Same main-thread hop and same helper `TranscribeAction` uses:
            // enigo and the clipboard are main-thread-bound on macOS.
            let app_for_paste = app.clone();
            app.run_on_main_thread(move || {
                if let Err(err) = utils::paste(text, app_for_paste.clone()) {
                    error!("Failed to paste assistant answer: {err}");
                    let _ = app_for_paste.emit(crate::events::PASTE_ERROR, ());
                }
                utils::hide_recording_overlay(&app_for_paste);
                change_tray_icon(&app_for_paste, TrayIconState::Idle);
            })
            .unwrap_or_else(|err| {
                error!("Failed to run assistant paste on main thread: {err:?}");
                utils::hide_recording_overlay(app);
                change_tray_icon(app, TrayIconState::Idle);
            });
            // The queued closure owns the tray reset.
            return;
        }
    }

    change_tray_icon(app, TrayIconState::Idle);
}

/// What [`deliver_assistant_answer`] should do with a finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AnswerDelivery {
    /// Emit only, and leave the overlay up for the answer card.
    ShowCard,
    /// Emit, put the text on the clipboard, hide the overlay.
    Copy(String),
    /// Emit, paste into the focused application, hide the overlay.
    Paste(String),
    /// Emit only, and hide the overlay: a `Copy`/`Paste` run that produced no
    /// answer to deliver.
    NothingToDeliver,
    /// A `Show` run whose card cannot appear (overlay switched off): copy the
    /// answer and say so, because emitting into a void is not delivery.
    CopyFallback(String),
}

/// Decide the delivery for one finished run.
///
/// Pure, so the routing rules are testable without an app handle, a clipboard,
/// or a focused window. Two rules carry the weight:
///
/// - A failed run never delivers text. `answer` is `None` on every error path,
///   and pasting an error message into whatever document the user had focused
///   would be considerably worse than the failure it describes.
/// - `Show` never delivers text even on success. It is the shipped default
///   precisely because an assistant that types into the focused application
///   unasked is destructive when the shortcut is hit by accident.
fn plan_answer_delivery(
    output_mode: AssistantOutputMode,
    answer: Option<&str>,
    card_available: bool,
) -> AnswerDelivery {
    let deliverable = answer.filter(|text| !text.is_empty());
    match (output_mode, deliverable) {
        (AssistantOutputMode::Show, _) if card_available => AnswerDelivery::ShowCard,
        // No card can appear (the overlay is off - the shipped default on
        // Linux). The answer still has to land somewhere the user can find it.
        (AssistantOutputMode::Show, Some(text)) => AnswerDelivery::CopyFallback(text.to_string()),
        (AssistantOutputMode::Show, None) => AnswerDelivery::NothingToDeliver,
        (AssistantOutputMode::Copy, Some(text)) => AnswerDelivery::Copy(text.to_string()),
        (AssistantOutputMode::Paste, Some(text)) => AnswerDelivery::Paste(text.to_string()),
        (AssistantOutputMode::Copy | AssistantOutputMode::Paste, None) => {
            AnswerDelivery::NothingToDeliver
        }
    }
}

/// Emit a failure that never reached the provider. Split out so every early
/// return is one line and cannot forget the event.
fn emit_assistant_failure(
    app: &AppHandle,
    profile_id: &str,
    question: &str,
    error: crate::assistant::AssistantError,
) {
    use tauri_specta::Event as _;

    let message = error.message();
    let mut event = crate::assistant::failed_answer(profile_id, question, error);
    // A card can carry the message only for a `Show` profile with the overlay
    // on; everywhere else the message has to travel another way or it is lost.
    let output_mode = crate::settings::get_settings(app)
        .assistant_profile(profile_id)
        .map(|profile| profile.output_mode)
        .unwrap_or_default();
    event.expects_card =
        output_mode == AssistantOutputMode::Show && crate::overlay::answer_card_can_be_shown(app);

    if let Err(err) = event.emit(app) {
        error!("Failed to emit assistant failure event: {err}");
    }
    if !event.expects_card {
        report_without_card(app, &message);
    }
}

impl ShortcutAction for AssistantAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        start_recording_session(app, binding_id);
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        app.state::<Arc<AudioRecordingManager>>()
            .invalidate_recording_readiness();
        shortcut::unregister_cancel_shortcut(app);

        debug!(
            "AssistantAction::stop called for binding: {} (profile {})",
            binding_id, self.profile_id
        );

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let azure_stream =
            Arc::clone(&app.state::<Arc<crate::speech_pipeline::AzureSpeechStreamManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);

        let settings = get_settings(app);
        let use_streaming_overlay = should_use_streaming_overlay(
            settings.overlay_style,
            tm.is_streaming() || azure_stream.is_active(),
        );
        if use_streaming_overlay {
            tm.emit_stream_working(StreamWorkKind::Transcribing);
        } else {
            show_transcribing_overlay(app);
        }

        rm.remove_recording_audio();
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string();
        let profile_id = self.profile_id;
        let cancel_generation = rm.cancel_generation();

        tauri::async_runtime::spawn(async move {
            // Releases the coordinator stage on every exit path, including the
            // early returns below and a panic.
            let _guard = FinishGuard(ah.clone());

            let Some(samples) = rm.stop_recording(&binding_id, cancel_generation) else {
                debug!("Assistant: no samples retrieved from recording stop");
                tm.cancel_stream();
                azure_stream.cancel();
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            };

            // A cancel is the user withdrawing the question; stay silent rather
            // than reporting an error for something they deliberately aborted.
            if rm.was_cancelled_since(cancel_generation) {
                debug!("Assistant cancelled after recording stop");
                tm.cancel_stream();
                azure_stream.cancel();
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            }

            if samples.is_empty() {
                debug!("Assistant recording produced no audio samples");
                tm.cancel_stream();
                azure_stream.cancel();
                emit_assistant_failure(
                    &ah,
                    profile_id,
                    "",
                    crate::assistant::AssistantError::BlankQuestion,
                );
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            }

            // Identical to dictation: same provider fork, same Azure recovery.
            // Deliberately without the WAV save that runs alongside it there.
            let (transcription_result, _live_translation) =
                acquire_transcript(&tm, &azure_stream, &settings, samples).await;

            if rm.was_cancelled_since(cancel_generation) {
                debug!("Assistant cancelled before asking");
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            }

            let question = match transcription_result {
                Ok(question) => question,
                Err(err) => {
                    azure_stream.cancel();
                    error!("Assistant transcription failed: {err}");
                    let _ = ah.emit(crate::events::TRANSCRIPTION_ERROR, err.to_string());
                    emit_assistant_failure(
                        &ah,
                        profile_id,
                        "",
                        // Not `Request`: this failure is speech recognition,
                        // and calling it an assistant request failure sends the
                        // user to the assistant's provider settings while the
                        // broken one is the speech provider.
                        crate::assistant::AssistantError::TranscriptionFailed {
                            details: err.to_string(),
                        },
                    );
                    utils::hide_recording_overlay(&ah);
                    change_tray_icon(&ah, TrayIconState::Idle);
                    return;
                }
            };

            // Silence must never reach the provider: the model would answer the
            // empty string with something confidently irrelevant, and the user
            // would be billed for it.
            if crate::assistant::is_assistant_question_blank(&question) {
                debug!("Assistant heard nothing; not calling the provider");
                emit_assistant_failure(
                    &ah,
                    profile_id,
                    &question,
                    crate::assistant::AssistantError::BlankQuestion,
                );
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            }

            // The existing processing state, not a new one - Phase E owns the
            // overlay file and the answer state that replaces this.
            if use_streaming_overlay {
                tm.emit_stream_working(StreamWorkKind::Polishing);
            } else {
                show_processing_overlay(&ah);
            }

            let event = crate::assistant::run_assistant(&ah, profile_id, &question).await;

            if rm.was_cancelled_since(cancel_generation) {
                debug!("Assistant cancelled before delivering the answer");
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
                return;
            }

            // Re-read the mode: the request may have taken a minute and the
            // user may have changed it in the meantime.
            let output_mode = get_settings(&ah)
                .assistant_profile(profile_id)
                .map(|profile| profile.output_mode)
                .unwrap_or_default();

            deliver_assistant_answer(&ah, event, output_mode);
        });
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        utils::cancel_current_operation(app);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: {})", // Changed "Pressed" to "Started" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: {})", // Changed "Released" to "Stopped" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction {
            post_process: false,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction { post_process: true }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        crate::settings::ASSISTANT_BINDING_FAST.to_string(),
        Arc::new(AssistantAction {
            profile_id: crate::settings::ASSISTANT_PROFILE_FAST,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        crate::settings::ASSISTANT_BINDING_SMART.to_string(),
        Arc::new(AssistantAction {
            profile_id: crate::settings::ASSISTANT_PROFILE_SMART,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::{
        azure_recovery_is_better, complete_unless_cancelled, plan_answer_delivery,
        should_use_streaming_overlay, strip_think_block, AnswerDelivery, ACTION_MAP,
    };
    use crate::settings::{AssistantOutputMode, OverlayStyle};
    use std::future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// With the overlay off there is no card; a Show run must still land.
    #[test]
    fn show_without_a_card_falls_back_to_the_clipboard() {
        assert!(matches!(
            plan_answer_delivery(AssistantOutputMode::Show, Some("42"), false),
            AnswerDelivery::CopyFallback(text) if text == "42"
        ));
    }

    #[test]
    fn show_without_a_card_and_without_an_answer_delivers_nothing() {
        assert!(matches!(
            plan_answer_delivery(AssistantOutputMode::Show, None, false),
            AnswerDelivery::NothingToDeliver
        ));
    }

    /// Copy and Paste never wanted a card, so its absence changes nothing.
    #[test]
    fn copy_and_paste_ignore_card_availability() {
        assert!(matches!(
            plan_answer_delivery(AssistantOutputMode::Copy, Some("x"), false),
            AnswerDelivery::Copy(text) if text == "x"
        ));
        assert!(matches!(
            plan_answer_delivery(AssistantOutputMode::Paste, Some("x"), false),
            AnswerDelivery::Paste(text) if text == "x"
        ));
    }

    #[test]
    fn blank_transcription_is_detected() {
        assert!(crate::assistant::is_assistant_question_blank(""));
        assert!(crate::assistant::is_assistant_question_blank("   "));
        assert!(crate::assistant::is_assistant_question_blank("\t\n  \r\n"));
    }

    #[test]
    fn non_blank_transcription_is_kept() {
        assert!(!crate::assistant::is_assistant_question_blank("hello"));
        assert!(!crate::assistant::is_assistant_question_blank("  hello  "));
    }

    #[test]
    fn completed_operation_returns_its_output() {
        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::ready("done"),
            || false,
        ));

        assert_eq!(result, Some("done"));
    }

    #[test]
    fn pending_operation_stops_after_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancelled_for_thread.store(true, Ordering::Release);
        });

        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::pending::<()>(),
            || cancelled.load(Ordering::Acquire),
        ));

        cancel_thread.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn leading_think_block_is_stripped() {
        assert_eq!(
            strip_think_block("<think>pondering...</think>Cleaned text."),
            "Cleaned text."
        );
        assert_eq!(
            strip_think_block("  \n<think>multi\nline</think>\n  Cleaned text."),
            "Cleaned text."
        );
    }

    #[test]
    fn content_without_think_block_is_unchanged() {
        assert_eq!(strip_think_block("Cleaned text."), "Cleaned text.");
        assert_eq!(
            strip_think_block("Mentions <think> mid-sentence."),
            "Mentions <think> mid-sentence."
        );
        // Unclosed block: leave untouched rather than guess
        assert_eq!(
            strip_think_block("<think>never closed"),
            "<think>never closed"
        );
    }

    #[test]
    fn live_overlay_uses_streaming_states_only_for_streaming_models() {
        assert!(should_use_streaming_overlay(OverlayStyle::Live, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::Live, false));
        assert!(!should_use_streaming_overlay(OverlayStyle::Minimal, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::None, true));
    }

    // -- assistant delivery routing -----------------------------------------

    /// `Show` is the shipped default. It must never touch the clipboard or the
    /// focused application, however good the answer is.
    #[test]
    fn show_mode_only_shows() {
        assert_eq!(
            plan_answer_delivery(AssistantOutputMode::Show, Some("Paris."), true),
            AnswerDelivery::ShowCard
        );
        assert_eq!(
            plan_answer_delivery(AssistantOutputMode::Show, None, true),
            AnswerDelivery::ShowCard
        );
    }

    #[test]
    fn copy_and_paste_modes_deliver_the_answer() {
        assert_eq!(
            plan_answer_delivery(AssistantOutputMode::Copy, Some("Paris."), true),
            AnswerDelivery::Copy("Paris.".to_string())
        );
        assert_eq!(
            plan_answer_delivery(AssistantOutputMode::Paste, Some("Paris."), true),
            AnswerDelivery::Paste("Paris.".to_string())
        );
    }

    /// A failed run carries `answer: None`. Delivering anything then would put
    /// an error message on the clipboard, or type it into the user's document.
    #[test]
    fn a_failed_run_delivers_nothing_in_any_mode() {
        for mode in [
            AssistantOutputMode::Copy,
            AssistantOutputMode::Paste,
            AssistantOutputMode::Show,
        ] {
            let delivery = plan_answer_delivery(mode, None, true);
            assert!(
                matches!(
                    delivery,
                    AnswerDelivery::NothingToDeliver | AnswerDelivery::ShowCard
                ),
                "{mode:?} delivered text for a failed run: {delivery:?}"
            );
        }
        // An empty-but-present answer is treated the same way.
        assert_eq!(
            plan_answer_delivery(AssistantOutputMode::Paste, Some(""), true),
            AnswerDelivery::NothingToDeliver
        );
    }

    /// Only `Show` keeps the overlay: Phase E renders its card there. Every
    /// other outcome finishes without a card and must clean up after itself,
    /// or the overlay would be stranded on screen.
    #[test]
    fn only_show_mode_leaves_the_overlay_up() {
        let keeps_overlay =
            |delivery: &AnswerDelivery| matches!(delivery, AnswerDelivery::ShowCard);

        assert!(keeps_overlay(&plan_answer_delivery(
            AssistantOutputMode::Show,
            Some("answer"),
            true
        )));
        assert!(!keeps_overlay(&plan_answer_delivery(
            AssistantOutputMode::Copy,
            Some("answer"),
            true
        )));
        assert!(!keeps_overlay(&plan_answer_delivery(
            AssistantOutputMode::Paste,
            Some("answer"),
            true
        )));
        assert!(!keeps_overlay(&plan_answer_delivery(
            AssistantOutputMode::Copy,
            None,
            true
        )));
    }

    // -- action registration -------------------------------------------------

    /// Both assistants must be reachable. A binding with no `ACTION_MAP` entry
    /// logs "No action in ACTION_MAP" and does nothing, which looks exactly
    /// like a broken shortcut.
    #[test]
    fn assistant_bindings_are_registered() {
        assert!(ACTION_MAP.contains_key(crate::settings::ASSISTANT_BINDING_FAST));
        assert!(ACTION_MAP.contains_key(crate::settings::ASSISTANT_BINDING_SMART));
    }

    /// Regression guard for the `TranscribeAction` refactor: extracting the
    /// shared start/transcript helpers must not have disturbed the dictation
    /// registrations or added a stray entry.
    #[test]
    fn action_map_registrations_are_unchanged_for_dictation() {
        assert!(ACTION_MAP.contains_key("transcribe"));
        assert!(ACTION_MAP.contains_key("transcribe_with_post_process"));
        assert!(ACTION_MAP.contains_key("cancel"));
        assert!(ACTION_MAP.contains_key("test"));

        let mut ids: Vec<&str> = ACTION_MAP.keys().map(String::as_str).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![
                "assistant_fast",
                "assistant_smart",
                "cancel",
                "test",
                "transcribe",
                "transcribe_with_post_process",
            ]
        );
    }

    /// Every binding the app ships must resolve to an action, except the two
    /// meeting bindings, which `handle_shortcut_event` routes to the meeting
    /// state machine before `ACTION_MAP` is consulted.
    #[test]
    fn every_shipped_binding_has_an_action_or_is_a_meeting_binding() {
        for id in crate::settings::get_default_settings().bindings.keys() {
            assert!(
                ACTION_MAP.contains_key(id) || crate::settings::is_meeting_binding(id),
                "binding '{id}' has no action and is not a meeting binding"
            );
        }
    }

    #[test]
    fn azure_recovery_only_replaces_meaningfully_longer_text() {
        assert!(azure_recovery_is_better(
            "Only the first sentence.",
            "Only the first sentence. This is the missing second sentence."
        ));
        assert!(!azure_recovery_is_better(
            "The complete sentence is here.",
            "The complete sentence is right here."
        ));
    }
}
