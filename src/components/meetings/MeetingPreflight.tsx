import React from "react";
import { useTranslation } from "react-i18next";
import {
  CircleAlert,
  CircleCheck,
  LoaderCircle,
  TriangleAlert,
} from "lucide-react";
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import type {
  MeetingPreflightCheck,
  MeetingPreflightReport,
  MeetingPreflightStatus as MeetingPreflightCheckStatus,
} from "@/lib/types/meeting";
import type { MeetingPreflightStatus } from "@/stores/meetingStore";

export interface MeetingPreflightProps {
  report: MeetingPreflightReport | null;
  status: MeetingPreflightStatus;
  /** Re-runs the checks (plan §6.7: repairs happen without losing the dialog). */
  onRecheck: () => void;
  /** Opens the settings for the check that failed. */
  onOpenSettings: () => void;
  /** Explicit mic-only fallback offered when loopback is unavailable (E5). */
  onRecordMicOnly?: () => void;
  microphoneOnly?: boolean;
}

/**
 * Start validation rows of the compact start dialog (plan §6.7 and the
 * "Startvalidierung" block).
 *
 * The plan is explicit that the dialog shows only understandable status lines
 * while the backend validates devices, free disk space, provider/model,
 * language, retention and permissions in full. Each row therefore renders a
 * plain label, the current value and a status word — never a technical code —
 * and a failing row offers the concrete repair action instead of a dead end
 * (§6.2 rule 8).
 *
 * Colour is never the only carrier (§6.8): each status has an icon *and* a
 * translated status word, and each row exposes its state to assistive tech
 * through the visible text.
 */
const STATUS_ICON: Record<MeetingPreflightCheckStatus, React.ReactNode> = {
  ok: <CircleCheck className="h-4 w-4 text-success" aria-hidden="true" />,
  warning: (
    <TriangleAlert className="h-4 w-4 text-warning" aria-hidden="true" />
  ),
  error: <CircleAlert className="h-4 w-4 text-error" aria-hidden="true" />,
};

const STATUS_TEXT: Record<MeetingPreflightCheckStatus, string> = {
  ok: "meeting.preflight.status.ok",
  warning: "meeting.preflight.status.warning",
  error: "meeting.preflight.status.error",
};

const ROW_LABEL: Record<MeetingPreflightCheck["id"], string> = {
  microphone: "meeting.preflight.row.microphone",
  systemAudio: "meeting.preflight.row.systemAudio",
  transcription: "meeting.preflight.row.transcription",
  language: "meeting.preflight.row.language",
  disk: "meeting.preflight.row.disk",
  permission: "meeting.preflight.row.permission",
};

export const MeetingPreflight: React.FC<MeetingPreflightProps> = ({
  report,
  status,
  onRecheck,
  onOpenSettings,
  onRecordMicOnly,
  microphoneOnly = false,
}) => {
  const { t } = useTranslation();

  if (status === "unavailable") {
    return (
      <div className="flex flex-col gap-2">
        <Alert variant="error">{t("meeting.preflight.unavailable")}</Alert>
        <div>
          <Button variant="secondary" size="sm" onClick={onRecheck}>
            {t("meeting.actions.tryAgain")}
          </Button>
        </div>
      </div>
    );
  }

  if (status === "running" || (status === "idle" && !report)) {
    return (
      <p
        className="flex items-center gap-2 text-sm text-text/60"
        // Status changes reach screen readers through a restrained live
        // region rather than a focus move (plan §6.14).
        role="status"
      >
        <LoaderCircle
          className="h-4 w-4 animate-spin motion-reduce:animate-none"
          aria-hidden="true"
        />
        {t("meeting.preflight.pending")}
      </p>
    );
  }

  if (!report) return null;

  return (
    <section
      className="flex flex-col gap-2"
      aria-label={t("meeting.preflight.title")}
    >
      <p className="text-xs text-text/60">
        {t("meeting.preflight.description")}
      </p>

      <ul className="flex flex-col divide-y divide-mid-gray/15 rounded-lg border border-mid-gray/20">
        {report.checks.map((check) => (
          <li
            key={check.id}
            className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-sm"
            data-preflight-row={check.id}
            data-preflight-status={check.status}
          >
            <span className="min-w-28 font-medium">
              {t(ROW_LABEL[check.id])}
            </span>
            <span className="flex-1 truncate text-text/70">
              {check.value ?? ""}
            </span>
            <span className="flex items-center gap-1.5 whitespace-nowrap">
              {STATUS_ICON[check.status]}
              {t(STATUS_TEXT[check.status])}
            </span>
          </li>
        ))}
      </ul>

      {!report.can_start && (
        <Alert variant="warning">{t("meeting.preflight.blocked")}</Alert>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="secondary" size="sm" onClick={onRecheck}>
          {t("meeting.preflight.recheck")}
        </Button>
        <Button variant="ghost" size="sm" onClick={onOpenSettings}>
          {t("meeting.actions.openSettings")}
        </Button>
        {/* Plan "Startvalidierung": the user may deliberately fall back to a
            microphone-only recording when loopback cannot be opened. */}
        {report.microphone_only_available &&
          !microphoneOnly &&
          onRecordMicOnly && (
            <Button variant="secondary" size="sm" onClick={onRecordMicOnly}>
              {t("meeting.actions.recordMicOnly")}
            </Button>
          )}
      </div>
    </section>
  );
};

export default MeetingPreflight;
