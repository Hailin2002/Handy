import { test, expect } from "@playwright/test";
import { installTauriMock, tEn, readBridge } from "./helpers/tauri-mock";

/**
 * Wave-1 primary meeting journey (plan §6.5), end to end against the real UI
 * with the Tauri IPC mocked at the boundary: navigate Meetings → create →
 * open detail → save an edited title → verify it in the list → delete.
 * Persistence semantics themselves are proven by the Rust unit tests in
 * `src-tauri/src/managers/meeting.rs`; this spec proves the frontend seam is
 * actually wired through production navigation.
 */
test.describe("Meetings journey (mocked Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
  });

  const openMeetings = async (page: import("@playwright/test").Page) => {
    await page
      .getByRole("navigation")
      .getByRole("button", { name: tEn("nav.meetings") })
      .click();
  };

  test("empty state offers the New Meeting action", async ({ page }) => {
    await openMeetings(page);
    await expect(
      page.getByRole("button", { name: tEn("meetings.newMeeting") }),
    ).toBeVisible();
    await expect(page.getByText(tEn("meetings.privacyLocal"))).toBeVisible();
  });

  test("create, open, save title and see it listed", async ({ page }) => {
    await openMeetings(page);

    // Create: the page generates a locale-formatted default title.
    await page
      .getByRole("button", { name: tEn("meetings.newMeeting") })
      .click();

    // Detail opens for the new draft session.
    const titleInput = page.getByLabel(tEn("meetings.titleInputLabel"));
    await expect(titleInput).toBeVisible();

    // Save an edited title.
    await titleInput.fill("Budget Review Sync");
    await page.getByRole("button", { name: tEn("meetings.saveTitle") }).click();

    // Back to the library: the saved title is listed.
    await page
      .getByRole("button", { name: tEn("meetings.backToList") })
      .first()
      .click();
    await expect(page.getByText("Budget Review Sync")).toBeVisible();

    // The rename crossed the IPC bridge with the right payload.
    const snapshot = await readBridge(page);
    expect(
      snapshot.invocations.some(
        (i) =>
          i.command === "meeting_rename_session" &&
          i.args.title === "Budget Review Sync",
      ),
    ).toBe(true);

    // Search filters the list.
    await page.getByLabel(tEn("meetings.searchPlaceholder")).fill("budget");
    await page.getByRole("button", { name: tEn("meetings.search") }).click();
    await expect(page.getByText("Budget Review Sync")).toBeVisible();

    // Re-open via the list card.
    await page.getByRole("button", { name: /Budget Review Sync/ }).click();
    await expect(
      page.getByRole("textbox", { name: tEn("meetings.titleInputLabel") }),
    ).toHaveValue("Budget Review Sync");
  });

  test("a no-match search shows a no-results message, never the empty library", async ({
    page,
  }) => {
    await openMeetings(page);
    await page
      .getByRole("button", { name: tEn("meetings.newMeeting") })
      .click();
    const titleInput = page.getByLabel(tEn("meetings.titleInputLabel"));
    await titleInput.fill("Budget Review Sync");
    await page.getByRole("button", { name: tEn("meetings.saveTitle") }).click();
    await page
      .getByRole("button", { name: tEn("meetings.backToList") })
      .first()
      .click();
    await expect(page.getByText("Budget Review Sync")).toBeVisible();

    // Regression: a search matching nothing used to fall through to the
    // first-run empty library, hiding existing meetings behind the
    // "New Meeting" hero. It must show the no-results feedback instead.
    await page
      .getByLabel(tEn("meetings.searchPlaceholder"))
      .fill("zzz-no-match");
    await page.getByRole("button", { name: tEn("meetings.search") }).click();
    await expect(page.getByText(tEn("common.noOptionsFound"))).toBeVisible();
    // The first-run empty-state hero must not appear: it would present the
    // library as empty even though meetings exist.
    await expect(page.getByText(tEn("meetings.privacyLocal"))).toBeHidden();
    await expect(page.getByText(tEn("meetings.emptyBenefit"))).toBeHidden();

    // Clearing the query restores the unfiltered list.
    await page.getByLabel(tEn("meetings.searchPlaceholder")).fill("");
    await page.getByRole("button", { name: tEn("meetings.search") }).click();
    await expect(page.getByText("Budget Review Sync")).toBeVisible();
  });

  test("delete removes the meeting from the list after confirmation", async ({
    page,
  }) => {
    await openMeetings(page);
    await page
      .getByRole("button", { name: tEn("meetings.newMeeting") })
      .click();
    const titleInput = page.getByLabel(tEn("meetings.titleInputLabel"));
    await titleInput.fill("Doomed Meeting");
    await page.getByRole("button", { name: tEn("meetings.saveTitle") }).click();

    // Leave the start dialog and reopen the meeting from the library: the
    // destructive action lives in the detail view, not in the start flow.
    await page
      .getByRole("button", { name: tEn("meetings.backToList") })
      .first()
      .click();
    await page.getByRole("button", { name: /Doomed Meeting/ }).click();

    // Delete behind the explicit confirmation dialog.
    await page
      .getByRole("button", { name: tEn("common.delete") })
      .first()
      .click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: tEn("common.delete") }).click();

    // Back on the empty library.
    await expect(
      page.getByRole("button", { name: tEn("meetings.newMeeting") }),
    ).toBeVisible();

    const snapshot = await readBridge(page);
    expect(
      snapshot.invocations.some((i) => i.command === "meeting_delete_session"),
    ).toBe(true);
  });
});
