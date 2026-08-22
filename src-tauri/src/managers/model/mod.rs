use super::model_capabilities::{
    CapabilityProbe, CapabilityProber, Compatibility, GgufHeaderProber,
};
use crate::settings::{get_settings, write_settings};
use anyhow::Result;
use hf_hub::api::tokio::CancellationToken;
use hf_hub::Cache;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;

mod discovery;
mod download;
pub mod downloads;
mod info;
mod migrations;
pub(crate) mod queries;
mod support;

pub use info::*;

pub struct ModelManager {
    app_handle: AppHandle,
    models_dir: PathBuf,
    available_models: Mutex<HashMap<String, ModelInfo>>,
    cancel_flags: Arc<Mutex<HashMap<String, CancellationToken>>>,
    extracting_models: Arc<Mutex<HashSet<String>>>,
    /// Single-flight guard for [`Self::rescan_local_models`] so concurrent
    /// refresh requests coalesce instead of scanning the disk in parallel.
    is_rescanning: Arc<AtomicBool>,
}

impl ModelManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create models directory in app data
        let models_dir = crate::portable::app_data_dir(app_handle)
            .map_err(|e| anyhow::anyhow!("Failed to get app data dir: {}", e))?
            .join("models");

        if !models_dir.exists() {
            fs::create_dir_all(&models_dir)?;
        }

        let mut available_models = HashMap::new();

        // Whisper supported languages (99 languages from tokenizer)
        let whisper_languages: Vec<String> = vec![
            "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar",
            "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu",
            "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa",
            "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn",
            "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc",
            "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn",
            "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw",
            "su", "yue",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "small".to_string(),
            ModelInfo {
                id: "small".to_string(),
                name: "Whisper Small".to_string(),
                description: "Fast and fairly accurate.".to_string(),
                filename: "ggml-small.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/ggml-small.bin".to_string(),
                    sha256: Some(
                        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
                            .to_string(),
                    ),
                },
                size_mb: 465,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.60,
                speed_score: 0.85,
                supports_translation: true,
                is_recommended: false,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // Add downloadable models
        available_models.insert(
            "medium".to_string(),
            ModelInfo {
                id: "medium".to_string(),
                name: "Whisper Medium".to_string(),
                description: "Good accuracy, medium speed".to_string(),
                filename: "whisper-medium-q4_1.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/whisper-medium-q4_1.bin".to_string(),
                    sha256: Some(
                        "79283fc1f9fe12ca3248543fbd54b73292164d8df5a16e095e2bceeaaabddf57"
                            .to_string(),
                    ),
                },
                size_mb: 469,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.75,
                speed_score: 0.60,
                supports_translation: true,
                is_recommended: false,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "turbo".to_string(),
            ModelInfo {
                id: "turbo".to_string(),
                name: "Whisper Turbo".to_string(),
                description: "Balanced accuracy and speed.".to_string(),
                filename: "ggml-large-v3-turbo.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/ggml-large-v3-turbo.bin".to_string(),
                    sha256: Some(
                        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"
                            .to_string(),
                    ),
                },
                size_mb: 1549,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.80,
                speed_score: 0.40,
                supports_translation: false, // Turbo doesn't support translation
                is_recommended: false,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "large".to_string(),
            ModelInfo {
                id: "large".to_string(),
                name: "Whisper Large".to_string(),
                description: "Good accuracy, but slow.".to_string(),
                filename: "ggml-large-v3-q5_0.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/ggml-large-v3-q5_0.bin".to_string(),
                    sha256: Some(
                        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1"
                            .to_string(),
                    ),
                },
                size_mb: 1031,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.85,
                speed_score: 0.30,
                supports_translation: true,
                is_recommended: false,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "breeze-asr".to_string(),
            ModelInfo {
                id: "breeze-asr".to_string(),
                name: "Breeze ASR".to_string(),
                description: "Optimized for Taiwanese Mandarin. Code-switching support."
                    .to_string(),
                filename: "breeze-asr-q5_k.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/breeze-asr-q5_k.bin".to_string(),
                    sha256: Some(
                        "8efbf0ce8a3f50fe332b7617da787fb81354b358c288b008d3bdef8359df64c6"
                            .to_string(),
                    ),
                },
                size_mb: 1030,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.85,
                speed_score: 0.35,
                supports_translation: false,
                is_recommended: false,
                supported_languages: whisper_languages,
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // Add NVIDIA Parakeet models (directory-based)
        available_models.insert(
            "parakeet-tdt-0.6b-v2".to_string(),
            ModelInfo {
                id: "parakeet-tdt-0.6b-v2".to_string(),
                name: "Parakeet V2".to_string(),
                description: "English only. The best model for English speakers.".to_string(),
                filename: "parakeet-tdt-0.6b-v2-int8".to_string(), // Directory name
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/parakeet-v2-int8.tar.gz".to_string(),
                    sha256: Some(
                        "ac9b9429984dd565b25097337a887bb7f0f8ac393573661c651f0e7d31563991"
                            .to_string(),
                    ),
                },
                size_mb: 451,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Parakeet,
                accuracy_score: 0.85,
                speed_score: 0.85,
                supports_translation: false,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // Parakeet V3 supported languages (25 EU languages + Russian/Ukrainian):
        // bg, hr, cs, da, nl, en, et, fi, fr, de, el, hu, it, lv, lt, mt, pl, pt, ro, sk, sl, es, sv, ru, uk
        let parakeet_v3_languages: Vec<String> = vec![
            "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv",
            "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "parakeet-tdt-0.6b-v3".to_string(),
            ModelInfo {
                id: "parakeet-tdt-0.6b-v3".to_string(),
                name: "Parakeet V3".to_string(),
                description: "Fast and accurate. Supports 25 European languages.".to_string(),
                filename: "parakeet-tdt-0.6b-v3-int8".to_string(), // Directory name
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/parakeet-v3-int8.tar.gz".to_string(),
                    sha256: Some(
                        "43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77"
                            .to_string(),
                    ),
                },
                size_mb: 456,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Parakeet,
                accuracy_score: 0.80,
                speed_score: 0.85,
                supports_translation: false,
                is_recommended: true,
                supported_languages: parakeet_v3_languages,
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "moonshine-base".to_string(),
            ModelInfo {
                id: "moonshine-base".to_string(),
                name: "Moonshine Base".to_string(),
                description: "Very fast, English only. Handles accents well.".to_string(),
                filename: "moonshine-base".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/moonshine-base.tar.gz".to_string(),
                    sha256: Some(
                        "04bf6ab012cfceebd4ac7cf88c1b31d027bbdd3cd704649b692e2e935236b7e8"
                            .to_string(),
                    ),
                },
                size_mb: 55,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Moonshine,
                accuracy_score: 0.70,
                speed_score: 0.90,
                supports_translation: false,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "moonshine-tiny-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-tiny-streaming-en".to_string(),
                name: "Moonshine V2 Tiny".to_string(),
                description: "Ultra-fast, English only".to_string(),
                filename: "moonshine-tiny-streaming-en".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/moonshine-tiny-streaming-en.tar.gz"
                        .to_string(),
                    sha256: Some(
                        "465addcfca9e86117415677dfdc98b21edc53537210333a3ecdb58509a80abaf"
                            .to_string(),
                    ),
                },
                size_mb: 31,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.55,
                speed_score: 0.95,
                supports_translation: false,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "moonshine-small-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-small-streaming-en".to_string(),
                name: "Moonshine V2 Small".to_string(),
                description: "Fast, English only. Good balance of speed and accuracy.".to_string(),
                filename: "moonshine-small-streaming-en".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/moonshine-small-streaming-en.tar.gz"
                        .to_string(),
                    sha256: Some(
                        "dbb3e1c1832bd88a4ac712f7449a136cc2c9a18c5fe33a12ed1b7cb1cfe9cdd5"
                            .to_string(),
                    ),
                },
                size_mb: 99,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.65,
                speed_score: 0.90,
                supports_translation: false,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        available_models.insert(
            "moonshine-medium-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-medium-streaming-en".to_string(),
                name: "Moonshine V2 Medium".to_string(),
                description: "English only. High quality.".to_string(),
                filename: "moonshine-medium-streaming-en".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/moonshine-medium-streaming-en.tar.gz"
                        .to_string(),
                    sha256: Some(
                        "07a66f3bff1c77e75a2f637e5a263928a08baae3c29c4c053fc968a9a9373d13"
                            .to_string(),
                    ),
                },
                size_mb: 192,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.75,
                speed_score: 0.80,
                supports_translation: false,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // SenseVoice supported languages
        let sense_voice_languages: Vec<String> = vec!["zh", "en", "yue", "ja", "ko"]
            .into_iter()
            .map(String::from)
            .collect();

        available_models.insert(
            "sense-voice-int8".to_string(),
            ModelInfo {
                id: "sense-voice-int8".to_string(),
                name: "SenseVoice".to_string(),
                description: "Very fast. Chinese, English, Japanese, Korean, Cantonese."
                    .to_string(),
                filename: "sense-voice-int8".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/sense-voice-int8.tar.gz".to_string(),
                    sha256: Some(
                        "171d611fe5d353a50bbb741b6f3ef42559b1565685684e9aa888ef563ba3e8a4"
                            .to_string(),
                    ),
                },
                size_mb: 152,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::SenseVoice,
                accuracy_score: 0.65,
                speed_score: 0.95,
                supports_translation: false,
                is_recommended: false,
                supported_languages: sense_voice_languages,
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // GigaAM v3 supported languages
        let gigaam_languages: Vec<String> = vec!["ru"].into_iter().map(String::from).collect();

        available_models.insert(
            "gigaam-v3-e2e-ctc".to_string(),
            ModelInfo {
                id: "gigaam-v3-e2e-ctc".to_string(),
                name: "GigaAM v3".to_string(),
                description: "Russian speech recognition. Fast and accurate.".to_string(),
                filename: "giga-am-v3-int8".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/giga-am-v3-int8.tar.gz".to_string(),
                    sha256: Some(
                        "d872462268430db140b69b72e0fc4b787b194c1dbe51b58de39444d55b6da45b"
                            .to_string(),
                    ),
                },
                size_mb: 151,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::GigaAM,
                accuracy_score: 0.85,
                speed_score: 0.75,
                supports_translation: false,
                is_recommended: false,
                supported_languages: gigaam_languages,
                supports_language_selection: false,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // Canary 180m Flash supported languages (4 languages)
        let canary_flash_languages: Vec<String> = vec!["en", "de", "es", "fr"]
            .into_iter()
            .map(String::from)
            .collect();

        available_models.insert(
            "canary-180m-flash".to_string(),
            ModelInfo {
                id: "canary-180m-flash".to_string(),
                name: "Canary 180M Flash".to_string(),
                description: "Very fast. English, German, Spanish, French. Supports translation."
                    .to_string(),
                filename: "canary-180m-flash".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/canary-180m-flash.tar.gz".to_string(),
                    sha256: Some(
                        "6d9cfca6118b296e196eaedc1c8fa9788305a7b0f1feafdb6dc91932ab6e53f7"
                            .to_string(),
                    ),
                },
                size_mb: 146,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Canary,
                accuracy_score: 0.75,
                speed_score: 0.85,
                supports_translation: true,
                is_recommended: false,
                supported_languages: canary_flash_languages,
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                // Canary (NeMo) requires an explicit source language — no auto-detect.
                supports_language_detection: false,
            },
        );

        // Canary 1B v2 supported languages (25 EU languages)
        let canary_1b_languages: Vec<String> = vec![
            "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv",
            "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "canary-1b-v2".to_string(),
            ModelInfo {
                id: "canary-1b-v2".to_string(),
                name: "Canary 1B v2".to_string(),
                description: "Accurate multilingual. 25 European languages. Supports translation."
                    .to_string(),
                filename: "canary-1b-v2".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/canary-1b-v2.tar.gz".to_string(),
                    sha256: Some(
                        "02305b2a25f9cf3e7deaffa7f94df00efa44f442cd55c101c2cb9c000f904666"
                            .to_string(),
                    ),
                },
                size_mb: 691,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Canary,
                accuracy_score: 0.85,
                speed_score: 0.70,
                supports_translation: true,
                is_recommended: false,
                supported_languages: canary_1b_languages,
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                // Canary (NeMo) requires an explicit source language — no auto-detect.
                supports_language_detection: false,
            },
        );

        let cohere_languages: Vec<String> = vec![
            "en", "fr", "de", "it", "es", "pt", "el", "nl", "pl", "zh", "ja", "ko", "vi", "ar",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "cohere-int8".to_string(),
            ModelInfo {
                id: "cohere-int8".to_string(),
                name: "Cohere".to_string(),
                description: "A large, slower, but very accurate multilingual model.".to_string(),
                filename: "cohere-int8".to_string(),
                source: ModelSource::Url {
                    url: "https://blob.handy.computer/cohere-int8.tar.gz".to_string(),
                    sha256: Some(
                        "ea2257d52434f3644574f187dcdcf666e302cd11b92866116ab8e14cd9c887f0"
                            .to_string(),
                    ),
                },
                size_mb: 1708,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Cohere,
                accuracy_score: 0.90,
                speed_score: 0.60,
                supports_translation: false,
                is_recommended: false,
                supported_languages: cohere_languages,
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                supports_language_detection: true,
            },
        );

        // Seed the bundled offline catalog before the on-disk scans, so a model
        // already in the HF cache dedups onto its richer catalog entry (the scans
        // only insert ids not already present) instead of showing as a bare cache
        // find. Additive — see `seed_catalog_models`.
        Self::seed_catalog_models(&mut available_models);

        // Auto-discover custom transcribe-cpp models (.bin / .gguf) in the models directory
        if let Err(e) = Self::discover_custom_transcribe_models(&models_dir, &mut available_models)
        {
            warn!("Failed to discover custom models: {}", e);
        }

        // Auto-discover transcribe-cpp GGUF models already in the shared HF cache.
        Self::discover_hf_cache_models(&mut available_models);

        let manager = Self {
            app_handle: app_handle.clone(),
            models_dir,
            available_models: Mutex::new(available_models),
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
            extracting_models: Arc::new(Mutex::new(HashSet::new())),
            is_rescanning: Arc::new(AtomicBool::new(false)),
        };

        // Migrate any bundled models to user directory
        manager.migrate_bundled_models()?;

        // Migrate GigaAM from single-file to directory format
        manager.migrate_gigaam_to_directory()?;

        // Check which models are already downloaded
        manager.update_download_status()?;

        // Auto-select a model if none is currently selected
        manager.auto_select_model_if_needed()?;

        Ok(manager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_effective_language_accepts_chinese_script_intent_for_zh_capability() {
        let languages = vec!["zh".to_string()];

        assert_eq!(effective_language("zh-Hans", &languages, false), "zh-Hans");
        assert_eq!(effective_language("zh-Hant", &languages, false), "zh-Hant");
    }

    #[test]
    fn test_effective_language_falls_back_to_canonical_chinese() {
        let languages = vec!["zh-Hant".to_string()];

        assert_eq!(effective_language("auto", &languages, false), "zh");
    }

    #[test]
    fn test_effective_language_resolves_bare_intent_to_concrete_locale() {
        // A model advertising full BCP-47 locales (e.g. Nemotron Streaming):
        // a bare intent must resolve to the exact code the engine expects, not
        // be handed back as the bare form the prompt table may not contain.
        let languages = vec![
            "en-US".to_string(),
            "en-GB".to_string(),
            "es-ES".to_string(),
            "zh-CN".to_string(),
            "ja-JP".to_string(),
        ];

        assert_eq!(effective_language("en", &languages, true), "en-US");
        assert_eq!(effective_language("es", &languages, true), "es-ES");
        // `zh`/`ja` have no bare entry in this model's table; resolve to locale.
        assert_eq!(effective_language("zh", &languages, true), "zh-CN");
        assert_eq!(effective_language("ja", &languages, true), "ja-JP");
        // An unsupported intent still auto-detects when the model can.
        assert_eq!(effective_language("fr", &languages, true), "auto");
    }

    #[test]
    fn test_effective_language_preserves_chinese_script_intent_for_locale_model() {
        // Script intents survive so Simplified/Traditional output conversion
        // still fires, even when the model advertises a regioned Chinese code.
        let languages = vec!["en-US".to_string(), "zh-CN".to_string()];

        assert_eq!(effective_language("zh-Hans", &languages, true), "zh-Hans");
        assert_eq!(effective_language("zh-Hant", &languages, true), "zh-Hant");
    }

    #[test]
    fn test_canonicalize_supported_languages_collapses_chinese_scripts() {
        let languages = canonicalize_supported_languages(
            vec!["en", "zh", "zh-Hans", "zh-Hant", "yue"]
                .into_iter()
                .map(String::from)
                .collect(),
        );

        assert_eq!(languages, vec!["en", "zh", "yue"]);
    }

    fn build_test_gguf_string_metadata(kvs: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for (key, value) in kvs {
            push_gguf_str(&mut out, key);
            out.extend_from_slice(&8u32.to_le_bytes()); // GGUF string value type.
            push_gguf_str(&mut out, value);
        }
        out
    }

    #[test]
    fn test_discover_custom_transcribe_models() {
        let temp_dir = TempDir::new().unwrap();
        let models_dir = temp_dir.path().to_path_buf();

        // Create test .bin files
        let mut custom_file = File::create(models_dir.join("my-custom-model.bin")).unwrap();
        custom_file.write_all(b"fake model data").unwrap();

        let mut another_file = File::create(models_dir.join("whisper_medical_v2.bin")).unwrap();
        another_file.write_all(b"another fake model").unwrap();

        // Custom GGUF model (also supported by transcribe-cpp)
        let mut gguf_file = File::create(models_dir.join("my-gguf-model.gguf")).unwrap();
        gguf_file
            .write_all(&build_test_gguf_string_metadata(&[(
                "general.name",
                "Friendly GGUF Name",
            )]))
            .unwrap();

        // Create files that should be ignored
        File::create(models_dir.join(".hidden-model.bin")).unwrap(); // Hidden file
        File::create(models_dir.join("readme.txt")).unwrap(); // Non-model file
        File::create(models_dir.join("ggml-small.bin")).unwrap(); // Predefined filename
        File::create(models_dir.join("download.bin.partial")).unwrap(); // Partial download
        fs::create_dir(models_dir.join("some-directory.bin")).unwrap(); // Directory

        // Set up available_models with a predefined Whisper model
        let mut models = HashMap::new();
        models.insert(
            "small".to_string(),
            ModelInfo {
                id: "small".to_string(),
                name: "Whisper Small".to_string(),
                description: "Test".to_string(),
                filename: "ggml-small.bin".to_string(),
                source: ModelSource::Url {
                    url: "https://example.com".to_string(),
                    sha256: None,
                },
                size_mb: 100,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::TranscribeCpp,
                accuracy_score: 0.5,
                speed_score: 0.5,
                supports_translation: true,
                is_recommended: false,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: true,
                is_custom: false,
                supports_streaming: false,
                // Legacy entry: preserve the historical "Auto offered" behavior.
                // (Catalog GGUFs and on-disk probes derive this from metadata.)
                supports_language_detection: true,
            },
        );

        // Discover custom models
        ModelManager::discover_custom_transcribe_models(&models_dir, &mut models).unwrap();

        // Should have discovered 2 custom models (my-custom-model and whisper_medical_v2)
        assert!(models.contains_key("my-custom-model"));
        assert!(models.contains_key("whisper_medical_v2"));

        // Verify custom model properties
        let custom = models.get("my-custom-model").unwrap();
        assert_eq!(custom.name, "My Custom Model");
        assert_eq!(custom.filename, "my-custom-model.bin");
        assert!(matches!(custom.source, ModelSource::Local)); // Custom models have no remote source
        assert!(custom.is_downloaded);
        assert!(custom.is_custom);
        assert_eq!(custom.accuracy_score, 0.0);
        assert_eq!(custom.speed_score, 0.0);
        assert!(custom.supported_languages.is_empty());

        // Verify underscore handling
        let medical = models.get("whisper_medical_v2").unwrap();
        assert_eq!(medical.name, "Whisper Medical V2");

        // Verify .gguf models are discovered too (extension stripped for the id)
        assert!(models.contains_key("my-gguf-model"));
        let gguf = models.get("my-gguf-model").unwrap();
        assert_eq!(gguf.filename, "my-gguf-model.gguf");
        assert_eq!(gguf.name, "Friendly GGUF Name");
        assert!(gguf.is_custom);
        assert!(matches!(gguf.engine_type, EngineType::TranscribeCpp));

        // Should NOT have discovered hidden, non-model, predefined, partial, or directories
        assert!(!models.contains_key(".hidden-model"));
        assert!(!models.contains_key("readme"));
        assert!(!models.contains_key("download.bin"));
        assert!(!models.contains_key("some-directory"));
    }

    #[test]
    fn test_discover_custom_models_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let models_dir = temp_dir.path().to_path_buf();

        let mut models = HashMap::new();
        let count_before = models.len();

        ModelManager::discover_custom_transcribe_models(&models_dir, &mut models).unwrap();

        // No new models should be added
        assert_eq!(models.len(), count_before);
    }

    #[test]
    fn test_discover_custom_models_nonexistent_dir() {
        let models_dir = PathBuf::from("/nonexistent/path/that/does/not/exist");

        let mut models = HashMap::new();
        let count_before = models.len();

        // Should not error, just return Ok
        let result = ModelManager::discover_custom_transcribe_models(&models_dir, &mut models);
        assert!(result.is_ok());
        assert_eq!(models.len(), count_before);
    }

    #[test]
    fn test_catalog_quant_rendering() {
        let desc = ModelDescriptor {
            id: "org/repo/model-Q8_0.gguf".to_string(),
            source: ModelSource::HuggingFace {
                repo_id: "org/repo".to_string(),
                revision: "main".to_string(),
            },
            name: "Model".to_string(),
            description: "desc".to_string(),
            engine_type: EngineType::TranscribeCpp,
            caps: CapabilityProbe::default(),
            files: vec![
                QuantFile {
                    filename: "model-Q4_K_M.gguf".to_string(),
                    quant: "Q4_K_M".to_string(),
                    size_bytes: 1,
                    sha256: None,
                },
                QuantFile {
                    filename: "model-Q8_0.gguf".to_string(),
                    quant: "Q8_0".to_string(),
                    size_bytes: 2,
                    sha256: None,
                },
            ],
            default_quant: Some("Q8_0".to_string()),
            speed_score: 0.5,
            accuracy_score: 0.5,
            recommended_rank: None,
            recommended: true,
        };
        let status = DiskStatus::default();

        // Default quant: plain name, badge intact.
        let default_info = desc.to_model_info(&status);
        assert_eq!(default_info.name, "Model");
        assert!(default_info.is_recommended);

        // Alternate quant: suffixed name, own id, no badge.
        let alt_info = desc.to_model_info_for_file(&desc.files[0], &status);
        assert_eq!(alt_info.id, "org/repo/model-Q4_K_M.gguf");
        assert_eq!(alt_info.name, "Model (Q4_K_M)");
        assert_eq!(alt_info.filename, "model-Q4_K_M.gguf");
        assert!(!alt_info.is_recommended);
        assert!(!alt_info.is_custom);

        // The default rendered through the per-file path matches to_model_info,
        // so seeded entries and discovered defaults can never diverge.
        let same = desc.to_model_info_for_file(&desc.files[1], &status);
        assert_eq!(same.id, default_info.id);
        assert_eq!(same.name, default_info.name);
        assert_eq!(same.is_recommended, default_info.is_recommended);
    }

    #[test]
    fn test_discover_catalog_alternate_quant_in_models_dir() {
        // A real catalog model with more than one quant.
        let desc = crate::catalog::CATALOG
            .iter()
            .find(|d| d.files.len() > 1)
            .expect("catalog has multi-quant models");
        let default_filename = default_quant_file(&desc.files, desc.default_quant.as_deref())
            .unwrap()
            .filename
            .clone();
        let alt = desc
            .files
            .iter()
            .find(|f| f.filename != default_filename)
            .unwrap();

        let temp_dir = TempDir::new().unwrap();
        // Content is never probed for catalog-matched files, so empty files do.
        fs::write(temp_dir.path().join(&alt.filename), b"").unwrap();
        fs::write(temp_dir.path().join(&default_filename), b"").unwrap();

        let mut models = HashMap::new();
        ModelManager::seed_catalog_models(&mut models);
        let seeded = models.len();
        ModelManager::discover_custom_transcribe_models(temp_dir.path(), &mut models).unwrap();

        // The alternate quant surfaces as a catalog-grade HF entry…
        let ModelSource::HuggingFace { repo_id, .. } = &desc.source else {
            panic!("catalog descriptors are HF-sourced");
        };
        let alt_id = format!("{}/{}", repo_id, alt.filename);
        let info = models.get(&alt_id).expect("alternate quant discovered");
        assert_eq!(info.name, format!("{} ({})", desc.name, alt.quant));
        assert_eq!(info.description, desc.description);
        assert!(info.is_downloaded);
        assert!(!info.is_custom);
        assert!(matches!(info.source, ModelSource::HuggingFace { .. }));

        // …while the default-quant file dedups onto its seeded entry: exactly
        // one new id, and no filename-stem custom entries for either file.
        assert_eq!(models.len(), seeded + 1);
        assert!(!models.contains_key(alt.filename.trim_end_matches(".gguf")));
        assert!(!models.contains_key(default_filename.trim_end_matches(".gguf")));
    }

    fn push_gguf_str(out: &mut Vec<u8>, val: &str) {
        out.extend_from_slice(&(val.len() as u64).to_le_bytes());
        out.extend_from_slice(val.as_bytes());
    }

    fn write_synthetic_gguf(path: &Path, arch: &str, languages: &[&str]) {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // magic "GGUF"
        out.extend_from_slice(&3u32.to_le_bytes()); // version
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&2u64.to_le_bytes()); // kv_count
                                                    // general.architecture : string
        push_gguf_str(&mut out, "general.architecture");
        out.extend_from_slice(&8u32.to_le_bytes()); // STRING
        push_gguf_str(&mut out, arch);
        // general.languages : array<string>
        push_gguf_str(&mut out, "general.languages");
        out.extend_from_slice(&9u32.to_le_bytes()); // ARRAY
        out.extend_from_slice(&8u32.to_le_bytes()); // elem STRING
        out.extend_from_slice(&(languages.len() as u64).to_le_bytes());
        for l in languages {
            push_gguf_str(&mut out, l);
        }
        fs::write(path, out).unwrap();
    }

    #[test]
    fn test_discover_hf_cache_models_filters_by_arch() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // ASR repo: a whisper gguf -> should be discovered.
        let repo = root.join("models--handy-computer--whisper-test");
        fs::create_dir_all(repo.join("snapshots").join("abc123")).unwrap();
        fs::create_dir_all(repo.join("refs")).unwrap();
        fs::write(repo.join("refs").join("main"), "abc123").unwrap();
        write_synthetic_gguf(
            &repo
                .join("snapshots")
                .join("abc123")
                .join("whisper-q8.gguf"),
            "whisper",
            &["en", "de"],
        );

        // Non-ASR (llama) gguf -> must be ignored.
        let repo2 = root.join("models--someone--llama-7b");
        fs::create_dir_all(repo2.join("snapshots").join("def456")).unwrap();
        fs::create_dir_all(repo2.join("refs")).unwrap();
        fs::write(repo2.join("refs").join("main"), "def456").unwrap();
        write_synthetic_gguf(
            &repo2.join("snapshots").join("def456").join("llama-q8.gguf"),
            "llama",
            &[],
        );

        let mut models = HashMap::new();
        ModelManager::discover_hf_cache_models_in(root, &mut models);

        let id = "handy-computer/whisper-test/whisper-q8.gguf";
        let m = models.get(id).expect("whisper gguf should be discovered");
        assert!(m.is_downloaded);
        assert!(
            matches!(&m.source, ModelSource::HuggingFace { repo_id, revision }
            if repo_id == "handy-computer/whisper-test" && revision == "main")
        );
        assert_eq!(
            m.supported_languages,
            vec!["en".to_string(), "de".to_string()]
        );
        assert!(
            !models.contains_key("someone/llama-7b/llama-q8.gguf"),
            "non-ASR gguf must be ignored"
        );
    }
}
