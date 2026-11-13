import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, RotateCcw, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import {
  commands,
  type MeetingAccuracyReport,
  type MeetingSnapshot,
  type MeetingSession,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";

type Provider = "local" | "azure" | "custom";

interface MeetingProcessingPanelProps {
  session: MeetingSession;
  snapshot: MeetingSnapshot | null;
  /** Reloads snapshot, transcript and summary after a job-changing action. */
  onChanged: () => void;
}

/**
 * `Verarbeitung` tab (plan §6.13): "Provider, Modell, Jobs, Fehler und
 * Wiederholungen".
 *
 * Every number here comes from the persisted job aggregates rather than a
 * local guess, so the tab cannot claim progress the backend does not have.
 * Re-transcription lives behind a focused dialog because it discards the
 * current transcript — the plan keeps that kind of decision out of a row of
 * toolbar buttons.
 */
export const MeetingProcessingPanel: React.FC<MeetingProcessingPanelProps> = ({
  session,
  snapshot,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [retrying, setRetrying] = useState(false);
  const [redoOpen, setRedoOpen] = useState(false);
  const [redoing, setRedoing] = useState(false);
  const [provider, setProvider] = useState<Provider>("local");
  const [accuracyOpen, setAccuracyOpen] = useState(false);
  const [accuracyRunning, setAccuracyRunning] = useState(false);
  const [accuracyProvider, setAccuracyProvider] = useState<Provider>("azure");
  const [accuracyReport, setAccuracyReport] =
    useState<MeetingAccuracyReport | null>(null);

  useEffect(() => {
    const name = snapshot?.provider_name?.toLowerCase() ?? "";
    const current: Provider = name.includes("azure")
      ? "azure"
      : name.includes("custom")
        ? "custom"
        : "local";
    setProvider(current);
    setAccuracyProvider(current === "azure" ? "local" : "azure");
  }, [snapshot?.provider_name]);

  const jobs = snapshot?.jobs ?? null;
  const failed = jobs?.oldest_failed ?? null;

  const rows: Array<{ key: string; label: string; value: string }> = [
    {
      key: "provider",
      label: t("meeting.detail.processing.provider"),
      value:
        snapshot?.provider_name ??
        t("meeting.detail.processing.providerUnknown"),
    },
    {
      key: "location",
      label: t("meeting.detail.processing.location"),
      value:
        snapshot?.provider_kind === "cloud"
          ? t("meeting.detail.processing.locationCloud")
          : t("meeting.detail.processing.locationLocal"),
    },
    {
      key: "backlog",
      label: t("meeting.detail.processing.backlog"),
      value: String(jobs?.total_backlog ?? 0),
    },
    {
      key: "succeeded",
      label: t("meeting.detail.processing.succeeded"),
      value: `${jobs?.succeeded_total ?? 0} / ${jobs?.eligible_total ?? 0}`,
    },
  ];

  const handleRetry = async () => {
    if (retrying) return;
    setRetrying(true);
    try {
      const result = await commands.retryMeetingJobs(session.id);
      if (result.status === "ok") {
        toast.success(t("meeting.detail.processing.retryQueued"));
        onChanged();
      } else {
        toast.error(t("meeting.detail.processing.retryFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      console.error("Retry failed:", error);
      toast.error(t("meeting.detail.processing.retryFailed"));
    } finally {
      setRetrying(false);
    }
  };

  const handleRetranscribe = async () => {
    if (redoing) return;
    setRedoing(true);
    try {
      const result = await commands.retranscribeMeeting(session.id, provider);
      if (result.status === "ok") {
        toast.success(t("meeting.detail.processing.retranscribeQueued"));
        onChanged();
      } else {
        toast.error(t("meeting.detail.processing.retranscribeFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      console.error("Re-transcription failed:", error);
      toast.error(t("meeting.detail.processing.retranscribeFailed"));
    } finally {
      setRedoing(false);
      setRedoOpen(false);
    }
  };

  const handleImproveAccuracy = async () => {
    if (accuracyRunning) return;
    setAccuracyRunning(true);
    setAccuracyReport(null);
    try {
      const result = await commands.improveMeetingAccuracy(
        session.id,
        accuracyProvider,
      );
      if (result.status === "ok") {
        setAccuracyReport(result.data);
        toast.success(
          t("meeting.detail.processing.accuracyDone", {
            count: result.data.chunksCorrected,
          }),
        );
        onChanged();
      } else {
        toast.error(t("meeting.detail.processing.accuracyFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      console.error("Accuracy improvement failed:", error);
      toast.error(t("meeting.detail.processing.accuracyFailed"));
    } finally {
      setAccuracyRunning(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        {rows.map((row) => (
          <React.Fragment key={row.key}>
            <dt className="text-text/50">{row.label}</dt>
            <dd className="text-text/80">{row.value}</dd>
          </React.Fragment>
        ))}
      </dl>

      {failed && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-warning/30 bg-warning/10 p-3">
          <span className="text-sm">
            {t("meeting.detail.processing.failedJobs", {
              count: failed.count,
            })}
          </span>
          <Button size="sm" onClick={handleRetry} disabled={retrying}>
            <RefreshCw className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
            {t("meeting.detail.processing.retry")}
          </Button>
        </div>
      )}

      <div className="flex flex-col gap-2 rounded-lg border border-mid-gray/20 p-3">
        <h3 className="text-sm font-medium">
          {t("meeting.detail.processing.retranscribeHeading")}
        </h3>
        <p className="text-xs text-text/60">
          {t("meeting.detail.processing.retranscribeBody")}
        </p>
        <div>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setRedoOpen(true)}
          >
            <RotateCcw className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
            {t("meeting.detail.processing.retranscribe")}
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-2 rounded-lg border border-logo-primary/25 bg-logo-primary/5 p-3">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <ShieldCheck
            className="h-4 w-4 text-logo-primary"
            aria-hidden="true"
          />
          {t("meeting.detail.processing.accuracyHeading")}
        </h3>
        <p className="text-xs text-text/60">
          {t("meeting.detail.processing.accuracyBody")}
        </p>
        {accuracyReport && (
          <p className="text-xs text-text/80">
            {t("meeting.detail.processing.accuracyResult", {
              checked: accuracyReport.chunksChecked,
              corrected: accuracyReport.chunksCorrected,
              removed: accuracyReport.silenceHallucinationsRemoved,
            })}
          </p>
        )}
        <div>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setAccuracyOpen(true)}
            disabled={
              snapshot?.state !== "completed" &&
              snapshot?.state !== "recoverable_error"
            }
          >
            <ShieldCheck className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
            {t("meeting.detail.processing.accuracyAction")}
          </Button>
        </div>
      </div>

      <Dialog
        open={redoOpen}
        title={t("meeting.detail.processing.retranscribeHeading")}
        description={t("meeting.detail.processing.retranscribeWarning")}
        closeLabel={t("common.close")}
        onOpenChange={(open) => !redoing && setRedoOpen(open)}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setRedoOpen(false)}
              disabled={redoing}
            >
              {t("common.close")}
            </Button>
            <Button size="sm" onClick={handleRetranscribe} disabled={redoing}>
              <RotateCcw className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
              {t("meeting.detail.processing.retranscribe")}
            </Button>
          </div>
        }
      >
        <label className="flex flex-col gap-1 text-sm">
          <span className="font-medium">
            {t("meeting.setup.providerLabel")}
          </span>
          <select
            className="rounded-md border border-mid-gray/30 bg-background px-2 py-1.5 text-sm"
            value={provider}
            onChange={(event) => setProvider(event.target.value as Provider)}
            disabled={redoing}
          >
            <option value="local">{t("meeting.setup.providerLocal")}</option>
            <option value="azure">{t("meeting.setup.providerAzure")}</option>
            <option value="custom">{t("meeting.setup.providerCustom")}</option>
          </select>
        </label>
      </Dialog>

      <Dialog
        open={accuracyOpen}
        title={t("meeting.detail.processing.accuracyHeading")}
        description={t("meeting.detail.processing.accuracyWarning")}
        closeLabel={t("common.close")}
        onOpenChange={(open) => !accuracyRunning && setAccuracyOpen(open)}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setAccuracyOpen(false)}
              disabled={accuracyRunning}
            >
              {t("common.close")}
            </Button>
            <Button
              size="sm"
              onClick={handleImproveAccuracy}
              disabled={accuracyRunning}
            >
              <ShieldCheck className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
              {accuracyRunning
                ? t("meeting.detail.processing.accuracyRunning")
                : t("meeting.detail.processing.accuracyAction")}
            </Button>
          </div>
        }
      >
        <label className="flex flex-col gap-1 text-sm">
          <span className="font-medium">
            {t("meeting.detail.processing.accuracyProvider")}
          </span>
          <select
            className="rounded-md border border-mid-gray/30 bg-background px-2 py-1.5 text-sm"
            value={accuracyProvider}
            onChange={(event) =>
              setAccuracyProvider(event.target.value as Provider)
            }
            disabled={accuracyRunning}
          >
            <option value="local">{t("meeting.setup.providerLocal")}</option>
            <option value="azure">{t("meeting.setup.providerAzure")}</option>
            <option value="custom">{t("meeting.setup.providerCustom")}</option>
          </select>
        </label>
      </Dialog>
    </div>
  );
};

export default MeetingProcessingPanel;
