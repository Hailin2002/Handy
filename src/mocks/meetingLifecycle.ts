import type {
  MeetingPreflightReport,
  MeetingRecoveryOffer,
  MeetingRecoveryResolution,
  MeetingSnapshot,
} from "@/lib/types/meeting";
import type { MockBackend } from "./backend";

/**
 * Mock of the meeting session lifecycle (`MeetingManager` + recorder + STT
 * queue) for the Playwright journey.
 *
 * The Rust side of these commands does not exist yet, so this module is the
 * only place where the agreed contract behaviour is executable: it implements
 * the transition table of
 * `docs/meeting-mode-architecture/session-state-contract.md` §2.2 and emits
 * the three live channels of
 * `docs/meeting-mode-architecture/live-view-streaming-contract.md` §2 with
 * monotonically increasing `state_revision`s and no gaps (SSC O2), so the
 * frontend's revision guard is exercised exactly as it will be in production.
 *
 * Timings are deliberately short (100–400 ms rather than the real
 * multi-second stages) so the e2e journey stays fast while every intermediate
 * state is still observable.
 */

const TICK = {
  /** `starting` → `recording` (device open). */
  startup: 120,
  /** Level meter cadence; the real recorder throttles to 100 ms per source. */
  level: 100,
  /** First and second transcript delta after capture began. */
  firstSegment: 350,
  secondSegment: 700,
  /** `stopping` → `transcribing` (chunk finalisation). */
  finalize: 150,
  /** Transcription progress steps and the summary stage. */
  transcribeStep: 250,
  summarize: 250,
} as const;

export const DEFAULT_MOCK_PREFLIGHT: MeetingPreflightReport = {
  can_start: true,
  microphone_only_available: true,
  checks: [
    { id: "microphone", status: "ok", value: "Mock Microphone" },
    { id: "systemAudio", status: "ok", value: "Speakers / Headset" },
    { id: "transcription", status: "ok", value: "Local · Meeting model" },
    { id: "language", status: "ok", value: "English" },
    { id: "disk", status: "ok", value: "42 GB free" },
    { id: "permission", status: "ok", value: "Granted" },
  ],
};

/** Per-meeting simulation state held by the mock backend. */
export interface MeetingLifecycleState {
  meetingId: string;
  title: string;
  revision: number;
  state: MeetingSnapshot["state"];
  startedAt: number | null;
  elapsedBaseMs: number;
  microphoneOnly: boolean;
  summaryEnabled: boolean;
  segments: number;
  succeeded: number;
  eligible: number;
  timers: number[];
  levelTimer: number | null;
}

const SEGMENT_TEXTS = [
  "Welcome everyone, let us start with the budget review.",
  "We agreed to move the release date to the following week.",
  "Action item: send the updated forecast by Friday.",
];

const clearTimers = (lifecycle: MeetingLifecycleState) => {
  for (const handle of lifecycle.timers) clearTimeout(handle);
  lifecycle.timers = [];
  if (lifecycle.levelTimer !== null) {
    clearInterval(lifecycle.levelTimer);
    lifecycle.levelTimer = null;
  }
};

const snapshotOf = (lifecycle: MeetingLifecycleState): MeetingSnapshot => ({
  meeting_id: lifecycle.meetingId,
  title: lifecycle.title,
  state: lifecycle.state,
  state_revision: lifecycle.revision,
  elapsed_ms: lifecycle.elapsedBaseMs,
  running_since: lifecycle.state === "recording" ? lifecycle.startedAt : null,
  microphone_only: lifecycle.microphoneOnly,
  provider_kind: "local",
  provider_name: null,
  summary_enabled: lifecycle.summaryEnabled,
  transcript_head_sequence: lifecycle.segments,
  jobs: jobsPayload(lifecycle),
});

const jobsPayload = (lifecycle: MeetingLifecycleState) => {
  const outstanding = Math.max(0, lifecycle.eligible - lifecycle.succeeded);
  return {
    meeting_id: lifecycle.meetingId,
    per_source: {
      microphone: {
        queued: outstanding,
        running: outstanding > 0 ? 1 : 0,
        retry_wait: 0,
        failed: 0,
        succeeded: lifecycle.succeeded,
      },
    },
    total_backlog: outstanding,
    state_revision: lifecycle.revision,
    eligible_total: lifecycle.eligible,
    succeeded_total: lifecycle.succeeded,
    prepared_total: lifecycle.eligible,
    prepared_done: lifecycle.eligible,
  };
};

/**
 * Drives one transition: bumps the revision, mirrors the state into the
 * CRUD row and publishes the event. Persist-before-publish (SSC O1) is
 * modelled by mutating the row before emitting.
 */
const transition = (
  backend: MockBackend,
  lifecycle: MeetingLifecycleState,
  to: MeetingSnapshot["state"],
) => {
  const from = lifecycle.state;
  const now = Date.now();
  if (
    from === "recording" &&
    to !== "recording" &&
    lifecycle.startedAt !== null
  ) {
    lifecycle.elapsedBaseMs += Math.max(0, now - lifecycle.startedAt);
    lifecycle.startedAt = null;
  }
  if (to === "recording" && from !== "recording") {
    lifecycle.startedAt = now;
  }
  lifecycle.state = to;
  lifecycle.revision += 1;

  const row = backend.meetings.find((m) => m.id === lifecycle.meetingId);
  if (row) {
    row.state = to;
    row.updated_at = now;
    if (to === "recording" && !row.started_at) row.started_at = now;
    if (["completed", "cancelled", "stopped_unprocessed"].includes(to)) {
      row.ended_at = now;
    }
  }

  backend.emit?.("meeting://session-state", {
    meeting_id: lifecycle.meetingId,
    from,
    to,
    state_revision: lifecycle.revision,
    microphone_only: lifecycle.microphoneOnly,
  });
};

const later = (
  lifecycle: MeetingLifecycleState,
  delay: number,
  run: () => void,
) => {
  lifecycle.timers.push(setTimeout(run, delay) as unknown as number);
};

const startLevels = (
  backend: MockBackend,
  lifecycle: MeetingLifecycleState,
) => {
  if (lifecycle.levelTimer !== null) return;
  let phase = 0;
  lifecycle.levelTimer = setInterval(() => {
    phase += 1;
    // Deterministic, clearly non-zero levels so the meters are assertable.
    const mic = 0.35 + 0.25 * Math.abs(Math.sin(phase / 3));
    const system = 0.2 + 0.2 * Math.abs(Math.cos(phase / 4));
    backend.emit?.("meeting://level-update", {
      meeting_id: lifecycle.meetingId,
      source: "microphone",
      peak: mic,
      rms: mic * 0.7,
      at_timeline_ms: phase * TICK.level,
    });
    backend.emit?.("meeting://level-update", {
      meeting_id: lifecycle.meetingId,
      source: "system",
      peak: system,
      rms: system * 0.7,
      at_timeline_ms: phase * TICK.level,
    });
  }, TICK.level) as unknown as number;
};

const stopLevels = (lifecycle: MeetingLifecycleState) => {
  if (lifecycle.levelTimer === null) return;
  clearInterval(lifecycle.levelTimer);
  lifecycle.levelTimer = null;
};

const emitSegment = (
  backend: MockBackend,
  lifecycle: MeetingLifecycleState,
) => {
  const index = lifecycle.segments;
  if (index >= SEGMENT_TEXTS.length) return;
  lifecycle.segments += 1;
  lifecycle.eligible = Math.max(lifecycle.eligible, lifecycle.segments);
  lifecycle.succeeded = lifecycle.segments;
  backend.emit?.("meeting://transcript-delta", {
    meeting_id: lifecycle.meetingId,
    segments: [
      {
        id: `seg-${lifecycle.meetingId}-${lifecycle.segments}`,
        job_id: `job-${lifecycle.segments}`,
        source: "microphone",
        start_ms: index * 5_000,
        end_ms: (index + 1) * 5_000,
        sequence: lifecycle.segments,
        text: SEGMENT_TEXTS[index],
      },
    ],
    last_sequence: lifecycle.segments,
    state_revision: lifecycle.revision,
  });
};

/**
 * Handles the session lifecycle commands. Returns `undefined` when `command`
 * is none of them so the caller can fall through to the other handlers.
 */
export const handleMeetingLifecycleCommand = (
  backend: MockBackend,
  command: string,
  args: Record<string, unknown>,
): unknown | undefined => {
  const meetingId = typeof args.meetingId === "string" ? args.meetingId : "";
  const lifecycle = meetingId ? backend.lifecycles.get(meetingId) : undefined;

  switch (command) {
    case "preflight_meeting":
      return backend.config.meetingPreflight ?? DEFAULT_MOCK_PREFLIGHT;

    case "start_meeting": {
      const row = backend.meetings.find((m) => m.id === meetingId);
      const next: MeetingLifecycleState = {
        meetingId,
        title:
          typeof args.title === "string" && args.title
            ? args.title
            : (row?.title ?? "Meeting"),
        revision: 1,
        state: "starting",
        startedAt: null,
        elapsedBaseMs: 0,
        microphoneOnly: args.microphoneOnly === true,
        summaryEnabled: args.summaryEnabled !== false,
        segments: 0,
        succeeded: 0,
        eligible: SEGMENT_TEXTS.length,
        timers: [],
        levelTimer: null,
      };
      backend.lifecycles.set(meetingId, next);
      if (row) {
        row.state = "starting";
        row.title = next.title;
      }

      // starting → recording once the devices are open (SSC T03), then the
      // live channels begin.
      later(next, TICK.startup, () => {
        transition(backend, next, "recording");
        startLevels(backend, next);
      });
      later(next, TICK.firstSegment, () => emitSegment(backend, next));
      later(next, TICK.secondSegment, () => emitSegment(backend, next));

      return snapshotOf(next);
    }

    case "pause_meeting": {
      if (!lifecycle) return null;
      stopLevels(lifecycle);
      transition(backend, lifecycle, "paused");
      return null;
    }

    case "resume_meeting": {
      if (!lifecycle) return null;
      transition(backend, lifecycle, "recording");
      startLevels(backend, lifecycle);
      return null;
    }

    case "stop_meeting": {
      if (!lifecycle) return null;
      clearTimers(lifecycle);
      transition(backend, lifecycle, "stopping");

      // Only after the recorder reported durability may the session leave
      // `stopping` (SSC §3.3 step 2). Aborting changes the destination, never
      // the durability work (T15).
      later(lifecycle, TICK.finalize, () => {
        if (args.abort === true) {
          transition(backend, lifecycle, "stopped_unprocessed");
          return;
        }
        transition(backend, lifecycle, "transcribing");
        backend.emit?.("meeting://jobs-updated", jobsPayload(lifecycle));

        later(lifecycle, TICK.transcribeStep, () => {
          lifecycle.succeeded = lifecycle.eligible;
          backend.emit?.("meeting://jobs-updated", jobsPayload(lifecycle));
          if (lifecycle.summaryEnabled) {
            transition(backend, lifecycle, "summarizing");
            later(lifecycle, TICK.summarize, () => {
              transition(backend, lifecycle, "completed");
              backend.emit?.("meeting://summary-available", {
                meeting_id: lifecycle.meetingId,
              });
            });
          } else {
            transition(backend, lifecycle, "completed");
          }
        });
      });
      return null;
    }

    case "cancel_meeting": {
      if (!lifecycle) return null;
      clearTimers(lifecycle);
      transition(backend, lifecycle, "cancelled");
      return null;
    }

    case "retry_meeting_jobs": {
      if (!lifecycle) return null;
      backend.emit?.("meeting://jobs-updated", jobsPayload(lifecycle));
      return null;
    }

    case "get_meeting_snapshot": {
      if (!lifecycle) return null;
      return snapshotOf(lifecycle);
    }

    case "meeting_list_recoveries":
      return backend.recoveries.map((offer) => ({ ...offer }));

    case "meeting_resolve_recovery": {
      const resolution = args.resolution as MeetingRecoveryResolution;
      const before = backend.recoveries.length;
      backend.recoveries = backend.recoveries.filter(
        (offer) => offer.meeting_id !== meetingId,
      );
      if (resolution === "recover") {
        const row = backend.meetings.find((m) => m.id === meetingId);
        if (row) row.state = "transcribing";
      }
      return before !== backend.recoveries.length;
    }

    default:
      return undefined;
  }
};

/** Seeds the recovery offers a test asked for. */
export const initialRecoveries = (
  offers: MeetingRecoveryOffer[] | undefined,
): MeetingRecoveryOffer[] => (offers ? offers.map((o) => ({ ...o })) : []);
