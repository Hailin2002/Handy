// Standalone render + type assertions for the MeetingList §6.5 first-run
// empty state (no JS unit-test runner in this repo). Run with:
//   node_modules\.bin\esbuild.cmd src/components/meetings/meetingList.render.test.tsx --bundle --format=esm --platform=node "--define:import.meta.env={}" "--banner:js=import { createRequire } from 'module'; const require = createRequire(import.meta.url);" --outfile=$env:TEMP\meetingList.render.test.mjs --log-level=warning
//   node $env:TEMP\meetingList.render.test.mjs
// (ESM is required because productNav.ts reads `import.meta.env`; the define
// stubs Vite's env object as empty, i.e. the build flag is off in tests, and
// the banner gives the bundled react-dom CJS deps a working `require`.)
import assert from "node:assert";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { MeetingList } from "./MeetingList";

i18next.use(initReactI18next).init({
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

const render = (props: Parameters<typeof MeetingList>[0]) =>
  renderToStaticMarkup(<MeetingList {...props} />);

// --- G3: hidden unless the meeting feature flag is enabled ------------------

assert.strictEqual(
  render({ flagEnabled: false }),
  "",
  "with the flag off the component must render nothing",
);

// The production default (flagEnabled omitted) resolves to the real build
// flag; in this bundle VITE_ENABLE_MEETING_MODE is unset, so it must hide.
const defaultFlagHtml = render({});
if (defaultFlagHtml !== "") {
  assert.fail(
    "with no override the component must follow MEETING_MODE_ENABLED (off in tests)",
  );
}

// --- G1/G2: exactly the §6.5 empty-state elements, via EmptyState ------------

const html = render({ flagEnabled: true });

assert.match(html, /<h2[^>]*>Meetings<\/h2>/, "§6.5 heading must render");
assert.ok(
  html.includes(
    "Record your meetings and get transcripts and summaries without leaving the app.",
  ),
  "one-sentence benefit description must render",
);
assert.ok(html.includes("New Meeting"), "primary action label must render");
assert.ok(
  html.includes("Default: local on this device"),
  "privacy status line must render",
);

// G2: rendered through the shared EmptyState primitive (status role, icon).
assert.match(html, /role="status"/, "must use the EmptyState primitive shell");
assert.ok(html.includes("<svg"), "EmptyState renders its required icon");
assert.ok(html.includes('aria-hidden="true"'), "decorative glyphs stay hidden");

// G1 "nothing more": no secondary import action while flag-gated off,
// no table, no configuration controls.
assert.ok(
  !html.includes("Import audio file"),
  "secondary import action must be flag-gated off by default (§6.5)",
);
assert.ok(!html.includes("<table"), "no empty table (§6.5)");
assert.ok(!html.includes("<input"), "no configuration inputs (§6.5)");
assert.ok(!html.includes("<select"), "no configuration selects (§6.5)");

// Large primary action: ui/Button lg = min-h-[48px].
const buttonMatch = /<button[^>]*class="([^"]*)"/.exec(html);
assert.ok(buttonMatch, "a button must render");
assert.ok(
  buttonMatch[1].includes("min-h-[48px]"),
  "the primary action must be large (lg)",
);

// i18n completeness: no raw keys leak into markup.
assert.ok(!html.includes("meetings."), "raw translation keys must not leak");
assert.ok(!html.includes("nav."), "raw translation keys must not leak");

// --- G1: secondary import action appears only when explicitly enabled -------

const htmlWithImport = render({ flagEnabled: true, importEnabled: true });
assert.ok(
  htmlWithImport.includes("Import audio file"),
  "flag-gated import action renders when importEnabled",
);
const primaryIdx = htmlWithImport.indexOf("bg-background-ui");
const secondaryIdx = htmlWithImport.indexOf("bg-mid-gray/10");
assert.notStrictEqual(primaryIdx, -1, "primary variant classes missing");
assert.notStrictEqual(secondaryIdx, -1, "secondary variant classes missing");
assert.ok(
  primaryIdx < secondaryIdx,
  "New Meeting must stay the primary action",
);

console.log("meetingList.render.test: all assertions passed");
