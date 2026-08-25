import { invoke } from "@tauri-apps/api/core";

/**
 * Thin typed IPC helper for the meeting session lifecycle commands.
 *
 * These commands are specified in
 * `docs/meeting-mode-architecture/session-state-contract.md` (§2.2, §5) and
 * `docs/meeting-mode-architecture/startup-recovery-dataflow.md` (§5) but the
 * Rust side is still being built. `bindings.ts` is generated from the Rust
 * `#[tauri::command]` set, so it only carries the five wave-1 CRUD commands;
 * the lifecycle commands cannot appear there until the backend lands.
 *
 * Consequently the store invokes them by name and this module classifies the
 * failure modes, so a missing backend surfaces as an explainable UI state
 * (plan §6.2 rule 8: "Fehler erklären die nächste Handlung") instead of an
 * unhandled rejection.
 */

/** Command names as fixed by the contracts; single source for the UI side. */
export const MEETING_COMMANDS = {
  createDraft: "create_meeting_draft",
  preflight: "preflight_meeting",
  start: "start_meeting",
  pause: "pause_meeting",
  resume: "resume_meeting",
  stop: "stop_meeting",
  cancel: "cancel_meeting",
  snapshot: "get_meeting_snapshot",
  retryJobs: "retry_meeting_jobs",
  listRecoveries: "meeting_list_recoveries",
  resolveRecovery: "meeting_resolve_recovery",
} as const;

/** Event topics owned exclusively by `meetingStore` (FSC §3.2). */
export const MEETING_EVENTS = {
  sessionCreated: "meeting://session-created",
  sessionState: "meeting://session-state",
  sessionError: "meeting://session-error",
  levelUpdate: "meeting://level-update",
  transcriptDelta: "meeting://transcript-delta",
  jobsUpdated: "meeting://jobs-updated",
  summaryAvailable: "meeting://summary-available",
  sessionRecovered: "meeting://session-recovered",
  recoveryReport: "meeting://recovery-report",
} as const;

/** Raised when the backend does not (yet) expose the invoked command. */
export class MeetingCommandUnavailableError extends Error {
  readonly command: string;

  constructor(command: string, cause: string) {
    super(`Meeting command "${command}" is not available: ${cause}`);
    this.name = "MeetingCommandUnavailableError";
    this.command = command;
  }
}

/** Raised when the command exists but rejected. `message` is already redacted. */
export class MeetingCommandError extends Error {
  readonly command: string;

  constructor(command: string, message: string) {
    super(message);
    this.name = "MeetingCommandError";
    this.command = command;
  }
}

/**
 * Signatures Tauri (and the e2e mock) produce for a command that has no
 * registered handler. Matching on them is the only way to tell "backend not
 * built yet" apart from "backend refused", because both arrive as a rejected
 * promise carrying a string.
 */
const UNAVAILABLE_PATTERNS: readonly RegExp[] = [
  /unhandled ipc command/i,
  /not allowed by ACL/i,
  /command\s+.*\s+not found/i,
  /unknown command/i,
  /__TAURI_INTERNALS__/i,
  /window\.__TAURI/i,
  /is not a function/i,
];

const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

/**
 * Invokes a meeting command, mapping "no such command" to a dedicated error so
 * callers can degrade instead of crashing.
 */
export const meetingInvoke = async <T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> => {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const message = messageOf(error);
    if (UNAVAILABLE_PATTERNS.some((pattern) => pattern.test(message))) {
      throw new MeetingCommandUnavailableError(command, message);
    }
    throw new MeetingCommandError(command, message);
  }
};
