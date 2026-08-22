import type { ComponentType } from "react";
import {
  AudioLines,
  CircleHelp,
  FolderKanban,
  History,
  Mic,
  Settings,
} from "lucide-react";

/**
 * Product navigation model (plan §6.3).
 *
 * The sidebar is no longer a list of settings sections. It is a product
 * navigation with two groups separated by a visual divider:
 *
 *   Diktieren / Meetings / Projekte / Diktatverlauf
 *   ─────────────
 *   Einstellungen / Hilfe & Über
 */

export interface NavIconProps {
  width?: number | string;
  height?: number | string;
  className?: string;
}

export type ProductViewId =
  | "dictate"
  | "meetings"
  | "projects"
  | "history"
  | "settings"
  | "about";

export interface ProductNavEntry {
  id: ProductViewId;
  labelKey: string;
  icon: ComponentType<NavIconProps>;
}

/** Primary product tools, rendered above the divider. */
export const PRIMARY_NAV: readonly ProductNavEntry[] = [
  { id: "dictate", labelKey: "nav.dictate", icon: Mic },
  { id: "meetings", labelKey: "nav.meetings", icon: AudioLines },
  { id: "projects", labelKey: "nav.projects", icon: FolderKanban },
  { id: "history", labelKey: "nav.history", icon: History },
];

/** Secondary destinations, rendered below the divider. */
export const SECONDARY_NAV: readonly ProductNavEntry[] = [
  { id: "settings", labelKey: "nav.settings", icon: Settings },
  { id: "about", labelKey: "nav.about", icon: CircleHelp },
];

/**
 * Feature flag for meeting mode. Meeting mode does not exist yet, so the
 * pinned active-meeting entry under "Meetings" stays hidden unless the flag
 * is enabled at build time AND an active meeting is reported (which cannot
 * happen today — see ActiveMeetingNavItem).
 */
export const MEETING_MODE_ENABLED: boolean =
  import.meta.env.VITE_ENABLE_MEETING_MODE === "true";

export interface ActiveMeetingInfo {
  /** Seconds elapsed since the meeting recording started. */
  elapsedSeconds: number;
}

/**
 * Formats an elapsed-meeting duration as `mm:ss`, switching to `hh:mm:ss`
 * once the recording passes one hour (plan §6.3 shows `00:42:18`).
 */
export function formatMeetingDuration(totalSeconds: number): string {
  const total = Math.max(0, Math.floor(totalSeconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  return hours > 0
    ? `${pad(hours)}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}
