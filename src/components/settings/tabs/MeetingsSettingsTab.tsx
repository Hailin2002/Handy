import React from "react";
import { useTranslation } from "react-i18next";
import { MeetingShortcutEditor } from "../meetings/MeetingShortcutEditor";

/**
 * "Meetings" tab on the Einstellungen page (plan §6.3). The meeting
 * lifecycle itself does not exist yet, so the tab currently carries only
 * the §6.9 meeting shortcut editor; the placeholder note explains when
 * the remaining meeting settings will appear.
 */
export const MeetingsSettingsTab: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-2 max-w-2xl mx-auto py-4">
      <h2 className="text-lg font-semibold px-4">{t("nav.meetings")}</h2>
      <MeetingShortcutEditor />
      <p className="text-sm text-mid-gray px-4">
        {t("settings.tabs.meetingsEmptyBody")}
      </p>
    </div>
  );
};
