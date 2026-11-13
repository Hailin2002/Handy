use crate::audio_ducking;
use crate::audio_toolkit::{
    list_input_devices,
    vad::{
        SmoothedVad, VAD_OFFLINE_HANGOVER_FRAMES, VAD_ONSET_FRAMES, VAD_PREFILL_FRAMES,
        VAD_STREAMING_HANGOVER_FRAMES,
    },
    AudioRecorder, SileroVad, VadPolicy,
};
use crate::helpers::clamshell;
use crate::managers::transcription::StreamRouter;
use crate::settings::{get_settings, AppSettings, RecordingAudioMode};
use crate::speech_pipeline::AzureSpeechStreamManager;
use crate::sync::MutexExt;
use crate::utils;
use log::{debug, error, info, trace, warn};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const VAD_THRESHOLD: f32 = 0.3;

fn set_mute(mute: bool) {
    // Expected behavior:
    // - Windows: works on most systems using standard audio drivers.
    // - Linux: works on many systems (PipeWire, PulseAudio, ALSA),
    //   but some distros may lack the tools used.
    // - macOS: works on most standard setups via AppleScript.
    // If unsupported, fails silently.

    #[cfg(target_os = "windows")]
    {
        unsafe {
            use windows::Win32::{
                Media::Audio::{
                    eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                    MMDeviceEnumerator,
                },
                System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
            };

            macro_rules! unwrap_or_return {
                ($expr:expr) => {
                    match $expr {
                        Ok(val) => val,
                        Err(_) => return,
                    }
                };
            }

            // Initialize the COM library for this thread.
            // If already initialized (e.g., by another library like Tauri), this does nothing.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let all_devices: IMMDeviceEnumerator =
                unwrap_or_return!(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL));
            let default_device =
                unwrap_or_return!(all_devices.GetDefaultAudioEndpoint(eRender, eMultimedia));
            let volume_interface = unwrap_or_return!(
                default_device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            );

            let _ = volume_interface.SetMute(mute, std::ptr::null());
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mute_val = if mute { "1" } else { "0" };
        let amixer_state = if mute { "mute" } else { "unmute" };

        // Try multiple backends to increase compatibility
        // 1. PipeWire (wpctl)
        if Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", mute_val])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return;
        }

        // 2. PulseAudio (pactl)
        if Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", mute_val])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return;
        }

        // 3. ALSA (amixer)
        let _ = Command::new("amixer")
            .args(["set", "Master", amixer_state])
            .output();
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let script = format!(
            "set volume output muted {}",
            if mute { "true" } else { "false" }
        );
        let _ = Command::new("osascript").args(["-e", &script]).output();
    }
}

/// Reads the current system output mute state, mirroring `set_mute`'s backends.
///
/// Returns `Some(true)`/`Some(false)` when the state could be determined, or
/// `None` when it couldn't (unsupported platform, missing CLI tools, or an
/// error). Callers treat `None` as "unknown" and fall back to unmuting on stop,
/// so we never strand the user's audio muted.
#[cfg(target_os = "windows")]
fn get_mute() -> Option<bool> {
    unsafe {
        use windows::Win32::{
            Media::Audio::{
                eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                MMDeviceEnumerator,
            },
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
        };

        // Matches set_mute: no-op if COM is already initialized on this thread.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let all_devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let default_device = all_devices
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?;
        let volume_interface = default_device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .ok()?;

        Some(volume_interface.GetMute().ok()?.as_bool())
    }
}

#[cfg(target_os = "linux")]
fn get_mute() -> Option<bool> {
    use std::process::Command;

    // 1. PipeWire (wpctl): prints "[MUTED]" in the volume line when muted.
    if let Ok(out) = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
    {
        if out.status.success() {
            return Some(String::from_utf8_lossy(&out.stdout).contains("[MUTED]"));
        }
    }

    // 2. PulseAudio (pactl): prints "Mute: yes" / "Mute: no".
    // Force LC_ALL=C so a localized system still emits the parseable English
    // "yes"/"no" instead of e.g. "ja"/"nein".
    if let Ok(out) = Command::new("pactl")
        .env("LC_ALL", "C")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if s.contains("yes") {
                return Some(true);
            }
            if s.contains("no") {
                return Some(false);
            }
        }
    }

    // 3. ALSA (amixer): prints "[off]" for muted channels, "[on]" otherwise.
    // LC_ALL=C keeps the "[on]"/"[off]" tokens stable across locales.
    if let Ok(out) = Command::new("amixer")
        .env("LC_ALL", "C")
        .args(["get", "Master"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if s.contains("[off]") {
                return Some(true);
            }
            if s.contains("[on]") {
                return Some(false);
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn get_mute() -> Option<bool> {
    use std::process::Command;

    let out = Command::new("osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn get_mute() -> Option<bool> {
    None
}

/// Restores the system mute state after our forced mute, given the state
/// captured just before we muted. We only ever need to unmute — and only when
/// the system was NOT already muted beforehand. If the prior state was muted,
/// we leave it muted (the user's own state). If it's unknown (`None`), we
/// default to unmuting so audio is never left stranded muted by us.
fn restore_mute(prev_muted: Option<bool>) {
    if prev_muted != Some(true) {
        set_mute(false);
    }
}

const WHISPER_SAMPLE_RATE: usize = 16000;

/* ──────────────────────────────────────────────────────────────── */

#[derive(Clone, Debug)]
pub enum RecordingState {
    Idle,
    Recording { binding_id: String },
    Stopping,
}

#[derive(Clone, Debug)]
pub enum MicrophoneMode {
    AlwaysOn,
    OnDemand,
}

/// Tracks whatever we did to the system's audio output for the current
/// recording, so we can put it back exactly as it was.
///
/// `engaged` is true only while an attenuation of ours is actually active — it
/// is the idempotency guard *and* the "is there anything to restore" answer.
/// Exactly one of the two snapshots is populated, decided by the mode that
/// engaged: `prev_muted` for [`RecordingAudioMode::Mute`], `prev_volume` for
/// [`RecordingAudioMode::Duck`]. Which one is set is how [`plan_restore`] knows
/// what to undo, so they must never both be `Some`.
#[derive(Debug, Default, Clone, Copy)]
struct DuckState {
    engaged: bool,
    prev_muted: Option<bool>,
    prev_volume: Option<f32>,
    /// The ramp length in effect when we engaged, captured alongside the
    /// volume. Two reasons: the restore uses the same ramp the duck did even if
    /// the user moved the slider mid-recording, and the restore path never has
    /// to read settings — so it never holds this lock across the global
    /// settings transaction lock.
    fade_ms: u16,
    /// The level we ducked *to*. Compared against the observed volume at
    /// restore time: a different value means the user moved the slider during
    /// the recording, and the restore is abandoned rather than reverting an
    /// explicit choice of theirs.
    duck_target: Option<f32>,
}

/// What [`AudioRecordingManager::apply_recording_audio`] should do, decided
/// without touching the platform so the policy is unit-testable.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DuckPlan {
    /// Leave system audio alone.
    Skip,
    /// Snapshot the mute state and mute.
    Mute,
    /// Snapshot the volume and ramp down to `target` (0.0..=1.0).
    Duck { target: f32 },
}

/// What [`AudioRecordingManager::remove_recording_audio`] should undo.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RestorePlan {
    /// Nothing was engaged — a no-op. Every safety-net caller relies on this
    /// being cheap and harmless, since several of them fire on paths where the
    /// primary restore has usually already run.
    Nothing,
    /// Restore the captured mute state.
    Mute(Option<bool>),
    /// Ramp back up to exactly `target`, over the ramp length captured at
    /// engage time.
    Volume {
        target: f32,
        fade_ms: u16,
        duck_target: Option<f32>,
    },
}

/// Decides whether and how to attenuate system audio for a recording.
///
/// `meeting_active` short-circuits everything: a meeting recording captures the
/// system loopback (`audio_toolkit/audio/meeting_recorder.rs`), so ducking or
/// muting the render endpoint would attenuate the meeting's own recording, not
/// just what the user hears. A slightly loud dictation is recoverable; a
/// half-silent meeting recording is not.
fn plan_recording_audio(
    mode: RecordingAudioMode,
    duck_volume_percent: u8,
    already_engaged: bool,
    stream_is_open: bool,
    meeting_active: bool,
) -> DuckPlan {
    // Already engaged this session: re-planning would re-snapshot on top of our
    // own forced state and strand the user muted/quiet on stop.
    if already_engaged || !stream_is_open || meeting_active {
        return DuckPlan::Skip;
    }

    match mode {
        RecordingAudioMode::Off => DuckPlan::Skip,
        RecordingAudioMode::Mute => DuckPlan::Mute,
        RecordingAudioMode::Duck => DuckPlan::Duck {
            target: duck_target_scalar(duck_volume_percent),
        },
    }
}

/// Decides what to undo. See [`DuckState`] for why the populated snapshot
/// identifies the mode.
fn plan_restore(state: &DuckState) -> RestorePlan {
    if !state.engaged {
        return RestorePlan::Nothing;
    }
    match state.prev_volume {
        Some(target) => RestorePlan::Volume {
            target,
            fade_ms: state.fade_ms,
            duck_target: state.duck_target,
        },
        None => RestorePlan::Mute(state.prev_muted),
    }
}

/// Clamps the persisted percentage and converts it to the 0.0..=1.0 scalar the
/// platform layer speaks. Defensive clamp: the command clamps on write, but a
/// hand-edited store must not be able to ask for 300%.
fn duck_target_scalar(duck_volume_percent: u8) -> f32 {
    f32::from(duck_volume_percent.min(100)) / 100.0
}

/// Steps in a fade ramp. Ten is enough to be inaudible as stepping at the
/// default 120ms and few enough that the per-step platform call (a COM
/// round-trip on Windows, a process spawn on Linux/macOS) stays affordable.
const FADE_STEPS: u32 = 10;

/// Runs one volume ramp to completion on the calling thread, aborting the
/// moment `fade_generation` moves past `my_generation`.
///
/// The final value is written as `to` verbatim rather than as the last
/// interpolation step, so restoring never lands a rounding-drifted volume on
/// the user's system.
/// Above this, the gap between the observed volume and the duck target is a
/// deliberate user action, not rounding. Platform backends round to whole
/// percent (`osascript`, `amixer`) or to the endpoint's own step, so drift of
/// about 0.01 can come from the write path alone; a single keyboard volume
/// press on Windows is 0.02 and is exactly the intent we must respect. 0.015
/// separates the two.
const USER_MOVED_EPSILON: f32 = 0.015;

/// Whether the user changed the volume while it was ducked.
///
/// Unknown on either side means "do not accuse the user of anything" and lets
/// the restore proceed.
fn user_moved_volume(current: Option<f32>, duck_target: Option<f32>) -> bool {
    match (current, duck_target) {
        (Some(current), Some(target)) => (current - target).abs() > USER_MOVED_EPSILON,
        _ => false,
    }
}

/// Serialises every platform volume write so a generation check and the write
/// it guards are atomic with respect to any other fade.
///
/// Without it a superseded ramp could pass its check, stall inside a process
/// spawn (`osascript`, `wpctl`) for longer than the whole replacement ramp, and
/// land its stale value *after* the restore had finished - leaving the volume
/// at the duck target for good.
static VOLUME_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Write `value` unless `fade_generation` has moved past `my_generation`.
/// Returns whether the write happened.
fn write_volume_if_current(fade_generation: &AtomicU64, my_generation: u64, value: f32) -> bool {
    let _serialised = VOLUME_WRITE_LOCK.lock_recover();
    if fade_generation.load(Ordering::SeqCst) != my_generation {
        return false;
    }
    audio_ducking::set_output_volume(value);
    true
}

/// Whether a ramp of `fade_ms` can be scheduled as `FADE_STEPS` steps at all.
///
/// Below this, ten back-to-back platform writes - ten process spawns on Linux
/// and macOS - for a "fade" of a few milliseconds is strictly worse than
/// landing on the target directly.
fn ramp_has_steps(fade_ms: u16) -> bool {
    u32::from(fade_ms) >= FADE_STEPS
}

fn run_fade(fade_generation: Arc<AtomicU64>, my_generation: u64, from: f32, to: f32, fade_ms: u16) {
    let total = Duration::from_millis(u64::from(fade_ms));
    let start = std::time::Instant::now();

    if ramp_has_steps(fade_ms) {
        for step in 1..FADE_STEPS {
            // Deadline-driven rather than a fixed per-step sleep: the ramp takes
            // `fade_ms` regardless of how long each write blocks, and a short
            // ramp no longer truncates `fade_ms / FADE_STEPS` to a zero sleep.
            let progress = step as f32 / FADE_STEPS as f32;
            let due = start + total.mul_f32(progress);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(wait);
            }
            if !write_volume_if_current(
                &fade_generation,
                my_generation,
                from + (to - from) * progress,
            ) {
                trace!("Volume fade superseded; aborting mid-ramp");
                return;
            }
        }
        if let Some(wait) = (start + total).checked_duration_since(std::time::Instant::now()) {
            std::thread::sleep(wait);
        }
    }

    if !write_volume_if_current(&fade_generation, my_generation, to) {
        trace!("Volume fade superseded; aborting before final step");
    }
}

/// The persisted microphone preference currently in effect. Clamshell and
/// regular selections are kept distinct so losing a clamshell-only device does
/// not erase the user's normal microphone preference.
enum DesiredMicrophone {
    Default,
    Selected(String),
    Clamshell(String),
}

/// Result of resolving the persisted preference to a live cpal device.
/// `device: None` means cpal should open the system default. The unavailable
/// name is populated only when enumeration succeeded and confirmed that the
/// user's regular selected microphone is missing.
struct MicrophoneResolution {
    device: Option<cpal::Device>,
    unavailable_selected_microphone: Option<String>,
}

/* ──────────────────────────────────────────────────────────────── */

fn create_audio_recorder(
    vad_path: &Path,
    app_handle: &tauri::AppHandle,
    selected_channel: Option<u16>,
    stream_router: Arc<StreamRouter>,
    azure_stream: Arc<AzureSpeechStreamManager>,
) -> Result<AudioRecorder, anyhow::Error> {
    // A single Silero engine covers both the offline and streaming policies (never
    // active at once within a recording), so the recorder reconfigures its
    // hangover tail per session rather than keeping two ONNX sessions resident.
    let silero = SileroVad::new(vad_path, VAD_THRESHOLD)
        .map_err(|e| anyhow::anyhow!("Failed to create SileroVad: {}", e))?;
    let smoothed_vad = SmoothedVad::new(
        Box::new(silero),
        VAD_PREFILL_FRAMES,
        VAD_OFFLINE_HANGOVER_FRAMES,
        VAD_ONSET_FRAMES,
    );

    // Recorder with VAD, a spectrum-level callback that forwards level updates to
    // the frontend, and an audio-frame callback that feeds live streaming via a
    // shared `StreamRouter` (captured directly, not via Tauri state — see its docs).
    let recorder = AudioRecorder::new()
        .map_err(|e| anyhow::anyhow!("Failed to create AudioRecorder: {}", e))?
        .with_vad(
            Box::new(smoothed_vad),
            VAD_OFFLINE_HANGOVER_FRAMES,
            VAD_STREAMING_HANGOVER_FRAMES,
        )
        .with_selected_channel(selected_channel)
        .with_level_callback({
            let app_handle = app_handle.clone();
            move |levels| {
                utils::emit_levels(&app_handle, &levels);
            }
        })
        .with_audio_callback({
            let router = stream_router;
            move |frame| {
                router.feed(frame);
                azure_stream.feed(frame);
            }
        });

    Ok(recorder)
}

/* ──────────────────────────────────────────────────────────────── */

/// One recording session's first-sample notification. Waiting on this never
/// blocks the shortcut coordinator: callers hand it to a dedicated worker.
pub struct RecordingReadiness {
    receiver: mpsc::Receiver<()>,
    generation: u64,
}

impl RecordingReadiness {
    pub fn wait(self) -> bool {
        self.receiver.recv().is_ok()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone)]
pub struct AudioRecordingManager {
    /// Never assign through this directly — route every write through
    /// `set_state()`, which keeps `recording_active` in sync.
    state: Arc<Mutex<RecordingState>>,
    mode: Arc<Mutex<MicrophoneMode>>,
    app_handle: tauri::AppHandle,

    recorder: Arc<Mutex<Option<AudioRecorder>>>,
    is_open: Arc<Mutex<bool>>,
    is_recording: Arc<Mutex<bool>>,
    duck_state: Arc<Mutex<DuckState>>,
    /// Bumped by every volume change we initiate. A fade thread compares it
    /// against the generation it was spawned with and stops as soon as they
    /// differ, so a restore-up ramp can never interleave with the duck-down
    /// ramp it supersedes (which would leave the volume at an arbitrary
    /// intermediate value).
    fade_generation: Arc<AtomicU64>,
    close_generation: Arc<AtomicU64>,
    cancel_generation: Arc<AtomicU64>,
    stream_router: Arc<StreamRouter>,
    azure_stream: Arc<AzureSpeechStreamManager>,
    /// Lock-free mirror of "is the state in {Recording, Stopping}",
    /// maintained by `set_state()`. The hot-path `is_recording()` reads THIS
    /// instead of the std `state` mutex, so a UI poll can no longer deadlock
    /// the main/webview thread when a worker holds `state` across a slow
    /// CoreAudio open/close.
    recording_active: Arc<AtomicBool>,
    /// Invalidates asynchronous first-sample UI/chime work when a recording is
    /// stopped or cancelled. This prevents a slow device from producing a late
    /// "ready" indication for a session the user already ended.
    capture_generation: Arc<AtomicU64>,
    /// Resolution of a *named* microphone (selected or clamshell) to its cpal
    /// device, cached so on-demand recording starts skip the full device
    /// enumeration (~40-110ms). Keyed by the resolved name, so a settings
    /// change misses naturally; cleared when an open fails (device unplugged)
    /// so the retry re-enumerates. The system-default case is never cached —
    /// the recorder resolves the current default itself, cheaply.
    cached_device: Arc<Mutex<Option<(String, cpal::Device)>>>,
}

/// Decision core of [`AudioRecordingManager::persist_default_microphone_after_fallback`],
/// kept pure so the settings-concurrency tests can pin the bool-returning
/// contract against its pre-refactor (`b677107^`) behavior: returns `true`
/// only when the persisted selection is still the microphone that fell back,
/// in which case it is cleared; any other current value is left untouched and
/// reported as "no change".
pub(crate) fn clear_unavailable_selected_microphone(
    settings: &mut AppSettings,
    unavailable_name: &str,
) -> bool {
    if settings.selected_microphone.as_deref() != Some(unavailable_name) {
        return false;
    }

    settings.selected_microphone = None;
    true
}

impl AudioRecordingManager {
    /* ---------- construction ------------------------------------------------ */

    pub fn new(
        app: &tauri::AppHandle,
        stream_router: Arc<StreamRouter>,
        azure_stream: Arc<AzureSpeechStreamManager>,
    ) -> Result<Self, anyhow::Error> {
        let settings = get_settings(app);
        let mode = if settings.always_on_microphone {
            MicrophoneMode::AlwaysOn
        } else {
            MicrophoneMode::OnDemand
        };

        let manager = Self {
            state: Arc::new(Mutex::new(RecordingState::Idle)),
            mode: Arc::new(Mutex::new(mode.clone())),
            app_handle: app.clone(),

            recorder: Arc::new(Mutex::new(None)),
            is_open: Arc::new(Mutex::new(false)),
            is_recording: Arc::new(Mutex::new(false)),
            duck_state: Arc::new(Mutex::new(DuckState::default())),
            fade_generation: Arc::new(AtomicU64::new(0)),
            close_generation: Arc::new(AtomicU64::new(0)),
            cancel_generation: Arc::new(AtomicU64::new(0)),
            stream_router,
            azure_stream,
            recording_active: Arc::new(AtomicBool::new(false)),
            capture_generation: Arc::new(AtomicU64::new(0)),
            cached_device: Arc::new(Mutex::new(None)),
        };

        // Always-on?  Open immediately.
        if matches!(mode, MicrophoneMode::AlwaysOn) {
            manager.start_microphone_stream()?;
        }

        Ok(manager)
    }

    /* ---------- helper methods --------------------------------------------- */

    /// The persisted microphone preference currently in effect. Only runs the
    /// clamshell probe (an `ioreg` subprocess, ~10-20ms) when a clamshell
    /// microphone is actually configured.
    fn desired_microphone(&self, settings: &AppSettings) -> DesiredMicrophone {
        if let Some(clamshell_microphone) = &settings.clamshell_microphone {
            let clamshell_started = Instant::now();
            let is_clamshell = clamshell::is_clamshell().unwrap_or(false);
            debug!(
                "device resolve: clamshell_check={:?} (clamshell={})",
                clamshell_started.elapsed(),
                is_clamshell
            );
            if is_clamshell {
                return DesiredMicrophone::Clamshell(clamshell_microphone.clone());
            }
        }
        match &settings.selected_microphone {
            Some(name) => DesiredMicrophone::Selected(name.clone()),
            None => DesiredMicrophone::Default,
        }
    }

    pub fn invalidate_device_cache(&self) {
        *self.cached_device.lock_recover() = None;
    }

    fn resolve_microphone_device(&self, settings: &AppSettings) -> MicrophoneResolution {
        let desired = self.desired_microphone(settings);
        let (device_name, selected_microphone) = match desired {
            DesiredMicrophone::Default => {
                debug!("device resolve: no mic configured -> system default");
                return MicrophoneResolution {
                    device: None,
                    unavailable_selected_microphone: None,
                };
            }
            DesiredMicrophone::Selected(name) => (name.clone(), Some(name)),
            DesiredMicrophone::Clamshell(name) => (name, None),
        };

        // Cache hit: skip the full enumeration. A stale device (unplugged)
        // fails at open, where the caller invalidates and retries fresh.
        if let Some((cached_name, device)) = self.cached_device.lock_recover().as_ref() {
            if *cached_name == device_name {
                debug!("device resolve: cache hit for '{}'", device_name);
                return MicrophoneResolution {
                    device: Some(device.clone()),
                    unavailable_selected_microphone: None,
                };
            }
        }

        // Only report a selected microphone as unavailable when enumeration
        // itself succeeded. A backend enumeration error may be transient and
        // must not erase the user's persisted preference.
        let enumerate_started = Instant::now();
        let (device, enumeration_succeeded) = match list_input_devices() {
            Ok(devices) => (
                devices
                    .into_iter()
                    .find(|d| d.name == device_name)
                    .map(|d| d.device),
                true,
            ),
            Err(e) => {
                debug!("Failed to list devices, using default: {}", e);
                (None, false)
            }
        };
        debug!(
            "device resolve: enumerate={:?} (found={})",
            enumerate_started.elapsed(),
            device.is_some()
        );
        if let Some(d) = &device {
            *self.cached_device.lock_recover() = Some((device_name, d.clone()));
        }

        let unavailable_selected_microphone = if enumeration_succeeded && device.is_none() {
            selected_microphone
        } else {
            None
        };
        MicrophoneResolution {
            device,
            unavailable_selected_microphone,
        }
    }

    /// Keep persisted settings and the UI aligned with a successful runtime
    /// fallback. Re-read first so recovery cannot clear a microphone the user
    /// selected concurrently while the stream was being rebuilt.
    fn persist_default_microphone_after_fallback(&self, unavailable_name: &str) {
        let changed = crate::settings::update_settings(&self.app_handle, |settings| {
            clear_unavailable_selected_microphone(settings, unavailable_name)
        });
        if !changed {
            return;
        }
        let _ = self.app_handle.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "selected_microphone",
                "value": "Default"
            }),
        );
    }

    fn schedule_lazy_close(&self) {
        let gen = self.close_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(STREAM_IDLE_TIMEOUT);
            let rm = app.state::<Arc<AudioRecordingManager>>();
            // Hold state lock across the check AND close to serialize against
            // try_start_recording, preventing a race where the stream is closed
            // under an active recording.
            let state = rm.state.lock_recover();
            if rm.close_generation.load(Ordering::SeqCst) == gen
                && matches!(*state, RecordingState::Idle)
            {
                // stop_microphone_stream does not acquire the state lock,
                // so holding it here is safe (no deadlock).
                info!(
                    "Closing idle microphone stream after {:?}",
                    STREAM_IDLE_TIMEOUT
                );
                rm.stop_microphone_stream();
            }
        });
    }

    /* ---------- microphone life-cycle -------------------------------------- */

    /// True while a meeting session owns the capture devices.
    ///
    /// Read *before* any audio lock is taken (see the call sites): the meeting
    /// manager holds its own `inner` mutex across settings reads and overlay
    /// work, so acquiring it under one of ours would build a lock cycle. It is
    /// also why this must never be called from inside a `settings::update_*`
    /// closure — that closure holds the global settings transaction lock, and
    /// the meeting manager's established order is `inner` → settings lock.
    fn meeting_capture_active(&self) -> bool {
        self.app_handle
            .try_state::<Arc<crate::managers::meeting::MeetingManager>>()
            .and_then(|manager| manager.active_capture_state())
            .is_some()
    }

    /// Attenuates system audio for this recording according to
    /// `recording_audio_mode`, snapshotting whatever it changes first so
    /// [`Self::remove_recording_audio`] can put it back exactly.
    ///
    /// Idempotent: a duplicate or late call after one already engaged is a
    /// no-op, because re-snapshotting on top of our own forced state would
    /// strand the user's audio muted or quiet on stop.
    pub fn apply_recording_audio(&self, generation: u64) {
        let settings = get_settings(&self.app_handle);
        if matches!(settings.recording_audio_mode, RecordingAudioMode::Off) {
            // Cheap exit before the meeting query, which takes a foreign mutex.
            return;
        }
        let meeting_active = self.meeting_capture_active();

        // Lock order: is_open before duck_state (matches stop_microphone_stream).
        let is_open = self.is_open.lock_recover();
        let mut duck_guard = self.duck_state.lock_recover();

        // Re-checked *under the lock*. The caller's check ran before the settings
        // read, the meeting query and the lock acquisitions above, any of which
        // can block long enough for the recording to have stopped in between -
        // and stop's restore is a no-op when nothing is engaged yet. Engaging
        // now would duck a machine with nothing recording.
        if !self.is_recording_readiness_current(generation) {
            debug!("Recording ended before audio attenuation engaged; skipping");
            return;
        }

        let plan = plan_recording_audio(
            settings.recording_audio_mode,
            settings.duck_volume_percent,
            duck_guard.engaged,
            *is_open,
            meeting_active,
        );

        match plan {
            DuckPlan::Skip => {
                if meeting_active {
                    debug!("Skipping recording audio attenuation: a meeting is capturing");
                }
            }
            DuckPlan::Mute => {
                duck_guard.prev_muted = get_mute();
                duck_guard.prev_volume = None;
                set_mute(true);
                duck_guard.engaged = true;
                debug!("Mute applied (prev_muted={:?})", duck_guard.prev_muted);
            }
            DuckPlan::Duck { target } => {
                // No trustworthy snapshot means no way to restore, so we would
                // rather not touch the volume at all than leave it lowered.
                let Some(previous) = audio_ducking::get_output_volume() else {
                    debug!("System output volume unavailable; not ducking");
                    return;
                };
                // Clamped defensively: the command clamps on write, but a
                // hand-edited store must not be able to ask for a 60s ramp.
                let fade_ms = crate::settings::clamp_duck_fade_ms(settings.duck_fade_ms);
                duck_guard.prev_volume = Some(previous);
                duck_guard.prev_muted = None;
                duck_guard.fade_ms = fade_ms;
                duck_guard.duck_target = Some(target);
                duck_guard.engaged = true;
                debug!("Ducking output volume {previous:.3} -> {target:.3}");
                self.begin_volume_fade(previous, target, fade_ms);

                // Persist only after both guards are released: `update_settings`
                // takes the settings transaction lock, and the established order
                // is settings before duck state, never the reverse.
                drop(duck_guard);
                drop(is_open);
                self.persist_duck_snapshot(Some(previous));
            }
        }
    }

    /// Record (or clear) the pre-duck volume in the settings store.
    ///
    /// The in-memory snapshot dies with the process. This is what lets the next
    /// start put the volume back after a crash, a kill or a quit mid-duck -
    /// the one failure that would otherwise leave the machine quiet for good.
    /// Must be called with no duck lock held (see the lock order above).
    fn persist_duck_snapshot(&self, snapshot: Option<f32>) {
        crate::settings::update_settings(&self.app_handle, |settings| {
            settings.pending_duck_restore = snapshot;
        });
    }

    /// Put the volume back if a previous run died while ducked.
    ///
    /// Called once at startup. A `Some` here means the process ended between
    /// engaging a duck and restoring it, so the machine is still quiet and
    /// nothing else knows the value it should return to.
    pub fn restore_pending_duck_snapshot(&self) {
        let Some(volume) = get_settings(&self.app_handle).pending_duck_restore else {
            return;
        };
        log::warn!("Restoring output volume to {volume:.3} left behind by a previous run");
        audio_ducking::set_output_volume(volume);
        self.persist_duck_snapshot(None);
    }

    /// Restore synchronously and without a ramp, for process exit.
    ///
    /// A detached fade thread dies with the process mid-ramp, so exit must not
    /// rely on one. The user-moved rule still applies: quitting is not a
    /// licence to revert a volume change they made during the recording.
    pub fn remove_recording_audio_immediately(&self) {
        let plan = {
            let mut duck_guard = self.duck_state.lock_recover();
            let plan = plan_restore(&duck_guard);
            *duck_guard = DuckState::default();
            plan
        };
        match plan {
            RestorePlan::Nothing => return,
            RestorePlan::Mute(prev_muted) => restore_mute(prev_muted),
            RestorePlan::Volume {
                target,
                duck_target,
                ..
            } => {
                // Bump the generation so any in-flight ramp stops writing, then
                // land the exact target under the same lock the ramps use.
                let my_generation = self.fade_generation.fetch_add(1, Ordering::SeqCst) + 1;
                if user_moved_volume(audio_ducking::get_output_volume(), duck_target) {
                    debug!("Output volume moved by the user during the duck; leaving it alone");
                } else {
                    write_volume_if_current(&self.fade_generation, my_generation, target);
                }
            }
        }
        self.persist_duck_snapshot(None);
    }

    /// Undoes whatever [`Self::apply_recording_audio`] did. Safe and cheap to
    /// call when nothing was engaged, which is what lets the several safety-net
    /// call sites fire unconditionally.
    pub fn remove_recording_audio(&self) {
        let mut duck_guard = self.duck_state.lock_recover();
        let mut clear_snapshot = false;
        match plan_restore(&duck_guard) {
            RestorePlan::Nothing => {}
            RestorePlan::Mute(prev_muted) => {
                restore_mute(prev_muted);
                debug!("Mute removed (restored prev_muted={prev_muted:?})");
            }
            RestorePlan::Volume {
                target,
                fade_ms,
                duck_target,
            } => {
                let current = audio_ducking::get_output_volume();
                if user_moved_volume(current, duck_target) {
                    // They turned it down (or up) themselves while it was
                    // ducked. Ramping back to the snapshot would silently undo
                    // that; the explicit action wins.
                    debug!("Output volume moved by the user during the duck; leaving it alone");
                } else {
                    debug!("Restoring output volume to {target:.3}");
                    // Ramp up from wherever the volume actually is now, not from
                    // the duck target: an interrupted down-ramp may have stopped
                    // short of it.
                    self.begin_volume_fade(current.unwrap_or(target), target, fade_ms);
                }
                clear_snapshot = true;
            }
        }
        *duck_guard = DuckState::default();
        drop(duck_guard);
        // Only when a volume duck was engaged: this runs on every stop and
        // several safety nets, and must not cost a disk write when idle.
        if clear_snapshot {
            self.persist_duck_snapshot(None);
        }
    }

    /// Ramps the system output volume from `from` to `to`, cancelling any fade
    /// still in flight.
    ///
    /// The generation is bumped *before* anything is written, so an older fade
    /// thread observes the mismatch and stops. A fade already past its check
    /// but not yet past its write can still land one stale intermediate value;
    /// that value is immediately overwritten by this ramp's first step, and the
    /// exact final write happens `fade_ms` later, well outside that window.
    ///
    /// `fade_ms == 0` writes the target inline — no thread, no ramp.
    fn begin_volume_fade(&self, from: f32, to: f32, fade_ms: u16) {
        let my_generation = self.fade_generation.fetch_add(1, Ordering::SeqCst) + 1;

        if fade_ms == 0 {
            write_volume_if_current(&self.fade_generation, my_generation, to);
            return;
        }

        // A dedicated OS thread, per the concurrency split in AGENTS.md: this is
        // blocking, platform-bound work (COM round-trips on Windows, process
        // spawns elsewhere) and must not occupy an async runtime worker. It is
        // also deliberately not joined — the caller (stop path) must return
        // immediately so the stop chime is not delayed by the ramp.
        let fade_generation = Arc::clone(&self.fade_generation);
        std::thread::spawn(move || {
            run_fade(fade_generation, my_generation, from, to, fade_ms);
        });
    }

    pub fn preload_vad(&self) -> Result<(), anyhow::Error> {
        let mut recorder_opt = self.recorder.lock_recover();
        if recorder_opt.is_none() {
            let vad_path = self
                .app_handle
                .path()
                .resolve(
                    "resources/models/silero_vad_v4.onnx",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|e| anyhow::anyhow!("Failed to resolve VAD path: {}", e))?;
            let settings = get_settings(&self.app_handle);
            *recorder_opt = Some(create_audio_recorder(
                &vad_path,
                &self.app_handle,
                settings.selected_channel,
                Arc::clone(&self.stream_router),
                Arc::clone(&self.azure_stream),
            )?);
        }
        Ok(())
    }

    pub fn start_microphone_stream(&self) -> Result<(), anyhow::Error> {
        let mut open_flag = self.is_open.lock_recover();
        if *open_flag {
            // `is_open` only records that we opened a stream at some point, not
            // that one is still running. If capture has since failed (mic
            // unplugged mid-session, USB dropout), rebuild it before the next
            // recording instead of handing the caller a stalled recorder.
            let needs_reopen = self
                .recorder
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|rec| rec.needs_reopen());

            if !needs_reopen {
                // trace, not debug: with the aliveness check in
                // try_start_recording this now fires on every keypress in
                // always-on mode.
                trace!("Microphone stream already active");
                return Ok(());
            }

            warn!("Microphone stream is no longer running (device disconnected?); reopening");

            // Torn down inline rather than via stop_microphone_stream(), which
            // takes the `is_open` lock we are already holding. Restoring mute
            // *and* volume matters here: the stream died mid-recording, so an
            // active duck would otherwise survive into the reopened session.
            // Safe under `is_open`: the lock order is is_open → duck_state.
            self.remove_recording_audio();
            if let Some(rec) = self.recorder.lock_recover().as_mut() {
                let _ = rec.close();
            }
            *self.is_recording.lock_recover() = false;
            *open_flag = false;
            self.invalidate_device_cache();
            // Fall through to the same fresh resolution and fallback path used
            // when an on-demand stream opens after its device was unplugged.
        }

        let start_time = Instant::now();

        // Don't attenuate immediately - caller will handle it after audio
        // feedback. The previous stream restored audio on close, so nothing
        // should be engaged here; if something somehow is, restore rather than
        // just clearing the flag, which would strand system audio muted or
        // quiet.
        self.remove_recording_audio();

        // Get the selected device from settings, considering clamshell mode.
        // No pre-flight enumeration here: when nothing is configured the
        // recorder resolves the system default itself, and a machine with no
        // input devices at all fails inside open() with the same
        // "No input device found" error this used to check for.
        let settings = get_settings(&self.app_handle);
        let resolve_started = Instant::now();
        let mut resolution = self.resolve_microphone_device(&settings);
        let resolve_elapsed = resolve_started.elapsed();

        // Ensure VAD is loaded if it wasn't for whatever reason
        let vad_started = Instant::now();
        self.preload_vad()?;
        let vad_elapsed = vad_started.elapsed();

        let open_started = Instant::now();
        let mut recorder_opt = self.recorder.lock_recover();
        if let Some(rec) = recorder_opt.as_mut() {
            if let Err(first_err) = rec.open(resolution.device.clone()) {
                // A cached device or config may have gone stale (unplugged,
                // rate/format changed). Re-resolve from a fresh enumeration and
                // retry once before surfacing the error.
                warn!("Recorder open failed ({first_err}); re-resolving device and retrying once");
                self.invalidate_device_cache();
                resolution = self.resolve_microphone_device(&settings);
                rec.open(resolution.device.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to open recorder: {}", e))?;
            }
        }
        debug!(
            "mic stream breakdown: device_resolve={:?} vad_ensure={:?} open={:?}",
            resolve_elapsed,
            vad_elapsed,
            open_started.elapsed()
        );
        drop(recorder_opt);

        *open_flag = true;
        if let Some(unavailable_name) = resolution.unavailable_selected_microphone {
            // Do this only after the default stream opened successfully. A
            // failed fallback must not erase the user's microphone preference.
            self.persist_default_microphone_after_fallback(&unavailable_name);
        }
        // This timing covers through cpal's stream.play() returning — i.e. the
        // point cpal surfaces as "stream running." It does NOT guarantee the
        // host audio device is producing samples yet; the first input callback
        // fires asynchronously one buffer period later (hardware dependent,
        // typically ~10–200ms on macOS, longer on Bluetooth/USB).
        info!(
            "Microphone stream initialized in {:?}",
            start_time.elapsed()
        );
        Ok(())
    }

    pub fn stop_microphone_stream(&self) {
        let mut open_flag = self.is_open.lock_recover();
        if !*open_flag {
            return;
        }

        // Closing the stream always hands the user's audio back, whether we
        // muted it or ducked it.
        self.remove_recording_audio();

        if let Some(rec) = self.recorder.lock_recover().as_mut() {
            // If still recording, stop first.
            if *self.is_recording.lock_recover() {
                let _ = rec.stop();
                *self.is_recording.lock_recover() = false;
            }
            let _ = rec.close();
        }

        *open_flag = false;
        debug!("Microphone stream stopped");
    }

    /* ---------- mode switching --------------------------------------------- */

    pub fn update_mode(&self, new_mode: MicrophoneMode) -> Result<(), anyhow::Error> {
        let cur_mode = self.mode.lock_recover().clone();

        match (cur_mode, &new_mode) {
            (MicrophoneMode::AlwaysOn, MicrophoneMode::OnDemand) => {
                if matches!(*self.state.lock_recover(), RecordingState::Idle) {
                    self.close_generation.fetch_add(1, Ordering::SeqCst);
                    self.stop_microphone_stream();
                }
            }
            (MicrophoneMode::OnDemand, MicrophoneMode::AlwaysOn) => {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.start_microphone_stream()?;
            }
            _ => {}
        }

        *self.mode.lock_recover() = new_mode;
        Ok(())
    }

    /* ---------- recording --------------------------------------------------- */

    /// The one place `state` is written. Derives `recording_active` (the
    /// lock-free mirror read by `is_recording()`) from the new value itself,
    /// so the two can never drift: a new `RecordingState` variant only needs
    /// its active-set membership decided here, once.
    fn set_state(&self, guard: &mut RecordingState, new_state: RecordingState) {
        *guard = new_state;
        self.recording_active.store(
            matches!(
                *guard,
                RecordingState::Recording { .. } | RecordingState::Stopping
            ),
            Ordering::SeqCst,
        );
    }

    pub fn try_start_recording(
        &self,
        binding_id: &str,
        vad_policy: VadPolicy,
    ) -> Result<RecordingReadiness, String> {
        let mut state = self.state.lock_recover();

        if let RecordingState::Idle = *state {
            // Cancel any pending lazy close (no-op in always-on mode, where
            // closes are never scheduled).
            self.close_generation.fetch_add(1, Ordering::SeqCst);
            // Opens the stream in on-demand mode. In always-on mode the stream
            // is normally already open and this is a cheap aliveness check —
            // but if the capture worker died (device disconnect), it rebuilds
            // the stream instead of leaving every subsequent start wedged on
            // "Recorder not available".
            if let Err(e) = self.start_microphone_stream() {
                let msg = format!("{e}");
                error!("Failed to open microphone stream: {msg}");
                return Err(msg);
            }

            if let Some(rec) = self.recorder.lock_recover().as_ref() {
                match rec.start(vad_policy) {
                    Ok(receiver) => {
                        let generation = self.capture_generation.fetch_add(1, Ordering::AcqRel) + 1;
                        *self.is_recording.lock_recover() = true;
                        self.set_state(
                            &mut state,
                            RecordingState::Recording {
                                binding_id: binding_id.to_string(),
                            },
                        );
                        debug!("Recording requested for binding {binding_id}");
                        return Ok(RecordingReadiness {
                            receiver,
                            generation,
                        });
                    }
                    Err(error) => return Err(format!("Failed to start recorder: {error}")),
                }
            }
            Err("Recorder not available".to_string())
        } else {
            Err("Already recording".to_string())
        }
    }

    pub fn update_selected_device(&self) -> Result<(), anyhow::Error> {
        // Device settings changed; re-enumerate the device and restart capture.
        self.invalidate_device_cache();
        let was_open = *self.is_open.lock_recover();
        if was_open {
            self.close_generation.fetch_add(1, Ordering::SeqCst);
            self.stop_microphone_stream();
            self.start_microphone_stream()?;
        }
        Ok(())
    }

    pub fn update_selected_channel(
        &self,
        selected_channel: Option<u16>,
    ) -> Result<(), anyhow::Error> {
        // Serialize against recording start/stop. Restarting an active capture
        // would discard its samples and leave the manager's recording state out
        // of sync with the new recorder.
        let state = self.state.lock_recover();
        if !matches!(*state, RecordingState::Idle) {
            return Err(anyhow::anyhow!(
                "Cannot change the input channel while recording"
            ));
        }

        let previous_channel = get_settings(&self.app_handle).selected_channel;
        let was_open = *self.is_open.lock_recover();
        if was_open {
            self.close_generation.fetch_add(1, Ordering::SeqCst);
            self.stop_microphone_stream();
        }
        if let Some(recorder) = self.recorder.lock_recover().as_mut() {
            recorder.set_selected_channel(selected_channel);
        }
        if was_open {
            if let Err(error) = self.start_microphone_stream() {
                if let Some(recorder) = self.recorder.lock_recover().as_mut() {
                    recorder.set_selected_channel(previous_channel);
                }
                return Err(error);
            }
        }
        drop(state);
        Ok(())
    }

    /// Invalidate pending first-sample UI and audio-feedback work immediately.
    /// Called at the beginning of stop, before the slower capture drain starts.
    pub fn invalidate_recording_readiness(&self) {
        self.capture_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn is_recording_readiness_current(&self, generation: u64) -> bool {
        self.capture_generation.load(Ordering::Acquire) == generation
    }

    pub fn cancel_generation(&self) -> u64 {
        self.cancel_generation.load(Ordering::Acquire)
    }

    pub fn was_cancelled_since(&self, generation: u64) -> bool {
        self.cancel_generation.load(Ordering::Acquire) != generation
    }

    pub fn stop_recording(&self, binding_id: &str, cancel_generation: u64) -> Option<Vec<f32>> {
        self.invalidate_recording_readiness();
        let mut state = self.state.lock_recover();

        match *state {
            RecordingState::Recording {
                binding_id: ref active,
            } if active == binding_id => {
                self.set_state(&mut state, RecordingState::Stopping);
                drop(state);

                // Optionally keep recording for a bit longer to capture trailing audio.
                // This is only the explicit user setting; streaming VAD must not add
                // hidden post-release capture time.
                let settings = get_settings(&self.app_handle);
                let buffer_ms = settings.extra_recording_buffer_ms;
                if buffer_ms > 0 {
                    debug!(
                        "Extra recording buffer: sleeping {}ms before stopping",
                        buffer_ms
                    );
                    let started = Instant::now();
                    let buffer = Duration::from_millis(buffer_ms);
                    while started.elapsed() < buffer {
                        if self.was_cancelled_since(cancel_generation) {
                            debug!("Recording stop cancelled during extra buffer");
                            break;
                        }
                        let remaining = buffer.saturating_sub(started.elapsed());
                        std::thread::sleep(remaining.min(Duration::from_millis(25)));
                    }
                }

                let samples = if let Some(rec) = self.recorder.lock_recover().as_ref() {
                    match rec.stop() {
                        Ok(buf) => buf,
                        Err(e) => {
                            error!("stop() failed: {e}");
                            Vec::new()
                        }
                    }
                } else {
                    error!("Recorder not available");
                    Vec::new()
                };

                *self.is_recording.lock_recover() = false;
                self.set_state(&mut self.state.lock_recover(), RecordingState::Idle);

                // Defensive restore: the normal stop path releases via
                // `remove_recording_audio` in actions.rs before the stop chime,
                // but a stop reached any other way (or a chime-less path) must
                // not depend on that. A no-op once the release already ran.
                self.remove_recording_audio();

                // In on-demand mode, close the mic (lazily if the setting is enabled)
                if matches!(*self.mode.lock_recover(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }

                if self.was_cancelled_since(cancel_generation) {
                    debug!("Recording stop cancelled; discarding captured samples");
                    return None;
                }

                // Pad if very short
                let s_len = samples.len();
                // debug!("Got {} samples", s_len);
                if s_len < WHISPER_SAMPLE_RATE && s_len > 0 {
                    let mut padded = samples;
                    padded.resize(WHISPER_SAMPLE_RATE * 5 / 4, 0.0);
                    Some(padded)
                } else {
                    Some(samples)
                }
            }
            _ => None,
        }
    }
    pub fn is_recording(&self) -> bool {
        // Lock-free: mirrors the `state` {Recording, Stopping} membership via
        // an atomic maintained by `set_state()`. Polled from the webview/main
        // thread, so it MUST NOT take the `state` mutex (a worker can hold it
        // across a slow CoreAudio open/close → main-thread deadlock / UI
        // freeze).
        self.recording_active.load(Ordering::SeqCst)
    }

    /// Cancel any ongoing recording without returning audio samples
    pub fn cancel_recording(&self) {
        self.invalidate_recording_readiness();
        self.cancel_generation.fetch_add(1, Ordering::AcqRel);
        let mut state = self.state.lock_recover();

        match *state {
            RecordingState::Recording { .. } => {
                self.set_state(&mut state, RecordingState::Idle);
                drop(state);

                if let Some(rec) = self.recorder.lock_recover().as_ref() {
                    let _ = rec.stop(); // Discard the result
                }

                *self.is_recording.lock_recover() = false;

                // Restore explicitly, before deciding what to do with the
                // stream. Cancelling used to restore only as a side effect of
                // `stop_microphone_stream()`, which does not run in always-on
                // mode and is deferred by `lazy_stream_close` — so a cancelled
                // recording could leave the system muted for up to 30s, or
                // indefinitely. With ducking that would be a permanently quiet
                // machine.
                self.remove_recording_audio();

                // In on-demand mode, close the mic (lazily if the setting is enabled)
                if matches!(*self.mode.lock_recover(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }
            }
            RecordingState::Stopping => {
                debug!("Cancellation requested while recording is stopping");
            }
            RecordingState::Idle => {}
        }
    }
}

#[cfg(test)]
mod ducking_tests {
    use super::*;

    /// The plan the engage path takes with a healthy, non-meeting stream.
    fn plan(mode: RecordingAudioMode, engaged: bool) -> DuckPlan {
        plan_recording_audio(mode, 20, engaged, true, false)
    }

    /// The user turning the volume down while it is ducked is an explicit
    /// choice; the restore must not ramp back over it.
    #[test]
    fn a_volume_moved_by_the_user_abandons_the_restore() {
        assert!(user_moved_volume(Some(0.10), Some(0.20)));
        // One keyboard volume step on Windows is 0.02: that is a user action.
        assert!(user_moved_volume(Some(0.22), Some(0.20)));
        // Backend rounding drift is around 0.01: not a user action.
        assert!(!user_moved_volume(Some(0.207), Some(0.20)));
        // Unknown on either side never accuses the user.
        assert!(!user_moved_volume(None, Some(0.20)));
        assert!(!user_moved_volume(Some(0.20), None));
    }

    #[test]
    fn plan_restore_carries_the_duck_target() {
        let state = DuckState {
            engaged: true,
            prev_muted: None,
            prev_volume: Some(0.8),
            fade_ms: 120,
            duck_target: Some(0.2),
        };
        match plan_restore(&state) {
            RestorePlan::Volume {
                target,
                duck_target,
                ..
            } => {
                assert_eq!(target, 0.8);
                assert_eq!(duck_target, Some(0.2));
            }
            _ => panic!("expected a volume restore"),
        }
    }

    /// Ramps shorter than their step count land directly instead of firing
    /// ten platform writes back to back.
    #[test]
    fn short_ramps_skip_the_step_schedule() {
        assert!(!ramp_has_steps(0));
        assert!(!ramp_has_steps(9));
        assert!(ramp_has_steps(10));
        assert!(ramp_has_steps(120));
    }

    /// A superseded ramp must not write, and the refusal happens under the
    /// same lock as the write, so nothing can slip in between.
    #[test]
    fn a_superseded_write_is_refused_under_the_lock() {
        let generation = AtomicU64::new(5);
        assert!(!write_volume_if_current(&generation, 4, 0.5));
    }

    #[test]
    fn off_never_touches_system_audio() {
        assert_eq!(plan(RecordingAudioMode::Off, false), DuckPlan::Skip);
    }

    #[test]
    fn mute_and_duck_engage_their_own_plans() {
        assert_eq!(plan(RecordingAudioMode::Mute, false), DuckPlan::Mute);
        assert_eq!(
            plan(RecordingAudioMode::Duck, false),
            DuckPlan::Duck { target: 0.2 }
        );
    }

    /// Idempotency: a duplicate or late apply must not re-snapshot, or the
    /// second snapshot would capture our own forced state and the restore
    /// would put the user back to "muted"/"quiet".
    #[test]
    fn a_second_apply_is_skipped_for_every_mode() {
        for mode in [
            RecordingAudioMode::Off,
            RecordingAudioMode::Duck,
            RecordingAudioMode::Mute,
        ] {
            assert_eq!(plan(mode, true), DuckPlan::Skip, "mode {mode:?}");
        }
    }

    #[test]
    fn nothing_is_attenuated_while_the_stream_is_closed() {
        assert_eq!(
            plan_recording_audio(RecordingAudioMode::Mute, 20, false, false, false),
            DuckPlan::Skip
        );
        assert_eq!(
            plan_recording_audio(RecordingAudioMode::Duck, 20, false, false, false),
            DuckPlan::Skip
        );
    }

    /// Attenuating the render endpoint would attenuate the meeting's own
    /// system-loopback recording, not just what the user hears.
    #[test]
    fn an_active_meeting_suppresses_both_attenuation_modes() {
        assert_eq!(
            plan_recording_audio(RecordingAudioMode::Mute, 20, false, true, true),
            DuckPlan::Skip
        );
        assert_eq!(
            plan_recording_audio(RecordingAudioMode::Duck, 20, false, true, true),
            DuckPlan::Skip
        );
    }

    #[test]
    fn duck_target_is_scaled_and_defensively_clamped() {
        assert_eq!(duck_target_scalar(0), 0.0);
        assert_eq!(duck_target_scalar(20), 0.2);
        assert_eq!(duck_target_scalar(100), 1.0);
        // A hand-edited store must not be able to ask for 255%.
        assert_eq!(duck_target_scalar(255), 1.0);
    }

    /// Every safety-net call site fires unconditionally, so restoring when
    /// nothing was engaged has to be a genuine no-op.
    #[test]
    fn restoring_when_nothing_is_engaged_is_a_no_op() {
        assert_eq!(plan_restore(&DuckState::default()), RestorePlan::Nothing);

        // Even with stale snapshots left over, `engaged` is the only authority.
        let stale = DuckState {
            engaged: false,
            prev_muted: Some(false),
            prev_volume: Some(0.75),
            fade_ms: 120,
            duck_target: None,
        };
        assert_eq!(plan_restore(&stale), RestorePlan::Nothing);
    }

    #[test]
    fn the_populated_snapshot_selects_what_gets_restored() {
        let muted = DuckState {
            engaged: true,
            prev_muted: Some(true),
            prev_volume: None,
            fade_ms: 0,
            duck_target: None,
        };
        assert_eq!(plan_restore(&muted), RestorePlan::Mute(Some(true)));

        // An unknown prior mute state still restores (as "unmute"), so we never
        // strand the user muted.
        let unknown = DuckState {
            engaged: true,
            prev_muted: None,
            prev_volume: None,
            fade_ms: 0,
            duck_target: None,
        };
        assert_eq!(plan_restore(&unknown), RestorePlan::Mute(None));

        let ducked = DuckState {
            engaged: true,
            prev_muted: None,
            prev_volume: Some(0.63),
            fade_ms: 120,
            duck_target: None,
        };
        assert_eq!(
            plan_restore(&ducked),
            RestorePlan::Volume {
                target: 0.63,
                fade_ms: 120,
                duck_target: None
            }
        );
    }

    /// The ramp length is snapshotted at engage time, so moving the slider
    /// mid-recording cannot change the ramp that is already half-run — and the
    /// restore path never has to read settings under the duck lock.
    #[test]
    fn the_restore_reuses_the_ramp_length_captured_at_engage_time() {
        let ducked = DuckState {
            engaged: true,
            prev_muted: None,
            prev_volume: Some(0.5),
            fade_ms: 350,
            duck_target: None,
        };
        match plan_restore(&ducked) {
            RestorePlan::Volume { fade_ms, .. } => assert_eq!(fade_ms, 350),
            other => panic!("expected a volume restore, got {other:?}"),
        }
    }

    /// The restore must land on the snapshot verbatim. Interpolating the last
    /// step instead would leave a drifted volume on the user's system.
    #[test]
    fn a_restore_plan_carries_the_exact_snapshot_value() {
        // A value that is not representable as a round percentage: it must
        // survive the plan untouched.
        let exact = 0.123_456_79_f32;
        let ducked = DuckState {
            engaged: true,
            prev_muted: None,
            prev_volume: Some(exact),
            fade_ms: 120,
            duck_target: None,
        };
        match plan_restore(&ducked) {
            RestorePlan::Volume { target, .. } => assert_eq!(target.to_bits(), exact.to_bits()),
            other => panic!("expected a volume restore, got {other:?}"),
        }
    }

    /// A superseded fade must abort instead of continuing to write, which is
    /// what keeps a down-ramp from crossing the up-ramp that replaced it.
    #[test]
    fn a_superseded_fade_stops_before_writing_its_target() {
        let generation = Arc::new(AtomicU64::new(7));
        // Spawned with a stale generation: every checkpoint mismatches, so the
        // ramp returns without ever reaching set_output_volume.
        let started = Instant::now();
        run_fade(Arc::clone(&generation), 6, 1.0, 0.0, 100);
        assert!(
            started.elapsed() < Duration::from_millis(90),
            "an aborted fade must not run its full duration"
        );
        assert_eq!(generation.load(Ordering::SeqCst), 7);
    }
}
