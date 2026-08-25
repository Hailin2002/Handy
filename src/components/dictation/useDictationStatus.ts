import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { commands, events } from "@/bindings";

/**
 * Real dictation status for the DictationHome page (plan §6.4).
 *
 * The phase mirrors the backend state machine using only existing events and
 * commands — no new backend surface:
 *
 * - `commands.isRecording()` is polled while the page is mounted. A `true`
 *   result means "Aufnahme"; a `true → false` transition hands over to
 *   "Transkription" because the pipeline transcribes right after recording.
 * - The backend saves a history entry when a recording ends and updates it
 *   once transcription (and post-processing) finished. Those transitions are
 *   broadcast as `history-update-payload` events (`added` / `updated`) and
 *   mark the Transkription phase as done.
 * - `recording-error` and `transcription-error` events put the page into the
 *   honest "Fehler" state (plan §6.2 rule 5) together with the error kind so
 *   the UI can offer a concrete next action (rule 8).
 */

export type DictationPhase = "ready" | "recording" | "transcribing" | "error";

export type DictationErrorKind =
  | "microphone_permission"
  | "no_input_device"
  | "recording"
  | "transcription";

export interface DictationStatusError {
  kind: DictationErrorKind;
  /** Raw backend detail; only ever rendered as secondary context text. */
  detail?: string;
}

interface UseDictationStatusOptions {
  /** How often `isRecording()` is polled. */
  pollIntervalMs?: number;
  /**
   * Safety net: if no history update resolves the Transkription phase within
   * this window (e.g. the user cancelled and the backend discarded the audio
   * silently), fall back to "ready" instead of spinning forever.
   */
  transcribingTimeoutMs?: number;
}

const DEFAULT_POLL_INTERVAL_MS = 750;
const DEFAULT_TRANSCRIBING_TIMEOUT_MS = 60_000;

export const useDictationStatus = ({
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  transcribingTimeoutMs = DEFAULT_TRANSCRIBING_TIMEOUT_MS,
}: UseDictationStatusOptions = {}): {
  phase: DictationPhase;
  error: DictationStatusError | null;
} => {
  const [phase, setPhaseState] = useState<DictationPhase>("ready");
  const [error, setErrorState] = useState<DictationStatusError | null>(null);

  // Refs mirror the latest state inside listeners/polling without
  // resubscribing on every change.
  const phaseRef = useRef<DictationPhase>("ready");
  const errorKindRef = useRef<DictationErrorKind | null>(null);
  const disposedRef = useRef(false);

  useEffect(() => {
    disposedRef.current = false;

    const unlisteners: Array<Promise<UnlistenFn>> = [];
    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let transcribeTimer: ReturnType<typeof setTimeout> | null = null;

    const applyPhase = (next: DictationPhase) => {
      phaseRef.current = next;
      if (!disposedRef.current) setPhaseState(next);
    };

    const applyError = (next: DictationStatusError | null) => {
      errorKindRef.current = next?.kind ?? null;
      if (!disposedRef.current) setErrorState(next);
    };

    const clearTranscribeTimer = () => {
      if (transcribeTimer !== null) {
        clearTimeout(transcribeTimer);
        transcribeTimer = null;
      }
    };

    const armTranscribeTimer = () => {
      clearTranscribeTimer();
      transcribeTimer = setTimeout(() => {
        transcribeTimer = null;
        if (!disposedRef.current && phaseRef.current === "transcribing") {
          applyPhase("ready");
        }
      }, transcribingTimeoutMs);
    };

    const poll = async () => {
      try {
        const recording = await commands.isRecording();
        if (disposedRef.current) return;
        if (recording) {
          // A fresh recording makes any previous error obsolete.
          applyError(null);
          clearTranscribeTimer();
          if (phaseRef.current !== "recording") applyPhase("recording");
        } else if (phaseRef.current === "recording") {
          // Recording just ended: the backend now transcribes (or discards).
          armTranscribeTimer();
          applyPhase("transcribing");
        }
      } catch {
        // Backend unreachable — keep the last known phase instead of lying.
      }
    };

    pollTimer = setInterval(() => void poll(), pollIntervalMs);
    void poll();

    unlisteners.push(
      listen<{ error_type: string; detail?: string }>(
        "recording-error",
        (event) => {
          if (disposedRef.current) return;
          clearTranscribeTimer();
          const kind: DictationErrorKind =
            event.payload.error_type === "microphone_permission_denied"
              ? "microphone_permission"
              : event.payload.error_type === "no_input_device"
                ? "no_input_device"
                : "recording";
          applyError({ kind, detail: event.payload.detail });
          applyPhase("error");
        },
      ),
    );

    unlisteners.push(
      listen<string>("transcription-error", (event) => {
        if (disposedRef.current) return;
        clearTranscribeTimer();
        applyError({ kind: "transcription", detail: event.payload });
        applyPhase("error");
      }),
    );

    unlisteners.push(
      events.historyUpdatePayload.listen((event) => {
        if (disposedRef.current) return;
        const { action } = event.payload;
        if (action === "added") {
          // Audio was persisted; speech-to-text is running for it now.
          if (
            phaseRef.current === "ready" ||
            phaseRef.current === "transcribing"
          ) {
            armTranscribeTimer();
            applyPhase("transcribing");
          }
        } else if (action === "updated") {
          // Transcription/post-processing finished for the tracked entry.
          if (
            phaseRef.current === "transcribing" ||
            (phaseRef.current === "error" &&
              errorKindRef.current === "transcription")
          ) {
            clearTranscribeTimer();
            applyError(null);
            applyPhase("ready");
          }
        }
        // "toggled" / "deleted" carry no pipeline information — ignore them.
      }),
    );

    return () => {
      disposedRef.current = true;
      if (pollTimer !== null) clearInterval(pollTimer);
      clearTranscribeTimer();
      for (const promise of unlisteners) {
        promise.then((unlisten) => unlisten()).catch(() => undefined);
      }
    };
  }, [pollIntervalMs, transcribingTimeoutMs]);

  return { phase, error };
};
