import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../hooks/useSettings";
import { commands, type ModelUnloadTimeout } from "@/bindings";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";

interface ModelUnloadTimeoutProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const ModelUnloadTimeoutSetting: React.FC<ModelUnloadTimeoutProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting } = useSettings();

  // Built inside the memo so the option arrays are not rebuilt on every render
  // just to be thrown away, and so the memo actually depends on what it reads
  // (`t` for the labels, `debug_mode` for the extra entry) rather than on the
  // whole settings object, which changes identity on any unrelated setting.
  const options = useMemo(() => {
    const base = [
      {
        value: "never" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.never"),
      },
      {
        value: "immediately" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.immediately"),
      },
      {
        value: "min2" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min2"),
      },
      {
        value: "min5" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min5"),
      },
      {
        value: "min10" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min10"),
      },
      {
        value: "min15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min15"),
      },
      {
        value: "hour1" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.hour1"),
      },
    ];

    if (settings?.debug_mode !== true) return base;
    return [
      ...base,
      {
        value: "sec15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.sec15"),
      },
    ];
  }, [settings?.debug_mode, t]);

  const handleChange = async (event: React.ChangeEvent<HTMLSelectElement>) => {
    const newTimeout = event.target.value as ModelUnloadTimeout;

    try {
      await commands.setModelUnloadTimeout(newTimeout);
      updateSetting("model_unload_timeout", newTimeout);
    } catch (error) {
      console.error("Failed to update model unload timeout:", error);
    }
  };

  const currentValue = getSetting("model_unload_timeout") ?? "never";

  return (
    <SettingContainer
      title={t("settings.advanced.modelUnload.title")}
      description={t("settings.advanced.modelUnload.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={currentValue}
        onSelect={(value) =>
          handleChange({
            target: { value },
          } as React.ChangeEvent<HTMLSelectElement>)
        }
        disabled={false}
      />
    </SettingContainer>
  );
};
