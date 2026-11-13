/**
 * The assistant hotkeys are edited together on the Assistants tab. This pins
 * that the group renders exactly the two backend binding ids, in profile
 * order, so a typo here cannot point an editor at a binding the shortcut
 * handler never dispatches on.
 *
 * Same harness as the sibling render tests: `react-dom/server`, no DOM. The
 * shortcut editor itself is stubbed — it is covered by its own tests and drags
 * in the whole global-shortcut stack.
 */
import { describe, expect, it, vi } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("@/components/settings/general/ShortcutInput", () => ({
  ShortcutInput: ({ shortcutId }: { shortcutId: string }) =>
    React.createElement("div", { "data-shortcut-id": shortcutId }),
}));

import { AssistantShortcuts } from "../AssistantShortcuts";

describe("AssistantShortcuts", () => {
  it("renders the fast and smart binding editors, fast first", () => {
    const html = renderToStaticMarkup(React.createElement(AssistantShortcuts));

    const fast = html.indexOf('data-shortcut-id="assistant_fast"');
    const smart = html.indexOf('data-shortcut-id="assistant_smart"');
    expect(fast).toBeGreaterThanOrEqual(0);
    expect(smart).toBeGreaterThan(fast);
    // Exactly two editors: no third profile has been invented here.
    expect(html.match(/data-shortcut-id=/g)).toHaveLength(2);
  });
});
