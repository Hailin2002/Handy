import { defineConfig } from "vitest/config";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";

const dirName = dirname(fileURLToPath(import.meta.url));

// Unit-test-only config. Kept separate from vite.config.ts so the app build
// never picks up test files and test runs stay deterministic (no watch mode:
// `vitest run` is invoked by the `test:unit` script).
export default defineConfig({
  resolve: {
    alias: {
      "@": resolve(dirName, "./src"),
      "@/bindings": resolve(dirName, "./src/bindings.ts"),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
    // The Tauri IPC bridge does not exist outside the webview; store tests must
    // mock `@/bindings` and `@tauri-apps/api/event`. Fail fast if a test hits
    // the real module graph unexpectedly deep.
    restoreMocks: true,
  },
});
