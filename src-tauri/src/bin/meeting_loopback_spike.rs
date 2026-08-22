//! Meeting-mode Phase 1 loopback spike harness (see docs/meeting-mode-plan.md §7).
//!
//! # What this is
//!
//! A standalone, separately reviewable measurement binary. It opens the selected
//! Windows output device as a CPAL *input* stream (WASAPI loopback, enabled
//! transparently by cpal 0.16 for render devices) and the default microphone
//! simultaneously, and logs per-callback timing/level measurements to a local CSV
//! file. It exists to answer, on real target hardware, before the MeetingRecorder
//! is built:
//!
//! - do default and manually selected output devices deliver loopback audio;
//! - what format, channel count and native sample rate does loopback deliver;
//! - what happens during genuine silence (packets, gaps, levels);
//! - how large are callback gaps and clock drift between the two sources;
//! - can both sources run together for hours.
//!
//! This binary deliberately shares no code path with the dictation recorder. It
//! only reuses device listing from `audio_toolkit`. It never writes audio to disk
//! and never touches recorder/dictation code.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin meeting_loopback_spike -- --list-devices
//! cargo run --release --bin meeting_loopback_spike -- --duration 30
//! cargo run --release --bin meeting_loopback_spike -- \
//!     --duration 7200 --output-device "Speakers" \
//!     --output-log spike_120min.csv
//! ```
//!
//! Arguments:
//!
//! - `--list-devices`          print input and output devices (with indices and
//!                             the default flag) and exit.
//! - `--duration <SECONDS>`    bounded capture duration; default 7200 (120 min),
//!                             hard-capped at 86400 (24 h).
//! - `--output-log <PATH>`     CSV log path; default
//!                             `meeting_loopback_spike_<YYYYMMDD_HHMMSS>.csv`.
//! - `--output-device <X>`     output (loopback) device by index or name
//!                             substring; default = system default output device.
//! - `--input-device <X>`      microphone by index or name substring; default =
//!                             system default input device.
//! - `--silence-threshold <F>` peak amplitude below which a callback counts as
//!                             silent; default 0.0001.
//!
//! # How to run the 120-minute measurement
//!
//! 1. `cargo run --release --bin meeting_loopback_spike -- --list-devices`
//!    and note the index/name of the intended meeting output device.
//! 2. Start the run with that device selected explicitly (the case the plan
//!    cares about most):
//!    `cargo run --release --bin meeting_loopback_spike -- --duration 7200 --output-device <NAME>`
//! 3. During the run: play real meeting audio over that output device, speak
//!    into the microphone, and at some point exercise the plan's scenarios —
//!    pure silence (no playback), headphone connect/disconnect, default-device
//!    change, and sleep/wake.
//! 4. Let the run end on its own (bounded duration) so the final summary rows
//!    are written. Ctrl+C terminates immediately and skips the summary.
//! 5. Evaluate the CSV: look at the `gap_class` distribution, the
//!    `drift_frames` trend (clock divergence between the two sources),
//!    loopback `silent` behavior during genuine silence, and any `error` rows.
//!
//! # Log format
//!
//! Lines starting with `#` are run metadata. Data rows are CSV with columns:
//!
//! | Column            | Meaning                                                             |
//! |-------------------|---------------------------------------------------------------------|
//! | `source`          | `microphone` or `loopback`                                          |
//! | `cb_index`        | 1-based callback counter for this source                            |
//! | `t_rel_ms`        | monotonic callback arrival time, ms since both streams started      |
//! | `gap_ms`          | ms since previous callback of the same source (first row = t_rel)   |
//! | `frames`          | frames (per-channel samples) delivered in this callback             |
//! | `total_frames`    | cumulative frames for this source                                   |
//! | `sample_rate`     | configured stream sample rate (Hz)                                  |
//! | `channels`        | configured channel count                                            |
//! | `sample_format`   | configured sample format (f32, i16, ...)                            |
//! | `expected_frames` | sample_rate × elapsed_s — frames a gap-free stream would have sent  |
//! | `drift_frames`    | total_frames − expected_frames (negative = slow clock)              |
//! | `gap_class`       | `normal` (≤250 ms), `sparse` (≤1 s), `gap` (>1 s)                   |
//! | `silent`          | `true` when peak amplitude < silence threshold                      |
//! | `peak`            | peak absolute mono sample in this callback                          |
//! | `rms`             | RMS of the mono mix in this callback                                |
//! | `error`           | stream error message, only on `error` rows (all other columns zero) |
//!
//! The final `# SUMMARY` block per source reports callback count, total frames,
//! wall duration, drift at end, max gap, gap-class histogram, silent fraction
//! and stream error count.
//!
//! # Tests
//!
//! Timestamp/gap/drift math is pure and unit tested:
//! `cargo test --bin meeting_loopback_spike`

use std::fs::File;
use std::io::{LineWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;

use speak_app_lib::audio_toolkit::{list_input_devices, list_output_devices, CpalDeviceInfo};

/// Gap classification buckets. Spike constants, documented here so measured
/// target-hardware values can later be promoted into plan §7 tolerances.
const GAP_NORMAL_MAX_S: f64 = 0.25;
const GAP_SPARSE_MAX_S: f64 = 1.0;
/// Hard upper bound for `--duration` (24 h) to keep the run bounded.
const MAX_DURATION_S: u64 = 86_400;

#[derive(Parser, Debug)]
#[command(
    name = "meeting_loopback_spike",
    about = "Meeting-mode Phase 1: WASAPI loopback + microphone measurement spike"
)]
struct Cli {
    /// List input and output devices, then exit.
    #[arg(long)]
    list_devices: bool,
    /// Bounded capture duration in seconds (default: 120 minutes).
    #[arg(long, default_value_t = 7200)]
    duration: u64,
    /// CSV log file path.
    #[arg(long)]
    output_log: Option<PathBuf>,
    /// Output (loopback) device index or name substring; default = default output.
    #[arg(long)]
    output_device: Option<String>,
    /// Microphone index or name substring; default = default input.
    #[arg(long)]
    input_device: Option<String>,
    /// Peak amplitude below which a callback counts as silent.
    #[arg(long, default_value_t = 1e-4)]
    silence_threshold: f32,
}

/// Pure per-source timeline math. `t_rel_s` is a monotonic timestamp in seconds
/// from a clock shared by both sources; frames are per-channel sample counts.
struct SourceTimeline {
    sample_rate: u32,
    total_frames: u64,
    callbacks: u64,
    prev_t_rel_s: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct CallbackMetrics {
    callback_index: u64,
    /// Seconds since the previous callback of this source; for the first
    /// callback this equals the callback's own timestamp (startup latency).
    gap_s: f64,
    /// sample_rate × t_rel_s — frames a gap-free stream would have delivered.
    expected_frames: f64,
    /// total_frames − expected_frames; negative means the source clock lags.
    drift_frames: f64,
    total_frames: u64,
}

impl SourceTimeline {
    fn new(sample_rate: u32) -> Self {
        SourceTimeline {
            sample_rate,
            total_frames: 0,
            callbacks: 0,
            prev_t_rel_s: None,
        }
    }

    fn record(&mut self, t_rel_s: f64, frames: u64) -> CallbackMetrics {
        let gap_s = match self.prev_t_rel_s {
            Some(prev) => (t_rel_s - prev).max(0.0),
            None => t_rel_s.max(0.0),
        };
        self.prev_t_rel_s = Some(t_rel_s);
        self.total_frames += frames;
        self.callbacks += 1;
        let expected_frames = expected_frames(t_rel_s, self.sample_rate);
        CallbackMetrics {
            callback_index: self.callbacks,
            gap_s,
            expected_frames,
            drift_frames: drift_frames(self.total_frames, expected_frames),
            total_frames: self.total_frames,
        }
    }
}

fn expected_frames(t_rel_s: f64, sample_rate: u32) -> f64 {
    if t_rel_s <= 0.0 {
        0.0
    } else {
        t_rel_s * f64::from(sample_rate)
    }
}

fn drift_frames(total_frames: u64, expected_frames: f64) -> f64 {
    total_frames as f64 - expected_frames
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GapClass {
    Normal,
    Sparse,
    Gap,
}

impl GapClass {
    fn as_str(self) -> &'static str {
        match self {
            GapClass::Normal => "normal",
            GapClass::Sparse => "sparse",
            GapClass::Gap => "gap",
        }
    }
}

fn classify_gap(gap_s: f64) -> GapClass {
    if gap_s <= GAP_NORMAL_MAX_S {
        GapClass::Normal
    } else if gap_s <= GAP_SPARSE_MAX_S {
        GapClass::Sparse
    } else {
        GapClass::Gap
    }
}

fn is_silent(peak: f32, threshold: f32) -> bool {
    peak.abs() < threshold
}

/// Running aggregates written to the `# SUMMARY` block at the end of the run.
#[derive(Default)]
struct SourceSummary {
    callbacks: u64,
    total_frames: u64,
    max_gap_s: f64,
    normal_gaps: u64,
    sparse_gaps: u64,
    long_gaps: u64,
    silent_callbacks: u64,
    stream_errors: u64,
    wall_s: f64,
    last_drift_frames: f64,
}

impl SourceSummary {
    fn observe(&mut self, metrics: &CallbackMetrics, silent: bool) {
        self.callbacks = metrics.callback_index;
        self.total_frames = metrics.total_frames;
        self.max_gap_s = self.max_gap_s.max(metrics.gap_s);
        match classify_gap(metrics.gap_s) {
            GapClass::Normal => self.normal_gaps += 1,
            GapClass::Sparse => self.sparse_gaps += 1,
            GapClass::Gap => self.long_gaps += 1,
        }
        if silent {
            self.silent_callbacks += 1;
        }
        // Wall time accumulates inter-arrival gaps, so it tracks the span the
        // source was actually delivering (independent of the shared clock).
        self.wall_s += metrics.gap_s;
        self.last_drift_frames = metrics.drift_frames;
    }
}

/// Minimal CSV writer; line-buffered so a hard crash loses at most one row.
struct CsvLog {
    writer: LineWriter<File>,
}

impl CsvLog {
    fn create(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(CsvLog {
            writer: LineWriter::new(File::create(path)?),
        })
    }

    fn meta_line(&mut self, line: &str) {
        let _ = writeln!(self.writer, "# {line}");
    }

    fn row(&mut self, fields: &[String]) {
        let quoted: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        let _ = writeln!(self.writer, "{}", quoted.join(","));
    }

    fn flush(&mut self) {
        let _ = self.writer.flush();
    }
}

fn csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Lightweight, testable device descriptor (cpal devices cannot be constructed
/// in unit tests).
#[derive(Debug, Clone, PartialEq)]
struct DeviceEntry {
    name: String,
    is_default: bool,
}

fn resolve_device(
    selector: Option<&str>,
    mut devices: Vec<DeviceEntry>,
    kind: &str,
) -> Result<DeviceEntry, Box<dyn std::error::Error>> {
    match selector {
        None => devices
            .iter()
            .position(|d| d.is_default)
            .or(if devices.is_empty() { None } else { Some(0) })
            .map(|i| devices.swap_remove(i))
            .ok_or_else(|| format!("no {kind} device found").into()),
        Some(sel) => {
            if let Ok(index) = sel.parse::<usize>() {
                return devices
                    .into_iter()
                    .nth(index)
                    .ok_or_else(|| format!("{kind} device index {index} out of range").into());
            }
            let needle = sel.to_lowercase();
            devices
                .into_iter()
                .find(|d| d.name.to_lowercase().contains(&needle))
                .ok_or_else(|| format!("no {kind} device matching '{sel}'").into())
        }
    }
}

fn device_entries(infos: Vec<CpalDeviceInfo>) -> Vec<DeviceEntry> {
    infos
        .into_iter()
        .map(|d| DeviceEntry {
            name: d.name,
            is_default: d.is_default,
        })
        .collect()
}

fn find_device(
    selector: Option<&str>,
    infos: Vec<CpalDeviceInfo>,
    kind: &str,
) -> Result<(cpal::Device, String), Box<dyn std::error::Error>> {
    let chosen = resolve_device(selector, device_entries(infos), kind)?;
    let chosen_name = chosen.name.clone();
    // Re-list to obtain the cpal handle for the chosen name (device listing is
    // the only audio_toolkit surface the spike reuses).
    let host = speak_app_lib::audio_toolkit::get_cpal_host();
    let mut iter = if kind == "output" {
        host.output_devices()?
    } else {
        host.input_devices()?
    };
    iter.find(|d| d.name().map(|n| n == chosen_name).unwrap_or(false))
        .map(|d| (d, chosen_name))
        .ok_or_else(|| {
            format!(
                "device '{}' disappeared while opening ({kind})",
                chosen.name
            )
            .into()
        })
}

fn print_devices() -> Result<(), Box<dyn std::error::Error>> {
    println!("Input devices (microphones):");
    for d in list_input_devices()? {
        println!(
            "  [{}] {}{}",
            d.index,
            d.name,
            if d.is_default { "  (default)" } else { "" }
        );
    }
    println!("Output devices (loopback candidates):");
    for d in list_output_devices()? {
        println!(
            "  [{}] {}{}",
            d.index,
            d.name,
            if d.is_default { "  (default)" } else { "" }
        );
    }
    Ok(())
}

type LogHandle = Arc<Mutex<CsvLog>>;

/// Everything needed to keep one capture source alive and summarize it later.
struct SourceHandles {
    _stream: cpal::Stream,
    summary: Arc<Mutex<SourceSummary>>,
    errored: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
    sample_format: String,
    device_name: String,
}

/// Builds and starts one capture source. Works for real capture devices
/// (microphone) and, on WASAPI, for render devices used as loopback inputs —
/// cpal 0.16 sets AUDCLNT_STREAMFLAGS_LOOPBACK automatically when the device
/// data flow is eRender. The stream config must come from the matching
/// direction (`default_input_config` for capture, `default_output_config` for
/// loopback).
fn open_source(
    device: cpal::Device,
    device_name: &str,
    is_loopback: bool,
    start: Instant,
    silence_threshold: f32,
    log: &LogHandle,
) -> Result<SourceHandles, Box<dyn std::error::Error>> {
    // Render devices expose their mix format only via default_output_config;
    // real capture devices via default_input_config.
    let config = if is_loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let sample_rate = config.sample_rate().0;
    let channels = usize::from(config.channels().max(1));
    let channel_count = config.channels();
    let sample_format = config.sample_format().to_string();
    let source: &'static str = if is_loopback {
        "loopback"
    } else {
        "microphone"
    };

    let timeline = Arc::new(Mutex::new(SourceTimeline::new(sample_rate)));
    let summary = Arc::new(Mutex::new(SourceSummary::default()));
    let errored = Arc::new(AtomicBool::new(false));

    macro_rules! build {
        ($ty:ty) => {{
            let timeline = Arc::clone(&timeline);
            let summary = Arc::clone(&summary);
            let summary_err = Arc::clone(&summary);
            let log_data = Arc::clone(log);
            let log_err = Arc::clone(log);
            let errored_flag = Arc::clone(&errored);
            let start_data = start;
            let sample_format_data = sample_format.clone();

            let data_cb = move |data: &[$ty], _: &cpal::InputCallbackInfo| {
                let t_rel_s = start_data.elapsed().as_secs_f64();
                let frames = data.len() / channels;

                let mut sum_sq = 0f64;
                let mut peak = 0f32;
                let mut mono_count = 0usize;
                for frame in data.chunks_exact(channels) {
                    let mono =
                        frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32;
                    sum_sq += f64::from(mono) * f64::from(mono);
                    peak = peak.max(mono.abs());
                    mono_count += 1;
                }
                let rms = if mono_count > 0 {
                    (sum_sq / mono_count as f64).sqrt() as f32
                } else {
                    0.0
                };
                let silent = is_silent(peak, silence_threshold);

                let metrics = timeline
                    .lock()
                    .expect("spike timeline mutex poisoned")
                    .record(t_rel_s, frames as u64);
                summary
                    .lock()
                    .expect("spike summary mutex poisoned")
                    .observe(&metrics, silent);

                log_data.lock().expect("spike log mutex poisoned").row(&[
                    source.to_string(),
                    metrics.callback_index.to_string(),
                    format!("{:.3}", t_rel_s * 1000.0),
                    format!("{:.3}", metrics.gap_s * 1000.0),
                    frames.to_string(),
                    metrics.total_frames.to_string(),
                    sample_rate.to_string(),
                    channels.to_string(),
                    sample_format_data.clone(),
                    format!("{:.1}", metrics.expected_frames),
                    format!("{:.1}", metrics.drift_frames),
                    classify_gap(metrics.gap_s).as_str().to_string(),
                    silent.to_string(),
                    format!("{peak:.6}"),
                    format!("{rms:.6}"),
                    String::new(),
                ]);
            };

            let err_cb = move |err: cpal::StreamError| {
                let t_rel_s = start.elapsed().as_secs_f64();
                errored_flag.store(true, Ordering::Relaxed);
                let mut summary = summary_err.lock().expect("spike summary mutex poisoned");
                summary.stream_errors += 1;
                drop(summary);
                log_err.lock().expect("spike log mutex poisoned").row(&[
                    source.to_string(),
                    "0".to_string(),
                    format!("{:.3}", t_rel_s * 1000.0),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    String::new(),
                    "0".to_string(),
                    "0".to_string(),
                    String::new(),
                    String::new(),
                    "0".to_string(),
                    "0".to_string(),
                    err.to_string(),
                ]);
            };

            device.build_input_stream(&config.clone().into(), data_cb, err_cb, None)
        }};
    }

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32)?,
        cpal::SampleFormat::I16 => build!(i16)?,
        cpal::SampleFormat::I32 => build!(i32)?,
        cpal::SampleFormat::I8 => build!(i8)?,
        cpal::SampleFormat::U8 => build!(u8)?,
        other => {
            return Err(format!("unsupported sample format {other:?} for '{device_name}'").into())
        }
    };
    stream.play()?;

    Ok(SourceHandles {
        _stream: stream,
        summary,
        errored,
        sample_rate,
        channels: channel_count,
        sample_format,
        device_name: device_name.to_string(),
    })
}

fn write_summary(log: &mut CsvLog, source: &'static str, handles: &SourceHandles) {
    let s = handles
        .summary
        .lock()
        .expect("spike summary mutex poisoned");
    log.meta_line(&format!(
        "SUMMARY source={source} device=\"{dev}\" callbacks={cbs} total_frames={tf} wall_s={wall:.3} drift_frames={drift:.1} max_gap_ms={mg:.3} gaps(normal/sparse/long)={n}/{sp}/{lg} silent_frac={sf:.4} stream_errors={errs} config={rate}:{ch}:{fmt}",
        dev = handles.device_name,
        cbs = s.callbacks,
        tf = s.total_frames,
        wall = s.wall_s,
        drift = s.last_drift_frames,
        mg = s.max_gap_s * 1000.0,
        n = s.normal_gaps,
        sp = s.sparse_gaps,
        lg = s.long_gaps,
        sf = if s.callbacks > 0 {
            s.silent_callbacks as f64 / s.callbacks as f64
        } else {
            0.0
        },
        errs = s.stream_errors,
        rate = handles.sample_rate,
        ch = handles.channels,
        fmt = handles.sample_format,
    ));
}

fn default_log_path() -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    PathBuf::from(format!("meeting_loopback_spike_{stamp}.csv"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.list_devices {
        print_devices()?;
        return Ok(());
    }

    let duration = Duration::from_secs(cli.duration.min(MAX_DURATION_S));
    let output_log = cli.output_log.clone().unwrap_or_else(default_log_path);

    let (mic_device, mic_name) =
        find_device(cli.input_device.as_deref(), list_input_devices()?, "input")?;
    let (out_device, out_name) = find_device(
        cli.output_device.as_deref(),
        list_output_devices()?,
        "output",
    )?;

    println!("Microphone : {mic_name}");
    println!("Loopback   : {out_name}");
    println!("Duration   : {} s", duration.as_secs());
    println!("Log file   : {}", output_log.display());

    let log: LogHandle = Arc::new(Mutex::new(CsvLog::create(&output_log)?));
    {
        let mut l = log.lock().expect("spike log mutex poisoned");
        l.meta_line("meeting_loopback_spike run");
        l.meta_line(&format!("microphone_device={mic_name}"));
        l.meta_line(&format!("loopback_device={out_name}"));
        l.meta_line(&format!("duration_s={}", duration.as_secs()));
        l.meta_line(&format!("silence_threshold={}", cli.silence_threshold));
        l.row(&[
            "source".into(),
            "cb_index".into(),
            "t_rel_ms".into(),
            "gap_ms".into(),
            "frames".into(),
            "total_frames".into(),
            "sample_rate".into(),
            "channels".into(),
            "sample_format".into(),
            "expected_frames".into(),
            "drift_frames".into(),
            "gap_class".into(),
            "silent".into(),
            "peak".into(),
            "rms".into(),
            "error".into(),
        ]);
        l.flush();
    }

    // One shared monotonic clock for both sources: timestamps in every callback
    // are taken from the same Instant, per plan §7 "gemeinsame monotone
    // Zeitachse".
    let start = Instant::now();

    let mic = open_source(
        mic_device,
        &mic_name,
        false,
        start,
        cli.silence_threshold,
        &log,
    )
    .map_err(|e| format!("failed to open microphone '{mic_name}': {e}"))?;
    println!(
        "Microphone : open ({} Hz, {} ch, {})",
        mic.sample_rate, mic.channels, mic.sample_format
    );

    // Loopback failure must not kill the run: the plan explicitly wants a
    // microphone-only fallback mode to be measurable.
    let loopback = match open_source(
        out_device,
        &out_name,
        true,
        start,
        cli.silence_threshold,
        &log,
    ) {
        Ok(handles) => {
            println!(
                "Loopback   : open ({} Hz, {} ch, {})",
                handles.sample_rate, handles.channels, handles.sample_format
            );
            Some(handles)
        }
        Err(e) => {
            println!("Loopback   : FAILED to open ({e}) — continuing microphone-only");
            log.lock()
                .expect("spike log mutex poisoned")
                .meta_line(&format!("loopback_open_error={e}"));
            None
        }
    };

    {
        let mut l = log.lock().expect("spike log mutex poisoned");
        l.meta_line(&format!(
            "microphone_config={}:{}:{}",
            mic.sample_rate, mic.channels, mic.sample_format
        ));
        if let Some(lb) = &loopback {
            l.meta_line(&format!(
                "loopback_config={}:{}:{}",
                lb.sample_rate, lb.channels, lb.sample_format
            ));
        }
        l.meta_line("streams_started");
        l.flush();
    }

    println!("Capturing… (Ctrl+C aborts without summary)");
    let mut last_report = Instant::now();
    let mut mic_errored_seen = false;
    let mut loopback_errored_seen = !loopback.is_some();
    while start.elapsed() < duration {
        std::thread::sleep(Duration::from_millis(200));
        // Surface stream errors live so device loss, sleep/wake and
        // shared-mode failures show up during the run, not only in the CSV.
        if !mic_errored_seen && mic.errored.load(Ordering::Relaxed) {
            mic_errored_seen = true;
            println!("Microphone : STREAM ERROR detected — see error rows in log");
            log.lock()
                .expect("spike log mutex poisoned")
                .meta_line("stream_error_detected source=microphone");
        }
        if let Some(lb) = &loopback {
            if !loopback_errored_seen && lb.errored.load(Ordering::Relaxed) {
                loopback_errored_seen = true;
                println!("Loopback   : STREAM ERROR detected — see error rows in log");
                log.lock()
                    .expect("spike log mutex poisoned")
                    .meta_line("stream_error_detected source=loopback");
            }
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            println!("  {:>7.1}s elapsed", start.elapsed().as_secs_f64());
        }
    }

    // Streams stop when dropped at the end of main; they stay alive for the
    // whole run above.
    let elapsed = start.elapsed().as_secs_f64();
    let mut l = log.lock().expect("spike log mutex poisoned");
    l.meta_line(&format!("run_finished elapsed_s={elapsed:.3}"));
    write_summary(&mut l, "microphone", &mic);
    if let Some(lb) = &loopback {
        write_summary(&mut l, "loopback", lb);
    }
    l.flush();

    println!(
        "Done after {elapsed:.1}s. Log written to {}",
        output_log.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    #[test]
    fn first_callback_gap_equals_startup_latency() {
        let mut tl = SourceTimeline::new(RATE);
        let m = tl.record(0.0105, 480);
        assert_eq!(m.callback_index, 1);
        assert!((m.gap_s - 0.0105).abs() < 1e-9);
        assert_eq!(m.total_frames, 480);
    }

    #[test]
    fn gap_is_time_between_consecutive_callbacks() {
        let mut tl = SourceTimeline::new(RATE);
        tl.record(1.0, 480);
        let m = tl.record(1.01, 480);
        assert!((m.gap_s - 0.01).abs() < 1e-9);
        assert_eq!(m.total_frames, 960);
    }

    #[test]
    fn negative_time_deltas_are_clamped_to_zero() {
        let mut tl = SourceTimeline::new(RATE);
        tl.record(2.0, 480);
        let m = tl.record(1.999, 480);
        assert_eq!(m.gap_s, 0.0);
    }

    #[test]
    fn expected_frames_scales_with_elapsed_time() {
        assert!((expected_frames(1.0, RATE) - 48_000.0).abs() < 1e-6);
        assert!((expected_frames(0.5, RATE) - 24_000.0).abs() < 1e-6);
        assert_eq!(expected_frames(0.0, RATE), 0.0);
        assert_eq!(expected_frames(-1.0, RATE), 0.0);
    }

    #[test]
    fn zero_drift_for_gap_free_stream() {
        // 100 callbacks of 480 frames at exact 10 ms intervals: perfectly on clock.
        let mut tl = SourceTimeline::new(RATE);
        let mut t = 0.0;
        let mut last = None;
        for _ in 0..100 {
            t += 0.01;
            last = Some(tl.record(t, 480));
        }
        assert!(last.unwrap().drift_frames.abs() < 1e-6);
    }

    #[test]
    fn slow_clock_produces_negative_drift() {
        // After 10 s only 479_900 frames were delivered: exactly 100 behind.
        let mut tl = SourceTimeline::new(RATE);
        let mut t = 0.0;
        let mut last = None;
        for _ in 0..100 {
            t += 0.1;
            last = Some(tl.record(t, 4_799));
        }
        let m = last.unwrap();
        assert!((m.expected_frames - 480_000.0).abs() < 1e-6);
        assert!((m.drift_frames - (-100.0)).abs() < 1e-6);
    }

    #[test]
    fn extra_samples_produce_positive_drift() {
        let drift = drift_frames(48_001, expected_frames(1.0, RATE));
        assert!((drift - 1.0).abs() < 1e-6);
    }

    #[test]
    fn gap_classification_buckets() {
        assert_eq!(classify_gap(0.0), GapClass::Normal);
        assert_eq!(classify_gap(0.01), GapClass::Normal);
        assert_eq!(classify_gap(GAP_NORMAL_MAX_S), GapClass::Normal);
        assert_eq!(classify_gap(0.26), GapClass::Sparse);
        assert_eq!(classify_gap(GAP_SPARSE_MAX_S), GapClass::Sparse);
        assert_eq!(classify_gap(1.001), GapClass::Gap);
        assert_eq!(classify_gap(30.0), GapClass::Gap);
    }

    #[test]
    fn silence_detection_uses_peak_threshold() {
        assert!(is_silent(0.0, 1e-4));
        assert!(is_silent(-5e-5, 1e-4));
        assert!(!is_silent(2e-4, 1e-4));
        assert!(!is_silent(0.5, 1e-4));
    }

    #[test]
    fn csv_fields_with_special_characters_are_quoted() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn summary_accumulates_gap_histogram_and_silence() {
        let mut tl = SourceTimeline::new(RATE);
        let mut sum = SourceSummary::default();
        let mut t = 0.0;
        // normal, normal, sparse, long(silent)
        for (dt, frames, silent) in [
            (0.01, 480, false),
            (0.01, 480, false),
            (0.5, 480, false),
            (2.0, 480, true),
        ] {
            t += dt;
            let m = tl.record(t, frames);
            sum.observe(&m, silent);
        }
        assert_eq!(sum.callbacks, 4);
        assert_eq!(sum.total_frames, 1_920);
        assert_eq!(sum.normal_gaps, 2); // gaps: 10 ms, 10 ms, 500 ms, 2 s
        assert_eq!(sum.sparse_gaps, 1);
        assert_eq!(sum.long_gaps, 1);
        assert_eq!(sum.silent_callbacks, 1);
        assert!((sum.max_gap_s - 2.0).abs() < 1e-9);
        assert!((sum.wall_s - 2.52).abs() < 1e-9);
    }

    #[test]
    fn resolve_device_matches_by_index_substring_and_default() {
        let devices = vec![
            DeviceEntry {
                name: "Internal Speakers".into(),
                is_default: true,
            },
            DeviceEntry {
                name: "Headset".into(),
                is_default: false,
            },
        ];
        // Substring, case-insensitive.
        assert_eq!(
            resolve_device(Some("head"), devices.clone(), "output")
                .unwrap()
                .name,
            "Headset"
        );
        // Index.
        assert_eq!(
            resolve_device(Some("0"), devices.clone(), "output")
                .unwrap()
                .name,
            "Internal Speakers"
        );
        // Default when unspecified.
        assert_eq!(
            resolve_device(None, devices.clone(), "output")
                .unwrap()
                .name,
            "Internal Speakers"
        );
        // No match errors.
        assert!(resolve_device(Some("nope"), devices.clone(), "output").is_err());
        // Out-of-range index errors.
        assert!(resolve_device(Some("7"), devices, "output").is_err());
        // Empty device list errors.
        assert!(resolve_device(None, Vec::new(), "input").is_err());
    }
}
