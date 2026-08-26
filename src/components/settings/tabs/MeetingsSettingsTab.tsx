import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MeetingDeviceSelectors } from "../meetings/MeetingDeviceSelectors";
import { MeetingShortcutEditor } from "../meetings/MeetingShortcutEditor";

/**
 * "Meetings" tab on the Einstellungen page (plan §6.3): the meeting shortcuts
 * plus the capture devices meetings own independently of dictation.
 */
export const MeetingsSettingsTab: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-2 max-w-2xl mx-auto py-4">
      <h2 className="text-lg font-semibold px-4">{t("nav.meetings")}</h2>
      <MeetingShortcutEditor />
      <SettingsGroup title={t("settings.meetings.devices.groupTitle")}>
        <MeetingDeviceSelectors />
      </SettingsGroup>
      <p className="text-sm text-mid-gray px-4">
        {t("settings.tabs.meetingsEmptyBody")}
      </p>
    </div>
  );
};
