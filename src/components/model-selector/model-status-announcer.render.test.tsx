// Standalone render assertions for the model selector's polite status live
// region (no JS unit-test runner in this repo). Run with:
//   node_modules\.bin\esbuild.cmd src/components/model-selector/model-status-announcer.render.test.tsx --bundle --format=cjs --platform=node --outfile=$env:TEMP\model-status-announcer.render.test.cjs --log-level=warning
//   node $env:TEMP\model-status-announcer.render.test.cjs
//
// Static-markup checks only (effects don't run under SSR). The hook's
// once-per-transition settle timing is verified against the real hook with a
// throwaway Chromium harness driven by the repo's Playwright dependency; see
// the D5 completion notes for that session's transcript.
import assert from "node:assert";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import ModelSelector from "./ModelSelector";
import { getStatusAnnouncementText } from "./useModelStatusAnnouncer";

i18next.use(initReactI18next).init({
  lng: "en",
  initImmediate: false,
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

const html = renderToStaticMarkup(<ModelSelector />);

// Exactly one polite live region, and it is visually hidden (screen-reader-only).
const politeCount = (html.match(/aria-live="polite"/g) || []).length;
assert.strictEqual(
  politeCount,
  1,
  `expected exactly one aria-live="polite" region, found ${politeCount}`,
);
assert.match(html, /class="sr-only" aria-live="polite" aria-atomic="true"/);

// Before any transition settles the region carries no content.
assert.match(
  html,
  /aria-atomic="true"><\/div>/,
  "live region should be empty on first render",
);

// --- Announcement-text purity: volatile numerals never reach the region ----

const echoT = (key: string, options?: Record<string, unknown>) =>
  `${key}:${JSON.stringify(options ?? {})}`;

const base = {
  downloadCount: 0,
  displayText: "Whisper Small",
  t: echoT,
};

// Download progress ticks collapse to one constant message regardless of the
// percentage shown on screen.
assert.strictEqual(
  getStatusAnnouncementText({
    ...base,
    displayStatus: "downloading",
    downloadCount: 1,
    displayText: "Downloading... 29%",
  }),
  "modelSelector.downloadingGeneric:{}",
);
assert.strictEqual(
  getStatusAnnouncementText({
    ...base,
    displayStatus: "downloading",
    downloadCount: 1,
    displayText: "Downloading... 83%",
  }),
  "modelSelector.downloadingGeneric:{}",
  "different percentages must map to the same announcement text",
);
assert.strictEqual(
  getStatusAnnouncementText({
    ...base,
    displayStatus: "downloading",
    downloadCount: 3,
    displayText: "Downloading 3 models...",
  }),
  'modelSelector.downloadingMultiple:{"count":3}',
);

// Non-downloading statuses reuse their (already churn-free) display text.
for (const status of [
  "ready",
  "loading",
  "verifying",
  "extracting",
  "error",
  "unloaded",
  "none",
]) {
  assert.strictEqual(
    getStatusAnnouncementText({ ...base, displayStatus: status }),
    "Whisper Small",
    `${status} should announce its display text`,
  );
}

console.log("model-status-announcer.render.test: all assertions passed");
