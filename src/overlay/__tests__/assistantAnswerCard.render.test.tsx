/**
 * Render assertions for the assistant result card.
 *
 * The repo has no DOM and no @testing-library, so rendering goes through
 * `react-dom/server` against the real `en` translation file — the same pattern
 * as `settings/assistants/__tests__/assistantSettings.render.test.tsx`.
 *
 * Only the card is rendered, never the overlay shell around it. The shell
 * returns `null` until an event listener sets `isVisible`, and effects do not
 * run under `renderToStaticMarkup`, so a shell render would assert nothing.
 * That is also why the card is exported separately.
 */
import { beforeAll, describe, expect, it, vi } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(async () => undefined),
}));

// The real `@/i18n` reaches for `import.meta.glob` and the Tauri OS plugin at
// module scope. The card only needs `t`, which the block below wires up.
vi.mock("@/i18n", () => ({
  default: { language: "en" },
  syncLanguageFromSettings: vi.fn(async () => undefined),
}));

vi.mock("@/bindings", () => ({
  commands: {
    assistantCopy: vi.fn(),
    assistantPaste: vi.fn(),
    assistantReplace: vi.fn(),
    assistantDismiss: vi.fn(),
    assistantEscalate: vi.fn(),
    assistantPresentAnswer: vi.fn(),
    getAppSettings: vi.fn(),
    cancelOperation: vi.fn(),
  },
  events: {
    streamTextEvent: { listen: vi.fn(async () => () => undefined) },
    streamPhaseEvent: { listen: vi.fn(async () => () => undefined) },
    assistantAnswerEvent: { listen: vi.fn(async () => () => undefined) },
  },
}));

import type { AssistantAnswerEvent } from "@/bindings";
import enTranslation from "@/i18n/locales/en/translation.json";
import { AssistantAnswerCard } from "../RecordingOverlay";

const strings = enTranslation.overlay.assistant;

const answerEvent = (
  overrides: Partial<AssistantAnswerEvent> = {},
): AssistantAnswerEvent => ({
  profile_id: "fast_helper",
  question: "what is the capital of France",
  answer: "Paris.",
  error: null,
  links: [],
  elapsed_ms: 812,
  expects_card: true,
  truncated: false,
  ...overrides,
});

const render = (answer: AssistantAnswerEvent) =>
  renderToStaticMarkup(<AssistantAnswerCard answer={answer} />);

beforeAll(async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: "en",
      fallbackLng: "en",
      resources: { en: { translation: enTranslation } },
    });
  }
});

describe("AssistantAnswerCard", () => {
  it("shows the answer, the question it was asked, and every action", () => {
    const html = render(answerEvent());

    expect(html).toContain("Paris.");
    // The question is on the card because a misheard question is the most
    // common cause of a surprising answer.
    expect(html).toContain("what is the capital of France");
    expect(html).toContain(strings.copy);
    expect(html).toContain(strings.paste);
    expect(html).toContain(strings.replace);
    expect(html).toContain(strings.close);
    expect(html).toContain('tabindex="0"');
    expect(html).toContain(
      'aria-keyshortcuts="Control+C Meta+C Control+V Meta+V"',
    );
  });

  /** A translation key rendered verbatim means the i18n wiring is broken. */
  it("leaks no raw translation keys", () => {
    expect(render(answerEvent())).not.toContain("overlay.assistant");
  });

  /**
   * Error strings come from `AssistantError::message()` already phrased for the
   * user and naming the setting to change, so the card must print them as-is
   * rather than substituting a generic failure message.
   */
  it("renders a failure verbatim and offers nothing to copy or paste", () => {
    // A real message from the `AssistantError::message()` table. Chosen for
    // having no apostrophe, so the assertion can compare against the source
    // text rather than against React's HTML entity escaping.
    const message =
      "The assistant did not answer within 15s. Raise its timeout in Settings, or choose a faster model.";
    const html = render(
      answerEvent({ answer: null, error: message, links: [] }),
    );

    expect(html).toContain(message);
    expect(html).toContain('role="alert"');
    // There is no text, so acting on it would be meaningless.
    expect(html).not.toContain(strings.copy);
    expect(html).not.toContain(strings.paste);
    expect(html).not.toContain(strings.replace);
    // Closing and escalating both still make sense.
    expect(html).toContain(strings.close);
    expect(html).toContain(strings.escalate);
  });

  /**
   * A cut-off answer must say so: `finish_reason == "length"` is otherwise
   * indistinguishable from a complete one, and the user copies half a sentence.
   */
  it("flags an answer that hit the token ceiling, and only then", () => {
    const cut = render(
      answerEvent({ answer: "The capital of", truncated: true }),
    );
    expect(cut).toContain(strings.truncated);
    expect(cut).toContain('role="note"');

    const whole = render(answerEvent({ truncated: false }));
    expect(whole).not.toContain(strings.truncated);
  });

  /** Escalating an answer to the model that just produced it is a dead end. */
  it("hides the escalate button once the answer came from the smart assistant", () => {
    const fast = render(answerEvent());
    expect(fast).toContain(strings.escalate);
    expect(fast).toContain(strings.label);

    const smart = render(answerEvent({ profile_id: "smart_assistant" }));
    expect(smart).not.toContain(strings.escalate);
    expect(smart).toContain(strings.smartLabel);
  });

  it("offers no link button when the answer carries no URL", () => {
    const html = render(answerEvent());
    expect(html).not.toContain(strings.openLink);
    expect(html).not.toContain("Open link (");
  });

  it("opens a lone link directly instead of asking which one", () => {
    const html = render(answerEvent({ links: ["https://example.com/paris"] }));
    expect(html).toContain(strings.openLink);
    // Nothing to choose between, so no list is rendered.
    expect(html).not.toContain("https://example.com/paris");
  });

  it("asks which link to open when there is more than one", () => {
    const html = render(
      answerEvent({
        links: ["https://example.com/a", "https://example.com/b"],
      }),
    );

    expect(html).toContain("Open link (2)");
    // The list itself is collapsed until the button is pressed, so the window
    // does not have to be tall enough for every URL an answer might cite.
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain("https://example.com/a");
  });
});
