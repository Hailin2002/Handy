import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Copy,
  Download,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  commands,
  type ExportFormat,
  type MeetingAudioFileDto,
  type MeetingSession,
  type MeetingSnapshot,
  type MeetingSummaryDto,
  type TranscriptRowDto,
} from "@/bindings";
import { IconButton } from "@/components/ui/IconButton";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { formatDateTime } from "@/utils/dateFormat";
import { formatMeetingDuration } from "@/components/shell/productNav";
import { MeetingAudioPanel } from "./MeetingAudioPanel";
import { MeetingProcessingPanel } from "./MeetingProcessingPanel";
import { MeetingDetailPanel, MeetingDetailTabs } from "./MeetingDetailTabs";
import { useMeetingDetailTab } from "./useMeetingDetailTab";
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
  // Plan §11: deletion is not one button. The user chooses between dropping
  // only the raw recording (transcript and summary survive) and erasing the
  // meeting for good, and the dialog names exactly what each option removes
  // before anything irreversible happens.
  const [deleteScope, setDeleteScope] = useState<"audioOnly" | "everything">(
    "everything",
  );
  const [deleting, setDeleting] = useState(false);
  const lastSessionId = useRef(session.id);

  // Phase 7 detail content.
  const [summary, setSummary] = useState<MeetingSummaryDto | null>(null);
  const [transcript, setTranscript] = useState<TranscriptRowDto[]>([]);
  const [resummarizing, setResummarizing] = useState(false);
  const [transcriptQuery, setTranscriptQuery] = useState("");
  const [audioFiles, setAudioFiles] = useState<MeetingAudioFileDto[]>([]);
  const [snapshot, setSnapshot] = useState<MeetingSnapshot | null>(null);
  // Timeline position a transcript row asked the Audio tab to play from.
  const [seekToMs, setSeekToMs] = useState<number | null>(null);
  const [tab, setTab] = useMeetingDetailTab(session.id);

  const visibleTranscript = transcriptQuery.trim()
    ? transcript.filter((row) =>
        row.text.toLowerCase().includes(transcriptQuery.trim().toLowerCase()),
      )
    : transcript;

  const loadContent = useCallback(async () => {
    try {
      const [summaryResult, transcriptResult, audioResult, snapshotResult] =
        await Promise.all([
          commands.getMeetingSummary(session.id),
          commands.meetingTranscript(session.id),
          commands.meetingAudioFiles(session.id),
          commands.getMeetingSnapshot(session.id),
        ]);
      if (summaryResult.status === "ok" && summaryResult.data) {
        setSummary(summaryResult.data);
      }
      if (transcriptResult.status === "ok" && transcriptResult.data) {
        setTranscript(transcriptResult.data);
      }
      if (audioResult.status === "ok") {
        setAudioFiles(audioResult.data);
      }
      if (snapshotResult.status === "ok") {
        setSnapshot(snapshotResult.data);
      }
    } catch (error) {
      console.error("Failed to load meeting content:", error);
    }
  }, [session.id]);

  useEffect(() => {
    void loadContent();
  }, [loadContent]);

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
      const result =
        deleteScope === "audioOnly"
          ? await commands.deleteMeetingData(session.id, "audioOnly")
          : await commands.meetingDeleteSession(session.id);
      if (result.status === "ok") {
        if (deleteScope === "audioOnly") {
          // The meeting itself is still there, so the view stays open and
          // only reports what actually happened.
          toast.success(t("meetings.deleteAudioDone"));
        } else {
          onDeleted(session.id);
        }
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

  const handleResummarize = async () => {
    if (resummarizing) return;
    setResummarizing(true);
    try {
      const result = await commands.requestMeetingSummary(session.id);
      if (result.status === "ok") {
        toast.success(t("meeting.detail.resummarizeDone"));
        await loadContent();
      } else {
        toast.error(t("meeting.detail.resummarizeFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      console.error("Failed to re-summarize meeting:", error);
      toast.error(t("meeting.detail.resummarizeFailed"));
    } finally {
      setResummarizing(false);
    }
  };

  const copyText = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t(key));
    } catch (error) {
      console.error("Clipboard write failed:", error);
      toast.error(t("meetings.copyFailed"));
    }
  };

  /** Plan §6.13: a transcript row jumps to its position in the recording. */
  const jumpToAudio = (startMs: number) => {
    setSeekToMs(startMs);
    setTab("audio");
  };

  const handleExport = async (format: ExportFormat) => {
    try {
      const result = await commands.exportMeeting(session.id, format);
      if (result.status !== "ok" || !result.data) {
        toast.error(t("meetings.exportError"));
        return;
      }
      // Browser-style download works inside the Tauri webview for blobs.
      const blob = new Blob([result.data.content], { type: result.data.mime });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = result.data.filename;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      console.error("Export failed:", error);
      toast.error(t("meetings.exportError"));
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

        <MeetingDetailTabs
          active={tab}
          onChange={setTab}
          disabled={{
            audio:
              audioFiles.length === 0
                ? t("meeting.detail.audio.none")
                : undefined,
          }}
        />

        <MeetingDetailPanel tab="overview" active={tab}>
          {/* Phase 7 — summary, copy/export/re-summarize. */}
          {summary?.state === "completed" && (
            <div className="rounded-lg border border-mid-gray/20 p-3">
              <div className="flex items-center justify-between gap-2">
                <h3 className="text-sm font-medium">
                  {t("meeting.detail.summaryHeading")}
                </h3>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void copyText(summary.summary_markdown, "meetings.copied")
                  }
                >
                  <Copy className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
                  {t("meetings.copySummary")}
                </Button>
              </div>
              <p className="mt-2 whitespace-pre-wrap text-sm text-text/80">
                {summary.summary_markdown}
              </p>
              {summary.decisions.length > 0 && (
                <p className="mt-2 text-xs text-text/60">
                  {t("meeting.detail.decisions")}:{" "}
                  {summary.decisions.join("; ")}
                </p>
              )}
              {summary.action_items.length > 0 && (
                <p className="mt-1 text-xs text-text/60">
                  {t("meeting.detail.actionItems")}:{" "}
                  {summary.action_items.join("; ")}
                </p>
              )}
              {summary.open_questions.length > 0 && (
                <p className="mt-1 text-xs text-text/60">
                  {t("meeting.detail.openQuestions")}:{" "}
                  {summary.open_questions.join("; ")}
                </p>
              )}
            </div>
          )}

          {summary?.state === "failed" && (
            <div className="flex items-center justify-between gap-2 rounded-lg border border-warning/30 bg-warning/10 p-3">
              <span className="text-sm">
                {t("meeting.detail.summaryFailed")}
              </span>
              <Button
                size="sm"
                onClick={handleResummarize}
                disabled={resummarizing}
              >
                <RefreshCw className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
                {t("meeting.detail.resummarize")}
              </Button>
            </div>
          )}

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
        </MeetingDetailPanel>

        <MeetingDetailPanel tab="transcript" active={tab}>
          {transcript.length === 0 && (
            <p className="text-sm text-text/60">
              {t("meeting.detail.transcriptEmpty")}
            </p>
          )}
          {transcript.length > 0 && (
            <div className="rounded-lg border border-mid-gray/20 p-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-medium">
                  {t("meeting.detail.transcriptHeading")}
                </h3>
                <div className="flex items-center gap-1">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      void copyText(
                        transcript.map((row) => `${row.text}`).join("\n"),
                        "meetings.copied",
                      )
                    }
                  >
                    <Copy className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
                    {t("meetings.copyTranscript")}
                  </Button>
                  {(["markdown", "txt", "json"] as ExportFormat[]).map(
                    (format) => (
                      <Button
                        key={format}
                        size="sm"
                        variant="ghost"
                        onClick={() => void handleExport(format)}
                      >
                        <Download
                          className="mr-1 h-3.5 w-3.5"
                          aria-hidden="true"
                        />
                        {format.toUpperCase()}
                      </Button>
                    ),
                  )}
                </div>
              </div>
              <ul className="mt-2 max-h-64 space-y-1 overflow-y-auto text-sm text-text/80">
                {visibleTranscript.map((row, index) => (
                  <li key={`${row.start_ms}-${index}`} className="flex gap-2">
                    {/* Timestamp doubles as the jump target, so the position is
                      both readable and actionable without a hover-only
                      control (§6.14). Disabled while no audio remains rather
                      than hidden, so the row layout never shifts. */}
                    <button
                      type="button"
                      onClick={() => jumpToAudio(row.start_ms)}
                      disabled={audioFiles.length === 0}
                      title={
                        audioFiles.length === 0
                          ? t("meeting.detail.audio.none")
                          : t("meeting.detail.jumpToAudio")
                      }
                      aria-label={t("meeting.detail.jumpToAudioAt", {
                        time: formatMeetingDuration(
                          Math.floor(row.start_ms / 1000),
                        ),
                      })}
                      className="shrink-0 tabular-nums text-xs text-logo-primary hover:underline disabled:cursor-not-allowed disabled:text-text/40 disabled:no-underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
                    >
                      {formatMeetingDuration(Math.floor(row.start_ms / 1000))}
                    </button>
                    <span>
                      <span className="mr-1.5 rounded bg-mid-gray/10 px-1 py-0.5 text-[10px] font-medium uppercase text-text/45">
                        {row.source === "system"
                          ? t("meeting.detail.transcriptSourceSystem")
                          : t("meeting.detail.transcriptSourceMicrophone")}
                      </span>
                      {row.text}
                    </span>
                  </li>
                ))}
              </ul>
              <Input
                value={transcriptQuery}
                onChange={(event) => setTranscriptQuery(event.target.value)}
                placeholder={t("meeting.detail.searchPlaceholder")}
                aria-label={t("meeting.detail.searchLabel")}
                className="mt-2"
              />
              {transcriptQuery.trim() && (
                <p className="mt-1 text-xs text-text/50">
                  {t("meeting.detail.searchMatches", {
                    count: visibleTranscript.length,
                  })}
                </p>
              )}
            </div>
          )}
        </MeetingDetailPanel>

        <MeetingDetailPanel tab="audio" active={tab}>
          <MeetingAudioPanel files={audioFiles} seekToMs={seekToMs} />
        </MeetingDetailPanel>

        <MeetingDetailPanel tab="processing" active={tab}>
          <MeetingProcessingPanel
            session={session}
            snapshot={snapshot}
            onChanged={() => void loadContent()}
          />
        </MeetingDetailPanel>
      </div>

      <Dialog
        open={confirmingDelete}
        title={t("meetings.deleteConfirmTitle")}
        description={
          deleteScope === "audioOnly"
            ? t("meetings.deleteScope.audioOnly.warning")
            : t("meetings.deleteScope.everything.warning")
        }
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
            <Button
              variant="danger"
              size="sm"
              onClick={handleDelete}
              disabled={deleting}
            >
              <Trash2 className="mr-1 h-4 w-4" aria-hidden="true" />
              {t("common.delete")}
            </Button>
          </div>
        }
      >
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium">
            {t("meetings.deleteScope.legend")}
          </legend>
          {(["audioOnly", "everything"] as const).map((scope) => (
            <label key={scope} className="flex items-start gap-2 text-sm">
              <input
                type="radio"
                name="meeting-delete-scope"
                className="mt-1"
                value={scope}
                checked={deleteScope === scope}
                disabled={deleting}
                onChange={() => setDeleteScope(scope)}
              />
              <span className="flex flex-col">
                <span className="font-medium">
                  {t(`meetings.deleteScope.${scope}.title`)}
                </span>
                <span className="text-xs text-text/60">
                  {t(`meetings.deleteScope.${scope}.description`)}
                </span>
              </span>
            </label>
          ))}
        </fieldset>
      </Dialog>
    </section>
  );
};

export default MeetingDetail;
