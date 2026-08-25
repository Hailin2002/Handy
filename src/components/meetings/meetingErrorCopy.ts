import type { MeetingErrorState } from "@/stores/meetingStore";

/**
 * Maps a redacted backend error code onto its copy keys.
 *
 * Plan §6.2 rule 8 forbids raw backend errors and demands that every error
 * names the next action. The copy catalog defines one `title`/`body` pair per
 * taxonomy code; unknown codes fall back to `meeting.errors.generic.*`, which
 * the i18n namespace contract makes a mandatory key (it may never be missing
 * while any `meeting.errors.*` key exists).
 */
export interface MeetingErrorCopy {
  /** i18n key list; i18next picks the first key that resolves. */
  titleKeys: string[];
  bodyKeys: string[];
  /** Error states that keep the recording alive render as warnings. */
  severity: "error" | "warning";
}

/** Codes that describe a degraded but still-running capture (plan §6.x). */
const NON_FATAL_CODES = new Set([
  "micLost",
  "systemLost",
  "cloudUnreachable",
  "backlog",
  "diskLow",
  "silence",
  "devicesChanged",
]);

export const meetingErrorCopy = (
  error: MeetingErrorState | null,
): MeetingErrorCopy | null => {
  if (!error) return null;
  const code = error.code || "generic";
  return {
    titleKeys: [
      `meeting.errors.${code}.title`,
      `meeting.warnings.${code}.title`,
      "meeting.errors.generic.title",
    ],
    bodyKeys: [
      `meeting.errors.${code}.body`,
      `meeting.warnings.${code}.body`,
      "meeting.errors.generic.body",
    ],
    severity: NON_FATAL_CODES.has(code) ? "warning" : "error",
  };
};
