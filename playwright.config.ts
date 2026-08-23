import { defineConfig, devices } from "@playwright/test";

const DEV_BASE_URL = "http://localhost:1420";
const PREVIEW_BASE_URL = "http://localhost:4173";

const DEV_PROJECT = "chromium";
const PROD_PROJECT = "chromium-production-preview";

/**
 * Projects explicitly selected on the CLI (`--project=x`, repeated `-p x`).
 * Used to start only the web servers the selected projects actually need,
 * so the plain dev workflow (`--project=chromium`) never pays for the
 * production build.
 */
const selectedProjects = (() => {
  const argv = process.argv;
  const names: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith("--project=")) {
      names.push(arg.slice("--project=".length));
    } else if (arg === "--project" || arg === "-p") {
      names.push(argv[i + 1] ?? "");
    }
  }
  return names;
})();

const KNOWN_PROJECTS = [DEV_PROJECT, PROD_PROJECT];
// Unknown filters (e.g. globs) may still select any project -> start both.
const selectionIsKnown =
  selectedProjects.length === 0 ||
  selectedProjects.some((name) => KNOWN_PROJECTS.includes(name));

const wantsDevServer =
  !selectionIsKnown || selectedProjects.includes(DEV_PROJECT);
const wantsProdServer =
  !selectionIsKnown || selectedProjects.includes(PROD_PROJECT);

const devServer = {
  command: "npx vite dev",
  url: DEV_BASE_URL,
  reuseExistingServer: !process.env.CI,
  timeout: 30000,
};

/**
 * Serves the production bundle: full typecheck + minified build (with the
 * E2E Tauri mock bundled ahead of the app entry, see
 * `vite.e2e-mock.config.ts`), then `vite preview` on a fixed port.
 */
const prodPreviewServer = {
  command:
    "npx tsc && npx vite build --config vite.e2e-mock.config.ts && npx vite preview --port 4173 --strictPort",
  url: PREVIEW_BASE_URL,
  reuseExistingServer: !process.env.CI,
  timeout: 180000,
};

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL: DEV_BASE_URL,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: DEV_PROJECT,
      use: { ...devices["Desktop Chrome"], baseURL: DEV_BASE_URL },
    },
    {
      name: PROD_PROJECT,
      use: { ...devices["Desktop Chrome"], baseURL: PREVIEW_BASE_URL },
    },
    {
      // Cross-engine smoke: verifies the dev-server surface boots outside
      // Chromium. WebKit is unsupported on Windows and intentionally absent.
      name: "firefox",
      use: { ...devices["Desktop Firefox"] },
    },
  ],
  webServer: [
    ...(wantsDevServer ? [devServer] : []),
    ...(wantsProdServer ? [prodPreviewServer] : []),
  ],
});
