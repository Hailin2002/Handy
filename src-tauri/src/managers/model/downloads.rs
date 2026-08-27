//! Model download, deletion, path resolution, and cancellation.

use anyhow::Result;
use flate2::read::GzDecoder;
use hf_hub::api::tokio::{ApiBuilder, CancellationToken};
use hf_hub::{Repo, RepoType};
use log::{debug, error, info, warn};
use std::fs::{self, File};
use std::path::PathBuf;
use tar::Archive;
use tauri::Emitter;

use super::download::{HttpDownloadOutcome, DOWNLOAD_STALL_TIMEOUT};
use super::support::{hf_cached_path, DownloadCleanup, HfDownloadProgress};
use super::*;
use crate::sync::MutexExt;

/// Download a Hugging Face-sourced model into the shared HF cache via
/// hf-hub, reporting progress through the same `model-download-progress`
/// event the URL path uses. Uses hf-hub's stock cache, but deliberately
/// disables authentication because every catalog repository is public.
impl ModelManager {
    pub(super) async fn download_hf_model(
        &self,
        model_info: &ModelInfo,
        repo_id: String,
        revision: String,
    ) -> Result<()> {
        let model_id = model_info.id.clone();
        let filename = model_info.filename.clone();

        // Already in the shared cache (possibly from another tool), or dropped
        // into the models dir (mirror fallback / manual install)? Done.
        if hf_cached_path(&repo_id, &revision, &filename).is_some()
            || self.models_dir.join(&filename).exists()
        {
            self.update_download_status()?;
            let _ = self.app_handle.emit("model-download-complete", &model_id);
            return Ok(());
        }

        // Mark downloading; the guard resets the flag on any error path.
        {
            let mut models = self.available_models.lock_recover();
            if let Some(model) = models.get_mut(&model_id) {
                model.is_downloading = true;
            }
        }

        // Register a cancellation token so `cancel_download` can abort this
        // transfer promptly. The guard removes it on every exit path.
        let cancel_token = CancellationToken::new();
        {
            let mut flags = self.cancel_flags.lock_recover();
            flags.insert(model_id.clone(), cancel_token.clone());
        }

        let mut cleanup = DownloadCleanup {
            available_models: &self.available_models,
            cancel_flags: &self.cancel_flags,
            model_id: model_id.clone(),
            disarmed: false,
        };

        info!(
            "Downloading HF model {} from {}@{} ({})",
            model_id, repo_id, revision, filename
        );

        // hf-hub has no working internal retry (its retry knobs are hardcoded
        // to zero), so a single transient fault — dropped connection, a 429
        // from the resolve endpoint, a CDN blip — would otherwise fail the
        // whole download. Each attempt resumes from the `.sync.part`
        // committed-offset marker, so a retry only re-fetches what the failed
        // attempt hadn't finished.
        // Start moderately parallel for normal-network throughput, then stay
        // sequential after the first failure. Eight simultaneous connections
        // were all reset on an affected network in #1579, while one stream
        // succeeded; four is a less aggressive fast path, and every retry uses
        // the known-compatible request pattern.
        const ATTEMPT_STREAMS: [usize; 4] = [4, 1, 1, 1];
        let mut attempt: usize = 1;
        let hf_error = loop {
            let stream_count = ATTEMPT_STREAMS[attempt - 1];
            info!(
                "HF download attempt {}/{} for {} using {} concurrent stream(s)",
                attempt,
                ATTEMPT_STREAMS.len(),
                model_id,
                stream_count
            );

            // Fresh client per attempt so a wedged connection from the previous
            // try can't poison the retry.
            let api = ApiBuilder::from_env()
                // Ignore cached and environment-provided credentials. A stale token
                // can make otherwise-public downloads fail authentication.
                .with_token(None)
                .with_progress(false)
                .with_max_files(stream_count)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to init Hugging Face API: {}", e))?;
            let repo = api.repo(Repo::with_revision(
                repo_id.clone(),
                RepoType::Model,
                revision.clone(),
            ));
            let progress = HfDownloadProgress::new(self.app_handle.clone(), model_id.clone());

            // hf-hub has no internal timeouts, so a wedged connection would
            // otherwise hang this attempt forever and neither the retry loop
            // nor the mirror fallback would ever fire. The watchdog cancels a
            // per-attempt child token when progress goes stale; a user cancel
            // on the parent propagates through the same child.
            let attempt_token = cancel_token.child_token();
            let watchdog = tokio::spawn({
                let probe = progress.clone();
                let attempt_token = attempt_token.clone();
                async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        if attempt_token.is_cancelled() {
                            break;
                        }
                        if probe.last_activity().elapsed() > DOWNLOAD_STALL_TIMEOUT {
                            attempt_token.cancel();
                            break;
                        }
                    }
                }
            });
            // hf-hub only observes its token inside the chunk loop — the
            // metadata/resolve request and cache lock run before it, so a hang
            // there would ignore the cancel entirely. Race the whole future
            // against the token: on cancel, grant a short grace so an attempt
            // that IS in the chunk loop can unwind gracefully (committing the
            // `.sync.part` resume offset), then drop the future outright,
            // which aborts whatever request it was wedged in.
            let mut download = std::pin::pin!(repo.download_with_progress_cancellable(
                &filename,
                progress,
                attempt_token.clone()
            ));
            let result = tokio::select! {
                r = &mut download => r,
                _ = attempt_token.cancelled() => {
                    match tokio::time::timeout(Duration::from_secs(5), &mut download).await {
                        Ok(r) => r,
                        Err(_) => Err(hf_hub::api::tokio::ApiError::Cancelled),
                    }
                }
            };
            watchdog.abort();

            match result {
                Ok(_) => break None,
                Err(hf_hub::api::tokio::ApiError::Cancelled) if cancel_token.is_cancelled() => {
                    // User cancelled. hf-hub leaves the partially downloaded
                    // `.sync.part` in the shared cache, so a later attempt resumes
                    // instead of restarting. The guard resets is_downloading and
                    // drops the token; `cancel_download` already emitted
                    // `model-download-cancelled`.
                    info!("HF download cancelled for: {}", model_id);
                    return Ok(());
                }
                Err(hf_hub::api::tokio::ApiError::Cancelled) => {
                    let err = anyhow::anyhow!(
                        "transfer stalled: no progress for {}s",
                        DOWNLOAD_STALL_TIMEOUT.as_secs()
                    );
                    // A parallel attempt may be what wedged the network. Give
                    // the connection pool a brief pause, then retry once using
                    // the known-compatible single-stream path. A sequential
                    // stall already cost DOWNLOAD_STALL_TIMEOUT, so further
                    // retries would likely just repeat it — use the mirror.
                    if stream_count == 1 || attempt >= ATTEMPT_STREAMS.len() {
                        break Some(err);
                    }
                    let delay = Duration::from_secs(1_u64 << attempt);
                    warn!(
                        "HF download attempt {}/{} stalled for {} using {} concurrent stream(s); retrying with {} stream(s) in {}s",
                        attempt,
                        ATTEMPT_STREAMS.len(),
                        model_id,
                        stream_count,
                        ATTEMPT_STREAMS[attempt],
                        delay.as_secs()
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel_token.cancelled() => {
                            info!("HF download cancelled for: {}", model_id);
                            return Ok(());
                        }
                    }
                    attempt += 1;
                }
                Err(e) => {
                    // {:?} keeps the error source chain (reset vs TLS vs timeout);
                    // Display truncates it to "error sending request".
                    let err = anyhow::anyhow!("{:?}", e);
                    if attempt >= ATTEMPT_STREAMS.len() {
                        break Some(err);
                    }
                    let delay = Duration::from_secs(1_u64 << attempt);
                    warn!(
                        "HF download attempt {}/{} failed for {} using {} concurrent stream(s): {}; retrying with {} stream(s) in {}s",
                        attempt,
                        ATTEMPT_STREAMS.len(),
                        model_id,
                        stream_count,
                        err,
                        ATTEMPT_STREAMS[attempt],
                        delay.as_secs()
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel_token.cancelled() => {
                            info!("HF download cancelled for: {}", model_id);
                            return Ok(());
                        }
                    }
                    attempt += 1;
                }
            }
        };

        if let Some(hf_error) = hf_error {
            // `attempt`, not the schedule length: a sequential stall breaks out early.
            error!(
                "HF download failed for {} after {} attempt(s): {:?}",
                model_id, attempt, hf_error
            );
            let mirrors = crate::catalog::mirror_fallbacks(&model_id);
            if mirrors.is_empty() {
                return Err(anyhow::anyhow!(
                    "Hugging Face download failed after {} attempt(s): {}",
                    attempt,
                    hf_error
                ));
            }
            let mut completed = false;
            for mirror in &mirrors {
                info!("Falling back to mirror for {}: {}", model_id, mirror.url);
                match self
                    .download_from_mirror(&model_id, &filename, mirror, cancel_token.clone())
                    .await
                {
                    Ok(true) => {
                        completed = true;
                        break;
                    }
                    Ok(false) => {
                        info!("Mirror download cancelled for: {}", model_id);
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(
                            "Mirror download failed for {} from {}: {:?}",
                            model_id, mirror.url, e
                        );
                    }
                }
            }
            if !completed {
                return Err(anyhow::anyhow!(
                    "Download failed from Hugging Face ({}) and {} mirror(s)",
                    hf_error,
                    mirrors.len()
                ));
            }
        }

        cleanup.disarmed = true;
        self.update_download_status()?;
        self.cancel_flags.lock_recover().remove(&model_id);
        let _ = self.app_handle.emit("model-download-complete", &model_id);
        info!("HF model {} downloaded", model_id);
        Ok(())
    }

    /// Direct-HTTP download of a catalog model's file from a mirror into the
    /// models dir (HF-model resolution accepts that location too). Returns
    /// `Ok(true)` on completion, `Ok(false)` if cancelled (partial kept).
    pub(super) async fn download_from_mirror(
        &self,
        model_id: &str,
        filename: &str,
        mirror: &crate::catalog::MirrorFile,
        cancel_token: CancellationToken,
    ) -> Result<bool> {
        fs::create_dir_all(&self.models_dir)?;
        let model_path = self.models_dir.join(filename);
        let partial_path = self.models_dir.join(format!("{}.partial", filename));

        if model_path.exists() {
            return Ok(true);
        }

        match self
            .download_http_resumable(
                model_id,
                &mirror.url,
                &partial_path,
                Some(mirror.size_bytes),
                Some(&mirror.sha256),
                &cancel_token,
            )
            .await?
        {
            HttpDownloadOutcome::Cancelled => Ok(false),
            HttpDownloadOutcome::Completed => {
                fs::rename(&partial_path, &model_path)?;
                info!(
                    "Mirror download of {} completed and verified ({:?})",
                    model_id, model_path
                );
                Ok(true)
            }
        }
    }

    pub async fn download_model(&self, model_id: &str) -> Result<()> {
        let model_info = {
            let models = self.available_models.lock_recover();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        let (url, expected_sha256) = match &model_info.source {
            ModelSource::Url { url, sha256 } => (url.clone(), sha256.clone()),
            ModelSource::HuggingFace { repo_id, revision } => {
                return self
                    .download_hf_model(&model_info, repo_id.clone(), revision.clone())
                    .await;
            }
            ModelSource::Local => {
                return Err(anyhow::anyhow!("No download source for model"));
            }
        };
        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", model_info.filename));

        // Don't download if complete version already exists
        if model_path.exists() {
            // Clean up any partial file that might exist
            if partial_path.exists() {
                let _ = fs::remove_file(&partial_path);
            }
            self.update_download_status()?;
            return Ok(());
        }

        // Mark as downloading
        {
            let mut models = self.available_models.lock_recover();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = true;
            }
        }

        // Create cancellation token for this download
        let cancel_token = CancellationToken::new();
        {
            let mut flags = self.cancel_flags.lock_recover();
            flags.insert(model_id.to_string(), cancel_token.clone());
        }

        // Guard ensures is_downloading and cancel_flags are cleaned up on every
        // error path. Disarmed only on success (which sets is_downloaded = true).
        let mut cleanup = DownloadCleanup {
            available_models: &self.available_models,
            cancel_flags: &self.cancel_flags,
            model_id: model_id.to_string(),
            disarmed: false,
        };

        // URL sources carry no authoritative size, so the helper falls back to
        // the server's content-length for progress and completeness checks.
        match self
            .download_http_resumable(
                model_id,
                &url,
                &partial_path,
                None,
                expected_sha256.as_deref(),
                &cancel_token,
            )
            .await?
        {
            HttpDownloadOutcome::Cancelled => {
                info!("Download cancelled for: {}", model_id);
                // Keep partial file for resume functionality.
                // Guard handles is_downloading + cancel_flags cleanup on drop.
                return Ok(());
            }
            HttpDownloadOutcome::Completed => {}
        }

        // Handle directory-based models (extract tar.gz) vs file-based models
        if model_info.is_directory {
            // Track that this model is being extracted
            {
                let mut extracting = self.extracting_models.lock_recover();
                extracting.insert(model_id.to_string());
            }

            // Emit extraction started event
            let _ = self.app_handle.emit("model-extraction-started", model_id);
            info!("Extracting archive for directory-based model: {}", model_id);

            // Use a temporary extraction directory to ensure atomic operations
            let temp_extract_dir = self
                .models_dir
                .join(format!("{}.extracting", model_info.filename));
            let final_model_dir = self.models_dir.join(&model_info.filename);

            // Clean up any previous incomplete extraction
            if temp_extract_dir.exists() {
                let _ = fs::remove_dir_all(&temp_extract_dir);
            }

            // Create temporary extraction directory
            fs::create_dir_all(&temp_extract_dir)?;

            // Open the downloaded tar.gz file
            let tar_gz = File::open(&partial_path)?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);

            // Extract to the temporary directory first
            archive.unpack(&temp_extract_dir).map_err(|e| {
                let error_msg = format!("Failed to extract archive: {}", e);
                // Clean up failed extraction
                let _ = fs::remove_dir_all(&temp_extract_dir);
                // Delete the corrupt partial file so the next download attempt starts fresh
                // instead of resuming from a broken archive (issue #858).
                let _ = fs::remove_file(&partial_path);
                // Remove from extracting set
                {
                    let mut extracting = self.extracting_models.lock_recover();
                    extracting.remove(model_id);
                }
                let _ = self.app_handle.emit(
                    "model-extraction-failed",
                    &serde_json::json!({
                        "model_id": model_id,
                        "error": error_msg
                    }),
                );
                anyhow::anyhow!(error_msg)
            })?;

            // Find the actual extracted directory (archive might have a nested structure)
            let extracted_dirs: Vec<_> = fs::read_dir(&temp_extract_dir)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .collect();

            if extracted_dirs.len() == 1 {
                // Single directory extracted, move it to the final location
                let source_dir = extracted_dirs[0].path();
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&source_dir, &final_model_dir)?;
                // Clean up temp directory
                let _ = fs::remove_dir_all(&temp_extract_dir);
            } else {
                // Multiple items or no directories, rename the temp directory itself
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&temp_extract_dir, &final_model_dir)?;
            }

            info!("Successfully extracted archive for model: {}", model_id);
            // Remove from extracting set
            {
                let mut extracting = self.extracting_models.lock_recover();
                extracting.remove(model_id);
            }
            // Emit extraction completed event
            let _ = self.app_handle.emit("model-extraction-completed", model_id);

            // Remove the downloaded tar.gz file
            let _ = fs::remove_file(&partial_path);
        } else {
            // Move partial file to final location for file-based models
            fs::rename(&partial_path, &model_path)?;
        }

        // Disarm the guard — success path does its own cleanup because it
        // additionally sets is_downloaded = true.
        cleanup.disarmed = true;
        {
            let mut models = self.available_models.lock_recover();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
                model.is_downloaded = true;
                model.partial_size = 0;
            }
        }
        self.cancel_flags.lock_recover().remove(model_id);

        // Emit completion event
        let _ = self.app_handle.emit("model-download-complete", model_id);

        info!(
            "Successfully downloaded model {} to {:?}",
            model_id, model_path
        );

        Ok(())
    }

    pub fn delete_model(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: delete_model called for: {}", model_id);

        let model_info = {
            let models = self.available_models.lock_recover();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        debug!("ModelManager: Found model info: {:?}", model_info);

        if let ModelSource::HuggingFace { repo_id, revision } = &model_info.source {
            let is_alternate_quant =
                Self::is_catalog_alternate_quant(repo_id, &model_info.filename);
            let mut deleted = false;
            if is_alternate_quant {
                // Only this quant's own file: the snapshot pointer and its
                // blob. The default (and any other quants) survive in the
                // cache — the entry never owned more than its one file.
                deleted |= Self::delete_hf_cache_file(repo_id, revision, &model_info.filename);
            } else if let Some(file) = hf_cached_path(repo_id, revision, &model_info.filename) {
                // Cached at <cache>/models--org--name/snapshots/<rev>/<file>; remove
                // the whole repo dir (blobs + refs + snapshots). Per product decision,
                // delete hard-removes from the shared HF cache.
                if let Some(repo_dir) = file.ancestors().nth(3) {
                    if repo_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("models--"))
                    {
                        info!("Deleting HF cache repo at: {:?}", repo_dir);
                        fs::remove_dir_all(repo_dir)?;
                        deleted = true;
                    }
                }
            }
            // Also remove a models-dir copy (mirror fallback / manual drop-in)
            // and any resumable partial next to it.
            for path in [
                self.models_dir.join(&model_info.filename),
                self.models_dir
                    .join(format!("{}.partial", model_info.filename)),
            ] {
                if path.exists() {
                    info!("Deleting model file at: {:?}", path);
                    fs::remove_file(&path)?;
                    deleted = true;
                }
            }
            if !deleted {
                return Err(anyhow::anyhow!("No model files found to delete"));
            }
            // Alternate-quant entries are discovery-created (the catalog only
            // seeds defaults), so deleting one un-discovers it rather than
            // leaving a permanent "(Q4_K_M)" row in the list.
            if is_alternate_quant {
                self.available_models.lock_recover().remove(model_id);
            }
            self.update_download_status()?;
            let _ = self.app_handle.emit("model-deleted", model_id);
            return Ok(());
        }

        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", model_info.filename));
        debug!("ModelManager: Model path: {:?}", model_path);
        debug!("ModelManager: Partial path: {:?}", partial_path);

        let mut deleted_something = false;

        if model_info.is_directory {
            // Delete complete model directory if it exists
            if model_path.exists() && model_path.is_dir() {
                info!("Deleting model directory at: {:?}", model_path);
                fs::remove_dir_all(&model_path)?;
                info!("Model directory deleted successfully");
                deleted_something = true;
            }
        } else {
            // Delete complete model file if it exists
            if model_path.exists() {
                info!("Deleting model file at: {:?}", model_path);
                fs::remove_file(&model_path)?;
                info!("Model file deleted successfully");
                deleted_something = true;
            }
        }

        // Delete partial file if it exists (same for both types)
        if partial_path.exists() {
            info!("Deleting partial file at: {:?}", partial_path);
            fs::remove_file(&partial_path)?;
            info!("Partial file deleted successfully");
            deleted_something = true;
        }

        if !deleted_something {
            return Err(anyhow::anyhow!("No model files found to delete"));
        }

        // Custom models should be removed from the list entirely since they
        // have no download URL and can't be re-downloaded
        if model_info.is_custom {
            let mut models = self.available_models.lock_recover();
            models.remove(model_id);
            debug!("ModelManager: removed custom model from available models");
        } else {
            // Update download status (marks predefined models as not downloaded)
            self.update_download_status()?;
            debug!("ModelManager: download status updated");
        }

        // Emit event to notify UI
        let _ = self.app_handle.emit("model-deleted", model_id);

        Ok(())
    }

    pub fn get_model_path(&self, model_id: &str) -> Result<PathBuf> {
        let model_info = self
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        if !model_info.is_downloaded {
            return Err(anyhow::anyhow!("Model not available: {}", model_id));
        }

        // Ensure we don't return partial files/directories
        if model_info.is_downloading {
            return Err(anyhow::anyhow!(
                "Model is currently downloading: {}",
                model_id
            ));
        }

        if let ModelSource::HuggingFace { repo_id, revision } = &model_info.source {
            if let Some(path) = hf_cached_path(repo_id, revision, &model_info.filename) {
                return Ok(path);
            }
            // Mirror-fallback download or manual drop-in in the models dir.
            // The complete file only ever appears after verification, so a
            // stale `.partial` alongside it is leftover noise, not a veto —
            // clear it rather than declaring the model missing.
            let local_path = self.models_dir.join(&model_info.filename);
            if local_path.exists() {
                let partial_path = self
                    .models_dir
                    .join(format!("{}.partial", model_info.filename));
                if partial_path.exists() {
                    let _ = fs::remove_file(&partial_path);
                }
                return Ok(local_path);
            }
            return Err(anyhow::anyhow!(
                "Complete model file not found in HF cache or models dir: {}",
                model_id
            ));
        }

        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", model_info.filename));

        if model_info.is_directory {
            // For directory-based models, ensure the directory exists and is complete
            if model_path.exists() && model_path.is_dir() && !partial_path.exists() {
                Ok(model_path)
            } else {
                Err(anyhow::anyhow!(
                    "Complete model directory not found: {}",
                    model_id
                ))
            }
        } else {
            // For file-based models (existing logic)
            if model_path.exists() && !partial_path.exists() {
                Ok(model_path)
            } else {
                Err(anyhow::anyhow!(
                    "Complete model file not found: {}",
                    model_id
                ))
            }
        }
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: cancel_download called for: {}", model_id);

        // Trigger the cancellation token to stop the download. The HF path
        // aborts its in-flight chunk tasks and unwinds promptly; the URL path
        // observes it on the next chunk of its stream loop.
        {
            let flags = self.cancel_flags.lock_recover();
            if let Some(token) = flags.get(model_id) {
                token.cancel();
                info!("Cancellation token triggered for: {}", model_id);
            } else {
                warn!("No active download found for: {}", model_id);
            }
        }

        // Update state immediately for UI responsiveness
        {
            let mut models = self.available_models.lock_recover();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
            }
        }

        // Update download status to reflect current state
        self.update_download_status()?;

        // Emit cancellation event so all UI components can clear their state
        let _ = self.app_handle.emit("model-download-cancelled", model_id);

        info!("Download cancellation initiated for: {}", model_id);
        Ok(())
    }
}
