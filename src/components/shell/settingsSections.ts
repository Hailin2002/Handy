import React from "react";
import {
  AudioLines,
  Cog,
  Cpu,
  FlaskConical,
  Mic2,
  Sparkles,
} from "lucide-react";
import {
  AdvancedSettings,
  DebugSettings,
  GeneralSettings,
  ModelsSettings,
  PostProcessingSettings,
  SpeechPipelineSettings,
} from "../settings";

/**
 * Settings sections, shown as secondary tabs on the Einstellungen page
 * (plan §6.3) instead of as primary navigation entries. History moved to the
 * top-level "Diktatverlauf" entry and About to "Hilfe & Über".
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
  general: {
    labelKey: "sidebar.general",
    icon: Mic2,
    component: GeneralSettings,
    enabled: () => true,
  },
  speechPipeline: {
    labelKey: "settings.speechPipeline.title",
    icon: AudioLines,
    component: SpeechPipelineSettings,
    enabled: () => true,
  },
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: Cog,
    component: AdvancedSettings,
    enabled: () => true,
  },
  postprocessing: {
    labelKey: "sidebar.postProcessing",
    icon: Sparkles,
    component: PostProcessingSettings,
    enabled: (settings) => settings?.post_process_enabled ?? false,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
} as const satisfies Record<string, SettingsSectionConfig>;

export type SettingsSectionId = keyof typeof SECTIONS_CONFIG;

/** Tab order on the Einstellungen page. */
export const SETTINGS_TAB_ORDER: readonly SettingsSectionId[] = [
  "general",
  "speechPipeline",
  "models",
  "advanced",
  "postprocessing",
  "debug",
];
