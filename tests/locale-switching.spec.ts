import { test, expect, type Page } from "@playwright/test";
import { LANGUAGE_METADATA } from "../src/i18n/languages";
import {
  installTauriMock,
  readBridge,
  tDe,
  tEn,
  wasInvokedWith,
} from "./helpers/tauri-mock";

// Dropdown option labels are "<nativeName> (<name>)", derived from language
// metadata rather than from a translation catalog.
const germanOptionLabel = `${LANGUAGE_METADATA.de.nativeName} (${LANGUAGE_METADATA.de.name})`;
const englishOptionLabel = `${LANGUAGE_METADATA.en.nativeName} (${LANGUAGE_METADATA.en.name})`;

// The application-language selector lives on Einstellungen → Allgemein
// (AppGeneralTab, plan §6.3); About no longer hosts it.
const openLanguageSettings = async (page: Page) => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: tEn("nav.settings") })
    .click();
  await page
    .getByRole("tablist", { name: tEn("nav.settingsTabsAriaLabel") })
    .getByRole("tab", { name: tEn("sidebar.general") })
    .click();
};

test.describe("Locale switching (en/de)", () => {
  test("switching to German re-renders translated strings and persists app_language", async ({
    page,
  }) => {
    await installTauriMock(page);
    await page.goto("/");
    await openLanguageSettings(page);

    // Open the application-language dropdown (its closed state shows the
    // currently selected language) and pick German.
    await page.getByRole("button", { name: englishOptionLabel }).click();
    await page
      .locator(".absolute.top-full")
      .getByRole("button", { name: germanOptionLabel })
      .click();

    // Sidebar re-renders with German catalog strings (product navigation,
    // plan §6.3).
    const sidebar = page.getByRole("navigation");
    await expect(
      sidebar.getByRole("button", { name: tDe("nav.dictate") }),
    ).toBeVisible();
    await expect(
      sidebar.getByRole("button", { name: tDe("nav.history") }),
    ).toBeVisible();

    // Document language attribute follows the active locale.
    await expect(page.locator("html")).toHaveAttribute("lang", "de");

    // The choice was persisted through the store bridge.
    const snapshot = await readBridge(page);
    expect(
      wasInvokedWith(snapshot, "change_app_language_setting", {
        language: "de",
      }),
    ).toBe(true);
    expect(snapshot.settings.app_language).toBe("de");
  });

  test("a persisted German locale renders translated strings at boot", async ({
    page,
  }) => {
    // Simulates an app restart where app_language=de was previously saved.
    await installTauriMock(page, { settings: { app_language: "de" } });
    await page.goto("/");

    const sidebar = page.getByRole("navigation");
    for (const key of ["nav.dictate", "nav.meetings", "nav.settings"]) {
      await expect(
        sidebar.getByRole("button", { name: tDe(key) }),
      ).toBeVisible();
    }
    await expect(page.locator("html")).toHaveAttribute("lang", "de");
  });
});
