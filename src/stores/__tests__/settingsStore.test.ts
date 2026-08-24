import { beforeEach, describe, expect, it, vi } from "vitest";

// The settings store talks to the Tauri IPC bridge, which only exists inside
// the webview. Both boundary modules are replaced with deterministic fakes so
// the state transitions under test are fully observable.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@/bindings", () => ({
  commands: {
    getAppSettings: vi.fn(),
    getDefaultSettings: vi.fn(),
    getAvailableMicrophones: vi.fn(),
    getAvailableOutputDevices: vi.fn(),
    checkCustomSounds: vi.fn(),
    changeAudioFeedbackSetting: vi.fn(),
  },
}));

import type { AppSettings } from "@/bindings";
import { commands } from "@/bindings";
import { useSettingsStore } from "../settingsStore";

const mockedChangeAudioFeedback = vi.mocked(commands.changeAudioFeedbackSetting);
const mockedGetAppSettings = vi.mocked(commands.getAppSettings);

const makeSettings = (overrides: Partial<AppSettings> = {}): AppSettings => ({
  push_to_talk: true,
  audio_feedback: true,
  ...overrides,
});

/** Reset the module-level zustand singleton between tests. */
const seedStore = (settings: AppSettings | null) => {
  useSettingsStore.setState({
    settings,
    defaultSettings: null,
    isLoading: false,
    isUpdating: {},
  });
};

beforeEach(() => {
  vi.clearAllMocks();
  seedStore(makeSettings());
});

describe("settingsStore.updateSetting", () => {
  it("applies the change optimistically, persists it, and clears the updating flag", async () => {
    mockedChangeAudioFeedback.mockResolvedValue({ status: "ok", data: null });

    const pending = useSettingsStore
      .getState()
      .updateSetting("audio_feedback", false);

    // Optimistic write is visible while the backend command is in flight and
    // the per-key updating guard is raised.
    expect(useSettingsStore.getState().settings?.audio_feedback).toBe(false);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      true,
    );

    await pending;

    expect(mockedChangeAudioFeedback).toHaveBeenCalledTimes(1);
    expect(mockedChangeAudioFeedback).toHaveBeenCalledWith(false);
    const { settings } = useSettingsStore.getState();
    expect(settings?.audio_feedback).toBe(false);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      false,
    );
  });

  it("rejects a value the backend refuses and rolls back to the original", async () => {
    mockedChangeAudioFeedback.mockResolvedValue({
      status: "error",
      error: "invalid value",
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const snapshot = useSettingsStore.getState().settings;

    await useSettingsStore.getState().updateSetting("audio_feedback", false);

    // The optimistic value was applied and then reverted because the backend
    // rejected the write; the updating flag must not stay stuck.
    expect(useSettingsStore.getState().settings).toEqual(snapshot);
    expect(useSettingsStore.getState().settings?.audio_feedback).toBe(true);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      false,
    );
    expect(errorSpy).toHaveBeenCalledWith(
      expect.stringContaining("audio_feedback"),
      expect.any(Error),
    );
  });

  it("keeps state stable across repeated unchanged writes", async () => {
    mockedChangeAudioFeedback.mockResolvedValue({ status: "ok", data: null });
    const before = useSettingsStore.getState().settings;

    // Writing the already-persisted value twice must be a no-op on state while
    // still completing both queued writes cleanly (no rollback, no stuck flag).
    await useSettingsStore.getState().updateSetting("audio_feedback", true);
    expect(useSettingsStore.getState().settings).toEqual(before);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      false,
    );

    await useSettingsStore.getState().updateSetting("audio_feedback", true);

    expect(mockedChangeAudioFeedback).toHaveBeenCalledTimes(2);
    expect(mockedChangeAudioFeedback).toHaveBeenNthCalledWith(1, true);
    expect(mockedChangeAudioFeedback).toHaveBeenNthCalledWith(2, true);
    expect(useSettingsStore.getState().settings).toEqual(before);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      false,
    );
  });
});

describe("settingsStore.resetSetting", () => {
  it("restores the loaded default through the normal update path", async () => {
    mockedChangeAudioFeedback.mockResolvedValue({ status: "ok", data: null });
    useSettingsStore.setState({
      defaultSettings: { audio_feedback: false },
      settings: makeSettings({ audio_feedback: true }),
    });

    await useSettingsStore.getState().resetSetting("audio_feedback");

    expect(mockedChangeAudioFeedback).toHaveBeenCalledWith(false);
    expect(useSettingsStore.getState().settings?.audio_feedback).toBe(false);
    expect(useSettingsStore.getState().isUpdatingKey("audio_feedback")).toBe(
      false,
    );
  });

  it("does nothing when defaults have not been loaded", async () => {
    useSettingsStore.setState({ defaultSettings: null });

    await useSettingsStore.getState().resetSetting("audio_feedback");

    expect(mockedChangeAudioFeedback).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().settings?.audio_feedback).toBe(true);
  });
});

describe("settingsStore.refreshSettings (persisted-field hydration)", () => {
  it("hydrates null persisted fields with their safe defaults", async () => {
    // The Rust side persists optional fields; legacy/partial stores surface
    // nulls here. The store must normalize them exactly like production does.
    mockedGetAppSettings.mockResolvedValue({
      status: "ok",
      data: {
        audio_feedback: true,
        always_on_microphone: null,
        selected_microphone: null,
        clamshell_microphone: null,
        selected_output_device: null,
      } as unknown as AppSettings,
    });
    useSettingsStore.setState({ settings: null, isLoading: true });

    await useSettingsStore.getState().refreshSettings();

    const { settings, isLoading } = useSettingsStore.getState();
    expect(isLoading).toBe(false);
    expect(settings?.always_on_microphone).toBe(false);
    expect(settings?.selected_microphone).toBe("Default");
    expect(settings?.clamshell_microphone).toBe("Default");
    expect(settings?.selected_output_device).toBe("Default");
    expect(settings?.audio_feedback).toBe(true);
  });

  it("ends loading without touching settings when the backend fails", async () => {
    mockedGetAppSettings.mockResolvedValue({
      status: "error",
      error: "store unavailable",
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    useSettingsStore.setState({ settings: null, isLoading: true });

    await useSettingsStore.getState().refreshSettings();

    expect(useSettingsStore.getState().isLoading).toBe(false);
    expect(useSettingsStore.getState().settings).toBeNull();
    expect(errorSpy).toHaveBeenCalled();
  });
});
