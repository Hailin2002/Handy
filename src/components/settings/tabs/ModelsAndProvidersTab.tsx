import React from "react";
import { ModelsSettings } from "../models/ModelsSettings";
import { SpeechPipelineSettings } from "../general/SpeechPipelineSettings";

/**
 * "Modelle & Provider" tab on the Einstellungen page (plan §6.3): local
 * model management plus the speech-input provider configuration
 * (local / Azure / custom) and DeepL translation.
 */
export const ModelsAndProvidersTab: React.FC = () => (
  <>
    <ModelsSettings />
    <SpeechPipelineSettings />
  </>
);
