import type { ModelInfo } from "@/bindings";
import type { MockInvocation, SpeakMockConfig } from "./types";
import { applySettingWrite } from "./settingCommands";
import {
  DEFAULT_MOCK_APP_VERSION,
  DEFAULT_MOCK_CURRENT_MODEL,
  DEFAULT_MOCK_LOCALE,
  DEFAULT_MOCK_SETTINGS,
  MOCK_MODELS,
} from "./fixtures";

/**
 * In-memory stand-in for the Rust backend: settings store, model catalog and
 * the invocation log used by tests to assert what crossed the IPC bridge.
 */
export class MockBackend {
  readonly config: SpeakMockConfig;

  settings: typeof DEFAULT_MOCK_SETTINGS;

  models: ModelInfo[];

  currentModel: string;

  readonly invocations: MockInvocation[];

  constructor(config: SpeakMockConfig) {
    this.config = config;
    this.settings = mergeSettings(config);
    this.models = config.models ?? MOCK_MODELS;
    this.currentModel =
      config.currentModel ??
      config.settings?.selected_model ??
      DEFAULT_MOCK_CURRENT_MODEL;
    this.invocations = [];
  }

  record(command: string, args: Record<string, unknown>): void {
    this.invocations.push({ command, args });
  }
}

const mergeSettings = (config: SpeakMockConfig): typeof DEFAULT_MOCK_SETTINGS => ({
  ...DEFAULT_MOCK_SETTINGS,
  ...config.settings,
});

/**
 * Resolves one IPC command. Returns `undefined` when no handler matches so
 * the runtime can fail loudly on genuinely unimplemented commands.
 *
 * Handlers return the RAW payload exactly as the Rust backend would serialize
 * it over the wire: the tauri-specta generated bindings in `src/bindings.ts`
 * add their own `{ status: "ok", data }` wrapper around every invoke result,
 * so wrapping here would double-encode and break every `result.data` reader.
 */
export const handleCommand = (
  backend: MockBackend,
  command: string,
  args: Record<string, unknown>,
): unknown | undefined => {
  // Persisted setting writes (tauri-specta Result commands resolve to null).
  if (applySettingWrite(backend, command, args)) return null;

  switch (command) {
    // --- app / window plumbing -------------------------------------------
    case "initialize_enigo":
    case "initialize_shortcuts":
    case "show_main_window_command":
    case "suspend_all_bindings":
    case "resume_all_bindings":
    case "play_test_sound":
    case "cancel_operation":
    case "rescan_local_models":
    case "set_selected_microphone":
    case "set_selected_output_device":
    case "set_clamshell_microphone":
    case "set_selected_channel":
    case "set_log_level":
    case "update_custom_words":
    case "set_model_unload_timeout":
    case "open_recordings_folder":
    case "open_log_dir":
    case "open_app_data_dir":
    case "open_microphone_privacy_settings":
    case "change_binding":
    case "reset_binding":
      return null;

    case "set_active_model": {
      const modelId = args.modelId;
      if (typeof modelId === "string") backend.currentModel = modelId;
      return null;
    }

    // --- tauri-specta readers (raw payloads; bindings add the Result) -----
    case "get_app_settings":
      return { ...backend.settings };
    case "get_default_settings":
      return { ...DEFAULT_MOCK_SETTINGS };
    case "get_app_dir_path":
      return "C:\\mock\\speak\\data";
    case "get_log_dir_path":
      return "C:\\mock\\speak\\logs";
    case "get_available_microphones":
      return [
        { index: "default", name: "Default", is_default: true },
        { index: "mic-0", name: "Mock Microphone", is_default: false },
      ];
    case "get_available_output_devices":
      return [{ index: "default", name: "Default", is_default: true }];
    case "get_microphone_channels":
      return 2;
    case "get_selected_microphone":
    case "get_selected_output_device":
    case "get_clamshell_microphone":
      return "default";
    case "get_history_entries":
      return { entries: [], has_more: false };
    case "fetch_post_process_models":
      return [];
    case "is_laptop":
      return false;

    // --- raw (non-Result) bindings ---------------------------------------
    case "get_available_models":
      return backend.models.map((model) => ({ ...model }));
    case "get_current_model":
      return backend.currentModel;
    case "get_transcription_model_status":
      return backend.currentModel.length > 0 ? backend.currentModel : null;
    case "check_custom_sounds":
      return { start: false, stop: false };
    case "is_recording":
      return false;
    case "is_portable":
      return false;
    case "get_secure_input_status":
      return {
        enabled: false,
        sustained: false,
        culprit_pid: null,
        culprit_name: null,
        fallback_active: false,
        covered_bindings: [],
        degraded_bindings: [],
        uncovered_bindings: [],
        recorder_blocked: false,
      };
    case "get_windows_microphone_permission_status":
      return {
        supported: false,
        overall_access: "allowed",
        device_access: "allowed",
        app_access: "allowed",
        desktop_app_access: "allowed",
      };
    case "get_keyboard_implementation":
      return "tauri";
    case "get_available_accelerators":
      return { transcribe: ["cpu"], ort: ["cpu"], gpu_devices: [] };
    case "azure_speech_status":
      return null;
    case "test_azure_speech_connection":
      return "disabled in e2e mocks";

    // --- plugins -----------------------------------------------------------
    case "plugin:os|platform":
    case "plugin:os|type":
      return "linux";
    case "plugin:os|family":
      return "unix";
    case "plugin:os|arch":
      return "x86_64";
    case "plugin:os|version":
      return "6.6.0";
    case "plugin:os|locale":
      return backend.config.locale ?? DEFAULT_MOCK_LOCALE;
    case "plugin:os|hostname":
      return "e2e-host";
    case "plugin:app|version":
      return backend.config.appVersion ?? DEFAULT_MOCK_APP_VERSION;
    case "plugin:app|name":
      return "Speak";
    case "plugin:app|os":
      return "linux";
    case "plugin:app|identifier":
      return "com.e2e.speak";
    case "plugin:event|listen":
      // Handled by the runtime (needs the callback registry).
      return LISTEN_SENTINEL;
    case "plugin:event|unlisten":
      return null;
    case "plugin:event|emit":
      return null;
    case "plugin:updater|check":
      // No update available.
      return null;
    case "plugin:process|relaunch":
    case "plugin:process|exit":
      return null;
    case "plugin:opener|open_url":
    case "plugin:opener|open_path":
      return null;
    case "plugin:dialog|ask":
    case "plugin:dialog|confirm":
      return true;
    case "plugin:dialog|message":
      return null;
    default:
      return undefined;
  }
};

// Marker consumed by the runtime; never returned to the frontend.
export const LISTEN_SENTINEL = Symbol("mock-event-listen");
