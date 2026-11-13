//! Keyboard shortcut management module
//!
//! This module provides a unified interface for keyboard shortcuts with
//! multiple backend implementations:
//!
//! - `tauri`: Uses Tauri's built-in global-shortcut plugin
//! - `handy_keys`: Uses the handy-keys library for more control
//!
//! The active implementation is determined by the `keyboard_implementation`
//! setting and can be changed at runtime.

mod handler;
pub mod handy_keys;
pub mod tauri_impl;

use std::collections::HashMap;

/// Meeting bindings are declared by the settings schema; re-exported here so
/// callers reason about "which tool owns this shortcut" in one place. The
/// routing types are the shortcut module's public vocabulary for the tray and
/// for tests, even where the crate does not consume them yet.
#[allow(unused_imports)]
pub use crate::settings::is_meeting_binding;
#[allow(unused_imports)]
pub use handler::{dispatch_meeting_action, route_binding, MeetingShortcutAction, ShortcutRoute};

use log::{debug, error, info, warn};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::settings::APPLE_INTELLIGENCE_DEFAULT_MODEL_ID;
use crate::settings::{
    self, get_settings, AssistantOutputMode, AutoSubmitKey, ClipboardHandling,
    KeyboardImplementation, LLMPrompt, OverlayPosition, OverlayStyle, PasteMethod,
    RecordingAudioMode, ShortcutBinding, SoundTheme, SpeechInputProvider, Theme, TypingTool,
    APPLE_INTELLIGENCE_PROVIDER_ID,
};
use crate::tray;

// Note: Commands are accessed via shortcut::handy_keys:: in lib.rs

/// Initialize shortcuts using the configured implementation
pub fn init_shortcuts(app: &AppHandle) {
    let user_settings = settings::load_or_create_app_settings(app);

    // Check which implementation to use
    match user_settings.keyboard_implementation {
        KeyboardImplementation::Tauri => {
            tauri_impl::init_shortcuts(app);
        }
        KeyboardImplementation::HandyKeys => {
            if let Err(e) = handy_keys::init_shortcuts(app) {
                error!("Failed to initialize handy-keys shortcuts: {}", e);
                // Fall back to Tauri implementation and persist this fallback
                warn!("Falling back to Tauri global shortcut implementation and saving fallback to settings");

                // Update settings to persist the fallback so we don't retry HandyKeys on next launch
                settings::update_settings(app, |settings| {
                    settings.keyboard_implementation = KeyboardImplementation::Tauri
                });

                tauri_impl::init_shortcuts(app);
            }
        }
    }
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    // Track recording lifecycle independently of the current implementation so
    // switching implementations mid-recording cannot leave stale fallback state.
    crate::secure_input::register_cancel_fallback(app);

    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::register_cancel_shortcut(app),
        KeyboardImplementation::HandyKeys => handy_keys::register_cancel_shortcut(app),
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    crate::secure_input::unregister_cancel_fallback(app);

    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::unregister_cancel_shortcut(app),
        KeyboardImplementation::HandyKeys => handy_keys::unregister_cancel_shortcut(app),
    }
}

/// Register a shortcut using the appropriate implementation
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // An empty binding means "deliberately unbound" (plan §6.9 lists the
    // meeting pause/resume action as unassigned by default). Registering an
    // empty accelerator would fail noisily in both implementations.
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::register_shortcut(app, binding),
        KeyboardImplementation::HandyKeys => handy_keys::register_shortcut(app, binding),
    }
}

/// Unregister a shortcut using the appropriate implementation
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::unregister_shortcut(app, binding),
        KeyboardImplementation::HandyKeys => handy_keys::unregister_shortcut(app, binding),
    }
}

// ============================================================================
// Binding Management Commands
// ============================================================================

#[derive(Serialize, Type)]
pub struct BindingResponse {
    success: bool,
    binding: Option<ShortcutBinding>,
    error: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn change_binding(
    app: AppHandle,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    // Reject empty bindings — every shortcut should have a value
    if binding.trim().is_empty() {
        return Err("Binding cannot be empty".to_string());
    }

    let settings = settings::get_settings(&app);

    // Get the binding to modify, or create it from defaults if it doesn't exist
    let binding_to_modify = match settings.bindings.get(&id) {
        Some(binding) => binding.clone(),
        None => {
            // Try to get the default binding for this id
            let default_settings = settings::get_default_settings();
            match default_settings.bindings.get(&id) {
                Some(default_binding) => {
                    warn!(
                        "Binding '{}' not found in settings, creating from defaults",
                        id
                    );
                    default_binding.clone()
                }
                None => {
                    let error_msg = format!("Binding with id '{}' not found in defaults", id);
                    warn!("change_binding error: {}", error_msg);
                    return Ok(BindingResponse {
                        success: false,
                        binding: None,
                        error: Some(error_msg),
                    });
                }
            }
        }
    };

    // If this is the cancel binding, just update the settings and return
    // It's managed dynamically, so we don't register/unregister here
    if id == "cancel" {
        if let Some(mut b) = settings.bindings.get(&id).cloned() {
            b.current_binding = binding;
            settings::update_settings(&app, |settings| {
                settings.bindings.insert(id.clone(), b.clone());
            });
            crate::secure_input::reconcile_fallback(&app);
            return Ok(BindingResponse {
                success: true,
                binding: Some(b.clone()),
                error: None,
            });
        }
    }

    // Unregister the existing binding
    if let Err(e) = unregister_shortcut(&app, binding_to_modify.clone()) {
        let error_msg = format!("Failed to unregister shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
    }

    // Validate the new shortcut for the current keyboard implementation
    if let Err(e) = validate_shortcut_for_implementation(&binding, settings.keyboard_implementation)
    {
        warn!("change_binding validation error: {}", e);
        restore_registration(&app, &binding_to_modify);
        return Err(e);
    }

    // Create an updated binding
    let mut updated_binding = binding_to_modify.clone();
    updated_binding.current_binding = binding;

    // Register the new binding
    if let Err(e) = register_shortcut(&app, updated_binding.clone()) {
        let error_msg = format!("Failed to register shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
        restore_registration(&app, &binding_to_modify);
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error_msg),
        });
    }

    // Save the settings and synchronize any active Secure Input shadows.
    settings::update_settings(&app, |settings| {
        settings.bindings.insert(id, updated_binding.clone());
    });
    crate::secure_input::reconcile_fallback(&app);

    // Return the updated binding
    Ok(BindingResponse {
        success: true,
        binding: Some(updated_binding),
        error: None,
    })
}

/// Best-effort re-register of the previous binding after a failed change,
/// so a failure leaves the user's shortcut working exactly as before.
fn restore_registration(app: &AppHandle, binding: &ShortcutBinding) {
    if let Err(e) = register_shortcut(app, binding.clone()) {
        error!(
            "Failed to restore previous binding '{}' ({}): {}",
            binding.id, binding.current_binding, e
        );
    }
}

#[tauri::command]
#[specta::specta]
pub fn reset_binding(app: AppHandle, id: String) -> Result<BindingResponse, String> {
    let binding = settings::get_stored_binding(&app, &id);
    change_binding(app, id, binding.default_binding)
}

/// Unregister every binding while the user is recording a new shortcut in
/// the UI, so no existing shortcut can fire — or swallow the keystrokes —
/// mid-capture. The "cancel" binding is untouched: it is managed dynamically
/// by the recording lifecycle.
pub fn suspend_all_shortcuts(app: &AppHandle) {
    for (id, binding) in settings::get_bindings(app) {
        if id == "cancel" {
            continue;
        }
        if let Err(e) = unregister_shortcut(app, binding) {
            debug!(
                "suspend_all_shortcuts: could not unregister '{}': {}",
                id, e
            );
        }
    }
}

/// Re-register every binding from settings after shortcut recording ends.
/// Registering an already-registered shortcut fails cleanly in both
/// implementations, so this is idempotent and safe on every exit path.
pub fn resume_all_shortcuts(app: &AppHandle) {
    let settings = get_settings(app);
    for (id, binding) in &settings.bindings {
        if id == "cancel" {
            continue;
        }
        if id == "transcribe_with_post_process" && !settings.post_process_enabled {
            continue;
        }
        if let Err(e) = register_shortcut(app, binding.clone()) {
            debug!("resume_all_shortcuts: could not register '{}': {}", id, e);
        }
    }
}

/// Temporarily unregister all bindings while the user is recording a
/// shortcut in the UI. This avoids firing actions while keys are recorded.
#[tauri::command]
#[specta::specta]
pub fn suspend_all_bindings(app: AppHandle) -> Result<(), String> {
    suspend_all_shortcuts(&app);
    Ok(())
}

/// Re-register all bindings after the user has finished recording.
#[tauri::command]
#[specta::specta]
pub fn resume_all_bindings(app: AppHandle) -> Result<(), String> {
    resume_all_shortcuts(&app);
    Ok(())
}

// ============================================================================
// Keyboard Implementation Switching
// ============================================================================

/// Result of changing keyboard implementation
#[derive(Serialize, Type)]
pub struct ImplementationChangeResult {
    pub success: bool,
    /// List of binding IDs that were reset to defaults due to incompatibility
    pub reset_bindings: Vec<String>,
}

/// Change the keyboard implementation with runtime switching.
/// This will unregister all shortcuts from the old implementation,
/// validate shortcuts for the new implementation (resetting invalid ones to defaults),
/// and register them with the new implementation.
#[tauri::command]
#[specta::specta]
pub fn change_keyboard_implementation_setting(
    app: AppHandle,
    implementation: String,
) -> Result<ImplementationChangeResult, String> {
    let current_settings = settings::get_settings(&app);
    let current_impl = current_settings.keyboard_implementation;
    let new_impl = parse_keyboard_implementation(&implementation);

    // If same implementation, nothing to do
    if current_impl == new_impl {
        return Ok(ImplementationChangeResult {
            success: true,
            reset_bindings: vec![],
        });
    }

    info!(
        "Switching keyboard implementation from {:?} to {:?}",
        current_impl, new_impl
    );

    // Unregister all shortcuts from the current implementation
    unregister_all_shortcuts(&app, current_impl);

    // Update the setting
    settings::update_settings(&app, |settings| settings.keyboard_implementation = new_impl);

    // Carbon fallback registrations use the Tauri plugin. Remove them before
    // registering the full Tauri implementation to avoid duplicate conflicts.
    if new_impl == KeyboardImplementation::Tauri {
        crate::secure_input::reconcile_fallback(&app);
    }

    // Initialize new implementation if needed (HandyKeys needs state)
    if new_impl == KeyboardImplementation::HandyKeys && initialize_handy_keys_with_rollback(&app)? {
        // Shortcuts already registered during init.
        crate::secure_input::reconcile_fallback(&app);
        return Ok(ImplementationChangeResult {
            success: true,
            reset_bindings: vec![],
        });
    }

    // Register all shortcuts with new implementation, resetting invalid ones
    let reset_bindings = register_all_shortcuts_for_implementation(&app, new_impl);
    crate::secure_input::reconcile_fallback(&app);

    // Emit event to notify frontend of the change
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "keyboard_implementation",
            "value": implementation,
            "reset_bindings": reset_bindings
        }),
    );

    info!("Keyboard implementation switched to {:?}", new_impl);

    Ok(ImplementationChangeResult {
        success: true,
        reset_bindings,
    })
}

/// Get the current keyboard implementation
#[tauri::command]
#[specta::specta]
pub fn get_keyboard_implementation(app: AppHandle) -> String {
    let settings = settings::get_settings(&app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => "tauri".to_string(),
        KeyboardImplementation::HandyKeys => "handy_keys".to_string(),
    }
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate a shortcut for a specific implementation
fn validate_shortcut_for_implementation(
    raw: &str,
    implementation: KeyboardImplementation,
) -> Result<(), String> {
    match implementation {
        KeyboardImplementation::Tauri => tauri_impl::validate_shortcut(raw),
        KeyboardImplementation::HandyKeys => handy_keys::validate_shortcut(raw),
    }
}

/// Parse a keyboard implementation string into the enum
fn parse_keyboard_implementation(s: &str) -> KeyboardImplementation {
    match s {
        "tauri" => KeyboardImplementation::Tauri,
        "handy_keys" => KeyboardImplementation::HandyKeys,
        other => {
            warn!(
                "Invalid keyboard implementation '{}', defaulting to tauri",
                other
            );
            KeyboardImplementation::Tauri
        }
    }
}

/// Unregister all shortcuts for the current implementation
fn unregister_all_shortcuts(app: &AppHandle, implementation: KeyboardImplementation) {
    let bindings = settings::get_bindings(app);

    for (id, binding) in bindings {
        // Skip cancel shortcut as it's dynamically registered
        if id == "cancel" {
            continue;
        }

        let result = match implementation {
            KeyboardImplementation::Tauri => tauri_impl::unregister_shortcut(app, binding),
            KeyboardImplementation::HandyKeys => handy_keys::unregister_shortcut(app, binding),
        };

        if let Err(e) = result {
            warn!(
                "Failed to unregister shortcut '{}' during switch: {}",
                id, e
            );
        }
    }
}

/// Register all shortcuts for a specific implementation, validating and resetting invalid ones
fn register_all_shortcuts_for_implementation(
    app: &AppHandle,
    implementation: KeyboardImplementation,
) -> Vec<String> {
    let mut reset_bindings = Vec::new();
    let default_bindings = settings::get_default_settings().bindings;
    let mut current_settings = settings::get_settings(app);

    for (id, default_binding) in &default_bindings {
        // Skip cancel shortcut as it's dynamically registered
        if id == "cancel" {
            continue;
        }

        // Skip post-processing shortcut when the feature is disabled
        if id == "transcribe_with_post_process" && !current_settings.post_process_enabled {
            continue;
        }

        let mut binding = current_settings
            .bindings
            .get(id)
            .cloned()
            .unwrap_or_else(|| default_binding.clone());

        // Validate the shortcut for the target implementation
        if let Err(e) =
            validate_shortcut_for_implementation(&binding.current_binding, implementation)
        {
            info!(
                "Shortcut '{}' ({}) is invalid for {:?}: {}. Resetting to default.",
                id, binding.current_binding, implementation, e
            );

            // Reset to default
            binding.current_binding = default_binding.current_binding.clone();
            current_settings
                .bindings
                .insert(id.clone(), binding.clone());
            reset_bindings.push(id.clone());
        }

        // Register with the appropriate implementation
        let result = match implementation {
            KeyboardImplementation::Tauri => tauri_impl::register_shortcut(app, binding),
            KeyboardImplementation::HandyKeys => handy_keys::register_shortcut(app, binding),
        };

        if let Err(e) = result {
            error!(
                "Failed to register shortcut '{}' for {:?}: {}",
                id, implementation, e
            );
        }
    }

    // Save settings if any bindings were reset
    if !reset_bindings.is_empty() {
        settings::update_settings(app, |settings| {
            for id in &reset_bindings {
                if let Some(reset_binding) = current_settings.bindings.get(id) {
                    settings.bindings.insert(id.clone(), reset_binding.clone());
                }
            }
        });
    }

    reset_bindings
}

/// Initialize HandyKeys if not already initialized, with rollback on failure
fn initialize_handy_keys_with_rollback(app: &AppHandle) -> Result<bool, String> {
    if app.try_state::<handy_keys::HandyKeysState>().is_some() {
        return Ok(false); // Already initialized, caller should continue
    }

    if let Err(e) = handy_keys::init_shortcuts(app) {
        error!("Failed to initialize HandyKeys: {}", e);
        // Rollback to Tauri
        settings::update_settings(app, |settings| {
            settings.keyboard_implementation = KeyboardImplementation::Tauri
        });
        crate::secure_input::reconcile_fallback(app);
        tauri_impl::init_shortcuts(app);
        return Err(format!(
            "Failed to initialize HandyKeys: {}. Reverted to Tauri.",
            e
        ));
    }

    // init_shortcuts already registered shortcuts
    Ok(true)
}

// ============================================================================
// General Settings Commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub fn change_ptt_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.push_to_talk = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.audio_feedback = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_volume_setting(app: AppHandle, volume: f32) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.audio_feedback_volume = volume);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_sound_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let parsed = match theme.as_str() {
        "marimba" => SoundTheme::Marimba,
        "pop" => SoundTheme::Pop,
        "custom" => SoundTheme::Custom,
        other => {
            warn!("Invalid sound theme '{}', defaulting to marimba", other);
            SoundTheme::Marimba
        }
    };
    settings::update_settings(&app, |settings| settings.sound_theme = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let parsed = match theme.as_str() {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        other => {
            warn!("Invalid theme '{}', defaulting to system", other);
            Theme::System
        }
    };
    settings::update_settings(&app, |settings| settings.theme = parsed);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    apply_window_theme(&app, parsed);
    // Notify other webviews (the recording overlay) so they re-apply the palette
    // live — they set `data-theme` on their own document and can't see this one.
    let _ = app.emit(crate::events::THEME_CHANGED, parsed);
    Ok(())
}

/// Applies the appearance setting to the native window chrome (title bar), which
/// CSS `data-theme` cannot reach. `System` clears the override so the window
/// follows the OS. Call this on startup and whenever the setting changes to keep
/// the title bar in sync with the in-app palette.
///
/// On Windows this themes the title bar only. On macOS `set_theme` sets
/// `NSApp.appearance` app-wide, which is what we want here: it darkens the title
/// bar and keeps the overlay in step. Linux is left to `data-theme` alone, since
/// its window theming is backend-dependent and unreliable.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn apply_window_theme(app: &AppHandle, theme: Theme) {
    let window_theme = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.set_theme(window_theme) {
            warn!("Failed to apply window theme: {}", e);
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_translate_to_english_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.translate_to_english = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_selected_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.selected_language = language);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_speech_input_provider_setting(
    app: AppHandle,
    provider: SpeechInputProvider,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.speech_input_provider = provider);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_stt_provider_setting(
    app: AppHandle,
    provider: SpeechInputProvider,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.meeting_stt_provider = provider);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_azure_speech_endpoint_setting(
    app: AppHandle,
    endpoint: String,
) -> Result<(), String> {
    let endpoint = endpoint.trim().to_string();
    settings::update_settings(&app, |settings| settings.azure_speech_endpoint = endpoint);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_azure_speech_language_setting(
    app: AppHandle,
    language: String,
) -> Result<(), String> {
    let language = language.trim().to_string();
    settings::update_settings(&app, |settings| settings.azure_speech_language = language);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_azure_stable_partial_threshold_setting(
    app: AppHandle,
    threshold: u32,
) -> Result<(), String> {
    if !(1..=5).contains(&threshold) {
        return Err("Azure stable partial threshold must be between 1 and 5".to_string());
    }
    settings::update_settings(&app, |settings| {
        settings.azure_stable_partial_threshold = threshold
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_azure_segmentation_silence_setting(
    app: AppHandle,
    milliseconds: u32,
) -> Result<(), String> {
    if !(300..=5_000).contains(&milliseconds) {
        return Err("Azure phrase silence must be between 300 and 5000 ms".to_string());
    }
    settings::update_settings(&app, |settings| {
        settings.azure_segmentation_silence_ms = milliseconds
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_custom_stt_endpoint_setting(app: AppHandle, endpoint: String) -> Result<(), String> {
    let endpoint = endpoint.trim().to_string();
    settings::update_settings(&app, |settings| settings.custom_stt_endpoint = endpoint);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_speech_pipeline_api_key_setting(
    _app: AppHandle,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    if !matches!(provider_id.as_str(), "azure" | "custom" | "deepl") {
        return Err(format!(
            "Unsupported speech pipeline provider: {provider_id}"
        ));
    }
    // Keys never enter the settings store; they live in the OS keychain.
    if api_key.trim().is_empty() {
        crate::secret_store::delete_secret(
            crate::secret_store::SPEECH_PIPELINE_SCOPE,
            &provider_id,
        );
        Ok(())
    } else {
        crate::secret_store::set_secret(
            crate::secret_store::SPEECH_PIPELINE_SCOPE,
            &provider_id,
            &api_key,
        )
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_deepl_translation_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.deepl_translation_enabled = enabled
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_deepl_target_language_setting(
    app: AppHandle,
    language: String,
) -> Result<(), String> {
    let language = language.trim().to_uppercase();
    settings::update_settings(&app, |settings| settings.deepl_target_language = language);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_deepl_sentence_delay_setting(
    app: AppHandle,
    milliseconds: u64,
) -> Result<(), String> {
    if milliseconds > 10_000 {
        return Err("DeepL sentence delay must not exceed 10000 ms".to_string());
    }
    settings::update_settings(&app, |settings| {
        settings.deepl_sentence_delay_ms = milliseconds
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_position_setting(app: AppHandle, position: String) -> Result<(), String> {
    let parsed = match position.as_str() {
        // "none" is retired (visibility is overlay_style now); fold legacy callers
        // onto Bottom rather than warn.
        "none" | "bottom" => OverlayPosition::Bottom,
        "top" => OverlayPosition::Top,
        other => {
            warn!("Invalid overlay position '{}', defaulting to bottom", other);
            OverlayPosition::Bottom
        }
    };
    settings::update_settings(&app, |settings| settings.overlay_position = parsed);

    // Whether the overlay shows at all is owned by overlay_style now; position
    // only ever toggles Top/Bottom, so the enabled cache is untouched here.
    // Update overlay position without recreating window
    crate::utils::update_overlay_position(&app);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_style_setting(app: AppHandle, style: String) -> Result<(), String> {
    let parsed = match style.as_str() {
        "none" => OverlayStyle::None,
        "minimal" => OverlayStyle::Minimal,
        "live" => OverlayStyle::Live,
        other => {
            warn!("Invalid overlay style '{}', defaulting to minimal", other);
            OverlayStyle::Minimal
        }
    };
    settings::update_settings(&app, |settings| settings.overlay_style = parsed);

    // Keep the cached overlay-enabled flag in sync so emit_levels stops (or
    // resumes) emitting on the next audio callback.
    crate::overlay::update_overlay_enabled_cache(parsed != OverlayStyle::None);

    // "Overlay off" has to mean off: an open answer card would otherwise
    // survive the setting, because the card deliberately blocks every other
    // hide path, and the early return for `None` in `show_overlay_state`
    // never gets to release it.
    if parsed == OverlayStyle::None {
        crate::overlay::hide_answer_overlay(&app);
    }

    // Reposition in case the window needs to re-center for the new style.
    crate::utils::update_overlay_position(&app);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_debug_mode_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.debug_mode = enabled);

    // Keep webview log streaming in sync: the live log viewer only exists in
    // debug mode, so logs are forwarded to the frontend only while it is on.
    crate::logging::set_webview_log_streaming(enabled);

    // Emit event to notify frontend of debug mode change
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "debug_mode",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_start_hidden_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.start_hidden = enabled);

    // Notify frontend
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "start_hidden",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_autostart_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.autostart_enabled = enabled);

    // Apply the autostart setting immediately
    crate::autostart::apply_autostart(&app, enabled);

    // Notify frontend
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "autostart_enabled",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_update_checks_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.update_checks_enabled = enabled);

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "update_checks_enabled",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_whats_new_on_update_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.show_whats_new_on_update = enabled);

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "show_whats_new_on_update",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_whats_new_last_seen_version_setting(
    app: AppHandle,
    version: String,
) -> Result<(), String> {
    let version = version.trim().to_string();
    settings::update_settings(&app, |settings| {
        settings.whats_new_last_seen_version = version.clone()
    });

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "whats_new_last_seen_version",
            "value": version
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn update_custom_words(app: AppHandle, words: Vec<String>) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.custom_words = words);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_word_correction_threshold_setting(
    app: AppHandle,
    threshold: f64,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.word_correction_threshold = threshold
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_extra_recording_buffer_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.extra_recording_buffer_ms = ms);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.paste_delay_ms = ms);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_after_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.paste_delay_after_ms = ms);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_reliable_paste_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.reliable_paste = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_method_setting(app: AppHandle, method: String) -> Result<(), String> {
    let parsed = match method.as_str() {
        "ctrl_v" => PasteMethod::CtrlV,
        "direct" => PasteMethod::Direct,
        "none" => PasteMethod::None,
        "shift_insert" => PasteMethod::ShiftInsert,
        "ctrl_shift_v" => PasteMethod::CtrlShiftV,
        "external_script" => PasteMethod::ExternalScript,
        other => {
            warn!("Invalid paste method '{}', defaulting to ctrl_v", other);
            PasteMethod::CtrlV
        }
    };
    settings::update_settings(&app, |settings| settings.paste_method = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_available_typing_tools() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        crate::clipboard::get_available_typing_tools()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec!["auto".to_string()]
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_typing_tool_setting(app: AppHandle, tool: String) -> Result<(), String> {
    let parsed = match tool.as_str() {
        "auto" => TypingTool::Auto,
        "wtype" => TypingTool::Wtype,
        "kwtype" => TypingTool::Kwtype,
        "dotool" => TypingTool::Dotool,
        "ydotool" => TypingTool::Ydotool,
        "xdotool" => TypingTool::Xdotool,
        other => {
            warn!("Invalid typing tool '{}', defaulting to auto", other);
            TypingTool::Auto
        }
    };
    settings::update_settings(&app, |settings| settings.typing_tool = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_external_script_path_setting(
    app: AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.external_script_path = path);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_handling_setting(app: AppHandle, handling: String) -> Result<(), String> {
    let parsed = match handling.as_str() {
        "dont_modify" => ClipboardHandling::DontModify,
        "copy_to_clipboard" => ClipboardHandling::CopyToClipboard,
        other => {
            warn!(
                "Invalid clipboard handling '{}', defaulting to dont_modify",
                other
            );
            ClipboardHandling::DontModify
        }
    };
    settings::update_settings(&app, |settings| settings.clipboard_handling = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.auto_submit = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_key_setting(app: AppHandle, key: String) -> Result<(), String> {
    let parsed = match key.as_str() {
        "enter" => AutoSubmitKey::Enter,
        "ctrl_enter" => AutoSubmitKey::CtrlEnter,
        "cmd_enter" => AutoSubmitKey::CmdEnter,
        other => {
            warn!("Invalid auto submit key '{}', defaulting to enter", other);
            AutoSubmitKey::Enter
        }
    };
    settings::update_settings(&app, |settings| settings.auto_submit_key = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let binding = settings::update_settings(&app, |settings| {
        settings.post_process_enabled = enabled;
        settings
            .bindings
            .get("transcribe_with_post_process")
            .cloned()
    });

    // Register or unregister the post-processing shortcut
    if let Some(binding) = binding {
        if enabled {
            let _ = register_shortcut(&app, binding);
        } else {
            let _ = unregister_shortcut(&app, binding);
        }
    }

    crate::secure_input::reconcile_fallback(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_experimental_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.experimental_enabled = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_base_url_setting(
    app: AppHandle,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        let label = settings
            .post_process_provider(&provider_id)
            .map(|provider| provider.label.clone())
            .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

        let provider = settings
            .post_process_provider_mut(&provider_id)
            .expect("Provider looked up above must exist");

        if provider.id != "custom" {
            return Err(format!(
                "Provider '{}' does not allow editing the base URL",
                label
            ));
        }

        provider.base_url = base_url;
        Ok(())
    })
}

/// Generic helper to validate provider exists
fn validate_provider_exists(
    settings: &settings::AppSettings,
    provider_id: &str,
) -> Result<(), String> {
    if !settings
        .post_process_providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        return Err(format!("Provider '{}' not found", provider_id));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_api_key_setting(
    app: AppHandle,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    validate_provider_exists(&settings, &provider_id)?;
    // Keys never enter the settings store; they live in the OS keychain.
    if api_key.trim().is_empty() {
        crate::secret_store::delete_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider_id);
        Ok(())
    } else {
        crate::secret_store::set_secret(
            crate::secret_store::POST_PROCESS_SCOPE,
            &provider_id,
            &api_key,
        )
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_model_setting(
    app: AppHandle,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_provider_exists(settings, &provider_id)?;
        settings.post_process_models.insert(provider_id, model);
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_post_process_provider(app: AppHandle, provider_id: String) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_provider_exists(settings, &provider_id)?;
        settings.post_process_provider_id = provider_id;
        Ok(())
    })
}

/* ---------- assistant profiles ------------------------------------------ */

/// Generic helper to validate an assistant profile exists.
///
/// Every assistant command goes through this: profiles are addressed by a
/// string from the frontend, and silently creating one on a typo would leave a
/// profile no UI ever shows and no shortcut ever reaches.
fn validate_profile_exists(
    settings: &settings::AppSettings,
    profile_id: &str,
) -> Result<(), String> {
    if settings.assistant_profile(profile_id).is_none() {
        return Err(format!("Assistant profile '{}' not found", profile_id));
    }
    Ok(())
}

/// Points an assistant at a different provider.
///
/// A profile stores exactly one model id, so switching providers resets it —
/// keeping the old one would leave a name the new endpoint rejects. Selecting
/// the provider that is already set is a no-op and preserves the model.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_provider(
    app: AppHandle,
    profile_id: String,
    provider_id: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_profile_exists(settings, &profile_id)?;
        validate_provider_exists(settings, &provider_id)?;
        let default_model = settings::default_model_for_provider(&provider_id);
        let profile = settings
            .assistant_profile_mut(&profile_id)
            .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;
        if profile.provider_id != provider_id {
            profile.provider_id = provider_id;
            profile.model = default_model;
        }
        Ok(())
    })
}

/// Stores an assistant's API key in the OS keychain under
/// [`crate::secret_store::ASSISTANT_SCOPE`], keyed by profile id — never in the
/// settings store, and never shared with post-processing.
#[tauri::command]
#[specta::specta]
pub fn change_assistant_api_key_setting(
    app: AppHandle,
    profile_id: String,
    api_key: String,
) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    validate_profile_exists(&settings, &profile_id)?;
    if api_key.trim().is_empty() {
        crate::secret_store::delete_secret(crate::secret_store::ASSISTANT_SCOPE, &profile_id);
        Ok(())
    } else {
        crate::secret_store::set_secret(crate::secret_store::ASSISTANT_SCOPE, &profile_id, &api_key)
    }
}

/// Reports whether each assistant profile has a usable credential without
/// ever serializing the credential itself across IPC.
///
/// `SecretMap` intentionally serializes as an empty object so secrets cannot
/// leak into settings JSON or the webview. The UI therefore needs this
/// separate boolean-only view to distinguish "stored securely" from "missing".
#[tauri::command]
#[specta::specta]
pub fn get_assistant_api_key_status(app: AppHandle) -> Result<HashMap<String, bool>, String> {
    let settings = settings::get_settings(&app);
    Ok(settings
        .assistant_profiles
        .keys()
        .map(|profile_id| {
            let configured =
                crate::secret_store::get_secret(crate::secret_store::ASSISTANT_SCOPE, profile_id)
                    .is_some_and(|value| !value.trim().is_empty());
            (profile_id.clone(), configured)
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub fn change_assistant_model_setting(
    app: AppHandle,
    profile_id: String,
    model: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_profile_exists(settings, &profile_id)?;
        let profile = settings
            .assistant_profile_mut(&profile_id)
            .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;
        profile.model = model.trim().to_string();
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn change_assistant_system_prompt_setting(
    app: AppHandle,
    profile_id: String,
    prompt: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_profile_exists(settings, &profile_id)?;
        let profile = settings
            .assistant_profile_mut(&profile_id)
            .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;
        profile.system_prompt = prompt;
        Ok(())
    })
}

/// Sets answer length and request deadline together — they are one decision
/// ("how much work may this assistant do") and are clamped to the ranges the
/// schema enforces on load, so the UI cannot store a value the repair pass
/// would immediately rewrite.
#[tauri::command]
#[specta::specta]
pub fn change_assistant_limits_setting(
    app: AppHandle,
    profile_id: String,
    max_output_tokens: u32,
    timeout_secs: u32,
) -> Result<(), String> {
    let tokens = settings::clamp_assistant_output_tokens(max_output_tokens);
    let timeout = settings::clamp_assistant_timeout_secs(timeout_secs);
    settings::try_update_settings(&app, |settings| {
        validate_profile_exists(settings, &profile_id)?;
        let profile = settings
            .assistant_profile_mut(&profile_id)
            .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;
        profile.max_output_tokens = tokens;
        profile.timeout_secs = timeout;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_output_mode(
    app: AppHandle,
    profile_id: String,
    mode: AssistantOutputMode,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_profile_exists(settings, &profile_id)?;
        let profile = settings
            .assistant_profile_mut(&profile_id)
            .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;
        profile.output_mode = mode;
        Ok(())
    })
}

/// List the models the given assistant profile can actually reach.
///
/// Deliberately not `fetch_post_process_models`: that one reads the key stored
/// under [`crate::secret_store::POST_PROCESS_SCOPE`] for the provider, while an
/// assistant's credential lives under
/// [`crate::secret_store::ASSISTANT_SCOPE`] keyed by *profile* id. Reusing the
/// post-processing path made the dropdown permanently empty for anyone who had
/// only entered an assistant key — the list was asking a different credential
/// whether it could see any models.
#[tauri::command]
#[specta::specta]
pub async fn fetch_assistant_models(
    app: AppHandle,
    profile_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(&app);

    let provider_id = settings
        .assistant_profile(&profile_id)
        .map(|profile| profile.provider_id.clone())
        .ok_or_else(|| format!("Assistant profile '{}' not found", profile_id))?;

    let provider = settings
        .post_process_provider(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Ok(vec![APPLE_INTELLIGENCE_DEFAULT_MODEL_ID.to_string()]);
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            return Err("Apple Intelligence is only available on Apple silicon Macs running macOS 15 or later.".to_string());
        }
    }

    let api_key =
        crate::secret_store::get_secret(crate::secret_store::ASSISTANT_SCOPE, &profile_id)
            .unwrap_or_default();

    // Locally hosted providers authenticate nothing; everything else needs the
    // key before it will answer, and saying so beats an empty dropdown.
    if api_key.trim().is_empty()
        && provider.id != "custom"
        && provider.id != settings::LOCAL_MEETING_SUMMARY_PROVIDER_ID
    {
        return Err(format!(
            "Add this assistant's own API key for {} before listing its models.",
            provider.label
        ));
    }

    crate::llm_client::fetch_models(provider, api_key).await
}

#[tauri::command]
#[specta::specta]
pub fn set_meeting_summary_provider(app: AppHandle, provider_id: String) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_provider_exists(settings, &provider_id)?;
        if !matches!(
            provider_id.as_str(),
            settings::LOCAL_MEETING_SUMMARY_PROVIDER_ID | settings::OPENROUTER_PROVIDER_ID
        ) {
            return Err("Meeting summaries support Local GPU (Ollama) or OpenRouter".to_string());
        }
        settings.meeting_summary_provider_id = provider_id;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_summary_model_setting(
    app: AppHandle,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        validate_provider_exists(settings, &provider_id)?;
        if !matches!(
            provider_id.as_str(),
            settings::LOCAL_MEETING_SUMMARY_PROVIDER_ID | settings::OPENROUTER_PROVIDER_ID
        ) {
            return Err("Unsupported meeting summary provider".to_string());
        }
        settings
            .meeting_summary_models
            .insert(provider_id, model.trim().to_string());
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_summary_cloud_consent_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.meeting_summary_cloud_consent = enabled
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn add_post_process_prompt(
    app: AppHandle,
    name: String,
    prompt: String,
) -> Result<LLMPrompt, String> {
    // Generate unique ID using timestamp and random component
    let id = format!("prompt_{}", chrono::Utc::now().timestamp_millis());

    let new_prompt = LLMPrompt {
        id: id.clone(),
        name,
        prompt,
    };

    settings::update_settings(&app, |settings| {
        settings.post_process_prompts.push(new_prompt.clone())
    });

    Ok(new_prompt)
}

#[tauri::command]
#[specta::specta]
pub fn update_post_process_prompt(
    app: AppHandle,
    id: String,
    name: String,
    prompt: String,
) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        let existing_prompt = settings
            .post_process_prompts
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Prompt with id '{}' not found", id))?;
        existing_prompt.name = name;
        existing_prompt.prompt = prompt;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_post_process_prompt(app: AppHandle, id: String) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        // Don't allow deleting the last prompt
        if settings.post_process_prompts.len() <= 1 {
            return Err("Cannot delete the last prompt".to_string());
        }

        let original_len = settings.post_process_prompts.len();
        settings.post_process_prompts.retain(|p| p.id != id);
        if settings.post_process_prompts.len() == original_len {
            return Err(format!("Prompt with id '{}' not found", id));
        }

        if settings.post_process_selected_prompt_id.as_ref() == Some(&id) {
            settings.post_process_selected_prompt_id =
                settings.post_process_prompts.first().map(|p| p.id.clone());
        }
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_post_process_models(
    app: AppHandle,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(&app);

    // Find the provider
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Ok(vec![APPLE_INTELLIGENCE_DEFAULT_MODEL_ID.to_string()]);
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            return Err("Apple Intelligence is only available on Apple silicon Macs running macOS 15 or later.".to_string());
        }
    }

    // Get API key
    let api_key =
        crate::secret_store::get_secret(crate::secret_store::POST_PROCESS_SCOPE, &provider_id)
            .unwrap_or_default();

    // Skip fetching if no API key for providers that typically need one
    if api_key.trim().is_empty()
        && provider.id != "custom"
        && provider.id != settings::LOCAL_MEETING_SUMMARY_PROVIDER_ID
    {
        return Err(format!(
            "API key is required for {}. Please add an API key to list available models.",
            provider.label
        ));
    }

    crate::llm_client::fetch_models(provider, api_key).await
}

#[tauri::command]
#[specta::specta]
pub fn set_post_process_selected_prompt(app: AppHandle, id: String) -> Result<(), String> {
    settings::try_update_settings(&app, |settings| {
        if !settings.post_process_prompts.iter().any(|p| p.id == id) {
            return Err(format!("Prompt with id '{}' not found", id));
        }
        settings.post_process_selected_prompt_id = Some(id);
        Ok(())
    })
}

/// Legacy toggle, kept working for callers (and stores) that predate
/// `recording_audio_mode`. It now writes *both* fields, so the bool and the
/// mode can never disagree: enabling it means Mute, disabling it means Off.
///
/// Note the asymmetry this implies — turning it off from Duck also lands on
/// Off, which is the honest reading of "don't touch my audio while recording".
#[tauri::command]
#[specta::specta]
pub fn change_mute_while_recording_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.recording_audio_mode = RecordingAudioMode::from_legacy_mute_flag(enabled);
        settings.mute_while_recording = enabled;
    });
    Ok(())
}

/// Sets what happens to system audio while recording.
///
/// Write-through mirror (plan decision E3): `mute_while_recording` is rewritten
/// from the new mode on every change, so the legacy field never goes stale and
/// an older build reading the same store still behaves sanely.
#[tauri::command]
#[specta::specta]
pub fn change_recording_audio_mode_setting(
    app: AppHandle,
    mode: RecordingAudioMode,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.recording_audio_mode = mode;
        settings.mute_while_recording = mode.mirrors_mute_flag();
    });
    Ok(())
}

/// Sets the ducking target as a percentage of full output volume.
/// Out-of-range input is clamped rather than rejected — a slider is the only
/// caller, and silently correcting beats failing a settings write.
#[tauri::command]
#[specta::specta]
pub fn change_duck_volume_percent_setting(app: AppHandle, percent: u8) -> Result<(), String> {
    let clamped = settings::clamp_duck_volume_percent(percent);
    settings::update_settings(&app, |settings| settings.duck_volume_percent = clamped);
    Ok(())
}

/// Sets the duck/restore ramp length. Clamped to 1s: anything longer would
/// still be ramping well after a short dictation has ended.
#[tauri::command]
#[specta::specta]
pub fn change_duck_fade_ms_setting(app: AppHandle, ms: u16) -> Result<(), String> {
    let clamped = settings::clamp_duck_fade_ms(ms);
    settings::update_settings(&app, |settings| settings.duck_fade_ms = clamped);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_append_trailing_space_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.append_trailing_space = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_lazy_stream_close_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.lazy_stream_close = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_vad_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.vad_enabled = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_filler_word_removal_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.filler_word_removal_enabled = enabled
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_app_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.app_language = language.clone());

    // Refresh the tray menu with the new language
    tray::update_tray_menu(&app, Some(&language));

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_tray_icon_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.show_tray_icon = enabled);

    // Apply change immediately
    tray::set_tray_visibility(&app, enabled);

    Ok(())
}

/// Save accelerator settings and make the next model use reload with them.
/// The currently running transcription, if any, keeps its existing engine.
fn reload_model_on_next_use(app: &AppHandle) {
    let tm = app.state::<std::sync::Arc<crate::managers::transcription::TranscriptionManager>>();
    tm.reload_model_on_next_use();
}

#[tauri::command]
#[specta::specta]
pub fn change_transcribe_accelerator_setting(
    app: AppHandle,
    accelerator: settings::TranscribeAcceleratorSetting,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| {
        settings.transcribe_accelerator = accelerator
    });
    reload_model_on_next_use(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_ort_accelerator_setting(
    app: AppHandle,
    accelerator: settings::OrtAcceleratorSetting,
) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.ort_accelerator = accelerator);
    reload_model_on_next_use(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_transcribe_gpu_device(app: AppHandle, device: Option<String>) -> Result<(), String> {
    settings::update_settings(&app, |settings| settings.transcribe_gpu_device = device);
    reload_model_on_next_use(&app);
    Ok(())
}

/// Return which accelerators and GPU devices are available for this build.
///
/// First-call cost is dominated by enumerating GPU devices through the
/// transcribe.cpp Metal/Vulkan backend, which loads dynamic libraries and
/// probes hardware. Run it on the blocking pool so the webview thread
/// stays responsive — see also the startup pre-warm in `lib.rs`.
#[tauri::command]
#[specta::specta]
pub async fn get_available_accelerators() -> crate::managers::transcription::AvailableAccelerators {
    tauri::async_runtime::spawn_blocking(crate::managers::transcription::get_available_accelerators)
        .await
        .expect("get_available_accelerators panicked")
}
