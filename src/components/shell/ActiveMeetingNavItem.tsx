import React from "react";
import { useTranslation } from "react-i18next";
import {
  MEETING_MODE_ENABLED,
  formatMeetingDuration,
  type ActiveMeetingInfo,
} from "./productNav";

interface ActiveMeetingNavItemProps {
  /** Currently recording meeting, or null while meeting mode is absent. */
  meeting: ActiveMeetingInfo | null;
  onOpen?: () => void;
}

/**
 * Pinned `Aufnahme läuft · mm:ss` entry rendered directly below the
 * "Meetings" navigation item (plan §6.3).
 *
 * Rendering is double-gated:
 *  1. the MEETING_MODE_ENABLED feature flag must be on (it defaults to off
 *     until meeting mode exists), and
 *  2. an active meeting must be reported — which no caller can do yet.
 */
export const ActiveMeetingNavItem: React.FC<ActiveMeetingNavItemProps> = ({
  meeting,
  onOpen,
}) => {
  const { t } = useTranslation();

  if (!MEETING_MODE_ENABLED || !meeting) return null;

  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={t("nav.activeMeeting", {
        duration: formatMeetingDuration(meeting.elapsedSeconds),
      })}
      title={t("nav.activeMeeting", {
        duration: formatMeetingDuration(meeting.elapsedSeconds),
      })}
      className="flex items-center gap-2 ps-6 pe-2 w-full min-h-[36px] rounded-lg cursor-pointer bg-red-500/10 text-mid-gray transition-colors outline-none focus-visible:ring-2 focus-visible:ring-logo-primary hover:bg-red-500/20"
    >
      <span
        aria-hidden="true"
        className="w-2 h-2 rounded-full bg-red-500 shrink-0"
      />
      <span className="text-xs font-medium truncate tabular-nums">
        {t("nav.activeMeeting", {
          duration: formatMeetingDuration(meeting.elapsedSeconds),
        })}
      </span>
    </button>
  );
};
