//! System output-volume access for "duck while recording".
//!
//! This is the platform layer only: read the master output volume, write the
//! master output volume. Everything policy-shaped (when to duck, by how much,
//! how to fade, how to restore) lives in [`crate::managers::audio`].
//!
//! The backends mirror the mute plumbing in `managers/audio.rs` one for one —
//! Windows via `IAudioEndpointVolume`, Linux via `wpctl` → `pactl` → `amixer`,
//! macOS via `osascript` — so a machine where muting works also ducks, and a
//! machine where it does not degrades the same way: every failure is swallowed
//! and reported as `None` / a no-op rather than surfaced to the user. A missing
//! CLI tool must never break dictation.

use log::debug;

/// Reads the current system output volume as a scalar in `0.0..=1.0`.
///
/// Returns `None` when the volume could not be determined (unsupported
/// platform, missing CLI tool, parse failure). Callers must treat `None` as
/// "do not duck" — without a trustworthy snapshot we cannot restore, and
/// leaving a user's system permanently quiet is far worse than not ducking.
pub fn get_output_volume() -> Option<f32> {
    let volume = get_output_volume_impl()?;
    if !volume.is_finite() {
        return None;
    }
    Some(volume.clamp(0.0, 1.0))
}

/// Sets the system output volume. `v` is clamped to `0.0..=1.0`; non-finite
/// input is ignored. Failures are swallowed and logged at debug level.
pub fn set_output_volume(v: f32) {
    if !v.is_finite() {
        debug!("Ignoring non-finite output volume request");
        return;
    }
    set_output_volume_impl(v.clamp(0.0, 1.0));
}

/* ---------- Windows ------------------------------------------------------ */

#[cfg(target_os = "windows")]
fn get_output_volume_impl() -> Option<f32> {
    unsafe {
        use windows::Win32::{
            Media::Audio::{
                eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                MMDeviceEnumerator,
            },
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
        };

        // Matches set_mute/get_mute: a no-op when COM is already initialized on
        // this thread (Tauri or a fade thread may have got there first).
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let all_devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let default_device = all_devices
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?;
        let volume_interface = default_device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .ok()?;

        volume_interface.GetMasterVolumeLevelScalar().ok()
    }
}

#[cfg(target_os = "windows")]
fn set_output_volume_impl(v: f32) {
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

        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let all_devices: IMMDeviceEnumerator =
            unwrap_or_return!(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL));
        let default_device =
            unwrap_or_return!(all_devices.GetDefaultAudioEndpoint(eRender, eMultimedia));
        let volume_interface =
            unwrap_or_return!(default_device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None));

        let _ = volume_interface.SetMasterVolumeLevelScalar(v, std::ptr::null());
    }
}

/* ---------- Linux -------------------------------------------------------- */

#[cfg(target_os = "linux")]
fn get_output_volume_impl() -> Option<f32> {
    use std::process::Command;

    // Every read forces LC_ALL=C: a localized system would otherwise emit a
    // comma decimal separator ("Volume: 0,40") that `f32::from_str` rejects.
    // Same reasoning as `get_mute` in managers/audio.rs.

    // 1. PipeWire (wpctl): "Volume: 0.40" (plus " [MUTED]" when muted).
    if let Ok(out) = Command::new("wpctl")
        .env("LC_ALL", "C")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
    {
        if out.status.success() {
            if let Some(v) = parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout)) {
                return Some(v);
            }
        }
    }

    // 2. PulseAudio (pactl): "Volume: front-left: 26214 /  40% / -23.88 dB, ..."
    if let Ok(out) = Command::new("pactl")
        .env("LC_ALL", "C")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
    {
        if out.status.success() {
            if let Some(v) = parse_pactl_volume(&String::from_utf8_lossy(&out.stdout)) {
                return Some(v);
            }
        }
    }

    // 3. ALSA (amixer): "Front Left: Playback 42 [42%] [-15.00dB] [on]".
    if let Ok(out) = Command::new("amixer")
        .env("LC_ALL", "C")
        .args(["get", "Master"])
        .output()
    {
        if out.status.success() {
            if let Some(v) = parse_amixer_volume(&String::from_utf8_lossy(&out.stdout)) {
                return Some(v);
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn set_output_volume_impl(v: f32) {
    use std::process::Command;

    let percent = scalar_to_percent(v);

    // 1. PipeWire (wpctl) takes the 0.0..=1.0 scalar directly.
    if Command::new("wpctl")
        .env("LC_ALL", "C")
        .args([
            "set-volume",
            "@DEFAULT_AUDIO_SINK@",
            &format!("{:.3}", v.clamp(0.0, 1.0)),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    // 2. PulseAudio (pactl).
    if Command::new("pactl")
        .env("LC_ALL", "C")
        .args([
            "set-sink-volume",
            "@DEFAULT_SINK@",
            &format!("{}%", percent),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    // 3. ALSA (amixer).
    let _ = Command::new("amixer")
        .env("LC_ALL", "C")
        .args(["set", "Master", &format!("{}%", percent)])
        .output();
}

/* ---------- macOS -------------------------------------------------------- */

#[cfg(target_os = "macos")]
fn get_output_volume_impl() -> Option<f32> {
    use std::process::Command;

    let out = Command::new("osascript")
        .args(["-e", "output volume of (get volume settings)"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_percent_scalar(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(target_os = "macos")]
fn set_output_volume_impl(v: f32) {
    use std::process::Command;

    let script = format!("set volume output volume {}", scalar_to_percent(v));
    let _ = Command::new("osascript").args(["-e", &script]).output();
}

/* ---------- unsupported targets ------------------------------------------ */

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn get_output_volume_impl() -> Option<f32> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn set_output_volume_impl(v: f32) {
    let _ = v;
}

/* ---------- pure parsing helpers ----------------------------------------- */
//
// Kept free of `cfg` so they compile — and are unit-tested — on every platform,
// not just the one that happens to call them. `#[allow(dead_code)]` because
// only the Linux/macOS backends consume them at runtime.

/// `0.0..=1.0` → the integer percentage the CLI tools expect.
#[allow(dead_code)]
fn scalar_to_percent(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Parses a bare integer/float percentage (0..=100), as printed by
/// `osascript -e "output volume of (get volume settings)"`.
#[allow(dead_code)]
fn parse_percent_scalar(stdout: &str) -> Option<f32> {
    let value: f32 = stdout.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((value / 100.0).clamp(0.0, 1.0))
}

/// Parses `wpctl get-volume @DEFAULT_AUDIO_SINK@` output.
///
/// Example: `Volume: 0.40` — or `Volume: 0.40 [MUTED]` when the sink is muted.
/// The mute flag is deliberately ignored: it is `get_mute`'s job, and a muted
/// sink still has a volume worth snapshotting.
#[allow(dead_code)]
fn parse_wpctl_volume(stdout: &str) -> Option<f32> {
    let rest = stdout.split("Volume:").nth(1)?;
    let token = rest.split_whitespace().next()?;
    let value: f32 = token.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(value.clamp(0.0, 1.0))
}

/// Parses `pactl get-sink-volume @DEFAULT_SINK@` output by taking the first
/// per-channel percentage.
///
/// Example: `Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: ...`
/// Only the first channel is used — we restore a single master scalar, so a
/// per-channel imbalance is not something this layer can represent anyway.
#[allow(dead_code)]
fn parse_pactl_volume(stdout: &str) -> Option<f32> {
    for token in stdout.split_whitespace() {
        let Some(number) = token.trim_end_matches(',').strip_suffix('%') else {
            continue;
        };
        if let Ok(percent) = number.parse::<f32>() {
            if percent.is_finite() {
                return Some((percent / 100.0).clamp(0.0, 1.0));
            }
        }
    }
    None
}

/// Parses `amixer get Master` output by taking the first `[42%]` token.
#[allow(dead_code)]
fn parse_amixer_volume(stdout: &str) -> Option<f32> {
    let mut rest = stdout;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        // An unterminated bracket means truncated output: give up rather than
        // slice past the end.
        let close = after.find(']')?;
        let inner = &after[..close];
        rest = &after[close + 1..];

        if let Some(number) = inner.strip_suffix('%') {
            if let Ok(percent) = number.parse::<f32>() {
                if percent.is_finite() {
                    return Some((percent / 100.0).clamp(0.0, 1.0));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Option<f32>, expected: f32) {
        let a = a.expect("expected a parsed volume");
        assert!((a - expected).abs() < 1e-4, "expected ~{expected}, got {a}");
    }

    #[test]
    fn wpctl_volume_is_parsed() {
        approx(parse_wpctl_volume("Volume: 0.40\n"), 0.40);
        approx(parse_wpctl_volume("Volume: 1.00\n"), 1.0);
        approx(parse_wpctl_volume("Volume: 0.00\n"), 0.0);
    }

    #[test]
    fn wpctl_muted_marker_does_not_break_parsing() {
        approx(parse_wpctl_volume("Volume: 0.65 [MUTED]\n"), 0.65);
    }

    #[test]
    fn wpctl_overdrive_is_clamped_to_one() {
        // wpctl happily reports >100% when the sink is boosted; we only ever
        // restore into 0..=1, so clamp on the way in too.
        approx(parse_wpctl_volume("Volume: 1.40\n"), 1.0);
    }

    #[test]
    fn wpctl_garbage_is_rejected() {
        assert_eq!(parse_wpctl_volume(""), None);
        assert_eq!(parse_wpctl_volume("Volume: loud\n"), None);
        assert_eq!(parse_wpctl_volume("no volume line here"), None);
    }

    #[test]
    fn pactl_first_channel_percentage_is_used() {
        let out = "Volume: front-left: 26214 /  40% / -23.88 dB,   \
                   front-right: 26214 /  40% / -23.88 dB\n";
        approx(parse_pactl_volume(out), 0.40);
    }

    #[test]
    fn pactl_handles_single_channel_and_zero() {
        approx(
            parse_pactl_volume("Volume: mono: 0 /   0% / -inf dB\n"),
            0.0,
        );
        approx(
            parse_pactl_volume("Volume: mono: 65536 / 100% / 0.00 dB\n"),
            1.0,
        );
    }

    #[test]
    fn pactl_garbage_is_rejected() {
        assert_eq!(parse_pactl_volume(""), None);
        assert_eq!(parse_pactl_volume("Volume: front-left: 26214\n"), None);
    }

    #[test]
    fn amixer_first_percentage_token_is_used() {
        let out = "Simple mixer control 'Master',0\n  \
                   Capabilities: pvolume pswitch\n  \
                   Front Left: Playback 42 [42%] [-15.00dB] [on]\n  \
                   Front Right: Playback 42 [42%] [-15.00dB] [on]\n";
        approx(parse_amixer_volume(out), 0.42);
    }

    #[test]
    fn amixer_skips_non_percentage_brackets() {
        // The dB and on/off brackets must not be mistaken for the volume.
        approx(
            parse_amixer_volume("Mono: Playback 0 [off] [0%] [-60.00dB]\n"),
            0.0,
        );
    }

    #[test]
    fn amixer_garbage_is_rejected() {
        assert_eq!(parse_amixer_volume(""), None);
        assert_eq!(parse_amixer_volume("Front Left: Playback 42 [on]"), None);
        // Unterminated bracket must not panic or slice out of bounds.
        assert_eq!(parse_amixer_volume("Front Left: [42"), None);
    }

    #[test]
    fn macos_percent_scalar_is_parsed() {
        approx(parse_percent_scalar("50\n"), 0.5);
        approx(parse_percent_scalar("  0  "), 0.0);
        approx(parse_percent_scalar("100"), 1.0);
        assert_eq!(parse_percent_scalar("missing value"), None);
        assert_eq!(parse_percent_scalar(""), None);
    }

    #[test]
    fn scalar_to_percent_rounds_and_clamps() {
        assert_eq!(scalar_to_percent(0.0), 0);
        assert_eq!(scalar_to_percent(0.204), 20);
        assert_eq!(scalar_to_percent(0.205), 21);
        assert_eq!(scalar_to_percent(1.0), 100);
        assert_eq!(scalar_to_percent(2.5), 100);
        assert_eq!(scalar_to_percent(-1.0), 0);
    }

    #[test]
    fn public_setter_ignores_non_finite_input() {
        // Must not panic; on a headless CI box the backend is a no-op anyway.
        set_output_volume(f32::NAN);
        set_output_volume(f32::INFINITY);
    }
}
