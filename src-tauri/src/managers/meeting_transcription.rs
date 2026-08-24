//! Local STT queue for meeting mode (plan §13 Phase 5, stt-provider-contract).
//!
//! One worker drains the persistent `meeting_stt_jobs` table: it claims a due
//! job, decodes the referenced chunk WAVs from the session directory, trims
//! leading/trailing silence with Silero VAD, runs the local Whisper engine
//! through [`TranscriptionManager`], and persists the resulting segments.
//!
//! Coexistence: while a dictation recording is active, the queue pauses at
//! segment boundaries (between jobs) so both consumers share the single local
//! model lease without interleaving loads.
//!
//! Retry policy (stt-provider-contract §3): transient engine failures are
//! rescheduled with bounded exponential backoff (`base * 2^(attempt-1)`,
//! capped at 10 minutes); after [`QueueConfig::max_attempts`] total attempts
//! the job moves to terminal `failed`. Missing/corrupt chunk audio is treated
//! as permanent — no retry can repair absent bytes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};

use super::meeting::{MeetingAudioSource, MeetingTranscriptSegmentDto};
use super::meeting_store::{
    MeetingRepository, NewTranscriptSegment, SqliteMeetingStore, StoreError, SttJob, SttJobState,
    TranscriptSegment, WorkerId,
};
use super::transcription::TranscriptionManager;
use crate::audio_toolkit::vad::{SileroVad, VoiceActivityDetector};

/// One recognized span, in milliseconds relative to the submitted audio start.
#[derive(Debug, Clone)]
pub struct EngineSegment {
    /// Span start relative to the job audio.
    pub start_ms_rel: i64,
    /// Span end relative to the job audio.
    pub end_ms_rel: i64,
    /// Recognized display text.
    pub text: String,
    /// Recognition confidence, when the engine reports one.
    pub confidence: Option<f32>,
}

/// Per-job execution context the routing engine dispatches on. Carried from
/// the session's persisted provider snapshot so a meeting keeps its provider
/// even if global settings change mid-recording.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// Provider name stored on the session (routing key), e.g. "Azure Speech".
    pub provider_name: String,
    /// Recognition language hint, when configured.
    pub language: Option<String>,
}

/// Why an engine call failed. Drives the queue's retry policy: permanent
/// failures (bad key, wrong endpoint) never retry; transient ones back off.
#[derive(Debug, Clone)]
pub enum EngineError {
    /// No retry can help — surface immediately as terminal.
    Permanent(String),
    /// Network hiccups, throttling and 5xx — retry with backoff.
    Transient(String),
}

impl From<CloudJobError> for EngineError {
    fn from(value: CloudJobError) -> Self {
        match value {
            CloudJobError::Permanent(m) => EngineError::Permanent(m),
            CloudJobError::Transient(m) => EngineError::Transient(m),
        }
    }
}

/// Re-exported so cloud engines can share one error vocabulary with the queue.
pub use super::meeting_cloud::CloudJobError;

/// Engine abstraction over every meeting STT backend (local Whisper, Azure
/// Fast Transcription, custom endpoints). Kept as a trait so unit tests can
/// substitute deterministic fakes instead of loading GGML or hitting network.
pub trait MeetingSttEngine: Send + Sync {
    /// Transcribes 16 kHz mono samples into relative segments.
    fn transcribe(
        &self,
        ctx: &JobContext,
        audio: &[f32],
    ) -> Result<Vec<EngineSegment>, EngineError>;

    /// Stable model label persisted on produced segments (`model_id`).
    fn model_label(&self) -> String;
}

/// Real engine backed by the shared [`TranscriptionManager`] (local Whisper).
pub struct LocalMeetingSttEngine {
    manager: Arc<TranscriptionManager>,
}

impl LocalMeetingSttEngine {
    pub fn new(manager: Arc<TranscriptionManager>) -> Self {
        Self { manager }
    }
}

impl MeetingSttEngine for LocalMeetingSttEngine {
    fn transcribe(
        &self,
        _ctx: &JobContext,
        audio: &[f32],
    ) -> Result<Vec<EngineSegment>, EngineError> {
        // The queue runs outside the dictation UI: warm the engine from the
        // user's selected model on first use, then transcribe. A missing
        // download is a config mistake (permanent); anything else may recover.
        if let Err(e) = self.manager.ensure_model_loaded() {
            let message = format!("meeting model warmup failed: {e:#}");
            return Err(
                if message.contains("not downloaded") || message.contains("no model selected") {
                    EngineError::Permanent(message)
                } else {
                    EngineError::Transient(message)
                },
            );
        }
        let text = self
            .manager
            .transcribe(audio.to_vec())
            .map_err(|e| EngineError::Transient(format!("meeting stt engine failed: {e:#}")))?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![EngineSegment {
            start_ms_rel: 0,
            end_ms_rel: i64::try_from(audio.len())
                .unwrap_or(i64::MAX)
                .saturating_mul(1000)
                / 16_000,
            text: text.to_string(),
            confidence: None,
        }])
    }

    fn model_label(&self) -> String {
        "local-whisper".to_string()
    }
}

/// Dispatches to the engine registered for the job's provider name. Assembled
/// once at startup from the configured providers; unknown names fall back to
/// whichever engine was registered first (the local engine).
pub struct RoutingMeetingSttEngine {
    engines: Mutex<Vec<(String, Arc<dyn MeetingSttEngine>)>>,
    fallback: Arc<dyn MeetingSttEngine>,
}

impl RoutingMeetingSttEngine {
    pub fn new(fallback: Arc<dyn MeetingSttEngine>) -> Self {
        Self {
            engines: Mutex::new(Vec::new()),
            fallback,
        }
    }

    pub fn register(self: &Arc<Self>, provider_name: &str, engine: Arc<dyn MeetingSttEngine>) {
        self.engines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((provider_name.to_string(), engine));
    }

    fn resolve(&self, provider_name: &str) -> Arc<dyn MeetingSttEngine> {
        self.engines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(name, _)| name == provider_name)
            .map(|(_, engine)| Arc::clone(engine))
            .unwrap_or_else(|| Arc::clone(&self.fallback))
    }
}

impl MeetingSttEngine for RoutingMeetingSttEngine {
    fn transcribe(
        &self,
        ctx: &JobContext,
        audio: &[f32],
    ) -> Result<Vec<EngineSegment>, EngineError> {
        self.resolve(&ctx.provider_name).transcribe(ctx, audio)
    }

    fn model_label(&self) -> String {
        "routed".to_string()
    }
}

/// Callbacks into the session owner. Implemented by `MeetingManager` behind a
/// `Weak` reference so the worker never keeps the manager alive.
pub trait QueueDelegate: Send + Sync {
    /// Freshly persisted segments; the delegate publishes `transcript-delta`.
    fn on_segments_persisted(&self, meeting_id: &str, segments: Vec<MeetingTranscriptSegmentDto>);

    /// A job reached a terminal or waiting state; recompute counters/state.
    fn on_queue_progress(&self, meeting_id: &str);
}

/// Tunables; exposed for tests.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Idle poll cadence when nothing wakes the worker earlier.
    pub poll_interval: Duration,
    /// Total attempts allowed per job before terminal failure.
    pub max_attempts: u32,
    /// First backoff duration in seconds; doubled each further attempt.
    pub backoff_base_secs: i64,
    /// Upper clamp for any computed backoff.
    pub backoff_max_secs: i64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            max_attempts: 3,
            backoff_base_secs: 30,
            backoff_max_secs: 600,
        }
    }
}

const WORKER_LABEL: &str = "meeting-stt-local";
/// VAD speech threshold, matching the dictation recorder's calibration.
const VAD_THRESHOLD: f32 = 0.3;
const VAD_FRAME_MS: usize = 30;
/// Frames of non-speech kept after a speech run (~450 ms at 30 ms frames).
const VAD_HANGOVER_FRAMES: usize = 15;
/// Frames of context prepended before each speech onset (~150 ms).
const VAD_PREFILL_FRAMES: usize = 5;
/// Silence gaps shorter than this many frames are merged into speech (~400 ms).
const VAD_MERGE_GAP_FRAMES: usize = 13;
/// Error strings are truncated before they reach persistence/UI (O7 redaction).
const MAX_ERROR_CHARS: usize = 300;

/// Classification of job failures.
enum JobFailure {
    /// No retry can help (missing/corrupt audio) — fail immediately.
    Permanent(String),
    /// Engine hiccup — backoff and retry until attempts are exhausted.
    Transient(String),
    /// Store-level trouble — leave the running lease to expire so another
    /// pass can reclaim the job.
    Store(StoreError),
}

struct WakeState {
    pending: Mutex<bool>,
    signal: Condvar,
}

/// Persistent job queue draining meeting STT work onto the local engine.
pub struct MeetingTranscriptionQueue {
    store: Arc<SqliteMeetingStore>,
    /// Session-directory root (`assets_root`); chunk keys resolve beneath it.
    audio_root: PathBuf,
    engine: Arc<dyn MeetingSttEngine>,
    dictation_active: Arc<dyn Fn() -> bool + Send + Sync>,
    vad_model_path: Option<PathBuf>,
    delegate: Arc<dyn QueueDelegate>,
    config: QueueConfig,
    wake: Arc<WakeState>,
    shutdown: AtomicBool,
}

impl MeetingTranscriptionQueue {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<SqliteMeetingStore>,
        audio_root: PathBuf,
        engine: Arc<dyn MeetingSttEngine>,
        dictation_active: Arc<dyn Fn() -> bool + Send + Sync>,
        vad_model_path: Option<PathBuf>,
        delegate: Arc<dyn QueueDelegate>,
        config: QueueConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            audio_root,
            engine,
            dictation_active,
            vad_model_path,
            delegate,
            config,
            wake: Arc::new(WakeState {
                pending: Mutex::new(false),
                signal: Condvar::new(),
            }),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Nudges the worker: sets the pending flag and interrupts the idle wait.
    pub fn wake(&self) {
        if let Ok(mut pending) = self.wake.pending.lock() {
            *pending = true;
            self.wake.signal.notify_all();
        }
    }

    /// Requests worker termination; takes effect between jobs.
    #[allow(dead_code)] // graceful-shutdown hook; exercised in unit tests
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// Spawns the background drain loop.
    pub fn spawn(self: &Arc<Self>) {
        let worker = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(WORKER_LABEL.to_string())
            .spawn(move || worker.run_loop());
        match spawned {
            Ok(handle) => std::mem::forget(handle),
            Err(e) => log::error!("Failed to spawn meeting STT worker: {}", e),
        }
    }

    /// Drain loop: wait for wake or poll timeout, then process every due job.
    fn run_loop(self: Arc<Self>) {
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            // Interruptible idle wait: sleep until woken or the poll cadence
            // elapses, whichever comes first.
            let mut pending = self.wake.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !*pending {
                let deadline = Instant::now() + self.config.poll_interval;
                while !*pending {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    let (guard, _) = self
                        .wake
                        .signal
                        .wait_timeout(pending, deadline - now)
                        .unwrap_or_else(|e| e.into_inner());
                    pending = guard;
                }
            }
            *pending = false;
            drop(pending);

            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            // Drain everything that is due right now.
            loop {
                if self.shutdown.load(Ordering::SeqCst) || !self.poll_once() {
                    break;
                }
            }
        }
    }

    /// Claims and processes at most one due job.
    ///
    /// Returns `true` when a job was processed (caller should keep polling);
    /// `false` means nothing is claimable right now — including while a
    /// dictation recording holds the model (coexistence gate, plan §9).
    fn poll_once(&self) -> bool {
        if (self.dictation_active)() {
            return false;
        }
        let now = Utc::now();
        match self
            .store
            .claim_due_stt_job(WorkerId(WORKER_LABEL.to_string()), now)
        {
            Ok(Some(claim)) => {
                self.process_job(claim.job);
                true
            }
            Ok(None) => false,
            Err(e) => {
                log::error!("Meeting STT queue failed to claim a job: {}", e);
                false
            }
        }
    }

    /// Runs one claimed job to a terminal or waiting state.
    fn process_job(&self, job: SttJob) {
        let meeting_id = job.meeting_id.0.clone();
        match self.run_job(&job) {
            Ok(segments) => {
                if let Err(e) =
                    self.store
                        .transition_stt_job(job.id.clone(), SttJobState::Succeeded, None)
                {
                    log::error!("Failed to mark meeting STT job succeeded: {}", e);
                    self.delegate.on_queue_progress(&meeting_id);
                    return;
                }
                if !segments.is_empty() {
                    self.delegate.on_segments_persisted(&meeting_id, segments);
                }
                self.delegate.on_queue_progress(&meeting_id);
            }
            Err(JobFailure::Permanent(reason)) => {
                log::warn!(
                    "Meeting STT job {} failed permanently: {}",
                    job.id.0,
                    reason
                );
                let _ = self.store.transition_stt_job(
                    job.id.clone(),
                    SttJobState::Failed,
                    Some(redact(&reason)),
                );
                self.delegate.on_queue_progress(&meeting_id);
            }
            Err(JobFailure::Transient(reason)) => {
                let attempt = job.attempt_count.max(1);
                if attempt >= self.config.max_attempts {
                    log::warn!(
                        "Meeting STT job {} failed after {} attempts: {}",
                        job.id.0,
                        attempt,
                        reason
                    );
                    let _ = self.store.transition_stt_job(
                        job.id.clone(),
                        SttJobState::Failed,
                        Some(redact(&reason)),
                    );
                } else {
                    let backoff = self.backoff_after(attempt);
                    let next_at = Utc::now() + TimeDelta::seconds(backoff);
                    log::info!(
                        "Meeting STT job {} attempt {} failed, retrying in {}s: {}",
                        job.id.0,
                        attempt,
                        backoff,
                        reason
                    );
                    if let Err(e) = self.store.reschedule_stt_job(
                        job.id.clone(),
                        next_at,
                        Some(redact(&reason)),
                    ) {
                        log::error!("Failed to reschedule meeting STT job: {}", e);
                    }
                }
                self.delegate.on_queue_progress(&meeting_id);
            }
            Err(JobFailure::Store(e)) => {
                // Leave the running lease; it expires and the job is reclaimed.
                log::error!("Meeting STT job {} store error: {}", job.id.0, e);
            }
        }
    }

    /// The actual pipeline: load → decode → trim → transcribe → persist.
    fn run_job(&self, job: &SttJob) -> Result<Vec<MeetingTranscriptSegmentDto>, JobFailure> {
        let chunks = self
            .store
            .chunks_for_job(job.id.clone())
            .map_err(|e| match e {
                StoreError::NotFound { .. } => {
                    JobFailure::Permanent(format!("chunk row missing for job {}: {}", job.id.0, e))
                }
                other => JobFailure::Store(other),
            })?;
        if chunks.is_empty() {
            return Err(JobFailure::Permanent(format!(
                "job {} references no chunks",
                job.id.0
            )));
        }

        let mut audio = Vec::new();
        for chunk in &chunks {
            let path = self.chunk_path(&job.meeting_id.0, &chunk.storage_key);
            let decoded = decode_wav_pcm16_mono(&path).map_err(|reason| {
                JobFailure::Permanent(format!(
                    "chunk {} unreadable: {}",
                    chunk.envelope.id.0, reason
                ))
            })?;
            audio.extend_from_slice(&decoded);
        }

        let audio = match &self.vad_model_path {
            Some(vad_path) => match trim_silence(&audio, vad_path) {
                Some(trimmed) if !trimmed.is_empty() => trimmed,
                Some(_) => {
                    // All silence — succeed without segments instead of letting
                    // Whisper hallucinate on noise (plan line 1336 intent).
                    return Ok(Vec::new());
                }
                None => audio,
            },
            None => audio,
        };
        if audio.is_empty() {
            return Ok(Vec::new());
        }

        let ctx = JobContext {
            provider_name: job.provider.clone(),
            language: job
                .config_snapshot
                .get("language")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        };
        let engine_segments = self.engine.transcribe(&ctx, &audio).map_err(|e| match e {
            EngineError::Permanent(message) => JobFailure::Permanent(message),
            EngineError::Transient(message) => JobFailure::Transient(message),
        })?;
        if engine_segments.is_empty() {
            return Ok(Vec::new());
        }

        // Defensive pre-filter: a misbehaving engine may emit blank spans
        // (e.g. a custom endpoint echoing whitespace). Persisting them would
        // put unrenderable rows into every transcript view — and filtering
        // before sequence assignment keeps the numbering gap-free.
        let kept: Vec<EngineSegment> = engine_segments
            .into_iter()
            .filter(|segment| !segment.text.trim().is_empty())
            .collect();
        if kept.is_empty() {
            return Ok(Vec::new());
        }
        let model_label = self.engine.model_label();
        let mut drafts = Vec::with_capacity(kept.len());
        for (sequence, segment) in (self.next_sequence(&job.meeting_id)?..).zip(kept) {
            // Engine offsets are relative to the submitted audio; project them
            // onto the session timeline and clamp to the job's coverage.
            let rel_start = segment.start_ms_rel.max(0);
            let rel_end = segment.end_ms_rel.max(rel_start);
            let start_ms = (job.timeline_start_ms + rel_start).min(job.timeline_end_ms);
            let end_ms = (job.timeline_start_ms + rel_end).clamp(start_ms, job.timeline_end_ms);
            drafts.push(NewTranscriptSegment {
                source: job.source.clone(),
                start_ms,
                end_ms,
                text: segment.text.trim().to_string(),
                word_timestamps: None,
                confidence: segment.confidence,
                provider: job.provider.clone(),
                model_id: Some(model_label.clone()),
                sequence,
            });
        }
        let inserted = self
            .store
            .insert_segments(job.id.clone(), drafts)
            .map_err(JobFailure::Store)?;
        Ok(inserted.iter().map(segment_dto).collect())
    }

    /// Absolute path of a stored chunk: `<assets_root>/<session>/storage_key`.
    fn chunk_path(&self, meeting_id: &str, storage_key: &str) -> PathBuf {
        self.audio_root.join(meeting_id).join(storage_key)
    }

    /// Next free segment ordering slot within the session.
    /// Next free segment ordering slot within the session. A single indexed
    /// MAX lookup — this runs once per persisted job, so materializing every
    /// existing segment (texts, word timestamps) just for its maximum would
    /// scale linearly with transcript length.
    fn next_sequence(&self, meeting_id: &super::meeting_store::Uuid) -> Result<i64, JobFailure> {
        self.store
            .max_segment_sequence(meeting_id.clone())
            .map(|max| max + 1)
            .map_err(JobFailure::Store)
    }

    /// Bounded exponential backoff for the attempt just failed.
    fn backoff_after(&self, attempt: u32) -> i64 {
        let exp = attempt.saturating_sub(1).min(8);
        (self.config.backoff_base_secs.saturating_mul(1i64 << exp))
            .min(self.config.backoff_max_secs)
    }
}

/// Maps a stored segment onto its wire shape.
fn segment_dto(segment: &TranscriptSegment) -> MeetingTranscriptSegmentDto {
    MeetingTranscriptSegmentDto {
        id: segment.envelope.id.0.clone(),
        job_id: Some(segment.job_id.0.clone()),
        source: source_token(&segment.source),
        start_ms: segment.start_ms,
        end_ms: segment.end_ms,
        sequence: segment.sequence,
        text: segment.text.clone(),
        confidence: segment.confidence,
    }
}

fn source_token(token: &str) -> MeetingAudioSource {
    match token {
        "system" => MeetingAudioSource::System,
        _ => MeetingAudioSource::Microphone,
    }
}

fn redact(message: &str) -> String {
    let cleaned: String = message.chars().filter(|c| !c.is_control()).collect();
    let mut out: String = cleaned.chars().take(MAX_ERROR_CHARS).collect();
    if cleaned.chars().count() > MAX_ERROR_CHARS {
        out.push('…');
    }
    out
}

/// Decodes a PCM16 mono WAV written by the recorder into f32 samples.
pub(crate) fn decode_wav_pcm16_mono(path: &Path) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("open failed: {e}"))?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(format!("expected mono, got {} channels", spec.channels));
    }
    if spec.sample_rate != 16_000 {
        return Err(format!("expected 16000 Hz, got {}", spec.sample_rate));
    }
    match spec.sample_format {
        hound::SampleFormat::Int => {
            if spec.bits_per_sample != 16 {
                return Err(format!("expected 16-bit PCM, got {}", spec.bits_per_sample));
            }
            Ok(reader
                .into_samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("decode failed: {e}"))?
                .into_iter()
                .map(|sample| f32::from(sample) / 32768.0)
                .collect())
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("decode failed: {e}")),
    }
}

/// Drops leading/trailing silence (and merges short gaps) using Silero VAD.
///
/// Returns `None` when VAD is unavailable (fall back to raw audio); an empty
/// vector when every frame is classified as silence.
fn trim_silence(samples: &[f32], vad_model_path: &Path) -> Option<Vec<f32>> {
    let frame_len = 16_000 * VAD_FRAME_MS / 1000;
    if samples.len() < frame_len {
        return None;
    }
    let mut vad = SileroVad::new(vad_model_path, VAD_THRESHOLD).ok()?;
    vad.set_hangover_frames(VAD_HANGOVER_FRAMES);

    let keep: Vec<bool> = samples
        .chunks_exact(frame_len)
        .map(|frame| vad.is_voice(frame).ok())
        .collect::<Option<Vec<_>>>()?;

    // Expand speech runs with prefill/hangover and merge tiny gaps.
    let mut expanded = vec![false; keep.len()];
    for (index, voiced) in keep.iter().enumerate() {
        if !voiced {
            continue;
        }
        let from = index.saturating_sub(VAD_PREFILL_FRAMES);
        let to = (index + VAD_HANGOVER_FRAMES).min(keep.len().saturating_sub(1));
        expanded[from..=to].fill(true);
    }
    merge_short_gaps(&mut expanded, VAD_MERGE_GAP_FRAMES);

    let mut out = Vec::with_capacity(samples.len());
    for (index, voiced) in expanded.iter().enumerate() {
        if *voiced {
            out.extend_from_slice(&samples[index * frame_len..(index + 1) * frame_len]);
        }
    }
    // Trailing partial frame follows the last kept frame when voiced.
    let remainder = samples.len() % frame_len;
    if remainder > 0 && expanded.last().copied().unwrap_or(false) {
        out.extend_from_slice(&samples[samples.len() - remainder..]);
    }
    Some(out)
}

/// Fills silence gaps shorter than `max_gap` frames inside speech regions.
fn merge_short_gaps(flags: &mut [bool], max_gap: usize) {
    let len = flags.len();
    let mut index = 0;
    while index < len {
        if flags[index] {
            index += 1;
            continue;
        }
        let gap_start = index;
        let gap_end = flags[gap_start..]
            .iter()
            .position(|voiced| *voiced)
            .map_or(len, |offset| gap_start + offset);
        if gap_start > 0 && gap_end < len && gap_end - gap_start <= max_gap {
            flags[gap_start..gap_end].fill(true);
        }
        index = gap_end;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::managers::meeting_store::{
        AudioChunk, ChunkFacts, MeetingRepository, NewAudioChunk, NewMeetingSession, NewSttJob,
        Uuid,
    };

    type TestStore = SqliteMeetingStore;

    struct FakeEngine {
        results: Mutex<VecDeque<Result<String, String>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeEngine {
        /// Engine that answers every call with the given text.
        fn always(text: &str) -> Self {
            Self::default().with_scripted(vec![Ok(text.to_string())])
        }

        fn with_scripted(self, script: Vec<Result<String, String>>) -> Self {
            *self.results.lock().expect("results") = script.into_iter().collect();
            self
        }

        /// Fails `times` times, then succeeds once; further calls fall back.
        fn failing(times: usize) -> Self {
            let script = (0..times)
                .map(|_| Err("engine boom".to_string()) as Result<String, String>)
                .chain(std::iter::once(Ok("recovered".to_string())))
                .collect();
            Self::default().with_scripted(script)
        }
    }

    impl Default for FakeEngine {
        fn default() -> Self {
            Self {
                results: Mutex::new(VecDeque::new()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl MeetingSttEngine for FakeEngine {
        fn transcribe(
            &self,
            _ctx: &JobContext,
            audio: &[f32],
        ) -> Result<Vec<EngineSegment>, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let text = match self.results.lock().expect("results").pop_front() {
                Some(Ok(text)) => Ok(text),
                Some(Err(err)) => Err(err),
                None => Ok("fallback".to_string()),
            };
            match text {
                Ok(text) => {
                    let text = text.trim();
                    if text.is_empty() {
                        return Ok(Vec::new());
                    }
                    Ok(vec![EngineSegment {
                        start_ms_rel: 0,
                        end_ms_rel: (audio.len() as i64) * 1000 / 16_000,
                        text: text.to_string(),
                        confidence: None,
                    }])
                }
                Err(err) => Err(EngineError::Transient(err)),
            }
        }

        fn model_label(&self) -> String {
            "fake-model".to_string()
        }
    }

    #[derive(Default)]
    struct RecordingDelegate {
        deltas: Mutex<Vec<(String, usize)>>,
        progress: Mutex<Vec<String>>,
    }

    impl QueueDelegate for RecordingDelegate {
        fn on_segments_persisted(
            &self,
            meeting_id: &str,
            segments: Vec<MeetingTranscriptSegmentDto>,
        ) {
            self.deltas
                .lock()
                .expect("deltas")
                .push((meeting_id.to_string(), segments.len()));
        }

        fn on_queue_progress(&self, meeting_id: &str) {
            self.progress
                .lock()
                .expect("progress")
                .push(meeting_id.to_string());
        }
    }

    fn temp_store() -> (tempfile::TempDir, Arc<TestStore>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TestStore::open(dir.path().join("meetings.db")).expect("open store");
        (dir, Arc::new(store))
    }

    fn session_draft(title: &str) -> NewMeetingSession {
        NewMeetingSession {
            title: title.to_string(),
            started_at: Utc::now(),
            stt_provider: "local-whisper".to_string(),
            provider_config_snapshot: serde_json::json!({}),
            meeting_model_id: None,
            language: None,
            microphone_device: None,
            output_device: None,
            has_microphone_audio: true,
            has_system_audio: false,
            recording_retention_policy: "keep".to_string(),
            summary_provider: None,
        }
    }

    fn ready_chunk(
        store: &TestStore,
        meeting_id: &Uuid,
        sequence: i64,
        key: &str,
        samples: i64,
    ) -> AudioChunk {
        let chunk = store
            .append_chunk(NewAudioChunk {
                meeting_id: meeting_id.clone(),
                sequence,
                source: "microphone".to_string(),
                timeline_start_ms: sequence * 60_000,
                timeline_end_ms: (sequence + 1) * 60_000,
                storage_key: key.to_string(),
                sample_rate: 16_000,
                sample_count: samples,
                device_id: None,
            })
            .expect("append chunk");
        store
            .finalize_chunk(
                chunk.envelope.id.clone(),
                ChunkFacts {
                    file_size: samples * 2 + 44,
                    sample_count: samples,
                    checksum: "ab".repeat(32),
                    gap_drift_metadata: None,
                },
            )
            .expect("finalize chunk")
    }

    fn write_wav(path: &Path, sample_count: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create audio dir");
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).expect("writer");
        for index in 0..sample_count {
            let value = ((index % 16000) as i16).wrapping_mul(2);
            writer.write_sample(value).expect("sample");
        }
        writer.finalize().expect("finalize wav");
    }

    fn queue_for(
        store: Arc<TestStore>,
        audio_root: PathBuf,
        engine: Arc<dyn MeetingSttEngine>,
        paused: Arc<std::sync::atomic::AtomicBool>,
        config: QueueConfig,
        delegate: Arc<RecordingDelegate>,
    ) -> Arc<MeetingTranscriptionQueue> {
        MeetingTranscriptionQueue::new(
            store,
            audio_root,
            engine,
            Arc::new(move || paused.load(Ordering::SeqCst)),
            None,
            delegate,
            config,
        )
    }

    fn test_config(max_attempts: u32, backoff_base_secs: i64) -> QueueConfig {
        QueueConfig {
            poll_interval: Duration::from_millis(20),
            max_attempts,
            backoff_base_secs,
            backoff_max_secs: 600,
        }
    }

    #[test]
    fn processes_due_job_and_persists_segments() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Weekly"))
            .expect("session");

        let key = "audio/000001-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            16_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 1, key, 16_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 0,
                timeline_end_ms: 60_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let engine = Arc::new(FakeEngine::always("Hallo Welt"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 30),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once(), "one due job should be processed");
        assert!(!queue.poll_once(), "nothing left to claim");

        let segments = store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Hallo Welt");
        assert_eq!(segments[0].sequence, 0);
        assert_eq!(segments[0].model_id.as_deref(), Some("fake-model"));

        let deltas = delegate.deltas.lock().expect("deltas");
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].0, session.envelope.id.0);
        assert_eq!(deltas[0].1, 1);
        assert!(!delegate.progress.lock().expect("progress").is_empty());

        // Succeeded jobs are no longer claimable.
        let due = store.due_stt_jobs(Utc::now(), 10).expect("due jobs");
        assert!(due.is_empty());
    }

    #[test]
    fn retries_transient_failures_then_recovers() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Retro"))
            .expect("session");

        let key = "audio/000002-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            8_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 2, key, 8_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 60_000,
                timeline_end_ms: 120_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let engine = Arc::new(FakeEngine::failing(1));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        // Zero backoff makes the rescheduled job immediately due again.
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine.clone(),
            paused,
            test_config(3, 0),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once(), "first attempt claims the job");
        // After the transient failure the job waits for its (zero-length) backoff.
        assert!(queue.poll_once(), "second attempt succeeds");

        let segments = store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "recovered");
        drop(queue);
        assert_eq!(engine.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn exhausts_attempts_and_fails_terminally() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Standup"))
            .expect("session");

        let key = "audio/000003-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            4_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 3, key, 4_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 120_000,
                timeline_end_ms: 180_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        struct AlwaysBroken;
        impl MeetingSttEngine for AlwaysBroken {
            fn transcribe(
                &self,
                _ctx: &JobContext,
                _audio: &[f32],
            ) -> Result<Vec<EngineSegment>, EngineError> {
                Err(EngineError::Transient("engine broken".to_string()))
            }
            fn model_label(&self) -> String {
                "broken".to_string()
            }
        }

        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            Arc::new(AlwaysBroken),
            paused,
            test_config(3, 0),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        assert!(queue.poll_once());
        assert!(queue.poll_once());
        // Terminal failure: no fourth claim despite zero backoff.
        assert!(!queue.poll_once());

        let due = store.due_stt_jobs(Utc::now(), 10).expect("due");
        assert!(due.is_empty(), "failed jobs are not claimable");
    }

    #[test]
    fn missing_chunk_audio_fails_immediately_without_retry() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("NoFile"))
            .expect("session");

        let chunk = ready_chunk(&store, &session.envelope.id, 4, "audio/ghost.wav", 1_000);
        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 180_000,
                timeline_end_ms: 240_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let engine = Arc::new(FakeEngine::always("never"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 0),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        assert!(!queue.poll_once(), "permanent failure must not be retried");
        assert!(store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments")
            .is_empty());
    }

    #[test]
    fn empty_transcription_succeeds_without_segments() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Silence"))
            .expect("session");

        let key = "audio/000005-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            2_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 5, key, 2_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 240_000,
                timeline_end_ms: 300_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let engine = Arc::new(FakeEngine::always("   "));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 30),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        assert!(!queue.poll_once());
        assert!(store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments")
            .is_empty());
        assert!(delegate.deltas.lock().expect("deltas").is_empty());
        assert!(!delegate.progress.lock().expect("progress").is_empty());
    }

    #[test]
    fn pauses_between_jobs_while_dictation_is_active() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Coexist"))
            .expect("session");

        let key = "audio/000006-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            2_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 6, key, 2_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 300_000,
                timeline_end_ms: 360_000,
                provider: "local-whisper".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let engine = Arc::new(FakeEngine::always("später"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine.clone(),
            Arc::clone(&paused),
            test_config(3, 30),
            Arc::clone(&delegate),
        );

        assert!(!queue.poll_once(), "dictation pause blocks claiming");
        assert_eq!(engine.calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.due_stt_jobs(Utc::now(), 10).expect("due").len(), 1);

        paused.store(false, Ordering::SeqCst);
        assert!(queue.poll_once(), "resumes once dictation ends");
    }

    #[test]
    fn sequences_continue_across_jobs_in_one_session() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Ordering"))
            .expect("session");

        let first_key = "audio/000007-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(first_key),
            2_000,
        );
        let first = ready_chunk(&store, &session.envelope.id, 7, first_key, 2_000);
        let second_key = "audio/000008-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(second_key),
            2_000,
        );
        let second = ready_chunk(&store, &session.envelope.id, 8, second_key, 2_000);

        for (chunk, start) in [(first, 360_000i64), (second, 420_000)] {
            store
                .enqueue_stt_job(NewSttJob {
                    meeting_id: session.envelope.id.clone(),
                    chunk_ids: vec![chunk.envelope.id],
                    source: "microphone".to_string(),
                    timeline_start_ms: start,
                    timeline_end_ms: start + 60_000,
                    provider: "local-whisper".to_string(),
                    config_snapshot: serde_json::json!({}),
                    next_attempt_at: None,
                })
                .expect("enqueue");
        }

        let engine = Arc::new(FakeEngine::always("text"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 30),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        assert!(queue.poll_once());

        let segments = store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments");
        let sequences: Vec<i64> = segments.iter().map(|s| s.sequence).collect();
        assert_eq!(sequences, vec![0, 1]);
    }

    #[test]
    fn merge_short_gaps_only_bridges_interior_silence() {
        let mut flags = vec![
            true, false, false, true, // interior gap of 2 — merged
            false, false, false, // leading/trailing untouched below
        ];
        merge_short_gaps(&mut flags, 2);
        assert_eq!(flags[..4].to_vec(), vec![true, true, true, true]);
        assert!(!flags[4]);

        let mut trailing = vec![true, false, false];
        merge_short_gaps(&mut trailing, 5);
        assert_eq!(trailing[1..].to_vec(), vec![false, false]);
    }

    #[test]
    fn redact_truncates_and_strips_control_characters() {
        let long = "a".repeat(500);
        let redacted = redact(&long);
        assert_eq!(redacted.chars().count(), 301);

        let control = "boom\u{0007}\nsecret";
        assert!(!redact(control).contains('\u{7}'));
    }

    #[test]
    fn chunk_paths_resolve_under_the_session_directory() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let engine = Arc::new(FakeEngine::always("x"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            store,
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 30),
            delegate,
        );
        let path = queue.chunk_path("sess-1", "audio/000001-microphone.wav");
        assert_eq!(
            path,
            audio_dir
                .path()
                .join("sess-1")
                .join("audio/000001-microphone.wav")
        );
    }

    #[test]
    fn decode_rejects_wrong_channel_count() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).expect("writer");
        writer.write_sample(0).expect("left");
        writer.write_sample(0).expect("right");
        writer.finalize().expect("finalize");

        assert!(decode_wav_pcm16_mono(&path).is_err());
    }

    /// A misbehaving engine that emits blank spans must not put unrenderable
    /// rows into the transcript.
    #[test]
    fn blank_engine_segments_are_not_persisted() {
        struct Blankish;
        impl MeetingSttEngine for Blankish {
            fn transcribe(
                &self,
                _ctx: &JobContext,
                _audio: &[f32],
            ) -> Result<Vec<EngineSegment>, EngineError> {
                let span = |text: &str| EngineSegment {
                    start_ms_rel: 0,
                    end_ms_rel: 100,
                    text: text.to_string(),
                    confidence: None,
                };
                Ok(vec![span(""), span("   \n\t "), span("real")])
            }
            fn model_label(&self) -> String {
                "blankish".to_string()
            }
        }

        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("Blank"))
            .expect("session");
        let key = "audio/000010-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            1_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 10, key, 1_000);
        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 0,
                timeline_end_ms: 1_000,
                provider: "Local".to_string(),
                config_snapshot: serde_json::json!({}),
                next_attempt_at: None,
            })
            .expect("enqueue");

        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            Arc::new(Blankish),
            paused,
            test_config(3, 0),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        let segments = store
            .segments_for_meeting(session.envelope.id.clone())
            .expect("segments");
        assert_eq!(segments.len(), 1, "only the non-blank span is persisted");
        assert_eq!(segments[0].text, "real");
        assert_eq!(segments[0].sequence, 0);
    }

    #[test]
    fn permanent_engine_errors_fail_immediately_without_retry() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let session = store
            .create_session(session_draft("BadKey"))
            .expect("session");

        let key = "audio/000009-microphone.wav";
        write_wav(
            &audio_dir
                .path()
                .join(session.envelope.id.0.as_str())
                .join(key),
            2_000,
        );
        let chunk = ready_chunk(&store, &session.envelope.id, 9, key, 2_000);

        store
            .enqueue_stt_job(NewSttJob {
                meeting_id: session.envelope.id.clone(),
                chunk_ids: vec![chunk.envelope.id.clone()],
                source: "microphone".to_string(),
                timeline_start_ms: 480_000,
                timeline_end_ms: 540_000,
                provider: "Azure Speech".to_string(),
                config_snapshot: serde_json::json!({ "kind": "cloud" }),
                next_attempt_at: None,
            })
            .expect("enqueue");

        struct InvalidApiKey;
        impl MeetingSttEngine for InvalidApiKey {
            fn transcribe(
                &self,
                _ctx: &JobContext,
                _audio: &[f32],
            ) -> Result<Vec<EngineSegment>, EngineError> {
                Err(EngineError::Permanent("invalid api key".to_string()))
            }
            fn model_label(&self) -> String {
                "azure-fast-transcription".to_string()
            }
        }

        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        // Zero backoff would make a transient failure instantly claimable —
        // the assertion below proves the failure was treated as permanent.
        let queue = queue_for(
            Arc::clone(&store),
            audio_dir.path().to_path_buf(),
            Arc::new(InvalidApiKey),
            paused,
            test_config(3, 0),
            Arc::clone(&delegate),
        );

        assert!(queue.poll_once());
        assert!(!queue.poll_once(), "permanent errors must not be retried");
    }

    #[test]
    fn routing_dispatches_by_provider_name_with_local_fallback() {
        struct Named(&'static str);
        impl MeetingSttEngine for Named {
            fn transcribe(
                &self,
                _ctx: &JobContext,
                audio: &[f32],
            ) -> Result<Vec<EngineSegment>, EngineError> {
                Ok(vec![EngineSegment {
                    start_ms_rel: 0,
                    end_ms_rel: (audio.len() as i64) * 1000 / 16_000,
                    text: self.0.to_string(),
                    confidence: None,
                }])
            }
            fn model_label(&self) -> String {
                self.0.to_string()
            }
        }

        let fallback = Arc::new(Named("local"));
        let routing = Arc::new(RoutingMeetingSttEngine::new(fallback));
        routing.register("Azure Speech", Arc::new(Named("azure")));

        let audio = vec![0.0f32; 16_000];
        let azure_ctx = JobContext {
            provider_name: "Azure Speech".to_string(),
            language: None,
        };
        let local_ctx = JobContext {
            provider_name: "Local".to_string(),
            language: None,
        };
        let unknown_ctx = JobContext {
            provider_name: "Not Registered".to_string(),
            language: None,
        };

        assert_eq!(
            routing.transcribe(&azure_ctx, &audio).expect("ok")[0].text,
            "azure"
        );
        assert_eq!(
            routing.transcribe(&local_ctx, &audio).expect("ok")[0].text,
            "local",
            "unregistered names fall back to the local engine"
        );
        assert_eq!(
            routing.transcribe(&unknown_ctx, &audio).expect("ok")[0].text,
            "local"
        );
    }

    #[test]
    fn wake_and_stop_are_safe_before_spawn() {
        let (_dir, store) = temp_store();
        let audio_dir = tempfile::tempdir().expect("audio root");
        let engine = Arc::new(FakeEngine::always("x"));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delegate = Arc::new(RecordingDelegate::default());
        let queue = queue_for(
            store,
            audio_dir.path().to_path_buf(),
            engine,
            paused,
            test_config(3, 30),
            delegate,
        );
        queue.wake();
        queue.stop();
        assert!(queue.shutdown.load(Ordering::SeqCst));
    }
}
