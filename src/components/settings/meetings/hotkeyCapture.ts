/**
 * Key-capture state machine for the §6.9 meeting shortcut editor.
 *
 * Deliberately framework-free so the editor rules can be proven without a
 * DOM: the React component forwards `keydown` events into
 * {@link HotkeyCaptureController.handleKeyDown}, tests push plain objects.
 *
 * §6.9 rules implemented here:
 * - clicking the keycap begins capture (`begin`);
 * - `Escape` aborts the change — the callbacks only ever observe `onCancel`,
 *   nothing is written, so the previous binding stays intact (gate G4);
 * - modifier-only presses are swallowed while capturing (a meeting shortcut
 *   needs at least modifier + key, so they can never commit on their own).
 */

import { getKeyName } from "@/lib/utils/keyboard";

/** Structural subset of DOM KeyboardEvent, satisfiable from tests. */
export interface CapturedKeyEvent {
  key: string;
  code?: string;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
}

const MODIFIER_EVENT_KEYS = new Set([
  "Shift",
  "Control",
  "Alt",
  "AltGraph",
  "Meta",
]);

/**
 * Build the raw hotkey string ("ctrl+alt+space" shape) from a key event.
 * Returns null for modifier-only presses, which never complete a capture.
 */
export function buildRawHotkey(event: CapturedKeyEvent): string | null {
  if (MODIFIER_EVENT_KEYS.has(event.key)) return null;
  const keyName = getKeyName({
    code: event.code,
    key: event.key,
    keyCode: 0,
  } as unknown as KeyboardEvent);
  if (!keyName) return null;
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("ctrl");
  if (event.shiftKey) modifiers.push("shift");
  if (event.altKey) modifiers.push("alt");
  if (event.metaKey) modifiers.push("meta");
  return [...modifiers, keyName.toLowerCase()].join("+");
}

export interface HotkeyCaptureCallbacks {
  /** Capture aborted (Escape or explicit cancel); previous binding stands. */
  onCancel: () => void;
  /** A full combination was pressed; `rawHotkey` still needs validation. */
  onCommit: (rawHotkey: string) => void;
}

export class HotkeyCaptureController {
  private active = false;

  constructor(private readonly callbacks: HotkeyCaptureCallbacks) {}

  isActive(): boolean {
    return this.active;
  }

  begin(): void {
    this.active = true;
  }

  cancel(): void {
    if (!this.active) return;
    this.active = false;
    this.callbacks.onCancel();
  }

  /**
   * Feed one keydown in. Returns true when the event was consumed and
   * should not reach the rest of the page.
   */
  handleKeyDown(event: CapturedKeyEvent): boolean {
    if (!this.active) return false;
    if (event.key === "Escape") {
      this.cancel();
      return true;
    }
    const raw = buildRawHotkey(event);
    if (raw === null) {
      // Modifier-only press: keep listening, swallow the event.
      return true;
    }
    this.active = false;
    this.callbacks.onCommit(raw);
    return true;
  }
}
