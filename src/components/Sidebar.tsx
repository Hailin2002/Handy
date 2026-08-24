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
import {
  isMeetingCapturing,
  useMeetingElapsedSeconds,
  useMeetingStore,
} from "@/stores/meetingStore";

/**
 * Product navigation sidebar (plan §6.3):
 *
 *   Diktieren / Meetings / Projekte / Diktatverlauf
 *   ─────────────
 *   Einstellungen / Hilfe & Über
 *
 * While a meeting is recording and meeting mode is enabled, a pinned
 * `Aufnahme läuft · hh:mm:ss` entry with a red status dot appears directly
 * below "Meetings". The entry is derived from `meetingStore` - the same
 * backend state machine the recorder view reads - so the recording status is
 * consistent across every surface (plan 6.2 rule 9).
 */
interface SidebarProps {
  activeView: ProductViewId;
  onViewChange: (view: ProductViewId) => void;
  /**
   * Optional override for the pinned active-meeting entry. Left undefined in
   * the app, where the value is derived from `meetingStore` below; render
   * tests pass it explicitly to pin either branch.
   */
  activeMeeting?: ActiveMeetingInfo | null;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeView,
  onViewChange,
  activeMeeting,
}) => {
  const { t } = useTranslation();
  // Single source of truth for "is a meeting running" (plan line 926: main
  // window, overlay and tray all read the same MeetingManager state). The
  // clock ticks in a local hook, never in the store (FSC B5).
  const session = useMeetingStore((state) => state.session);
  const elapsedSeconds = useMeetingElapsedSeconds();
  const derivedActiveMeeting: ActiveMeetingInfo | null = isMeetingCapturing(
    session?.state,
  )
    ? { elapsedSeconds }
    : null;
  const meeting = activeMeeting !== undefined ? activeMeeting : derivedActiveMeeting;

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
      // renders nothing while no meeting is capturing.
      ...(entry.id === "meetings"
        ? [
            <ActiveMeetingNavItem
              key="active-meeting"
              meeting={meeting}
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
