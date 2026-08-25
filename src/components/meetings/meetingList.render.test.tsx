// Render + type assertions for the MeetingList §6.5 first-run empty state.
// Runs under vitest (`npx vitest run`).
import { describe, it, expect, beforeAll } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { MeetingList } from "./MeetingList";

beforeAll(async () => {
  await i18next.use(initReactI18next).init({
    lng: "en",
    initImmediate: false,
    resources: {
      en: {
        translation: {
          nav: { meetings: "Meetings" },
          meetings: {
            emptyBenefit:
              "Record your meetings and get transcripts and summaries without leaving the app.",
            newMeeting: "New Meeting",
            importAudio: "Import audio file",
            privacyLocal: "Default: local on this device",
          },
        },
      },
    },
  });
});

const render = (props: Parameters<typeof MeetingList>[0]) =>
  renderToStaticMarkup(<MeetingList {...props} />);

describe("MeetingList feature flag (G3)", () => {
  it("renders nothing with the flag off", () => {
    expect(render({ flagEnabled: false })).toBe("");
  });

  it("follows MEETING_MODE_ENABLED (on by default) with no override", () => {
    // The production default (flagEnabled omitted) resolves to the real build
    // flag; since the wave-1 delivery the flag defaults to ON (productNav.ts),
    // so the component must render the §6.5 state without any explicit
    // override.
    const defaultFlagHtml = render({});
    expect(defaultFlagHtml).not.toBe("");
    // the default-flag render must show the primary action
    expect(defaultFlagHtml.includes("New Meeting")).toBe(true);
  });
});

describe("MeetingList §6.5 empty state (G1/G2)", () => {
  let html: string;

  beforeAll(() => {
    html = render({ flagEnabled: true });
  });

  it("renders the §6.5 heading, benefit, primary action and privacy line", () => {
    expect(html).toMatch(/<h2[^>]*>Meetings<\/h2>/);
    expect(
      html.includes(
        "Record your meetings and get transcripts and summaries without leaving the app.",
      ),
    ).toBe(true);
    expect(html.includes("New Meeting")).toBe(true);
    expect(html.includes("Default: local on this device")).toBe(true);
  });

  it("renders through the shared EmptyState primitive (status role, icon)", () => {
    expect(html).toMatch(/role="status"/);
    expect(html.includes("<svg")).toBe(true); // EmptyState renders its icon
    expect(html.includes('aria-hidden="true"')).toBe(true); // decorative glyphs hidden
  });

  it("renders nothing more: no import action, table or configuration controls", () => {
    // secondary import action must be flag-gated off by default (§6.5)
    expect(html.includes("Import audio file")).toBe(false);
    expect(html.includes("<table")).toBe(false);
    expect(html.includes("<input")).toBe(false);
    expect(html.includes("<select")).toBe(false);
  });

  it("renders the primary action large (ui/Button lg = min-h-[48px])", () => {
    const buttonMatch = /<button[^>]*class="([^"]*)"/.exec(html);
    expect(buttonMatch).toBeTruthy();
    expect((buttonMatch as RegExpExecArray)[1].includes("min-h-[48px]")).toBe(
      true,
    );
  });

  it("leaks no raw translation keys into markup", () => {
    expect(html.includes("meetings.")).toBe(false);
    expect(html.includes("nav.")).toBe(false);
  });
});

describe("MeetingList secondary import action (G1)", () => {
  it("appears only when explicitly enabled and stays secondary", () => {
    const htmlWithImport = render({ flagEnabled: true, importEnabled: true });
    expect(htmlWithImport.includes("Import audio file")).toBe(true);
    const primaryIdx = htmlWithImport.indexOf("bg-background-ui");
    const secondaryIdx = htmlWithImport.indexOf("bg-mid-gray/10");
    expect(primaryIdx).not.toBe(-1); // primary variant classes missing
    expect(secondaryIdx).not.toBe(-1); // secondary variant classes missing
    // New Meeting must stay the primary action
    expect(primaryIdx < secondaryIdx).toBe(true);
  });
});
