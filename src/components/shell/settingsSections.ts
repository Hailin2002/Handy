import type React from "react";
import {
  AudioLines,
  Mic,
  Settings,
  SlidersHorizontal,
  Cpu,
  FlaskConical,
} from "lucide-react";
import { AdvancedSettings, DebugSettings } from "../settings";
import {
  AppGeneralTab,
  DictationSettingsTab,
  MeetingsSettingsTab,
  ModelsAndProvidersTab,
} from "../settings/tabs";

/**
 * Secondary tabs of the consolidated Einstellungen page (plan §6.3):
 *
 *   Diktieren | Meetings | Modelle & Provider | Allgemein | Erweitert
 *
 * plus Debug, which stays visible only while debug mode is enabled
 * (Ctrl+Shift+D), exactly as before the consolidation.
 *
 * Mapping of the former primary-navigation sections:
 *   - general        → Diktieren (dictation controls) / Allgemein (app prefs)
 *   - postprocessing → Diktieren (visible while post-processing is enabled)
 *   - speechPipeline → Modelle & Provider
 *   - models         → Modelle & Provider
 *   - advanced       → Erweitert; app-wide toggles moved to Allgemein
 *   - debug          → Debug (unchanged)
 * History and About are top-level views ("Diktatverlauf", "Hilfe & Über")
 * and are not settings tabs.
 */
export interface SettingsSectionConfig {
  labelKey: string;
  icon: React.ComponentType<{
    width?: number | string;
    height?: number | string;
    className?: string;
  }>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
}

export const SECTIONS_CONFIG = {
  dictate: {
    labelKey: "nav.dictate",
    icon: Mic,
    component: DictationSettingsTab,
    enabled: () => true,
  },
  meetings: {
    labelKey: "nav.meetings",
    icon: AudioLines,
    component: MeetingsSettingsTab,
    enabled: () => true,
  },
  modelsProviders: {
    labelKey: "settings.tabs.modelsProviders",
    icon: Cpu,
    component: ModelsAndProvidersTab,
    enabled: () => true,
  },
  app: {
    labelKey: "sidebar.general",
    icon: Settings,
    component: AppGeneralTab,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: SlidersHorizontal,
    component: AdvancedSettings,
    enabled: () => true,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
} as const satisfies Record<string, SettingsSectionConfig>;

export type SettingsSectionId = keyof typeof SECTIONS_CONFIG;

/** Tab order on the Einstellungen page (plan §6.3). */
export const SETTINGS_TAB_ORDER: readonly SettingsSectionId[] = [
  "dictate",
  "meetings",
  "modelsProviders",
  "app",
  "advanced",
  "debug",
];
