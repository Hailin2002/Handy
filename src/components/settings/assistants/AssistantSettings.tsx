import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { AssistantShortcuts } from "../general/AssistantShortcuts";
import { AssistantProfileSettings } from "./AssistantProfileSettings";
import { ASSISTANT_PROFILE_IDS } from "./assistantProfiles";

/**
 * The two AI assistants, one settings group each (plan Phase F).
 *
 * Both profiles are configured independently — separate provider, separate key,
 * separate model — because that is exactly what the backend stores: keys live
 * under the assistant secret scope keyed by *profile* id, so the same provider
 * can back both assistants with different credentials.
 */
export const AssistantSettings: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-6 max-w-2xl mx-auto py-4">
      <div className="px-4">
        <h2 className="text-lg font-semibold">
          {t("settings.assistants.title")}
        </h2>
        <p className="text-sm text-mid-gray mt-1">
          {t("settings.assistants.description")}
        </p>
      </div>
      <SettingsGroup title={t("settings.general.shortcut.title")}>
        <AssistantShortcuts />
      </SettingsGroup>
      {ASSISTANT_PROFILE_IDS.map((profileId) => (
        <AssistantProfileSettings key={profileId} profileId={profileId} />
      ))}
    </div>
  );
};
