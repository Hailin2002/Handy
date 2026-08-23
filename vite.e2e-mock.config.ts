import { defineConfig, mergeConfig, type Plugin } from "vite";
import baseConfig from "./vite.config";

const MAIN_ENTRY_TAG = '<script type="module" src="/src/main.tsx"></script>';
const MOCK_ENTRY_TAG =
  '<script type="module" src="/src/mocks/install.ts"></script>';

/**
 * Build-only plugin that prepends the Tauri IPC mock module to `index.html`.
 *
 * The Playwright suite normally injects the mock by rewriting the HTML served
 * by the dev server (see `tests/helpers/tauri-mock.ts`, APP_ENTRY_MARKER).
 * A production bundle has no `/src/main.tsx` tag left to rewrite, so this
 * plugin performs the equivalent injection at build time: Vite bundles
 * `/src/mocks/install.ts` into its own chunk and keeps document order, so the
 * mock installs `window.__TAURI_INTERNALS__` before the app entry evaluates.
 */
const speakE2eMockEntry = (): Plugin => ({
  name: "speak-e2e-mock-entry",
  apply: "build",
  transformIndexHtml: {
    // `pre` runs before Vite's own HTML transformation, so the injected tag
    // is picked up as a regular bundled entry instead of being emitted raw.
    order: "pre",
    handler: (html) =>
      html.replace(MAIN_ENTRY_TAG, `${MOCK_ENTRY_TAG}\n    ${MAIN_ENTRY_TAG}`),
  },
});

// Identical to `vite build`, plus the E2E mock entry injection.
export default defineConfig(async () =>
  mergeConfig(await baseConfig(), {
    plugins: [speakE2eMockEntry()],
  }),
);
