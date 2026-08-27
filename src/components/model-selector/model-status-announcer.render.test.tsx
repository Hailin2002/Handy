// Render assertions for the model selector's polite status live region.
// Runs under vitest (`npx vitest run`).
//
// Static-markup checks only (effects don't run under SSR). The hook's
// once-per-transition settle timing is verified against the real hook with a
// throwaway Chromium harness driven by the repo's Playwright dependency; see
// the D5 completion notes for that session's transcript.
import { describe, it, expect, beforeAll } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import ModelSelector from "./ModelSelector";
import { getStatusAnnouncementText } from "./useModelStatusAnnouncer";

let html: string;

beforeAll(async () => {
  await i18next.use(initReactI18next).init({
    lng: "en",
    resources: {
      en: {
        translation: {
          modelSelector: {
            verifyingGeneric: "Verifying...",
            verifying: "Verifying {{modelName}}...",
            extractingGeneric: "Extracting...",
            extracting: "Extracting {{modelName}}...",
            extractingMultiple: "Extracting {{count}} models...",
            downloading: "Downloading... {{percentage}}%",
            downloadingMultiple: "Downloading {{count}} models...",
            modelReady: "Model ready",
            loading: "Loading {{modelName}}...",
            loadingGeneric: "Loading model...",
            modelError: "Model error",
            modelUnloaded: "Model unloaded",
            noModelDownloadRequired: "No model selected",
          },
        },
      },
    },
  });
  html = renderToStaticMarkup(<ModelSelector />);
});

describe("ModelSelector polite live region", () => {
  it("renders exactly one visually hidden aria-live=polite region", () => {
    const politeCount = (html.match(/aria-live="polite"/g) || []).length;
    expect(politeCount).toBe(1);
    expect(html).toMatch(
      /class="sr-only" aria-live="polite" aria-atomic="true"/,
    );
  });

  it("carries no content before any transition settles", () => {
    expect(html).toMatch(/aria-atomic="true"><\/div>/);
  });
});

// --- Announcement-text purity: volatile numerals never reach the region ----

const echoT = (key: string, options?: Record<string, unknown>) =>
  `${key}:${JSON.stringify(options ?? {})}`;

const base = {
  downloadCount: 0,
  displayText: "Whisper Small",
  t: echoT,
};

describe("getStatusAnnouncementText", () => {
  it("collapses download progress ticks to one constant message", () => {
    expect(
      getStatusAnnouncementText({
        ...base,
        displayStatus: "downloading",
        downloadCount: 1,
        displayText: "Downloading... 29%",
      }),
    ).toBe("modelSelector.downloadingGeneric:{}");
    // different percentages must map to the same announcement text
    expect(
      getStatusAnnouncementText({
        ...base,
        displayStatus: "downloading",
        downloadCount: 1,
        displayText: "Downloading... 83%",
      }),
    ).toBe("modelSelector.downloadingGeneric:{}");
  });

  it("announces the model count for multi-model downloads", () => {
    expect(
      getStatusAnnouncementText({
        ...base,
        displayStatus: "downloading",
        downloadCount: 3,
        displayText: "Downloading 3 models...",
      }),
    ).toBe('modelSelector.downloadingMultiple:{"count":3}');
  });

  it.each([
    "ready",
    "loading",
    "verifying",
    "extracting",
    "error",
    "unloaded",
    "none",
  ])("reuses the (churn-free) display text for status %s", (status) => {
    expect(getStatusAnnouncementText({ ...base, displayStatus: status })).toBe(
      "Whisper Small",
    );
  });
});
