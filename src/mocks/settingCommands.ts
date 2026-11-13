import type {
  AppSettings,
  AutoSubmitKey,
  ClipboardHandling,
  OverlayStyle,
  PasteMethod,
  RecordingAudioMode,
  RecordingRetentionPeriod,
  SoundTheme,
  SpeechInputProvider,
  Theme,
} from "@/bindings";
import type { MockBackend } from "./backend";

/**
 * Persisted-setting handlers for the IPC mock.
 *
 * Every tauri-specta generated `change_*_setting` command resolves to
 * `Result<null, string>` on the real backend and persists one settings key;
 * the definitions below do exactly that against the in-memory mock store,
 * so tests can assert persistence through the same bridge the UI uses.
 */

const asBoolean = (raw: unknown): boolean => raw === true;

const asString = (raw: unknown): string =>
  typeof raw === "string" ? raw : String(raw ?? "");

const asNumber = (raw: unknown): number =>
  typeof raw === "number" ? raw : Number(raw);

/** Coerces an IPC string into a settings enum member (mock input is trusted). */
const asEnum = <T extends string>(raw: unknown): T => asString(raw) as T;

interface SettingWriteDef {
  /** Rust command name as referenced by `src/bindings.ts`. */
  command: string;
  apply: (settings: AppSettings, args: Record<string, unknown>) => void;
}

/** Command-name → settings-key writes, mirroring the Rust store. */
export const SETTING_WRITE_DEFS: readonly SettingWriteDef[] = [
  {
    command: "change_audio_feedback_setting",
    apply: (s, a) => {
      s.audio_feedback = asBoolean(a.enabled);
    },
  },
  {
    command: "change_audio_feedback_volume_setting",
    apply: (s, a) => {
      s.audio_feedback_volume = asNumber(a.volume);
    },
  },
  {
    command: "change_sound_theme_setting",
    apply: (s, a) => {
      s.sound_theme = asEnum<SoundTheme>(a.theme);
    },
  },
  {
    command: "change_theme_setting",
    apply: (s, a) => {
      s.theme = asEnum<Theme>(a.theme);
    },
  },
  {
    command: "change_app_language_setting",
    apply: (s, a) => {
      s.app_language = asString(a.language);
    },
  },
  {
    command: "change_debug_mode_setting",
    apply: (s, a) => {
      s.debug_mode = asBoolean(a.enabled);
    },
  },
  {
    command: "change_start_hidden_setting",
    apply: (s, a) => {
      s.start_hidden = asBoolean(a.enabled);
    },
  },
  {
    command: "change_autostart_setting",
    apply: (s, a) => {
      s.autostart_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_update_checks_setting",
    apply: (s, a) => {
      s.update_checks_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_show_whats_new_on_update_setting",
    apply: (s, a) => {
      s.show_whats_new_on_update = asBoolean(a.enabled);
    },
  },
  {
    command: "change_whats_new_last_seen_version_setting",
    apply: (s, a) => {
      s.whats_new_last_seen_version = asString(a.version);
    },
  },
  {
    command: "change_ptt_setting",
    apply: (s, a) => {
      s.push_to_talk = asBoolean(a.enabled);
    },
  },
  {
    command: "change_translate_to_english_setting",
    apply: (s, a) => {
      s.translate_to_english = asBoolean(a.enabled);
    },
  },
  {
    command: "change_selected_language_setting",
    apply: (s, a) => {
      s.selected_language = asString(a.language);
    },
  },
  {
    command: "change_speech_input_provider_setting",
    apply: (s, a) => {
      s.speech_input_provider = asEnum<SpeechInputProvider>(a.provider);
    },
  },
  {
    command: "change_azure_speech_endpoint_setting",
    apply: (s, a) => {
      s.azure_speech_endpoint = asString(a.endpoint);
    },
  },
  {
    command: "change_azure_speech_language_setting",
    apply: (s, a) => {
      s.azure_speech_language = asString(a.language);
    },
  },
  {
    command: "change_deepl_translation_enabled_setting",
    apply: (s, a) => {
      s.deepl_translation_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_overlay_style_setting",
    apply: (s, a) => {
      s.overlay_style = asEnum<OverlayStyle>(a.style);
    },
  },
  {
    command: "change_post_process_enabled_setting",
    apply: (s, a) => {
      s.post_process_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_experimental_enabled_setting",
    apply: (s, a) => {
      s.experimental_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_lazy_stream_close_setting",
    apply: (s, a) => {
      s.lazy_stream_close = asBoolean(a.enabled);
    },
  },
  {
    command: "change_vad_enabled_setting",
    apply: (s, a) => {
      s.vad_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_filler_word_removal_enabled_setting",
    apply: (s, a) => {
      s.filler_word_removal_enabled = asBoolean(a.enabled);
    },
  },
  {
    command: "change_show_tray_icon_setting",
    apply: (s, a) => {
      s.show_tray_icon = asBoolean(a.enabled);
    },
  },
  {
    command: "change_mute_while_recording_setting",
    apply: (s, a) => {
      s.mute_while_recording = asBoolean(a.enabled);
      // The real backend mirrors the legacy bool onto the mode; mirror it here
      // too, or a test that writes the bool would leave the two disagreeing.
      s.recording_audio_mode = asBoolean(a.enabled) ? "mute" : "off";
    },
  },
  {
    command: "change_recording_audio_mode_setting",
    apply: (s, a) => {
      const mode = asEnum<RecordingAudioMode>(a.mode);
      s.recording_audio_mode = mode;
      s.mute_while_recording = mode === "mute";
    },
  },
  {
    command: "change_duck_volume_percent_setting",
    apply: (s, a) => {
      s.duck_volume_percent = asNumber(a.percent);
    },
  },
  {
    command: "change_duck_fade_ms_setting",
    apply: (s, a) => {
      s.duck_fade_ms = asNumber(a.ms);
    },
  },
  {
    command: "change_append_trailing_space_setting",
    apply: (s, a) => {
      s.append_trailing_space = asBoolean(a.enabled);
    },
  },
  {
    command: "change_reliable_paste_setting",
    apply: (s, a) => {
      s.reliable_paste = asBoolean(a.enabled);
    },
  },
  {
    command: "change_paste_method_setting",
    apply: (s, a) => {
      s.paste_method = asEnum<PasteMethod>(a.method);
    },
  },
  {
    command: "change_clipboard_handling_setting",
    apply: (s, a) => {
      s.clipboard_handling = asEnum<ClipboardHandling>(a.handling);
    },
  },
  {
    command: "change_auto_submit_setting",
    apply: (s, a) => {
      s.auto_submit = asBoolean(a.enabled);
    },
  },
  {
    command: "change_auto_submit_key_setting",
    apply: (s, a) => {
      s.auto_submit_key = asEnum<AutoSubmitKey>(a.key);
    },
  },
  {
    command: "change_word_correction_threshold_setting",
    apply: (s, a) => {
      s.word_correction_threshold = asNumber(a.threshold);
    },
  },
  {
    command: "change_paste_delay_ms_setting",
    apply: (s, a) => {
      s.paste_delay_ms = asNumber(a.ms);
    },
  },
  {
    command: "change_paste_delay_after_ms_setting",
    apply: (s, a) => {
      s.paste_delay_after_ms = asNumber(a.ms);
    },
  },
  {
    command: "change_extra_recording_buffer_setting",
    apply: (s, a) => {
      s.extra_recording_buffer_ms = asNumber(a.ms);
    },
  },
  {
    command: "update_history_limit",
    apply: (s, a) => {
      s.history_limit = asNumber(a.limit);
    },
  },
  {
    command: "update_recording_retention_period",
    apply: (s, a) => {
      s.recording_retention_period = asEnum<RecordingRetentionPeriod>(a.period);
    },
  },
];

/**
 * Applies a known setting-write command. Returns `false` when `command` is
 * not a recognized settings write so callers can fall through to other
 * handlers.
 */
export const applySettingWrite = (
  backend: MockBackend,
  command: string,
  args: Record<string, unknown>,
): boolean => {
  const def = SETTING_WRITE_DEFS.find((entry) => entry.command === command);
  if (!def) return false;
  def.apply(backend.settings, args);
  return true;
};
