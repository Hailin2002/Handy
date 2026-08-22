//! Model discovery: custom models dropped into the models directory and
//! Hugging Face cache scans, plus automatic selection of a usable default.

use anyhow::Result;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::support::{local_caps, probed_display_name};
use super::*;

impl ModelManager {
    pub(super) fn auto_select_model_if_needed(&self) -> Result<()> {
        let mut settings = get_settings(&self.app_handle);

        // Clear stale selection: selected model is set but doesn't exist
        // in available_models (e.g. deleted custom model file)
        if !settings.selected_model.is_empty() {
            let models = self.available_models.lock().unwrap();
            let exists = models.contains_key(&settings.selected_model);
            drop(models);

            if !exists {
                info!(
                    "Selected model '{}' not found in available models, clearing selection",
                    settings.selected_model
                );
                settings.selected_model = String::new();
                write_settings(&self.app_handle, settings.clone());
            }
        }

        // If onboarding is still pending, do not auto-select just because a
        // compatible model exists on disk or in the shared HF cache. The
        // onboarding model step should present that choice explicitly.
        if !settings.onboarding_completed {
            debug!("Skipping model auto-selection until onboarding is complete");
            return Ok(());
        }

        // If no model is selected, pick the first downloaded one using the same
        // ranked order the UI receives.
        if settings.selected_model.is_empty() {
            if let Some(available_model) = self
                .get_available_models()
                .into_iter()
                .find(|model| model.is_downloaded)
            {
                info!(
                    "Auto-selecting model: {} ({})",
                    available_model.id, available_model.name
                );

                // Update settings with the selected model
                let mut updated_settings = settings;
                updated_settings.selected_model = available_model.id.clone();
                write_settings(&self.app_handle, updated_settings);

                info!("Successfully auto-selected model: {}", available_model.id);
            }
        }

        Ok(())
    }

    /// Discover custom Whisper-family models in the models directory: legacy
    /// GGML `.bin` files and `.gguf` files (both load through transcribe-cpp).
    /// Skips files that match predefined model filenames.
    pub(super) fn discover_custom_transcribe_models(
        models_dir: &Path,
        available_models: &mut HashMap<String, ModelInfo>,
    ) -> Result<()> {
        if !models_dir.exists() {
            return Ok(());
        }

        // Collect filenames of predefined transcribe-cpp file-based models to skip
        let predefined_filenames: HashSet<String> = available_models
            .values()
            .filter(|m| matches!(m.engine_type, EngineType::TranscribeCpp) && !m.is_directory)
            .map(|m| m.filename.clone())
            .collect();

        // Scan models directory for .bin / .gguf files
        for entry in fs::read_dir(models_dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            // Skip directories; the .bin / .gguf extension filter is below.
            if !path.is_file() {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Skip hidden files
            if filename.starts_with('.') {
                continue;
            }

            // Only process Whisper-family model files: legacy GGML `.bin` or
            // GGUF `.gguf` (both load through transcribe-cpp). Anything else —
            // including `.partial` downloads like "model.bin.partial" — is
            // skipped, since it ends in neither extension. The model ID is the
            // filename with its extension removed.
            let (model_id, is_gguf) = if let Some(stem) = filename.strip_suffix(".bin") {
                (stem.to_string(), false)
            } else if let Some(stem) = filename.strip_suffix(".gguf") {
                (stem.to_string(), true)
            } else {
                continue;
            };

            // Skip predefined model files
            if predefined_filenames.contains(&filename) {
                continue;
            }

            // A file matching ANY catalog-listed quant surfaces as that catalog
            // model — full name/description/scores, quant-suffixed name for
            // non-defaults — instead of as an anonymous custom entry. (Default
            // quants never reach here: they're in `predefined_filenames`.)
            if let Some((desc, quant_file)) = crate::catalog::file_in_catalog(&filename, None) {
                let info = desc.to_model_info_for_file(
                    quant_file,
                    &DiskStatus {
                        is_downloaded: true,
                        ..Default::default()
                    },
                );
                if !available_models.contains_key(&info.id) {
                    info!(
                        "Discovered catalog quant in models dir: {} ({})",
                        info.id, filename
                    );
                    available_models.insert(info.id.clone(), info);
                }
                continue;
            }

            // Skip if model ID already exists (shouldn't happen, but be safe)
            if available_models.contains_key(&model_id) {
                continue;
            }

            // Generate display name: replace - and _ with space, capitalize words
            let fallback_display_name = model_id
                .replace(['-', '_'], " ")
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            // Get file size in MB
            let size_mb = match path.metadata() {
                Ok(meta) => meta.len() / (1024 * 1024),
                Err(e) => {
                    warn!("Failed to get metadata for {}: {}", filename, e);
                    0
                }
            };

            // Probe GGUF headers for advertised capabilities so a dropped-in
            // model surfaces streaming / translation / languages just like a
            // Handy-downloaded one. Legacy `.bin` files have no GGUF header, so
            // they stay "unknown" until transcribe-cpp reconciles them at load.
            let probe = if is_gguf {
                GgufHeaderProber.probe_file(&path)
            } else {
                CapabilityProbe::default()
            };
            let caps = local_caps(&probe);
            let display_name = probed_display_name(&probe).unwrap_or(fallback_display_name);

            info!(
                "Discovered custom transcribe-cpp model: {} ({}, {} MB, streaming={})",
                model_id, filename, size_mb, caps.supports_streaming
            );

            available_models.insert(
                model_id.clone(),
                ModelInfo {
                    id: model_id,
                    name: display_name,
                    description: "Not officially supported".to_string(),
                    filename,
                    source: ModelSource::Local, // already on disk; nothing to download
                    size_mb,
                    is_downloaded: true, // Already present on disk
                    is_downloading: false,
                    partial_size: 0,
                    is_directory: false,
                    engine_type: EngineType::TranscribeCpp,
                    accuracy_score: 0.0, // Sentinel: UI hides score bars when both are 0
                    speed_score: 0.0,
                    supports_translation: caps.supports_translation,
                    is_recommended: false,
                    supported_languages: caps.supported_languages,
                    supports_language_selection: caps.supports_language_selection,
                    is_custom: true,
                    supports_streaming: caps.supports_streaming,
                    supports_language_detection: caps.supports_language_detection,
                },
            );
        }

        Ok(())
    }

    /// Discover transcribe-cpp-compatible GGUF models already present in the
    /// shared Hugging Face cache, so models downloaded by Handy (or any other
    /// tool) appear in "Your Models" without re-downloading. Only architectures
    /// transcribe-cpp recognises are surfaced; arbitrary (e.g. LLM) GGUFs that
    /// share the cache are ignored.
    pub(super) fn discover_hf_cache_models(available_models: &mut HashMap<String, ModelInfo>) {
        Self::discover_hf_cache_models_in(Cache::from_env().path(), available_models);
    }

    /// Scan a Hugging Face cache root (`<cache>/models--*`) for GGUF snapshots.
    /// Split from [`Self::discover_hf_cache_models`] so it can be tested against
    /// a synthetic cache directory.
    pub(super) fn discover_hf_cache_models_in(
        cache_root: &Path,
        available_models: &mut HashMap<String, ModelInfo>,
    ) {
        if !cache_root.is_dir() {
            return;
        }

        // Repo+file pairs already represented (e.g. recommended/added models) so
        // the same file is not listed twice.
        let known_hf: HashSet<(String, String)> = available_models
            .values()
            .filter_map(|m| match &m.source {
                ModelSource::HuggingFace { repo_id, .. } => {
                    Some((repo_id.clone(), m.filename.clone()))
                }
                _ => None,
            })
            .collect();

        let prober = GgufHeaderProber;

        let entries = match fs::read_dir(cache_root) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let folder = entry.file_name();
            let folder = folder.to_string_lossy();
            let Some(rest) = folder.strip_prefix("models--") else {
                continue;
            };
            // Reverse hf-hub's `org/name` -> `models--org--name` folder naming.
            let repo_id = rest.replace("--", "/");

            let refs_dir = entry.path().join("refs");
            let Some(revision) = Self::pick_hf_revision(&refs_dir) else {
                continue;
            };
            let Ok(commit) = fs::read_to_string(refs_dir.join(&revision)) else {
                continue;
            };
            let snapshot = entry.path().join("snapshots").join(commit.trim());
            let Ok(files) = fs::read_dir(&snapshot) else {
                continue;
            };

            for file in files.flatten() {
                let fname = file.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".gguf") {
                    continue;
                }
                if known_hf.contains(&(repo_id.clone(), fname.clone())) {
                    continue;
                }
                let model_id = format!("{}/{}", repo_id, fname);
                if available_models.contains_key(&model_id) {
                    continue;
                }

                // Catalog-listed quants (same repo) surface with full catalog
                // metadata — quant-suffixed name for non-defaults — and skip
                // the header probe (the catalog is authoritative for its own
                // models). Everything else keeps the generic probed path.
                if let Some((desc, quant_file)) =
                    crate::catalog::file_in_catalog(&fname, Some(&repo_id))
                {
                    let info = desc.to_model_info_for_file(
                        quant_file,
                        &DiskStatus {
                            is_downloaded: true,
                            ..Default::default()
                        },
                    );
                    info!(
                        "Discovered catalog quant in HF cache: {} ({})",
                        info.id, repo_id
                    );
                    available_models.insert(info.id.clone(), info);
                    continue;
                }

                let path = snapshot.join(&fname);
                let probe = prober.probe_file(&path);
                // Only surface models transcribe-cpp recognises.
                if probe.verdict != Compatibility::Compatible {
                    continue;
                }
                let caps = local_caps(&probe);

                let size_mb = path
                    .metadata()
                    .map(|m| m.len() / (1024 * 1024))
                    .unwrap_or(0);
                let display = probed_display_name(&probe)
                    .unwrap_or_else(|| fname.trim_end_matches(".gguf").to_string());

                info!("Discovered HF cache model: {} ({})", model_id, repo_id);
                available_models.insert(
                    model_id.clone(),
                    ModelInfo {
                        id: model_id,
                        name: display,
                        description: format!("From Hugging Face cache: {}", repo_id),
                        filename: fname,
                        source: ModelSource::HuggingFace {
                            repo_id: repo_id.clone(),
                            revision: revision.clone(),
                        },
                        size_mb,
                        is_downloaded: true,
                        is_downloading: false,
                        partial_size: 0,
                        is_directory: false,
                        engine_type: EngineType::TranscribeCpp,
                        accuracy_score: 0.0,
                        speed_score: 0.0,
                        supports_translation: caps.supports_translation,
                        is_recommended: false,
                        supported_languages: caps.supported_languages,
                        supports_language_selection: caps.supports_language_selection,
                        is_custom: false,
                        supports_streaming: caps.supports_streaming,
                        supports_language_detection: caps.supports_language_detection,
                    },
                );
            }
        }
    }

    /// Pick a cache ref to resolve a snapshot from, preferring `main`.
    pub(super) fn pick_hf_revision(refs_dir: &Path) -> Option<String> {
        if refs_dir.join("main").is_file() {
            return Some("main".to_string());
        }
        fs::read_dir(refs_dir).ok()?.flatten().find_map(|e| {
            if e.path().is_file() {
                e.file_name().to_str().map(str::to_string)
            } else {
                None
            }
        })
    }
}
