import { useEffect } from "react";
import { useSettingsStore } from "../stores/settingsStore";
import type { AppSettings as Settings, AudioDevice } from "@/bindings";

interface UseSettingsReturn {
  // State
  settings: Settings | null;
  isLoading: boolean;
  isUpdating: (key: string) => boolean;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  audioFeedbackEnabled: boolean;
  postProcessModelOptions: Record<string, string[]>;

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
  };
};
