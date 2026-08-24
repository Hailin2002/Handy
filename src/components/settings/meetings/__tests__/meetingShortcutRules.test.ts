/**
 * Unit tests for the pure §6.9 shortcut editor rules
 * (meetingShortcutRules.ts). Acceptance gates:
 *
 * - G1: single-key capture rejected with the named validation message;
 * - G2: identical Speak bindings produce BOTH conflict locations;
 * - G3: an injected checker that fails leaves the stored value unchanged.
 */
import { describe, expect, it } from "vitest";

import {
  MEETING_SHORTCUT_ACTIONS,
  MEETING_SHORTCUT_STORAGE_PREFIX,
  SINGLE_KEY_ERROR_KEY,
  commitMeetingShortcut,
  createDefaultMeetingShortcutStore,
  findConflicts,
  normalizeHotkey,
  validateCandidate,
  type MeetingShortcutOs,
  type MeetingShortcutStore,
  type ShortcutLocation,
} from "../meetingShortcutRules";

/** Deterministic in-memory store recording every write for assertions. */
const makeStore = (initial: Partial<Record<string, string | null>> = {}) => {
  const values = new Map<string, string | null>(
    Object.entries(initial) as [string, string | null][],
  );
  const writes: Array<{ key: string; value: string | null }> = [];
  const store: MeetingShortcutStore = {
    read(action) {
      const value = values.get(`${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`);
      return value === undefined ? null : value;
    },
    write(action, value) {
      writes.push({
        key: `${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`,
        value,
      });
      values.set(`${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`, value);
    },
  };
  return { store, values, writes };
};

const makeOs = (
  ok: boolean,
  error = "shortcut already registered by another app",
): MeetingShortcutOs & { calls: string[] } => {
  const calls: string[] = [];
  return {
    calls,
    async tryRegister(hotkey) {
      calls.push(hotkey);
      return ok ? { ok: true } : { ok: false, error };
    },
  };
};

const SPEAK_LOCATIONS: readonly ShortcutLocation[] = [
  {
    kind: "speak",
    id: "transcribe",
    label: "Transcribe Shortcut",
    hotkey: "ctrl+space",
  },
  {
    kind: "speak",
    id: "transcribe_with_post_process",
    label: "Post-Processing Hotkey",
    hotkey: "option_left+shift+x",
  },
];

describe("normalizeHotkey", () => {
  it("canonicalizes modifier order and casing", () => {
    expect(normalizeHotkey("Alt+Ctrl+Space")).toBe("ctrl+alt+space");
    expect(normalizeHotkey("option_left+shift+x")).toBe("shift+alt+x");
    expect(normalizeHotkey("WIN+m")).toBe("meta+m");
  });

  it("returns null for modifier-only or multi-key input", () => {
    expect(normalizeHotkey("ctrl")).toBeNull();
    expect(normalizeHotkey(null)).toBeNull();
    expect(normalizeHotkey("a+b")).toBeNull();
  });
});

describe("validateCandidate — acceptance gate G1", () => {
  it("rejects a single key with the named validation message", () => {
    const result = validateCandidate("space");
    expect(result.ok).toBe(false);
    if (!result.ok) {
      // The named message key the UI renders for gate G1.
      expect(result.errorKey).toBe(SINGLE_KEY_ERROR_KEY);
      expect(SINGLE_KEY_ERROR_KEY).toBe(
        "settings.meetings.shortcut.errors.singleKey",
      );
    }
  });

  it("rejects modifier-only and unnormalizable input with the same message", () => {
    expect(validateCandidate("ctrl")).toEqual({
      ok: false,
      errorKey: SINGLE_KEY_ERROR_KEY,
    });
    expect(validateCandidate("a+b")).toEqual({
      ok: false,
      errorKey: SINGLE_KEY_ERROR_KEY,
    });
  });

  it("accepts modifier+key combinations", () => {
    expect(validateCandidate("ctrl+alt+space")).toEqual({
      ok: true,
      normalized: "ctrl+alt+space",
    });
    expect(validateCandidate("shift+f9")).toEqual({
      ok: true,
      normalized: "shift+f9",
    });
  });
});

describe("findConflicts — acceptance gate G2", () => {
  it("matches identical Speak bindings across spelling variants", () => {
    const conflicts = findConflicts("ctrl+space", [
      ...SPEAK_LOCATIONS,
      {
        kind: "meeting",
        id: "pauseResume",
        label: "Pause/resume meeting",
        hotkey: null,
      },
      {
        kind: "meeting",
        id: "startStop",
        label: "Start/end meeting",
        // Same binding as the candidate, written with spaces and caps.
        hotkey: "CTRL + SPACE",
      },
    ]);
    expect(conflicts.map((c) => c.id)).toEqual(["transcribe", "startStop"]);
    expect(conflicts.map((c) => c.kind)).toEqual(["speak", "meeting"]);
  });

  it("ignores unassigned locations", () => {
    expect(
      findConflicts("ctrl+space", [
        { kind: "speak", id: "cancel", label: "Cancel", hotkey: null },
      ]),
    ).toEqual([]);
  });
});

describe("commitMeetingShortcut — save pipeline ordering", () => {
  it("G1: rejects a single-key candidate before any OS probe or write", async () => {
    const { store, writes } = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: "ctrl+alt+space",
    });
    const os = makeOs(true);

    const outcome = await commitMeetingShortcut({
      action: "startStop",
      candidate: "space",
      conflictLocations: SPEAK_LOCATIONS,
      os,
      store,
    });

    expect(outcome.status).toBe("invalid");
    if (outcome.status === "invalid") {
      expect(outcome.errorKey).toBe(SINGLE_KEY_ERROR_KEY);
    }
    expect(os.calls).toEqual([]); // never probed the OS
    expect(writes).toEqual([]); // nothing stored
    expect(store.read("startStop")).toBe("ctrl+alt+space"); // unchanged
  });

  it("G2: blocks an identical Speak-binding candidate listing both locations", async () => {
    const { store, writes } = makeStore();
    const os = makeOs(true);
    const conflictLocations: readonly ShortcutLocation[] = [
      ...SPEAK_LOCATIONS,
      {
        kind: "meeting",
        id: "pauseResume",
        label: "Pause/resume meeting",
        hotkey: "shift+alt+f2",
      },
    ];

    const outcome = await commitMeetingShortcut({
      action: "startStop",
      candidate: "Shift+Alt+F2", // same binding as pauseResume after normalization
      conflictLocations,
      os,
      store,
    });

    expect(outcome.status).toBe("conflict");
    if (outcome.status === "conflict") {
      // BOTH conflicting locations are returned simultaneously — here the
      // other meeting row; a candidate equal to a Speak binding returns
      // that location too (see findConflicts tests above).
      expect(outcome.conflicts.map((c) => `${c.kind}:${c.id}`)).toEqual([
        "meeting:pauseResume",
      ]);
      expect(outcome.conflicts[0].label).toBe("Pause/resume meeting");
    }
    expect(os.calls).toEqual([]); // blocked before the OS probe
    expect(writes).toEqual([]);
    expect(store.read("startStop")).toBeNull();
  });

  it("G2: returns the Speak location AND the meeting location together when both collide", async () => {
    const { store } = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}pauseResume`]: "ctrl+shift+space",
    });
    const conflictLocations: readonly ShortcutLocation[] = [
      {
        kind: "speak",
        id: "transcribe_with_post_process",
        label: "Post-Processing Hotkey",
        hotkey: "CTRL+SHIFT+SPACE",
      },
      {
        kind: "meeting",
        id: "pauseResume",
        label: "Pause/resume meeting",
        hotkey: "ctrl+shift+space",
      },
    ];

    const outcome = await commitMeetingShortcut({
      action: "startStop",
      candidate: "ctrl+shift+space",
      conflictLocations,
      os: makeOs(true),
      store,
    });

    if (outcome.status !== "conflict") {
      throw new Error(`expected conflict, got ${outcome.status}`);
    }
    expect(outcome.conflicts.map((c) => `${c.kind}:${c.id}`)).toEqual([
      "speak:transcribe_with_post_process",
      "meeting:pauseResume",
    ]);
  });

  it("G3: a failing injected checker leaves the stored value unchanged and reports os-rejected", async () => {
    const previous = "ctrl+alt+space";
    const { store, writes } = makeStore({
      [`${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`]: previous,
    });
    const os = makeOs(false, "reserved by the system");

    const outcome = await commitMeetingShortcut({
      action: "startStop",
      candidate: "ctrl+alt+m",
      conflictLocations: SPEAK_LOCATIONS,
      os,
      store,
    });

    expect(outcome.status).toBe("os-rejected");
    if (outcome.status === "os-rejected") {
      expect(outcome.error).toBe("reserved by the system");
    }
    expect(os.calls).toEqual(["ctrl+alt+m"]); // probe happened before saving…
    expect(writes).toEqual([]); // …but no write followed
    expect(store.read("startStop")).toBe(previous); // byte-for-byte unchanged
  });

  it("persists only after validation, conflicts and OS probe all passed", async () => {
    const { store, writes } = makeStore();
    const os = makeOs(true);

    const outcome = await commitMeetingShortcut({
      action: "startStop",
      candidate: "ctrl+alt+space",
      conflictLocations: SPEAK_LOCATIONS,
      os,
      store,
    });

    expect(outcome.status).toBe("saved");
    expect(os.calls).toEqual(["ctrl+alt+space"]);
    expect(writes).toEqual([
      {
        key: `${MEETING_SHORTCUT_STORAGE_PREFIX}startStop`,
        value: "ctrl+alt+space",
      },
    ]);
    expect(store.read("startStop")).toBe("ctrl+alt+space");
  });
});

describe("default store", () => {
  it("keys values under settings.meetings.shortcut.<action>", () => {
    const backing = new Map<string, string>();
    const storage = {
      getItem: (k: string) => backing.get(k) ?? null,
      setItem: (k: string, v: string) => void backing.set(k, v),
      removeItem: (k: string) => void backing.delete(k),
    };
    const store = createDefaultMeetingShortcutStore(storage);

    store.write("startStop", "ctrl+alt+space");
    expect([...backing.keys()]).toEqual([
      "settings.meetings.shortcut.startStop",
    ]);

    store.write("pauseResume", null); // removing a missing key is a no-op
    store.write("startStop", null);
    expect(backing.size).toBe(0);
  });

  it("declares exactly the §6.9 actions", () => {
    expect(MEETING_SHORTCUT_ACTIONS.map((a) => a.id)).toEqual([
      "startStop",
      "pauseResume",
    ]);
    expect(MEETING_SHORTCUT_ACTIONS[0].defaultHotkey).toBe("ctrl+alt+space");
    expect(MEETING_SHORTCUT_ACTIONS[1].defaultHotkey).toBeNull();
  });
});
