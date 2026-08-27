import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { navItemClassName } from "./navItemClassName";
import {
  SECTIONS_CONFIG,
  SETTINGS_TAB_ORDER,
  type SettingsSectionId,
} from "./settingsSections";
import { buildSettingsHash, settingsSectionFromHash } from "./settingsDeepLink";

/**
 * Einstellungen page (plan §6.3): a dedicated settings view with secondary
 * tabs "Diktieren | Meetings | Modelle & Provider | Allgemein | Erweitert",
 * replacing the former per-section primary navigation. Debug remains visible
 * only while debug mode is enabled, as before.
 *
 * Accessibility: the tab strip is an ARIA tablist with automatic activation
 * and a roving tabindex; Left/Right (direction-aware for RTL), Home and End
 * move both focus and selection. The active tab is mirrored into the URL
 * hash (`#/settings/<tabId>`) so links — including hashes built from the old
 * sidebar section ids — open the mapped tab directly.
 */
export const SettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [selectedTab, setSelectedTab] = useState<SettingsSectionId>(
    () => settingsSectionFromHash(window.location.hash) ?? "dictate",
  );
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const availableTabs = SETTINGS_TAB_ORDER.filter((id) =>
    SECTIONS_CONFIG[id].enabled(settings),
  );
  const activeTab = availableTabs.includes(selectedTab)
    ? selectedTab
    : availableTabs[0];
  const activeIndex = availableTabs.indexOf(activeTab);

  // Reflect the active tab in the URL hash so the view stays deep-linkable.
  // replaceState (unlike location.hash assignment) neither scrolls nor adds
  // history entries nor fires hashchange.
  useEffect(() => {
    const expected = buildSettingsHash(activeTab);
    if (window.location.hash !== expected) {
      window.history.replaceState(null, "", expected);
    }
  }, [activeTab]);

  // Follow external hash changes (back/forward navigation, manually edited
  // or programmatically set deep links).
  useEffect(() => {
    const onHashChange = () => {
      const section = settingsSectionFromHash(window.location.hash);
      if (section) setSelectedTab(section);
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const handleTablistKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const count = availableTabs.length;
    let nextIndex: number;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowLeft": {
        // In RTL layouts ArrowRight visually means "previous".
        const rtl =
          event.currentTarget.closest("[dir]")?.getAttribute("dir") === "rtl";
        const step = (event.key === "ArrowRight" ? 1 : -1) * (rtl ? -1 : 1);
        nextIndex = (activeIndex + step + count) % count;
        break;
      }
      case "Home":
        nextIndex = 0;
        break;
      case "End":
        nextIndex = count - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    setSelectedTab(availableTabs[nextIndex]);
    // Automatic activation: selection follows focus.
    tabRefs.current[nextIndex]?.focus();
  };

  const ActiveComponent = SECTIONS_CONFIG[activeTab].component;

  return (
    <div className="w-full flex flex-col items-center gap-4">
      <div
        role="tablist"
        aria-label={t("nav.settingsTabsAriaLabel")}
        aria-orientation="horizontal"
        className="flex flex-wrap justify-center gap-1 w-full border-b border-mid-gray/20 pb-2"
        onKeyDown={handleTablistKeyDown}
      >
        {availableTabs.map((id, index) => {
          const config = SECTIONS_CONFIG[id];
          const Icon = config.icon;
          const isActive = id === activeTab;
          const label = t(config.labelKey);
          return (
            <button
              key={id}
              ref={(node) => {
                tabRefs.current[index] = node;
              }}
              type="button"
              role="tab"
              id={`settings-tab-${id}`}
              aria-selected={isActive}
              aria-controls={`settings-panel-${id}`}
              tabIndex={isActive ? 0 : -1}
              onClick={() => setSelectedTab(id)}
              title={label}
              className={navItemClassName(isActive)}
            >
              <Icon width={20} height={20} className="shrink-0" />
              <span className="text-sm font-medium truncate">{label}</span>
            </button>
          );
        })}
      </div>
      <div
        role="tabpanel"
        id={`settings-panel-${activeTab}`}
        aria-labelledby={`settings-tab-${activeTab}`}
        className="w-full"
      >
        <ActiveComponent />
      </div>
    </div>
  );
};
