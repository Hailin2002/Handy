import { defineConfig, mergeConfig, type Plugin } from "vite";
import { resolve } from "path";
import baseConfig from "./vite.config";

/**
 * Build-only plugin that guarantees the Tauri IPC mock module evaluates
 * before any application module.
 *
 * The Playwright suite normally injects the mock by rewriting the HTML served
 * by the dev server (see `tests/helpers/tauri-mock.ts`, APP_ENTRY_MARKER).
 * Rewriting the built `index.html` with a second module-script tag is NOT
 * sufficient for that guarantee under `vite build`: Vite folds the extra
 * entry into the main chunk behind the application modules, so boot-time IPC
 * (e.g. `syncLanguageFromSettings`) runs before `window.__TAURI_INTERNALS__`
 * exists and every persisted-preference assertion breaks in the
 * chromium-production-preview project only.
 *
 * Prepending a static import to `src/main.tsx` instead uses ES module
 * semantics directly: imports evaluate depth-first in declaration order, so
 * the mock runtime is installed before `@/i18n` (or anything else) can touch
 * the Tauri bridge.
 */
const speakE2eMockEntry = (): Plugin => ({
  name: "speak-e2e-mock-entry",
  apply: "build",
  enforce: "pre",
  transform(code, id) {
    const normalized = id.replace(/\\/g, "/");
    // `src/i18n/index.ts` performs the earliest boot-time IPC
    // (`syncLanguageFromSettings`) at module-evaluation time, and code
    // sharing can hoist it into a vendor chunk that executes before the
    // main chunk body. Depending on the mock directly therefore anchors
    // ES-module evaluation order (`install.ts` first) no matter how Vite
    // splits chunks. `main.tsx` is covered as well so any future early IPC
    // caller outside the i18n graph stays safe; re-imports are idempotent.
    if (
      normalized.endsWith("/src/main.tsx") ||
      normalized.endsWith("/src/i18n/index.ts")
    ) {
      return `import "/src/mocks/install.ts";\n${code}`;
    }
    return null;
  },
});

// Identical to `vite build`, plus:
//  - the E2E mock entry injected ahead of the app entry (plugin above), and
//  - the browser-only settings harness (`tests/harness/settings.html`) as an
//    additional build input. The dev server serves that file straight from
//    the repository tree, but `vite preview` only serves emitted artifacts —
//    without this input every `tests/settings-page.spec.ts` case in the
//    chromium-production-preview project hits the SPA fallback (the full app)
//    instead of the harness and fails deterministically.
export default defineConfig(async () =>
  mergeConfig(await baseConfig(), {
    plugins: [speakE2eMockEntry()],
    build: {
      rollupOptions: {
        input: {
          harness: resolve(__dirname, "tests/harness/settings.html"),
        },
      },
    },
  }),
);
