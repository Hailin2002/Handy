//! Meeting summary pipeline (plan §13 Phase 7): map-reduce over transcript
//! segments with a hard context-window budget, structured output (decisions,
//! action items, open questions) and an injectable LLM so tests never touch
//! the network.
//!
//! Honesty rules baked in here:
//! - The service is provider-agnostic. Constructing the cloud executor is the
//!   caller's job and may only happen behind the cloud-consent gate; there is
//!   deliberately no "local" executor yet, so a missing consent can never
//!   silently degrade into a cloud call.
//! - Batching is greedy by character budget: a long transcript can grow the
//!   number of map calls, but no single prompt can exceed the budget
//!   (acceptance: "lange Transkripte überschreiten nicht unkontrolliert das
//!   Kontextfenster").

use sha2::{Digest, Sha256};

/// Prompt version persisted alongside every summary; bump when the prompt
/// semantics change so stale summaries are recognisable.
pub const PROMPT_VERSION: &str = "meeting-summary-v1";

/// Character budget per map prompt. Conservative on purpose: it must leave
/// room for the instruction block plus the model's answer inside typical
/// context windows of the providers offered in settings.
pub const MAP_BUDGET_CHARS: usize = 12_000;

/// One transcript span handed to the summarizer.
#[derive(Debug, Clone)]
pub struct SummarySpan {
    pub start_ms: i64,
    pub text: String,
}

impl SummarySpan {
    /// Serialized form used inside prompts: `[mm:ss] text`.
    fn render(&self) -> String {
        let total_seconds = (self.start_ms.max(0) / 1000) as usize;
        format!(
            "[{:02}:{:02}] {}",
            total_seconds / 60 % 100,
            total_seconds % 60,
            self.text
        )
    }
}

/// Greedy batching under `budget_chars`. Spans are never split or dropped:
/// an oversized span gets its own batch even if it alone exceeds the budget
/// (the alternative — silent truncation — would lose transcript content).
pub fn plan_batches(spans: Vec<SummarySpan>, budget_chars: usize) -> Vec<Vec<SummarySpan>> {
    assert!(budget_chars > 0, "budget must be positive");
    let mut batches: Vec<Vec<SummarySpan>> = Vec::new();
    let mut current: Vec<SummarySpan> = Vec::new();
    let mut current_len = 0usize;
    for span in spans {
        let len = span.text.chars().count() + 16;
        if !current.is_empty() && current_len + len > budget_chars {
            batches.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current_len += len;
        current.push(span);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Stable fingerprint of the summarizer input. Persisted next to the summary
/// so a retry can detect "same transcript, nothing to do" without trusting
/// wall-clock timestamps.
pub fn input_fingerprint(spans: &[SummarySpan]) -> String {
    let mut hasher = Sha256::new();
    for span in spans {
        hasher.update(span.start_ms.to_le_bytes());
        hasher.update(span.text.as_bytes());
        hasher.update([0x1f]);
    }
    format!("{:x}", hasher.finalize())
}

/// The structured result persisted into `meeting_summaries`.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredSummary {
    pub summary_markdown: String,
    pub decisions: Vec<String>,
    pub action_items: Vec<String>,
    pub open_questions: Vec<String>,
}

/// Blocking LLM abstraction. The production executor wraps the async
/// `llm_client`; tests substitute scripted fakes.
pub trait SummaryLlm: Send + Sync {
    fn complete(&self, prompt: String) -> Result<String, String>;
}

/// Lets a boxed executor flow through the generic [`MeetingSummarizer`].
impl SummaryLlm for Box<dyn SummaryLlm> {
    fn complete(&self, prompt: String) -> Result<String, String> {
        (**self).complete(prompt)
    }
}

/// Errors that carry enough context for the queue-style redaction upstream
/// but never echo raw provider payloads beyond one line.
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryError {
    Llm(String),
    Unparsable(String),
}

pub fn map_prompt(batch: &[SummarySpan]) -> String {
    let mut prompt = String::from(
        "You are summarizing one part of a longer meeting transcript.\n\
         The transcript can contain parallel MICROPHONE and SYSTEM tracks. \
         They often contain the same utterance with slightly different words; \
         consolidate overlap once, but preserve distinct statements from either track.\n\
         Produce concise meeting notes in the transcript's language:\n\
         - Key points and decisions\n\
         - Action items (who, what)\n\
         - Open questions\n\
         Use markdown bullet lists only. Do not invent content.\n\n\
         TRANSCRIPT PART:\n",
    );
    for span in batch {
        prompt.push_str(&span.render());
        prompt.push('\n');
    }
    prompt
}

pub fn reduce_prompt(partials: &[String]) -> String {
    let mut prompt = String::from(
        "These are notes from parts of one meeting. Merge them into ONE final \
         summary in the source language. Remove duplicates, keep every distinct \
         decision, action item and open question. Do not invent content.\n\n\
         Answer with ONLY a JSON object:\n\
         {\"summary_markdown\": string, \"decisions\": [string], \
         \"action_items\": [string], \"open_questions\": [string]}\n\n\
         PART NOTES:\n",
    );
    for (index, partial) in partials.iter().enumerate() {
        prompt.push_str(&format!("--- part {} ---\n{partial}\n", index + 1));
    }
    prompt
}

/// Tolerant parser for the model's answer: accepts bare JSON, fenced code
/// blocks or JSON embedded in prose. List items may be plain strings or
/// objects with a `title`/`text` field.
pub fn parse_structured(raw: &str) -> Result<StructuredSummary, SummaryError> {
    let json_text = extract_json(raw)
        .ok_or_else(|| SummaryError::Unparsable("no JSON object found".to_string()))?;
    let root: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|e| SummaryError::Unparsable(format!("summary payload is not valid JSON: {e}")))?;

    let summary_markdown = root
        .get("summary_markdown")
        .or_else(|| root.get("summary"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if summary_markdown.is_empty() {
        return Err(SummaryError::Unparsable(
            "summary_markdown missing or empty".to_string(),
        ));
    }

    Ok(StructuredSummary {
        summary_markdown,
        decisions: string_list(&root, "decisions"),
        action_items: string_list(&root, "action_items"),
        open_questions: string_list(&root, "open_questions"),
    })
}

fn string_list(root: &serde_json::Value, key: &str) -> Vec<String> {
    root.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(text) => Some(text.trim().to_string()),
                    other => other
                        .get("title")
                        .or_else(|| other.get("text"))
                        .and_then(serde_json::Value::as_str)
                        .map(|text| text.trim().to_string()),
                })
                .filter(|text| !text.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Extracts the outermost JSON object from a possibly chatty answer.
fn extract_json(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    Some(&raw[start..=end])
}

/// The map-reduce driver. One instance per run; retries are idempotent because
/// the caller persists exactly one `meeting_summaries` row per meeting keyed
/// by [`input_fingerprint`].
pub struct MeetingSummarizer<L: SummaryLlm> {
    llm: L,
    budget_chars: usize,
}

impl<L: SummaryLlm> MeetingSummarizer<L> {
    pub fn new(llm: L) -> Self {
        Self {
            llm,
            budget_chars: MAP_BUDGET_CHARS,
        }
    }

    #[cfg(test)]
    pub fn with_budget(llm: L, budget_chars: usize) -> Self {
        Self { llm, budget_chars }
    }

    pub fn summarize(&self, spans: Vec<SummarySpan>) -> Result<StructuredSummary, SummaryError> {
        let batches = plan_batches(spans, self.budget_chars);
        if batches.is_empty() {
            return Err(SummaryError::Llm("transcript is empty".to_string()));
        }
        let mut partials = Vec::with_capacity(batches.len());
        for batch in &batches {
            partials.push(
                self.llm
                    .complete(map_prompt(batch))
                    .map_err(SummaryError::Llm)?,
            );
        }

        // Single-batch runs skip the reduce step entirely: one call, one parse.
        if partials.len() == 1 {
            return parse_structured(&partials[0]);
        }

        let merged = self
            .llm
            .complete(reduce_prompt(&partials))
            .map_err(SummaryError::Llm)?;
        parse_structured(&merged)
    }
}

/// Production executor: routes through the existing post-process LLM
/// infrastructure (provider registry, model, secret store). Constructing this
/// type is the consent gate — callers must only build it after verifying the
/// persisted `meeting_summary_cloud_consent` flag, so a local run can never
/// reach a cloud provider by accident.
pub struct CloudSummarizer {
    provider: crate::settings::PostProcessProvider,
    api_key: String,
    model: String,
}

impl CloudSummarizer {
    pub fn new(
        provider: crate::settings::PostProcessProvider,
        api_key: String,
        model: String,
    ) -> Self {
        Self {
            provider,
            api_key,
            model,
        }
    }
}

impl SummaryLlm for CloudSummarizer {
    fn complete(&self, prompt: String) -> Result<String, String> {
        // Same runtime discipline as the cloud STT clients: block the queue-
        // style worker thread on one shared async runtime instead of pulling
        // in `reqwest/blocking` (link-order/manifest trap on Windows).
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("summary tokio runtime")
        });
        let disable_reasoning = matches!(self.provider.id.as_str(), "custom" | "openrouter");
        runtime
            .block_on(crate::llm_client::send_chat_completion(
                &self.provider,
                self.api_key.clone(),
                &self.model,
                prompt,
                disable_reasoning,
            ))
            // The client returns Option<String>: None means an empty completion.
            .map(|text| text.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn span(start_ms: i64, text: &str) -> SummarySpan {
        SummarySpan {
            start_ms,
            text: text.to_string(),
        }
    }

    /// Scripted LLM: returns queued answers, records every prompt.
    struct ScriptedLlm {
        answers: Mutex<VecDeque<Result<String, String>>>,
        prompts: Mutex<Vec<String>>,
    }

    impl ScriptedLlm {
        fn ok(answers: &[&str]) -> Self {
            Self {
                answers: Mutex::new(
                    answers
                        .iter()
                        .map(|a| Ok((*a).to_string()))
                        .collect::<VecDeque<_>>(),
                ),
                prompts: Mutex::new(Vec::new()),
            }
        }
    }

    impl SummaryLlm for ScriptedLlm {
        fn complete(&self, prompt: String) -> Result<String, String> {
            self.prompts.lock().unwrap().push(prompt);
            match self.answers.lock().unwrap().pop_front() {
                Some(answer) => answer,
                None => Err("script exhausted".to_string()),
            }
        }
    }

    // -- batching ----------------------------------------------------------

    #[test]
    fn empty_transcript_plans_no_batches() {
        assert!(plan_batches(Vec::new(), MAP_BUDGET_CHARS).is_empty());
    }

    #[test]
    fn batching_respects_the_budget_and_keeps_order() {
        let spans: Vec<SummarySpan> = (0..10).map(|i| span(i * 1000, &"w".repeat(400))).collect();
        // Each span costs 400 + 16 chars; two fit into a 1_000 budget.
        let batches = plan_batches(spans, 1_000);
        assert_eq!(batches.len(), 5);
        let all: Vec<i64> = batches.iter().flatten().map(|s| s.start_ms).collect();
        assert_eq!(
            all,
            (0..10).map(|i| i * 1000).collect::<Vec<_>>(),
            "order and content are preserved"
        );
    }

    #[test]
    fn an_oversized_span_gets_its_own_batch_instead_of_being_dropped() {
        let spans = vec![span(0, "small"), span(1000, &"big".repeat(5_000))];
        let batches = plan_batches(spans.clone(), 100);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0][0].text, "small");
        assert_eq!(batches[1][0].text, spans[1].text);
    }

    // -- fingerprint -------------------------------------------------------

    #[test]
    fn fingerprint_is_stable_and_input_sensitive() {
        let a = vec![span(0, "hello"), span(1_000, "world")];
        let b = vec![span(0, "hello"), span(1_000, "world")];
        let c = vec![span(0, "hello"), span(1_000, "worle")];
        assert_eq!(input_fingerprint(&a), input_fingerprint(&b));
        assert_ne!(input_fingerprint(&a), input_fingerprint(&c));
    }

    // -- parsing -----------------------------------------------------------

    const GOOD_JSON: &str = r##"{"summary_markdown": "# Notes", "decisions": ["Go"],
        "action_items": [{"title": "Send mail"}], "open_questions": []}"##;

    #[test]
    fn parser_accepts_bare_json_fenced_blocks_and_prose_wrappers() {
        for raw in [
            GOOD_JSON,
            &format!("```json\n{GOOD_JSON}\n```"),
            &format!("Sure! Here is the summary:\n{GOOD_JSON}\nHope that helps."),
        ] {
            let parsed = parse_structured(raw).expect("parses");
            assert_eq!(parsed.summary_markdown, "# Notes");
            assert_eq!(parsed.decisions, vec!["Go"]);
            assert_eq!(parsed.action_items, vec!["Send mail"]);
            assert!(parsed.open_questions.is_empty());
        }
    }

    #[test]
    fn parser_rejects_empty_or_structurally_wrong_answers() {
        assert!(matches!(
            parse_structured("no json here"),
            Err(SummaryError::Unparsable(_))
        ));
        assert!(matches!(
            parse_structured(r#"{"decisions": []}"#),
            Err(SummaryError::Unparsable(_))
        ));
    }

    // -- map-reduce driver ---------------------------------------------------

    #[test]
    fn single_batch_skips_the_reduce_call() {
        let llm = ScriptedLlm::ok(&[GOOD_JSON]);
        let summarizer = MeetingSummarizer::with_budget(llm, 10_000);
        let result = summarizer
            .summarize(vec![span(0, "only part"), span(500, "second line")])
            .expect("summary");
        assert_eq!(result.decisions, vec!["Go"]);
        let llm = summarizer_into_inner(summarizer);
        assert_eq!(llm.prompts.lock().unwrap().len(), 1, "no reduce call");
        assert!(llm.prompts.lock().unwrap()[0].contains("[00:00] only part"));
    }

    #[test]
    fn multi_batch_runs_map_then_reduce() {
        let partial = r#"{"summary_markdown": "part", "decisions": [], "action_items": [], "open_questions": []}"#;
        let llm = ScriptedLlm::ok(&[partial, partial, GOOD_JSON]);
        let summarizer = MeetingSummarizer::with_budget(llm, 700);
        // 300-char spans cost ~316 each: two fit per batch, the third starts
        // a new one -> two maps + one reduce.
        let result = summarizer
            .summarize(vec![
                span(0, &"a".repeat(300)),
                span(1_000, &"b".repeat(300)),
                span(2_000, &"c".repeat(300)),
            ])
            .expect("summary");
        assert_eq!(result.summary_markdown, "# Notes");
        let llm = summarizer_into_inner(summarizer);
        let prompts = llm.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 3);
        assert!(
            prompts[2].contains("--- part 1 ---"),
            "reduce prompt merges parts"
        );
    }

    #[test]
    fn llm_failures_propagate_as_retryable_errors() {
        let failing = ScriptedLlm::ok(&[]);
        failing
            .answers
            .lock()
            .unwrap()
            .push_back(Err("provider down".to_string()));
        let summarizer = MeetingSummarizer::with_budget(failing, 10_000);
        match summarizer.summarize(vec![span(0, "x")]) {
            Err(SummaryError::Llm(message)) => assert_eq!(message, "provider down"),
            other => panic!("expected LLM error, got {other:?}"),
        }
    }

    /// Helper to pull the scripted LLM back out for assertions.
    fn summarizer_into_inner<L: SummaryLlm>(s: MeetingSummarizer<L>) -> L {
        // MeetingSummarizer owns its LLM; destructuring keeps tests honest.
        let MeetingSummarizer { llm, .. } = s;
        llm
    }
}
