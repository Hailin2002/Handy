import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { AppLanguageSelector } from "../about/AppLanguageSelector";
import { ThemeSelector } from "../about/ThemeSelector";
import { AutostartToggle } from "../advanced/AutostartToggle";
import { ShowOverlay } from "../advanced/ShowOverlay";
import { ShowTrayIcon } from "../advanced/ShowTrayIcon";
import { StartHidden } from "../advanced/StartHidden";

/**
 * "Allgemein" tab on the Einstellungen page (plan §6.3): app-wide
 * preferences that are not specific to one of the two tools (language,
 * theme, window/tray behavior). The controls were moved here from the old
 * Advanced and About sections; they keep their existing save behavior.
 */
export const AppGeneralTab: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.general.title")}>
        <AppLanguageSelector descriptionMode="tooltip" grouped={true} />
        <ThemeSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.advanced.groups.app")}>
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <ShowTrayIcon descriptionMode="tooltip" grouped={true} />
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </div>
  );
};
