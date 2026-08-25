//! Source ingestion (plan §5 `SourceIngestionService`, Phase 9): turns raw
//! WhatsApp chat exports into normalised `sources`/`source_items` rows.
//!
//! The service owns two policies:
//! - **Ambiguous date order fails visibly.** When the chat's first timestamps
//!   cannot disambiguate `dd.mm` vs `mm.dd`, import is refused until the
//!   caller passes an explicit order chosen in the preview UI (D3 doctrine).
//! - **Idempotent re-import.** The content hash decides creation; a repeated
//!   or overlapping export returns the existing source with `created: false`.

use super::meeting_store::{
    MeetingRepository, NewWhatsappSource, Source, SourceItem, StoreError, Uuid,
};
use super::whatsapp_importer::{self, DateOrder};
use sha2::Digest;
use std::sync::Arc;

pub const PARSER_VERSION: &str = "whatsapp-txt-v1";

#[derive(Clone)]
pub struct SourceIngestionService {
    pub(crate) store: Arc<dyn MeetingRepository>,
}

/// Result of one import attempt.
pub struct IngestResult {
    pub source: Source,
    pub items: Vec<SourceItem>,
    /// False when the same export was already imported (no-op).
    pub created: bool,
}

impl SourceIngestionService {
    pub fn new(store: Arc<dyn MeetingRepository>) -> Self {
        Self { store }
    }

    /// Imports a pasted or extracted WhatsApp chat text.
    ///
    /// `date_order` mirrors the preview UI choice; `None` means "decide from
    /// the data" and errors with `dateOrderAmbiguous` when that is impossible.
    pub fn ingest_whatsapp_text(
        &self,
        title: &str,
        content: &str,
        date_order: Option<DateOrder>,
    ) -> Result<IngestResult, StoreError> {
        let preview =
            whatsapp_importer::parse_chat(content, date_order).map_err(|e| self.invalid(&e))?;
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

        let (source, created) = self.store.create_whatsapp_source(NewWhatsappSource {
            title: title.to_string(),
            parser_version: PARSER_VERSION.to_string(),
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

    fn invalid(&self, code: &str) -> StoreError {
        StoreError::Conflict {
            entity: "sources",
            id: Uuid(String::new()),
            reason: code.to_string(),
        }
    }

    /// Parses without persisting — the wizard's preview step.
    pub fn preview_whatsapp_text(
        &self,
        content: &str,
        date_order: Option<DateOrder>,
    ) -> Result<ChatPreviewDto, String> {
        let preview = whatsapp_importer::parse_chat(content, date_order)
            .map_err(|e| format!("previewFailed: {e}"))?;
        Ok(ChatPreviewDto {
            participants: preview.participants,
            message_count: preview.messages.len() as i64,
            media_count: preview.media_messages as i64,
            date_order_ambiguous: preview.date_order_ambiguous,
        })
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
