import React, { useEffect, useState } from "react";
import { Input } from "@/components/ui/Input";
import {
  type AssistantLimitRange,
  clampAssistantLimit,
} from "./assistantProfiles";

interface AssistantLimitFieldProps {
  value: number;
  range: AssistantLimitRange;
  step?: number;
  disabled?: boolean;
  ariaLabel: string;
  /** Receives an already-clamped, whole-number value. */
  onCommit: (value: number) => void;
}

/**
 * Whole-number field for one of the two assistant limits.
 *
 * Typing is held in a local draft so a half-entered number ("1", on the way to
 * "1024") is never sent to the backend, and the draft is clamped to `range` on
 * commit. Clamping in the field rather than only in Rust means the input snaps
 * to the value that was actually stored, instead of displaying a number the
 * backend already rewrote.
 */
export const AssistantLimitField: React.FC<AssistantLimitFieldProps> = ({
  value,
  range,
  step = 1,
  disabled = false,
  ariaLabel,
  onCommit,
}) => {
  const [draft, setDraft] = useState(String(value));

  // Follow the store: a refresh (or the backend clamping a value further than
  // the field did) has to win over a stale draft.
  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const commit = () => {
    const clamped = clampAssistantLimit(Number(draft), range, value);
    setDraft(String(clamped));
    onCommit(clamped);
  };

  return (
    <Input
      type="number"
      inputMode="numeric"
      value={draft}
      min={range.min}
      max={range.max}
      step={step}
      aria-label={ariaLabel}
      variant="compact"
      disabled={disabled}
      className="w-28 text-end"
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
      }}
    />
  );
};
