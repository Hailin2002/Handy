// Resolves the LocalizedText union to a string.
//
// Split out of stateShell.tsx so that file exports only components —
// react-refresh disables hot reloading for any module that also exports values.

import type { TFunction } from "i18next";
import type { LocalizedText } from "./stateShell";

export function resolveLocalizedText(
  t: TFunction,
  text: LocalizedText,
): string {
  if (typeof text === "string") {
    return text;
  }
  return String(t(text.key, text.values));
}
