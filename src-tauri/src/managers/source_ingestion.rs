//! Source ingestion (plan §5 `SourceIngestionService`, Phase 9): turns raw
//! WhatsApp chat exports into normalised `sources`/`source_items` rows.
//!
//! The service owns two policies:
//! - **Ambiguous date order fails visibly.** When the chat's first timestamps
//!   cannot disambiguate `dd.mm` vs `mm.dd`, import is refused until the
//!   caller passes an explicit order chosen in the preview UI (D3 doctrine).
//! - **Idempotent re-import.** The content hash decides creation; a repeated
//!   or overlapping export returns the existing source with `created: false`.

use super::chat_import::{ChatImporter, WhatsAppImporter};
use super::meeting_store::{
    MeetingRepository, NewChatSource, NewSourceAsset, Source, SourceItem, StoreError, Uuid,
};
use super::whatsapp_importer::{self, DateOrder, ZipLimits};
use crate::storage::asset_store::{
    AssetKind, AssetStore, ContentHash, PutRequest, PutSource, StorageKey,
};
use sha2::Digest;
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceIngestionService {
    pub(crate) store: Arc<dyn MeetingRepository>,
    /// Where extracted media lands. `None` in contexts without a resolved app
    /// data directory; ZIP import then refuses rather than dropping media.
    assets: Option<Arc<dyn AssetStore>>,
    /// The export format being read. Everything else in this service — hashing,
    /// archive walking, media storage, deduplication — is format-agnostic.
    importer: Arc<dyn ChatImporter>,
}

/// Result of one import attempt.
pub struct IngestResult {
    pub source: Source,
    pub items: Vec<SourceItem>,
    /// False when the same export was already imported (no-op).
    pub created: bool,
}

impl SourceIngestionService {
    /// Builds a service reading WhatsApp exports.
    pub fn new(store: Arc<dyn MeetingRepository>) -> Self {
        Self::with_importer(store, Arc::new(WhatsAppImporter))
    }

    /// Builds a service for an arbitrary export format.
    pub fn with_importer(
        store: Arc<dyn MeetingRepository>,
        importer: Arc<dyn ChatImporter>,
    ) -> Self {
        Self {
            store,
            assets: None,
            importer,
        }
    }

    /// Attaches the blob store used for media extracted from archives.
    pub fn with_assets(mut self, assets: Arc<dyn AssetStore>) -> Self {
        self.assets = Some(assets);
        self
    }

    /// Imports a pasted or extracted WhatsApp chat text.
    ///
    /// `date_order` mirrors the preview UI choice; `None` means "decide from
    /// the data" and errors with `dateOrderAmbiguous` when that is impossible.
    pub fn ingest_text(
        &self,
        title: &str,
        content: &str,
        date_order: Option<DateOrder>,
    ) -> Result<IngestResult, StoreError> {
        let preview = self
            .importer
            .parse(content, date_order)
            .map_err(|e| self.invalid(&e))?;
        if preview.date_order_ambiguous {
            return Err(self.invalid("dateOrderAmbiguous"));
        }

        let mut hasher = sha2::Sha256::default();
        hasher.update(content.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        let messages = preview
            .messages
            .iter()
            .map(|m| (m.author.clone(), m.text.clone(), m.is_media))
            .collect();

        let (source, created) = self.store.create_chat_source(NewChatSource {
            source_type: self.importer.source_type().to_string(),
            title: title.to_string(),
            parser_version: self.importer.parser_version().to_string(),
            content_hash,
            messages,
        })?;
        let items = self.store.list_source_items(Uuid(source.id.clone()))?;
        Ok(IngestResult {
            source,
            items,
            created,
        })
    }

    /// Inspects an archive without extracting or persisting anything.
    ///
    /// This is the wizard's step 2: the user sees participants, message and
    /// media counts *before* any byte leaves the archive, and an unsafe or
    /// malformed archive is refused here rather than half-imported.
    pub fn preview_zip(
        &self,
        archive: &[u8],
        date_order: Option<DateOrder>,
    ) -> Result<ZipPreviewDto, String> {
        let limits = ZipLimits::default();
        let entries = whatsapp_importer::inspect_zip(archive, &limits)
            .map_err(|e| format!("archiveRejected: {e}"))?;
        let Some(chat_entry) = self.importer.pick_chat_entry(&entries) else {
            return Err("noChatFileInArchive".to_string());
        };
        let bytes = whatsapp_importer::extract_entry(archive, chat_entry, &limits)
            .map_err(|e| format!("archiveRejected: {e}"))?;
        let content = self.importer.decode_export(&bytes);
        let chat = self.preview_text(&content, date_order)?;

        let media: Vec<String> = entries
            .iter()
            .filter(|e| e.name != chat_entry.name && !e.name.ends_with('/'))
            .map(|e| e.name.clone())
            .collect();
        Ok(ZipPreviewDto {
            chat_file: chat_entry.name.clone(),
            media_files: media.len() as i64,
            // Named so the user can recognise what would be stored; the list is
            // capped because a year of group chat can hold thousands.
            media_sample: media.into_iter().take(20).collect(),
            chat,
        })
    }

    /// Imports the chat text and stores every other archive entry as media.
    ///
    /// Order matters: the text is parsed, deduplicated and persisted first, so
    /// a user without any cloud account still ends up with a searchable chat
    /// even if a media file later turns out to be unreadable. Media is stored
    /// verbatim and marked `not_analysed` — the plan forbids presenting
    /// unsupported files as understood.
    pub fn ingest_zip(
        &self,
        title: &str,
        archive: &[u8],
        date_order: Option<DateOrder>,
    ) -> Result<ZipIngestResult, String> {
        let assets = self
            .assets
            .as_ref()
            .ok_or_else(|| "assetStoreUnavailable".to_string())?;
        let limits = ZipLimits::default();
        let entries = whatsapp_importer::inspect_zip(archive, &limits)
            .map_err(|e| format!("archiveRejected: {e}"))?;
        let Some(chat_entry) = self.importer.pick_chat_entry(&entries) else {
            return Err("noChatFileInArchive".to_string());
        };
        let bytes = whatsapp_importer::extract_entry(archive, chat_entry, &limits)
            .map_err(|e| format!("archiveRejected: {e}"))?;
        let content = self.importer.decode_export(&bytes);

        let imported = self.ingest_text(title, &content, date_order).map_err(|e| {
            let message = e.to_string();
            if message.contains("dateOrderAmbiguous") {
                "dateOrderAmbiguous".to_string()
            } else {
                message
            }
        })?;

        let mut drafts = Vec::new();
        let mut skipped = Vec::new();
        for entry in &entries {
            if entry.name == chat_entry.name || entry.name.ends_with('/') {
                continue;
            }
            let payload = match whatsapp_importer::extract_entry(archive, entry, &limits) {
                Ok(payload) => payload,
                // One unreadable member must not abort an otherwise good
                // import; it is reported instead of silently dropped.
                Err(e) => {
                    skipped.push(format!("{}: {e}", entry.name));
                    continue;
                }
            };
            let digest = format!("{:x}", sha2::Sha256::digest(&payload));
            let asset_id = format!("asst_{}", uuid::Uuid::new_v4());
            let key = match StorageKey::asset_blob(&asset_id, extension_of(&entry.name)) {
                Ok(key) => key,
                Err(e) => {
                    skipped.push(format!("{}: {e}", entry.name));
                    continue;
                }
            };
            let size = payload.len() as i64;
            let put = assets.put(PutRequest {
                key: key.clone(),
                kind: AssetKind::Blob,
                expected_hash: ContentHash::from_digest_hex(&digest).ok(),
                content: PutSource::Reader(Box::new(std::io::Cursor::new(payload))),
            });
            match put {
                Ok(_) => drafts.push(NewSourceAsset {
                    storage_key: key.as_str().to_string(),
                    original_name: entry.name.clone(),
                    mime: mime_of(&entry.name).to_string(),
                    size_bytes: size,
                    content_hash: digest,
                }),
                Err(e) => skipped.push(format!("{}: {e}", entry.name)),
            }
        }

        let stored = self
            .store
            .attach_source_assets(Uuid(imported.source.id.clone()), drafts)
            .map_err(|e| e.to_string())?;

        Ok(ZipIngestResult {
            item_count: imported.items.len() as i64,
            media_count: stored.len() as i64,
            skipped,
            created: imported.created,
            source: imported.source,
        })
    }

    fn invalid(&self, code: &str) -> StoreError {
        StoreError::Conflict {
            entity: "sources",
            id: Uuid(String::new()),
            reason: code.to_string(),
        }
    }

    /// Parses without persisting — the wizard's preview step.
    pub fn preview_text(
        &self,
        content: &str,
        date_order: Option<DateOrder>,
    ) -> Result<ChatPreviewDto, String> {
        let preview = self
            .importer
            .parse(content, date_order)
            .map_err(|e| format!("previewFailed: {e}"))?;
        Ok(ChatPreviewDto {
            participants: preview.participants,
            message_count: preview.messages.len() as i64,
            media_count: preview.media_messages as i64,
            date_order_ambiguous: preview.date_order_ambiguous,
        })
    }
}

/// Decodes chat bytes, tolerating the BOM-prefixed UTF-8 and UTF-16 exports
/// WhatsApp produces on different platforms.
pub(crate) fn decode_chat_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(body).into_owned()
}

/// Lower-cased extension, when it is plain enough to embed in a storage key.
fn extension_of(name: &str) -> Option<&str> {
    let file = name.rsplit('/').next()?;
    let ext = file.rsplit_once('.')?.1;
    // Storage keys stay predictable: anything exotic is stored without one.
    (!ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then_some(ext)
}

/// Coarse media type for display. Deliberately small: the value only drives
/// icons and the "not analysed" wording, never a processing decision.
fn mime_of(name: &str) -> &'static str {
    match extension_of(name)
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("opus") => "audio/opus",
        Some("ogg") => "audio/ogg",
        Some("m4a") => "audio/mp4",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Serializable preview for the import wizard.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ChatPreviewDto {
    pub participants: Vec<String>,
    pub message_count: i64,
    pub media_count: i64,
    pub date_order_ambiguous: bool,
}

/// Archive-level preview: what the wizard shows in step 2.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ZipPreviewDto {
    pub chat_file: String,
    pub media_files: i64,
    pub media_sample: Vec<String>,
    pub chat: ChatPreviewDto,
}

/// Outcome of a ZIP import.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ZipIngestResult {
    pub source: Source,
    pub created: bool,
    pub item_count: i64,
    pub media_count: i64,
    /// Entries that could not be stored, with the reason. Never silently lost.
    pub skipped: Vec<String>,
}
