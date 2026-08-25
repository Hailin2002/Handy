import { createMockRuntime } from "./runtime";

/**
 * Browser entry point for the Tauri IPC mock.
 *
 * A Playwright test injects this module as the first module script of
 * `index.html` (see `tests/helpers/tauri-mock.ts`) so it evaluates before
 * `src/main.tsx`. Installing `window.__TAURI_INTERNALS__` here means every
 * later `@tauri-apps/api` call — commands, events and plugins — resolves
 * against the deterministic in-memory backend instead of a Rust process.
 */

const runtime = createMockRuntime(window.__SPEAK_E2E_MOCK_CONFIG__ ?? {});

window.__TAURI_INTERNALS__ = runtime.internals;
// `@tauri-apps/plugin-os` reads its static values from this synchronous
// global instead of invoking a command (see its dist-js/index.js), so the
// mock must provide it or every platform() call throws before render.
window.__TAURI_OS_PLUGIN_INTERNALS__ = {
  eol: "\n",
  platform: "linux",
  version: "6.6.0",
  family: "unix",
  os_type: "linux",
  arch: "x86_64",
  exe_extension: "",
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: () => undefined,
};
window.__SPEAK_E2E_MOCK_BRIDGE__ = runtime.bridge;
window.isTauri = true;
