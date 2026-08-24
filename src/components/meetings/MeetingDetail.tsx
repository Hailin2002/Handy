import React, { useEffect, useRef, useState } from "react";
import { ArrowLeft, Play, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type MeetingSession } from "@/bindings";
import { IconButton } from "@/components/ui/IconButton";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { formatDateTime } from "@/utils/dateFormat";
import { meetingStateLabelKey } from "./meetingStates";

export interface MeetingDetailProps {
  session: MeetingSession;
  /** Navigate back to the meetings library. */
  onBack: () => void;
  /** Called after a successful title save with the updated session. */
  onSaved: (session: MeetingSession) => void;
  /** Called after a successful delete; the detail view closes. */
  onDeleted: (id: string) => void;
  /**
   * Opens the compact start dialog for a draft session (plan §6.7). Omitted
   * where the detail view is rendered outside the meetings router.
   */
  onStartMeeting?: (session: MeetingSession) => void;
}

/**
 * Meeting detail view (plan §6.13, wave-1 subset): editable title, honest
 * status/date metadata and the destructive delete behind an explicit,
 * non-hover confirmation dialog that names what is removed.
 */
export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  session,
  onBack,
  onSaved,
  onDeleted,
  onStartMeeting,
}) => {
  const { t, i18n } = useTranslation();
  const [titleDraft, setTitleDraft] = useState(session.title);
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const lastSessionId = useRef(session.id);

  // Re-seed the editor when a different meeting is opened; keep in-place
  // drafts when only the same session's data refreshes after a save.
  useEffect(() => {
    if (lastSessionId.current !== session.id) {
      lastSessionId.current = session.id;
      setTitleDraft(session.title);
    }
  }, [session.id, session.title]);

  const dirty = titleDraft !== session.title;

  const handleSave = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    try {
      const result = await commands.meetingRenameSession(
        session.id,
        titleDraft,
      );
      if (result.status === "ok") {
        onSaved(result.data);
      } else {
        toast.error(t("meetings.saveError"), { description: result.error });
      }
    } catch (error) {
      console.error("Failed to save meeting title:", error);
      toast.error(t("meetings.saveError"));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (deleting) return;
    setDeleting(true);
    try {
      const result = await commands.meetingDeleteSession(session.id);
      if (result.status === "ok") {
        onDeleted(session.id);
      } else {
        toast.error(t("meetings.deleteError"), { description: result.error });
      }
    } catch (error) {
      console.error("Failed to delete meeting:", error);
      toast.error(t("meetings.deleteError"));
    } finally {
      setDeleting(false);
      setConfirmingDelete(false);
    }
  };

  return (
    <section className="w-full" aria-label={t("nav.meetings")}>
      <div className="flex items-center gap-2">
        <IconButton onClick={onBack} aria-label={t("meetings.backToList")}>
          <ArrowLeft className="h-4 w-4" />
        </IconButton>
        <Button variant="ghost" size="sm" onClick={onBack}>
          {t("meetings.backToList")}
        </Button>
      </div>

      <div className="mt-4 flex w-full max-w-xl flex-col gap-3">
        <label className="flex flex-col gap-1 text-sm">
          <span className="font-medium">{t("meetings.titleInputLabel")}</span>
          <Input
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            aria-label={t("meetings.titleInputLabel")}
          />
        </label>

        {/* A draft has exactly one obvious next step (§6.2 rule 2): start it.
            Brand colour, never red — red is reserved for a running recording
            (§6.8). */}
        {session.state === "draft" && onStartMeeting && (
          <div>
            <Button onClick={() => onStartMeeting(session)}>
              <Play className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("meeting.actions.startMeeting")}
            </Button>
          </div>
        )}

        <div className="flex items-center gap-2">
          <Button size="sm" onClick={handleSave} disabled={!dirty || saving}>
            {t("meetings.saveTitle")}
          </Button>
          <Button
            size="sm"
            variant="danger-ghost"
            onClick={() => setConfirmingDelete(true)}
          >
            <Trash2 className="mr-1 h-4 w-4" aria-hidden="true" />
            {t("common.delete")}
          </Button>
        </div>

        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm text-text/80">
          <dt className="text-text/50">{t("meetings.statusLabel")}</dt>
          <dd>{t(meetingStateLabelKey(session.state))}</dd>
          <dt className="text-text/50">{t("meetings.createdLabel")}</dt>
          <dd>
            {formatDateTime(
              String(Math.floor(session.created_at / 1000)),
              i18n.language,
            )}
          </dd>
        </dl>
      </div>

      <Dialog
        open={confirmingDelete}
        title={t("meetings.deleteConfirmTitle")}
        description={t("meetings.deleteConfirmBody")}
        closeLabel={t("common.close")}
        onOpenChange={(open) => !deleting && setConfirmingDelete(open)}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setConfirmingDelete(false)}
              disabled={deleting}
            >
              {t("common.close")}
            </Button>
            <Button variant="danger" size="sm" onClick={handleDelete} disabled={deleting}>
              <Trash2 className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("common.delete")}
            </Button>
          </div>
        }
      >
        {/* Dialog body intentionally minimal; the description carries the warning. */}
        <span />
      </Dialog>
    </section>
  );
};

export default MeetingDetail;
