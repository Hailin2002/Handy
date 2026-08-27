// Keyboard contract + ARIA wiring of the §6.13 detail tabs. The navigation
// rules are pure functions, so they are asserted directly; the markup is
// checked through static rendering (the repo has no DOM test environment).
import { beforeAll, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import {
  MEETING_DETAIL_TABS,
  MeetingDetailPanel,
  MeetingDetailTabs,
  edgeTab,
  selectableTabs,
  stepTab,
  tabLabelKey,
  type MeetingDetailTab,
} from "./MeetingDetailTabs";

beforeAll(async () => {
  await i18next.use(initReactI18next).init({
    lng: "en",
    resources: {
      en: {
        translation: {
          meeting: {
            detail: {
              tabs: {
                label: "Meeting sections",
                overview: "Overview",
                transcript: "Transcript",
                audio: "Audio",
                processing: "Processing",
              },
            },
          },
        },
      },
    },
  });
});

const ALL = [...MEETING_DETAIL_TABS];

describe("tab navigation rules", () => {
  it("keeps the plan's tab order", () => {
    expect(ALL).toEqual(["overview", "transcript", "audio", "processing"]);
  });

  it("steps forward and backward one tab at a time", () => {
    expect(stepTab("overview", 1, ALL)).toBe("transcript");
    expect(stepTab("transcript", -1, ALL)).toBe("overview");
  });

  it("wraps around at both ends", () => {
    expect(stepTab("processing", 1, ALL)).toBe("overview");
    expect(stepTab("overview", -1, ALL)).toBe("processing");
  });

  it("jumps to the ends for Home and End", () => {
    expect(edgeTab("first", ALL)).toBe("overview");
    expect(edgeTab("last", ALL)).toBe("processing");
  });

  it("skips a tab that has nothing to show", () => {
    const reachable = selectableTabs({ audio: "no audio stored" });
    expect(reachable).toEqual(["overview", "transcript", "processing"]);
    expect(stepTab("transcript", 1, reachable)).toBe("processing");
    expect(edgeTab("last", reachable)).toBe("processing");
  });

  it("steps from the first reachable tab when the active one is unreachable", () => {
    const reachable = selectableTabs({ audio: "no audio stored" });
    // `audio` is active but disabled; the component corrects this on the next
    // render, and meanwhile a keypress must not return `undefined`.
    expect(stepTab("audio", 1, reachable)).toBe("transcript");
  });

  it("returns null rather than guessing when nothing is reachable", () => {
    expect(stepTab("overview", 1, [])).toBeNull();
    expect(edgeTab("first", [])).toBeNull();
  });
});

describe("tablist markup", () => {
  const markup = (
    active: MeetingDetailTab,
    disabled?: Partial<Record<MeetingDetailTab, string>>,
  ) =>
    renderToStaticMarkup(
      <MeetingDetailTabs
        active={active}
        onChange={() => undefined}
        disabled={disabled}
      />,
    );

  it("labels the strip and every tab", () => {
    const html = markup("overview");
    expect(html).toContain('role="tablist"');
    expect(html).toContain('aria-label="Meeting sections"');
    for (const tab of ALL) {
      expect(html).toContain(`id="meeting-tab-${tab}"`);
    }
  });

  it("marks exactly one tab selected and points it at its panel", () => {
    const html = markup("transcript");
    expect(html.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(html).toContain('aria-controls="meeting-panel-transcript"');
  });

  it("keeps a single tab stop for the whole strip", () => {
    const html = markup("overview");
    expect(html.match(/tabindex="0"/g)).toHaveLength(1);
    expect(html.match(/tabindex="-1"/g)).toHaveLength(ALL.length - 1);
  });

  it("renders an empty tab as disabled with its reason, never hidden", () => {
    const html = markup("overview", { audio: "no audio stored" });
    expect(html).toContain("Audio");
    expect(html).toContain('title="no audio stored"');
    expect(html).toContain("disabled");
  });

  it("ties each panel back to its tab", () => {
    const html = renderToStaticMarkup(
      <MeetingDetailPanel tab="audio" active="audio">
        <span>content</span>
      </MeetingDetailPanel>,
    );
    expect(html).toContain('role="tabpanel"');
    expect(html).toContain('id="meeting-panel-audio"');
    expect(html).toContain('aria-labelledby="meeting-tab-audio"');
  });

  it("renders nothing for an inactive panel", () => {
    expect(
      renderToStaticMarkup(
        <MeetingDetailPanel tab="audio" active="overview">
          <span>content</span>
        </MeetingDetailPanel>,
      ),
    ).toBe("");
  });

  it("derives every label key from the tab id", () => {
    expect(ALL.map(tabLabelKey)).toEqual([
      "meeting.detail.tabs.overview",
      "meeting.detail.tabs.transcript",
      "meeting.detail.tabs.audio",
      "meeting.detail.tabs.processing",
    ]);
  });
});
