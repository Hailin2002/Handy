import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { navItemClassName } from "./NavItem";
import {
  SECTIONS_CONFIG,
  SETTINGS_TAB_ORDER,
  type SettingsSectionId,
} from "./settingsSections";

/**
 * Einstellungen page (plan §6.3): a dedicated settings view with secondary
 * tabs, replacing the former per-section primary navigation. Debug remains
 * visible only while debug mode is enabled, as before.
 */
export const SettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [selectedTab, setSelectedTab] = useState<SettingsSectionId>("general");

  const availableTabs = SETTINGS_TAB_ORDER.filter((id) =>
    SECTIONS_CONFIG[id].enabled(settings),
  );
  const activeTab = availableTabs.includes(selectedTab)
    ? selectedTab
    : availableTabs[0];

  const ActiveComponent = SECTIONS_CONFIG[activeTab].component;

  return (
    <div className="w-full flex flex-col items-center gap-4">
      <div
        role="group"
        aria-label={t("nav.settingsTabsAriaLabel")}
        className="flex flex-wrap justify-center gap-1 w-full border-b border-mid-gray/20 pb-2"
      >
        {availableTabs.map((id) => {
          const config = SECTIONS_CONFIG[id];
          const Icon = config.icon;
          const isActive = id === activeTab;
          const label = t(config.labelKey);
          return (
            <button
              key={id}
              type="button"
              onClick={() => setSelectedTab(id)}
              aria-pressed={isActive}
              title={label}
              className={navItemClassName(isActive)}
            >
              <Icon width={20} height={20} className="shrink-0" />
              <span className="text-sm font-medium truncate">{label}</span>
            </button>
          );
        })}
      </div>
      <div className="w-full">
        <ActiveComponent />
      </div>
    </div>
  );
};
