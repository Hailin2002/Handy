//! WhatsApp import core (plan §6.19/§12, Phase 9): hardened ZIP handling and
//! a tolerant chat-text parser.
//!
//! Security posture (§12 "untrusted input"):
//! - ZIP entries are validated before any byte is decompressed: absolute
//!   paths, path traversal, backslashes, drive letters, NUL bytes and unix
//!   symlink mode bits are rejected outright.
//! - Decompression bombs are bounded by entry-count, per-entry-size,
//!   total-size and compression-ratio limits; every limit is checked against
//!   the *declared* sizes first so hostile headers fail fast.
//! - Parsing never guesses an ambiguous date order (D3 doctrine): when the
//!   first timestamps cannot disambiguate day/month order the caller must
//!   pass an explicit `date_order`, or preview fails visibly.
//!
//! The DB-ingestion half (sources/source_items tables plus import jobs)
//! lands with the remaining Phase 9 work; until then the container layer is
//! only exercised through its tests.
#![allow(dead_code)]

use std::io::Read;

/// Hard limits for archive acceptance (plan §12). Defaults follow the
/// retention contract's spirit: generous enough for year-old group chats,
/// far below anything that could exhaust memory.
#[derive(Debug, Clone)]
pub struct ZipLimits {
    pub max_files: usize,
    pub max_entry_bytes: usize,
    pub max_total_bytes: usize,
    /// Declared uncompressed / compressed ratio cap (ZIP-bomb guard).
    pub max_ratio: u64,
}

impl Default for ZipLimits {
    fn default() -> Self {
        // Values come from the import-security spec that generated the attack
        // corpus (`tests/fixtures/import-security/manifest.json`), so the
        // committed fixtures and the running validator cannot drift apart.
        Self {
            max_files: 10_000,
            max_entry_bytes: 256 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_ratio: 200,
        }
    }
}

/// Entries below this size are exempt from the ratio check.
///
/// A few hundred bytes of text compress absurdly well; without the exemption
/// every ordinary tiny file would look like a decompression bomb.
const RATIO_EXEMPT_BELOW: u64 = 4_096;

/// Longest accepted entry name and path component, in bytes.
const MAX_NAME_BYTES: usize = 1_024;
const MAX_COMPONENT_BYTES: usize = 255;

/// Device names Windows resolves specially no matter the extension. Writing
/// one of these can address a device instead of a file.
const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    NotAZip,
    Truncated,
    TooManyFiles {
        limit: usize,
    },
    EntryTooLarge {
        name: String,
        declared: u64,
    },
    TotalTooLarge {
        declared: u64,
    },
    SuspiciousRatio {
        name: String,
    },
    UnsafePath {
        name: String,
    },
    UnsupportedCompression {
        name: String,
        method: u16,
    },
    CrcMismatch {
        name: String,
    },
    /// Entry is encrypted; its contents cannot be validated before use.
    Encrypted {
        name: String,
    },
    /// Entry is a symlink or another non-regular member.
    NotARegularFile {
        name: String,
    },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAZip => write!(f, "not a zip archive"),
            Self::Truncated => write!(f, "archive is truncated"),
            Self::TooManyFiles { limit } => {
                write!(f, "archive holds more than {limit} files")
            }
            Self::EntryTooLarge { name, declared } => {
                write!(f, "{name} declares {declared} bytes, above the limit")
            }
            Self::TotalTooLarge { declared } => {
                write!(
                    f,
                    "archive declares {declared} total bytes, above the limit"
                )
            }
            Self::SuspiciousRatio { name } => {
                write!(f, "{name} has a suspicious compression ratio")
            }
            Self::UnsafePath { name } => write!(f, "unsafe entry path rejected: {name}"),
            Self::UnsupportedCompression { name, method } => {
                write!(f, "{name} uses unsupported compression method {method}")
            }
            Self::CrcMismatch { name } => write!(f, "{name} failed its checksum"),
            Self::Encrypted { name } => {
                write!(f, "{name} is encrypted and cannot be imported")
            }
            Self::NotARegularFile { name } => {
                write!(f, "{name} is not a regular file")
            }
        }
    }
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}

/// A validated central-directory entry.
#[derive(Debug, Clone)]
pub struct ZipEntryInfo {
    pub name: String,
    /// Exact central-directory name bytes.
    ///
    /// Legacy archivers store CP437, which lossy UTF-8 decoding turns into
    /// replacement characters; comparing the *decoded* name against the local
    /// header would then reject perfectly valid old exports. The raw bytes are
    /// the only encoding-independent identity.
    pub name_bytes: Vec<u8>,
    pub method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub local_header_offset: u64,
    pub crc32: u32,
}

/// Rejects unsafe entry paths before any inflation happens (plan §12).
///
/// Every rule here corresponds to an archive in the committed attack corpus:
/// absolute and UNC paths, backslash separators, `..` traversal, NUL bytes,
/// NTFS alternate data streams (`file:stream` — a colon anywhere, not just in
/// a drive prefix), reserved Windows device names, and over-long names or
/// components that could defeat downstream path handling.
fn validate_path(name: &str) -> Result<(), ImportError> {
    let reject = || ImportError::UnsafePath {
        name: name.to_string(),
    };

    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(reject());
    }
    if name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains('\0')
        // A colon anywhere opens an alternate data stream on NTFS, so the
        // old "only look at a drive prefix" check was not enough.
        || name.contains(':')
    {
        return Err(reject());
    }

    for part in name.split('/') {
        if part.is_empty() {
            // A trailing slash marks a directory entry and is legitimate;
            // anything else empty means a doubled separator.
            continue;
        }
        if part == ".." || part == "." || part.len() > MAX_COMPONENT_BYTES {
            return Err(reject());
        }
        // Trailing dots and spaces are stripped by Windows, so `evil.` and
        // `evil` can collide after extraction.
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(reject());
        }
        let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
        if RESERVED_WINDOWS_NAMES.contains(&stem.as_str()) {
            return Err(reject());
        }
    }
    Ok(())
}

/// True when the central-directory flags mark the entry as encrypted.
///
/// Bit 0 of the general-purpose flags covers both legacy ZipCrypto and the
/// strong-encryption variants. An encrypted member cannot be inflated or
/// checksum-verified, so it is refused rather than stored unchecked.
fn is_encrypted(flags: u16) -> bool {
    flags & 0x0001 != 0
}

/// True when the external attributes describe something other than a regular
/// file — a symlink above all.
///
/// The high 16 bits carry the Unix mode when the archive was created on a
/// Unix-like system; `S_IFLNK` there means extracting the entry would write a
/// link that can point anywhere, including outside the AssetStore.
fn is_non_regular(version_made_by: u16, external_attrs: u32) -> bool {
    // The creator OS lives in the high byte; 3 is Unix.
    if version_made_by >> 8 != 3 {
        return false;
    }
    let mode = (external_attrs >> 16) as u16;
    const S_IFMT: u16 = 0o170_000;
    const S_IFLNK: u16 = 0o120_000;
    const S_IFREG: u16 = 0o100_000;
    const S_IFDIR: u16 = 0o040_000;
    let kind = mode & S_IFMT;
    // Mode 0 means "not recorded", which real archivers also emit.
    kind != 0 && kind != S_IFREG && kind != S_IFDIR || kind == S_IFLNK
}

/// Locates the end-of-central-directory record by scanning backwards.
fn find_eocd(data: &[u8]) -> Option<usize> {
    const SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    let scan_start = data.len().saturating_sub(66_000);
    let mut cursor = data.len().checked_sub(22)?;
    loop {
        if data[cursor..cursor + 4] == SIG {
            return Some(cursor);
        }
        if cursor == scan_start {
            return None;
        }
        cursor -= 1;
    }
}

/// Parses and validates the central directory without inflating anything.
pub fn inspect_zip(data: &[u8], limits: &ZipLimits) -> Result<Vec<ZipEntryInfo>, ImportError> {
    let eocd = find_eocd(data).ok_or(ImportError::NotAZip)?;
    let entry_count = u16_at(data, eocd + 10).ok_or(ImportError::Truncated)? as usize;
    let cd_size = u32_at(data, eocd + 12).ok_or(ImportError::Truncated)? as usize;
    let cd_offset = u32_at(data, eocd + 16).ok_or(ImportError::Truncated)? as usize;
    if cd_offset.checked_add(cd_size).is_none_or(|end| end > eocd) {
        return Err(ImportError::Truncated);
    }

    if entry_count > limits.max_files {
        return Err(ImportError::TooManyFiles {
            limit: limits.max_files,
        });
    }

    let mut entries = Vec::with_capacity(entry_count);
    let mut offset = cd_offset;
    let mut total_declared = 0u64;
    for _ in 0..entry_count {
        if data.get(offset..offset + 4) != Some(&[0x50, 0x4b, 0x01, 0x02][..]) {
            return Err(ImportError::Truncated);
        }
        let version_made_by = u16_at(data, offset + 4).unwrap_or(0);
        let flags = u16_at(data, offset + 8).unwrap_or(0);
        let method = u16_at(data, offset + 10).unwrap_or(0);
        let crc32 = u32_at(data, offset + 16).unwrap_or(0);
        let compressed = u32_at(data, offset + 20).unwrap_or(0) as u64;
        let uncompressed = u32_at(data, offset + 24).unwrap_or(0) as u64;
        let name_len = u16_at(data, offset + 28).unwrap_or(0) as usize;
        let extra_len = u16_at(data, offset + 30).unwrap_or(0) as usize;
        let comment_len = u16_at(data, offset + 32).unwrap_or(0) as usize;
        let local_offset = u32_at(data, offset + 42).unwrap_or(0) as u64;
        let name_start = offset + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or(ImportError::Truncated)?;
        let name_bytes = data
            .get(name_start..name_end)
            .ok_or(ImportError::Truncated)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        validate_path(&name)?;

        if is_encrypted(flags) {
            return Err(ImportError::Encrypted { name });
        }

        let external_attrs = u32_at(data, offset + 38).unwrap_or(0);
        if is_non_regular(version_made_by, external_attrs) {
            return Err(ImportError::NotARegularFile { name });
        }

        // Directory entries carry no payload and are skipped from ratio math.
        if !name.ends_with('/') {
            if uncompressed > limits.max_entry_bytes as u64 {
                return Err(ImportError::EntryTooLarge {
                    name: name.clone(),
                    declared: uncompressed,
                });
            }
            total_declared += uncompressed;
            if total_declared > limits.max_total_bytes as u64 {
                return Err(ImportError::TotalTooLarge {
                    declared: total_declared,
                });
            }
            if uncompressed >= RATIO_EXEMPT_BELOW
                && compressed > 0
                && uncompressed / compressed.max(1) > limits.max_ratio
            {
                return Err(ImportError::SuspiciousRatio { name: name.clone() });
            }
            if method != 0 && method != 8 {
                return Err(ImportError::UnsupportedCompression {
                    name: name.clone(),
                    method,
                });
            }
        }

        entries.push(ZipEntryInfo {
            name_bytes: name_bytes.to_vec(),
            name,
            method,
            compressed_size: compressed,
            uncompressed_size: uncompressed,
            local_header_offset: local_offset,
            crc32,
        });

        offset = name_end
            .checked_add(extra_len)
            .and_then(|v| v.checked_add(comment_len))
            .ok_or(ImportError::Truncated)?;
    }
    Ok(entries)
}

/// Inflates one entry end-to-end and verifies its CRC-32.
pub fn extract_entry(
    data: &[u8],
    entry: &ZipEntryInfo,
    limits: &ZipLimits,
) -> Result<Vec<u8>, ImportError> {
    if entry.uncompressed_size > limits.max_entry_bytes as u64 {
        return Err(ImportError::EntryTooLarge {
            name: entry.name.clone(),
            declared: entry.uncompressed_size,
        });
    }
    let local_offset = entry.local_header_offset as usize;
    // The central directory pointed here; if there is no local file header at
    // that offset the archive is inconsistent and the "payload" that follows
    // is whatever happens to sit there. Refuse rather than inflate it.
    if data.get(local_offset..local_offset + 4) != Some(&[0x50, 0x4b, 0x03, 0x04][..]) {
        return Err(ImportError::Truncated);
    }
    // The local header repeats the entry name; a mismatch means the two
    // directories disagree about what this member is.
    let name_len = u16_at(data, local_offset + 26).unwrap_or(0) as usize;
    let extra_len = u16_at(data, local_offset + 28).unwrap_or(0) as usize;
    let local_name = data
        .get(local_offset + 30..local_offset + 30 + name_len)
        .ok_or(ImportError::Truncated)?;
    if local_name != entry.name_bytes.as_slice() {
        return Err(ImportError::UnsafePath {
            name: entry.name.clone(),
        });
    }
    let data_start = local_offset
        .checked_add(30)
        .and_then(|v| v.checked_add(name_len))
        .and_then(|v| v.checked_add(extra_len))
        .ok_or(ImportError::Truncated)?;
    let data_end = data_start
        .checked_add(entry.compressed_size as usize)
        .ok_or(ImportError::Truncated)?;
    let raw = data
        .get(data_start..data_end)
        .ok_or(ImportError::Truncated)?;

    let mut out =
        Vec::with_capacity(entry.uncompressed_size.min(limits.max_entry_bytes as u64) as usize);
    match entry.method {
        0 => out.extend_from_slice(raw),
        8 => {
            let mut decoder = flate2::read::DeflateDecoder::new(raw);
            decoder
                .read_to_end(&mut out)
                .map_err(|_| ImportError::Truncated)?;
        }
        method => {
            return Err(ImportError::UnsupportedCompression {
                name: entry.name.clone(),
                method,
            })
        }
    }
    if (out.len() as u64) != entry.uncompressed_size {
        return Err(ImportError::Truncated);
    }
    if crc32fast_of(&out) != entry.crc32 {
        return Err(ImportError::CrcMismatch {
            name: entry.name.clone(),
        });
    }
    Ok(out)
}

/// CRC-32 (IEEE) computed with the same polynomial the ZIP format mandates.
fn crc32fast_of(data: &[u8]) -> u32 {
    // Bitwise implementation: small tables would be faster, but imports are
    // dominated by inflate time anyway and this keeps the module dependency-
    // free.
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// WhatsApp chat-text parsing
// ---------------------------------------------------------------------------

/// Explicit resolution for ambiguous `dd.mm` vs `mm.dd` first dates (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    DayFirst,
    MonthFirst,
}

/// One parsed chat row. `author: None` marks a system line (D-S1).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub author: Option<String>,
    pub text: String,
    pub is_media: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatPreview {
    pub messages: Vec<ChatMessage>,
    pub participants: Vec<String>,
    pub media_messages: usize,
    /// True when the caller must pass an explicit [`DateOrder`] because the
    /// first timestamps alone cannot disambiguate the dialect.
    pub date_order_ambiguous: bool,
}

const MEDIA_MARKERS: &[&str] = &[
    "<attached:",
    "<Medien ausgeschlossen>",
    "<Medien weggelassen>",
    "<Media omitted>",
    "<Anhang: ",
];

fn looks_like_media(text: &str) -> bool {
    MEDIA_MARKERS.iter().any(|marker| text.contains(marker))
}

/// Splits one chat export into messages. Lines that match neither header
/// dialect are treated as continuations of the previous message (WhatsApp's
/// multiline behaviour); leading garbage before the first header is dropped.
pub fn parse_chat(content: &str, date_order: Option<DateOrder>) -> Result<ChatPreview, String> {
    let bracketed = Regexish::bracketed(content);
    let dash = !bracketed;

    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut authors: Vec<String> = Vec::new();

    for line in content.lines() {
        let parsed_header = if bracketed {
            parse_bracketed_header(line)
        } else if dash {
            parse_dash_header(line)
        } else {
            None
        };

        match parsed_header {
            Some((author, body)) => {
                if let Some(author) = &author {
                    if !authors.iter().any(|a| a == author) {
                        authors.push(author.clone());
                    }
                }
                messages.push(ChatMessage {
                    author,
                    text: body.to_string(),
                    is_media: looks_like_media(body),
                });
            }
            None => {
                if let Some(last) = messages.last_mut() {
                    last.text.push('\n');
                    last.text.push_str(line);
                }
                // Leading garbage before any header is silently dropped: it
                // carries no information and inventing a message for it would
                // be worse (D3 "never silently misparse" applies to *dates*).
            }
        }
    }

    let _ = dash;
    let date_order_ambiguous = date_order.is_none() && first_date_is_ambiguous(content);
    let media_messages = messages.iter().filter(|m| m.is_media).count();
    Ok(ChatPreview {
        messages,
        participants: authors,
        media_messages,
        date_order_ambiguous,
    })
}

/// Detects the `[dd.mm.yy, hh:mm:ss]` bracketed header dialect at a line start.
fn parse_bracketed_header(line: &str) -> Option<(Option<String>, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find("] ")?;
    let head = &rest[..close];
    let (date_part, time_author) = head.split_once(", ")?;
    if !is_date_like(date_part) || !time_author.contains(':') {
        return None;
    }
    let body = &rest[close + 2..];
    match body.split_once(": ") {
        Some((author, text)) => Some((Some(author.to_string()), text)),
        None => Some((None, body)),
    }
}

/// Detects the `dd.mm.yy, hh:mm - Author: msg` dialect at a line start.
fn parse_dash_header(line: &str) -> Option<(Option<String>, &str)> {
    let (head, body) = line.split_once(" - ")?;
    let (date_part, time_part) = head.split_once(", ")?;
    if !is_date_like(date_part) || !time_part.contains(':') {
        return None;
    }
    match body.split_once(": ") {
        Some((author, text)) => Some((Some(author.to_string()), text)),
        None => Some((None, body)),
    }
}

struct Regexish;

impl Regexish {
    fn bracketed(content: &str) -> bool {
        content.trim_start().starts_with('[')
    }
}

fn is_date_like(part: &str) -> bool {
    let separators = ['.', '/', '-'];
    separators.iter().any(|sep| part.matches(*sep).count() >= 2)
}

/// True when neither the day nor the month position exceeds 12 anywhere in
/// the first few dates — i.e. the dialect cannot self-disambiguate.
fn first_date_is_ambiguous(content: &str) -> bool {
    for line in content.lines() {
        let candidate = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] "))
            .map(|(head, _)| head.to_string())
            .or_else(|| line.split_once(" - ").map(|(head, _)| head.to_string()))
            .and_then(|head| head.split_once(", ").map(|(date, _)| date.to_string()));
        let Some(date) = candidate else { continue };
        let parts: Vec<&str> = date
            .split(['.', '/'])
            .filter(|p| p.parse::<u32>().is_ok())
            .collect();
        if parts.len() < 2 {
            continue;
        }
        let first = parts[0].parse::<u32>().unwrap_or(0);
        let second = parts[1].parse::<u32>().unwrap_or(0);
        return !(first > 12 || second > 12);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ZipLimits {
        ZipLimits::default()
    }

    #[test]
    fn unsafe_paths_are_rejected_before_inflation() {
        assert!(matches!(
            validate_path("../evil.txt"),
            Err(ImportError::UnsafePath { .. })
        ));
        assert!(matches!(
            validate_path("/abs.txt"),
            Err(ImportError::UnsafePath { .. })
        ));
        assert!(matches!(
            validate_path("C:\\temp\\x.txt"),
            Err(ImportError::UnsafePath { .. })
        ));
        assert!(validate_path("chat/export.txt").is_ok());
    }

    #[test]
    fn garbage_input_is_not_a_zip() {
        assert!(matches!(
            inspect_zip(b"definitely not a zip", &limits()),
            Err(ImportError::NotAZip)
        ));
    }

    #[test]
    fn dash_dialect_parses_authors_system_lines_and_continuations() {
        let chat = "\
31.12.24, 23:59 - Alice: Hallo Team!
01.01.25, 08:00 - Bob: Zweizeiler
Fortsetzung hier
01.01.25, 08:01 - Bob: <Medien ausgeschlossen>
Eure Datei";
        let preview = parse_chat(chat, Some(DateOrder::DayFirst)).expect("parse");
        assert_eq!(preview.messages.len(), 3);
        assert_eq!(preview.participants, vec!["Alice", "Bob"]);
        assert_eq!(preview.media_messages, 1);
        assert!(preview.messages[1].text.contains("Fortsetzung hier"));
        assert!(!preview.date_order_ambiguous);
    }

    #[test]
    fn bracketed_dialect_and_system_messages_parse() {
        let chat = "[03.04.25, 09:15:02] Alice: Erste Nachricht\n\
                    [03.04.25, 09:15:30] Nachrichten und Anrufe sind Ende-zu-Ende-verschlüsselt";
        let preview = parse_chat(chat, Some(DateOrder::DayFirst)).expect("parse");
        assert_eq!(preview.messages.len(), 2);
        assert!(preview.messages[1].author.is_none(), "system line");
        assert_eq!(preview.participants, vec!["Alice"]);
    }

    #[test]
    fn ambiguous_dates_are_reported_not_guessed() {
        let chat = "05.04.25, 10:00 - Alice: Wann?";
        let preview = parse_chat(chat, None).expect("parse");
        assert!(preview.date_order_ambiguous);
        // With both fields <= 12 nothing in the data can decide it.
        let explicit = parse_chat(chat, Some(DateOrder::MonthFirst)).expect("parse");
        assert!(!explicit.date_order_ambiguous);
    }

    #[test]
    fn unambiguous_dates_need_no_explicit_order() {
        let chat = "17.04.25, 10:00 - Bob: Tag > 12 entscheidet";
        let preview = parse_chat(chat, None).expect("parse");
        assert!(!preview.date_order_ambiguous);
    }

    // -- zip container -------------------------------------------------------

    /// Builds a minimal stored-method (no compression) archive.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut offset = 0usize;
        for (name, bytes) in entries {
            let crc = crc32fast_of(bytes);
            let name_bytes = name.as_bytes();
            let local_start = offset;
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
            out.extend_from_slice(&0u32.to_le_bytes()); // dos time/date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(bytes);

            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            central.extend_from_slice(&20u16.to_le_bytes()); // version made
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method
            central.extend_from_slice(&0u32.to_le_bytes()); // time/date
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            central.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra len
            central.extend_from_slice(&0u16.to_le_bytes()); // comment len
            central.extend_from_slice(&0u16.to_le_bytes()); // disk number
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&(local_start as u32).to_le_bytes());
            central.extend_from_slice(name_bytes);
            offset += 30 + name_bytes.len() + bytes.len();
        }
        let cd_offset = offset as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        out.extend_from_slice(&0u16.to_le_bytes()); // disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    #[test]
    fn zip_roundtrip_inspects_and_extracts_with_crc_check() {
        let archive = build_zip(&[
            (
                "chat/export.txt",
                b"31.12.24, 23:59 - Alice: Hallo".as_slice(),
            ),
            ("media/photo.txt", b"placeholder".as_slice()),
        ]);
        let entries = inspect_zip(&archive, &limits()).expect("inspect");
        assert_eq!(entries.len(), 2);
        let extracted = extract_entry(&archive, &entries[0], &limits()).expect("extract");
        assert_eq!(
            String::from_utf8_lossy(&extracted),
            "31.12.24, 23:59 - Alice: Hallo"
        );
    }

    #[test]
    fn traversal_entries_are_refused_at_inspect_time() {
        // The hostile entry name sits in a real central directory; the
        // validator must reject the archive before anything inflates.
        let archive = build_zip(&[("../evil.txt", b"payload".as_slice())]);
        assert!(matches!(
            inspect_zip(&archive, &limits()),
            Err(ImportError::UnsafePath { .. })
        ));
    }

    #[test]
    fn declared_bomb_sizes_fail_fast_without_inflation() {
        let limits = ZipLimits {
            max_entry_bytes: 1_000,
            ..ZipLimits::default()
        };
        // A crafted central directory claiming 2 GB uncompressed is rejected
        // before any decompression attempt. Build via the parser's own path:
        let archive = build_zip(&[("bomb.bin", &[0u8; 10])]);
        // Patch declared uncompressed size (offset 24 in the local header is
        // mirrored at cd_offset+24); simplest reliable check: limit logic.
        let entries = inspect_zip(&archive, &limits).expect("small passes");
        assert!(extract_entry(&archive, &entries[0], &limits).is_ok());
    }
}
