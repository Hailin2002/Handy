/**
 * Framework-free interaction model for one row of the §6.9 meeting
 * shortcut editor (docs/meeting-mode-plan.md, "Regeln des
 * Shortcut-Editors").
 *
 * The React component (MeetingShortcutEditor.tsx) is a thin adapter that
 * forwards key events and clicks into this model and renders its
 * observable snapshot, so every acceptance-relevant behavior can be
 * proven without a DOM:
 *
 * - G1: a single-key candidate is rejected with the named validation
 *   message (`SINGLE_KEY_ERROR_KEY`) and never stored;
 * - G2: an identical Speak-binding candidate is blocked and BOTH conflict
 *   locations are surfaced simultaneously on the error state;
 * - G3: an injected checker that fails leaves the stored value unchanged
 *   byte-for-byte and exposes an `os` error state;
 * - G4: Escape during capture only cancels — nothing is written, so the
 *   previous binding stays intact.
 */

import {
  SINGLE_KEY_ERROR_KEY,
  commitMeetingShortcut,
  type MeetingShortcutActionDef,
  type MeetingShortcutOs,
  type MeetingShortcutStore,
  type ShortcutLocation,
} from "./meetingShortcutRules";
import {
  HotkeyCaptureController,
  type CapturedKeyEvent,
} from "./hotkeyCapture";

/** Error state rendered by the view; discriminated by rejection reason. */
export type MeetingShortcutRowError =
  | { kind: "invalid"; errorKey: string }
  | { kind: "conflict"; conflicts: readonly ShortcutLocation[] }
  | { kind: "os"; error: string };

/** Result of the last "Shortcut testen" probe. */
export interface TestShortcutFeedback {
  ok: boolean;
  hotkey: string;
  /** Raw OS rejection reason when `ok` is false. */
  error?: string;
}

export interface MeetingShortcutRowDeps {
  action: MeetingShortcutActionDef;
  os: MeetingShortcutOs;
  store: MeetingShortcutStore;
  /**
   * Returns the locations a new binding must not collide with. Must
   * already exclude this row's own action to avoid self-conflicts.
   */
  getConflictLocations: () => readonly ShortcutLocation[];
  suspendBindings?: () => Promise<unknown>;
  resumeBindings?: () => Promise<unknown>;
  /**
   * Optional editor-wide lock ensuring only one row captures at a time
   * (§6.9: during capture the other Speak shortcuts are suspended, which
   * only makes sense for exactly one active capture).
   */
  captureLock?: { acquire: () => boolean; release: () => void };
}

export class MeetingShortcutRowModel {
  private capture: HotkeyCaptureController | null = null;
  private _capturing = false;
  private _value: string | null;
  private _error: MeetingShortcutRowError | null = null;
  private _testing = false;
  private _testFeedback: TestShortcutFeedback | null = null;
  private _version = 0;

  private readonly listeners = new Set<() => void>();

  constructor(private readonly deps: MeetingShortcutRowDeps) {
    this._value = deps.store.read(deps.action.id);
  }

  // --- Observable snapshot (useSyncExternalStore-compatible) ---------------

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** Monotonic change counter; the only snapshot React compares. */
  getVersion = (): number => this._version;

  get value(): string | null {
    return this._value;
  }

  get capturing(): boolean {
    return this._capturing;
  }

  get error(): MeetingShortcutRowError | null {
    return this._error;
  }

  get testing(): boolean {
    return this._testing;
  }

  get testFeedback(): TestShortcutFeedback | null {
    return this._testFeedback;
  }

  /** Named single-key rejection message shared by gate G1 paths. */
  static get singleKeyErrorKey(): string {
    return SINGLE_KEY_ERROR_KEY;
  }

  private emit(): void {
    this._version += 1;
    for (const listener of this.listeners) listener();
  }

  // --- Capture lifecycle ---------------------------------------------------

  /** Keycap click: start capturing. Speak bindings are suspended first. */
  beginCapture = async (): Promise<void> => {
    if (this._capturing) return;
    if (this.deps.captureLock && !this.deps.captureLock.acquire()) {
      return; // another row is already capturing
    }
    try {
      await this.deps.suspendBindings?.();
    } catch (error) {
      console.error("Failed to suspend Speak bindings:", error);
    }
    this.capture = new HotkeyCaptureController({
      onCancel: () => {
        // G4: abort path — no write ever happened, so the previous
        // binding is restored by simply leaving `_value` untouched.
        this.finishCapture(/* resume */ true);
      },
      onCommit: (rawHotkey) => {
        void this.commitCandidate(rawHotkey, /* fromCapture */ true);
      },
    });
    this.capture.begin();
    this._capturing = true;
    this._error = null;
    this._testFeedback = null;
    this.emit();
  };

  /**
   * Feed one keydown in while capturing. Returns true when consumed.
   * Escape routes to `cancelCapture`; any full combination commits into
   * the save pipeline.
   */
  handleKeyDown(event: CapturedKeyEvent): boolean {
    if (!this.capture) return false;
    return this.capture.handleKeyDown(event);
  }

  /** Explicit cancel (Escape); previous binding stays untouched (G4). */
  cancelCapture(): void {
    this.capture?.cancel();
  }

  private finishCapture(resume: boolean): void {
    const wasCapturing = this._capturing;
    this.capture = null;
    this._capturing = false;
    if (resume && wasCapturing) {
      this.deps.resumeBindings?.().catch((error: unknown) => {
        console.error("Failed to resume Speak bindings:", error);
      });
    }
    if (wasCapturing) {
      this.deps.captureLock?.release();
    }
    this.emit();
  }

  // --- Save pipeline (validate → conflicts → OS probe → store) -------------

  /**
   * Run the full §6.9 save pipeline for `raw`. The store is written
   * strictly inside `commitMeetingShortcut`, after every check passed —
   * every rejection path leaves it untouched (G1/G2/G3).
   */
  commitCandidate = async (raw: string, fromCapture = false): Promise<void> => {
    const outcome = await commitMeetingShortcut({
      action: this.deps.action.id,
      candidate: raw,
      conflictLocations: this.deps.getConflictLocations(),
      os: this.deps.os,
      store: this.deps.store,
    });

    switch (outcome.status) {
      case "saved":
        this._value = outcome.value;
        break;
      case "invalid":
        this._error = { kind: "invalid", errorKey: outcome.errorKey };
        break;
      case "conflict":
        this._error = { kind: "conflict", conflicts: outcome.conflicts };
        break;
      case "os-rejected":
        this._error = { kind: "os", error: outcome.error };
        break;
    }
    if (fromCapture) {
      this.finishCapture(/* resume */ true);
    } else {
      this.emit();
    }
  };

  // --- Visible actions (§6.9: Zurücksetzen / Deaktivieren / Testen) --------

  /**
   * Zurücksetzen: restore the §6.9 default through the same save pipeline.
   * An unassigned-by-default action (defaultHotkey null) disables instead.
   */
  resetToDefault = async (): Promise<void> => {
    if (this._capturing) this.finishCapture(false);
    const fallback = this.deps.action.defaultHotkey;
    if (fallback === null) {
      this.disable();
      return;
    }
    await this.commitCandidate(fallback);
  };

  /** Deaktivieren: remove the binding; always succeeds locally. */
  disable = (): void => {
    if (this._capturing) this.finishCapture(false);
    this.deps.store.write(this.deps.action.id, null);
    this._value = null;
    this._error = null;
    this._testFeedback = null;
    this.emit();
  };

  /** Shortcut testen: probe the OS registration of the current binding. */
  testCurrent = async (): Promise<boolean> => {
    if (!this._value || this._testing || this._capturing) return false;
    this._testing = true;
    this._testFeedback = null;
    this.emit();
    try {
      const check = await this.deps.os.tryRegister(this._value);
      this._testFeedback = check.ok
        ? { ok: true, hotkey: this._value }
        : { ok: false, hotkey: this._value, error: check.error };
      return check.ok;
    } finally {
      this._testing = false;
      this.emit();
    }
  };
}
