import React from "react";
import { useTranslation } from "react-i18next";
import {
  CircleCheck,
  CircleDot,
  CircleSlash,
  LoaderCircle,
  Pause,
  Square,
  TriangleAlert,
} from "lucide-react";
import { type LocalizedText } from "./stateShell";
import { resolveLocalizedText } from "./resolveLocalizedText";

/**
 * MeetingStateBadge primitive (plan §6.8).
 *
 * Encodes the §6.8 color/action semantics for meeting states:
 * - red    = recording / stopping
 * - orange = interrupted / attention required
 * - green  = fully and safely completed
 * - blue   = processing / information
 * - neutral = paused (clearly labeled, deliberately unobtrusive)
 *
 * Invariant (§6.8): color is NEVER the sole information carrier. Every state
 * renders BOTH a non-empty translated text label AND a distinct decorative
 * icon (`aria-hidden`), so the badge stays readable without color vision and
 * in screen readers. `meetingStateBadgeStates` is the closed list of states
 * and the render test asserts the invariant for every entry.
 */

/** The seven §6.8 meeting states, grouped by palette semantics. */
export type MeetingStateBadgeState =
  | "recording" // red — actively recording ("Rot erscheint erst, wenn wirklich aufgenommen wird")
  | "stopping" // red — stop action/transition ("Meeting beenden … Stop-Symbol plus Text")
  | "interrupted" // orange — recording was interrupted
  | "attention" // orange — attention required
  | "complete" // green — fully and safely completed
  | "processing" // blue — processing / information
  | "paused"; // neutral — pause is neutral and clearly labeled

interface BadgeVisual {
  /** Distinct decorative glyph; always rendered `aria-hidden` next to text. */
  icon: React.ReactNode;
  /** i18n key of the non-empty text label. */
  labelKey: string;
  /** Palette classes encoding the §6.8 color semantics. */
  className: string;
}

const ICON_SIZE = 12;

/**
 * Per-state visuals. Typed as `Record<MeetingStateBadgeState, BadgeVisual>`
 * so adding a state without icon/text/classes is a compile error — the
 * §6.8 invariant cannot regress silently.
 */
const BADGE_VISUALS: Record<MeetingStateBadgeState, BadgeVisual> = {
  recording: {
    icon: <CircleDot size={ICON_SIZE} />,
    labelKey: "meetings.badgeRecording",
    className: "border-error/30 bg-error/10 text-error",
  },
  stopping: {
    icon: <Square size={ICON_SIZE} />,
    labelKey: "meetings.badgeStopping",
    className: "border-error/30 bg-error/10 text-error",
  },
  interrupted: {
    icon: <CircleSlash size={ICON_SIZE} />,
    labelKey: "meetings.badgeInterrupted",
    className: "border-warning/30 bg-warning/10 text-warning",
  },
  attention: {
    icon: <TriangleAlert size={ICON_SIZE} />,
    labelKey: "meetings.badgeAttention",
    className: "border-warning/30 bg-warning/10 text-warning",
  },
  complete: {
    icon: <CircleCheck size={ICON_SIZE} />,
    labelKey: "meetings.badgeComplete",
    className: "border-success/30 bg-success/10 text-success",
  },
  processing: {
    icon: <LoaderCircle size={ICON_SIZE} />,
    labelKey: "meetings.badgeProcessing",
    className: "border-info/30 bg-info/10 text-info",
  },
  paused: {
    icon: <Pause size={ICON_SIZE} />,
    labelKey: "meetings.badgePaused",
    className: "border-mid-gray/25 bg-mid-gray/10 text-text/70",
  },
};

export interface MeetingStateBadgeProps {
  /** One of the seven §6.8 states; decides icon, text and palette. */
  state: MeetingStateBadgeState;
  /**
   * Optional label override (e.g. a countdown or remaining time); defaults to
   * the state's translated label. An empty resolved label falls back to the
   * default so the text carrier can never disappear.
   */
  label?: LocalizedText;
  className?: string;
}

export const MeetingStateBadge: React.FC<MeetingStateBadgeProps> = ({
  state,
  label,
  className = "",
}) => {
  const { t } = useTranslation();
  const visual = BADGE_VISUALS[state];
  const defaultLabel = String(t(visual.labelKey));
  const resolvedLabel =
    label === undefined ? defaultLabel : resolveLocalizedText(t, label);
  const finalLabel =
    resolvedLabel.trim().length > 0 ? resolvedLabel : defaultLabel;

  return (
    <span
      data-meeting-badge={state}
      className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap ${visual.className} ${className}`}
    >
      {/* Decorative: the visible text carries the meaning, never the color. */}
      <span aria-hidden="true" className="inline-flex shrink-0">
        {visual.icon}
      </span>
      {finalLabel}
    </span>
  );
};

export default MeetingStateBadge;
