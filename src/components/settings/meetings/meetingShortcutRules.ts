/**
 * Pure rules and seams for the §6.9 meeting shortcut editor
 * (docs/meeting-mode-plan.md, "Regeln des Shortcut-Editors").
 *
 * Everything decision-relevant lives here so it can be tested without a DOM:
 * minimum modifier+key validation, conflict detection against Speak
 * bindings, the injected OS registration/checker seam, and the save
 * pipeline that only ever touches the store after every check passed.
 *
 * Persistence keys are exactly `settings.meetings.shortcut.<action>` as
 * mandated by the leaf. The Rust AppSettings struct has no `meetings`
 * section yet, so this module persists through an injectable store whose
 * default implementation uses localStorage. When the backend gains a real
 * `meetings.shortcut` setting, only `createDefaultMeetingShortcutStore`
 * needs to be repointed — see the operator handoff in MeetingShortcutEditor.
 */

/** Meeting shortcut actions from the §6.9 table (dictation stays untouched). */
export type MeetingShortcutActionId = "startStop" | "pauseResume";

export interface MeetingShortcutActionDef {
  id: MeetingShortcutActionId;
  /** i18n key of the human-readable action name. */
  nameKey: string;
  /** i18n key of the one-sentence behavior description. */
  descriptionKey: string;
  /**
   * Canonical default hotkey (`null` = unassigned by default, per §6.9 for
   * "Meeting pausieren/fortsetzen").
   */
  defaultHotkey: string | null;
}

export const MEETING_SHORTCUT_ACTIONS: readonly MeetingShortcutActionDef[] = [
  {
    id: "startStop",
    nameKey: "settings.meetings.shortcut.actions.startStop.name",
    descriptionKey: "settings.meetings.shortcut.actions.startStop.description",
    defaultHotkey: "ctrl+alt+space",
  },
  {
    id: "pauseResume",
    nameKey: "settings.meetings.shortcut.actions.pauseResume.name",
    descriptionKey:
      "settings.meetings.shortcut.actions.pauseResume.description",
    defaultHotkey: null,
  },
] as const;

/** Exact persistence-key prefix mandated by the leaf definition. */
export const MEETING_SHORTCUT_STORAGE_PREFIX = "settings.meetings.shortcut.";

// ---------------------------------------------------------------------------
// Hotkey normalization
// ---------------------------------------------------------------------------

const MODIFIER_ALIASES: Record<string, string> = {
  ctrl: "ctrl",
  control: "ctrl",
  shift: "shift",
  alt: "alt",
  option: "alt",
  meta: "meta",
  cmd: "meta",
  command: "meta",
  super: "meta",
  win: "meta",
};

/** Canonical modifier order used for comparisons and storage. */
const MODIFIER_ORDER = ["ctrl", "shift", "alt", "meta"] as const;

/**
 * Normalize any hotkey spelling (Speak bindings use `option_left+shift+x`,
 * the meeting editor produces `ctrl+alt+space`) into the canonical
 * `<modifiers in MODIFIER_ORDER order>+<key>` form. Returns null when the
 * string contains no usable key or more than one non-modifier key.
 */
export function normalizeHotkey(
  hotkey: string | null | undefined,
): string | null {
  if (!hotkey) return null;
  const parts = hotkey
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part.length > 0);
  const modifiers = new Set<string>();
  let key: string | null = null;
  const aliasOf = (part: string): string | undefined =>
    MODIFIER_ALIASES[part] ??
    // Speak stores side-specific modifiers such as `option_left`;
    // collapse them onto their base modifier so both spellings compare equal.
    (part.endsWith("_left") || part.endsWith("_right")
      ? MODIFIER_ALIASES[part.slice(0, part.lastIndexOf("_"))]
      : undefined);
  for (const part of parts) {
    const alias = aliasOf(part);
    if (alias) {
      modifiers.add(alias);
    } else if (key === null) {
      key = part;
    } else {
      // More than one non-modifier key cannot be registered meaningfully.
      return null;
    }
  }
  if (key === null) return null;
  const ordered = MODIFIER_ORDER.filter((m) => modifiers.has(m));
  return [...ordered, key].join("+");
}

// ---------------------------------------------------------------------------
// Validation (§6.9: minimum modifier plus key) — acceptance gate G1
// ---------------------------------------------------------------------------

export type CandidateValidation =
  | { ok: true; normalized: string }
  | { ok: false; errorKey: string };

/** i18n key of the named single-key rejection message (gate G1). */
export const SINGLE_KEY_ERROR_KEY =
  "settings.meetings.shortcut.errors.singleKey";

export function validateCandidate(
  candidate: string | null | undefined,
): CandidateValidation {
  const normalized = normalizeHotkey(candidate);
  if (normalized === null) {
    return {
      ok: false,
      errorKey: SINGLE_KEY_ERROR_KEY,
    };
  }
  const modifierCount =
    normalized.split("+").length - 1; /* total minus the trailing key */
  if (modifierCount < 1) {
    // e.g. plain "space" — §6.9 forbids single-key meeting shortcuts.
    return { ok: false, errorKey: SINGLE_KEY_ERROR_KEY };
  }
  return { ok: true, normalized };
}

// ---------------------------------------------------------------------------
// Conflict detection (identical Speak/meeting bindings) — acceptance gate G2
// ---------------------------------------------------------------------------

/** One place a shortcut can collide with, already localized for display. */
export interface ShortcutLocation {
  kind: "meeting" | "speak";
  /** Meeting action id or Speak binding id — unique within its kind. */
  id: string;
  /** Display label of the conflicting action, resolved by the caller. */
  label: string;
  /** Current hotkey at that location (any spelling). */
  hotkey: string | null;
}

export function findConflicts(
  normalizedCandidate: string,
  locations: readonly ShortcutLocation[],
): ShortcutLocation[] {
  return locations.filter(
    (location) =>
      location.hotkey !== null &&
      normalizeHotkey(location.hotkey) === normalizedCandidate,
  );
}

// ---------------------------------------------------------------------------
// Injected OS registration/checker seam — acceptance gate G3
// ---------------------------------------------------------------------------

export type RegistrationCheck = { ok: true } | { ok: false; error: string };

/**
 * Seam the editor uses to test OS registration *before* saving. Tests inject
 * deterministic fakes; production injects the Tauri global-shortcut plugin.
 */
export interface MeetingShortcutOs {
  /**
   * Probe whether the OS would accept registering `hotkey`. Must be
   * residue-free: a successful probe must leave nothing registered.
   */
  tryRegister(hotkey: string): Promise<RegistrationCheck>;
}

/**
 * Production seam backed by the Tauri global-shortcut plugin: actually
 * register with the OS and immediately unregister again. If the OS refuses
 * (already taken, reserved by the system, …) the thrown error becomes the
 * rejection reason. Imported dynamically so node-side tests of this module
 * never resolve the webview-only plugin.
 */
export const createTauriMeetingShortcutOs = (): MeetingShortcutOs => ({
  async tryRegister(hotkey: string): Promise<RegistrationCheck> {
    try {
      const plugin = await import("@tauri-apps/plugin-global-shortcut");
      await plugin.register(hotkey, () => {});
      await plugin.unregister(hotkey);
      return { ok: true };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  },
});

// ---------------------------------------------------------------------------
// Injected persistence seam (keys: settings.meetings.shortcut.<action>)
// ---------------------------------------------------------------------------

export interface MeetingShortcutStore {
  read(action: MeetingShortcutActionId): string | null;
  write(action: MeetingShortcutActionId, value: string | null): void;
}

interface MinimalStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/**
 * Default store. Uses localStorage when available (webview) and falls back
 * to an in-process map otherwise (node tests, SSR-ish contexts), so reads
 * never crash outside the app.
 */
export const createDefaultMeetingShortcutStore = (
  storage: MinimalStorage | null = typeof globalThis !== "undefined"
    ? (globalThis.localStorage ?? null)
    : null,
): MeetingShortcutStore => {
  const memory = new Map<string, string>();
  const keyOf = (action: MeetingShortcutActionId) =>
    `${MEETING_SHORTCUT_STORAGE_PREFIX}${action}`;
  return {
    read(action) {
      const raw = storage
        ? storage.getItem(keyOf(action))
        : (memory.get(keyOf(action)) ?? null);
      return raw === undefined || raw === null ? null : raw;
    },
    write(action, value) {
      if (storage) {
        if (value === null) storage.removeItem(keyOf(action));
        else storage.setItem(keyOf(action), value);
      } else if (value === null) {
        memory.delete(keyOf(action));
      } else {
        memory.set(keyOf(action), value);
      }
    },
  };
};

// ---------------------------------------------------------------------------
// Save pipeline — a non-registerable shortcut is never saved as success
// ---------------------------------------------------------------------------

export type MeetingShortcutSaveOutcome =
  | { status: "saved"; action: MeetingShortcutActionId; value: string }
  | { status: "invalid"; action: MeetingShortcutActionId; errorKey: string }
  | {
      status: "conflict";
      action: MeetingShortcutActionId;
      conflicts: ShortcutLocation[];
    }
  | {
      status: "os-rejected";
      action: MeetingShortcutActionId;
      error: string;
    };

/**
 * Validate → conflict-check → probe OS registration → persist. The store is
 * written strictly last: every rejection path leaves the previously stored
 * value byte-for-byte unchanged (acceptance gate G3).
 *
 * `conflictLocations` must already exclude the row being edited itself so
 * re-saving its own current shortcut is not flagged as self-conflict.
 */
export async function commitMeetingShortcut(input: {
  action: MeetingShortcutActionId;
  candidate: string;
  conflictLocations: readonly ShortcutLocation[];
  os: MeetingShortcutOs;
  store: MeetingShortcutStore;
}): Promise<MeetingShortcutSaveOutcome> {
  const validation = validateCandidate(input.candidate);
  if (!validation.ok) {
    return {
      status: "invalid",
      action: input.action,
      errorKey: validation.errorKey,
    };
  }

  const conflicts = findConflicts(
    validation.normalized,
    input.conflictLocations,
  );
  if (conflicts.length > 0) {
    return { status: "conflict", action: input.action, conflicts };
  }

  const check = await input.os.tryRegister(validation.normalized);
  if (!check.ok) {
    return { status: "os-rejected", action: input.action, error: check.error };
  }

  input.store.write(input.action, validation.normalized);
  return {
    status: "saved",
    action: input.action,
    value: validation.normalized,
  };
}
