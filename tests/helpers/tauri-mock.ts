import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { Page } from "@playwright/test";
import type { AppSettings } from "../../src/bindings";
import type {
  MockInvocation,
  SpeakMockBridge,
  SpeakMockConfig,
} from "../../src/mocks/types";

/**
 * Playwright-side installer for the Tauri IPC mock layer (`src/mocks`).
 *
 * The app is served unchanged by the Vite dev server; the mock is injected by
 * rewriting the document HTML so `src/mocks/install.ts` executes as the first
 * module script, before `src/main.tsx`. That lets every test run the real UI
 * against deterministic in-memory fixtures instead of a Rust backend.
 */

/**
 * Matches the app entry script as served by the Vite dev server. Vite adds a
 * `?t=<timestamp>` query to this tag whenever the module graph was
 * invalidated (e.g. after an edit), so the match must tolerate any query.
 */
const APP_ENTRY_MARKER =
  /<script type="module" src="\/src\/main\.tsx(\?[^"]*)?"?><\/script>/;

const MOCK_ENTRY_TAG =
  '<script type="module" src="/src/mocks/install.ts"></script>';

/**
 * Installs the mock for every navigation of `page`.
 *
 * @param page Playwright page to arm.
 * @param config Per-test overrides merged on top of the default fixtures.
 */
export const installTauriMock = async (
  page: Page,
  config: SpeakMockConfig = {},
): Promise<void> => {
  // Runs before any document script, so install.ts can read it.
  await page.addInitScript((cfg) => {
    window.__SPEAK_E2E_MOCK_CONFIG__ = cfg;
  }, config);

  await page.route("**/*", async (route) => {
    if (route.request().resourceType() !== "document") {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    const html = await response.text();
    if (!APP_ENTRY_MARKER.test(html)) {
      await route.fulfill({ response });
      return;
    }
    await route.fulfill({
      response,
      contentType: "text/html; charset=utf-8",
      body: html.replace(
        APP_ENTRY_MARKER,
        (entry) => `${MOCK_ENTRY_TAG}${entry}`,
      ),
    });
  });
};

/** Snapshot of the mocked backend state exposed through `window`. */
export interface MockBridgeSnapshot {
  settings: AppSettings;
  invocations: readonly MockInvocation[];
}

/**
 * Reads mock-backend state from the page. The bridge handle itself cannot be
 * handed out of `page.evaluate` (its functions would be stripped during
 * structured clone), so every read runs inside the page context.
 */
const ensureBridgeInstalled = async (page: Page): Promise<void> => {
  const installed = await page.evaluate(
    () => !!window.__SPEAK_E2E_MOCK_BRIDGE__,
  );
  if (!installed) {
    throw new Error("Tauri mock bridge is not installed on this page");
  }
};

export const readBridge = async (page: Page): Promise<MockBridgeSnapshot> => {
  await ensureBridgeInstalled(page);
  return page.evaluate(() => {
    const bridge = window.__SPEAK_E2E_MOCK_BRIDGE__;
    if (!bridge) {
      throw new Error("Tauri mock bridge is not installed on this page");
    }
    return {
      settings: bridge.getSettings(),
      invocations: [...bridge.getInvocations()],
    };
  });
};

export const getBridgeSetting = async <K extends keyof AppSettings>(
  page: Page,
  key: K,
): Promise<AppSettings[K]> => {
  await ensureBridgeInstalled(page);
  return page.evaluate((k) => {
    const bridge = window.__SPEAK_E2E_MOCK_BRIDGE__;
    if (!bridge) {
      throw new Error("Tauri mock bridge is not installed on this page");
    }
    return bridge.getSettings()[k];
  }, key);
};

/** True when `command` was invoked at least once with exactly these args. */
export const wasInvokedWith = (
  snapshot: MockBridgeSnapshot,
  command: string,
  args: Record<string, unknown>,
): boolean =>
  snapshot.invocations.some(
    (invocation) =>
      invocation.command === command &&
      Object.entries(args).every(
        ([key, value]) => invocation.args[key] === value,
      ),
  );

/* ---------------------------------------------------------------------- */
/* i18n helpers: expected user-visible strings are always derived from the */
/* translation catalogs via keys, never hardcoded in the specs.           */
/* ---------------------------------------------------------------------- */

type TranslationDict = Record<string, unknown>;

const loadLocaleDict = (locale: string): TranslationDict => {
  const localesDir = fileURLToPath(
    new URL("../../src/i18n/locales", import.meta.url),
  );
  const raw = readFileSync(`${localesDir}/${locale}/translation.json`, "utf-8");
  return JSON.parse(raw) as TranslationDict;
};

const EN_DICT = loadLocaleDict("en");
const DE_DICT = loadLocaleDict("de");

/**
 * Resolves an i18n dot-key against the catalog of `locale`.
 *
 * Segment boundaries are ambiguous — model ids such as
 * `onboarding.models.parakeet-tdt-0.6b-v3.name` contain dots of their own — so
 * resolution greedily backtracks over every possible split instead of assuming
 * one dot == one nesting level.
 */
const resolveKey = (dict: TranslationDict, key: string): unknown => {
  const tokens = key.split(".");
  const walk = (node: unknown, index: number): unknown => {
    if (index === tokens.length) return node;
    if (node === null || typeof node !== "object") return undefined;
    const record = node as TranslationDict;
    for (let take = 1; index + take <= tokens.length; take++) {
      const segment = tokens.slice(index, index + take).join(".");
      if (!(segment in record)) continue;
      const deeper = walk(record[segment], index + take);
      if (deeper !== undefined) return deeper;
    }
    return undefined;
  };
  return walk(dict, 0);
};

export const t = (key: string, locale: "en" | "de" = "en"): string => {
  const dict = locale === "de" ? DE_DICT : EN_DICT;
  const resolved = resolveKey(dict, key);
  if (typeof resolved !== "string") {
    throw new Error(`Missing ${locale} translation for key "${key}"`);
  }
  return resolved;
};

export const tEn = (key: string): string => t(key, "en");
export const tDe = (key: string): string => t(key, "de");
