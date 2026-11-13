import type { AssistantOutputMode } from "@/bindings";

/**
 * The two shipped assistant profiles.
 *
 * These ids are a backend contract, not a UI choice: they are the
 * `assistant_profiles` map keys, the secret id under `ASSISTANT_SCOPE`, and
 * the `AssistantProfile::id` field, all kept in sync by
 * `ensure_assistant_defaults`. Nothing here may invent a third one.
 */
export const ASSISTANT_PROFILE_IDS = [
  "fast_helper",
  "smart_assistant",
] as const;

export type AssistantProfileId = (typeof ASSISTANT_PROFILE_IDS)[number];

/**
 * Shortcut binding that triggers each profile. Shipped unbound, so the
 * settings row is also the only place a user discovers the feature exists.
 */
export const ASSISTANT_SHORTCUT_IDS: Record<AssistantProfileId, string> = {
  fast_helper: "assistant_fast",
  smart_assistant: "assistant_smart",
};

/** Presentation order of the output-mode choice: least to most intrusive. */
export const ASSISTANT_OUTPUT_MODES: readonly AssistantOutputMode[] = [
  "show",
  "copy",
  "paste",
];

export interface AssistantLimitRange {
  min: number;
  max: number;
}

/**
 * Mirrors the ranges `change_assistant_limits_setting` clamps to on the Rust
 * side. Duplicated here on purpose: clamping in the form means the field snaps
 * to a legal value the moment it is left, instead of silently disagreeing with
 * the store until the next refresh reveals the correction.
 */
export const ASSISTANT_LIMITS: {
  maxOutputTokens: AssistantLimitRange;
  timeoutSecs: AssistantLimitRange;
} = {
  maxOutputTokens: { min: 64, max: 8192 },
  timeoutSecs: { min: 5, max: 300 },
};

/**
 * Clamps one limit into `range`, rounding to a whole number.
 *
 * A non-finite input (an emptied or half-typed number field yields `NaN`)
 * falls back to `fallback` rather than to a bound: blanking the field should
 * restore the value that was there, not silently rewrite it to 64.
 */
export const clampAssistantLimit = (
  value: number,
  range: AssistantLimitRange,
  fallback: number,
): number => {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(range.max, Math.max(range.min, Math.round(value)));
};
