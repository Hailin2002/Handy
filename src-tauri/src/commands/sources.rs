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

/// A failed import, as a code the UI can act on plus English specifics.
///
/// # Why this is not a `String`
///
/// These commands already had error *codes* — `dateOrderAmbiguous`,
/// `noChatFileInArchive`, `archiveRejected: <detail>` — they were just smuggled
/// inside the message. Two things followed. The frontend piped `result.error`
/// straight into a toast, so a German user read the literal text
/// `archiveRejected: entry name contains ..` in a UI where ESLint otherwise
/// forbids untranslated strings. And Rust had to parse its own message back
/// (`if message.contains("dateOrderAmbiguous")`) to recover what it had just
/// formatted.
///
/// Splitting the two makes the code translatable and the detail diagnostic.
#[derive(Debug, Serialize, Type)]
pub struct ImportError {
    /// Stable and machine-readable. The frontend maps it to a translated
    /// message; unknown codes fall back to `detail`.
    pub code: String,
    /// English specifics for diagnostics — a rejected ZIP entry name, an IO
    /// error. Absent when the code says everything.
    pub detail: Option<String>,
}

impl ImportError {
    fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
            detail: None,
        }
    }

    /// Splits the legacy `"code: detail"` convention the ingestion service
    /// still produces. Once the service returns codes directly this becomes a
    /// plain constructor.
    fn from_service(message: String) -> Self {
        match message.split_once(": ") {
            Some((code, detail)) if !code.contains(' ') => Self {
                code: code.to_string(),
                detail: Some(detail.to_string()),
            },
            _ => Self {
                code: message,
                detail: None,
            },
        }
    }
}

fn parse_date_order(order: Option<String>) -> Result<Option<DateOrder>, ImportError> {
    match order.as_deref() {
        None | Some("") => Ok(None),
        Some("dayFirst") => Ok(Some(DateOrder::DayFirst)),
        Some("monthFirst") => Ok(Some(DateOrder::MonthFirst)),
        Some(other) => Err(ImportError {
            code: "unknownDateOrder".to_string(),
            detail: Some(other.to_string()),
        }),
    }
}

fn ingestion_error(error: &crate::managers::meeting_store::StoreError) -> ImportError {
    let message = error.to_string();
    if message.contains("dateOrderAmbiguous") {
        ImportError::new("dateOrderAmbiguous")
    } else {
        ImportError::from_service(message)
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
) -> Result<WhatsappImportResult, ImportError> {
    let order = parse_date_order(date_order)?;
    let result = ingestion
        .ingest_text(&title, &content, order)
        .map_err(|e| ingestion_error(&e))?;
    Ok(WhatsappImportResult {
        item_count: result.items.len() as i64,
        source: result.source,
        created: result.created,
    })
}

/// Phase 9 wizard preview: parse stats without persisting anything.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_preview_text(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    content: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ChatPreviewDto, ImportError> {
    let order = parse_date_order(date_order)?;
    ingestion
        .preview_text(&content, order)
        .map_err(ImportError::from_service)
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

/// Largest archive accepted from disk.
///
/// The importer's own limits guard the *contents*; this bounds the read itself
/// so a mistakenly chosen multi-gigabyte file cannot exhaust memory before any
/// validation runs.
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn read_archive(path: &str) -> Result<Vec<u8>, ImportError> {
    let unreadable = |e: std::io::Error| ImportError {
        code: "archiveUnreadable".to_string(),
        detail: Some(e.to_string()),
    };
    let meta = std::fs::metadata(path).map_err(unreadable)?;
    if meta.len() > MAX_ARCHIVE_BYTES {
        return Err(ImportError::new("archiveTooLarge"));
    }
    std::fs::read(path).map_err(unreadable)
}

/// Phase 9 wizard step 2 for `.zip`: inspect without extracting or persisting.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_preview_zip(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    path: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ZipPreviewDto, ImportError> {
    let order = parse_date_order(date_order)?;
    let bytes = read_archive(&path)?;
    ingestion
        .preview_zip(&bytes, order)
        .map_err(ImportError::from_service)
}

/// Phase 9 wizard step 4 for `.zip`: import the chat plus its media.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_import_zip(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    path: String,
    title: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ZipIngestResult, ImportError> {
    let order = parse_date_order(date_order)?;
    let bytes = read_archive(&path)?;
    ingestion
        .ingest_zip(&title, &bytes, order)
        .map_err(ImportError::from_service)
}

/// Media extracted from one imported archive.
#[tauri::command]
#[specta::specta]
pub async fn source_assets(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    source_id: String,
) -> Result<Vec<crate::managers::meeting_store::SourceAsset>, String> {
    ingestion
        .store
        .list_source_assets(crate::managers::meeting_store::Uuid(source_id))
        .map_err(|e| e.to_string())
}
