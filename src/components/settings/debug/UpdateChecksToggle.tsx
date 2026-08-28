import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

interface UpdateChecksToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const UpdateChecksToggle: React.FC<UpdateChecksToggleProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  // Must match `default_update_checks_enabled` in settings/schema.rs. This
  // fallback applies while settings load, and it used to be `true` — so the
  // toggle rendered "on" for a setting the backend defaults to off, and
  // would stay that way if the load never completed.
  const updateChecksEnabled = getSetting("update_checks_enabled") ?? false;

  return (
    <ToggleSwitch
      checked={updateChecksEnabled}
      onChange={(enabled) => updateSetting("update_checks_enabled", enabled)}
      isUpdating={isUpdating("update_checks_enabled")}
      label={t("settings.debug.updateChecks.label")}
      description={t("settings.debug.updateChecks.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    />
  );
};
