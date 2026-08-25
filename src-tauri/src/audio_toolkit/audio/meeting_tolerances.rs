//! Tolerance constants for the meeting recorder (plan §7 "Gemeinsame
//! Zeitachse", contract `docs/meeting-mode-architecture/recorder-contract.md`
//! §5.1).
//!
//! The contract requires that these live in **one** module so that revising a
//! measured value is a single-file change plus test updates. Every value below
//! is either fixed by the plan/contract or derived from the Phase-1 spike
//! measurements; none is guessed.
//!
//! # Measurement basis
//!
//! Source data (CSV schema in `src-tauri/src/bin/meeting_loopback_spike.rs`):
//!
//! - `docs/phase1-spike/run-20260822/meeting_loopback_spike_120min.csv`
//!   (9 743 callbacks per source, 97.4 s span, 48 kHz/2ch/f32)
//! - `docs/phase1-spike/run-20260822-r2/...` (17 967 callbacks, 179.7 s)
//! - `docs/phase1-spike/run-20260822-r3/...` (14 658 callbacks, 146.6 s)
//! - `docs/phase1-spike/tools/runs/smoke-final-verify/meeting_loopback_spike_smoke.csv`
//!   (mic 8 990 callbacks / 90.1 s, loopback 3 034 callbacks / 36.1 s)
//!
//! Aggregated from the `gap_ms` column, restricted to `gap_class='normal'`:
//!
//! | run | source | p50 | p99 | p99.9 | max |
//! | --- | --- | --- | --- | --- | --- |
//! | r1 | microphone | 9.94 | 20.88 | 21.22 | 21.41 |
//! | r1 | loopback | 10.01 | 11.15 | 19.40 | 30.78 |
//! | r2 | microphone | 10.05 | 20.70 | 21.08 | 21.36 |
//! | r2 | loopback | 10.04 | 11.12 | 11.62 | 30.24 |
//! | r3 | microphone | 10.01 | 11.11 | 19.98 | 21.03 |
//! | r3 | loopback | 10.01 | 11.06 | 11.63 | 30.82 |
//! | smoke | microphone | 10.00 | 10.10 | 34.42 | **78.54** |
//! | smoke | loopback | 10.03 | 19.21 | 21.38 | 62.40 |
//!
//! Clock deviation, measured as delivered-frames vs. elapsed-time over
//! *contiguous* callback runs only (all consecutive callbacks with
//! `gap_ms <= 250`, i.e. excluding starvation intervals):
//!
//! | run | microphone | loopback |
//! | --- | --- | --- |
//! | r1 | −3.6 ppm | +7.2 ppm |
//! | r2 | −3.3 ppm | +3.1 ppm |
//! | r3 | +6.7 ppm | +70.3 ppm |
//!
//! (The smoke run's whole-stream ppm figures — −2 287 / −1 222 ppm — are *not*
//! clock drift: that run contains a 5.7 s loopback starvation interval and a
//! loopback stream that stops delivering entirely once playback ends. See
//! [`SEGMENT_DEGRADED_GAP_MS`] for why that case must not be treated as a
//! fault.)
//!
//! # Key qualitative finding: silent loopback delivers nothing
//!
//! In `smoke-final-verify` the loopback source produced its *first* callback at
//! `t_rel_ms = 5709` — exactly when the test tone started — and its last at
//! 36.1 s while the microphone kept running to 90.1 s. WASAPI loopback on a
//! render device with no active playback delivers **no callbacks at all**, not
//! silent ones. Consequently, for [`crate::audio_toolkit::audio::meeting_recorder::SourceKind::System`]
//! an arbitrarily long "gap" is the normal, legitimate state (contract §7.1
//! "Keine Systemlautstärke: **No error**"), and must be represented as silence
//! on the timeline without ever flagging the source degraded.

/// Target sample rate of the shared timeline. Fixed by plan §7.
pub const TIMELINE_SAMPLE_RATE_HZ: u32 = 16_000;

/// Target channel count of the shared timeline. Fixed by plan §7.
pub const TIMELINE_CHANNELS: u16 = 1;

/// Timeline samples per millisecond (16 kHz → 16 samples/ms). Exact integer,
/// which is why all timeline positions can be expressed losslessly in samples.
pub const TIMELINE_SAMPLES_PER_MS: f64 = TIMELINE_SAMPLE_RATE_HZ as f64 / 1000.0;

/// Rollover target per chunk. Initial value fixed by plan §7; Phase 1 did not
/// produce evidence for a different value, so it stays at 60 s.
pub const CHUNK_ROLLOVER_TARGET_MS: u64 = 60_000;

/// Level-event throttle. UI constant, not spike-derived.
pub const LEVEL_EVENT_INTERVAL_MS: u64 = 100;

/// Spike-harness gap-class boundary `normal`. Identical to
/// `docs/phase1-spike/tools/analyze-spike-csv.ps1` so recorder telemetry and
/// spike analysis stay directly comparable.
pub const GAP_CLASS_NORMAL_MAX_MS: f64 = 250.0;

/// Spike-harness gap-class boundary `sparse`.
pub const GAP_CLASS_SPARSE_MAX_MS: f64 = 1000.0;

/// Peak amplitude below which a block counts as silent. Taken from the spike
/// run metadata line `# silence_threshold=0.0001`.
pub const SILENCE_PEAK_THRESHOLD: f32 = 0.0001;

/// Inter-callback delta below which no gap bookkeeping happens; ordinary
/// scheduling jitter is absorbed by the drift controller instead of being
/// silence-filled.
///
/// **Measured basis:** the largest `gap_ms` ever observed in a `normal`-class
/// callback across all four runs is 78.54 ms (smoke run, microphone); the p99.9
/// across the three long runs is ≤ 21.22 ms. 100 ms sits above every observed
/// jitter sample with ~27 % headroom over the worst case, and well below the
/// harness `normal` class boundary of 250 ms, so a genuine dropout is still
/// classified as a gap.
pub const CALLBACK_JITTER_NORMAL_MAX_MS: f64 = 100.0;

/// Smallest silence-filled gap that is surfaced to the UI as
/// `RecorderEvent::TimelineGap`.
///
/// **Measured basis:** set to the harness `normal`/`sparse` class boundary so
/// the recorder reports exactly the gaps the spike analyzer counts as
/// non-`normal`. Across the three long runs the histogram is 100 % `normal`,
/// i.e. a correctly working device reports nothing at all.
pub const GAP_EVENT_MIN_MS: f64 = GAP_CLASS_NORMAL_MAX_MS;

/// Gap above which a **microphone** source segment is flagged `Degraded`.
///
/// **Measured basis:** the harness `sparse`/`gap` boundary. No microphone
/// callback in any run exceeded 78.54 ms, so a full second of missing
/// microphone audio is unambiguously a fault.
///
/// Deliberately *not* applied to [`crate::audio_toolkit::audio::meeting_recorder::SourceKind::System`]:
/// the smoke run shows silent loopback delivering zero callbacks for 5.7 s at
/// startup and stopping entirely after playback ends. Per contract §7.1 that is
/// not an error, so system-source gaps are silence-filled and reported but
/// never degrade the source. System-audio *faults* surface as
/// `RecorderEvent::SystemDeviceLost` via the cpal error callback instead.
pub const SEGMENT_DEGRADED_GAP_MS: f64 = GAP_CLASS_SPARSE_MAX_MS;

/// Maximum leading offset between session start and a source's first callback
/// that is pre-filled with silence without raising a startup warning.
///
/// **Measured basis:** the first microphone callback arrives at
/// `t_rel_ms = 11.48` (r3, `cb_index=1`); comparable in the other runs. 2 000 ms
/// leaves ~170× headroom for slow USB/Bluetooth device warm-up. Only evaluated
/// for the microphone — the 5 709 ms first-loopback-callback in the smoke run is
/// the legitimate silent-playback case and must not warn.
pub const INITIAL_OFFSET_FILL_TOLERANCE_MS: f64 = 2000.0;

/// Runtime below which drift is not evaluated at all (device warm-up), matching
/// the stable window used by `analyze-spike-csv.ps1`.
pub const DRIFT_STABLE_WINDOW_MS: f64 = 10_000.0;

/// Maximum resampler-ratio offset applied for slow-clock compensation.
///
/// **Measured basis:** the largest genuine contiguous-run deviation measured is
/// +70.3 ppm (r3, loopback). 200 ppm gives ~2.8× headroom over the worst
/// observed device while staying far below audible pitch change (200 ppm ≈
/// 0.0035 semitone).
pub const DRIFT_CORRECTION_MAX_PPM: f64 = 200.0;

/// Deadband: computed corrections smaller than this are not applied.
///
/// **Measured basis:** the run-to-run scatter of the contiguous-run ppm figure
/// on well-behaved devices is ±7 ppm (−3.6 / −3.3 / +6.7 for the microphone).
/// 25 ppm sits above that measurement noise, so a healthy device is never
/// retuned, while the +70.3 ppm outlier is corrected. It is also just above the
/// 1 Hz quantisation step of the integer effective input rate at 48 kHz
/// (1/48 000 ≈ 20.8 ppm), so every accepted correction is actually
/// representable.
pub const DRIFT_DEADBAND_PPM: f64 = 25.0;

/// Correction horizon: a measured drift is steered to zero over this span,
/// which keeps the applied ppm small and the correction inaudible.
pub const DRIFT_CORRECTION_HORIZON_MS: f64 = 60_000.0;

/// Minimum interval between two resampler retunes per source (contract §5.3.3:
/// "the resampler is retuned at most once per second per source").
pub const DRIFT_RETUNE_MIN_INTERVAL_MS: f64 = 1000.0;

/// Drift magnitude (in 16 kHz timeline frames) that flips a source to
/// `Degraded`.
///
/// **Measured basis:** 1 600 frames = 100 ms. At the worst measured genuine
/// drift rate (70.3 ppm ≈ 253 ms/h ≈ 4 048 timeline frames/h) an *uncorrected*
/// source needs ~24 minutes to reach it, while a corrected source converges
/// within the 60 s horizon. Reaching it therefore means correction is failing.
pub const DRIFT_WARN_FRAMES: i64 = 1_600;

/// Drift magnitude (in 16 kHz timeline frames) at which correction is abandoned
/// and `DriftToleranceExceeded` is emitted.
///
/// **Measured basis:** 16 000 frames = 1 000 ms. The worst measured device
/// accumulates 4 048 frames/h uncorrected, i.e. ~8 100 frames over a 2 h
/// meeting — this bound is ~2× that, so it cannot be reached by normal hardware
/// drift and only trips on a genuinely broken clock.
pub const DRIFT_EXCEEDED_FRAMES: i64 = 16_000;

/// Free-space watermark below which `DiskSpaceLow` is emitted.
///
/// Product constant, **not** spike-derived. Proposal per contract §5.1: ≥ 2 ×
/// the hourly two-track budget (≈ 230 MB/h, plan §12) → 512 MiB. Pending
/// product decision.
pub const DISK_LOW_WATERMARK_BYTES: u64 = 512 * 1024 * 1024;

/// Upper bound on the silence inserted for a single detected gap.
///
/// Guards against a pathological monotonic-clock jump turning into an unbounded
/// write loop. One hour of 16 kHz PCM16 silence is 115 MB — already far beyond
/// any real dropout, and the sleep/wake scenario is handled by the same path.
pub const MAX_SINGLE_GAP_FILL_MS: f64 = 3_600_000.0;

/// Capture blocks that may sit in a source's writer queue before the capture
/// callback starts dropping them.
///
/// The measured callback cadence is ~10 ms (p50 ≈ 10.0 ms in every run), so 512
/// blocks ≈ 5 s of backlog. Dropped blocks are not lost time: the gap detector
/// silence-fills the hole, keeping the timeline exact. This is the hard bound
/// that makes recorder RAM independent of meeting duration.
pub const MAX_QUEUED_CAPTURE_BLOCKS: usize = 512;

/// Frame duration handed to [`crate::audio_toolkit::audio::FrameResampler`];
/// 20 ms at 16 kHz = 320 samples per emitted frame.
pub const RESAMPLER_FRAME_MS: u64 = 20;
