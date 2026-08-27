import React, { useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  MEETING_SHORTCUT_ACTIONS,
  createDefaultMeetingShortcutStore,
  createTauriMeetingShortcutOs,
  type MeetingShortcutActionDef,
  type MeetingShortcutOs,
  type MeetingShortcutStore,
  type ShortcutLocation,
} from "./meetingShortcutRules";
import { MeetingShortcutRowModel } from "./meetingShortcutRowModel";
import { formatKeyCombination } from "../../../lib/utils/keyboard";

/**
 * §6.9 meeting shortcut editor (docs/meeting-mode-plan.md) on the
 * Meetings settings tab. Every rule is implemented in
 * `meetingShortcutRules.ts` / `meetingShortcutRowModel.ts`; this file is
 * the view adapter.
 *
 * Injected seams:
 * - `os` probes OS registration before anything is saved (tests inject a
 *   deterministic fake; production uses the Tauri global-shortcut plugin);
 * - `store` persists under exactly `settings.meetings.shortcut.<action>`.
 *
 * Operator handoff: the Rust AppSettings struct has no `meetings` section
 * yet, so the default store persists to localStorage. When the backend
 * gains `settings.meetings.shortcut`, repoint
 * `createDefaultMeetingShortcutStore` — nothing else changes.
 */
export interface MeetingShortcutEditorProps {
  os?: MeetingShortcutOs;
  store?: MeetingShortcutStore;
  /** Speak dictation bindings as conflict locations; defaults to the live settings store. */
  speakBindings?: readonly ShortcutLocation[];
  suspendBindings?: () => Promise<unknown>;
  resumeBindings?: () => Promise<unknown>;
}

/** Lazy Tauri seam so importing this module never touches IPC in tests. */
const defaultSuspendBindings = async (): Promise<unknown> => {
  const { commands } = await import("@/bindings");
  return commands.suspendAllBindings();
};
const defaultResumeBindings = async (): Promise<unknown> => {
  const { commands } = await import("@/bindings");
  return commands.resumeAllBindings();
};

export const MeetingShortcutEditor: React.FC<MeetingShortcutEditorProps> = ({
  os = createTauriMeetingShortcutOs(),
  store = createDefaultMeetingShortcutStore(),
  speakBindings,
  suspendBindings = defaultSuspendBindings,
  resumeBindings = defaultResumeBindings,
}) => {
  const { t } = useTranslation();
  const bindingsMap = useSettingsStore((s) => s.settings?.bindings);

  const resolvedSpeakBindings = useMemo<readonly ShortcutLocation[]>(() => {
    if (speakBindings) return speakBindings;
    return Object.entries(bindingsMap ?? {}).flatMap(([id, binding]) =>
      binding
        ? [
            {
              kind: "speak" as const,
              id,
              label: t(`settings.general.shortcut.bindings.${id}.name`, {
                defaultValue: binding.name,
              }),
              hotkey: binding.current_binding,
            },
          ]
        : [],
    );
  }, [speakBindings, bindingsMap, t]);

  // Bumped whenever any row saves, so cross-meeting conflict locations
  // re-read from the injected store.
  const [meetingValuesVersion, bumpMeetingValuesVersion] = useState(0);
  const meetingLocations = useMemo<readonly ShortcutLocation[]>(
    () =>
      MEETING_SHORTCUT_ACTIONS.map((action) => ({
        kind: "meeting" as const,
        id: action.id,
        label: t(action.nameKey),
        hotkey: store.read(action.id),
      })),
    // meetingValuesVersion intentionally invalidates the store reads: the
    // injected store is mutable outside React's knowledge, so bumping the
    // counter is the only signal that `store.read` will return something new.
    // The rule cannot see that and reports it as an unnecessary dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [store, t, meetingValuesVersion],
  );

  // Editor-wide lock: only one row may capture at a time.
  const captureLock = useRef({ acquired: false });
  const sharedCaptureLock = useMemo(
    () => ({
      acquire: () => {
        if (captureLock.current.acquired) return false;
        captureLock.current.acquired = true;
        return true;
      },
      release: () => {
        captureLock.current.acquired = false;
      },
    }),
    [],
  );

  return (
    <div className="flex flex-col gap-2">
      <div className="px-4 p-2">
        <h3 className="text-sm font-medium">
          {t("settings.meetings.shortcut.title")}
        </h3>
        <p className="text-sm text-mid-gray">
          {t("settings.meetings.shortcut.description")}
        </p>
      </div>
      {MEETING_SHORTCUT_ACTIONS.map((action) => (
        <MeetingShortcutRow
          key={action.id}
          action={action}
          os={os}
          store={store}
          suspendBindings={suspendBindings}
          resumeBindings={resumeBindings}
          captureLock={sharedCaptureLock}
          onChanged={() => bumpMeetingValuesVersion((v) => v + 1)}
          getConflictLocations={() => [
            ...resolvedSpeakBindings,
            ...meetingLocations.filter(
              (location) =>
                !(location.kind === "meeting" && location.id === action.id),
            ),
          ]}
        />
      ))}
    </div>
  );
};

interface MeetingShortcutRowProps {
  action: MeetingShortcutActionDef;
  os: MeetingShortcutOs;
  store: MeetingShortcutStore;
  suspendBindings: () => Promise<unknown>;
  resumeBindings: () => Promise<unknown>;
  captureLock: { acquire: () => boolean; release: () => void };
  onChanged: () => void;
  getConflictLocations: () => readonly ShortcutLocation[];
}

const MeetingShortcutRow: React.FC<MeetingShortcutRowProps> = ({
  action,
  os,
  store,
  suspendBindings,
  resumeBindings,
  captureLock,
  onChanged,
  getConflictLocations,
}) => {
  const { t } = useTranslation();

  // Keep the latest conflict accessor reachable from the model without
  // recreating the model (its state must survive re-renders).
  const conflictRef = useRef(getConflictLocations);
  conflictRef.current = getConflictLocations;

  const modelRef = useRef<MeetingShortcutRowModel | null>(null);
  if (!modelRef.current) {
    modelRef.current = new MeetingShortcutRowModel({
      action,
      os,
      store,
      getConflictLocations: () => conflictRef.current(),
      suspendBindings,
      resumeBindings,
      captureLock,
    });
  }
  const model = modelRef.current;
  useSyncExternalStore(model.subscribe, model.getVersion, model.getVersion);

  const display =
    model.value !== null
      ? formatKeyCombination(model.value, "unknown")
      : t("settings.meetings.shortcut.unassigned");
  const busy = model.capturing || model.testing;

  return (
    <SettingCard
      title={t(action.nameKey)}
      description={t(action.descriptionKey)}
    >
      {/* Live region announces capture start/end and probe results */}
      <div aria-live="polite">
        {model.capturing ? (
          <span
            className="inline-block px-2 py-1 text-sm font-semibold border border-logo-primary bg-logo-primary/30 rounded-md"
            role="status"
          >
            {t("settings.meetings.shortcut.pressKeys")}
            <span className="sr-only">
              {t("settings.meetings.shortcut.escapeHint")}
            </span>
          </span>
        ) : (
          <button
            type="button"
            className="px-2 py-1 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 hover:bg-logo-primary/10 rounded-md cursor-pointer hover:border-logo-primary min-w-[44px] min-h-[44px]"
            aria-label={t("settings.meetings.shortcut.recordButtonAria", {
              name: t(action.nameKey),
              shortcut: display,
            })}
            onClick={() => void model.beginCapture()}
          >
            {display}
          </button>
        )}
      </div>

      <div className="flex items-center gap-1 flex-wrap">
        <button
          type="button"
          disabled={busy}
          className="px-2 py-1 text-sm rounded-md border border-mid-gray/40 hover:bg-logo-primary/10 disabled:opacity-50 disabled:cursor-not-allowed min-h-[44px]"
          onClick={() => void model.resetToDefault().then(onChanged)}
        >
          {t("settings.meetings.shortcut.reset")}
        </button>
        <button
          type="button"
          disabled={busy || model.value === null}
          className="px-2 py-1 text-sm rounded-md border border-mid-gray/40 hover:bg-logo-primary/10 disabled:opacity-50 disabled:cursor-not-allowed min-h-[44px]"
          onClick={() => {
            model.disable();
            onChanged();
          }}
        >
          {t("settings.meetings.shortcut.disable")}
        </button>
        <button
          type="button"
          disabled={busy || model.value === null}
          className="px-2 py-1 text-sm rounded-md border border-mid-gray/40 hover:bg-logo-primary/10 disabled:opacity-50 disabled:cursor-not-allowed min-h-[44px]"
          onClick={() => void model.testCurrent()}
        >
          {model.testing
            ? t("settings.meetings.shortcut.testing")
            : t("settings.meetings.shortcut.test")}
        </button>
      </div>

      {model.error?.kind === "invalid" && (
        <p role="alert" className="text-sm text-red-500 px-4">
          {t(model.error.errorKey)}
        </p>
      )}
      {model.error?.kind === "conflict" && (
        <MeetingShortcutConflictList conflicts={model.error.conflicts} />
      )}
      {model.error?.kind === "os" && (
        <p role="alert" className="text-sm text-red-500 px-4">
          {t("settings.meetings.shortcut.errors.osRejected", {
            error: model.error.error,
          })}
        </p>
      )}

      {model.testFeedback && (
        <p
          role="status"
          className={`text-sm px-4 ${
            model.testFeedback.ok ? "text-green-600" : "text-red-500"
          }`}
        >
          {model.testFeedback.ok
            ? t("settings.meetings.shortcut.testOk", {
                shortcut: formatKeyCombination(
                  model.testFeedback.hotkey,
                  "unknown",
                ),
              })
            : t("settings.meetings.shortcut.testFail", {
                shortcut: formatKeyCombination(
                  model.testFeedback.hotkey,
                  "unknown",
                ),
                error: model.testFeedback.error ?? "",
              })}
        </p>
      )}
    </SettingCard>
  );
};

/** Stacked setting card matching the tab's visual language. */
const SettingCard: React.FC<{
  title: string;
  description: string;
  children: React.ReactNode;
}> = ({ title, description, children }) => (
  <div className="flex items-start justify-between gap-4 px-4 p-2 rounded-lg border border-mid-gray/20">
    <div className="max-w-2/3">
      <h4 className="text-sm font-medium">{title}</h4>
      <p className="text-sm text-mid-gray">{description}</p>
      <div className="mt-2 flex flex-col gap-2">{children}</div>
    </div>
  </div>
);

/**
 * G2 display surface: every conflicting location is rendered
 * simultaneously, each with its action label and current binding.
 * Exported so the both-locations-visible requirement can be asserted
 * directly.
 */
export const MeetingShortcutConflictList: React.FC<{
  conflicts: readonly ShortcutLocation[];
}> = ({ conflicts }) => {
  const { t } = useTranslation();
  return (
    <div role="alert" className="text-sm text-red-500 px-4">
      <p>{t("settings.meetings.shortcut.errors.conflict")}</p>
      <ul className="list-disc list-inside">
        {conflicts.map((conflict) => (
          <li key={`${conflict.kind}:${conflict.id}`}>
            {conflict.label}
            {conflict.hotkey !== null &&
              ` (${formatKeyCombination(conflict.hotkey, "unknown")})`}
          </li>
        ))}
      </ul>
    </div>
  );
};
