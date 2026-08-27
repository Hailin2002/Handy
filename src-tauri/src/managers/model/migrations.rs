//! One-time model migrations and disk-status refreshes.

use anyhow::Result;
use log::{info, warn};
use std::fs;
use tauri::Manager;

use super::support::hf_cached_path;
use super::*;
use crate::sync::MutexExt;

impl ModelManager {
    pub(super) fn migrate_bundled_models(&self) -> Result<()> {
        // Check for bundled models and copy them to user directory
        let bundled_models = ["ggml-small.bin"]; // Add other bundled models here if any

        for filename in &bundled_models {
            let bundled_path = self.app_handle.path().resolve(
                format!("resources/models/{}", filename),
                tauri::path::BaseDirectory::Resource,
            );

            if let Ok(bundled_path) = bundled_path {
                if bundled_path.exists() {
                    let user_path = self.models_dir.join(filename);

                    // Only copy if user doesn't already have the model
                    if !user_path.exists() {
                        info!("Migrating bundled model {} to user directory", filename);
                        fs::copy(&bundled_path, &user_path)?;
                        info!("Successfully migrated {}", filename);
                    }
                }
            }
        }

        Ok(())
    }

    /// Migrate GigaAM from the old single-file format (giga-am-v3.int8.onnx)
    /// to the new directory format (giga-am-v3-int8/model.int8.onnx + vocab.txt).
    /// This was required by the transcribe-rs 0.3.x upgrade.
    pub(super) fn migrate_gigaam_to_directory(&self) -> Result<()> {
        let old_file = self.models_dir.join("giga-am-v3.int8.onnx");
        let new_dir = self.models_dir.join("giga-am-v3-int8");

        if !old_file.exists() || new_dir.exists() {
            return Ok(());
        }

        info!("Migrating GigaAM from single-file to directory format");

        let vocab_path = self
            .app_handle
            .path()
            .resolve(
                "resources/models/gigaam_vocab.txt",
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| anyhow::anyhow!("Failed to resolve GigaAM vocab path: {}", e))?;

        info!(
            "Resolved vocab path: {:?} (exists: {})",
            vocab_path,
            vocab_path.exists()
        );
        info!("Old file: {:?} (exists: {})", old_file, old_file.exists());
        info!("New dir: {:?} (exists: {})", new_dir, new_dir.exists());

        fs::create_dir_all(&new_dir)?;
        fs::rename(&old_file, new_dir.join("model.int8.onnx"))?;
        fs::copy(&vocab_path, new_dir.join("vocab.txt"))?;

        // Clean up old partial file if it exists
        let old_partial = self.models_dir.join("giga-am-v3.int8.onnx.partial");
        if old_partial.exists() {
            let _ = fs::remove_file(&old_partial);
        }

        info!("GigaAM migration complete");
        Ok(())
    }

    pub(super) fn update_download_status(&self) -> Result<()> {
        // Snapshot in-flight download ids before taking the registry lock (the
        // two locks are never nested) so a mid-download entry is never dropped.
        let downloading_ids: HashSet<String> =
            self.cancel_flags.lock_recover().keys().cloned().collect();
        let mut models = self.available_models.lock_recover();
        let mut vanished_alternates: Vec<String> = Vec::new();

        for model in models.values_mut() {
            if let ModelSource::HuggingFace { repo_id, revision } = &model.source {
                // A models-dir copy counts too: mirror-fallback downloads land
                // there, and it makes manual drop-ins of catalog files work.
                let local_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", model.filename));
                model.is_downloaded = hf_cached_path(repo_id, revision, &model.filename).is_some()
                    || local_path.exists();
                model.is_downloading = false;
                model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
                // Alternate-quant entries exist only because their file was
                // discovered on disk — the catalog offers just the default
                // quant, so they are never presented for download. When the
                // file is gone, the entry goes with it.
                if !model.is_downloaded
                    && !downloading_ids.contains(&model.id)
                    && Self::is_catalog_alternate_quant(repo_id, &model.filename)
                {
                    vanished_alternates.push(model.id.clone());
                }
                continue;
            }
            if model.is_directory {
                // For directory-based models, check if the directory exists
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", model.filename));
                let extracting_path = self
                    .models_dir
                    .join(format!("{}.extracting", model.filename));

                // Clean up any leftover .extracting directories from interrupted extractions
                // But only if this model is NOT currently being extracted
                let is_currently_extracting = {
                    let extracting = self.extracting_models.lock_recover();
                    extracting.contains(&model.id)
                };
                if extracting_path.exists() && !is_currently_extracting {
                    warn!("Cleaning up interrupted extraction for model: {}", model.id);
                    let _ = fs::remove_dir_all(&extracting_path);
                }

                model.is_downloaded = model_path.exists() && model_path.is_dir();
                model.is_downloading = false;

                // Get partial file size if it exists (for the .tar.gz being downloaded)
                if partial_path.exists() {
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
                } else {
                    model.partial_size = 0;
                }
            } else {
                // For file-based models (existing logic)
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", model.filename));

                model.is_downloaded = model_path.exists();
                model.is_downloading = false;

                // Get partial file size if it exists
                if partial_path.exists() {
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
                } else {
                    model.partial_size = 0;
                }
            }
        }

        for id in vanished_alternates {
            models.remove(&id);
        }

        Ok(())
    }

    /// Whether `filename` is a catalog-listed quant of `repo_id` other than
    /// the default — the only quant the catalog seeds and offers for download.
    pub(super) fn is_catalog_alternate_quant(repo_id: &str, filename: &str) -> bool {
        crate::catalog::file_in_catalog(filename, Some(repo_id)).is_some_and(|(desc, file)| {
            desc.default_file()
                .is_some_and(|d| d.filename != file.filename)
        })
    }

    /// Remove a single file from the shared HF cache: the snapshot pointer for
    /// the resolved revision and, when the pointer is a symlink, the blob it
    /// points to. Everything else in the repo (other quants, refs) is left
    /// untouched. Returns whether anything was removed.
    pub(super) fn delete_hf_cache_file(repo_id: &str, revision: &str, filename: &str) -> bool {
        let Some(pointer) = hf_cached_path(repo_id, revision, filename) else {
            return false;
        };
        // Resolve the blob before the pointer goes away. On Windows the
        // pointer may be a plain file (hf-hub's symlink fallback renames the
        // blob into the snapshot), in which case there is no separate blob.
        let is_symlink = fs::symlink_metadata(&pointer)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            if let Ok(blob) = fs::canonicalize(&pointer) {
                info!("Deleting HF cache blob at: {:?}", blob);
                let _ = fs::remove_file(&blob);
            }
        }
        info!("Deleting HF cache file at: {:?}", pointer);
        fs::remove_file(&pointer).is_ok()
    }
}
