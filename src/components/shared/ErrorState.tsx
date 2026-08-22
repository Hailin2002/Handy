import React from "react";
import { StateShell, type StateShellProps, type StateTone } from "./stateShell";

/**
 * ErrorState primitive (plan §6.2 rule 8, §6.8).
 *
 * Errors explain the next action: icon + headline + at least one labeled
 * recovery button (`Erneut versuchen`, `Gerät wechseln`, …) are mandatory —
 * the non-empty `actions` tuple makes a dead-end error unrepresentable.
 * The danger color always ships with the required icon and title text, so
 * color alone never carries the information. Rendered as `role="alert"` so
 * screen readers announce it immediately.
 */
export interface ErrorStateProps extends Omit<StateShellProps, "tone"> {
  /** Defaults to `danger`; `warning` and `info` are valid softer variants (§6.8). */
  tone?: Extract<StateTone, "danger" | "warning" | "info">;
}

export const ErrorState: React.FC<ErrorStateProps> = (props) => (
  <StateShell {...props} tone={props.tone ?? "danger"} role="alert" />
);

export default ErrorState;
