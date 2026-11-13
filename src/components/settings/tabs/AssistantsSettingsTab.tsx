import React from "react";
import { AssistantSettings } from "../assistants/AssistantSettings";

/**
 * "Assistants" tab on the Einstellungen page.
 *
 * The assistants are a third tool alongside dictation and meetings, with their
 * own shortcuts, so they get their own tab rather than a section inside
 * "Modelle & Provider" — that tab is about speech-to-text models. This follows
 * the tab layout's existing rule: each tool configures its own LLM provider
 * inside its own tab (post-processing lives under Diktieren, the summary
 * provider under Meetings), instead of a shared provider page.
 */
export const AssistantsSettingsTab: React.FC = () => <AssistantSettings />;
