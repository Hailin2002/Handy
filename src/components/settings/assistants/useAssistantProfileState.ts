import { useCallback, useMemo } from "react";
import type { AssistantOutputMode, AssistantProfile } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import type { DropdownOption } from "@/components/ui/Dropdown";
import type { ModelOption } from "../PostProcessingSettingsApi/types";
import {
  ASSISTANT_LIMITS,
  type AssistantProfileId,
  clampAssistantLimit,
} from "./assistantProfiles";

const APPLE_PROVIDER_ID = "apple_intelligence";
const CUSTOM_PROVIDER_ID = "custom";

/**
 * Excluded for the same reason the post-processing hook excludes it: the local
 * Ollama entry exists only to back meeting summaries, and offering it here
 * would put a provider in the list that the rest of the assistant path was
 * never sized for.
 */
const MEETING_ONLY_LOCAL_PROVIDER_ID = "local_ollama";

export interface AssistantProfileState {
  /** Undefined only before the first settings read resolves. */
  profile: AssistantProfile | undefined;
  profileLoaded: boolean;

  providerOptions: DropdownOption[];
  providerId: string;
  isProviderUpdating: boolean;
  handleProviderSelect: (providerId: string) => void;

  isApiKeyConfigured: boolean;
  isApiKeyUpdating: boolean;
  handleApiKeyChange: (value: string) => void;
  handleRemoveApiKey: () => void;

  model: string;
  modelOptions: ModelOption[];
  isModelUpdating: boolean;
  isFetchingModels: boolean;
  /**
   * Whether the model list can be fetched right now: this profile has its own
   * key (`fetch_assistant_models` reads it under the assistant keychain scope),
   * or the provider is local/custom and needs none. When it cannot, the model
   * id can still be typed in, and the UI says so rather than looking broken.
   */
  canListModels: boolean;
  handleModelSelect: (value: string) => void;
  handleModelCreate: (value: string) => void;
  handleRefreshModels: () => void;

  systemPrompt: string;
  isSystemPromptUpdating: boolean;
  handleSystemPromptChange: (value: string) => void;

  maxOutputTokens: number;
  timeoutSecs: number;
  isLimitsUpdating: boolean;
  handleMaxOutputTokensChange: (value: number) => void;
  handleTimeoutSecsChange: (value: number) => void;

  outputMode: AssistantOutputMode;
  isOutputModeUpdating: boolean;
  handleOutputModeSelect: (mode: AssistantOutputMode) => void;
}

/**
 * Per-profile settings state, in the shape `usePostProcessProviderState`
 * established: settings are the single source of truth (no mirrored local copy
 * that can drift), every write is guarded by its own `isUpdating` key so the
 * two profiles never disable each other's controls, and the model list is the
 * fetched options merged with whatever value is currently stored.
 */
export const useAssistantProfileState = (
  profileId: AssistantProfileId,
): AssistantProfileState => {
  const {
    settings,
    isUpdating,
    fetchAssistantModels,
    assistantModelOptions,
    assistantApiKeyConfigured,
    setAssistantProvider,
    updateAssistantApiKey,
    updateAssistantModel,
    updateAssistantSystemPrompt,
    updateAssistantLimits,
    setAssistantOutputMode,
  } = useSettings();

  const profile = settings?.assistant_profiles?.[profileId];
  const profileLoaded = profile !== undefined;

  // Memoized because the `?? []` fallback would otherwise hand out a fresh
  // array identity on every render, defeating every memo below that lists it.
  const providers = useMemo(
    () =>
      (settings?.post_process_providers ?? []).filter(
        (provider) => provider.id !== MEETING_ONLY_LOCAL_PROVIDER_ID,
      ),
    [settings?.post_process_providers],
  );

  const providerOptions = useMemo<DropdownOption[]>(
    () =>
      providers.map((provider) => ({
        value: provider.id,
        label: provider.label,
      })),
    [providers],
  );

  const providerId = profile?.provider_id ?? "";
  const isApiKeyConfigured = assistantApiKeyConfigured[profileId] ?? false;
  const model = profile?.model ?? "";
  const systemPrompt = profile?.system_prompt ?? "";
  const maxOutputTokens =
    profile?.max_output_tokens ?? ASSISTANT_LIMITS.maxOutputTokens.min;
  const timeoutSecs = profile?.timeout_secs ?? ASSISTANT_LIMITS.timeoutSecs.min;
  const outputMode = profile?.output_mode ?? "show";

  const isProviderUpdating = isUpdating(`assistant_provider:${profileId}`);
  const isApiKeyUpdating = isUpdating(`assistant_api_key:${profileId}`);
  const isModelUpdating = isUpdating(`assistant_model:${profileId}`);
  const isSystemPromptUpdating = isUpdating(
    `assistant_system_prompt:${profileId}`,
  );
  const isLimitsUpdating = isUpdating(`assistant_limits:${profileId}`);
  const isOutputModeUpdating = isUpdating(`assistant_output_mode:${profileId}`);
  const isFetchingModels = isUpdating(`assistant_models_fetch:${profileId}`);

  const canListModels = useMemo(() => {
    if (!providerId || providerId === APPLE_PROVIDER_ID) return false;
    if (providerId === CUSTOM_PROVIDER_ID) {
      const provider = providers.find((entry) => entry.id === providerId);
      return (provider?.base_url ?? "").trim() !== "";
    }
    // This profile's own key. Checking the post-processing key here is what
    // made the dropdown stay empty for anyone who had only filled in the
    // assistant's field — the two are separate secrets by design.
    return isApiKeyConfigured;
  }, [isApiKeyConfigured, providerId, providers]);

  const handleProviderSelect = useCallback(
    (nextProviderId: string) => {
      if (!nextProviderId || nextProviderId === providerId) return;
      void setAssistantProvider(profileId, nextProviderId).then(() => {
        // The backend clears the model on a provider switch, so the dropdown
        // would otherwise sit empty with nothing to choose from. The key is
        // this profile's own, and it does not change with the provider.
        if (nextProviderId !== APPLE_PROVIDER_ID && isApiKeyConfigured) {
          void fetchAssistantModels(profileId);
        }
      });
    },
    [
      fetchAssistantModels,
      isApiKeyConfigured,
      profileId,
      providerId,
      setAssistantProvider,
    ],
  );

  const handleApiKeyChange = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      // An empty new-entry field is not a deletion request. Existing secrets
      // are removed only through the explicit Clear action below.
      if (!trimmed) return;
      void updateAssistantApiKey(profileId, trimmed);
    },
    [profileId, updateAssistantApiKey],
  );

  const handleRemoveApiKey = useCallback(() => {
    if (!isApiKeyConfigured) return;
    void updateAssistantApiKey(profileId, "");
  }, [isApiKeyConfigured, profileId, updateAssistantApiKey]);

  const handleModelSelect = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (trimmed === model) return;
      void updateAssistantModel(profileId, trimmed);
    },
    [model, profileId, updateAssistantModel],
  );

  const handleModelCreate = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (!trimmed || trimmed === model) return;
      void updateAssistantModel(profileId, trimmed);
    },
    [model, profileId, updateAssistantModel],
  );

  const handleRefreshModels = useCallback(() => {
    if (!providerId || providerId === APPLE_PROVIDER_ID) return;
    void fetchAssistantModels(profileId);
  }, [fetchAssistantModels, profileId, providerId]);

  const handleSystemPromptChange = useCallback(
    (value: string) => {
      if (value === systemPrompt) return;
      void updateAssistantSystemPrompt(profileId, value);
    },
    [profileId, systemPrompt, updateAssistantSystemPrompt],
  );

  const handleMaxOutputTokensChange = useCallback(
    (value: number) => {
      const clamped = clampAssistantLimit(
        value,
        ASSISTANT_LIMITS.maxOutputTokens,
        maxOutputTokens,
      );
      if (clamped === maxOutputTokens) return;
      void updateAssistantLimits(profileId, clamped, timeoutSecs);
    },
    [maxOutputTokens, profileId, timeoutSecs, updateAssistantLimits],
  );

  const handleTimeoutSecsChange = useCallback(
    (value: number) => {
      const clamped = clampAssistantLimit(
        value,
        ASSISTANT_LIMITS.timeoutSecs,
        timeoutSecs,
      );
      if (clamped === timeoutSecs) return;
      void updateAssistantLimits(profileId, maxOutputTokens, clamped);
    },
    [maxOutputTokens, profileId, timeoutSecs, updateAssistantLimits],
  );

  const handleOutputModeSelect = useCallback(
    (mode: AssistantOutputMode) => {
      if (mode === outputMode) return;
      void setAssistantOutputMode(profileId, mode);
    },
    [outputMode, profileId, setAssistantOutputMode],
  );

  // Same identity trap as `providers`: a fresh `[]` per render would make
  // `modelOptions` recompute on every render instead of when models change.
  const availableModels = useMemo(
    () => assistantModelOptions[profileId] ?? [],
    [assistantModelOptions, profileId],
  );

  const modelOptions = useMemo<ModelOption[]>(() => {
    const seen = new Set<string>();
    const options: ModelOption[] = [];

    const upsert = (value: string | null | undefined) => {
      const trimmed = value?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      options.push({ value: trimmed, label: trimmed });
    };

    for (const candidate of availableModels) upsert(candidate);
    // The stored value last, so a model the endpoint no longer lists is still
    // selectable rather than vanishing from its own dropdown.
    upsert(model);

    return options;
  }, [availableModels, model]);

  return {
    profile,
    profileLoaded,
    providerOptions,
    providerId,
    isProviderUpdating,
    handleProviderSelect,
    isApiKeyConfigured,
    isApiKeyUpdating,
    handleApiKeyChange,
    handleRemoveApiKey,
    model,
    modelOptions,
    isModelUpdating,
    isFetchingModels,
    canListModels,
    handleModelSelect,
    handleModelCreate,
    handleRefreshModels,
    systemPrompt,
    isSystemPromptUpdating,
    handleSystemPromptChange,
    maxOutputTokens,
    timeoutSecs,
    isLimitsUpdating,
    handleMaxOutputTokensChange,
    handleTimeoutSecsChange,
    outputMode,
    isOutputModeUpdating,
    handleOutputModeSelect,
  };
};
