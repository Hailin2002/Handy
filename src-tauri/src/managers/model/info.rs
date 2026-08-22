//! Public model types: engine/source enums, catalog entries, descriptors,
//! and the language-code helpers shared by discovery and transcription.

use crate::managers::model_capabilities::CapabilityProbe;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum EngineType {
    /// Any GGML/GGUF model loaded through transcribe-cpp (Whisper, Parakeet,
    /// Voxtral, Qwen3-ASR, Nemotron, …). The architecture is auto-detected from
    /// the file, so this one variant covers the whole transcribe-cpp family.
    TranscribeCpp,
    Parakeet,
    Moonshine,
    MoonshineStreaming,
    SenseVoice,
    GigaAM,
    Canary,
    Cohere,
}

/// Where a model comes from and how Handy obtains it — the routing discriminant
/// for downloading and on-disk resolution.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum ModelSource {
    /// Direct HTTP download from a URL (current blob.handy.computer hosting).
    Url {
        url: String,
        /// Expected SHA-256 for integrity verification; `None` skips it.
        sha256: Option<String>,
    },
    /// A file inside a Hugging Face Hub repo, fetched via hf-hub into the shared
    /// HF cache (so other tools reuse it). The file within the repo is
    /// [`ModelInfo::filename`].
    HuggingFace { repo_id: String, revision: String },
    /// Already present on disk — a user-provided custom model, or one discovered
    /// in a shared cache. Nothing to download.
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub source: ModelSource,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
    pub is_directory: bool,
    pub engine_type: EngineType,
    pub accuracy_score: f32,        // 0.0 to 1.0, higher is more accurate
    pub speed_score: f32,           // 0.0 to 1.0, higher is faster
    pub supports_translation: bool, // Whether the model supports translating to English
    pub is_recommended: bool,       // Whether this is the recommended model for new users
    pub supported_languages: Vec<String>, // Languages this model can transcribe
    pub supports_language_selection: bool, // Whether the user can explicitly pick a language
    pub is_custom: bool,            // Whether this is a user-provided custom model
    pub supports_streaming: bool, // Whether this model supports live streaming preview (transcribe-cpp)
    pub supports_language_detection: bool, // Whether the model can auto-detect language (gates the "Auto" option)
}

const CHINESE_LANGUAGE_CODE: &str = "zh";

fn recognition_language(language: &str) -> &str {
    match language {
        "zh-Hans" | "zh-Hant" => CHINESE_LANGUAGE_CODE,
        other => other,
    }
}

/// The base code Handy matches a language *intent* on: a tag's primary subtag,
/// with any BCP-47 region or script suffix dropped (`en-US` → `en`, `zh-CN` →
/// `zh`, `zh-Hant` → `zh`). Bare and three-letter codes (`haw`) pass through
/// unchanged. Lets a bare intent (`en`) match a model that advertises full
/// locales (`en-US`) without discarding the real code the engine needs.
fn base_language(language: &str) -> &str {
    match language.split_once('-') {
        Some((base, _)) => base,
        None => language,
    }
}

pub(super) fn canonicalize_supported_languages(languages: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut canonical = Vec::with_capacity(languages.len());

    for language in languages {
        let language = recognition_language(&language).to_string();
        if seen.insert(language.clone()) {
            canonical.push(language);
        }
    }

    canonical
}

/// One downloadable quantization of a model. Mirrors a `files[]` entry in
/// `catalog.json`, so it deserializes straight from the catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct QuantFile {
    pub filename: String,
    pub quant: String,
    pub size_bytes: u64,
    /// Content sha256 — the trust anchor for downloads from any source (HF or
    /// mirror). `None` only for catalogs predating the field.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Pick the default quant among `files`: the one whose `quant` matches
/// `default_quant`, else the first file. The single source of the "which file do
/// we surface" rule — shared by [`ModelDescriptor::default_file`] and the
/// catalog's id construction so the two can never drift.
pub(crate) fn default_quant_file<'a>(
    files: &'a [QuantFile],
    default_quant: Option<&str>,
) -> Option<&'a QuantFile> {
    files
        .iter()
        .find(|f| Some(f.quant.as_str()) == default_quant)
        .or_else(|| files.first())
}

/// Live, on-disk status — the half of [`ModelInfo`] that isn't part of the
/// static spec. Kept separate so a descriptor stays purely descriptive and
/// status can be recomputed without rebuilding it.
#[derive(Debug, Clone, Default)]
pub struct DiskStatus {
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
}

/// The spec of a bundled catalog model: everything in `catalog.json` normalised
/// into one shape, rendered into the frontend-facing [`ModelInfo`] via
/// [`ModelDescriptor::to_model_info`] by combining it with a [`DiskStatus`].
/// (The catalog is the only producer that routes through this; the legacy table
/// and on-disk scans build `ModelInfo` directly.)
#[derive(Debug, Clone)]
pub struct ModelDescriptor {
    pub id: String,
    pub source: ModelSource,
    pub name: String,
    pub description: String,
    pub engine_type: EngineType,
    pub caps: CapabilityProbe,
    pub files: Vec<QuantFile>,
    pub default_quant: Option<String>,
    pub speed_score: f32,
    pub accuracy_score: f32,
    /// Editorial sort priority across the whole catalog (lower = higher). Drives
    /// list ordering; independent of `recommended`.
    pub recommended_rank: Option<u32>,
    /// Whether this is part of the small curated set shown to new users in
    /// onboarding (and badged "Recommended"). A model can be ranked for ordering
    /// without being in this set.
    pub recommended: bool,
}

impl ModelDescriptor {
    /// The quant we surface for download/size: the declared default, else the
    /// first file.
    pub(super) fn default_file(&self) -> Option<&QuantFile> {
        default_quant_file(&self.files, self.default_quant.as_deref())
    }

    /// Render the frontend-facing [`ModelInfo`] by combining this spec with live
    /// disk `status`.
    pub fn to_model_info(&self, status: &DiskStatus) -> ModelInfo {
        self.render_model_info(self.default_file(), status)
    }

    /// [`ModelInfo`] for one specific quant `file` of this catalog model — how
    /// alternate-quant files found on disk surface with full catalog metadata
    /// instead of as anonymous customs. The default quant keeps the plain
    /// catalog name; any other quant appends it ("Name (Q4_K_M)") so two
    /// quants of one model stay tellable apart, and only the default carries
    /// the Recommended badge. Identity stays `"{repo_id}/{filename}"`, so the
    /// default quant renders identically to the seeded entry.
    pub fn to_model_info_for_file(&self, file: &QuantFile, status: &DiskStatus) -> ModelInfo {
        self.render_model_info(Some(file), status)
    }

    fn render_model_info(&self, file: Option<&QuantFile>, status: &DiskStatus) -> ModelInfo {
        let is_default = match (file, self.default_file()) {
            (Some(f), Some(d)) => f.filename == d.filename,
            _ => true,
        };
        let id = match (&self.source, file) {
            (ModelSource::HuggingFace { repo_id, .. }, Some(f)) => {
                format!("{}/{}", repo_id, f.filename)
            }
            _ => self.id.clone(),
        };
        let name = if is_default {
            self.name.clone()
        } else {
            format!(
                "{} ({})",
                self.name,
                file.map(|f| f.quant.as_str()).unwrap_or("")
            )
        };
        let languages =
            canonicalize_supported_languages(self.caps.languages.clone().unwrap_or_default());
        ModelInfo {
            id,
            name,
            description: self.description.clone(),
            filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
            source: self.source.clone(),
            size_mb: file.map(|f| f.size_bytes / (1024 * 1024)).unwrap_or(0),
            is_downloaded: status.is_downloaded,
            is_downloading: status.is_downloading,
            partial_size: status.partial_size,
            is_directory: false,
            engine_type: self.engine_type.clone(),
            accuracy_score: self.accuracy_score,
            speed_score: self.speed_score,
            supports_translation: self.caps.supports_translation.unwrap_or(false),
            is_recommended: self.recommended && is_default,
            supports_language_selection: languages.len() > 1,
            supported_languages: languages,
            // Catalog models are always HF-sourced downloads, never user-dropped
            // custom files (those bypass the descriptor and set this directly).
            is_custom: false,
            supports_streaming: self.caps.supports_streaming.unwrap_or(false),
            supports_language_detection: self.caps.supports_language_detect.unwrap_or(false),
        }
    }
}

/// Resolve the user's persisted language *intent* (`"auto"` or a language code)
/// into the language a given model will actually use.
///
/// The canonical coercion used on every transcription path: computed at the
/// point of use and **never written back** to settings, so the user's last
/// explicit intent survives switching to an incompatible model and back.
///
/// Matching is base-aware ([`base_language`]) and returns the model's own
/// *concrete* code, so a bare intent (`en`) resolves to the exact string the
/// engine's prompt table expects (`en-US`) for models that advertise full
/// BCP-47 locales. Chinese *script* intents (`zh-Hans`/`zh-Hant`) are the sole
/// exception: they pass through unchanged so the downstream Simplified /
/// Traditional output conversion still fires (the engine path collapses them to
/// a plain Chinese code separately).
pub fn effective_language(
    intent: &str,
    supported_languages: &[String],
    supports_language_detection: bool,
) -> String {
    if supported_languages.is_empty() {
        return intent.to_string();
    }

    if intent != "auto" {
        if let Some(code) = supported_languages
            .iter()
            .find(|language| base_language(language) == base_language(intent))
        {
            if intent == "zh-Hans" || intent == "zh-Hant" {
                return intent.to_string();
            }
            return code.clone();
        }
    }

    if supports_language_detection {
        return "auto".to_string();
    }

    // Model can't auto-detect and the intent isn't usable: fall back to a
    // concrete language (prefer English) so we never hand the engine "auto".
    if let Some(en) = supported_languages
        .iter()
        .find(|language| base_language(language) == "en")
    {
        return en.clone();
    }
    recognition_language(&supported_languages[0]).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
}
