import React from "react";
import { useTranslation } from "react-i18next";
import { AudioLines, Lock } from "lucide-react";
import { EmptyState, type StateAction } from "@/components/shared";
import { MEETING_MODE_ENABLED } from "@/components/shell/productNav";

export interface MeetingListProps {
  /** Invoked when the user activates the primary `New Meeting` action. */
  onNewMeeting?: () => void;
  /**
   * Importing an audio file is flag-gated off until the importer ships
   * (plan §6.5: "sekundär `Audiodatei importieren`, falls diese Funktion
   * später freigegeben ist").
   */
  importEnabled?: boolean;
  /** Invoked when the user activates the secondary import action. */
  onImportAudio?: () => void;
  /**
   * Overrides the build-time meeting-mode flag. Defaults to
   * `MEETING_MODE_ENABLED` so production stays hidden until the flag is on;
   * tests and future embedders can pass this explicitly.
   */
  flagEnabled?: boolean;
}

/**
 * Meetings page (plan §6.5), first-run empty state.
 *
 * While no meetings exist the page shows exactly:
 * - a clear `Meetings` heading;
 * - one sentence of benefit description;
 * - one large primary action `Neues Meeting`;
 * - a secondary `Audiodatei importieren` action, only when `importEnabled`;
 * - a short privacy status line `Standard: lokal auf diesem Gerät`.
 *
 * No technical configuration and no empty table (§6.5). The non-empty list
 * view (search, filters, meeting cards) ships with a later leaf.
 *
 * The whole component is gated behind the meeting feature flag: with the flag
 * off it renders nothing, leaving the default UX untouched.
 */
export const MeetingList: React.FC<MeetingListProps> = ({
  onNewMeeting,
  importEnabled = false,
  onImportAudio,
  flagEnabled,
}) => {
  const { t } = useTranslation();

  if (!(flagEnabled ?? MEETING_MODE_ENABLED)) {
    return null;
  }

  // The tuple type keeps at least one (primary) action representable-only.
  const actions: [StateAction, ...StateAction[]] = [
    {
      label: { key: "meetings.newMeeting" },
      onClick: onNewMeeting,
      size: "lg",
    },
  ];
  if (importEnabled) {
    actions.push({
      label: { key: "meetings.importAudio" },
      onClick: onImportAudio,
      variant: "secondary",
    });
  }

  return (
    <div className="flex w-full flex-col items-center">
      <EmptyState
        icon={<AudioLines size={48} />}
        title={{ key: "nav.meetings" }}
        description={{ key: "meetings.emptyBenefit" }}
        actions={actions}
      />
      {/* Privacy status line (§6.5): short, honest, non-technical. */}
      <p className="mt-2 flex items-center gap-1.5 text-xs text-text/50">
        <Lock size={12} aria-hidden="true" className="shrink-0" />
        {t("meetings.privacyLocal")}
      </p>
    </div>
  );
};

export default MeetingList;
