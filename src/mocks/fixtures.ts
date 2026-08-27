import type { AppSettings, ModelInfo, ShortcutBinding } from "@/bindings";

/**
 * Deterministic fixtures for the IPC mock. Values mirror what the real
 * backend returns for a fully onboarded Linux install with two downloaded
 * models so the settings UI renders every primary region.
 */

const MOCK_BINDINGS: Record<string, ShortcutBinding> = {
  transcribe: {
    id: "transcribe",
    name: "Transcribe",
    description: "Start or stop transcription",
    default_binding: "CommandOrControl+Shift+Space",
    current_binding: "CommandOrControl+Shift+Space",
  },
  cancel: {
    id: "cancel",
    name: "Cancel",
    description: "Cancel the current transcription",
    default_binding: "Escape",
    current_binding: "Escape",
  },
};

export const DEFAULT_MOCK_SETTINGS: AppSettings = {
  settings_schema_version: 1,
  bindings: MOCK_BINDINGS,
  onboarding_completed: true,
  push_to_talk: false,
  audio_feedback: false,
  audio_feedback_volume: 0.8,
  sound_theme: "pop",
  start_hidden: false,
  autostart_enabled: false,
  update_checks_enabled: false,
  show_whats_new_on_update: false,
  whats_new_last_seen_version: "9999.0.0",
  selected_model: "small",
  always_on_microphone: false,
  selected_microphone: "Default",
  selected_channel: null,
  clamshell_microphone: null,
  selected_output_device: "Default",
  translate_to_english: false,
  selected_language: "auto",
  speech_input_provider: "local",
  azure_speech_endpoint: "",
  azure_speech_language: "en-US",
  custom_stt_endpoint: "",
  deepl_translation_enabled: false,
  deepl_target_language: "en-US",
  overlay_style: "minimal",
  debug_mode: false,
  log_level: "info",
  custom_words: [],
  model_unload_timeout: "never",
  word_correction_threshold: 0,
  history_limit: 100,
  recording_retention_period: "preserve_limit",
  paste_method: "ctrl_v",
  clipboard_handling: "dont_modify",
  auto_submit: false,
  auto_submit_key: "enter",
  post_process_enabled: false,
  post_process_provider_id: "",
  post_process_providers: [],
  post_process_api_keys: {},
  post_process_models: {},
  post_process_prompts: [],
  post_process_selected_prompt_id: null,
  mute_while_recording: false,
  append_trailing_space: true,
  app_language: "en",
  theme: "system",
  experimental_enabled: false,
  lazy_stream_close: false,
  keyboard_implementation: "tauri",
  show_tray_icon: true,
  paste_delay_ms: 0,
  paste_delay_after_ms: 0,
  reliable_paste: false,
  typing_tool: "auto",
  external_script_path: null,
  filler_word_removal_enabled: false,
  custom_filler_words: null,
  transcribe_accelerator: "auto",
  ort_accelerator: "auto",
  transcribe_gpu_device: null,
  extra_recording_buffer_ms: 0,
  vad_enabled: true,
};

const baseModel = {
  is_downloading: false,
  partial_size: 0,
  is_directory: false,
  // 0..1, matching the contract catalog/mod.rs enforces
  // (`assert!((0.0..=1.0).contains(&d.accuracy_score))`) after dividing the
  // catalog's 0..100 figure by 100. The fixture used a 0..10 scale, so the
  // score bars rendered at 400% width — clipped by the parent, and therefore
  // invisible, but wrong.
  accuracy_score: 0.8,
  speed_score: 0.6,
  supports_translation: false,
  is_recommended: false,
  supports_language_selection: true,
  is_custom: false,
  supports_streaming: false,
  supports_language_detection: true,
} satisfies Omit<
  ModelInfo,
  | "id"
  | "name"
  | "description"
  | "filename"
  | "source"
  | "size_mb"
  | "is_downloaded"
  | "engine_type"
  | "supported_languages"
>;

/** Downloaded and downloadable models; ids match i18n `onboarding.models.*`. */
export const MOCK_MODELS: ModelInfo[] = [
  {
    ...baseModel,
    id: "small",
    name: "Whisper Small",
    description: "Small Whisper model",
    filename: "ggml-small.bin",
    source: "Local",
    size_mb: 466,
    is_downloaded: true,
    engine_type: "TranscribeCpp",
    supported_languages: ["en", "de"],
  },
  {
    ...baseModel,
    id: "parakeet-tdt-0.6b-v3",
    name: "Parakeet TDT 0.6b v3",
    description: "Fast multilingual Parakeet model",
    filename: "parakeet-tdt-0.6b-v3.int8.onnx",
    source: { HuggingFace: { repo_id: "acme/parakeet", revision: "main" } },
    size_mb: 620,
    is_downloaded: true,
    engine_type: "Parakeet",
    supports_streaming: true,
    supported_languages: ["en", "de"],
  },
  {
    ...baseModel,
    id: "large",
    name: "Whisper Large",
    description: "Large Whisper model",
    filename: "ggml-large.bin",
    source: {
      Url: { url: "https://example.invalid/ggml-large.bin", sha256: null },
    },
    size_mb: 2900,
    is_downloaded: false,
    engine_type: "TranscribeCpp",
    supported_languages: ["en", "de"],
  },
];

export const DEFAULT_MOCK_CURRENT_MODEL = "small";

export const DEFAULT_MOCK_APP_VERSION = "0.0.0-e2e";

export const DEFAULT_MOCK_LOCALE = "en-US";
