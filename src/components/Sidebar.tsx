import React from "react";
import { useTranslation } from "react-i18next";
import SpeakTextLogo from "./icons/SpeakTextLogo";
import {
  PRIMARY_NAV,
  SECONDARY_NAV,
  type ActiveMeetingInfo,
  type ProductViewId,
} from "./shell/productNav";
import { NavItem } from "./shell/NavItem";
import { ActiveMeetingNavItem } from "./shell/ActiveMeetingNavItem";

/**
 * Product navigation sidebar (plan §6.3):
 *
 *   Diktieren / Meetings / Projekte / Diktatverlauf
 *   ─────────────
 *   Einstellungen / Hilfe & Über
 *
 * While a meeting is recording and meeting mode is enabled, a pinned
 * `Aufnahme läuft · mm:ss` entry with a red status dot appears directly below
 * "Meetings". Until then the slot renders nothing.
 */
interface SidebarProps {
  activeView: ProductViewId;
  onViewChange: (view: ProductViewId) => void;
  /** Reported by meeting mode once it exists; always null today. */
  activeMeeting?: ActiveMeetingInfo | null;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeView,
  onViewChange,
  activeMeeting = null,
}) => {
  const { t } = useTranslation();

  const renderEntries = (entries: typeof PRIMARY_NAV) =>
    entries.flatMap((entry) => [
      <NavItem
        key={entry.id}
        label={t(entry.labelKey)}
        icon={entry.icon}
        active={activeView === entry.id}
        onClick={() => onViewChange(entry.id)}
      />,
      // Pinned active-meeting slot directly below "Meetings" (plan §6.3);
      // renders nothing until meeting mode exists.
      ...(entry.id === "meetings"
        ? [
            <ActiveMeetingNavItem
              key="active-meeting"
              meeting={activeMeeting}
              onOpen={() => onViewChange("meetings")}
            />,
          ]
        : []),
    ]);

  return (
    <div className="flex flex-col w-40 h-full border-e border-mid-gray/20 items-center px-2">
      <SpeakTextLogo width={120} className="m-4" />
      <nav
        aria-label={t("nav.primaryAriaLabel")}
        className="flex flex-col w-full items-center gap-1 pt-2 border-t border-mid-gray/20"
      >
        {renderEntries(PRIMARY_NAV)}
        <hr className="w-full border-mid-gray/20 my-1" aria-hidden="true" />
        {renderEntries(SECONDARY_NAV)}
      </nav>
    </div>
  );
};
