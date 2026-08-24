//! Persistence: loading, salvaging, migrating, and writing the settings
//! store (tauri-plugin-store JSON).

use log::{debug, error, warn};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::{Store, StoreExt};

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::schema::AppSettings;
use super::schema::{default_transcribe_gpu_device, CURRENT_SETTINGS_SCHEMA_VERSION};
use super::schema::{ensure_post_process_defaults, get_default_settings};
use super::types::*;

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";

/// Test-only redirect for the settings store location. Once set (first caller
/// wins), every store open in this process resolves inside this directory so
/// tests run against an isolated temp dir instead of the real user data.
#[cfg(test)]
static TEST_STORE_DIR: LazyLock<std::sync::OnceLock<std::path::PathBuf>> =
    LazyLock::new(std::sync::OnceLock::new);

/// Test-only: point [`settings_store_path`] at `dir`. Best-effort — if
/// another test already claimed the location, that directory stays in effect;
/// the actually active directory is returned either way.
#[cfg(test)]
pub(crate) fn set_test_store_dir(dir: &std::path::Path) -> std::path::PathBuf {
    let _ = TEST_STORE_DIR.set(dir.to_path_buf());
    TEST_STORE_DIR.get().unwrap().clone()
}

/// Resolves the on-disk location of the settings store file.
fn settings_store_path() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_STORE_DIR.get() {
        return dir.join(SETTINGS_STORE_PATH);
    }
    crate::portable::store_path(SETTINGS_STORE_PATH)
}

/// Serializes settings transactions inside this process. The store plugin
/// protects individual reads and writes, but a read-modify-write sequence still
/// needs one lock around the whole transaction to avoid lost updates.
static SETTINGS_TRANSACTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn lock_settings_transaction() -> MutexGuard<'static, ()> {
    SETTINGS_TRANSACTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Startup entry point. Same load-or-create/salvage/migrate behavior as
/// `get_settings`; kept as a named alias for call-site clarity, plus a
/// one-time debug dump of the loaded settings.
pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    let settings = get_settings(app);
    debug!("Loaded settings: {:?}", settings);
    settings
}

pub fn get_settings<T: Runtime>(app: &AppHandle<T>) -> AppSettings {
    let _transaction = lock_settings_transaction();
    get_settings_unlocked(app)
}

fn get_settings_unlocked<T: Runtime>(app: &AppHandle<T>) -> AppSettings {
    let store = app
        .store(settings_store_path())
        .expect("Failed to initialize store");

    // Settings reads also persist one-time migrations. Migration helpers are
    // idempotent, so this converges after the first read of an older store.
    let mut settings = if let Some(settings_value) = store.get("settings") {
        let (mut settings, mut updated) =
            match serde_json::from_value::<AppSettings>(settings_value.clone()) {
                Ok(settings) => (settings, false),
                Err(e) => {
                    warn!("Failed to parse stored settings ({e}); salvaging valid fields");
                    (salvage_settings(&settings_value), true)
                }
            };

        if apply_settings_migrations(&mut settings, &settings_value) {
            updated = true;
        }

        // Merge in any bindings added since this store was written.
        for (key, value) in get_default_settings().bindings {
            if let std::collections::hash_map::Entry::Vacant(entry) = settings.bindings.entry(key) {
                debug!("Adding missing binding: {}", entry.key());
                entry.insert(value);
                updated = true;
            }
        }

        if updated {
            store.set("settings", serde_json::to_value(&settings).unwrap());
            flush_settings_store(&store);
        }

        settings
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        flush_settings_store(&store);
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
        flush_settings_store(&store);
    }

    // Surface keychain-stored API keys to callers/UI. The maps on disk stay
    // empty; this only fills the in-memory copy.
    crate::secret_store::hydrate_secret_maps(&mut settings);

    settings
}

/// Rebuilds settings from a store value that failed to deserialize as a whole.
/// Every stored field that is individually valid is kept; only broken values
/// (e.g. an enum variant written by a newer or older version) fall back to
/// their default. This means one bad field can never reset the rest of the
/// user's configuration (#1619).
pub(super) fn salvage_settings(stored: &serde_json::Value) -> AppSettings {
    let Some(stored_map) = stored.as_object() else {
        warn!("Stored settings are not a JSON object; falling back to defaults");
        return get_default_settings();
    };

    let mut merged = serde_json::to_value(get_default_settings())
        .expect("default settings serialize to a JSON object");

    for (key, value) in stored_map {
        let previous = merged
            .as_object_mut()
            .expect("merged settings stay an object")
            .insert(key.clone(), value.clone());
        if serde_json::from_value::<AppSettings>(merged.clone()).is_err() {
            // Log only the key: values may hold secrets (e.g. API keys).
            warn!("Dropping invalid settings field '{key}', keeping its default");
            let map = merged
                .as_object_mut()
                .expect("merged settings stay an object");
            match previous {
                Some(previous) => map.insert(key.clone(), previous),
                None => map.remove(key),
            };
        }
    }

    serde_json::from_value(merged).unwrap_or_else(|e| {
        warn!("Failed to reassemble salvaged settings ({e}); falling back to defaults");
        get_default_settings()
    })
}

pub(super) fn apply_settings_migrations(
    settings: &mut AppSettings,
    settings_value: &serde_json::Value,
) -> bool {
    let mut updated = false;

    // One-time onboarding migration: users with an explicit selected model have
    // already made it through model selection. Users who merely have compatible
    // files on disk should still see onboarding.
    if settings_value.get("onboarding_completed").is_none() {
        settings.onboarding_completed = !settings.selected_model.is_empty();
        updated = true;
    }

    // One-time What's New migration: migrations only run on an existing store
    // (fresh installs stamp the current version via get_default_settings). A
    // missing key here means a user upgrading from before it existed — blank it
    // so they see the current release's What's New, mirroring the onboarding
    // migration's explicit first-run-vs-upgrade decision.
    if settings_value.get("whats_new_last_seen_version").is_none() {
        settings.whats_new_last_seen_version = String::new();
        updated = true;
    }

    let stored_schema_version = settings_value
        .get("settings_schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if stored_schema_version < 1 {
        // Before schema 1 this was a UI ordinal. Preserve the original safety
        // migration: a positive selection was ambiguous even in 0.1.
        let had_positive_legacy_selection = settings_value
            .get("transcribe_gpu_device")
            .and_then(|value| value.as_i64())
            .is_some_and(|value| value > 0);
        if had_positive_legacy_selection {
            settings.transcribe_accelerator = TranscribeAcceleratorSetting::Auto;
        }
    }
    if stored_schema_version < 2 {
        // transcribe.cpp 0.2 replaced integer registry indices with opaque
        // process-local handles. Clear every old index once.
        settings.transcribe_gpu_device = default_transcribe_gpu_device();
        settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
        updated = true;
    }

    // The generic GPU choice was removed in favor of Auto or an exact device.
    // Normalize settings created by builds that exposed that short-lived option.
    if settings.transcribe_accelerator == TranscribeAcceleratorSetting::Gpu
        && settings.transcribe_gpu_device.is_none()
    {
        settings.transcribe_accelerator = TranscribeAcceleratorSetting::Auto;
        updated = true;
    }

    // One-time overlay migration (only while the new key is absent): the retired
    // overlay_position `none` meant "hide the overlay" → OverlayStyle::None; any
    // other position had it visible → Live. The position enum no longer has a
    // `none` variant (legacy "none" deserializes to Bottom via a serde alias), so
    // read the raw stored string to recover the old intent.
    if settings_value.get("overlay_style").is_none() {
        let was_hidden = settings_value
            .get("overlay_position")
            .and_then(|v| v.as_str())
            == Some("none");
        settings.overlay_style = if was_hidden {
            OverlayStyle::None
        } else {
            OverlayStyle::Live
        };
        updated = true;
    }

    updated
}

/// Durable flush point for every acknowledged settings write.
///
/// Why an explicit `save()` is required here: tauri-plugin-store's
/// [`Store::set`] only mutates the in-memory cache and schedules a *debounced*
/// background auto-save — `StoreBuilder::default()` (what `StoreExt::store`
/// uses) enables `auto_save(100 ms)`, and the debounce timer runs on the async
/// runtime. An immediate process kill inside that window (crash, SIGKILL,
/// power loss) would lose an update that `update_settings` /
/// `try_update_settings` had already acknowledged to its caller. Every write
/// site therefore ends with this synchronous flush: [`Store::save`] cancels
/// any pending debounced auto-save and then blocking-writes the whole JSON
/// document via `fs::write`, so once the surrounding transaction returns, the
/// new value is on disk and observable by a freshly opened store handle.
///
/// A failed flush is logged loudly but does not abort the transaction: the
/// in-memory state stays coherent, callers keep their acknowledged value, and
/// the next settings write retries persistence.
fn flush_settings_store<T: Runtime>(store: &Store<T>) {
    if let Err(e) = store.save() {
        error!("Failed to persist settings store to disk: {e}");
    }
}

fn write_settings_unlocked<T: Runtime>(app: &AppHandle<T>, settings: &AppSettings) {
    let store = app
        .store(settings_store_path())
        .expect("Failed to initialize store");

    // See [`flush_settings_store`]: `set` alone is not durable; the explicit
    // save below is the durability barrier of every settings transaction.
    store.set("settings", serde_json::to_value(settings).unwrap());
    flush_settings_store(&store);
}

/// Runs one serialized settings transaction against pluggable load/persist
/// backends. The transaction lock is held across the whole read-modify-write,
/// so concurrent callers cannot interleave a load between another caller's
/// load and persist (the lost-update race the pre-b677107
/// `get_settings`/`write_settings` pairing had). Split out from
/// [`update_settings`] so tests can exercise the real locking behavior
/// without a Tauri `AppHandle`.
pub(super) fn run_transaction<R>(
    load: impl FnOnce() -> AppSettings,
    persist: impl FnOnce(&AppSettings),
    update: impl FnOnce(&mut AppSettings) -> R,
) -> R {
    let _transaction = lock_settings_transaction();
    let mut settings = load();
    let result = update(&mut settings);
    persist(&settings);
    result
}

/// [`run_transaction`] variant which persists only when the mutation succeeds.
/// The error is returned verbatim to the caller and the loaded snapshot is
/// discarded, so a failed transaction leaves the persisted settings untouched.
pub(super) fn try_run_transaction<R, E>(
    load: impl FnOnce() -> AppSettings,
    persist: impl FnOnce(&AppSettings),
    update: impl FnOnce(&mut AppSettings) -> Result<R, E>,
) -> Result<R, E> {
    let _transaction = lock_settings_transaction();
    let mut settings = load();
    let result = update(&mut settings)?;
    persist(&settings);
    Ok(result)
}

/// Atomically loads, mutates, and persists settings.
pub fn update_settings<T: Runtime, R>(
    app: &AppHandle<T>,
    update: impl FnOnce(&mut AppSettings) -> R,
) -> R {
    run_transaction(
        || get_settings_unlocked(app),
        |settings| write_settings_unlocked(app, settings),
        update,
    )
}

/// Atomic settings transaction which persists only when the mutation succeeds.
pub fn try_update_settings<T: Runtime, R, E>(
    app: &AppHandle<T>,
    update: impl FnOnce(&mut AppSettings) -> Result<R, E>,
) -> Result<R, E> {
    try_run_transaction(
        || get_settings_unlocked(app),
        |settings| write_settings_unlocked(app, settings),
        update,
    )
}

pub fn get_bindings<T: Runtime>(app: &AppHandle<T>) -> HashMap<String, ShortcutBinding> {
    let settings = get_settings(app);

    settings.bindings
}

pub fn get_stored_binding<T: Runtime>(app: &AppHandle<T>, id: &str) -> ShortcutBinding {
    let bindings = get_bindings(app);

    let binding = bindings.get(id).unwrap().clone();

    binding
}

pub fn get_history_limit<T: Runtime>(app: &AppHandle<T>) -> usize {
    let settings = get_settings(app);
    settings.history_limit
}

pub fn get_recording_retention_period<T: Runtime>(app: &AppHandle<T>) -> RecordingRetentionPeriod {
    let settings = get_settings(app);
    settings.recording_retention_period
}
