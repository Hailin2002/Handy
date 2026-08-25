import React from "react";
import { useTranslation } from "react-i18next";
import { Circle, CircleCheck, CircleDot } from "lucide-react";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { MeetingStateBadge } from "@/components/shared";
import { useMeetingStore } from "@/stores/meetingStore";
import { meetingErrorCopy } from "./meetingErrorCopy";

export interface MeetingProcessingViewProps {
  /** Leaves the processing view; jobs keep running in the background. */
  onContinueInBackground: () => void;
  /** Opens the finished meeting in the detail view. */
  onOpenMeeting: (meetingId: string) => void;
}

/** One checklist line; the three states are visually and textually distinct. */
type StepState = "pending" | "running" | "done";

const STEP_ICON: Record<StepState, React.ReactNode> = {
  done: <CircleCheck className="h-4 w-4 text-success" aria-hidden="true" />,
  running: <CircleDot className="h-4 w-4 text-info" aria-hidden="true" />,
  pending: <Circle className="h-4 w-4 text-text/40" aria-hidden="true" />,
};

const STEP_STATE_KEY: Record<StepState, string> = {
  done: "meeting.processing.state.done",
  running: "meeting.processing.state.running",
  pending: "meeting.processing.state.pending",
};

/**
 * Processing page shown after stopping (plan §6.12).
 *
 * The plan forbids pretending a finished summary while jobs are still
 * running and prescribes an explicit checklist:
 *
 *     ✓ Aufnahme sicher gespeichert
 *     ✓ 42 von 42 Audioabschnitten vorbereitet
 *     ● 37 von 42 Abschnitten transkribiert
 *     ○ Zusammenfassung wartet
 *
 * Together with §6.2 rule 5 ("Fortschritt ist ehrlich") this is why
 * "audio saved", "transcribing" and "summarizing" are four separate rows with
 * separate states rather than one blended progress bar — and why no
 * indeterminate percentage is rendered when the total is unknown.
 */
export const MeetingProcessingView: React.FC<MeetingProcessingViewProps> = ({
  onContinueInBackground,
  onOpenMeeting,
}) => {
  const { t } = useTranslation();
  const session = useMeetingStore((state) => state.session);
  const jobs = useMeetingStore((state) => state.jobs);
  const busy = useMeetingStore((state) => state.busy);
  const error = useMeetingStore((state) => state.error);
  const retryFailedJobs = useMeetingStore((state) => state.retryFailedJobs);
  const errorCopy = meetingErrorCopy(error);

  if (!session) return null;

  const perSource = Object.values(jobs?.per_source ?? {});
  const sum = (pick: (counts: NonNullable<(typeof perSource)[number]>) => number) =>
    perSource.reduce((total, counts) => total + (counts ? pick(counts) : 0), 0);

  const succeeded = jobs?.succeeded_total ?? sum((c) => c.succeeded);
  const failed = sum((c) => c.failed);
  const eligible =
    jobs?.eligible_total ??
    succeeded + failed + sum((c) => c.queued + c.running + c.retry_wait);
  const preparedTotal = jobs?.prepared_total ?? eligible;
  const preparedDone = jobs?.prepared_done ?? preparedTotal;
  const backlog = jobs?.total_backlog ?? 0;

  const transcribing = session.state === "transcribing";
  const summarizing = session.state === "summarizing";
  const completed = session.state === "completed";

  const transcriptionState: StepState = completed || summarizing
    ? "done"
    : transcribing
      ? "running"
      : "pending";
  const summaryState: StepState = !session.summaryEnabled
    ? "pending"
    : completed
      ? "done"
      : summarizing
        ? "running"
        : "pending";

  const rows: { key: string; label: string; state: StepState }[] = [
    {
      key: "saved",
      label: t("meeting.processing.step.saved"),
      state: "done",
    },
    {
      key: "prepared",
      label: t("meeting.processing.step.prepared", {
        done: preparedDone,
        total: preparedTotal,
      }),
      state: preparedDone >= preparedTotal ? "done" : "running",
    },
    {
      key: "transcribed",
      label: t("meeting.processing.step.transcribed", {
        done: succeeded,
        total: eligible,
      }),
      state: transcriptionState,
    },
    {
      key: "summary",
      label: session.summaryEnabled
        ? summarizing
          ? t("meeting.processing.summarizing")
          : t("meeting.processing.step.summary")
        : t("meeting.processing.step.summarySkipped"),
      state: summaryState,
    },
  ];

  return (
    <section
      className="flex w-full max-w-2xl flex-col gap-4"
      aria-label={t("meeting.processing.title")}
    >
      <header className="flex flex-wrap items-center gap-3">
        <h2 className="text-lg font-semibold">
          {completed
            ? t("meeting.session.completed.title")
            : t("meeting.processing.title")}
        </h2>
        <MeetingStateBadge state={completed ? "complete" : "processing"} />
      </header>

      <p className="text-sm text-text/70">{t("meeting.session.stopping.done")}</p>

      <ol className="flex flex-col gap-2" data-processing-checklist="">
        {rows.map((row) => (
          <li
            key={row.key}
            className="flex items-center gap-2 text-sm"
            data-step={row.key}
            data-step-state={row.state}
          >
            {STEP_ICON[row.state]}
            <span>{row.label}</span>
            {/* Never colour or icon alone: each row states its stage. */}
            <span className="text-xs text-text/50">
              {t(STEP_STATE_KEY[row.state])}
            </span>
          </li>
        ))}
      </ol>

      {backlog > 0 && (
        <p className="text-sm text-info">
          {t("meeting.processing.backlog", { count: backlog })}
        </p>
      )}

      {failed > 0 && (
        <Alert variant="warning">
          {t("meeting.processing.failed", { count: failed })}
        </Alert>
      )}

      {errorCopy && (
        <Alert variant={errorCopy.severity}>
          <span className="font-medium">{t(errorCopy.titleKeys)}</span>{" "}
          {t(errorCopy.bodyKeys)}
        </Alert>
      )}

      {completed ? (
        <p className="text-sm text-success">
          {t("meeting.session.completed.saved")}
        </p>
      ) : (
        <p className="text-sm text-text/60">{t("meeting.processing.closeOk")}</p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {completed ? (
          <Button onClick={() => onOpenMeeting(session.meetingId)}>
            {t("meeting.actions.openMeeting")}
          </Button>
        ) : (
          <Button onClick={onContinueInBackground}>
            {t("meeting.actions.workInBackground")}
          </Button>
        )}
        {failed > 0 && (
          <Button
            variant="secondary"
            onClick={() => void retryFailedJobs()}
            disabled={busy}
          >
            {t("meeting.actions.retryFailedSegments")}
          </Button>
        )}
        <Button variant="ghost" onClick={onContinueInBackground}>
          {t("meeting.actions.close")}
        </Button>
      </div>
    </section>
  );
};

export default MeetingProcessingView;
