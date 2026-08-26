import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type ChatPreviewDto,
  type ZipIngestResult,
  type ZipPreviewDto,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";

export interface WhatsAppImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a successful import so parents can refresh lists. */
  onImported: () => void;
}

type Step = "input" | "preview";

/**
 * WhatsApp import wizard, compact v1 (plan §6.19): paste or pick a chat
 * export, inspect the parsed preview (participants, counts, media, date
 * ambiguity) and confirm. Ambiguous day/month order is never guessed —
 * the user must choose explicitly before import unlocks.
 */
export const WhatsAppImportDialog: React.FC<WhatsAppImportDialogProps> = ({
  open,
  onOpenChange,
  onImported,
}) => {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("input");
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [dateOrder, setDateOrder] = useState<"" | "dayFirst" | "monthFirst">(
    "",
  );
  const [preview, setPreview] = useState<ChatPreviewDto | null>(null);
  const [busy, setBusy] = useState(false);
  /** Chosen `.zip` path, or `null` while importing pasted text. */
  const [archivePath, setArchivePath] = useState<string | null>(null);
  const [archive, setArchive] = useState<ZipPreviewDto | null>(null);

  const reset = () => {
    setStep("input");
    setTitle("");
    setContent("");
    setDateOrder("");
    setPreview(null);
    setArchivePath(null);
    setArchive(null);
  };

  /**
   * Step 1 for archives: pick the file. Nothing is read, extracted or sent
   * anywhere yet — the plan is explicit that step 1 involves no transfer.
   */
  const pickArchive = async () => {
    if (busy) return;
    const chosen = await openFileDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "WhatsApp export", extensions: ["zip"] }],
    });
    if (typeof chosen !== "string") return;
    setArchivePath(chosen);
    if (!title.trim()) {
      const base = chosen.split(/[\\/]/).pop() ?? "";
      setTitle(base.replace(/\.zip$/i, ""));
    }
  };

  const runPreview = async () => {
    if (busy) return;
    if (!archivePath && !content.trim()) return;
    setBusy(true);
    try {
      const result = archivePath
        ? await commands.whatsappPreviewZip(archivePath, dateOrder || null)
        : await commands.whatsappPreviewText(content, dateOrder || null);
      if (result.status === "ok" && result.data) {
        if (archivePath) {
          const zip = result.data as ZipPreviewDto;
          setArchive(zip);
          setPreview(zip.chat);
        } else {
          setArchive(null);
          setPreview(result.data as ChatPreviewDto);
        }
        setStep("preview");
      } else {
        toast.error(t("whatsapp.previewFailed"), {
          description: result.status === "error" ? result.error : undefined,
        });
      }
    } catch (error) {
      console.error("Preview failed:", error);
      toast.error(t("whatsapp.previewFailed"));
    } finally {
      setBusy(false);
    }
  };

  const runImport = async () => {
    // Synchronous re-entry guard against same-frame double-clicks.
    if (busy) return;
    setBusy(true);
    try {
      const resolvedTitle = title.trim() || t("whatsapp.defaultTitle");
      const result = archivePath
        ? await commands.whatsappImportZip(
            archivePath,
            resolvedTitle,
            dateOrder || null,
          )
        : await commands.whatsappImportText(
            resolvedTitle,
            content,
            dateOrder || null,
          );
      if (result.status === "ok") {
        if (!result.data.created) {
          toast.info(t("whatsapp.alreadyImported"));
        } else {
          toast.success(
            t("whatsapp.imported", { count: result.data.item_count }),
          );
        }
        // Only the archive path reports media; the pasted-text path has none.
        const zipResult = archivePath ? (result.data as ZipIngestResult) : null;
        if (zipResult && zipResult.media_count > 0) {
          toast.info(
            t("whatsapp.mediaImported", { count: zipResult.media_count }),
          );
        }
        // Never silent: an entry that could not be stored is named, not
        // quietly dropped (plan §6.19).
        if (zipResult && zipResult.skipped.length > 0) {
          toast.warning(
            t("whatsapp.mediaSkipped", { count: zipResult.skipped.length }),
            { description: zipResult.skipped.slice(0, 3).join("\n") },
          );
        }
        onImported();
        reset();
        onOpenChange(false);
      } else {
        toast.error(t("whatsapp.importFailed"), { description: result.error });
      }
    } catch (error) {
      console.error("Import failed:", error);
      toast.error(t("whatsapp.importFailed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      title={t("whatsapp.title")}
      closeLabel={t("common.close")}
      onOpenChange={(next) => {
        if (!busy) {
          if (!next) reset();
          onOpenChange(next);
        }
      }}
      footer={
        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => (step === "preview" ? setStep("input") : reset())}
            disabled={busy}
          >
            {t("common.back")}
          </Button>
          {step === "input" ? (
            <Button
              onClick={runPreview}
              disabled={busy || (!archivePath && !content.trim())}
            >
              {t("whatsapp.preview")}
            </Button>
          ) : (
            <Button
              onClick={runImport}
              disabled={busy || (preview?.date_order_ambiguous ?? false)}
            >
              {t("whatsapp.import")}
            </Button>
          )}
        </div>
      }
    >
      {step === "input" && (
        <div className="flex flex-col gap-3">
          <Input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder={t("whatsapp.titlePlaceholder")}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={pickArchive}
              disabled={busy}
            >
              {t("whatsapp.chooseArchive")}
            </Button>
            <span className="text-xs text-text/60">
              {archivePath
                ? t("whatsapp.archiveChosen", {
                    name: archivePath.split(/[\\/]/).pop() ?? archivePath,
                  })
                : t("whatsapp.archiveHint")}
            </span>
            {archivePath && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setArchivePath(null)}
                disabled={busy}
              >
                {t("whatsapp.clearArchive")}
              </Button>
            )}
          </div>
          <textarea
            className="h-48 w-full rounded-md border border-mid-gray/30 bg-background p-2 font-mono text-xs disabled:opacity-50"
            value={content}
            onChange={(event) => setContent(event.target.value)}
            placeholder={t("whatsapp.contentPlaceholder")}
            disabled={Boolean(archivePath)}
          />
          <label className="flex flex-col gap-1 text-sm">
            <span>{t("whatsapp.dateOrderLabel")}</span>
            <select
              className="rounded-md border border-mid-gray/30 bg-background px-2 py-1.5 text-sm"
              value={dateOrder}
              onChange={(event) =>
                setDateOrder(
                  event.target.value as "" | "dayFirst" | "monthFirst",
                )
              }
            >
              <option value="">{t("whatsapp.dateOrderAuto")}</option>
              <option value="dayFirst">
                {t("whatsapp.dateOrderDayFirst")}
              </option>
              <option value="monthFirst">
                {t("whatsapp.dateOrderMonthFirst")}
              </option>
            </select>
          </label>
        </div>
      )}

      {step === "preview" && preview && (
        <div className="flex flex-col gap-2 text-sm">
          {archive && (
            <p className="text-text/60">
              {t("whatsapp.previewArchive", {
                chat: archive.chat_file,
                count: archive.media_files,
              })}
            </p>
          )}
          <p>
            {t("whatsapp.previewMessages", { count: preview.message_count })} ·{" "}
            {t("whatsapp.previewParticipants", {
              count: preview.participants.length,
            })}
          </p>
          <p className="text-text/60">
            {preview.participants.join(", ") || "—"}
          </p>
          <p className="text-xs text-text/60">
            {t("whatsapp.previewMedia", { count: preview.media_count })}
          </p>
          {preview.date_order_ambiguous && (
            <div className="rounded-md border border-warning/40 bg-warning/10 p-2 text-xs">
              {t("whatsapp.ambiguousDates")}
              <div className="mt-2 flex flex-col gap-1">
                <label className="flex items-center gap-2">
                  <input
                    type="radio"
                    name="dateOrder"
                    checked={dateOrder === "dayFirst"}
                    onChange={() => setDateOrder("dayFirst")}
                  />
                  {t("whatsapp.dateOrderDayFirst")}
                </label>
                <label className="flex items-center gap-2">
                  <input
                    type="radio"
                    name="dateOrder"
                    checked={dateOrder === "monthFirst"}
                    onChange={() => setDateOrder("monthFirst")}
                  />
                  {t("whatsapp.dateOrderMonthFirst")}
                </label>
              </div>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
};

export default WhatsAppImportDialog;
