import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as SpeechSDK from "microsoft-cognitiveservices-speech-sdk";
import { commands } from "@/bindings";

interface AzureStreamControlPayload {
  session_id: number;
}

interface AzureAudioChunkPayload extends AzureStreamControlPayload {
  pcm_base64: string;
}

interface AzureRuntime {
  sessionId: number;
  pushStream: SpeechSDK.PushAudioInputStream | null;
  audioConfig: SpeechSDK.AudioConfig | null;
  recognizer: SpeechSDK.SpeechRecognizer | null;
  connection: SpeechSDK.Connection | null;
  pendingAudio: ArrayBuffer[];
  segmentChain: Promise<void>;
  translationTimer: ReturnType<typeof setTimeout> | null;
  tokenRefreshTimer: ReturnType<typeof setTimeout> | null;
  stopFallbackTimer: ReturnType<typeof setTimeout> | null;
  translationDelayMs: number;
  deeplEnabled: boolean;
  stopRequested: boolean;
  stopping: boolean;
  cleanupPromise: Promise<void> | null;
  receivedAudioBytes: number;
  receivedAudioChunks: number;
  writtenAudioBytes: number;
  writtenAudioChunks: number;
  recognizedAudioEndMs: number;
  keepAliveRelease: (() => void) | null;
  tailPaddingWritten: boolean;
}

const decodeBase64 = (encoded: string): ArrayBuffer => {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
};

const commandError = (
  result: { status: "ok" } | { status: "error"; error: string },
) => {
  if (result.status === "error") throw new Error(result.error);
};

const TOKEN_REFRESH_INTERVAL_MS = 8 * 60 * 1_000;
const TOKEN_REFRESH_RETRY_MS = 60 * 1_000;
// Rust already owns every committed phrase plus the latest interim. Do not let
// a delayed browser SDK stop callback turn an otherwise-live Azure dictation
// into a multi-second wait when translation is disabled.
const STOP_FALLBACK_MS = 700;
const TRANSLATED_STOP_FALLBACK_MS = 3_000;
const FINAL_SILENCE_MS = 300;

export const retainListenerUnlessDisposed = async (
  registration: Promise<UnlistenFn>,
  isDisposed: () => boolean,
  retain: (unlisten: UnlistenFn) => void,
) => {
  const unlisten = await registration;
  if (isDisposed()) {
    unlisten();
    return;
  }
  retain(unlisten);
};

/**
 * Hosts Microsoft's official continuous Speech SDK in the always-running main
 * webview. The Rust recorder remains the single microphone owner and forwards
 * its selected 16 kHz mono stream here, avoiding a second browser microphone
 * capture with a potentially different device.
 */
export const useAzureSpeechStream = () => {
  const runtimeRef = useRef<AzureRuntime | null>(null);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let disposed = false;

    const reportStatus = (runtime: AzureRuntime, status: string) => {
      void commands.azureSpeechStatus(runtime.sessionId, status);
    };

    const acquireBackgroundKeepAlive = (runtime: AzureRuntime) => {
      if (!("locks" in navigator)) {
        reportStatus(runtime, "background-keepalive-unavailable");
        return;
      }
      void navigator.locks
        .request(`speak-azure-${runtime.sessionId}`, async () => {
          reportStatus(runtime, "background-keepalive-acquired");
          await new Promise<void>((resolve) => {
            runtime.keepAliveRelease = resolve;
            if (runtime.stopping) resolve();
          });
        })
        .catch((error) => {
          reportStatus(runtime, "background-keepalive-failed");
          console.warn(
            "Failed to keep Azure Speech active in background",
            error,
          );
        });
    };

    const recognitionEndMs = (result: SpeechSDK.SpeechRecognitionResult) =>
      Math.max(0, Math.round((result.offset + result.duration) / 10_000));

    const writeAudio = (runtime: AzureRuntime, chunk: ArrayBuffer) => {
      if (!runtime.pushStream || runtime.stopping) return false;
      runtime.pushStream.write(chunk);
      runtime.writtenAudioBytes += chunk.byteLength;
      runtime.writtenAudioChunks += 1;
      return true;
    };

    const waitForCallback = (
      action: (done: () => void) => void,
      timeoutMs = 750,
    ) =>
      new Promise<void>((resolve) => {
        let completed = false;
        const done = () => {
          if (completed) return;
          completed = true;
          resolve();
        };
        try {
          action(done);
        } catch {
          done();
        }
        setTimeout(done, timeoutMs);
      });

    const cleanup = (runtime: AzureRuntime): Promise<void> => {
      if (runtime.cleanupPromise) return runtime.cleanupPromise;
      runtime.cleanupPromise = (async () => {
        try {
          runtime.stopping = true;
          if (runtime.translationTimer) clearTimeout(runtime.translationTimer);
          runtime.translationTimer = null;
          if (runtime.tokenRefreshTimer)
            clearTimeout(runtime.tokenRefreshTimer);
          runtime.tokenRefreshTimer = null;
          if (runtime.stopFallbackTimer)
            clearTimeout(runtime.stopFallbackTimer);
          runtime.stopFallbackTimer = null;
          runtime.pendingAudio = [];
          runtime.keepAliveRelease?.();
          runtime.keepAliveRelease = null;
          try {
            runtime.pushStream?.close();
          } catch {
            // The stream may already have been closed by a stop/cancel race.
          }
          const connection = runtime.connection;
          const recognizer = runtime.recognizer;
          const audioConfig = runtime.audioConfig;
          runtime.connection = null;
          runtime.pushStream = null;
          runtime.recognizer = null;
          runtime.audioConfig = null;
          if (connection) {
            await waitForCallback((done) =>
              connection.closeConnection(done, done),
            );
          }
          if (recognizer) {
            await waitForCallback((done) => recognizer.close(done, done));
          }
          try {
            connection?.close();
          } catch (error) {
            console.warn("Failed to dispose Azure Speech connection", error);
          }
          try {
            audioConfig?.close();
          } catch (error) {
            console.warn("Failed to dispose Azure audio config", error);
          }
        } catch (error) {
          console.warn("Azure Speech cleanup failed", error);
        } finally {
          reportStatus(runtime, "resources-closed");
        }
      })();
      return runtime.cleanupPromise;
    };

    const queueTranslation = (runtime: AzureRuntime) => {
      if (!runtime.deeplEnabled) return;
      if (runtime.translationTimer) clearTimeout(runtime.translationTimer);
      runtime.translationTimer = setTimeout(() => {
        runtime.translationTimer = null;
        runtime.segmentChain = runtime.segmentChain.then(async () => {
          const result = await commands.azureSpeechTranslatePending(
            runtime.sessionId,
          );
          commandError(result);
        });
      }, runtime.translationDelayMs);
    };

    const scheduleTokenRefresh = (
      runtime: AzureRuntime,
      delayMs = TOKEN_REFRESH_INTERVAL_MS,
    ) => {
      if (runtime.tokenRefreshTimer) clearTimeout(runtime.tokenRefreshTimer);
      runtime.tokenRefreshTimer = setTimeout(async () => {
        runtime.tokenRefreshTimer = null;
        if (
          runtimeRef.current !== runtime ||
          runtime.stopping ||
          !runtime.recognizer
        )
          return;

        try {
          const authResult = await commands.getAzureSpeechAuth();
          if (authResult.status === "error") throw new Error(authResult.error);
          if (runtimeRef.current !== runtime || !runtime.recognizer) return;
          runtime.recognizer.authorizationToken = authResult.data.token;
          scheduleTokenRefresh(runtime);
        } catch (error) {
          console.warn("Failed to refresh Azure Speech token", error);
          scheduleTokenRefresh(runtime, TOKEN_REFRESH_RETRY_MS);
        }
      }, delayMs);
    };

    const finishRuntime = async (runtime: AzureRuntime, sdkError?: string) => {
      if (runtime.stopping) return;
      runtime.stopping = true;
      if (runtime.translationTimer) clearTimeout(runtime.translationTimer);
      runtime.translationTimer = null;

      let finalError = sdkError;
      try {
        await runtime.segmentChain;
        if (!finalError && runtime.deeplEnabled) {
          const translated = await commands.azureSpeechTranslatePending(
            runtime.sessionId,
          );
          commandError(translated);
        }
      } catch (error) {
        finalError = error instanceof Error ? error.message : String(error);
      }

      reportStatus(
        runtime,
        `audio-stats-rx-${runtime.receivedAudioBytes}-bytes-${runtime.receivedAudioChunks}-chunks-written-${runtime.writtenAudioBytes}-bytes-${runtime.writtenAudioChunks}-chunks-pending-${runtime.pendingAudio.length}-coverage-${runtime.recognizedAudioEndMs}-ms`,
      );

      const completed = await commands.azureSpeechComplete(
        runtime.sessionId,
        finalError ?? null,
      );
      if (completed.status === "error") {
        console.warn(
          "Failed to complete Azure Speech session",
          completed.error,
        );
      }
      try {
        await cleanup(runtime);
      } finally {
        if (runtimeRef.current === runtime) runtimeRef.current = null;
      }
    };

    const stopRuntime = (runtime: AzureRuntime) => {
      if (runtime.stopping) return;
      reportStatus(runtime, "stop-requested");
      if (!runtime.recognizer || !runtime.pushStream) {
        runtime.stopRequested = true;
        return;
      }

      if (!runtime.tailPaddingWritten) {
        // A synthetic silence tail gives Azure a clean phrase boundary when
        // the shortcut is released in the middle of a word. It is pushed
        // immediately, so it adds no wall-clock recording delay.
        runtime.pushStream.write(
          new ArrayBuffer((16_000 * 2 * FINAL_SILENCE_MS) / 1_000),
        );
        runtime.tailPaddingWritten = true;
      }
      runtime.pushStream.close();
      runtime.stopFallbackTimer = setTimeout(
        () => void finishRuntime(runtime),
        runtime.deeplEnabled ? TRANSLATED_STOP_FALLBACK_MS : STOP_FALLBACK_MS,
      );
      runtime.recognizer.stopContinuousRecognitionAsync(
        () => void finishRuntime(runtime),
        (error) => void finishRuntime(runtime, String(error)),
      );
    };

    const startRuntime = async (sessionId: number) => {
      if (runtimeRef.current) {
        const previousRuntime = runtimeRef.current;
        try {
          await cleanup(previousRuntime);
        } finally {
          if (runtimeRef.current === previousRuntime) runtimeRef.current = null;
        }
      }
      const runtime: AzureRuntime = {
        sessionId,
        pushStream: null,
        audioConfig: null,
        recognizer: null,
        connection: null,
        pendingAudio: [],
        segmentChain: Promise.resolve(),
        translationTimer: null,
        tokenRefreshTimer: null,
        stopFallbackTimer: null,
        translationDelayMs: 2_000,
        deeplEnabled: false,
        stopRequested: false,
        stopping: false,
        cleanupPromise: null,
        receivedAudioBytes: 0,
        receivedAudioChunks: 0,
        writtenAudioBytes: 0,
        writtenAudioChunks: 0,
        recognizedAudioEndMs: 0,
        keepAliveRelease: null,
        tailPaddingWritten: false,
      };
      runtimeRef.current = runtime;
      reportStatus(runtime, "start-received");
      acquireBackgroundKeepAlive(runtime);

      try {
        const authResult = await commands.getAzureSpeechAuth();
        if (authResult.status === "error") throw new Error(authResult.error);
        if (runtimeRef.current !== runtime) return;
        const auth = authResult.data;
        reportStatus(runtime, "auth-ready");
        runtime.translationDelayMs = auth.translation_delay_ms;
        runtime.deeplEnabled = auth.deepl_enabled;

        const speechConfig = SpeechSDK.SpeechConfig.fromAuthorizationToken(
          auth.token,
          auth.region,
        );
        speechConfig.speechRecognitionLanguage = auth.language;
        speechConfig.setProperty(
          SpeechSDK.PropertyId
            .SpeechServiceResponse_StablePartialResultThreshold,
          String(auth.stable_partial_threshold),
        );
        speechConfig.setProperty(
          SpeechSDK.PropertyId.Speech_SegmentationSilenceTimeoutMs,
          String(auth.segmentation_silence_ms),
        );
        speechConfig.setProperty(
          SpeechSDK.PropertyId.Recognizer_StopTimeoutMs,
          auth.deepl_enabled ? "2500" : "600",
        );

        const format = SpeechSDK.AudioStreamFormat.getWaveFormatPCM(
          16_000,
          16,
          1,
        );
        runtime.pushStream =
          SpeechSDK.AudioInputStream.createPushStream(format);
        runtime.audioConfig = SpeechSDK.AudioConfig.fromStreamInput(
          runtime.pushStream,
        );
        runtime.recognizer = new SpeechSDK.SpeechRecognizer(
          speechConfig,
          runtime.audioConfig,
        );
        runtime.connection = SpeechSDK.Connection.fromRecognizer(
          runtime.recognizer,
        );
        runtime.connection.connected = () => reportStatus(runtime, "connected");
        runtime.connection.disconnected = () =>
          reportStatus(runtime, "disconnected");
        scheduleTokenRefresh(runtime);

        runtime.recognizer.sessionStarted = () => {
          reportStatus(runtime, "session-started");
        };

        runtime.recognizer.recognizing = (_sender, event) => {
          const text = event.result.text?.trim();
          if (!text || runtimeRef.current !== runtime) return;
          runtime.recognizedAudioEndMs = Math.max(
            runtime.recognizedAudioEndMs,
            recognitionEndMs(event.result),
          );
          void commands.azureSpeechInterim(
            runtime.sessionId,
            text,
            runtime.recognizedAudioEndMs,
          );
        };

        runtime.recognizer.recognized = (_sender, event) => {
          if (event.result.reason !== SpeechSDK.ResultReason.RecognizedSpeech)
            return;
          const text = event.result.text?.trim();
          if (!text || runtimeRef.current !== runtime) return;
          runtime.recognizedAudioEndMs = Math.max(
            runtime.recognizedAudioEndMs,
            recognitionEndMs(event.result),
          );
          runtime.segmentChain = runtime.segmentChain.then(async () => {
            // azure_speech_final is a fire-and-forget command (returns void,
            // no Result envelope) — errors are surfaced server-side via
            // transcription-error events.
            await commands.azureSpeechFinal(
              runtime.sessionId,
              text,
              runtime.recognizedAudioEndMs,
            );
          });
          queueTranslation(runtime);
        };

        runtime.recognizer.canceled = (_sender, event) => {
          reportStatus(runtime, `canceled-${event.reason}`);
          if (event.reason !== SpeechSDK.CancellationReason.Error) return;
          void finishRuntime(
            runtime,
            event.errorDetails || "Azure Speech canceled",
          );
        };

        runtime.recognizer.sessionStopped = () => {
          reportStatus(runtime, "session-stopped");
          void finishRuntime(runtime);
        };

        runtime.recognizer.startContinuousRecognitionAsync(
          () => {
            for (const chunk of runtime.pendingAudio)
              writeAudio(runtime, chunk);
            runtime.pendingAudio = [];
            if (runtime.stopRequested) stopRuntime(runtime);
          },
          (error) => void finishRuntime(runtime, String(error)),
        );
      } catch (error) {
        await finishRuntime(
          runtime,
          error instanceof Error ? error.message : String(error),
        );
      }
    };

    const setup = async () => {
      await Promise.all([
        retainListenerUnlessDisposed(
          listen<AzureStreamControlPayload>("azure-speech-start", (event) => {
            void startRuntime(event.payload.session_id);
          }),
          () => disposed,
          (unlisten) => unlisteners.push(unlisten),
        ),
        retainListenerUnlessDisposed(
          listen<AzureAudioChunkPayload>("azure-speech-audio", (event) => {
            const runtime = runtimeRef.current;
            if (
              !runtime ||
              runtime.stopping ||
              runtime.sessionId !== event.payload.session_id
            )
              return;
            const chunk = decodeBase64(event.payload.pcm_base64);
            runtime.receivedAudioBytes += chunk.byteLength;
            runtime.receivedAudioChunks += 1;
            if (!runtime.recognizer || !writeAudio(runtime, chunk))
              runtime.pendingAudio.push(chunk);
          }),
          () => disposed,
          (unlisten) => unlisteners.push(unlisten),
        ),
        retainListenerUnlessDisposed(
          listen<AzureStreamControlPayload>("azure-speech-stop", (event) => {
            const runtime = runtimeRef.current;
            if (!runtime || runtime.sessionId !== event.payload.session_id)
              return;
            stopRuntime(runtime);
          }),
          () => disposed,
          (unlisten) => unlisteners.push(unlisten),
        ),
        retainListenerUnlessDisposed(
          listen<AzureStreamControlPayload>("azure-speech-cancel", (event) => {
            const runtime = runtimeRef.current;
            if (!runtime || runtime.sessionId !== event.payload.session_id)
              return;
            runtimeRef.current = null;
            void cleanup(runtime);
          }),
          () => disposed,
          (unlisten) => unlisteners.push(unlisten),
        ),
      ]);
    };

    void setup().catch((error) => {
      console.warn("Failed to register Azure Speech listeners", error);
    });
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
      unlisteners.length = 0;
      if (runtimeRef.current) void cleanup(runtimeRef.current);
      runtimeRef.current = null;
    };
  }, []);
};
