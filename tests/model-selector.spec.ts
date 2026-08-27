import { test, expect, type Page } from "@playwright/test";
import {
  installTauriMock,
  readBridge,
  tEn,
  wasInvokedWith,
} from "./helpers/tauri-mock";

test.describe("Model selector (mocked Tauri IPC)", () => {
  // The footer status button's accessible name is the translated model name.
  const statusButton = (page: Page, key: string) =>
    page.getByRole("button", { name: tEn(key) });

  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
  });

  test("displays the active model name in the footer", async ({ page }) => {
    const status = statusButton(page, "onboarding.models.small.name");
    await expect(status).toBeVisible();
    await expect(status).toContainText(tEn("onboarding.models.small.name"));
  });

  test("dropdown lists the downloaded models with the active one marked", async ({
    page,
  }) => {
    await statusButton(page, "onboarding.models.small.name").click();

    const dropdown = page.locator(".absolute.bottom-full");
    const smallName = tEn("onboarding.models.small.name");
    const parakeetName = tEn("onboarding.models.parakeet-tdt-0.6b-v3.name");

    // Both downloaded fixture models are offered...
    await expect(dropdown.getByText(smallName)).toBeVisible();
    await expect(dropdown.getByText(parakeetName)).toBeVisible();

    // ...the not-downloaded model is not offered.
    expect(dropdown.getByText(tEn("onboarding.models.large.name"))).toHaveCount(
      0,
    );

    // The current model carries the "Active" badge.
    await expect(dropdown.getByText(tEn("modelSelector.active"))).toHaveCount(
      1,
    );
  });

  test("selecting another model persists the choice via set_active_model", async ({
    page,
  }) => {
    await statusButton(page, "onboarding.models.small.name").click();
    const parakeetName = tEn("onboarding.models.parakeet-tdt-0.6b-v3.name");
    await page.locator(".absolute.bottom-full").getByText(parakeetName).click();

    // Optimistic switch: footer now shows the new model.
    await expect(
      statusButton(page, "onboarding.models.parakeet-tdt-0.6b-v3.name"),
    ).toBeVisible();

    // And the backend was asked to persist it.
    const snapshot = await readBridge(page);
    expect(
      wasInvokedWith(snapshot, "set_active_model", {
        modelId: "parakeet-tdt-0.6b-v3",
      }),
    ).toBe(true);
  });
});
