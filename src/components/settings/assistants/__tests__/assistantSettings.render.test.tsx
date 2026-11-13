/**
 * Render + wiring assertions for the Phase F assistant settings section.
 *
 * Same shape as the sibling `recordingAudioSettings.render.test.tsx`: the repo
 * has no DOM and no @testing-library, so rendering is asserted through
 * `react-dom/server` against the REAL en translation file, and behaviour is
 * asserted where it is observable — at the generated Tauri command boundary.
 *
 * The things most worth pinning here are cross-cutting mistakes that
 * type-checking cannot catch:
 *
 * - every write must carry the **profile** id, never a provider id, because
 *   the backend keys both the profile map and the keychain scope by profile;
 * - the two profiles must not bleed into each other (a shared hook with a
 *   `profileId` prop is exactly the shape that regresses this way);
 * - an empty model is the shipped first-run state, so the "pick a model"
 *   prompt has to be visible rather than swallowed as an error.
 *
 * Why `useSettings` is mocked for the render half instead of seeding the
 * zustand store: under `react-dom/server`, `useSyncExternalStore` reads
 * `getServerSnapshot`, which zustand maps to `getInitialState()` — the state a
 * test writes with `setState` is invisible to a component that consumes the
 * store through the hook. (The sibling ducking test gets away with seeding
 * because it reads through `getSetting()`, a plain function that calls
 * `getState()` directly.) The store wiring is therefore asserted separately,
 * below, where `getState()` is what runs.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";

// Hoisted so the `vi.mock` factory below can close over it: each render swaps
// `current` for the settings that test wants the component to see.
const { settingsRef, keyStatusRef } = vi.hoisted(() => ({
  settingsRef: { current: {} as Record<string, unknown> },
  keyStatusRef: { current: {} as Record<string, boolean> },
}));

// The Tauri IPC bridge does not exist outside the webview.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

// The shortcut editor drags in the whole global-shortcut stack and is covered
// by its own tests. Stub it, but keep the binding id observable — passing the
// wrong one would silently point both profiles at the same chord.
vi.mock("@/components/settings/general/ShortcutInput", () => ({
  ShortcutInput: ({ shortcutId }: { shortcutId: string }) =>
    React.createElement("div", { "data-shortcut-id": shortcutId }),
}));

vi.mock("@/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: settingsRef.current,
    isUpdating: () => false,
    postProcessModelOptions: {},
    fetchPostProcessModels: vi.fn(async () => []),
    assistantModelOptions: {},
    assistantApiKeyConfigured: keyStatusRef.current,
    fetchAssistantModels: vi.fn(async () => []),
    setAssistantProvider: vi.fn(),
    updateAssistantApiKey: vi.fn(),
    updateAssistantModel: vi.fn(),
    updateAssistantSystemPrompt: vi.fn(),
    updateAssistantLimits: vi.fn(),
    setAssistantOutputMode: vi.fn(),
  }),
}));

vi.mock("@/bindings", () => ({
  commands: {
    getAppSettings: vi.fn(),
    getAssistantApiKeyStatus: vi.fn(),
    setAssistantProvider: vi.fn(),
    changeAssistantApiKeySetting: vi.fn(),
    fetchAssistantModels: vi.fn(),
    changeAssistantModelSetting: vi.fn(),
    changeAssistantSystemPromptSetting: vi.fn(),
    changeAssistantLimitsSetting: vi.fn(),
    setAssistantOutputMode: vi.fn(),
    changePostProcessApiKeySetting: vi.fn(),
    fetchPostProcessModels: vi.fn(),
  },
}));

import type { AppSettings } from "@/bindings";
import { commands } from "@/bindings";
import enTranslation from "@/i18n/locales/en/translation.json";
import { useSettingsStore } from "@/stores/settingsStore";
import { AssistantSettings } from "../AssistantSettings";
import { ASSISTANT_LIMITS, clampAssistantLimit } from "../assistantProfiles";

const initI18n = async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: "en",
      fallbackLng: "en",
      resources: { en: { translation: enTranslation } },
    });
  }
};

const provider = (id: string, label: string) => ({
  id,
  label,
  base_url: `https://api.${id}.test/v1`,
  allow_base_url_edit: false,
  models_endpoint: null,
  supports_structured_output: true,
});

const profile = (id: string, overrides: Record<string, unknown> = {}) => ({
  id,
  provider_id: "openai",
  model: "",
  system_prompt: "Answer briefly.",
  max_output_tokens: 512,
  timeout_secs: 15,
  output_mode: "show",
  disable_reasoning: true,
  ...overrides,
});

const baseSettings = (
  overrides: Record<string, unknown> = {},
): Record<string, unknown> => ({
  post_process_providers: [
    provider("openai", "OpenAI"),
    provider("anthropic", "Anthropic"),
  ],
  post_process_api_keys: { openai: "POST-PROCESS-KEY" },
  assistant_profiles: {
    fast_helper: profile("fast_helper"),
    smart_assistant: profile("smart_assistant", {
      max_output_tokens: 2048,
      timeout_secs: 90,
      disable_reasoning: false,
    }),
  },
  assistant_api_keys: {},
  ...overrides,
});

const renderAt = async (
  settings: Record<string, unknown>,
  keyStatus: Record<string, boolean> = {},
) => {
  await initI18n();
  settingsRef.current = settings;
  keyStatusRef.current = keyStatus;
  return renderToStaticMarkup(React.createElement(AssistantSettings));
};

beforeEach(() => {
  vi.clearAllMocks();
  keyStatusRef.current = {};
});

describe("AssistantSettings render", () => {
  it("renders both profiles and both hotkey editors in the assistant tab", async () => {
    const html = await renderAt(baseSettings());

    expect(html).toContain("Fast Helper");
    expect(html).toContain("Smart Assistant");
    expect(html).toContain('data-shortcut-id="assistant_fast"');
    expect(html).toContain('data-shortcut-id="assistant_smart"');
  });

  it("prompts for a model while none is chosen — the shipped first-run state", async () => {
    const html = await renderAt(baseSettings());

    expect(html).toContain("No model chosen yet");
  });

  it("drops the prompt once a model is set", async () => {
    const html = await renderAt(
      baseSettings({
        assistant_profiles: {
          fast_helper: profile("fast_helper", { model: "some-model" }),
          smart_assistant: profile("smart_assistant", { model: "other-model" }),
        },
      }),
    );

    expect(html).not.toContain("No model chosen yet");
  });

  it("shows the secure keychain status without exposing either secret", async () => {
    const html = await renderAt(
      baseSettings({ assistant_api_keys: { fast_helper: "MUST-NOT-LEAK" } }),
      { fast_helper: true, smart_assistant: false },
    );

    expect(html).not.toContain("MUST-NOT-LEAK");
    expect(html).not.toContain("POST-PROCESS-KEY");
    expect(html).toContain("API key for OpenAI is stored");
    expect(html).toContain("Clear");
  });

  it("shows clearly when the selected provider still needs a key", async () => {
    const html = await renderAt(baseSettings());

    expect(html).toContain("No API key stored for OpenAI");
  });

  it("renders each profile's own limits, not one profile's twice", async () => {
    const html = await renderAt(baseSettings());

    expect(html).toContain('value="512"');
    expect(html).toContain('value="2048"');
    expect(html).toContain('value="15"');
    expect(html).toContain('value="90"');
  });

  it("survives settings that have no assistant profiles yet", async () => {
    const html = await renderAt(baseSettings({ assistant_profiles: {} }));

    // Still renders its shell rather than throwing on the missing profile.
    expect(html).toContain("AI Assistants");
  });

  it("leaks no raw i18n keys", async () => {
    const html = await renderAt(baseSettings());

    expect(html).not.toContain("settings.assistants");
  });
});

describe("AssistantSettings settings wiring", () => {
  const ok = { status: "ok", data: null } as const;

  beforeEach(() => {
    useSettingsStore.setState({
      settings: baseSettings() as unknown as AppSettings,
      isLoading: false,
      isUpdating: {},
      assistantApiKeyConfigured: {},
    });
    // Every assistant write re-reads settings afterwards.
    vi.mocked(commands.getAppSettings).mockResolvedValue({
      status: "ok",
      data: baseSettings() as unknown as AppSettings,
    });
    vi.mocked(commands.getAssistantApiKeyStatus).mockResolvedValue({
      status: "ok",
      data: { fast_helper: true, smart_assistant: false },
    });
  });

  it("routes a provider change to the profile, not to post-processing", async () => {
    vi.mocked(commands.setAssistantProvider).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .setAssistantProvider("fast_helper", "anthropic");

    expect(commands.setAssistantProvider).toHaveBeenCalledWith(
      "fast_helper",
      "anthropic",
    );
  });

  it("writes the API key under the profile id", async () => {
    vi.mocked(commands.changeAssistantApiKeySetting).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .updateAssistantApiKey("smart_assistant", "sk-test");

    expect(commands.changeAssistantApiKeySetting).toHaveBeenCalledWith(
      "smart_assistant",
      "sk-test",
    );
    // The post-processing key command must stay untouched — the two scopes are
    // deliberately separate in the keychain.
    expect(commands.changePostProcessApiKeySetting).not.toHaveBeenCalled();
  });

  it("sends both limits together, as the backend command expects", async () => {
    vi.mocked(commands.changeAssistantLimitsSetting).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .updateAssistantLimits("fast_helper", 1024, 30);

    expect(commands.changeAssistantLimitsSetting).toHaveBeenCalledWith(
      "fast_helper",
      1024,
      30,
    );
  });

  it("routes the output mode per profile", async () => {
    vi.mocked(commands.setAssistantOutputMode).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .setAssistantOutputMode("smart_assistant", "paste");

    expect(commands.setAssistantOutputMode).toHaveBeenCalledWith(
      "smart_assistant",
      "paste",
    );
  });

  it("lists models with the assistant's own key, not the post-processing one", async () => {
    vi.mocked(commands.fetchAssistantModels).mockResolvedValue({
      status: "ok",
      data: ["openai/gpt-4o-mini"],
    });

    const models = await useSettingsStore
      .getState()
      .fetchAssistantModels("fast_helper");

    // Keyed by profile, because the credential is stored per profile under the
    // `assistant` keychain scope. Routing this through
    // `fetch_post_process_models` reads a different secret and returns nothing,
    // which is exactly how the dropdown ended up permanently empty.
    expect(commands.fetchAssistantModels).toHaveBeenCalledWith("fast_helper");
    expect(commands.fetchPostProcessModels).not.toHaveBeenCalled();
    expect(models).toEqual(["openai/gpt-4o-mini"]);
    expect(
      useSettingsStore.getState().assistantModelOptions.fast_helper,
    ).toEqual(["openai/gpt-4o-mini"]);
  });

  it("drops a cached model list when the key changes", async () => {
    useSettingsStore.setState({
      assistantModelOptions: { fast_helper: ["stale/model"] },
    });
    vi.mocked(commands.changeAssistantApiKeySetting).mockResolvedValue(ok);

    await useSettingsStore
      .getState()
      .updateAssistantApiKey("fast_helper", "sk-new");

    // A list produced by the old credential says nothing about the new one.
    expect(
      useSettingsStore.getState().assistantModelOptions.fast_helper,
    ).toEqual([]);
  });

  it("keeps the previous value on screen when the backend rejects a write", async () => {
    vi.mocked(commands.setAssistantProvider).mockResolvedValue({
      status: "error",
      error: "unknown provider",
    });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    await useSettingsStore
      .getState()
      .setAssistantProvider("fast_helper", "nope");

    // No refresh ran, so the store still holds the last good provider.
    expect(commands.getAppSettings).not.toHaveBeenCalled();
    expect(
      useSettingsStore.getState().settings?.assistant_profiles?.fast_helper
        ?.provider_id,
    ).toBe("openai");
    errorSpy.mockRestore();
  });
});

describe("clampAssistantLimit", () => {
  const { maxOutputTokens: tokens, timeoutSecs: timeout } = ASSISTANT_LIMITS;

  it("mirrors the ranges the Rust command clamps to", () => {
    expect(clampAssistantLimit(1, tokens, 512)).toBe(tokens.min);
    expect(clampAssistantLimit(99999, tokens, 512)).toBe(tokens.max);
    expect(clampAssistantLimit(1, timeout, 15)).toBe(timeout.min);
    expect(clampAssistantLimit(99999, timeout, 15)).toBe(timeout.max);
  });

  it("rounds fractional input to a whole number", () => {
    expect(clampAssistantLimit(512.4, tokens, 512)).toBe(512);
    expect(clampAssistantLimit(512.6, tokens, 512)).toBe(513);
  });

  it("restores the fallback for an emptied field rather than snapping to a bound", () => {
    expect(clampAssistantLimit(Number.NaN, tokens, 2048)).toBe(2048);
    expect(clampAssistantLimit(Number.POSITIVE_INFINITY, timeout, 90)).toBe(90);
  });
});
