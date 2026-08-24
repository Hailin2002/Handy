import { beforeEach, describe, expect, it, vi } from "vitest";

// The meeting store owns the `meeting://*` topics and the lifecycle commands.
// Both boundary modules only exist inside the webview, so they are replaced by
// deterministic fakes and the contract rules become directly observable.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) =>
    invokeMock(command, args),
}));

import type { MeetingSnapshot } from "@/lib/types/meeting";
import { MEETING_COMMANDS } from "@/lib/meetingIpc";
import {
  isMeetingCapturing,
  meetingElapsedMs,
  useMeetingStore,
} from "../meetingStore";

const snapshot = (overrides: Partial<MeetingSnapshot> = {}): MeetingSnapshot => ({
  meeting_id: "m-1",
  title: "Budget Review",
  state: "recording",
  state_revision: 3,
  elapsed_ms: 0,
  running_since: Date.now(),
  provider_kind: "local",
  ...overrides,
});

describe("meetingStore", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useMeetingStore.getState().resetForTests();
  });

  it("adopts a snapshot as the whole live projection", () => {
    useMeetingStore.getState().adoptSnapshot(snapshot({ elapsed_ms: 5_000 }));
    const session = useMeetingStore.getState().session;
    expect(session?.meetingId).toBe("m-1");
    expect(session?.state).toBe("recording");
    expect(session?.stateRevision).toBe(3);
    expect(session?.elapsedBaseMs).toBe(5_000);
  });

  it("discards stale and duplicate session-state events (SSC O3)", () => {
    const store = useMeetingStore.getState();
    store.adoptSnapshot(snapshot({ state: "recording", state_revision: 3 }));
    store.applySessionState({
      meeting_id: "m-1",
      to: "paused",
      state_revision: 3,
    });
    expect(useMeetingStore.getState().session?.state).toBe("recording");
    store.applySessionState({
      meeting_id: "m-1",
      to: "paused",
      state_revision: 2,
    });
    expect(useMeetingStore.getState().session?.state).toBe("recording");
  });

  it("resyncs from the snapshot when a revision gap appears (SSC O5)", async () => {
    invokeMock.mockResolvedValue(
      snapshot({ state: "stopping", state_revision: 9 }),
    );
    const store = useMeetingStore.getState();
    store.adoptSnapshot(snapshot({ state: "recording", state_revision: 3 }));
    store.applySessionState({
      meeting_id: "m-1",
      to: "paused",
      state_revision: 9,
    });
    // The gapped event itself is not applied; a snapshot refetch is issued.
    expect(invokeMock).toHaveBeenCalledWith(MEETING_COMMANDS.snapshot, {
      meetingId: "m-1",
    });
    await vi.waitFor(() =>
      expect(useMeetingStore.getState().session?.state).toBe("stopping"),
    );
  });

  it("freezes the clock on pause and continues it on resume", () => {
    const store = useMeetingStore.getState();
    const start = Date.now() - 10_000;
    store.adoptSnapshot(
      snapshot({ state: "recording", state_revision: 1, running_since: start }),
    );
    store.applySessionState({
      meeting_id: "m-1",
      to: "paused",
      state_revision: 2,
    });
    const paused = useMeetingStore.getState().session;
    expect(paused?.runningSince).toBeNull();
    expect(paused?.elapsedBaseMs).toBeGreaterThanOrEqual(10_000);
    // While paused the displayed duration must not advance.
    expect(meetingElapsedMs(paused, Date.now() + 5_000)).toBe(
      paused?.elapsedBaseMs,
    );

    store.applySessionState({
      meeting_id: "m-1",
      to: "recording",
      state_revision: 3,
    });
    expect(useMeetingStore.getState().session?.runningSince).not.toBeNull();
  });

  it("ignores level updates outside recording (LV 2.3 suppression)", () => {
    const store = useMeetingStore.getState();
    store.adoptSnapshot(snapshot({ state: "paused", state_revision: 4 }));
    store.applyLevelUpdate({
      meeting_id: "m-1",
      source: "microphone",
      peak: 0.8,
      rms: 0.4,
    });
    expect(useMeetingStore.getState().levels.microphone).toBe(0);
  });

  it("applies transcript deltas append-only and idempotently", () => {
    const store = useMeetingStore.getState();
    store.adoptSnapshot(snapshot());
    const delta = {
      meeting_id: "m-1",
      last_sequence: 2,
      segments: [
        {
          id: "s2",
          source: "system" as const,
          start_ms: 1_000,
          end_ms: 2_000,
          sequence: 2,
          text: "second",
        },
        {
          id: "s1",
          source: "microphone" as const,
          start_ms: 0,
          end_ms: 1_000,
          sequence: 1,
          text: "first",
        },
      ],
    };
    store.applyTranscriptDelta(delta);
    store.applyTranscriptDelta(delta);
    const transcript = useMeetingStore.getState().transcript;
    expect(transcript.map((s) => s.id)).toEqual(["s1", "s2"]);
    expect(useMeetingStore.getState().transcriptHeadSequence).toBe(2);
  });

  it("surfaces a missing backend command as an explainable error state", async () => {
    invokeMock.mockRejectedValue(
      new Error("[e2e-mock] unhandled IPC command: start_meeting"),
    );
    const started = await useMeetingStore.getState().startMeeting({
      meetingId: "m-1",
      title: "Budget Review",
      microphoneOnly: false,
      summaryEnabled: true,
      consentAcknowledged: true,
    });
    expect(started).toBe(false);
    const error = useMeetingStore.getState().error;
    expect(error?.code).toBe("backendUnavailable");
    expect(error?.backendAvailable).toBe(false);
    // Degrading must not leave a half-started session behind.
    expect(useMeetingStore.getState().session).toBeNull();
  });

  it("maps a real command rejection to its action-specific code", async () => {
    useMeetingStore.getState().adoptSnapshot(snapshot());
    invokeMock.mockRejectedValue(new Error("device busy"));
    await useMeetingStore.getState().pauseMeeting();
    expect(useMeetingStore.getState().error).toEqual({
      code: "pauseFailed",
      detail: "device busy",
      backendAvailable: true,
    });
  });

  it("keeps the recovery list empty when the backend has no command", async () => {
    invokeMock.mockRejectedValue(new Error("unknown command"));
    await useMeetingStore.getState().refreshRecoveries();
    expect(useMeetingStore.getState().recoveries).toEqual([]);
  });

  it("merges recovery reports by meeting id", () => {
    const store = useMeetingStore.getState();
    const offer = {
      meeting_id: "m-9",
      title: "Interrupted",
      cause: "crash" as const,
      recovered_chunks: 3,
      damaged_chunks: 0,
      quarantined_files: 0,
    };
    store.applyRecoveryReport({ offers: [offer] });
    store.applyRecoveryReport({ offers: [{ ...offer, recovered_chunks: 4 }] });
    const recoveries = useMeetingStore.getState().recoveries;
    expect(recoveries).toHaveLength(1);
    expect(recoveries[0].recovered_chunks).toBe(4);
  });

  it("treats exactly the capture states as an active meeting", () => {
    expect(isMeetingCapturing("recording")).toBe(true);
    expect(isMeetingCapturing("paused")).toBe(true);
    expect(isMeetingCapturing("starting")).toBe(true);
    expect(isMeetingCapturing("stopping")).toBe(true);
    expect(isMeetingCapturing("draft")).toBe(false);
    expect(isMeetingCapturing("transcribing")).toBe(false);
    expect(isMeetingCapturing(undefined)).toBe(false);
  });
});
