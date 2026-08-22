// Standalone render + type assertions for the EmptyState/ErrorState
// primitives (no JS unit-test runner in this repo). Run with:
//   node_modules\.bin\esbuild.cmd src/components/shared/states.render.test.tsx --bundle --format=cjs --platform=node --outfile=$env:TEMP\states.render.test.cjs --log-level=warning
//   node $env:TEMP\states.render.test.cjs
//
// The negative cases below are compile-time only (@ts-expect-error) and are
// never executed; `tsc --noEmit` fails if the API stops rejecting them.
import assert from "node:assert";
import { renderToStaticMarkup } from "react-dom/server";
import { Inbox, MicOff, RotateCcw } from "lucide-react";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";

i18next.use(initReactI18next).init({
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

// --- Positive cases -------------------------------------------------------

const emptyHtml = renderToStaticMarkup(
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

const emptyInfoHtml = renderToStaticMarkup(
  <EmptyState
    tone="info"
    headingLevel={3}
    icon={<Inbox size={32} />}
    title={{ key: "statesTest.emptyTitle" }}
    actions={[{ label: { key: "statesTest.newMeeting" } }]}
  />,
);

const errorHtml = renderToStaticMarkup(
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

const errorWarningHtml = renderToStaticMarkup(
  <ErrorState
    tone="warning"
    icon={<MicOff size={32} />}
    title={{ key: "statesTest.errorTitle" }}
    actions={[{ label: { key: "statesTest.switchDevice" } }]}
  />,
);

// G2: each state renders icon + text + action.
assert.ok(emptyHtml.includes("<svg"), "EmptyState must render an icon");
assert.ok(emptyHtml.includes("No meetings yet"), "title text must render");
assert.ok(
  emptyHtml.includes("Start your first meeting"),
  "description text must render",
);
assert.ok(
  emptyHtml.includes("New meeting") && emptyHtml.includes("Import audio file"),
  "at least one labeled action must render",
);

assert.ok(errorHtml.includes("<svg"), "ErrorState must render an icon");
assert.ok(errorHtml.includes("Microphone not available"));
assert.ok(errorHtml.includes("Switch device"));

// A11y primitives: status/alert roles, decorative icons hidden from AT,
// visible-text buttons with a real hit area (ui/Button md = min-h-[44px]).
assert.match(emptyHtml, /role="status"/);
assert.match(emptyInfoHtml, /role="status"/);
assert.match(errorHtml, /role="alert"/);
assert.ok(emptyHtml.includes('aria-hidden="true"'));
assert.ok(errorHtml.includes('aria-hidden="true"'));
assert.ok(emptyHtml.includes("min-h-[44px]"));
assert.ok(errorHtml.includes("min-h-[44px]"));
assert.match(emptyHtml, /<h2[ >]/);
assert.match(emptyInfoHtml, /<h3[ >]/);

// First action is the primary one, later actions render as secondary.
const primaryIdx = emptyHtml.indexOf("bg-background-ui");
const secondaryIdx = emptyHtml.indexOf("bg-mid-gray/10");
assert.notStrictEqual(primaryIdx, -1, "primary action classes missing");
assert.notStrictEqual(secondaryIdx, -1, "secondary action classes missing");
assert.ok(primaryIdx < secondaryIdx, "first action must be primary");

// §6.8: color never sole carrier — danger tone adds color AND keeps icon+text.
assert.ok(errorHtml.includes("text-red-400"), "danger tone color missing");
assert.ok(errorHtml.includes("Microphone not available"));
assert.ok(errorWarningHtml.includes("text-warning"));

// i18n keys resolve through the shared i18next instance (values interpolate).
assert.ok(
  !emptyHtml.includes("statesTest."),
  "raw translation keys must never leak into markup",
);

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

console.log("states.render.test: all assertions passed");
