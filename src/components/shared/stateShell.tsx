import React from "react";
import { resolveLocalizedText } from "./resolveLocalizedText";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";

/**
 * Shared shell for EmptyState and ErrorState (plan §6.2 rule 8).
 *
 * Invariants enforced here:
 * - every instance renders an icon, a title and at least one labeled action
 *   button (the tuple type makes an empty action list unrepresentable);
 * - color is never the sole information carrier: the icon and the title text
 *   are required props, tone only adds color/icon styling on top (§6.8);
 * - user-facing copy flows through i18n: either an already-translated string
 *   or an i18next key resolved via `useTranslation`.
 */

/** Text that supports i18n: a pre-translated string or an i18next key. */
export type LocalizedText =
  string | { key: string; values?: Record<string, unknown> };

/**
 * Semantic tone per plan §6.8. Tones always ship with icon + text, so color
 * alone never carries the state information.
 */
export type StateTone = "neutral" | "info" | "warning" | "danger";

/** A single next-action button. At least one is required by the shell API. */
export interface StateAction {
  /** Visible button label (i18n-aware). */
  label: LocalizedText;
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
  /** Defaults: first action renders as `primary`, later ones as `secondary`. */
  variant?: "primary" | "primary-soft" | "secondary" | "ghost";
  /** Optional button size; defaults to `md` (plan §6.5 uses `lg` for hero actions). */
  size?: "sm" | "md" | "lg";
  /** Optional small leading glyph, purely decorative (aria-hidden). */
  icon?: React.ReactNode;
  disabled?: boolean;
}

interface BaseStateShellProps {
  /** Required decorative symbol shown above the title (aria-hidden). */
  icon: React.ReactNode;
  /** Required headline text (i18n-aware); carries the state semantically. */
  title: LocalizedText;
  /** Optional supporting sentence (i18n-aware). */
  description?: LocalizedText;
  /** Heading level for the title, defaults to `h2`. */
  headingLevel?: 2 | 3 | 4;
  /** Visual tone; defaults are set by EmptyState (`neutral`) and ErrorState (`danger`). */
  tone?: StateTone;
  className?: string;
}

export type StateShellProps = BaseStateShellProps & {
  /**
   * Next-action buttons. The leading tuple element is mandatory, so TypeScript
   * rejects instances without at least one action (plan §6.2 rule 8).
   */
  actions: [StateAction, ...StateAction[]];
};

const toneClasses: Record<StateTone, { iconWrap: string }> = {
  neutral: { iconWrap: "text-text/50" },
  info: { iconWrap: "text-blue-400" },
  warning: { iconWrap: "text-warning" },
  danger: { iconWrap: "text-red-400" },
};

export const StateShell: React.FC<
  StateShellProps & { role: "status" | "alert" }
> = ({
  role,
  icon,
  title,
  description,
  headingLevel = 2,
  tone = "neutral",
  actions,
  className = "",
}) => {
  const { t } = useTranslation();
  const HeadingTag = `h${headingLevel}` as "h2" | "h3" | "h4";
  const resolvedTitle = resolveLocalizedText(t, title);

  return (
    <div
      role={role}
      className={`flex flex-col items-center justify-center px-6 py-10 text-center ${className}`}
    >
      {/* Decorative: the title text carries the meaning, never the color. */}
      <span aria-hidden="true" className={`mb-3 ${toneClasses[tone].iconWrap}`}>
        {icon}
      </span>
      <HeadingTag className="text-base font-semibold text-text">
        {resolvedTitle}
      </HeadingTag>
      {description !== undefined && (
        <p className="mt-1 max-w-sm text-sm text-text/70">
          {resolveLocalizedText(t, description)}
        </p>
      )}
      {/* Rule 8: a state without a next action is a type error, not a runtime hope. */}
      <div className="mt-5 flex flex-wrap items-center justify-center gap-2">
        {actions.map((action, index) => (
          <Button
            key={index}
            variant={action.variant ?? (index === 0 ? "primary" : "secondary")}
            size={action.size}
            onClick={action.onClick}
            disabled={action.disabled}
          >
            {action.icon != null && (
              <span aria-hidden="true" className="me-1.5 inline-flex">
                {action.icon}
              </span>
            )}
            {resolveLocalizedText(t, action.label)}
          </Button>
        ))}
      </div>
    </div>
  );
};
