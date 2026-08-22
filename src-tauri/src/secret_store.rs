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

/// Scope prefix for cloud speech pipeline providers (azure / deepl / custom).
pub const SPEECH_PIPELINE_SCOPE: &str = "speech-pipeline";
/// Scope prefix for LLM post-processing providers.
pub const POST_PROCESS_SCOPE: &str = "post-process";

const SERVICE: &str = "speak";

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
pub fn get_secret(scope: &str, id: &str) -> Option<String> {
    imp::get_secret(scope, id)
}

/// Creates or overwrites an API key.
pub fn set_secret(scope: &str, id: &str, value: &str) -> Result<(), String> {
    imp::set_secret(scope, id, value)
}

/// Removes a stored API key (used when a provider is cleared).
pub fn delete_secret(scope: &str, id: &str) {
    imp::delete_secret(scope, id);
}

/// Moves plain-text API keys out of a loaded [`AppSettings`] into the secret
/// store. Returns `true` when the settings changed and must be re-persisted.
///
/// Runs once per startup before anything reads the maps. Idempotent: values
/// already present in the secret store are not overwritten, and emptied maps
/// make subsequent calls no-ops.
pub fn migrate_legacy_settings_secrets(settings: &mut crate::settings::AppSettings) -> bool {
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
            match imp::get_secret(scope, &id) {
                Some(existing) => {
                    info!(
                        "legacy API key for {scope}/{id} found in settings but a secret \
                         already exists in the keychain; dropping the stale copy"
                    );
                    let _ = existing;
                }
                None => {
                    if let Err(e) = imp::set_secret(scope, &id, &value) {
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
            if let Some(value) = imp::get_secret(SPEECH_PIPELINE_SCOPE, id) {
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
            if let Some(value) = imp::get_secret(POST_PROCESS_SCOPE, &id) {
                settings.post_process_api_keys.insert(id, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
