import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import type {
  AssistantOutputMode,
  LogLevel,
  AppSettings as Settings,
  AudioDevice,
  TranscribeAcceleratorSetting,
  OrtAcceleratorSetting,
  Result,
  SpeechInputProvider,
  RecordingAudioMode,
} from "@/bindings";
import { commands } from "@/bindings";

interface SettingsStore {
  settings: Settings | null;
  defaultSettings: Settings | null;
  isLoading: boolean;
  isUpdating: Record<string, boolean>;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  customSounds: { start: boolean; stop: boolean };
  postProcessModelOptions: Record<string, string[]>;
  /** Model lists keyed by *profile* id — each assistant lists with its own key. */
  assistantModelOptions: Record<string, string[]>;
  /** Boolean-only keychain state; secret values never cross IPC. */
  assistantApiKeyConfigured: Record<string, boolean>;

  // Actions
  initialize: () => Promise<void>;
  loadDefaultSettings: () => Promise<void>;
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<void>;
  resetSetting: <K extends keyof Settings>(key: K) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;
  isUpdatingKey: (key: string) => boolean;
  playTestSound: (soundType: "start" | "stop") => Promise<void>;
  checkCustomSounds: () => Promise<void>;
  setPostProcessProvider: (providerId: string) => Promise<void>;
  updatePostProcessSetting: (
    settingType: "base_url" | "api_key" | "model",
    providerId: string,
    value: string,
  ) => Promise<void>;
  updatePostProcessBaseUrl: (
    providerId: string,
    baseUrl: string,
  ) => Promise<void>;
  updatePostProcessApiKey: (
    providerId: string,
    apiKey: string,
  ) => Promise<void>;
  updatePostProcessModel: (providerId: string, model: string) => Promise<void>;
  fetchPostProcessModels: (providerId: string) => Promise<string[]>;
  setPostProcessModelOptions: (providerId: string, models: string[]) => void;

  // Assistant-profile helpers. Every one of these is keyed by *profile* id
  // ("fast_helper" / "smart_assistant"), never by provider id: two profiles
  // may point at the same provider with different credentials.
  setAssistantProvider: (
    profileId: string,
    providerId: string,
  ) => Promise<void>;
  updateAssistantApiKey: (profileId: string, apiKey: string) => Promise<void>;
  updateAssistantModel: (profileId: string, model: string) => Promise<void>;
  updateAssistantSystemPrompt: (
    profileId: string,
    prompt: string,
  ) => Promise<void>;
  updateAssistantLimits: (
    profileId: string,
    maxOutputTokens: number,
    timeoutSecs: number,
  ) => Promise<void>;
  setAssistantOutputMode: (
    profileId: string,
    mode: AssistantOutputMode,
  ) => Promise<void>;
  fetchAssistantModels: (profileId: string) => Promise<string[]>;

  // Internal state setters
  setSettings: (settings: Settings | null) => void;
  setDefaultSettings: (defaultSettings: Settings | null) => void;
  setLoading: (loading: boolean) => void;
  setUpdating: (key: string, updating: boolean) => void;
  setAudioDevices: (devices: AudioDevice[]) => void;
  setOutputDevices: (devices: AudioDevice[]) => void;
  setCustomSounds: (sounds: { start: boolean; stop: boolean }) => void;
}

// Note: Default settings are now fetched from Rust via commands.getDefaultSettings()
// This ensures platform-specific defaults (like overlay_position, shortcuts, paste_method) work correctly

const DEFAULT_AUDIO_DEVICE: AudioDevice = {
  index: "default",
  name: "Default",
  is_default: true,
};

const requireCommandSuccess = <T>(result: Result<T, string>): T => {
  if (result.status === "error") {
    throw new Error(result.error);
  }
  return result.data;
};

// Preserve user order for repeated edits of the same setting. The version
// guard prevents an older failed request from rolling back a newer optimistic
// value while its command is still queued or in flight.
const settingUpdateQueues = new Map<string, Promise<void>>();
const settingUpdateVersions = new Map<string, number>();

const enqueueSettingUpdate = <T>(
  key: string,
  update: () => Promise<T>,
): Promise<T> => {
  const previous = settingUpdateQueues.get(key) ?? Promise.resolve();
  const result = previous.then(update, update);
  const tail = result.then(
    () => undefined,
    () => undefined,
  );
  settingUpdateQueues.set(key, tail);
  void tail.then(() => {
    if (settingUpdateQueues.get(key) === tail) {
      settingUpdateQueues.delete(key);
    }
  });
  return result;
};

/**
 * Shared shape of every assistant-profile write: raise the busy flag for
 * `updateKey`, run the command, then re-read settings so the UI shows what the
 * backend actually stored after its clamping and repair pass — not what the
 * form asked for.
 *
 * Failures are logged and swallowed, exactly like the post-processing
 * updaters: the refresh that then never runs leaves the previous, still-valid
 * value on screen instead of a half-applied one.
 */
const runAssistantWrite = async (
  get: () => SettingsStore,
  updateKey: string,
  run: () => Promise<Result<null, string>>,
): Promise<void> => {
  const { setUpdating, refreshSettings } = get();

  setUpdating(updateKey, true);
  try {
    requireCommandSuccess(await run());
    await refreshSettings();
  } catch (error) {
    console.error(`Failed to apply ${updateKey}:`, error);
  } finally {
    setUpdating(updateKey, false);
  }
};

const settingUpdaters: {
  [K in keyof Settings]?: (
    value: Settings[K],
  ) => Promise<Result<unknown, string>>;
} = {
  always_on_microphone: (value) =>
    commands.updateMicrophoneMode(value as boolean),
  audio_feedback: (value) =>
    commands.changeAudioFeedbackSetting(value as boolean),
  audio_feedback_volume: (value) =>
    commands.changeAudioFeedbackVolumeSetting(value as number),
  sound_theme: (value) => commands.changeSoundThemeSetting(value as string),
  start_hidden: (value) => commands.changeStartHiddenSetting(value as boolean),
  autostart_enabled: (value) =>
    commands.changeAutostartSetting(value as boolean),
  update_checks_enabled: (value) =>
    commands.changeUpdateChecksSetting(value as boolean),
  show_whats_new_on_update: (value) =>
    commands.changeShowWhatsNewOnUpdateSetting(value as boolean),
  whats_new_last_seen_version: (value) =>
    commands.changeWhatsNewLastSeenVersionSetting(value as string),
  push_to_talk: (value) => commands.changePttSetting(value as boolean),
  selected_microphone: (value) =>
    commands.setSelectedMicrophone(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  selected_channel: (value) =>
    commands.setSelectedChannel((value as number | null | undefined) ?? null),
  clamshell_microphone: (value) =>
    commands.setClamshellMicrophone(
      (value as string) === "Default" ? "default" : (value as string),
    ),
  selected_output_device: (value) =>
    commands.setSelectedOutputDevice(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  recording_retention_period: (value) =>
    commands.updateRecordingRetentionPeriod(value as string),
  translate_to_english: (value) =>
    commands.changeTranslateToEnglishSetting(value as boolean),
  selected_language: (value) =>
    commands.changeSelectedLanguageSetting(value as string),
  speech_input_provider: (value) =>
    commands.changeSpeechInputProviderSetting(value as SpeechInputProvider),
  meeting_stt_provider: (value) =>
    commands.changeMeetingSttProviderSetting(value as SpeechInputProvider),
  azure_speech_endpoint: (value) =>
    commands.changeAzureSpeechEndpointSetting(value as string),
  azure_speech_language: (value) =>
    commands.changeAzureSpeechLanguageSetting(value as string),
  azure_stable_partial_threshold: (value) =>
    commands.changeAzureStablePartialThresholdSetting(value as number),
  azure_segmentation_silence_ms: (value) =>
    commands.changeAzureSegmentationSilenceSetting(value as number),
  custom_stt_endpoint: (value) =>
    commands.changeCustomSttEndpointSetting(value as string),
  deepl_translation_enabled: (value) =>
    commands.changeDeeplTranslationEnabledSetting(value as boolean),
  deepl_target_language: (value) =>
    commands.changeDeeplTargetLanguageSetting(value as string),
  deepl_sentence_delay_ms: (value) =>
    commands.changeDeeplSentenceDelaySetting(value as number),
  overlay_position: (value) =>
    commands.changeOverlayPositionSetting(value as string),
  debug_mode: (value) => commands.changeDebugModeSetting(value as boolean),
  custom_words: (value) => commands.updateCustomWords(value as string[]),
  word_correction_threshold: (value) =>
    commands.changeWordCorrectionThresholdSetting(value as number),
  paste_delay_ms: (value) =>
    commands.changePasteDelayMsSetting(value as number),
  paste_delay_after_ms: (value) =>
    commands.changePasteDelayAfterMsSetting(value as number),
  reliable_paste: (value) =>
    commands.changeReliablePasteSetting(value as boolean),
  paste_method: (value) => commands.changePasteMethodSetting(value as string),
  typing_tool: (value) => commands.changeTypingToolSetting(value as string),
  external_script_path: (value) =>
    commands.changeExternalScriptPathSetting(value as string | null),
  clipboard_handling: (value) =>
    commands.changeClipboardHandlingSetting(value as string),
  auto_submit: (value) => commands.changeAutoSubmitSetting(value as boolean),
  auto_submit_key: (value) =>
    commands.changeAutoSubmitKeySetting(value as string),
  history_limit: (value) => commands.updateHistoryLimit(value as number),
  post_process_enabled: (value) =>
    commands.changePostProcessEnabledSetting(value as boolean),
  post_process_selected_prompt_id: (value) =>
    commands.setPostProcessSelectedPrompt(value as string),
  // Legacy bool, kept so an older call site keeps working. The backend
  // mirrors it onto `recording_audio_mode`; new UI writes the mode instead.
  mute_while_recording: (value) =>
    commands.changeMuteWhileRecordingSetting(value as boolean),
  recording_audio_mode: (value) =>
    commands.changeRecordingAudioModeSetting(value as RecordingAudioMode),
  duck_volume_percent: (value) =>
    commands.changeDuckVolumePercentSetting(value as number),
  duck_fade_ms: (value) => commands.changeDuckFadeMsSetting(value as number),
  append_trailing_space: (value) =>
    commands.changeAppendTrailingSpaceSetting(value as boolean),
  log_level: (value) => commands.setLogLevel(value as LogLevel),
  app_language: (value) => commands.changeAppLanguageSetting(value as string),
  theme: (value) => commands.changeThemeSetting(value as string),
  experimental_enabled: (value) =>
    commands.changeExperimentalEnabledSetting(value as boolean),
  lazy_stream_close: (value) =>
    commands.changeLazyStreamCloseSetting(value as boolean),
  overlay_style: (value) => commands.changeOverlayStyleSetting(value as string),
  vad_enabled: (value) => commands.changeVadEnabledSetting(value as boolean),
  filler_word_removal_enabled: (value) =>
    commands.changeFillerWordRemovalEnabledSetting(value as boolean),
  show_tray_icon: (value) =>
    commands.changeShowTrayIconSetting(value as boolean),
  transcribe_accelerator: (value) =>
    commands.changeTranscribeAcceleratorSetting(
      value as TranscribeAcceleratorSetting,
    ),
  ort_accelerator: (value) =>
    commands.changeOrtAcceleratorSetting(value as OrtAcceleratorSetting),
  transcribe_gpu_device: (value) =>
    commands.changeTranscribeGpuDevice(value as string | null),
  extra_recording_buffer_ms: (value) =>
    commands.changeExtraRecordingBufferSetting(value as number),
};

export const useSettingsStore = create<SettingsStore>()(
  subscribeWithSelector((set, get) => ({
    settings: null,
    defaultSettings: null,
    isLoading: true,
    isUpdating: {},
    audioDevices: [],
    outputDevices: [],
    customSounds: { start: false, stop: false },
    postProcessModelOptions: {},
    assistantModelOptions: {},
    assistantApiKeyConfigured: {},

    // Internal setters
    setSettings: (settings) => set({ settings }),
    setDefaultSettings: (defaultSettings) => set({ defaultSettings }),
    setLoading: (isLoading) => set({ isLoading }),
    setUpdating: (key, updating) =>
      set((state) => ({
        isUpdating: { ...state.isUpdating, [key]: updating },
      })),
    setAudioDevices: (audioDevices) => set({ audioDevices }),
    setOutputDevices: (outputDevices) => set({ outputDevices }),
    setCustomSounds: (customSounds) => set({ customSounds }),

    // Getters
    getSetting: (key) => get().settings?.[key],
    isUpdatingKey: (key) => get().isUpdating[key] || false,

    // Load settings from store
    refreshSettings: async () => {
      try {
        const [settingsAttempt, keyStatusAttempt] = await Promise.allSettled([
          commands.getAppSettings(),
          commands.getAssistantApiKeyStatus(),
        ]);

        if (keyStatusAttempt.status === "fulfilled") {
          const keyStatus = keyStatusAttempt.value;
          if (keyStatus.status === "ok") {
            const configured = Object.fromEntries(
              Object.entries(keyStatus.data).filter(
                (entry): entry is [string, boolean] =>
                  typeof entry[1] === "boolean",
              ),
            );
            set({ assistantApiKeyConfigured: configured });
          } else {
            console.error(
              "Failed to load assistant API key status:",
              keyStatus.error,
            );
          }
        } else {
          console.error(
            "Failed to load assistant API key status:",
            keyStatusAttempt.reason,
          );
        }

        if (settingsAttempt.status === "rejected") {
          throw settingsAttempt.reason;
        }
        const result = settingsAttempt.value;
        if (result.status === "ok") {
          const settings = result.data;
          const normalizedSettings: Settings = {
            ...settings,
            always_on_microphone: settings.always_on_microphone ?? false,
            selected_microphone: settings.selected_microphone ?? "Default",
            clamshell_microphone: settings.clamshell_microphone ?? "Default",
            selected_output_device:
              settings.selected_output_device ?? "Default",
          };
          set({ settings: normalizedSettings, isLoading: false });
        } else {
          console.error("Failed to load settings:", result.error);
          set({ isLoading: false });
        }
      } catch (error) {
        console.error("Failed to load settings:", error);
        set({ isLoading: false });
      }
    },

    // Load audio devices
    refreshAudioDevices: async () => {
      try {
        const result = await commands.getAvailableMicrophones();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ audioDevices: devicesWithDefault });
        } else {
          set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load audio devices:", error);
        set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Load output devices
    refreshOutputDevices: async () => {
      try {
        const result = await commands.getAvailableOutputDevices();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ outputDevices: devicesWithDefault });
        } else {
          set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load output devices:", error);
        set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Play a test sound
    playTestSound: async (soundType: "start" | "stop") => {
      try {
        await commands.playTestSound(soundType);
      } catch (error) {
        console.error(`Failed to play test sound (${soundType}):`, error);
      }
    },

    checkCustomSounds: async () => {
      try {
        const sounds = await commands.checkCustomSounds();
        get().setCustomSounds(sounds);
      } catch (error) {
        console.error("Failed to check custom sounds:", error);
      }
    },

    // Update a specific setting
    updateSetting: async <K extends keyof Settings>(
      key: K,
      value: Settings[K],
    ) => {
      const { settings, setUpdating } = get();
      const updateKey = String(key);
      const originalValue = settings?.[key];
      const updateVersion = (settingUpdateVersions.get(updateKey) ?? 0) + 1;
      settingUpdateVersions.set(updateKey, updateVersion);

      setUpdating(updateKey, true);

      try {
        set((state) => ({
          settings: state.settings ? { ...state.settings, [key]: value } : null,
        }));

        const updater = settingUpdaters[key];
        if (updater) {
          requireCommandSuccess(
            await enqueueSettingUpdate(updateKey, () => updater(value)),
          );
        } else if (key !== "bindings" && key !== "selected_model") {
          console.warn(`No handler for setting: ${String(key)}`);
        }
      } catch (error) {
        console.error(`Failed to update setting ${String(key)}:`, error);
        if (
          settings &&
          settingUpdateVersions.get(updateKey) === updateVersion
        ) {
          set((state) => ({
            settings: state.settings
              ? { ...state.settings, [key]: originalValue }
              : null,
          }));
        }
      } finally {
        if (settingUpdateVersions.get(updateKey) === updateVersion) {
          setUpdating(updateKey, false);
          settingUpdateVersions.delete(updateKey);
        }
      }
    },

    // Reset a setting to its default value
    resetSetting: async <K extends keyof Settings>(key: K) => {
      const { defaultSettings } = get();
      const defaultValue = defaultSettings?.[key];
      if (defaultValue !== undefined) {
        await get().updateSetting(key, defaultValue);
      }
    },

    // Update a specific binding
    updateBinding: async (id, binding) => {
      const { settings, setUpdating } = get();
      const updateKey = `binding_${id}`;
      const originalBinding = settings?.bindings?.[id]?.current_binding;

      setUpdating(updateKey, true);

      try {
        // Optimistic update.
        // The non-null assertion this replaces was hiding a real case: with no
        // existing entry for `id`, the spread produced `{ current_binding }`
        // alone — a Binding missing every other required field, which the UI
        // would then render. There is nothing to optimistically update for a
        // binding that does not exist, so leave the state alone.
        set((state) => {
          const existing = state.settings?.bindings?.[id];
          if (!state.settings || !existing) return state;
          return {
            settings: {
              ...state.settings,
              bindings: {
                ...state.settings.bindings,
                [id]: { ...existing, current_binding: binding },
              },
            },
          };
        });

        const result = await commands.changeBinding(id, binding);

        // Check if the command executed successfully
        if (result.status === "error") {
          throw new Error(result.error);
        }

        // Check if the binding change was successful
        if (!result.data.success) {
          throw new Error(result.data.error || "Failed to update binding");
        }
      } catch (error) {
        console.error(`Failed to update binding ${id}:`, error);

        // Rollback on error
        if (originalBinding) {
          set((state) => {
            const existing = state.settings?.bindings?.[id];
            if (!state.settings || !existing) return state;
            return {
              settings: {
                ...state.settings,
                bindings: {
                  ...state.settings.bindings,
                  [id]: { ...existing, current_binding: originalBinding },
                },
              },
            };
          });
        }

        // Re-throw to let the caller know it failed
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a specific binding
    resetBinding: async (id) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `binding_${id}`;

      setUpdating(updateKey, true);

      try {
        const result = requireCommandSuccess(await commands.resetBinding(id));
        if (!result.success) {
          throw new Error(result.error ?? "Failed to reset binding");
        }
        await refreshSettings();
      } catch (error) {
        console.error(`Failed to reset binding ${id}:`, error);
      } finally {
        setUpdating(updateKey, false);
      }
    },

    setPostProcessProvider: async (providerId) => {
      const {
        settings,
        setUpdating,
        refreshSettings,
        setPostProcessModelOptions,
      } = get();
      const updateKey = "post_process_provider_id";
      const previousId = settings?.post_process_provider_id ?? null;

      setUpdating(updateKey, true);

      if (settings) {
        set((state) => ({
          settings: state.settings
            ? { ...state.settings, post_process_provider_id: providerId }
            : null,
        }));
      }

      // Clear cached model options for the new provider so the dropdown
      // doesn't show stale models from a previous fetch or base_url.
      setPostProcessModelOptions(providerId, []);

      try {
        requireCommandSuccess(
          await commands.setPostProcessProvider(providerId),
        );
        await refreshSettings();
      } catch (error) {
        console.error("Failed to set post-process provider:", error);
        if (previousId !== null) {
          set((state) => ({
            settings: state.settings
              ? { ...state.settings, post_process_provider_id: previousId }
              : null,
          }));
        }
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Generic updater for post-processing provider settings
    updatePostProcessSetting: async (
      settingType: "base_url" | "api_key" | "model",
      providerId: string,
      value: string,
    ) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_${settingType}:${providerId}`;

      setUpdating(updateKey, true);

      try {
        if (settingType === "base_url") {
          requireCommandSuccess(
            await commands.changePostProcessBaseUrlSetting(providerId, value),
          );
        } else if (settingType === "api_key") {
          requireCommandSuccess(
            await commands.changePostProcessApiKeySetting(providerId, value),
          );
        } else if (settingType === "model") {
          requireCommandSuccess(
            await commands.changePostProcessModelSetting(providerId, value),
          );
        }
        await refreshSettings();
      } catch (error) {
        console.error(
          `Failed to update post-process ${settingType.replace("_", " ")}:`,
          error,
        );
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessBaseUrl: async (providerId, baseUrl) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_base_url:${providerId}`;

      setUpdating(updateKey, true);

      try {
        // Persist the new base URL first.
        requireCommandSuccess(
          await commands.changePostProcessBaseUrlSetting(providerId, baseUrl),
        );

        // Reset the stored model since the previous value is almost certainly
        // invalid for the new endpoint (e.g. switching Custom from Groq to
        // Cerebras). Only proceed if the reset succeeds.
        requireCommandSuccess(
          await commands.changePostProcessModelSetting(providerId, ""),
        );

        // Clear cached model options only after both backend writes succeed.
        set((state) => ({
          postProcessModelOptions: {
            ...state.postProcessModelOptions,
            [providerId]: [],
          },
        }));

        // Single refresh after both backend writes.
        await refreshSettings();
      } catch (error) {
        console.error("Failed to update post-process base URL:", error);
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessApiKey: async (providerId, apiKey) => {
      // Clear cached models when API key changes - user should click refresh after
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: [],
        },
      }));
      return get().updatePostProcessSetting("api_key", providerId, apiKey);
    },

    updatePostProcessModel: async (providerId, model) => {
      return get().updatePostProcessSetting("model", providerId, model);
    },

    fetchPostProcessModels: async (providerId) => {
      const updateKey = `post_process_models_fetch:${providerId}`;
      const { setUpdating, setPostProcessModelOptions } = get();

      setUpdating(updateKey, true);

      try {
        // Call Tauri backend command instead of fetch
        const result = await commands.fetchPostProcessModels(providerId);
        if (result.status === "ok") {
          setPostProcessModelOptions(providerId, result.data);
          return result.data;
        } else {
          console.error("Failed to fetch models:", result.error);
          return [];
        }
      } catch (error) {
        console.error("Failed to fetch models:", error);
        // Don't cache empty array on error - let user retry
        return [];
      } finally {
        setUpdating(updateKey, false);
      }
    },

    setPostProcessModelOptions: (providerId, models) =>
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: models,
        },
      })),

    setAssistantProvider: async (profileId, providerId) => {
      // Deliberately not optimistic, unlike `setPostProcessProvider`: the
      // backend resets the profile's model when the provider changes, so a
      // local guess would show a model the endpoint no longer accepts until
      // the refresh lands. `assistant_profiles` is one nested map, so the
      // refresh is a single read anyway.
      await runAssistantWrite(get, `assistant_provider:${profileId}`, () =>
        commands.setAssistantProvider(profileId, providerId),
      );
    },

    updateAssistantApiKey: async (profileId, apiKey) => {
      // A cached list was produced by the previous credential, so it says
      // nothing about what the new one can reach. Drop it and let the user
      // refresh rather than show models they may no longer have access to.
      set((state) => ({
        assistantModelOptions: {
          ...state.assistantModelOptions,
          [profileId]: [],
        },
      }));
      await runAssistantWrite(get, `assistant_api_key:${profileId}`, () =>
        commands.changeAssistantApiKeySetting(profileId, apiKey),
      );
    },

    updateAssistantModel: async (profileId, model) => {
      await runAssistantWrite(get, `assistant_model:${profileId}`, () =>
        commands.changeAssistantModelSetting(profileId, model),
      );
    },

    updateAssistantSystemPrompt: async (profileId, prompt) => {
      await runAssistantWrite(get, `assistant_system_prompt:${profileId}`, () =>
        commands.changeAssistantSystemPromptSetting(profileId, prompt),
      );
    },

    updateAssistantLimits: async (profileId, maxOutputTokens, timeoutSecs) => {
      // One command for both values: the backend clamps them together and a
      // split would let a rejected timeout leave a persisted token count.
      await runAssistantWrite(get, `assistant_limits:${profileId}`, () =>
        commands.changeAssistantLimitsSetting(
          profileId,
          maxOutputTokens,
          timeoutSecs,
        ),
      );
    },

    setAssistantOutputMode: async (profileId, mode) => {
      await runAssistantWrite(get, `assistant_output_mode:${profileId}`, () =>
        commands.setAssistantOutputMode(profileId, mode),
      );
    },

    /**
     * List the models this profile can reach, using the profile's own key.
     *
     * Not `fetchPostProcessModels`: that reads the post-processing credential
     * for the provider, which is a different secret. Asking it about an
     * assistant that only has its own key returns nothing, which is what made
     * the dropdown look broken.
     */
    fetchAssistantModels: async (profileId) => {
      const updateKey = `assistant_models_fetch:${profileId}`;
      const { setUpdating } = get();

      setUpdating(updateKey, true);
      try {
        const result = await commands.fetchAssistantModels(profileId);
        if (result.status === "ok") {
          set((state) => ({
            assistantModelOptions: {
              ...state.assistantModelOptions,
              [profileId]: result.data,
            },
          }));
          return result.data;
        }
        console.error("Failed to fetch assistant models:", result.error);
        return [];
      } catch (error) {
        console.error("Failed to fetch assistant models:", error);
        // Deliberately not cached: an error must stay retryable.
        return [];
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Load default settings from Rust
    loadDefaultSettings: async () => {
      try {
        const result = await commands.getDefaultSettings();
        if (result.status === "ok") {
          set({ defaultSettings: result.data });
        } else {
          console.error("Failed to load default settings:", result.error);
        }
      } catch (error) {
        console.error("Failed to load default settings:", error);
      }
    },

    // Initialize everything
    initialize: async () => {
      const { refreshSettings, checkCustomSounds, loadDefaultSettings } = get();

      // Note: Audio devices are NOT refreshed here. The frontend (App.tsx)
      // is responsible for calling refreshAudioDevices/refreshOutputDevices
      // after onboarding completes. This avoids triggering permission dialogs
      // on macOS before the user is ready.
      await Promise.all([
        loadDefaultSettings(),
        refreshSettings(),
        checkCustomSounds(),
      ]);

      // Re-fetch settings when the backend changes them (e.g. language
      // reset during model switch). The backend is the source of truth.
      listen("model-state-changed", () => {
        get().refreshSettings();
      });
      listen<{ setting?: string }>("settings-changed", (event) => {
        get().refreshSettings();
        if (event.payload.setting === "selected_microphone") {
          get().refreshAudioDevices();
        }
      });
    },
  })),
);
