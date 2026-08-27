use crate::audio_toolkit::{
    apply_custom_words, normalize_transcription_output, remove_filler_words, OutputLanguageEvidence,
};
use crate::settings::{AppSettings, SpeechInputProvider};
use crate::sync::MutexExt;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use log::{debug, warn};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tauri_specta::Event;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

const SAMPLE_RATE: usize = 16_000;
const AZURE_AUDIO_EVENT_BYTES: usize = SAMPLE_RATE * 2 / 10;
const AZURE_FINALIZE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

#[derive(Debug, Deserialize)]
struct DeepLTranslation {
    text: String,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct AzureSpeechAuth {
    pub token: String,
    pub region: String,
    pub language: String,
    pub stable_partial_threshold: u32,
    pub segmentation_silence_ms: u32,
    pub deepl_enabled: bool,
    pub translation_delay_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
struct AzureStreamStartPayload {
    session_id: u64,
}

#[derive(Clone, Debug, Serialize)]
struct AzureAudioChunkPayload {
    session_id: u64,
    pcm_base64: String,
}

#[derive(Clone, Debug, Serialize)]
struct AzureStreamControlPayload {
    session_id: u64,
}

#[derive(Clone, Debug)]
pub struct AzureStreamingResult {
    pub source_text: String,
    pub translated_text: Option<String>,
}

struct AzureSessionState {
    session_id: u64,
    deepl_enabled: bool,
    source_segments: Vec<String>,
    pending_translation: Vec<String>,
    translating_source: String,
    translated_segments: Vec<String>,
    interim: String,
    completion_tx: Option<oneshot::Sender<std::result::Result<(), String>>>,
    completion_status: Option<std::result::Result<(), String>>,
}

/// Bridges Speak's already-selected microphone stream into Microsoft's official
/// browser Speech SDK running in the hidden main webview. Audio is batched into
/// 100 ms PCM packets so the realtime recorder thread does not flood the Tauri
/// event bridge with one JSON event per 30 ms recorder frame.
pub struct AzureSpeechStreamManager {
    app: AppHandle,
    active: AtomicBool,
    next_session_id: AtomicU64,
    current_session_id: AtomicU64,
    audio_buffer: Mutex<Vec<u8>>,
    state: Mutex<Option<AzureSessionState>>,
    translation_gate: AsyncMutex<()>,
}

impl AzureSpeechStreamManager {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            active: AtomicBool::new(false),
            next_session_id: AtomicU64::new(1),
            current_session_id: AtomicU64::new(0),
            audio_buffer: Mutex::new(Vec::with_capacity(AZURE_AUDIO_EVENT_BYTES * 2)),
            state: Mutex::new(None),
            translation_gate: AsyncMutex::new(()),
        }
    }

    pub fn start(&self, settings: &AppSettings) -> Result<()> {
        azure_region(&settings.azure_speech_endpoint)?;
        let api_key =
            crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
                .unwrap_or_default();
        if api_key.is_empty() {
            bail!("Azure STT is selected but no API key is configured");
        }

        self.cancel();
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        *self.audio_buffer.lock_recover() = Vec::with_capacity(AZURE_AUDIO_EVENT_BYTES * 2);
        *self.state.lock_recover() = Some(AzureSessionState {
            session_id,
            deepl_enabled: settings.deepl_translation_enabled,
            source_segments: Vec::new(),
            pending_translation: Vec::new(),
            translating_source: String::new(),
            translated_segments: Vec::new(),
            interim: String::new(),
            completion_tx: None,
            completion_status: None,
        });
        self.current_session_id.store(session_id, Ordering::Release);
        self.active.store(true, Ordering::Release);

        self.app
            .emit_to(
                "main",
                "azure-speech-start",
                AzureStreamStartPayload { session_id },
            )
            .context("Failed to start Azure Speech webview session")?;
        debug!("Azure continuous STT session {session_id} requested");
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn feed(&self, frame: &[f32]) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }

        let mut ready = None;
        {
            let mut buffer = self.audio_buffer.lock_recover();
            for sample in frame {
                let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                buffer.extend_from_slice(&pcm.to_le_bytes());
            }
            if buffer.len() >= AZURE_AUDIO_EVENT_BYTES {
                ready = Some(std::mem::take(&mut *buffer));
            }
        }
        if let Some(bytes) = ready {
            self.emit_audio(bytes);
        }
    }

    fn emit_audio(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let session_id = self.current_session_id.load(Ordering::Acquire);
        let pcm_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        if let Err(error) = self.app.emit_to(
            "main",
            "azure-speech-audio",
            AzureAudioChunkPayload {
                session_id,
                pcm_base64,
            },
        ) {
            warn!("Failed to forward Azure audio chunk: {error}");
        }
    }

    fn flush_audio(&self) {
        let bytes = std::mem::take(&mut *self.audio_buffer.lock_recover());
        self.emit_audio(bytes);
    }

    pub fn set_interim(&self, session_id: u64, text: String) {
        let mut guard = self.state.lock_recover();
        let Some(state) = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
        else {
            return;
        };
        state.interim = clean_cloud_transcript(&text, &crate::settings::get_settings(&self.app));
        debug!(
            "Azure continuous STT session {session_id} interim received ({} chars)",
            state.interim.chars().count()
        );
        self.emit_overlay_text(state);
    }

    pub fn commit_segment(&self, session_id: u64, text: String) {
        let cleaned = clean_cloud_transcript(&text, &crate::settings::get_settings(&self.app));
        if cleaned.trim().is_empty() {
            return;
        }

        let mut guard = self.state.lock_recover();
        let Some(state) = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
        else {
            return;
        };
        if state.source_segments.last() == Some(&cleaned) {
            return;
        }
        state.source_segments.push(cleaned.clone());
        debug!(
            "Azure continuous STT session {session_id} final segment received ({} chars)",
            cleaned.chars().count()
        );
        if state.deepl_enabled {
            state.pending_translation.push(cleaned);
        }
        state.interim.clear();
        self.emit_overlay_text(state);
    }

    pub async fn translate_pending(&self, session_id: u64) -> Result<()> {
        let _gate = self.translation_gate.lock().await;
        let source = {
            let mut guard = self.state.lock_recover();
            let Some(state) = guard
                .as_mut()
                .filter(|state| state.session_id == session_id)
            else {
                return Ok(());
            };
            if !state.deepl_enabled || state.pending_translation.is_empty() {
                return Ok(());
            }
            let source = join_segments(&state.pending_translation);
            state.pending_translation.clear();
            state.translating_source = source.clone();
            self.emit_overlay_text(state);
            source
        };

        let settings = crate::settings::get_settings(&self.app);
        let translated = maybe_translate_with_deepl(&source, &settings).await;
        let mut guard = self.state.lock_recover();
        let Some(state) = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
        else {
            return Ok(());
        };
        state.translating_source.clear();
        match translated {
            Ok(Some(text)) => state.translated_segments.push(text),
            Ok(None) => state.translated_segments.push(source),
            Err(error) => {
                state.translated_segments.push(source);
                self.emit_overlay_text(state);
                warn!("DeepL live translation failed; keeping source segment: {error}");
                let _ = self.app.emit(
                    "transcription-error",
                    format!("DeepL translation failed: {error}"),
                );
                return Ok(());
            }
        }
        self.emit_overlay_text(state);
        Ok(())
    }

    pub fn complete(
        &self,
        session_id: u64,
        error: Option<String>,
    ) -> std::result::Result<(), String> {
        let status = match error {
            Some(error) => Err(error),
            None => Ok(()),
        };
        debug!(
            "Azure continuous STT session {session_id} completed (error={})",
            status.is_err()
        );
        let mut guard = self.state.lock_recover();
        let state = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
            .ok_or_else(|| "Azure Speech session is no longer active".to_string())?;
        if let Some(tx) = state.completion_tx.take() {
            let _ = tx.send(status);
        } else {
            state.completion_status = Some(status);
        }
        Ok(())
    }

    pub async fn finalize(&self) -> Result<AzureStreamingResult> {
        self.finalize_with_timeout(AZURE_FINALIZE_TIMEOUT).await
    }

    pub async fn finalize_with_timeout(&self, timeout: Duration) -> Result<AzureStreamingResult> {
        self.active.store(false, Ordering::Release);
        self.flush_audio();
        let session_id = self.current_session_id.load(Ordering::Acquire);
        let rx = {
            let mut guard = self.state.lock_recover();
            let state = guard
                .as_mut()
                .filter(|state| state.session_id == session_id)
                .ok_or_else(|| anyhow!("Azure Speech session was not started"))?;
            if let Some(status) = state.completion_status.take() {
                drop(guard);
                status.map_err(anyhow::Error::msg)?;
                return self.take_result(session_id);
            }
            let (tx, rx) = oneshot::channel();
            state.completion_tx = Some(tx);
            rx
        };

        self.app
            .emit_to(
                "main",
                "azure-speech-stop",
                AzureStreamControlPayload { session_id },
            )
            .context("Failed to stop Azure Speech webview session")?;

        match tokio::time::timeout(timeout, rx).await {
            Ok(status) => {
                status
                    .context("Azure Speech completion channel closed")?
                    .map_err(anyhow::Error::msg)?;
                self.take_result(session_id)
            }
            Err(_) => {
                warn!(
                    "Azure continuous STT session {session_id} missed its completion event; using the latest received text"
                );
                let _ = self.app.emit_to(
                    "main",
                    "azure-speech-cancel",
                    AzureStreamControlPayload { session_id },
                );
                self.take_result(session_id)
            }
        }
    }

    fn take_result(&self, session_id: u64) -> Result<AzureStreamingResult> {
        let mut guard = self.state.lock_recover();
        let state = guard
            .take()
            .filter(|state| state.session_id == session_id)
            .ok_or_else(|| anyhow!("Azure Speech session result is unavailable"))?;
        self.current_session_id.store(0, Ordering::Release);
        let mut source_text = join_segments(&state.source_segments);
        if source_text.is_empty() && !state.interim.trim().is_empty() {
            source_text = state.interim.trim().to_string();
        }
        if source_text.is_empty() {
            bail!("Azure Speech returned no transcript");
        }
        let translated_text = if state.deepl_enabled {
            let translated = join_segments(&state.translated_segments);
            (!translated.is_empty()).then_some(translated)
        } else {
            None
        };
        Ok(AzureStreamingResult {
            source_text,
            translated_text,
        })
    }

    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
        self.audio_buffer.lock_recover().clear();
        let session = self.state.lock_recover().take();
        if let Some(state) = session {
            let _ = self.app.emit_to(
                "main",
                "azure-speech-cancel",
                AzureStreamControlPayload {
                    session_id: state.session_id,
                },
            );
        }
        self.current_session_id.store(0, Ordering::Release);
    }

    fn emit_overlay_text(&self, state: &AzureSessionState) {
        let (committed, tentative) = if state.deepl_enabled {
            let mut pending = state.pending_translation.clone();
            if !state.translating_source.is_empty() {
                pending.insert(0, state.translating_source.clone());
            }
            if !state.interim.is_empty() {
                pending.push(state.interim.clone());
            }
            (
                join_segments(&state.translated_segments),
                join_segments(&pending),
            )
        } else {
            (join_segments(&state.source_segments), state.interim.clone())
        };
        let _ = crate::managers::transcription::StreamTextEvent {
            committed,
            tentative,
        }
        .emit(&self.app);
    }
}

fn join_segments(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn azure_region(configured: &str) -> Result<String> {
    let configured = configured.trim();
    if configured.is_empty() {
        bail!("Azure STT is selected but no region is configured");
    }
    if configured.starts_with("https://") || configured.starts_with("http://") {
        let url = reqwest::Url::parse(configured).context("Invalid Azure Speech endpoint")?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("Azure Speech endpoint has no host"))?;
        if let Some(region) = host.split('.').next().filter(|part| !part.is_empty()) {
            return Ok(region.to_string());
        }
        bail!("Could not determine the Azure region from the endpoint");
    }
    if !configured
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        bail!("Azure Speech region contains unsupported characters");
    }
    Ok(configured.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_azure_speech_auth(app: AppHandle) -> std::result::Result<AzureSpeechAuth, String> {
    let settings = crate::settings::get_settings(&app);
    let region =
        azure_region(&settings.azure_speech_endpoint).map_err(|error| error.to_string())?;
    let api_key =
        crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
            .unwrap_or_default();
    if api_key.is_empty() {
        return Err("Azure STT is selected but no API key is configured".to_string());
    }

    let response = reqwest::Client::new()
        .post(format!(
            "https://{region}.api.cognitive.microsoft.com/sts/v1.0/issueToken"
        ))
        .header("Ocp-Apim-Subscription-Key", api_key)
        .header(CONTENT_LENGTH, "0")
        .body("")
        .send()
        .await
        .map_err(|error| format!("Failed to request Azure Speech token: {error}"))?;
    let status = response.status();
    let token = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Azure Speech token: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Azure Speech token request returned {status}: {}",
            compact_error(&token)
        ));
    }

    Ok(AzureSpeechAuth {
        token,
        region,
        language: settings.azure_speech_language,
        stable_partial_threshold: settings.azure_stable_partial_threshold,
        segmentation_silence_ms: settings.azure_segmentation_silence_ms,
        deepl_enabled: settings.deepl_translation_enabled,
        translation_delay_ms: settings.deepl_sentence_delay_ms,
    })
}

#[tauri::command]
#[specta::specta]
pub fn azure_speech_interim(
    manager: State<'_, Arc<AzureSpeechStreamManager>>,
    session_id: u64,
    text: String,
) {
    manager.set_interim(session_id, text);
}

#[tauri::command]
#[specta::specta]
pub fn azure_speech_final(
    manager: State<'_, Arc<AzureSpeechStreamManager>>,
    session_id: u64,
    text: String,
) {
    manager.commit_segment(session_id, text);
}

#[tauri::command]
#[specta::specta]
pub async fn azure_speech_translate_pending(
    manager: State<'_, Arc<AzureSpeechStreamManager>>,
    session_id: u64,
) -> std::result::Result<(), String> {
    manager
        .translate_pending(session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn azure_speech_complete(
    manager: State<'_, Arc<AzureSpeechStreamManager>>,
    session_id: u64,
    error: Option<String>,
) -> std::result::Result<(), String> {
    manager.complete(session_id, error)
}

#[tauri::command]
#[specta::specta]
pub fn azure_speech_status(session_id: u64, status: String) {
    debug!("Azure continuous STT session {session_id} SDK status: {status}");
}

/// Transcribe captured 16 kHz mono audio with the configured cloud provider.
/// Local models continue to be handled by `TranscriptionManager`.
pub async fn transcribe_cloud(samples: &[f32], settings: &AppSettings) -> Result<String> {
    let wav = encode_pcm16_wav(samples);
    let raw = match settings.speech_input_provider {
        SpeechInputProvider::Azure => {
            bail!("Azure STT uses the continuous streaming pipeline")
        }
        SpeechInputProvider::Custom => transcribe_custom(&wav, settings).await?,
        SpeechInputProvider::Local => bail!("Local STT must be handled by TranscriptionManager"),
    };

    Ok(clean_cloud_transcript(&raw, settings))
}

/// Save/test UI helper: sends a short silent WAV through the exact configured
/// Azure short-audio endpoint. A successful HTTP response proves that the
/// region/endpoint, API key, language and network path work together; the
/// expected recognition result itself is `NoMatch` because the audio is silent.
#[tauri::command]
#[specta::specta]
pub async fn test_azure_speech_connection(app: AppHandle) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    test_azure_speech_connection_with_settings(&settings)
        .await
        .map_err(|error| error.to_string())
}

async fn test_azure_speech_connection_with_settings(settings: &AppSettings) -> Result<String> {
    let endpoint = azure_short_audio_endpoint(&settings.azure_speech_endpoint)?;
    let api_key =
        crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
            .unwrap_or_default();
    if api_key.is_empty() {
        bail!("Azure STT is selected but no API key is configured");
    }

    let separator = if endpoint.contains('?') { '&' } else { '?' };
    let url = format!(
        "{}{separator}language={}&format=detailed",
        endpoint.trim_end_matches('/'),
        settings.azure_speech_language
    );
    let silence = vec![0.0_f32; SAMPLE_RATE / 4];
    let response = reqwest::Client::new()
        .post(url)
        .header("Ocp-Apim-Subscription-Key", api_key)
        .header(
            CONTENT_TYPE,
            "audio/wav; codecs=audio/pcm; samplerate=16000",
        )
        .header(ACCEPT, "application/json")
        .body(encode_pcm16_wav(&silence))
        .send()
        .await
        .context("Failed to contact Azure Speech")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read Azure Speech response")?;
    if !status.is_success() {
        bail!("Azure Speech returned {status}: {}", compact_error(&body));
    }

    Ok("Azure Speech connection verified".to_string())
}

/// Optionally translate a completed transcript before LLM post-processing and
/// paste. This keeps the costly LLM input in the configured target language.
pub async fn maybe_translate_with_deepl(
    text: &str,
    settings: &AppSettings,
) -> Result<Option<String>> {
    if !settings.deepl_translation_enabled || text.trim().is_empty() {
        return Ok(None);
    }

    let api_key =
        crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "deepl")
            .unwrap_or_default();
    if api_key.is_empty() {
        bail!("DeepL translation is enabled but no API key is configured");
    }

    let base_url = if api_key.ends_with(":fx") {
        "https://api-free.deepl.com"
    } else {
        "https://api.deepl.com"
    };

    let mut payload = serde_json::json!({
        "text": [text],
        "target_lang": settings.deepl_target_language.to_uppercase(),
        "model_type": "prefer_quality_optimized"
    });

    if settings.selected_language != "auto" {
        payload["source_lang"] = Value::String(
            settings
                .selected_language
                .split(['-', '_'])
                .next()
                .unwrap_or("de")
                .to_uppercase(),
        );
    }

    let response = reqwest::Client::new()
        .post(format!("{base_url}/v2/translate"))
        .header(AUTHORIZATION, format!("DeepL-Auth-Key {api_key}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed to contact DeepL")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read DeepL response")?;
    if !status.is_success() {
        bail!("DeepL returned {status}: {}", compact_error(&body));
    }

    let parsed: DeepLResponse =
        serde_json::from_str(&body).context("DeepL returned an invalid response")?;
    let translated = parsed
        .translations
        .into_iter()
        .next()
        .map(|translation| translation.text.trim().to_string())
        .filter(|translation| !translation.is_empty())
        .ok_or_else(|| anyhow!("DeepL returned no translation"))?;

    Ok(Some(translated))
}

fn azure_short_audio_endpoint(configured: &str) -> Result<String> {
    let configured = configured.trim();
    if configured.is_empty() {
        bail!("Azure STT is selected but no region or endpoint is configured");
    }
    if configured.starts_with("https://") || configured.starts_with("http://") {
        return Ok(configured.to_string());
    }
    if !configured
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        bail!("Azure Speech region contains unsupported characters");
    }

    Ok(format!(
        "https://{configured}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1"
    ))
}

async fn transcribe_custom(wav: &[u8], settings: &AppSettings) -> Result<String> {
    let endpoint = settings.custom_stt_endpoint.trim();
    if endpoint.is_empty() {
        bail!("Custom STT is selected but no endpoint is configured");
    }

    let mut request = reqwest::Client::new()
        .post(endpoint)
        .header(CONTENT_TYPE, "audio/wav")
        .header(ACCEPT, "application/json, text/plain")
        .header("X-Speech-Language", &settings.azure_speech_language)
        .body(wav.to_vec());

    if let Some(token) =
        crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "custom")
            .filter(|token| !token.is_empty())
    {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = request
        .send()
        .await
        .context("Failed to contact custom STT endpoint")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read custom STT response")?;
    if !status.is_success() {
        bail!("Custom STT returned {status}: {}", compact_error(&body));
    }

    extract_transcript(&body).context("Custom STT returned no transcript")
}

fn clean_cloud_transcript(text: &str, settings: &AppSettings) -> String {
    let corrected = apply_custom_words(
        text,
        &settings.custom_words,
        settings.word_correction_threshold,
    );
    let language = if settings.selected_language == "auto" {
        OutputLanguageEvidence::Unknown
    } else {
        OutputLanguageEvidence::UserSelected(settings.selected_language.clone())
    };
    let filtered = remove_filler_words(
        &corrected,
        &language,
        &settings.custom_filler_words,
        settings.filler_word_removal_enabled,
    );
    normalize_transcription_output(&filtered)
}

fn extract_transcript(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
        return Some(trimmed.to_string());
    };

    ["text", "transcript", "DisplayText", "display_text"]
        .iter()
        .find_map(|key| json.get(key).and_then(Value::as_str))
        .or_else(|| {
            json.get("NBest")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| {
                    item.get("Display")
                        .or_else(|| item.get("Lexical"))
                        .and_then(Value::as_str)
                })
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn encode_pcm16_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    wav.extend_from_slice(&((SAMPLE_RATE * 2) as u32).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

fn compact_error(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_encoder_writes_valid_header_and_samples() {
        let wav = encode_pcm16_wav(&[0.0, 1.0, -1.0]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(wav.len(), 50);
    }

    #[test]
    fn transcript_parser_supports_azure_and_custom_shapes() {
        assert_eq!(
            extract_transcript(r#"{"DisplayText":"Layher Ger\u00fcst"}"#).as_deref(),
            Some("Layher Gerüst")
        );
        assert_eq!(
            extract_transcript(r#"{"text":"custom result"}"#).as_deref(),
            Some("custom result")
        );
        assert_eq!(
            extract_transcript("plain text").as_deref(),
            Some("plain text")
        );
    }

    #[test]
    fn azure_region_expands_to_short_audio_endpoint() {
        assert_eq!(
            azure_short_audio_endpoint("germanywestcentral").unwrap(),
            "https://germanywestcentral.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1"
        );
        assert_eq!(
            azure_short_audio_endpoint("https://custom.example/stt").unwrap(),
            "https://custom.example/stt"
        );
    }
}
