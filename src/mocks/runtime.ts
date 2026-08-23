import type { MockTauriInternals, RegisteredCallback, SpeakMockBridge } from "./types";
import { LISTEN_SENTINEL, MockBackend, handleCommand } from "./backend";
import type { SpeakMockConfig } from "./types";

/**
 * Builds the mocked IPC runtime for one test run: a {@link MockBackend} plus
 * the `window.__TAURI_INTERNALS__` implementation consumed by
 * `@tauri-apps/api` and a test-facing bridge.
 */
export interface MockRuntime {
  backend: MockBackend;
  internals: MockTauriInternals;
  bridge: SpeakMockBridge;
}

export const createMockRuntime = (config: SpeakMockConfig): MockRuntime => {
  const backend = new MockBackend(config);
  const callbacks = new Map<number, RegisteredCallback>();
  let nextCallbackId = 1;

  const invoke = (
    cmd: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> => {
    const safeArgs: Record<string, unknown> = args ?? {};
    backend.record(cmd, safeArgs);

    // Bind an already-registered callback to its event name so simulated
    // backend events can find their listeners.
    if (cmd === "plugin:event|listen") {
      const handlerArg = safeArgs.handler;
      const handlerId = typeof handlerArg === "number" ? handlerArg : -1;
      const entry = callbacks.get(handlerId);
      if (entry && typeof safeArgs.event === "string") {
        entry.event = safeArgs.event;
      }
      return Promise.resolve(handlerId);
    }

    const handled = handleCommand(backend, cmd, safeArgs);
    if (handled === LISTEN_SENTINEL || handled === undefined) {
      const message = `[e2e-mock] unhandled IPC command: ${cmd}`;
      console.warn(message, safeArgs);
      return Promise.reject(new Error(message));
    }
    return Promise.resolve(handled);
  };

  const transformCallback = (
    callback: (response: unknown) => void,
    once = false,
  ): number => {
    const id = nextCallbackId;
    nextCallbackId += 1;
    callbacks.set(id, { handler: callback, once, event: null });
    return id;
  };

  const unregisterCallback = (id: number): void => {
    callbacks.delete(id);
  };

  const convertFileSrc = (filePath: string, protocol = "asset"): string =>
    `http://asset.localhost/${encodeURIComponent(filePath)}` +
    `?protocol=${encodeURIComponent(protocol)}`;

  const emitEvent = (event: string, payload?: unknown): void => {
    for (const [id, entry] of [...callbacks]) {
      if (entry.event !== event) continue;
      try {
        entry.handler({ event, id, payload });
      } catch (error) {
        console.error(`[e2e-mock] listener for "${event}" threw:`, error);
      }
      if (entry.once) callbacks.delete(id);
    }
  };

  return {
    backend,
    internals: {
      invoke,
      transformCallback,
      unregisterCallback,
      convertFileSrc,
    },
    bridge: {
      getSettings: () => ({ ...backend.settings }),
      getInvocations: () =>
        backend.invocations.map((invocation) => ({
          command: invocation.command,
          args: { ...invocation.args },
        })),
      emitEvent,
    },
  };
};
