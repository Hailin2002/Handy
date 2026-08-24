import React from "react";
import { useTranslation } from "react-i18next";
import { LifeBuoy } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { MeetingStateBadge } from "@/components/shared";
import type { MeetingRecoveryCause } from "@/lib/types/meeting";
import { useMeetingStore } from "@/stores/meetingStore";

export interface MeetingRecoveryBannerProps {
  /** Opens the recovered meeting after the user chose to process it. */
  onOpenMeeting?: (meetingId: string) => void;
}

const CAUSE_KEY: Record<MeetingRecoveryCause, string> = {
  crash: "meeting.recovery.causeCrash",
  sleep_wake: "meeting.recovery.causeSleepWake",
  unknown: "meeting.recovery.causeUnknown",
};

/**
 * Crash/interruption recovery banner (plan "Session-Zustände" and
 * `docs/meeting-mode-architecture/startup-recovery-dataflow.md` §3 S7 / §5).
 *
 * The plan requires that a session never stays stuck in `recording` and that
 * open files are marked recoverable, finalised and then **offered for
 * processing** — an offer the user can actually see. Rendering it as a
 * dismissible-by-choice banner rather than a silent background repair is the
 * point: recovery is a visible flow with three named outcomes (U2 recover,
 * U3 decide later, U4 delete), and "decide later" persists so the offer
 * returns on the next launch instead of quietly disappearing.
 *
 * The list itself is an idempotent pull (`meeting_list_recoveries`), so a
 * duplicated or lost `meeting://recovery-report` event can neither
 * double-render nor lose the offer.
 */
export const MeetingRecoveryBanner: React.FC<MeetingRecoveryBannerProps> = ({
  onOpenMeeting,
}) => {
  const { t } = useTranslation();
  const recoveries = useMeetingStore((state) => state.recoveries);
  const busy = useMeetingStore((state) => state.busy);
  const resolveRecovery = useMeetingStore((state) => state.resolveRecovery);

  if (recoveries.length === 0) return null;

  return (
    <section
      className="flex w-full flex-col gap-3"
      aria-label={t("meeting.recovery.heading")}
    >
      {recoveries.map((offer) => (
        <div
          key={offer.meeting_id}
          data-recovery-offer={offer.meeting_id}
          className="flex flex-col gap-2 rounded-lg border border-warning/30 bg-warning/10 p-4"
        >
          <div className="flex flex-wrap items-center gap-2">
            <LifeBuoy className="h-4 w-4 text-warning" aria-hidden="true" />
            <h3 className="text-sm font-semibold">
              {t("meeting.recovery.title")}
            </h3>
            <MeetingStateBadge state="interrupted" />
          </div>

          <p className="text-sm text-text/80">
            {t("meeting.recovery.body", { title: offer.title })}
          </p>
          <p className="text-xs text-text/60">{t(CAUSE_KEY[offer.cause])}</p>
          <p className="text-xs text-text/60">
            {t("meeting.recovery.counts", {
              recovered: offer.recovered_chunks,
              damaged: offer.damaged_chunks,
            })}
          </p>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              disabled={busy}
              onClick={() => {
                void resolveRecovery(offer.meeting_id, "recover").then(() =>
                  onOpenMeeting?.(offer.meeting_id),
                );
              }}
            >
              {t("meeting.actions.recover")}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onClick={() => void resolveRecovery(offer.meeting_id, "later")}
            >
              {t("meeting.actions.decideLater")}
            </Button>
            {/* Destructive path stays reachable without hover (§6.14) and
                routes into the staged delete flow on the backend side. */}
            <Button
              variant="danger-ghost"
              size="sm"
              disabled={busy}
              onClick={() => void resolveRecovery(offer.meeting_id, "delete")}
            >
              {t("meeting.actions.deleteRecording")}
            </Button>
          </div>
        </div>
      ))}
    </section>
  );
};

export default MeetingRecoveryBanner;
