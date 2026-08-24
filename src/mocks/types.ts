import type { AppSettings, ModelInfo } from "@/bindings";
import type {
  MeetingPreflightReport,
  MeetingRecoveryOffer,
} from "@/lib/types/meeting";

/**
 * Browser-side mock layer for the Tauri IPC bridge used by Playwright tests.
 *
 * The modules in this directory never run inside the real Tauri webview: they
 * are only loaded when a test injects them ahead of `src/main.tsx` (see
 * `tests/helpers/tauri-mock.ts`). They replace `window.__TAURI_INTERNALS__`
 * so that every `@tauri-apps/api` call resolves against deterministic
 * in-memory fixtures instead of a Rust backend.
 */

/** Shape of the arguments object Tauri passes for one command invocation. */
export type MockCommandArgs = Record<string, unknown>;

/** A single command handler. May return the value directly or a promise. */
export type MockCommandHandler = (args: MockCommandArgs) => unknown;

/** One recorded IPC invocation, kept for assertions from the tests. */
export interface MockInvocation {
  command: string;
  args: MockCommandArgs;
}

/**
 * Per-test configuration for the mock backend.
 */
export interface SpeakMockConfig {
  /** Overrides merged on top of {@link DEFAULT_MOCK_SETTINGS}. */
  settings?: Partial<AppSettings>;
  /** Models returned by `get_available_models` / shown in the selector. */
  models?: ModelInfo[];
  /** Model id reported by `get_current_model`. */
  currentModel?: string;
  /** Version reported by the app plugin (`getVersion`). */
  appVersion?: string;
  /** System locale reported by the os plugin. */
  locale?: string;
  /** Report returned by `preflight_meeting` (defaults to all-green). */
  meetingPreflight?: MeetingPreflightReport;
  /** Pending crash-recovery offers served by `meeting_list_recoveries`. */
  meetingRecoveries?: MeetingRecoveryOffer[];
}

/**
 * Test-facing handle installed on `window` so Playwright can inspect state
 * and drive simulated backend events through `page.evaluate`.
 */
export interface SpeakMockBridge {
  getSettings(): AppSettings;
  getInvocations(): readonly MockInvocation[];
  /** Simulates a Rust-side `emit` to all listeners registered for `event`. */
  emitEvent(event: string, payload?: unknown): void;
}

/**
 * Subset of `window.__TAURI_INTERNALS__` consumed by `@tauri-apps/api`.
 * Only the members actually referenced by the API version pinned in
 * `package.json` are declared here.
 */
export interface MockTauriInternals {
  invoke(cmd: string, args?: MockCommandArgs): Promise<unknown>;
  transformCallback(
    callback: (response: unknown) => void,
    once?: boolean,
  ): number;
  unregisterCallback(id: number): void;
  convertFileSrc(filePath: string, protocol?: string): string;
}

/** Internal bookkeeping for a registered IPC callback. */
export interface RegisteredCallback {
  handler: (response: unknown) => void;
  once: boolean;
  /** Event name bound via `plugin:event|listen`, if any. */
  event: string | null;
}

/**
 * Synchronous OS facts consumed by `@tauri-apps/plugin-os`
 * (`window.__TAURI_OS_PLUGIN_INTERNALS__`) are NOT redeclared here: the
 * plugin's own `dist-js/index.d.ts` already augments `Window` with that
 * property, and a duplicate declaration would break global merging.
 */

declare global {
  interface Window {
    __TAURI_INTERNALS__?: MockTauriInternals;
    __SPEAK_E2E_MOCK_BRIDGE__?: SpeakMockBridge;
    __SPEAK_E2E_MOCK_CONFIG__?: SpeakMockConfig;
    isTauri?: boolean;
  }
}

