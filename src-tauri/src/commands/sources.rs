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

/// Phase 9 wizard preview: parse stats without persisting anything.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_preview_text(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    content: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ChatPreviewDto, String> {
    let order = parse_date_order(date_order)?;
    ingestion.preview_whatsapp_text(&content, order)
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

fn read_archive(path: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("archiveUnreadable: {e}"))?;
    if meta.len() > MAX_ARCHIVE_BYTES {
        return Err("archiveTooLarge".to_string());
    }
    std::fs::read(path).map_err(|e| format!("archiveUnreadable: {e}"))
}

/// Phase 9 wizard step 2 for `.zip`: inspect without extracting or persisting.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_preview_zip(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    path: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ZipPreviewDto, String> {
    let order = parse_date_order(date_order)?;
    let bytes = read_archive(&path)?;
    ingestion.preview_whatsapp_zip(&bytes, order)
}

/// Phase 9 wizard step 4 for `.zip`: import the chat plus its media.
#[tauri::command]
#[specta::specta]
pub async fn whatsapp_import_zip(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    path: String,
    title: String,
    date_order: Option<String>,
) -> Result<crate::managers::source_ingestion::ZipIngestResult, String> {
    let order = parse_date_order(date_order)?;
    let bytes = read_archive(&path)?;
    ingestion.ingest_whatsapp_zip(&title, &bytes, order)
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

/// Full-text search across projects, meetings, transcripts, tasks and imports.
#[tauri::command]
#[specta::specta]
pub async fn search_everything(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    query: String,
    project_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<crate::managers::meeting_store::SearchHit>, String> {
    ingestion
        .store
        .search(
            &query,
            project_id.map(crate::managers::meeting_store::Uuid),
            limit.unwrap_or(50).min(200),
        )
        .map_err(|e| e.to_string())
}

/// Rebuilds the search index. Cheap enough to call after an import.
#[tauri::command]
#[specta::specta]
pub async fn reindex_search(
    ingestion: State<'_, Arc<SourceIngestionService>>,
) -> Result<i64, String> {
    ingestion
        .store
        .reindex_search()
        .map(|n| n as i64)
        .map_err(|e| e.to_string())
}

/// Links an imported source to a project (many-to-many, plan §9).
#[tauri::command]
#[specta::specta]
pub async fn project_link_source(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    project_id: String,
    source_id: String,
) -> Result<(), String> {
    ingestion
        .store
        .link_source_to_project(
            crate::managers::meeting_store::Uuid(project_id),
            crate::managers::meeting_store::Uuid(source_id),
        )
        .map_err(|e| e.to_string())
}

/// Removes the link without deleting the source.
#[tauri::command]
#[specta::specta]
pub async fn project_unlink_source(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    project_id: String,
    source_id: String,
) -> Result<(), String> {
    ingestion
        .store
        .unlink_source_from_project(
            crate::managers::meeting_store::Uuid(project_id),
            crate::managers::meeting_store::Uuid(source_id),
        )
        .map_err(|e| e.to_string())
}

/// Source ids linked to one project.
#[tauri::command]
#[specta::specta]
pub async fn project_source_ids(
    ingestion: State<'_, Arc<SourceIngestionService>>,
    project_id: String,
) -> Result<Vec<String>, String> {
    ingestion
        .store
        .list_source_ids_for_project(crate::managers::meeting_store::Uuid(project_id))
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Capability-filtered multimodal analysis (plan §6.19)
// ---------------------------------------------------------------------------

use crate::managers::multimodal::{self, ModelCapability, UploadPlan, VisionClient};

/// Reads the active provider and its key, or a named reason.
fn multimodal_provider(
    app: &tauri::AppHandle,
) -> Result<(crate::settings::PostProcessProvider, String), String> {
    let settings = crate::settings::get_settings(app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| "noProvider".to_string())?;
    let key =
        crate::secret_store::get_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider.id)
            .unwrap_or_default();
    Ok((provider, key))
}

/// Models of the active provider that accept `modality` (`image` or `file`).
///
/// Nothing is hard-coded: the list comes from the provider and is filtered by
/// what the job actually needs, so a provider without a capable model simply
/// offers none.
#[tauri::command]
#[specta::specta]
pub async fn multimodal_models(
    app: tauri::AppHandle,
    modality: String,
) -> Result<Vec<ModelCapability>, String> {
    let (provider, key) = multimodal_provider(&app)?;
    let base = provider.base_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("clientFailed: {e}"))?;
    let payload: serde_json::Value = client
        .get(format!("{base}/models"))
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("modelListFailed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("modelListUnparsable: {e}"))?;

    Ok(multimodal::models_for(
        &multimodal::parse_model_capabilities(&payload),
        &modality,
    ))
}

/// What a confirmed run would transmit — computed without sending anything.
#[tauri::command]
#[specta::specta]
pub async fn multimodal_plan(
    app: tauri::AppHandle,
    ingestion: State<'_, Arc<SourceIngestionService>>,
    source_id: String,
    asset_ids: Vec<String>,
    model: ModelCapability,
) -> Result<UploadPlan, String> {
    let (provider, _) = multimodal_provider(&app)?;
    let assets = ingestion
        .store
        .list_source_assets(crate::managers::meeting_store::Uuid(source_id))
        .map_err(|e| e.to_string())?;

    // Only the assets the user actually selected: the plan forbids handing a
    // whole archive to a provider.
    let selected: Vec<(String, String, String, i64)> = assets
        .into_iter()
        .filter(|a| asset_ids.contains(&a.id))
        .map(|a| (a.id, a.original_name, a.mime, a.size_bytes))
        .collect();

    Ok(multimodal::plan_upload(&provider.label, &model, &selected))
}

/// Result of one confirmed multimodal run.
#[derive(Serialize, Type)]
pub struct MultimodalRunDto {
    pub analysed: i64,
    /// Per-file failures, named rather than summarised away.
    pub failed: Vec<String>,
}

/// Runs the analysis the user confirmed.
///
/// Takes the plan back as an argument on purpose: the files that leave the
/// device are exactly the ones the confirmation dialog listed, not a set
/// recomputed after the fact.
#[tauri::command]
#[specta::specta]
pub async fn multimodal_analyse(
    app: tauri::AppHandle,
    ingestion: State<'_, Arc<SourceIngestionService>>,
    asset_store: State<'_, Arc<crate::storage::asset_store::LocalAssetStore>>,
    plan: UploadPlan,
) -> Result<MultimodalRunDto, String> {
    use crate::storage::asset_store::{AssetStore, ReadVerification, StorageKey};

    let (provider, key) = multimodal_provider(&app)?;
    let client = multimodal::HttpVisionClient::new(
        provider.base_url.clone(),
        key,
        std::time::Duration::from_secs(120),
    );

    let mut analysed = 0i64;
    let mut failed = Vec::new();
    for item in &plan.included {
        let outcome = (|| -> Result<(), String> {
            let key = StorageKey::parse(&asset_storage_key(&ingestion, &item.asset_id)?)
                .map_err(|e| e.to_string())?;
            let mut reader = asset_store
                .get(&key, ReadVerification::None)
                .map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut reader, &mut bytes)
                .map_err(|e| format!("readFailed: {e}"))?;
            let description = client.describe(&plan.model, &item.mime, &bytes)?;
            ingestion
                .store
                .record_asset_analysis(
                    crate::managers::meeting_store::Uuid(item.asset_id.clone()),
                    &description,
                )
                .map_err(|e| e.to_string())
        })();
        match outcome {
            Ok(()) => analysed += 1,
            // One unreadable file must not abort the rest, and must not be
            // silently counted as analysed.
            Err(e) => failed.push(format!("{}: {e}", item.original_name)),
        }
    }

    // New descriptions must be findable straight away.
    let _ = ingestion.store.reindex_search();
    Ok(MultimodalRunDto { analysed, failed })
}

/// Resolves an asset id back to its relative storage key.
fn asset_storage_key(ingestion: &SourceIngestionService, asset_id: &str) -> Result<String, String> {
    ingestion
        .store
        .list_sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find_map(|source| {
            ingestion
                .store
                .list_source_assets(crate::managers::meeting_store::Uuid(source.id))
                .ok()?
                .into_iter()
                .find(|a| a.id == asset_id)
                .map(|a| a.storage_key)
        })
        .ok_or_else(|| "unknownAsset".to_string())
}
