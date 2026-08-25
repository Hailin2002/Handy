//! Live summary run against a real meeting transcript through the real
//! llm_client -> CloudSummarizer -> store upsert chain. Point MOCK_URL at any
//! OpenAI-compatible endpoint (a local mock keeps everything on-machine).

use speak_app_lib::managers::meeting_store::{
    MeetingRepository, NewMeetingSummary, SqliteMeetingStore, Uuid,
};
use speak_app_lib::managers::meeting_summary as summarize;

fn main() {
    let db = std::env::var("SPEAK_MEETINGS_DB").unwrap_or_else(|_| {
        std::env::var("APPDATA")
            .map(|p| format!("{p}\\com.haeni.speak\\meetings.db"))
            .unwrap()
    });
    let store = SqliteMeetingStore::open(std::path::Path::new(&db).to_path_buf())
        .expect("open store");

    // The completed meeting with the most segments (the rescued one).
    let page = store
        .list_sessions(speak_app_lib::managers::meeting_store::Page {
            offset: 0,
            limit: 50,
        })
        .expect("sessions");
    let target = page
        .items
        .into_iter()
        .filter(|s| s.state == "completed")
        .map(|s| s.envelope.id.0)
        .next()
        .expect("a completed meeting");
    println!("meeting: {target}");

    let segments = store
        .segments_for_meeting(Uuid(target.clone()))
        .expect("segments");
    assert!(!segments.is_empty(), "meeting has no transcript");
    let spans: Vec<summarize::SummarySpan> = segments
        .iter()
        .map(|s| summarize::SummarySpan {
            start_ms: s.start_ms,
            text: s.text.clone(),
        })
        .collect();
    println!("transcript spans: {}", spans.len());

    let base_url =
        std::env::var("MOCK_URL").unwrap_or_else(|_| "http://127.0.0.1:8123/v1".to_string());
    let provider = speak_app_lib::settings::PostProcessProvider {
        id: "custom".to_string(),
        label: "Local mock".to_string(),
        base_url,
        allow_base_url_edit: true,
        models_endpoint: None,
        supports_structured_output: false,
    };
    let llm = summarize::CloudSummarizer::new(provider, "mock-key".to_string(), "mock-model".to_string());
    let summarizer = summarize::MeetingSummarizer::new(llm);

    let fingerprint = summarize::input_fingerprint(&spans);
    let summary = summarizer.summarize(spans).expect("summary");
    println!("markdown: {}", summary.summary_markdown.replace('\n', " | "));
    println!("decisions: {:?}", summary.decisions);

    store
        .upsert_summary(NewMeetingSummary {
            meeting_id: Uuid(target.clone()),
            provider: "custom".to_string(),
            model: Some("mock-model".to_string()),
            prompt_version: format!("{}:{fingerprint}", summarize::PROMPT_VERSION),
            state: "completed".to_string(),
            summary_markdown: summary.summary_markdown,
            decisions: serde_json::json!(summary.decisions),
            action_items: serde_json::json!(summary.action_items),
            open_questions: serde_json::json!(summary.open_questions),
            last_error: None,
        })
        .expect("persist");
    println!("summary persisted for {target}");
}
