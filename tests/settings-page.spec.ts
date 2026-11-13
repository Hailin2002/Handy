import { test, expect, type Page } from "@playwright/test";

/**
 * Acceptance gates for the consolidated Einstellungen page (plan §6.3):
 *  - G2: the Debug tab is hidden unless debug mode is enabled;
 *  - G3: keyboard-operable ARIA tablist (roles, roving tabindex,
 *        arrow/Home/End navigation);
 *  - deep links: canonical `#/settings/<tab>` and legacy section hashes
 *    open the mapped tab.
 *
 * The harness mounts SettingsPage without the Tauri backend, so settings
 * are still unloaded (null) — the same visibility rules apply as with
 * debug_mode disabled.
 */

const HARNESS = "/tests/harness/settings.html";

// Every call must produce a real document load: two URLs differing only in
// their hash are same-document navigations in Chromium, which would keep the
// previously mounted page (and its selected tab) alive instead of exercising
// the first-mount deep-link resolution under test.
let loadSeq = 0;

const TAB_LABELS = [
  "Dictate",
  "Meetings",
  "Models & Providers",
  "General",
  "Advanced",
];

async function gotoSettings(page: Page, hash = "") {
  await page.goto(`${HARNESS}?load=${++loadSeq}${hash}`);
  await expect(page.getByRole("tablist")).toBeVisible();
}

async function activeTabId(page: Page): Promise<string> {
  return page
    .getByRole("tab", { selected: true })
    .getAttribute("id")
    .then((id) => {
      if (!id) throw new Error("no active tab id");
      return id.replace("settings-tab-", "");
    });
}

test.describe("Einstellungen page tabs", () => {
  test("renders an ARIA tablist with five visible tabs and no Debug tab (G2)", async ({
    page,
  }) => {
    await gotoSettings(page);

    const tablist = page.getByRole("tablist");
    await expect(tablist).toHaveAttribute("aria-label", "Settings sections");
    const tabs = page.getByRole("tab");
    await expect(tabs).toHaveCount(5);
    await expect(tabs.filter({ hasText: "Debug" })).toHaveCount(0);

    for (const [index, label] of TAB_LABELS.entries()) {
      const tab = page.getByRole("tab", { name: label });
      await expect(tab).toBeVisible();
      await expect(tab).toHaveAttribute(
        "aria-selected",
        index === 0 ? "true" : "false",
      );
    }

    // Roving tabindex: only the active tab is in the page tab order.
    await expect(page.getByRole("tab", { name: "Dictate" })).toHaveAttribute(
      "tabindex",
      "0",
    );
    await expect(page.getByRole("tab", { name: "Meetings" })).toHaveAttribute(
      "tabindex",
      "-1",
    );

    // Panel wiring.
    const panel = page.getByRole("tabpanel");
    await expect(panel).toHaveAttribute("id", "settings-panel-dictate");
    await expect(panel).toHaveAttribute(
      "aria-labelledby",
      "settings-tab-dictate",
    );
  });

  test("click activates a tab, swaps the panel and mirrors the hash", async ({
    page,
  }) => {
    await gotoSettings(page);

    await page.getByRole("tab", { name: "Models & Providers" }).click();
    await expect(
      page.getByRole("tab", { name: "Models & Providers" }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(activeTabId(page)).resolves.toBe("modelsProviders");
    await expect(page).toHaveURL(/#\/settings\/modelsProviders$/);

    const panel = page.getByRole("tabpanel");
    await expect(panel).toHaveAttribute("id", "settings-panel-modelsProviders");
    await expect(panel).toHaveAttribute(
      "aria-labelledby",
      "settings-tab-modelsProviders",
    );
  });

  test("Meetings exposes local and OpenRouter summary configuration", async ({
    page,
  }) => {
    await gotoSettings(page);

    await page.getByRole("tab", { name: "Meetings" }).click();
    await expect(
      page.getByText("Meeting summary", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Local GPU (Ollama)", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(/Local summaries use Ollama at 127\.0\.0\.1/),
    ).toBeVisible();
  });

  test("arrow keys move focus and selection; Home/End jump (G3)", async ({
    page,
  }) => {
    await gotoSettings(page);

    const first = page.getByRole("tab", { name: "Dictate" });
    await first.focus();

    // ArrowRight: focus AND selection advance (automatic activation).
    await page.keyboard.press("ArrowRight");
    const meetings = page.getByRole("tab", { name: "Meetings" });
    await expect(meetings).toBeFocused();
    await expect(meetings).toHaveAttribute("aria-selected", "true");

    // ArrowLeft returns to the first tab.
    await page.keyboard.press("ArrowLeft");
    await expect(first).toBeFocused();
    await expect(first).toHaveAttribute("aria-selected", "true");

    // Home/End jump to the strip bounds.
    await page.keyboard.press("End");
    const last = page.getByRole("tab", { name: "Advanced" });
    await expect(last).toBeFocused();
    await expect(last).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Home");
    await expect(first).toBeFocused();
    await expect(first).toHaveAttribute("aria-selected", "true");
  });

  test("deep links open the mapped tab, including legacy section hashes", async ({
    page,
  }) => {
    // Eight full navigations, each a complete page load and React mount. That
    // does not fit Playwright's 30s default on a dev server, especially in
    // Firefox where React's development-mode double-invoked effects make each
    // mount measurably slower. `slow()` triples the budget rather than the test
    // being flaky about how busy the machine is.
    test.slow();
    // Canonical tab ids.
    await gotoSettings(page, "#/settings/modelsProviders");
    await expect(activeTabId(page)).resolves.toBe("modelsProviders");

    await gotoSettings(page, "#/settings/app");
    await expect(activeTabId(page)).resolves.toBe("app");

    // Legacy sidebar section ids map onto their new tabs.
    await gotoSettings(page, "#/settings/models");
    await expect(activeTabId(page)).resolves.toBe("modelsProviders");

    await gotoSettings(page, "#/settings/speechPipeline");
    await expect(activeTabId(page)).resolves.toBe("modelsProviders");

    await gotoSettings(page, "#/settings/postprocessing");
    await expect(activeTabId(page)).resolves.toBe("dictate");

    await gotoSettings(page, "#general");
    await expect(activeTabId(page)).resolves.toBe("dictate");

    await gotoSettings(page, "#/advanced");
    await expect(activeTabId(page)).resolves.toBe("advanced");

    // Unknown hashes fall back to the first tab.
    await gotoSettings(page, "#/something-else");
    await expect(activeTabId(page)).resolves.toBe("dictate");
  });

  test("external hashchange navigates to the mapped tab", async ({ page }) => {
    await gotoSettings(page);
    await page.evaluate(() => {
      window.location.hash = "#/settings/app";
    });
    await expect(activeTabId(page)).resolves.toBe("app");
  });

  test("Debug tab appears only while debug mode is enabled (G2)", async ({
    page,
  }) => {
    await gotoSettings(page);
    await expect(page.getByRole("tab", { name: "Debug" })).toHaveCount(0);

    await page.evaluate(() => {
      (
        window as unknown as { __setDebugMode: (on: boolean) => void }
      ).__setDebugMode(true);
    });
    const debugTab = page.getByRole("tab", { name: "Debug" });
    await expect(debugTab).toBeVisible();
    await debugTab.click();
    await expect(activeTabId(page)).resolves.toBe("debug");
    await expect(page).toHaveURL(/#\/settings\/debug$/);

    // Disabling debug mode again hides the tab and falls back to a valid one.
    await page.evaluate(() => {
      (
        window as unknown as { __setDebugMode: (on: boolean) => void }
      ).__setDebugMode(false);
    });
    await expect(page.getByRole("tab", { name: "Debug" })).toHaveCount(0);
    await expect(activeTabId(page)).resolves.toBe("dictate");
  });
});
