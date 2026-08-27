//! Negative-path tests for the cloud/custom STT client paths
//! (`speech_pipeline::transcribe_cloud` with `SpeechInputProvider::Custom`)
//! against a local mock HTTP server.
//!
//! Fault classes covered:
//! 1. rejected API key (HTTP 401)
//! 2. server error (HTTP 500)
//! 3. connection failure / timeout (closed loopback port, stalled endpoint)
//! 4. artificially slowed responses (delayed but valid transcript)
//!
//! Gates asserted per test:
//! - G1: each fault produces its expected typed error and the defined retry
//!   outcome. The current client is single-shot: exactly one request must hit
//!   the wire — no retry/backoff storm on failure.
//! - G2: a capturing `log` implementation records every log line emitted
//!   during the call; no line may contain the bearer token observed on the
//!   wire or any representation of the raw audio payload.
//! - G3: the harness binds to 127.0.0.1 only and every test verifies that the
//!   mock's port is released again after shutdown.

use base64::Engine as _;
use speak_app_lib::settings::{get_default_settings, AppSettings, SpeechInputProvider};
use speak_app_lib::speech_pipeline::transcribe_cloud;
use std::future::pending;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Serialization: one fault scenario at a time keeps log capture and port
// release checks deterministic even under --test-threads=1 reordering.
// ---------------------------------------------------------------------------

/// Serializes tests that mutate process-global state (environment variables and
/// the shared log buffer). The returned guard is deliberately held for the whole
/// test body, `.await` points included — that is precisely what serializes them,
/// and it is why the call sites carry `#[allow(clippy::await_holding_lock)]`.
/// The lint is right in general (a std guard across an await can deadlock a
/// single-threaded executor); here the guard is the test harness, not a
/// resource contended by the code under test.
fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = match SERIAL.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Install the capturing logger and reset the buffer BEFORE the code under
    // test runs, so every line emitted during the fault scenario is recorded.
    log_buffer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    guard
}

// ---------------------------------------------------------------------------
// Log capture (G2)
// ---------------------------------------------------------------------------

static LOG_LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

struct CaptureLogger;

impl log::Log for CaptureLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if let Some(lines) = LOG_LINES.get() {
            lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(format!(
                    "[{}] {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                ));
        }
    }

    fn flush(&self) {}
}

/// Installs the capturing logger once and returns the shared line buffer.
fn log_buffer() -> &'static Mutex<Vec<String>> {
    LOG_LINES.get_or_init(|| {
        // A second logger may already be installed by another harness; tests
        // still work because this binary only ever installs our own.
        let _ = log::set_boxed_logger(Box::new(CaptureLogger));
        log::set_max_level(log::LevelFilter::Trace);
        Mutex::new(Vec::new())
    });
    LOG_LINES.get().expect("logger state initialized above")
}

/// Snapshot of all log lines recorded so far in this process.
fn captured_log_lines() -> Vec<String> {
    log_buffer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

// ---------------------------------------------------------------------------
// Mock STT server (G3: binds 127.0.0.1 only, shuts down cleanly)
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Behavior {
    /// Reply immediately with a fixed status line and body.
    Respond { status_line: String, body: String },
    /// Stall for `delay`, then reply successfully.
    RespondAfter {
        delay: Duration,
        status_line: String,
        body: String,
    },
    /// Accept the request but never answer (simulates a hung endpoint).
    Stall,
}

fn status_line(status: u16) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    format!("{status} {reason}")
}

impl Behavior {
    fn respond(status: u16, body: &str) -> Self {
        Behavior::Respond {
            status_line: status_line(status),
            body: body.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RecordedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn bearer_token(&self) -> Option<&str> {
        self.header("authorization")?
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|token| !token.is_empty())
    }
}

struct MockSttServer {
    base_url: String,
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    accept_task: tokio::task::JoinHandle<()>,
    connection_tasks: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<RecordedRequest> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed before headers completed",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body_end = (header_end + content_length).min(buffer.len());
    let mut parts = request_line.split_whitespace();
    Ok(RecordedRequest {
        method: parts.next().unwrap_or_default().to_string(),
        path: parts.next().unwrap_or_default().to_string(),
        headers,
        body: buffer[header_end..body_end].to_vec(),
    })
}

async fn send_response(
    stream: &mut TcpStream,
    status_line: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn handle_connection(
    mut stream: TcpStream,
    behavior: Behavior,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let Ok(request) = read_request(&mut stream).await else {
        return;
    };
    requests.lock().unwrap().push(request);

    match behavior {
        Behavior::Respond { status_line, body } => {
            let _ = send_response(&mut stream, &status_line, &body).await;
        }
        Behavior::RespondAfter {
            delay,
            status_line,
            body,
        } => {
            tokio::time::sleep(delay).await;
            let _ = send_response(&mut stream, &status_line, &body).await;
        }
        // Park forever holding the connection open; the client must be the
        // party that gives up (bounded by the caller in these tests).
        Behavior::Stall => {
            pending::<()>().await;
        }
    }
}

impl MockSttServer {
    async fn spawn(behavior: Behavior) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock STT server binds 127.0.0.1:0");
        let addr = listener.local_addr().expect("local address of mock");
        assert_eq!(
            addr.ip(),
            IpAddr::from([127, 0, 0, 1]),
            "harness must bind the loopback interface only"
        );

        let requests = Arc::new(Mutex::new(Vec::new()));
        let connection_tasks: Arc<Mutex<Vec<tokio::task::AbortHandle>>> =
            Arc::new(Mutex::new(Vec::new()));

        let accept_requests = requests.clone();
        let accept_connections = connection_tasks.clone();
        let behavior = Arc::new(behavior);
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer)) = listener.accept().await else {
                    break;
                };
                let task = tokio::spawn(handle_connection(
                    stream,
                    (*behavior).clone(),
                    accept_requests.clone(),
                ));
                accept_connections.lock().unwrap().push(task.abort_handle());
            }
        });

        MockSttServer {
            base_url: format!("http://{addr}"),
            addr,
            requests,
            accept_task,
            connection_tasks,
        }
    }

    /// G1 evidence: the client made exactly one attempt — no retries.
    fn assert_single_request(&self) -> RecordedRequest {
        let requests = self.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "expected exactly one request (single-shot client), got {}",
            requests.len()
        );
        requests[0].clone()
    }

    /// G3 evidence: aborts every task (which drops the bound listener) and
    /// proves the OS released the port by rebinding it.
    async fn shutdown_verifying_port_release(self) {
        let addr = self.addr;
        drop(self);
        for _ in 0..100 {
            if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
                drop(listener);
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("mock port {addr} was not released after shutdown");
    }
}

impl Drop for MockSttServer {
    fn drop(&mut self) {
        self.accept_task.abort();
        for handle in self.connection_tasks.lock().unwrap().drain(..) {
            handle.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn custom_settings(endpoint: &str) -> AppSettings {
    let mut settings = get_default_settings();
    settings.speech_input_provider = SpeechInputProvider::Custom;
    settings.custom_stt_endpoint = endpoint.to_string();
    settings
}

/// Deterministic audio whose encoded PCM16 payload embeds an ASCII marker in
/// every other sample's low byte, so the exact bytes that reach the wire can
/// be compared against the logs byte-for-byte.
///
/// The sample is offset by half a quantization step (`+ 0.5`) because a bare
/// `byte / 32767.0` does not survive the encoder's `(s * 32767.0) as i16`
/// truncation exactly in f32 arithmetic; the half-step keeps the truncation
/// safely inside the target bucket, making the round-trip exact.
fn marker_samples() -> Vec<f32> {
    const MARKER: &[u8] = b"STT_NEGATIVE_PATH_AUDIO_PAYLOAD_MARKER_01";
    MARKER
        .iter()
        .flat_map(|&byte| [(byte as f32 + 0.5) / 32767.0, 0.0])
        .collect()
}

/// Replicates the production encoder sample loop (`encode_pcm16_wav`) so the
/// expected wire bytes are computed from the same arithmetic.
fn pcm16_data_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&pcm.to_le_bytes());
    }
    out
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| byte as char).collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// G2 evidence: neither the API key observed on the wire nor any
/// representation of the raw audio payload appears in any captured log line.
/// Beyond the full body, the encoded audio marker region is checked in several
/// encodings so a *partial* payload leak (e.g. a truncated debug dump) is also
/// caught.
fn assert_no_key_or_audio_leak(request: &RecordedRequest, samples: &[f32]) {
    let audio_bytes = pcm16_data_bytes(samples);
    // Prefix of the encoded audio: enough to identify a partial dump, short
    // enough that any encoding of it is a realistic log fragment.
    let audio_prefix = &audio_bytes[..audio_bytes.len().min(24)];

    let mut secrets: Vec<(String, &'static str)> = vec![
        (
            latin1(&request.body),
            "raw audio/WAV payload (latin-1 view)",
        ),
        (
            base64::engine::general_purpose::STANDARD.encode(&request.body),
            "base64 of audio payload",
        ),
        (hex_encode(&request.body), "hex of audio payload"),
        (latin1(audio_prefix), "audio payload prefix (latin-1 view)"),
        (
            base64::engine::general_purpose::STANDARD.encode(audio_prefix),
            "base64 of audio payload prefix",
        ),
        (hex_encode(audio_prefix), "hex of audio payload prefix"),
        (
            hex_encode(audio_prefix).to_uppercase(),
            "uppercase hex of audio payload prefix",
        ),
    ];
    if let Some(token) = request.bearer_token() {
        secrets.push((token.to_string(), "API bearer token"));
        secrets.push((format!("Bearer {token}"), "Authorization header value"));
    } else {
        eprintln!(
            "note: no bearer token was present on the wire (no credential stored); \
             token leak assertion covered via header-value check when present"
        );
    }

    for line in captured_log_lines() {
        for (secret, kind) in &secrets {
            assert!(
                !line.contains(secret.as_str()),
                "{kind} leaked into logs; offending line: {line}"
            );
        }
    }
}

/// Confirms the request actually seen by the mock is the WAV POST the custom
/// STT client is supposed to produce, including the embedded audio marker.
fn assert_is_wav_post_with_payload(request: &RecordedRequest, samples: &[f32]) {
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.path, "/",
        "unexpected request path: {}",
        request.path
    );
    let content_type = request.header("content-type").unwrap_or_default();
    assert!(
        content_type.starts_with("audio/wav"),
        "unexpected content-type: {content_type}"
    );
    let marker = pcm16_data_bytes(samples);
    assert!(
        find_subsequence(&request.body, &marker).is_some(),
        "server did not receive the encoded audio payload"
    );
}

const TRANSCRIPT_BODY: &str = r#"{"text":"slow hello from the mock"}"#;

/// Convenience wrapper so each fault-class test reads as
/// `spawn_mock(behavior).await`.
async fn spawn_mock(behavior: Behavior) -> MockSttServer {
    MockSttServer::spawn(behavior).await
}

// ---------------------------------------------------------------------------
// Fault class 1: rejected API key (401)
// ---------------------------------------------------------------------------

#[allow(clippy::await_holding_lock)] // serial_guard is held across .await on purpose: see its definition
#[tokio::test]
async fn rejected_api_key_yields_typed_error_without_retry() {
    let _serial = serial_guard();
    let server = spawn_mock(Behavior::respond(
        401,
        r#"{"error":{"message":"Invalid API key provided"}}"#,
    ))
    .await;

    let result = transcribe_cloud(&marker_samples(), &custom_settings(&server.base_url)).await;
    let error = result.expect_err("a rejected API key must fail");
    let message = error.to_string();
    assert!(
        message.contains("Custom STT returned 401 Unauthorized"),
        "unexpected typed error: {message}"
    );
    assert!(
        message.contains("Invalid API key"),
        "provider rejection detail missing from typed error: {message}"
    );

    let request = server.assert_single_request();
    assert_is_wav_post_with_payload(&request, &marker_samples());
    assert_no_key_or_audio_leak(&request, &marker_samples());
    server.shutdown_verifying_port_release().await;
}

// ---------------------------------------------------------------------------
// Fault class 2: server error (500)
// ---------------------------------------------------------------------------

#[allow(clippy::await_holding_lock)] // serial_guard is held across .await on purpose: see its definition
#[tokio::test]
async fn http_500_yields_typed_error_without_retry() {
    let _serial = serial_guard();
    let server = spawn_mock(Behavior::respond(
        500,
        r#"{"error":{"message":"upstream transcription worker crashed"}}"#,
    ))
    .await;

    let result = transcribe_cloud(&marker_samples(), &custom_settings(&server.base_url)).await;
    let error = result.expect_err("an HTTP 500 must fail");
    let message = error.to_string();
    assert!(
        message.contains("Custom STT returned 500 Internal Server Error"),
        "unexpected typed error: {message}"
    );
    assert!(
        message.contains("upstream transcription worker crashed"),
        "provider error detail missing from typed error: {message}"
    );

    let request = server.assert_single_request();
    assert_is_wav_post_with_payload(&request, &marker_samples());
    assert_no_key_or_audio_leak(&request, &marker_samples());
    server.shutdown_verifying_port_release().await;
}

// ---------------------------------------------------------------------------
// Fault class 3a: connection refused (endpoint down)
// ---------------------------------------------------------------------------

#[allow(clippy::await_holding_lock)] // serial_guard is held across .await on purpose: see its definition
#[tokio::test]
async fn connection_refusal_yields_contact_failure_typed_error() {
    let _serial = serial_guard();
    // Obtain a guaranteed-closed loopback port by binding it and dropping it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind throwaway listener");
    let closed_addr = listener.local_addr().expect("throwaway address");
    drop(listener);

    let started = Instant::now();
    let result = transcribe_cloud(
        &marker_samples(),
        &custom_settings(&format!("http://{closed_addr}")),
    )
    .await;
    let error = result.expect_err("connecting to a closed port must fail");

    // Typed error: the anyhow context chain names the failing stage and keeps
    // the transport cause reachable for diagnostics.
    let chain = format!("{:#}", error);
    assert!(
        chain.contains("Failed to contact custom STT endpoint"),
        "contact-failure context missing: {chain}"
    );
    let lowered = chain.to_lowercase();
    assert!(
        lowered.contains("refused") || lowered.contains("os error") || lowered.contains("connect"),
        "transport cause missing from error chain: {chain}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "connection refusal should fail fast"
    );

    // No leak assertions possible without a wire capture here, but the log set
    // recorded during the failed attempt must not contain the endpoint secret:
    // nothing sensitive exists in this scenario; assert the endpoint URL never
    // gained credentials or query tokens in logs.
    for line in captured_log_lines() {
        assert!(
            !line.contains("api_key=") && !line.contains("Bearer "),
            "credentials pattern leaked into logs: {line}"
        );
    }
}

// ---------------------------------------------------------------------------
// Fault class 3b: stalled endpoint (hang) — defined outcome: single attempt,
// no retry/backoff loop; only a caller-side bound ends the wait because the
// built-in reqwest client configures no timeout of its own.
// ---------------------------------------------------------------------------

#[allow(clippy::await_holding_lock)] // serial_guard is held across .await on purpose: see its definition
#[tokio::test]
async fn stalled_endpoint_performs_single_attempt_and_never_retries() {
    let _serial = serial_guard();
    let server = spawn_mock(Behavior::Stall).await;

    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        transcribe_cloud(&marker_samples(), &custom_settings(&server.base_url)),
    )
    .await;
    assert!(
        result.is_err(),
        "a stalled endpoint must not produce a transcript"
    );
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(2),
        "client gave up early ({waited:?}); expected the caller-side bound to fire"
    );

    // Exactly one attempt was put on the wire — no retry/backoff storm while
    // the endpoint hangs.
    let request = server.assert_single_request();
    assert_is_wav_post_with_payload(&request, &marker_samples());
    assert_no_key_or_audio_leak(&request, &marker_samples());
    server.shutdown_verifying_port_release().await;
}

// ---------------------------------------------------------------------------
// Fault class 4: artificially slowed response — slow-but-alive endpoints are
// not spuriously failed; the transcript still comes back intact.
// ---------------------------------------------------------------------------

#[allow(clippy::await_holding_lock)] // serial_guard is held across .await on purpose: see its definition
#[tokio::test]
async fn slowed_endpoint_still_returns_transcript_after_delay() {
    let _serial = serial_guard();
    let delay = Duration::from_millis(400);
    let server = spawn_mock(Behavior::RespondAfter {
        delay,
        status_line: status_line(200),
        body: TRANSCRIPT_BODY.to_string(),
    })
    .await;

    let started = Instant::now();
    let result = transcribe_cloud(&marker_samples(), &custom_settings(&server.base_url)).await;
    let elapsed = started.elapsed();
    let transcript = result.expect("slow-but-valid endpoint must succeed");
    assert_eq!(
        transcript, "slow hello from the mock",
        "transcript mangled on the slow path"
    );
    assert!(
        elapsed >= delay,
        "response arrived faster than the injected delay ({elapsed:?})"
    );

    let request = server.assert_single_request();
    assert_is_wav_post_with_payload(&request, &marker_samples());
    assert_no_key_or_audio_leak(&request, &marker_samples());
    server.shutdown_verifying_port_release().await;
}
