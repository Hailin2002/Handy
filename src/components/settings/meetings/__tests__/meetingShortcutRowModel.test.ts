/**
 * End-to-end tests for one §6.9 shortcut-editor row, driven through
 * MeetingShortcutRowModel (the exact seam the React view uses). Covers:
 *
 * - G1: single-key capture is rejected with the named validation message;
 * - G2: identical Speak-binding candidates surface BOTH conflict locations;
 * - G3: a failing injected checker leaves the stored value unchanged and
 *   exposes an error state;
 * - G4: Escape during capture restores the previous binding.
 */
import { describe, expect, it, vi } from "vitest";

import {
  MEETING_SHORTCUT_STORAGE_PREFIX,
  SINGLE_KEY_ERROR_KEY,
  type MeetingShortcutOs,
  type MeetingShortcutStore,
  type ShortcutLocation,
} from "../meetingShortcutRules";
import {
  MeetingShortcutRowModel,
  type MeetingShortcutRowError,
} from "../meetingShortcutRowModel";
import { MEETING_SHORTCUT_ACTIONS } from "../meetingShortcutRules";

const START_STOP = MEETING_SHORTCUT_ACTIONS[0]; // defaultHotkey: ctrl+alt+space

const makeStore = (initial: Partial<Record<string, string | null>> = {}) => {
  const values = new Map<string, string | null>(
    Object.entries(initial) as [string, string | null][],
  );
  const store: MeetingShortcutStore & {
    writes: Array<{ action: string; value: string | null }>;
  } = {
    writes: [],
    read(action) {
      return values.get(`${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`) ?? null;
    },
    write(action, value) {
      store.writes.push({ action, value });
      values.set(`${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`, value);
    },
  };
  return store;
};

const makeOs = (
  ok: boolean,
  error = "already taken",
): MeetingShortcutOs & { calls: string[] } => ({
  calls: [],
  async tryRegister(hotkey) {
    this.calls.push(hotkey);
    return ok ? { ok: true } : { ok: false, error };
  },
});

const SPEAK_LOCATIONS: readonly ShortcutLocation[] = [
  {
    kind: "speak",
    id: "transcribe",
    label: "Transcribe Shortcut",
    hotkey: "ctrl+space",
  },
];

/** Flush pending microtasks so async commit pipelines settle. */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("MeetingShortcutRowModel", () => {
  it("G4: Escape during capture aborts and restores the previous binding", async () => {
    const previous = "ctrl+alt+m";
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: previous,
    });
    const os = makeOs(true);
    const suspend = vi.fn(async () => undefined);
    const resume = vi.fn(async () => undefined);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
      suspendBindings: suspend,
      resumeBindings: resume,
    });

    await model.beginCapture();
    expect(model.capturing).toBe(true);
    expect(suspend).toHaveBeenCalledTimes(1); // §6.9: Speak bindings suspended

    // The user presses Escape instead of completing a combination.
    const consumed = model.handleKeyDown({ key: "Escape" });

    expect(consumed).toBe(true);
    expect(model.capturing).toBe(false);
    expect(model.value).toBe(previous); // previous binding intact
    expect(model.error).toBeNull();
    expect(os.calls).toEqual([]); // nothing was probed or saved
    expect(store.writes).toEqual([]);
    expect(resume).toHaveBeenCalledTimes(1); // bindings released again
  });

  it("G1: committing a single key shows the named validation message and stores nothing", async () => {
    const previous = "ctrl+alt+space";
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: previous,
    });
    const os = makeOs(true);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    await model.beginCapture();
    model.handleKeyDown({ key: " ", code: "Space" }); // plain Space, no modifier

    await flush();

    const error: MeetingShortcutRowError | null = model.error;
    expect(error?.kind).toBe("invalid");
    expect(error && error.kind === "invalid" && error.errorKey).toBe(
      SINGLE_KEY_ERROR_KEY,
    );
    expect(SINGLE_KEY_ERROR_KEY).toContain(".errors.");
    expect(model.value).toBe(previous); // unchanged
    expect(store.writes).toEqual([]);
    expect(os.calls).toEqual([]);
    expect(model.capturing).toBe(false); // capture ended after the rejection
  });

  it("modifier-only presses are swallowed and keep capture alive", async () => {
    const store = makeStore();
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os: makeOs(true),
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    await model.beginCapture();
    expect(model.handleKeyDown({ key: "Control", code: "ControlLeft" })).toBe(
      true,
    );
    expect(model.capturing).toBe(true); // still listening for the full combo

    model.cancelCapture(); // user presses Escape afterwards
    expect(model.capturing).toBe(false);
  });

  it("G2: an identical Speak binding is blocked with BOTH conflict locations exposed at once", async () => {
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}pauseResume`]: "ctrl+shift+space",
    });
    const conflictLocations: readonly ShortcutLocation[] = [
      {
        kind: "speak",
        id: "transcribe_with_post_process",
        label: "Post-Processing Hotkey",
        hotkey: "ctrl+shift_left+space", // same binding, different spelling
      },
      {
        kind: "meeting",
        id: "pauseResume",
        label: "Pause/resume meeting",
        hotkey: "ctrl+shift+space",
      },
    ];
    const os = makeOs(true);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => conflictLocations,
    });

    await model.beginCapture();
    model.handleKeyDown({
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
    });

    await flush();

    const error = model.error;
    if (!error || error.kind !== "conflict") {
      throw new Error(`expected conflict error, got ${JSON.stringify(error)}`);
    }
    // Both locations are on the single error state simultaneously…
    expect(error.conflicts.map((c) => `${c.kind}:${c.id}`)).toEqual([
      "speak:transcribe_with_post_process",
      "meeting:pauseResume",
    ]);
    // …with their human-readable labels for display.
    expect(error.conflicts.map((c) => c.label)).toEqual([
      "Post-Processing Hotkey",
      "Pause/resume meeting",
    ]);
    expect(store.read("startStop")).toBeNull(); // nothing saved
    expect(os.calls).toEqual([]);
  });

  it("G3: a failing checker leaves the stored value unchanged and raises an error state", async () => {
    const previous = "ctrl+alt+space";
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: previous,
    });
    const os = makeOs(false, "reserved by the system");
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    await model.beginCapture();
    model.handleKeyDown({
      key: "m",
      code: "KeyM",
      ctrlKey: true,
      altKey: true,
    });

    await flush();

    const error = model.error;
    expect(error?.kind).toBe("os");
    expect(error && error.kind === "os" && error.error).toBe(
      "reserved by the system",
    );
    expect(os.calls).toEqual(["ctrl+alt+m"]); // probe ran before saving…
    expect(store.read("startStop")).toBe(previous); // …and the store kept the old value
    expect(store.writes).toEqual([]);
  });

  it("a valid combination passes every check and is persisted under settings.meetings.shortcut.*", async () => {
    const store = makeStore();
    const os = makeOs(true);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    await model.beginCapture();
    model.handleKeyDown({
      key: "m",
      code: "KeyM",
      ctrlKey: true,
      altKey: true,
    });

    await flush();

    expect(model.error).toBeNull();
    expect(model.value).toBe("ctrl+alt+m");
    expect(store.writes).toEqual([
      { action: "startStop", value: "ctrl+alt+m" },
    ]);
    expect(store.read("startStop")).toBe("ctrl+alt+m");
  });

  it("Zurücksetzen restores the §6.9 default through the full save pipeline", async () => {
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: "ctrl+alt+f5",
    });
    const os = makeOs(true);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    await model.resetToDefault();

    expect(model.value).toBe(START_STOP.defaultHotkey); // ctrl+alt+space
    expect(os.calls).toEqual(["ctrl+alt+space"]); // OS probed before saving
    expect(store.writes.map((w) => w.value)).toEqual(["ctrl+alt+space"]);
  });

  it("Deaktivieren clears the binding without any OS round-trip", () => {
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: "ctrl+alt+space",
    });
    const os = makeOs(false);
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    model.disable();

    expect(model.value).toBeNull();
    expect(store.writes).toEqual([{ action: "startStop", value: null }]);
    expect(os.calls).toEqual([]);
  });

  it("Shortcut testen probes the current binding without touching the store", async () => {
    const store = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: "ctrl+alt+space",
    });
    const failing = makeOs(false, "taken by Teams");
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os: failing,
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });

    const ok = await model.testCurrent();

    expect(ok).toBe(false);
    expect(failing.calls).toEqual(["ctrl+alt+space"]);
    expect(model.testFeedback).toEqual({
      ok: false,
      hotkey: "ctrl+alt+space",
      error: "taken by Teams",
    });
    expect(store.writes).toEqual([]); // a failed test never rewrites the binding
  });

  it("only one row captures at a time (editor-wide capture lock)", async () => {
    let acquired = false;
    const captureLock = {
      acquire: () => {
        if (acquired) return false;
        acquired = true;
        return true;
      },
      release: () => {
        acquired = false;
      },
    };
    const makeRow = (store: ReturnType<typeof makeStore>) =>
      new MeetingShortcutRowModel({
        action: START_STOP,
        os: makeOs(true),
        store,
        getConflictLocations: () => SPEAK_LOCATIONS,
        captureLock,
      });

    const first = makeRow(makeStore());
    const second = makeRow(makeStore());

    await first.beginCapture();
    await second.beginCapture(); // must be refused while the first captures

    expect(first.capturing).toBe(true);
    expect(second.capturing).toBe(false);

    first.cancelCapture(); // releases the lock…
    expect(first.capturing).toBe(false);

    await second.beginCapture(); // …so the next row can take it
    expect(second.capturing).toBe(true);
  });

  it("notifies subscribers on every observable change", async () => {
    const store = makeStore();
    const model = new MeetingShortcutRowModel({
      action: START_STOP,
      os: makeOs(true),
      store,
      getConflictLocations: () => SPEAK_LOCATIONS,
    });
    let versions = 0;
    model.subscribe(() => {
      versions += 1;
    });
    const baseline = model.getVersion();

    await model.beginCapture();
    model.handleKeyDown({ key: "f9", code: "F9", shiftKey: true });
    await flush();

    expect(model.getVersion()).toBeGreaterThan(baseline);
    expect(versions).toBeGreaterThan(0);
    expect(model.value).toBe("shift+f9");
  });
});
