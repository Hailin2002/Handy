import { test, expect } from "@playwright/test";
import { installTauriMock, tEn } from "./helpers/tauri-mock";

test.describe("App shell (mocked Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
  });

  test("renders the primary regions: product navigation, dictate home, footer", async ({
    page,
  }) => {
    // Product navigation (plan §6.3): Diktieren / Meetings / Projekte /
    // Diktatverlauf above the divider, Einstellungen / Hilfe & Über below.
    const sidebar = page.getByRole("navigation");
    await expect(sidebar).toBeVisible();
    for (const key of [
      "nav.dictate",
      "nav.meetings",
      "nav.projects",
      "nav.history",
      "nav.settings",
      "nav.about",
    ]) {
      await expect(
        sidebar.getByRole("button", { name: tEn(key) }),
      ).toBeVisible();
    }

    // The default view is the DictationHome start page (plan §6.4).
    await expect(
      page.getByRole("heading", { name: tEn("dictation.title"), exact: true }),
    ).toBeVisible();

    // Footer hosts the model selector button (name comes from the
    // i18n-translated model name rendered inside the button).
    const modelName = tEn("onboarding.models.small.name");
    await expect(page.getByRole("button", { name: modelName })).toBeVisible();
  });

  test("general settings lists its sound controls with translated labels", async ({
    page,
  }) => {
    // Settings are now a dedicated view with secondary tabs (plan §6.3);
    // the sound controls live on the Diktieren tab via GeneralSettings.
    await page
      .getByRole("navigation")
      .getByRole("button", { name: tEn("nav.settings") })
      .click();
    await page
      .getByRole("tablist", { name: tEn("nav.settingsTabsAriaLabel") })
      .getByRole("tab", { name: tEn("nav.dictate") })
      .click();

    for (const key of [
      "settings.sound.microphone.title",
      "settings.sound.channel.title",
      "settings.sound.audioFeedback.label",
      "settings.sound.outputDevice.title",
      "settings.sound.volume.title",
    ]) {
      await expect(page.getByRole("heading", { name: tEn(key) })).toBeVisible();
    }

    // The shortcut group renders the transcribe binding name.
    await expect(
      page.getByRole("heading", {
        name: tEn("settings.general.shortcut.bindings.transcribe.name"),
      }),
    ).toBeVisible();
  });
});
