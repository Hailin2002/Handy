//! Internal helpers shared by discovery, migration, and download paths:
//! HF-cache lookup, capability summarizing, progress bridging into Tauri
//! events, and RAII guards.

use hf_hub::api::tokio::Progress;
use hf_hub::{Cache, Repo, RepoType};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::AppHandle;
use tauri::Emitter;

use super::info::{canonicalize_supported_languages, DownloadProgress, ModelInfo};
use crate::managers::model_capabilities::CapabilityProbe;
use crate::sync::MutexExt;
use hf_hub::api::tokio::CancellationToken;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Resolve a Hugging Face model file in the shared HF cache, if already present.
/// Uses hf-hub's stock location (HF_HOME or ~/.cache/huggingface/hub) so
/// downloads are shared with other tools.
///
/// hf-hub resolves purely through `refs/<revision>`. Pinned downloads write
/// `refs/<commit-sha>`, but caches populated before pinning — or by other
/// tools, which download via `main` — only have `refs/main`, so lookup falls
/// back to it. Grandfathered `main` copies may predate the pin; per policy a
/// working local model is never invalidated by routine catalog regeneration.
pub(super) fn hf_cached_path(repo_id: &str, revision: &str, filename: &str) -> Option<PathBuf> {
    let get = |rev: &str| {
        Cache::from_env()
            .repo(Repo::with_revision(
                repo_id.to_string(),
                RepoType::Model,
                rev.to_string(),
            ))
            .get(filename)
    };
    get(revision).or_else(|| (revision != "main").then(|| get("main")).flatten())
}

/// Friendly name advertised by GGUF metadata, if present. Empty strings are not
/// useful display names, so callers can keep their filename/repo fallback.
pub(super) fn probed_display_name(probe: &CapabilityProbe) -> Option<String> {
    probe
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// Capability fields for a locally-discovered on-disk model, derived from its
/// GGUF header probe. Anything without readable GGUF metadata — a legacy `.bin`
/// file, or a header that simply omits a key — collapses to "no advertised
/// capability"; transcribe-cpp still reconciles the real values at load time.
/// Shared by both local discovery paths (custom models dir + HF cache) so they
/// surface capabilities identically.
pub(super) struct LocalCaps {
    pub(super) supports_streaming: bool,
    pub(super) supports_translation: bool,
    pub(super) supports_language_selection: bool,
    pub(super) supports_language_detection: bool,
    pub(super) supported_languages: Vec<String>,
}

pub(super) fn local_caps(probe: &CapabilityProbe) -> LocalCaps {
    let languages = canonicalize_supported_languages(probe.languages.clone().unwrap_or_default());
    LocalCaps {
        supports_streaming: probe.supports_streaming.unwrap_or(false),
        supports_translation: probe.supports_translation.unwrap_or(false),
        // Only offer a language picker when there's more than one to choose.
        supports_language_selection: languages.len() > 1,
        supports_language_detection: probe.supports_language_detect.unwrap_or(false),
        supported_languages: languages,
    }
}

/// Bridges hf-hub's async download progress to Handy's `model-download-progress`
/// event. hf-hub clones the reporter, so shared state lives behind an `Arc`.
#[derive(Clone)]
pub(super) struct HfDownloadProgress {
    pub(super) app_handle: AppHandle,
    pub(super) model_id: String,
    pub(super) state: Arc<Mutex<HfProgressState>>,
}

pub(super) struct HfProgressState {
    pub(super) total: u64,
    pub(super) downloaded: u64,
    pub(super) last_emit: Instant,
    /// Every callback (even throttled-out ones) bumps this; the stall watchdog
    /// reads it. Starts at construction so a hang before the first byte —
    /// e.g. a wedged metadata/resolve request — also counts as a stall.
    pub(super) last_activity: Instant,
}

impl HfDownloadProgress {
    pub(super) fn new(app_handle: AppHandle, model_id: String) -> Self {
        Self {
            app_handle,
            model_id,
            state: Arc::new(Mutex::new(HfProgressState {
                total: 0,
                downloaded: 0,
                last_emit: Instant::now(),
                last_activity: Instant::now(),
            })),
        }
    }

    /// Instant of the most recent sign of life from the transfer.
    pub(super) fn last_activity(&self) -> Instant {
        self.state.lock_recover().last_activity
    }

    pub(super) fn emit(&self, downloaded: u64, total: u64) {
        let percentage = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = self.app_handle.emit(
            crate::events::MODEL_DOWNLOAD_PROGRESS,
            &DownloadProgress {
                model_id: self.model_id.clone(),
                downloaded,
                total,
                percentage,
            },
        );
    }
}

impl Progress for HfDownloadProgress {
    async fn init(&mut self, size: usize, _filename: &str) {
        {
            let mut st = self.state.lock_recover();
            st.total = size as u64;
            st.downloaded = 0;
            st.last_emit = Instant::now();
            st.last_activity = Instant::now();
        }
        self.emit(0, size as u64);
    }

    async fn update(&mut self, size: usize) {
        let (downloaded, total, emit) = {
            let mut st = self.state.lock_recover();
            st.downloaded = st.downloaded.saturating_add(size as u64);
            let now = Instant::now();
            st.last_activity = now;
            // Throttle to ~10 updates/sec, but always emit the final byte.
            let emit = now.duration_since(st.last_emit) >= Duration::from_millis(100)
                || (st.total > 0 && st.downloaded >= st.total);
            if emit {
                st.last_emit = now;
            }
            (st.downloaded, st.total, emit)
        };
        if emit {
            self.emit(downloaded, total);
        }
    }

    async fn finish(&mut self) {
        let total = {
            let st = self.state.lock_recover();
            st.total.max(st.downloaded)
        };
        self.emit(total, total);
    }
}

/// RAII guard that clears the `is_rescanning` single-flight flag on drop, so the
/// slot is released on every exit path (including early returns and `?`).
pub(super) struct RescanGuard {
    pub(super) flag: Arc<AtomicBool>,
}

impl Drop for RescanGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// RAII guard that cleans up download state (`is_downloading` flag and cancel flag)
/// when dropped, unless explicitly disarmed. This ensures consistent cleanup on
/// every error path without requiring manual cleanup at each `?` or `return Err`.
pub(super) struct DownloadCleanup<'a> {
    pub(super) available_models: &'a Mutex<HashMap<String, ModelInfo>>,
    pub(super) cancel_flags: &'a Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub(super) model_id: String,
    pub(super) disarmed: bool,
}

impl<'a> Drop for DownloadCleanup<'a> {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        {
            let mut models = self.available_models.lock_recover();
            if let Some(model) = models.get_mut(self.model_id.as_str()) {
                model.is_downloading = false;
            }
        }
        self.cancel_flags.lock_recover().remove(&self.model_id);
    }
}
