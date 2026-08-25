/**
 * Maps a persisted `meeting_sessions.state` value (plan §9 line 1169) to its
 * i18n label key. States beyond the wave-1 set fall back to a generic label;
 * color is never the sole carrier (plan §6.8) — every chip renders text.
 */
export const meetingStateLabelKey = (state: string): string => {
  switch (state) {
    case "draft":
      return "meetings.stateDraft";
    case "recording":
    case "starting":
    case "paused":
      return "meetings.stateRecording";
    case "completed":
      return "meetings.stateCompleted";
    default:
      return "meetings.stateOther";
  }
};
