import { useEffect, useState } from "react";
import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  MEETING_ACTIVE_STATES,
  type MeetingAudioSource,
  type MeetingJobsUpdatedEvent,
  type MeetingLevelUpdateEvent,
  type MeetingPreflightReport,
  type MeetingProviderKind,
  type MeetingRecoveryOffer,
  type MeetingRecoveryReportEvent,
  type MeetingRecoveryResolution,
  type MeetingSessionErrorEvent,
  type MeetingSessionState,
  type MeetingSessionStateEvent,
  type MeetingSnapshot,
  type MeetingStartRequest,
  type MeetingTranscriptDeltaEvent,
  type MeetingTranscriptSegment,
} from "@/lib/types/meeting";
import {
  MEETING_COMMANDS,
  MEETING_EVENTS,
  MeetingCommandUnavailableError,
  meetingInvoke,
} from "@/lib/meetingIpc";

/**
 * S1 `meetingStore` - the single frontend owner of the meeting-session
 * projection and the sole registrar of the `meeting://*` event topics
 * (`docs/meeting-mode-architecture/frontend-state-contract.md`, rules
 * B1-B6 and L1-L8).
 *
 * Contract points that shape this file:
 *
 * - **No second recording state** (plan line 927 / SSC 1.1): everything here
 *   is a projection of backend facts. Nothing is invented locally; the only
 *   locally computed value is the *displayed* clock, and even that is derived
 *   from the backend-provided `runningSince`/`elapsedBaseMs` pair.
 * - **Stale-revision discard** (SSC O3 / FSC L5): session events carrying a
 *   `state_revision` at or below the applied one are ignored; a revision gap
 *   triggers a snapshot refetch instead of guessing (SSC O5 / FSC L6).
 * - **Subscribe-then-snapshot** (FSC L6): `initListeners()` awaits every
 *   `listen()` before it resolves and is idempotent; `disposeListeners()` is
 *   symmetric (FSC L3/L4).
 * - **Never persisted** (SSC 5 / FSC B4): no `zustand/persist`, no settings
 *   key. After a restart the UI rebuilds from `get_meeting_snapshot`.
 * - **Degrade, never crash**: the Rust commands do not exist yet. Every
 *   action funnels through `runCommand`, which turns "command unavailable"
 *   into a visible, explainable error state.
 */

/** Live projection of one session; mirrors the snapshot fields we render. */
export interface MeetingLiveSession {
  meetingId: string;
  title: string;
  state: MeetingSessionState;
  stateRevision: number;
  /** Capture milliseconds already accumulated before the current run. */
  elapsedBaseMs: number;
  /** Epoch ms at which the current capture run started; null while frozen. */
  runningSince: number | null;
  microphoneOnly: boolean;
  providerKind: MeetingProviderKind;
  providerName: string | null;
  summaryEnabled: boolean;
}

/** Redacted, explainable error surface (plan 6.2 rule 8). */
export interface MeetingErrorState {
  /** Taxonomy code; drives which `meeting.errors.<code>.*` copy is shown. */
  code: string;
  detail?: string;
  /** False when the backend command is missing entirely. */
  backendAvailable: boolean;
}

export type MeetingPreflightStatus =
  | "idle"
  | "running"
  | "ready"
  | "unavailable";

export interface MeetingStoreState {
  session: MeetingLiveSession | null;
  levels: Record<MeetingAudioSource, number>;
  transcript: MeetingTranscriptSegment[];
  transcriptHeadSequence: number;
  jobs: MeetingJobsUpdatedEvent | null;
  error: MeetingErrorState | null;
  recoveries: MeetingRecoveryOffer[];
  preflight: MeetingPreflightReport | null;
  preflightStatus: MeetingPreflightStatus;
  /** True while a lifecycle command is in flight (buttons disable on it). */
  busy: boolean;
  /**
   * Meeting the global shortcut is asking to end (plan §6.9), or `null`.
   *
   * The backend owns the timing; the store only mirrors the question so the
   * window, the recorder view and the tray never disagree about whether a
   * stop is pending.
   */
  stopConfirmFor: string | null;
  listenersReady: boolean;

  // --- listener registrar (FSC L1/L3/L4) --------------------------------
  initListeners: () => Promise<void>;
  disposeListeners: () => Promise<void>;

  // --- actions (FSC B6: the only mutators) ------------------------------
  runPreflight: () => Promise<MeetingPreflightReport | null>;
  startMeeting: (request: MeetingStartRequest) => Promise<boolean>;
  pauseMeeting: () => Promise<void>;
  resumeMeeting: () => Promise<void>;
  stopMeeting: (options?: { abort?: boolean }) => Promise<void>;
  cancelMeeting: () => Promise<void>;
  retryFailedJobs: () => Promise<void>;
  refreshSnapshot: (meetingId: string) => Promise<void>;
  refreshRecoveries: () => Promise<void>;
  resolveRecovery: (
    meetingId: string,
    resolution: MeetingRecoveryResolution,
  ) => Promise<void>;

  clearError: () => void;
  /** Answers the shortcut's "really end this meeting?" with "keep recording". */
  dismissStopConfirm: () => void;
  /** Leaves the processing/completed view without touching backend state. */
  releaseSession: () => void;
  resetForTests: () => void;

  // --- listener appliers (exported for unit tests; never called by views) -
  applySessionState: (event: MeetingSessionStateEvent) => void;
  applySessionError: (event: MeetingSessionErrorEvent) => void;
  applyLevelUpdate: (event: MeetingLevelUpdateEvent) => void;
  applyTranscriptDelta: (event: MeetingTranscriptDeltaEvent) => void;
  applyJobsUpdated: (event: MeetingJobsUpdatedEvent) => void;
  applyRecoveryReport: (event: MeetingRecoveryReportEvent) => void;
  applyStopConfirm: (event: { meeting_id: string; armed: boolean }) => void;
  adoptSnapshot: (snapshot: MeetingSnapshot) => void;
}

const EMPTY_LEVELS: Record<MeetingAudioSource, number> = {
  microphone: 0,
  system: 0,
};

/**
 * Retained `UnlistenFn`s, keyed by topic (FSC L3). Module-scoped on purpose:
 * the registry is a property of the process, not of a store snapshot.
 */
const listeners = new Map<string, UnlistenFn>();
let listenerRegistration: Promise<void> | null = null;

const sessionFromSnapshot = (
  snapshot: MeetingSnapshot,
): MeetingLiveSession => ({
  meetingId: snapshot.meeting_id,
  title: snapshot.title,
  state: snapshot.state,
  stateRevision: snapshot.state_revision,
  elapsedBaseMs: Math.max(0, snapshot.elapsed_ms ?? 0),
  runningSince:
    snapshot.running_since ??
    (snapshot.state === "recording" ? Date.now() : null),
  microphoneOnly: snapshot.microphone_only ?? false,
  providerKind: snapshot.provider_kind ?? "local",
  providerName: snapshot.provider_name ?? null,
  summaryEnabled: snapshot.summary_enabled ?? false,
});

/**
 * Folds a state transition into the live session, keeping the pause-corrected
 * clock honest: leaving `recording` banks the elapsed run, entering it starts
 * a new one. The backend stays the authority for the state itself.
 */
const applyTransition = (
  session: MeetingLiveSession,
  to: MeetingSessionState,
  revision: number,
  microphoneOnly?: boolean,
): MeetingLiveSession => {
  const now = Date.now();
  const wasCapturing = session.runningSince !== null;
  const willCapture = to === "recording";
  const bankedMs =
    wasCapturing && !willCapture
      ? session.elapsedBaseMs + Math.max(0, now - (session.runningSince ?? now))
      : session.elapsedBaseMs;

  return {
    ...session,
    state: to,
    stateRevision: revision,
    elapsedBaseMs: bankedMs,
    runningSince: willCapture
      ? wasCapturing
        ? session.runningSince
        : now
      : null,
    microphoneOnly: microphoneOnly ?? session.microphoneOnly,
  };
};

export const useMeetingStore = create<MeetingStoreState>()((set, get) => {
  /**
   * Runs a lifecycle command with uniform busy handling and error mapping.
   * Returns `undefined` on failure - callers never see a rejected promise, so
   * an unbuilt backend can never take the app down.
   */
  const runCommand = async <T>(
    command: string,
    args: Record<string, unknown>,
    errorCode: string,
  ): Promise<T | undefined> => {
    set({ busy: true });
    try {
      const value = await meetingInvoke<T>(command, args);
      set({ busy: false, error: null });
      return value;
    } catch (error) {
      const unavailable = error instanceof MeetingCommandUnavailableError;
      set({
        busy: false,
        error: {
          code: unavailable ? "backendUnavailable" : errorCode,
          detail: unavailable ? undefined : (error as Error).message,
          backendAvailable: !unavailable,
        },
      });
      return undefined;
    }
  };

  return {
    session: null,
    levels: { ...EMPTY_LEVELS },
    transcript: [],
    transcriptHeadSequence: 0,
    jobs: null,
    error: null,
    recoveries: [],
    preflight: null,
    preflightStatus: "idle",
    busy: false,
    stopConfirmFor: null,
    listenersReady: false,

    // ---------------------------------------------------------------- L1/L3
    initListeners: async () => {
      if (listenerRegistration) return listenerRegistration;
      listenerRegistration = (async () => {
        const bind = async (
          topic: string,
          handler: (payload: unknown) => void,
        ) => {
          if (listeners.has(topic)) return;
          try {
            const unlisten = await listen(topic, (event) => {
              // L7: normalize and delegate; no branching lives in the handler.
              handler(event.payload);
            });
            listeners.set(topic, unlisten);
          } catch {
            // No Tauri runtime (plain browser / unit test): the store simply
            // stays event-less; snapshots still drive the UI.
          }
        };

        await Promise.all([
          bind(MEETING_EVENTS.sessionState, (payload) =>
            get().applySessionState(payload as MeetingSessionStateEvent),
          ),
          bind(MEETING_EVENTS.sessionError, (payload) =>
            get().applySessionError(payload as MeetingSessionErrorEvent),
          ),
          bind(MEETING_EVENTS.levelUpdate, (payload) =>
            get().applyLevelUpdate(payload as MeetingLevelUpdateEvent),
          ),
          bind(MEETING_EVENTS.transcriptDelta, (payload) =>
            get().applyTranscriptDelta(payload as MeetingTranscriptDeltaEvent),
          ),
          bind(MEETING_EVENTS.jobsUpdated, (payload) =>
            get().applyJobsUpdated(payload as MeetingJobsUpdatedEvent),
          ),
          bind(MEETING_EVENTS.recoveryReport, (payload) =>
            get().applyRecoveryReport(payload as MeetingRecoveryReportEvent),
          ),
          bind(MEETING_EVENTS.sessionRecovered, (payload) =>
            get().applyRecoveryReport(payload as MeetingRecoveryReportEvent),
          ),
          bind(MEETING_EVENTS.stopConfirm, (payload) =>
            get().applyStopConfirm(
              payload as { meeting_id: string; armed: boolean },
            ),
          ),
        ]);
        set({ listenersReady: true });
      })();
      return listenerRegistration;
    },

    disposeListeners: async () => {
      for (const [topic, unlisten] of [...listeners]) {
        try {
          unlisten();
        } catch {
          // Already torn down by the webview; dropping the handle is enough.
        }
        listeners.delete(topic);
      }
      listenerRegistration = null;
      set({ listenersReady: false });
    },

    // -------------------------------------------------------------- actions
    runPreflight: async () => {
      set({ preflightStatus: "running" });
      try {
        const report = await meetingInvoke<MeetingPreflightReport>(
          MEETING_COMMANDS.preflight,
          {},
        );
        set({ preflight: report, preflightStatus: "ready" });
        return report;
      } catch (error) {
        const unavailable = error instanceof MeetingCommandUnavailableError;
        set({
          preflight: null,
          preflightStatus: unavailable ? "unavailable" : "ready",
          error: {
            code: unavailable ? "backendUnavailable" : "preflightFailed",
            detail: unavailable ? undefined : (error as Error).message,
            backendAvailable: !unavailable,
          },
        });
        return null;
      }
    },

    startMeeting: async (request) => {
      const snapshot = await runCommand<MeetingSnapshot>(
        MEETING_COMMANDS.start,
        {
          meetingId: request.meetingId,
          title: request.title,
          microphoneOnly: request.microphoneOnly,
          summaryEnabled: request.summaryEnabled,
          consentAcknowledged: request.consentAcknowledged,
          sttProvider: request.sttProvider,
        },
        "startFailed",
      );
      if (!snapshot) return false;
      get().adoptSnapshot(snapshot);
      return true;
    },

    pauseMeeting: async () => {
      const session = get().session;
      if (!session) return;
      await runCommand(
        MEETING_COMMANDS.pause,
        { meetingId: session.meetingId },
        "pauseFailed",
      );
    },

    resumeMeeting: async () => {
      const session = get().session;
      if (!session) return;
      await runCommand(
        MEETING_COMMANDS.resume,
        { meetingId: session.meetingId },
        "resumeFailed",
      );
    },

    stopMeeting: async (options) => {
      const session = get().session;
      if (!session) return;
      await runCommand(
        MEETING_COMMANDS.stop,
        { meetingId: session.meetingId, abort: options?.abort ?? false },
        "stopFailed",
      );
    },

    cancelMeeting: async () => {
      const session = get().session;
      if (!session) return;
      await runCommand(
        MEETING_COMMANDS.cancel,
        { meetingId: session.meetingId },
        "cancelFailed",
      );
    },

    retryFailedJobs: async () => {
      const session = get().session;
      if (!session) return;
      await runCommand(
        MEETING_COMMANDS.retryJobs,
        { meetingId: session.meetingId },
        "retryFailed",
      );
    },

    refreshSnapshot: async (meetingId) => {
      try {
        const snapshot = await meetingInvoke<MeetingSnapshot>(
          MEETING_COMMANDS.snapshot,
          { meetingId },
        );
        get().adoptSnapshot(snapshot);
      } catch {
        // O5 resync is best-effort: a missing snapshot leaves the last known
        // projection in place rather than blanking a live recording view.
      }
    },

    refreshRecoveries: async () => {
      try {
        const offers = await meetingInvoke<MeetingRecoveryOffer[]>(
          MEETING_COMMANDS.listRecoveries,
          {},
        );
        set({ recoveries: Array.isArray(offers) ? offers : [] });
      } catch {
        // No backend yet: simply no recovery offers, the banner stays hidden.
        set({ recoveries: [] });
      }
    },

    resolveRecovery: async (meetingId, resolution) => {
      const done = await runCommand<boolean>(
        MEETING_COMMANDS.resolveRecovery,
        { meetingId, resolution },
        "recoveryFailed",
      );
      if (done === undefined) return;
      set((state) => ({
        recoveries: state.recoveries.filter((o) => o.meeting_id !== meetingId),
      }));
    },

    clearError: () => set({ error: null }),

    releaseSession: () =>
      set({
        session: null,
        transcript: [],
        transcriptHeadSequence: 0,
        jobs: null,
        levels: { ...EMPTY_LEVELS },
        error: null,
      }),

    resetForTests: () => {
      listeners.clear();
      listenerRegistration = null;
      set({
        session: null,
        levels: { ...EMPTY_LEVELS },
        transcript: [],
        transcriptHeadSequence: 0,
        jobs: null,
        error: null,
        recoveries: [],
        preflight: null,
        preflightStatus: "idle",
        busy: false,
        stopConfirmFor: null,
        listenersReady: false,
      });
    },

    // ------------------------------------------------------------ appliers
    applySessionState: (event) => {
      const session = get().session;
      if (!session || session.meetingId !== event.meeting_id) return;
      // O3: discard stale or duplicate deliveries.
      if (event.state_revision <= session.stateRevision) return;
      // O5: a gap means we missed transitions - resync instead of guessing.
      if (event.state_revision > session.stateRevision + 1) {
        void get().refreshSnapshot(session.meetingId);
        return;
      }
      set({
        session: applyTransition(
          session,
          event.to,
          event.state_revision,
          event.microphone_only,
        ),
      });
    },

    applySessionError: (event) => {
      const session = get().session;
      if (session && session.meetingId !== event.meeting_id) return;
      set({
        error: {
          code: event.code || "generic",
          detail: event.detail,
          backendAvailable: true,
        },
      });
    },

    applyLevelUpdate: (event) => {
      const session = get().session;
      if (!session || session.meetingId !== event.meeting_id) return;
      // Meters only move while capturing (LV 2.3 suppression rule).
      if (session.state !== "recording") return;
      // Monkey-proofing: an unknown source key or a non-finite level must not
      // pollute the meter state (NaN would stick until the next good frame).
      if (event.source !== "microphone" && event.source !== "system") return;
      const raw = event.peak ?? event.rms ?? 0;
      if (!Number.isFinite(raw)) return;
      const level = Math.min(1, Math.max(0, raw));
      set((state) => ({ levels: { ...state.levels, [event.source]: level } }));
    },

    applyTranscriptDelta: (event) => {
      const session = get().session;
      if (!session || session.meetingId !== event.meeting_id) return;
      set((state) => {
        // Append-only and idempotent: a re-delivered batch must not duplicate.
        const known = new Set(state.transcript.map((s) => s.id));
        const added = (event.segments ?? []).filter((s) => !known.has(s.id));
        if (added.length === 0) return state;
        const transcript = [...state.transcript, ...added].sort(
          // Sequence is the primary order; start_ms breaks ties so a hostile
          // or duplicated sequence cannot make rows flicker between renders.
          (a, b) => a.sequence - b.sequence || a.start_ms - b.start_ms,
        );
        return {
          transcript,
          transcriptHeadSequence: Math.max(
            state.transcriptHeadSequence,
            event.last_sequence ?? 0,
          ),
        };
      });
    },

    applyJobsUpdated: (event) => {
      const session = get().session;
      if (!session || session.meetingId !== event.meeting_id) return;
      set({ jobs: event });
    },

    applyRecoveryReport: (event) => {
      const offers = Array.isArray(event?.offers) ? event.offers : [];
      if (offers.length === 0) return;
      set((state) => {
        const byId = new Map(state.recoveries.map((o) => [o.meeting_id, o]));
        for (const offer of offers) byId.set(offer.meeting_id, offer);
        return { recoveries: [...byId.values()] };
      });
    },

    dismissStopConfirm: () => set({ stopConfirmFor: null }),

    applyStopConfirm: (event) => {
      const meetingId = event?.meeting_id;
      if (typeof meetingId !== "string" || meetingId.length === 0) return;
      set((state) => {
        if (event.armed) return { stopConfirmFor: meetingId };
        // Only the meeting that was asked about may clear the question, so a
        // late "disarmed" for an older meeting cannot cancel a fresh one.
        return state.stopConfirmFor === meetingId
          ? { stopConfirmFor: null }
          : {};
      });
    },

    adoptSnapshot: (snapshot) => {
      const next = sessionFromSnapshot(snapshot);
      set((state) => {
        const sameSession = state.session?.meetingId === next.meetingId;
        return {
          session: next,
          transcript: sameSession ? state.transcript : [],
          transcriptHeadSequence:
            snapshot.transcript_head_sequence ??
            (sameSession ? state.transcriptHeadSequence : 0),
          jobs: snapshot.jobs ?? (sameSession ? state.jobs : null),
          levels: {
            microphone: snapshot.levels?.microphone ?? 0,
            system: snapshot.levels?.system ?? 0,
          },
          error: snapshot.last_error
            ? {
                code: snapshot.last_error.code,
                detail: snapshot.last_error.detail,
                backendAvailable: true,
              }
            : null,
        };
      });
    },
  };
});

// ---------------------------------------------------------------- selectors

/** True while the session occupies one of the capture states (SSC 2.1). */
export const isMeetingCapturing = (
  state: MeetingSessionState | undefined,
): boolean => !!state && MEETING_ACTIVE_STATES.includes(state);

/** Milliseconds of capture at wall-clock `now`; frozen while paused. */
export const meetingElapsedMs = (
  session: MeetingLiveSession | null,
  now: number = Date.now(),
): number => {
  if (!session) return 0;
  if (session.runningSince === null) return session.elapsedBaseMs;
  return session.elapsedBaseMs + Math.max(0, now - session.runningSince);
};

/**
 * Ticking clock for the recorder view and the sidebar pill.
 *
 * Deliberately a local `useState` rather than store state: FSC B5 forbids
 * writing a per-tick value into a Zustand store, because every subscriber
 * would re-render once per second. The store keeps only the two backend facts
 * (`elapsedBaseMs`, `runningSince`) this derives from.
 */
export const useMeetingElapsedSeconds = (): number => {
  const session = useMeetingStore((state) => state.session);
  const [seconds, setSeconds] = useState(() =>
    Math.floor(meetingElapsedMs(session) / 1000),
  );

  useEffect(() => {
    setSeconds(Math.floor(meetingElapsedMs(session) / 1000));
    if (!session || session.runningSince === null) return;
    const handle = window.setInterval(() => {
      setSeconds(Math.floor(meetingElapsedMs(session) / 1000));
    }, 1000);
    return () => window.clearInterval(handle);
  }, [session]);

  return seconds;
};
