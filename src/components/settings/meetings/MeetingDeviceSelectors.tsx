import React from "react";
import { useTranslation } from "react-i18next";
import type { AudioDevice } from "@/bindings";
import { Dropdown } from "../../ui/Dropdown";
import { ResetButton } from "../../ui/ResetButton";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";

/** Sentinel shown when no explicit device is pinned. */
const FOLLOW_SYSTEM = "";

/**
 * Meeting capture devices (plan §6.6 step 2, start validation).
 *
 * Meetings pin their own microphone and their own loopback source. In
 * particular the loopback source is *not* `selected_output_device`: that
 * setting only decides where feedback beeps are played, and the plan requires
 * it to stay untouched by meeting mode ("`selected_output_device` für
 * Feedback-Sounds bleibt unangetastet. Der Meeting-Modus erhält eigene
 * Einstellungen für Mikrofon und Loopback-Output"). Sharing one setting meant
 * changing where a beep plays silently changed what a meeting recorded.
 *
 * Both selectors accept "follow the system default", which is the honest
 * default for people who never touch audio settings.
 */
export const MeetingDeviceSelectors: React.FC = () => {
  const { t } = useTranslation();
  const {
    getSetting,
    updateSetting,
    resetSetting,
    isUpdating,
    isLoading,
    audioDevices,
    outputDevices,
    refreshAudioDevices,
    refreshOutputDevices,
  } = useSettings();

  const followLabel = t("settings.meetings.devices.followSystem");

  const toOptions = (devices: AudioDevice[]) => [
    { value: FOLLOW_SYSTEM, label: followLabel },
    ...devices.map((device) => ({ value: device.name, label: device.name })),
  ];

  return (
    <>
      <SettingContainer
        title={t("settings.meetings.devices.microphone.title")}
        description={t("settings.meetings.devices.microphone.description")}
        descriptionMode="inline"
        grouped
      >
        <div className="flex items-center space-x-1">
          <Dropdown
            options={toOptions(audioDevices)}
            selectedValue={getSetting("meeting_microphone") ?? FOLLOW_SYSTEM}
            onSelect={(value) =>
              void updateSetting(
                "meeting_microphone",
                value === FOLLOW_SYSTEM ? null : value,
              )
            }
            placeholder={followLabel}
            disabled={isUpdating("meeting_microphone") || isLoading}
            onRefresh={refreshAudioDevices}
          />
          <ResetButton
            onClick={() => void resetSetting("meeting_microphone")}
            disabled={isUpdating("meeting_microphone") || isLoading}
          />
        </div>
      </SettingContainer>

      <SettingContainer
        title={t("settings.meetings.devices.systemAudio.title")}
        description={t("settings.meetings.devices.systemAudio.description")}
        descriptionMode="inline"
        grouped
      >
        <div className="flex items-center space-x-1">
          <Dropdown
            options={toOptions(outputDevices)}
            selectedValue={getSetting("meeting_output_device") ?? FOLLOW_SYSTEM}
            onSelect={(value) =>
              void updateSetting(
                "meeting_output_device",
                value === FOLLOW_SYSTEM ? null : value,
              )
            }
            placeholder={followLabel}
            disabled={isUpdating("meeting_output_device") || isLoading}
            onRefresh={refreshOutputDevices}
          />
          <ResetButton
            onClick={() => void resetSetting("meeting_output_device")}
            disabled={isUpdating("meeting_output_device") || isLoading}
          />
        </div>
      </SettingContainer>
    </>
  );
};

export default MeetingDeviceSelectors;
