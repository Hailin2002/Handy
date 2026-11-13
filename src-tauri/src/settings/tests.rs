//! Tests for settings parsing, salvage, migrations, and redaction.

use std::collections::HashMap;

use super::schema::*;
use super::store::*;
use super::*;

fn default_settings_json() -> serde_json::Value {
    serde_json::to_value(get_default_settings()).unwrap()
}

/// Every field must survive a partial store: a missing key must never fail
/// the whole-settings parse (#1619). `json!({})` is the extreme case.
#[test]
fn empty_store_parses_with_defaults() {
    let settings: AppSettings = serde_json::from_value(serde_json::json!({}))
        .expect("all AppSettings fields need serde defaults");
    assert!(settings.push_to_talk);
    assert!(!settings.audio_feedback);
    assert!(settings.filler_word_removal_enabled);
    // Bindings default to empty; the load path merges the real defaults in.
    assert!(settings.bindings.is_empty());
}

#[test]
fn meeting_summaries_default_to_a_small_local_model() {
    let settings = get_default_settings();
    assert_eq!(
        settings.meeting_summary_provider_id,
        LOCAL_MEETING_SUMMARY_PROVIDER_ID
    );
    assert_eq!(
        settings
            .meeting_summary_models
            .get(LOCAL_MEETING_SUMMARY_PROVIDER_ID)
            .map(String::as_str),
        Some("qwen2.5:3b")
    );
    let provider = settings
        .meeting_summary_provider()
        .expect("local meeting summary provider must exist");
    assert_eq!(provider.base_url, "http://127.0.0.1:11434/v1");
    assert!(!settings.meeting_summary_cloud_consent);
}

#[test]
fn meeting_summary_models_keep_local_and_openrouter_choices_separate() {
    let mut settings = get_default_settings();
    settings.meeting_summary_models.insert(
        OPENROUTER_PROVIDER_ID.to_string(),
        "vendor/model".to_string(),
    );
    settings.meeting_summary_provider_id = OPENROUTER_PROVIDER_ID.to_string();

    assert_eq!(
        settings
            .meeting_summary_models
            .get(LOCAL_MEETING_SUMMARY_PROVIDER_ID)
            .map(String::as_str),
        Some("qwen2.5:3b")
    );
    assert_eq!(
        settings
            .meeting_summary_models
            .get(OPENROUTER_PROVIDER_ID)
            .map(String::as_str),
        Some("vendor/model")
    );
}

/// Frozen snapshot of a real v0.9.0-era settings store, as written to
/// disk. This pins backwards compatibility: it must always parse strictly
/// (no salvage). Schema migrations may then rewrite fields whose native
/// meaning changed.
///
/// If a schema change breaks this test, do NOT just update the fixture —
/// it stands in for the stores on users' machines. Add a
/// `#[serde(alias)]`/`#[serde(other)]` or a one-time migration in
/// `apply_settings_migrations` so old values keep loading, and only extend
/// the fixture alongside that.
#[test]
fn frozen_v0_9_store_parses_strictly_then_migrates_device_index() {
    // Note "log_level": 2 — the legacy numeric format, kept deliberately.
    let stored: serde_json::Value = serde_json::from_str(
        r##"{
            "settings_schema_version": 1,
            "bindings": {
                "transcribe": {
                    "id": "transcribe",
                    "name": "Transcribe",
                    "description": "Converts your speech into text.",
                    "default_binding": "option+space",
                    "current_binding": "f13"
                },
                "transcribe_with_post_process": {
                    "id": "transcribe_with_post_process",
                    "name": "Transcribe with Post-Processing",
                    "description": "Converts your speech into text and applies AI post-processing.",
                    "default_binding": "option+shift+space",
                    "current_binding": "option+shift+space"
                },
                "cancel": {
                    "id": "cancel",
                    "name": "Cancel",
                    "description": "Cancels the current recording.",
                    "default_binding": "escape",
                    "current_binding": "escape"
                }
            },
            "push_to_talk": false,
            "audio_feedback": true,
            "audio_feedback_volume": 0.8,
            "sound_theme": "pop",
            "start_hidden": false,
            "autostart_enabled": true,
            "update_checks_enabled": true,
            "show_whats_new_on_update": true,
            "whats_new_last_seen_version": "0.9.0",
            "selected_model": "whisper-large-v3-turbo",
            "onboarding_completed": true,
            "always_on_microphone": false,
            "selected_microphone": "MacBook Pro Microphone",
            "clamshell_microphone": null,
            "selected_output_device": null,
            "translate_to_english": false,
            "selected_language": "en",
            "overlay_position": "bottom",
            "debug_mode": false,
            "log_level": 2,
            "custom_words": ["Handy", "cjpais"],
            "model_unload_timeout": "min5",
            "word_correction_threshold": 0.18,
            "history_limit": 5,
            "recording_retention_period": "preserve_limit",
            "paste_method": "ctrl_v",
            "clipboard_handling": "dont_modify",
            "auto_submit": false,
            "auto_submit_key": "enter",
            "post_process_enabled": false,
            "post_process_provider_id": "openai",
            "post_process_providers": [
                {
                    "id": "openai",
                    "label": "OpenAI",
                    "base_url": "https://api.openai.com/v1",
                    "allow_base_url_edit": false,
                    "models_endpoint": null,
                    "supports_structured_output": true
                }
            ],
            "post_process_api_keys": { "openai": "" },
            "post_process_models": { "openai": "gpt-4o-mini" },
            "post_process_prompts": [
                { "id": "default", "name": "Default", "prompt": "Clean up the transcript." }
            ],
            "post_process_selected_prompt_id": null,
            "mute_while_recording": false,
            "append_trailing_space": false,
            "app_language": "en",
            "experimental_enabled": false,
            "lazy_stream_close": false,
            "keyboard_implementation": "handy_keys",
            "show_tray_icon": true,
            "paste_delay_ms": 60,
            "typing_tool": "auto",
            "external_script_path": null,
            "custom_filler_words": null,
            "transcribe_accelerator": "gpu",
            "ort_accelerator": "auto",
            "transcribe_gpu_device": 0,
            "extra_recording_buffer_ms": 0,
            "vad_enabled": true,
            "overlay_style": "live"
        }"##,
    )
    .expect("fixture is valid JSON");

    let mut settings: AppSettings = serde_json::from_value(stored.clone())
        .expect("a stored v0.9.0 settings object must keep parsing strictly");

    assert_eq!(settings.selected_model, "whisper-large-v3-turbo");
    assert_eq!(settings.bindings["transcribe"].current_binding, "f13");
    assert_eq!(settings.log_level, LogLevel::Debug);
    assert_eq!(settings.sound_theme, SoundTheme::Pop);
    assert!(settings.filler_word_removal_enabled);

    // The 0.1 integer device index is cleared once for transcribe.cpp 0.2.
    // Without an exact device, the retired generic GPU choice becomes Auto.
    assert!(apply_settings_migrations(&mut settings, &stored));
    assert_eq!(
        settings.settings_schema_version,
        CURRENT_SETTINGS_SCHEMA_VERSION
    );
    assert_eq!(
        settings.transcribe_accelerator,
        TranscribeAcceleratorSetting::Auto
    );
    assert_eq!(settings.transcribe_gpu_device, None);
}

#[test]
fn salvage_preserves_valid_fields_when_one_value_is_invalid() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert(
        "selected_model".into(),
        serde_json::json!("parakeet-tdt-0.6b-v3"),
    );
    map.insert("onboarding_completed".into(), serde_json::json!(true));
    // An enum variant this build doesn't know, e.g. written by a newer
    // version before a downgrade.
    map.insert("sound_theme".into(), serde_json::json!("theremin"));
    stored["bindings"]["transcribe"]["current_binding"] = serde_json::json!("f13");

    // Precondition: this is exactly the whole-store parse failure from
    // #1619 that used to reset everything to defaults.
    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_model, "parakeet-tdt-0.6b-v3");
    assert!(salvaged.onboarding_completed);
    assert_eq!(salvaged.bindings["transcribe"].current_binding, "f13");
    assert_eq!(salvaged.sound_theme, default_sound_theme());
}

#[test]
fn salvage_drops_only_wrong_typed_fields() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert("paste_delay_ms".into(), serde_json::json!("sixty"));
    map.insert("sound_theme".into(), serde_json::json!(42));
    map.insert("custom_words".into(), serde_json::json!(["handy"]));

    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.paste_delay_ms, default_paste_delay_ms());
    assert_eq!(salvaged.sound_theme, default_sound_theme());
    assert_eq!(salvaged.custom_words, vec!["handy".to_string()]);
}

#[test]
fn salvage_of_poisoned_bindings_keeps_other_fields() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    // One malformed entry poisons the whole bindings map, but must not
    // take the rest of the settings down with it.
    map.insert(
        "bindings".into(),
        serde_json::json!({ "transcribe": { "id": 42 } }),
    );
    map.insert("selected_model".into(), serde_json::json!("whisper-small"));

    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_model, "whisper-small");
    let defaults = get_default_settings();
    assert_eq!(
        salvaged.bindings["transcribe"].current_binding,
        defaults.bindings["transcribe"].current_binding
    );
}

#[test]
fn salvage_tolerates_unknown_keys() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert(
        "field_from_the_future".into(),
        serde_json::json!({ "nested": true }),
    );
    map.insert("selected_model".into(), serde_json::json!("kept"));
    map.insert("sound_theme".into(), serde_json::json!("theremin"));

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_model, "kept");
    assert_eq!(salvaged.sound_theme, default_sound_theme());
}

#[test]
fn salvage_of_non_object_store_falls_back_to_defaults() {
    for stored in [
        serde_json::json!("corrupt"),
        serde_json::json!(null),
        serde_json::json!([1, 2, 3]),
    ] {
        let salvaged = salvage_settings(&stored);
        assert_eq!(
            serde_json::to_value(&salvaged).unwrap(),
            default_settings_json()
        );
    }
}

#[test]
fn default_settings_disable_auto_submit() {
    let settings = get_default_settings();
    assert!(!settings.auto_submit);
    assert_eq!(settings.auto_submit_key, AutoSubmitKey::Enter);
    assert_eq!(
        settings.settings_schema_version,
        CURRENT_SETTINGS_SCHEMA_VERSION
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn default_overlay_style_is_live_when_overlay_defaults_on() {
    let settings = get_default_settings();
    assert_eq!(settings.overlay_style, OverlayStyle::Live);
}

#[test]
fn overlay_migration_keeps_disabled_overlay_off() {
    let mut settings = get_default_settings();

    // Legacy store: overlay was hidden via the retired position "none".
    let raw = serde_json::json!({
        "selected_model": "",
        "overlay_position": "none"
    });

    assert!(apply_settings_migrations(&mut settings, &raw));
    assert_eq!(settings.overlay_style, OverlayStyle::None);
}

#[test]
fn legacy_none_overlay_position_deserializes_to_bottom() {
    // A persisted "none" must not fail the whole settings load; the serde
    // alias folds it onto Bottom (visibility is owned by overlay_style).
    let raw = serde_json::json!({ "overlay_position": "none" });
    let position: OverlayPosition =
        serde_json::from_value(raw.get("overlay_position").unwrap().clone())
            .expect("legacy \"none\" should deserialize, not error");
    assert_eq!(position, OverlayPosition::Bottom);
}

#[test]
fn overlay_migration_promotes_enabled_overlay_to_live() {
    let mut settings = get_default_settings();
    settings.overlay_position = OverlayPosition::Top;
    settings.overlay_style = OverlayStyle::Minimal;

    let raw = serde_json::json!({
        "selected_model": "",
        "overlay_position": "top"
    });

    assert!(apply_settings_migrations(&mut settings, &raw));
    assert_eq!(settings.overlay_style, OverlayStyle::Live);
    assert_eq!(settings.overlay_position, OverlayPosition::Top);
}

#[test]
fn gpu_device_migration_resets_legacy_positive_selection_to_auto() {
    let mut settings = get_default_settings();
    settings.transcribe_accelerator = TranscribeAcceleratorSetting::Gpu;

    let raw = serde_json::json!({
        "transcribe_accelerator": "gpu",
        "transcribe_gpu_device": 2
    });

    assert!(apply_settings_migrations(&mut settings, &raw));
    assert_eq!(
        settings.transcribe_accelerator,
        TranscribeAcceleratorSetting::Auto
    );
    assert_eq!(settings.transcribe_gpu_device, None);
    assert_eq!(
        settings.settings_schema_version,
        CURRENT_SETTINGS_SCHEMA_VERSION
    );
}

#[test]
fn gpu_device_migration_maps_v1_automatic_gpu_to_auto() {
    let raw = serde_json::json!({
        "settings_schema_version": 1,
        "transcribe_accelerator": "gpu",
        "transcribe_gpu_device": 2
    });
    let mut settings: AppSettings = serde_json::from_value(raw.clone()).unwrap();

    assert!(apply_settings_migrations(&mut settings, &raw));
    assert_eq!(
        settings.transcribe_accelerator,
        TranscribeAcceleratorSetting::Auto
    );
    assert_eq!(settings.transcribe_gpu_device, None);
}

#[test]
fn gpu_device_migration_maps_current_automatic_gpu_to_auto() {
    let raw = serde_json::json!({
        "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
        "onboarding_completed": false,
        "whats_new_last_seen_version": default_whats_new_last_seen_version(),
        "overlay_style": "live",
        "transcribe_accelerator": "gpu",
        "transcribe_gpu_device": null
    });
    let mut settings: AppSettings = serde_json::from_value(raw.clone()).unwrap();

    assert!(apply_settings_migrations(&mut settings, &raw));
    assert_eq!(
        settings.transcribe_accelerator,
        TranscribeAcceleratorSetting::Auto
    );
    assert_eq!(settings.transcribe_gpu_device, None);
}

#[test]
fn gpu_device_migration_keeps_current_stable_selection() {
    let mut settings = get_default_settings();
    settings.transcribe_accelerator = TranscribeAcceleratorSetting::Gpu;
    settings.transcribe_gpu_device = Some("[\"vulkan\",\"id\",\"0000:01:00.0\"]".into());

    let raw = serde_json::json!({
        "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
        "onboarding_completed": false,
        "whats_new_last_seen_version": default_whats_new_last_seen_version(),
        "overlay_style": "live",
        // Present so the audio-ducking migration sees an already-migrated
        // store: this fixture stands for "nothing left to do".
        "recording_audio_mode": "off",
        "transcribe_accelerator": "gpu",
        "transcribe_gpu_device": settings.transcribe_gpu_device
    });

    assert!(!apply_settings_migrations(&mut settings, &raw));
    assert_eq!(
        settings.transcribe_gpu_device.as_deref(),
        Some("[\"vulkan\",\"id\",\"0000:01:00.0\"]")
    );
}

#[test]
fn debug_output_redacts_api_keys() {
    let mut settings = get_default_settings();
    settings
        .post_process_api_keys
        .insert("openai".to_string(), "sk-proj-secret-key-12345".to_string());
    settings.post_process_api_keys.insert(
        "anthropic".to_string(),
        "sk-ant-secret-key-67890".to_string(),
    );
    settings
        .post_process_api_keys
        .insert("empty_provider".to_string(), "".to_string());

    let debug_output = format!("{:?}", settings);

    assert!(!debug_output.contains("sk-proj-secret-key-12345"));
    assert!(!debug_output.contains("sk-ant-secret-key-67890"));
    assert!(debug_output.contains("[REDACTED]"));
}

#[test]
fn secret_map_debug_redacts_values() {
    let map = SecretMap(HashMap::from([("key".into(), "secret".into())]));
    let out = format!("{:?}", map);
    assert!(!out.contains("secret"));
    assert!(out.contains("[REDACTED]"));
}

/// Durability gate for `update_settings`: the moment the transaction returns,
/// the new value must already be on disk — observable through a freshly
/// opened store handle, with no reliance on `Drop` or tauri-plugin-store's
/// ~100 ms debounced auto-save (which an immediate process kill would lose).
///
/// The writer and reader are two *separate* mock apps: each app's store plugin
/// keeps its own handle cache, so the reader's store is a brand-new instance
/// that must read the value back from disk at open time. Without the explicit
/// `store.save()` flush at the write site (see
/// `store::flush_settings_store`), this test fails because the reader opens
/// before any debounced save has fired.
#[test]
fn flushed_update_is_immediately_visible_to_a_freshly_opened_handle() {
    use tempfile::TempDir;

    // Isolate the store file from the real user data directory. (The redirect
    // is process-global and first-caller-wins; this is the only test that
    // uses it.)
    let dir = TempDir::new().expect("temp dir for isolated settings store");
    super::store::set_test_store_dir(dir.path());
    let store_file = dir.path().join(super::store::SETTINGS_STORE_PATH);

    let build_app = || {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app with store plugin builds")
    };

    let marker = "durability-check-model";
    {
        let writer_app = build_app();
        super::update_settings(writer_app.handle(), |settings| {
            settings.selected_model = marker.to_string();
        });
        // No Drop of any handle may be required for durability: assert the
        // raw file contents synchronously, before any debounced save could
        // have run.
        let on_disk = std::fs::read_to_string(&store_file)
            .expect("update_settings must leave the store file on disk");
        assert!(
            on_disk.contains(marker),
            "acknowledged update must be flushed to disk synchronously"
        );
    }

    // Fresh app => fresh plugin state => a brand-new store handle that reads
    // the persisted value back from disk when it opens.
    let reader_app = build_app();
    let reread = get_settings(reader_app.handle());
    assert_eq!(
        reread.selected_model, marker,
        "a freshly opened store handle must observe the acknowledged update"
    );
}

/* ---------- audio ducking ---------------------------------------------- */

/// Building the migration input the way the real load path does: serialize a
/// settings object, then remove the key we want to look "not yet written".
fn stored_without(settings: &AppSettings, key: &str) -> serde_json::Value {
    let mut value = serde_json::to_value(settings).unwrap();
    value.as_object_mut().unwrap().remove(key);
    value
}

/// An upgrading user who had muting on must keep it. Losing it silently would
/// be the whole point of the migration.
#[test]
fn absent_recording_audio_mode_migrates_from_a_true_legacy_mute_flag() {
    let mut settings = get_default_settings();
    settings.mute_while_recording = true;
    settings.recording_audio_mode = RecordingAudioMode::Off;
    let stored = stored_without(&settings, "recording_audio_mode");

    assert!(apply_settings_migrations(&mut settings, &stored));
    assert_eq!(settings.recording_audio_mode, RecordingAudioMode::Mute);
    assert!(settings.mute_while_recording);
}

/// The other direction: a user who never enabled muting must not suddenly get
/// their audio touched.
#[test]
fn absent_recording_audio_mode_migrates_from_a_false_legacy_mute_flag() {
    let mut settings = get_default_settings();
    settings.mute_while_recording = false;
    settings.recording_audio_mode = RecordingAudioMode::Mute;
    let stored = stored_without(&settings, "recording_audio_mode");

    assert!(apply_settings_migrations(&mut settings, &stored));
    assert_eq!(settings.recording_audio_mode, RecordingAudioMode::Off);
    assert!(!settings.mute_while_recording);
}

/// Once the key exists, the mode is authoritative — the migration must not
/// re-derive it from the legacy bool and clobber a deliberate `Duck` choice.
#[test]
fn present_recording_audio_mode_wins_over_the_legacy_flag() {
    let mut settings = get_default_settings();
    settings.recording_audio_mode = RecordingAudioMode::Duck;
    settings.mute_while_recording = true; // stale/drifted shadow
    let stored = serde_json::to_value(&settings).unwrap();

    apply_settings_migrations(&mut settings, &stored);
    assert_eq!(settings.recording_audio_mode, RecordingAudioMode::Duck);
    // The write-through mirror repairs the drift instead of preserving it.
    assert!(!settings.mute_while_recording);
}

/// A store already carrying a consistent pair must not be reported as changed
/// — a spurious `updated = true` means a disk write on every single load.
#[test]
fn consistent_recording_audio_settings_do_not_report_a_migration() {
    let mut settings = get_default_settings();
    settings.recording_audio_mode = RecordingAudioMode::Mute;
    settings.mute_while_recording = true;
    settings.onboarding_completed = true;
    settings.whats_new_last_seen_version = "1.0.0".into();
    settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
    let stored = serde_json::to_value(&settings).unwrap();

    assert!(!apply_settings_migrations(&mut settings, &stored));
}

#[test]
fn recording_audio_defaults_are_off_with_a_usable_duck_preset() {
    let settings = get_default_settings();
    assert_eq!(settings.recording_audio_mode, RecordingAudioMode::Off);
    assert!(!settings.mute_while_recording);
    assert_eq!(settings.duck_volume_percent, 20);
    assert_eq!(settings.duck_fade_ms, 120);
}

#[test]
fn duck_settings_are_clamped_to_their_valid_ranges() {
    assert_eq!(clamp_duck_volume_percent(0), 0);
    assert_eq!(clamp_duck_volume_percent(20), 20);
    assert_eq!(clamp_duck_volume_percent(100), 100);
    assert_eq!(clamp_duck_volume_percent(255), 100);

    assert_eq!(clamp_duck_fade_ms(0), 0);
    assert_eq!(clamp_duck_fade_ms(120), 120);
    assert_eq!(clamp_duck_fade_ms(1000), 1000);
    assert_eq!(clamp_duck_fade_ms(u16::MAX), 1000);
}

/// The legacy bool is a derived shadow; both helpers must agree on the mapping
/// so the migration and the legacy command can never disagree.
#[test]
fn legacy_mute_flag_round_trips_through_the_mode() {
    assert!(RecordingAudioMode::Mute.mirrors_mute_flag());
    assert!(!RecordingAudioMode::Duck.mirrors_mute_flag());
    assert!(!RecordingAudioMode::Off.mirrors_mute_flag());

    assert_eq!(
        RecordingAudioMode::from_legacy_mute_flag(true),
        RecordingAudioMode::Mute
    );
    assert_eq!(
        RecordingAudioMode::from_legacy_mute_flag(false),
        RecordingAudioMode::Off
    );
}

/// Ducking is a new capability, so an unknown mode written by a newer build
/// must degrade to the untouched-audio default rather than failing the load.
#[test]
fn unknown_recording_audio_mode_is_salvaged_to_a_safe_default() {
    let mut stored = default_settings_json();
    stored.as_object_mut().unwrap().insert(
        "recording_audio_mode".into(),
        serde_json::json!("sidechain"),
    );

    let settings = salvage_settings(&stored);
    assert_eq!(settings.recording_audio_mode, RecordingAudioMode::Off);
}

/* ---------- assistant profiles ------------------------------------------ */

/// Both shipped profiles must exist, be internally consistent, and point at a
/// provider that is actually in the catalogue — a profile is unusable the
/// moment any of those is false.
#[test]
fn default_assistant_profiles_are_present_and_valid() {
    let settings = get_default_settings();
    assert_eq!(settings.assistant_profiles.len(), 2);

    for id in [ASSISTANT_PROFILE_FAST, ASSISTANT_PROFILE_SMART] {
        let profile = settings
            .assistant_profile(id)
            .unwrap_or_else(|| panic!("profile '{id}' must ship by default"));
        assert_eq!(profile.id, id, "map key and inner id must agree");
        assert!(
            settings
                .post_process_provider(&profile.provider_id)
                .is_some(),
            "profile '{id}' points at unknown provider '{}'",
            profile.provider_id
        );
        assert!(!profile.system_prompt.is_empty());
        assert_eq!(
            clamp_assistant_output_tokens(profile.max_output_tokens),
            profile.max_output_tokens
        );
        assert_eq!(
            clamp_assistant_timeout_secs(profile.timeout_secs),
            profile.timeout_secs
        );
        // Neither assistant may touch the user's document unasked.
        assert_eq!(profile.output_mode, AssistantOutputMode::Show);
    }
}

/// The fast/smart split is carried by the budgets and the reasoning flag, not
/// by a model id — so it has to actually be a split.
#[test]
fn fast_and_smart_profiles_differ_in_budget_and_reasoning() {
    let settings = get_default_settings();
    let fast = settings.assistant_profile(ASSISTANT_PROFILE_FAST).unwrap();
    let smart = settings.assistant_profile(ASSISTANT_PROFILE_SMART).unwrap();

    assert_eq!(fast.max_output_tokens, 512);
    assert_eq!(fast.timeout_secs, 15);
    assert!(fast.disable_reasoning);

    // Sized for reasoning *plus* answer: providers count thinking tokens
    // against this ceiling and bill them as output, so a reply-sized budget
    // returns empty content after a long, paid wait.
    assert_eq!(smart.max_output_tokens, 4096);
    assert_eq!(smart.timeout_secs, 90);
    assert!(!smart.disable_reasoning);

    assert_ne!(fast.system_prompt, smart.system_prompt);
}

/// An existing store written before assistants existed has no
/// `assistant_profiles` key at all. It must parse, and the repair pass must
/// seed both profiles — that is what an upgrading user gets.
#[test]
fn seeding_repairs_a_store_without_assistant_profiles() {
    let mut stored = default_settings_json();
    stored
        .as_object_mut()
        .unwrap()
        .remove("assistant_profiles")
        .expect("fixture must have had the key");

    let mut settings: AppSettings =
        serde_json::from_value(stored).expect("a store predating assistants must still parse");
    settings.assistant_profiles.clear();

    assert!(
        ensure_assistant_defaults(&mut settings),
        "seeding must report a change so the repaired store is persisted"
    );
    assert!(settings.assistant_profile(ASSISTANT_PROFILE_FAST).is_some());
    assert!(settings
        .assistant_profile(ASSISTANT_PROFILE_SMART)
        .is_some());

    // Idempotent: a second pass must not report another change, or every
    // settings read would rewrite the store.
    assert!(!ensure_assistant_defaults(&mut settings));
}

/// A profile whose provider was removed from the catalogue must be repaired to
/// a real provider. Leaving it dangling would fail every request with an
/// error the settings UI cannot even explain.
#[test]
fn a_profile_pointing_at_a_removed_provider_is_repaired() {
    let mut settings = get_default_settings();
    let profile = settings
        .assistant_profile_mut(ASSISTANT_PROFILE_FAST)
        .unwrap();
    profile.provider_id = "provider_that_was_deleted".to_string();
    profile.model = "model-of-the-deleted-provider".to_string();

    assert!(ensure_assistant_defaults(&mut settings));

    let repaired = settings.assistant_profile(ASSISTANT_PROFILE_FAST).unwrap();
    assert!(
        settings
            .post_process_provider(&repaired.provider_id)
            .is_some(),
        "repair left provider '{}' dangling",
        repaired.provider_id
    );
    assert_eq!(
        repaired.model,
        default_model_for_provider(&repaired.provider_id),
        "the model belonged to the provider that vanished and must not survive it"
    );
}

/// Out-of-range budgets from a hand-edited store are clamped on load rather
/// than reaching the LLM client.
#[test]
fn assistant_limits_are_clamped_on_load() {
    let mut settings = get_default_settings();
    {
        let fast = settings
            .assistant_profile_mut(ASSISTANT_PROFILE_FAST)
            .unwrap();
        fast.max_output_tokens = 0;
        fast.timeout_secs = 0;
    }
    {
        let smart = settings
            .assistant_profile_mut(ASSISTANT_PROFILE_SMART)
            .unwrap();
        smart.max_output_tokens = u32::MAX;
        smart.timeout_secs = u32::MAX;
    }

    assert!(ensure_assistant_defaults(&mut settings));

    let fast = settings.assistant_profile(ASSISTANT_PROFILE_FAST).unwrap();
    assert_eq!(fast.max_output_tokens, MIN_ASSISTANT_OUTPUT_TOKENS);
    assert_eq!(fast.timeout_secs, MIN_ASSISTANT_TIMEOUT_SECS);

    let smart = settings.assistant_profile(ASSISTANT_PROFILE_SMART).unwrap();
    assert_eq!(smart.max_output_tokens, MAX_ASSISTANT_OUTPUT_TOKENS);
    assert_eq!(smart.timeout_secs, MAX_ASSISTANT_TIMEOUT_SECS);
}

#[test]
fn assistant_clamp_helpers_cover_their_boundaries() {
    assert_eq!(
        clamp_assistant_output_tokens(0),
        MIN_ASSISTANT_OUTPUT_TOKENS
    );
    assert_eq!(clamp_assistant_output_tokens(512), 512);
    assert_eq!(
        clamp_assistant_output_tokens(u32::MAX),
        MAX_ASSISTANT_OUTPUT_TOKENS
    );

    assert_eq!(clamp_assistant_timeout_secs(0), MIN_ASSISTANT_TIMEOUT_SECS);
    assert_eq!(clamp_assistant_timeout_secs(90), 90);
    assert_eq!(
        clamp_assistant_timeout_secs(u32::MAX),
        MAX_ASSISTANT_TIMEOUT_SECS
    );
}

/// Assistant keys must be as unspillable as the post-processing ones: the
/// `SecretMap` serializer writes `{}` no matter what is held in memory.
#[test]
fn assistant_api_keys_never_serialize_a_secret_to_disk() {
    let mut settings = get_default_settings();
    settings.assistant_api_keys.insert(
        ASSISTANT_PROFILE_FAST.to_string(),
        "sk-assistant-secret-abcdef".to_string(),
    );

    let serialized = serde_json::to_string(&settings).unwrap();
    assert!(!serialized.contains("sk-assistant-secret-abcdef"));
    assert!(serialized.contains("\"assistant_api_keys\":{}"));

    let debug_output = format!("{:?}", settings);
    assert!(!debug_output.contains("sk-assistant-secret-abcdef"));
    assert!(debug_output.contains("[REDACTED]"));
}

/// The two bindings must exist in the defaults — `get_stored_binding` panics
/// on an unknown id — and must ship unbound, because a shortcut that fires a
/// paid LLM request has to be opted into.
#[test]
fn assistant_bindings_ship_present_but_unbound() {
    let settings = get_default_settings();

    for id in [ASSISTANT_BINDING_FAST, ASSISTANT_BINDING_SMART] {
        let binding = settings
            .bindings
            .get(id)
            .unwrap_or_else(|| panic!("binding '{id}' must exist in the defaults"));
        assert_eq!(binding.id, id);
        assert!(
            binding.current_binding.is_empty(),
            "binding '{id}' must ship unbound"
        );
        assert!(
            !binding.default_binding.is_empty(),
            "binding '{id}' needs a suggested default for the reset button"
        );
        assert!(!binding.name.is_empty());
        assert!(!binding.description.is_empty());
    }
}
