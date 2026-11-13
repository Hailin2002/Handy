import { useEffect } from "react";
import { useSettingsStore } from "../stores/settingsStore";
import type {
  AssistantOutputMode,
  AppSettings as Settings,
  AudioDevice,
} from "@/bindings";

interface UseSettingsReturn {
  // State
  settings: Settings | null;
  isLoading: boolean;
  isUpdating: (key: string) => boolean;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  audioFeedbackEnabled: boolean;
  postProcessModelOptions: Record<string, string[]>;
  assistantApiKeyConfigured: Record<string, boolean>;

  // Actions
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<void>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;

  // Binding-specific actions
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;

  // Convenience getters
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;

  // Post-processing helpers
  setPostProcessProvider: (providerId: string) => Promise<void>;
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

  // Assistant-profile helpers (keyed by profile id, not provider id)
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
  assistantModelOptions: Record<string, string[]>;
}

export const useSettings = (): UseSettingsReturn => {
  const store = useSettingsStore();
  // Read through selectors rather than off `store`: calling useSettingsStore()
  // without one returns a fresh object identity on every state change, so an
  // effect depending on `store` would re-run on every unrelated update.
  const initialize = useSettingsStore((state) => state.initialize);
  const isLoading = useSettingsStore((state) => state.isLoading);

  // Initialize on first mount
  useEffect(() => {
    if (isLoading) {
      initialize();
    }
  }, [initialize, isLoading]);

  return {
    settings: store.settings,
    isLoading: store.isLoading,
    isUpdating: store.isUpdatingKey,
    audioDevices: store.audioDevices,
    outputDevices: store.outputDevices,
    audioFeedbackEnabled: store.settings?.audio_feedback || false,
    postProcessModelOptions: store.postProcessModelOptions,
    assistantApiKeyConfigured: store.assistantApiKeyConfigured,
    updateSetting: store.updateSetting,
    resetSetting: store.resetSetting,
    refreshSettings: store.refreshSettings,
    refreshAudioDevices: store.refreshAudioDevices,
    refreshOutputDevices: store.refreshOutputDevices,
    updateBinding: store.updateBinding,
    resetBinding: store.resetBinding,
    getSetting: store.getSetting,
    setPostProcessProvider: store.setPostProcessProvider,
    updatePostProcessBaseUrl: store.updatePostProcessBaseUrl,
    updatePostProcessApiKey: store.updatePostProcessApiKey,
    updatePostProcessModel: store.updatePostProcessModel,
    fetchPostProcessModels: store.fetchPostProcessModels,
    setAssistantProvider: store.setAssistantProvider,
    updateAssistantApiKey: store.updateAssistantApiKey,
    updateAssistantModel: store.updateAssistantModel,
    updateAssistantSystemPrompt: store.updateAssistantSystemPrompt,
    updateAssistantLimits: store.updateAssistantLimits,
    setAssistantOutputMode: store.setAssistantOutputMode,
    fetchAssistantModels: store.fetchAssistantModels,
    assistantModelOptions: store.assistantModelOptions,
  };
};
