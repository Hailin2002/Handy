import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, Pause, Play, Square } from "lucide-react";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { MeetingStateBadge } from "@/components/shared";
import { formatMeetingDuration } from "@/components/shell/productNav";
import type { MeetingAudioSource } from "@/lib/types/meeting";
import {
  useMeetingElapsedSeconds,
  useMeetingStore,
} from "@/stores/meetingStore";
import { meetingErrorCopy } from "./meetingErrorCopy";

export interface MeetingRecorderViewProps {
  /** Invoked once the backend confirmed the session left the capture states. */
  onStopped: () => void;
}

/**
 * Running meeting view (plan §6.11) with the three mandated layers:
 *
 * 1. a fixed status bar (`Aufnahme läuft`/`Pausiert`, duration, local or cloud
 *    provider, `Audio gespeichert` or a concrete warning);
 * 2. the main area with the most recently finished transcript segments and
 *    their timestamps, plus the honest listening/transcribing/behind state —
 *    and no auto-scroll once the user scrolled up, where a separate
 *    `Zum aktuellen Text` button appears instead;
 * 3. fixed controls: pause/resume, end meeting, small separate microphone and
 *    system level meters with text labels, and the waiting-sections count only
 *    when a backlog actually exists.
 *
 * Everything rendered here is a projection of `meetingStore`; the view owns no
 * recording state of its own (plan line 927).
 */
const formatTimestamp = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
};

const LevelMeter: React.FC<{
  source: MeetingAudioSource;
  level: number;
}> = ({ source, level }) => {
  const { t } = useTranslation();
  const label = t(
    source === "microphone"
      ? "meeting.live.levels.microphone"
      : "meeting.live.levels.system",
  );
  const percent = Math.round(Math.min(1, Math.max(0, level)) * 100);

  return (
    <div className="flex min-w-40 flex-col gap-1" data-level-source={source}>
      {/* Text label next to the bar: the meter is never colour-only (§6.8),
          and screen readers get the value through the same visible text
          (§6.14 "Screenreader-Namen für alle Icons, Pegel und
          Statusänderungen"). */}
      <span className="flex items-center justify-between text-xs text-text/60">
        <span>{label}</span>
        <span className="tabular-nums" data-level-percent={percent}>
          {t("meeting.live.levels.value", { label: "", percent }).trim()}
        </span>
      </span>
      <span
        role="meter"
        aria-label={label}
        aria-valuenow={percent}
        aria-valuemin={0}
        aria-valuemax={100}
        className="block h-1.5 w-full overflow-hidden rounded-full bg-mid-gray/20"
      >
        <span
          className="block h-full rounded-full bg-logo-primary transition-[width] duration-100 motion-reduce:transition-none"
          style={{ width: `${percent}%` }}
        />
      </span>
    </div>
  );
};

export const MeetingRecorderView: React.FC<MeetingRecorderViewProps> = ({
  onStopped,
}) => {
  const { t } = useTranslation();
  const session = useMeetingStore((state) => state.session);
  const levels = useMeetingStore((state) => state.levels);
  const transcript = useMeetingStore((state) => state.transcript);
  const jobs = useMeetingStore((state) => state.jobs);
  const busy = useMeetingStore((state) => state.busy);
  const error = useMeetingStore((state) => state.error);
  const pauseMeeting = useMeetingStore((state) => state.pauseMeeting);
  const resumeMeeting = useMeetingStore((state) => state.resumeMeeting);
  const stopMeeting = useMeetingStore((state) => state.stopMeeting);
  const elapsedSeconds = useMeetingElapsedSeconds();

  const [confirmingStop, setConfirmingStop] = useState(false);
  const [following, setFollowing] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);
  const errorCopy = meetingErrorCopy(error);

  const state = session?.state;
  const paused = state === "paused";
  const stopping = state === "stopping";

  // The state event is the UI's permission to leave the recording view
  // (SSC T08/T09): never a local timer, never an optimistic guess.
  useEffect(() => {
    if (state && !["starting", "recording", "paused", "stopping"].includes(state)) {
      onStopped();
    }
  }, [state, onStopped]);

  // "Keine automatisch springende Ansicht, sobald der Nutzer nach oben
  // scrollt" (§6.11): following is disabled by scrolling away from the end.
  const handleScroll = useCallback(() => {
    const node = scrollRef.current;
    if (!node) return;
    const atBottom =
      node.scrollHeight - node.scrollTop - node.clientHeight < 24;
    setFollowing(atBottom);
  }, []);

  useEffect(() => {
    const node = scrollRef.current;
    if (!node || !following) return;
    node.scrollTop = node.scrollHeight;
  }, [transcript, following]);

  if (!session) return null;

  const backlog = jobs?.total_backlog ?? 0;
  const running = Object.values(jobs?.per_source ?? {}).some(
    (counts) => (counts?.running ?? 0) > 0,
  );
  const liveStatusKey = paused
    ? "meeting.session.paused.title"
    : backlog > 3
      ? "meeting.live.status.behind"
      : running
        ? "meeting.live.status.transcribing"
        : "meeting.live.status.listening";

  const providerLabel =
    session.providerKind === "cloud"
      ? t("meeting.session.recording.provider.cloud", {
          provider: session.providerName ?? "",
        })
      : t("meeting.session.recording.provider.local");

  const handleStop = async () => {
    setConfirmingStop(false);
    await stopMeeting();
  };

  return (
    <section
      className="flex h-full w-full flex-col gap-3"
      aria-label={t("meeting.session.recording.title")}
    >
      {/* 1 — fixed status bar (§6.11) */}
      <header className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-mid-gray/20 bg-mid-gray/5 px-4 py-3">
        <MeetingStateBadge
          state={paused ? "paused" : stopping ? "stopping" : "recording"}
        />
        <span
          className="text-lg font-semibold tabular-nums"
          aria-label={t("meeting.session.durationLabel", {
            duration: formatMeetingDuration(elapsedSeconds),
          })}
        >
          {formatMeetingDuration(elapsedSeconds)}
        </span>
        <span className="text-sm text-text/70">{providerLabel}</span>
        <span className="text-sm text-success">
          {t("meeting.session.recording.saved")}
        </span>
        {session.microphoneOnly && (
          <span className="text-sm text-warning">
            {t("meeting.session.micOnly.badge")}
          </span>
        )}
        {/* Consent indicator, visible for the whole recording (§12 / P2). */}
        <span className="w-full text-xs text-text/60">
          {t("meeting.consent.indicator.label")}
        </span>
      </header>

      {stopping && (
        <Alert variant="info">
          <span className="font-medium">
            {t("meeting.session.stopping.title")}
          </span>{" "}
          {t("meeting.session.stopping.body")}
        </Alert>
      )}

      {paused && <Alert variant="info">{t("meeting.session.paused.body")}</Alert>}

      {errorCopy && (
        <Alert variant={errorCopy.severity}>
          <span className="font-medium">{t(errorCopy.titleKeys)}</span>{" "}
          {t(errorCopy.bodyKeys)}
        </Alert>
      )}

      {/* 2 — main area: finished segments with timestamps (§6.11) */}
      <div className="flex min-h-40 flex-1 flex-col gap-2">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium">{t("meeting.live.heading")}</h3>
          {/* Honest live state, announced politely rather than by moving
              focus (§6.14). */}
          <span className="text-xs text-text/60" role="status">
            {t(liveStatusKey)}
          </span>
        </div>
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="max-h-72 min-h-32 overflow-y-auto rounded-lg border border-mid-gray/20 p-3"
        >
          {transcript.length === 0 ? (
            <p className="text-sm text-text/50">
              {t("meeting.live.transcriptEmpty")}
            </p>
          ) : (
            <ol className="flex flex-col gap-2">
              {transcript.map((segment) => (
                <li key={segment.id} className="flex gap-3 text-sm">
                  <span className="shrink-0 tabular-nums text-text/50">
                    {formatTimestamp(segment.start_ms)}
                  </span>
                  <span>{segment.text}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
        {!following && (
          <div>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setFollowing(true);
                const node = scrollRef.current;
                if (node) node.scrollTop = node.scrollHeight;
              }}
            >
              <ArrowDown className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("meeting.actions.jumpToLatest")}
            </Button>
          </div>
        )}
      </div>

      {/* 3 — fixed controls (§6.11) */}
      <footer className="flex flex-wrap items-end justify-between gap-4 rounded-lg border border-mid-gray/20 bg-mid-gray/5 px-4 py-3">
        <div className="flex flex-wrap items-center gap-2">
          {paused ? (
            <Button onClick={() => void resumeMeeting()} disabled={busy}>
              <Play className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("meeting.actions.resume")}
            </Button>
          ) : (
            <Button
              variant="secondary"
              onClick={() => void pauseMeeting()}
              disabled={busy || stopping}
            >
              <Pause className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("meeting.actions.pause")}
            </Button>
          )}
          {/* Red plus stop symbol plus text (§6.8); confirmation before a
              meeting can be ended by accident (§6.15 release gate). */}
          <Button
            variant="danger"
            onClick={() => setConfirmingStop(true)}
            disabled={busy || stopping}
          >
            <Square className="mr-1 h-4 w-4" aria-hidden="true" />
            {t("meeting.actions.endMeeting")}
          </Button>
        </div>

        <div className="flex flex-wrap items-end gap-4">
          <LevelMeter source="microphone" level={levels.microphone} />
          <LevelMeter source="system" level={levels.system} />
          {/* Only shown when a backlog actually exists (§6.11). */}
          {backlog > 0 && (
            <span className="text-xs text-info">
              {t("meeting.live.backlog", { count: backlog })}
            </span>
          )}
        </div>
      </footer>

      <Dialog
        open={confirmingStop}
        title={t("meeting.live.stopConfirm.title")}
        description={t("meeting.live.stopConfirm.body")}
        closeLabel={t("meeting.actions.close")}
        onOpenChange={(open) => setConfirmingStop(open)}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setConfirmingStop(false)}
            >
              {t("meeting.actions.keepRecording")}
            </Button>
            <Button variant="danger" size="sm" onClick={() => void handleStop()}>
              <Square className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("meeting.actions.endMeeting")}
            </Button>
          </div>
        }
      >
        {/* The description carries the explanation; the body stays empty so
            the dialog keeps a single, clear message. */}
        <span />
      </Dialog>
    </section>
  );
};

export default MeetingRecorderView;
