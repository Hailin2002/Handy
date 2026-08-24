//! Source ingestion commands (Phase 9): WhatsApp text import plus reads.

use crate::managers::meeting_store::Source;
use crate::managers::source_ingestion::SourceIngestionService;
use crate::managers::whatsapp_importer::DateOrder;
use serde::Serialize;
use specta::Type;
use std::sync::Arc;
use tauri::State;

/// Import outcome: the source plus whether this call created it.
#[derive(Serialize, Type)]
pub struct WhatsappImportResult {
    pub source: Source,
    pub created: bool,
    pub item_count: i64,
}

fn parse_date_order(order: Option<String>) -> Result<Option<DateOrder>, String> {
    match order.as_deref() {
        None | Some("") => Ok(None),
        Some("dayFirst") => Ok(Some(DateOrder::DayFirst)),
        Some("monthFirst") => Ok(Some(DateOrder::MonthFirst)),
        Some(other) => Err(format!("unknown dateOrder '{other}'")),
    }
}

fn ingestion_error(error: &crate::managers::meeting_store::StoreError) -> String {
    let message = error.to_string();
    if message.contains("dateOrderAmbiguous") {
        "dateOrderAmbiguous".to_string()
    } else {
        message
    }
}

/// Imports a WhatsApp chat text. `date_order` is only needed when the preview
/// cannot disambiguate day/month; pass `dateOrderAmbiguous` handling in UI.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_import_text(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    title: String,
    content: String,
    date_order: Option<String>,
) -> Result<WhatsappImportResult, String> {
    let order = parse_date_order(date_order)?;
    let result = ingestion
        .ingest_whatsapp_text(&title, &content, order)
        .map_err(|e| ingestion_error(&e))?;
    Ok(WhatsappImportResult {
        item_count: result.items.len() as i64,
        source: result.source,
        created: result.created,
    })
}

/// Lists imported sources, newest first.
#[tauri::command]
#[specta::specta]
pub async fn source_list(
    ingestion: State<'_, Arc<SourceIngestionService>>,
) -> Result<Vec<Source>, String> {
    ingestion.store.list_sources().map_err(|e| e.to_string())
}

/// Normalised items of one source (chat rows, chronological).
#[tauri::command]
#[specta::specta]
pub async fn source_items(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    source_id: String,
) -> Result<Vec<crate::managers::meeting_store::SourceItem>, String> {
    ingestion
        .store
        .list_source_items(crate::managers::meeting_store::Uuid(source_id))
        .map_err(|e| e.to_string())
}
