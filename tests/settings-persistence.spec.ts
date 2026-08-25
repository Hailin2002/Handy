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

/**
 * Clicks the visible toggle track for the Audio Feedback row.
 *
 * The checkbox itself is `sr-only` (1px, clipped, negative margin) and only
 * exists as the peer/accessible input of the styled switch. A forced click on
 * that invisible 1px box is not a faithful user interaction — Firefox's real
 * hit-test lands on the surrounding container instead of the label and the
 * state never flips ("Clicking the checkbox did not change its state").
 * Users click the visible track, which sits inside the wrapping <label> and
 * toggles the very same input in every browser.
 */
const clickAudioFeedbackSwitch = async (page: Page): Promise<void> => {
  await page
    .locator(".min-h-12", {
      has: page.getByRole("heading", {
        name: tEn("settings.sound.audioFeedback.label"),
      }),
    })
    .locator("label")
    .click();
};

/**
 * The sound controls live on the Einstellungen page's "Diktieren" tab
 * (plan §6.3); the app boots on the DictationHome start view, so navigate
 * there first.
 */
const openDictationSettingsTab = async (page: Page): Promise<void> => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: tEn("nav.settings") })
    .click();
  await page
    .getByRole("tablist", { name: tEn("nav.settingsTabsAriaLabel") })
    .getByRole("tab", { name: tEn("nav.dictate") })
    .click();
};

test.describe("Settings persistence through the mocked store bridge", () => {
  test("toggling audio feedback persists the change via the store bridge", async ({
    page,
  }) => {
    await installTauriMock(page);
    await page.goto("/");
    await openDictationSettingsTab(page);

    const toggle = audioFeedbackToggle(page);
    await expect(toggle).not.toBeChecked();

    await clickAudioFeedbackSwitch(page);

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
    await openDictationSettingsTab(page);

    const toggle = audioFeedbackToggle(page);
    await expect(toggle).toBeChecked();

    // And the loaded value matches what the bridge reports.
    expect(await getBridgeSetting(page, "audio_feedback")).toBe(true);
  });
});
