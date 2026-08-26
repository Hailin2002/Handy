mod actions;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple_intelligence;
mod audio_feedback;
pub mod audio_toolkit;
mod autostart;
mod catalog;
pub mod cli;
mod clipboard;
mod commands;
mod headless;
mod helpers;
mod input;
mod llm_client;
mod logging;
pub mod managers;
mod memory;
mod overlay;
mod paste_tx;
pub mod portable;
mod secret_store;
mod secure_input;
// Public for integration tests (src-tauri/tests): they exercise the cloud STT
// client paths and need to construct `AppSettings`.
pub mod settings;
mod shortcut;
mod signal_handle;
// Persistence seam (plan §5, module-placement-map B11/E3); scaffold only.
mod storage;
// Public for integration tests (src-tauri/tests): negative-path STT tests call
// `transcribe_cloud` against a local mock HTTP server.
pub mod speech_pipeline;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod utils;

pub use cli::CliArgs;
// Re-exported so integration tests can open history connections configured
// exactly like the app does (WAL + busy_timeout) without exposing the whole
// private `managers` tree, and can exercise the corrupt-database
// quarantine-and-rebuild startup path directly.
pub use managers::history::{
    create_and_migrate_history_db, ensure_usable_history_db, open_configured_connection,
};
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::meeting::MeetingManager;
use managers::model::ModelManager;
use managers::transcription::TranscriptionManager;
use std::sync::Arc;
pub use transcription_coordinator::TranscriptionCoordinator;

use logging::{
    build_console_filter, level_filter_from_u8, set_file_log_level, set_webview_log_streaming,
    FILE_LOG_LEVEL, WEBVIEW_LOG_STREAMING,
};
use tauri::{AppHandle, Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use std::sync::atomic::Ordering;

#[allow(unused_variables)]
fn should_force_show_permissions_window(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let model_manager = app.state::<Arc<ModelManager>>();
        let has_downloaded_models = model_manager
            .get_available_models()
            .iter()
            .any(|model| model.is_downloaded);

        if !has_downloaded_models {
            return false;
        }

        let status = commands::audio::get_windows_microphone_permission_status();
        if status.supported && status.overall_access == commands::audio::PermissionAccess::Denied {
            log::info!(
                "Windows microphone permissions are denied; forcing main window visible for onboarding"
            );
            return true;
        }
    }

    false
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers. The audio recorder receives the streaming router
    // explicitly, so always-on microphone startup can wire live-preview frames
    // even before Tauri state is populated.
    let model_manager =
        Arc::new(ModelManager::new(app_handle).expect("Failed to initialize model manager"));
    let transcription_manager = Arc::new(
        TranscriptionManager::new(app_handle, model_manager.clone())
            .expect("Failed to initialize transcription manager"),
    );
    let azure_stream_manager = Arc::new(speech_pipeline::AzureSpeechStreamManager::new(app_handle));
    let recording_manager = Arc::new(
        AudioRecordingManager::new(
            app_handle,
            transcription_manager.stream_router(),
            azure_stream_manager.clone(),
        )
        .expect("Failed to initialize recording manager"),
    );
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));
    let meeting_manager =
        Arc::new(MeetingManager::new(app_handle).expect("Failed to initialize meeting manager"));
    // The recorder-event pump needs the manager behind its `Arc` (it holds a
    // `Weak` back-reference), so it is attached right after construction.
    meeting_manager.attach_recorder_events();

    // Phase 8: projects share the meeting repository (same SQLite file).
    let project_manager = Arc::new(crate::managers::project::ProjectManager::new(
        meeting_manager.store_handle(),
    ));
    app_handle.manage(project_manager.clone());

    // Phase 9: source ingestion shares the same repository.
    let source_ingestion = Arc::new(
        crate::managers::source_ingestion::SourceIngestionService::new(
            meeting_manager.store_handle(),
        ),
    );
    app_handle.manage(source_ingestion);

    // Local STT queue for meetings (plan §13 Phases 5–6). Pauses between jobs
    // while dictation recording/streaming holds the shared local model. The
    // routing engine dispatches per job to the provider chosen *for that
    // meeting* — independent of the dictation provider.
    {
        let vad_model_path = app_handle
            .path()
            .resolve(
                "resources/models/silero_vad_v4.onnx",
                tauri::path::BaseDirectory::Resource,
            )
            .ok();
        let recording_for_gate = Arc::clone(&recording_manager);
        let transcription_for_gate = Arc::clone(&transcription_manager);

        let local_engine = Arc::new(managers::meeting_transcription::LocalMeetingSttEngine::new(
            transcription_manager.clone(),
        ));
        let routing = Arc::new(
            managers::meeting_transcription::RoutingMeetingSttEngine::new(local_engine.clone()),
        );
        routing.register("Local", local_engine);

        let settings = crate::settings::get_settings(app_handle);
        // Cloud STT round-trips are bounded so a hung endpoint can never stall
        // the queue worker (and thus settlement) indefinitely.
        let cloud_timeout = std::time::Duration::from_secs(MEETING_CLOUD_TIMEOUT_SECS);

        // Every *configured* cloud backend is registered, not just the one the
        // global `meeting_stt_provider` happens to name. The provider is a
        // per-meeting choice made in the start dialog (plan Phase 6: "Local,
        // Azure und Custom koennen pro Meeting gewaehlt werden"), so gating
        // registration on the global setting made those choices unroutable:
        // the session was persisted and displayed as "Azure Speech" while the
        // router silently fell back to the local engine.
        if !settings.azure_speech_endpoint.trim().is_empty() {
            if let Some(key) =
                crate::secret_store::get_secret(crate::secret_store::SPEECH_PIPELINE_SCOPE, "azure")
            {
                match managers::meeting_cloud::AzureFastTranscriptionClient::new(
                    settings.azure_speech_endpoint.clone(),
                    key,
                    cloud_timeout,
                ) {
                    Ok(client) => routing.register(
                        "Azure Speech",
                        Arc::new(managers::meeting_cloud::AzureMeetingEngine::new(Arc::new(
                            client,
                        ))),
                    ),
                    Err(e) => log::warn!(
                        "Azure meeting STT not registered: {}. Meetings that select Azure will fail visibly.",
                        e.message()
                    ),
                }
            }
        }

        if !settings.custom_stt_endpoint.trim().is_empty() {
            let key = crate::secret_store::get_secret(
                crate::secret_store::SPEECH_PIPELINE_SCOPE,
                "custom",
            )
            .unwrap_or_default();
            match managers::meeting_cloud::CustomEndpointClient::new(
                settings.custom_stt_endpoint.clone(),
                key,
                cloud_timeout,
            ) {
                Ok(client) => routing.register(
                    "Custom endpoint",
                    Arc::new(managers::meeting_cloud::CustomMeetingEngine::new(Arc::new(
                        client,
                    ))),
                ),
                Err(e) => log::warn!(
                    "Custom meeting STT not registered: {}. Meetings that select Custom will fail visibly.",
                    e.message()
                ),
            }
        }

        meeting_manager.attach_transcription_queue(
            routing,
            Arc::new(move || {
                recording_for_gate.is_recording() || transcription_for_gate.is_streaming()
            }),
            vad_model_path,
        );
    }

    // Durable, content-addressed asset storage for meeting audio (plan 5,
    // module-placement-map B11). Managed so later leaves resolve it from
    // Tauri state instead of constructing a second root.
    match storage::asset_store::LocalAssetStore::for_app(app_handle) {
        Ok(asset_store) => {
            app_handle.manage(Arc::new(asset_store));
        }
        Err(e) => log::error!("Failed to initialize meeting asset store: {}", e),
    }

    // Initialize the transcribe-cpp native backend (logging + backend module
    // registration) once, before any whisper model is loaded.
    managers::transcription::init_transcribe_backend();

    // Apply accelerator preferences before any model loads
    managers::transcription::apply_accelerator_settings(app_handle);

    // Add managers to Tauri's managed state
    app_handle.manage(recording_manager.clone());
    app_handle.manage(model_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(azure_stream_manager);
    app_handle.manage(history_manager.clone());
    app_handle.manage(meeting_manager.clone());
    app_handle.manage(tray::CurrentTrayIconState::new());

    // Startup reconciliation (session-state-contract 6.2 R0-R6): no meeting
    // session may stay in an active state after a crashed process. Runs once,
    // after migrations, before any window can subscribe.
    match meeting_manager.run_startup_recovery() {
        Ok(offers) if !offers.is_empty() => {
            log::info!("Recovered {} interrupted meeting session(s)", offers.len());
        }
        Ok(_) => {}
        Err(e) => log::error!("Meeting startup recovery failed: {}", e),
    }

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    // Set up signal handlers for toggling transcription. On Linux, SIGUSR1 is
    // deliberately not handled — it belongs to WebKitGTK's garbage collector
    // (#1660) — see signal_handle.rs.
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone());

    // Apply macOS Accessory policy if starting hidden and tray is available.
    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Build the tray icon and register its menu handler. A tray failure is
    // logged, not fatal: the app stays usable without a tray icon (the main
    // window remains reachable via single-instance forwarding).
    if let Err(e) = tray::init_tray(app_handle) {
        log::error!("Failed to initialize tray icon: {}", e);
    }

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, None);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Refresh tray menu when model state changes
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::update_tray_menu(&app_handle_for_listener, None);
    });

    // Apply the autostart preference (SMAppService login item on macOS 13+,
    // tauri-plugin-autostart elsewhere)
    autostart::apply_autostart(app_handle, settings.autostart_enabled);

    // Create the recording overlay window (hidden by default)
    utils::create_recording_overlay(app_handle);
}

/// Per-request timeout for cloud meeting-STT HTTP calls (Azure Fast
/// Transcription, custom endpoints). Generous: chunks are up to a few minutes
/// of audio and providers queue internally.
const MEETING_CLOUD_TIMEOUT_SECS: u64 = 120;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Avoid ggml-metal residency-set teardown assertions when a native engine
    // outlives the Tauri shutdown sequence (#1902). This must happen before
    // transcribe-cpp initializes its Metal device. Advanced users can restore
    // upstream residency behavior with HANDY_METAL_RESIDENCY=1.
    #[cfg(target_os = "macos")]
    if std::env::var("HANDY_METAL_RESIDENCY").as_deref() == Ok("1") {
        // ggml treats GGML_METAL_NO_RESIDENCY as presence-based, so remove an
        // inherited value as well when explicitly opting back in.
        std::env::remove_var("GGML_METAL_NO_RESIDENCY");
    } else {
        std::env::set_var("GGML_METAL_NO_RESIDENCY", "1");
    }

    // Pin glibc's dynamic mmap threshold before the first large allocation,
    // so per-dictation transient buffers are returned to the OS on free
    // instead of accumulating in malloc arenas (#1792). No-op off Linux/glibc.
    memory::init_allocator();

    // Detect portable mode before anything else
    portable::init();

    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::change_ptt_setting,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_theme_setting,
            shortcut::change_start_hidden_setting,
            shortcut::change_autostart_setting,
            shortcut::change_translate_to_english_setting,
            shortcut::change_selected_language_setting,
            shortcut::change_speech_input_provider_setting,
            shortcut::change_azure_speech_endpoint_setting,
            shortcut::change_azure_speech_language_setting,
            shortcut::change_azure_stable_partial_threshold_setting,
            shortcut::change_azure_segmentation_silence_setting,
            shortcut::change_custom_stt_endpoint_setting,
            shortcut::change_speech_pipeline_api_key_setting,
            speech_pipeline::test_azure_speech_connection,
            speech_pipeline::get_azure_speech_auth,
            speech_pipeline::azure_speech_interim,
            speech_pipeline::azure_speech_final,
            speech_pipeline::azure_speech_translate_pending,
            speech_pipeline::azure_speech_complete,
            speech_pipeline::azure_speech_status,
            shortcut::change_deepl_translation_enabled_setting,
            shortcut::change_deepl_target_language_setting,
            shortcut::change_deepl_sentence_delay_setting,
            shortcut::change_overlay_position_setting,
            shortcut::change_overlay_style_setting,
            shortcut::change_debug_mode_setting,
            shortcut::change_word_correction_threshold_setting,
            shortcut::change_extra_recording_buffer_setting,
            shortcut::change_paste_delay_ms_setting,
            shortcut::change_paste_delay_after_ms_setting,
            shortcut::change_reliable_paste_setting,
            shortcut::change_paste_method_setting,
            shortcut::get_available_typing_tools,
            shortcut::change_typing_tool_setting,
            shortcut::change_external_script_path_setting,
            shortcut::change_clipboard_handling_setting,
            shortcut::change_auto_submit_setting,
            shortcut::change_auto_submit_key_setting,
            shortcut::change_post_process_enabled_setting,
            shortcut::change_experimental_enabled_setting,
            shortcut::change_post_process_base_url_setting,
            shortcut::change_post_process_api_key_setting,
            shortcut::change_post_process_model_setting,
            shortcut::set_post_process_provider,
            shortcut::fetch_post_process_models,
            shortcut::add_post_process_prompt,
            shortcut::update_post_process_prompt,
            shortcut::delete_post_process_prompt,
            shortcut::set_post_process_selected_prompt,
            shortcut::update_custom_words,
            shortcut::suspend_all_bindings,
            shortcut::resume_all_bindings,
            shortcut::change_mute_while_recording_setting,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            shortcut::change_vad_enabled_setting,
            shortcut::change_filler_word_removal_enabled_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_show_whats_new_on_update_setting,
            shortcut::change_whats_new_last_seen_version_setting,
            shortcut::change_keyboard_implementation_setting,
            shortcut::get_keyboard_implementation,
            shortcut::change_show_tray_icon_setting,
            shortcut::change_transcribe_accelerator_setting,
            shortcut::change_ort_accelerator_setting,
            shortcut::change_transcribe_gpu_device,
            shortcut::get_available_accelerators,
            shortcut::handy_keys::start_handy_keys_recording,
            shortcut::handy_keys::stop_handy_keys_recording,
            secure_input::get_secure_input_status,
            secure_input::run_keyboard_diagnostic,
            commands::trigger_update_check,
            commands::show_main_window_command,
            commands::cancel_operation,
            commands::is_portable,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::check_apple_intelligence_available,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::models::get_available_models,
            commands::models::get_model_info,
            commands::models::download_model,
            commands::models::delete_model,
            commands::models::cancel_download,
            commands::models::set_active_model,
            commands::models::get_current_model,
            commands::models::get_transcription_model_status,
            commands::models::is_model_loading,
            commands::models::rescan_local_models,
            commands::audio::update_microphone_mode,
            commands::audio::get_microphone_mode,
            commands::audio::get_windows_microphone_permission_status,
            commands::audio::open_microphone_privacy_settings,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::get_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_clamshell_microphone,
            commands::audio::is_recording,
            commands::audio::get_microphone_channels,
            commands::audio::set_selected_channel,
            commands::transcription::set_model_unload_timeout,
            commands::transcription::get_model_load_status,
            commands::transcription::unload_model_manually,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            commands::meetings::meeting_create_session,
            commands::meetings::meeting_get_session,
            commands::meetings::meeting_list_sessions,
            commands::meetings::meeting_rename_session,
            commands::meetings::meeting_delete_session,
            commands::meetings::create_meeting_draft,
            commands::meetings::preflight_meeting,
            commands::meetings::start_meeting,
            commands::meetings::pause_meeting,
            commands::meetings::resume_meeting,
            commands::meetings::stop_meeting,
            commands::meetings::cancel_meeting,
            commands::meetings::get_meeting_snapshot,
            commands::meetings::retry_meeting_jobs,
            commands::meetings::get_meeting_summary,
            commands::meetings::request_meeting_summary,
            commands::meetings::meeting_transcript,
            commands::meetings::export_meeting,
            commands::meetings::delete_meeting_data,
            commands::projects::project_create,
            commands::projects::project_list,
            commands::projects::project_rename,
            commands::projects::project_delete,
            commands::projects::project_assign_meeting,
            commands::projects::project_unassign_meeting,
            commands::projects::project_meeting_ids,
            commands::projects::project_inbox_meeting_ids,
            commands::sources::whatsapp_import_text,
            commands::sources::source_list,
            commands::sources::source_items,
            commands::sources::whatsapp_preview_text,
            commands::tasks::task_create_manual,
            commands::tasks::task_suggest,
            commands::tasks::task_list,
            commands::tasks::task_decide,
            commands::tasks::snapshot_create,
            commands::tasks::snapshot_list,
            commands::meetings::meeting_list_recoveries,
            commands::meetings::meeting_resolve_recovery,
            helpers::clamshell::is_laptop,
        ])
        .events(collect_events![
            managers::history::HistoryUpdatePayload,
            managers::transcription::StreamTextEvent,
            managers::transcription::StreamPhaseEvent,
        ]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    // The headless path must run as its own instance (see the single-instance
    // note below), not forward to an already-running app.
    let headless_mode =
        cli_args.transcribe_file.is_some() || cli_args.list_devices || cli_args.list_models;

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable. In
                    // headless mode (--transcribe-file/--list-devices/--list-models)
                    // stdout carries only the result (JSON or plain), so send console
                    // logs to stderr instead to keep stdout clean for CI parsing.
                    Target::new(if headless_mode {
                        TargetKind::Stderr
                    } else {
                        TargetKind::Stdout
                    })
                    .filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(if let Some(data_dir) = portable::data_dir() {
                        TargetKind::Folder {
                            path: data_dir.join("logs"),
                            file_name: Some("speak".into()),
                        }
                    } else {
                        TargetKind::LogDir {
                            file_name: Some("speak".into()),
                        }
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                    // Stream logs to the webview (via the `log://log` event) so the
                    // debug panel's live log viewer can show them in real time. Only
                    // active while debug mode is on (its sole consumer), and shares the
                    // file log level so the "Log Level" setting controls verbosity.
                    Target::new(TargetKind::Webview).filter(|metadata| {
                        WEBVIEW_LOG_STREAMING.load(Ordering::Relaxed)
                            && metadata.level()
                                <= level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed))
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    // Single-instance forwards CLI args to an already-running Speak process and exits.
    // That would make the headless path
    // (--transcribe-file/--list-devices/--list-models) a silent no-op whenever the
    // app is already open, so skip it in headless mode and run a standalone
    // instance instead.
    if !headless_mode {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--toggle-post-process") {
                signal_handle::send_transcription_input(app, "transcribe_with_post_process", "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else {
                utils::show_main_window(app);
            }
        }));
    }

    builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .setup(move |app| {
            specta_builder.mount_events(app);

            // Headless one-shot path (`--transcribe-file` / `--list-devices` /
            // `--list-models`): initialize only what transcription needs — the
            // store/paths plugins, the model + transcription managers, and the
            // transcribe-cpp backend + accelerator settings — then run on a worker
            // thread and exit. Deliberately skips the window, tray, overlay, audio
            // recorder (so it never opens the mic, even with always_on_microphone),
            // signal handlers, and autostart that initialize_core_logic sets up.
            if headless_mode {
                // One-time legacy-secret migration BEFORE anything reads
                // settings on this path (audit F-6,
                // docs/reviews/secrets-exposure-and-durability-audit.md §3.2):
                // ModelManager/TranscriptionManager init and
                // apply_accelerator_settings all call get_settings, and a read
                // of an older store re-persists it with the API-key maps
                // serialized as `{}` — which would silently destroy plain-text
                // keys that were never moved into the OS keychain. The load
                // inside this transaction also hydrates the in-memory maps.
                secret_store::migrate_secrets_at_startup(app.handle());

                let app_handle = app.handle().clone();
                let model_manager = Arc::new(
                    ModelManager::new(&app_handle).expect("Failed to initialize model manager"),
                );
                let transcription_manager = Arc::new(
                    TranscriptionManager::new(&app_handle, model_manager.clone())
                        .expect("Failed to initialize transcription manager"),
                );
                app_handle.manage(model_manager);
                app_handle.manage(transcription_manager);
                managers::transcription::init_transcribe_backend();
                managers::transcription::apply_accelerator_settings(&app_handle);

                let handle = app_handle.clone();
                let args = cli_args.clone();
                std::thread::spawn(move || {
                    let code = headless::run_headless_guarded(|| {
                        headless::run_headless_transcription(&handle, &args)
                    });
                    // Drop the loaded engine before teardown: ggml-metal's global
                    // device free asserts (SIGABRT) if a model's Metal resources
                    // are still alive at C++ static-destructor time.
                    if let Some(tm) = handle.try_state::<Arc<TranscriptionManager>>() {
                        let _ = tm.unload_model();
                    }
                    // process::exit (not app.exit, which exits 0 regardless) so the
                    // exit code propagates to the shell for CI gating. Flush first
                    // since process::exit runs no destructors / buffer flushes.
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(code);
                });
                return Ok(());
            }

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Speak")
                    .inner_size(680.0, 570.0)
                    .min_inner_size(680.0, 570.0)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false);

            if let Some(data_dir) = portable::data_dir() {
                win_builder = win_builder.data_directory(data_dir.join("webview"));
            }

            win_builder.build()?;

            // One-time migration: move plain-text API keys from the settings
            // store into the OS keychain (secret_store). Keep the migration and
            // persistence in one transaction so startup cannot overwrite a
            // concurrent settings change.
            let mut settings = settings::update_settings(app.handle(), |settings| {
                secret_store::migrate_legacy_settings_secrets(settings);
                settings.clone()
            });

            // Apply the persisted appearance theme to the native title bar before
            // the window is shown, so it matches the in-app palette without a flash
            // of the wrong theme. See `apply_window_theme` for what this does per
            // platform.
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            shortcut::apply_window_theme(app.handle(), settings.theme);

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            set_file_log_level(file_log_level.to_level_filter());
            // Only forward logs to the webview while debug mode is on (the live log
            // viewer is the sole consumer and only exists in debug mode). This also
            // honors the runtime `--debug` override applied to `settings` above.
            set_webview_log_streaming(settings.debug_mode);
            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));

            initialize_core_logic(&app_handle);

            // Secure Input monitor (macOS): detects stuck secure input that
            // silently blocks keyed shortcuts, warns the user, and activates
            // the Carbon fallback. See secure_input.rs and issue #1578.
            secure_input::init(&app_handle);

            // Populate the overlay-enabled cache from initial settings so the
            // audio path (overlay::emit_levels, called ~24 Hz during recording)
            // can do a single atomic load instead of reading the Tauri store.
            // Kept in sync by shortcut::change_overlay_style_setting.
            overlay::update_overlay_enabled_cache(
                settings.overlay_style != settings::OverlayStyle::None,
            );

            // Pre-warm GPU/accelerator enumeration on a background thread. The first
            // get_available_accelerators call enumerates ORT execution providers and
            // transcribe-cpp compute devices, which can take a moment; without this
            // the cost is paid synchronously when the user first opens Advanced
            // settings, freezing the UI. Result is cached in a OnceLock.
            std::thread::spawn(|| {
                let _ = crate::managers::transcription::get_available_accelerators();
            });

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            // But if permission onboarding is required, always show the window.
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            let should_force_show = should_force_show_permissions_window(&app_handle);

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            if should_force_show || !should_hide || !tray_available {
                utils::show_main_window(&app_handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _res = window.hide();

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Re-apply the current tray state with the new theme's icon set
                utils::refresh_tray_icon(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match &event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                utils::show_main_window(app);
            }
            // Teardown transcribe.cpp before exit
            tauri::RunEvent::Exit => {
                if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
                    let _ = tm.unload_model();
                }
            }
            _ => {}
        });
}
