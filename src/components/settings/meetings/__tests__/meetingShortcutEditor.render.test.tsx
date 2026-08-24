/**
 * Render assertions for the §6.9 meeting shortcut editor on the
 * Meetings settings tab (MeetingShortcutEditor.tsx /
 * MeetingsSettingsTab.tsx).
 *
 * Runs in the repo's node-environment vitest setup via
 * react-dom/server, initialized with the REAL en and de translation
 * files so the i18n wiring itself is under test:
 *
 * - every §6.9 visible action exists as real button text: Reset,
 *   Disable and "Test shortcut" next to each clickable keycap;
 * - unassigned actions say "Not configured";
 * - G2 display surface: BOTH conflicting locations render
 *   simultaneously on a single alert;
 * - de/translation.json carries every new key.
 */
import { describe, expect, it } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";

import enTranslation from "@/i18n/locales/en/translation.json";
import deTranslation from "@/i18n/locales/de/translation.json";
import type { ShortcutLocation } from "../meetingShortcutRules";
import {
  MeetingShortcutConflictList,
  MeetingShortcutEditor,
} from "../MeetingShortcutEditor";
import { MeetingsSettingsTab } from "../../tabs/MeetingsSettingsTab";

const initI18n = async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: "en",
      fallbackLng: "en",
      initImmediate: false,
      resources: { en: { translation: enTranslation } },
    });
  }
};

describe("MeetingShortcutEditor (§6.9)", () => {
  it("renders both action rows with visible keycap, Reset, Disable and Test shortcut controls", async () => {
    await initI18n();

    const html = renderToStaticMarkup(
      React.createElement(MeetingShortcutEditor),
    );

    // Section header.
    expect(html).toContain("Meeting Shortcuts");

    // Both §6.9 actions with their descriptions.
    expect(html).toContain("Start/end meeting");
    expect(html).toContain("Pause/resume meeting");
    expect(html).toContain("short countdown");

    // The three mandatory visible actions appear once per row.
    const resetCount = html.split(">Reset<").length - 1;
    const disableCount = html.split(">Disable<").length - 1;
    const testCount = html.split(">Test shortcut<").length - 1;
    expect(resetCount).toBe(2);
    expect(disableCount).toBe(2);
    expect(testCount).toBe(2);

    // Unassigned-by-default rows show an honest placeholder…
    expect(html).toContain("Not configured");
    // …and every keycap is a real button announcing its action + value.
    // ("/" is HTML-escaped in attributes by react-dom/server.)
    expect(html).toContain(
      'aria-label="Change Start&amp;#x2F;end meeting shortcut, currently Not configured',
    );
    expect(html).toContain(
      'aria-label="Change Pause&amp;#x2F;resume meeting shortcut, currently Not configured',
    );
    expect(html).toMatch(/<button type="button"[^>]*>Not configured<\/button>/);

    // No raw i18n keys leak into the markup.
    expect(html).not.toContain("settings.meetings.shortcut");
  });

  it("G2 display surface: renders BOTH conflict locations simultaneously", async () => {
    await initI18n();

    const conflicts: readonly ShortcutLocation[] = [
      {
        kind: "speak",
        id: "transcribe",
        label: "Transcribe Shortcut",
        hotkey: "ctrl+space",
      },
      {
        kind: "meeting",
        id: "pauseResume",
        label: "Pause/resume meeting",
        hotkey: "ctrl+space",
      },
    ];

    const html = renderToStaticMarkup(
      React.createElement(MeetingShortcutConflictList, { conflicts }),
    );

    // One alert contains both locations at the same time…
    expect(html).toContain('role="alert"');
    expect(html).toContain("This shortcut is already used by:");
    expect(html).toContain("<li>Transcribe Shortcut (Ctrl + Space)</li>");
    expect(html).toContain("<li>Pause/resume meeting (Ctrl + Space)</li>");
    // …and no location is hidden behind interaction.
    expect(html).not.toContain("display: none");
  });
});

describe("MeetingsSettingsTab integration", () => {
  it("embeds the shortcut editor below the tab heading and keeps the honest placeholder note", async () => {
    await initI18n();

    const html = renderToStaticMarkup(React.createElement(MeetingsSettingsTab));

    expect(html).toContain(">Meetings</h2>");
    expect(html).toContain("Meeting Shortcuts");
    expect(html).toContain("Meeting settings will arrive with meeting mode.");
  });
});

describe("i18n completeness for the meeting shortcut editor", () => {
  const collectKeyPaths = (value: unknown, prefix: string[] = []): string[] => {
    if (typeof value !== "object" || value === null) return [];
    return Object.entries(value as Record<string, unknown>).flatMap(
      ([key, child]) =>
        typeof child === "object" && child !== null
          ? collectKeyPaths(child, [...prefix, key])
          : [[...prefix, key].join(".")],
    );
  };

  it("de carries exactly the en keys under settings.meetings.shortcut", () => {
    const enRoot = enTranslation as Record<string, unknown>;
    const deRoot = deTranslation as Record<string, unknown>;
    const enSection = (enRoot.settings as Record<string, unknown>)
      .meetings as Record<string, unknown>;
    const deSection = (deRoot.settings as Record<string, unknown>)
      .meetings as Record<string, unknown>;

    const enKeys = collectKeyPaths(enSection).sort();
    const deKeys = collectKeyPaths(deSection).sort();
    expect(deKeys).toEqual(enKeys);
    // And the gate-critical named message exists in both languages.
    expect(enKeys).toContain("shortcut.errors.singleKey");
    expect(enKeys).toContain("shortcut.errors.conflict");
  });
});
