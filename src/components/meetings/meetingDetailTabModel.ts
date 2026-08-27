// The meeting-detail tab model: which tabs exist, how they are labelled, and
// how the wizard steps between them.
//
// Split out of MeetingDetailTabs.tsx so that file exports only components —
// react-refresh disables hot reloading for any module that also exports values.
// Named `...TabModel` rather than `meetingDetailTabs` because a lowercase twin
// of MeetingDetailTabs.tsx would be a trap on case-insensitive filesystems.

/** The four detail tabs of plan §6.13, in their fixed order. */
export const MEETING_DETAIL_TABS = [
  "overview",
  "transcript",
  "audio",
  "processing",
] as const;

export type MeetingDetailTab = (typeof MEETING_DETAIL_TABS)[number];

/** i18n key of a tab's visible label. */
export const tabLabelKey = (tab: MeetingDetailTab): string =>
  `meeting.detail.tabs.${tab}`;

/** Tabs a user can actually reach, in display order. */
export const selectableTabs = (
  disabled: Partial<Record<MeetingDetailTab, string>>,
): MeetingDetailTab[] => MEETING_DETAIL_TABS.filter((tab) => !disabled[tab]);

/**
 * Roving-tabindex step: the tab `delta` positions away from `active`, wrapping
 * around and skipping unreachable tabs.
 *
 * Pure so the keyboard contract (§6.14 "vollständige Tastaturbedienung") is
 * testable without a DOM. An `active` tab that is itself unreachable — the
 * caller is about to correct that — steps relative to the first reachable one.
 */
export const stepTab = (
  active: MeetingDetailTab,
  delta: number,
  selectable: MeetingDetailTab[],
): MeetingDetailTab | null => {
  if (selectable.length === 0) return null;
  const current = selectable.indexOf(active);
  const base = current === -1 ? 0 : current;
  const index = (base + delta) % selectable.length;
  return selectable[index < 0 ? index + selectable.length : index];
};

/** Home/End target among the reachable tabs. */
export const edgeTab = (
  edge: "first" | "last",
  selectable: MeetingDetailTab[],
): MeetingDetailTab | null => {
  if (selectable.length === 0) return null;
  return edge === "first" ? selectable[0] : selectable[selectable.length - 1];
};
