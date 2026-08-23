import { test, expect, type Page } from "@playwright/test";
import { installTauriMock, tEn } from "./helpers/tauri-mock";

/**
 * Dictation-first shell regression (plan §6.2–§6.4), guarding the dictation
 * workflows through the shell rework:
 *
 * 1. DictationHome renders status, shortcut and the safe in-app test preview.
 * 2. No meeting control leaks onto dictation surfaces (plan §6.4: dictation
 *    and meetings are two products, never two modes of one button).
 * 3. The product sidebar is fully keyboard-operable.
 * 4. History deletion always requires explicit confirmation and only the
 *    confirmed "Delete permanently" action crosses the IPC bridge.
 *
 * Every spec runs against the Vite dev server with the Tauri IPC bridge fully
 * stubbed (`installTauriMock` → `src/mocks`); no Rust backend, no OS side
 * effects. Expected strings are derived from the i18n catalogs via
 * `tEn`, never hardcoded.
 */

/**
 * Display form of the transcribe shortcut from the mock fixture
 * (`src/mocks/fixtures.ts`: `CommandOrControl+Shift+Space`) as rendered by
 * `formatKeyCombination` (each `+` part capitalized, joined with " + ").
 */
const MOCK_SHORTCUT_TEXT = "CommandOrControl + Shift + Space";

/** Fixture history entry ids used by the deletion specs (see `stubHistory`). */
const HISTORY_ENTRY_A = 101;
const HISTORY_ENTRY_B = 102;

/** Test-side log of ids that reached the mocked delete command. */
declare global {
  interface Window {
    __HISTORY_E2E__?: { deleted: number[] };
  }
}

test.describe("Dictation shell regression (mocked Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
    // Onboarding completed in the mock settings → the app lands on the
    // dictation home ("Diktieren") view by default. Exact match: several
    // nested headings also contain the word "Dictation".
    await expect(
      page.getByRole("heading", { name: tEn("dictation.title"), exact: true }),
    ).toBeVisible();
  });

  test("dictation home renders status, shortcut and the safe test preview", async ({
    page,
  }) => {
    // Status group shows the honest backend-driven phase as a live region.
    const status = page.getByRole("status");
    await expect(status).toBeVisible();
    await expect(status).toHaveText(tEn("dictation.status.ready"));

    // Shortcut keycap: a clickable button whose accessible name carries both
    // the binding and its destination (open dictation settings).
    const expectedShortcutAria = tEn("dictation.shortcut.openAria").replace(
      "{{shortcut}}",
      MOCK_SHORTCUT_TEXT,
    );
    const shortcutButton = page.getByRole("button", {
      name: expectedShortcutAria,
    });
    await expect(shortcutButton).toBeVisible();
    await expect(shortcutButton).toHaveText(MOCK_SHORTCUT_TEXT);

    // Safe in-app dictation test: present with its privacy promise, but no
    // preview exists until the user records one themselves.
    await expect(
      page.getByRole("heading", {
        name: tEn("dictation.test.title"),
        exact: true,
        level: 2,
      }),
    ).toBeVisible();
    await expect(
      page.getByText(tEn("dictation.test.privacy")).first(),
    ).toBeVisible();

    const previewRegion = page.getByRole("region", {
      name: tEn("dictation.test.preview.label"),
    });
    await expect(previewRegion).toHaveCount(0);

    // SAFETY: rendering the dictation home must not cross any clipboard or
    // paste/output command over the IPC bridge — the test recording can only
    // ever surface inside this window.
    const offenders = await page.evaluate(() => {
      const bridge = window.__SPEAK_E2E_MOCK_BRIDGE__;
      if (!bridge) throw new Error("Tauri mock bridge missing");
      return bridge
        .getInvocations()
        .filter((invocation) => /paste|clipboard/i.test(invocation.command))
        .map((invocation) => invocation.command);
    });
    expect(offenders).toEqual([]);
  });

  test("no meeting controls leak onto dictation surfaces", async ({ page }) => {
    // The pinned active-meeting nav slot stays hidden while meeting mode does
    // not exist (no `Recording · mm:ss` entry anywhere).
    await expect(page.getByText(tEn("nav.activeMeeting"))).toHaveCount(0);

    // Every interactive element mentioning meetings/recording on the
    // dictation home must live inside the sidebar <nav> (the plain navigation
    // entry). Anything else would be a leaked meeting start control (G2).
    const leakCheck = async () => {
      const offenders = await page.evaluate(() => {
        const pattern = /meeting|record|aufnahme/i;
        return [...document.querySelectorAll("button")]
          .filter((button) => {
            const label =
              (button.textContent ?? "") +
              (button.getAttribute("aria-label") ?? "");
            return pattern.test(label) && !button.closest("nav");
          })
          .map((button) => button.textContent?.trim() ?? "");
      });
      expect(offenders).toEqual([]);
    };
    await leakCheck();

    // Even the Meetings destination is a placeholder without any control:
    // navigating there must never reveal a start/record affordance either.
    await page
      .getByRole("navigation")
      .getByRole("button", { name: tEn("nav.meetings") })
      .click();
    await expect(page.getByText(tEn("nav.meetingsEmptyBody"))).toBeVisible();
    await leakCheck();
  });

  test("sidebar product navigation works from the keyboard alone", async ({
    page,
  }) => {
    const nav = page.getByRole("navigation");
    const dictate = nav.getByRole("button", { name: tEn("nav.dictate") });

    await dictate.focus();
    await expect(dictate).toBeFocused();
    await expect(dictate).toHaveAttribute("aria-current", "page");

    // Tab walks every primary and secondary entry in DOM order; each stop is
    // a real focusable <button> with a visible-focus contract.
    for (const key of [
      "nav.meetings",
      "nav.projects",
      "nav.history",
      "nav.settings",
      "nav.about",
    ]) {
      const entry = nav.getByRole("button", { name: tEn(key) });
      await page.keyboard.press("Tab");
      await expect(entry).toBeFocused();
    }

    // Enter activates: Help & About opens the about content.
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("heading", { name: tEn("settings.about.title") }),
    ).toBeVisible();

    // Shift+Tab back to Settings and activate it; the view switches and the
    // aria-current marker follows the keyboard.
    const settings = nav.getByRole("button", { name: tEn("nav.settings") });
    await page.keyboard.press("Shift+Tab");
    await expect(settings).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(settings).toHaveAttribute("aria-current", "page");
    await expect(
      page.getByRole("heading", { name: tEn("settings.general.title") }),
    ).toBeVisible();

    // And back to History via keyboard, which mounts the history view.
    const history = nav.getByRole("button", { name: tEn("nav.history") });
    await history.focus();
    await page.keyboard.press("Enter");
    await expect(history).toHaveAttribute("aria-current", "page");
    await expect(page.getByText(tEn("settings.history.empty"))).toBeVisible();
  });

  test("history deletion requires explicit confirmation before IPC", async ({
    page,
  }) => {
    await stubHistory(page);
    await page
      .getByRole("navigation")
      .getByRole("button", { name: tEn("nav.history") })
      .click();

    const deleteButtons = page.locator("[data-history-delete]");
    await expect(deleteButtons).toHaveCount(2);

    const dialog = page.getByRole("dialog", {
      name: tEn("settings.history.deleteConfirmTitle"),
    });

    // Opening the dialog stages the delete only; nothing crossed IPC yet (G1).
    await firstDeleteButton(page, HISTORY_ENTRY_A).click();
    await expect(dialog).toBeVisible();
    expect(await deletedIds(page)).toEqual([]);

    // Escape dismisses without deleting; the entry survives.
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(deleteButtons).toHaveCount(2);
    expect(await deletedIds(page)).toEqual([]);

    // Cancel keeps the entry intact as well. The dialog's close icon button
    // shares the accessible name, so match the footer button by its text.
    await firstDeleteButton(page, HISTORY_ENTRY_A).click();
    await expect(dialog).toBeVisible();
    await dialog
      .getByRole("button")
      .filter({ hasText: tEn("settings.history.deleteCancel") })
      .click();
    await expect(dialog).toBeHidden();
    expect(await deletedIds(page)).toEqual([]);

    // Only the explicit destructive action deletes exactly the staged entry.
    await firstDeleteButton(page, HISTORY_ENTRY_A).click();
    await expect(dialog).toBeVisible();
    await dialog
      .getByRole("button")
      .filter({ hasText: tEn("settings.history.deleteConfirmAction") })
      .click();
    await expect(dialog).toBeHidden();
    await expect(
      page.locator(`[data-history-delete="${HISTORY_ENTRY_A}"]`),
    ).toHaveCount(0);
    await expect(
      page.locator(`[data-history-delete="${HISTORY_ENTRY_B}"]`),
    ).toHaveCount(1);
    expect(await deletedIds(page)).toEqual([HISTORY_ENTRY_A]);
  });
});

/* -------------------------------------------------------------------------- */
/* helpers                                                                    */
/* -------------------------------------------------------------------------- */

const firstDeleteButton = (page: Page, id: number) =>
  page.locator(`[data-history-delete="${id}"]`);

/** Reads the ids that reached the mocked backend's delete command. */
const deletedIds = (page: Page): Promise<number[]> =>
  page.evaluate(() => {
    const log = window.__HISTORY_E2E__;
    if (!log) throw new Error("history stub not installed");
    return [...log.deleted];
  });

/**
 * Per-test override of the stubbed IPC bridge serving two deterministic
 * history entries and recording `delete_history_entry` invocations. This runs
 * entirely inside the browser against the already-installed Tauri mock; the
 * real Rust backend is never contacted (G4).
 */
const stubHistory = async (page: Page): Promise<void> => {
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    if (!internals) throw new Error("Tauri mock internals missing");
    const originalInvoke = internals.invoke.bind(internals);
    const log = (window.__HISTORY_E2E__ ??= { deleted: [] as number[] });
    internals.invoke = async (cmd, args) => {
      if (cmd === "get_history_entries") {
        return {
          entries: [
            {
              id: 101,
              file_name: "e2e-a.wav",
              timestamp: 1767225600,
              saved: false,
              title: "",
              transcription_text: "First mock transcript",
              post_processed_text: null,
              post_process_prompt: null,
              post_process_requested: false,
            },
            {
              id: 102,
              file_name: "e2e-b.wav",
              timestamp: 1767229200,
              saved: true,
              title: "",
              transcription_text: "Second mock transcript",
              post_processed_text: null,
              post_process_prompt: null,
              post_process_requested: false,
            },
          ],
          has_more: false,
        };
      }
      if (cmd === "delete_history_entry") {
        const id = (args as { id?: unknown })?.id;
        if (typeof id === "number") log.deleted.push(id);
        return null;
      }
      return originalInvoke(cmd, args);
    };
  });
};
