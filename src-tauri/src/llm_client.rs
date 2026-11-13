use crate::settings::PostProcessProvider;
use log::{debug, error, info};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::error::Error as StdError;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Prefix of every error returned because a request exceeded
/// [`ChatOptions::timeout`].
///
/// The string is part of this module's contract: callers may match on it with
/// `err.starts_with(llm_client::TIMEOUT_ERROR_PREFIX)` to tell "the model took
/// too long" apart from a transport or API failure and show a dedicated
/// message. Do not reword it without updating those callers.
pub const TIMEOUT_ERROR_PREFIX: &str = "timeout: ";

/// Per-request tuning for a chat completion.
///
/// `Default` reproduces the behaviour of the pre-existing entry points:
/// reasoning left untouched, no `max_tokens` in the payload, and no client-side
/// request timeout.
/// A completion plus the one piece of metadata callers must not ignore.
///
/// `truncated` is `finish_reason == "length"`: the model hit the token ceiling
/// mid-sentence rather than finishing. Without it a cut-off answer is
/// indistinguishable from a complete one, so the user copies half a sentence
/// and never learns why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatCompletion {
    pub content: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatOptions {
    /// Ask the endpoint to skip reasoning/thinking output. See
    /// [`send_chat_completion_with_schema`] for the retry behaviour this
    /// implies.
    pub disable_reasoning: bool,
    /// Upper bound on generated tokens. `None` omits the field entirely, so
    /// providers see a byte-for-byte unchanged payload.
    pub max_tokens: Option<u32>,
    /// Total per-request deadline applied to the underlying HTTP client. On
    /// expiry the call fails with an error prefixed by [`TIMEOUT_ERROR_PREFIX`].
    pub timeout: Option<Duration>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
}

/// Request fields used to ask an endpoint to skip reasoning/thinking.
/// Providers disagree on the field name and accepted values, so at most one of
/// these is set per request (see `reasoning_disable_params`).
#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Value>,
}

impl ReasoningParams {
    fn is_empty(&self) -> bool {
        self.reasoning_effort.is_none() && self.reasoning.is_none() && self.thinking.is_none()
    }
}

/// Pick the reasoning-disable request fields an endpoint understands.
/// Unknown endpoints get the common OpenAI-style field; if they reject it,
/// the request is retried without it (see `send_chat_completion_with_schema`).
fn reasoning_disable_params(provider: &PostProcessProvider) -> ReasoningParams {
    let base_url = provider.base_url.to_lowercase();
    if base_url.contains("api.deepseek.com") {
        // DeepSeek rejects reasoning_effort "none" and uses its own field:
        // https://api-docs.deepseek.com/guides/thinking_mode
        ReasoningParams {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            ..Default::default()
        }
    } else if provider.id == "openrouter" {
        // OpenRouter nested object; exclude:true also keeps reasoning text out
        // of the response so it can't pollute structured-output JSON parsing
        ReasoningParams {
            reasoning: Some(ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
            ..Default::default()
        }
    } else {
        ReasoningParams {
            reasoning_effort: Some("none".to_string()),
            ..Default::default()
        }
    }
}

/// Endpoints (base_url|model) that rejected the reasoning-disable fields with a
/// 4xx. Remembered for the lifetime of the process so every dictation after the
/// first skips the doomed attempt and goes straight to a plain request.
fn reasoning_rejections() -> &'static Mutex<HashSet<String>> {
    static REJECTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REJECTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn endpoint_key(provider: &PostProcessProvider, model: &str) -> String {
    format!("{}|{}", provider.base_url.trim_end_matches('/'), model)
}

fn is_known_rejected(key: &str) -> bool {
    reasoning_rejections()
        .lock()
        .map(|set| set.contains(key))
        .unwrap_or(false)
}

fn remember_rejection(key: String) {
    if let Ok(mut set) = reasoning_rejections().lock() {
        set.insert(key);
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(flatten)]
    reasoning: ReasoningParams,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
    /// Why generation stopped. `"length"` means the answer was cut off by the
    /// token ceiling rather than finished. Optional because not every
    /// OpenAI-compatible provider sends it.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/Hailin2002/Handy"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Speak/1.0 (+https://github.com/Hailin2002/Handy)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Speak"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers.
///
/// `timeout` bounds the whole request, connection setup and body read included.
/// `None` leaves the client without a deadline, which is what every caller did
/// before [`ChatOptions`] existed.
fn create_client(
    provider: &PostProcessProvider,
    api_key: &str,
    timeout: Option<Duration>,
) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    let mut builder = reqwest::Client::builder().default_headers(headers);
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder
        .build()
        .map_err(|e| report_reqwest_error("Failed to build HTTP client", &e))
}

/// Format a bounded error source chain.
///
/// `reqwest::Error`'s Display implementation intentionally gives only a short
/// summary. Nested causes contain the useful transport details, such as a
/// certificate validation failure, an HTTP/2 error, or a connection reset.
/// Callers must skip source types whose Display text can quote payload data.
fn error_source_chain(error: &(dyn StdError + 'static)) -> Vec<String> {
    let mut causes = Vec::new();
    let mut source = error.source();

    // Defensive cap in case a third-party error exposes a cyclic source chain.
    for _ in 0..16 {
        let Some(cause) = source else {
            break;
        };
        causes.push(cause.to_string());
        source = cause.source();
    }

    causes
}

fn reqwest_error_kinds(error: &reqwest::Error) -> String {
    let mut kinds = Vec::new();

    if error.is_builder() {
        kinds.push("builder");
    }
    if error.is_connect() {
        kinds.push("connect");
    }
    if error.is_request() {
        kinds.push("request");
    }
    if error.is_redirect() {
        kinds.push("redirect");
    }
    if error.is_timeout() {
        kinds.push("timeout");
    }
    if error.is_status() {
        kinds.push("status");
    }
    if error.is_body() {
        kinds.push("body");
    }
    if error.is_decode() {
        kinds.push("decode");
    }
    if error.is_upgrade() {
        kinds.push("upgrade");
    }

    if kinds.is_empty() {
        "unknown".to_string()
    } else {
        kinds.join(", ")
    }
}

fn sanitized_url(url: &reqwest::Url) -> String {
    let mut url = url.clone();

    // Custom endpoints should not contain credentials or query-string tokens,
    // but omit them from diagnostics in case one does.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);

    url.to_string()
}

fn sanitized_url_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| sanitized_url(&url))
        // Do not echo an invalid URL: the parse failure might have been caused
        // by sensitive data entered in the custom endpoint field.
        .unwrap_or_else(|_| "<invalid URL>".to_string())
}

fn report_reqwest_error(context: &str, error: &reqwest::Error) -> String {
    let kinds = reqwest_error_kinds(error);
    let url = error
        .url()
        .map(sanitized_url)
        .map(|url| format!(", url: {url}"))
        .unwrap_or_default();

    // serde_json's error text can quote values from a malformed response. That
    // response may contain transcription content, so retain the useful decode
    // classification but never put its nested source in logs or UI errors.
    let causes = if error.is_decode() {
        Vec::new()
    } else {
        error_source_chain(error)
    };
    let cause_details = if !causes.is_empty() {
        format!(": caused by: {}", causes.join(" -> "))
    } else if error.url().is_none() {
        // Reqwest's short Display text is safe when it cannot append a raw URL.
        format!(": {error}")
    } else {
        // The sanitized URL is already included above. Avoid formatting the
        // original error because its Display implementation includes the raw URL.
        String::new()
    };

    let details = format!("{context} (kind: {kinds}{url}){cause_details}");
    error!("{details}");
    details
}

/// Report a request failure, tagging deadline expiry with
/// [`TIMEOUT_ERROR_PREFIX`] so callers can special-case it.
fn report_request_error(context: &str, error: &reqwest::Error) -> String {
    let details = report_reqwest_error(context, error);
    if error.is_timeout() {
        format!("{TIMEOUT_ERROR_PREFIX}{details}")
    } else {
        details
    }
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    disable_reasoning: bool,
) -> Result<Option<String>, String> {
    // The legacy callers (post-processing, meeting summaries) have no place to
    // report truncation, so the signal is dropped here rather than forced on
    // them. Assistant answers go through `send_chat_completion_opts` directly.
    send_chat_completion_opts(
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        ChatOptions {
            disable_reasoning,
            ..Default::default()
        },
    )
    .await
    .map(|completion| completion.content)
}

/// Send a chat completion request with structured output support.
/// When json_schema is provided, uses structured outputs mode.
/// system_prompt is used as the system message when provided.
///
/// When disable_reasoning is set, the request carries the reasoning-disable
/// fields the endpoint is expected to understand. Not every OpenAI-compatible
/// endpoint accepts them (DeepSeek, Gemini's compat layer, and some OpenRouter
/// upstreams reject with 400), so a 400/422 answer to such a request triggers
/// one retry without the fields, and the rejection is remembered per
/// (base_url, model) so later requests skip the failing attempt entirely.
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    disable_reasoning: bool,
) -> Result<Option<String>, String> {
    // The legacy callers (post-processing, meeting summaries) have no place to
    // report truncation, so the signal is dropped here rather than forced on
    // them. Assistant answers go through `send_chat_completion_opts` directly.
    send_chat_completion_opts(
        provider,
        api_key,
        model,
        user_content,
        system_prompt,
        json_schema,
        ChatOptions {
            disable_reasoning,
            ..Default::default()
        },
    )
    .await
    .map(|completion| completion.content)
}

/// Send a chat completion request with structured output support and
/// per-request [`ChatOptions`].
///
/// This is the full implementation behind [`send_chat_completion`] and
/// [`send_chat_completion_with_schema`]; those two are thin wrappers passing
/// `ChatOptions::default()` apart from `disable_reasoning`, so their observable
/// behaviour is unchanged.
///
/// Beyond the wrappers:
/// - `opts.max_tokens` is forwarded as the OpenAI-style `max_tokens` field and
///   omitted from the payload entirely when `None`.
/// - `opts.timeout` bounds the request. On expiry the call fails with an `Err`
///   whose message starts with [`TIMEOUT_ERROR_PREFIX`]. The reasoning-rejection
///   retry gets a fresh deadline rather than sharing one budget.
pub async fn send_chat_completion_opts(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    opts: ChatOptions,
) -> Result<ChatCompletion, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    debug!(
        "Sending chat completion request to: {}",
        sanitized_url_for_log(&url)
    );

    let client = create_client(provider, &api_key, opts.timeout)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }

    // Add user message
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let key = endpoint_key(provider, model);
    let reasoning = if opts.disable_reasoning && !is_known_rejected(&key) {
        reasoning_disable_params(provider)
    } else {
        ReasoningParams::default()
    };

    let mut request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        stream: false,
        max_tokens: opts.max_tokens,
        response_format,
        reasoning,
    };

    let mut response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| report_request_error("HTTP request failed", &e))?;
    let mut status = response.status();
    debug!(
        "Chat completion response received with status {} over {:?} from {}",
        status,
        response.version(),
        sanitized_url(response.url())
    );

    // A 400/422 on a request carrying reasoning-disable fields is almost always
    // the endpoint rejecting those fields — retry once without them.
    if !status.is_success()
        && matches!(status.as_u16(), 400 | 422)
        && !request_body.reasoning.is_empty()
    {
        let error_text = response.text().await.unwrap_or_else(|e| {
            report_reqwest_error("Failed to read reasoning rejection response", &e)
        });
        info!(
            "Endpoint rejected request with reasoning disabled (status {}): {}. Retrying without reasoning fields",
            status, error_text
        );

        request_body.reasoning = ReasoningParams::default();
        response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| report_request_error("HTTP retry failed", &e))?;
        status = response.status();
        debug!(
            "Chat completion retry response received with status {} over {:?} from {}",
            status,
            response.version(),
            sanitized_url(response.url())
        );

        if status.is_success() {
            info!(
                "Retry without reasoning fields succeeded; '{}' (model '{}') will skip them from now on",
                sanitized_url_for_log(base_url), model
            );
            remember_rejection(key);
        }
    }

    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read API error response", &e));
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| report_request_error("Failed to parse API response", &e))?;

    let Some(choice) = completion.choices.first() else {
        return Ok(ChatCompletion::default());
    };

    Ok(ChatCompletion {
        content: choice.message.content.clone(),
        truncated: choice.finish_reason.as_deref() == Some("length"),
    })
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", sanitized_url_for_log(&url));

    let client = create_client(provider, &api_key, None)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| report_reqwest_error("Failed to fetch models", &e))?;

    let status = response.status();
    debug!(
        "Model list response received with status {} over {:?} from {}",
        status,
        response.version(),
        sanitized_url(response.url())
    );
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read model list error", &e));
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| report_reqwest_error("Failed to parse model list response", &e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Debug)]
    struct TestError {
        message: &'static str,
        source: Option<Box<TestError>>,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl StdError for TestError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.source
                .as_deref()
                .map(|source| source as &(dyn StdError + 'static))
        }
    }

    fn provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url: base_url.to_string(),
            allow_base_url_edit: true,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    fn request_json_with(reasoning: ReasoningParams, max_tokens: Option<u32>) -> Value {
        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            stream: false,
            max_tokens,
            response_format: None,
            reasoning,
        };
        serde_json::to_value(&request).unwrap()
    }

    fn request_json(reasoning: ReasoningParams) -> Value {
        request_json_with(reasoning, None)
    }

    /// A listener that accepts connections and never answers, so a client with a
    /// timeout has something to time out against.
    async fn serve_hanging_endpoint() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let mut accepted = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                // Hold the socket open without writing a response.
                accepted.push(stream);
            }
        });

        format!("http://{address}")
    }

    async fn serve_one_response(status: &str, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        format!("http://{address}")
    }

    #[test]
    fn error_source_chain_includes_all_nested_causes() {
        let error = TestError {
            message: "request failed",
            source: Some(Box::new(TestError {
                message: "TLS handshake failed",
                source: Some(Box::new(TestError {
                    message: "unknown certificate authority",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            error_source_chain(&error),
            vec!["TLS handshake failed", "unknown certificate authority"]
        );
    }

    #[test]
    fn log_url_sanitization_removes_credentials_and_tokens() {
        let url = "https://user:password@example.com/v1/models?api_key=secret#private";
        assert_eq!(sanitized_url_for_log(url), "https://example.com/v1/models");
    }

    #[test]
    fn invalid_log_urls_are_not_echoed() {
        assert_eq!(
            sanitized_url_for_log("not a URL containing secret"),
            "<invalid URL>"
        );
    }

    #[tokio::test]
    async fn decode_error_does_not_echo_response_values() {
        let base_url =
            serve_one_response("200 OK", r#"{"choices":"PRIVATE TRANSCRIPTION CONTENT"}"#).await;
        let error = reqwest::get(base_url)
            .await
            .unwrap()
            .json::<ChatCompletionResponse>()
            .await
            .unwrap_err();

        let details = report_reqwest_error("Failed to parse API response", &error);
        assert!(details.contains("kind: decode"));
        assert!(!details.contains("PRIVATE TRANSCRIPTION CONTENT"));
    }

    #[tokio::test]
    async fn raw_error_url_is_not_reintroduced_without_a_source() {
        let base_url = serve_one_response("400 Bad Request", "bad request").await;
        let error = reqwest::get(format!(
            "{base_url}/private?api_key=SECRET_QUERY_TOKEN#private"
        ))
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();

        let details = report_reqwest_error("Request failed", &error);
        assert!(details.contains(&format!("url: {base_url}/private")));
        assert!(!details.contains("SECRET_QUERY_TOKEN"));
        assert!(!details.contains("#private"));
    }

    #[test]
    fn requests_explicitly_disable_streaming() {
        let json = request_json(ReasoningParams::default());
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn default_reasoning_params_serialize_to_no_fields() {
        let json = request_json(ReasoningParams::default());
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn custom_provider_uses_top_level_reasoning_effort() {
        let params = reasoning_disable_params(&provider("custom", "http://localhost:11434/v1"));
        let json = request_json(params);
        assert_eq!(json["reasoning_effort"], "none");
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn openrouter_uses_nested_reasoning_object() {
        let params =
            reasoning_disable_params(&provider("openrouter", "https://openrouter.ai/api/v1"));
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert_eq!(json["reasoning"]["effort"], "none");
        assert_eq!(json["reasoning"]["exclude"], true);
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn deepseek_base_url_uses_thinking_disabled() {
        let params = reasoning_disable_params(&provider("custom", "https://api.deepseek.com"));
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert_eq!(json["thinking"]["type"], "disabled");
    }

    #[test]
    fn reasoning_params_is_empty_tracks_all_fields() {
        assert!(ReasoningParams::default().is_empty());
        assert!(!ReasoningParams {
            reasoning_effort: Some("none".to_string()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ReasoningParams {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn omitted_max_tokens_is_absent_from_the_request_payload() {
        let json = request_json_with(ReasoningParams::default(), None);
        assert!(
            json.get("max_tokens").is_none(),
            "max_tokens must not appear when unset: {json}"
        );
    }

    #[test]
    fn requested_max_tokens_is_sent_as_a_number() {
        let json = request_json_with(ReasoningParams::default(), Some(256));
        assert_eq!(json["max_tokens"], 256);
    }

    #[test]
    fn chat_options_default_is_fully_inert() {
        assert_eq!(
            ChatOptions::default(),
            ChatOptions {
                disable_reasoning: false,
                max_tokens: None,
                timeout: None,
            }
        );
    }

    #[tokio::test]
    async fn expired_timeout_is_reported_with_the_documented_marker() {
        let base_url = serve_hanging_endpoint().await;
        let error = send_chat_completion_opts(
            &provider("custom", &base_url),
            String::new(),
            "test-model",
            "hi".to_string(),
            None,
            None,
            ChatOptions {
                timeout: Some(Duration::from_millis(150)),
                ..Default::default()
            },
        )
        .await
        .expect_err("a hanging endpoint must not produce a completion");

        assert!(
            error.starts_with(TIMEOUT_ERROR_PREFIX),
            "expected the timeout marker, got: {error}"
        );
    }

    #[tokio::test]
    async fn a_request_within_the_timeout_is_not_marked_as_a_timeout() {
        let base_url =
            serve_one_response("200 OK", r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
        let content = send_chat_completion_opts(
            &provider("custom", &base_url),
            String::new(),
            "test-model",
            "hi".to_string(),
            None,
            None,
            ChatOptions {
                max_tokens: Some(16),
                timeout: Some(Duration::from_secs(10)),
                ..Default::default()
            },
        )
        .await
        .expect("the endpoint answered in time");

        assert_eq!(content.content.as_deref(), Some("ok"));
        assert!(!content.truncated, "no finish_reason means not truncated");
    }

    /// A cut-off answer must be distinguishable from a finished one, otherwise
    /// the user copies half a sentence and never learns why.
    #[tokio::test]
    async fn a_length_finish_reason_marks_the_completion_as_truncated() {
        let base_url = serve_one_response(
            "200 OK",
            r#"{"choices":[{"message":{"content":"half a sen"},"finish_reason":"length"}]}"#,
        )
        .await;
        let completion = send_chat_completion_opts(
            &provider("custom", &base_url),
            String::new(),
            "test-model",
            "hi".to_string(),
            None,
            None,
            ChatOptions::default(),
        )
        .await
        .expect("the endpoint answered");

        assert!(completion.truncated);
        assert_eq!(completion.content.as_deref(), Some("half a sen"));
    }

    /// The reasoning-token trap: the ceiling is spent on thinking and the
    /// content comes back empty.
    #[tokio::test]
    async fn a_truncated_empty_completion_is_reported_as_truncated() {
        let base_url = serve_one_response(
            "200 OK",
            r#"{"choices":[{"message":{"content":""},"finish_reason":"length"}]}"#,
        )
        .await;
        let completion = send_chat_completion_opts(
            &provider("custom", &base_url),
            String::new(),
            "test-model",
            "hi".to_string(),
            None,
            None,
            ChatOptions::default(),
        )
        .await
        .expect("the endpoint answered");

        assert!(completion.truncated);
        assert_eq!(completion.content.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_stop_finish_reason_is_not_truncated() {
        let base_url = serve_one_response(
            "200 OK",
            r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#,
        )
        .await;
        let completion = send_chat_completion_opts(
            &provider("custom", &base_url),
            String::new(),
            "test-model",
            "hi".to_string(),
            None,
            None,
            ChatOptions::default(),
        )
        .await
        .expect("the endpoint answered");

        assert!(!completion.truncated);
    }

    #[test]
    fn rejection_memo_is_keyed_by_base_url_and_model() {
        let deepseek = provider("custom", "https://api.deepseek.com/");
        let key = endpoint_key(&deepseek, "deepseek-chat");
        assert_eq!(key, "https://api.deepseek.com|deepseek-chat");
        assert!(!is_known_rejected(&key));
        remember_rejection(key.clone());
        assert!(is_known_rejected(&key));
        // A different model on the same endpoint is tracked separately
        assert!(!is_known_rejected(&endpoint_key(&deepseek, "other-model")));
    }

    // -----------------------------------------------------------------------
    // Live provider tests.
    //
    // `#[ignore]` on purpose, following the precedent in
    // `tests/loopback_external_audio.rs`: these leave the machine, need a real
    // credential and spend real tokens, so they must never run as part of
    // `cargo test` or the gate. Run them deliberately:
    //
    //     cargo test --lib llm_client::tests::live_ -- --ignored --nocapture
    //
    // They earn their keep because every other test in this file talks to a
    // loopback socket that answers exactly how we told it to. That proves the
    // request we build is the request we intended — not that a real provider
    // accepts it. `max_tokens` is the specific worry: it is a new field on the
    // wire, and an unrecognised field is exactly what a provider answers with
    // a 400.
    // -----------------------------------------------------------------------

    /// The shipped OpenRouter catalogue entry, not a hand-built stand-in.
    ///
    /// The point is to exercise the base URL, the `/models` endpoint and the
    /// auth header that users actually get, so a typo in the shipped defaults
    /// fails here rather than in someone's settings panel.
    fn live_openrouter() -> PostProcessProvider {
        crate::settings::get_default_settings()
            .post_process_provider(crate::settings::OPENROUTER_PROVIDER_ID)
            .expect("the shipped provider catalogue must contain OpenRouter")
            .clone()
    }

    /// The credential, or `None` so the test skips rather than fails.
    fn live_api_key() -> Option<String> {
        std::env::var("OPENROUTER_API_KEY")
            .ok()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
    }

    /// A cheap model the account can actually reach.
    ///
    /// Resolved from the live catalogue instead of hardcoded, because model ids
    /// are retired without notice and a test that fails because some id was
    /// sunset would say nothing at all about this code. `SPEAK_LIVE_MODEL`
    /// overrides it when a specific model is what you meant to test.
    async fn live_model(provider: &PostProcessProvider, api_key: &str) -> String {
        if let Some(model) = std::env::var("SPEAK_LIVE_MODEL")
            .ok()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
        {
            return model;
        }

        let models = fetch_models(provider, api_key.to_string())
            .await
            .expect("listing models is the first thing a wrong key breaks");

        for hint in ["gpt-4o-mini", "haiku", "-mini", "flash"] {
            if let Some(model) = models.iter().find(|model| model.contains(hint)) {
                return model.clone();
            }
        }

        models
            .first()
            .cloned()
            .expect("OpenRouter returned an empty model list")
    }

    #[tokio::test]
    #[ignore = "hits the real OpenRouter API and needs OPENROUTER_API_KEY; run deliberately"]
    async fn live_openrouter_lists_models() {
        let Some(api_key) = live_api_key() else {
            eprintln!("skipping: OPENROUTER_API_KEY is not set");
            return;
        };

        let models = fetch_models(&live_openrouter(), api_key)
            .await
            .expect("the shipped OpenRouter entry must be able to list models");

        assert!(!models.is_empty(), "OpenRouter returned no models");
        println!("OpenRouter offers {} models", models.len());
    }

    /// The whole Phase B addition, against a provider that can reject it.
    #[tokio::test]
    #[ignore = "hits the real OpenRouter API and spends tokens; run deliberately"]
    async fn live_openrouter_accepts_a_capped_assistant_request() {
        let Some(api_key) = live_api_key() else {
            eprintln!("skipping: OPENROUTER_API_KEY is not set");
            return;
        };

        let provider = live_openrouter();
        let model = live_model(&provider, &api_key).await;
        println!("asking {model}");

        // Shaped exactly like the Fast Helper profile: a system prompt, a small
        // token budget, reasoning disabled, and a deadline.
        let answer = send_chat_completion_opts(
            &provider,
            api_key,
            &model,
            "Reply with the single word OK and nothing else.".to_string(),
            Some("You answer in as few words as possible.".to_string()),
            None,
            ChatOptions {
                disable_reasoning: true,
                max_tokens: Some(64),
                timeout: Some(Duration::from_secs(60)),
            },
        )
        .await
        .expect("a capped request must be accepted, not rejected as malformed");

        assert!(
            !answer.truncated,
            "a one-word reply must not exhaust a 64-token budget"
        );
        let answer = answer
            .content
            .expect("the provider returned no message content");
        println!("answered: {answer:?}");
        assert!(
            !answer.trim().is_empty(),
            "a capped request must still produce an answer"
        );
    }

    /// The timeout marker the answer card matches on, against a real endpoint
    /// rather than a hanging loopback socket.
    #[tokio::test]
    #[ignore = "hits the real OpenRouter API; run deliberately"]
    async fn live_openrouter_timeout_is_marked() {
        let Some(api_key) = live_api_key() else {
            eprintln!("skipping: OPENROUTER_API_KEY is not set");
            return;
        };

        let error = send_chat_completion_opts(
            &live_openrouter(),
            api_key,
            "openai/gpt-4o-mini",
            "Write a long essay about the sea.".to_string(),
            None,
            None,
            ChatOptions {
                // Far below any achievable round trip, so this is deterministic
                // without depending on how slow the provider happens to be.
                timeout: Some(Duration::from_millis(1)),
                ..Default::default()
            },
        )
        .await
        .expect_err("a 1ms deadline cannot be met over the public internet");

        assert!(
            error.starts_with(TIMEOUT_ERROR_PREFIX),
            "the card matches on this prefix; got: {error}"
        );
    }
}
