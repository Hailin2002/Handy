import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { Slider } from "../../ui/Slider";
import { useSettings } from "../../../hooks/useSettings";
import type { RecordingAudioMode } from "@/bindings";

interface RecordingAudioSettingsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/** Presentation order of the three-way choice: least to most intrusive. */
const MODES: readonly RecordingAudioMode[] = ["off", "duck", "mute"];

/** Mirrors `AppSettings::duck_volume_percent`'s Rust default. */
const DEFAULT_DUCK_VOLUME_PERCENT = 20;

/**
 * What Handy does to system audio playback while it records.
 *
 * `recording_audio_mode` is the single source of truth. The legacy
 * `mute_while_recording` bool still exists in the schema, but the backend
 * keeps it write-through synced with the mode, so this component must never
 * write it — doing so would fight the mirror.
 *
 * The ducking target only makes sense in `duck` mode, so its slider is
 * mounted conditionally rather than disabled: a permanently greyed-out row
 * in the Sound group reads as broken. `duck_fade_ms` is deliberately not
 * exposed; its default is good and a second slider here would bury the
 * one control that matters.
 */
export const RecordingAudioSettings: React.FC<RecordingAudioSettingsProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating, isLoading } = useSettings();

    const mode = getSetting("recording_audio_mode") ?? "off";
    const duckVolumePercent =
      getSetting("duck_volume_percent") ?? DEFAULT_DUCK_VOLUME_PERCENT;

    const options = MODES.map((value) => ({
      value,
      label: t(`settings.general.recordingAudio.modes.${value}`),
    }));

    const handleModeSelect = async (value: string) => {
      await updateSetting("recording_audio_mode", value as RecordingAudioMode);
    };

    const handleVolumeChange = (value: number) => {
      updateSetting("duck_volume_percent", Math.round(value));
    };

    return (
      <>
        <SettingContainer
          title={t("settings.general.recordingAudio.label")}
          description={t("settings.general.recordingAudio.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <Dropdown
            options={options}
            selectedValue={mode}
            onSelect={handleModeSelect}
            disabled={isUpdating("recording_audio_mode") || isLoading}
          />
        </SettingContainer>
        {mode === "duck" && (
          <Slider
            value={duckVolumePercent}
            onChange={handleVolumeChange}
            min={0}
            max={100}
            step={5}
            label={t("settings.general.recordingAudio.volume.label")}
            description={t(
              "settings.general.recordingAudio.volume.description",
            )}
            descriptionMode={descriptionMode}
            grouped={grouped}
            formatValue={(value) =>
              t("settings.general.recordingAudio.volume.unit", {
                percent: Math.round(value),
              })
            }
            disabled={isUpdating("duck_volume_percent")}
          />
        )}
      </>
    );
  });

RecordingAudioSettings.displayName = "RecordingAudioSettings";
