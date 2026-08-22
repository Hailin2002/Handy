// Settings section pages
export { GeneralSettings } from "./general/GeneralSettings";
export { AdvancedSettings } from "./advanced/AdvancedSettings";
export { DebugSettings } from "./debug/DebugSettings";
export { HistorySettings } from "./history/HistorySettings";
export { AboutSettings } from "./about/AboutSettings";
export { PostProcessingSettings } from "./post-processing/PostProcessingSettings";
export { ModelsSettings } from "./models/ModelsSettings";
export { SpeechPipelineSettings } from "./general/SpeechPipelineSettings";

// Individual setting components, grouped by the section that owns them.
// Cross-section reuse lives in shared/.
export { MicrophoneSelector } from "./general/MicrophoneSelector";
export { ChannelSelector } from "./general/ChannelSelector";
export { ClamshellMicrophoneSelector } from "./debug/ClamshellMicrophoneSelector";
export { OutputDeviceSelector } from "./general/OutputDeviceSelector";
export { AlwaysOnMicrophone } from "./debug/AlwaysOnMicrophone";
export { PushToTalk } from "./general/PushToTalk";
export { AudioFeedback } from "./general/AudioFeedback";
export { ShowOverlay } from "./advanced/ShowOverlay";
export { GlobalShortcutInput } from "./shared/GlobalShortcutInput";
export { HandyKeysShortcutInput } from "./shared/HandyKeysShortcutInput";
export { ShortcutInput } from "./general/ShortcutInput";
export { TranslateToEnglish } from "./general/TranslateToEnglish";
export { CustomWords } from "./advanced/CustomWords";
export { PostProcessingToggle } from "./advanced/PostProcessingToggle";
export { PostProcessingSettingsApi } from "./PostProcessingSettingsApi";
export { PostProcessingSettingsPrompts } from "./post-processing/PostProcessingSettings";
export { FillerWordRemoval } from "./advanced/FillerWordRemoval";
export { AppDataDirectory } from "./about/AppDataDirectory";
export { ModelUnloadTimeoutSetting } from "./advanced/ModelUnloadTimeout";
export { StartHidden } from "./advanced/StartHidden";
export { HistoryLimit } from "./history/HistoryLimit";
export { RecordingRetentionPeriodSelector } from "./history/RecordingRetentionPeriod";
export { AutostartToggle } from "./advanced/AutostartToggle";
export { UpdateChecksToggle } from "./debug/UpdateChecksToggle";
export { ShowWhatsNewOnUpdate } from "./about/ShowWhatsNewOnUpdate";
