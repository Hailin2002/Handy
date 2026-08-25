// Render + interaction assertions for the pinned active-meeting nav slot
// (plan §6.3; navigation-seam-contract §5 M2 "activating it navigates to
// { live }"; formatting-contract §2 class A status clock).
// Runs under vitest (`npx vitest run`).
//
// Two mechanisms carry over from the previous standalone harness:
//
//  1. The feature flag is read at module scope of productNav.ts from
//     `import.meta.env.VITE_ENABLE_MEETING_MODE`, so each branch stubs the env
//     and re-imports the module graph (`vi.resetModules`). Everything the
//     render touches (react, react-dom/server, i18next) is re-imported from
//     the same fresh registry so a single React instance is in play.
//  2. react-dom/server markup carries no event wiring, so the G3 activation
//     check captures the button element the server renderer emits. The
//     automatic JSX runtime is mocked with a recording pass-through and
//     React.createElement is patched for the duration of one synchronous
//     render; the captured element onClick is the single navigation seam of
//     this slot.
import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import { formatMeetingDuration } from "./productNav";

// Recording sink for the JSX runtime mock. Null except during the one render
// the G3 case inspects, so no other test pays for the bookkeeping.
const sink = vi.hoisted(() => ({ recorded: null as any[] | null }));

const wrapFactory =
  (fn: any) =>
  (...args: any[]) => {
    const el = fn(...args);
    if (sink.recorded) sink.recorded.push(el);
    return el;
  };

vi.mock("react/jsx-runtime", async () => {
  const actual = await vi.importActual<any>("react/jsx-runtime");
  return {
    ...actual,
    jsx: wrapFactory(actual.jsx),
    jsxs: wrapFactory(actual.jsxs),
  };
});

vi.mock("react/jsx-dev-runtime", async () => {
  const actual = await vi.importActual<any>("react/jsx-dev-runtime");
  return { ...actual, jsxDEV: wrapFactory(actual.jsxDEV) };
});

const TRANSLATION_RESOURCES = {
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
};

interface PillProps {
  meeting: { elapsedSeconds: number } | null;
  onOpen?: () => void;
}

interface Harness {
  React: any;
  renderToStaticMarkup: (el: any) => string;
  ActiveMeetingNavItem: any;
  Sidebar: any;
  MEETING_MODE_ENABLED: boolean;
  renderPill: (props: PillProps) => string;
}

/**
 * Loads the component graph with the meeting-mode flag pinned to `flag`.
 * `import.meta.env` is read at module scope, so the registry must be reset
 * before the import for the stub to take effect.
 */
const loadHarness = async (flag: string): Promise<Harness> => {
  vi.resetModules();
  vi.stubEnv("VITE_ENABLE_MEETING_MODE", flag);

  const { MEETING_MODE_ENABLED } = await import("./productNav");
  const { ActiveMeetingNavItem } = await import("./ActiveMeetingNavItem");
  const { Sidebar } = await import("../Sidebar");
  const React = (await import("react")).default;
  const { renderToStaticMarkup } = await import("react-dom/server");
  const i18next = (await import("i18next")).default;
  const { initReactI18next } = await import("react-i18next");

  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: "en",
      initImmediate: false,
      resources: TRANSLATION_RESOURCES,
    });
  }

  return {
    React,
    renderToStaticMarkup,
    ActiveMeetingNavItem,
    Sidebar,
    MEETING_MODE_ENABLED,
    renderPill: (props: PillProps) =>
      renderToStaticMarkup(React.createElement(ActiveMeetingNavItem, props)),
  };
};

afterAll(() => {
  vi.unstubAllEnvs();
  vi.resetModules();
});

// --- Exported formatter seam: class A status clock --------------------------

describe("formatMeetingDuration", () => {
  it("formats exactly at the one-hour boundary and on both sides of it (G1)", () => {
    expect(formatMeetingDuration(3598)).toBe("00:59:58");
    expect(formatMeetingDuration(3599)).toBe("00:59:59");
    expect(formatMeetingDuration(3600)).toBe("01:00:00");
    expect(formatMeetingDuration(3601)).toBe("01:00:01");
  });

  it("always shows the hours field (plan line 427: 00:42:18 at 42 minutes)", () => {
    expect(formatMeetingDuration(42 * 60 + 18)).toBe("00:42:18");
  });

  it("zero-pads and stays unbounded beyond 24 h (multi-day sessions)", () => {
    expect(formatMeetingDuration(26 * 3600 + 3 * 60 + 5)).toBe("26:03:05");
  });

  it("clamps genuine zero, negatives and non-finite input", () => {
    expect(formatMeetingDuration(0)).toBe("00:00:00");
    expect(formatMeetingDuration(-5)).toBe("00:00:00");
    expect(formatMeetingDuration(Number.NaN)).toBe("00:00:00");
  });
});

// --- Fake timers: manual clock + interval queue drained by advance() ---------

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

// --- Run A / G2 branch 1: emergency-rollback build (flag off) ---------------

describe("ActiveMeetingNavItem with meeting mode disabled (G2 branch 1)", () => {
  let harness: Harness;

  beforeAll(async () => {
    harness = await loadHarness("false");
  });

  it("pins the flag off for this branch", () => {
    expect(harness.MEETING_MODE_ENABLED).toBe(false);
  });

  it("renders nothing even when an active meeting is reported", () => {
    expect(
      harness.renderPill({
        meeting: { elapsedSeconds: 42 },
        onOpen: () => {},
      }),
    ).toBe("");
  });
});

// --- Run B: flag on (default since wave 1) ----------------------------------

describe("ActiveMeetingNavItem with meeting mode enabled", () => {
  let harness: Harness;

  beforeAll(async () => {
    harness = await loadHarness("true");
  });

  it("pins the flag on for this branch", () => {
    expect(harness.MEETING_MODE_ENABLED).toBe(true);
  });

  it("renders nothing when there is no active meeting (G2 branch 2)", () => {
    expect(harness.renderPill({ meeting: null, onOpen: () => {} })).toBe("");
  });

  it("renders the pinned pill with label, dot, tabular digits and a11y name", () => {
    const pillHtml = harness.renderPill({
      meeting: { elapsedSeconds: 600 },
      onOpen: () => {},
    });
    // i18n label with the formatted duration
    expect(pillHtml.includes("Recording · 00:10:00")).toBe(true);
    expect(pillHtml).toMatch(/<button type="button"/); // a real button
    // red status dot required by plan §6.3
    expect(pillHtml).toMatch(/aria-hidden="true"[^>]*class="[^"]*bg-red-500/);
    expect(pillHtml).toMatch(/tabular-nums/); // duration digits are tabular
    // accessible name must carry the duration
    expect(pillHtml).toMatch(/aria-label="Recording · 00:10:00"/);
    expect(pillHtml.includes("nav.")).toBe(false); // no raw keys leak
  });

  it("pins the pill directly below the Meetings nav entry", () => {
    const navigatedViews: string[] = [];
    const sidebarHtml = harness.renderToStaticMarkup(
      harness.React.createElement(harness.Sidebar, {
        activeView: "dictate",
        onViewChange: (view: string) => navigatedViews.push(view),
        activeMeeting: { elapsedSeconds: 600 },
      }),
    );
    const meetingsIdx = sidebarHtml.indexOf(">Meetings</span>");
    const pillIdx = sidebarHtml.indexOf("Recording · 00:10:00");
    expect(meetingsIdx).not.toBe(-1); // Meetings entry must render
    expect(pillIdx).not.toBe(-1); // pinned pill must render under Meetings
    expect(meetingsIdx < pillIdx).toBe(true);
  });

  it("ticks the duration across the hh:mm:ss boundary via an injected clock (G1)", () => {
    // The component is prop-driven by contract (navigation-seam-contract §6:
    // the future meetingStore rAF clock feeds elapsedSeconds). The fake-timer
    // pump below emulates that store: a 1 s interval derives elapsedSeconds
    // from the injected clock and re-renders the pill, which must follow.
    const timers = createFakeTimers();
    const displayed: string[] = [];
    let navigateCalls = 0;
    const START_SECONDS = 3598; // two ticks before the hh:mm:ss boundary
    let elapsedSeconds = START_SECONDS;
    const rerenderPill = () =>
      displayed.push(
        harness.renderPill({
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

    expect(displayed.length).toBe(5); // initial render plus four ticks
    expect(displayed[0]).toMatch(/Recording · 00:59:58/); // tick 0 pre-boundary
    expect(displayed[1]).toMatch(/Recording · 00:59:59/); // last second below 1 h
    expect(displayed[2]).toMatch(/Recording · 01:00:00/); // first second above 1 h
    expect(displayed[3]).toMatch(/Recording · 01:00:01/); // continues past boundary
    expect(displayed[4]).toMatch(/Recording · 01:00:02/); // ticker keeps running
    // ticking alone must never activate the navigation seam
    expect(navigateCalls).toBe(0);
  });

  it("invokes the navigate seam exactly once per activation (G3)", () => {
    const createdElements: any[] = [];
    const record =
      (original: any) =>
      (...args: any[]) => {
        const el = original(...args);
        createdElements.push(el);
        return el;
      };
    const originalCreateElement = harness.React.createElement;
    harness.React.createElement = record(originalCreateElement);
    sink.recorded = createdElements;

    let navigateCalls = 0;
    const onOpen = () => {
      navigateCalls += 1;
    };
    let activationHtml = "";
    try {
      activationHtml = harness.renderToStaticMarkup(
        harness.React.createElement(harness.ActiveMeetingNavItem, {
          meeting: { elapsedSeconds: 3600 },
          onOpen,
        }),
      );
    } finally {
      harness.React.createElement = originalCreateElement;
      sink.recorded = null;
    }

    expect(activationHtml.includes("Recording · 01:00:00")).toBe(true);
    // element factories must have been invoked during the render
    expect(createdElements.length).toBeGreaterThan(0);
    const buttonEl = [...createdElements]
      .reverse()
      .find((el) => el && el.type === "button");
    expect(buttonEl).toBeTruthy(); // the pill must render a button element
    // activation target must be a plain button
    expect(buttonEl.props.type).toBe("button");
    // the button onClick must be wired to the injected navigate seam
    expect(buttonEl.props.onClick).toBe(onOpen);
    buttonEl.props.onClick(); // one activation == one click
    expect(navigateCalls).toBe(1); // exactly one navigate per activation
  });
});
