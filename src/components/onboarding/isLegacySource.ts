// Whether a model comes from the pre-catalog download source.
//
// Split out of ModelCard.tsx so that file exports only its component;
// react-refresh cannot hot-reload a module that also exports values.

import type { ModelInfo } from "@/bindings";

export const isLegacySource = (model: ModelInfo): boolean =>
  typeof model.source === "object" && "Url" in model.source;
