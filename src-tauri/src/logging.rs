//! Central log-level plumbing shared by the Tauri log target filters
//! ([`crate::run`]) and the runtime verbosity commands
//! (`set_log_level`, debug-mode toggle).
//!
//! The file log level lives in an atomic so the log target filters — which run
//! on every record — never touch the Tauri store. The webview streaming flag
//! works the same way: log records must not be broadcast onto the frontend
//! event bus unless a consumer (the debug panel's live log viewer) exists.

use env_filter::Builder as EnvFilterBuilder;

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the webview via the
/// `log://log` event for the debug panel's live log viewer. Gated on debug
/// mode — the live log viewer is its only consumer and only exists in debug
/// mode — so normal runs never broadcast log records (which can include file
/// paths or transcribed text) onto the frontend event bus. Synced at startup
/// and whenever debug mode is toggled (see `shortcut::change_debug_mode_setting`).
pub static WEBVIEW_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

pub fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

/// Applies a new file-log level and webview-streaming flag in one place so
/// startup, `set_log_level`, and the debug-mode toggle stay consistent.
pub fn set_file_log_level(level: log::LevelFilter) {
    FILE_LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn set_webview_log_streaming(enabled: bool) {
    WEBVIEW_LOG_STREAMING.store(enabled, Ordering::Relaxed);
}
