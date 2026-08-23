import { test, expect } from "@playwright/test";
import {
  installTauriMock,
  tEn,
} from "./helpers/tauri-mock";

test.describe("App shell (mocked Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
  });

  test("renders the primary regions: sidebar navigation, settings content, footer", async ({
    page,
  }) => {
    // Sidebar navigation with translated section labels (i18n keys).
    const sidebar = page.getByRole("navigation");
    await expect(sidebar).toBeVisible();
    for (const key of [
      "sidebar.general",
      "sidebar.history",
      "sidebar.models",
      "sidebar.advanced",
      "sidebar.about",
    ]) {
      await expect(
        sidebar.getByRole("button", { name: tEn(key) }),
      ).toBeVisible();
    }

    // Main settings content area shows the general and sound groups.
    await expect(
      page.getByRole("heading", { name: tEn("settings.general.title") }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: tEn("settings.sound.title") }),
    ).toBeVisible();

    // Footer hosts the model selector button (name comes from the
    // i18n-translated model name rendered inside the button).
    const modelName = tEn("onboarding.models.small.name");
    await expect(
      page.getByRole("button", { name: modelName }),
    ).toBeVisible();
  });

  test("general settings lists its sound controls with translated labels", async ({
    page,
  }) => {
    for (const key of [
      "settings.sound.microphone.title",
      "settings.sound.channel.title",
      "settings.sound.audioFeedback.label",
      "settings.sound.outputDevice.title",
      "settings.sound.volume.title",
    ]) {
      await expect(
        page.getByRole("heading", { name: tEn(key) }),
      ).toBeVisible();
    }

    // The shortcut group renders the transcribe binding name.
    await expect(
      page.getByRole("heading", { name: tEn("settings.general.shortcut.bindings.transcribe.name") }),
    ).toBeVisible();
  });
});
