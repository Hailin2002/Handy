import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, HardDrive, Mic, Play } from "lucide-react";
import { toast } from "sonner";
import { commands, type MeetingSession } from "@/bindings";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { useMeetingStore } from "@/stores/meetingStore";
import { MeetingPreflight } from "./MeetingPreflight";
import { meetingErrorCopy } from "./meetingErrorCopy";

export interface MeetingSetupProps {
  /** Persisted draft session this dialog will start. */
  session: MeetingSession;
  /** Back to the meetings library; the draft stays persisted. */
  onBack: () => void;
  /** Called once the backend confirmed the session left `draft`. */
  onStarted: () => void;
  /** Called after a successful title save so the library can refresh. */
  onSaved?: (session: MeetingSession) => void;
  onOpenSettings?: () => void;
}

/**
 * Compact start dialog for recurring users (plan §6.7).
 *
 * Fits on one screen and contains exactly what the plan lists: the start
 * validation status lines (delegated to `MeetingPreflight`), an optional
 * title pre-filled with date and time, the summary switch, a short storage
 * hint, exactly one primary button `Meeting starten` and a secondary link to
 * the remaining settings.
 *
 * Colour logic (§6.8): the start button uses the normal brand/primary colour —
 * red is reserved for a recording that is actually running. The consent notice
 * (§12, copy catalog P1) gates the primary action: Speak states the consent
 * obligation before the first byte is captured and is deliberately not a
 * stealth recorder.
 */
export const MeetingSetup: React.FC<MeetingSetupProps> = ({
  session,
  onBack,
  onStarted,
  onSaved,
  onOpenSettings,
}) => {
  const { t } = useTranslation();
  const [title, setTitle] = useState(session.title);
  const [saving, setSaving] = useState(false);
  const [summaryEnabled, setSummaryEnabled] = useState(true);
  const [consent, setConsent] = useState(false);
  const [microphoneOnly, setMicrophoneOnly] = useState(false);
  const startingRef = useRef(false);

  const preflight = useMeetingStore((state) => state.preflight);
  const preflightStatus = useMeetingStore((state) => state.preflightStatus);
  const runPreflight = useMeetingStore((state) => state.runPreflight);
  const startMeeting = useMeetingStore((state) => state.startMeeting);
  const busy = useMeetingStore((state) => state.busy);
  const error = useMeetingStore((state) => state.error);
  const errorCopy = meetingErrorCopy(error);

  useEffect(() => {
    void runPreflight();
  }, [runPreflight]);

  const dirty = title.trim() !== session.title && title.trim().length > 0;

  const persistTitle = useCallback(async () => {
    if (!dirty || saving) return;
    setSaving(true);
    try {
      const result = await commands.meetingRenameSession(session.id, title.trim());
      if (result.status === "ok") {
        onSaved?.(result.data);
      } else {
        toast.error(t("meetings.saveError"), { description: result.error });
      }
    } catch (cause) {
      console.error("Failed to save meeting title:", cause);
      toast.error(t("meetings.saveError"));
    } finally {
      setSaving(false);
    }
  }, [dirty, saving, session.id, title, onSaved, t]);

  // The start button only becomes available once the devices can be opened,
  // enough storage is free and the provider data is present (plan
  // "Startvalidierung"), and only after the consent acknowledgement (P1).
  const canStart =
    Boolean(preflight?.can_start) &&
    preflightStatus === "ready" &&
    consent &&
    !busy;

  const handleStart = async () => {
    // Synchronous re-entry guard: `canStart` derives from store state that
    // updates asynchronously, so a same-frame double-click would otherwise
    // fire two start commands (the second failing loudly against the
    // capture-owner guard).
    if (startingRef.current) return;
    if (!canStart) return;
    startingRef.current = true;
    try {
      if (dirty) await persistTitle();
      const started = await startMeeting({
        meetingId: session.id,
        title: title.trim() || session.title,
        microphoneOnly,
        summaryEnabled,
        consentAcknowledged: consent,
      });
      if (started) onStarted();
    } finally {
      startingRef.current = false;
    }
  };

  return (
    <section className="flex w-full max-w-2xl flex-col gap-4" aria-label={t("meeting.setup.title")}>
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="mr-1 h-4 w-4" aria-hidden="true" />
          {t("meetings.backToList")}
        </Button>
      </div>

      <header className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold">{t("meeting.setup.title")}</h2>
        <p className="text-sm text-text/60">{t("meeting.setup.description")}</p>
        <p className="text-xs text-text/50">{t("meeting.session.draft.hint")}</p>
      </header>

      <label className="flex flex-col gap-1 text-sm">
        <span className="font-medium">{t("meetings.titleInputLabel")}</span>
        <Input
          value={title}
          onChange={(event) => setTitle(event.target.value)}
          onBlur={() => void persistTitle()}
          aria-label={t("meetings.titleInputLabel")}
        />
      </label>
      <div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void persistTitle()}
          disabled={!dirty || saving}
        >
          {t("meetings.saveTitle")}
        </Button>
      </div>

      <MeetingPreflight
        report={preflight}
        status={preflightStatus}
        onRecheck={() => void runPreflight()}
        onOpenSettings={() => onOpenSettings?.()}
        onRecordMicOnly={() => setMicrophoneOnly(true)}
        microphoneOnly={microphoneOnly}
      />

      {microphoneOnly && (
        <p className="flex items-center gap-2 text-sm text-warning">
          <Mic className="h-4 w-4 shrink-0" aria-hidden="true" />
          {t("meeting.session.micOnly.badge")}
        </p>
      )}

      {/* Summary switch (§6.7). A plain checkbox keeps the whole dialog
          keyboard operable with a single tab order (§6.14). */}
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="h-4 w-4 accent-logo-primary"
          checked={summaryEnabled}
          onChange={(event) => setSummaryEnabled(event.target.checked)}
        />
        <span>
          <span className="font-medium">{t("meeting.setup.summaryToggle")}</span>
          <span className="block text-xs text-text/50">
            {t("meeting.setup.summaryHint")}
          </span>
        </span>
      </label>

      <div className="rounded-lg border border-mid-gray/20 bg-mid-gray/5 p-3">
        <h3 className="text-sm font-medium">
          {t("meeting.consent.beforeStart.title")}
        </h3>
        <p className="mt-1 text-xs text-text/70">
          {t("meeting.consent.beforeStart.body")}
        </p>
        <label className="mt-2 flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="h-4 w-4 accent-logo-primary"
            checked={consent}
            onChange={(event) => setConsent(event.target.checked)}
          />
          {t("meeting.consent.beforeStart.acknowledge")}
        </label>
      </div>

      <p className="flex items-center gap-2 text-xs text-text/50">
        <HardDrive className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        {t("meeting.setup.storageHint")}
      </p>

      {errorCopy && (
        <Alert variant={errorCopy.severity}>
          <span className="font-medium">{t(errorCopy.titleKeys)}</span>{" "}
          {t(errorCopy.bodyKeys)}
        </Alert>
      )}

      {/* Exactly one obvious primary action (§6.2 rule 2), icon plus text
          (rule 3), brand colour rather than red (§6.8). */}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="lg" onClick={() => void handleStart()} disabled={!canStart}>
          <Play className="mr-2 h-4 w-4" aria-hidden="true" />
          {t("meeting.actions.startMeeting")}
        </Button>
        <Button variant="secondary" onClick={onBack}>
          {t("meeting.actions.cancel")}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => onOpenSettings?.()}>
          {t("meeting.setup.moreSettings")}
        </Button>
      </div>
    </section>
  );
};

export default MeetingSetup;
