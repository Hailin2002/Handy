//! Real-microphone capture, end to end: enumerate → open → record → stop.
//!
//! # Why this exists
//!
//! `AudioRecorder`'s unit tests cover error classification and state
//! transitions against fakes. Nothing opened an actual device, so the step that
//! fails on a user's machine — cpal picking a host, negotiating a config the
//! driver accepts, and delivering callbacks — had no coverage at all. The
//! Phase-0 baseline records the same gap from the other direction (L6: the
//! dictation baseline was never executed on hardware).
//!
//! # Silence is a pass
//!
//! These tests do not assert on *content*. Nobody is speaking into the
//! microphone on a build machine, and a muted or silent input is a legitimate
//! outcome. What is asserted is that the pipeline ran: a device was opened, the
//! stream produced callbacks, and `stop()` returned a buffer whose length is
//! consistent with the time spent recording. A driver that accepts `open()` and
//! then never calls back — the actual failure mode behind "the app records
//! nothing" — fails here.
//!
//! # Skipping
//!
//! CI runners have no audio hardware, so every test skips (passes with a
//! printed note) when no input device is present. Set
//! `SPEAK_REQUIRE_AUDIO_HARDWARE=1` to turn a missing device into a failure —
//! useful on a machine where a device is expected and its absence is the bug.

use speak_app_lib::audio_toolkit::audio::{list_input_devices, AudioRecorder, VadPolicy};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Long enough that any sane device delivers several callbacks, short enough
/// that the suite stays quick.
const RECORD_FOR: Duration = Duration::from_millis(600);

/// Returns false (and explains) when there is no capture hardware.
fn audio_available() -> bool {
    match list_input_devices() {
        Ok(devices) if !devices.is_empty() => true,
        other => {
            let reason = match other {
                Ok(_) => "no input devices enumerated".to_string(),
                Err(e) => format!("device enumeration failed: {e}"),
            };
            if std::env::var("SPEAK_REQUIRE_AUDIO_HARDWARE").as_deref() == Ok("1") {
                panic!("SPEAK_REQUIRE_AUDIO_HARDWARE=1 but {reason}");
            }
            eprintln!("skipping microphone capture test: {reason}");
            false
        }
    }
}

#[test]
fn input_devices_can_be_enumerated_and_carry_usable_names() {
    if !audio_available() {
        return;
    }
    let devices = list_input_devices().expect("enumeration succeeded above");

    for device in &devices {
        assert!(
            !device.name.trim().is_empty(),
            "a device with a blank name cannot be selected in settings, \
             and would be indistinguishable from another blank one"
        );
    }

    let defaults = devices.iter().filter(|d| d.is_default).count();
    assert!(
        defaults <= 1,
        "at most one input device may claim to be the default, found {defaults}"
    );
    eprintln!("enumerated {} input device(s)", devices.len());
}

#[test]
fn recording_from_the_default_device_delivers_samples() {
    if !audio_available() {
        return;
    }

    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&callbacks);

    let mut recorder = AudioRecorder::new()
        .expect("constructing a recorder must not require a stream")
        .with_audio_callback(move |samples: &[f32]| {
            if !samples.is_empty() {
                seen.fetch_add(samples.len(), Ordering::Relaxed);
            }
        });

    // `None` = the system default input, which is what the app uses until the
    // user picks one in settings.
    recorder
        .open(None)
        .expect("opening the default input device failed");

    // VAD off: this measures capture, not speech detection, and silence must
    // still count as a working stream.
    let ready = recorder
        .start(VadPolicy::Disabled)
        .expect("starting the stream failed");

    let started = Instant::now();
    // `start` resolves this only once a real sample chunk has entered the
    // capture path — `Stream::play()` returning proves nothing, as the comment
    // on `start` notes for slow Bluetooth and USB devices.
    ready
        .recv_timeout(Duration::from_secs(5))
        .expect("the stream started but never delivered a first sample chunk");
    std::thread::sleep(RECORD_FOR);
    let elapsed = started.elapsed();

    let samples = recorder.stop().expect("stopping the stream failed");
    recorder.close().expect("closing the device failed");

    let delivered = callbacks.load(Ordering::Relaxed);
    assert!(
        delivered > 0,
        "the device opened and started but never delivered a callback in {elapsed:?} — \
         this is the failure mode behind 'the app records nothing'"
    );

    // The recorder resamples to its working rate, so compare against a
    // deliberately loose floor rather than an exact count: a tenth of the
    // recorded wall time at 16 kHz. Anything above zero but far below this
    // means the stream stalled after starting.
    let floor = (RECORD_FOR.as_secs_f64() * 16_000.0 * 0.1) as usize;
    assert!(
        samples.len() >= floor,
        "recorded {} samples for {elapsed:?}; expected at least {floor}",
        samples.len()
    );
    eprintln!(
        "captured {} samples ({} delivered to the callback) in {elapsed:?}",
        samples.len(),
        delivered
    );
}

#[test]
fn a_recorder_can_be_reopened_after_being_closed() {
    if !audio_available() {
        return;
    }

    // Matches what the app does between dictations in on-demand microphone
    // mode: open, record, close, and later open again. A device left in a bad
    // state by the first cycle shows up here.
    let mut recorder = AudioRecorder::new().expect("constructing a recorder");

    for cycle in 1..=2 {
        recorder
            .open(None)
            .unwrap_or_else(|e| panic!("cycle {cycle}: open failed: {e}"));
        let ready = recorder
            .start(VadPolicy::Disabled)
            .unwrap_or_else(|e| panic!("cycle {cycle}: start failed: {e}"));
        ready
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("cycle {cycle}: no first sample chunk: {e}"));
        std::thread::sleep(Duration::from_millis(150));
        recorder
            .stop()
            .unwrap_or_else(|e| panic!("cycle {cycle}: stop failed: {e}"));
        recorder
            .close()
            .unwrap_or_else(|e| panic!("cycle {cycle}: close failed: {e}"));
        assert!(
            !recorder.needs_reopen(),
            "cycle {cycle}: a clean close must not leave the recorder needing a reopen"
        );
    }
}
