import type { MeetingSession, ModelInfo, Project } from "@/bindings";
import type { MeetingRecoveryOffer } from "@/lib/types/meeting";
import type { MockInvocation, SpeakMockConfig } from "./types";
import {
  handleMeetingLifecycleCommand,
  initialRecoveries,
  type MeetingLifecycleState,
} from "./meetingLifecycle";
import { applySettingWrite } from "./settingCommands";
import {
  DEFAULT_MOCK_APP_VERSION,
  DEFAULT_MOCK_CURRENT_MODEL,
  DEFAULT_MOCK_LOCALE,
  DEFAULT_MOCK_SETTINGS,
  MOCK_MODELS,
} from "./fixtures";

/**
 * In-memory stand-in for the Rust backend: settings store, model catalog,
 * meeting sessions and the invocation log used by tests to assert what
 * crossed the IPC bridge.
 */
export class MockBackend {
  readonly config: SpeakMockConfig;

  settings: typeof DEFAULT_MOCK_SETTINGS;

  models: ModelInfo[];

  currentModel: string;

  /** Mirror of `MeetingManager` (SQLite) for e2e runs of the meetings journey. */
  meetings: MeetingSession[];

  /** Per-meeting session-lifecycle simulation (see `meetingLifecycle.ts`). */
  readonly lifecycles: Map<string, MeetingLifecycleState>;

  /** Pending crash-recovery offers served by `meeting_list_recoveries`. */
  recoveries: MeetingRecoveryOffer[];

  /**
   * Mirror of the `projects` table.
   *
   * The Projects surface shipped without mock coverage, so every e2e run hit
   * `unhandled IPC command: project_list` and the page rendered as if the user
   * had no projects. Nothing failed, which is why it went unnoticed — the
   * failure mode was a *plausible* screen.
   */
  projects: Project[];

  /**
   * Bound by the runtime to its `emitEvent` so lifecycle handlers can publish
   * `meeting://*` events exactly like the Rust side would.
   */
  emit?: (event: string, payload?: unknown) => void;

  readonly invocations: MockInvocation[];

  constructor(config: SpeakMockConfig) {
    this.config = config;
    this.settings = mergeSettings(config);
    this.models = config.models ?? MOCK_MODELS;
    this.currentModel =
      config.currentModel ??
      config.settings?.selected_model ??
      DEFAULT_MOCK_CURRENT_MODEL;
    this.meetings = [];
    this.lifecycles = new Map();
    this.recoveries = initialRecoveries(config.meetingRecoveries);
    this.projects = [];
    this.invocations = [];
  }

  record(command: string, args: Record<string, unknown>): void {
    this.invocations.push({ command, args });
  }
}

const mergeSettings = (
  config: SpeakMockConfig,
): typeof DEFAULT_MOCK_SETTINGS => ({
  ...DEFAULT_MOCK_SETTINGS,
  ...config.settings,
});

/**
 * Resolves one IPC command. Returns `undefined` when no handler matches so
 * the runtime can fail loudly on genuinely unimplemented commands.
 *
 * Handlers return the RAW payload exactly as the Rust backend would serialize
 * it over the wire: the tauri-specta generated bindings in `src/bindings.ts`
 * add their own `{ status: "ok", data }` wrapper around every invoke result,
 * so wrapping here would double-encode and break every `result.data` reader.
 */
export const handleCommand = (
  backend: MockBackend,
  command: string,
  args: Record<string, unknown>,
): unknown | undefined => {
  // Persisted setting writes (tauri-specta Result commands resolve to null).
  if (applySettingWrite(backend, command, args)) return null;

  // --- meetings journey (mirror of managers/meeting.rs) -------------------
  const meetingHandler = handleMeetingCommand(backend, command, args);
  if (meetingHandler !== undefined) return meetingHandler;

  // --- meeting session lifecycle (mirror of the planned MeetingManager) ---
  const lifecycleHandler = handleMeetingLifecycleCommand(
    backend,
    command,
    args,
  );
  if (lifecycleHandler !== undefined) return lifecycleHandler;

  switch (command) {
    // --- projects ---------------------------------------------------------
    case "project_list":
      return backend.projects.map((p) => ({ ...p }));

    case "project_inbox_meeting_ids":
      // Meetings not assigned to any project. The mock has no assignment
      // table yet, so every alive meeting is unassigned — which is the
      // correct answer while `projects` is empty and an honest placeholder
      // once it is not.
      return backend.meetings
        .filter((m) => (m as MockMeetingRow).deleted_at === null)
        .map((m) => m.id);

    case "project_meeting_ids":
      // No assignment table in the mock yet, so a project has no members. The
      // page must render that as an empty membership list, not as a failure.
      return [];

    // --- WhatsApp import (text paths; the zip paths need a file dialog) ----
    case "whatsapp_preview_text": {
      const content = typeof args.content === "string" ? args.content : "";
      // Close enough to managers/whatsapp_importer.rs for the wizard: one
      // message per "[date, time] Author: text" line, media markers counted.
      const lines = content.split("\n").filter((l) => l.trim());
      const authors = new Set<string>();
      let media = 0;
      for (const line of lines) {
        const m = /^\[[^\]]+\]\s*([^:]+):\s*(.*)$/.exec(line);
        if (!m) continue;
        authors.add(m[1].trim());
        if (/<attached:|<Medien ausgeschlossen>|<Media omitted>/.test(m[2])) {
          media++;
        }
      }
      return {
        participants: [...authors],
        message_count: lines.length,
        media_count: media,
        date_order_ambiguous: false,
      };
    }

    case "whatsapp_import_text": {
      const title = typeof args.title === "string" ? args.title.trim() : "";
      const body = typeof args.content === "string" ? args.content : "";
      const now = Date.now();
      return {
        source: {
          id: `src_${now}`,
          source_type: "whatsapp_export",
          title,
          ingestion_state: "imported",
          parser_version: "whatsapp-txt-v1",
          content_hash: String(now),
          created_at: now,
        },
        created: true,
        item_count: body.split("\n").filter((l) => l.trim()).length,
      };
    }

    case "source_list":
    case "source_items":
    case "source_assets":
      return [];

    case "task_list":
      // No task store in the mock; the page must render an empty task list
      // rather than surfacing a failure.
      return [];

    case "project_create": {
      const name = typeof args.name === "string" ? args.name.trim() : "";
      const now = Date.now();
      const project: Project = {
        id: `proj_${backend.projects.length + 1}`,
        name,
        created_at: now,
        updated_at: now,
      } as Project;
      backend.projects.push(project);
      return { ...project };
    }

    case "project_delete": {
      const before = backend.projects.length;
      backend.projects = backend.projects.filter((p) => p.id !== args.id);
      return before !== backend.projects.length;
    }

    // --- app / window plumbing -------------------------------------------
    case "initialize_enigo":
    case "initialize_shortcuts":
    case "show_main_window_command":
    case "suspend_all_bindings":
    case "resume_all_bindings":
    case "play_test_sound":
    case "cancel_operation":
    case "rescan_local_models":
    case "set_selected_microphone":
    case "set_selected_output_device":
    case "set_clamshell_microphone":
    case "set_selected_channel":
    case "set_log_level":
    case "update_custom_words":
    case "set_model_unload_timeout":
    case "open_recordings_folder":
    case "open_log_dir":
    case "open_app_data_dir":
    case "open_microphone_privacy_settings":
    case "change_binding":
    case "reset_binding":
      return null;

    case "set_active_model": {
      const modelId = args.modelId;
      if (typeof modelId === "string") backend.currentModel = modelId;
      return null;
    }

    // --- tauri-specta readers (raw payloads; bindings add the Result) -----
    case "get_app_settings":
      return { ...backend.settings };
    case "get_default_settings":
      return { ...DEFAULT_MOCK_SETTINGS };
    case "get_app_dir_path":
      return "C:\\mock\\speak\\data";
    case "get_log_dir_path":
      return "C:\\mock\\speak\\logs";
    case "get_available_microphones":
      return [
        { index: "default", name: "Default", is_default: true },
        { index: "mic-0", name: "Mock Microphone", is_default: false },
      ];
    case "get_available_output_devices":
      return [{ index: "default", name: "Default", is_default: true }];
    case "get_microphone_channels":
      return 2;
    case "get_selected_microphone":
    case "get_selected_output_device":
    case "get_clamshell_microphone":
      return "default";
    case "get_history_entries":
      return { entries: [], has_more: false };
    case "fetch_post_process_models":
      return [];
    case "is_laptop":
      return false;

    // --- raw (non-Result) bindings ---------------------------------------
    case "get_available_models":
      return backend.models.map((model) => ({ ...model }));
    case "get_current_model":
      return backend.currentModel;
    case "get_transcription_model_status":
      return backend.currentModel.length > 0 ? backend.currentModel : null;
    case "check_custom_sounds":
      return { start: false, stop: false };
    case "is_recording":
      return false;
    case "is_portable":
      return false;
    case "get_secure_input_status":
      return {
        enabled: false,
        sustained: false,
        culprit_pid: null,
        culprit_name: null,
        fallback_active: false,
        covered_bindings: [],
        degraded_bindings: [],
        uncovered_bindings: [],
        recorder_blocked: false,
      };
    case "get_windows_microphone_permission_status":
      return {
        supported: false,
        overall_access: "allowed",
        device_access: "allowed",
        app_access: "allowed",
        desktop_app_access: "allowed",
      };
    case "get_keyboard_implementation":
      return "tauri";
    case "get_available_accelerators":
      return { transcribe: ["cpu"], ort: ["cpu"], gpu_devices: [] };
    case "azure_speech_status":
      return null;
    case "test_azure_speech_connection":
      return "disabled in e2e mocks";

    // --- plugins -----------------------------------------------------------
    case "plugin:os|platform":
    case "plugin:os|type":
      return "linux";
    case "plugin:os|family":
      return "unix";
    case "plugin:os|arch":
      return "x86_64";
    case "plugin:os|version":
      return "6.6.0";
    case "plugin:os|locale":
      return backend.config.locale ?? DEFAULT_MOCK_LOCALE;
    case "plugin:os|hostname":
      return "e2e-host";
    case "plugin:app|version":
      return backend.config.appVersion ?? DEFAULT_MOCK_APP_VERSION;
    case "plugin:app|name":
      return "Speak";
    case "plugin:app|os":
      return "linux";
    case "plugin:app|identifier":
      return "com.e2e.speak";
    case "plugin:event|listen":
      // Handled by the runtime (needs the callback registry).
      return LISTEN_SENTINEL;
    case "plugin:event|unlisten":
      return null;
    case "plugin:event|emit":
      return null;
    case "plugin:updater|check":
      // No update available.
      return null;
    case "plugin:process|relaunch":
    case "plugin:process|exit":
      return null;
    case "plugin:opener|open_url":
    case "plugin:opener|open_path":
      return null;
    case "plugin:dialog|ask":
    case "plugin:dialog|confirm":
      return true;
    case "plugin:dialog|message":
      return null;
    default:
      return undefined;
  }
};

// Marker consumed by the runtime; never returned to the frontend.
export const LISTEN_SENTINEL = Symbol("mock-event-listen");

/** Mock-side session row; carries the soft-delete marker internally only. */
type MockMeetingRow = MeetingSession & { deleted_at: number | null };

/**
 * In-memory mirror of the Rust `MeetingManager` semantics (soft delete,
 * newest-first list, title LIKE filter). Returns `undefined` when `command`
 * is not a meeting command so the main switch can handle it (or fail loudly).
 */
const handleMeetingCommand = (
  backend: MockBackend,
  command: string,
  args: Record<string, unknown>,
): unknown | undefined => {
  const alive = () =>
    backend.meetings.filter((m) => (m as MockMeetingRow).deleted_at === null);
  switch (command) {
    case "meeting_create_session": {
      const now = Date.now();
      const title =
        typeof args.title === "string" && args.title.trim().length > 0
          ? args.title.trim()
          : "Meeting";
      const session: MockMeetingRow = {
        id: crypto.randomUUID(),
        title,
        state: "draft",
        started_at: now,
        ended_at: null,
        created_at: now,
        updated_at: now,
        deleted_at: null,
      };
      backend.meetings.push(session);
      const { deleted_at: _deleted, ...visible } = session;
      return { ...visible };
    }
    case "meeting_get_session":
      return alive().find((m) => m.id === args.id) ?? null;
    case "meeting_list_sessions": {
      const query = typeof args.query === "string" ? args.query : "";
      // Newest first like the manager (`ORDER BY created_at DESC, rowid DESC`):
      // array index stands in for rowid so same-millisecond inserts keep the
      // newest-insert-first order instead of JS sort stability preserving
      // insertion order.
      return alive()
        .map((m, idx) => ({ m, idx }))
        .filter(
          ({ m }) =>
            query === "" || m.title.toLowerCase().includes(query.toLowerCase()),
        )
        .sort((a, b) => b.m.created_at - a.m.created_at || b.idx - a.idx)
        .map(({ m }) => {
          const { deleted_at: _deleted, ...visible } = m as MockMeetingRow;
          return { ...visible };
        });
    }
    case "meeting_rename_session": {
      const session = alive().find((m) => m.id === args.id);
      if (!session) throw new Error(`Meeting ${String(args.id)} not found`);
      const title = typeof args.title === "string" ? args.title.trim() : "";
      if (!title) throw new Error("Meeting title must not be empty");
      session.title = title;
      session.updated_at = Date.now();
      // Strip the internal soft-delete marker: the Rust command returns the
      // `MeetingSession` DTO only, so the mock must not leak `deleted_at`.
      const { deleted_at: _deleted, ...visible } = session as MockMeetingRow;
      return { ...visible };
    }
    case "meeting_delete_session": {
      const session = alive().find((m) => m.id === args.id);
      if (!session) return false;
      (session as MockMeetingRow).deleted_at = Date.now();
      session.updated_at = Date.now();
      return true;
    }
    default:
      return undefined;
  }
};
