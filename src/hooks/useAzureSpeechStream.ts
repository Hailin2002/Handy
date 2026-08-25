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
const STOP_FALLBACK_MS = 3_000;

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

    const reportStatus = (runtime: AzureRuntime, status: string) => {
      void commands.azureSpeechStatus(runtime.sessionId, status);
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

      runtime.pushStream.close();
      runtime.stopFallbackTimer = setTimeout(
        () => void finishRuntime(runtime),
        STOP_FALLBACK_MS,
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
      };
      runtimeRef.current = runtime;
      reportStatus(runtime, "start-received");

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
          "2500",
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
          void commands.azureSpeechInterim(runtime.sessionId, text);
        };

        runtime.recognizer.recognized = (_sender, event) => {
          if (event.result.reason !== SpeechSDK.ResultReason.RecognizedSpeech)
            return;
          const text = event.result.text?.trim();
          if (!text || runtimeRef.current !== runtime) return;
          runtime.segmentChain = runtime.segmentChain.then(async () => {
            // azure_speech_final is a fire-and-forget command (returns void,
            // no Result envelope) — errors are surfaced server-side via
            // transcription-error events.
            await commands.azureSpeechFinal(runtime.sessionId, text);
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
              runtime.pushStream?.write(chunk);
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
      unlisteners.push(
        await listen<AzureStreamControlPayload>(
          "azure-speech-start",
          (event) => {
            void startRuntime(event.payload.session_id);
          },
        ),
      );
      unlisteners.push(
        await listen<AzureAudioChunkPayload>("azure-speech-audio", (event) => {
          const runtime = runtimeRef.current;
          if (!runtime || runtime.sessionId !== event.payload.session_id)
            return;
          const chunk = decodeBase64(event.payload.pcm_base64);
          if (runtime.pushStream && runtime.recognizer)
            runtime.pushStream.write(chunk);
          else runtime.pendingAudio.push(chunk);
        }),
      );
      unlisteners.push(
        await listen<AzureStreamControlPayload>(
          "azure-speech-stop",
          (event) => {
            const runtime = runtimeRef.current;
            if (!runtime || runtime.sessionId !== event.payload.session_id)
              return;
            stopRuntime(runtime);
          },
        ),
      );
      unlisteners.push(
        await listen<AzureStreamControlPayload>(
          "azure-speech-cancel",
          (event) => {
            const runtime = runtimeRef.current;
            if (!runtime || runtime.sessionId !== event.payload.session_id)
              return;
            runtimeRef.current = null;
            void cleanup(runtime);
          },
        ),
      );
    };

    void setup();
    return () => {
      for (const unlisten of unlisteners) unlisten();
      if (runtimeRef.current) void cleanup(runtimeRef.current);
      runtimeRef.current = null;
    };
  }, []);
};
