import type { SettingsSectionId } from "./settingsSections";

/**
 * Deep-link support for the consolidated Einstellungen page (plan §6.3).
 *
 * The app has no router; the only addressable state is the URL hash of the
 * main window. New links use `#/settings/<tabId>`. Links that predate the
 * consolidation used the old sidebar section ids as the last hash segment
 * (`#general`, `#/models`, ...). Both forms are accepted so that old entry
 * points keep working:
 *
 *   legacy section      new tab
 *   ------------------  --------------------------------
 *   general             dictate       (dictation settings)
 *   postprocessing      dictate
 *   speechPipeline      modelsProviders
 *   models              modelsProviders
 *   advanced            advanced
 *   debug               debug
 *   dictate             dictate
 *   meetings            meetings
 *   assistants          assistants
 */

const SECTION_ALIASES: Record<string, SettingsSectionId> = {
  // canonical tab ids ("app" is the Allgemein tab)
  dictate: "dictate",
  meetings: "meetings",
  assistants: "assistants",
  modelsproviders: "modelsProviders",
  app: "app",
  advanced: "advanced",
  debug: "debug",
  // legacy sidebar section ids
  speechpipeline: "modelsProviders",
  models: "modelsProviders",
  providers: "modelsProviders",
  postprocessing: "dictate",
  // pre-consolidation "General" held the dictation controls
  general: "dictate",
};

/**
 * Resolves a location hash to the settings tab it targets, or null when the
 * hash does not address the settings page. Comparison is case-insensitive;
 * accepted shapes are `#<section>`, `#/<section>` and `#/settings/<section>`.
 */
export const settingsSectionFromHash = (
  hash: string,
): SettingsSectionId | null => {
  const match = /^#\/?(?:settings\/)?([A-Za-z_-]+)$/.exec(hash.trim());
  if (!match) return null;
  return SECTION_ALIASES[match[1].toLowerCase()] ?? null;
};

/** True when the given hash should open the app on the settings view. */
export const hashTargetsSettingsView = (hash: string): boolean =>
  settingsSectionFromHash(hash) !== null;

/** Canonical hash for a settings tab, written back on every tab change. */
export const buildSettingsHash = (section: SettingsSectionId): string =>
  `#/settings/${section}`;
