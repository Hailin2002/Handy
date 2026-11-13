//! OS-keychain-backed storage for provider API keys.
//!
//! Keys were previously persisted inside the settings store JSON in plain
//! text (the [`crate::settings::SecretMap`] wrapper only redacted debug
//! output). They now live in the OS credential store where one exists:
//!
//! - **Windows**: Credential Manager (`keyring`, windows-native)
//! - **macOS**: Keychain (`keyring`, apple-native)
//! - **Linux/other**: no universal keychain is guaranteed; keys fall back to
//!   a dedicated `secrets.json` file under the app data dir. This is still an
//!   improvement over the previous behavior — secrets are separated from the
//!   settings store (which is debug-dumped and synced by different backup
//!   paths) instead of living inline in it.
//!
//! Migration: legacy plain-text values are moved out of the settings store on
//! first startup ([`migrate_legacy_settings_secrets`]) and the settings fields
//! are then always empty on disk (defense in depth: [`crate::settings`]
//! serializes `SecretMap`s as `{}`).

use log::{error, info};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::AppHandle;

/// Scope prefix for cloud speech pipeline providers (azure / deepl / custom).
pub const SPEECH_PIPELINE_SCOPE: &str = "speech-pipeline";
/// Scope prefix for LLM post-processing providers.
pub const POST_PROCESS_SCOPE: &str = "post-process";
/// Scope prefix for assistant profiles, keyed by *profile* id rather than
/// provider id (accounts read `speak:assistant/fast_helper`). Both assistants
/// may point at the same provider while using different credentials, so they
/// must not share the post-processing entries.
pub const ASSISTANT_SCOPE: &str = "assistant";

const SERVICE: &str = "speak";

// OS credential APIs are synchronous and comparatively expensive. Settings
// are read from several UI/backend paths, so hydrating every read directly
// from Credential Manager/Keychain caused hundreds of blocking calls during
// startup and severe intermittent stalls during live speech recognition.
// Cache both present and absent values for this process; every in-process
// mutation updates the cache immediately.
static SECRET_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn account(scope: &str, id: &str) -> String {
    format!("{SERVICE}:{scope}/{id}")
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use super::{account, SERVICE};
    use keyring::Entry;
    use log::{error, warn};

    fn entry(scope: &str, id: &str) -> Option<Entry> {
        Entry::new(SERVICE, &account(scope, id))
            .map_err(|e| error!("keychain entry creation failed for {scope}/{id}: {e}"))
            .ok()
    }

    pub fn get_secret(scope: &str, id: &str) -> Option<String> {
        let entry = entry(scope, id)?;
        match entry.get_password() {
            Ok(v) => Some(v),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => {
                warn!("keychain read failed for {scope}/{id}: {e}");
                None
            }
        }
    }

    pub fn set_secret(scope: &str, id: &str, value: &str) -> Result<(), String> {
        let entry = entry(scope, id).ok_or_else(|| "keychain unavailable".to_string())?;
        entry
            .set_password(value)
            .map_err(|e| format!("keychain write failed for {scope}/{id}: {e}"))
    }

    pub fn delete_secret(scope: &str, id: &str) {
        if let Some(entry) = entry(scope, id) {
            // A missing entry is exactly what we want.
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::fallback_file;
    use std::collections::HashMap;

    fn read_all() -> HashMap<String, String> {
        let Some(path) = fallback_file() else {
            return HashMap::new();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn write_all(map: &HashMap<String, String>) -> Result<(), String> {
        let Some(path) = fallback_file() else {
            return Err("no usable app data directory".to_string());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create secrets dir: {e}"))?;
        }
        let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("failed to write secrets file: {e}"))
    }

    pub fn get_secret(scope: &str, id: &str) -> Option<String> {
        read_all().get(&account(scope, id)).cloned()
    }

    pub fn set_secret(scope: &str, id: &str, value: &str) -> Result<(), String> {
        let mut map = read_all();
        map.insert(account(scope, id), value.to_string());
        write_all(&map)
    }

    pub fn delete_secret(scope: &str, id: &str) {
        let mut map = read_all();
        if map.remove(&account(scope, id)).is_some() {
            let _ = write_all(&map);
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn fallback_file() -> Option<std::path::PathBuf> {
    crate::portable::data_dir().cloned().or_else(|| {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
            })
            .map(|base| base.join("speak"))
            .map(|dir| dir.join("secrets.json"))
    })
}

/// Reads a stored API key. Returns `None` when unset or unreadable.
fn get_secret_cached_with(
    scope: &str,
    id: &str,
    load: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let cache_key = account(scope, id);
    if let Some(cached) = SECRET_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&cache_key)
        .cloned()
    {
        return cached;
    }

    let value = load();
    SECRET_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(cache_key, value.clone());
    value
}

/// Reads a stored API key. Returns `None` when unset or unreadable.
pub fn get_secret(scope: &str, id: &str) -> Option<String> {
    get_secret_cached_with(scope, id, || imp::get_secret(scope, id))
}

/// Creates or overwrites an API key.
pub fn set_secret(scope: &str, id: &str, value: &str) -> Result<(), String> {
    imp::set_secret(scope, id, value)?;
    SECRET_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(account(scope, id), Some(value.to_string()));
    Ok(())
}

/// Removes a stored API key (used when a provider is cleared).
pub fn delete_secret(scope: &str, id: &str) {
    imp::delete_secret(scope, id);
    SECRET_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(account(scope, id), None);
}

/// Moves plain-text API keys out of a loaded [`AppSettings`] into the secret
/// store. Returns `true` when the settings changed and must be re-persisted.
///
/// Runs once per startup before anything reads the maps. Idempotent: values
/// already present in the secret store are not overwritten, and emptied maps
/// make subsequent calls no-ops.
pub fn migrate_legacy_settings_secrets(settings: &mut crate::settings::AppSettings) -> bool {
    migrate_legacy_settings_secrets_with(settings, get_secret, set_secret)
}

/// Backend-injected core of [`migrate_legacy_settings_secrets`], so tests can
/// exercise the migration/ordering contract against an in-memory store
/// instead of the real OS keychain.
fn migrate_legacy_settings_secrets_with(
    settings: &mut crate::settings::AppSettings,
    get_secret: impl Fn(&str, &str) -> Option<String>,
    set_secret: impl Fn(&str, &str, &str) -> Result<(), String>,
) -> bool {
    let mut migrated_any = false;
    let scopes = [
        (
            SPEECH_PIPELINE_SCOPE,
            &mut settings.speech_pipeline_api_keys,
        ),
        (POST_PROCESS_SCOPE, &mut settings.post_process_api_keys),
    ];

    for (scope, map) in scopes {
        let legacy: Vec<(String, String)> = map
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        for (id, value) in legacy {
            match get_secret(scope, &id) {
                Some(existing) => {
                    info!(
                        "legacy API key for {scope}/{id} found in settings but a secret \
                         already exists in the keychain; dropping the stale copy"
                    );
                    let _ = existing;
                }
                None => {
                    if let Err(e) = set_secret(scope, &id, &value) {
                        // Keep the value in the settings map so nothing is lost;
                        // migration retried on next startup.
                        error!("failed to migrate API key for {scope}/{id}: {e}");
                        continue;
                    }
                    info!("migrated API key for {scope}/{id} into the OS keychain");
                }
            }
            map.remove(&id);
            migrated_any = true;
        }
    }

    migrated_any
}

/// Startup entry point for the legacy-secret migration. Runs
/// [`migrate_legacy_settings_secrets`] inside one serialized settings
/// transaction and re-persists the (emptied) store.
///
/// Ordering invariant (docs/reviews/secrets-exposure-and-durability-audit.md
/// §3.2, finding F-6): every launch path — GUI and headless/CLI alike — MUST
/// call this before its first `get_settings`/`update_settings`. A settings
/// read that finds an older store re-persists it with schema migrations or
/// default merges applied, and `SecretMap::serialize` writes `{}` — silently
/// destroying plain-text keys that were never moved into the secret store.
/// The settings load inside this transaction also hydrates the in-memory maps
/// from the secret store (`settings::store::get_settings_unlocked`), so later
/// consumers keep seeing configured keys.
pub fn migrate_secrets_at_startup(app: &AppHandle) -> crate::settings::AppSettings {
    crate::settings::update_settings(app, |settings| {
        migrate_legacy_settings_secrets(settings);
        settings.clone()
    })
}

/// Populates the in-memory API-key maps from the secret store so callers of
/// [`crate::settings::get_settings`] (including the settings UI) keep seeing
/// the configured keys as before. Only fills missing/empty entries; persisted
/// settings still serialize the maps as `{}`.
pub fn hydrate_secret_maps(settings: &mut crate::settings::AppSettings) {
    const SPEECH_IDS: [&str; 3] = ["azure", "custom", "deepl"];
    for id in SPEECH_IDS {
        if settings
            .speech_pipeline_api_keys
            .get(id)
            .is_none_or(|v| v.is_empty())
        {
            if let Some(value) = get_secret(SPEECH_PIPELINE_SCOPE, id) {
                settings.speech_pipeline_api_keys.insert(id.into(), value);
            }
        }
    }

    let provider_ids: Vec<String> = settings
        .post_process_providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    for id in provider_ids {
        if settings
            .post_process_api_keys
            .get(&id)
            .is_none_or(|v| v.is_empty())
        {
            if let Some(value) = get_secret(POST_PROCESS_SCOPE, &id) {
                settings.post_process_api_keys.insert(id, value);
            }
        }
    }

    // Keyed by profile id — see `ASSISTANT_SCOPE`. Without this the assistant
    // settings UI would show an empty key field for a configured assistant.
    let profile_ids: Vec<String> = settings.assistant_profiles.keys().cloned().collect();
    for id in profile_ids {
        if settings
            .assistant_api_keys
            .get(&id)
            .is_none_or(|v| v.is_empty())
        {
            if let Some(value) = get_secret(ASSISTANT_SCOPE, &id) {
                settings.assistant_api_keys.insert(id, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_cache_avoids_repeated_backend_reads_including_misses() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let reads = AtomicUsize::new(0);
        let load = || {
            reads.fetch_add(1, Ordering::Relaxed);
            None
        };

        assert_eq!(get_secret_cached_with("cache-test", "missing", load), None);
        assert_eq!(get_secret_cached_with("cache-test", "missing", load), None);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    /// The migration must leave empty-string entries untouched (they mean "not
    /// configured") and report no changes for an already-clean settings map.
    #[test]
    fn migration_ignores_empty_and_clean_maps() {
        let mut settings = crate::settings::get_default_settings();
        assert!(settings.speech_pipeline_api_keys.is_empty());
        assert!(!migrate_legacy_settings_secrets(&mut settings));
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn fallback_roundtrip_is_scoped() {
        // Only meaningful where the file fallback is the backing store.
        imp::set_secret("test-scope", "prov", "value-1").unwrap();
        assert_eq!(
            imp::get_secret("test-scope", "prov"),
            Some("value-1".into())
        );
        assert_eq!(imp::get_secret("other-scope", "prov"), None);
        imp::delete_secret("test-scope", "prov");
        assert_eq!(imp::get_secret("test-scope", "prov"), None);
    }

    /// Regression test for the headless ordering contract
    /// (secrets-exposure-and-durability-audit.md §3.2 / F-6, task gates G1/G2):
    /// a CLI-only session loads a legacy store whose API-key maps hold
    /// plain-text values. Running the migration *before* the (any) persist
    /// must leave the key maps that reach disk empty while every legacy value
    /// has landed in the secret store. Uses an injected backend so it never
    /// touches the real OS keychain.
    #[test]
    fn headless_migration_before_first_persist_empties_maps_and_fills_secret_store() {
        // What get_settings_unlocked deserializes from a not-yet-migrated
        // settings_store.json.
        let mut settings: crate::settings::AppSettings =
            serde_json::from_value(serde_json::json!({
                "speech_pipeline_api_keys": { "azure": "legacy-azure-key", "deepl": "" },
                "post_process_api_keys": { "openai": "legacy-openai-key" },
            }))
            .expect("partial stores parse with defaults");

        let backend: std::sync::Mutex<std::collections::HashMap<String, String>> =
            std::sync::Mutex::new(std::collections::HashMap::new());
        let migrated = {
            let backend = &backend;
            migrate_legacy_settings_secrets_with(
                &mut settings,
                |scope, id| backend.lock().unwrap().get(&account(scope, id)).cloned(),
                |scope, id, value| {
                    backend
                        .lock()
                        .unwrap()
                        .insert(account(scope, id), value.to_string());
                    Ok(())
                },
            )
        };
        assert!(migrated);

        // Whatever persists next (schema migrations, default merges — the F-6
        // trigger) writes the maps through SecretMap::serialize: empty on disk.
        let persisted = serde_json::to_value(&settings).unwrap();
        assert_eq!(persisted["speech_pipeline_api_keys"], serde_json::json!({}));
        assert_eq!(persisted["post_process_api_keys"], serde_json::json!({}));

        // ...and every configured legacy key now lives in the secret store,
        // under the same scope/account the GUI path and readers use.
        let stored = backend.lock().unwrap();
        assert_eq!(
            stored.get(&account(SPEECH_PIPELINE_SCOPE, "azure")),
            Some(&"legacy-azure-key".to_string())
        );
        assert_eq!(
            stored.get(&account(POST_PROCESS_SCOPE, "openai")),
            Some(&"legacy-openai-key".to_string())
        );
        // Empty-string entries mean "not configured" and stay out of the store.
        assert!(!stored.contains_key(&account(SPEECH_PIPELINE_SCOPE, "deepl")));
        assert_eq!(stored.len(), 2);
        drop(stored);

        // Idempotent: a second startup pass is a no-op and does not clobber
        // existing secrets with the (already removed) settings copies.
        let second = {
            let backend = &backend;
            migrate_legacy_settings_secrets_with(
                &mut settings,
                |scope, id| backend.lock().unwrap().get(&account(scope, id)).cloned(),
                |scope, id, value| {
                    backend
                        .lock()
                        .unwrap()
                        .insert(account(scope, id), value.to_string());
                    Ok(())
                },
            )
        };
        assert!(!second);
        assert_eq!(backend.lock().unwrap().len(), 2);
    }

    /// Counterexample pinning WHY the ordering matters: persisting a legacy
    /// store *without* running the migration first (the F-6 headless bug)
    /// strips the keys from disk via `SecretMap::serialize` while nothing ever
    /// reaches the secret store — an unrecoverable loss.
    #[test]
    fn persisting_before_migration_destroys_legacy_keys_unmigrated() {
        let settings: crate::settings::AppSettings = serde_json::from_value(serde_json::json!({
            "speech_pipeline_api_keys": { "azure": "lost-without-migration" },
        }))
        .unwrap();

        let backend: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        // No migration ran; the first read re-persists schema migrations:
        let persisted = serde_json::to_value(&settings).unwrap();
        assert_eq!(persisted["speech_pipeline_api_keys"], serde_json::json!({}));
        assert!(backend.is_empty(), "no secret was ever stored");
    }
}
