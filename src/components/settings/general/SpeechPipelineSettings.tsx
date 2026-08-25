import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type SpeechInputProvider } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { Button } from "@/components/ui/Button";
import { Alert } from "@/components/ui/Alert";
import { Slider } from "@/components/ui/Slider";
import { ChevronDown } from "lucide-react";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";

type AzureValidationState =
  | { status: "idle" | "saving" }
  | { status: "success" | "error"; message: string };

interface DeferredInputProps {
  value: string;
  placeholder: string;
  disabled: boolean;
  onBlur: (value: string) => void;
}

const DeferredInput: React.FC<DeferredInputProps> = ({
  value,
  placeholder,
  disabled,
  onBlur,
}) => {
  const [localValue, setLocalValue] = useState(value);

  useEffect(() => setLocalValue(value), [value]);

  return (
    <Input
      value={localValue}
      onChange={(event) => setLocalValue(event.target.value)}
      onBlur={() => onBlur(localValue)}
      placeholder={placeholder}
      disabled={disabled}
      className="min-w-[360px]"
    />
  );
};

export const SpeechPipelineSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting, isUpdating, refreshSettings } =
    useSettings();

  const provider = getSetting("speech_input_provider") ?? "local";
  const deeplEnabled = getSetting("deepl_translation_enabled") ?? false;
  const apiKeys = settings?.speech_pipeline_api_keys ?? {};
  const [azureEndpointDraft, setAzureEndpointDraft] = useState("");
  const [azureLanguageDraft, setAzureLanguageDraft] = useState("de-DE");
  const [azureApiKeyDraft, setAzureApiKeyDraft] = useState("");
  const [azureValidation, setAzureValidation] = useState<AzureValidationState>({
    status: "idle",
  });
  const [advancedOpen, setAdvancedOpen] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setAzureEndpointDraft(settings.azure_speech_endpoint ?? "");
    setAzureLanguageDraft(settings.azure_speech_language ?? "de-DE");
    setAzureApiKeyDraft(settings.speech_pipeline_api_keys?.azure ?? "");
  }, [settings]);

  const providerOptions = [
    { value: "local", label: t("settings.speechPipeline.provider.local") },
    { value: "azure", label: t("settings.speechPipeline.provider.azure") },
    { value: "custom", label: t("settings.speechPipeline.provider.custom") },
  ];

  const targetLanguageOptions = [
    { value: "EN", label: t("settings.speechPipeline.deepl.targetEnglish") },
    {
      value: "EN-GB",
      label: t("settings.speechPipeline.deepl.targetEnglishBritish"),
    },
    {
      value: "EN-US",
      label: t("settings.speechPipeline.deepl.targetEnglishAmerican"),
    },
    { value: "DE", label: t("settings.speechPipeline.deepl.targetGerman") },
  ];

  const updateApiKey = async (providerId: string, apiKey: string) => {
    await commands.changeSpeechPipelineApiKeySetting(providerId, apiKey);
    await refreshSettings();
  };

  const saveAndTestAzure = async () => {
    setAzureValidation({ status: "saving" });
    try {
      const endpointResult = await commands.changeAzureSpeechEndpointSetting(
        azureEndpointDraft.trim(),
      );
      if (endpointResult.status === "error") {
        throw new Error(endpointResult.error);
      }
      const languageResult = await commands.changeAzureSpeechLanguageSetting(
        azureLanguageDraft.trim(),
      );
      if (languageResult.status === "error") {
        throw new Error(languageResult.error);
      }
      const keyResult = await commands.changeSpeechPipelineApiKeySetting(
        "azure",
        azureApiKeyDraft.trim(),
      );
      if (keyResult.status === "error") {
        throw new Error(keyResult.error);
      }

      const testResult = await commands.testAzureSpeechConnection();
      if (testResult.status === "error") {
        throw new Error(testResult.error);
      }
      await refreshSettings();
      setAzureValidation({
        status: "success",
        message: t("settings.speechPipeline.azureValidation.success"),
      });
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      setAzureValidation({
        status: "error",
        message: t("settings.speechPipeline.azureValidation.failed", {
          error: detail,
        }),
      });
    }
  };

  const resetAzureValidation = () => setAzureValidation({ status: "idle" });

  const providerLabel =
    providerOptions.find((option) => option.value === provider)?.label ??
    provider;

  return (
    <div className="max-w-3xl w-full mx-auto">
      <SettingsGroup
        title={t("settings.speechPipeline.title")}
        description={t("settings.speechPipeline.description")}
      >
        <div className="px-4 py-3">
          <div className="flex flex-wrap items-center gap-2 text-xs font-medium">
            <span className="rounded-full bg-logo-primary/15 px-3 py-1.5 text-logo-primary">
              {providerLabel}
            </span>
            <span className="text-mid-gray" aria-hidden="true">
              →
            </span>
            <span className="rounded-full bg-mid-gray/10 px-3 py-1.5">
              {t("settings.speechPipeline.preview.terminology")}
            </span>
            {deeplEnabled && (
              <>
                <span className="text-mid-gray" aria-hidden="true">
                  →
                </span>
                <span className="rounded-full bg-mid-gray/10 px-3 py-1.5">
                  {t("settings.speechPipeline.preview.translation")}
                </span>
              </>
            )}
            <span className="text-mid-gray" aria-hidden="true">
              →
            </span>
            <span className="rounded-full bg-mid-gray/10 px-3 py-1.5">
              {t("settings.speechPipeline.preview.paste")}
            </span>
          </div>
        </div>

        <SettingContainer
          title={t("settings.speechPipeline.provider.title")}
          description={t("settings.speechPipeline.provider.description")}
          grouped
        >
          <Select
            value={provider}
            options={providerOptions}
            onChange={(value) =>
              value &&
              updateSetting(
                "speech_input_provider",
                value as SpeechInputProvider,
              )
            }
            disabled={isUpdating("speech_input_provider")}
            isClearable={false}
            className="min-w-[240px]"
          />
        </SettingContainer>

        {provider === "azure" && (
          <>
            <SettingContainer
              title={t("settings.speechPipeline.azure.endpoint.title")}
              description={t(
                "settings.speechPipeline.azure.endpoint.description",
              )}
              grouped
            >
              <Input
                value={azureEndpointDraft}
                onChange={(event) => {
                  setAzureEndpointDraft(event.target.value);
                  resetAzureValidation();
                }}
                placeholder={t(
                  "settings.speechPipeline.azure.endpoint.placeholder",
                )}
                disabled={azureValidation.status === "saving"}
                className="min-w-[360px]"
              />
            </SettingContainer>
            <SettingContainer
              title={t("settings.speechPipeline.azure.language.title")}
              description={t(
                "settings.speechPipeline.azure.language.description",
              )}
              grouped
            >
              <Input
                value={azureLanguageDraft}
                onChange={(event) => {
                  setAzureLanguageDraft(event.target.value);
                  resetAzureValidation();
                }}
                placeholder={t(
                  "settings.speechPipeline.azure.language.placeholder",
                )}
                disabled={azureValidation.status === "saving"}
                className="min-w-[360px]"
              />
            </SettingContainer>
            <SettingContainer
              title={t("settings.speechPipeline.azure.apiKey.title")}
              description={t(
                "settings.speechPipeline.azure.apiKey.description",
              )}
              grouped
            >
              <Input
                type="password"
                value={azureApiKeyDraft}
                onChange={(event) => {
                  setAzureApiKeyDraft(event.target.value);
                  resetAzureValidation();
                }}
                placeholder={t(
                  "settings.speechPipeline.azure.apiKey.placeholder",
                )}
                variant="compact"
                disabled={azureValidation.status === "saving"}
                className="min-w-[360px]"
              />
            </SettingContainer>
            <SettingContainer
              title={t("settings.speechPipeline.azureValidation.title")}
              description={t(
                "settings.speechPipeline.azureValidation.description",
              )}
              grouped
              layout="stacked"
            >
              <div className="space-y-3">
                <Button
                  onClick={saveAndTestAzure}
                  disabled={
                    azureValidation.status === "saving" ||
                    !azureEndpointDraft.trim() ||
                    !azureLanguageDraft.trim() ||
                    !azureApiKeyDraft.trim()
                  }
                >
                  {azureValidation.status === "saving"
                    ? t("settings.speechPipeline.azureValidation.testing")
                    : t("settings.speechPipeline.azureValidation.button")}
                </Button>
                {azureValidation.status === "success" && (
                  <Alert variant="success" contained>
                    {azureValidation.message}
                  </Alert>
                )}
                {azureValidation.status === "error" && (
                  <Alert variant="error" contained>
                    {azureValidation.message}
                  </Alert>
                )}
              </div>
            </SettingContainer>
          </>
        )}

        {provider === "custom" && (
          <>
            <SettingContainer
              title={t("settings.speechPipeline.custom.endpoint.title")}
              description={t(
                "settings.speechPipeline.custom.endpoint.description",
              )}
              grouped
            >
              <DeferredInput
                value={getSetting("custom_stt_endpoint") ?? ""}
                onBlur={(value) => updateSetting("custom_stt_endpoint", value)}
                placeholder={t(
                  "settings.speechPipeline.custom.endpoint.placeholder",
                )}
                disabled={isUpdating("custom_stt_endpoint")}
              />
            </SettingContainer>
            <SettingContainer
              title={t("settings.speechPipeline.custom.token.title")}
              description={t(
                "settings.speechPipeline.custom.token.description",
              )}
              grouped
            >
              <ApiKeyField
                value={apiKeys.custom ?? ""}
                onBlur={(value) => updateApiKey("custom", value)}
                placeholder={t(
                  "settings.speechPipeline.custom.token.placeholder",
                )}
                disabled={false}
              />
            </SettingContainer>
          </>
        )}

        <ToggleSwitch
          checked={deeplEnabled}
          onChange={(enabled) =>
            updateSetting("deepl_translation_enabled", enabled)
          }
          isUpdating={isUpdating("deepl_translation_enabled")}
          label={t("settings.speechPipeline.deepl.enabled.title")}
          description={t("settings.speechPipeline.deepl.enabled.description")}
          grouped
        />

        {deeplEnabled && (
          <>
            <SettingContainer
              title={t("settings.speechPipeline.deepl.apiKey.title")}
              description={t(
                "settings.speechPipeline.deepl.apiKey.description",
              )}
              grouped
            >
              <ApiKeyField
                value={apiKeys.deepl ?? ""}
                onBlur={(value) => updateApiKey("deepl", value)}
                placeholder={t(
                  "settings.speechPipeline.deepl.apiKey.placeholder",
                )}
                disabled={false}
              />
            </SettingContainer>
            <SettingContainer
              title={t("settings.speechPipeline.deepl.target.title")}
              description={t(
                "settings.speechPipeline.deepl.target.description",
              )}
              grouped
            >
              <Select
                value={getSetting("deepl_target_language") ?? "EN"}
                options={targetLanguageOptions}
                onChange={(value) =>
                  value && updateSetting("deepl_target_language", value)
                }
                disabled={isUpdating("deepl_target_language")}
                isClearable={false}
                className="min-w-[240px]"
              />
            </SettingContainer>
          </>
        )}

        {(provider === "azure" || deeplEnabled) && (
          <>
            <button
              type="button"
              className="flex w-full items-center justify-between px-4 py-3 text-left text-sm font-medium hover:bg-mid-gray/5"
              onClick={() => setAdvancedOpen((open) => !open)}
              aria-expanded={advancedOpen}
            >
              <span>
                {t("settings.speechPipeline.advanced.title")}
                <span className="ml-2 text-xs font-normal text-mid-gray">
                  {t("settings.speechPipeline.advanced.description")}
                </span>
              </span>
              <ChevronDown
                className={`h-4 w-4 transition-transform ${advancedOpen ? "rotate-180" : ""}`}
              />
            </button>

            {advancedOpen && (
              <>
                {provider === "azure" && (
                  <>
                    <Slider
                      value={getSetting("azure_stable_partial_threshold") ?? 3}
                      onChange={(value) =>
                        updateSetting(
                          "azure_stable_partial_threshold",
                          Math.round(value),
                        )
                      }
                      min={1}
                      max={5}
                      step={1}
                      label={t(
                        "settings.speechPipeline.advanced.stability.title",
                      )}
                      description={t(
                        "settings.speechPipeline.advanced.stability.description",
                      )}
                      grouped
                      formatValue={(value) => String(Math.round(value))}
                    />
                    <Slider
                      value={
                        getSetting("azure_segmentation_silence_ms") ?? 1200
                      }
                      onChange={(value) =>
                        updateSetting(
                          "azure_segmentation_silence_ms",
                          Math.round(value),
                        )
                      }
                      min={300}
                      max={5000}
                      step={100}
                      label={t(
                        "settings.speechPipeline.advanced.phraseSilence.title",
                      )}
                      description={t(
                        "settings.speechPipeline.advanced.phraseSilence.description",
                      )}
                      grouped
                      formatValue={(value) => `${(value / 1000).toFixed(1)} s`}
                    />
                  </>
                )}
                {deeplEnabled && (
                  <Slider
                    value={getSetting("deepl_sentence_delay_ms") ?? 2000}
                    onChange={(value) =>
                      updateSetting(
                        "deepl_sentence_delay_ms",
                        Math.round(value),
                      )
                    }
                    min={0}
                    max={10000}
                    step={250}
                    label={t(
                      "settings.speechPipeline.advanced.translationDelay.title",
                    )}
                    description={t(
                      "settings.speechPipeline.advanced.translationDelay.description",
                    )}
                    grouped
                    formatValue={(value) => `${(value / 1000).toFixed(2)} s`}
                  />
                )}
              </>
            )}
          </>
        )}
      </SettingsGroup>
    </div>
  );
};
