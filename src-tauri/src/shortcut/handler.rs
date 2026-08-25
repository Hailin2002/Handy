//! Shared shortcut event handling logic
//!
//! This module contains the common logic for handling shortcut events,
//! used by both the Tauri and handy-keys implementations.

use log::warn;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::meeting::{MeetingManager, MeetingState};
use crate::settings::{get_settings, MEETING_BINDING_PAUSE_RESUME, MEETING_BINDING_START_STOP};
use crate::transcription_coordinator::is_transcribe_binding;
use crate::TranscriptionCoordinator;

/// Which meeting action a binding addresses (plan 6.9 rows 3 and 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingShortcutAction {
    /// Start a meeting, or stop the running one.
    StartStop,
    /// Pause the running meeting, or resume the paused one.
    PauseResume,
}

/// Where a shortcut press is dispatched.
///
/// Pure function of the binding id, so the routing rules of plan 6.9/6.10
/// ("Diktat und Meeting sind nicht verwechselbar"; "ein laufendes Meeting
/// verhindert kein kurzes Diktat") are unit-testable without an app handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutRoute {
    /// Handled by the dictation coordinator; never influenced by meetings.
    Dictation,
    /// Handled by the meeting state machine.
    Meeting(MeetingShortcutAction),
    /// The dynamic dictation-cancel binding.
    Cancel,
    /// Anything else in `ACTION_MAP`.
    Other,
}

/// Classifies one binding id. The two tool families own disjoint ids.
pub fn route_binding(binding_id: &str) -> ShortcutRoute {
    if is_transcribe_binding(binding_id) {
        return ShortcutRoute::Dictation;
    }
    match binding_id {
        MEETING_BINDING_START_STOP => ShortcutRoute::Meeting(MeetingShortcutAction::StartStop),
        MEETING_BINDING_PAUSE_RESUME => ShortcutRoute::Meeting(MeetingShortcutAction::PauseResume),
        "cancel" => ShortcutRoute::Cancel,
        _ => ShortcutRoute::Other,
    }
}

/// Applies a meeting shortcut press.
///
/// Meeting shortcuts are always toggles on key-down; they deliberately ignore
/// the dictation push-to-talk setting so neither tool's behaviour leaks into
/// the other (plan 6.10).
///
/// Starting is intentionally not done silently: the consent gate of
/// `session-state-contract.md` 7 is fail-closed until the consent audit lands,
/// so the shortcut brings up the start dialog instead of opening capture
/// devices without an acknowledged consent. Stopping, pausing and resuming an
/// already-consented session are applied directly.
pub fn dispatch_meeting_action(app: &AppHandle, action: MeetingShortcutAction) {
    let Some(manager) = app.try_state::<Arc<MeetingManager>>() else {
        warn!("MeetingManager is not initialized");
        return;
    };
    let manager = manager.inner().clone();
    let active = manager.active_capture_session();
    let app = app.clone();

    // The stop path blocks until every open chunk is durably finalized, so it
    // must not run on the key-event thread.
    let spawned = std::thread::Builder::new()
        .name("meeting-shortcut".into())
        .spawn(move || match (action, active) {
            (MeetingShortcutAction::StartStop, Some(meeting_id)) => {
                if let Err(e) = manager.stop(&meeting_id, false) {
                    warn!("Meeting shortcut stop failed: {}", e);
                }
            }
            (MeetingShortcutAction::StartStop, None) => {
                crate::utils::show_main_window(&app);
            }
            (MeetingShortcutAction::PauseResume, Some(meeting_id)) => {
                let result = match manager.active_capture_state() {
                    Some(MeetingState::Recording) => manager.pause(&meeting_id).map(|_| ()),
                    Some(MeetingState::Paused) => manager.resume(&meeting_id).map(|_| ()),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    warn!("Meeting shortcut pause/resume failed: {}", e);
                }
            }
            (MeetingShortcutAction::PauseResume, None) => {}
        });
    if let Err(e) = spawned {
        warn!("Could not spawn meeting shortcut worker: {}", e);
    }
}

/// Handle a shortcut event from either implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding (only fires when recording)
/// - Handling push-to-talk mode (start on press, stop on release)
/// - Handling toggle mode (toggle state on press only)
///
/// # Arguments
/// * `app` - The Tauri app handle
/// * `binding_id` - The ID of the binding (e.g., "transcribe", "cancel")
/// * `hotkey_string` - The string representation of the hotkey
/// * `is_pressed` - Whether this is a key press (true) or release (false)
pub fn handle_shortcut_event(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    let settings = get_settings(app);

    // Transcribe bindings are handled by the coordinator. This path is
    // unchanged and never consults meeting state (plan 6.10).
    if is_transcribe_binding(binding_id) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.send_input(binding_id, hotkey_string, is_pressed, settings.push_to_talk);
        } else {
            warn!("TranscriptionCoordinator is not initialized");
        }
        return;
    }

    // Meeting bindings act on key-down only and bypass ACTION_MAP entirely:
    // the meeting state machine, not a dictation action, owns them.
    if let ShortcutRoute::Meeting(action) = route_binding(binding_id) {
        if is_pressed {
            dispatch_meeting_action(app, action);
        }
        return;
    }

    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!(
            "No action defined in ACTION_MAP for shortcut ID '{}'. Shortcut: '{}', Pressed: {}",
            binding_id, hotkey_string, is_pressed
        );
        return;
    };

    // Cancel binding: only fires when recording and key is pressed
    if binding_id == "cancel" {
        let audio_manager = app.state::<Arc<AudioRecordingManager>>();
        if audio_manager.is_recording() && is_pressed {
            action.start(app, binding_id, hotkey_string);
        }
        return;
    }

    // Remaining bindings (e.g. "test") use simple start/stop on press/release.
    if is_pressed {
        action.start(app, binding_id, hotkey_string);
    } else {
        action.stop(app, binding_id, hotkey_string);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    /// Plan 6.9: every action has its own clearly named entry, and the two
    /// tool families route apart.
    #[test]
    fn meeting_and_dictation_bindings_route_independently() {
        assert_eq!(route_binding("transcribe"), ShortcutRoute::Dictation);
        assert_eq!(
            route_binding("transcribe_with_post_process"),
            ShortcutRoute::Dictation
        );
        assert_eq!(route_binding("cancel"), ShortcutRoute::Cancel);
        assert_eq!(
            route_binding(MEETING_BINDING_START_STOP),
            ShortcutRoute::Meeting(MeetingShortcutAction::StartStop)
        );
        assert_eq!(
            route_binding(MEETING_BINDING_PAUSE_RESUME),
            ShortcutRoute::Meeting(MeetingShortcutAction::PauseResume)
        );
        // No meeting id is ever mistaken for a dictation id, in either
        // direction (plan 6.9: "nicht verwechselbar").
        assert!(!is_transcribe_binding(MEETING_BINDING_START_STOP));
        assert!(!is_transcribe_binding(MEETING_BINDING_PAUSE_RESUME));
        assert!(!crate::settings::is_meeting_binding("transcribe"));
        assert!(!crate::settings::is_meeting_binding("cancel"));
    }

    /// The meeting bindings exist in the settings schema, carry their own
    /// defaults, and do not collide with the dictation defaults.
    #[test]
    fn default_bindings_contain_independent_meeting_entries() {
        let defaults = get_default_settings();
        let start = defaults
            .bindings
            .get(MEETING_BINDING_START_STOP)
            .expect("meeting start/stop binding");
        let pause = defaults
            .bindings
            .get(MEETING_BINDING_PAUSE_RESUME)
            .expect("meeting pause/resume binding");
        assert_eq!(start.id, MEETING_BINDING_START_STOP);
        assert!(!start.current_binding.is_empty());
        // Plan 6.9: pause/resume ships unassigned.
        assert!(pause.current_binding.is_empty());
        assert!(!pause.default_binding.is_empty());

        let dictation: Vec<&str> = defaults
            .bindings
            .values()
            .filter(|b| !crate::settings::is_meeting_binding(&b.id))
            .map(|b| b.current_binding.as_str())
            .collect();
        assert!(
            !dictation.contains(&start.current_binding.as_str()),
            "meeting default must not collide with a dictation default"
        );
        // The dictation defaults themselves are untouched.
        assert_eq!(defaults.bindings["cancel"].current_binding, "escape");
        assert_eq!(defaults.bindings["transcribe"].id, "transcribe");
    }
}
