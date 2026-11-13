/**
 * Render + wiring assertions for the Phase A4 audio-ducking settings block
 * (general/RecordingAudioSettings.tsx).
 *
 * Runs in the repo's node-environment vitest setup via react-dom/server,
 * initialized with the REAL en translation file so the i18n wiring is under
 * test too. There is no DOM and no @testing-library in this project, so the
 * two halves of the feature are asserted where each is observable:
 *
 * - render: the ducking slider is mounted ONLY in `duck` mode, and each mode
 *   renders its own translated label with no raw i18n key leaking;
 * - wiring: writing each settings key the component writes reaches the
 *   matching generated Tauri command, which is the part a typo in the
 *   settingsStore updater map would silently break.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";

// The Tauri IPC bridge does not exist outside the webview.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@/bindings", () => ({
  commands: {
    changeRecordingAudioModeSetting: vi.fn(),
    changeDuckVolumePercentSetting: vi.fn(),
    changeDuckFadeMsSetting: vi.fn(),
    changeMuteWhileRecordingSetting: vi.fn(),
  },
}));

import type { AppSettings, RecordingAudioMode } from "@/bindings";
import { commands } from "@/bindings";
import enTranslation from "@/i18n/locales/en/translation.json";
import { useSettingsStore } from "@/stores/settingsStore";
import { RecordingAudioSettings } from "../RecordingAudioSettings";

const initI18n = async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: "en",
      fallbackLng: "en",
      resources: { en: { translation: enTranslation } },
    });
  }
};

/** Seed the module-level zustand singleton so render sees a settled store. */
const seedStore = (settings: Partial<AppSettings>) => {
  useSettingsStore.setState({
    settings: settings as AppSettings,
    defaultSettings: null,
    isLoading: false,
    isUpdating: {},
  });
};

const renderAt = async (settings: Partial<AppSettings>) => {
  await initI18n();
  seedStore(settings);
  return renderToStaticMarkup(React.createElement(RecordingAudioSettings));
};

/** react-dom/server emits the range input's attributes verbatim. */
const SLIDER = 'type="range"';

beforeEach(() => {
  vi.clearAllMocks();
});

describe("RecordingAudioSettings render", () => {
  it("shows the three-way choice and no slider in `off` mode", async () => {
    const html = await renderAt({ recording_audio_mode: "off" });

    expect(html).toContain("Audio While Recording");
    expect(html).toContain("Do nothing");
    expect(html).not.toContain(SLIDER);
    expect(html).not.toContain("Ducking Volume");
  });

  it("mounts the ducking slider ONLY in `duck` mode, with its percent value", async () => {
    const html = await renderAt({
      recording_audio_mode: "duck",
      duck_volume_percent: 35,
    });

    expect(html).toContain("Lower volume");
    expect(html).toContain("Ducking Volume");
    expect(html).toContain(SLIDER);
    // Bounds and granularity come from the u8 the backend clamps to 0..=100.
    expect(html).toContain('min="0"');
    expect(html).toContain('max="100"');
    expect(html).toContain('step="5"');
    // The value is rendered through the translated unit, not a hardcoded "%".
    expect(html).toContain("35%");
  });

  it("shows no slider in `mute` mode", async () => {
    const html = await renderAt({ recording_audio_mode: "mute" });

    expect(html).toContain("Mute completely");
    expect(html).not.toContain(SLIDER);
  });

  it("falls back to `off` when the backend has not supplied a mode yet", async () => {
    const html = await renderAt({});

    expect(html).toContain("Do nothing");
    expect(html).not.toContain(SLIDER);
  });

  it("leaks no raw i18n keys", async () => {
    const html = await renderAt({
      recording_audio_mode: "duck",
      duck_volume_percent: 20,
    });

    expect(html).not.toContain("settings.general.recordingAudio");
  });
});

describe("RecordingAudioSettings settings wiring", () => {
  const ok = { status: "ok", data: null } as const;

  beforeEach(() => {
    seedStore({ recording_audio_mode: "off", duck_volume_percent: 20 });
  });

  it("routes a mode switch to change_recording_audio_mode_setting", async () => {
    vi.mocked(commands.changeRecordingAudioModeSetting).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .updateSetting("recording_audio_mode", "duck" as RecordingAudioMode);

    expect(commands.changeRecordingAudioModeSetting).toHaveBeenCalledWith(
      "duck",
    );
    expect(useSettingsStore.getState().settings?.recording_audio_mode).toBe(
      "duck",
    );
    // The legacy bool is the backend's mirror to maintain, never the UI's.
    expect(commands.changeMuteWhileRecordingSetting).not.toHaveBeenCalled();
  });

  it("routes the ducking volume to change_duck_volume_percent_setting", async () => {
    vi.mocked(commands.changeDuckVolumePercentSetting).mockResolvedValue(ok);

    await useSettingsStore.getState().updateSetting("duck_volume_percent", 35);

    expect(commands.changeDuckVolumePercentSetting).toHaveBeenCalledWith(35);
  });

  it("routes the fade duration to change_duck_fade_ms_setting", async () => {
    vi.mocked(commands.changeDuckFadeMsSetting).mockResolvedValue(ok);

    await useSettingsStore.getState().updateSetting("duck_fade_ms", 200);

    expect(commands.changeDuckFadeMsSetting).toHaveBeenCalledWith(200);
  });

  it("rolls the mode back when the backend rejects it", async () => {
    vi.mocked(commands.changeRecordingAudioModeSetting).mockResolvedValue({
      status: "error",
      error: "invalid mode",
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    await useSettingsStore
      .getState()
      .updateSetting("recording_audio_mode", "duck" as RecordingAudioMode);

    expect(useSettingsStore.getState().settings?.recording_audio_mode).toBe(
      "off",
    );
    errorSpy.mockRestore();
  });
});
