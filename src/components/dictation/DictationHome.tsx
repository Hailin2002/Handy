import React from "react";
import { useTranslation } from "react-i18next";
import { MicOff, Square } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { getLanguageLabel } from "@/lib/constants/languages";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { Button } from "@/components/ui/Button";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ErrorState } from "@/components/shared/ErrorState";
import { useOsType } from "@/hooks/useOsType";
import {
  useDictationStatus,
  type DictationErrorKind,
  type DictationPhase,
} from "./useDictationStatus";
import { useDictationMicTest } from "./useDictationMicTest";

/**
 * DictationHome — the "Diktieren" start page (plan §6.4).
 *
 * Shows only the current dictation state and the most important settings:
 * status, the current shortcut as a clickable keycap, mode, speech-to-text
 * target, language/model, a safe in-app test button and a link to the
 * dictation settings.
 *
 * Deliberately NOT on this page: any meeting affordance. Dictation and
 * meetings are two products in one frame (plan §6.2 rule 1), so a user can
 * never mistake them for two modes of the same record button (plan §6.4).
 *
 * SAFETY (gate G2): the test button lives in `useDictationMicTest`, which
 * captures audio inside this webview only. This file contains no clipboard
 * access and invokes no output/paste command — the test result can only ever
 * appear in the in-app preview below.
 */

interface DictationHomeProps {
  /**
   * Opens the dictation settings (Einstellungen → Diktieren). Optional so the
   * component stays renderable standalone (e.g. in render tests).
   */
  onOpenSettings?: () => void;
}

const phaseDotClass: Record<DictationPhase, string> = {
  ready: "bg-mid-gray/60",
  recording: "bg-red-500 animate-pulse",
  transcribing: "bg-blue-400",
  error: "bg-red-500",
};

/** Formats elapsed milliseconds as a short m:ss clock. */
const formatClock = (totalMs: number): string => {
  const total = Math.max(0, Math.floor(totalMs / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
};

export const DictationHome: React.FC<DictationHomeProps> = ({
  onOpenSettings,
}) => {
  const { t } = useTranslation();
  const osType = useOsType();
  const { settings } = useSettings();
  const { currentModel, models } = useModelStore();
  const { phase, error } = useDictationStatus();
  const micTest = useDictationMicTest();

  const bindings = settings?.bindings ?? {};
  const transcribeBinding = bindings.transcribe;
  const shortcutText = transcribeBinding
    ? formatKeyCombination(transcribeBinding.current_binding, osType)
    : null;

  const pushToTalk = settings?.push_to_talk ?? false;
  const provider = settings?.speech_input_provider ?? "local";

  const selectedLanguage =
    provider === "azure"
      ? (settings?.azure_speech_language ?? settings?.selected_language ?? "")
      : (settings?.selected_language ?? "auto");
  const languageLabel =
    selectedLanguage === "auto"
      ? t("settings.general.language.auto")
      : (getLanguageLabel(selectedLanguage) ?? selectedLanguage);

  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);
  const modelName =
    provider === "local" && currentModelInfo
      ? getTranslatedModelName(currentModelInfo, t)
      : null;

  const capturing = micTest.status === "capturing";

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <header className="text-center space-y-1">
        <h1 className="text-lg font-semibold">{t("dictation.title")}</h1>
        <p className="text-sm text-mid-gray">{t("dictation.subtitle")}</p>
      </header>

      {/* Status — honest, backend-driven states (plan §6.2 rule 5). */}
      <SettingsGroup title={t("dictation.status.title")}>
        <div className="px-4 p-3">
          <div
            role="status"
            aria-live="polite"
            className="flex items-center gap-2"
          >
            <span
              aria-hidden="true"
              className={`inline-block w-2.5 h-2.5 rounded-full ${phaseDotClass[phase]}`}
            />
            <span className="text-sm font-medium">
              {t(`dictation.status.${phase}`)}
            </span>
          </div>
          {phase === "error" && error !== null && (
            <div className="mt-2 text-sm text-mid-gray">
              <p>{errorDescription(t, error.kind, error.detail)}</p>
              {onOpenSettings && (
                <Button
                  variant="secondary"
                  size="sm"
                  className="mt-2"
                  onClick={onOpenSettings}
                >
                  {t("dictation.errorOpenSettings")}
                </Button>
              )}
            </div>
          )}
        </div>
      </SettingsGroup>

      {/* Current configuration — display-only; changes happen in settings. */}
      <SettingsGroup title={t("dictation.config.title")}>
        <SettingContainer
          title={t("settings.general.shortcut.bindings.transcribe.name")}
          description={t("dictation.shortcut.description")}
          descriptionMode="tooltip"
          grouped
        >
          {shortcutText && onOpenSettings ? (
            <button
              type="button"
              onClick={onOpenSettings}
              aria-label={t("dictation.shortcut.openAria", {
                shortcut: shortcutText,
              })}
              className="px-3 py-2 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 rounded-md hover:bg-logo-primary/10 hover:border-logo-primary cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:ring-offset-2 focus-visible:ring-offset-background"
            >
              {shortcutText}
            </button>
          ) : (
            <span className="text-sm font-semibold px-3 py-2 bg-mid-gray/10 border border-mid-gray/80 rounded-md">
              {shortcutText ?? t("dictation.shortcut.missing")}
            </span>
          )}
        </SettingContainer>

        <SettingContainer
          title={t("dictation.mode.title")}
          description={
            pushToTalk
              ? t("dictation.mode.holdDescription")
              : t("dictation.mode.toggleDescription")
          }
          descriptionMode="tooltip"
          grouped
        >
          <span className="text-sm">
            {pushToTalk ? t("dictation.mode.hold") : t("dictation.mode.toggle")}
          </span>
        </SettingContainer>

        <SettingContainer
          title={t("dictation.target.title")}
          description={t("dictation.target.description")}
          descriptionMode="tooltip"
          grouped
        >
          <span className="text-sm">
            {provider === "local"
              ? t("dictation.target.local")
              : provider === "azure"
                ? t("dictation.target.azure")
                : t("dictation.target.custom")}
          </span>
        </SettingContainer>

        <SettingContainer
          title={t("dictation.language.title")}
          description={t("dictation.language.description")}
          descriptionMode="tooltip"
          grouped
        >
          <span className="text-sm">
            {modelName
              ? t("dictation.language.withModel", {
                  language: languageLabel,
                  model: modelName,
                })
              : languageLabel}
          </span>
        </SettingContainer>
      </SettingsGroup>

      {/* Safe test — result appears ONLY in the in-app preview below. */}
      <SettingsGroup title={t("dictation.test.title")}>
        <SettingContainer
          title={t("dictation.test.button")}
          description={t("dictation.test.privacy")}
          descriptionMode="inline"
          layout="stacked"
          grouped
        >
          <div className="space-y-3 pb-2">
            <div className="flex flex-wrap items-center gap-3">
              {capturing ? (
                <Button variant="danger" onClick={micTest.stop}>
                  <span aria-hidden="true" className="inline-flex me-1.5">
                    <Square size={14} />
                  </span>
                  {t("dictation.test.stop")}
                </Button>
              ) : (
                <Button variant="secondary" onClick={micTest.start}>
                  {t("dictation.test.start")}
                </Button>
              )}
              <span className="text-xs text-mid-gray">
                {capturing
                  ? t("dictation.test.recordingHint", {
                      seconds: micTest.maxSeconds,
                    })
                  : t("dictation.test.idleHint", {
                      seconds: micTest.maxSeconds,
                    })}
              </span>
            </div>

            {capturing && (
              <div className="flex items-center gap-3">
                <div
                  aria-hidden="true"
                  className="h-2 flex-1 max-w-xs bg-mid-gray/20 rounded-full overflow-hidden"
                >
                  <div
                    className="h-full bg-logo-primary transition-[width] duration-100"
                    style={{ width: `${Math.round(micTest.level * 100)}%` }}
                  />
                </div>
                <span className="text-xs tabular-nums text-mid-gray">
                  {formatClock(micTest.elapsedMs)}
                </span>
              </div>
            )}

            {micTest.status === "error" && micTest.errorKind !== null && (
              <ErrorState
                headingLevel={3}
                icon={<MicOff size={28} />}
                title={{
                  key: `dictation.test.error.${micTest.errorKind}.title`,
                }}
                description={{
                  key: `dictation.test.error.${micTest.errorKind}.description`,
                }}
                actions={[
                  {
                    label: { key: "dictation.test.retry" },
                    onClick: () => micTest.start(),
                  },
                ]}
              />
            )}
          </div>
        </SettingContainer>

        {micTest.preview !== null && (
          <div
            role="region"
            aria-label={t("dictation.test.preview.label")}
            className="px-4 p-3 space-y-2"
          >
            <p className="text-sm font-medium">
              {t("dictation.test.preview.label")}
            </p>
            <audio
              controls
              src={micTest.preview.url}
              aria-label={t("dictation.test.preview.playbackAria")}
              className="w-full"
            />
            <p className="text-xs text-mid-gray">
              {t("dictation.test.preview.duration", {
                duration: formatClock(micTest.preview.seconds * 1000),
              })}
            </p>
            <p className="text-xs text-mid-gray">
              {t("dictation.test.privacy")}
            </p>
            <Button variant="danger-ghost" size="sm" onClick={micTest.discard}>
              {t("dictation.test.preview.discard")}
            </Button>
          </div>
        )}
      </SettingsGroup>

      {onOpenSettings && (
        <div className="flex justify-center pb-2">
          <Button variant="ghost" onClick={onOpenSettings}>
            {t("dictation.settingsLink")}
          </Button>
        </div>
      )}
    </div>
  );
};

/** Localized, next-action-oriented copy per backend error kind (§6.2 rule 8). */
const errorDescription = (
  t: ReturnType<typeof useTranslation>["t"],
  kind: DictationErrorKind,
  detail?: string,
): string => {
  switch (kind) {
    case "microphone_permission":
      return t("errors.micPermissionDenied.generic");
    case "no_input_device":
      return t("errors.noInputDevice");
    case "transcription":
      return t("errors.transcriptionFailedTitle");
    default:
      return detail
        ? t("errors.recordingFailed", { error: detail })
        : t("dictation.status.genericError");
  }
};
