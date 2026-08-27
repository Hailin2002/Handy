import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Quote } from "lucide-react";
import { commands, type Evidence } from "@/bindings";
import { formatMeetingDuration } from "@/components/shell/productNav";

interface TaskEvidenceListProps {
  taskId: string;
  /** Opens the meeting a citation points at, when the parent can navigate. */
  onOpenMeeting?: (meetingId: string, startMs: number | null) => void;
}

/**
 * Citations behind one AI-suggested task (plan §6.17, §10).
 *
 * The plan lets no AI statement stand without a reachable source, so this is
 * not decoration: a suggestion whose citations cannot be shown is a suggestion
 * the user has no basis to accept. A meeting citation carries its timeline
 * position and is clickable; a chat citation shows the quoted message.
 */
export const TaskEvidenceList: React.FC<TaskEvidenceListProps> = ({
  taskId,
  onOpenMeeting,
}) => {
  const { t } = useTranslation();
  const [evidence, setEvidence] = useState<Evidence[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const result = await commands.taskEvidence(taskId);
        if (!cancelled && result.status === "ok") setEvidence(result.data);
      } catch (error) {
        console.error("Failed to load task evidence:", error);
        if (!cancelled) setEvidence([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [taskId]);

  if (evidence === null) {
    return <p className="text-xs text-text/50">{t("common.loading")}</p>;
  }
  if (evidence.length === 0) {
    // Should be unreachable — the store refuses to publish an unevidenced
    // suggestion — but saying so beats rendering an empty box.
    return <p className="text-xs text-text/50">{t("evidence.none")}</p>;
  }

  return (
    <ul className="flex flex-col gap-1" aria-label={t("evidence.label")}>
      {evidence.map((item) => {
        const meetingId = item.meeting_id;
        const startMs = item.segment_start_ms;
        const position =
          startMs !== null && startMs !== undefined
            ? formatMeetingDuration(Math.floor(startMs / 1000))
            : null;
        const body = (
          <>
            <Quote
              className="mt-0.5 h-3 w-3 shrink-0 text-text/40"
              aria-hidden="true"
            />
            <span className="flex flex-col gap-0.5">
              <span className="text-xs text-text/50">
                {meetingId
                  ? t("evidence.fromMeeting", { position: position ?? "" })
                  : t("evidence.fromChat")}
              </span>
              <span className="text-sm text-text/80">{item.quote}</span>
            </span>
          </>
        );

        return (
          <li key={item.id}>
            {meetingId && onOpenMeeting ? (
              <button
                type="button"
                onClick={() => onOpenMeeting(meetingId, startMs ?? null)}
                className="flex w-full items-start gap-2 rounded-md border border-mid-gray/20 p-2 text-left hover:bg-mid-gray/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
              >
                {body}
              </button>
            ) : (
              <div className="flex items-start gap-2 rounded-md border border-mid-gray/20 p-2">
                {body}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
};

export default TaskEvidenceList;
