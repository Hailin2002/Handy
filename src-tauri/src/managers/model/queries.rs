//! Registry queries and rescan orchestration on `ModelManager`.

use anyhow::Result;
use log::{debug, warn};
use std::collections::HashMap;
use tauri::Emitter;

use super::info::canonicalize_supported_languages;
use super::support::RescanGuard;
use super::*;
use crate::sync::MutexExt;

impl ModelManager {
    pub fn get_available_models(&self) -> Vec<ModelInfo> {
        let mut list: Vec<ModelInfo> = {
            let models = self.available_models.lock_recover();
            models.values().cloned().collect()
        };
        // Stable, reasonable order: catalog editorial rank first (lower = higher
        // priority), then any other recommended model, then by accuracy, speed,
        // and name. `ModelInfo` doesn't carry rank, so resolve it by id from the
        // catalog here.
        list.sort_by(|a, b| {
            crate::catalog::rank_of(&a.id)
                .cmp(&crate::catalog::rank_of(&b.id))
                .then((!a.is_recommended).cmp(&(!b.is_recommended)))
                .then(b.accuracy_score.total_cmp(&a.accuracy_score))
                .then(b.speed_score.total_cmp(&a.speed_score))
                .then_with(|| a.name.cmp(&b.name))
        });
        list
    }

    /// Seed the bundled catalog ([`crate::catalog::CATALOG`]) into the registry,
    /// inserting each model whose id isn't already present (additive).
    ///
    /// Catalog (`.gguf`, `HuggingFace`) and legacy (`.bin`/ONNX, `Url`) entries
    /// stay SEPARATE — different files, ids, and runtimes. Nothing is merged or
    /// removed; the UI just hides not-on-disk `Url` entries to deprecate legacy
    /// downloads, while already-downloaded ones stay runnable. Runs before the
    /// on-disk scans so a cached model dedups onto its catalog entry.
    pub(super) fn seed_catalog_models(available_models: &mut HashMap<String, ModelInfo>) {
        use std::collections::hash_map::Entry;
        let mut added = 0usize;
        for desc in crate::catalog::CATALOG.iter() {
            if let Entry::Vacant(slot) = available_models.entry(desc.id.clone()) {
                slot.insert(desc.to_model_info(&DiskStatus::default()));
                added += 1;
            }
        }
        info!("Seeded {} catalog model(s) into the registry", added);
    }

    /// Claim the single rescan slot. Returns a guard that releases it on drop,
    /// or `None` if a rescan is already running (callers should just skip).
    pub(super) fn try_start_rescan(&self) -> Option<RescanGuard> {
        if self.is_rescanning.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(RescanGuard {
                flag: self.is_rescanning.clone(),
            })
        }
    }

    /// Re-run the local discovery scans (custom models dir + shared HF cache) so
    /// models dropped in or downloaded outside Handy show up without a restart.
    /// The merge is additive: only new ids are inserted, so existing entries keep
    /// their values — including runtime-probed capabilities from
    /// [`Self::set_runtime_capabilities`]. It then runs [`Self::update_download_status`],
    /// which recomputes disk-derived flags for *every* entry; a rescan racing an
    /// in-flight download can briefly clear its `is_downloading`, but the download
    /// continues and the event-driven UI self-corrects.
    ///
    /// The disk walk and 64 KiB header probes run against a cloned snapshot
    /// *off-lock* so readers never block on I/O; only the brief merge takes the
    /// registry lock. Concurrent calls coalesce via [`Self::try_start_rescan`].
    pub fn rescan_local_models(&self) -> Result<()> {
        let _guard = match self.try_start_rescan() {
            Some(g) => g,
            None => {
                debug!("Model rescan already in progress; skipping");
                return Ok(());
            }
        };

        // Snapshot the current registry and discover against the copy off-lock.
        // The discover_* helpers are purely additive (they skip ids already in
        // the map), so the snapshot ends up as {current} ∪ {newly-found}.
        let mut snapshot = self.available_models.lock_recover().clone();
        if let Err(e) = Self::discover_custom_transcribe_models(&self.models_dir, &mut snapshot) {
            warn!("Rescan: failed to discover custom models: {}", e);
        }
        Self::discover_hf_cache_models(&mut snapshot);

        // Merge only the genuinely-new ids back into the live registry. `or_insert`
        // leaves every existing entry exactly as it was.
        let mut added = 0usize;
        {
            let mut live = self.available_models.lock_recover();
            for (id, info) in snapshot {
                if let std::collections::hash_map::Entry::Vacant(entry) = live.entry(id) {
                    entry.insert(info);
                    added += 1;
                }
            }
        }

        self.update_download_status()?;
        self.auto_select_model_if_needed()?;
        if added > 0 {
            info!("Model rescan discovered {} new model(s)", added);
        }
        let _ = self.app_handle.emit(crate::events::MODELS_UPDATED, ());
        Ok(())
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.available_models.lock_recover();
        models.get(model_id).cloned()
    }

    /// Reconcile a model's advertised capabilities with the ground truth from the
    /// loaded model (transcribe-cpp's GGUF-derived capabilities), overwriting the
    /// pre-download view (catalog metadata or a header probe — see
    /// [`super::model_capabilities`]).
    ///
    /// This corrects the header probe's gaps. It matters most for **streaming**
    /// (transcribe-cpp infers it at load for parakeet/streaming families, where
    /// the flat GGUF key can be absent, and it gates whether streaming is even
    /// attempted — see `actions.rs`) and for **language detection** / the
    /// **supported-language set**, which feed [`effective_language`]; a mislabeled
    /// header would otherwise coerce an "auto" intent to a forced language for good.
    /// Translate is reconciled too for badge accuracy, though run paths re-read it
    /// live regardless.
    pub fn set_runtime_capabilities(
        &self,
        model_id: &str,
        supports_streaming: bool,
        supports_translation: bool,
        supports_language_detection: bool,
        supported_languages: Vec<String>,
    ) {
        let supported_languages = canonicalize_supported_languages(supported_languages);
        let mut models = self.available_models.lock_recover();
        if let Some(model) = models.get_mut(model_id) {
            model.supports_streaming = supports_streaming;
            model.supports_translation = supports_translation;
            model.supports_language_detection = supports_language_detection;
            // An empty set means the model is language-agnostic — but it is also
            // what a failed capability read leaves behind, so keep the probed /
            // catalog list rather than blanking a known one to nothing.
            if !supported_languages.is_empty() {
                model.supports_language_selection = supported_languages.len() > 1;
                model.supported_languages = supported_languages;
            }
        }
    }
}
