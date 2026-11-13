import React from "react";
import { useTranslation } from "react-i18next";
import type { SpeechInputProvider } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Select } from "../../ui/Select";
import { SettingContainer } from "../../ui/SettingContainer";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MeetingDeviceSelectors } from "../meetings/MeetingDeviceSelectors";
import { MeetingShortcutEditor } from "../meetings/MeetingShortcutEditor";
import { MeetingSummarySettings } from "../meetings/MeetingSummarySettings";

/**
 * "Meetings" tab on the Einstellungen page (plan §6.3): the meeting shortcuts
 * plus the capture devices meetings own independently of dictation.
 */
export const MeetingsSettingsTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, isLoading } = useSettings();
  const provider = getSetting("meeting_stt_provider") ?? "local";
  const providerOptions = [
    { value: "local", label: t("meeting.setup.providerLocal") },
    { value: "azure", label: t("meeting.setup.providerAzure") },
    { value: "custom", label: t("meeting.setup.providerCustom") },
  ];

  return (
    <div className="flex flex-col gap-2 max-w-2xl mx-auto py-4">
      <h2 className="text-lg font-semibold px-4">{t("nav.meetings")}</h2>
      <SettingsGroup title={t("meeting.setup.providerLabel")}>
        <SettingContainer
          title={t("meeting.preflight.row.transcription")}
          description={t("meeting.setup.providerHint")}
          descriptionMode="inline"
          grouped
        >
          <Select
            value={provider}
            options={providerOptions}
            onChange={(value) =>
              value &&
              updateSetting(
                "meeting_stt_provider",
                value as SpeechInputProvider,
              )
            }
            disabled={isUpdating("meeting_stt_provider") || isLoading}
            isClearable={false}
            className="min-w-[240px]"
          />
        </SettingContainer>
      </SettingsGroup>
      <SettingsGroup title={t("settings.meetings.devices.groupTitle")}>
        <MeetingDeviceSelectors />
      </SettingsGroup>
      <MeetingSummarySettings />
      <MeetingShortcutEditor />
    </div>
  );
};
