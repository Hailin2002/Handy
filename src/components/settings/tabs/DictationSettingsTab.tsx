import React from "react";
import { useSettings } from "../../../hooks/useSettings";
import { GeneralSettings } from "../general/GeneralSettings";
import { PostProcessingSettings } from "../post-processing/PostProcessingSettings";

/**
 * "Diktieren" tab on the Einstellungen page (plan §6.3): the dictation
 * workflow settings (shortcuts, audio, model) plus AI post-processing,
 * which only appears while post-processing is enabled — same visibility
 * rule as before the consolidation.
 */
export const DictationSettingsTab: React.FC = () => {
  const { getSetting } = useSettings();
  const postProcessEnabled = getSetting("post_process_enabled") ?? false;

  return (
    <>
      <GeneralSettings />
      {postProcessEnabled && <PostProcessingSettings />}
    </>
  );
};
