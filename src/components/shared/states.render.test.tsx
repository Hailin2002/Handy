// Render + type assertions for the EmptyState/ErrorState primitives.
// Runs under vitest (`npx vitest run`).
//
// The negative cases below are compile-time only (@ts-expect-error) and are
// never executed; `tsc --noEmit` fails if the API stops rejecting them.
import { describe, it, expect, beforeAll } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Inbox, MicOff, RotateCcw } from "lucide-react";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";

let emptyHtml: string;
let emptyInfoHtml: string;
let errorHtml: string;
let errorWarningHtml: string;

beforeAll(async () => {
  await i18next.use(initReactI18next).init({
    lng: "en",
    initImmediate: false,
    resources: {
      en: {
        translation: {
          statesTest: {
            emptyTitle: "No meetings yet",
            emptyDescription: "Start your first meeting to see it here.",
            newMeeting: "New meeting",
            importAudio: "Import audio file",
            errorTitle: "Microphone not available",
            errorDescription: "Pick another input device to continue.",
            switchDevice: "Switch device",
          },
        },
      },
    },
  });

  emptyHtml = renderToStaticMarkup(
    <EmptyState
      icon={<Inbox size={32} />}
      title={{ key: "statesTest.emptyTitle" }}
      description={{ key: "statesTest.emptyDescription" }}
      actions={[
        { label: { key: "statesTest.newMeeting" } },
        { label: { key: "statesTest.importAudio" } },
      ]}
    />,
  );

  emptyInfoHtml = renderToStaticMarkup(
    <EmptyState
      tone="info"
      headingLevel={3}
      icon={<Inbox size={32} />}
      title={{ key: "statesTest.emptyTitle" }}
      actions={[{ label: { key: "statesTest.newMeeting" } }]}
    />,
  );

  errorHtml = renderToStaticMarkup(
    <ErrorState
      icon={<MicOff size={32} />}
      title={{ key: "statesTest.errorTitle" }}
      description={{ key: "statesTest.errorDescription" }}
      actions={[
        {
          label: { key: "statesTest.switchDevice" },
          icon: <RotateCcw size={14} />,
        },
      ]}
    />,
  );

  errorWarningHtml = renderToStaticMarkup(
    <ErrorState
      tone="warning"
      icon={<MicOff size={32} />}
      title={{ key: "statesTest.errorTitle" }}
      actions={[{ label: { key: "statesTest.switchDevice" } }]}
    />,
  );
});

describe("EmptyState / ErrorState render icon + text + action (G2)", () => {
  it("renders the EmptyState icon, title, description and labeled actions", () => {
    expect(emptyHtml.includes("<svg")).toBe(true);
    expect(emptyHtml.includes("No meetings yet")).toBe(true);
    expect(emptyHtml.includes("Start your first meeting")).toBe(true);
    expect(
      emptyHtml.includes("New meeting") &&
        emptyHtml.includes("Import audio file"),
    ).toBe(true);
  });

  it("renders the ErrorState icon, title and action", () => {
    expect(errorHtml.includes("<svg")).toBe(true);
    expect(errorHtml.includes("Microphone not available")).toBe(true);
    expect(errorHtml.includes("Switch device")).toBe(true);
  });
});

describe("EmptyState / ErrorState a11y primitives", () => {
  it("uses status roles for empty states and alert for errors", () => {
    expect(emptyHtml).toMatch(/role="status"/);
    expect(emptyInfoHtml).toMatch(/role="status"/);
    expect(errorHtml).toMatch(/role="alert"/);
  });

  it("hides decorative icons from assistive technology", () => {
    expect(emptyHtml.includes('aria-hidden="true"')).toBe(true);
    expect(errorHtml.includes('aria-hidden="true"')).toBe(true);
  });

  it("gives buttons a real hit area (ui/Button md = min-h-[44px])", () => {
    expect(emptyHtml.includes("min-h-[44px]")).toBe(true);
    expect(errorHtml.includes("min-h-[44px]")).toBe(true);
  });

  it("honours the requested heading level", () => {
    expect(emptyHtml).toMatch(/<h2[ >]/);
    expect(emptyInfoHtml).toMatch(/<h3[ >]/);
  });
});

describe("EmptyState action ordering", () => {
  it("renders the first action as primary and later actions as secondary", () => {
    const primaryIdx = emptyHtml.indexOf("bg-background-ui");
    const secondaryIdx = emptyHtml.indexOf("bg-mid-gray/10");
    expect(primaryIdx).not.toBe(-1); // primary action classes missing
    expect(secondaryIdx).not.toBe(-1); // secondary action classes missing
    expect(primaryIdx < secondaryIdx).toBe(true);
  });
});

describe("§6.8: color is never the sole carrier", () => {
  it("adds tone color while keeping icon and text", () => {
    expect(errorHtml.includes("text-red-400")).toBe(true); // danger tone color
    expect(errorHtml.includes("Microphone not available")).toBe(true);
    expect(errorWarningHtml.includes("text-warning")).toBe(true);
  });
});

describe("i18n hygiene", () => {
  it("resolves keys through the shared i18next instance without leaking them", () => {
    expect(emptyHtml.includes("statesTest.")).toBe(false);
  });
});

// --- Negative cases (compile-time only, G1) --------------------------------

// prettier-ignore
function negativeTypeCases() {
  return (
    <>
      {/* @ts-expect-error G1: `actions` is required */}
      <EmptyState icon={<Inbox size={32} />} title={{ key: "statesTest.emptyTitle" }} />
      {/* @ts-expect-error G1: empty action lists are rejected by the tuple type */}
      <EmptyState icon={<Inbox size={32} />} title={{ key: "statesTest.emptyTitle" }} actions={[]} />
      {/* @ts-expect-error G1: icon is required (§6.8: color/text alone is not enough) */}
      <EmptyState title={{ key: "statesTest.emptyTitle" }} actions={[{ label: { key: "statesTest.newMeeting" } }]} />
      {/* @ts-expect-error G1: title text is required */}
      <EmptyState icon={<Inbox size={32} />} actions={[{ label: { key: "statesTest.newMeeting" } }]} />
      {/* @ts-expect-error G1: errors demand a next action too */}
      <ErrorState icon={<MicOff size={32} />} title={{ key: "statesTest.errorTitle" }} />
      {/* @ts-expect-error G1: empty action lists are rejected on ErrorState */}
      <ErrorState icon={<MicOff size={32} />} title={{ key: "statesTest.errorTitle" }} actions={[]} />
    </>
  );
}
void negativeTypeCases;
