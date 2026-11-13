import React from "react";
import { ShortcutInput } from "./ShortcutInput";
import { ASSISTANT_SHORTCUT_IDS } from "../assistants/assistantProfiles";

/**
 * The two assistant hotkeys shown on the Assistants tab. The binding ids stay
 * a backend contract (`assistantProfiles.ts`), so the visible editors cannot
 * drift from what the shortcut handler dispatches on.
 */
export const AssistantShortcuts: React.FC = () => (
  <>
    {Object.values(ASSISTANT_SHORTCUT_IDS).map((shortcutId) => (
      <ShortcutInput key={shortcutId} shortcutId={shortcutId} grouped={true} />
    ))}
  </>
);
