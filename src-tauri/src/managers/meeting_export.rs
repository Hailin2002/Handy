//! Meeting export rendering (plan §11): Markdown, TXT and JSON exports built
//! deterministically from persisted rows. No i18n in the backend — labels are
//! fixed English tokens so exported files stay byte-stable across locales
//! (formatting-contract §4: export reproducibility beats UI locale).

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Markdown,
    Txt,
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Txt => "txt",
            ExportFormat::Json => "json",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "text/markdown",
            ExportFormat::Txt => "text/plain",
            ExportFormat::Json => "application/json",
        }
    }
}

/// One transcript row as needed for rendering.
pub struct ExportSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub source: String,
    pub text: String,
}

/// Summary projection (already structured by the summary pipeline).
pub struct ExportSummary {
    pub provider: String,
    pub model: Option<String>,
    pub prompt_version: String,
    pub summary_markdown: String,
    pub decisions: Vec<String>,
    pub action_items: Vec<String>,
    pub open_questions: Vec<String>,
}

/// The finished export: filename suggestion plus full content.
pub struct MeetingExport {
    pub filename: String,
    pub mime: String,
    pub content: String,
}

/// Timestamp as `[h:mm:ss]` along the session timeline.
fn marker(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as usize;
    format!(
        "[{}:{:02}:{:02}]",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

fn source_label(source: &str) -> &'static str {
    match source {
        "system" => "System",
        _ => "Microphone",
    }
}

/// ASCII-slug of the title for filenames; non-ASCII collapses to `-`.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "meeting".to_string()
    } else {
        trimmed
    }
}

pub fn render(
    format: ExportFormat,
    meeting_id: &str,
    title: &str,
    started_at_ms: i64,
    segments: &[ExportSegment],
    summary: Option<&ExportSummary>,
) -> MeetingExport {
    // UTC date-stamp per formatting-contract R2 (filename rule).
    let stamp = chrono::DateTime::from_timestamp_millis(started_at_ms)
        .map(|dt| dt.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| "unknown-date".to_string());
    let filename = format!(
        "{}-{}.{extension}",
        stamp,
        slug(title),
        extension = format.extension()
    );

    let content = match format {
        ExportFormat::Json => render_json(meeting_id, title, started_at_ms, segments, summary),
        ExportFormat::Markdown => render_markdown(title, segments, summary),
        ExportFormat::Txt => render_txt(segments),
    };

    MeetingExport {
        filename,
        mime: format.mime().to_string(),
        content,
    }
}

fn render_json(
    meeting_id: &str,
    title: &str,
    started_at_ms: i64,
    segments: &[ExportSegment],
    summary: Option<&ExportSummary>,
) -> String {
    let body = serde_json::json!({
        "export_version": 1,
        "meeting_id": meeting_id,
        "title": title,
        "started_at_utc_ms": started_at_ms,
        "summary": summary.map(|s| serde_json::json!({
            "provider": s.provider,
            "model": s.model,
            "prompt_version": s.prompt_version,
            "summary_markdown": s.summary_markdown,
            "decisions": s.decisions,
            "action_items": s.action_items,
            "open_questions": s.open_questions,
        })),
        "transcript": segments.iter().map(|s| serde_json::json!({
            "start_ms": s.start_ms,
            "end_ms": s.end_ms,
            "source": s.source,
            "text": s.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

fn render_markdown(
    title: &str,
    segments: &[ExportSegment],
    summary: Option<&ExportSummary>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    if let Some(s) = summary {
        out.push_str("## Summary\n\n");
        out.push_str(&s.summary_markdown);
        out.push_str("\n\n");
        append_list(&mut out, "Decisions", &s.decisions);
        append_list(&mut out, "Action items", &s.action_items);
        append_list(&mut out, "Open questions", &s.open_questions);
        let meta = format!("{}{}", s.provider, s.model.as_deref().unwrap_or(""));
        out.push_str(&format!(
            "\n_Generated by {meta} ({})_\n\n",
            s.prompt_version
        ));
    }
    out.push_str("## Transcript\n\n");
    for segment in segments {
        out.push_str(&format!(
            "- {} {} {}\n",
            marker(segment.start_ms),
            source_label(&segment.source),
            segment.text
        ));
    }
    out
}

fn append_list(out: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("### {heading}\n\n"));
    for item in items {
        out.push_str(&format!("- {item}\n"));
    }
    out.push('\n');
}

fn render_txt(segments: &[ExportSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push_str(&format!(
            "{} {}: {}\n",
            marker(segment.start_ms),
            source_label(&segment.source),
            segment.text
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: i64, text: &str) -> ExportSegment {
        ExportSegment {
            start_ms: start,
            end_ms: start + 900,
            source: "microphone".to_string(),
            text: text.to_string(),
        }
    }

    fn sample_summary() -> ExportSummary {
        ExportSummary {
            provider: "openrouter".to_string(),
            model: Some("test-model".to_string()),
            prompt_version: "meeting-summary-v1".to_string(),
            summary_markdown: "# Besprechung".to_string(),
            decisions: vec!["Montag".to_string()],
            action_items: vec![],
            open_questions: vec!["Budget?".to_string()],
        }
    }

    #[test]
    fn markdown_contains_summary_and_timed_transcript() {
        let export = render(
            ExportFormat::Markdown,
            "id-1",
            "Weekly Sync",
            0,
            &[seg(3_661_000, "Beschluss gefasst")],
            Some(&sample_summary()),
        );
        assert!(export.content.contains("# Weekly Sync"));
        assert!(export.content.contains("## Summary"));
        assert!(export
            .content
            .contains("- [1:01:01] Microphone Beschluss gefasst"));
        assert!(export.filename.ends_with(".md"), "{}", export.filename);
    }

    #[test]
    fn txt_is_only_the_timed_transcript() {
        let export = render(
            ExportFormat::Txt,
            "id-1",
            "Weekly",
            0,
            &[seg(61_000, "Hallo")],
            Some(&sample_summary()),
        );
        assert_eq!(export.content.trim(), "[0:01:01] Microphone: Hallo");
    }

    #[test]
    fn json_round_trips_through_serde() {
        let export = render(
            ExportFormat::Json,
            "id-1",
            "Weekly",
            0,
            &[seg(0, "Hi")],
            Some(&sample_summary()),
        );
        let value: serde_json::Value = serde_json::from_str(&export.content).expect("valid json");
        assert_eq!(value["title"], "Weekly");
        assert_eq!(value["transcript"][0]["text"], "Hi");
        assert_eq!(value["summary"]["decisions"][0], "Montag");
        assert!(export.filename.ends_with(".json"));
    }

    #[test]
    fn filename_slug_collapses_non_ascii_and_dates_are_utc() {
        let export = render(
            ExportFormat::Markdown,
            "id-1",
            "Überblick / Plan!",
            1_755_950_400_000, // 2025-08-23T12:00:00Z
            &[],
            None,
        );
        assert_eq!(export.filename, "20250823-120000-berblick-plan.md");
    }
}
