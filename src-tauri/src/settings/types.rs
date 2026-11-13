//! Settings value types: user-facing enums, small structs, and the
//! debug-redacting [`SecretMap`] wrapper.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fmt;

pub const APPLE_INTELLIGENCE_PROVIDER_ID: &str = "apple_intelligence";
pub const APPLE_INTELLIGENCE_DEFAULT_MODEL_ID: &str = "Apple Intelligence";

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Custom deserializer to handle both old numeric format (1-5) and new string format ("trace", "debug", etc.)
impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or integer representing log level")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &"1-5")),
                }
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

impl From<LogLevel> for tauri_plugin_log::LogLevel {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tauri_plugin_log::LogLevel::Trace,
            LogLevel::Debug => tauri_plugin_log::LogLevel::Debug,
            LogLevel::Info => tauri_plugin_log::LogLevel::Info,
            LogLevel::Warn => tauri_plugin_log::LogLevel::Warn,
            LogLevel::Error => tauri_plugin_log::LogLevel::Error,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct LLMPrompt {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PostProcessProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub supports_structured_output: bool,
}

/// What an assistant does with the answer it produced.
///
/// `Show` only renders the result card and is the default for both shipped
/// profiles: an assistant that types into the focused app without being asked
/// would be destructive when the shortcut is hit by accident.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssistantOutputMode {
    /// Display the answer in the overlay card and do nothing else.
    #[default]
    Show,
    /// Display the answer and copy it to the clipboard.
    Copy,
    /// Display the answer and paste it into the focused application.
    Paste,
}

/// One independently configured assistant.
///
/// Profiles deliberately do **not** share provider credentials with
/// post-processing: the same provider can be used with a different key per
/// profile, so keys live under [`crate::secret_store::ASSISTANT_SCOPE`] keyed
/// by [`AssistantProfile::id`], not by `provider_id`.
///
/// `max_output_tokens`, `timeout_secs` and `disable_reasoning` map directly
/// onto `llm_client::ChatOptions`; keep them in that shape so the execution
/// path stays a plain field-for-field translation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Type)]
pub struct AssistantProfile {
    /// Stable identity. Also the `assistant_profiles` map key and the secret
    /// id — the two are kept in sync by `ensure_assistant_defaults`.
    pub id: String,
    /// Id of an entry in `post_process_providers`; the provider catalogue is
    /// shared, only the credentials are not.
    pub provider_id: String,
    pub model: String,
    pub system_prompt: String,
    pub max_output_tokens: u32,
    pub timeout_secs: u32,
    pub output_mode: AssistantOutputMode,
    /// Ask the endpoint to skip reasoning output — the difference between a
    /// snappy lookup and a model that thinks for ten seconds first.
    pub disable_reasoning: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    // `none` is retired: overlay visibility is owned by `OverlayStyle` now. The
    // alias keeps legacy stores (`"overlay_position": "none"`) deserializing
    // instead of failing the whole load; the one-time overlay migration reads the
    // raw stored string to recover the old "hidden" intent as `OverlayStyle::None`.
    #[serde(alias = "none")]
    Bottom,
}

/// Which recording overlay to display. `Minimal` and `Live` share one base
/// (the pill); `Live` grows into the panel that shows live transcription text.
/// `None` hides the overlay entirely. Decoupled from whether the model runs in
/// streaming mode (that is driven purely by model capability).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayStyle {
    None,
    Minimal,
    Live,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnloadTimeout {
    Never,
    Immediately,
    Min2,
    #[default]
    Min5,
    Min10,
    Min15,
    Hour1,
    Sec15, // Debug mode only
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    CtrlV,
    Direct,
    None,
    ShiftInsert,
    CtrlShiftV,
    ExternalScript,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHandling {
    #[default]
    DontModify,
    CopyToClipboard,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSubmitKey {
    #[default]
    Enter,
    CtrlEnter,
    CmdEnter,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecordingRetentionPeriod {
    Never,
    PreserveLimit,
    Days3,
    Weeks2,
    Months3,
}

/// How long a meeting's raw audio is kept after it finished processing
/// (plan §12).
///
/// [`MeetingRetentionPolicy::Ask`] is the default and deletes **nothing**: the
/// plan is explicit that "ohne beantwortete Auswahl wird nichts automatisch
/// gelöscht", so an unanswered prompt can never cost a user their recordings.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingRetentionPolicy {
    /// No automatic deletion until the user actually chooses.
    #[default]
    Ask,
    /// Drop the audio as soon as processing finished successfully.
    Immediately,
    /// Keep the audio for seven days after the meeting ended.
    // Renamed explicitly: `snake_case` would emit `days_7`, and the wire value
    // doubles as the i18n key suffix, where the underscore reads as a typo.
    #[serde(rename = "days7")]
    Days7,
    /// Keep the audio for thirty days after the meeting ended.
    #[serde(rename = "days30")]
    Days30,
    /// Never delete automatically.
    Forever,
}

impl MeetingRetentionPolicy {
    /// Stable lower-case token, matching the serialized form.
    ///
    /// Persisted on a session so a meeting keeps the rule that was in force
    /// when it was recorded, even if the global setting changes later.
    pub fn as_token(self) -> &'static str {
        match self {
            MeetingRetentionPolicy::Ask => "ask",
            MeetingRetentionPolicy::Immediately => "immediately",
            MeetingRetentionPolicy::Days7 => "days7",
            MeetingRetentionPolicy::Days30 => "days30",
            MeetingRetentionPolicy::Forever => "forever",
        }
    }

    /// Age after which audio may be deleted, or `None` when this policy never
    /// deletes on its own.
    pub fn max_age(self) -> Option<chrono::Duration> {
        match self {
            MeetingRetentionPolicy::Ask | MeetingRetentionPolicy::Forever => None,
            MeetingRetentionPolicy::Immediately => Some(chrono::Duration::zero()),
            MeetingRetentionPolicy::Days7 => Some(chrono::Duration::days(7)),
            MeetingRetentionPolicy::Days30 => Some(chrono::Duration::days(30)),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardImplementation {
    Tauri,
    HandyKeys,
}

impl Default for KeyboardImplementation {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        return KeyboardImplementation::Tauri;
        #[cfg(not(target_os = "linux"))]
        return KeyboardImplementation::HandyKeys;
    }
}

impl Default for PasteMethod {
    fn default() -> Self {
        // Shift+Insert bypasses user-level Ctrl+V remaps (for example an
        // AutoHotkey macOS-style Alt+V mapping) while still pasting plain
        // Unicode clipboard text into standard Windows controls and webviews.
        #[cfg(target_os = "windows")]
        return PasteMethod::ShiftInsert;
        #[cfg(target_os = "linux")]
        return PasteMethod::Direct;
        #[cfg(target_os = "macos")]
        return PasteMethod::CtrlV;
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        return PasteMethod::CtrlV;
    }
}

impl ModelUnloadTimeout {
    pub fn to_minutes(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Min2 => Some(2),
            ModelUnloadTimeout::Min5 => Some(5),
            ModelUnloadTimeout::Min10 => Some(10),
            ModelUnloadTimeout::Min15 => Some(15),
            ModelUnloadTimeout::Hour1 => Some(60),
            ModelUnloadTimeout::Sec15 => Some(0), // Special case for debug - handled separately
        }
    }

    pub fn to_seconds(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Sec15 => Some(15),
            _ => self.to_minutes().map(|m| m * 60),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SoundTheme {
    Marimba,
    Pop,
    Custom,
}

impl SoundTheme {
    fn as_str(&self) -> &'static str {
        match self {
            SoundTheme::Marimba => "marimba",
            SoundTheme::Pop => "pop",
            SoundTheme::Custom => "custom",
        }
    }

    pub fn to_start_path(self) -> String {
        format!("resources/{}_start.wav", self.as_str())
    }

    pub fn to_stop_path(self) -> String {
        format!("resources/{}_stop.wav", self.as_str())
    }
}

/// UI appearance mode. `System` follows the OS `prefers-color-scheme`; `Light`
/// and `Dark` force one of the two palettes Handy already ships.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// What Handy does to the system's audio output while it is recording.
///
/// Supersedes the older `mute_while_recording` bool, which is kept in the
/// schema and write-through mirrored (`mirrors_mute_flag`) so existing UI,
/// fixtures and the frozen-store test keep working. `recording_audio_mode` is
/// the runtime truth; the bool is a derived shadow.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingAudioMode {
    /// Leave system audio alone.
    #[default]
    Off,
    /// Lower system output volume to `duck_volume_percent` while recording.
    Duck,
    /// Mute system output entirely — the pre-existing behavior.
    Mute,
}

impl RecordingAudioMode {
    /// The value the legacy `mute_while_recording` bool must carry for this
    /// mode. Single source of the write-through mirror described above, so the
    /// two fields can never drift apart by someone forgetting one side.
    pub const fn mirrors_mute_flag(self) -> bool {
        matches!(self, RecordingAudioMode::Mute)
    }

    /// Reconstructs a mode from the legacy bool. Used by the one-time store
    /// migration and by the legacy `change_mute_while_recording_setting`
    /// command, so both interpret an old store identically.
    pub const fn from_legacy_mute_flag(mute_while_recording: bool) -> Self {
        if mute_while_recording {
            RecordingAudioMode::Mute
        } else {
            RecordingAudioMode::Off
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypingTool {
    #[default]
    Auto,
    Wtype,
    Kwtype,
    Dotool,
    Ydotool,
    Xdotool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscribeAcceleratorSetting {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrtAcceleratorSetting {
    #[default]
    Auto,
    Cpu,
    Cuda,
    #[serde(rename = "directml")]
    DirectMl,
    Rocm,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpeechInputProvider {
    #[default]
    Local,
    Azure,
    Custom,
}

#[derive(Clone, Deserialize, Type)]
#[serde(transparent)]
pub struct SecretMap(pub(crate) HashMap<String, String>);

impl Serialize for SecretMap {
    /// Secrets never reach disk through this type: API keys live in the OS
    /// keychain (see [`crate::secret_store`]). Serializing as an empty map is
    /// defense in depth — even if a value is still present in memory (legacy
    /// migration input, placeholder defaults), settings persistence can only
    /// ever write `{}`.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let map = serializer.serialize_map(Some(0))?;
        map.end()
    }
}

impl fmt::Debug for SecretMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted: HashMap<&String, &str> = self
            .0
            .iter()
            .map(|(k, v)| (k, if v.is_empty() { "" } else { "[REDACTED]" }))
            .collect();
        redacted.fmt(f)
    }
}

impl std::ops::Deref for SecretMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SecretMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/* still handy for composing the initial JSON in the store ------------- */
