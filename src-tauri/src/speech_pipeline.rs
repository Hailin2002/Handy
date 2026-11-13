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
const AZURE_AUDIO_EVENT_BYTES: usize = SAMPLE_RATE * 2 * 30 / 1_000;
const AZURE_TARGET_PEAK: f32 = 0.75;
const AZURE_MAX_INPUT_GAIN: f32 = 8.0;
const AZURE_GAIN_RECOVERY_RATE: f32 = 0.15;
// Without translation, stopping must feel immediate. All completed phrases and
// the current interim are already mirrored into Rust, so a delayed browser SDK
// callback must not hold the paste path beyond one second. DeepL intentionally
// keeps a longer budget because it performs a final network translation batch.
const AZURE_FINALIZE_TIMEOUT: Duration = Duration::from_millis(900);
const AZURE_TRANSLATED_FINALIZE_TIMEOUT: Duration = Duration::from_secs(30);

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
    pub recognized_audio_end_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct CloudTranscriptCleanupSettings {
    custom_words: Vec<String>,
    word_correction_threshold: f64,
    selected_language: String,
    custom_filler_words: Option<Vec<String>>,
    filler_word_removal_enabled: bool,
}

impl From<&AppSettings> for CloudTranscriptCleanupSettings {
    fn from(settings: &AppSettings) -> Self {
        Self {
            custom_words: settings.custom_words.clone(),
            word_correction_threshold: settings.word_correction_threshold,
            selected_language: settings.selected_language.clone(),
            custom_filler_words: settings.custom_filler_words.clone(),
            filler_word_removal_enabled: settings.filler_word_removal_enabled,
        }
    }
}

struct AzureSessionState {
    session_id: u64,
    deepl_enabled: bool,
    cleanup: CloudTranscriptCleanupSettings,
    source_segments: Vec<String>,
    pending_translation: Vec<String>,
    translating_source: String,
    translated_segments: Vec<String>,
    interim: String,
    recognized_audio_end_ms: u64,
    completion_tx: Option<oneshot::Sender<std::result::Result<(), String>>>,
    completion_status: Option<std::result::Result<(), String>>,
}

impl AzureSessionState {
    fn new(session_id: u64, settings: &AppSettings) -> Self {
        Self {
            session_id,
            deepl_enabled: settings.deepl_translation_enabled,
            cleanup: settings.into(),
            source_segments: Vec::new(),
            pending_translation: Vec::new(),
            translating_source: String::new(),
            translated_segments: Vec::new(),
            interim: String::new(),
            recognized_audio_end_ms: 0,
            completion_tx: None,
            completion_status: None,
        }
    }
}

#[derive(Debug)]
struct AzureLiveGain {
    current: f32,
    minimum: f32,
    maximum: f32,
    sum: f64,
    frames: u64,
}

impl AzureLiveGain {
    fn new() -> Self {
        Self {
            // Start ready for quiet USB interfaces. A loud frame reduces the
            // gain before that same frame is encoded, so this cannot clip the
            // beginning of a loud recording.
            current: AZURE_MAX_INPUT_GAIN,
            minimum: AZURE_MAX_INPUT_GAIN,
            maximum: AZURE_MAX_INPUT_GAIN,
            sum: 0.0,
            frames: 0,
        }
    }

    fn apply(&mut self, frame: &[f32], output: &mut Vec<u8>) {
        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let desired = if peak <= f32::EPSILON {
            AZURE_MAX_INPUT_GAIN
        } else {
            (AZURE_TARGET_PEAK / peak).clamp(1.0, AZURE_MAX_INPUT_GAIN)
        };

        // Reduce gain immediately when a loud frame arrives. Recover toward a
        // higher gain smoothly so a single transient cannot make the following
        // quiet words disappear or create audible pumping for Azure's VAD.
        self.current = if desired < self.current {
            desired
        } else {
            self.current + (desired - self.current) * AZURE_GAIN_RECOVERY_RATE
        };
        self.minimum = self.minimum.min(self.current);
        self.maximum = self.maximum.max(self.current);
        self.sum += self.current as f64;
        self.frames += 1;

        for sample in frame {
            let pcm = ((sample * self.current).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            output.extend_from_slice(&pcm.to_le_bytes());
        }
    }

    fn stats(&self) -> (f32, f32, f32) {
        let average = if self.frames == 0 {
            self.current
        } else {
            (self.sum / self.frames as f64) as f32
        };
        (self.minimum, average, self.maximum)
    }
}

#[derive(Debug)]
struct AzureLiveAudioBuffer {
    pcm: Vec<u8>,
    gain: AzureLiveGain,
}

impl AzureLiveAudioBuffer {
    fn new() -> Self {
        Self {
            pcm: Vec::with_capacity(AZURE_AUDIO_EVENT_BYTES * 2),
            gain: AzureLiveGain::new(),
        }
    }
}

/// Bridges Speak's already-selected microphone stream into Microsoft's official
/// browser Speech SDK running in the hidden main webview. Audio is forwarded in
/// fixed 30 ms PCM packets, matching the recorder and Azure's native realtime
/// cadence. Keeping exact packet sizes also prevents a delayed webview event
/// from turning several recorder frames into one burst.
pub struct AzureSpeechStreamManager {
    app: AppHandle,
    active: AtomicBool,
    next_session_id: AtomicU64,
    current_session_id: AtomicU64,
    emitted_audio_bytes: AtomicU64,
    emitted_audio_chunks: AtomicU64,
    audio_buffer: Mutex<AzureLiveAudioBuffer>,
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
            emitted_audio_bytes: AtomicU64::new(0),
            emitted_audio_chunks: AtomicU64::new(0),
            audio_buffer: Mutex::new(AzureLiveAudioBuffer::new()),
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
        *self.audio_buffer.lock_recover() = AzureLiveAudioBuffer::new();
        *self.state.lock_recover() = Some(AzureSessionState::new(session_id, settings));
        self.current_session_id.store(session_id, Ordering::Release);
        self.emitted_audio_bytes.store(0, Ordering::Release);
        self.emitted_audio_chunks.store(0, Ordering::Release);
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

        let mut ready = Vec::new();
        {
            let mut buffer = self.audio_buffer.lock_recover();
            let AzureLiveAudioBuffer { pcm, gain } = &mut *buffer;
            gain.apply(frame, pcm);
            while pcm.len() >= AZURE_AUDIO_EVENT_BYTES {
                ready.push(pcm.drain(..AZURE_AUDIO_EVENT_BYTES).collect());
            }
        }
        for bytes in ready {
            self.emit_audio(bytes);
        }
    }

    fn emit_audio(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let byte_count = bytes.len() as u64;
        let session_id = self.current_session_id.load(Ordering::Acquire);
        let pcm_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        match self.app.emit_to(
            "main",
            "azure-speech-audio",
            AzureAudioChunkPayload {
                session_id,
                pcm_base64,
            },
        ) {
            Ok(()) => {
                self.emitted_audio_bytes
                    .fetch_add(byte_count, Ordering::Relaxed);
                self.emitted_audio_chunks.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => warn!("Failed to forward Azure audio chunk: {error}"),
        }
    }

    fn flush_audio(&self) {
        let bytes = std::mem::take(&mut self.audio_buffer.lock_recover().pcm);
        self.emit_audio(bytes);
    }

    pub fn set_interim(&self, session_id: u64, text: String, audio_end_ms: u64) {
        let mut guard = self.state.lock_recover();
        let Some(state) = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
        else {
            return;
        };
        // Interim events can arrive many times per second. Keep this path free
        // of settings-store and Windows Credential Manager I/O; final segments
        // receive the full custom-word/filler cleanup from the session snapshot.
        state.interim = normalize_transcription_output(&text);
        state.recognized_audio_end_ms = state.recognized_audio_end_ms.max(audio_end_ms);
        debug!(
            "Azure continuous STT session {session_id} interim received ({} chars)",
            state.interim.chars().count()
        );
        self.emit_overlay_text(state);
    }

    pub fn commit_segment(&self, session_id: u64, text: String, audio_end_ms: u64) {
        let mut guard = self.state.lock_recover();
        let Some(state) = guard
            .as_mut()
            .filter(|state| state.session_id == session_id)
        else {
            return;
        };
        // Pin cleanup settings at session start. `get_settings()` hydrates all
        // secret maps from the OS keychain and caused severe intermittent UI
        // and audio stalls when called for every Azure recognition event.
        let cleaned = clean_cloud_transcript_with_settings(&text, &state.cleanup);
        if cleaned.trim().is_empty() {
            return;
        }
        state.recognized_audio_end_ms = state.recognized_audio_end_ms.max(audio_end_ms);
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
                    crate::events::TRANSCRIPTION_ERROR,
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
        let deepl_enabled = self
            .state
            .lock_recover()
            .as_ref()
            .is_some_and(|state| state.deepl_enabled);
        let timeout = if deepl_enabled {
            AZURE_TRANSLATED_FINALIZE_TIMEOUT
        } else {
            AZURE_FINALIZE_TIMEOUT
        };
        self.finalize_with_timeout(timeout).await
    }

    pub async fn finalize_with_timeout(&self, timeout: Duration) -> Result<AzureStreamingResult> {
        self.active.store(false, Ordering::Release);
        self.flush_audio();
        let session_id = self.current_session_id.load(Ordering::Acquire);
        debug!(
            "Azure continuous STT session {session_id} transport sent {} bytes in {} chunks",
            self.emitted_audio_bytes.load(Ordering::Acquire),
            self.emitted_audio_chunks.load(Ordering::Acquire)
        );
        let (minimum_gain, average_gain, maximum_gain) =
            self.audio_buffer.lock_recover().gain.stats();
        debug!(
            "Azure continuous STT session {session_id} live input gain min={minimum_gain:.2}x avg={average_gain:.2}x max={maximum_gain:.2}x"
        );
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
        let source_text = complete_source_text(&state);
        if source_text.is_empty() {
            bail!("Azure Speech returned no transcript");
        }
        let translated_text = if state.deepl_enabled {
            // A timeout must degrade to untranslated trailing text, never to
            // missing text. `translating_source` and `pending_translation`
            // cover a DeepL request still in flight; `interim` covers Azure's
            // last phrase when its final event missed the deadline.
            let translated = complete_translated_text(&state);
            (!translated.is_empty()).then_some(translated)
        } else {
            None
        };
        Ok(AzureStreamingResult {
            source_text,
            translated_text,
            recognized_audio_end_ms: state.recognized_audio_end_ms,
        })
    }

    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
        self.audio_buffer.lock_recover().pcm.clear();
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

fn push_distinct_segment(segments: &mut Vec<String>, segment: &str) {
    let segment = segment.trim();
    if segment.is_empty() || segments.last().is_some_and(|last| last.trim() == segment) {
        return;
    }
    segments.push(segment.to_string());
}

fn complete_source_text(state: &AzureSessionState) -> String {
    let mut segments = state.source_segments.clone();
    push_distinct_segment(&mut segments, &state.interim);
    join_segments(&segments)
}

fn complete_translated_text(state: &AzureSessionState) -> String {
    let mut segments = state.translated_segments.clone();
    push_distinct_segment(&mut segments, &state.translating_source);
    for pending in &state.pending_translation {
        push_distinct_segment(&mut segments, pending);
    }
    push_distinct_segment(&mut segments, &state.interim);
    join_segments(&segments)
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
    audio_end_ms: u64,
) {
    manager.set_interim(session_id, text, audio_end_ms);
}

#[tauri::command]
#[specta::specta]
pub fn azure_speech_final(
    manager: State<'_, Arc<AzureSpeechStreamManager>>,
    session_id: u64,
    text: String,
    audio_end_ms: u64,
) {
    manager.commit_segment(session_id, text, audio_end_ms);
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
    let raw = match settings.speech_input_provider {
        SpeechInputProvider::Azure => return transcribe_azure_recording(samples, settings).await,
        SpeechInputProvider::Custom => {
            transcribe_custom(&encode_pcm16_wav(samples), settings).await?
        }
        SpeechInputProvider::Local => bail!("Local STT must be handled by TranscriptionManager"),
    };

    Ok(clean_cloud_transcript(&raw, settings))
}

/// Detect a continuous Azure result that stopped covering the captured speech.
/// Azure result offsets tell us how far the recognizer actually got; checking
/// for voiced audio after that point catches the important failure mode where a
/// plausible first sentence hid an entirely missing tail. The older text-rate
/// heuristic remains as a conservative secondary signal for SDKs that omit
/// offsets.
pub(crate) fn azure_stream_needs_recovery(
    samples: &[f32],
    transcript: &str,
    recognized_audio_end_ms: u64,
) -> bool {
    let captured_audio_ms = (samples.len() as u64)
        .saturating_mul(1_000)
        .saturating_div(SAMPLE_RATE as u64);
    if recognized_audio_end_ms > captured_audio_ms.saturating_add(1_000) {
        // The SDK cannot have recognized materially more audio than the
        // recorder captured. This indicates duplicate frontend consumers (or
        // another corrupt SDK timeline), so offsets cannot prove completeness.
        return true;
    }

    if recognized_audio_end_ms > 0
        && has_likely_speech_after(samples, recognized_audio_end_ms.saturating_add(600))
    {
        return true;
    }

    let duration_seconds = samples.len() as f64 / SAMPLE_RATE as f64;
    if duration_seconds < 8.0 {
        return false;
    }

    let visible_characters = transcript
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    let minimum_expected = (duration_seconds * 3.0).ceil() as usize;
    visible_characters < minimum_expected
}

fn has_likely_speech_after(samples: &[f32], after_ms: u64) -> bool {
    let start = ((after_ms as usize).saturating_mul(SAMPLE_RATE) / 1_000).min(samples.len());
    let frame_samples = SAMPLE_RATE * 30 / 1_000;
    let mut consecutive_voiced = 0;
    for frame in samples[start..].chunks(frame_samples) {
        if frame.is_empty() {
            continue;
        }
        let square_sum = frame
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>();
        let rms = (square_sum / frame.len() as f64).sqrt() as f32;
        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        if rms >= 0.003 && peak >= 0.02 {
            consecutive_voiced += 1;
            if consecutive_voiced >= 2 {
                return true;
            }
        } else {
            consecutive_voiced = 0;
        }
    }
    false
}

/// Re-run Azure recognition from the complete recorder-owned PCM buffer using
/// Azure's synchronous fast-transcription API. Unlike the legacy short-audio
/// endpoint this accepts long recordings and was measured at roughly two
/// seconds for the 19-second regression memo instead of 18 seconds.
pub async fn transcribe_azure_recording(samples: &[f32], settings: &AppSettings) -> Result<String> {
    let endpoint = azure_fast_audio_endpoint(&settings.azure_speech_endpoint)?;
    let api_key =
        crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
            .unwrap_or_default();
    if api_key.is_empty() {
        bail!("Azure STT is selected but no API key is configured");
    }

    let url = format!(
        "{}/speechtotext/transcriptions:transcribe?api-version=2025-10-15",
        endpoint.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .context("Failed to create Azure Speech HTTP client")?;
    let recovery_gain = azure_recovery_gain(samples);
    debug!(
        "Azure recording recovery is sending {} samples with {:.2}x input gain",
        samples.len(),
        recovery_gain
    );

    let audio = reqwest::multipart::Part::bytes(encode_pcm16_wav_with_gain(samples, recovery_gain))
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .context("Failed to create Azure audio upload")?;
    let definition = serde_json::json!({
        "locales": [settings.azure_speech_language]
    })
    .to_string();
    let form = reqwest::multipart::Form::new()
        .text("definition", definition)
        .part("audio", audio);
    let response = client
        .post(url)
        .header("Ocp-Apim-Subscription-Key", &api_key)
        .header(ACCEPT, "application/json")
        .multipart(form)
        .send()
        .await
        .context("Failed to contact Azure fast transcription")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read Azure fast transcription response")?;
    if !status.is_success() {
        bail!(
            "Azure fast transcription returned {status}: {}",
            compact_error(&body)
        );
    }

    let transcript = extract_transcript(&body).unwrap_or_default();
    if transcript.is_empty() {
        bail!("Azure Speech recovery returned no transcript");
    }
    Ok(clean_cloud_transcript(&transcript, settings))
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

fn azure_fast_audio_endpoint(configured: &str) -> Result<String> {
    let region = azure_region(configured)?;
    Ok(format!("https://{region}.api.cognitive.microsoft.com"))
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
    clean_cloud_transcript_with_settings(text, &CloudTranscriptCleanupSettings::from(settings))
}

fn clean_cloud_transcript_with_settings(
    text: &str,
    settings: &CloudTranscriptCleanupSettings,
) -> String {
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
        .map(str::to_string)
        .or_else(|| {
            let phrases = json.get("combinedPhrases")?.as_array()?;
            let text = phrases
                .iter()
                .filter_map(|phrase| phrase.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (!text.is_empty()).then_some(text)
        })
        .or_else(|| {
            json.get("NBest")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| {
                    item.get("Display")
                        .or_else(|| item.get("Lexical"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string)
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn encode_pcm16_wav(samples: &[f32]) -> Vec<u8> {
    encode_pcm16_wav_with_gain(samples, 1.0)
}

fn azure_recovery_gain(samples: &[f32]) -> f32 {
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    if peak <= f32::EPSILON {
        return 1.0;
    }
    (AZURE_TARGET_PEAK / peak).clamp(1.0, AZURE_MAX_INPUT_GAIN)
}

fn encode_pcm16_wav_with_gain(samples: &[f32], gain: f32) -> Vec<u8> {
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
        let pcm = ((sample * gain).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
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
    fn azure_recovery_gain_lifts_quiet_audio_without_overamplifying() {
        assert_eq!(azure_recovery_gain(&[0.0, 0.05, -0.05]), 8.0);
        assert_eq!(azure_recovery_gain(&[0.0, 0.75, -0.75]), 1.0);
        assert_eq!(azure_recovery_gain(&[0.0, 0.5, -0.5]), 1.5);
        assert_eq!(azure_recovery_gain(&[0.0, 0.0]), 1.0);
    }

    #[test]
    fn azure_live_gain_lifts_quiet_frames_before_streaming() {
        let mut gain = AzureLiveGain::new();
        let mut pcm = Vec::new();
        gain.apply(&[0.0, 0.05, -0.05], &mut pcm);

        let positive = i16::from_le_bytes(pcm[2..4].try_into().unwrap()) as f32 / i16::MAX as f32;
        let negative = i16::from_le_bytes(pcm[4..6].try_into().unwrap()) as f32 / i16::MAX as f32;
        assert!((positive - 0.4).abs() < 0.001);
        assert!((negative + 0.4).abs() < 0.001);
        assert_eq!(gain.stats(), (8.0, 8.0, 8.0));
    }

    #[test]
    fn azure_live_gain_reacts_before_encoding_a_loud_frame() {
        let mut gain = AzureLiveGain::new();
        let mut pcm = Vec::new();
        gain.apply(&[0.5, -0.5], &mut pcm);

        let positive = i16::from_le_bytes(pcm[0..2].try_into().unwrap()) as f32 / i16::MAX as f32;
        let negative = i16::from_le_bytes(pcm[2..4].try_into().unwrap()) as f32 / i16::MAX as f32;
        assert!((positive - AZURE_TARGET_PEAK).abs() < 0.001);
        assert!((negative + AZURE_TARGET_PEAK).abs() < 0.001);
        assert_eq!(gain.current, 1.5);
    }

    #[test]
    fn azure_live_gain_recovers_smoothly_after_a_transient() {
        let mut gain = AzureLiveGain::new();
        let mut pcm = Vec::new();
        gain.apply(&[0.5], &mut pcm);
        gain.apply(&[0.05], &mut pcm);

        assert!((gain.current - 2.475).abs() < 0.001);
        assert!(gain.current < AZURE_MAX_INPUT_GAIN);
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
        assert_eq!(
            extract_transcript(
                r#"{"combinedPhrases":[{"text":"First phrase."},{"text":"Second phrase."}]}"#
            )
            .as_deref(),
            Some("First phrase. Second phrase.")
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

    fn session_with_segments() -> AzureSessionState {
        let mut settings = crate::settings::get_default_settings();
        settings.deepl_translation_enabled = true;
        let mut state = AzureSessionState::new(1, &settings);
        state.source_segments = vec!["First sentence.".to_string()];
        state.pending_translation = vec!["Third sentence.".to_string()];
        state.translating_source = "Second sentence.".to_string();
        state.translated_segments = vec!["Erster Satz.".to_string()];
        state.interim = "Fourth sentence.".to_string();
        state
    }

    #[test]
    fn azure_cleanup_settings_are_pinned_for_the_session() {
        let mut settings = crate::settings::get_default_settings();
        settings.custom_words = vec!["Speak".to_string()];
        let state = AzureSessionState::new(1, &settings);

        settings.custom_words = vec!["Changed later".to_string()];

        assert_eq!(state.cleanup.custom_words, vec!["Speak"]);
        assert_ne!(
            state.cleanup,
            CloudTranscriptCleanupSettings::from(&settings)
        );
    }

    #[test]
    fn azure_timeout_keeps_the_last_interim_phrase() {
        let state = session_with_segments();
        assert_eq!(
            complete_source_text(&state),
            "First sentence. Fourth sentence."
        );
    }

    #[test]
    fn azure_timeout_never_drops_untranslated_trailing_segments() {
        let state = session_with_segments();
        assert_eq!(
            complete_translated_text(&state),
            "Erster Satz. Second sentence. Third sentence. Fourth sentence."
        );
    }

    #[test]
    fn azure_final_interim_is_not_duplicated() {
        let mut state = session_with_segments();
        state.source_segments.push("Fourth sentence.".to_string());
        state.interim = "Fourth sentence.".to_string();
        assert_eq!(
            complete_source_text(&state),
            "First sentence. Fourth sentence."
        );
    }

    #[test]
    fn short_azure_recordings_do_not_trigger_recovery() {
        let samples = vec![0.0; SAMPLE_RATE * 7];
        assert!(!azure_stream_needs_recovery(&samples, "Hello.", 6_500));
    }

    #[test]
    fn implausibly_short_azure_stream_triggers_recovery() {
        let samples = vec![0.0; SAMPLE_RATE * 24];
        assert!(azure_stream_needs_recovery(
            &samples,
            "This is only the first sentence.",
            0,
        ));
        assert!(!azure_stream_needs_recovery(
            &samples,
            "This sufficiently long transcript contains all of the spoken content from the recording and therefore does not need a recovery request.",
            23_500,
        ));
    }

    #[test]
    fn speech_after_azure_coverage_triggers_recovery_even_with_plausible_text() {
        let mut samples = vec![0.0; SAMPLE_RATE * 12];
        let trailing_start = SAMPLE_RATE * 8;
        for (index, sample) in samples[trailing_start..trailing_start + SAMPLE_RATE]
            .iter_mut()
            .enumerate()
        {
            *sample = if index % 20 < 10 { 0.04 } else { -0.04 };
        }

        assert!(azure_stream_needs_recovery(
            &samples,
            "This first sentence is long enough to fool a character-count heuristic.",
            6_000,
        ));
    }

    #[test]
    fn impossible_azure_coverage_triggers_recovery() {
        let samples = vec![0.0; SAMPLE_RATE * 33];
        assert!(azure_stream_needs_recovery(
            &samples,
            "A plausible but incomplete streaming transcript.",
            53_950,
        ));
    }

    #[test]
    fn trailing_silence_after_azure_coverage_does_not_trigger_recovery() {
        let samples = vec![0.0; SAMPLE_RATE * 12];
        assert!(!azure_stream_needs_recovery(
            &samples,
            "This complete sentence has enough visible characters for twelve seconds.",
            6_000,
        ));
    }
}
