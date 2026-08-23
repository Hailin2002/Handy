// Standalone render + interaction assertions for the pinned active-meeting
// nav slot (plan §6.3; navigation-seam-contract §5 M2 "activating it
// navigates to { live }"; formatting-contract §2 class A status clock).
// No JS unit-test runner in this repo — run with (PowerShell, from repo root):
//
//   Set-Content $env:TEMP\amn-empty-tsconfig.json '{}'
//   $banner = '--banner:js=import { createRequire } from ''module''; const require = createRequire(process.cwd() + ''/''); Object.defineProperty(globalThis, ''React'', { get: function() { return require(''react''); } });'
//   node_modules\@esbuild\win32-x64\esbuild.exe src/components/shell/ActiveMeetingNavItem.render.test.tsx --bundle --format=esm --platform=node '--define:import.meta.env=__VITE_ENV__' "--tsconfig=$env:TEMP\amn-empty-tsconfig.json" --jsx=transform --jsx-factory=React.createElement --jsx-fragment=React.Fragment $banner --outfile=$env:TEMP\ActiveMeetingNavItem.render.test.mjs --log-level=warning
//   node $env:TEMP\ActiveMeetingNavItem.render.test.mjs                          # Run A — feature flag OFF (G2 branch 1)
//   $env:VITE_ENABLE_MEETING_MODE = "true"; node $env:TEMP\ActiveMeetingNavItem.render.test.mjs   # Run B — feature flag ON (G2 branch 2, G1 live tick, G3 activation)
//
// The single-quoted 'module', 'react' and '/' inside the banner matter:
// invoking the esbuild shim from PowerShell strips embedded double quotes,
// which would corrupt the banner into invalid JS.
//
// esbuild's classic JSX transform (--jsx=transform) is REQUIRED for the G3
// check below: it makes the component's JSX patchable so the test can capture
// the exact button element the server renderer emits and invoke the onClick
// handler a real DOM click would dispatch through. Two traps make the flags
// above load-bearing:
//  1. tsconfig precedence — this repo's tsconfig.json sets "jsx": "react-jsx"
//     and esbuild lets that per-file setting override the CLI --jsx=transform.
//     Under the automatic runtime esbuild binds jsx/jsxs directly at import
//     time into frozen ESM namespace getters that cannot be reassigned, so
//     component renders cannot be intercepted and G3 would fail with "the
//     pill must render a button element". Pointing --tsconfig at an empty
//     file restores CLI control.
//  2. missing React imports — components rendered transitively (e.g. the
//     Sidebar's logo) were written for the automatic runtime and never import
//     React, so under the classic transform their emitted bare
//     `React.createElement` references need a binding. The banner defines a
//     lazy global `React` getter (same module instance esbuild bundled), and
//     anchors createRequire at process.cwd() so both it and react-dom's
//     dynamic requires of node builtins resolve when the bundle is executed
//     from the repo root with its outfile in %TEMP%.
//
// ESM + the `import.meta.env=__VITE_ENV__` define are required because
// productNav.ts reads Vite's env at module scope; the identifier define
// routes it to a global this script installs from process.env BEFORE the
// component modules are imported, so a single bundle serves both runs.
import assert from "node:assert";

// Install before anything else reads import.meta.env (dynamic imports below
// evaluate strictly afterwards).
(globalThis as { __VITE_ENV__?: Record<string, string> }).__VITE_ENV__ = {
  VITE_ENABLE_MEETING_MODE: process.env.VITE_ENABLE_MEETING_MODE ?? "",
};

void (async () => {
  const { MEETING_MODE_ENABLED, formatMeetingDuration } = await import(
    "./productNav"
  );
  const { ActiveMeetingNavItem } = await import("./ActiveMeetingNavItem");
  const { Sidebar } = await import("../Sidebar");
  const React = (await import("react")).default;
  type ReactElement = React.ReactElement;
  const { renderToStaticMarkup } = await import("react-dom/server");
  const i18next = (await import("i18next")).default;
  const { initReactI18next } = await import("react-i18next");

  i18next.use(initReactI18next).init({
    lng: "en",
    initImmediate: false,
    resources: {
      en: {
        translation: {
          nav: {
            primaryAriaLabel: "Main navigation",
            dictate: "Dictate",
            meetings: "Meetings",
            projects: "Projects",
            history: "Dictation History",
            settings: "Settings",
            about: "Help & About",
            activeMeeting: "Recording · {{duration}}",
          },
        },
      },
    },
  });

  interface PillProps {
    meeting: { elapsedSeconds: number } | null;
    onOpen?: () => void;
  }
  const renderPill = (props: PillProps) =>
    renderToStaticMarkup(React.createElement(ActiveMeetingNavItem, props));

  // --- Exported formatter seam: class A status clock ------------------------

  // G1 precondition: exactly at the boundary and on both sides of it.
  assert.strictEqual(formatMeetingDuration(3598), "00:59:58");
  assert.strictEqual(formatMeetingDuration(3599), "00:59:59");
  assert.strictEqual(formatMeetingDuration(3600), "01:00:00");
  assert.strictEqual(formatMeetingDuration(3601), "01:00:01");
  // Hours field always shown (plan line 427: `00:42:18` at 42 minutes)…
  assert.strictEqual(formatMeetingDuration(42 * 60 + 18), "00:42:18");
  // …zero-padded, unbounded beyond 24 h (multi-day sessions)…
  assert.strictEqual(formatMeetingDuration(26 * 3600 + 3 * 60 + 5), "26:03:05");
  // …and clamped: genuine zero, negatives and non-finite input.
  assert.strictEqual(formatMeetingDuration(0), "00:00:00");
  assert.strictEqual(formatMeetingDuration(-5), "00:00:00");
  assert.strictEqual(formatMeetingDuration(Number.NaN), "00:00:00");

  // --- Fake timers: manual clock + interval queue drained by advance() -----

  interface FakeTimers {
    now: () => number;
    setInterval: (fn: () => void, ms: number) => number;
    clearInterval: (id: number) => void;
    advance: (ms: number) => void;
  }
  const createFakeTimers = (): FakeTimers => {
    let nowMs = 0;
    let nextId = 1;
    const tasks = new Map<number, { fn: () => void; ms: number; due: number }>();
    return {
      now: () => nowMs,
      setInterval: (fn, ms) => {
        const id = nextId++;
        tasks.set(id, { fn, ms, due: nowMs + ms });
        return id;
      },
      clearInterval: (id) => tasks.delete(id),
      advance: (delta) => {
        const until = nowMs + delta;
        for (;;) {
          const dueTasks = [...tasks.values()].sort((a, b) => a.due - b.due);
          const dueTask = dueTasks[0];
          if (!dueTask || dueTask.due > until) break;
          nowMs = dueTask.due;
          dueTask.due += dueTask.ms;
          dueTask.fn();
        }
        nowMs = until;
      },
    };
  };

  if (!MEETING_MODE_ENABLED) {
    // --- Run A / G2 branch 1: with the flag off nothing renders, even when an
    // active meeting is reported -------------------------------------------
    assert.strictEqual(
      renderPill({ meeting: { elapsedSeconds: 42 }, onOpen: () => {} }),
      "",
      "with the flag off the pinned slot must render nothing",
    );
    console.log(
      "ActiveMeetingNavItem.render.test [flag off]: all assertions passed",
    );
    return;
  }

  // --- Run B: flag is on -----------------------------------------------------

  // --- G2 branch 2: with the flag on but no meeting, nothing renders --------
  assert.strictEqual(
    renderPill({ meeting: null, onOpen: () => {} }),
    "",
    "with a null meeting the pinned slot must render nothing",
  );

  // --- Positive render: pinned pill appearance ------------------------------
  const pillHtml = renderPill({
    meeting: { elapsedSeconds: 600 },
    onOpen: () => {},
  });
  assert.ok(
    pillHtml.includes("Recording · 00:10:00"),
    "pill must show the i18n label with the formatted duration",
  );
  assert.match(pillHtml, /<button type="button"/, "must be a real button");
  assert.match(
    pillHtml,
    /aria-hidden="true"[^>]*class="[^"]*bg-red-500/,
    "red status dot required by plan §6.3",
  );
  assert.match(pillHtml, /tabular-nums/, "duration digits must be tabular");
  assert.match(
    pillHtml,
    /aria-label="Recording · 00:10:00"/,
    "accessible name must carry the duration",
  );
  assert.ok(!pillHtml.includes("nav."), "raw keys must not leak");

  // --- Pinned placement: directly below the Meetings nav entry --------------
  const navigatedViews: string[] = [];
  const sidebarHtml = renderToStaticMarkup(
    React.createElement(Sidebar, {
      activeView: "dictate",
      onViewChange: (view: string) => navigatedViews.push(view),
      activeMeeting: { elapsedSeconds: 600 },
    }),
  );
  const meetingsIdx = sidebarHtml.indexOf(">Meetings</span>");
  const pillIdx = sidebarHtml.indexOf("Recording · 00:10:00");
  assert.notStrictEqual(meetingsIdx, -1, "Meetings entry must render");
  assert.notStrictEqual(pillIdx, -1, "pinned pill must render under Meetings");
  assert.ok(
    meetingsIdx < pillIdx,
    "pinned pill must appear after the Meetings entry",
  );

  // --- G1: duration ticks across the hh:mm:ss boundary via injected clock ---
  //
  // The component is prop-driven by contract (navigation-seam-contract §6:
  // the future meetingStore rAF clock feeds elapsedSeconds). The fake-timer
  // pump below emulates that store: a 1 s interval derives elapsedSeconds
  // from the injected clock and re-renders the pill, which must follow.
  const timers = createFakeTimers();
  let displayed: string[] = [];
  let navigateCalls = 0;
  const START_SECONDS = 3598; // two ticks before the hh:mm:ss boundary
  let elapsedSeconds = START_SECONDS;
  const rerenderPill = () =>
    displayed.push(
      renderPill({
        meeting: { elapsedSeconds },
        onOpen: () => {
          navigateCalls += 1;
        },
      }),
    );
  rerenderPill();
  timers.setInterval(() => {
    elapsedSeconds = START_SECONDS + Math.floor(timers.now() / 1000);
    rerenderPill();
  }, 1000);
  timers.advance(4_000);

  assert.strictEqual(displayed.length, 5, "initial render plus four ticks");
  assert.match(displayed[0], /Recording · 00:59:58/, "tick 0 pre-boundary");
  assert.match(displayed[1], /Recording · 00:59:59/, "G1: last second below 1 h");
  assert.match(displayed[2], /Recording · 01:00:00/, "G1: first second above 1 h");
  assert.match(displayed[3], /Recording · 01:00:01/, "tick continues past boundary");
  assert.match(displayed[4], /Recording · 01:00:02/, "ticker keeps running");
  assert.strictEqual(
    navigateCalls,
    0,
    "ticking alone must never activate the navigation seam",
  );

  // --- G3: activation invokes the navigate seam exactly once ---------------
  //
  // react-dom/server markup carries no event wiring, so capture the rendered
  // button element (the target a real DOM click dispatches through) by
  // patching the element factories (classic createElement plus the automatic
  // jsx/jsxs runtime — whichever way the bundle compiled the JSX) for one
  // synchronous server render, then invoke its onClick — the single
  // navigation seam of this slot.
  const createdElements: ReactElement[] = [];
  const record =
    <T extends (...args: never[]) => ReactElement>(original: T): T =>
    ((...args: Parameters<T>) => {
      const el = original(...args);
      createdElements.push(el);
      return el;
    }) as T;
  type CreateElement = typeof React.createElement;
  type ElementFactory = (type: unknown, props: unknown, ...children: unknown[]) => ReactElement;
  const originalCreateElement: CreateElement = React.createElement;
  const jsxRuntimeModule = await import("react/jsx-runtime");
  const jsxHost = jsxRuntimeModule as unknown as {
    jsx: ElementFactory;
    jsxs: ElementFactory;
  };
  const originalJsx: ElementFactory = jsxHost.jsx;
  const originalJsxs: ElementFactory = jsxHost.jsxs;
  const originalFactoryProps: CreateElement = record(originalCreateElement);
  React.createElement = originalFactoryProps;
  // The jsx runtime patch is best-effort: with the forced classic transform
  // all component JSX goes through the mutable React.createElement above;
  // a bundled react/jsx-runtime can expose a frozen ESM namespace whose
  // exports cannot be reassigned. If the runtime were used and unpatchable,
  // the "element factories must have been invoked" assertion below fails
  // loudly instead.
  const patchJsx = (
    name: "jsx" | "jsxs",
    original: ElementFactory,
  ): boolean => {
    try {
      (jsxHost as Record<string, unknown>)[name] = record(original);
      return true;
    } catch {
      // Frozen ESM namespace (getter-only exports): leave untouched. Under
      // esbuild's classic JSX transform nothing routes through here anyway;
      // if it did, the "element factories must have been invoked" assertion
      // below fails loudly instead.
      return false;
    }
  };
  const jsxPatched = patchJsx("jsx", originalJsx);
  const jsxsPatched = patchJsx("jsxs", originalJsxs);
  const onOpen = () => {
    navigateCalls += 1;
  };
  let activationHtml = "";
  try {
    activationHtml = renderPill({
      meeting: { elapsedSeconds: 3600 },
      onOpen,
    });
  } finally {
    React.createElement = originalCreateElement;
    if (jsxPatched) jsxHost.jsx = originalJsx;
    if (jsxsPatched) jsxHost.jsxs = originalJsxs;
  }
  assert.ok(activationHtml.includes("Recording · 01:00:00"));
  assert.ok(
    createdElements.length > 0,
    "element factories must have been invoked during the render",
  );
  const buttonEl = [...createdElements]
    .reverse()
    .find((el) => el.type === "button");
  assert.ok(buttonEl, "the pill must render a button element");
  assert.strictEqual(
    buttonEl.props.type,
    "button",
    "activation target must be a plain button",
  );
  assert.strictEqual(
    buttonEl.props.onClick,
    onOpen,
    "the button's onClick must be wired to the injected navigate seam",
  );
  buttonEl.props.onClick(); // one activation == one click
  assert.strictEqual(navigateCalls, 1, "G3: exactly one navigate per activation");

  console.log(
    "ActiveMeetingNavItem.render.test [flag on]: all assertions passed",
  );
})();
