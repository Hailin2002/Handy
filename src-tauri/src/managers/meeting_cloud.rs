//! Cloud STT backends for meeting jobs (plan §13 Phase 6).
//!
//! [`AzureFastTranscriptionClient`] submits one WAV package per job to the
//! Azure Speech fast-transcription REST endpoint and maps the returned
//! phrases back onto relative segments. [`CustomEndpointClient`] speaks a
//! small tolerant dialect on the same abstraction.
//!
//! Error classification drives retry policy upstream (`EngineError`):
//! authentication/configuration problems are permanent and never retried;
//! timeouts, throttling (429), network failures and 5xx are transient.
//! Secrets are only ever sent as request headers — they never appear in
//! error strings or logs.

use serde_json::Value;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// One recognized phrase from a cloud backend, already converted to
/// milliseconds relative to the submitted audio.
#[derive(Debug, Clone)]
pub struct CloudPhrase {
    /// Input audio channel the phrase was recognized on (0-based).
    pub channel: u16,
    /// Phrase start, milliseconds from audio start.
    pub offset_ms: i64,
    /// Phrase length, milliseconds.
    pub duration_ms: i64,
    /// Display-form recognized text.
    pub text: String,
    /// Confidence when the backend reports one.
    pub confidence: Option<f32>,
}

/// Failure classification for cloud calls; mirrors `EngineError`.
#[derive(Debug, Clone)]
pub enum CloudJobError {
    /// Configuration/authentication problems — never retried.
    Permanent(String),
    /// Network/throttling/server problems — retried with backoff.
    Transient(String),
}

impl CloudJobError {
    pub fn message(&self) -> &str {
        match self {
            CloudJobError::Permanent(m) | CloudJobError::Transient(m) => m,
        }
    }
}

/// Transport seam so tests can script responses without a network.
pub trait CloudTranscriber: Send + Sync {
    /// Submits a PCM16 mono WAV package and returns recognized phrases.
    fn transcribe_wav(
        &self,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Vec<CloudPhrase>, CloudJobError>;
}

/// HTTP status classification (STT provider contract §3.3): permanent for
/// client errors that no retry can fix, transient for everything else.
fn classify_status(status: u16) -> bool {
    matches!(status, 400 | 401 | 402 | 403 | 404 | 405 | 413 | 415)
}

/// Shared single-thread runtime the queue worker threads block on for cloud
/// HTTP round-trips. One handle for all engines keeps connection pools warm
/// without spawning a runtime per job.
///
/// Deliberately uses async reqwest instead of the `blocking` feature: enabling
/// `reqwest/blocking` changes feature unification in a way that pulls
/// comctl32 v6 imports into test binaries, which then fail to load without an
/// application manifest (STATUS_ENTRYPOINT_NOT_FOUND).
static CLOUD_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn cloud_runtime() -> &'static tokio::runtime::Runtime {
    CLOUD_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("cloud STT tokio runtime")
    })
}

/// Builds the HTTP client used by both cloud backends.
fn http_client(timeout: Duration) -> Result<reqwest::Client, CloudJobError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| CloudJobError::Permanent(format!("http client unavailable: {e}")))
}

/// One field of a `multipart/form-data` request body.
///
/// Hand-rolled instead of using `reqwest`'s `multipart` feature: adding that
/// feature changes workspace dependency resolution enough to alter link order,
/// which makes Windows test binaries retain comctl32 v6 imports they cannot
/// resolve without an application manifest (STATUS_ENTRYPOINT_NOT_FOUND). The
/// encoding is plain RFC 2046 and trivial to keep in-tree.
pub struct MultipartField<'a> {
    pub name: &'a str,
    pub filename: Option<&'a str>,
    pub mime: Option<&'a str>,
    pub data: Vec<u8>,
}

impl<'a> MultipartField<'a> {
    pub fn text(name: &'a str, value: &str) -> MultipartField<'a> {
        MultipartField {
            name,
            filename: None,
            mime: None,
            data: value.as_bytes().to_vec(),
        }
    }

    pub fn file(name: &'a str, filename: &'a str, mime: &'a str, data: Vec<u8>) -> Self {
        MultipartField {
            name,
            filename: Some(filename),
            mime: Some(mime),
            data,
        }
    }
}

/// Serializes fields as a complete `multipart/form-data` body (including the
/// terminating boundary). Pair with a
/// `Content-Type: multipart/form-data; boundary={boundary}` header.
pub fn encode_multipart(fields: &[MultipartField<'_>], boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for field in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            match field.filename {
                Some(filename) => format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                    field.name, filename
                ),
                None => format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n",
                    field.name
                ),
            }
            .as_bytes(),
        );
        if let Some(mime) = field.mime {
            body.extend_from_slice(format!("Content-Type: {mime}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&field.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Per-request boundary base. Uniqueness only matters within one request.
fn multipart_boundary(attempt: usize) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "speak-stt-{nanos}-{}",
        attempt.wrapping_mul(7).wrapping_add(3)
    )
}

/// Collision-checked variant of [`encode_multipart`]: returns `None` when any
/// field payload would fake one of the delimiters under this boundary.
fn try_encode_multipart(fields: &[MultipartField<'_>], boundary: &str) -> Option<Vec<u8>> {
    let body = encode_multipart(fields, boundary);
    let delim = format!("--{boundary}").into_bytes();
    let expected = fields.len() + 1;
    let occurrences = body.windows(delim.len()).filter(|w| *w == delim).count();
    (occurrences == expected).then_some(body)
}

/// Builds a complete multipart request: `(content-type header value, body)`.
///
/// Adversarial or simply unlucky audio bytes could otherwise contain a byte
/// sequence that fakes a delimiter and corrupts the request server-side, so
/// the chosen boundary is verified against the fully encoded body and rotated
/// on collision.
pub fn multipart_request(
    fields: &[MultipartField<'_>],
) -> Result<(String, Vec<u8>), CloudJobError> {
    for attempt in 0..16 {
        let boundary = multipart_boundary(attempt);
        if let Some(body) = try_encode_multipart(fields, &boundary) {
            return Ok((format!("multipart/form-data; boundary={boundary}"), body));
        }
    }
    Err(CloudJobError::Permanent(
        "multipart boundary collision".to_string(),
    ))
}

/// Validates an endpoint once at construction: malformed URLs are permanent
/// configuration errors, not transient network failures worth retrying.
fn validated_endpoint(endpoint: &str) -> Result<String, CloudJobError> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err(CloudJobError::Permanent("endpoint is empty".to_string()));
    }
    // `azure_speech_endpoint` holds a *region* ("germanywestcentral") in every
    // other consumer — the dictation path expands it the same way. Demanding a
    // full URL here meant a perfectly configured Azure account still could not
    // be used for meetings: registration failed at startup with "relative URL
    // without a base", and the per-meeting Azure choice was unreachable.
    let trimmed = &expand_region(trimmed);
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| CloudJobError::Permanent(format!("invalid endpoint: {e}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CloudJobError::Permanent(format!(
                "endpoint scheme must be http(s), got '{other}:'"
            )))
        }
    }
    if url.host_str().is_none() || url.host_str() == Some("") {
        return Err(CloudJobError::Permanent("endpoint has no host".to_string()));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Turns a bare Azure region into its Fast Transcription base URL.
///
/// Anything that already looks like a URL is left untouched, so a custom or
/// sovereign-cloud endpoint still works. A region is accepted only when it
/// consists of the characters Azure actually uses, which keeps a typo from
/// being silently turned into a request to an attacker-chosen host.
fn expand_region(configured: &str) -> String {
    if configured.contains("://") {
        return configured.to_string();
    }
    let looks_like_region = !configured.is_empty()
        && configured
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if looks_like_region {
        return format!("https://{configured}.api.cognitive.microsoft.com");
    }
    // Not a region and not a URL: hand it to the parser, which produces the
    // precise error the user needs to see.
    configured.to_string()
}

/// Strips an endpoint down to its host so diagnostics never leak paths,
/// queries or credentials.
fn endpoint_host(endpoint: &str) -> String {
    endpoint
        .split("://")
        .nth(1)
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or("invalid-endpoint")
        .to_string()
}

/// Azure Speech **fast transcription** client (REST, synchronous).
pub struct AzureFastTranscriptionClient {
    endpoint: String,
    key: String,
    http: reqwest::Client,
}

impl AzureFastTranscriptionClient {
    const API_VERSION: &str = "2024-11-15";

    pub fn new(endpoint: String, key: String, timeout: Duration) -> Result<Self, CloudJobError> {
        Ok(Self {
            endpoint: validated_endpoint(&endpoint)?,
            key,
            http: http_client(timeout)?,
        })
    }

    fn url(&self) -> String {
        format!(
            "{}/speechtotext/transcriptions:transcribe?api-version={}",
            self.endpoint,
            Self::API_VERSION
        )
    }
}

impl CloudTranscriber for AzureFastTranscriptionClient {
    fn transcribe_wav(
        &self,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Vec<CloudPhrase>, CloudJobError> {
        let mut definition = serde_json::json!({ "channels": [0] });
        if let Some(language) = language {
            definition["language"] = Value::String(language.to_string());
        }

        let url = self.url();
        let key = self.key.clone();
        let endpoint_host = endpoint_host(&self.endpoint);
        cloud_runtime().block_on(async move {
            let (content_type, body) = multipart_request(&[
                MultipartField::text("definition", &definition.to_string()),
                MultipartField::file("audio", "audio.wav", "audio/wav", wav),
            ])?;

            let response = self
                .http
                .post(url)
                .header("Ocp-Apim-Subscription-Key", key)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .body(body)
                .send()
                .await
                // Transport-level failures (connect, DNS, timeout, TLS) are
                // always worth a retry.
                .map_err(|error| {
                    CloudJobError::Transient(format!(
                        "azure request failed ({}): {error}",
                        endpoint_host
                    ))
                })?;

            interpret_response(response, "azure", endpoint_host, parse_azure_response).await
        })
    }
}

/// Shared request→parse tail for both cloud clients: transport failures are
/// transient by definition, HTTP statuses are classified once, and the success
/// body is handed to the provider-specific parser. Keeping this in one place
/// guarantees both clients stay byte-identical in their error vocabulary.
async fn interpret_response<F>(
    response: reqwest::Response,
    label: &'static str,
    host: String,
    parse: F,
) -> Result<Vec<CloudPhrase>, CloudJobError>
where
    F: FnOnce(&str) -> Result<Vec<CloudPhrase>, String>,
{
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = first_error_line(&body);
        let kind = if classify_status(status.as_u16()) {
            CloudJobError::Permanent
        } else {
            CloudJobError::Transient
        };
        return Err(kind(format!(
            "{label} returned HTTP {} ({}): {}",
            status.as_u16(),
            host,
            detail
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|e| CloudJobError::Transient(format!("{label} response read failed: {e}")))?;
    parse(&body).map_err(CloudJobError::Transient)
}

/// Parses a fast-transcription JSON payload into phrases.
///
/// Tolerant by design: missing optional fields are skipped, unknown shapes
/// produce a descriptive error instead of panicking. Two API generations are
/// accepted — the current `phrases`/`offsetMilliseconds` shape (verified live
/// against `api-version=2024-11-15`, region germanywestcentral) and the older
/// `recognizedPhrases`/ticks shape — so a regional rollout difference cannot
/// silently empty a meeting transcript.
pub fn parse_azure_response(body: &str) -> Result<Vec<CloudPhrase>, String> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| format!("azure payload is not JSON: {e}"))?;

    // Current generation: phrases[] with millisecond offsets and plain text.
    if let Some(phrases) = root.get("phrases").and_then(Value::as_array) {
        let mut out = Vec::with_capacity(phrases.len());
        for phrase in phrases {
            let text = phrase
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            out.push(CloudPhrase {
                channel: phrase
                    .get("channel")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .clamp(0, u16::MAX.into()) as u16,
                offset_ms: phrase
                    .get("offsetMilliseconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                duration_ms: phrase
                    .get("durationMilliseconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                text,
                confidence: None,
            });
        }
        if !out.is_empty() {
            out.sort_by_key(|phrase| (phrase.channel, phrase.offset_ms));
            return Ok(out);
        }
        // Empty phrases fall through to the legacy shape / combined fallback.
    }

    // Legacy generation: recognizedPhrases[] with 100 ns ticks and nBest.
    if let Some(phrases) = root.get("recognizedPhrases").and_then(Value::as_array) {
        let mut out = Vec::with_capacity(phrases.len());
        for phrase in phrases {
            let offset_ticks = phrase.get("offsetInTicks").and_then(Value::as_i64);
            let duration_ticks = phrase.get("durationInTicks").and_then(Value::as_i64);
            let best = phrase
                .get("nBest")
                .and_then(Value::as_array)
                .and_then(|candidates| candidates.first());
            let text = best
                .map(|candidate| {
                    candidate
                        .get("display")
                        .or_else(|| candidate.get("lexical"))
                        .or_else(|| candidate.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
            if text.is_empty() || offset_ticks.is_none() {
                continue;
            }
            out.push(CloudPhrase {
                // Clamp instead of casting: a hostile or buggy payload could carry
                // negative or oversized channel numbers that would silently wrap.
                channel: phrase
                    .get("channel")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .clamp(0, u16::MAX.into()) as u16,
                // Azure ticks are 100 ns; 10_000 ticks per millisecond.
                offset_ms: offset_ticks.unwrap_or(0) / 10_000,
                duration_ms: duration_ticks.unwrap_or(0) / 10_000,
                text,
                confidence: best
                    .and_then(|candidate| candidate.get("confidence"))
                    .and_then(Value::as_f64)
                    .map(|value| value as f32),
            });
        }
        if !out.is_empty() {
            out.sort_by_key(|phrase| (phrase.channel, phrase.offset_ms));
            return Ok(out);
        }
    }

    // Last resort: combinedPhrases[] as a single span covering the clip.
    if let Some(combined) = root.get("combinedPhrases").and_then(Value::as_array) {
        let text = combined
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if !text.is_empty() {
            return Ok(vec![CloudPhrase {
                channel: 0,
                offset_ms: 0,
                duration_ms: root
                    .get("durationMilliseconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                text,
                confidence: None,
            }]);
        }
    }

    Err("azure payload has no recognized phrases".to_string())
}

/// Generic custom-endpoint client: POSTs the same multipart package with a
/// bearer token and accepts three response dialects (segments list, bare
/// text object, plain text).
pub struct CustomEndpointClient {
    endpoint: String,
    key: String,
    http: reqwest::Client,
}

impl CustomEndpointClient {
    pub fn new(endpoint: String, key: String, timeout: Duration) -> Result<Self, CloudJobError> {
        Ok(Self {
            endpoint: validated_endpoint(&endpoint)?,
            key,
            http: http_client(timeout)?,
        })
    }
}

impl CloudTranscriber for CustomEndpointClient {
    fn transcribe_wav(
        &self,
        wav: Vec<u8>,
        _language: Option<&str>,
    ) -> Result<Vec<CloudPhrase>, CloudJobError> {
        let endpoint = self.endpoint.clone();
        let key = self.key.clone();
        let endpoint_host = endpoint_host(&self.endpoint);
        cloud_runtime().block_on(async move {
            let (content_type, body) =
                multipart_request(&[MultipartField::file("audio", "audio.wav", "audio/wav", wav)])?;

            let response = self
                .http
                .post(endpoint)
                .bearer_auth(key)
                .header("Content-Type", content_type)
                .body(body)
                .send()
                .await
                .map_err(|error| {
                    CloudJobError::Transient(format!(
                        "custom endpoint failed ({}): {error}",
                        endpoint_host
                    ))
                })?;

            interpret_response(
                response,
                "custom endpoint",
                endpoint_host,
                parse_custom_response,
            )
            .await
        })
    }
}

/// Parses the custom dialects into phrases (single virtual channel).
pub fn parse_custom_response(body: &str) -> Result<Vec<CloudPhrase>, String> {
    let trimmed = body.trim();
    let root: Value = serde_json::from_str(trimmed).map_err(|_| {
        format!(
            "custom payload is neither JSON nor plain text: {}",
            preview(trimmed)
        )
    })?;

    if let Some(segments) = root.get("segments").and_then(Value::as_array) {
        let mut out = Vec::with_capacity(segments.len());
        for segment in segments {
            let text = segment
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            let start = segment.get("start_ms").and_then(Value::as_i64).unwrap_or(0);
            let end = segment
                .get("end_ms")
                .and_then(Value::as_i64)
                .unwrap_or(start);
            out.push(CloudPhrase {
                channel: 0,
                offset_ms: start,
                duration_ms: (end - start).max(0),
                text,
                confidence: None,
            });
        }
        return Ok(out);
    }

    if let Some(text) = root.get("text").and_then(Value::as_str) {
        return single_span_phrase(text);
    }
    Err(format!(
        "unsupported custom payload shape: {}",
        preview(trimmed)
    ))
}

fn single_span_phrase(text: &str) -> Result<Vec<CloudPhrase>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![CloudPhrase {
        channel: 0,
        offset_ms: 0,
        duration_ms: 0,
        text: text.to_string(),
        confidence: None,
    }])
}

fn preview(text: &str) -> String {
    text.chars().take(120).collect()
}

fn first_error_line(body: &str) -> String {
    let line = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    preview(line)
}

/// Encodes samples as PCM16 mono WAV bytes for cloud submission.
pub fn encode_wav_pcm16_mono(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("in-memory wav writer");
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            // Round on the 32768 scale so the queue's decoder
            // (`sample / 32768.0`) reproduces values within half an LSB.
            let scaled = (clamped * 32768.0).round().clamp(-32768.0, 32_767.0);
            writer.write_sample(scaled as i16).expect("in-memory write");
        }
        writer.finalize().expect("in-memory finalize");
    }
    cursor.into_inner()
}

// ---------------------------------------------------------------------------
// MeetingSttEngine adapters — the queue only ever sees these
// ---------------------------------------------------------------------------

use crate::managers::meeting_transcription::{
    EngineError, EngineSegment, JobContext, MeetingSttEngine,
};

/// Converts cloud phrases into queue segments, dropping empty text and
/// clamping spans to the submitted audio length.
fn phrases_to_segments(phrases: Vec<CloudPhrase>, audio_samples: usize) -> Vec<EngineSegment> {
    // Saturating conversion: sample counts cannot reach i64 range in practice,
    // but the clamp keeps the invariant explicit instead of relying on the
    // two's-complement wrap of an unchecked cast.
    let duration_ms_rel_cap = i64::try_from(audio_samples)
        .unwrap_or(i64::MAX)
        .saturating_mul(1000)
        / 16_000;
    phrases
        .into_iter()
        .filter(|phrase| !phrase.text.trim().is_empty())
        .map(|phrase| {
            let start = phrase.offset_ms.max(0).min(duration_ms_rel_cap);
            let end = (phrase.offset_ms + phrase.duration_ms)
                .max(start)
                .min(duration_ms_rel_cap);
            EngineSegment {
                start_ms_rel: start,
                end_ms_rel: end,
                text: phrase.text,
                confidence: phrase.confidence,
            }
        })
        .collect()
}

/// Azure Fast Transcription meeting engine.
pub struct AzureMeetingEngine {
    client: Arc<dyn CloudTranscriber>,
}

impl AzureMeetingEngine {
    pub fn new(client: Arc<dyn CloudTranscriber>) -> Self {
        Self { client }
    }
}

impl MeetingSttEngine for AzureMeetingEngine {
    fn transcribe(
        &self,
        ctx: &JobContext,
        audio: &[f32],
    ) -> Result<Vec<EngineSegment>, EngineError> {
        let wav = encode_wav_pcm16_mono(audio);
        let phrases = self
            .client
            .transcribe_wav(wav, ctx.language.as_deref())
            .map_err(EngineError::from)?;
        Ok(phrases_to_segments(phrases, audio.len()))
    }

    fn model_label(&self) -> String {
        "azure-fast-transcription".to_string()
    }
}

/// Custom-endpoint meeting engine.
pub struct CustomMeetingEngine {
    client: Arc<dyn CloudTranscriber>,
}

impl CustomMeetingEngine {
    pub fn new(client: Arc<dyn CloudTranscriber>) -> Self {
        Self { client }
    }
}

impl MeetingSttEngine for CustomMeetingEngine {
    fn transcribe(
        &self,
        ctx: &JobContext,
        audio: &[f32],
    ) -> Result<Vec<EngineSegment>, EngineError> {
        let wav = encode_wav_pcm16_mono(audio);
        let phrases = self
            .client
            .transcribe_wav(wav, ctx.language.as_deref())
            .map_err(EngineError::from)?;
        Ok(phrases_to_segments(phrases, audio.len()))
    }

    fn model_label(&self) -> String {
        "custom-endpoint".to_string()
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    /// Regression: the shared setting holds a region, so meetings must accept
    /// one. Requiring a URL left a configured Azure account unusable.
    #[test]
    fn a_bare_region_becomes_the_fast_transcription_endpoint() {
        assert_eq!(
            validated_endpoint("germanywestcentral").unwrap(),
            "https://germanywestcentral.api.cognitive.microsoft.com"
        );
    }

    #[test]
    fn an_explicit_endpoint_is_left_alone() {
        assert_eq!(
            validated_endpoint("https://my-speech.cognitiveservices.azure.com/").unwrap(),
            "https://my-speech.cognitiveservices.azure.com"
        );
    }

    /// A value that is neither a region nor a URL must fail loudly rather than
    /// be coerced into a request to somewhere unexpected.
    #[test]
    fn nonsense_endpoints_are_still_rejected() {
        for value in ["   ", "not a region", "ftp://example.com", "https://"] {
            assert!(
                validated_endpoint(value).is_err(),
                "{value:?} must be rejected"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meeting_transcription::{JobContext, MeetingSttEngine};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// Scripted transcriber: answers each call with the queued outcome.
    struct ScriptedTranscriber {
        script: Mutex<VecDeque<Result<Vec<CloudPhrase>, CloudJobError>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl ScriptedTranscriber {
        fn once(outcome: Result<Vec<CloudPhrase>, CloudJobError>) -> Arc<dyn CloudTranscriber> {
            Arc::new(Self {
                script: Mutex::new(VecDeque::from([outcome])),
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }
    }

    impl CloudTranscriber for ScriptedTranscriber {
        fn transcribe_wav(
            &self,
            _wav: Vec<u8>,
            _language: Option<&str>,
        ) -> Result<Vec<CloudPhrase>, CloudJobError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.script.lock().expect("script").pop_front() {
                Some(outcome) => outcome,
                None => Err(CloudJobError::Transient("no scripted response".into())),
            }
        }
    }

    #[test]
    fn azure_engine_maps_phrases_to_relative_segments() {
        let transcriber = ScriptedTranscriber::once(Ok(vec![CloudPhrase {
            channel: 0,
            offset_ms: 250,
            duration_ms: 500,
            text: "Hallo Meeting.".to_string(),
            confidence: Some(0.9),
        }]));
        let engine = AzureMeetingEngine::new(transcriber);

        // One second of "audio".
        let audio = vec![0.0f32; 16_000];
        let ctx = JobContext {
            provider_name: "Azure Speech".to_string(),
            language: Some("de-DE".to_string()),
        };
        let segments = engine.transcribe(&ctx, &audio).expect("segments");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms_rel, 250);
        assert_eq!(segments[0].end_ms_rel, 750);
        assert!((segments[0].confidence.unwrap() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn transient_network_errors_are_retriable_but_permanent_ones_not() {
        let flaky = Arc::new(ScriptedTranscriber {
            script: Mutex::new(VecDeque::from([Err(CloudJobError::Transient(
                "connection reset".to_string(),
            ))])),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let engine = AzureMeetingEngine::new(Arc::clone(&flaky) as Arc<dyn CloudTranscriber>);
        let ctx = JobContext {
            provider_name: "Azure Speech".to_string(),
            language: None,
        };
        match engine.transcribe(&ctx, &[0.0; 16_000]) {
            Err(crate::managers::meeting_transcription::EngineError::Transient(_)) => {}
            other => panic!("expected transient error, got {other:?}"),
        }
        assert_eq!(
            flaky.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the queue owns retries — the engine must not loop"
        );

        let broken = ScriptedTranscriber::once(Err(CloudJobError::Permanent(
            "HTTP 401 (region.api.cognitive.microsoft.com): invalid key".to_string(),
        )));
        let engine = AzureMeetingEngine::new(broken);
        match engine.transcribe(&ctx, &[0.0; 16_000]) {
            Err(crate::managers::meeting_transcription::EngineError::Permanent(message)) => {
                assert!(!message.contains("super-secret-key-value"));
            }
            other => panic!("expected permanent error, got {other:?}"),
        }
    }

    #[test]
    fn custom_engine_uses_the_same_abstraction() {
        let transcriber = ScriptedTranscriber::once(Ok(vec![CloudPhrase {
            channel: 0,
            offset_ms: 0,
            duration_ms: 100,
            text: "Notiz".to_string(),
            confidence: None,
        }]));
        let engine = CustomMeetingEngine::new(transcriber);
        let ctx = JobContext {
            provider_name: "Custom endpoint".to_string(),
            language: None,
        };
        let segments = engine.transcribe(&ctx, &[0.0; 8_000]).expect("segments");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Notiz");
        assert_eq!(engine.model_label(), "custom-endpoint");
    }

    #[test]
    fn multipart_encoding_matches_rfc2046() {
        let body = encode_multipart(
            &[
                MultipartField::text("definition", "{\"channels\":[0]}"),
                MultipartField::file("audio", "audio.wav", "audio/wav", vec![1, 2, 3]),
            ],
            "BOUND",
        );
        let text = String::from_utf8(body).expect("ascii headers");
        assert!(text.contains("--BOUND\r\nContent-Disposition: form-data; name=\"definition\"\r\n\r\n{\"channels\":[0]}\r\n"));
        assert!(text.contains("Content-Disposition: form-data; name=\"audio\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"));
        assert!(text.ends_with("\r\n--BOUND--\r\n"));
        // Binary payload must survive verbatim.
        assert!(text.as_bytes().windows(3).any(|w| w == [1, 2, 3]));
    }

    // -- fail-testing: hostile inputs ---------------------------------------

    #[test]
    fn multipart_boundary_collision_is_detected_not_corrupted() {
        // Payload engineered to look like a multipart delimiter: a naive
        // encoder would produce a body a server cannot parse.
        let evil =
            b"--BOUND\r\nContent-Disposition: form-data; name=\"fake\"\r\n\r\nx\r\n".to_vec();
        let fields = [MultipartField::file("audio", "a.wav", "audio/wav", evil)];
        let body = encode_multipart(&fields, "BOUND");
        // Exactly one delimiter per field plus the terminator is allowed; the
        // payload above fakes an extra one.
        let needle = b"--BOUND";
        let occurrences = body.windows(needle.len()).filter(|w| *w == needle).count();
        assert!(
            occurrences >= 3,
            "precondition: adversarial payload injects delimiters ({occurrences})"
        );
        assert!(
            try_encode_multipart(&fields, "BOUND").is_none(),
            "colliding boundary must be rejected"
        );
    }

    #[test]
    fn multipart_request_survives_adversarial_payloads() {
        let evil = "--speak-stt-1\r\nContent-Disposition: form-data; name=\"f\""
            .as_bytes()
            .to_vec();
        let fields = [
            MultipartField::text("definition", "{\"channels\":[0]}"),
            MultipartField::file("audio", "a.wav", "audio/wav", evil),
        ];
        let (content_type, body) = multipart_request(&fields).expect("request built");
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .expect("content type");
        let delim = format!("--{boundary}").into_bytes();
        let occurrences = body.windows(delim.len()).filter(|w| *w == delim).count();
        assert_eq!(
            occurrences,
            fields.len() + 1,
            "exactly one delimiter per field plus terminator"
        );
    }

    #[test]
    fn nonfinite_samples_encode_without_panicking() {
        let samples = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25];
        let encoded = encode_wav_pcm16_mono(&samples);
        // Header (44 bytes standard PCM) + 4 samples * 2 bytes.
        assert_eq!(encoded.len(), 44 + 8);
        let pcm = &encoded[44..];
        let ints: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(ints[0], 0, "NaN maps to silence");
        assert_eq!(ints[1], 32_767, "+inf clamps");
        assert_eq!(ints[2], -32_768, "-inf clamps");
        assert!(ints[3] > 0 && ints[3] < 32_767);
    }

    #[test]
    fn azure_parser_handles_the_live_2024_11_15_response_shape() {
        // Verbatim shape from a real call against germanywestcentral
        // (api-version=2024-11-15): phrases with millisecond offsets plus a
        // combined summary. The legacy parser rejected this payload outright.
        let live = r##"{
          "durationMilliseconds": 8015,
          "combinedPhrases": [{"channel": 0, "text":
            "Guten Tag, wir beginnen jetzt die Besprechung. Der erste Punkt ist der Projektstatus."}],
          "phrases": [
            {"channel": 0, "offsetMilliseconds": 160, "durationMilliseconds": 2840,
             "text": "Guten Tag, wir beginnen jetzt die Besprechung."},
            {"channel": 0, "offsetMilliseconds": 3000, "durationMilliseconds": 2500,
             "text": "Der erste Punkt ist der Projektstatus."}
          ]
        }"##;
        let phrases = parse_azure_response(live).expect("live shape parses");
        assert_eq!(phrases.len(), 2);
        assert_eq!(phrases[0].offset_ms, 160);
        assert!(phrases[0].text.starts_with("Guten Tag"));

        // Combined-only fallback (e.g. a clip too short for phrase split).
        let combined_only = r#"{"durationMilliseconds": 900,
            "combinedPhrases": [{"channel": 0, "text": "Ja."}]}"#;
        let single = parse_azure_response(combined_only).expect("combined fallback");
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].text, "Ja.");
        assert_eq!(single[0].duration_ms, 900);
    }

    #[test]
    fn azure_parser_skips_hostile_field_types() {
        let body = serde_json::json!({
            "recognizedPhrases": [
                { "offsetInTicks": "not-a-number", "nBest": [{ "display": "skipped" }] },
                { "offsetInTicks": 5_000_000i64, "durationInTicks": -10i64,
                  "channel": 1.9, "nBest": "not-an-array" },
                { "offsetInTicks": 9_000_000i64, "durationInTicks": 2_000_000i64,
                  "nBest": [{ "display": 42 }] },
                { "offsetInTicks": 1_000_000i64, "durationInTicks": 1_000_000i64,
                  "nBest": [{ "display": "ok", "confidence": "high" }] }
            ]
        });
        let phrases = parse_azure_response(&body.to_string()).expect("parse");
        // Only the last entry has both a numeric offset and readable text.
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].text, "ok");
        assert_eq!(
            phrases[0].confidence, None,
            "non-numeric confidence dropped"
        );
    }

    #[test]
    fn custom_parser_tolerates_wrong_types_and_inverted_ranges() {
        let body = serde_json::json!({
            "segments": [
                { "start_ms": 1.5, "end_ms": 900, "text": "float start falls back to 0" },
                { "start_ms": 500, "end_ms": 100, "text": "inverted" },
                { "start_ms": 200, "end_ms": 300, "text": 12345 },
                { "start_ms": 0, "end_ms": 100, "text": "   " }
            ]
        });
        let phrases = parse_custom_response(&body.to_string()).expect("parse");
        assert_eq!(phrases.len(), 2, "non-string and blank text are skipped");
        assert_eq!(phrases[0].offset_ms, 0);
        assert_eq!(phrases[0].duration_ms, 900);
        assert_eq!(phrases[1].duration_ms, 0, "inverted range clamps to zero");
    }

    #[test]
    fn invalid_endpoints_are_rejected_at_construction() {
        let timeout = Duration::from_secs(1);
        for endpoint in ["not a url", "", "http://", "ftp://x/y"] {
            let custom = CustomEndpointClient::new(endpoint.to_string(), "k".into(), timeout);
            assert!(
                matches!(custom, Err(CloudJobError::Permanent(_))),
                "custom rejects invalid endpoint {endpoint:?}"
            );
            let azure =
                AzureFastTranscriptionClient::new(endpoint.to_string(), "k".into(), timeout);
            assert!(
                matches!(azure, Err(CloudJobError::Permanent(_))),
                "azure rejects invalid endpoint {endpoint:?}"
            );
        }
        AzureFastTranscriptionClient::new(
            "https://region.api.cognitive.microsoft.com".into(),
            "k".into(),
            timeout,
        )
        .expect("valid azure endpoint accepted");
        CustomEndpointClient::new("https://example.com/stt".into(), "k".into(), timeout)
            .expect("valid custom endpoint accepted");
    }

    #[test]
    fn classifies_config_errors_as_permanent_and_rest_as_transient() {
        for status in [400u16, 401, 402, 403, 404, 405, 413, 415] {
            assert!(classify_status(status), "{status} must be permanent");
        }
        for status in [408u16, 409, 429, 500, 502, 503, 504] {
            assert!(!classify_status(status), "{status} must be transient");
        }
    }

    #[test]
    fn parses_azure_phrases_across_channels_sorted_by_time() {
        let body = serde_json::json!({
            "duration": "PT10S",
            "recognizedPhrases": [
                {
                    "channel": 1,
                    "offset": "PT2S",
                    "duration": "PT1S",
                    "offsetInTicks": 20_000_000,
                    "durationInTicks": 10_000_000,
                    "nBest": [
                        { "confidence": 0.91, "lexical": "hallo welt", "display": "Hallo Welt." }
                    ]
                },
                {
                    "channel": 0,
                    "offset": "PT0.5S",
                    "duration": "PT1S",
                    "offsetInTicks": 5_000_000,
                    "durationInTicks": 10_000_000,
                    "nBest": [
                        { "confidence": 0.95, "lexical": "guten morgen", "display": "Guten Morgen." }
                    ]
                },
                {
                    "channel": 0,
                    "offsetInTicks": 99_000_000,
                    "durationInTicks": 0,
                    "nBest": [{ "display": "" }]
                }
            ]
        })
        .to_string();

        let phrases = parse_azure_response(&body).expect("parsed");
        assert_eq!(phrases.len(), 2, "empty phrases are dropped");
        assert_eq!(phrases[0].channel, 0);
        assert_eq!(phrases[0].offset_ms, 500);
        assert_eq!(phrases[0].duration_ms, 1_000);
        assert_eq!(phrases[0].text, "Guten Morgen.");
        assert!((phrases[0].confidence.unwrap() - 0.95).abs() < 1e-6);
        assert_eq!(phrases[1].channel, 1);
        assert_eq!(phrases[1].offset_ms, 2_000);
        assert_eq!(phrases[1].text, "Hallo Welt.");
    }

    #[test]
    fn azure_parse_rejects_unknown_shapes_with_description() {
        assert!(parse_azure_response("not json").is_err());
        assert!(parse_azure_response("{}").is_err());
    }

    #[test]
    fn parses_segment_dialect_from_custom_endpoint() {
        let body = serde_json::json!({
            "segments": [
                { "start_ms": 100, "end_ms": 900, "text": "Erster Satz" },
                { "start_ms": 1_000, "end_ms": 2_000, "text": "Zweiter Satz" }
            ]
        })
        .to_string();
        let phrases = parse_custom_response(&body).expect("parsed");
        assert_eq!(phrases.len(), 2);
        assert_eq!(phrases[0].offset_ms, 100);
        assert_eq!(phrases[0].duration_ms, 800);
        assert_eq!(phrases[1].text, "Zweiter Satz");
    }

    #[test]
    fn parses_text_object_and_skips_empty_text() {
        let phrases = parse_custom_response(r#"{"text": "  Nur Text  "}"#).expect("parsed");
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].text, "Nur Text");

        assert!(parse_custom_response(r#"{"text": "   "}"#)
            .expect("parsed")
            .is_empty());
    }

    #[test]
    fn custom_parse_reports_unsupported_shapes() {
        assert!(parse_custom_response("{\"other\": 1}").is_err());
        assert!(parse_custom_response("plain words").is_err());
    }

    #[test]
    fn wav_encode_roundtrips_through_the_queue_decoder() {
        let samples: Vec<f32> = (0..1_600)
            .map(|index| ((index % 100) as f32 / 100.0) - 0.5)
            .collect();
        let encoded = encode_wav_pcm16_mono(&samples);

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("package.wav");
        std::fs::write(&path, &encoded).expect("write");

        let decoded =
            super::super::meeting_transcription::decode_wav_pcm16_mono(&path).expect("decode");
        assert_eq!(decoded.len(), samples.len());
        for (original, restored) in samples.iter().zip(decoded.iter()) {
            assert!((original - restored).abs() < 1.0 / 32768.0 + 1e-6);
        }
    }

    #[test]
    fn endpoint_host_strips_scheme_path_and_query() {
        assert_eq!(
            endpoint_host("https://region.api.cognitive.microsoft.com/speechtotext/x?y=1"),
            "region.api.cognitive.microsoft.com"
        );
        assert_eq!(
            endpoint_host("no-scheme.example.com/path"),
            "no-scheme.example.com"
        );
    }
}
