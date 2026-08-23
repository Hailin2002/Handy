import React from "react";
import { useTranslation } from "react-i18next";

/**
 * "Meetings" tab on the Einstellungen page (plan §6.3). Meeting mode does
 * not exist yet, so this tab is an honest placeholder: it explains when
 * meeting settings will appear instead of showing dead controls.
 */
export const MeetingsSettingsTab: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col items-center justify-center text-center gap-2 py-16 max-w-md mx-auto">
      <h2 className="text-lg font-semibold">{t("nav.meetings")}</h2>
      <p className="text-sm text-mid-gray">
        {t("settings.tabs.meetingsEmptyBody")}
      </p>
    </div>
  );
};
