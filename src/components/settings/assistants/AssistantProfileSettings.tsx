import React from "react";
import { CheckCircle2, RefreshCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantOutputMode } from "@/bindings";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { Dropdown } from "@/components/ui/Dropdown";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { ModelSelect } from "../PostProcessingSettingsApi/ModelSelect";
import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { AssistantLimitField } from "./AssistantLimitField";
import { AssistantPromptField } from "./AssistantPromptField";
import {
  ASSISTANT_LIMITS,
  ASSISTANT_OUTPUT_MODES,
  type AssistantProfileId,
} from "./assistantProfiles";
import { useAssistantProfileState } from "./useAssistantProfileState";

interface AssistantProfileSettingsProps {
  profileId: AssistantProfileId;
}

/**
 * Everything one assistant profile owns, in one group: its shortcut, where it
 * asks (provider / key / model), what it is told to be (system prompt), how
 * much work it may do (answer length, deadline) and what happens to the answer.
 *
 * The profiles ship with an empty model on purpose — the repo does not hardcode
 * vendor model ids — so "no model yet" is the expected first-run state and is
 * surfaced as a visible prompt to pick one, not hidden as an error.
 */
export const AssistantProfileSettings: React.FC<
  AssistantProfileSettingsProps
> = ({ profileId }) => {
  const { t } = useTranslation();
  const {
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
  } = useAssistantProfileState(profileId);

  const outputModeOptions = ASSISTANT_OUTPUT_MODES.map((mode) => ({
    value: mode,
    label: t(`settings.assistants.outputModes.${mode}.label`),
  }));
  const providerLabel =
    providerOptions.find((option) => option.value === providerId)?.label ??
    (providerId || t(`settings.assistants.profiles.${profileId}.title`));
  return (
    <SettingsGroup
      title={t(`settings.assistants.profiles.${profileId}.title`)}
      description={t(`settings.assistants.profiles.${profileId}.description`)}
    >
      {model.trim() === "" && (
        <Alert variant="warning" contained>
          {t("settings.assistants.model.required")}
        </Alert>
      )}

      <SettingContainer
        title={t("settings.assistants.provider.title")}
        description={t("settings.assistants.provider.description")}
        grouped
      >
        <ProviderSelect
          options={providerOptions}
          value={providerId}
          onChange={handleProviderSelect}
          disabled={isProviderUpdating}
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.apiKey.title")}
        description={t("settings.assistants.apiKey.description")}
        grouped
      >
        <div className="flex w-full flex-col items-end gap-1.5">
          <div className="flex w-full items-center gap-2">
            <ApiKeyField
              value=""
              onBlur={handleApiKeyChange}
              disabled={isApiKeyUpdating}
              placeholder={t("settings.assistants.apiKey.placeholder")}
              className="w-full"
              clearAfterBlur
            />
            {isApiKeyConfigured && (
              <Button
                variant="secondary"
                size="md"
                onClick={handleRemoveApiKey}
                disabled={isApiKeyUpdating}
              >
                {t("common.clear")}
              </Button>
            )}
          </div>
          <p
            role="status"
            aria-live="polite"
            className={`flex items-center gap-1 text-xs ${
              isApiKeyConfigured ? "text-green-600" : "text-mid-gray"
            }`}
          >
            {isApiKeyConfigured && (
              <CheckCircle2 className="h-3.5 w-3.5" aria-hidden="true" />
            )}
            {t(
              isApiKeyConfigured
                ? "settings.assistants.apiKey.configured"
                : "settings.assistants.apiKey.notConfigured",
              { provider: providerLabel },
            )}
          </p>
        </div>
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.model.title")}
        description={t("settings.assistants.model.description")}
        layout="stacked"
        grouped
      >
        <div className="flex w-full items-center gap-2">
          <ModelSelect
            value={model}
            options={modelOptions}
            onSelect={handleModelSelect}
            onCreate={handleModelCreate}
            onBlur={() => undefined}
            placeholder={t("settings.assistants.model.placeholder")}
            disabled={isModelUpdating}
            isLoading={isFetchingModels}
          />
          <Button
            variant="secondary"
            size="md"
            onClick={handleRefreshModels}
            disabled={isFetchingModels || isModelUpdating}
            aria-label={t("settings.assistants.model.refresh")}
          >
            <RefreshCcw
              className={`h-4 w-4 ${isFetchingModels ? "animate-spin" : ""}`}
              aria-hidden="true"
            />
          </Button>
        </div>
        {!canListModels && (
          <p className="mt-2 text-xs text-mid-gray">
            {t("settings.assistants.model.listingHint")}
          </p>
        )}
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.systemPrompt.title")}
        description={t("settings.assistants.systemPrompt.description")}
        layout="stacked"
        grouped
      >
        <AssistantPromptField
          value={systemPrompt}
          onCommit={handleSystemPromptChange}
          disabled={isSystemPromptUpdating}
          placeholder={t("settings.assistants.systemPrompt.placeholder")}
          ariaLabel={t("settings.assistants.systemPrompt.title")}
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.maxOutputTokens.title")}
        description={t("settings.assistants.maxOutputTokens.description", {
          min: ASSISTANT_LIMITS.maxOutputTokens.min,
          max: ASSISTANT_LIMITS.maxOutputTokens.max,
        })}
        grouped
      >
        <AssistantLimitField
          value={maxOutputTokens}
          range={ASSISTANT_LIMITS.maxOutputTokens}
          step={64}
          disabled={isLimitsUpdating}
          ariaLabel={t("settings.assistants.maxOutputTokens.title")}
          onCommit={handleMaxOutputTokensChange}
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.timeout.title")}
        description={t("settings.assistants.timeout.description", {
          min: ASSISTANT_LIMITS.timeoutSecs.min,
          max: ASSISTANT_LIMITS.timeoutSecs.max,
        })}
        grouped
      >
        <AssistantLimitField
          value={timeoutSecs}
          range={ASSISTANT_LIMITS.timeoutSecs}
          step={5}
          disabled={isLimitsUpdating}
          ariaLabel={t("settings.assistants.timeout.title")}
          onCommit={handleTimeoutSecsChange}
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistants.outputMode.title")}
        description={t(`settings.assistants.outputModes.${outputMode}.hint`)}
        grouped
      >
        <Dropdown
          options={outputModeOptions}
          selectedValue={outputMode}
          onSelect={(value) =>
            handleOutputModeSelect(value as AssistantOutputMode)
          }
          disabled={isOutputModeUpdating}
        />
      </SettingContainer>
    </SettingsGroup>
  );
};
