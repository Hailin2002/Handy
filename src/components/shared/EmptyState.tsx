import React from "react";
import { StateShell, type StateShellProps, type StateTone } from "./stateShell";

/**
 * EmptyState primitive (plan §6.2 rule 8, §6.5).
 *
 * Renders icon + title + at least one labeled next-action button; TypeScript
 * rejects instances without an action because `actions` is a non-empty tuple
 * type. Color is never the sole carrier: the icon and the title text are
 * required props and the default tone is neutral.
 */
export interface EmptyStateProps extends Omit<StateShellProps, "tone"> {
  /** Empty states are informational by default; `info` is allowed (§6.8). */
  tone?: Extract<StateTone, "neutral" | "info">;
}

export const EmptyState: React.FC<EmptyStateProps> = (props) => (
  <StateShell {...props} tone={props.tone ?? "neutral"} role="status" />
);

export default EmptyState;
