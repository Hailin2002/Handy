import React, { useMemo, useState } from "react";
import { RefreshCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { Select } from "@/components/ui/Select";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";

const LOCAL_PROVIDER = "local_ollama";
const OPENROUTER_PROVIDER = "openrouter";

export const MeetingSummarySettings: React.FC = () => {
  const { t } = useTranslation();
  const {
    settings,
    refreshSettings,
    updatePostProcessApiKey,
    fetchPostProcessModels,
    postProcessModelOptions,
    isUpdating,
  } = useSettings();
  const [pendingUpdate, setPendingUpdate] = useState<
    "provider" | "model" | "consent" | null
  >(null);

  const providerId = settings?.meeting_summary_provider_id ?? LOCAL_PROVIDER;
  const model = settings?.meeting_summary_models?.[providerId] ?? "";
  const cloudConsent = settings?.meeting_summary_cloud_consent ?? false;
  const apiKey = settings?.post_process_api_keys?.[OPENROUTER_PROVIDER] ?? "";
  const models = useMemo(
    () => postProcessModelOptions[providerId] ?? [],
    [postProcessModelOptions, providerId],
  );
  const modelOptions = useMemo(() => {
    const values = new Set(models);
    if (model.trim()) values.add(model.trim());
    return Array.from(values).map((value) => ({ value, label: value }));
  }, [model, models]);

  const setProvider = async (nextProvider: string | null) => {
    if (!nextProvider || nextProvider === providerId) return;
    setPendingUpdate("provider");
    try {
      const result = await commands.setMeetingSummaryProvider(nextProvider);
      if (result.status === "error") throw new Error(result.error);
      await refreshSettings();
      void fetchPostProcessModels(nextProvider);
    } catch (error) {
      console.error("Failed to update meeting summary provider:", error);
      toast.error(String(error));
    } finally {
      setPendingUpdate(null);
    }
  };

  const setModel = async (nextModel: string) => {
    const trimmed = nextModel.trim();
    if (!trimmed || trimmed === model) return;
    setPendingUpdate("model");
    try {
      const result = await commands.changeMeetingSummaryModelSetting(
        providerId,
        trimmed,
      );
      if (result.status === "error") throw new Error(result.error);
      await refreshSettings();
    } catch (error) {
      console.error("Failed to update meeting summary model:", error);
      toast.error(String(error));
    } finally {
      setPendingUpdate(null);
    }
  };

  const setCloudConsent = async (enabled: boolean) => {
    if (enabled === cloudConsent) return;
    setPendingUpdate("consent");
    try {
      const result =
        await commands.changeMeetingSummaryCloudConsentSetting(enabled);
      if (result.status === "error") throw new Error(result.error);
      await refreshSettings();
    } catch (error) {
      console.error("Failed to update meeting summary consent:", error);
      toast.error(String(error));
    } finally {
      setPendingUpdate(null);
    }
  };

  return (
    <SettingsGroup
      title={t("settings.meetings.summary.groupTitle")}
      description={t("settings.meetings.summary.groupDescription")}
    >
      <SettingContainer
        title={t("settings.meetings.summary.provider.title")}
        description={t("settings.meetings.summary.provider.description")}
        grouped
      >
        <Select
          value={providerId}
          options={[
            {
              value: LOCAL_PROVIDER,
              label: t("settings.meetings.summary.provider.local"),
            },
            {
              value: OPENROUTER_PROVIDER,
              label: t("settings.meetings.summary.provider.openrouter"),
            },
          ]}
          onChange={(value) => void setProvider(value)}
          isClearable={false}
          disabled={pendingUpdate !== null}
          className="min-w-[280px]"
        />
      </SettingContainer>

      {providerId === LOCAL_PROVIDER && (
        <Alert variant="info" contained>
          {t("settings.meetings.summary.localHint")}
        </Alert>
      )}

      {providerId === OPENROUTER_PROVIDER && (
        <>
          <SettingContainer
            title={t("settings.meetings.summary.apiKey.title")}
            description={t("settings.meetings.summary.apiKey.description")}
            grouped
          >
            <ApiKeyField
              value={apiKey}
              onBlur={(value) =>
                void updatePostProcessApiKey(OPENROUTER_PROVIDER, value)
              }
              placeholder={t("settings.meetings.summary.apiKey.placeholder")}
              disabled={isUpdating(
                `post_process_api_key:${OPENROUTER_PROVIDER}`,
              )}
              className="min-w-[320px]"
            />
          </SettingContainer>
          <ToggleSwitch
            checked={cloudConsent}
            onChange={(enabled) => void setCloudConsent(enabled)}
            isUpdating={pendingUpdate === "consent"}
            label={t("settings.meetings.summary.cloudConsent.title")}
            description={t(
              "settings.meetings.summary.cloudConsent.description",
            )}
            grouped
          />
        </>
      )}

      <SettingContainer
        title={t("settings.meetings.summary.model.title")}
        description={t("settings.meetings.summary.model.description")}
        layout="stacked"
        grouped
      >
        <div className="flex w-full items-center gap-2">
          <Select
            value={model || null}
            options={modelOptions}
            onChange={(value) => value && void setModel(value)}
            onCreateOption={(value) => void setModel(value)}
            isCreatable
            isClearable={false}
            isLoading={isUpdating(`post_process_models_fetch:${providerId}`)}
            disabled={pendingUpdate !== null}
            placeholder={t("settings.meetings.summary.model.placeholder")}
            formatCreateLabel={(value) =>
              t("settings.meetings.summary.model.use", { model: value })
            }
            className="min-w-[360px] flex-1"
          />
          <Button
            variant="secondary"
            size="md"
            onClick={() => void fetchPostProcessModels(providerId)}
            disabled={
              pendingUpdate !== null ||
              isUpdating(`post_process_models_fetch:${providerId}`)
            }
            aria-label={t("settings.meetings.summary.model.refresh")}
          >
            <RefreshCcw
              className={`h-4 w-4 ${isUpdating(`post_process_models_fetch:${providerId}`) ? "animate-spin" : ""}`}
              aria-hidden="true"
            />
          </Button>
        </div>
      </SettingContainer>
    </SettingsGroup>
  );
};
