import React from "react";
import { createRoot } from "react-dom/client";
import "@/i18n";
import { SettingsPage } from "@/components/shell/SettingsPage";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * Browser-only harness for Playwright: mounts the consolidated
 * Einstellungen page without the Tauri shell so tablist semantics,
 * keyboard navigation, deep links and debug visibility can be exercised.
 * The store stays at its unloaded defaults (settings = null), which is the
 * same state as before settings load; tests flip `debug_mode` through the
 * exposed helper below to verify G2.
 */
(window as unknown as Record<string, unknown>).__setDebugMode = (on: boolean) =>
  useSettingsStore.setState((state) => ({
    settings: { ...(state.settings ?? {}), debug_mode: on },
  }));

document.documentElement.lang = "en";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <div className="p-4">
      <SettingsPage />
    </div>
  </React.StrictMode>,
);
