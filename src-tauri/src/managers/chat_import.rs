//! The seam between "a chat export format" and "importing a chat export".
//!
//! # Why
//!
//! [`super::source_ingestion::SourceIngestionService`] does a lot that has
//! nothing to do with WhatsApp: hashing the export for deduplication, walking a
//! ZIP central directory under size limits, storing media blobs, recording
//! skipped entries, persisting messages. Exactly four things are
//! platform-specific — the parser, the version string stamped on the row, how
//! the chat transcript is recognised inside an archive, and how its bytes are
//! decoded.
//!
//! Those four are this trait. A second platform (Telegram, Signal, Slack) is
//! then an implementation plus a registry entry, not a second copy of the
//! ingest and archive logic.
//!
//! # Why now, with one implementation
//!
//! Normally one waits for the second case before abstracting. The seam is
//! drawn here anyway because the duplication it prevents is already visible:
//! the text and ZIP paths each needed their own preview *and* ingest method,
//! and every one of them names WhatsApp in its signature. Adding a platform
//! without this would mean four more methods and a second copy of the archive
//! handling.
//!
//! The trait is deliberately narrow. It does not model "a source" or "an
//! importer pipeline" — only the four decisions the pipeline cannot make on its
//! own.

use super::whatsapp_importer::{self, ChatPreview, DateOrder, ZipEntryInfo};

/// One chat-export format the app knows how to read.
pub trait ChatImporter: Send + Sync {
    /// Discriminator written to `sources.source_type`.
    ///
    /// Persisted, so it is effectively frozen once any row carries it: existing
    /// sources are found by this value.
    fn source_type(&self) -> &'static str;

    /// Written to `sources.parser_version`. Bump it whenever a parser change
    /// would produce different messages from the same bytes, so previously
    /// imported sources can be recognised as parsed by an older reading.
    fn parser_version(&self) -> &'static str;

    /// Parses an export into messages and participants without persisting.
    ///
    /// `date_order` mirrors the wizard's choice; `None` means "infer", and the
    /// implementation reports [`ChatPreview::date_order_ambiguous`] when the
    /// data alone cannot decide.
    fn parse(&self, content: &str, date_order: Option<DateOrder>) -> Result<ChatPreview, String>;

    /// Turns raw export bytes into text.
    ///
    /// Separate from [`Self::parse`] because archives hand over bytes whose
    /// encoding is a platform quirk (WhatsApp ships UTF-16 with a BOM on some
    /// platforms), while pasted text arrives already decoded.
    fn decode_export(&self, bytes: &[u8]) -> String;

    /// Picks the chat transcript out of an archive's entries.
    ///
    /// Returns `None` when the archive contains nothing this importer
    /// recognises as a transcript; the caller turns that into a user-facing
    /// `noChatFileInArchive`.
    fn pick_chat_entry<'a>(&self, entries: &'a [ZipEntryInfo]) -> Option<&'a ZipEntryInfo>;
}

/// WhatsApp's `_chat.txt` exports, plain or inside the ZIP the app produces.
pub struct WhatsAppImporter;

impl ChatImporter for WhatsAppImporter {
    fn source_type(&self) -> &'static str {
        "whatsapp_export"
    }

    fn parser_version(&self) -> &'static str {
        "whatsapp-txt-v1"
    }

    fn parse(&self, content: &str, date_order: Option<DateOrder>) -> Result<ChatPreview, String> {
        whatsapp_importer::parse_chat(content, date_order)
    }

    fn decode_export(&self, bytes: &[u8]) -> String {
        super::source_ingestion::decode_chat_text(bytes)
    }

    /// The shallowest `.txt`, breaking ties by size.
    ///
    /// WhatsApp exports put the transcript at the archive root while media sits
    /// in subdirectories, so depth is the strongest signal; size settles the
    /// case where an export also ships a README-like text file.
    fn pick_chat_entry<'a>(&self, entries: &'a [ZipEntryInfo]) -> Option<&'a ZipEntryInfo> {
        entries
            .iter()
            .filter(|e| e.name.to_lowercase().ends_with(".txt"))
            .min_by_key(|e| {
                (
                    e.name.matches('/').count(),
                    std::cmp::Reverse(e.uncompressed_size),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only `name` and `uncompressed_size` matter to `pick_chat_entry`; the
    /// rest is filled with values a real central directory could hold.
    fn entry(name: &str, size: u64) -> ZipEntryInfo {
        ZipEntryInfo {
            name: name.to_string(),
            name_bytes: name.as_bytes().to_vec(),
            method: 0,
            compressed_size: size,
            uncompressed_size: size,
            local_header_offset: 0,
            crc32: 0,
        }
    }

    #[test]
    fn the_parser_version_string_is_pinned() {
        // Every already-imported source carries this string in
        // `sources.parser_version`. Changing it silently would make existing
        // rows look like they came from an unknown parser, so a change has to
        // be deliberate enough to also update this test.
        assert_eq!(WhatsAppImporter.parser_version(), "whatsapp-txt-v1");
    }

    #[test]
    fn picks_the_shallowest_transcript_and_ignores_media() {
        let entries = vec![
            entry("media/photo.jpg", 900_000),
            entry("nested/dir/notes.txt", 50),
            entry("_chat.txt", 1_000),
        ];
        assert_eq!(
            WhatsAppImporter
                .pick_chat_entry(&entries)
                .map(|e| e.name.as_str()),
            Some("_chat.txt")
        );
    }

    #[test]
    fn breaks_depth_ties_by_size() {
        let entries = vec![entry("readme.txt", 20), entry("_chat.txt", 90_000)];
        assert_eq!(
            WhatsAppImporter
                .pick_chat_entry(&entries)
                .map(|e| e.name.as_str()),
            Some("_chat.txt")
        );
    }

    #[test]
    fn an_archive_without_text_has_no_transcript() {
        let entries = vec![entry("media/photo.jpg", 10), entry("media/clip.mp4", 20)];
        assert!(WhatsAppImporter.pick_chat_entry(&entries).is_none());
    }
}
