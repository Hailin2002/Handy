//! Names of the events the backend emits to the webview.
//!
//! # Why these are constants
//!
//! These names were string literals spread across ten modules, several of them
//! emitted from more than one place — `model-download-complete` from three.
//! A typo in one emitter produces no error anywhere: the event simply never
//! arrives, and the symptom is a frontend that quietly stops updating.
//!
//! Collecting them here makes a misspelling a compile error on the emitting
//! side and gives one place to see the whole event surface.
//!
//! # What this does not fix
//!
//! The listener side is still a string literal in TypeScript, so a typo *there*
//! remains silent. Closing that properly means exporting these names into
//! `src/bindings.ts` through `tauri_specta::Builder::constant` and having the
//! frontend import them — which needs a bindings regeneration, now possible via
//! `bun run bindings:generate`. Worth doing; deliberately not bundled into the
//! commit that introduced the constants.
//!
//! Payload-carrying events registered through `collect_events!` in `lib.rs` are
//! a separate, already-typed mechanism and are not listed here.

/// The user asked for an update check (tray menu or the updater UI).
pub const CHECK_FOR_UPDATES: &str = "check-for-updates";

/// A raw key event from the handy-keys backend, for the shortcut recorder.
pub const HANDY_KEYS_EVENT: &str = "handy-keys-event";

/// macOS secure-input state changed; the warning banner follows this.
pub const SECURE_INPUT_CHANGED: &str = "secure-input-changed";

/// The active theme changed and the webview should re-read it.
pub const THEME_CHANGED: &str = "theme-changed";

// -- recording overlay -------------------------------------------------------

/// Show the recording overlay window.
pub const SHOW_OVERLAY: &str = "show-overlay";

/// Hide the recording overlay window.
pub const HIDE_OVERLAY: &str = "hide-overlay";

/// Capture is live and the overlay may start animating.
pub const RECORDING_READY: &str = "recording-ready";

/// Input level sample driving the overlay's meter.
pub const MIC_LEVEL: &str = "mic-level";

// -- transcription and paste -------------------------------------------------

/// Transcription failed; carries a user-facing message.
pub const TRANSCRIPTION_ERROR: &str = "transcription-error";

/// Delivering the transcript to the focused application failed.
pub const PASTE_ERROR: &str = "paste-error";

// -- model lifecycle ---------------------------------------------------------

/// Byte-level progress of a model download.
pub const MODEL_DOWNLOAD_PROGRESS: &str = "model-download-progress";

/// A model download finished successfully.
pub const MODEL_DOWNLOAD_COMPLETE: &str = "model-download-complete";

/// A model download was cancelled by the user.
pub const MODEL_DOWNLOAD_CANCELLED: &str = "model-download-cancelled";

/// Checksum verification of a downloaded model started.
pub const MODEL_VERIFICATION_STARTED: &str = "model-verification-started";

/// Checksum verification finished.
pub const MODEL_VERIFICATION_COMPLETED: &str = "model-verification-completed";

/// Extraction of a downloaded archive started.
pub const MODEL_EXTRACTION_STARTED: &str = "model-extraction-started";

/// Extraction finished.
pub const MODEL_EXTRACTION_COMPLETED: &str = "model-extraction-completed";

/// A model was removed from disk.
pub const MODEL_DELETED: &str = "model-deleted";

/// The installed-model list changed and should be re-fetched.
pub const MODELS_UPDATED: &str = "models-updated";

#[cfg(test)]
mod tests {
    /// The names are a wire contract with the frontend's `listen(...)` calls,
    /// which are still literals over there. Pinning them here means a rename
    /// has to be deliberate enough to also touch this test — and to go looking
    /// for the listener.
    #[test]
    fn event_names_are_pinned() {
        assert_eq!(super::CHECK_FOR_UPDATES, "check-for-updates");
        assert_eq!(super::HANDY_KEYS_EVENT, "handy-keys-event");
        assert_eq!(super::SECURE_INPUT_CHANGED, "secure-input-changed");
        assert_eq!(super::THEME_CHANGED, "theme-changed");
        assert_eq!(super::SHOW_OVERLAY, "show-overlay");
        assert_eq!(super::HIDE_OVERLAY, "hide-overlay");
        assert_eq!(super::RECORDING_READY, "recording-ready");
        assert_eq!(super::MIC_LEVEL, "mic-level");
        assert_eq!(super::TRANSCRIPTION_ERROR, "transcription-error");
        assert_eq!(super::PASTE_ERROR, "paste-error");
        assert_eq!(super::MODEL_DOWNLOAD_PROGRESS, "model-download-progress");
        assert_eq!(super::MODEL_DOWNLOAD_COMPLETE, "model-download-complete");
        assert_eq!(super::MODEL_DOWNLOAD_CANCELLED, "model-download-cancelled");
        assert_eq!(
            super::MODEL_VERIFICATION_STARTED,
            "model-verification-started"
        );
        assert_eq!(
            super::MODEL_VERIFICATION_COMPLETED,
            "model-verification-completed"
        );
        assert_eq!(super::MODEL_EXTRACTION_STARTED, "model-extraction-started");
        assert_eq!(
            super::MODEL_EXTRACTION_COMPLETED,
            "model-extraction-completed"
        );
        assert_eq!(super::MODEL_DELETED, "model-deleted");
        assert_eq!(super::MODELS_UPDATED, "models-updated");
    }
}
