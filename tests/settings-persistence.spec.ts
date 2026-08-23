import { test, expect, type Page } from "@playwright/test";
import {
  getBridgeSetting,
  installTauriMock,
  readBridge,
  tEn,
  wasInvokedWith,
} from "./helpers/tauri-mock";

/**
 * The settings row (`div.min-h-12`) whose h3 title is the translated
 * "Audio Feedback" label; returns its on/off switch.
 */
const audioFeedbackToggle = (page: Page) =>
  page
    .locator(".min-h-12", {
      has: page.getByRole("heading", {
        name: tEn("settings.sound.audioFeedback.label"),
      }),
    })
    .getByRole("checkbox");

test.describe("Settings persistence through the mocked store bridge", () => {
  test("toggling audio feedback persists the change via the store bridge", async ({
    page,
  }) => {
    await installTauriMock(page);
    await page.goto("/");

    const toggle = audioFeedbackToggle(page);
    await expect(toggle).not.toBeChecked();

    await toggle.check({ force: true });

    // UI reflects the optimistic update.
    await expect(toggle).toBeChecked();

    // The change crossed the IPC bridge and was persisted in the mock store.
    const snapshot = await readBridge(page);
    expect(
      wasInvokedWith(snapshot, "change_audio_feedback_setting", {
        enabled: true,
      }),
    ).toBe(true);
    expect(snapshot.settings.audio_feedback).toBe(true);
  });

  test("a persisted setting is rendered as checked on load", async ({
    page,
  }) => {
    // Simulate a backend where audio feedback was previously enabled.
    await installTauriMock(page, {
      settings: { audio_feedback: true },
    });
    await page.goto("/");

    const toggle = audioFeedbackToggle(page);
    await expect(toggle).toBeChecked();

    // And the loaded value matches what the bridge reports.
    expect(await getBridgeSetting(page, "audio_feedback")).toBe(true);
  });
});
