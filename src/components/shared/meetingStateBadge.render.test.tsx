// Render + type assertions for the MeetingStateBadge primitive (plan §6.8).
// Runs under vitest (`npx vitest run`).
//
// The negative case below is compile-time only (@ts-expect-error) and is
// never executed; `tsc --noEmit` (part of `bun run build`) fails if the API
// stops rejecting unknown states.
import { describe, it, expect, beforeAll } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import {
  MeetingStateBadge,
  meetingStateBadgeStates,
} from "./MeetingStateBadge";

// i18n fixture mirroring src/i18n/locales/en/translation.json (`meetings.*`).
const TRANSLATION: Record<string, string> = {
  badgeRecording: "Recording",
  badgeStopping: "Stopping",
  badgeInterrupted: "Interrupted",
  badgeAttention: "Attention needed",
  badgeComplete: "Completed",
  badgeProcessing: "Processing",
  badgePaused: "Paused",
};

// Expected visible label per badge state.
const LABELS: Record<string, string> = {
  recording: TRANSLATION.badgeRecording,
  stopping: TRANSLATION.badgeStopping,
  interrupted: TRANSLATION.badgeInterrupted,
  attention: TRANSLATION.badgeAttention,
  complete: TRANSLATION.badgeComplete,
  processing: TRANSLATION.badgeProcessing,
  paused: TRANSLATION.badgePaused,
};

const htmlByState = new Map<string, string>();

beforeAll(async () => {
  await i18next.use(initReactI18next).init({
    lng: "en",
    resources: {
      en: {
        translation: {
          meetings: TRANSLATION,
        },
      },
    },
  });
  for (const state of meetingStateBadgeStates) {
    htmlByState.set(
      state,
      renderToStaticMarkup(<MeetingStateBadge state={state} />),
    );
  }
});

describe("MeetingStateBadge states (G1)", () => {
  it("defines exactly seven badge states (§6.8)", () => {
    expect(meetingStateBadgeStates.length).toBe(7);
  });

  it("renders every state with a non-empty text label, a decorative icon and a data hook", () => {
    for (const state of meetingStateBadgeStates) {
      const html = htmlByState.get(state) as string;
      const label = LABELS[state];
      // the state's own translated, non-empty text label
      expect(html.includes(`>${label}<`)).toBe(true);
      // an icon is rendered alongside the text
      expect(html.includes("<svg")).toBe(true);
      // the icon is decorative only; the text carries the meaning for AT
      expect(html.includes('aria-hidden="true"')).toBe(true);
      // the state is exposed as data for tests/automation
      expect(html).toMatch(new RegExp(`data-meeting-badge="${state}"`));
    }
  });

  it("uses distinct markup and distinct icons for every pair of states", () => {
    for (let i = 0; i < meetingStateBadgeStates.length; i++) {
      for (let j = i + 1; j < meetingStateBadgeStates.length; j++) {
        const a = htmlByState.get(meetingStateBadgeStates[i]) as string;
        const b = htmlByState.get(meetingStateBadgeStates[j]) as string;
        // visually distinguishable beyond color
        expect(a).not.toBe(b);
        const svgA = /<svg[\s\S]*?<\/svg>/.exec(a)?.[0] ?? "";
        const svgB = /<svg[\s\S]*?<\/svg>/.exec(b)?.[0] ?? "";
        expect(svgA).not.toBe(svgB);
      }
    }
  });
});

describe("MeetingStateBadge never relies on color alone (G2)", () => {
  // Simulation of the failure mode: remove every Tailwind color/palette class
  // from each rendered badge. If any state depended on color to communicate,
  // what remains would lose its identity. We assert that after stripping ALL
  // class attributes every state is still uniquely identifiable by its text
  // plus icon structure.
  const strip = () => {
    const stripped = new Map<string, string>();
    for (const [state, html] of htmlByState) {
      stripped.set(state, html.replace(/class="[^"]*"/g, ""));
    }
    return stripped;
  };

  it("keeps each state unique after all styling is removed", () => {
    const stripped = strip();
    expect(new Set(stripped.values()).size).toBe(stripped.size);
  });

  it("keeps label and icon after all styling is removed", () => {
    for (const [state, html] of strip()) {
      expect(html.includes(`>${LABELS[state]}<`)).toBe(true);
      expect(html.includes("<svg")).toBe(true);
    }
  });
});

describe("MeetingStateBadge label override safety", () => {
  it("renders an explicit label override and keeps the icon", () => {
    const overrideHtml = renderToStaticMarkup(
      <MeetingStateBadge state="recording" label="02:31" />,
    );
    expect(overrideHtml.includes(">02:31<")).toBe(true);
    expect(overrideHtml.includes("<svg")).toBe(true);
  });

  it("falls back to the default label for an empty override so text never disappears", () => {
    const emptyOverrideHtml = renderToStaticMarkup(
      <MeetingStateBadge state="paused" label="" />,
    );
    expect(emptyOverrideHtml.includes(">Paused<")).toBe(true);
  });
});

describe("MeetingStateBadge i18n hygiene", () => {
  it("never leaks raw translation keys into markup", () => {
    for (const html of htmlByState.values()) {
      expect(html.includes("meetings.")).toBe(false);
    }
  });
});

// --- Negative case (compile-time only, G1) -----------------------------------

function negativeTypeCase() {
  // @ts-expect-error unknown states are rejected by the closed union type
  return <MeetingStateBadge state="exploded" />;
}
void negativeTypeCase;
