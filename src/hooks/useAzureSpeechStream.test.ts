import { describe, expect, it, vi } from "vitest";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { retainListenerUnlessDisposed } from "./useAzureSpeechStream";

describe("Azure Speech listener lifecycle", () => {
  it("releases a listener that resolves after its effect was disposed", async () => {
    let resolveRegistration: ((unlisten: UnlistenFn) => void) | undefined;
    const registration = new Promise<UnlistenFn>((resolve) => {
      resolveRegistration = resolve;
    });
    const unlisten = vi.fn();
    const retained: UnlistenFn[] = [];
    let disposed = false;

    const pending = retainListenerUnlessDisposed(
      registration,
      () => disposed,
      (listener) => retained.push(listener),
    );
    disposed = true;
    resolveRegistration?.(unlisten);
    await pending;

    expect(unlisten).toHaveBeenCalledOnce();
    expect(retained).toEqual([]);
  });

  it("retains a listener while its effect is active", async () => {
    const unlisten = vi.fn();
    const retained: UnlistenFn[] = [];

    await retainListenerUnlessDisposed(
      Promise.resolve(unlisten),
      () => false,
      (listener) => retained.push(listener),
    );

    expect(unlisten).not.toHaveBeenCalled();
    expect(retained).toEqual([unlisten]);
  });
});
