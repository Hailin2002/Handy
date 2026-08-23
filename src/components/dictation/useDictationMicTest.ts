import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Microphone test for the DictationHome page (plan §6.4 "Diktat testen").
 *
 * SAFETY CONTRACT (gate G2) — this test is deliberately, inspectably separate
 * from the dictation output path:
 *
 * - It captures audio exclusively inside the webview via getUserMedia /
 *   MediaRecorder. It never invokes a backend command, so it can never reach
 *   the clipboard or the paste/typing pipeline (`actions.rs`).
 * - The recording exists only as an in-memory blob URL shown in the in-app
 *   preview player. Nothing is written to disk, nothing is auto-pasted.
 * - The blob URL is revoked on discard and on unmount.
 *
 * The recording stops on demand and automatically after
 * {@link MAX_TEST_MS}, so it can never silently keep the microphone open.
 */

const MAX_TEST_MS = 10_000;
const ELAPSED_TICK_MS = 200;

export type MicTestStatus = "idle" | "capturing" | "preview" | "error";

export type MicTestErrorKind = "unavailable" | "denied" | "failed";

export interface MicTestPreview {
  url: string;
  seconds: number;
}

const classifyCaptureError = (error: unknown): MicTestErrorKind => {
  const name = error instanceof Error ? error.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") return "denied";
  if (name === "NotFoundError" || name === "OverconstrainedError")
    return "unavailable";
  return "failed";
};

export const useDictationMicTest = (): {
  status: MicTestStatus;
  level: number;
  elapsedMs: number;
  preview: MicTestPreview | null;
  errorKind: MicTestErrorKind | null;
  maxSeconds: number;
  start: () => void;
  stop: () => void;
  discard: () => void;
} => {
  const [status, setStatus] = useState<MicTestStatus>("idle");
  const [level, setLevel] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [preview, setPreview] = useState<MicTestPreview | null>(null);
  const [errorKind, setErrorKind] = useState<MicTestErrorKind | null>(null);

  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const rafRef = useRef<number | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const autoStopTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startedAtRef = useRef<number>(0);
  const previewUrlRef = useRef<string | null>(null);
  // Guards against MediaRecorder.stop() being requested twice (manual stop
  // racing the auto-stop timer).
  const stoppingRef = useRef(false);

  const clearTimers = useCallback(() => {
    if (elapsedTimerRef.current !== null) {
      clearInterval(elapsedTimerRef.current);
      elapsedTimerRef.current = null;
    }
    if (autoStopTimerRef.current !== null) {
      clearTimeout(autoStopTimerRef.current);
      autoStopTimerRef.current = null;
    }
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  }, []);

  const releaseCapture = useCallback(() => {
    clearTimers();
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    if (audioCtxRef.current !== null) {
      void audioCtxRef.current.close().catch(() => undefined);
      audioCtxRef.current = null;
    }
    if (streamRef.current !== null) {
      for (const track of streamRef.current.getTracks()) track.stop();
      streamRef.current = null;
    }
    recorderRef.current = null;
    setLevel(0);
  }, [clearTimers]);

  const discard = useCallback(() => {
    if (previewUrlRef.current !== null) {
      URL.revokeObjectURL(previewUrlRef.current);
      previewUrlRef.current = null;
    }
    setPreview(null);
    setElapsedMs(0);
    setStatus((current) => (current === "preview" ? "idle" : current));
  }, []);

  const finishCapture = useCallback(() => {
    releaseCapture();
    const seconds = Math.max(
      1,
      Math.round((Date.now() - startedAtRef.current) / 1000),
    );
    const blob = new Blob(chunksRef.current, {
      type: recorderMimeType() ?? "audio/webm",
    });
    chunksRef.current = [];
    const url = URL.createObjectURL(blob);
    previewUrlRef.current = url;
    setPreview({ url, seconds });
    setElapsedMs(0);
    stoppingRef.current = false;
    setStatus("preview");
  }, [releaseCapture]);

  const stop = useCallback(() => {
    if (stoppingRef.current) return;
    const recorder = recorderRef.current;
    if (recorder === null || recorder.state === "inactive") return;
    stoppingRef.current = true;
    clearTimers();
    try {
      recorder.stop();
    } catch {
      stoppingRef.current = false;
      releaseCapture();
      setErrorKind("failed");
      setStatus("error");
    }
  }, [clearTimers, releaseCapture]);

  const start = useCallback(() => {
    if (status === "capturing") return;
    setErrorKind(null);
    discard();

    if (
      typeof navigator === "undefined" ||
      !navigator.mediaDevices?.getUserMedia ||
      typeof MediaRecorder === "undefined"
    ) {
      setErrorKind("unavailable");
      setStatus("error");
      return;
    }

    void (async () => {
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      } catch (error) {
        setErrorKind(classifyCaptureError(error));
        setStatus("error");
        return;
      }
      streamRef.current = stream;

      let recorder: MediaRecorder;
      try {
        recorder = new MediaRecorder(stream);
      } catch {
        releaseCapture();
        setErrorKind("failed");
        setStatus("error");
        return;
      }
      recorderRef.current = recorder;
      chunksRef.current = [];
      stoppingRef.current = false;
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunksRef.current.push(event.data);
      };
      recorder.onstop = () => {
        if (!stoppingRef.current) return;
        finishCapture();
      };
      recorder.start(250);

      startedAtRef.current = Date.now();
      setElapsedMs(0);
      setStatus("capturing");

      // Live level meter (decorative, aria-hidden in the UI).
      try {
        const AudioCtx =
          window.AudioContext ??
          (window as unknown as { webkitAudioContext?: typeof AudioContext })
            .webkitAudioContext;
        if (AudioCtx) {
          const ctx = new AudioCtx();
          audioCtxRef.current = ctx;
          void ctx.resume().catch(() => undefined);
          const source = ctx.createMediaStreamSource(stream);
          const analyser = ctx.createAnalyser();
          analyser.fftSize = 512;
          source.connect(analyser);
          const buffer = new Uint8Array(analyser.frequencyBinCount);
          const tick = () => {
            analyser.getByteTimeDomainData(buffer);
            let sum = 0;
            for (let i = 0; i < buffer.length; i += 1) {
              const v = (buffer[i] - 128) / 128;
              sum += v * v;
            }
            const rms = Math.sqrt(sum / buffer.length);
            setLevel(Math.min(1, rms * 3));
            rafRef.current = requestAnimationFrame(tick);
          };
          rafRef.current = requestAnimationFrame(tick);
        }
      } catch {
        // The meter is cosmetic — recording continues without it.
      }

      elapsedTimerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - startedAtRef.current);
      }, ELAPSED_TICK_MS);

      autoStopTimerRef.current = setTimeout(() => {
        autoStopTimerRef.current = null;
        stop();
      }, MAX_TEST_MS);
    })();
    // `discard` is stable and `status` is intentionally read once: a second
    // click while capturing must be a no-op, not a restart.
  }, [discard, finishCapture, releaseCapture, status, stop]);

  // Unmount cleanup: never leave the microphone open or a blob URL behind.
  useEffect(() => {
    return () => {
      stoppingRef.current = true;
      clearTimers();
      if (
        recorderRef.current !== null &&
        recorderRef.current.state !== "inactive"
      ) {
        try {
          recorderRef.current.stop();
        } catch {
          // Already stopped — nothing to do.
        }
      }
      if (audioCtxRef.current !== null) {
        void audioCtxRef.current.close().catch(() => undefined);
      }
      if (streamRef.current !== null) {
        for (const track of streamRef.current.getTracks()) track.stop();
      }
      if (previewUrlRef.current !== null) {
        URL.revokeObjectURL(previewUrlRef.current);
        previewUrlRef.current = null;
      }
    };
  }, [clearTimers]);

  return {
    status,
    level,
    elapsedMs,
    preview,
    errorKind,
    maxSeconds: MAX_TEST_MS / 1000,
    start,
    stop,
    discard,
  };
};

const recorderMimeType = (): string | null => {
  if (typeof MediaRecorder === "undefined") return null;
  if (MediaRecorder.isTypeSupported("audio/webm")) return "audio/webm";
  if (MediaRecorder.isTypeSupported("audio/mp4")) return "audio/mp4";
  if (MediaRecorder.isTypeSupported("audio/ogg")) return "audio/ogg";
  return null;
};
