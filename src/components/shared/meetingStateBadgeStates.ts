// The badge's state list, used by tests and by the badge itself.
//
// Split out of MeetingStateBadge.tsx so that file exports only its component;
// react-refresh cannot hot-reload a module that also exports values.

import type { MeetingStateBadgeState } from "./MeetingStateBadge";

/** Closed, ordered list of all §6.8 badge states. */
export const meetingStateBadgeStates = [
  "recording",
  "stopping",
  "interrupted",
  "attention",
  "complete",
  "processing",
  "paused",
] as const satisfies readonly MeetingStateBadgeState[];
